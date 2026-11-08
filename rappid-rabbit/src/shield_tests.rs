use super::*;
use rappid_rabbit::content_shield::{self, FilterListManager};
use std::sync::atomic::{AtomicUsize, Ordering};
static FETCHED: AtomicUsize = AtomicUsize::new(0);
fn manager() -> FilterListManager {
    let mut m = FilterListManager::new();
    m.add(
        "test",
        "Test",
        "bundled:test",
        "[Adblock Plus 2.0]\n/ads/*$image\n@@/ads/allow.png$image\n##.advert",
        content_shield::selector_valid,
    )
    .unwrap();
    m
}
fn app() -> RabbitApp {
    let mut app = RabbitApp::new(None);
    app.shield = manager();
    let url = ParsedUrl::parse("http://page.test/").unwrap();
    app.url_input.set_text(&format_url(&url));
    app.begin_navigation_session(&format_url(&url), &build_get_request(&url));
    app.apply_result(&format_url(&url),RequestResult{cert_info:None,connection_security:Default::default(),response:sunlight_http::HttpResponse{status_code:200,status_text:"OK".into(),headers:vec![("content-type".into(),"text/html".into())],header_len:0},body:b"<div class='advert'><a href='/ad'>Ad</a></div><p>Good</p><img src='/ads/banner.png'><img src='/images/good.png'><img src='/ads/allow.png'>".to_vec(),final_url:Some(url),duration_ms:None});
    app
}
fn fetch(
    _url: ParsedUrl,
    _request: HttpRequest,
    _limit: usize,
) -> Result<RequestResult, FetchError> {
    FETCHED.fetch_add(1, Ordering::Relaxed);
    Ok(RequestResult {
        connection_security: sunlight_cert::ConnectionSecurity::NotSecure,
        cert_info: None,
        response: sunlight_http::HttpResponse {
            status_code: 200,
            status_text: "OK".into(),
            headers: Vec::new(),
            header_len: 0,
        },
        body: include_bytes!("../../sun-img/tests/fixtures/gray.jpg").to_vec(),
        final_url: None,
        duration_ms: None,
    })
}
#[test]
fn blocked_images_fail_before_fetch_but_allow_rule_and_normal_image_dispatch() {
    let mut app = app();
    app.image_fetcher = fetch;
    let candidates = app.discovered_resources.clone();
    let (tx, _rx) = oneshot::channel();
    FETCHED.store(0, Ordering::Relaxed);
    app.fetch_images(&candidates, &tx);
    assert_eq!(FETCHED.load(Ordering::Relaxed), 2);
    assert!(app
        .image_cache
        .failed("http://page.test/ads/banner.png")
        .is_some());
    assert!(app
        .image_cache
        .decoded("http://page.test/images/good.png")
        .is_some());
    assert!(app
        .image_cache
        .decoded("http://page.test/ads/allow.png")
        .is_some());
    assert!(app.developer_tools.network.entries().iter().any(|e| e
        .error_text
        .as_deref()
        .is_some_and(|s| s.contains("Content Shield"))));
}
#[test]
fn settings_changes_cancel_old_jobs_and_reversibly_update_current_scene() {
    let mut app = app();
    let old_stats = app.shield_stats.clone();
    let old_revision = app.shield.revision;
    let dom = app.render_state.as_ref().unwrap().dom.clone();
    assert!(
        app.render_state
            .as_ref()
            .unwrap()
            .styles
            .shield_hidden_count()
            > 0
    );
    app.shield.settings.enabled = false;
    app.shield_changed();
    assert!(old_stats.cancelled.load(Ordering::Acquire));
    assert!(app.shield.revision > old_revision);
    assert_eq!(
        app.render_state
            .as_ref()
            .unwrap()
            .styles
            .shield_hidden_count(),
        0
    );
    assert!(std::sync::Arc::ptr_eq(
        &dom,
        &app.render_state.as_ref().unwrap().dom
    ));
    let (tx, rx) = oneshot::channel();
    app.image_completion = Some(rx);
    let mut cache = ImageCache::default();
    cache.insert_failed("http://stale.test/".into(), "stale");
    assert!(tx
        .send(ImageOutput {
            generation: app.document_lifecycle.generation(),
            shield_revision: old_revision,
            cache,
            diagnostics: DeveloperToolsState::default()
        })
        .is_ok());
    app.poll_images();
    assert!(app.image_cache.get("http://stale.test/").is_none());
    app.shield.settings.enabled = true;
    app.shield_changed();
    assert!(
        app.render_state
            .as_ref()
            .unwrap()
            .styles
            .shield_hidden_count()
            > 0
    );
}
#[test]
fn list_replacement_during_navigation_rejects_old_output() {
    let mut app = app();
    let mut old = RabbitApp::new(None);
    old.shield = app.shield.clone();
    old.document_lifecycle = app.document_lifecycle.clone();
    let (tx, rx) = oneshot::channel();
    app.completion = Some(rx);
    app.pending_fetch = true;
    app.shield
        .replace(
            "test",
            "[Adblock Plus 2.0]\n/new-rule",
            1,
            content_shield::selector_valid,
        )
        .unwrap();
    assert!(tx.send(old.into_navigation_output()).is_ok());
    app.perform_pending_fetch();
    assert!(app.render_status.contains("Content Shield changed"));
}
#[test]
fn popup_consumes_events_and_checkboxes_do_not_activate_page_links() {
    let mut app = app();
    let r = app.layout.shield;
    app.update_event(Event::Click {
        x: r.x + 2,
        y: r.y + 2,
    });
    assert!(app.shield_settings_open);
    assert!(app.update_event(Event::MouseDown {
        x: 20,
        y: 150,
        button: 0
    }));
    let site_before = app.shield.revision;
    let r = app.shield_settings_rect();
    app.update_event(Event::Click {
        x: r.x + 16,
        y: r.y + 43,
    });
    assert!(!app.shield.settings.enabled);
    assert!(app.shield.revision > site_before);
}

#[test]
fn stale_filter_reflow_cannot_commit_even_with_current_viewport_version() {
    let mut app = app();
    let old = app.render_state.as_ref().unwrap().reflow_snapshot();
    app.shield.settings.enabled = false;
    app.shield_changed();
    let revision = app.render_state.as_ref().unwrap().styles.shield_revision();
    let (tx, rx) = oneshot::channel();
    app.reflow_completion = Some(rx);
    assert!(tx
        .send(ReflowOutput {
            version: app.viewport_version,
            state: old
        })
        .is_ok());
    app.poll_reflow();
    assert_eq!(
        app.render_state.as_ref().unwrap().styles.shield_revision(),
        revision
    );
    assert_eq!(
        app.render_state
            .as_ref()
            .unwrap()
            .styles
            .shield_hidden_count(),
        0
    );
}

#[test]
fn navigation_shortcuts_remain_available_during_policy_reflow() {
    let mut app = app();
    app.shield_reflow_pending = true;
    app.update_event(Event::key_press(KEY_L, true, false, true, false, false));
    assert!(app.url_input.active);
    app.update_event(Event::Key('\n'));
    assert!(app.pending_fetch);
}

#[test]
fn stylesheet_block_fails_before_dns_and_keeps_embedded_content() {
    let mut app = RabbitApp::new(None);
    app.shield = manager();
    app.shield
        .replace(
            "test",
            "[Adblock Plus 2.0]\n||blocked.invalid^$stylesheet",
            1,
            content_shield::selector_valid,
        )
        .unwrap();
    let url = ParsedUrl::parse("http://page.test/").unwrap();
    app.url_input.set_text(&format_url(&url));
    app.begin_navigation_session(&format_url(&url), &build_get_request(&url));
    app.apply_result(
        &format_url(&url),
        RequestResult {
            connection_security: sunlight_cert::ConnectionSecurity::NotSecure,
            cert_info: None,
            response: sunlight_http::HttpResponse {
                status_code: 200,
                status_text: "OK".into(),
                headers: vec![("content-type".into(), "text/html".into())],
                header_len: 0,
            },
            body: b"<link rel='stylesheet' href='http://blocked.invalid/a.css'><p>Good</p>"
                .to_vec(),
            final_url: Some(url),
            duration_ms: None,
        },
    );
    assert!(app.render_state.is_some());
    assert!(app
        .developer_tools
        .network
        .entries()
        .iter()
        .any(|e| e.resource_type == ResourceType::Stylesheet
            && e.error_text
                .as_deref()
                .is_some_and(|s| s.contains("Content Shield"))));
}
