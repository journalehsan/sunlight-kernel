#![cfg_attr(feature = "dom", allow(dead_code, unused_imports))]

extern crate alloc;

mod browser_ui;
#[cfg(feature = "dom")]
mod shield_ui;
#[cfg(all(test, feature = "dom"))]
mod browser_ui_tests;
#[cfg(all(test, feature = "dom"))]
mod shield_tests;
#[cfg(feature = "js-dom-fixture")]
mod shield_probe;

use alloc::{borrow::Cow, collections::BTreeMap, format, string::String, vec::Vec};
// Heap diagnostics use a stack buffer to avoid allocating while logging.
use core::fmt::Write as _;
use tokio::{runtime::Handle, sync::oneshot};

#[cfg(feature = "dom")]
use rappid_rabbit::images::{decode_image, ImageCache, MAX_IMAGE_RESPONSE_BYTES};
#[cfg(feature = "dom")]
use rappid_rabbit::render::DocumentRenderState;
use rappid_rabbit::{
    body_is_probably_text, build_get_request,
    developer_tools::{
        console::{ConsoleSeverity, ConsoleSource},
        dom_inspector::{DomInspectorPane, StylesMode},
        network::NetworkPaneFocus,
        panel::{DeveloperPanelLayout, DeveloperPanelState, MIN_MAIN_CONTENT_H},
        state::DeveloperToolsState,
        tabs::DeveloperToolTab,
    },
    document_lifecycle::DocumentLifecycle,
    form::{FormControlKind, FormControlState, FormState},
    format_url, normalize_url_input,
    resources::{
        discovery::{ResourceCandidate, ResourceQueue},
        request::RequestState,
    },
};

#[cfg(feature = "dom")]
use golden_fish::{parse_html_with_limits, ParseLimits};
#[cfg(feature = "dom")]
use rappid_rabbit::css::{
    collect_embedded_stylesheets, import_media_active, order_document_stylesheets,
    parse_leading_imports, parse_stylesheet, StyleContext, Stylesheet, StylesheetSource,
    MAX_IMPORTED_STYLESHEETS, MAX_IMPORT_DEPTH, MAX_STYLESHEET_BYTES, MAX_TOTAL_CSS_RULES,
    MAX_TOTAL_STYLESHEET_BYTES,
};
#[cfg(feature = "dom")]
use rappid_rabbit::resources::request::{ResourcePriority, ResourceType};
use sun_font::{FontRole, VecFont};
use sunlight_fetch::backend::{perform_request, perform_request_limited, RequestResult};
use sunlight_fetch::FetchError;
use sunlight_http::{HttpRequest, ParsedUrl};
use sunlight_ipc::{
    debug_log,
    launch_trace::{self, LaunchSource, LaunchTrace},
};
use sunlight_ui::scroll::{draw_scrollbar, ScrollPolicy, ScrollState, SCROLLBAR_WIDTH};
use sunlight_ui::widgets::{
    Button, ButtonState, Column as TableColumn, DocumentCanvas, DocumentCanvasPresentation, Label,
    Panel, TabBar, Table, TextInput, TextView, TreeHitTarget, TreeView,
};
#[cfg(feature = "dom")]
use sunlight_ui::widgets::{
    DocumentFontFamily, DocumentNodeId, RenderInteraction, RenderObjectKind,
};
use sunlight_ui::{
    request_close, set_client_cursor, App, AxisSizing, Canvas, Color, Column, CursorShape, Event,
    LayoutBox, LayoutInvalidation, Point, Rect, Row, Size, Sizing, Theme, VecText, Window,
    WindowConfig, WindowDecoration, WindowEvent,
};

static F_UI: VecFont = VecFont(FontRole::UiRegular);
static F_SMALL: VecFont = VecFont(FontRole::UiSmall);
static F_MONO: VecFont = VecFont(FontRole::MonoRegular);
static F_LARGE: VecFont = VecFont(FontRole::UiLarge);
static F_SERIF: VecFont = VecFont(FontRole::SerifRegular);

#[cfg(feature = "dom")]
#[derive(Clone)]
struct CachedStylesheetResponse {
    final_url: String,
    body: Vec<u8>,
    request_id: u64,
}

#[cfg(feature = "dom")]
#[derive(Default)]
struct StylesheetLoadState {
    cache: BTreeMap<String, CachedStylesheetResponse>,
    stylesheet_count: usize,
    total_bytes: usize,
    total_rules: usize,
}

struct RabbitFonts;

impl rappid_rabbit::render::TextMeasurer for RabbitFonts {
    fn measure_width(&self, text: &str) -> u32 {
        F_UI.measure_w(text)
    }
    fn line_height(&self) -> u32 {
        VecText::line_height(&F_UI)
    }
    fn measure_width_for(&self, family: DocumentFontFamily, text: &str) -> u32 {
        match family {
            DocumentFontFamily::Serif => F_SERIF.measure_w(text),
            DocumentFontFamily::Monospace => F_MONO.measure_w(text),
            DocumentFontFamily::SansSerif => F_UI.measure_w(text),
        }
    }
    fn line_height_for(&self, family: DocumentFontFamily) -> u32 {
        match family {
            DocumentFontFamily::Serif => VecText::line_height(&F_SERIF),
            DocumentFontFamily::Monospace => VecText::line_height(&F_MONO),
            DocumentFontFamily::SansSerif => VecText::line_height(&F_UI),
        }
    }

    fn measure_width_for_size(
        &self,
        family: DocumentFontFamily,
        font_size: u32,
        text: &str,
    ) -> u32 {
        // This must mirror DocumentCanvas::draw_scene.  CSS headings request
        // sizes larger than the installed bitmap/vector faces, so measuring a
        // synthetic 24px advance while painting the 16px large face leaves a
        // visible gap after every word.
        match family {
            DocumentFontFamily::Serif => F_SERIF.measure_w(text),
            DocumentFontFamily::Monospace => F_MONO.measure_w(text),
            DocumentFontFamily::SansSerif if font_size >= 24 => F_LARGE.measure_w(text),
            DocumentFontFamily::SansSerif => F_UI.measure_w(text),
        }
    }
}

static RABBIT_FONTS: RabbitFonts = RabbitFonts;

const WIN_W: u32 = 1080;
const WIN_H: u32 = 720;
const TOP_BAR_H: u32 = 64;
const TAB_H: u32 = 26;
const NAV_W: u32 = 28;
const GUTTER: i32 = 12;
const URL_INPUT_CAP: usize = 512;
const URL_BAR_RADIUS: u32 = 8;
const URL_SITE_W: u32 = 26;
const URL_ACTION_W: u32 = 26;

const KEY_Q: u8 = 0x10;
const KEY_L: u8 = 0x26;
const KEY_F6: u8 = 0x40;
const KEY_UP: u8 = 0x48;
const KEY_DOWN: u8 = 0x50;
const KEY_LEFT: u8 = 0x4B;
const KEY_RIGHT: u8 = 0x4D;
const KEY_HOME: u8 = 0x47;
const KEY_END: u8 = 0x4F;
const KEY_PGUP: u8 = 0x49;
const KEY_PGDN: u8 = 0x51;
const NETWORK_LIST_MIN_W: u32 = 320;
const NETWORK_DETAIL_MIN_W: u32 = 320;

const HEAP_SIZE: usize = 128 * 1024 * 1024;

fn heap_used() -> usize {
    #[cfg(target_os = "sunlightos")]
    {
        sunlight_libc::alloc::heap_stats().allocated_backing_bytes
    }
    #[cfg(not(target_os = "sunlightos"))]
    {
        0
    }
}

/// Stack-buffer writer for fatal-handler messages. Implements `core::fmt::Write` over a
/// fixed byte array and never allocates, so it is safe to call from the allocation-error
/// path (where `format!`/`String` would themselves fail). Truncates on overflow; `as_str`
/// falls back if a truncated multibyte sequence left the buffer non-UTF-8.
struct LogBuf<const N: usize> {
    buf: [u8; N],
    len: usize,
}

impl<const N: usize> LogBuf<N> {
    fn new() -> Self {
        Self {
            buf: [0; N],
            len: 0,
        }
    }

    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("[rabbit] (log buffer)")
    }
}

impl<const N: usize> core::fmt::Write for LogBuf<N> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let bytes = s.as_bytes();
        let remaining = self.buf.len().saturating_sub(self.len);
        let take = bytes.len().min(remaining);
        if take > 0 {
            self.buf[self.len..self.len + take].copy_from_slice(&bytes[..take]);
            self.len += take;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FocusPane {
    Source,
    Render,
    DeveloperTools,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DocumentView {
    Source,
    Render,
}

enum SourceContent {
    Placeholder(&'static str),
    Utf8(Vec<u8>),
    Text(String),
    Message(String),
}

impl SourceContent {
    fn as_str(&self) -> &str {
        match self {
            Self::Placeholder(text) => text,
            Self::Utf8(bytes) => core::str::from_utf8(bytes).unwrap_or(""),
            Self::Text(text) | Self::Message(text) => text.as_str(),
        }
    }
}

struct NavigationOutput {
    cert_info: Option<sunlight_cert::CertInfo>,
    connection_security: sunlight_cert::ConnectionSecurity,
    certificate_error: Option<String>,
    shield_revision: u64,
    final_url: String,
    status: String,
    source: SourceContent,
    render_status: String,
    #[cfg(feature = "dom")]
    render_state: Option<DocumentRenderState>,
    form_state: FormState,
    active_navigation_request_id: Option<u64>,
    developer_tools: DeveloperToolsState,
    discovered_resources: Vec<ResourceCandidate>,
    resource_queue: ResourceQueue,
    #[cfg(feature = "dom")]
    image_cache: ImageCache,
    document_lifecycle: DocumentLifecycle,
}

#[cfg(feature = "dom")]
struct ImageOutput {
    generation: u64,
    shield_revision: u64,
    cache: ImageCache,
    diagnostics: DeveloperToolsState,
}

/// One image job across navigation generations, including canceled blocking IO.
/// A guard prevents canceled generations accumulating queued image closures.
#[cfg(feature = "dom")]
struct ImageLaneGuard(alloc::sync::Arc<core::sync::atomic::AtomicBool>);
#[cfg(feature = "dom")]
impl Drop for ImageLaneGuard {
    fn drop(&mut self) {
        self.0.store(false, core::sync::atomic::Ordering::Release);
    }
}

#[derive(Default)]
struct PresentationMetrics {
    started: Option<std::time::Instant>,
    initial_us: Option<u128>,
    first_image_us: Option<u128>,
    visible_complete_us: Option<u128>,
    page_ready_us: Option<u128>,
    visible_view: Option<(u64, Rect)>,
    repaints: usize,
    cache_hits: usize,
    cache_misses: usize,
}

#[cfg(feature = "dom")]
struct ReflowOutput {
    version: u64,
    state: DocumentRenderState,
}

struct RabbitApp {
    shield: golden_fish::shield::FilterListManager,
    shield_stats: alloc::sync::Arc<rappid_rabbit::content_shield::ShieldStats>,
    certificate_panel: sunlight_ui::widgets::CertificatePanel<'static>,
    cert_info: Option<sunlight_cert::CertInfo>,
    connection_security: sunlight_cert::ConnectionSecurity,
    certificate_error: Option<String>,
    shield_settings_open: bool,
    shield_persistence_status: String,
    shield_reflow_pending: bool,
    #[cfg(feature = "dom")]
    image_completion: Option<oneshot::Receiver<ImageOutput>>,
    #[cfg(feature = "dom")]
    image_runtime: Option<Handle>,
    #[cfg(feature = "dom")]
    image_lane: alloc::sync::Arc<core::sync::atomic::AtomicBool>,
    #[cfg(feature = "dom")]
    image_attempts: Vec<String>,
    #[cfg(feature = "dom")]
    image_discovery_revision: u64,
    #[cfg(feature = "dom")]
    presentation: rappid_rabbit::presentation::ImagePresentation,
    metrics: PresentationMetrics,
    #[cfg(test)]
    image_fetcher: fn(ParsedUrl, HttpRequest, usize) -> Result<RequestResult, FetchError>,
    #[cfg(feature = "dom")]
    reflow_completion: Option<oneshot::Receiver<ReflowOutput>>,
    #[cfg(feature = "dom")]
    viewport_version: u64,
    #[cfg(feature = "dom")]
    latest_viewport: Size,
    navigation_url: String,
    runtime: Option<Handle>,
    completion: Option<oneshot::Receiver<NavigationOutput>>,
    url_input: TextInput<'static, URL_INPUT_CAP>,
    url_hovered: bool,
    url_action_hovered: bool,
    status: String,
    pending_fetch: bool,
    focus: FocusPane,
    source_scroll: usize,
    source: SourceContent,
    document_view: DocumentView,
    document_scroll: ScrollState,
    suppress_scroll_click: bool,
    pointer: Option<Point>,
    cursor: CursorShape,
    navigation_draining: bool,
    tab_title: String,
    title_revision: Option<(u64, u64)>,
    render_status: String,
    #[cfg(feature = "dom")]
    render_state: Option<DocumentRenderState>,
    form_state: FormState,
    active_navigation_request_id: Option<u64>,
    developer_tools: DeveloperToolsState,
    discovered_resources: Vec<ResourceCandidate>,
    resource_queue: ResourceQueue,
    #[cfg(feature = "dom")]
    image_cache: ImageCache,
    document_lifecycle: DocumentLifecycle,
    client_bounds: Rect,
    layout_invalidation: LayoutInvalidation,
    layout: RabbitLayout,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct RabbitLayout {
    root: Rect,
    top_bar: Rect,
    content: Rect,
    tab: Rect,
    back: Rect,
    forward: Rect,
    url_bar: Rect,
    view: Rect,
    devtools: Rect,
    shield: Rect,
}

impl RabbitApp {
    fn new(initial_url: Option<&str>) -> Self {
        let mut url_input = TextInput::new(Rect::default())
            .with_font(&F_UI)
            .with_clipboard_source(b"rappid-rabbit")
            .with_placeholder("Search or enter address...");
        url_input.set_text("rappid:startpage");

        let mut developer_tools = DeveloperToolsState::default();
        developer_tools.panel.close();
        developer_tools.console.push(
            ConsoleSeverity::Quiet,
            ConsoleSource::Browser,
            "Developer tools panel ready.",
        );
        developer_tools.console.push(
            ConsoleSeverity::Quiet,
            ConsoleSource::Browser,
            "DocumentCanvas configured for read-only Render mode.",
        );

        #[cfg(feature = "dom")]
        developer_tools.console.push(
            ConsoleSeverity::Quiet,
            ConsoleSource::Browser,
            "Layout engine: Modern (Two-Pass Arena)",
        );

        let mut app = Self {
            shield: rappid_rabbit::content_shield::bundled_manager(),
            shield_stats: Default::default(),
            certificate_panel: sunlight_ui::widgets::CertificatePanel::default().with_font(&F_UI),
            cert_info: None,
            connection_security: Default::default(),
            certificate_error: None,
            shield_settings_open: false,
            shield_persistence_status: String::new(),
            shield_reflow_pending: false,
            #[cfg(feature = "dom")]
            image_completion: None,
            #[cfg(feature = "dom")]
            image_runtime: None,
            #[cfg(feature = "dom")]
            image_lane: alloc::sync::Arc::new(core::sync::atomic::AtomicBool::new(false)),
            #[cfg(feature = "dom")]
            image_attempts: Vec::new(),
            #[cfg(feature = "dom")]
            image_discovery_revision: 0,
            #[cfg(feature = "dom")]
            presentation: Default::default(),
            metrics: PresentationMetrics::default(),
            #[cfg(test)]
            image_fetcher: perform_request_limited,
            #[cfg(feature = "dom")]
            reflow_completion: None,
            #[cfg(feature = "dom")]
            viewport_version: 0,
            #[cfg(feature = "dom")]
            latest_viewport: Size::new(0, 0),
            navigation_url: String::new(),
            runtime: None,
            completion: None,
            url_input,
            url_hovered: false,
            url_action_hovered: false,
            status: String::from("Idle"),
            pending_fetch: false,
            focus: FocusPane::Render,
            source_scroll: 0,
            source: SourceContent::Placeholder("Enter a URL and press Enter to open a page."),
            document_view: DocumentView::Render,
            document_scroll: ScrollState::new(),
            suppress_scroll_click: false,
            pointer: None,
            cursor: CursorShape::Pointer,
            navigation_draining: false,
            tab_title: String::from("New page"),
            title_revision: None,
            render_status: String::from("Enter a URL and press Enter."),
            #[cfg(feature = "dom")]
            render_state: None,
            form_state: FormState::new(),
            active_navigation_request_id: None,
            developer_tools,
            discovered_resources: Vec::new(),
            resource_queue: ResourceQueue::default(),
            #[cfg(feature = "dom")]
            image_cache: ImageCache::default(),
            document_lifecycle: DocumentLifecycle::default(),
            client_bounds: Rect::new(0, 0, WIN_W, WIN_H),
            layout_invalidation: LayoutInvalidation::new(),
            layout: RabbitLayout::default(),
        };
        let _ = app.ensure_layout();
        if let Some(url) = initial_url {
            app.url_input.set_text(url);
            app.queue_fetch();
        }
        app
    }

    fn queue_fetch(&mut self) {
        if self.pending_fetch || self.navigation_draining {
            return;
        }
        self.cert_info = None;
        self.connection_security = sunlight_cert::ConnectionSecurity::Unknown;
        self.certificate_error = None;
        self.certificate_panel.open = false;
        self.shield_stats.cancelled.store(true, core::sync::atomic::Ordering::Release);
        self.shield_stats = Default::default();
        // Navigation is an explicit request to show a new page. Completion
        // never changes this choice, so Source/DevTools opened while loading
        // retain focus on success and failure alike.
        self.document_view = DocumentView::Render;
        if self.focus != FocusPane::DeveloperTools {
            self.focus = FocusPane::Render;
        }
        // Drop the retained scene before the worker starts. Without this,
        // the previous document (often the start page) remains painted while
        // a new navigation is pending and looks like it is still loading.
        self.source = SourceContent::Placeholder("Loading...");
        #[cfg(feature = "dom")]
        {
            self.render_state = None;
            self.resource_queue.clear();
            self.discovered_resources.clear();
        }
        self.metrics = PresentationMetrics {
            started: Some(std::time::Instant::now()),
            ..Default::default()
        };
        self.document_scroll.end_drag();
        self.suppress_scroll_click = false;
        self.title_revision = None;
        self.pending_fetch = true;
        self.status = String::from("Fetching...");
        self.render_status = String::from("Loading rendered document...");
        self.developer_tools.console.push(
            ConsoleSeverity::Quiet,
            ConsoleSource::Browser,
            format!("Navigation queued for {}", self.url_input.value().trim()),
        );
    }

    // A single in-flight navigation bounds source/DOM scratch memory. The
    // blocking pool performs DNS, TLS, subresources, parsing and initial layout.
    fn perform_pending_fetch(&mut self) -> bool {
        if let Some(receiver) = self.completion.as_mut() {
            match receiver.try_recv() {
                Ok(output) => {
                    self.completion = None;
                    self.pending_fetch = false;
                    if output.shield_revision != self.shield.revision {
                        self.set_navigation_error("Content Shield changed; reload the page");
                        return true;
                    }
                    if output.document_lifecycle.generation()
                        != self.document_lifecycle.generation()
                    {
                        return true;
                    }
                    let panel = self.developer_tools.panel.clone();
                    if self.url_input.value().trim() == self.navigation_url.as_str() {
                        self.url_input.set_text(&output.final_url);
                    }
                    self.cert_info = output.cert_info;
                    self.connection_security = output.connection_security;
                    self.certificate_error = output.certificate_error;
                    self.status = output.status;
                    self.source = output.source;
                    self.render_status = output.render_status;
                    #[cfg(feature = "dom")]
                    {
                        self.render_state = output.render_state;
                    }
                    self.form_state = output.form_state;
                    self.active_navigation_request_id = output.active_navigation_request_id;
                    self.developer_tools = output.developer_tools;
                    self.discovered_resources = output.discovered_resources;
                    self.resource_queue = output.resource_queue;
                    #[cfg(feature = "dom")]
                    {
                        self.image_cache = output.image_cache;
                    }
                    self.document_lifecycle = output.document_lifecycle;
                    self.developer_tools.panel = panel;
                    self.source_scroll = 0;
                    self.document_scroll.offset_y = 0;
                    #[cfg(feature = "dom")]
                    self.refresh_render_viewport();
                }
                Err(oneshot::error::TryRecvError::Empty) => return false,
                Err(oneshot::error::TryRecvError::Closed) => {
                    self.completion = None;
                    self.pending_fetch = false;
                    self.set_navigation_error("Navigation worker stopped");
                }
            }
            return true;
        }
        let Some(runtime) = self.runtime.as_ref() else {
            self.pending_fetch = false;
            self.set_navigation_error("Navigation runtime unavailable");
            return true;
        };
        let runtime = runtime.clone();
        let normalized = match normalize_url_input(self.url_input.value()) {
            Ok(url) => url,
            Err(error) => {
                self.pending_fetch = false;
                self.set_error(format!("{error}"));
                return true;
            }
        };
        let internal = rappid_rabbit::startpage::is_custom_url(&normalized);
        let parsed = if internal {
            None
        } else {
            match ParsedUrl::parse(&normalized) {
                Ok(url) => Some(url),
                Err(error) => {
                    self.pending_fetch = false;
                    self.set_error(format!("{error}"));
                    return true;
                }
            }
        };
        let normalized = rappid_rabbit::startpage::resolve_custom_url(&normalized)
            .map(String::from)
            .unwrap_or(normalized);
        self.url_input.set_text(&normalized);
        self.navigation_url = normalized.clone();
        if let Some(parsed) = parsed.as_ref() {
            self.begin_navigation_session(&normalized, &build_get_request(parsed));
        } else {
            self.begin_internal_navigation_session(&normalized);
        }
        // Capture the lifecycle after beginning this navigation. Capturing it
        // before the session starts leaves the worker at generation zero, so
        // its completed document is rejected as stale and the UI stays on the
        // loading placeholder forever.
        let lifecycle = self.document_lifecycle.clone();
        // Release the previous source before allocating the next response.
        self.source = SourceContent::Placeholder("Loading...");
        let bounds = self.client_bounds;
        let panel = self.developer_tools.panel.clone();
        let shield = self.shield.clone();
        let shield_stats = self.shield_stats.clone();
        let (sender, receiver) = oneshot::channel();
        self.completion = Some(receiver);
        runtime.spawn_blocking(move || {
            let mut worker = RabbitApp::new(None);
            worker.shield = shield;
            worker.shield_stats = shield_stats;
            worker.navigation_url = normalized.clone();
            worker.url_input.set_text(&normalized);
            worker.document_lifecycle = lifecycle;
            worker.developer_tools.panel = panel;
            worker.set_client_bounds(bounds.w, bounds.h);
            worker.perform_fetch_sync_existing_session();
            let output = worker.into_navigation_output();
            // A closed window drops the receiver and the completed document.
            let _ = sender.send(output);
        });
        true
    }

    fn into_navigation_output(self) -> NavigationOutput {
        NavigationOutput {
            cert_info: self.cert_info,
            connection_security: self.connection_security,
            certificate_error: self.certificate_error,
            shield_revision: self.shield.revision,
            final_url: String::from(self.url_input.value()),
            status: self.status,
            source: self.source,
            render_status: self.render_status,
            #[cfg(feature = "dom")]
            render_state: self.render_state,
            form_state: self.form_state,
            active_navigation_request_id: self.active_navigation_request_id,
            developer_tools: self.developer_tools,
            discovered_resources: self.discovered_resources,
            resource_queue: self.resource_queue,
            #[cfg(feature = "dom")]
            image_cache: self.image_cache,
            document_lifecycle: self.document_lifecycle,
        }
    }

    fn perform_fetch_sync(&mut self) {
        self.perform_fetch_sync_inner(true);
    }

    // The UI thread has already opened the navigation session before handing
    // work to the blocking pool. Starting another session here would advance
    // the document generation and cause the completed output to be discarded
    // as stale.
    fn perform_fetch_sync_existing_session(&mut self) {
        self.perform_fetch_sync_inner(false);
    }

    fn perform_fetch_sync_inner(&mut self, begin_session: bool) {
        self.pending_fetch = false;
        self.source_scroll = 0;

        let normalized = match normalize_url_input(self.url_input.value()) {
            Ok(url) => url,
            Err(err) => {
                self.set_error(format!("{err}"));
                return;
            }
        };
        self.url_input.set_text(&normalized);

        if rappid_rabbit::startpage::is_custom_url(&normalized) {
            let Some(canonical) = rappid_rabbit::startpage::resolve_custom_url(&normalized) else {
                self.status = String::from("Unknown internal page");
                self.render_status = format!("No internal page is registered for {normalized}.");
                self.source = SourceContent::Message(self.render_status.clone());
                self.active_navigation_request_id = None;
                #[cfg(feature = "dom")]
                {
                    self.render_state = None;
                }
                return;
            };
            self.url_input.set_text(canonical);
            self.navigation_url = String::from(canonical);
            let body = rappid_rabbit::startpage::source().as_bytes().to_vec();
            self.apply_result(
                canonical,
                RequestResult {
                    connection_security: sunlight_cert::ConnectionSecurity::NotSecure,
                    cert_info: None,
                    response: sunlight_http::HttpResponse {
                        status_code: 200,
                        status_text: String::from("OK"),
                        headers: vec![(
                            String::from("content-type"),
                            String::from("text/html; charset=utf-8"),
                        )],
                        header_len: 0,
                    },
                    body,
                    final_url: None,
                    duration_ms: Some(0),
                },
            );
            self.status = String::from("Internal page");
            return;
        }

        let parsed = match ParsedUrl::parse(&normalized) {
            Ok(url) => url,
            Err(err) => {
                self.set_error(format!("{err}"));
                return;
            }
        };

        let request = build_get_request(&parsed);
        if begin_session {
            self.begin_navigation_session(&normalized, &request);
        }
        match perform_request(parsed, request) {
            Ok(result) => self.apply_result(&normalized, result),
            Err(err) => self.apply_fetch_error(err),
        }
    }

    fn begin_navigation_session(&mut self, requested_url: &str, request: &HttpRequest) {
        let generation = self.reset_navigation_state();

        let request_id = self.developer_tools.network.begin_main_document_request(
            request.method,
            String::from(requested_url),
            request.headers.clone(),
        );
        self.active_navigation_request_id = Some(request_id);
        self.developer_tools
            .network
            .set_request_state(request_id, RequestState::Connecting);
        self.developer_tools.console.push(
            ConsoleSeverity::Quiet,
            ConsoleSource::Fetch,
            format!("Main document request started: {requested_url}"),
        );
        self.log_heap_stage("before fetch", generation, None);
    }

    fn begin_internal_navigation_session(&mut self, requested_url: &str) {
        let generation = self.reset_navigation_state();
        self.active_navigation_request_id = None;
        self.developer_tools.console.push(
            ConsoleSeverity::Quiet,
            ConsoleSource::Browser,
            format!("Opened internal page: {requested_url}"),
        );
        self.log_heap_stage("before internal page", generation, None);
    }

    fn reset_navigation_state(&mut self) -> u64 {
        self.cert_info = None;
        self.connection_security = sunlight_cert::ConnectionSecurity::Unknown;
        self.certificate_error = None;
        self.certificate_panel.open = false;
        self.shield_reflow_pending = false;
        let generation = self.document_lifecycle.begin_navigation();
        self.document_scroll = ScrollState::new();
        self.title_revision = None;
        self.discovered_resources.clear();
        self.resource_queue.clear();
        #[cfg(feature = "dom")]
        {
            self.image_cache = ImageCache::default();
            self.image_completion = None;
            self.image_attempts.clear();
            self.image_discovery_revision = 0;
            self.presentation = Default::default();
        }
        self.document_scroll.offset_y = 0;
        self.render_status = String::from("Loading rendered document...");
        #[cfg(feature = "dom")]
        {
            self.render_state = None;
            self.reflow_completion = None; // the bounded worker drops cancelled output
            self.viewport_version = self.viewport_version.wrapping_add(1);
            self.form_state = FormState::new();
        }
        self.developer_tools.network.clear_for_new_page();
        self.developer_tools
            .dom
            .clear_with_message("Waiting for parsed HTML document.");
        generation
    }

    fn apply_result(&mut self, requested_url: &str, result: RequestResult) {
        self.cert_info = result.cert_info.clone();
        self.connection_security = result.connection_security;
        self.certificate_error = None;
        let generation = self.document_lifecycle.generation();
        let status_code = result.response.status_code;
        let status_text = if result.response.status_text.is_empty() {
            String::from("(no reason phrase)")
        } else {
            result.response.status_text.clone()
        };
        let final_url_parsed = result.final_url.clone();
        let final_url = final_url_parsed
            .as_ref()
            .map(format_url)
            .unwrap_or_else(|| String::from(requested_url));
        self.navigation_url = final_url.clone();
        // Follow redirects only while the requested URL is still displayed.
        if self.url_input.value().trim() == requested_url {
            self.url_input.set_text(&final_url);
        }
        let content_type = result
            .response
            .header("content-type")
            .map_or("(missing)", |value| value);
        let body_size = result.body.len();
        trace_resource("html", &final_url, &result.body);
        self.log_heap_stage("after response completion", generation, Some(body_size));

        self.status = if rappid_rabbit::startpage::resolve_custom_url(&final_url).is_some() {
            String::from("Internal page")
        } else {
            format!("HTTP {status_code} {status_text}")
        };
        self.source = self.build_source_content(content_type, result.body);
        if let Some(request_id) = self.active_navigation_request_id {
            self.developer_tools
                .network
                .set_request_state(request_id, RequestState::Receiving);
            self.developer_tools.network.complete_request(
                request_id,
                Some(final_url.clone()),
                status_code,
                status_text.clone(),
                result.duration_ms,
                Some(body_size),
                Some(String::from(content_type)),
                result.response.headers.clone(),
                None,
                Some(false),
                Some(false),
            );
        }

        if rappid_rabbit::startpage::resolve_custom_url(&final_url).is_some() {
            self.developer_tools.console.push(
                ConsoleSeverity::Quiet,
                ConsoleSource::Browser,
                "Compiled internal document rendered without a network request.",
            );
        } else if (400..500).contains(&status_code) {
            self.developer_tools.console.push(
                ConsoleSeverity::Warn,
                ConsoleSource::Fetch,
                format!("Client error: HTTP {status_code} {status_text}"),
            );
        } else if status_code >= 500 {
            self.developer_tools.console.push(
                ConsoleSeverity::Error,
                ConsoleSource::Fetch,
                format!("Server error: HTTP {status_code} {status_text}"),
            );
        } else {
            self.developer_tools.console.push(
                ConsoleSeverity::Quiet,
                ConsoleSource::Fetch,
                format!("Request completed: HTTP {status_code} {status_text}"),
            );
        }

        #[cfg(feature = "dom")]
        {
            self.discovered_resources.clear();
            self.resource_queue.clear();
            let source_text = self.source.as_str();
            if rappid_rabbit::looks_like_html(Some(content_type), source_text) {
                if !self.document_lifecycle.start_parse(generation) {
                    return;
                }
                self.log_heap_stage("before DOM parsing", generation, Some(source_text.len()));
                let document_result = if rappid_rabbit::startpage::resolve_custom_url(&final_url)
                    .is_some()
                {
                    Ok(rappid_rabbit::startpage::document())
                } else {
                    parse_html_with_limits(source_text, ParseLimits::default())
                };
                match document_result {
                    Ok(document) => {
                        let stats = document.stats();
                        let embedded_stylesheets = collect_embedded_stylesheets(&document);
                        let mut linked_stylesheets = Vec::new();
                        if let Some(base_url) = final_url_parsed.as_ref() {
                            let candidates =
                                rappid_rabbit::resources::discovery::discover_resources(
                                    &document, base_url,
                                );
                            linked_stylesheets = self.fetch_external_stylesheets(&candidates);
                            self.discovered_resources = candidates;
                        }
                        let stylesheets = order_document_stylesheets(
                            &document,
                            embedded_stylesheets,
                            linked_stylesheets,
                        );
                        let viewport = self.render_viewport();
                        let layout_started = std::time::Instant::now();
                        let style_context =
                            StyleContext::build_with_viewport(&document, &stylesheets, viewport.w)
                                .with_shield(&document, Some(rappid_rabbit::content_shield::CosmeticPolicy::for_document(&self.shield, &final_url)));
                        let style_ms = layout_started.elapsed().as_millis();
                        if let Some(base_url) = final_url_parsed.as_ref() {
                            discover_background_images(
                                &mut self.discovered_resources,
                                &document,
                                &style_context,
                                &stylesheets,
                                base_url,
                            );
                            let candidates = core::mem::take(&mut self.discovered_resources);
                            self.resource_queue.replace_from_candidates(&candidates);
                            self.discovered_resources = candidates;
                        }
                        let layout_started = std::time::Instant::now();
                        self.document_scroll.offset_y = 0;
                        self.render_state = Some(DocumentRenderState::new_with_images(
                            generation,
                            final_url.clone(),
                            document,
                            style_context,
                            viewport,
                            &RABBIT_FONTS,
                            self.image_cache.clone(),
                        ));
                        if let Some(render_state) = self.render_state.as_ref() {
                            self.form_state.build_from_dom(&render_state.dom);
                        }
                        self.patch_all_controls();
                        debug_log(&format!("[RABBIT][LAYOUT] initial gen={} viewport={}x{} style_ms={} layout_ms={} worker=yes", generation, viewport.w, viewport.h, style_ms, layout_started.elapsed().as_millis()));
                        self.log_render_scene("initial scene");
                        if TRACE_DOCUMENT.load(core::sync::atomic::Ordering::Relaxed) {
                            trace_layout(self.render_state.as_ref().unwrap());
                        }
                        self.developer_tools.dom.set_document_with_styles(
                            self.render_state.as_ref().unwrap().dom.clone(),
                            self.render_state.as_ref().unwrap().styles.clone(),
                        );
                        let visible_rows = self.developer_tools.dom.visible_row_count();
                        let _ = self.document_lifecycle.finish_ready(generation);
                        self.log_dom_result(
                            generation,
                            stats.node_count,
                            stats.max_depth,
                            stats.total_text_bytes,
                            visible_rows,
                        );
                        self.developer_tools.console.push(
                            ConsoleSeverity::Quiet,
                            ConsoleSource::Parser,
                            format!(
                                "Golden Fish parsed HTML document ({} nodes).",
                                stats.node_count
                            ),
                        );
                    }
                    Err(err) => {
                        let error = format!("{err}");
                        let _ = self
                            .document_lifecycle
                            .finish_failed(generation, error.clone());
                        self.developer_tools
                            .dom
                            .clear_with_message(format!("Parser error: {error}"));
                        self.render_state = None;
                        self.render_status = format!("Render failed: {error}");
                        self.log_heap_stage(
                            "after DOM parsing",
                            generation,
                            Some(source_text.len()),
                        );
                        self.developer_tools.console.push(
                            ConsoleSeverity::Warn,
                            ConsoleSource::Parser,
                            format!("Golden Fish parse error: {error}"),
                        );
                    }
                }
            } else {
                self.developer_tools
                    .dom
                    .clear_with_message("Current response is not HTML.");
                self.render_state = None;
                self.render_status = String::from("This response is not an HTML document.");
            }
        }

        #[cfg(not(feature = "dom"))]
        {
            self.developer_tools
                .dom
                .clear_with_message("Golden Fish DOM inspector is unavailable in this build.");
        }
    }

    #[cfg(feature = "dom")]
    fn fetch_external_stylesheets(
        &mut self,
        candidates: &[ResourceCandidate],
    ) -> Vec<Option<Vec<Stylesheet>>> {
        let mut stylesheets = Vec::new();
        let mut state = StylesheetLoadState::default();
        for candidate in candidates {
            if candidate.resource_type != ResourceType::Stylesheet {
                continue;
            }
            let mut stack = Vec::new();
            stylesheets.push(self.fetch_stylesheet_tree(
                &candidate.resolved_url,
                None,
                0,
                &mut stack,
                &mut state,
            ));
        }
        stylesheets
    }

    #[cfg(feature = "dom")]
    fn fetch_stylesheet_tree(
        &mut self,
        request_url: &str,
        parent_url: Option<&str>,
        depth: usize,
        stack: &mut Vec<String>,
        state: &mut StylesheetLoadState,
    ) -> Option<Vec<Stylesheet>> {
        if depth > MAX_IMPORT_DEPTH
            || state.stylesheet_count >= MAX_IMPORTED_STYLESHEETS
            || state.total_bytes >= MAX_TOTAL_STYLESHEET_BYTES
            || state.total_rules >= MAX_TOTAL_CSS_RULES
        {
            self.developer_tools.console.push(
                ConsoleSeverity::Warn,
                ConsoleSource::Fetch,
                format!("Stylesheet import limit reached: {request_url} (depth={depth})"),
            );
            return None;
        }
        if stack.iter().any(|url| url == request_url) {
            self.developer_tools.console.push(
                ConsoleSeverity::Warn,
                ConsoleSource::Fetch,
                format!("Stylesheet import cycle ignored: {request_url}"),
            );
            return Some(Vec::new());
        }

        let response = if let Some(cached) = state.cache.get(request_url) {
            self.developer_tools.console.push(
                ConsoleSeverity::Quiet,
                ConsoleSource::Fetch,
                format!("Stylesheet cache hit: {request_url}"),
            );
            cached.clone()
        } else {
            let parsed_url = match ParsedUrl::parse(request_url) {
                Ok(url) => url,
                Err(error) => {
                    self.developer_tools.console.push(
                        ConsoleSeverity::Warn,
                        ConsoleSource::Fetch,
                        format!("Stylesheet skipped ({request_url}): {error}"),
                    );
                    return None;
                }
            };
            let request = build_get_request(&parsed_url);
            let request_id = self.developer_tools.network.begin_request(
                request.method,
                String::from(request_url),
                request.headers.clone(),
                ResourceType::Stylesheet,
                ResourcePriority::RenderCritical,
                RequestState::Queued,
            );
            self.developer_tools.network.set_stylesheet_metadata(
                request_id,
                parent_url.map(String::from),
                depth,
                None,
                None,
            );
            self.developer_tools
                .network
                .set_request_state(request_id, RequestState::Connecting);
            let document = self.navigation_url.clone();
            let mut policy = |url: &ParsedUrl| rappid_rabbit::content_shield::check_request(
                &self.shield, &self.shield_stats, url, &document,
                golden_fish::shield::ResourceType::Stylesheet);
            let result = match sunlight_fetch::backend::perform_request_filtered(parsed_url, request, MAX_STYLESHEET_BYTES, &mut policy) {
                Ok(result) => result,
                Err(error) => {
                    self.developer_tools.network.fail_request(
                        request_id,
                        None,
                        None,
                        None,
                        format!("Stylesheet request failed: {error}"),
                    );
                    return None;
                }
            };
            let status_code = result.response.status_code;
            let status_text = if result.response.status_text.is_empty() {
                String::from("(no reason phrase)")
            } else {
                result.response.status_text.clone()
            };
            let final_url = result
                .final_url
                .as_ref()
                .map(format_url)
                .unwrap_or_else(|| String::from(request_url));
            let body_size = result.body.len();
            self.developer_tools.network.complete_request(
                request_id,
                Some(final_url.clone()),
                status_code,
                status_text.clone(),
                result.duration_ms,
                Some(body_size),
                result.response.header("content-type").map(String::from),
                result.response.headers.clone(),
                None,
                Some(false),
                Some(false),
            );
            if !(200..400).contains(&status_code)
                || body_size > MAX_STYLESHEET_BYTES
                || state.total_bytes.saturating_add(body_size) > MAX_TOTAL_STYLESHEET_BYTES
            {
                self.developer_tools.console.push(
                    ConsoleSeverity::Warn,
                    ConsoleSource::Fetch,
                    format!("Stylesheet ignored: HTTP {status_code} {status_text}, {body_size} bytes ({request_url})"),
                );
                return None;
            }
            let response = CachedStylesheetResponse {
                final_url,
                body: result.body,
                request_id,
            };
            state.total_bytes = state.total_bytes.saturating_add(response.body.len());
            state
                .cache
                .insert(String::from(request_url), response.clone());
            state
                .cache
                .insert(response.final_url.clone(), response.clone());
            response
        };

        trace_resource("css", &response.final_url, &response.body);
        let css = match core::str::from_utf8(&response.body) {
            Ok(css) => css,
            Err(_) => {
                self.developer_tools.network.set_stylesheet_metadata(
                    response.request_id,
                    parent_url.map(String::from),
                    depth,
                    Some(false),
                    Some(0),
                );
                self.developer_tools.console.push(
                    ConsoleSeverity::Warn,
                    ConsoleSource::Fetch,
                    format!("Stylesheet ignored: non-UTF-8 response ({request_url})"),
                );
                return None;
            }
        };
        if stack.iter().any(|url| url == &response.final_url) {
            self.developer_tools.console.push(
                ConsoleSeverity::Warn,
                ConsoleSource::Fetch,
                format!(
                    "Stylesheet import cycle ignored after redirect: {}",
                    response.final_url
                ),
            );
            return Some(Vec::new());
        }
        state.stylesheet_count = state.stylesheet_count.saturating_add(1);
        stack.push(response.final_url.clone());
        let mut sheets = Vec::new();
        let viewport_width = self.render_viewport().w;
        for import in parse_leading_imports(css) {
            let (active, unsupported) = import_media_active(&import.media, viewport_width);
            if let Some(reason) = unsupported {
                self.developer_tools.console.push(
                    ConsoleSeverity::Warn,
                    ConsoleSource::Parser,
                    format!(
                        "@import media at {}:{}: {reason}",
                        import.location.line, import.location.column
                    ),
                );
            }
            if !active {
                self.developer_tools.console.push(
                    ConsoleSeverity::Quiet,
                    ConsoleSource::Parser,
                    format!(
                        "Inactive @import '{}' media='{}' viewport={}px",
                        import.raw_url, import.media, viewport_width
                    ),
                );
                continue;
            }
            let base = match ParsedUrl::parse(&response.final_url) {
                Ok(base) => base,
                Err(_) => continue,
            };
            let resolved =
                match rappid_rabbit::resources::discovery::resolve_url(&base, &import.raw_url) {
                    Ok(url) => url,
                    Err(error) => {
                        self.developer_tools.console.push(
                            ConsoleSeverity::Warn,
                            ConsoleSource::Fetch,
                            format!(
                                "Invalid stylesheet import '{}' from {}: {error:?}",
                                import.raw_url, response.final_url
                            ),
                        );
                        continue;
                    }
                };
            self.developer_tools.console.push(
                ConsoleSeverity::Quiet,
                ConsoleSource::Fetch,
                format!(
                    "Stylesheet import scheduled: {resolved} parent={} depth={}",
                    parent_url.unwrap_or(&response.final_url),
                    depth + 1
                ),
            );
            if let Some(imported) = self.fetch_stylesheet_tree(
                &resolved,
                Some(&response.final_url),
                depth + 1,
                stack,
                state,
            ) {
                sheets.extend(imported);
            }
        }
        stack.pop();
        let sheet = parse_stylesheet(css, StylesheetSource::External(response.final_url.clone()));
        if state.total_rules.saturating_add(sheet.rules.len()) > MAX_TOTAL_CSS_RULES {
            self.developer_tools.console.push(
                ConsoleSeverity::Warn,
                ConsoleSource::Parser,
                format!(
                    "Stylesheet rule limit reached before {}",
                    response.final_url
                ),
            );
            return Some(sheets);
        }
        state.total_rules = state.total_rules.saturating_add(sheet.rules.len());
        self.developer_tools.network.set_stylesheet_metadata(
            response.request_id,
            parent_url.map(String::from),
            depth,
            Some(true),
            Some(sheet.rules.len()),
        );
        self.developer_tools.console.push(
            ConsoleSeverity::Quiet,
            ConsoleSource::Parser,
            format!(
                "Stylesheet parsed: {} rules={} depth={depth}",
                response.final_url,
                sheet.rules.len()
            ),
        );
        sheets.push(sheet);
        Some(sheets)
    }

    #[cfg(feature = "dom")]
    fn fetch_images(
        &mut self,
        candidates: &[ResourceCandidate],
        completion: &oneshot::Sender<ImageOutput>,
    ) {
        use rappid_rabbit::images::{detect_format, ImageFormat};
        for candidate in rappid_rabbit::images::initial_image_candidates(candidates) {
            if completion.is_closed() {
                return;
            }
            let url = &candidate.resolved_url;
            if self.image_cache.get(url).is_some() {
                continue;
            }
            self.log_image("fetch", url, "unknown", None, "started");
            let parsed = match ParsedUrl::parse(url) {
                Ok(url) => url,
                Err(error) => {
                    self.log_image("fetch", url, "unknown", None, &format!("failed: {error}"));
                    self.image_cache
                        .insert_failed(url.clone(), format!("invalid URL: {error}"));
                    continue;
                }
            };
            let request = build_get_request(&parsed);
            let request_id = self.developer_tools.network.begin_request(
                request.method,
                url.clone(),
                request.headers.clone(),
                ResourceType::Image,
                ResourcePriority::Embedded,
                RequestState::Connecting,
            );
            #[cfg(test)]
            let fetcher = self.image_fetcher;
            let document = self.navigation_url.clone();
            let mut policy = |url: &ParsedUrl| rappid_rabbit::content_shield::check_request(
                &self.shield, &self.shield_stats, url, &document,
                golden_fish::shield::ResourceType::Image);
            #[cfg(test)]
            let result = policy(&parsed).and_then(|()|fetcher(parsed, request, MAX_IMAGE_RESPONSE_BYTES));
            #[cfg(not(test))]
            let result = sunlight_fetch::backend::perform_request_filtered(parsed, request, MAX_IMAGE_RESPONSE_BYTES, &mut policy);
            match result {
                Ok(result) => {
                    if completion.is_closed() {
                        return;
                    }
                    let status = result.response.status_code;
                    let content_type = result.response.header("content-type").map(String::from);
                    let final_url = result.final_url.as_ref().map(format_url);
                    let format = detect_format(
                        &result.body,
                        content_type.as_deref(),
                        final_url.as_deref().unwrap_or(url),
                    );
                    let dimensions =
                        sunlight_ui::image::inspect_image_dimensions(&result.body).ok();
                    self.log_image(
                        "response",
                        url,
                        format.label(),
                        dimensions,
                        &format!(
                            "status={status} bytes={} type={}",
                            result.body.len(),
                            content_type.as_deref().unwrap_or("missing")
                        ),
                    );
                    self.developer_tools.network.complete_request(
                        request_id,
                        final_url,
                        status,
                        result.response.status_text.clone(),
                        result.duration_ms,
                        Some(result.body.len()),
                        content_type.clone(),
                        result.response.headers.clone(),
                        None,
                        Some(false),
                        Some(false),
                    );
                    if !(200..300).contains(&status) {
                        let reason = format!("HTTP {status}");
                        self.log_image("http", url, format.label(), dimensions, &reason);
                        self.image_cache.insert_failed(url.clone(), reason);
                        continue;
                    }
                    #[cfg(target_os = "sunlightos")]
                    if let Some((width, height)) = dimensions {
                        // Progressive coefficients + raw RGB + conversion and
                        // allocator overhead. No concurrent decodes exist.
                        let reserve = (width as usize)
                            .checked_mul(height as usize)
                            .and_then(|n| n.checked_mul(12))
                            .and_then(|n| n.checked_add(4 * 1024 * 1024));
                        if reserve.is_none_or(|n| n > HEAP_SIZE.saturating_sub(heap_used())) {
                            let reason = "native decode working-set limit";
                            self.log_image(
                                "size_validation",
                                url,
                                format.label(),
                                dimensions,
                                reason,
                            );
                            self.image_cache.insert_failed(url.clone(), reason);
                            continue;
                        }
                    }
                    match decode_image(&result.body, content_type.as_deref(), url) {
                        Ok(decoded) => {
                            self.log_image(
                                "decode",
                                url,
                                format.label(),
                                Some((decoded.image.width, decoded.image.height)),
                                "ok",
                            );
                            self.image_cache.insert_decoded(url.clone(), decoded);
                            if self.image_cache.decoded(url).is_none() {
                                self.log_image(
                                    "scene_attachment",
                                    url,
                                    format.label(),
                                    dimensions,
                                    "cache rejected image",
                                );
                            }
                        }
                        Err(reason) => {
                            let stage = if format == ImageFormat::Unknown
                                || reason.contains("unavailable")
                            {
                                "unsupported_format"
                            } else if reason.contains("limit")
                                || reason.contains("overflow")
                                || reason.contains("LimitExceeded")
                            {
                                "size_validation"
                            } else {
                                "decode"
                            };
                            self.log_image(
                                stage,
                                url,
                                format.label(),
                                dimensions,
                                &format!("failed: {reason}"),
                            );
                            self.image_cache.insert_failed(url.clone(), reason);
                        }
                    }
                }
                Err(error) => {
                    let reason = format!("{error}");
                    self.developer_tools.network.fail_request(
                        request_id,
                        None,
                        None,
                        None,
                        reason.clone(),
                    );
                    self.log_image("fetch", url, "unknown", None, &format!("failed: {reason}"));
                    self.image_cache.insert_failed(url.clone(), reason);
                }
            }
        }
    }

    /// Resource jobs are bounded independently of presentation. Offscreen
    /// resources still reach terminal readiness under the existing contract.
    #[cfg(feature = "dom")]
    fn poll_images(&mut self) -> bool {
        let mut changed = false;
        if let Some(receiver) = self.image_completion.as_mut() {
            match receiver.try_recv() {
                Ok(output) => {
                    self.image_completion = None;
                    if output.generation != self.document_lifecycle.generation()
                        || output.shield_revision != self.shield.revision {
                        return false;
                    }
                    self.image_cache = output.cache;
                    if let Some(state) = self.render_state.as_mut() {
                        state.stage_images(output.generation, self.image_cache.clone());
                    }
                    self.developer_tools
                        .network
                        .append_completed(output.diagnostics.network.entries());
                    for entry in output.diagnostics.console.entries() {
                        self.developer_tools.console.push(
                            entry.severity,
                            entry.source,
                            entry.message.clone(),
                        );
                    }
                    changed = self.developer_tools.panel.open;
                }
                Err(oneshot::error::TryRecvError::Empty) => return false,
                Err(oneshot::error::TryRecvError::Closed) => {
                    self.image_completion = None;
                    // The URL was recorded before submission: never spin on
                    // a failed worker. Other bounded resources may proceed.
                    if let Some(url) = self.image_attempts.last() {
                        self.image_cache
                            .insert_failed(url.clone(), "Image worker stopped");
                        if let Some(state) = self.render_state.as_mut() {
                            state.stage_images(
                                self.document_lifecycle.generation(),
                                self.image_cache.clone(),
                            );
                        }
                    }
                    changed = self.developer_tools.panel.open;
                }
            }
        }
        if self.pending_fetch {
            return changed;
        }
        if let Some(state) = self.render_state.as_ref() {
            if self.image_discovery_revision != state.dom_revision {
                let Ok(base) = ParsedUrl::parse(&state.final_url) else {
                    return changed;
                };
                let mut candidates =
                    rappid_rabbit::resources::discovery::discover_resources(&state.dom, &base);
                discover_background_images(&mut candidates, &state.dom, &state.styles, &[], &base);
                self.discovered_resources = candidates;
                self.image_discovery_revision = state.dom_revision;
                self.metrics.page_ready_us = None;
            }
        }
        let (Some(state), Some(runtime)) = (
            self.render_state.as_ref(),
            self.image_runtime.as_ref().or(self.runtime.as_ref()),
        ) else {
            return changed;
        };
        if self.metrics.page_ready_us.is_some() {
            return changed;
        }
        let viewport = Rect::new(
            0,
            self.document_scroll.offset_y,
            self.latest_viewport.w,
            self.latest_viewport.h,
        );
        let priorities = state.image_resource_priorities(viewport);
        let candidate = rappid_rabbit::images::initial_image_candidates(&self.discovered_resources)
            .filter(|c| !self.image_attempts.contains(&c.resolved_url))
            .min_by_key(|c| priorities.get(&c.resolved_url).copied().unwrap_or(2))
            .cloned();
        let candidate = candidate
            .filter(|_| self.image_attempts.len() < rappid_rabbit::images::MAX_IMAGE_CACHE_ENTRIES);
        let Some(candidate) = candidate else {
            if self.metrics.page_ready_us.is_none() {
                self.metrics.page_ready_us = self.metrics.started.map(|s| s.elapsed().as_micros());
                self.log_presentation_metrics("page-ready");
            }
            return changed;
        };
        if self
            .image_lane
            .compare_exchange(
                false,
                true,
                core::sync::atomic::Ordering::Acquire,
                core::sync::atomic::Ordering::Relaxed,
            )
            .is_err()
        {
            return changed;
        }
        let lane = ImageLaneGuard(self.image_lane.clone());
        self.image_attempts.push(candidate.resolved_url.clone());
        if self.image_cache.get(&candidate.resolved_url).is_some() {
            self.metrics.cache_hits += 1;
            return changed;
        }
        self.metrics.cache_misses += 1;
        let generation = self.document_lifecycle.generation();
        let lifecycle = self.document_lifecycle.clone();
        let cache = self.image_cache.clone();
        let shield = self.shield.clone();
        let shield_stats = self.shield_stats.clone();
        let document = state.final_url.clone();
        let (sender, receiver) = oneshot::channel();
        self.image_completion = Some(receiver);
        #[cfg(test)]
        let fetcher = self.image_fetcher;
        runtime.spawn_blocking(move || {
            let _lane = lane;
            if sender.is_closed() {
                return;
            }
            let mut worker = RabbitApp::new(None);
            worker.shield = shield;
            worker.shield_stats = shield_stats;
            worker.navigation_url = document;
            worker.document_lifecycle = lifecycle;
            // Diagnostics carry the target generation; no DOM crosses this job.
            worker.image_cache = cache;
            #[cfg(test)]
            {
                worker.image_fetcher = fetcher;
            }
            worker.developer_tools.console.clear();
            worker.fetch_images(&[candidate], &sender);
            let _ = sender.send(ImageOutput {
                generation,
                shield_revision: worker.shield.revision,
                cache: worker.image_cache,
                diagnostics: worker.developer_tools,
            });
        });
        changed
    }

    fn log_presentation_metrics(&self, milestone: &str) {
        #[cfg(feature = "dom")]
        debug_log(&format!("[RABBIT][PRESENT] gen={} milestone={} initial_us={:?} first_image_us={:?} visible_complete_us={:?} page_ready_us={:?} layout_runs={} layout_passes={} repaints={} cache_hits={} cache_misses={}",
            self.document_lifecycle.generation(), milestone, self.metrics.initial_us, self.metrics.first_image_us,
            self.metrics.visible_complete_us, self.metrics.page_ready_us,
            self.render_state.as_ref().map_or(0, |s| s.executed_layout_runs()), self.render_state.as_ref().map_or(0, |s| 2 * s.executed_layout_runs()), self.metrics.repaints, self.metrics.cache_hits, self.metrics.cache_misses));
    }

    #[cfg(feature = "dom")]
    fn log_image(
        &mut self,
        stage: &str,
        url: &str,
        format: &str,
        dimensions: Option<(u32, u32)>,
        result: &str,
    ) {
        let (width, height) = dimensions.unwrap_or((0, 0));
        self.developer_tools.console.push(ConsoleSeverity::Quiet, ConsoleSource::Fetch, format!(
            "[RABBIT][IMAGE] gen={} stage={stage} url={url} format={format} size={width}x{height} {result}",
            self.document_lifecycle.generation(),
        ));
    }

    fn log_heap_stage(&self, stage: &str, generation: u64, body_bytes: Option<usize>) {
        let mut buf = LogBuf::<256>::new();
        let _ = write!(
            buf,
            "[RABBIT][DOM] gen={} stage={} heap={}/{} parse_attempts={}",
            generation,
            stage,
            heap_used(),
            HEAP_SIZE,
            self.document_lifecycle.parse_attempts()
        );
        if let Some(body_bytes) = body_bytes {
            let _ = write!(buf, " body_bytes={body_bytes}");
        }
        let _ = writeln!(buf);
        debug_log(buf.as_str());
    }

    #[cfg(feature = "dom")]
    fn log_dom_result(
        &self,
        generation: u64,
        node_count: usize,
        max_depth: usize,
        total_text_bytes: usize,
        visible_rows: usize,
    ) {
        let mut buf = LogBuf::<256>::new();
        let _ = writeln!(
            buf,
            "[RABBIT][DOM] gen={} stage=after DOM parsing heap={}/{} nodes={} max_depth={} text_bytes={} visible_rows={} parse_attempts={} projection_per_tick=no",
            generation,
            heap_used(),
            HEAP_SIZE,
            node_count,
            max_depth,
            total_text_bytes,
            visible_rows,
            self.document_lifecycle.parse_attempts()
        );
        debug_log(buf.as_str());
    }

    fn build_source_content(&self, content_type: &str, body: Vec<u8>) -> SourceContent {
        if body.is_empty() {
            return SourceContent::Message(String::from("Empty response body."));
        }

        if core::str::from_utf8(&body).is_ok() {
            return SourceContent::Utf8(body);
        }

        if body_is_probably_text(Some(content_type), &body) {
            let lossy: Cow<'_, str> = String::from_utf8_lossy(&body);
            return SourceContent::Text(lossy.into_owned());
        }

        SourceContent::Message(format!(
            "Binary response body ({}) bytes.\nContent-Type: {content_type}",
            body.len()
        ))
    }

    fn apply_fetch_error(&mut self, err: FetchError) {
        self.cert_info = None;
        self.connection_security = if matches!(&err, FetchError::TlsCertExpired | FetchError::TlsCertInvalid(_)) {
            sunlight_cert::ConnectionSecurity::Invalid
        } else { sunlight_cert::ConnectionSecurity::Unknown };
        self.certificate_error = Some(format!("{err}"));
        self.status = String::from("Request failed");
        self.render_status = format!("Render unavailable: {err}");
        self.source = SourceContent::Message(String::from(
            "No response body was captured for this request.",
        ));
        self.discovered_resources.clear();
        self.resource_queue.clear();
        self.developer_tools
            .dom
            .clear_with_message("No DOM available for the failed request.");
        #[cfg(feature = "dom")]
        {
            self.render_state = None;
        }
        if let Some(request_id) = self.active_navigation_request_id {
            let (status_code, status_text) = match &err {
                FetchError::HttpError { status, message } if *status > 0 => {
                    (Some(*status), Some(message.clone()))
                }
                _ => (None, None),
            };
            self.developer_tools.network.fail_request(
                request_id,
                None,
                status_code,
                status_text,
                format!("{err}"),
            );
        }
        self.developer_tools.console.push(
            match err {
                FetchError::InvalidUrl(_) | FetchError::HttpError { .. } => ConsoleSeverity::Warn,
                _ => ConsoleSeverity::Error,
            },
            ConsoleSource::Fetch,
            format!("{err}"),
        );
    }

    fn set_navigation_error(&mut self, message: &str) {
        self.status = String::from("Navigation failed");
        self.render_status = String::from(message);
        self.source = SourceContent::Message(String::from(message));
        self.developer_tools
            .console
            .push(ConsoleSeverity::Error, ConsoleSource::Browser, message);
    }

    fn set_error(&mut self, message: String) {
        self.status = String::from("Invalid URL");
        self.render_status = String::from("Render unavailable: fix the URL and fetch again.");
        self.source = SourceContent::Message(String::from("Fix the URL and fetch again."));
        self.active_navigation_request_id = None;
        #[cfg(feature = "dom")]
        {
            self.render_state = None;
        }
        self.developer_tools
            .console
            .push(ConsoleSeverity::Warn, ConsoleSource::Browser, message);
    }

    fn compute_layout(root: Rect) -> RabbitLayout {
        let fill = Sizing::new(AxisSizing::Fill, AxisSizing::Fill);
        let mut root_children = [
            LayoutBox::new(Rect::new(0, 0, 0, TOP_BAR_H))
                .with_sizing(Sizing::new(AxisSizing::Fill, AxisSizing::Fixed(TOP_BAR_H))),
            LayoutBox::new(Rect::default()).with_sizing(fill),
        ];
        let _ = Column::new(root).arrange(&mut root_children);
        let top_bar = root_children[0].bounds();
        let content = root_children[1].bounds();
        let controls = Rect::new(
            top_bar.x + 6,
            top_bar.y + TAB_H as i32 + 3,
            top_bar.w.saturating_sub(12),
            top_bar.h.saturating_sub(TAB_H + 6),
        );
        // Below 420px omit unavailable history buttons; keep the address field
        // and tools usable. Row clips fixed controls for even smaller surfaces.
        let history_w = if root.w >= 420 { NAV_W } else { 0 };
        let fixed = |width| {
            LayoutBox::new(Rect::new(0, 0, width, 30))
                .with_sizing(Sizing::new(AxisSizing::Fixed(width), AxisSizing::Fixed(30)))
        };
        let mut toolbar = [
            fixed(history_w),
            fixed(history_w),
            LayoutBox::new(Rect::new(0, 0, 0, 30))
                .with_sizing(Sizing::new(AxisSizing::Fill, AxisSizing::Fixed(30))),
            fixed(NAV_W),
            fixed(NAV_W),
        ];
        let _ = Row::new(controls).with_gap(4).arrange(&mut toolbar);
        RabbitLayout {
            root,
            top_bar,
            content,
            tab: Rect::new(
                root.x + 6,
                root.y + 3,
                root.w.saturating_sub(46).min(240),
                TAB_H.saturating_sub(3).min(top_bar.h),
            ),
            back: toolbar[0].bounds(),
            forward: toolbar[1].bounds(),
            url_bar: toolbar[2].bounds(),
            view: toolbar[3].bounds(),
            devtools: toolbar[4].bounds(),
            shield: Rect::new(root.right()-34,root.y+3,28,TAB_H.saturating_sub(3).min(top_bar.h)),
        }
    }

    fn ensure_layout(&mut self) -> bool {
        if !self.layout_invalidation.update(self.client_bounds) {
            return false;
        }
        self.layout = Self::compute_layout(self.client_bounds);
        true
    }

    fn set_client_bounds(&mut self, width: u32, height: u32) -> bool {
        let bounds = Rect::new(0, 0, width, height);
        if bounds == self.client_bounds {
            return false;
        }
        self.client_bounds = bounds;
        self.layout_invalidation.invalidate();
        let changed = self.ensure_layout();
        self.document_scroll.end_drag();
        #[cfg(feature = "dom")]
        self.refresh_render_viewport();
        self.clamp_scrolls();
        self.refresh_cursor();
        changed
    }

    fn top_bar_rect(&self) -> Rect {
        self.layout.top_bar
    }

    fn devtools_button_rect(&self) -> Rect {
        self.layout.devtools
    }

    fn view_button_rect(&self) -> Rect {
        self.layout.view
    }

    fn url_bar_rect(&self) -> Rect {
        self.layout.url_bar
    }

    fn url_site_rect(&self) -> Rect {
        let bar = self.url_bar_rect();
        Rect::new(
            bar.x + 4.min(bar.w as i32),
            bar.y + 4,
            URL_SITE_W.min(bar.w.saturating_sub(8)),
            bar.h.saturating_sub(8),
        )
    }

    fn url_action_rect(&self) -> Rect {
        let bar = self.url_bar_rect();
        Rect::new(
            (bar.right() - URL_ACTION_W as i32 - 4).max(bar.x),
            bar.y + 4,
            URL_ACTION_W.min(bar.w.saturating_sub(8)),
            bar.h.saturating_sub(8),
        )
    }

    fn url_text_rect(&self) -> Rect {
        let bar = self.url_bar_rect();
        let site = self.url_site_rect();
        let action = self.url_action_rect();
        Rect::new(
            (site.right() + 5).min(bar.right()),
            bar.y + 2,
            (action.x - site.right() - 10).max(0) as u32,
            bar.h.saturating_sub(4),
        )
    }

    fn update_url_chrome_hover(&mut self, x: i32, y: i32) -> bool {
        let point = Point::new(x, y);
        let hovered = self.url_bar_rect().contains(point);
        let action_hovered = !self.navigation_draining && self.url_action_rect().contains(point);
        let changed = self.url_hovered != hovered || self.url_action_hovered != action_hovered;
        self.url_hovered = hovered;
        self.url_action_hovered = action_hovered;
        changed
    }

    fn content_rect(&self) -> Rect {
        self.layout.content
    }

    fn developer_panel_layout(&mut self) -> DeveloperPanelLayout {
        let available = self.content_rect();
        self.developer_tools.panel.compute_layout(available)
    }

    fn source_panel_rect(&mut self) -> Rect {
        self.developer_panel_layout().main_rect
    }

    fn source_view(&mut self) -> TextView<'_> {
        let content = Panel::with_title(self.source_panel_rect(), "Source")
            .content_rect()
            .inset(8);
        TextView::new(content, self.source.as_str())
            .with_scroll_offset(self.source_scroll)
            .with_focus(self.focus == FocusPane::Source)
            .with_font(&F_MONO)
    }

    fn render_viewport(&mut self) -> sunlight_ui::Size {
        DocumentCanvas::new(self.document_rect(), &[])
            .with_presentation(DocumentCanvasPresentation::Browser)
            .viewport_size()
    }

    #[cfg(feature = "dom")]
    fn refresh_render_viewport(&mut self) {
        let viewport = self.render_viewport();
        if viewport != self.latest_viewport {
            self.latest_viewport = viewport;
            self.viewport_version = self.viewport_version.wrapping_add(1);
        }
        // Resize handlers only publish the latest request. A tick submits at
        // most one owned layout snapshot; intervening events coalesce here.
    }

    #[cfg(feature = "dom")]
    fn poll_reflow(&mut self) -> bool {
        let ui_started = std::time::Instant::now();
        let mut changed = false;
        if let Some(receiver) = self.reflow_completion.as_mut() {
            match receiver.try_recv() {
                Ok(output) => {
                    self.reflow_completion = None;
                    let current = self.render_state.as_ref();
                    if output.version == self.viewport_version
                        && output.state.document_id == self.document_lifecycle.generation()
                        && output.state.styles.shield_revision().is_none_or(|revision| revision == self.shield.revision)
                        && current.is_some_and(|state| {
                            state.image_revision == output.state.image_revision
                                && state.dom_revision == output.state.dom_revision
                        })
                    {
                        let viewport = Rect::new(
                            0,
                            self.document_scroll.offset_y,
                            self.latest_viewport.w,
                            self.latest_viewport.h,
                        );
                        let repaint = self.developer_tools.panel.open
                            || current.is_none_or(|state| {
                                state.viewport != output.state.viewport
                                    || state.current_scene.content_size
                                        != output.state.current_scene.content_size
                            })
                            || output
                                .state
                                .last_patch
                                .dirty_regions
                                .iter()
                                .any(|region| region.intersect(viewport).is_some());
                        self.developer_tools
                            .dom
                            .update_styles(output.state.styles.clone());
                        self.render_state = Some(output.state);
                        self.shield_reflow_pending = false;
                        self.patch_all_controls();
                        self.clamp_scrolls();
                        // Counts are already computed on the worker. Avoid
                        // rescanning the scene and serializing every image URL
                        // during the UI commit.
                        self.developer_tools.console.push(
                            ConsoleSeverity::Quiet,
                            ConsoleSource::Browser,
                            self.render_state.as_ref().unwrap().diagnostics.summary(),
                        );
                        debug_log(&format!("[RABBIT][LAYOUT] commit gen={} version={} viewport={}x{} ui_us={} ui_thread=yes",
                            self.document_lifecycle.generation(), self.viewport_version, self.latest_viewport.w, self.latest_viewport.h, ui_started.elapsed().as_micros()));
                        changed |= repaint;
                    } else {
                        debug_log(&format!(
                            "[RABBIT][LAYOUT] discard gen={} version={} latest={}",
                            output.state.document_id, output.version, self.viewport_version
                        ));
                    }
                }
                Err(oneshot::error::TryRecvError::Empty) => return false,
                Err(oneshot::error::TryRecvError::Closed) => {
                    self.reflow_completion = None;
                    self.render_status = String::from("Layout worker stopped");
                    // Do not retry a failed worker on every tick.
                    self.render_state = None;
                    return true;
                }
            }
        }
        if self.pending_fetch {
            return changed;
        }
        let (Some(state), Some(runtime)) = (self.render_state.as_ref(), self.runtime.as_ref())
        else {
            return changed;
        };
        if state.viewport == self.latest_viewport && !state.images_need_update() && !self.shield_reflow_pending {
            return changed;
        }
        let mut snapshot = if !self.shield_reflow_pending && state.viewport == self.latest_viewport && state.images_need_update() {
            state.image_snapshot()
        } else {
            state.reflow_snapshot()
        };
        let viewport = self.latest_viewport;
        let version = self.viewport_version;
        let shield = self.shield_reflow_pending.then(|| self.shield.clone());
        let (sender, receiver) = oneshot::channel();
        self.reflow_completion = Some(receiver);
        debug_log(&format!(
            "[RABBIT][LAYOUT] submit gen={} version={} viewport={}x{} ui_us={}",
            self.document_lifecycle.generation(),
            version,
            viewport.w,
            viewport.h,
            ui_started.elapsed().as_micros()
        ));
        runtime.spawn_blocking(move || {
            if sender.is_closed() {
                return;
            }
            let start = std::time::Instant::now();
            if let Some(shield) = shield {
                let policy = rappid_rabbit::content_shield::CosmeticPolicy::for_document(&shield, &snapshot.final_url);
                snapshot.set_shield_policy(policy, &RABBIT_FONTS);
            }
            snapshot.update_for_viewport(viewport, &RABBIT_FONTS);
            debug_log(&format!(
                "[RABBIT][LAYOUT] reflow gen={} version={} viewport={}x{} elapsed_ms={} worker=yes",
                snapshot.document_id,
                version,
                viewport.w,
                viewport.h,
                start.elapsed().as_millis()
            ));
            let _ = sender.send(ReflowOutput {
                version,
                state: snapshot,
            });
        });
        changed
    }

    #[cfg(feature = "dom")]
    fn log_render_scene(&mut self, reason: &str) {
        let Some(render_state) = self.render_state.as_ref() else {
            return;
        };
        let mut text_count = 0usize;
        let mut rectangle_count = 0usize;
        let mut link_count = 0usize;
        let mut image_count = 0usize;
        let mut placeholder_count = 0usize;
        let mut image_transitions = Vec::new();
        for object in &render_state.current_scene.objects {
            match &object.kind {
                RenderObjectKind::Text { .. } => text_count += 1,
                RenderObjectKind::Rectangle { .. } => rectangle_count += 1,
                RenderObjectKind::Link { .. } => link_count += 1,
                RenderObjectKind::Image { source_url, .. } => {
                    image_count += 1;
                    image_transitions.push(format!(
                        "[RABBIT][IMAGE] scene gen={} node={} object={} kind=Image url={} bounds={},{} {}x{}",
                        render_state.document_id,
                        object.owner_node_id.0,
                        object.id.0,
                        source_url,
                        object.bounds.x,
                        object.bounds.y,
                        object.bounds.w,
                        object.bounds.h,
                    ));
                }
                RenderObjectKind::ImagePlaceholder { src, .. } => {
                    placeholder_count += 1;
                    image_transitions.push(format!(
                        "[RABBIT][IMAGE] scene gen={} node={} object={} kind=ImagePlaceholder url={} bounds={},{} {}x{}",
                        render_state.document_id,
                        object.owner_node_id.0,
                        object.id.0,
                        src,
                        object.bounds.x,
                        object.bounds.y,
                        object.bounds.w,
                        object.bounds.h,
                    ));
                }
                _ => {}
            }
        }
        self.developer_tools.console.push(
            ConsoleSeverity::Quiet,
            ConsoleSource::Browser,
            format!(
                "DocumentCanvas {reason}: gen={} viewport={}x{} objects={} content={}x{} text={} rectangles={} links={} images={} placeholders={} patches={}",
                render_state.scene_generation,
                render_state.viewport.w,
                render_state.viewport.h,
                render_state.current_scene.objects.len(),
                render_state.current_scene.content_size.w,
                render_state.current_scene.content_size.h,
                text_count,
                rectangle_count,
                link_count,
                image_count,
                placeholder_count,
                render_state.last_patch.operations.len(),
            ),
        );
        for transition in image_transitions {
            self.developer_tools.console.push(
                ConsoleSeverity::Quiet,
                ConsoleSource::Browser,
                transition,
            );
        }
    }

    fn document_canvas(&mut self) -> DocumentCanvas<'_> {
        let rect = self.document_rect();
        let canvas = DocumentCanvas::new(rect, &[])
            .with_presentation(DocumentCanvasPresentation::Browser)
            .with_empty_label(self.render_status.as_str())
            .with_fonts(Some(&F_LARGE), Some(&F_SMALL), Some(&F_UI), Some(&F_SMALL))
            .with_scene_font_families(Some(&F_SERIF), Some(&F_MONO));
        #[cfg(feature = "dom")]
        if let Some(render_state) = self.render_state.as_ref() {
            return canvas
                .with_scene(&render_state.current_scene)
                .with_image_presentation(self.presentation.admitted())
                .with_scroll_y(self.document_scroll.offset_y as u32);
        }
        canvas
    }

    #[cfg(feature = "dom")]
    fn patch_all_controls(&mut self) {
        let Some(render_state) = self.render_state.as_mut() else {
            return;
        };
        let focused = self.form_state.focused_control;
        for (id, state) in &self.form_state.controls {
            let _ = render_state.patch_control(*id, state, focused == Some(*id), &RABBIT_FONTS);
        }
    }

    #[cfg(feature = "dom")]
    fn patch_focused_control(&mut self) {
        let Some(id) = self.form_state.focused_control else {
            self.patch_all_controls();
            return;
        };
        let Some(state) = self.form_state.controls.get(&id).cloned() else {
            return;
        };
        if let Some(render_state) = self.render_state.as_mut() {
            let _ = render_state.patch_control(id, &state, true, &RABBIT_FONTS);
        }
    }

    #[cfg(feature = "dom")]
    fn handle_form_control_click(&mut self, point: Point) -> bool {
        if self.document_view != DocumentView::Render {
            return false;
        }
        let canvas = self.document_canvas();
        let Some(object) = canvas.hit_test(point) else {
            return false;
        };
        let Some(RenderInteraction::Control { owner_node_id }) = object.interaction.clone() else {
            return false;
        };
        if let Some(pressed) = self.form_state.pressed_control {
            if pressed != owner_node_id {
                self.form_state.pressed_control = None;
                return true;
            }
        }
        self.form_state.pressed_control = None;
        let Some(control) = self.form_state.controls.get(&owner_node_id).cloned() else {
            return true;
        };
        if control.is_disabled() {
            return true;
        }
        match control.kind() {
            FormControlKind::TextInput | FormControlKind::SearchInput => {
                self.form_state.focus_control(owner_node_id);
                self.patch_all_controls();
            }
            FormControlKind::SubmitInput
            | FormControlKind::ButtonElement(rappid_rabbit::form::ButtonType::Submit) => {
                self.form_state.focused_control = Some(owner_node_id);
                self.submit_form(owner_node_id);
            }
            _ => {
                self.form_state.focused_control = Some(owner_node_id);
                self.patch_all_controls();
            }
        }
        true
    }

    #[cfg(feature = "dom")]
    fn submit_form(&mut self, submitter: DocumentNodeId) -> bool {
        let Some(render_state) = self.render_state.as_ref() else {
            return false;
        };
        let Some(control) = self.form_state.controls.get(&submitter) else {
            return false;
        };
        let Some(form_id) = control.form_owner() else {
            return true;
        };
        let method = rappid_rabbit::form::get_form_method(&render_state.dom, form_id.0 as usize);
        if method != "get" {
            self.developer_tools.console.push(
                ConsoleSeverity::Warn,
                ConsoleSource::Browser,
                format!(
                    "[RABBIT][FORM] method={} forms are not supported.",
                    method.to_ascii_uppercase()
                ),
            );
            return true;
        }
        let base = ParsedUrl::parse(&render_state.final_url).ok();
        let target = rappid_rabbit::form::build_get_submission_url(
            &self.form_state,
            &render_state.dom,
            form_id,
            &render_state.final_url,
            base.as_ref(),
            Some(submitter),
        );
        self.developer_tools.console.push(
            ConsoleSeverity::Quiet,
            ConsoleSource::Browser,
            format!("Form submitted: GET {target}"),
        );
        self.url_input.set_text(&target);
        self.queue_fetch();
        true
    }

    #[cfg(feature = "dom")]
    fn handle_form_key(&mut self, event: Event) -> bool {
        let Some(id) = self.form_state.focused_control else {
            return false;
        };
        let Some(control) = self.form_state.controls.get(&id) else {
            return false;
        };
        if control.is_disabled() {
            return false;
        }
        match event {
            Event::Key(ch) if ch == '\n' || ch == '\r' => {
                if control.kind().is_text() || control.kind().is_submit() {
                    return self.submit_form(id);
                }
                false
            }
            Event::Key(ch) if ch == '\u{8}' => {
                let changed = self.form_state.backspace();
                if changed {
                    self.patch_focused_control();
                }
                changed
            }
            Event::Key(ch) if !ch.is_control() => {
                let changed = self.form_state.insert_char(ch);
                if changed {
                    self.patch_focused_control();
                }
                changed
            }
            Event::KeyPress {
                keycode,
                pressed: true,
                shift,
                ..
            } => {
                if keycode == 0x1C && (control.kind().is_text() || control.kind().is_submit()) {
                    return self.submit_form(id);
                }
                let changed = match keycode {
                    KEY_LEFT => self.form_state.move_cursor_left(),
                    KEY_RIGHT => self.form_state.move_cursor_right(),
                    KEY_HOME => self.form_state.move_cursor_home(),
                    KEY_END => self.form_state.move_cursor_end(),
                    0x53 => self.form_state.delete_forward(),
                    0x0F if shift => self
                        .form_state
                        .focus_prev_control(&self.render_state.as_ref().unwrap().dom)
                        .is_some(),
                    0x0F => self
                        .form_state
                        .focus_next_control(&self.render_state.as_ref().unwrap().dom)
                        .is_some(),
                    _ => false,
                };
                if changed {
                    self.patch_all_controls();
                }
                changed
            }
            _ => false,
        }
    }

    #[cfg(feature = "dom")]
    fn handle_render_click(&mut self, point: Point) -> bool {
        if self.document_view != DocumentView::Render {
            return false;
        }
        let interaction = self
            .document_canvas()
            .hit_test(point)
            .and_then(|object| object.interaction.clone());
        let Some(RenderInteraction::Link {
            href, resolved_url, ..
        }) = interaction
        else {
            return false;
        };
        let target = resolved_url.unwrap_or(href);
        if target.is_empty() {
            self.developer_tools.console.push(
                ConsoleSeverity::Warn,
                ConsoleSource::Browser,
                "Canvas link has no navigable URL.",
            );
            return true;
        }
        self.developer_tools.console.push(
            ConsoleSeverity::Quiet,
            ConsoleSource::Browser,
            format!("Canvas link selected: {target}"),
        );
        self.url_input.set_text(&target);
        self.queue_fetch();
        true
    }

    fn toggle_developer_tools(&mut self) {
        if self.developer_tools.panel.open {
            self.developer_tools.panel.close();
            self.focus = match self.document_view {
                DocumentView::Source => FocusPane::Source,
                DocumentView::Render => FocusPane::Render,
            };
        } else {
            self.developer_tools.panel.open();
            self.focus = FocusPane::DeveloperTools;
        }
        #[cfg(feature = "dom")]
        self.refresh_render_viewport();
        self.clamp_scrolls();
    }

    fn certificate_panel_rect(&self) -> Rect {
        let anchor = self.url_site_rect();
        Rect::new(
            anchor.x.max(0),
            anchor.bottom() + 6,
            self.client_bounds
                .w
                .saturating_sub(anchor.x.max(0) as u32)
                .min(560),
            self.client_bounds
                .h
                .saturating_sub(anchor.bottom().max(0) as u32 + 12)
                .min(540),
        )
    }

    fn draw_url_bar(&mut self, canvas: &mut Canvas, theme: &Theme) {
        let rect = self.url_bar_rect();
        let focused = self.url_input.active;
        let disabled = self.navigation_draining;
        let fill = if disabled {
            theme.panel_alt.darken(18)
        } else if focused {
            theme.chrome.input_bg.lighten(10)
        } else if self.url_hovered {
            theme.chrome.input_bg.lighten(5)
        } else {
            theme.chrome.input_bg
        };
        let border = if disabled {
            theme.chrome.subtle_border
        } else if focused {
            theme.accent
        } else if self.url_hovered {
            theme.border.lighten(38)
        } else {
            theme.chrome.subtle_border
        };
        canvas.fill_rounded_rect(rect, URL_BAR_RADIUS, fill);
        canvas.stroke_rounded_rect(rect, URL_BAR_RADIUS, 1, border);
        let site = self.url_site_rect();
        let cx = site.x + site.w as i32 / 2;
        let cy = site.y + site.h as i32 / 2;
        if rappid_rabbit::startpage::is_custom_url(self.url_input.value().trim()) {
            let sun = theme.accent;
            canvas.stroke_rounded_rect(Rect::new(cx - 3, cy - 3, 7, 7), 4, 1, sun);
            canvas.hline(cx - 6, cy, 2, sun);
            canvas.hline(cx + 5, cy, 2, sun);
            canvas.vline(cx, cy - 6, 2, sun);
            canvas.vline(cx, cy + 5, 2, sun);
            canvas.fill_rect(Rect::new(cx - 4, cy - 4, 1, 1), sun);
            canvas.fill_rect(Rect::new(cx + 4, cy - 4, 1, 1), sun);
            canvas.fill_rect(Rect::new(cx - 4, cy + 4, 1, 1), sun);
            canvas.fill_rect(Rect::new(cx + 4, cy + 4, 1, 1), sun);
        } else if self.connection_security == sunlight_cert::ConnectionSecurity::Secure {
            canvas.stroke_rounded_rect(Rect::new(cx - 4, cy - 7, 9, 10), 4, 1, theme.ok);
            canvas.fill_rounded_rect(Rect::new(cx - 6, cy - 1, 13, 9), 2, theme.ok);
        } else if self.connection_security == sunlight_cert::ConnectionSecurity::Invalid {
            canvas.draw_text(cx - 4, cy - 5, "!", theme.warn);
        } else {
            // The URL scheme alone cannot establish that TLS was verified.
            canvas.stroke_rounded_rect(Rect::new(cx - 5, cy - 5, 11, 11), 5, 1, theme.icon_muted);
            canvas.vline(cx, cy, 3, theme.icon_muted);
            canvas.fill_rect(Rect::new(cx, cy - 3, 1, 1), theme.icon_muted);
        }

        let action = self.url_action_rect();
        if self.url_action_hovered {
            canvas.fill_rounded_rect(action, 5, theme.chrome.control_hover);
        }
        Self::draw_reload_indicator(
            canvas,
            action,
            if disabled {
                theme.chrome.disabled_fg
            } else if self.url_action_hovered {
                theme.text
            } else {
                theme.icon_muted
            },
            self.pending_fetch,
        );

        self.url_input.rect = self.url_text_rect();
        self.url_input.draw_content(canvas, theme);
    }

    fn draw_reload_indicator(canvas: &mut Canvas, rect: Rect, color: Color, loading: bool) {
        let cx = rect.x + rect.w as i32 / 2;
        let cy = rect.y + rect.h as i32 / 2;
        if loading {
            canvas.fill_rounded_rect(Rect::new(cx - 4, cy - 4, 8, 8), 1, color);
            return;
        }
        canvas.hbar(cx - 5, cy - 5, 7, 1, color);
        canvas.vline(cx - 5, cy - 5, 6, color);
        canvas.hbar(cx - 5, cy + 5, 7, 1, color);
        canvas.vline(cx + 5, cy, 6, color);
        canvas.hbar(cx + 2, cy - 7, 4, 1, color);
        canvas.hbar(cx - 6, cy + 7, 4, 1, color);
        canvas.vline(cx + 5, cy - 7, 3, color);
        canvas.vline(cx - 5, cy + 5, 3, color);
    }

    fn draw_resize_handle(
        &self,
        canvas: &mut Canvas,
        theme: &Theme,
        panel_state: &DeveloperPanelState,
        handle_rect: Rect,
    ) {
        canvas.fill_rect(handle_rect, theme.panel);
        canvas.draw_rect(
            handle_rect,
            if panel_state.is_resizing() {
                theme.accent
            } else {
                theme.border
            },
        );

        let grip_w = 56u32;
        let start_x = handle_rect.x + (handle_rect.w as i32 - grip_w as i32) / 2;
        let center_y = handle_rect.y + handle_rect.h as i32 / 2 - 2;
        for offset in [0, 4, 8] {
            canvas.hbar(start_x, center_y + offset, grip_w, 1, theme.text_dim);
        }
    }

    fn draw_developer_tools(&mut self, canvas: &mut Canvas, theme: &Theme) {
        let layout = self.developer_panel_layout();
        let Some(panel_rect) = layout.panel_rect else {
            return;
        };
        let handle_rect = layout.resize_handle_rect.unwrap();
        let tab_bar_rect = layout.tab_bar_rect.unwrap();
        let close_button_rect = layout.close_button_rect.unwrap();
        let content_rect = layout.content_rect.unwrap();

        self.draw_resize_handle(canvas, theme, &self.developer_tools.panel, handle_rect);
        Panel::new(panel_rect).draw(canvas, theme);

        let tab_labels = ["Console", "DOM Inspector", "Network"];
        let tab_bar = TabBar::new(
            tab_bar_rect,
            &tab_labels,
            self.developer_tools.panel.active_tab.index(),
        );
        tab_bar.draw(canvas, theme);

        let mut close_button = Button::secondary(close_button_rect, "Close").with_font(&F_UI);
        close_button.state = ButtonState::Normal;
        close_button.draw(canvas, theme);

        match self.developer_tools.panel.active_tab {
            DeveloperToolTab::Console => {
                let console_rect = content_rect.inset(8);
                let scroll_offset = self.developer_tools.console.scroll_offset();
                let console_text = self.developer_tools.console.rendered_text();
                TextView::new(console_rect, console_text)
                    .with_scroll_offset(scroll_offset)
                    .with_focus(self.focus == FocusPane::DeveloperTools)
                    .with_font(&F_MONO)
                    .draw(canvas, theme);
            }
            DeveloperToolTab::DomInspector => {
                self.draw_dom_inspector_tab(canvas, theme, content_rect)
            }
            DeveloperToolTab::Network => self.draw_network_tab(canvas, theme, content_rect),
        }
    }

    fn dom_column_rects(content_rect: Rect) -> (Rect, Rect, Rect) {
        let styles_w = ((content_rect.w as i32 * 26) / 100).max(180) as u32;
        let props_w = ((content_rect.w as i32 * 30) / 100).max(220) as u32;
        let tree_w = content_rect
            .w
            .saturating_sub(styles_w)
            .saturating_sub(props_w)
            .saturating_sub((GUTTER.max(0) as u32) * 2);

        let styles = Rect::new(content_rect.x, content_rect.y, styles_w, content_rect.h);
        let properties = Rect::new(
            styles.right() + GUTTER,
            content_rect.y,
            props_w,
            content_rect.h,
        );
        let tree = Rect::new(
            properties.right() + GUTTER,
            content_rect.y,
            tree_w.max(220),
            content_rect.h,
        );
        (styles, properties, tree)
    }

    fn draw_dom_inspector_tab(&mut self, canvas: &mut Canvas, theme: &Theme, content_rect: Rect) {
        let (styles_rect, properties_rect, tree_rect) =
            Self::dom_column_rects(content_rect.inset(8));

        // Inject render correlation + real box geometry into inspector when available
        #[cfg(feature = "dom")]
        if let Some(rs) = self.render_state.as_ref() {
            if let Some(node) = self.developer_tools.dom.selected_node() {
                let mut extra = String::new();
                // Render objects
                let dom_id = sunlight_ui::widgets::DocumentNodeId(node as u64);
                let obj_ids = rs.current_scene.objects_for_node(dom_id);
                if !obj_ids.is_empty() {
                    extra.push_str("\nRender Objects:\n");
                    for &oid in obj_ids.iter().take(6) {
                        if let Some(obj) = rs.current_scene.object(oid) {
                            extra.push_str("  ");
                            extra.push_str(&format!(
                                "{:?} id={:?} bounds={:?}\n",
                                obj.kind, obj.id, obj.bounds
                            ));
                        }
                    }
                }
                if let Some(geometry) = rs.layout_geometry.iter().find(|item| item.node_id == node)
                {
                    extra.push_str("\nLayout Box:\n");
                    extra.push_str(&format!(
                        "  content: {:?}\n  padding: {:?}\n  border: {:?}\n  margin: {:?}\n",
                        geometry.content_box,
                        geometry.padding_box,
                        geometry.border_box,
                        geometry.margin_box,
                    ));
                }
                self.developer_tools.dom.set_extra_info(extra);
            }
        }

        let styles_title = format!(
            "Styles [{}]",
            self.developer_tools.dom.styles_mode().label()
        );
        let styles_panel = Panel::with_title(styles_rect, &styles_title);
        styles_panel.draw(canvas, theme);
        let styles_scroll = self.developer_tools.dom.styles_scroll();
        let styles_focused = self.focus == FocusPane::DeveloperTools
            && self.developer_tools.dom.focused_pane() == DomInspectorPane::Styles;
        let styles_text = self.developer_tools.dom.styles_text();
        TextView::new(styles_panel.content_rect().inset(8), styles_text)
            .with_scroll_offset(styles_scroll)
            .with_focus(styles_focused)
            .with_font(&F_MONO)
            .draw(canvas, theme);

        let properties_panel = Panel::with_title(properties_rect, "Properties");
        properties_panel.draw(canvas, theme);
        let properties_scroll = self.developer_tools.dom.properties_scroll();
        let properties_focused = self.focus == FocusPane::DeveloperTools
            && self.developer_tools.dom.focused_pane() == DomInspectorPane::Properties;
        let properties_text = self.developer_tools.dom.node_properties_text();
        TextView::new(properties_panel.content_rect().inset(8), properties_text)
            .with_scroll_offset(properties_scroll)
            .with_focus(properties_focused)
            .with_font(&F_MONO)
            .draw(canvas, theme);

        self.draw_dom_tree(canvas, theme, tree_rect);
    }

    fn draw_dom_tree(&mut self, canvas: &mut Canvas, theme: &Theme, rect: Rect) {
        let panel = Panel::with_title(rect, "DOM Tree");
        panel.draw(canvas, theme);
        let content = panel.content_rect().inset(8);
        let tree_scroll = self.developer_tools.dom.tree_scroll();
        let tree_focused = self.focus == FocusPane::DeveloperTools
            && self.developer_tools.dom.focused_pane() == DomInspectorPane::Tree;
        let rows = self.developer_tools.dom.tree_rows();

        if rows.is_empty() {
            TextView::new(content, self.developer_tools.dom.empty_message())
                .with_focus(tree_focused)
                .with_font(&F_MONO)
                .draw(canvas, theme);
            return;
        }

        TreeView::new(content, rows)
            .with_scroll_offset(tree_scroll)
            .with_focus(tree_focused)
            .with_font(&F_MONO)
            .draw(canvas, theme);
    }

    fn dom_tree_visible_rows(&mut self, tree_rect: Rect) -> usize {
        let content = Panel::with_title(tree_rect, "DOM Tree")
            .content_rect()
            .inset(8);
        let rows = self.developer_tools.dom.tree_rows();
        TreeView::new(content, rows)
            .with_font(&F_MONO)
            .visible_row_count()
    }

    fn network_layout(content_rect: Rect) -> (Rect, Rect) {
        let gap = GUTTER.max(0) as u32;
        let available_w = content_rect.w.saturating_sub(gap);
        let min_list = NETWORK_LIST_MIN_W.min(available_w.max(1));
        let min_detail = NETWORK_DETAIL_MIN_W.min(available_w.max(1));
        let preferred_list = ((available_w as i32 * 45) / 100)
            .max(min_list as i32)
            .min(available_w.saturating_sub(min_detail).max(1) as i32)
            as u32;
        let list_w = preferred_list.max(available_w / 3).min(available_w.max(1));
        let detail_w = available_w.saturating_sub(list_w).max(1);

        let list_rect = Rect::new(content_rect.x, content_rect.y, list_w, content_rect.h);
        let detail_rect = Rect::new(
            list_rect.right() + GUTTER,
            content_rect.y,
            detail_w,
            content_rect.h,
        );
        (list_rect, detail_rect)
    }

    fn network_header_h() -> u32 {
        (F_SMALL.line_height() + 8).max(18)
    }

    fn network_row_h() -> u32 {
        (F_SMALL.line_height() + 5).max(16)
    }

    fn draw_network_tab(&mut self, canvas: &mut Canvas, theme: &Theme, content_rect: Rect) {
        let (list_rect, detail_rect) = Self::network_layout(content_rect.inset(8));
        let list_panel = Panel::with_title(list_rect, "Requests");
        list_panel.draw(canvas, theme);
        let detail_panel = Panel::with_title(detail_rect, "Details");
        detail_panel.draw(canvas, theme);

        let selected_row = self.developer_tools.network.selected_row();
        let list_scroll_offset = self.developer_tools.network.list_scroll_offset();
        let rows = self.developer_tools.network.summary_rows();
        let columns = [
            TableColumn {
                header: "Name",
                width: list_panel
                    .content_rect()
                    .w
                    .saturating_sub(92 + 108 + 96 + 88 + 92),
                right_align: false,
            },
            TableColumn {
                header: "Method",
                width: 92,
                right_align: false,
            },
            TableColumn {
                header: "Status",
                width: 108,
                right_align: false,
            },
            TableColumn {
                header: "Type",
                width: 96,
                right_align: false,
            },
            TableColumn {
                header: "Size",
                width: 88,
                right_align: true,
            },
            TableColumn {
                header: "Time",
                width: 92,
                right_align: true,
            },
        ];

        let row_refs: Vec<[&str; 6]> = rows
            .iter()
            .map(|row| {
                [
                    row[0].as_str(),
                    row[1].as_str(),
                    row[2].as_str(),
                    row[3].as_str(),
                    row[4].as_str(),
                    row[5].as_str(),
                ]
            })
            .collect();
        let row_slices: Vec<&[&str]> = row_refs.iter().map(|row| row.as_slice()).collect();

        Table::new(list_panel.content_rect(), &columns, &row_slices)
            .with_selected(selected_row)
            .with_scroll_offset(list_scroll_offset)
            .with_font(&F_SMALL)
            .draw(canvas, theme);

        let detail_focused = self.focus == FocusPane::DeveloperTools
            && self.developer_tools.network.focused_pane() == NetworkPaneFocus::Details;
        let detail_scroll_offset = self.developer_tools.network.detail_scroll_offset();
        let detail_text = self.developer_tools.network.selected_request_detail_text();
        TextView::new(detail_panel.content_rect().inset(8), detail_text)
            .with_scroll_offset(detail_scroll_offset)
            .with_focus(detail_focused)
            .with_font(&F_MONO)
            .draw(canvas, theme);

        if self.focus == FocusPane::DeveloperTools {
            match self.developer_tools.network.focused_pane() {
                NetworkPaneFocus::RequestList => canvas.draw_rect(list_panel.rect, theme.accent),
                NetworkPaneFocus::Details => canvas.draw_rect(detail_panel.rect, theme.accent),
            }
        }
    }

    fn clamp_scrolls(&mut self) {
        if self.document_view == DocumentView::Source {
            self.source_scroll = self.source_scroll.min(self.source_view().max_scroll());
        }
        self.sync_document_scroll();

        let dev_layout = self.developer_panel_layout();
        if let Some(content_rect) = dev_layout.content_rect {
            match self.developer_tools.panel.active_tab {
                DeveloperToolTab::Console => {
                    let console_text = self.developer_tools.console.rendered_text();
                    let max = TextView::new(content_rect.inset(8), console_text)
                        .with_font(&F_MONO)
                        .max_scroll();
                    self.developer_tools
                        .console
                        .set_scroll_offset(self.developer_tools.console.scroll_offset().min(max));
                }
                DeveloperToolTab::DomInspector => {
                    let (styles_rect, properties_rect, tree_rect) =
                        Self::dom_column_rects(content_rect.inset(8));
                    let styles_max = {
                        let mode_label = self.developer_tools.dom.styles_mode().label();
                        let styles_text = self.developer_tools.dom.styles_text();
                        TextView::new(
                            Panel::with_title(styles_rect, &format!("Styles [{}]", mode_label))
                                .content_rect()
                                .inset(8),
                            styles_text,
                        )
                        .with_font(&F_MONO)
                        .max_scroll()
                    };
                    let properties_max = {
                        let properties_text = self.developer_tools.dom.node_properties_text();
                        TextView::new(
                            Panel::with_title(properties_rect, "Properties")
                                .content_rect()
                                .inset(8),
                            properties_text,
                        )
                        .with_font(&F_MONO)
                        .max_scroll()
                    };
                    let visible = self.dom_tree_visible_rows(tree_rect);
                    self.developer_tools.dom.set_styles_scroll(
                        self.developer_tools.dom.styles_scroll().min(styles_max),
                    );
                    self.developer_tools.dom.set_properties_scroll(
                        self.developer_tools
                            .dom
                            .properties_scroll()
                            .min(properties_max),
                    );
                    self.developer_tools.dom.clamp_tree_scroll(visible);
                }
                DeveloperToolTab::Network => {
                    let (list_rect, detail_rect) = Self::network_layout(content_rect.inset(8));
                    let list_content = Panel::with_title(list_rect, "Requests").content_rect();
                    let visible = ((list_content.h.saturating_sub(Self::network_header_h()))
                        / Self::network_row_h())
                    .max(1) as usize;
                    let max = self
                        .developer_tools
                        .network
                        .entries()
                        .len()
                        .saturating_sub(visible);
                    self.developer_tools.network.set_list_scroll_offset(
                        self.developer_tools.network.list_scroll_offset().min(max),
                    );

                    let detail_text = self.developer_tools.network.selected_request_detail_text();
                    let detail_max = TextView::new(
                        Panel::with_title(detail_rect, "Details")
                            .content_rect()
                            .inset(8),
                        detail_text,
                    )
                    .with_font(&F_MONO)
                    .max_scroll();
                    self.developer_tools.network.set_detail_scroll_offset(
                        self.developer_tools
                            .network
                            .detail_scroll_offset()
                            .min(detail_max),
                    );
                }
            }
        }
    }

    fn scroll_focused(&mut self, delta: i32, page: bool, home: bool, end: bool) {
        match self.focus {
            FocusPane::Source => {
                let (visible, max_scroll) = {
                    let view = self.source_view();
                    (view.visible_line_count(), view.max_scroll())
                };
                adjust_scroll(
                    &mut self.source_scroll,
                    delta,
                    page,
                    home,
                    end,
                    visible,
                    max_scroll,
                );
            }
            FocusPane::Render => {
                self.sync_document_scroll();
                if home {
                    self.document_scroll.scroll_to_y(0);
                } else if end {
                    self.document_scroll
                        .scroll_to_y(self.document_scroll.max_offset_y());
                } else {
                    let step = if page {
                        self.document_scroll.viewport_h.saturating_sub(24) as i32
                    } else {
                        24
                    };
                    self.document_scroll.scroll_by(delta.saturating_mul(step));
                }
            }

            FocusPane::DeveloperTools => self.scroll_developer_tools(delta, page, home, end),
        }
    }

    fn scroll_developer_tools(&mut self, delta: i32, page: bool, home: bool, end: bool) {
        let layout = self.developer_panel_layout();
        let Some(content_rect) = layout.content_rect else {
            return;
        };
        match self.developer_tools.panel.active_tab {
            DeveloperToolTab::Console => {
                let mut scroll = self.developer_tools.console.scroll_offset();
                let console_text = self.developer_tools.console.rendered_text();
                let view = TextView::new(content_rect.inset(8), console_text).with_font(&F_MONO);
                adjust_scroll(
                    &mut scroll,
                    delta,
                    page,
                    home,
                    end,
                    view.visible_line_count(),
                    view.max_scroll(),
                );
                self.developer_tools.console.set_scroll_offset(scroll);
            }
            DeveloperToolTab::DomInspector => {
                let (styles_rect, properties_rect, tree_rect) =
                    Self::dom_column_rects(content_rect.inset(8));
                match self.developer_tools.dom.focused_pane() {
                    DomInspectorPane::Styles => {
                        let mut scroll = self.developer_tools.dom.styles_scroll();
                        let (visible_lines, max_scroll) = {
                            let mode_label = self.developer_tools.dom.styles_mode().label();
                            let styles_text = self.developer_tools.dom.styles_text();
                            let view = TextView::new(
                                Panel::with_title(styles_rect, &format!("Styles [{}]", mode_label))
                                    .content_rect()
                                    .inset(8),
                                styles_text,
                            )
                            .with_font(&F_MONO);
                            (view.visible_line_count(), view.max_scroll())
                        };
                        adjust_scroll(
                            &mut scroll,
                            delta,
                            page,
                            home,
                            end,
                            visible_lines,
                            max_scroll,
                        );
                        self.developer_tools.dom.set_styles_scroll(scroll);
                    }
                    DomInspectorPane::Properties => {
                        let mut scroll = self.developer_tools.dom.properties_scroll();
                        let (visible_lines, max_scroll) = {
                            let properties_text = self.developer_tools.dom.node_properties_text();
                            let view = TextView::new(
                                Panel::with_title(properties_rect, "Properties")
                                    .content_rect()
                                    .inset(8),
                                properties_text,
                            )
                            .with_font(&F_MONO);
                            (view.visible_line_count(), view.max_scroll())
                        };
                        adjust_scroll(
                            &mut scroll,
                            delta,
                            page,
                            home,
                            end,
                            visible_lines,
                            max_scroll,
                        );
                        self.developer_tools.dom.set_properties_scroll(scroll);
                    }
                    DomInspectorPane::Tree => {
                        let visible = self.dom_tree_visible_rows(tree_rect);
                        let row_count = self.developer_tools.dom.tree_rows().len();
                        let max_scroll = row_count.saturating_sub(visible);
                        let mut scroll = self.developer_tools.dom.tree_scroll();
                        adjust_scroll(&mut scroll, delta, page, home, end, visible, max_scroll);
                        self.developer_tools.dom.set_tree_scroll(scroll);
                    }
                }
            }
            DeveloperToolTab::Network => {
                let (list_rect, detail_rect) = Self::network_layout(content_rect.inset(8));
                match self.developer_tools.network.focused_pane() {
                    NetworkPaneFocus::RequestList => {
                        let content = Panel::with_title(list_rect, "Requests").content_rect();
                        let visible = ((content.h.saturating_sub(Self::network_header_h()))
                            / Self::network_row_h())
                        .max(1) as usize;
                        let max_scroll = self
                            .developer_tools
                            .network
                            .entries()
                            .len()
                            .saturating_sub(visible);
                        let mut scroll = self.developer_tools.network.list_scroll_offset();
                        adjust_scroll(&mut scroll, delta, page, home, end, visible, max_scroll);
                        self.developer_tools.network.set_list_scroll_offset(scroll);
                    }
                    NetworkPaneFocus::Details => {
                        let mut scroll = self.developer_tools.network.detail_scroll_offset();
                        let detail_text =
                            self.developer_tools.network.selected_request_detail_text();
                        let view = TextView::new(
                            Panel::with_title(detail_rect, "Details")
                                .content_rect()
                                .inset(8),
                            detail_text,
                        )
                        .with_font(&F_MONO);
                        adjust_scroll(
                            &mut scroll,
                            delta,
                            page,
                            home,
                            end,
                            view.visible_line_count(),
                            view.max_scroll(),
                        );
                        self.developer_tools
                            .network
                            .set_detail_scroll_offset(scroll);
                    }
                }
            }
        }
    }

    fn handle_developer_tools_click(&mut self, point: Point) -> bool {
        let layout = self.developer_panel_layout();
        let Some(panel_rect) = layout.panel_rect else {
            return false;
        };
        let tab_labels = ["Console", "DOM Inspector", "Network"];
        let tab_bar = TabBar::new(
            layout.tab_bar_rect.unwrap(),
            &tab_labels,
            self.developer_tools.panel.active_tab.index(),
        );

        if let Some(tab_index) = tab_bar.hit_test(point.x, point.y) {
            if let Some(tab) = DeveloperToolTab::from_index(tab_index) {
                self.developer_tools.set_active_tab(tab);
                self.focus = FocusPane::DeveloperTools;
                self.clamp_scrolls();
                return true;
            }
        }

        let close_button = Button::secondary(layout.close_button_rect.unwrap(), "Close");
        if close_button.hit_test(point.x, point.y) {
            self.developer_tools.panel.close();
            self.clamp_scrolls();
            return true;
        }

        if !panel_rect.contains(point) {
            return false;
        }

        self.focus = FocusPane::DeveloperTools;

        match self.developer_tools.panel.active_tab {
            DeveloperToolTab::Console => true,
            DeveloperToolTab::DomInspector => {
                self.handle_dom_click(point, layout.content_rect.unwrap())
            }
            DeveloperToolTab::Network => {
                self.handle_network_click(point, layout.content_rect.unwrap())
            }
        }
    }

    fn handle_dom_click(&mut self, point: Point, content_rect: Rect) -> bool {
        let (styles_rect, properties_rect, tree_rect) =
            Self::dom_column_rects(content_rect.inset(8));
        if styles_rect.contains(point) {
            if self.developer_tools.dom.focused_pane() == DomInspectorPane::Styles {
                self.developer_tools.dom.cycle_styles_mode();
            } else {
                self.developer_tools
                    .dom
                    .set_focused_pane(DomInspectorPane::Styles);
            }
            self.developer_tools.dom.styles_text(); // force refresh
            return true;
        }
        if properties_rect.contains(point) {
            self.developer_tools
                .dom
                .set_focused_pane(DomInspectorPane::Properties);
            return true;
        }
        if !tree_rect.contains(point) {
            return false;
        }

        self.developer_tools
            .dom
            .set_focused_pane(DomInspectorPane::Tree);
        let content = Panel::with_title(tree_rect, "DOM Tree")
            .content_rect()
            .inset(8);
        let tree_scroll = self.developer_tools.dom.tree_scroll();
        let rows = self.developer_tools.dom.tree_rows();
        let tree_view = TreeView::new(content, rows)
            .with_scroll_offset(tree_scroll)
            .with_font(&F_MONO);
        if let Some(hit) = tree_view.hit_test(point.x, point.y) {
            match hit.target {
                TreeHitTarget::Disclosure => self.developer_tools.dom.toggle_node(hit.id),
                TreeHitTarget::Row => self.developer_tools.dom.select_node(hit.id),
            }
        }
        self.clamp_scrolls();
        true
    }

    fn handle_network_click(&mut self, point: Point, content_rect: Rect) -> bool {
        let (list_rect, detail_rect) = Self::network_layout(content_rect.inset(8));
        if detail_rect.contains(point) {
            self.developer_tools
                .network
                .set_focused_pane(NetworkPaneFocus::Details);
            return true;
        }
        if !list_rect.contains(point) {
            return false;
        }

        self.developer_tools
            .network
            .set_focused_pane(NetworkPaneFocus::RequestList);
        let content = Panel::with_title(list_rect, "Requests").content_rect();
        let rel_y = point.y - content.y - Self::network_header_h() as i32;
        if rel_y < 0 {
            return true;
        }
        let local_row = (rel_y as u32 / Self::network_row_h()) as usize;
        let row_index = self.developer_tools.network.list_scroll_offset() + local_row;
        if let Some(request_id) = self.developer_tools.network.request_id_at_row(row_index) {
            self.developer_tools
                .network
                .select_request(Some(request_id));
        }
        true
    }
}

#[cfg(feature = "dom")]
fn discover_background_images(
    candidates: &mut Vec<ResourceCandidate>,
    document: &golden_fish::Document,
    styles: &StyleContext,
    sheets: &[Stylesheet],
    document_url: &ParsedUrl,
) {
    // Retain backgrounds from inactive media rules as well: a viewport change
    // can then reuse decoded pixels without doing network work in a reflow.
    for sheet in sheets {
        for rule in &sheet.rules {
            for declaration in &rule.declarations {
                if !matches!(
                    declaration.property,
                    rappid_rabbit::css::Property::BackgroundImage
                        | rappid_rabbit::css::Property::Custom(_)
                ) {
                    continue;
                }
                let raw = &declaration.raw_value;
                let Some(start) = raw.to_ascii_lowercase().find("url(") else {
                    continue;
                };
                let Some(end) = raw[start + 4..].find(')') else {
                    continue;
                };
                if let Some(url) =
                    rappid_rabbit::css::css_image_url(&raw[start..start + 4 + end + 1])
                {
                    push_background_candidate(candidates, document_url, url);
                }
            }
        }
    }
    for node in 0..document.node_count() {
        let Some(raw) = styles
            .style_for(node)
            .and_then(|s| s.value(&rappid_rabbit::css::Property::BackgroundImage))
            .map(|v| v.display())
        else {
            continue;
        };
        let Some(url) = rappid_rabbit::css::css_image_url(&raw) else {
            continue;
        };
        push_background_candidate(candidates, document_url, url);
    }
}

#[cfg(feature = "dom")]
fn push_background_candidate(
    candidates: &mut Vec<ResourceCandidate>,
    document_url: &ParsedUrl,
    url: &str,
) {
    // Ordinary navigation links do not consume the subresource image budget.
    if candidates
        .iter()
        .filter(|c| c.resource_type == ResourceType::Image)
        .count()
        >= rappid_rabbit::MAX_DISCOVERED_RESOURCES
    {
        return;
    }
    let Ok(resolved) = rappid_rabbit::resources::discovery::resolve_url(document_url, url) else {
        return;
    };
    if !candidates
        .iter()
        .any(|c| c.resource_type == ResourceType::Image && c.resolved_url == resolved)
    {
        candidates.push(ResourceCandidate {
            raw_url: String::from(url),
            resolved_url: resolved,
            resource_type: ResourceType::Image,
            classification:
                rappid_rabbit::resources::discovery::DiscoveryClassification::EmbeddedResource,
            enqueue_for_fetch: false,
        });
    }
}

impl App for RabbitApp {
    fn view(&mut self, canvas: &mut Canvas, theme: &Theme) {
        if self.client_bounds.size() != Size::new(canvas.width, canvas.height) {
            let _ = self.set_client_bounds(canvas.width, canvas.height);
        } else {
            let _ = self.ensure_layout();
        }
        canvas.fill_rect(self.layout.root, theme.bg);
        self.sync_document_scroll();
        self.refresh_tab_title();
        self.refresh_cursor();
        self.draw_top_bar(canvas, theme);

        match self.document_view {
            DocumentView::Source => {
                let source_panel = Panel::with_title(self.source_panel_rect(), "Source");
                source_panel.draw(canvas, theme);
                self.source_view().draw(canvas, theme);
            }
            DocumentView::Render => {
                let written = core::cell::Cell::new(0);
                self.document_canvas()
                    .with_image_paint_counter(&written)
                    .draw(canvas, theme);
                self.metrics.repaints += 1;
                #[cfg(feature = "dom")]
                if let Some(state) = self.render_state.as_ref() {
                    if self.metrics.initial_us.is_none() {
                        self.metrics.initial_us =
                            self.metrics.started.map(|s| s.elapsed().as_micros());
                        self.log_presentation_metrics("initial-document");
                    }
                    if written.get() > 0 && self.metrics.first_image_us.is_none() {
                        self.metrics.first_image_us =
                            self.metrics.started.map(|s| s.elapsed().as_micros());
                        self.log_presentation_metrics("first-visible-image");
                    }
                    let viewport = Rect::new(
                        0,
                        self.document_scroll.offset_y,
                        self.latest_viewport.w,
                        self.latest_viewport.h,
                    );
                    let view = (state.scene_generation, viewport);
                    if self.metrics.visible_view != Some(view) {
                        self.metrics.visible_view = Some(view);
                        self.metrics.visible_complete_us = None;
                    }
                    if self.metrics.visible_complete_us.is_none() {
                        let priorities = state.image_resource_priorities(viewport);
                        let pending_visible = rappid_rabbit::images::initial_image_candidates(
                            &self.discovered_resources,
                        )
                        .any(|c| {
                            priorities.get(&c.resolved_url) == Some(&0)
                                && self.image_cache.get(&c.resolved_url).is_none()
                                && ((self.image_attempts.len()
                                    < rappid_rabbit::images::MAX_IMAGE_CACHE_ENTRIES
                                    && !self.image_attempts.contains(&c.resolved_url))
                                    || (self.image_completion.is_some()
                                        && self.image_attempts.last() == Some(&c.resolved_url)))
                        });
                        if !pending_visible
                            && !state.images_need_update()
                            && self
                                .presentation
                                .visible_complete(&state.current_scene, viewport)
                        {
                            if self.metrics.visible_complete_us.is_none() {
                                self.metrics.visible_complete_us =
                                    self.metrics.started.map(|s| s.elapsed().as_micros());
                                self.log_presentation_metrics("visible-images-complete");
                            }
                        } else {
                            self.metrics.visible_complete_us = None;
                        }
                    }
                }
            }
        }

        if self.document_view == DocumentView::Render {
            self.draw_document_scrollbar(canvas, theme);
        }
        self.draw_developer_tools(canvas, theme);
        // TextInput's menu is a window-local overlay.  It must paint after
        // document and developer-tool surfaces rather than as part of the
        // top-bar child draw, or those surfaces conceal its lower rows.
        if !self.url_input.context_menu_open() {
            self.draw_chrome_hint(canvas, theme);
        }
        self.url_input.draw_context_menu(canvas, theme);
        self.draw_shield_settings(canvas, theme);
        self.certificate_panel.draw(
            canvas, theme, self.certificate_panel_rect(), self.connection_security,
            self.cert_info.as_ref(), self.certificate_error.as_deref(),
        );
    }

    fn update(&mut self, event: Event) -> bool {
        if let Event::MouseDown { x, y, button: 0 } = event {
            let point = Point::new(x, y);
            if self.url_site_rect().contains(point) {
                self.certificate_panel.open = !self.certificate_panel.open;
                self.certificate_panel.scroll = 0;
                return true;
            }
            if self.certificate_panel.open {
                let rect = self.certificate_panel_rect();
                if sunlight_ui::widgets::CertificatePanel::details_rect(rect).contains(point) {
                    self.certificate_panel.expanded = !self.certificate_panel.expanded;
                    self.certificate_panel.scroll = 0;
                } else if !rect.contains(point) {
                    self.certificate_panel.open = false;
                }
                return true;
            }
        }
        if self.certificate_panel.open {
            if let Event::MouseWheel { x, y, delta } = event {
                if self.certificate_panel_rect().contains(Point::new(x, y)) {
                    let count = self
                        .certificate_panel
                        .lines(
                            self.certificate_panel_rect().w,
                            self.connection_security,
                            self.cert_info.as_ref(),
                            self.certificate_error.as_deref(),
                        )
                        .len();
                    if delta < 0 {
                        self.certificate_panel.scroll =
                            (self.certificate_panel.scroll + 1).min(count.saturating_sub(1));
                    } else {
                        self.certificate_panel.scroll =
                            self.certificate_panel.scroll.saturating_sub(1);
                    }
                    return true;
                }
            }
            if matches!(
                event,
                Event::Key('\u{1b}') | Event::FocusChanged { focused: false }
            ) {
                self.certificate_panel.open = false;
                return true;
            }
            if event
                .pos()
                .is_some_and(|p| self.certificate_panel_rect().contains(p))
            {
                return true;
            }
        }


        let previous_pointer = self.pointer;
        let previous_cursor = self.cursor;
        let previous_title_revision = self.title_revision;
        let previous_hover = self.document_scroll.hovered;
        if let Some(point) = event.pos() {
            self.pointer = Some(point);
        }
        if matches!(
            event,
            Event::PointerOwnership {
                owned: false,
                captured: false
            } | Event::FocusChanged { focused: false }
        ) {
            self.pointer = None;
            self.document_scroll.end_drag();
            self.suppress_scroll_click = false;
        }
        self.sync_document_scroll();
        let changed = self.update_event(event);
        self.sync_document_scroll();
        self.refresh_tab_title();
        self.refresh_cursor();
        // Publish after all widget dispatch. Painting must not leave a cursor
        // request queued into the next event (Pointer is a fallback request).
        set_client_cursor(self.cursor);
        changed
            || previous_title_revision != self.title_revision
            || previous_cursor != self.cursor
            || previous_hover != self.document_scroll.hovered
            || (matches!(event, Event::MouseMove { .. })
                && (previous_pointer.is_some_and(|p| self.top_bar_rect().contains(p))
                    || self
                        .pointer
                        .is_some_and(|p| self.top_bar_rect().contains(p))))
    }

    fn window_event(&mut self, event: WindowEvent) -> bool {
        let WindowEvent::Resized { width, height } = event;
        self.set_client_bounds(width, height)
    }
}

impl RabbitApp {
    fn update_event(&mut self, event: Event) -> bool {
        self.url_input.rect = self.url_text_rect();
        if self.url_input.context_menu_open() {
            return self.url_input.update(event);
        }
        if self.update_shield_ui(event) { return true; }

        if self.handle_document_scroll(event) {
            return true;
        }

        if let Event::Click { x, y } = event {
            if !self.navigation_draining && self.url_action_rect().contains(Point::new(x, y)) {
                self.url_input.active = false;
                if self.pending_fetch {
                    self.stop_navigation();
                } else {
                    self.reload();
                }
                return true;
            }
        }

        if self.url_input.active && matches!(event, Event::Key('\n') | Event::Key('\r')) {
            self.queue_fetch();
            return true;
        }
        let url_was_active = self.url_input.active;
        let url_changed = self.url_input.update(event);
        if url_changed && (!matches!(event, Event::Click { .. }) || self.url_input.active) {
            return true;
        }
        #[cfg(feature = "dom")]
        if !url_was_active
            && self.document_view == DocumentView::Render
            && self.handle_form_key(event)
        {
            return true;
        }

        match event {
            Event::Tick => {
                if self.navigation_draining {
                    return self.poll_stopped_navigation();
                }
                if self.pending_fetch {
                    let changed = self.perform_pending_fetch();
                    if changed {
                        self.clamp_scrolls();
                    }
                    return changed;
                }
                #[cfg(feature = "dom")]
                {
                    let mut changed = self.poll_images();
                    changed |= self.poll_reflow();
                    if let Some(state) = self.render_state.as_ref() {
                        let viewport = Rect::new(
                            0,
                            self.document_scroll.offset_y,
                            self.latest_viewport.w,
                            self.latest_viewport.h,
                        );
                        changed |= self.presentation.advance(
                            &state.current_scene,
                            state.scene_generation,
                            viewport,
                        );
                    }
                    return changed;
                }
                #[cfg(not(feature = "dom"))]
                false
            }
            Event::MouseDown { x, y, button: 0 } => {
                #[cfg(feature = "dom")]
                if self.document_view == DocumentView::Render {
                    let canvas = self.document_canvas();
                    if let Some(object) = canvas.hit_test(Point::new(x, y)) {
                        if let Some(RenderInteraction::Control { owner_node_id }) =
                            object.interaction.clone()
                        {
                            self.form_state.pressed_control = Some(owner_node_id);
                            return true;
                        }
                    }
                }
                let layout = self.developer_panel_layout();
                if self
                    .developer_tools
                    .panel
                    .begin_resize(Point::new(x, y), layout)
                {
                    return true;
                }
                false
            }
            Event::MouseMove { x, y } => {
                let url_hover_changed = self.update_url_chrome_hover(x, y);
                let changed = self
                    .developer_tools
                    .panel
                    .update_resize(y, self.content_rect());
                if changed {
                    #[cfg(feature = "dom")]
                    self.refresh_render_viewport();
                    self.clamp_scrolls();
                }
                changed || url_hover_changed
            }
            Event::MouseUp { x, y, .. } => {
                #[cfg(feature = "dom")]
                if self.document_view == DocumentView::Render
                    && self.form_state.pressed_control.is_some()
                {
                    let canvas = self.document_canvas();
                    let same_control = canvas.hit_test(Point::new(x, y)).and_then(|object| {
                        match object.interaction.clone() {
                            Some(RenderInteraction::Control { owner_node_id }) => {
                                Some(owner_node_id)
                            }
                            _ => None,
                        }
                    }) == self.form_state.pressed_control;
                    if !same_control {
                        self.form_state.pressed_control = None;
                    }
                }
                let was_resizing = self.developer_tools.panel.is_resizing();
                self.developer_tools.panel.finish_resize();
                was_resizing
            }
            Event::PointerOwnership { owned: false, .. } => {
                let changed = self.url_hovered || self.url_action_hovered;
                self.url_hovered = false;
                self.url_action_hovered = false;
                changed
            }
            Event::Click { x, y } => {
                let point = Point::new(x, y);

                let devtools_label = if self.developer_tools.panel.open {
                    "Hide Tools"
                } else {
                    "DevTools"
                };
                let devtools_button =
                    Button::secondary(self.devtools_button_rect(), devtools_label);
                if devtools_button.hit_test(x, y) {
                    self.toggle_developer_tools();
                    return true;
                }

                let view_label = match self.document_view {
                    DocumentView::Source => "Render",
                    DocumentView::Render => "Source",
                };
                if Button::secondary(self.view_button_rect(), view_label).hit_test(x, y) {
                    self.document_view = match self.document_view {
                        DocumentView::Source => DocumentView::Render,
                        DocumentView::Render => DocumentView::Source,
                    };
                    self.focus = match self.document_view {
                        DocumentView::Source => FocusPane::Source,
                        DocumentView::Render => FocusPane::Render,
                    };
                    #[cfg(feature = "dom")]
                    self.refresh_render_viewport();
                    self.clamp_scrolls();
                    return true;
                }

                #[cfg(feature = "dom")]
                if self.handle_form_control_click(point) || self.handle_render_click(point) {
                    return true;
                }

                #[cfg(feature = "dom")]
                if self.document_view == DocumentView::Render
                    && self.form_state.focused_control.is_some()
                {
                    self.form_state.blur();
                    self.patch_all_controls();
                }

                if self.handle_developer_tools_click(point) {
                    return true;
                }

                if self.source_panel_rect().contains(point) {
                    self.focus = match self.document_view {
                        DocumentView::Source => FocusPane::Source,
                        DocumentView::Render => FocusPane::Render,
                    };
                    return true;
                }
                false
            }
            // Some keyboard backends surface Ctrl+L as ASCII form-feed while
            // others retain the raw keycode/modifier tuple below.
            Event::Key('\u{c}') => {
                self.url_input.set_text("");
                self.url_input.active = true;
                true
            }
            Event::Key('\n') | Event::Key(' ')
                if self.focus == FocusPane::DeveloperTools
                    && self.developer_tools.panel.active_tab == DeveloperToolTab::DomInspector
                    && self.developer_tools.dom.focused_pane() == DomInspectorPane::Tree =>
            {
                let Some(content_rect) = self.developer_panel_layout().content_rect else {
                    return false;
                };
                let (_, _, tree_rect) = Self::dom_column_rects(content_rect.inset(8));
                let visible_rows = self.dom_tree_visible_rows(tree_rect);
                self.developer_tools
                    .dom
                    .toggle_selected_tree_node(visible_rows)
            }
            Event::KeyPress {
                keycode,
                pressed: true,
                ctrl,
                ..
            } => {
                if keycode == 0x58 {
                    self.toggle_developer_tools();
                    return true;
                }
                if ctrl && keycode == 0x13 {
                    self.reload();
                    return true;
                }
                if ctrl && keycode == KEY_Q {
                    request_close();
                    return true;
                }
                if ctrl && keycode == KEY_L {
                    self.url_input.set_text("");
                    self.url_input.active = true;
                    return true;
                }
                if keycode == KEY_F6 {
                    self.url_input.set_text("");
                    self.url_input.active = true;
                    return true;
                }
                if self.focus == FocusPane::DeveloperTools
                    && self.developer_tools.panel.active_tab == DeveloperToolTab::DomInspector
                    && self.developer_tools.dom.focused_pane() == DomInspectorPane::Tree
                {
                    let Some(content_rect) = self.developer_panel_layout().content_rect else {
                        return false;
                    };
                    let (_, _, tree_rect) = Self::dom_column_rects(content_rect.inset(8));
                    let visible_rows = self.dom_tree_visible_rows(tree_rect);
                    match keycode {
                        KEY_UP => {
                            return self
                                .developer_tools
                                .dom
                                .move_tree_selection(-1, visible_rows)
                        }
                        KEY_DOWN => {
                            return self
                                .developer_tools
                                .dom
                                .move_tree_selection(1, visible_rows)
                        }
                        KEY_LEFT => return self.developer_tools.dom.tree_left(visible_rows),
                        KEY_RIGHT => return self.developer_tools.dom.tree_right(visible_rows),
                        _ => {}
                    }
                }
                if self.url_input.active {
                    return url_changed;
                }
                match keycode {
                    KEY_UP => self.scroll_focused(-1, false, false, false),
                    KEY_DOWN => self.scroll_focused(1, false, false, false),
                    KEY_PGUP => self.scroll_focused(-1, true, false, false),
                    KEY_PGDN => self.scroll_focused(1, true, false, false),
                    KEY_HOME => self.scroll_focused(0, false, true, false),
                    KEY_END => self.scroll_focused(0, false, false, true),
                    _ => return false,
                }
                true
            }
            _ => false,
        }
    }
}

// Explicit diagnostic capture stays on the document worker. Disabled by default;
// it never selects a renderer or changes document/network ownership.
static TRACE_DOCUMENT: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

fn trace_resource(kind: &str, url: &str, bytes: &[u8]) {
    if !TRACE_DOCUMENT.load(core::sync::atomic::Ordering::Relaxed) {
        return;
    }
    debug_log(&format!(
        "[RABBIT][CAPTURE] begin kind={kind} bytes={} url={url}",
        bytes.len()
    ));
    for (i, chunk) in bytes.chunks(64).enumerate() {
        let mut line = String::with_capacity(320);
        let _ = write!(line, "[RABBIT][CAPTURE] data offset={} hex=", i * 64);
        for byte in chunk {
            let _ = write!(line, "{byte:02x}");
        }
        debug_log(&line);
    }
    debug_log("[RABBIT][CAPTURE] end");
}

#[cfg(feature = "dom")]
fn trace_layout(state: &DocumentRenderState) {
    use rappid_rabbit::css::Property;
    debug_log(&format!(
        "[RABBIT][TREE] begin url={} viewport={}x{}",
        state.final_url, state.viewport.w, state.viewport.h
    ));
    for g in &state.layout_geometry {
        let id = g.node_id;
        let mut line = format!(
            "[RABBIT][TREE] node={} parent={:?} tag={:?} attrs={:?} content={:?} border={:?}",
            id,
            state.dom.parent(id),
            state.dom.tag_name(id),
            state.dom.attributes(id),
            g.content_box,
            g.border_box
        );
        if let Some(style) = state.styles.style_for(id) {
            for property in [
                Property::Display,
                Property::Float,
                Property::Clear,
                Property::Width,
                Property::Height,
                Property::MarginLeft,
                Property::MarginRight,
                Property::MarginTop,
                Property::MarginBottom,
                Property::AlignItems,
                Property::JustifyContent,
                Property::BackgroundColor,
            ] {
                if let Some(value) = style.value(&property) {
                    let _ = write!(line, " {}={}", property.name(), value.display());
                }
            }
        }
        // Serial debug messages are bounded by the syscall. Chunk long attribute
        // values without splitting UTF-8, keeping every DOM/layout record.
        while line.len() > 200 {
            let mut end = 200;
            while !line.is_char_boundary(end) {
                end -= 1;
            }
            debug_log(&line[..end]);
            line.drain(..end);
        }
        debug_log(&line);
    }
    debug_log("[RABBIT][TREE] end");
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(feature = "js-dom-fixture")]
    {
        let args:Vec<String>=std::env::args().collect();
        if args.get(1).map(String::as_str)==Some("--shield-probe") {
            return shield_probe::run(args.get(2).ok_or("fixture URL required")?);
        }
    }
    #[cfg(feature = "js-dom-fixture")]
    if std::env::args().any(|arg| arg == "--account-fixture") {
        return rappid_rabbit::account_fixture::run();
    }
    let arguments: Vec<String> = std::env::args().collect();
    let broker_url = if arguments.get(1).map(String::as_str) == Some("--account-broker") {
        let id = arguments
            .get(2)
            .and_then(|s| s.parse::<u64>().ok())
            .ok_or("invalid broker flow")?;
        let cap = sunlight_ipc::nameserver_lookup("sunlight-account-broker")
            .ok_or("broker unavailable")?;
        let reply = sunlight_ipc::ipc_call_timeout(
            cap,
            sunlight_ipc::IpcMsg::with_label(0xAB05).word(0, id),
            2000,
        )
        .map_err(|_| "broker claim timeout")?;
        if reply.label != 0xABFF
            || reply.words[0] != 0
            || reply.words[1] == 0
            || reply.words[1] > 2048
        {
            return Err("broker claim denied".into());
        }
        let token = reply.caps[0];
        let ptr = sunlight_ipc::shm_map(token).map_err(|_| "broker URL transfer failed")?;
        let bytes = unsafe { core::slice::from_raw_parts(ptr, reply.words[1] as usize) };
        let url = core::str::from_utf8(bytes)
            .map(String::from)
            .map_err(|_| "invalid broker URL");
        let _ = sunlight_ipc::shm_free(token);
        let _ = sunlight_ipc::ipc_call_timeout(
            cap,
            sunlight_ipc::IpcMsg::with_label(0xAB08).word(0, id),
            2000,
        );
        let url = url?;
        if !url.starts_with("https://") {
            return Err("broker URL must use HTTPS".into());
        }
        debug_log("[RABBIT] native broker authorization URL received");
        Some(url)
    } else {
        None
    };
    let mut initial_url = broker_url;
    for argument in std::env::args().skip(1) {
        if let Some(trace) = launch_trace::parse_launch_arg(argument.as_bytes()) {
            launch_trace::set_current(trace);
        } else if argument == "--trace-document" {
            TRACE_DOCUMENT.store(true, core::sync::atomic::Ordering::Relaxed);
        } else if initial_url.is_none() {
            initial_url =
                rappid_rabbit::launch_url::initial_url_from_values(&[argument.as_bytes()])
                    .map(String::from);
        }
    }
    let trace = launch_trace::current().unwrap_or(LaunchTrace::new(0, LaunchSource::Unknown, 0));
    launch_trace::log_phase_now(
        trace,
        "app=rappid-rabbit",
        "app_main_started",
        Some(sunlight_ipc::getpid()),
    );
    // No socket reactor: SunlightOS networking uses the capability IPC backend.
    // Native std supplies threads, parking, TLS and monotonic time to Tokio.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .max_blocking_threads(1)
        .thread_stack_size(4 * 1024 * 1024)
        .build()?;
    // Resource IO/decode cannot hold up a navigation or viewport reflow.
    // Layout/DOM still use only the original single blocking worker.
    #[cfg(feature = "dom")]
    let image_runtime = tokio::runtime::Builder::new_current_thread()
        .max_blocking_threads(1)
        .thread_stack_size(4 * 1024 * 1024)
        .build()?;
    let mut app = RabbitApp::new(initial_url.as_deref());
    if let Some(settings) = rappid_rabbit::content_shield::load_settings() {
        app.shield.settings = settings;
        app.shield.changed();
        app.shield_persistence_status = "Preferences restored".into();
    }
    app.runtime = Some(runtime.handle().clone());
    if initial_url.is_none() {
        app.queue_fetch();
    }
    #[cfg(feature = "dom")]
    {
        app.image_runtime = Some(image_runtime.handle().clone());
    }
    let mut window = Window::connect(WindowConfig {
        width: WIN_W,
        height: WIN_H,
        title: "Rappid Rabbit",
        decoration: WindowDecoration::Normal,
    })
    .ok_or("failed to connect window")?;
    window.run(&mut app);
    drop(app);
    // Blocking IPC cannot be aborted by Tokio. Do not hold window close up
    // waiting for a network timeout; native std group exit reaps all workers.
    runtime.shutdown_timeout(std::time::Duration::ZERO);
    #[cfg(feature = "dom")]
    image_runtime.shutdown_timeout(std::time::Duration::ZERO);
    Ok(())
}

fn adjust_scroll(
    scroll: &mut usize,
    delta: i32,
    page: bool,
    home: bool,
    end: bool,
    visible: usize,
    max_scroll: usize,
) {
    if home {
        *scroll = 0;
        return;
    }
    if end {
        *scroll = max_scroll;
        return;
    }
    let step = if page { visible.max(1) } else { 1 };
    if delta < 0 {
        *scroll = scroll.saturating_sub(step);
    } else if delta > 0 {
        *scroll = (*scroll + step).min(max_scroll);
    }
}

#[cfg(test)]
mod responsive_tests {
    use super::{RabbitApp, Rect, WIN_H, WIN_W};

    #[test]
    fn toolbar_and_page_area_follow_live_root() {
        let initial = RabbitApp::compute_layout(Rect::new(0, 0, WIN_W, WIN_H));
        let resized = RabbitApp::compute_layout(Rect::new(0, 0, 1360, 880));
        assert_eq!(initial.root.w, WIN_W);
        assert_eq!(resized.root.w, 1360);
        assert_eq!(resized.top_bar.w, 1360);
        assert_eq!(resized.content.w, resized.top_bar.w);
        assert_eq!(resized.content.bottom(), resized.root.bottom());
        assert!(resized.content.h > initial.content.h);
    }

    #[test]
    fn address_field_is_the_horizontal_fill_participant() {
        let narrow = RabbitApp::compute_layout(Rect::new(0, 0, 820, 720));
        let wide = RabbitApp::compute_layout(Rect::new(0, 0, 1280, 720));
        assert_eq!(narrow.back.w, wide.back.w);
        assert_eq!(narrow.forward.w, wide.forward.w);
        assert_eq!(narrow.devtools.w, wide.devtools.w);
        assert_eq!(wide.url_bar.w - narrow.url_bar.w, 460);
    }

    #[test]
    fn resize_updates_browser_viewport_without_queuing_navigation() {
        let mut app = RabbitApp::new(None);
        let initial = app.render_viewport();
        let pending_fetch = app.pending_fetch;
        let request = app.active_navigation_request_id;
        assert!(app.set_client_bounds(840, 540));
        let smaller = app.render_viewport();
        assert!(smaller.w < initial.w && smaller.h < initial.h);
        assert_eq!(app.pending_fetch, pending_fetch);
        assert_eq!(app.active_navigation_request_id, request);
        assert!(!app.set_client_bounds(840, 540));
    }

    #[test]
    fn zero_and_tiny_roots_produce_zero_page_area_safely() {
        for root in [Rect::new(0, 0, 0, 0), Rect::new(0, 0, 1, 1)] {
            let layout = RabbitApp::compute_layout(root);
            assert_eq!(layout.root, root);
            assert_eq!(layout.content.h, 0);
            assert!(layout.content.w <= root.w);
            assert_eq!(layout.url_bar.w, 0);
        }
    }

    #[test]
    fn grow_shrink_grow_is_deterministic() {
        let large = Rect::new(0, 0, 1280, 800);
        let first = RabbitApp::compute_layout(large);
        let _ = RabbitApp::compute_layout(Rect::new(0, 0, 600, 240));
        assert_eq!(first, RabbitApp::compute_layout(large));
    }
}

#[cfg(all(test, feature = "dom"))]
mod navigation_tests {
    use super::*;

    #[test]
    fn start_page_renders_from_compiled_dom_without_network_request() {
        let mut worker = RabbitApp::new(None);
        worker.url_input.set_text("rappid:startpage");
        worker.navigation_url = String::from("rappid:startpage");
        worker.begin_internal_navigation_session("rappid:startpage");

        worker.perform_fetch_sync();

        let render = worker.render_state.as_ref().expect("start page scene");
        assert_eq!(worker.status, "Internal page");
        assert_eq!(worker.navigation_url, "rappid:startpage");
        assert_eq!(worker.active_navigation_request_id, None);
        assert!(worker.developer_tools.network.entries().is_empty());
        assert!(render.diagnostics.object_count > 0);
        assert!(render.diagnostics.link_count >= 3);
        assert!(render.dom.find_first_element("input").is_some());
    }

    #[test]
    fn unknown_rappid_url_is_reported_without_network_request() {
        let mut worker = RabbitApp::new(None);
        worker.url_input.set_text("rappid:unknown");
        worker.navigation_url = String::from("rappid:unknown");
        worker.begin_internal_navigation_session("rappid:unknown");

        worker.perform_fetch_sync();

        assert_eq!(worker.status, "Unknown internal page");
        assert_eq!(worker.active_navigation_request_id, None);
        assert!(worker.developer_tools.network.entries().is_empty());
        assert!(worker.render_state.is_none());
        assert!(worker.render_status.contains("rappid:unknown"));
    }

    fn document_output() -> NavigationOutput {
        let mut worker = RabbitApp::new(None);
        let url = "http://example.com/";
        let parsed = ParsedUrl::parse(url).unwrap();
        worker.begin_navigation_session(url, &build_get_request(&parsed));
        let body = format!(
            "<html><body>{}</body></html>",
            "<p>Rabbit worker document</p>".repeat(1000)
        );
        worker.apply_result(
            url,
            RequestResult {
                connection_security: sunlight_cert::ConnectionSecurity::NotSecure,
                cert_info: None,
                response: sunlight_http::HttpResponse {
                    status_code: 200,
                    status_text: String::from("OK"),
                    headers: vec![(String::from("content-type"), String::from("text/html"))],
                    header_len: 0,
                },
                body: body.into_bytes(),
                final_url: Some(parsed),
                duration_ms: Some(1),
            },
        );
        worker.into_navigation_output()
    }

    #[test]
    fn large_document_crosses_worker_channel_while_ui_keeps_running() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .max_blocking_threads(1)
            .build()
            .unwrap();
        let (sender, receiver) = oneshot::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let ui_thread = std::thread::current().id();
        runtime.spawn_blocking(move || {
            assert_ne!(ui_thread, std::thread::current().id());
            wait.recv().unwrap();
            assert!(sender.send(document_output()).is_ok());
        });
        let mut app = RabbitApp::new(None);
        app.document_lifecycle.begin_navigation();
        app.pending_fetch = true;
        app.completion = Some(receiver);
        app.navigation_url = String::from("http://example.com/");
        app.url_input.set_text("http://edited.example/");
        app.developer_tools.panel.open();
        app.perform_pending_fetch(); // Empty channel must not wait for the worker.
        assert!(app.pending_fetch);
        app.set_client_bounds(840, 540);
        release.send(()).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.pending_fetch {
            assert!(std::time::Instant::now() < deadline);
            app.perform_pending_fetch();
            std::thread::yield_now();
        }
        assert_eq!(app.url_input.value(), "http://edited.example/");
        assert!(app.developer_tools.panel.open);
        let state = app.render_state.as_ref().unwrap();
        assert!(state.dom.node_count() > 900);
        assert_eq!(app.document_lifecycle.parse_attempts(), 1);
    }

    #[test]
    fn stale_completion_and_closed_worker_do_not_replace_document() {
        let mut app = RabbitApp::new(None);
        app.document_lifecycle.begin_navigation();
        app.document_lifecycle.begin_navigation();
        let (sender, receiver) = oneshot::channel();
        assert!(sender.send(document_output()).is_ok());
        app.completion = Some(receiver);
        app.pending_fetch = true;
        app.perform_pending_fetch();
        assert!(app.render_state.is_none());
        assert_eq!(app.document_lifecycle.generation(), 2);
        let (sender, receiver) = oneshot::channel();
        drop(sender);
        app.completion = Some(receiver);
        app.pending_fetch = true;
        app.perform_pending_fetch();
        assert!(!app.pending_fetch);
        assert!(app.completion.is_none());
        assert!(app.source.as_str().contains("worker stopped"));
    }

    #[test]
    fn stale_image_pixels_are_dropped_even_when_loading_the_same_url_again() {
        let mut output = document_output();
        let decoded = decode_image(
            include_bytes!("../../sun-img/tests/fixtures/gray.jpg"),
            None,
            "photo.jpg",
        )
        .unwrap();
        let weak = std::sync::Arc::downgrade(&decoded.image);
        output
            .image_cache
            .insert_decoded("http://example.com/photo.jpg".into(), decoded);
        let mut app = RabbitApp::new(None);
        app.document_lifecycle.begin_navigation();
        app.document_lifecycle.begin_navigation(); // same URL, new generation
        let (sender, receiver) = oneshot::channel();
        assert!(sender.send(output).is_ok());
        app.completion = Some(receiver);
        app.pending_fetch = true;
        assert!(app.perform_pending_fetch());
        assert!(app
            .image_cache
            .get("http://example.com/photo.jpg")
            .is_none());
        assert!(weak.upgrade().is_none());
    }
}

#[cfg(all(test, feature = "dom"))]
mod view_and_reflow_tests {
    use super::*;
    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .max_blocking_threads(1)
            .build()
            .unwrap()
    }
    fn app() -> RabbitApp {
        let mut app = RabbitApp::new(None);
        app.document_lifecycle.begin_navigation();
        let dom = golden_fish::parse_html(
            "<style>body{margin:0}p{width:100%}</style><p>owned worker reflow text</p>",
        )
        .unwrap();
        let styles = StyleContext::build(&dom, &collect_embedded_stylesheets(&dom));
        let viewport = app.render_viewport();
        app.render_state = Some(DocumentRenderState::new(
            1,
            "https://example.com/".into(),
            dom,
            styles,
            viewport,
            &RABBIT_FONTS,
        ));
        app.refresh_render_viewport();
        app
    }
    fn click_view(app: &mut RabbitApp) {
        let rect = app.view_button_rect();
        app.update(Event::Click {
            x: rect.x + 1,
            y: rect.y + 1,
        });
    }
    fn click_tools(app: &mut RabbitApp) {
        let rect = app.devtools_button_rect();
        app.update(Event::Click {
            x: rect.x + 1,
            y: rect.y + 1,
        });
    }
    fn settle(app: &mut RabbitApp) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            app.poll_reflow();
            if app.reflow_completion.is_none()
                && app.render_state.as_ref().unwrap().viewport == app.latest_viewport
            {
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
    }
    #[test]
    fn startup_and_explicit_view_actions() {
        let mut app = RabbitApp::new(None);
        assert_eq!(app.document_view, DocumentView::Render);
        assert_eq!(app.focus, FocusPane::Render);
        assert!(!app.developer_tools.panel.open);
        click_view(&mut app);
        assert_eq!(app.document_view, DocumentView::Source);
        click_tools(&mut app);
        assert_eq!(app.focus, FocusPane::DeveloperTools);
        assert!(app.developer_tools.panel.open);
        click_tools(&mut app);
        assert_eq!(app.focus, FocusPane::Source);
        click_view(&mut app);
        assert_eq!(app.document_view, DocumentView::Render);
    }
    #[test]
    fn navigation_opens_render_but_success_and_failure_preserve_deliberate_views() {
        for success in [true, false] {
            for chosen_view in [DocumentView::Source, DocumentView::Render] {
                let mut app = app();
                app.document_view = DocumentView::Source;
                app.queue_fetch();
                assert_eq!(app.document_view, DocumentView::Render);
                app.document_view = chosen_view;
                click_tools(&mut app);
                let mut worker = RabbitApp::new(None);
                worker.begin_navigation_session(
                    "http://example.com/",
                    &build_get_request(&ParsedUrl::parse("http://example.com/").unwrap()),
                );
                if success {
                    let body = include_bytes!("../../golden-fish/tests/fixtures/example-live.html")
                        .to_vec();
                    worker.apply_result(
                        "http://example.com/",
                        RequestResult {
                            connection_security: sunlight_cert::ConnectionSecurity::NotSecure,
                            cert_info: None,
                            response: sunlight_http::HttpResponse {
                                status_code: 200,
                                status_text: "OK".into(),
                                headers: vec![("content-type".into(), "text/html".into())],
                                header_len: 0,
                            },
                            body,
                            final_url: Some(ParsedUrl::parse("http://example.com/").unwrap()),
                            duration_ms: Some(1),
                        },
                    );
                } else {
                    worker.apply_fetch_error(FetchError::InvalidUrl("failed".into()));
                }
                let (send, recv) = oneshot::channel();
                assert!(send.send(worker.into_navigation_output()).is_ok());
                app.completion = Some(recv);
                app.perform_pending_fetch();
                assert_eq!(app.document_view, chosen_view);
                assert_eq!(app.focus, FocusPane::DeveloperTools);
                assert!(app.developer_tools.panel.open);
                assert_eq!(app.render_state.is_some(), success);
                app.queue_fetch();
                assert_eq!(app.document_view, DocumentView::Render);
            }
        }
    }
    #[test]
    fn rapid_resize_coalesces_and_commits_final_geometry_on_ui_thread() {
        let runtime = runtime();
        let mut app = app();
        app.runtime = Some(runtime.handle().clone());
        let initial_scene = app.render_state.as_ref().unwrap().current_scene.clone();
        for width in 700..900 {
            app.set_client_bounds(width, 500);
        }
        assert!(app.reflow_completion.is_none());
        assert!(std::sync::Arc::ptr_eq(
            &initial_scene,
            &app.render_state.as_ref().unwrap().current_scene
        ));
        app.poll_reflow();
        for width in 900..1000 {
            app.set_client_bounds(width, 600);
        }
        settle(&mut app);
        let state = app.render_state.as_ref().unwrap();
        assert_eq!(state.viewport, app.latest_viewport);
        assert_eq!(state.current_scene.viewport_size, state.viewport);
        assert_eq!(state.scene_generation, 2); // discarded layouts never publish
        let paragraph = state.dom.find_first_element("p").unwrap();
        assert_eq!(
            state
                .layout_geometry
                .iter()
                .find(|g| g.node_id == paragraph)
                .unwrap()
                .content_box
                .width,
            state.viewport.w as f32
        );
    }
    #[test]
    #[cfg(feature = "js-dom-fixture")]
    fn dom_mutation_rejects_a_resize_completion_from_the_previous_revision() {
        let mut app = app();
        let viewport = app.latest_viewport;
        let dom = golden_fish::parse_html("<p id='counter'>Old</p>").unwrap();
        let styles = StyleContext::build(&dom, &[]);
        app.render_state = Some(DocumentRenderState::new(
            1,
            "fixture:stale-worker".into(),
            dom,
            styles,
            viewport,
            &RABBIT_FONTS,
        ));
        let mut old = app.render_state.as_ref().unwrap().reflow_snapshot();
        old.rebuild(viewport, &RABBIT_FONTS);
        let result = rappid_rabbit::dom_fixture::run_fixture_script(
            app.render_state.as_mut().unwrap(),
            "document.getElementById('counter').textContent='New Text';",
            "stale-worker.js",
            &RABBIT_FONTS,
        );
        result.evaluation.unwrap();
        assert!(result.repaint_requested);
        let scene = app.render_state.as_ref().unwrap().current_scene.clone();
        let (sender, receiver) = oneshot::channel();
        app.reflow_completion = Some(receiver);
        assert!(sender
            .send(ReflowOutput {
                version: app.viewport_version,
                state: old
            })
            .is_ok());
        assert!(!app.poll_reflow());
        let state = app.render_state.as_ref().unwrap();
        assert_eq!(state.dom_revision, 1);
        assert!(std::sync::Arc::ptr_eq(&scene, &state.current_scene));
        let counter = state.dom.get_element_by_id("counter").unwrap();
        assert_eq!(
            state.dom.element_text_content(counter).as_deref(),
            Some("New Text")
        );
    }

    #[test]
    fn navigation_and_late_images_invalidate_inflight_results() {
        let runtime = runtime();
        let mut app = app();
        app.runtime = Some(runtime.handle().clone());
        let mut old = app.render_state.as_ref().unwrap().reflow_snapshot();
        app.set_client_bounds(800, 500);
        old.rebuild_for_viewport(app.latest_viewport, &RABBIT_FONTS);
        let (send, recv) = oneshot::channel();
        app.reflow_completion = Some(recv);
        let decoded = decode_image(
            include_bytes!("../../sun-img/tests/fixtures/gray.jpg"),
            None,
            "a.jpg",
        )
        .unwrap();
        let mut images = ImageCache::default();
        images.insert_decoded("https://example.com/a.jpg".into(), decoded);
        app.render_state
            .as_mut()
            .unwrap()
            .complete_images(1, &RABBIT_FONTS, images);
        assert!(send
            .send(ReflowOutput {
                version: app.viewport_version,
                state: old
            })
            .is_ok());
        app.poll_reflow();
        settle(&mut app);
        assert_eq!(app.render_state.as_ref().unwrap().image_revision, 1);
        app.set_client_bounds(1000, 600);
        app.poll_reflow();
        let parsed = ParsedUrl::parse("http://other.example/").unwrap();
        app.begin_navigation_session("http://other.example/", &build_get_request(&parsed));
        assert!(app.reflow_completion.is_none());
        assert!(app.render_state.is_none());
        assert_eq!(app.document_lifecycle.generation(), 2);
        runtime.shutdown_timeout(std::time::Duration::from_secs(1));
    }
    #[test]
    fn media_rules_recompute_for_latest_viewport() {
        let mut app = app();
        let runtime = runtime();
        app.runtime = Some(runtime.handle().clone());
        let state = app.render_state.as_mut().unwrap();
        state.styles = std::sync::Arc::new(StyleContext::build(
            &state.dom,
            &[parse_stylesheet(
                "p{width:100px}@media(max-width:600px){p{width:50px}}",
                StylesheetSource::Embedded,
            )],
        ));
        app.set_client_bounds(500, 400);
        settle(&mut app);
        assert_eq!(
            app.render_state
                .as_ref()
                .unwrap()
                .layout_geometry
                .iter()
                .find(|g| app.render_state.as_ref().unwrap().dom.tag_name(g.node_id) == Some("p"))
                .unwrap()
                .content_box
                .width,
            50.0
        );
        let first_styles = app.render_state.as_ref().unwrap().styles.clone();
        app.set_client_bounds(450, 400);
        settle(&mut app);
        assert!(std::sync::Arc::ptr_eq(
            &first_styles,
            &app.render_state.as_ref().unwrap().styles
        ));
        app.set_client_bounds(900, 600);
        settle(&mut app);
        assert!(!std::sync::Arc::ptr_eq(
            &first_styles,
            &app.render_state.as_ref().unwrap().styles
        ));
    }

    #[test]
    fn background_discovery_preloads_media_resources_and_deduplicates() {
        let dom = golden_fish::parse_html(&format!(
            "{}<div style='background-image:url(../inline.svg)'></div>",
            (0..100)
                .map(|i| format!("<a href='/page{i}'>link</a>"))
                .collect::<String>()
        ))
        .unwrap();
        let sheets = vec![parse_stylesheet(
            "@media(max-width:600px){div{background-image:url(../img/mobile.svg)}}",
            StylesheetSource::External("https://cdn.example/css/nav.css".into()),
        )];
        let styles = StyleContext::build_with_viewport(&dom, &sheets, 1000);
        let base = ParsedUrl::parse("https://document.example/pages/index.html").unwrap();
        let mut candidates = rappid_rabbit::resources::discovery::discover_resources(&dom, &base);
        assert!(candidates.len() > rappid_rabbit::MAX_DISCOVERED_RESOURCES);
        discover_background_images(&mut candidates, &dom, &styles, &sheets, &base);
        discover_background_images(&mut candidates, &dom, &styles, &sheets, &base);
        assert_eq!(
            candidates
                .iter()
                .filter(|c| c.resource_type == ResourceType::Image)
                .count(),
            2
        );
        assert!(candidates
            .iter()
            .any(|c| c.resolved_url == "https://cdn.example/img/mobile.svg"));
        assert!(candidates
            .iter()
            .any(|c| c.resolved_url == "https://document.example/inline.svg"));
    }
}

#[cfg(all(test, feature = "dom"))]
mod progressive_tests {
    use super::*;
    fn fixture_fetch(
        url: ParsedUrl,
        _: HttpRequest,
        _: usize,
    ) -> Result<RequestResult, FetchError> {
        Ok(RequestResult {
            connection_security: sunlight_cert::ConnectionSecurity::NotSecure,
            cert_info: None,
            response: sunlight_http::HttpResponse {
                status_code: 200,
                status_text: "OK".into(),
                headers: vec![("content-type".into(), "image/jpeg".into())],
                header_len: 0,
            },
            body: if url.path.contains("bad") {
                b"broken".to_vec()
            } else {
                include_bytes!("../../sun-img/tests/fixtures/gray.jpg").to_vec()
            },
            final_url: Some(url),
            duration_ms: Some(30),
        })
    }
    fn app(html: &str, runtime: &tokio::runtime::Runtime) -> RabbitApp {
        let mut app = RabbitApp::new(None);
        app.runtime = Some(runtime.handle().clone());
        app.image_fetcher = fixture_fetch;
        app.metrics.started = Some(std::time::Instant::now());
        let url = ParsedUrl::parse("http://fixture/").unwrap();
        app.begin_navigation_session("http://fixture/", &build_get_request(&url));
        app.apply_result(
            "http://fixture/",
            RequestResult {
                connection_security: sunlight_cert::ConnectionSecurity::NotSecure,
                cert_info: None,
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
        app
    }
    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .max_blocking_threads(1)
            .build()
            .unwrap()
    }
    fn paint(app: &mut RabbitApp) {
        let mut pixels = vec![0; 1080 * 720];
        app.view(
            &mut Canvas::new(&mut pixels, 1080, 1080, 720),
            &Theme::sunlight_light(),
        );
    }
    fn settle(app: &mut RabbitApp) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.metrics.page_ready_us.is_none() || app.reflow_completion.is_some() {
            app.update(Event::Tick);
            paint(app);
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
    }
    #[test]
    fn initial_document_is_interactive_while_image_result_is_deliberately_withheld() {
        let runtime = runtime();
        let mut app = app(
            include_str!("../tests/fixtures/progressive/archwiki.html"),
            &runtime,
        );
        assert_eq!(
            app.render_state
                .as_ref()
                .unwrap()
                .diagnostics
                .placeholder_count,
            3
        );
        assert!(app.image_cache.get("http://fixture/logo.jpg").is_none());
        let (sender, receiver) = oneshot::channel();
        app.image_completion = Some(receiver);
        paint(&mut app);
        assert!(app.metrics.initial_us.is_some());
        assert!(app.metrics.first_image_us.is_none());
        assert!(app.metrics.page_ready_us.is_none());
        assert!(!app.pending_fetch);
        let control = app
            .form_state
            .controls
            .keys()
            .copied()
            .find(|id| matches!(app.form_state.controls[id], FormControlState::Text { .. }))
            .unwrap();
        app.form_state.focus_control(control);
        app.form_state.insert_char('X');
        {
            let text = app.form_state.focused_text_state_mut().unwrap();
            text.selection_start = 1;
            text.selection_end = 3;
        }
        let expected = app.form_state.focused_text_state_mut().unwrap().clone();
        let rect = app.document_rect();
        app.update(Event::mouse_wheel(rect.x + 5, rect.y + 5, 3));
        assert_eq!(app.document_scroll.offset_y, 144);
        let mut cache = ImageCache::default();
        cache.insert_decoded(
            "http://fixture/logo.jpg".into(),
            decode_image(
                include_bytes!("../../sun-img/tests/fixtures/gray.jpg"),
                None,
                "logo.jpg",
            )
            .unwrap(),
        );
        assert!(sender
            .send(ImageOutput {
                shield_revision: app.shield.revision,
                generation: 1,
                cache,
                diagnostics: DeveloperToolsState::default()
            })
            .is_ok());
        app.image_attempts.push("http://fixture/logo.jpg".into());
        app.set_client_bounds(840, 540);
        settle(&mut app);
        assert_eq!(app.form_state.focused_control, Some(control));
        assert_eq!(app.form_state.focused_text_state_mut().unwrap(), &expected);
        assert_eq!(app.document_scroll.offset_y, 144);
        assert!(app.metrics.first_image_us.is_some());
        assert_eq!(
            app.render_state.as_ref().unwrap().viewport,
            app.latest_viewport
        );
        assert_eq!(app.metrics.cache_misses, 2);
        for _ in 0..100 {
            assert!(!app.update(Event::Tick));
        }
    }
    #[test]
    fn scroll_before_images_arrive_prioritizes_current_view_and_keeps_terminal_failures() {
        let runtime = runtime();
        let html = include_str!("../tests/fixtures/progressive/archwiki.html")
            .replace("screen.jpg", "bad.jpg");
        let mut app = app(&html, &runtime);
        let rect = app.document_rect();
        app.update(Event::mouse_wheel(rect.x + 5, rect.y + 5, i16::MAX));
        assert_eq!(
            app.document_scroll.offset_y,
            app.document_scroll.max_offset_y()
        );
        app.poll_images();
        assert_eq!(
            app.image_attempts.first().unwrap(),
            "http://fixture/footer.jpg"
        );
        settle(&mut app);
        assert!(app.image_cache.failed("http://fixture/bad.jpg").is_some());
        assert_eq!(app.metrics.cache_misses, 3);
        assert_eq!(app.image_attempts.len(), 3);
        app.document_scroll.offset_y = 0;
        for _ in 0..10 {
            app.update(Event::Tick);
            paint(&mut app);
        }
        let state = app.render_state.as_ref().unwrap();
        assert!(app.presentation.visible_complete(
            &state.current_scene,
            Rect::new(0, 0, app.latest_viewport.w, app.latest_viewport.h)
        ));
        for _ in 0..100 {
            assert!(!app.update(Event::Tick));
        }
    }
    #[test]
    fn rapid_navigation_and_close_drop_pending_image_results_and_owned_pixels() {
        let runtime = runtime();
        for _ in 0..32 {
            let mut app = app("<img src='a.jpg' width=16 height=16>", &runtime);
            let (sender, receiver) = oneshot::channel();
            app.image_completion = Some(receiver);
            let decoded = decode_image(
                include_bytes!("../../sun-img/tests/fixtures/gray.jpg"),
                None,
                "a.jpg",
            )
            .unwrap();
            let weak = std::sync::Arc::downgrade(&decoded.image);
            let mut cache = ImageCache::default();
            cache.insert_decoded("http://fixture/a.jpg".into(), decoded);
            let url = ParsedUrl::parse("http://fixture/next").unwrap();
            app.begin_navigation_session("http://fixture/next", &build_get_request(&url));
            assert!(app.image_completion.is_none());
            let result = sender.send(ImageOutput {
                shield_revision: app.shield.revision,
                generation: 1,
                cache,
                diagnostics: DeveloperToolsState::default(),
            });
            assert!(result.is_err());
            drop(result);
            assert!(weak.upgrade().is_none());
            assert!(app.render_state.is_none());
        }
    }
    #[cfg(feature = "js-dom-fixture")]
    #[test]
    fn fixture_src_changes_discover_resources_without_reexecuting_scripts() {
        let runtime = runtime();
        let mut app = app(
            "<p id='out'>ready</p><img id='img' src='a.jpg' width=16 height=16>",
            &runtime,
        );
        settle(&mut app);
        let mut js =
            rappid_rabbit::dom_fixture::FixtureSession::new(app.render_state.as_ref().unwrap())
                .unwrap();
        js.evaluate(
            app.render_state.as_mut().unwrap(),
            "let executions=1;document.getElementById('img').setAttribute('src','new.jpg');",
            &RABBIT_FONTS,
        )
        .unwrap();
        // The prior page-ready milestone is invalidated by resource discovery.
        app.update(Event::Tick);
        settle(&mut app);
        assert_eq!(
            app.image_attempts,
            vec!["http://fixture/a.jpg", "http://fixture/new.jpg"]
        );
        assert!(app.image_cache.decoded("http://fixture/new.jpg").is_some());
        js.evaluate(
            app.render_state.as_mut().unwrap(),
            "if(executions!==1){throw 'script reordered';}",
            &RABBIT_FONTS,
        )
        .unwrap();
    }

    #[test]
    fn canceled_image_io_cannot_queue_more_images_or_hold_the_navigation_worker() {
        let images = runtime();
        let runtime = runtime();
        let mut app = app("<img src='a.jpg' width=16 height=16>", &runtime);
        app.image_runtime = Some(images.handle().clone());
        // Hold the image lane as a real blocking IO operation would. The
        // navigation/layout pool must remain independently runnable.
        let (release, wait) = std::sync::mpsc::channel();
        let lane = ImageLaneGuard(app.image_lane.clone());
        app.image_lane
            .store(true, core::sync::atomic::Ordering::Release);
        images.spawn_blocking(move || {
            let _lane = lane;
            wait.recv().unwrap();
        });
        for _ in 0..100 {
            app.poll_images();
        }
        assert!(app.image_attempts.is_empty());
        assert!(app.image_completion.is_none());
        let (sender, receiver) = oneshot::channel();
        runtime.spawn_blocking(move || {
            sender.send(42).unwrap();
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut receiver = receiver;
        loop {
            if let Ok(value) = receiver.try_recv() {
                assert_eq!(value, 42);
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        release.send(()).unwrap();
        settle(&mut app);
        assert_eq!(app.image_attempts.len(), 1);
    }

    #[test]
    fn resource_completion_does_not_wait_for_layout_or_presentation() {
        let runtime = runtime();
        let mut app = app(
            include_str!("../tests/fixtures/progressive/archwiki.html"),
            &runtime,
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        // Deliberately never run poll_reflow or advance image presentation.
        while app.metrics.page_ready_us.is_none() {
            app.poll_images();
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert_eq!(app.image_attempts.len(), 3);
        assert!(app.render_state.as_ref().unwrap().images_need_update());
        assert_eq!(
            app.render_state
                .as_ref()
                .unwrap()
                .diagnostics
                .placeholder_count,
            3
        );
        assert!(app.metrics.first_image_us.is_none());
        assert!(app.presentation.admitted().is_empty());
        assert_eq!(app.metrics.cache_misses, 3);
    }

    #[test]
    fn closed_image_worker_is_terminal_and_text_only_navigation_needs_no_image_job() {
        let runtime = runtime();
        let mut text = app(
            include_str!("../tests/fixtures/progressive/text.html"),
            &runtime,
        );
        settle(&mut text);
        assert_eq!(text.metrics.cache_misses, 0);
        assert!(text.metrics.first_image_us.is_none());
        let mut broken = app("<img src='bad.jpg' width=16 height=16>", &runtime);
        let (sender, receiver) = oneshot::channel();
        drop(sender);
        broken.image_completion = Some(receiver);
        broken.image_attempts.push("http://fixture/bad.jpg".into());
        settle(&mut broken);
        assert_eq!(
            broken.image_cache.failed("http://fixture/bad.jpg"),
            Some("Image worker stopped")
        );
        for _ in 0..100 {
            assert!(!broken.update(Event::Tick));
        }
    }
}

#[cfg(test)]
mod certificate_panel_tests {
    use super::*;

    #[test]
    fn certificate_follows_navigation_and_panel_clicks() {
        let mut app = RabbitApp::new(None);
        app.set_client_bounds(900, 700);
        let cert = sunlight_cert::peer_chain([include_bytes!(
            "../../sunlight-tls/src/roots/gts_root_r4.der"
        )
        .as_slice()])
        .unwrap();
        app.cert_info = Some(cert.clone());
        app.connection_security = sunlight_cert::ConnectionSecurity::Secure;
        let output = app.into_navigation_output();
        assert_eq!(output.cert_info, Some(cert));
        assert_eq!(
            output.connection_security,
            sunlight_cert::ConnectionSecurity::Secure
        );

        let mut app = RabbitApp::new(None);
        app.set_client_bounds(900, 700);
        let site = app.url_site_rect();
        assert!(app.update(Event::MouseDown {
            x: site.x + 2,
            y: site.y + 2,
            button: 0
        }));
        assert!(app.certificate_panel.open);
        let details =
            sunlight_ui::widgets::CertificatePanel::details_rect(app.certificate_panel_rect());
        app.update(Event::MouseDown {
            x: details.x + 2,
            y: details.y + 2,
            button: 0,
        });
        assert!(app.certificate_panel.expanded);
        app.update(Event::Key('\u{1b}'));
        assert!(!app.certificate_panel.open);
        app.apply_fetch_error(FetchError::TlsCertInvalid("Wrong hostname".into()));
        assert_eq!(
            app.connection_security,
            sunlight_cert::ConnectionSecurity::Invalid
        );
        assert!(app
            .certificate_error
            .as_deref()
            .unwrap()
            .contains("Wrong hostname"));
        app.pending_fetch = false;
        app.queue_fetch();
        assert!(app.cert_info.is_none());
        assert!(app.certificate_error.is_none());
        assert_eq!(
            app.connection_security,
            sunlight_cert::ConnectionSecurity::Unknown
        );
    }
}
