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

struct UnavailableClient;
impl DocClient for UnavailableClient {
    fn request(&mut self, _: DocRequest) -> Result<DocResponse> {
        anyhow::bail!("docserv is unavailable")
    }
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
