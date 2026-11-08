#![cfg(feature = "dom")]
use golden_fish::parse_html;
use rappid_rabbit::{
    css::{collect_embedded_stylesheets, StyleContext},
    images::{decode_image, ImageCache},
    presentation::{ImagePresentation, IMAGE_PAINT_BATCH},
    render::{DocumentRenderState, TextMeasurer},
};
use std::{cell::Cell, sync::Arc};
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
fn state(html: &str) -> DocumentRenderState {
    let dom = parse_html(html).unwrap();
    let styles = StyleContext::build(&dom, &collect_embedded_stylesheets(&dom));
    DocumentRenderState::new(
        1,
        "http://fixture/".into(),
        dom,
        styles,
        Size::new(640, 480),
        &Fonts,
    )
}
fn cache(urls: &[&str]) -> ImageCache {
    let mut cache = ImageCache::default();
    for url in urls {
        cache.insert_decoded(
            format!("http://fixture/{url}"),
            decode_image(
                include_bytes!("../../sun-img/tests/fixtures/gray.jpg"),
                None,
                url,
            )
            .unwrap(),
        );
    }
    cache
}
fn paint(
    state: &DocumentRenderState,
    policy: &ImagePresentation,
    scroll: u32,
) -> (Vec<u32>, usize) {
    let mut pixels = vec![0; 640 * 480];
    let writes = Cell::new(0);
    DocumentCanvas::new(Rect::new(0, 0, 640, 480), &[])
        .with_scene(&state.current_scene)
        .with_presentation(DocumentCanvasPresentation::Browser)
        .with_scroll_y(scroll)
        .with_image_presentation(policy.admitted())
        .with_image_paint_counter(&writes)
        .draw(
            &mut Canvas::new(&mut pixels, 640, 640, 480),
            &Theme::sunlight_light(),
        );
    (pixels, writes.get())
}
#[test]
fn document_first_then_one_batched_pixel_patch_without_layout_or_geometry_change() {
    let mut state = state(include_str!("fixtures/progressive/archwiki.html"));
    let mut policy = ImagePresentation::default();
    let (initial, writes) = paint(&state, &policy, 0);
    assert_eq!(writes, 0);
    assert!(state.diagnostics.control_count >= 2);
    assert!(state.diagnostics.link_count > 0);
    let geometry = state
        .current_scene
        .objects
        .iter()
        .map(|o| (o.id, o.bounds))
        .collect::<Vec<_>>();
    let images = cache(&["logo.jpg", "screen.jpg", "footer.jpg"]);
    let pixels = images
        .decoded("http://fixture/logo.jpg")
        .unwrap()
        .image
        .clone();
    assert!(state.complete_images(1, &Fonts, images.clone()));
    assert_eq!(state.layout_runs, 1);
    assert_eq!(state.scene_generation, 2);
    assert!(!state.complete_images(1, &Fonts, images));
    assert_eq!(
        geometry,
        state
            .current_scene
            .objects
            .iter()
            .map(|o| (o.id, o.bounds))
            .collect::<Vec<_>>()
    );
    assert!(!state.last_patch.dirty_regions.is_empty());
    assert_eq!(paint(&state, &policy, 0).1, 0);
    assert!(policy.advance(
        &state.current_scene,
        state.scene_generation,
        Rect::new(0, 0, 640, 480)
    ));
    let (after, writes) = paint(&state, &policy, 0);
    assert!(writes > 0);
    assert_ne!(initial, after);
    assert!(state.current_scene.objects.iter().any(
        |o| matches!(&o.kind,RenderObjectKind::Image { image,.. } if Arc::ptr_eq(image,&pixels))
    ));
    let footer=state.current_scene.objects.iter().find(|o| matches!(&o.kind,RenderObjectKind::Image {source_url,..} if source_url.ends_with("footer.jpg"))).unwrap();
    assert!(!policy.admitted().contains(&footer.id));
    let bottom = state.current_scene.max_scroll_y(480);
    assert!(policy.advance(
        &state.current_scene,
        state.scene_generation,
        Rect::new(0, bottom as i32, 640, 480)
    ));
    assert!(policy.admitted().contains(&footer.id));
    assert!(paint(&state, &policy, bottom).1 > 0);
    policy.advance(
        &state.current_scene,
        state.scene_generation,
        Rect::new(0, 0, 640, 480),
    );
    assert!(paint(&state, &policy, 0).1 > 0);
    for _ in 0..100 {
        assert!(!policy.advance(
            &state.current_scene,
            state.scene_generation,
            Rect::new(0, 0, 640, 480)
        ));
    }
}
#[test]
fn visible_objects_cannot_starve_behind_nearby_objects_and_batches_are_bounded() {
    let html = format!(
        "<style>body{{margin:0}}img{{display:block}}</style>{}",
        "<img src='a.jpg' width=16 height=80>".repeat(30)
    );
    let mut state = state(&html);
    state.complete_images(1, &Fonts, cache(&["a.jpg"]));
    let mut policy = ImagePresentation::default();
    let viewport = Rect::new(0, 800, 640, 480);
    let visible = state
        .current_scene
        .objects
        .iter()
        .filter(|o| {
            matches!(o.kind, RenderObjectKind::Image { .. })
                && o.bounds.intersect(viewport).is_some()
        })
        .map(|o| o.id)
        .collect::<Vec<_>>();
    assert!(visible.len() > IMAGE_PAINT_BATCH);
    policy.advance(&state.current_scene, state.scene_generation, viewport);
    assert_eq!(policy.admitted().len(), IMAGE_PAINT_BATCH);
    assert!(policy.admitted().iter().all(|id| visible.contains(id)));
    for _ in 0..10 {
        policy.advance(&state.current_scene, state.scene_generation, viewport);
    }
    assert!(policy.visible_complete(&state.current_scene, viewport));
    assert!(!policy.advance(&state.current_scene, state.scene_generation, viewport));
    // Resize and a scaled scene (the policy's zoom boundary) use the same
    // document-space intersection logic, with no decode or DOM work.
    let mut scene = (*state.current_scene).clone();
    for object in &mut scene.objects {
        object.bounds.y *= 2;
        object.bounds.h *= 2;
    }
    scene.finalize();
    policy.advance(
        &scene,
        state.scene_generation + 1,
        Rect::new(0, 1600, 320, 600),
    );
    for _ in 0..10 {
        policy.advance(
            &scene,
            state.scene_generation + 1,
            Rect::new(0, 1600, 320, 600),
        );
    }
    assert!(policy.visible_complete(&scene, Rect::new(0, 1600, 320, 600)));
}
#[test]
fn intrinsic_changes_reflow_once_but_presentation_does_not_reflow_or_distort() {
    let mut state = state("<img src='a.jpg' width=32><p>after</p>");
    state.complete_images(1, &Fonts, cache(&["a.jpg"]));
    assert_eq!(state.layout_runs, 2);
    let image = state
        .current_scene
        .objects
        .iter()
        .find(|o| matches!(o.kind, RenderObjectKind::Image { .. }))
        .unwrap();
    assert_eq!((image.bounds.w, image.bounds.h), (32, 32));
    let mut policy = ImagePresentation::default();
    policy.advance(
        &state.current_scene,
        state.scene_generation,
        Rect::new(0, 0, 640, 480),
    );
    paint(&state, &policy, 0);
    assert_eq!(state.layout_runs, 2);
}
#[test]
fn late_backgrounds_use_cached_geometry_and_match_full_canonical_reflow_pixels() {
    let mut state=state("<style>body{margin:0}div{width:80px;height:40px;background-image:url(a.jpg);background-repeat:repeat;overflow:hidden}</style><div><p>hello</p></div>");
    state.complete_images(1, &Fonts, cache(&["a.jpg"]));
    assert_eq!(state.layout_runs, 1);
    let mut policy = ImagePresentation::default();
    for _ in 0..100 {
        if !policy.advance(
            &state.current_scene,
            state.scene_generation,
            Rect::new(0, 0, 640, 480),
        ) {
            break;
        }
    }
    let patched = paint(&state, &policy, 0).0;
    state.rebuild(state.viewport, &Fonts);
    for _ in 0..100 {
        if !policy.advance(
            &state.current_scene,
            state.scene_generation,
            Rect::new(0, 0, 640, 480),
        ) {
            break;
        }
    }
    assert_eq!(patched, paint(&state, &policy, 0).0);
}
#[test]
fn failures_and_repeated_close_release_pixels_and_never_queue_a_paint_loop() {
    assert!(decode_image(b"broken", None, "bad.jpg").is_err());
    for generation in 1..33 {
        let mut state =
            state("<img src='a.jpg' width=80 height=40><img src='bad.jpg' width=32 height=32>");
        let mut cache = cache(&["a.jpg"]);
        let weak = Arc::downgrade(&cache.decoded("http://fixture/a.jpg").unwrap().image);
        cache.insert_failed("http://fixture/bad.jpg".into(), "decode failed");
        state.complete_images(1, &Fonts, cache.clone());
        assert!(!state.complete_images(generation + 1, &Fonts, cache.clone()));
        assert!(!state.complete_images(1, &Fonts, cache));
        let mut policy = ImagePresentation::default();
        policy.advance(
            &state.current_scene,
            state.scene_generation,
            Rect::new(0, 0, 640, 480),
        );
        assert!(!policy.advance(
            &state.current_scene,
            state.scene_generation,
            Rect::new(0, 0, 640, 480)
        ));
        drop(state);
        assert!(weak.upgrade().is_none()); // policy owns no raster or document
    }
}
#[cfg(feature = "js-dom-fixture")]
#[test]
fn sunjs_image_mutations_and_listeners_remain_independent_of_resource_and_paint_state() {
    use rappid_rabbit::dom_fixture::FixtureSession;
    let mut state=state("<p id='result'>start</p><img id='img' src='a.jpg' width=16 height=16><button id='go'>go</button><img id='added' hidden src='a.jpg' width=20 height=10>");
    let mut js = FixtureSession::new(&state).unwrap();
    js.evaluate(&mut state,"let order='A'; let events=0; document.getElementById('img').addEventListener('load',function(){events=events+1;}); document.getElementById('go').addEventListener('click',function(){order=order+'C';document.getElementById('result').textContent=order;}); order=order+'B';",&Fonts).unwrap();
    state.complete_images(1, &Fonts, cache(&["a.jpg", "b.jpg"]));
    let mut policy = ImagePresentation::default();
    policy.advance(
        &state.current_scene,
        state.scene_generation,
        Rect::new(0, 0, 640, 480),
    );
    paint(&state, &policy, 0);
    js.evaluate(&mut state,"if(events!==0){throw 'paint fired load';} document.getElementById('img').setAttribute('src','b.jpg'); document.getElementById('added').removeAttribute('hidden');",&Fonts).unwrap();
    let button = state.dom.get_element_by_id("go").unwrap();
    js.click(&mut state, button, &Fonts).unwrap();
    let result = state.dom.get_element_by_id("result").unwrap();
    assert_eq!(
        state.dom.element_text_content(result).as_deref(),
        Some("ABC")
    );
    assert!(state.current_scene.objects.iter().any(|o|matches!(&o.kind,RenderObjectKind::Image {source_url,..} if source_url.ends_with("b.jpg"))));
    assert_eq!(state.diagnostics.image_count, 2);
    state.document_id = 2;
    assert!(js
        .evaluate(&mut state, "throw new Error('stale callback ran')", &Fonts)
        .is_err());
}

#[test]
fn fixed_geometry_pixel_patch_matches_canonical_layout_in_nested_contexts() {
    for wrapper in [
        ("<div style='display:inline-block'>", "</div>"),
        ("<div style='display:flex;flex-direction:column'>", "</div>"),
        ("<table><tr><td>", "</td></tr></table>"),
        ("<div style='float:left'>", "</div>"),
    ] {
        let mut state = state(&format!(
            "<style>body{{margin:0}}</style>{}<img src='a.jpg' width=80 height=40>{}<p>after</p>",
            wrapper.0, wrapper.1
        ));
        state.complete_images(1, &Fonts, cache(&["a.jpg"]));
        assert_eq!(state.layout_runs, 1);
        let patched = state.current_scene.clone();
        state.rebuild(state.viewport, &Fonts);
        assert_eq!(patched, state.current_scene);
    }
}

#[test]
fn pixel_budget_eviction_retains_known_dimensions_without_refetch_or_reflow() {
    use rappid_rabbit::images::{DecodedImage, ImageFormat};
    use sunlight_ui::widgets::RasterImage;
    let mut state = state("<img src='a0.jpg'><p>after</p>");
    let mut images = ImageCache::default();
    let image = || DecodedImage {
        image: Arc::new(RasterImage {
            width: 1024,
            height: 1024,
            pixels: vec![0xff445566; 1024 * 1024],
        }),
        format: ImageFormat::Jpeg,
        byte_size: 100,
    };
    images.insert_decoded("http://fixture/a0.jpg".into(), image());
    state.complete_images(1, &Fonts, images.clone());
    let geometry = state
        .current_scene
        .objects
        .iter()
        .map(|o| (o.id, o.bounds))
        .collect::<Vec<_>>();
    for n in 1..8 {
        images.insert_decoded(format!("http://fixture/a{n}.jpg"), image());
    }
    assert!(images.decoded("http://fixture/a0.jpg").is_none());
    assert_eq!(
        images.intrinsic_dimensions("http://fixture/a0.jpg"),
        Some((1024, 1024))
    );
    let runs = state.layout_runs;
    state.complete_images(1, &Fonts, images);
    assert_eq!(state.layout_runs, runs);
    assert_eq!(
        geometry,
        state
            .current_scene
            .objects
            .iter()
            .map(|o| (o.id, o.bounds))
            .collect::<Vec<_>>()
    );
}
