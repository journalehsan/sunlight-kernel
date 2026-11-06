#![cfg(all(feature = "dom", feature = "svg"))]
use golden_fish::parse_html;
use rappid_rabbit::{
    css::{parse_stylesheet, Property, StyleContext, StylesheetSource},
    images::{decode_image, ImageCache},
    render::{DocumentRenderState, TextMeasurer},
};
use sunlight_ui::{
    widgets::{DocumentCanvas, DocumentCanvasPresentation, RenderObjectKind},
    Canvas, Rect, Size, Theme,
};
struct Fonts;
impl TextMeasurer for Fonts {
    fn measure_width(&self, s: &str) -> u32 {
        s.len() as u32 * 8
    }
    fn line_height(&self) -> u32 {
        16
    }
}
fn state(html: &str, css: &str) -> DocumentRenderState {
    let dom = parse_html(html).unwrap();
    let styles = StyleContext::build(
        &dom,
        &[parse_stylesheet(
            css,
            StylesheetSource::External("https://cdn.example/assets/css/nav.css".into()),
        )],
    );
    DocumentRenderState::new(
        42,
        "https://document.example/other/page".into(),
        dom,
        styles,
        Size::new(320, 240),
        &Fonts,
    )
}
fn cache() -> ImageCache {
    let mut cache = ImageCache::default();
    cache.insert_decoded("https://cdn.example/assets/img/tile.svg".into(), decode_image(br##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="5"><rect width="10" height="5" fill="#ff0000"/></svg>"##, Some("image/svg+xml"), "tile.svg").unwrap());
    cache
}
fn image_bounds(state: &DocumentRenderState) -> Vec<Rect> {
    state
        .current_scene
        .objects
        .iter()
        .filter_map(|o| matches!(o.kind, RenderObjectKind::Image { .. }).then_some(o.bounds))
        .collect()
}
fn pixels(state: &DocumentRenderState) -> Vec<u32> {
    let mut pixels = vec![0; 320 * 240];
    DocumentCanvas::new(Rect::new(0, 0, 320, 240), &[])
        .with_scene(&state.current_scene)
        .with_presentation(DocumentCanvasPresentation::Browser)
        .draw(
            &mut Canvas::new(&mut pixels, 320, 320, 240),
            &Theme::sunlight_dark(),
        );
    pixels
}
#[test]
fn external_custom_property_url_survives_inheritance_and_late_completion() {
    let mut s=state("<div id=parent><div id=child></div></div>", "body{margin:0} #parent{--logo:url(../img/tile.svg)} #child{background:blue var(--logo) no-repeat scroll 0 0; background-size:100%;width:100px;height:50px}");
    let child = s.dom.find_first_element("div").unwrap() + 1;
    assert_eq!(
        s.styles
            .style_for(child)
            .unwrap()
            .value(&Property::BackgroundImage)
            .unwrap()
            .display(),
        "url(\"https://cdn.example/assets/img/tile.svg\")"
    );
    assert!(image_bounds(&s).is_empty());
    assert!(!s.complete_images(41, &Fonts, cache()));
    assert!(s.complete_images(42, &Fonts, cache()));
    assert_eq!(image_bounds(&s), vec![Rect::new(0, 0, 100, 50)]);
    assert!(!s.complete_images(42, &Fonts, cache()));
    assert_eq!(pixels(&s)[20 * 320 + 20], 0xff0000);
}
#[test]
fn position_size_repeat_and_border_clip_geometry() {
    for (repeat, expected) in [
        ("no-repeat", 1),
        ("repeat-x", 5),
        ("repeat-y", 6),
        ("repeat", 30),
    ] {
        let mut s=state("<div></div>", &format!("body{{margin:0}}div{{width:30px;height:15px;border:2px solid blue;padding:3px;background-image:url(../img/tile.svg);background-repeat:{repeat};background-size:10px 5px;background-position:50% 100%}}"));
        s.complete_images(42, &Fonts, cache());
        let images: Vec<_> = s
            .current_scene
            .objects
            .iter()
            .filter(|o| matches!(o.kind, RenderObjectKind::Image { .. }))
            .collect();
        assert_eq!(images.len(), expected, "{repeat}: {:?}", image_bounds(&s));
        assert!(images
            .iter()
            .all(|o| o.clip_bounds == Some(Rect::new(0, 0, 40, 25))));
        if repeat == "no-repeat" {
            assert_eq!(images[0].bounds, Rect::new(15, 18, 10, 5));
        }
    }
}
#[test]
fn color_then_image_then_descendant_background_and_border_paint() {
    let mut s=state("<div id=parent><div id=child></div></div>","body{margin:0}#parent{width:40px;height:30px;background:blue url(../img/tile.svg) no-repeat 0 0;background-size:40px 30px;border:2px solid green}#child{width:10px;height:10px;background:yellow;margin:5px}");
    s.complete_images(42, &Fonts, cache());
    let p = pixels(&s);
    assert_eq!(p[0], 0xff008000); // border after image
    assert_eq!(p[3 * 320 + 3], 0xff0000); // image over blue
    assert_eq!(p[9 * 320 + 9], 0xffffff00); // descendant over parent's image
}
#[test]
fn real_arch_navbar_uses_190_by_40_logo_from_stylesheet_directory() {
    let mut s = state(
        "<div id=archnavbar><h1 id=logo><a>Arch Linux</a></h1></div>",
        include_str!("fixtures/arch-navbar-live.css"),
    );
    let mut cache = ImageCache::default();
    cache.insert_decoded(
        "https://cdn.example/assets/css/archlogo.svg".into(),
        decode_image(
            include_bytes!("fixtures/archlogo.svg"),
            Some("image/svg+xml"),
            "archlogo.svg",
        )
        .unwrap(),
    );
    s.complete_images(42, &Fonts, cache);
    let bounds = image_bounds(&s);
    assert_eq!(bounds.len(), 1);
    assert_eq!((bounds[0].w, bounds[0].h), (190, 40));
    assert!(!s.diagnostics.unsupported.contains(&"background-image"));
}
#[test]
fn image_failure_resize_and_tile_budget_are_bounded() {
    let mut s=state("<div></div>","body{margin:0}div{width:100%;height:200px;background-image:url(../img/tile.svg);background-size:1px 1px}");
    assert!(image_bounds(&s).is_empty());
    s.complete_images(42, &Fonts, cache());
    assert_eq!(image_bounds(&s).len(), 4096);
    s.rebuild_for_viewport(Size::new(80, 240), &Fonts);
    assert_eq!(image_bounds(&s).len(), 4096);
    assert!(s
        .current_scene
        .objects
        .iter()
        .filter(|o| matches!(o.kind, RenderObjectKind::Image { .. }))
        .all(|o| o.clip_bounds.unwrap().w == 80));
}

#[test]
fn single_layer_url_policy_and_sizing_variants() {
    use rappid_rabbit::css::css_image_url;
    assert_eq!(css_image_url("URL('a b.svg')"), Some("a b.svg"));
    for raw in [
        "url(a.svg),url(b.svg)",
        "url(a b.svg)",
        "url(a\\b.svg)",
        "url('unterminated.svg)",
    ] {
        assert_eq!(css_image_url(raw), None, "{raw}");
    }
    for (size, expected) in [
        ("auto", (10, 5)),
        ("contain", (40, 20)),
        ("cover", (60, 30)),
        ("auto 20px", (40, 20)),
    ] {
        let mut s=state("<div></div>",&format!("body{{margin:0}}div{{width:40px;height:30px;background:url(../img/tile.svg) no-repeat;background-size:{size};background-position:right bottom}}"));
        s.complete_images(42, &Fonts, cache());
        let bounds = image_bounds(&s)[0];
        assert_eq!((bounds.w, bounds.h), expected, "{size}");
        assert_eq!((bounds.right(), bounds.bottom()), (40, 30));
    }
}
