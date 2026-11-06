#![cfg(feature = "dom")]

use golden_fish::parse_html;
use rappid_rabbit::{
    css::StyleContext,
    images::{decode_image, ImageCache},
    render::{DocumentRenderState, TextMeasurer},
};
use std::sync::Arc;
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
    let styles = StyleContext::build(&dom, &[]);
    DocumentRenderState::new(
        11,
        "http://example.com/".into(),
        dom,
        styles,
        Size::new(640, 480),
        &Fonts,
    )
}

#[test]
fn late_jpeg_completion_paints_pixels_and_obeys_generation_and_geometry() {
    for (sizing, expected) in [
        ("", (8, 8)),
        ("width=32", (32, 32)),
        ("width=32 height=16", (32, 16)),
        ("style='width:40px;height:auto'", (40, 40)),
    ] {
        let mut state = state(&format!("<style>body{{margin:0}}img{{display:block}}</style><img src='photo.jpg' {sizing}><p>after</p>"));
        assert_eq!(state.diagnostics.placeholder_count, 1);
        let decoded = decode_image(
            include_bytes!("../../sun-img/tests/fixtures/gray.jpg"),
            None,
            "photo.jpg",
        )
        .unwrap();
        let pixels = decoded.image.clone();
        let mut images = ImageCache::default();
        images.insert_decoded("http://example.com/photo.jpg".into(), decoded);
        assert!(!state.complete_images(10, &Fonts, images.clone()));
        assert_eq!(state.diagnostics.placeholder_count, 1);
        assert!(state.complete_images(11, &Fonts, images.clone()));
        assert!(!state.complete_images(11, &Fonts, images));
        assert_eq!(state.scene_generation, 2);
        assert_eq!(state.diagnostics.placeholder_count, 0);
        let image = state
            .current_scene
            .objects
            .iter()
            .find(|o| matches!(o.kind, RenderObjectKind::Image { .. }))
            .unwrap();
        assert_eq!(image.bounds.w, expected.0);
        // Outward rounding at fractional CSS positions may cover one extra row.
        assert!((expected.1..=expected.1 + 1).contains(&image.bounds.h));
        if let RenderObjectKind::Image { image, .. } = &image.kind {
            assert!(Arc::ptr_eq(image, &pixels));
        }
        let mut framebuffer = vec![0; 640 * 480];
        DocumentCanvas::new(Rect::new(0, 0, 640, 480), &[])
            .with_scene(&state.current_scene)
            .with_presentation(DocumentCanvasPresentation::Browser)
            .draw(
                &mut Canvas::new(&mut framebuffer, 640, 640, 480),
                &Theme::sunlight_dark(),
            );
        assert_eq!(
            framebuffer[image.bounds.y as usize * 640 + image.bounds.x as usize],
            0x007b7b7b
        );
    }
}

#[test]
fn portrait_sized_progressive_jpeg_fits_both_decode_and_cache_budgets() {
    let decoded = decode_image(
        include_bytes!("fixtures/portrait-size.jpg"),
        None,
        "portrait.jpg",
    )
    .unwrap();
    assert_eq!((decoded.image.width, decoded.image.height), (2000, 2588));
    assert!(decoded.image.pixels.iter().all(|p| p >> 24 == 255));
    let mut cache = ImageCache::default();
    cache.insert_decoded("portrait.jpg".into(), decoded);
    assert!(cache.decoded("portrait.jpg").is_some());
}

#[cfg(feature = "svg")]
#[test]
fn svg_intrinsic_size_transparency_and_css_reach_document_paint() {
    let bytes = include_bytes!("fixtures/viewbox.svg");
    let decoded = decode_image(bytes, Some("image/svg+xml"), "logo.svg").unwrap();
    assert_eq!((decoded.image.width, decoded.image.height), (80, 40));
    assert_eq!(decoded.image.pixels[0], 0);
    assert_eq!(decoded.image.pixels[20 * 80 + 40], 0x80ff0000);
    for (attributes, expected) in [
        ("", (80, 40)),
        ("width=160", (160, 80)),
        ("style='width:120px;height:auto'", (120, 60)),
    ] {
        let mut state = state(&format!(
            "<style>body{{margin:0}}</style><img src='logo.svg' {attributes}>"
        ));
        let mut cache = ImageCache::default();
        cache.insert_decoded("http://example.com/logo.svg".into(), decoded.clone());
        state.complete_images(11, &Fonts, cache);
        let image = state
            .current_scene
            .objects
            .iter()
            .find(|o| matches!(o.kind, RenderObjectKind::Image { .. }))
            .unwrap();
        assert_eq!((image.bounds.w, image.bounds.h), expected);
        let mut framebuffer = vec![0; 640 * 480];
        DocumentCanvas::new(Rect::new(0, 0, 640, 480), &[])
            .with_scene(&state.current_scene)
            .with_presentation(DocumentCanvasPresentation::Browser)
            .draw(
                &mut Canvas::new(&mut framebuffer, 640, 640, 480),
                &Theme::sunlight_dark(),
            );
        let center = (image.bounds.y as usize + expected.1 as usize / 2) * 640
            + image.bounds.x as usize
            + expected.0 as usize / 2;
        assert_ne!(
            framebuffer[center],
            framebuffer[image.bounds.y as usize * 640 + image.bounds.x as usize]
        );
    }
    assert!(decode_image(include_bytes!("fixtures/logo.svg"), None, "logo.svg").is_ok());
    for bytes in [
        include_bytes!("fixtures/malformed.svg").as_slice(),
        include_bytes!("fixtures/oversized.svg").as_slice(),
        br#"<!DOCTYPE svg [<!ENTITY a "bad">]><svg>&a;</svg>"#,
        br#"<svg xmlns="http://www.w3.org/2000/svg"><image href="/etc/passwd"/></svg>"#,
    ] {
        assert!(decode_image(bytes, None, "bad.svg").is_err());
    }
}
