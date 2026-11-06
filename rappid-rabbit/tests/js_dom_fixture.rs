#![cfg(feature = "js-dom-fixture")]

use std::sync::Arc;

use golden_fish::parse_html;
use rappid_rabbit::{
    css::{collect_embedded_stylesheets, StyleContext},
    dom_fixture::run_fixture_script,
    render::{DocumentRenderState, TextMeasurer},
};
use sunlight_js::ErrorKind;
use sunlight_ui::{
    widgets::{DocumentCanvas, DocumentCanvasPresentation, RenderObjectKind},
    Canvas, Rect, Size, Theme,
};

const HTML: &str = include_str!("fixtures/js_dom_smoke.html");
struct Measure;
impl TextMeasurer for Measure {
    fn measure_width(&self, text: &str) -> u32 {
        Canvas::measure_text(text)
    }
    fn line_height(&self) -> u32 {
        16
    }
}

fn render(html: &str) -> DocumentRenderState {
    let dom = parse_html(html).unwrap();
    let styles = StyleContext::build_with_viewport(&dom, &collect_embedded_stylesheets(&dom), 320);
    DocumentRenderState::new(
        7,
        "fixture:js_dom_smoke".into(),
        dom,
        styles,
        Size::new(320, 240),
        &Measure,
    )
}

fn text(state: &DocumentRenderState) -> String {
    state
        .current_scene
        .objects
        .iter()
        .filter_map(|object| match &object.kind {
            RenderObjectKind::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn paint(state: &DocumentRenderState) -> Vec<u32> {
    let mut pixels = vec![0; 320 * 240];
    let mut canvas = Canvas::new(&mut pixels, 320, 320, 240);
    DocumentCanvas::new(Rect::new(0, 0, 320, 240), &[])
        .with_presentation(DocumentCanvasPresentation::Browser)
        .with_scene(&state.current_scene)
        .draw(&mut canvas, &Theme::sunlight_light());
    pixels
}

#[test]
fn local_fixture_updates_dom_arena_scene_patch_and_viewport_pixels() {
    let mut state = render(HTML);
    let counter = state.dom.get_element_by_id("counter").unwrap();
    let script = state.dom.get_element_by_id("fixture-script").unwrap();
    let source = state.dom.element_text_content(script).unwrap();
    let before = paint(&state);
    let snapshot = state.reflow_snapshot();
    assert!(text(&state).contains("Old"));
    assert!(!text(&state).contains("incorrect")); // scripts never paint
    let result = run_fixture_script(&mut state, &source, "js_dom_smoke.html:inline", &Measure);
    result.evaluation.unwrap();
    assert!(result.repaint_requested);
    assert_eq!(
        state.dom.element_text_content(counter).as_deref(),
        Some("New Text")
    );
    assert!(text(&state).contains("New Text"));
    assert!(!text(&state).contains("Old"));
    assert_eq!(state.scene_generation, 2);
    assert_eq!(state.dom_revision, 1);
    assert_eq!(snapshot.dom_revision, 0);
    assert_eq!(
        snapshot.dom.element_text_content(counter).as_deref(),
        Some("Old")
    );
    assert!(state
        .layout_geometry
        .iter()
        .any(|g| g.node_id == counter && g.content_box.height > 0.0));
    assert!(!state.last_patch.is_empty());
    assert!(!state.last_patch.dirty_regions.is_empty());
    assert_ne!(before, paint(&state));
}

#[test]
fn parsing_and_rendering_never_execute_inline_or_external_scripts() {
    let state = render("<div id='counter'>Old</div><script>document.getElementById('counter').textContent='BAD';</script><script src='https://example.test/untrusted.js'></script>");
    assert!(text(&state).contains("Old"));
    assert!(!text(&state).contains("BAD"));
    assert_eq!(state.scene_generation, 1);
    assert_eq!(state.dom_revision, 0);
}

#[test]
fn writes_coalesce_and_flush_on_exception_and_fuel_exhaustion() {
    for tail in ["throw 'stop';", "while(true) {}"] {
        let mut state = render("<div id='a'>Old</div>");
        let source = format!("const a=document.getElementById('a');a.textContent='First';a.textContent='Final';{tail}");
        let result = run_fixture_script(&mut state, &source, "exception.js", &Measure);
        assert!(result.evaluation.is_err());
        assert!(result.repaint_requested);
        assert_eq!(state.scene_generation, 2);
        assert_eq!(state.dom_revision, 1);
        assert_eq!(text(&state), "Final");
    }
}

#[test]
fn empty_text_detachment_and_identity_are_safe() {
    let mut state = render("<div id='a'>Old<!--skip--><b id='b'>nested</b></div>");
    let result = run_fixture_script(
        &mut state,
        r#"
        const a=document.getElementById('a');
        const b=document.getElementById('b');
        if(a.textContent!=='Oldnested') { throw 'descendant text'; }
        a.textContent='';
        if(a.textContent!=='' || document.getElementById('b')!==null) { throw 'detach'; }
        if(b.textContent!=='nested') { throw 'stable detached node'; }
        b.textContent='detached';
        a.textContent='<b>plain</b>';
        if(a!==document.getElementById('a')) { throw 'identity'; }
    "#,
        "detached.js",
        &Measure,
    );
    result.evaluation.unwrap();
    assert!(result.repaint_requested);
    assert_eq!(text(&state), "<b>plain</b>");
}

#[test]
fn invalid_receivers_and_non_unicode_or_non_string_arguments_do_not_mutate() {
    for source in [
        "document.getElementById.call({}, 'a');",
        "Object.getOwnPropertyDescriptor(Object.getPrototypeOf(document.getElementById('a')), 'textContent').set.call({}, 'BAD');",
        "Object.create(document.getElementById('a')).textContent='BAD';",
        "Object.create(document.getElementById('a')).textContent;",
        "document.getElementById('a').textContent={toString(){throw 'must not run';}};",
        "document.getElementById('a').textContent='\\uD800';",
        "document.getElementById(1);",
    ] {
        let mut state = render("<div id='a'>Old</div>");
        let result = run_fixture_script(&mut state, source, "invalid.js", &Measure);
        assert_eq!(result.evaluation.unwrap_err().kind, ErrorKind::Type, "{source}");
        assert!(!result.repaint_requested);
        assert_eq!(state.scene_generation, 1);
        assert_eq!(text(&state), "Old");
    }
}

#[test]
fn reads_and_unchanged_leaf_writes_keep_the_scene_and_dom_snapshot() {
    let mut state = render("<div id='a'>Old</div>");
    let scene = state.current_scene.clone();
    let dom = state.dom.clone();
    let result = run_fixture_script(&mut state, "if(document.getElementById('missing')!==null){throw 'missing';}document.getElementById('a').textContent='Old';", "read.js", &Measure);
    result.evaluation.unwrap();
    assert!(!result.repaint_requested);
    assert!(Arc::ptr_eq(&scene, &state.current_scene));
    assert!(Arc::ptr_eq(&dom, &state.dom));
    assert_eq!(state.dom_revision, 0);
}

#[test]
fn fixture_host_limits_mutation_work_and_contexts_are_fresh_each_turn() {
    let mut state = render("<div id='a'>Old</div>");
    let result = run_fixture_script(
        &mut state,
        "const a=document.getElementById('a');for(let i=0;i<300;i=i+1){a.textContent='New';}",
        "bounded.js",
        &Measure,
    );
    assert_eq!(result.evaluation.unwrap_err().kind, ErrorKind::Limit);
    assert!(result.repaint_requested);
    let result = run_fixture_script(
        &mut state,
        "const a=document.getElementById('a');a.textContent='Fresh';",
        "fresh.js",
        &Measure,
    );
    result.evaluation.unwrap();
    assert_eq!(text(&state), "Fresh");
}

#[test]
fn style_elements_are_recomputed_before_relayout() {
    let mut state = render("<style id='css'>#a{display:none}</style><div id='a'>Shown</div>");
    assert!(!text(&state).contains("Shown"));
    run_fixture_script(
        &mut state,
        "document.getElementById('css').textContent='#a{display:block}';",
        "style.js",
        &Measure,
    )
    .evaluation
    .unwrap();
    assert!(text(&state).contains("Shown"));
}

#[test]
fn host_traversal_budget_bounds_native_work_independently_of_js_fuel() {
    let mut state = render(&format!(
        "<div id='a'>Old</div>{}",
        "<span></span>".repeat(1024)
    ));
    let result = run_fixture_script(
        &mut state,
        "for(let i=0;i<500;i=i+1){document.getElementById('a').textContent;}",
        "host-work.js",
        &Measure,
    );
    let error = result.evaluation.unwrap_err();
    assert_eq!(error.kind, ErrorKind::Limit);
    assert_eq!(error.message, "fixture DOM work limit");
    assert!(!result.repaint_requested);
}

#[test]
fn thrown_wrappers_do_not_escape_the_discarded_context() {
    let mut state = render("<div id='a'>Old</div>");
    let result = run_fixture_script(
        &mut state,
        "throw document.getElementById('a');",
        "throw-wrapper.js",
        &Measure,
    );
    let error = result.evaluation.unwrap_err();
    assert_eq!(error.kind, ErrorKind::Exception);
    assert!(error.thrown.is_none());
    assert!(!result.repaint_requested);
}
