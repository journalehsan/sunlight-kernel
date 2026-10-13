//! Real-PDF navigation, zoom, resize, and document-widget pixel verification.
use anyhow::Result;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use sunlight_docserv::{
    client::DocClient,
    ipc::{DocRequest, DocResponse},
    service::DocService,
};
use sunlight_docview::ui::DocumentApp;
use sunlight_ui::{app::App, paint::Canvas, Event, Theme, WindowEvent};

struct LocalClient(Arc<Mutex<DocService>>);
impl DocClient for LocalClient {
    fn request(&mut self, request: DocRequest) -> Result<DocResponse> {
        Ok(self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("service lock poisoned"))?
            .dispatch(1, request))
    }
}

fn wait_for_page(app: &mut DocumentApp, page: u32) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        app.update(Event::Tick);
        if !app.is_rendering() && app.title().ends_with(&format!("Page {page} of 6")) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    anyhow::bail!("UI did not render page {page}: {}", app.title())
}

fn key(code: u8) -> Event {
    Event::key_press(code, true, false, false, false, false)
}

#[test]
fn real_pdf_navigates_zooms_resizes_and_closes() -> Result<()> {
    let service = Arc::new(Mutex::new(DocService::default()));
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../sunlight-docserv/tests/fixtures/sample.pdf");
    let mut app = DocumentApp::new_with_client(path, LocalClient(service.clone()))?;
    wait_for_page(&mut app, 1)?;
    let initial_cache = app.cache_metrics();
    for page in 2..=6 {
        app.update(key(0x4d));
        wait_for_page(&mut app, page)?;
        assert!(app.cache_metrics().0 <= 5);
    }
    for page in (1..=5).rev() {
        app.update(key(0x4b));
        wait_for_page(&mut app, page)?;
    }
    assert_eq!(
        app.cache_metrics(),
        initial_cache,
        "cache grew after navigating away and back"
    );
    app.update(Event::Key('+'));
    wait_for_page(&mut app, 1)?;
    app.update(Event::Key('-'));
    wait_for_page(&mut app, 1)?;
    app.window_event(WindowEvent::Resized {
        width: 640,
        height: 560,
    });
    wait_for_page(&mut app, 1)?;
    assert!(
        app.cache_metrics().1 < initial_cache.1,
        "resize did not render at a new scale"
    );
    let mut pixels = vec![0; 640 * 560];
    app.view(
        &mut Canvas::new(&mut pixels, 640, 640, 560),
        &Theme::sunlight_dark(),
    );
    let blue_painted = pixels.iter().any(|&p| {
        p & 0xff == 204 && matches!((p >> 16) & 0xff, 25 | 26) && matches!((p >> 8) & 0xff, 76 | 77)
    });
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target");
    std::fs::create_dir_all(&directory)?;
    let mut ppm = b"P6\n640 560\n255\n".to_vec();
    for pixel in pixels {
        ppm.extend_from_slice(&[(pixel >> 16) as u8, (pixel >> 8) as u8, pixel as u8]);
    }
    std::fs::write(directory.join("docview-widget.ppm"), ppm)?;
    assert!(blue_painted, "PDF's blue square was not painted");
    app.shutdown()?;
    assert_eq!(
        service
            .lock()
            .map_err(|_| anyhow::anyhow!("service lock poisoned"))?
            .handle_count(),
        0
    );
    Ok(())
}

/// Wrap the in-process service and count RenderPage requests, so tests can
/// assert how many renders a UI action actually triggered.
struct CountingClient(Arc<Mutex<DocService>>, Arc<std::sync::atomic::AtomicUsize>);
impl DocClient for CountingClient {
    fn request(&mut self, request: DocRequest) -> Result<DocResponse> {
        if matches!(request, DocRequest::RenderPage { .. }) {
            self.1.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        Ok(self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("service lock poisoned"))?
            .dispatch(1, request))
    }
}

fn counting_app(
    service: Arc<Mutex<DocService>>,
) -> Result<(DocumentApp, Arc<std::sync::atomic::AtomicUsize>)> {
    let renders = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../sunlight-docserv/tests/fixtures/sample.pdf");
    let app = DocumentApp::new_with_client(path, CountingClient(service, renders.clone()))?;
    Ok((app, renders))
}

fn point_in(rect: sunlight_ui::Rect) -> (i32, i32) {
    (rect.x + (rect.w as i32 / 2), rect.y + (rect.h as i32 / 2))
}

#[test]
fn zoom_dropdown_presets_select_and_sync_slider() -> Result<()> {
    let service = Arc::new(Mutex::new(DocService::default()));
    let (mut app, _renders) = counting_app(service.clone())?;
    wait_for_page(&mut app, 1)?;
    // Open the dropdown, then choose "125%".
    let (dx, dy) = point_in(app.dropdown_rect());
    app.update(Event::Click { x: dx, y: dy });
    let item = sunlight_docview::zoom::ZoomPreset::ALL
        .iter()
        .position(|p| p.label() == "125%")
        .expect("125% preset");
    let (ix, iy) = point_in(app.dropdown_item_rect(item));
    app.update(Event::Click { x: ix, y: iy });
    wait_for_page(&mut app, 1)?;
    let (mode, percent, _page) = app.zoom_state();
    assert_eq!(mode, sunlight_docview::zoom::ZoomMode::Fixed(1.25));
    assert_eq!(percent, 125);
    Ok(())
}

#[test]
fn manual_zoom_switches_to_custom_and_plus_minus_step() -> Result<()> {
    let service = Arc::new(Mutex::new(DocService::default()));
    let (mut app, _renders) = counting_app(service.clone())?;
    wait_for_page(&mut app, 1)?;
    // Establish a known 100% baseline: the default Fit Page resolves to the
    // page's fitted percentage, so a bare −25 step would clamp to the slider
    // minimum instead of landing on 75%.
    let (dx, dy) = point_in(app.dropdown_rect());
    app.update(Event::Click { x: dx, y: dy });
    let item = sunlight_docview::zoom::ZoomPreset::ALL
        .iter()
        .position(|p| p.label() == "100%")
        .expect("100% preset");
    let (ix, iy) = point_in(app.dropdown_item_rect(item));
    app.update(Event::Click { x: ix, y: iy });
    wait_for_page(&mut app, 1)?;
    assert_eq!(app.zoom_state().1, 100);
    let (mx, my) = point_in(app.minus_rect());
    app.update(Event::Click { x: mx, y: my });
    wait_for_page(&mut app, 1)?;
    let (mode, percent, _) = app.zoom_state();
    // 100 -> 75 with a single 25-point step.
    assert_eq!(percent, 75);
    assert_eq!(mode, sunlight_docview::zoom::ZoomMode::Fixed(0.75));
    let (px, py) = point_in(app.plus_rect());
    app.update(Event::Click { x: px, y: py });
    wait_for_page(&mut app, 1)?;
    assert_eq!(app.zoom_state().1, 100);
    Ok(())
}

#[test]
fn navigation_buttons_stop_at_document_boundaries() -> Result<()> {
    let service = Arc::new(Mutex::new(DocService::default()));
    let (mut app, _renders) = counting_app(service.clone())?;
    wait_for_page(&mut app, 1)?;
    // Previous is disabled on the first page.
    let (x, y) = point_in(app.prev_rect());
    app.update(Event::Click { x, y });
    wait_for_page(&mut app, 1)?;
    assert_eq!(app.displayed_page().0, 0);
    // Walk to the last page with the Next button.
    let (x, y) = point_in(app.next_rect());
    for expected in 2..=6 {
        app.update(Event::Click { x, y });
        wait_for_page(&mut app, expected)?;
    }
    assert_eq!(app.displayed_page(), (5, 6));
    // Next is disabled on the last page.
    app.update(Event::Click { x, y });
    wait_for_page(&mut app, 6)?;
    assert_eq!(app.displayed_page(), (5, 6));
    Ok(())
}

#[test]
fn rapid_slider_movement_is_coalesced() -> Result<()> {
    let service = Arc::new(Mutex::new(DocService::default()));
    let (mut app, renders) = counting_app(service.clone())?;
    wait_for_page(&mut app, 1)?;
    let slider = app.slider_rect();
    let baseline = renders.load(std::sync::atomic::Ordering::SeqCst);
    let start_x = slider.x + 4;
    let (_, drag_y) = point_in(slider);
    app.update(Event::MouseDown {
        x: start_x,
        y: drag_y,
        button: 0,
    });
    // Many intermediate slider positions inside the debounce window.
    for step in 0..8 {
        let x = slider.x + (slider.w as i32 * (step + 1) / 9);
        app.update(Event::MouseMove { x, y: drag_y });
    }
    app.update(Event::MouseUp {
        x: slider.right() - 2,
        y: drag_y,
        button: 0,
    });
    // No intermediate value triggered a render RPC.
    assert_eq!(
        renders.load(std::sync::atomic::Ordering::SeqCst),
        baseline,
        "slider drag rendered an intermediate value"
    );
    // After the debounce window, the drag settles into a single render *scale*.
    std::thread::sleep(std::time::Duration::from_millis(200));
    app.update(Event::Tick);
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.is_rendering() && Instant::now() < deadline {
        app.update(Event::Tick);
        std::thread::sleep(Duration::from_millis(5));
    }
    let after = renders.load(std::sync::atomic::Ordering::SeqCst);
    // One settled scale re-renders the current page plus up to two prefetch
    // neighbors; crucially this is far below the eight intermediate values.
    let settle_renders = after.saturating_sub(baseline);
    assert!(
        (1..=3).contains(&settle_renders),
        "coalesced drag should render one scale, not every value (rendered {settle_renders})"
    );
    // Slider and mode agree after settling.
    let (mode, percent, _) = app.zoom_state();
    assert!(matches!(mode, sunlight_docview::zoom::ZoomMode::Fixed(_)));
    assert!((70..=200).contains(&percent));
    Ok(())
}

#[test]
fn zoom_persists_across_page_navigation() -> Result<()> {
    let service = Arc::new(Mutex::new(DocService::default()));
    let (mut app, _renders) = counting_app(service.clone())?;
    wait_for_page(&mut app, 1)?;
    let (px, py) = point_in(app.plus_rect());
    app.update(Event::Click { x: px, y: py });
    wait_for_page(&mut app, 1)?;
    let percent = app.zoom_state().1;
    // Navigate; the fixed zoom must be re-applied to the new page.
    app.update(key(0x4d));
    wait_for_page(&mut app, 2)?;
    assert_eq!(app.zoom_state().1, percent);
    assert_eq!(app.displayed_page().0, 1);
    Ok(())
}

/// Process resident set size in kilobytes, when the host exposes it.
fn resident_kb() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            return rest.split_whitespace().next()?.parse().ok();
        }
    }
    None
}

#[test]
fn repeated_open_render_zoom_close_is_memory_stable() -> Result<()> {
    let service = Arc::new(Mutex::new(DocService::default()));
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../sunlight-docserv/tests/fixtures/sample.pdf");
    // Warm the allocator with one full cycle before sampling.
    let mut warm = DocumentApp::new_with_client(path.clone(), LocalClient(service.clone()))?;
    wait_for_page(&mut warm, 1)?;
    warm.shutdown()?;
    drop(warm);
    let mut baseline = None;
    for cycle in 0..6 {
        let mut app = DocumentApp::new_with_client(path.clone(), LocalClient(service.clone()))?;
        wait_for_page(&mut app, 1)?;
        for pages in 0..2 {
            for page in 2..=6 {
                app.update(key(0x4d));
                wait_for_page(&mut app, page)?;
                assert!(app.cache_metrics().0 <= 5, "page cache is unbounded");
                assert!(
                    app.cache_metrics().1 <= 40 * 1024 * 1024,
                    "page cache exceeded its byte budget"
                );
            }
            for page in (1..=5).rev() {
                app.update(key(0x4b));
                wait_for_page(&mut app, page)?;
            }
            // Exercise zoom entry points between navigation sweeps.
            let (px, py) = point_in(app.plus_rect());
            app.update(Event::Click { x: px, y: py });
            wait_for_page(&mut app, 1)?;
            let (mx, my) = point_in(app.minus_rect());
            app.update(Event::Click { x: mx, y: my });
            wait_for_page(&mut app, 1)?;
            let _ = pages;
        }
        app.shutdown()?;
        drop(app);
        if let Some(rss) = resident_kb() {
            baseline.get_or_insert(rss);
            eprintln!("cycle {cycle}: rss={rss} KiB");
        }
    }
    if let (Some(first), Some(last)) = (baseline, resident_kb()) {
        // Allow allocator slack, but a real leak would grow per cycle.
        assert!(
            last < first + 40 * 1024,
            "resident memory grew across cycles: {first} -> {last} KiB"
        );
    }
    assert_eq!(
        service
            .lock()
            .map_err(|_| anyhow::anyhow!("service lock poisoned"))?
            .handle_count(),
        0,
        "document handles leaked across cycles"
    );
    Ok(())
}

struct UnavailableClient;
impl DocClient for UnavailableClient {
    fn request(&mut self, _: DocRequest) -> Result<DocResponse> {
        anyhow::bail!("docserv is unavailable")
    }
}

#[test]
fn malformed_pdf_fails_cleanly_and_releases_handles() -> Result<()> {
    let service = Arc::new(Mutex::new(DocService::default()));
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target");
    std::fs::create_dir_all(&directory)?;
    let path = directory.join("malformed-audit.pdf");
    // A syntactically PDF-ish header with a truncated/garbage body.
    std::fs::write(
        &path,
        b"%PDF-1.7\n1 0 obj\n<< bad >>\nendobj\ntrailer\n%%EOF\n",
    )?;
    let mut app = DocumentApp::new_with_client(path, LocalClient(service.clone()))?;
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.is_rendering() && Instant::now() < deadline {
        app.update(Event::Tick);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!app.is_rendering(), "malformed PDF left the viewer loading");
    assert_eq!(app.displayed_page().1, 0, "malformed PDF reported pages");
    assert_eq!(
        service
            .lock()
            .map_err(|_| anyhow::anyhow!("service lock poisoned"))?
            .handle_count(),
        0,
        "malformed PDF leaked a document handle"
    );
    app.shutdown()?;
    Ok(())
}

#[test]
fn backend_failure_stops_loading() -> Result<()> {
    let mut app = DocumentApp::new_with_client("missing.pdf".into(), UnavailableClient)?;
    let deadline = Instant::now() + Duration::from_secs(2);
    while app.is_rendering() && Instant::now() < deadline {
        app.update(Event::Tick);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        !app.is_rendering(),
        "backend failure left the viewer loading"
    );
    // Further view intents must also settle when the failed worker has exited.
    app.update(Event::Key('+'));
    app.update(Event::Tick);
    assert!(!app.is_rendering());
    app.shutdown()?;
    Ok(())
}
