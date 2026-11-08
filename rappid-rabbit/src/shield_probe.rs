//! Opt-in native probe for a controlled local fixture; not reachable by a URL.
use super::*;
use rappid_rabbit::content_shield::{
    self, CanonicalUrl, CosmeticPolicy, Decision, ResourceType as ShieldType,
};
pub(super) fn run(url: &str) -> Result<(), Box<dyn std::error::Error>> {
    let started = std::time::Instant::now();
    let parsed = ParsedUrl::parse(url).map_err(|e| format!("{e}"))?;
    if !matches!(parsed.host.as_str(), "10.0.2.2" | "127.0.0.1") || parsed.uses_tls() {
        return Err("probe requires a local HTTP fixture".into());
    }
    let mut app = RabbitApp::new(None);
    app.metrics.started = Some(started);
    app.url_input.set_text(url);
    app.begin_navigation_session(url, &build_get_request(&parsed));
    let result =
        perform_request(parsed.clone(), build_get_request(&parsed)).map_err(|e| format!("{e}"))?;
    app.apply_result(url, result);
    // This direct fixture path bypasses the normal navigation-output commit.
    app.refresh_render_viewport();
    let initial_scene_us = started.elapsed().as_micros();
    let state = app.render_state.as_ref().ok_or("fixture did not parse")?;
    let dom = state.dom.clone();
    let node_count = dom.node_count();
    assert!(state.styles.shield_hidden_count() > 0);
    let candidates = app.discovered_resources.clone();
    let (tx, _rx) = oneshot::channel();
    app.fetch_images(&candidates, &tx);
    let base = format!("http://{}:{}", parsed.host, parsed.port);
    assert!(app
        .image_cache
        .failed(&format!("{base}/ads/banners/sale.png"))
        .is_some());
    assert!(app
        .image_cache
        .decoded(&format!("{base}/images/good.jpg"))
        .is_some());
    let cache = app.image_cache.clone();
    app.render_state
        .as_mut()
        .unwrap()
        .rebuild_for_images(&RABBIT_FONTS, cache);
    let resources_ready_us = started.elapsed().as_micros();
    debug_log(&format!("[SHIELD-NATIVE] initial_scene_us={initial_scene_us} resources_ready_us={resources_ready_us}"));
    for i in 0..12 {
        let source=format!("document.getElementById('ad-banner-top').setAttribute('class','state{i}'); if(!document.getElementById('ad-banner-top')) throw 'removed';");
        let result = rappid_rabbit::dom_fixture::run_fixture_script(
            app.render_state.as_mut().unwrap(),
            &source,
            "shield-native",
            &RABBIT_FONTS,
        );
        result.evaluation?;
        assert_eq!(
            app.render_state.as_ref().unwrap().dom.node_count(),
            node_count
        );
    }
    let current = app.render_state.as_mut().unwrap();
    let retained = current.dom.clone();
    let mut disabled = app.shield.clone();
    disabled.settings.enabled = false;
    disabled.changed();
    current.set_shield_policy(CosmeticPolicy::for_document(&disabled, url), &RABBIT_FONTS);
    assert_eq!(current.styles.shield_hidden_count(), 0);
    current.set_shield_policy(
        CosmeticPolicy::for_document(&app.shield, url),
        &RABBIT_FONTS,
    );
    assert!(current.styles.shield_hidden_count() > 0);
    assert!(alloc::sync::Arc::ptr_eq(&retained, &current.dom));
    let doc = CanonicalUrl::parse("https://page.test/").unwrap();
    let request = CanonicalUrl::parse("https://cdn.test/ads/banner.png").unwrap();
    let mut custom = golden_fish::shield::FilterListManager::new();
    custom
        .add(
            "test",
            "Test",
            "bundled:test",
            "[Adblock Plus 2.0]\n/ads/*$image,third-party\n@@/ads/allow.png$image",
            content_shield::selector_valid,
        )
        .unwrap();
    assert_eq!(
        custom.evaluate(&request, &doc, ShieldType::Image),
        Decision::Block
    );
    custom.settings.set_site_exception(&doc.host, true);
    assert_eq!(
        custom.evaluate(&request, &doc, ShieldType::Image),
        Decision::Allow
    );
    #[cfg(target_os = "sunlightos")]
    {
        assert!(content_shield::save_settings(&app.shield.settings));
        assert_eq!(
            content_shield::load_settings(),
            Some(app.shield.settings.clone())
        );
        debug_log("[SHIELD-NATIVE] preferences roundtrip PASS");
    }
    debug_log("[SHIELD-NATIVE] PASS network_block=1 allowed_image=1 reversible=1 mutations=12 dom_preserved=1 exceptions=1");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .max_blocking_threads(1)
        .thread_stack_size(4 * 1024 * 1024)
        .build()?;
    app.runtime = Some(runtime.handle().clone());
    app.metrics.page_ready_us = Some(resources_ready_us);
    app.image_attempts = candidates.iter().map(|c| c.resolved_url.clone()).collect();
    let mut window = Window::connect(WindowConfig {
        width: WIN_W,
        height: WIN_H,
        title: "Content Shield fixture",
        decoration: WindowDecoration::Normal,
    })
    .ok_or("fixture window")?;
    debug_log("[SHIELD-NATIVE] fixture window ready");
    window.run(&mut app);
    drop(app);
    runtime.shutdown_timeout(std::time::Duration::ZERO);
    Ok(())
}
