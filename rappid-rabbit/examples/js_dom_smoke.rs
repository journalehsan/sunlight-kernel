//! Local visual harness. Only the compiled-in fixture is executed.

use std::{fs, io::Write, path::Path};

use golden_fish::parse_html;
use rappid_rabbit::{
    css::{collect_embedded_stylesheets, StyleContext},
    dom_fixture::run_fixture_script,
    render::{DocumentRenderState, TextMeasurer},
};
use sunlight_ui::{
    widgets::{DocumentCanvas, DocumentCanvasPresentation},
    Canvas, Rect, Size, Theme,
};

struct Measure;
impl TextMeasurer for Measure {
    fn measure_width(&self, text: &str) -> u32 {
        Canvas::measure_text(text)
    }
    fn line_height(&self) -> u32 {
        16
    }
}

fn save_viewport(state: &DocumentRenderState, path: &Path) -> std::io::Result<()> {
    let mut pixels = vec![0; 320 * 240];
    DocumentCanvas::new(Rect::new(0, 0, 320, 240), &[])
        .with_presentation(DocumentCanvasPresentation::Browser)
        .with_scene(&state.current_scene)
        .draw(
            &mut Canvas::new(&mut pixels, 320, 320, 240),
            &Theme::sunlight_light(),
        );
    let mut file = std::io::BufWriter::new(fs::File::create(path)?);
    file.write_all(b"P6\n320 240\n255\n")?;
    for pixel in pixels {
        file.write_all(&[(pixel >> 16) as u8, (pixel >> 8) as u8, pixel as u8])?;
    }
    file.flush()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dom = parse_html(include_str!("../tests/fixtures/js_dom_smoke.html"))?;
    let script = dom.get_element_by_id("fixture-script").unwrap();
    let source = dom.element_text_content(script).unwrap();
    let styles = StyleContext::build_with_viewport(&dom, &collect_embedded_stylesheets(&dom), 320);
    let mut state = DocumentRenderState::new(
        1,
        "fixture:js_dom_smoke".into(),
        dom,
        styles,
        Size::new(320, 240),
        &Measure,
    );
    let output = Path::new("target/js-dom-smoke");
    fs::create_dir_all(output)?;
    save_viewport(&state, &output.join("before.ppm"))?;
    let result = run_fixture_script(&mut state, &source, "js_dom_smoke.html:inline", &Measure);
    if result.repaint_requested {
        save_viewport(&state, &output.join("after.ppm"))?;
    }
    result.evaluation?;
    assert!(result.repaint_requested);
    println!("SunJS DOM smoke PASS: New Text; scene={} dom_revision={}; target/js-dom-smoke/{{before,after}}.ppm", state.scene_generation, state.dom_revision);
    Ok(())
}
