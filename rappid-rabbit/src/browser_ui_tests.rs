use super::*;
use sunlight_ui::widgets::{PaintOrder, RenderObjectId};

fn app() -> RabbitApp {
    fixture(include_str!("../tests/fixtures/browser-interaction.html"))
}

fn fixture(html: &str) -> RabbitApp {
    let mut app = RabbitApp::new(None);
    app.set_client_bounds(640, 480);
    let url = ParsedUrl::parse("http://fixture/").unwrap();
    app.begin_navigation_session("http://fixture/", &build_get_request(&url));
    app.apply_result(
        "http://fixture/",
        RequestResult {
            response: sunlight_http::HttpResponse {
                status_code: 200,
                status_text: "OK".into(),
                headers: vec![("content-type".into(), "text/html".into())],
                header_len: 0,
            },
            body: html.as_bytes().to_vec(),
            final_url: Some(url),
            duration_ms: Some(1),
        },
    );
    app.refresh_render_viewport();
    app.sync_document_scroll();
    app.refresh_tab_title();
    app
}

fn point(app: &mut RabbitApp, id: &str) -> Point {
    let rect = app.document_rect();
    let state = app.render_state.as_ref().unwrap();
    let node = state.dom.get_element_by_id(id).unwrap();
    let object = state
        .current_scene
        .objects
        .iter()
        .rev()
        .find(|o| o.owner_node_id.0 == node as u64 && o.bounds.w > 0 && o.bounds.h > 0)
        .unwrap();
    Point::new(
        rect.x + object.bounds.x + 1,
        rect.y + object.bounds.y + 1 - app.document_scroll.offset_y,
    )
}

fn wheel(app: &mut RabbitApp, delta: i16) -> bool {
    let rect = app.document_rect();
    app.update(Event::mouse_wheel(rect.x + 5, rect.y + 5, delta))
}

#[test]
fn wheel_preserves_both_directions_magnitude_and_boundaries() {
    let mut app = app();
    wheel(&mut app, -3);
    assert_eq!(app.document_scroll.offset_y, 0);
    assert!(wheel(&mut app, 3));
    assert_eq!(app.document_scroll.offset_y, 144);
    wheel(&mut app, -1);
    assert_eq!(app.document_scroll.offset_y, 96);
    wheel(&mut app, 180);
    assert_eq!(app.document_scroll.offset_y, 168);
    for _ in 0..10 {
        wheel(&mut app, i16::MAX);
    }
    assert_eq!(
        app.document_scroll.offset_y,
        app.document_scroll.max_offset_y()
    );
    let before = app.document_scroll.offset_y;
    wheel(&mut app, 120);
    assert_eq!(app.document_scroll.offset_y, before);
    wheel(&mut app, i16::MIN);
    assert_eq!(app.document_scroll.offset_y, 0);
}

#[test]
fn wheel_does_not_scroll_toolbar_tools_source_or_other_windows() {
    let mut app = app();
    let doc = app.document_rect();
    for (x, y) in [(20, 10), (-5, doc.y + 10), (650, doc.y + 10), (20, 490)] {
        app.update(Event::mouse_wheel(x, y, 3));
        assert_eq!(app.document_scroll.offset_y, 0);
    }
    app.developer_tools.panel.open();
    let panel = app.developer_panel_layout().panel_rect.unwrap();
    app.update(Event::mouse_wheel(panel.x + 10, panel.y + 10, 3));
    assert_eq!(app.document_scroll.offset_y, 0);
    app.document_view = DocumentView::Source;
    wheel(&mut app, 3);
    assert_eq!(app.document_scroll.offset_y, 0);
}

#[test]
fn thumb_drag_native_click_release_and_track_paging_share_keyboard_offset() {
    let mut app = app();
    let region = app.source_panel_rect();
    let track = app.document_scroll.track_rect(region);
    let thumb = app.document_scroll.thumb_rect(track).unwrap();
    app.update(Event::mouse_down(thumb.x + 2, thumb.y + 2, 0));
    assert!(app.document_scroll.dragging);
    app.update(Event::mouse_move(track.x + 2, track.bottom() + 100));
    assert_eq!(
        app.document_scroll.offset_y,
        app.document_scroll.max_offset_y()
    );
    // Native left release emits Click without a preceding MouseUp.
    app.update(Event::click(100, track.bottom() - 5));
    assert!(!app.document_scroll.dragging);
    assert!(!app.pending_fetch);
    app.update(Event::key_press(KEY_HOME, true, false, false, false, false));
    assert_eq!(app.document_scroll.offset_y, 0);
    app.update(Event::mouse_down(track.x + 2, track.bottom() - 5, 0));
    assert_eq!(
        app.document_scroll.offset_y,
        app.document_scroll.viewport_h as i32
    );
    app.update(Event::click(track.x + 2, track.bottom() - 5));
    assert_eq!(
        app.document_scroll.offset_y,
        app.document_scroll.viewport_h as i32
    );
    app.update(Event::key_press(KEY_UP, true, false, false, false, false));
    let before = app.document_scroll.offset_y;
    wheel(&mut app, -1);
    assert_eq!(app.document_scroll.offset_y, before - 48);
    let updated = app.document_scroll.thumb_rect(track).unwrap();
    assert!(updated.y > thumb.y);
    app.update(Event::click(track.x + 2, track.y + 1));
    assert_eq!(app.document_scroll.offset_y, 0);
}

#[test]
fn drag_cancels_on_focus_loss_and_window_resize() {
    let mut app = app();
    let region = app.source_panel_rect();
    let track = app.document_scroll.track_rect(region);
    let thumb = app.document_scroll.thumb_rect(track).unwrap();
    for loss in [
        Event::FocusChanged { focused: false },
        Event::PointerOwnership {
            owned: false,
            captured: false,
        },
    ] {
        app.update(Event::mouse_down(thumb.x + 1, thumb.y + 1, 0));
        app.update(loss);
        assert!(!app.document_scroll.dragging);
        assert_eq!(app.cursor, CursorShape::Pointer);
    }
    app.update(Event::mouse_down(thumb.x + 1, thumb.y + 1, 0));
    app.window_event(WindowEvent::Resized {
        width: 800,
        height: 600,
    });
    assert!(!app.document_scroll.dragging);
}

#[test]
fn resize_and_dimension_changes_clamp_without_moving_a_valid_offset() {
    let mut app = app();
    wheel(&mut app, 3);
    let before = app.document_scroll.offset_y;
    let generation = app.render_state.as_ref().unwrap().scene_generation;
    app.set_client_bounds(800, 600);
    assert_eq!(app.document_scroll.offset_y, before);
    assert_eq!(
        app.render_state.as_ref().unwrap().scene_generation,
        generation
    );
    app.set_client_bounds(800, 3000);
    assert_eq!(app.document_scroll.offset_y, 0);
    let region = app.source_panel_rect();
    assert!(app.document_scroll.thumb_rect(region).is_none());
}

#[test]
fn wheel_and_scrollbar_do_not_rebuild_or_replace_the_scene() {
    let mut app = app();
    let state = app.render_state.as_ref().unwrap();
    let scene = state.current_scene.clone();
    let version = app.viewport_version;
    wheel(&mut app, 3);
    let region = app.source_panel_rect();
    let track = app.document_scroll.track_rect(region);
    app.update(Event::click(track.x + 2, track.bottom() - 5));
    assert_eq!(app.viewport_version, version);
    assert!(alloc::sync::Arc::ptr_eq(
        &scene,
        &app.render_state.as_ref().unwrap().current_scene
    ));
    assert!(app.reflow_completion.is_none());
    assert_eq!(
        app.render_state
            .as_ref()
            .unwrap()
            .image_resource_priorities(Rect::new(
                0,
                app.document_scroll.offset_y,
                app.latest_viewport.w,
                app.latest_viewport.h
            ))
            .len(),
        0
    );
}

#[test]
fn short_document_has_no_interactive_scrollbar_and_gutter_never_covers_content() {
    let mut app = fixture("<p>Hello</p>");
    let region = app.source_panel_rect();
    assert_eq!(
        app.document_rect().right(),
        app.document_scroll.track_rect(region).x
    );
    assert!(!app.document_scroll.can_scroll_y());
    let track = app.document_scroll.track_rect(region);
    app.update(Event::mouse_down(track.x + 1, track.y + 1, 0));
    assert!(!app.document_scroll.dragging);
    assert_eq!(app.cursor, CursorShape::Pointer);
    wheel(&mut app, 120);
    assert_eq!(app.document_scroll.offset_y, 0);
}

#[test]
fn hover_classifies_links_inputs_controls_textareas_and_editable_text() {
    let mut app = app();
    for (id, cursor) in [
        ("link", CursorShape::Hand),
        ("field", CursorShape::Text),
        ("button", CursorShape::Hand),
        ("disabled", CursorShape::Pointer),
        ("textarea", CursorShape::Text),
        ("editable", CursorShape::Text),
    ] {
        let p = point(&mut app, id);
        app.update(Event::mouse_move(p.x, p.y));
        assert_eq!(app.cursor, cursor, "{id}");
    }
    app.update(Event::mouse_move(600, 100));
    assert_eq!(app.cursor, CursorShape::Pointer);
    let p = app.url_text_rect();
    app.update(Event::mouse_move(p.x + 1, p.y + 1));
    assert_eq!(app.cursor, CursorShape::Text);
    let p = app.devtools_button_rect();
    app.update(Event::mouse_move(p.x + 1, p.y + 1));
    assert_eq!(app.cursor, CursorShape::Hand);
}

#[test]
fn cursor_uses_topmost_clipped_target_and_scrolled_coordinates() {
    let mut app = app();
    let p = point(&mut app, "link");
    assert_eq!(app.cursor_at(p), CursorShape::Hand);
    let state = app.render_state.as_mut().unwrap();
    let mut scene = (*state.current_scene).clone();
    let link = scene
        .objects
        .iter()
        .find(|o| matches!(o.interaction, Some(RenderInteraction::Link { .. })))
        .unwrap()
        .clone();
    // A topmost opaque, noninteractive rectangle covers only part of the link.
    let mut overlay = link.clone();
    overlay.id = RenderObjectId(u64::MAX);
    overlay.interaction = None;
    overlay.kind = RenderObjectKind::Rectangle {
        fill: Color::rgb(0, 0, 0),
    };
    overlay.paint_order = PaintOrder { ..link.paint_order };
    overlay.clip_bounds = Some(Rect::new(link.bounds.x, link.bounds.y, 4, link.bounds.h));
    scene.objects.push(overlay);
    state.current_scene = alloc::sync::Arc::new(scene);
    assert_eq!(app.cursor_at(p), CursorShape::Pointer);
    assert_eq!(app.cursor_at(Point::new(p.x + 6, p.y)), CursorShape::Hand);
    assert_eq!(app.cursor_at(Point::new(p.x, 0)), CursorShape::Pointer);
    wheel(&mut app, 3);
    assert_eq!(app.cursor_at(p), CursorShape::Pointer);
}

#[test]
fn navigation_while_hovering_resets_cursor_and_wheel_is_safe_without_document() {
    let mut app = app();
    let p = point(&mut app, "link");
    app.update(Event::mouse_move(p.x, p.y));
    let url = ParsedUrl::parse("https://fixture/next").unwrap();
    app.begin_navigation_session("https://fixture/next", &build_get_request(&url));
    app.update(Event::mouse_wheel(p.x, p.y, 120));
    assert_eq!(app.cursor, CursorShape::Pointer);
    assert_eq!(app.document_scroll.offset_y, 0);
    assert!(app.render_state.is_none());
}

#[test]
fn enter_navigation_precedes_stale_form_focus_and_keeps_normalization() {
    let mut app = app();
    let field = app
        .render_state
        .as_ref()
        .unwrap()
        .dom
        .get_element_by_id("field")
        .unwrap();
    app.form_state.focus_control(DocumentNodeId(field as u64));
    app.url_input.active = true;
    app.url_input.set_text("example.com/a");
    app.update(Event::key_press(0x1c, true, false, false, false, false));
    assert!(!app.pending_fetch);
    assert!(app.update(Event::Key('\n')));
    assert!(app.pending_fetch);
    assert_eq!(app.url_input.value(), "example.com/a");
    assert_eq!(
        normalize_url_input(app.url_input.value()).unwrap(),
        "http://example.com/a"
    );
    assert_eq!(app.document_view, DocumentView::Render);
}

#[test]
fn compact_controls_keep_source_and_all_devtools_available() {
    let mut app = app();
    let button = app.devtools_button_rect();
    app.url_input.active = true;
    app.update(Event::click(button.x + 1, button.y + 1));
    assert!(app.developer_tools.panel.open);
    for tab in [
        DeveloperToolTab::Console,
        DeveloperToolTab::DomInspector,
        DeveloperToolTab::Network,
    ] {
        app.developer_tools.panel.active_tab = tab;
        assert!(app.developer_panel_layout().content_rect.is_some());
    }
    let view = app.view_button_rect();
    app.update(Event::click(view.x + 1, view.y + 1));
    assert_eq!(app.document_view, DocumentView::Source);
    app.update(Event::click(view.x + 1, view.y + 1));
    assert_eq!(app.document_view, DocumentView::Render);
}

#[test]
fn developer_tools_and_reload_shortcuts_remain_accessible() {
    let mut app = app();
    assert!(app.update(Event::key_press(0x58, true, false, false, false, false)));
    assert!(app.developer_tools.panel.open);
    app.update(Event::key_press(0x58, true, false, false, false, false));
    assert!(!app.developer_tools.panel.open);
    app.update(Event::key_press(0x13, true, false, true, false, false));
    assert!(app.pending_fetch);
}

#[test]
fn disabled_history_has_no_action_and_reload_uses_current_document_url() {
    let mut app = app();
    for button in [app.layout.back, app.layout.forward] {
        app.update(Event::click(button.x + 1, button.y + 1));
        assert!(!app.pending_fetch);
    }
    app.url_input.set_text("http://edited.invalid/");
    let action = app.url_action_rect();
    app.update(Event::click(action.x + 1, action.y + 1));
    assert!(app.pending_fetch);
    assert_eq!(app.url_input.value(), "http://fixture/");
    app.update(Event::click(action.x + 1, action.y + 1));
    assert!(!app.pending_fetch);
    assert_eq!(app.status, "Stopped");
}

#[test]
fn stop_discards_late_navigation_and_bounds_outstanding_work() {
    let mut app = app();
    let scene = app.render_state.as_ref().unwrap().current_scene.clone();
    let (sender, receiver) = oneshot::channel();
    app.queue_fetch();
    app.completion = Some(receiver);
    app.stop_navigation();
    assert!(app.navigation_draining);
    for _ in 0..10 {
        app.queue_fetch();
        app.update(Event::Tick);
    }
    assert!(!app.pending_fetch);
    let output = fixture("<title>Must not appear</title>").into_navigation_output();
    assert!(sender.send(output).is_ok());
    app.update(Event::Tick);
    assert!(!app.navigation_draining);
    assert!(alloc::sync::Arc::ptr_eq(
        &scene,
        &app.render_state.as_ref().unwrap().current_scene
    ));
    assert_ne!(app.tab_title, "Must not appear");
}

#[test]
fn narrow_toolbar_keeps_address_and_devtools_with_one_tab() {
    for width in [240, 320, 419, 420, 640, 1080] {
        let layout = RabbitApp::compute_layout(Rect::new(0, 0, width, 480));
        assert!(layout.url_bar.w >= if width < 420 { 150 } else { 250 });
        assert!(layout.devtools.right() <= width as i32);
        assert!(layout.url_bar.right() <= layout.view.x);
        assert!(layout.tab.right() <= width as i32);
        assert_eq!(layout.back.w == 0, width < 420);
    }
}

#[test]
fn link_and_get_form_still_navigate_and_post_remains_explicitly_unsupported() {
    let mut app = app();
    let p = point(&mut app, "link");
    app.update(Event::click(p.x, p.y));
    assert!(app.pending_fetch);
    assert_eq!(app.url_input.value(), "http://fixture/next");
    let mut app = fixture(include_str!("../tests/fixtures/browser-interaction.html"));
    let p = point(&mut app, "button");
    app.update(Event::click(p.x, p.y));
    assert!(app.pending_fetch);
    assert_eq!(app.url_input.value(), "http://fixture/search?q=rabbit");
    let mut app = fixture("<form method='post' action='/save'><input name='v' value='x'><button id='submit'>Save</button></form>");
    let p = point(&mut app, "submit");
    app.update(Event::click(p.x, p.y));
    assert!(!app.pending_fetch);
    assert!(app
        .developer_tools
        .console
        .rendered_text()
        .contains("POST forms are not supported"));
}

#[test]
#[cfg(feature = "js-dom-fixture")]
fn dom_mutations_refresh_cursor_title_and_scroll_geometry_on_idle() {
    let mut app = app();
    let p = point(&mut app, "field");
    app.update(Event::mouse_move(p.x, p.y));
    assert_eq!(app.cursor, CursorShape::Text);
    let result = rappid_rabbit::dom_fixture::run_fixture_script(app.render_state.as_mut().unwrap(),
        "document.getElementById('field').setAttribute('disabled',''); document.getElementById('title').textContent='Changed title';",
        "interaction.js", &RABBIT_FONTS);
    result.evaluation.unwrap();
    assert!(app.update(Event::Tick));
    assert_eq!(app.cursor, CursorShape::Pointer);
    assert_eq!(app.tab_title, "Changed title");
    let result = rappid_rabbit::dom_fixture::run_fixture_script(
        app.render_state.as_mut().unwrap(),
        "document.getElementById('disabled').removeAttribute('disabled');",
        "enable.js",
        &RABBIT_FONTS,
    );
    result.evaluation.unwrap();
    let enabled = point(&mut app, "disabled");
    app.update(Event::mouse_move(enabled.x, enabled.y));
    assert_eq!(app.cursor, CursorShape::Hand);
    wheel(&mut app, 3);
    let result = rappid_rabbit::dom_fixture::run_fixture_script(
        app.render_state.as_mut().unwrap(),
        "document.querySelector('body').textContent='Short document';",
        "short.js",
        &RABBIT_FONTS,
    );
    result.evaluation.unwrap();
    app.update(Event::Tick);
    assert_eq!(app.document_scroll.offset_y, 0);
    assert!(!app.document_scroll.can_scroll_y());
}

#[test]
fn tab_uses_title_or_safe_url_fallback() {
    let mut app = app();
    assert_eq!(app.tab_title, "Rabbit interaction fixture");
    app.queue_fetch();
    assert!(app.pending_fetch);
    let mut app = fixture("<p>No title</p>");
    assert_eq!(app.tab_title, "http://fixture/");
    app.render_state = None;
    app.navigation_url = "https://fixture/\n\rpage".into();
    app.refresh_tab_title();
    assert_eq!(app.tab_title, "https://fixture/ page");
}
