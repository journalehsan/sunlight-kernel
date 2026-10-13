//! Native document canvas with a zoom/navigation toolbar and status bar.
//!
//! All document RPCs run on a background rendering worker. The UI thread only
//! computes zoom from cached page dimensions and the live viewport, coalesces
//! rapid zoom intent, and paints the retained scene.
use crate::viewer::{BitmapRef, DocViewer, ViewerMemorySnapshot};
use crate::zoom::{
    mode_label, scale_from_percent, step_percent, ZoomMode, ZoomPreset, MAX_ZOOM_PERCENT,
    MIN_ZOOM_PERCENT,
};
use anyhow::{Context, Result};
use std::{
    path::PathBuf,
    sync::{
        mpsc::{self, Receiver, SyncSender, TrySendError},
        Arc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use sun_font::{FontRole, VecFont};
use sunlight_docserv::client::{DocClient, NativeClient};
use sunlight_ui::{
    app::App,
    geom::{Rect, Size},
    paint::Canvas,
    theme::Theme,
    widgets::{
        button::ButtonState,
        document_canvas::{
            DocumentCanvas, DocumentCanvasMode, DocumentCanvasPresentation, DocumentScene,
            PaintOrder, RasterImage, RenderObject, RenderObjectKind, ScrollView,
        },
        section::{Section, SectionState},
        slider::Slider,
        IconButton,
    },
    Event, UiSymbol, VecText, WindowEvent,
};

/// Debounce window for continuous zoom (slider) changes.
const ZOOM_DEBOUNCE: Duration = Duration::from_millis(150);
const TOOLBAR_H: u32 = 44;
const STATUS_H: u32 = 32;
static FONT_UI: VecFont = VecFont(FontRole::UiRegular);
static FONT_UI_MEDIUM: VecFont = VecFont(FontRole::UiMedium);
static FONT_UI_SMALL: VecFont = VecFont(FontRole::UiSmall);
/// Clamp on the preview-only rebase scale applied to the last rendered bitmap.
const PREVIEW_SCALE_MIN: f32 = 0.1;
const PREVIEW_SCALE_MAX: f32 = 10.0;

#[derive(Clone, Copy)]
struct Intent {
    page: u32,
    mode: ZoomMode,
    viewport: (u32, u32),
    generation: u64,
}

struct Snapshot {
    image: Arc<RasterImage>,
    page: u32,
    count: u32,
    generation: u64,
    cached_pages: usize,
    cached_bytes: usize,
    zoom_percent: u32,
    page_pt: (f32, f32),
}

/// UI-thread-local memory contributors, distinct from the worker's cache.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LocalMemorySnapshot {
    /// The render worker's `DocViewer` cache, reported through the snapshot.
    pub worker: ViewerMemorySnapshot,
    /// This thread's retained scene bitmap (`RasterImage::pixels`, ARGB8888).
    pub scene_preview_bytes: u64,
}

/// Live bytes held by this process's allocator, when the target exposes it.
///
/// The native std build uses the `sunlight-libc` `System` allocator, whose
/// `heap_stats()` reports live requested bytes. Host/alloc-only builds return 0
/// = "not accessible".
fn heap_estimate_bytes() -> u64 {
    #[cfg(all(feature = "native-std", target_os = "sunlightos"))]
    {
        sunlight_libc::alloc::heap_stats().requested_user_bytes as u64
    }
    #[cfg(not(all(feature = "native-std", target_os = "sunlightos")))]
    {
        0
    }
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
    mode: ZoomMode,
    zoom_percent: u32,
    /// Cached point dimensions of the displayed page, for fit-mode preview.
    page_pt: Option<(f32, f32)>,
    width: u32,
    height: u32,
    scene: Option<DocumentScene>,
    status: String,
    render_failed: bool,
    jump_input: Option<String>,
    scroll_y: u32,
    cache_pages: usize,
    cache_bytes: usize,
    dropdown_open: bool,
    zoom_settle_at: Option<Instant>,
    pending_zoom_percent: Option<u32>,
    slider: Slider,
    /// Page index whose memory snapshot was last logged, to log per page turn.
    page_turn_logged_page: Option<u32>,
    /// Whether the first render's memory snapshot has been logged.
    first_render_logged: bool,
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
                        viewer.set_viewport(intent.viewport.0, intent.viewport.1);
                        let page = intent.page.min(viewer.total_pages() - 1);
                        viewer.set_mode(page, intent.mode)?;
                        let dimensions = viewer.page_dimensions(page)?;
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
                            zoom_percent: viewer.zoom_percent(),
                            page_pt: dimensions,
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
            mode: ZoomMode::FitPage,
            zoom_percent: 100,
            page_pt: None,
            width: 800,
            height: 700,
            scene: None,
            status: "Opening document…".into(),
            render_failed: false,
            jump_input: None,
            scroll_y: 0,
            cache_pages: 0,
            cache_bytes: 0,
            dropdown_open: false,
            zoom_settle_at: None,
            pending_zoom_percent: None,
            slider: Slider::horizontal(Rect::new(0, 0, 1, 1)),
            page_turn_logged_page: None,
            first_render_logged: false,
        };
        app.log_local_memory("startup");
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
        !self.render_failed
            && (self.zoom_settle_at.is_some()
                || self
                    .scene
                    .as_ref()
                    .is_none_or(|s| s.document_id != self.generation))
    }
    /// Resident rendered-page count and bytes, for memory-budget verification.
    pub fn cache_metrics(&self) -> (usize, usize) {
        (self.cache_pages, self.cache_bytes)
    }

    /// Viewer-local memory contributors, measured from the last render frame.
    ///
    /// `worker_page_cache_bytes`/`worker_cached_pages` are the render worker's
    /// `DocViewer` cache (reported through the snapshot). `scene_preview_bytes`
    /// is this UI thread's retained scene bitmap (`RasterImage::pixels`,
    /// ARGB8888), which lives in a *separate* allocator from the worker. The
    /// transient BGRA->ARGB conversion buffer is dropped before the worker
    /// sends the snapshot, so it is not counted.
    pub fn local_memory_snapshot(&self) -> LocalMemorySnapshot {
        let mut scene_preview_bytes = 0u64;
        if let Some(scene) = &self.scene {
            for object in &scene.objects {
                if let sunlight_ui::widgets::RenderObjectKind::Image { image, .. } = &object.kind {
                    scene_preview_bytes = scene_preview_bytes
                        .saturating_add((image.pixels.len() * core::mem::size_of::<u32>()) as u64);
                }
            }
        }
        LocalMemorySnapshot {
            worker: ViewerMemorySnapshot {
                page_cache_bytes: self.cache_bytes as u64,
                cached_pages: self.cache_pages,
                page_dimension_bytes: 0,
            },
            scene_preview_bytes,
        }
    }

    /// Log the UI-local memory snapshot with a caller-supplied phase tag.
    pub fn log_local_memory(&self, phase: &str) {
        let snapshot = self.local_memory_snapshot();
        sunlight_ipc::debug_log(&format!(
            "[DOCVIEW] mem phase={} worker_page_cache_bytes={} worker_cached_pages={} \
             scene_preview_bytes={} heap_estimate_bytes={}\n",
            phase,
            snapshot.worker.page_cache_bytes,
            snapshot.worker.cached_pages,
            snapshot.scene_preview_bytes,
            heap_estimate_bytes(),
        ));
    }

    /// Last successfully displayed page and its document's page count.
    pub fn displayed_page(&self) -> (u32, u32) {
        (self.shown_page, self.total_pages)
    }
    /// Active zoom mode, resolved percentage, and current page.
    pub fn zoom_state(&self) -> (ZoomMode, u32, u32) {
        (self.mode, self.zoom_percent, self.shown_page)
    }

    /// Drawable document-canvas rectangle below the toolbar and above the bar.
    pub fn canvas_rect(&self) -> Rect {
        let h = self
            .height
            .saturating_sub(TOOLBAR_H)
            .saturating_sub(STATUS_H);
        Rect::new(0, TOOLBAR_H as i32, self.width, h)
    }

    pub fn toolbar_rect(&self) -> Rect {
        Rect::new(0, 0, self.width, TOOLBAR_H)
    }

    pub fn status_rect(&self) -> Rect {
        Rect::new(
            0,
            self.height.saturating_sub(STATUS_H) as i32,
            self.width,
            STATUS_H,
        )
    }

    pub fn prev_rect(&self) -> Rect {
        Rect::new(10, 6, 32, 32)
    }
    pub fn next_rect(&self) -> Rect {
        Rect::new(48, 6, 32, 32)
    }
    pub fn page_label_rect(&self) -> Rect {
        Rect::new(92, 6, 132, 32)
    }
    pub fn dropdown_rect(&self) -> Rect {
        let w = 124u32.min(self.width / 3);
        Rect::new((self.width.saturating_sub(w + 10)) as i32, 6, w, 32)
    }
    pub fn dropdown_item_rect(&self, index: usize) -> Rect {
        let base = self.dropdown_rect();
        let item_h = 24u32;
        let total = item_h * ZoomPreset::ALL.len() as u32;
        let top = if base.bottom() + total as i32 + 2 <= self.height as i32 {
            base.bottom() + 2
        } else {
            (base.y - total as i32 - 2).max(TOOLBAR_H as i32)
        };
        Rect::new(base.x, top + (index as u32 * item_h) as i32, base.w, item_h)
    }
    pub fn minus_rect(&self) -> Rect {
        let bar = self.status_rect();
        Rect::new(bar.x + 10, bar.y + 5, 28, 22)
    }
    pub fn plus_rect(&self) -> Rect {
        let bar = self.status_rect();
        Rect::new(bar.x + 10 + 28 + 6 + 180 + 6, bar.y + 5, 28, 22)
    }
    pub fn slider_rect(&self) -> Rect {
        let bar = self.status_rect();
        Rect::new(bar.x + 10 + 28 + 6, bar.y + 8, 180, 16)
    }

    fn can_prev(&self) -> bool {
        self.total_pages > 0 && self.shown_page > 0
    }
    fn can_next(&self) -> bool {
        self.total_pages > 0 && self.shown_page + 1 < self.total_pages
    }

    /// Point the slider widget at its live rect/range and mirror the resolved
    /// zoom into it, unless the user is currently dragging (the widget owns the
    /// value then, and the resolved zoom is derived from it).
    fn refresh_slider(&mut self) {
        self.slider.rect = self.slider_rect();
        self.slider.set_range(MIN_ZOOM_PERCENT, MAX_ZOOM_PERCENT);
        if !self.slider.dragging {
            self.slider.set_value(self.slider_percent());
        }
    }

    /// Percentage shown on the slider, clamped to the slider's range.
    fn slider_percent(&self) -> u32 {
        self.zoom_percent.clamp(MIN_ZOOM_PERCENT, MAX_ZOOM_PERCENT)
    }

    /// The scale the UI believes it is showing for the desired page.
    ///
    /// Uses cached page dimensions when available so fit modes track the
    /// viewport immediately; otherwise falls back to the last resolved percent.
    fn desired_scale(&self) -> f32 {
        match self.page_pt {
            Some(pt) => {
                let size = self.canvas_rect().size();
                self.mode.resolve(pt, (size.w, size.h))
            }
            None => scale_from_percent(self.zoom_percent),
        }
    }

    fn request_render(&mut self) {
        self.render_failed = false;
        self.generation = self.generation.wrapping_add(1);
        let viewport = self.canvas_rect().size();
        self.pending = Some(Intent {
            page: self.desired_page,
            mode: self.mode,
            viewport: (viewport.w, viewport.h),
            generation: self.generation,
        });
        self.scroll_y = 0;
        self.flush_request();
    }

    /// Queue a coalesced render that only fires after the zoom settles.
    fn schedule_zoom(&mut self, mode: ZoomMode) {
        self.mode = mode;
        self.pending_zoom_percent = Some(self.slider_percent());
        self.zoom_settle_at = Some(Instant::now() + ZOOM_DEBOUNCE);
    }

    fn apply_settled_zoom(&mut self) -> bool {
        if let Some(deadline) = self.zoom_settle_at {
            if Instant::now() >= deadline {
                self.zoom_settle_at = None;
                self.pending_zoom_percent = None;
                self.request_render();
                return true;
            }
        }
        false
    }

    fn flush_request(&mut self) {
        if let (Some(sender), Some(intent)) = (&self.requests, self.pending.take()) {
            match sender.try_send(intent) {
                Ok(()) => {}
                Err(TrySendError::Full(intent)) => self.pending = Some(intent),
                Err(TrySendError::Disconnected(_)) => {
                    self.render_failed = true;
                    self.status = "Document rendering worker stopped".into();
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
                    self.zoom_percent = snapshot.zoom_percent;
                    self.page_pt = Some(snapshot.page_pt);
                    self.desired_page = self.desired_page.min(self.total_pages - 1);
                    let mode = mode_label(self.mode, self.slider_percent());
                    self.status = format!(
                        "Page {} of {}   {} ({}%)",
                        self.shown_page + 1,
                        self.total_pages,
                        mode,
                        snapshot.zoom_percent
                    );
                    self.scene = Some(self.build_scene(&snapshot));
                    sunlight_ipc::debug_log(&format!(
                        "[DOCVIEW] page={} cache_pages={} cache_bytes={} zoom={}%\n",
                        self.shown_page + 1,
                        snapshot.cached_pages,
                        snapshot.cached_bytes,
                        snapshot.zoom_percent
                    ));
                    // Log a memory snapshot on the first render and on each page
                    // turn (not on every coalesced zoom settle of the same page).
                    let phase = if self.page_turn_logged_page != Some(self.shown_page) {
                        self.page_turn_logged_page = Some(self.shown_page);
                        if self.first_render_logged {
                            "page-turn"
                        } else {
                            self.first_render_logged = true;
                            "first-render"
                        }
                    } else {
                        "zoom"
                    };
                    self.log_local_memory(phase);
                    changed = true;
                }
                Ok(_) => {}
                Err(error) => {
                    self.render_failed = true;
                    sunlight_ipc::debug_log(&format!("[DOCVIEW] render failed: {error}\n"));
                    self.status = error;
                    changed = true;
                }
            }
        }
        self.flush_request();
        changed
    }

    /// Preview the last rendered bitmap at the desired scale.
    ///
    /// The worker re-renders at the exact scale asynchronously; until it
    /// arrives the retained bitmap is rebased by `desired / rendered` so the
    /// page tracks zoom and resize smoothly without a blank frame.
    fn build_scene(&self, snapshot: &Snapshot) -> DocumentScene {
        let rendered_scale = (snapshot.image.width as f32 / snapshot.page_pt.0.max(1.0)).max(1e-3);
        let preview =
            (self.desired_scale() / rendered_scale).clamp(PREVIEW_SCALE_MIN, PREVIEW_SCALE_MAX);
        let width = ((snapshot.image.width as f32 * preview).round() as u32).max(1);
        let height = ((snapshot.image.height as f32 * preview).round() as u32).max(1);
        let canvas = self.canvas_rect().size();
        let mut scene = DocumentScene::new(snapshot.generation, canvas, Size::new(width, height));
        scene.push(RenderObject {
            id: sunlight_ui::widgets::RenderObjectId(1),
            owner_node_id: sunlight_ui::widgets::DocumentNodeId(1),
            bounds: Rect::new(0, 0, width, height),
            clip_bounds: None,
            paint_order: PaintOrder::default(),
            interaction: None,
            kind: RenderObjectKind::Image {
                image: snapshot.image.clone(),
                source_url: String::new(),
                intrinsic_width: snapshot.image.width,
                intrinsic_height: snapshot.image.height,
                alt: "Document page".into(),
            },
        });
        scene.finalize();
        scene
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

    fn select_zoom_preset(&mut self, preset: ZoomPreset) {
        self.dropdown_open = false;
        self.mode = preset.mode();
        self.request_render();
    }

    fn nudge_zoom(&mut self, steps: i32) {
        let percent = step_percent(self.slider_percent(), steps);
        self.mode = ZoomMode::Fixed(scale_from_percent(percent));
        self.zoom_percent = percent;
        self.request_render();
    }

    fn navigate(&mut self, delta: i32) {
        let next = self.desired_page as i64 + delta as i64;
        if next < 0 || next >= self.total_pages as i64 {
            return;
        }
        self.desired_page = next as u32;
        self.request_render();
    }
}

impl App for DocumentApp {
    fn view(&mut self, canvas: &mut Canvas, theme: &Theme) {
        self.refresh_slider();
        canvas.fill_rect(Rect::new(0, 0, self.width, self.height), theme.bg);
        let rect = self.canvas_rect();
        let mut document = DocumentCanvas::new(rect, &[])
            .with_mode(DocumentCanvasMode::ReadOnly)
            .with_presentation(DocumentCanvasPresentation::Browser)
            .with_empty_label(if self.render_failed {
                "Unable to render document"
            } else {
                "Rendering document…"
            })
            .with_scroll_y(self.scroll_y);
        if let Some(scene) = &self.scene {
            document = document.with_scene(scene);
        }
        document.draw(canvas, theme);

        // Scrollbar thumb for the retained scene.
        if let Some(scene) = &self.scene {
            let max_scroll = scene.max_scroll_y(rect.h);
            if max_scroll > 0 {
                let bar = ScrollView::geometry(
                    rect.h,
                    scene.content_size.h,
                    self.scroll_y,
                    max_scroll,
                    rect,
                    ScrollView::WIDTH,
                );
                canvas.fill_rect(bar.track, theme.panel_alt);
                canvas.fill_rect(bar.thumb, theme.border);
            }
        }

        self.draw_toolbar(canvas, theme);
        self.draw_status(canvas, theme);
        if self.dropdown_open {
            self.draw_dropdown(canvas, theme);
        }
        if let Some(input) = &self.jump_input {
            let message = format!("Go to page: {input} (Enter to confirm, Escape to cancel)");
            FONT_UI.draw_vcenter(
                canvas,
                &message,
                12,
                self.height.saturating_sub(STATUS_H + 28) as i32,
                20,
                theme.text,
            );
        }
    }

    fn update(&mut self, event: Event) -> bool {
        let mut changed = self.accept_results();
        if self.apply_settled_zoom() {
            changed = true;
        }
        self.refresh_slider();
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

        // Dropdown menu swallows clicks while open.
        if self.dropdown_open {
            if let Event::Click { x, y } = event {
                for (index, preset) in ZoomPreset::ALL.iter().enumerate() {
                    if self
                        .dropdown_item_rect(index)
                        .contains(sunlight_ui::Point::new(x, y))
                    {
                        self.select_zoom_preset(*preset);
                        return true;
                    }
                }
                self.dropdown_open = false;
                return true;
            }
        }

        // Slider drag state (continuous, debounced zoom).
        let was_dragging = self.slider.dragging;
        if self.slider.update(event) {
            let percent = self.slider.value;
            let mode = ZoomMode::Fixed(scale_from_percent(percent));
            self.mode = mode;
            self.zoom_percent = percent;
            self.schedule_zoom(mode);
            changed = true;
        }
        if self.slider.dragging || was_dragging {
            return changed;
        }

        match event {
            Event::Click { x, y } => {
                let point = sunlight_ui::Point::new(x, y);
                if self.prev_rect().contains(point) {
                    if self.can_prev() {
                        self.navigate(-1);
                    }
                } else if self.next_rect().contains(point) {
                    if self.can_next() {
                        self.navigate(1);
                    }
                } else if self.dropdown_rect().contains(point) {
                    self.dropdown_open = true;
                } else if self.minus_rect().contains(point) {
                    self.nudge_zoom(-1);
                } else if self.plus_rect().contains(point) {
                    self.nudge_zoom(1);
                } else if self.slider_rect().contains(point) {
                    // A click inside the slider handled by Slider::update above.
                }
                changed = true;
            }
            Event::KeyPress {
                keycode: 0x4d | 0x51,
                pressed: true,
                ..
            } if self.total_pages > 0 => {
                self.navigate(1);
                changed = true;
            }
            Event::KeyPress {
                keycode: 0x4b | 0x49,
                pressed: true,
                ..
            } if self.total_pages > 0 => {
                self.navigate(-1);
                changed = true;
            }
            Event::Key('+') | Event::Key('=') => {
                self.nudge_zoom(1);
                changed = true;
            }
            Event::Key('-') => {
                self.nudge_zoom(-1);
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
        // Fit modes must recompute against the new viewport.
        self.request_render();
        true
    }
}

impl DocumentApp {
    fn draw_toolbar(&self, canvas: &mut Canvas, theme: &Theme) {
        let bar = self.toolbar_rect();
        canvas.fill_rect(bar, theme.panel);
        canvas.hbar(bar.x, bar.bottom() - 1, bar.w, 1, theme.border);

        IconButton::new(self.prev_rect(), UiSymbol::Back)
            .with_state(if self.can_prev() {
                ButtonState::Normal
            } else {
                ButtonState::Disabled
            })
            .draw(canvas, theme);
        IconButton::new(self.next_rect(), UiSymbol::Forward)
            .with_state(if self.can_next() {
                ButtonState::Normal
            } else {
                ButtonState::Disabled
            })
            .draw(canvas, theme);

        let page_text = if self.total_pages == 0 {
            "– / –".to_string()
        } else {
            format!("{} / {}", self.shown_page + 1, self.total_pages)
        };
        let page_rect = self.page_label_rect();
        canvas.fill_rounded_rect_with_border(page_rect, 8, theme.panel_alt, theme.border, 1);
        FONT_UI_SMALL.draw_vcenter(
            canvas,
            "PAGE",
            page_rect.x + 10,
            page_rect.y,
            page_rect.h,
            theme.text_dim,
        );
        FONT_UI_MEDIUM.draw_vcenter(
            canvas,
            &page_text,
            page_rect.x + 54,
            page_rect.y,
            page_rect.h,
            theme.text,
        );

        let zoom = Section::new(
            self.dropdown_rect(),
            mode_label(self.mode, self.slider_percent()),
        )
        .with_font(&FONT_UI_MEDIUM);
        zoom.draw(canvas, theme);
        let zoom_rect = self.dropdown_rect();
        FONT_UI_SMALL.draw_vcenter(
            canvas,
            "v",
            zoom_rect.right() - 20,
            zoom_rect.y,
            zoom_rect.h,
            theme.icon_muted,
        );
    }

    fn draw_status(&self, canvas: &mut Canvas, theme: &Theme) {
        let bar = self.status_rect();
        canvas.fill_rect(bar, theme.panel_alt);
        canvas.hbar(bar.x, bar.y, bar.w, 1, theme.border);

        IconButton::new(self.minus_rect(), UiSymbol::Minus).draw(canvas, theme);
        self.slider.draw(canvas, theme);
        IconButton::new(self.plus_rect(), UiSymbol::Plus).draw(canvas, theme);

        let percent_text = format!("{}%", self.slider_percent());
        let label_x = self.plus_rect().right() + 8;
        FONT_UI_MEDIUM.draw_vcenter(canvas, &percent_text, label_x, bar.y, bar.h, theme.text);
        let page_text = format!("Page {} / {}", self.shown_page + 1, self.total_pages.max(1));
        let page_width = FONT_UI_SMALL.measure_w(&page_text);
        let page_x = bar.right() - page_width as i32 - 14;
        FONT_UI_SMALL.draw_vcenter(canvas, &page_text, page_x, bar.y, bar.h, theme.text_dim);
    }

    fn draw_dropdown(&self, canvas: &mut Canvas, theme: &Theme) {
        for (index, preset) in ZoomPreset::ALL.iter().enumerate() {
            let rect = self.dropdown_item_rect(index);
            let active = preset.matches(self.mode);
            let section = Section::new(rect, preset.label())
                .with_state(if active {
                    SectionState::Active
                } else {
                    SectionState::Normal
                })
                .with_font(&FONT_UI_MEDIUM);
            section.draw(canvas, theme);
        }
    }
}

impl Drop for DocumentApp {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}
