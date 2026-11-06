#![cfg(feature = "dom")]

use rappid_rabbit::{
    css::{collect_embedded_stylesheets, StyleContext},
    render::{DocumentRenderState, TextMeasurer},
};
use sunlight_ui::{widgets::RenderObjectKind, Size};

struct Fonts;
impl TextMeasurer for Fonts {
    fn measure_width(&self, text: &str) -> u32 {
        text.chars().count() as u32 * 8
    }
    fn line_height(&self) -> u32 {
        16
    }
}

#[test]
fn head_scripts_do_not_hide_the_search_page() {
    let dom = golden_fish::parse_html(include_str!(
        "../../golden-fish/tests/fixtures/script-head.html"
    ))
    .unwrap();
    let styles = StyleContext::build(&dom, &collect_embedded_stylesheets(&dom));
    let state = DocumentRenderState::new(
        1,
        "https://example.com/".into(),
        dom,
        styles,
        Size::new(1600, 650),
        &Fonts,
    );
    let visible_text = state
        .current_scene
        .objects
        .iter()
        .filter_map(|object| match &object.kind {
            RenderObjectKind::Text { text, .. } if object.bounds.y < 650 => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ");
    assert!(visible_text.contains("Search the web"), "{visible_text}");
    assert_eq!(state.diagnostics.control_count, 2);
}
