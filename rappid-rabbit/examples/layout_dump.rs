//! Dump the actual modern layout arena and computed declarations for a captured page.
//! cargo run -p rappid-rabbit --example layout_dump --target x86_64-unknown-linux-gnu -- page.html WIDTH HEIGHT [sheet.css ...]
use rappid_rabbit::{
    css::{collect_embedded_stylesheets, parse_stylesheet, StyleContext, StylesheetSource},
    layout::{layout_document, FontMeasurer, NoImageMetrics},
};
use sunlight_ui::Size;
struct Fonts;
impl FontMeasurer for Fonts {
    fn measure_width(&self, _: &str, size: f32, text: &str) -> f32 {
        text.chars().count() as f32 * size * 0.5
    }
}
fn main() {
    let a: Vec<_> = std::env::args().collect();
    let html = std::fs::read_to_string(&a[1]).expect("HTML");
    let dom = golden_fish::parse_html(&html).expect("DOM");
    let mut sheets = collect_embedded_stylesheets(&dom);
    for file in &a[4..] {
        sheets.push(parse_stylesheet(
            &std::fs::read_to_string(file).expect("CSS"),
            StylesheetSource::External(file.clone()),
        ));
    }
    let width = a[2].parse().unwrap();
    let height = a[3].parse().unwrap();
    let styles = StyleContext::build_with_viewport(&dom, &sheets, width);
    let result = layout_document(
        &dom,
        &styles,
        Size::new(width, height),
        &Fonts,
        &NoImageMetrics,
    );
    for b in &result.geometry.boxes {
        println!(
            "node={} parent={:?} dom={:?} border={:?} content={:?} margin={:?} intrinsic={:?}",
            b.dom_node,
            b.parent,
            dom.get(b.dom_node),
            b.border_box,
            b.content_box,
            b.used_margin,
            b.intrinsic
        );
        if let Some(style) = styles.nearest_element_style(&dom, b.dom_node) {
            for p in &style.properties {
                println!(
                    "  {}={} source={:?}",
                    p.property.name(),
                    p.value.display(),
                    p.matched
                );
            }
        }
    }
}
