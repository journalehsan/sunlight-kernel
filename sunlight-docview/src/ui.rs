//! Native document canvas, with all document RPCs on a rendering worker.
use crate::viewer::{BitmapRef, DocViewer};
use anyhow::{Context, Result};
use std::{
    path::PathBuf,
    sync::{
        mpsc::{self, Receiver, SyncSender, TrySendError},
        Arc,
    },
    thread::{self, JoinHandle},
};
use sunlight_docserv::client::{DocClient, NativeClient};
use sunlight_ui::{
    app::App,
    geom::{Rect, Size},
    paint::Canvas,
    theme::Theme,
    widgets::document_canvas::{
        DocumentCanvas, DocumentCanvasMode, DocumentScene, PaintOrder, RasterImage, RenderObject,
        RenderObjectKind,
    },
    Event, WindowEvent,
};

#[derive(Clone, Copy)]
struct Intent {
    page: u32,
    scale: f32,
    generation: u64,
}

struct Snapshot {
    image: Arc<RasterImage>,
    page: u32,
    count: u32,
    generation: u64,
    cached_pages: usize,
    cached_bytes: usize,
}

/// Convert BGRA8 into the retained canvas's ARGB8888 raster format.
pub fn raster_image(bitmap: BitmapRef<'_>) -> Result<RasterImage> {
    anyhow::ensure!(
        bitmap.width > 0
            && bitmap.height > 0
            && bitmap.stride >= bitmap.width.checked_mul(4).context("stride overflow")?,
        "invalid raster dimensions"
    );
    let length = (bitmap.stride as usize)
        .checked_mul(bitmap.height as usize)
        .context("raster length overflow")?;
    anyhow::ensure!(bitmap.data.len() == length, "invalid raster storage");
    let pixels = bitmap
        .data
        .chunks_exact(bitmap.stride as usize)
        .flat_map(|row| {
            row[..bitmap.width as usize * 4]
                .chunks_exact(4)
                .map(|pixel| u32::from_le_bytes([pixel[0], pixel[1], pixel[2], pixel[3]]))
        })
        .collect();
    Ok(RasterImage {
        width: bitmap.width,
        height: bitmap.height,
        pixels,
    })
}

/// Graphical viewer. Its event callbacks enqueue intent and never perform RPCs.
pub struct DocumentApp {
    filename: String,
    requests: Option<SyncSender<Intent>>,
    results: Option<Receiver<std::result::Result<Snapshot, String>>>,
    worker: Option<JoinHandle<()>>,
    pending: Option<Intent>,
    generation: u64,
    desired_page: u32,
    shown_page: u32,
    total_pages: u32,
    zoom: f32,
    width: u32,
    height: u32,
    scene: Option<DocumentScene>,
    status: String,
    jump_input: Option<String>,
    scroll_y: u32,
    cache_pages: usize,
    cache_bytes: usize,
}

impl DocumentApp {
    /// Start rendering in the background while the native window paints.
    pub fn new(path: PathBuf) -> Result<Self> {
        Self::with_factory(path, NativeClient::connect)
    }

    /// Start with an injectable client, useful for deterministic UI validation.
    pub fn new_with_client<C: DocClient + Send + 'static>(
        path: PathBuf,
        client: C,
    ) -> Result<Self> {
        Self::with_factory(path, || Ok(client))
    }

    fn with_factory<C: DocClient + Send + 'static>(
        path: PathBuf,
        factory: impl FnOnce() -> Result<C> + Send + 'static,
    ) -> Result<Self> {
        let filename = path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Document".into());
        let (requests, incoming) = mpsc::sync_channel::<Intent>(1);
        let (outgoing, results) = mpsc::sync_channel(1);
        let worker = thread::Builder::new().spawn(move || {
            let operation = || -> Result<()> {
                let mut viewer = DocViewer::open_with_client(path, factory()?)?;
                while let Ok(intent) = incoming.recv() {
                    let rendered = (|| -> Result<Snapshot> {
                        viewer.set_view(intent.page.min(viewer.total_pages() - 1), intent.scale)?;
                        let image = Arc::new(raster_image(
                            viewer
                                .bitmap(viewer.current_page())
                                .context("current bitmap is missing")?,
                        )?);
                        Ok(Snapshot {
                            image,
                            page: viewer.current_page(),
                            count: viewer.total_pages(),
                            generation: intent.generation,
                            cached_pages: viewer.cached_pages(),
                            cached_bytes: viewer.cached_bytes(),
                        })
                    })()
                    .map_err(|e| format!("{e:#}"));
                    if outgoing.send(rendered).is_err() {
                        break;
                    }
                }
                viewer.close()
            };
            if let Err(error) = operation() {
                let _ = outgoing.send(Err(format!("{error:#}")));
            }
        })?;
        let mut app = Self {
            filename,
            requests: Some(requests),
            results: Some(results),
            worker: Some(worker),
            pending: None,
            generation: 0,
            desired_page: 0,
            shown_page: 0,
            total_pages: 0,
            zoom: 1.5,
            width: 800,
            height: 700,
            scene: None,
            status: "Opening document…".into(),
            jump_input: None,
            scroll_y: 0,
            cache_pages: 0,
            cache_bytes: 0,
        };
        app.request_render();
        Ok(app)
    }

    /// Filename and page indicator for the compositor's window title.
    pub fn title(&self) -> String {
        format!(
            "{} — Page {} of {}",
            self.filename,
            self.shown_page + 1,
            self.total_pages
        )
    }
    /// Whether the visible frame is waiting for the latest render intent.
    pub fn is_rendering(&self) -> bool {
        self.scene
            .as_ref()
            .is_none_or(|s| s.document_id != self.generation)
    }
    /// Resident rendered-page count and bytes, for memory-budget verification.
    pub fn cache_metrics(&self) -> (usize, usize) {
        (self.cache_pages, self.cache_bytes)
    }

    fn request_render(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        let fit = (self.width as f32 / 800.0).min(self.height as f32 / 700.0);
        self.pending = Some(Intent {
            page: self.desired_page,
            scale: (self.zoom * fit).clamp(0.5, 4.0),
            generation: self.generation,
        });
        self.scroll_y = 0;
        self.flush_request();
    }

    fn flush_request(&mut self) {
        if let (Some(sender), Some(intent)) = (&self.requests, self.pending.take()) {
            match sender.try_send(intent) {
                Ok(()) => {}
                Err(TrySendError::Full(intent)) => self.pending = Some(intent),
                Err(TrySendError::Disconnected(_)) => {
                    self.status = "Document rendering worker stopped".into()
                }
            }
        }
    }

    fn accept_results(&mut self) -> bool {
        let mut changed = false;
        while let Some(result) = self.results.as_ref().and_then(|r| r.try_recv().ok()) {
            match result {
                Ok(snapshot) if snapshot.generation == self.generation => {
                    self.shown_page = snapshot.page;
                    self.total_pages = snapshot.count;
                    self.cache_pages = snapshot.cached_pages;
                    self.cache_bytes = snapshot.cached_bytes;
                    self.desired_page = self.desired_page.min(self.total_pages - 1);
                    self.status = format!(
                        "Page {} of {}   Zoom {:.2}×",
                        self.shown_page + 1,
                        self.total_pages,
                        self.zoom
                    );
                    let image_size = Size::new(snapshot.image.width, snapshot.image.height);
                    let mut scene = DocumentScene::new(
                        snapshot.generation,
                        Size::new(self.width, self.height),
                        image_size,
                    );
                    scene.push(RenderObject {
                        id: sunlight_ui::widgets::RenderObjectId(1),
                        owner_node_id: sunlight_ui::widgets::DocumentNodeId(1),
                        bounds: Rect::new(0, 0, image_size.w, image_size.h),
                        clip_bounds: None,
                        paint_order: PaintOrder::default(),
                        interaction: None,
                        kind: RenderObjectKind::Image {
                            image: snapshot.image,
                            source_url: String::new(),
                            intrinsic_width: image_size.w,
                            intrinsic_height: image_size.h,
                            alt: "Document page".into(),
                        },
                    });
                    scene.finalize();
                    self.scene = Some(scene);
                    sunlight_ipc::debug_log(&format!(
                        "[DOCVIEW] page={} cache_pages={} cache_bytes={}\n",
                        self.shown_page + 1,
                        snapshot.cached_pages,
                        snapshot.cached_bytes
                    ));
                    changed = true;
                }
                Ok(_) => {}
                Err(error) => {
                    self.status = error;
                    changed = true;
                }
            }
        }
        self.flush_request();
        changed
    }

    /// Stop the rendering worker and wait for the document's Close reply.
    pub fn shutdown(&mut self) -> Result<()> {
        self.requests.take();
        self.results.take();
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| anyhow::anyhow!("document rendering worker panicked"))?;
        }
        Ok(())
    }
}

impl App for DocumentApp {
    fn view(&mut self, canvas: &mut Canvas, theme: &Theme) {
        canvas.fill_rect(Rect::new(0, 0, self.width, self.height), theme.bg);
        let rect = Rect::new(
            8,
            36,
            self.width.saturating_sub(16),
            self.height.saturating_sub(44),
        );
        let mut document = DocumentCanvas::new(rect, &[])
            .with_mode(DocumentCanvasMode::ReadOnly)
            .with_empty_label("Rendering document…")
            .with_scroll_y(self.scroll_y);
        if let Some(scene) = &self.scene {
            document = document.with_scene(scene);
        }
        document.draw(canvas, theme);
        let status = self
            .jump_input
            .as_ref()
            .map(|s| format!("Go to page: {s} (Enter to confirm, Escape to cancel)"))
            .unwrap_or_else(|| self.status.clone());
        canvas.draw_text(12, 12, &status, theme.text);
    }

    fn update(&mut self, event: Event) -> bool {
        let mut changed = self.accept_results();
        if let Some(input) = &mut self.jump_input {
            match event {
                Event::Key(ch) if ch.is_ascii_digit() && input.len() < 6 => {
                    input.push(ch);
                    return true;
                }
                Event::Key('\u{8}') => {
                    input.pop();
                    return true;
                }
                Event::Key('\n') => {
                    let page = input
                        .parse::<u32>()
                        .ok()
                        .filter(|&p| p > 0 && p <= self.total_pages);
                    self.jump_input = None;
                    if let Some(page) = page {
                        self.desired_page = page - 1;
                        self.request_render();
                    } else {
                        self.status = "Page number is out of range".into();
                    }
                    return true;
                }
                Event::KeyPress {
                    keycode: 0x01,
                    pressed: true,
                    ..
                } => {
                    self.jump_input = None;
                    return true;
                }
                _ => return changed,
            }
        }
        match event {
            Event::KeyPress {
                keycode: 0x4d | 0x51,
                pressed: true,
                ..
            } if self.total_pages > 0 => {
                self.desired_page = (self.desired_page + 1).min(self.total_pages - 1);
                self.request_render();
                changed = true;
            }
            Event::KeyPress {
                keycode: 0x4b | 0x49,
                pressed: true,
                ..
            } if self.total_pages > 0 => {
                self.desired_page = self.desired_page.saturating_sub(1);
                self.request_render();
                changed = true;
            }
            Event::Key('+') | Event::Key('=') => {
                self.zoom = (self.zoom + 0.25).min(4.0);
                self.request_render();
                changed = true;
            }
            Event::Key('-') => {
                self.zoom = (self.zoom - 0.25).max(0.5);
                self.request_render();
                changed = true;
            }
            Event::KeyPress {
                keycode: 0x22,
                pressed: true,
                ctrl: true,
                ..
            } => {
                self.jump_input = Some(String::new());
                changed = true;
            }
            Event::MouseWheel { delta, .. } => {
                self.scroll_y = if delta > 0 {
                    self.scroll_y.saturating_add(48)
                } else {
                    self.scroll_y.saturating_sub(48)
                };
                changed = true;
            }
            _ => {}
        }
        changed
    }

    fn window_event(&mut self, event: WindowEvent) -> bool {
        let WindowEvent::Resized { width, height } = event;
        self.width = width;
        self.height = height;
        self.request_render();
        true
    }
}

impl Drop for DocumentApp {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}
