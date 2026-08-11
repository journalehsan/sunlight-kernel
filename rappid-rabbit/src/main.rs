#![cfg_attr(not(test), no_std)]
#![cfg_attr(feature = "dom", allow(dead_code, unused_imports))]
#![cfg_attr(not(test), no_main)]
// Required for the custom OOM handler below; matches kernel/src/main.rs.
#![feature(alloc_error_handler)]

extern crate alloc;

use alloc::{borrow::Cow, collections::BTreeMap, format, string::String, vec::Vec};
// Needed for `write!`/`writeln!` against the stack-buffer writer in the fatal handlers.
use core::fmt::Write as _;
use linked_list_allocator::LockedHeap;

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
use sunlight_fetch::backend::{perform_request, RequestResult};
use sunlight_fetch::FetchError;
use sunlight_http::{HttpRequest, ParsedUrl};
use sunlight_ipc::{
    debug_log,
    launch_trace::{self, LaunchSource, LaunchTrace},
    process_yield, ProcessExit,
};
use sunlight_libc::crt0;
use sunlight_ui::widgets::{
    Button, ButtonState, Column as TableColumn, DocumentCanvas, DocumentCanvasPresentation, Label,
    Panel, TabBar, Table, TextInput, TextView, TreeHitTarget, TreeView,
};
#[cfg(feature = "dom")]
use sunlight_ui::widgets::{
    DocumentFontFamily, DocumentNodeId, RenderInteraction, RenderObjectKind,
};
use sunlight_ui::{
    request_close, App, AxisSizing, Canvas, Color, Column, Event, LayoutBox, LayoutInvalidation,
    Point, Rect, Row, Size, Sizing, Theme, VecText, Window, WindowConfig, WindowDecoration,
    WindowEvent,
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
const PAD: i32 = 12;
const TOP_BAR_H: u32 = 42;
const METHOD_W: u32 = 56;
const FETCH_W: u32 = 104;
const DEVTOOLS_W: u32 = 96;
const VIEW_W: u32 = 72;
const STATUS_W: u32 = 220;
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

const HEAP_SIZE: usize = 32 * 1024 * 1024;

static mut HEAP: [u8; HEAP_SIZE] = [0; HEAP_SIZE];
#[cfg_attr(not(test), global_allocator)]
static ALLOC: LockedHeap = LockedHeap::empty();

fn heap_used() -> usize {
    ALLOC.lock().used()
}

unsafe fn init_heap() {
    ALLOC
        .lock()
        .init(core::ptr::addr_of_mut!(HEAP).cast::<u8>(), HEAP_SIZE);
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

#[cfg(not(test))]
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    let mut buf = LogBuf::<256>::new();
    let _ = write!(buf, "[RABBIT] panic");
    if let Some(loc) = info.location() {
        let _ = write!(buf, " at {}:{}:{}", loc.file(), loc.line(), loc.column());
    }
    // `PanicInfo::message()` returns a `PanicMessage` (Display) on this nightly, which is
    // what surfaces the OOM reason ("memory allocation of N bytes failed") when applicable.
    let _ = write!(buf, ": {}", info.message());
    let _ = writeln!(buf);
    debug_log(buf.as_str());
    // Clean exit instead of an infinite loop: a panic is now a visible one-liner in the
    // QEMU log and the app closes, rather than silently freezing the window.
    ProcessExit::exit(1);
}

#[cfg(not(test))]
#[alloc_error_handler]
fn alloc_error(layout: core::alloc::Layout) -> ! {
    let mut buf = LogBuf::<160>::new();
    let _ = writeln!(
        buf,
        "[RABBIT] OOM: alloc {} bytes (align {}) failed; heap {}/{}",
        layout.size(),
        layout.align(),
        heap_used(),
        HEAP_SIZE
    );
    debug_log(buf.as_str());
    ProcessExit::exit(1);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FocusPane {
    Source,
    Render,
    DeveloperTools,
}

#[derive(Clone, Copy, PartialEq, Eq)]
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

struct RabbitApp {
    url_input: TextInput<'static, URL_INPUT_CAP>,
    url_hovered: bool,
    url_action_hovered: bool,
    status: String,
    pending_fetch: bool,
    focus: FocusPane,
    source_scroll: usize,
    source: SourceContent,
    document_view: DocumentView,
    render_scroll: u32,
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
    method: Rect,
    url_bar: Rect,
    fetch: Rect,
    view: Rect,
    devtools: Rect,
    status: Rect,
}

impl RabbitApp {
    fn new(initial_url: Option<&str>) -> Self {
        let mut url_input = TextInput::new(Rect::default())
            .with_font(&F_UI)
            .with_clipboard_source(b"rappid-rabbit")
            .with_placeholder("Enter URL (http:// or https://)");
        url_input.set_text("http://example.com/");

        let mut developer_tools = DeveloperToolsState::default();
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

        let mut app = Self {
            url_input,
            url_hovered: false,
            url_action_hovered: false,
            status: String::from("Idle"),
            pending_fetch: false,
            focus: FocusPane::Source,
            source_scroll: 0,
            source: SourceContent::Placeholder(
                "Enter a URL, then click Fetch/Open to inspect the response body.",
            ),
            document_view: DocumentView::Source,
            render_scroll: 0,
            render_status: String::from(
                "No rendered document yet. Enter a URL and select Fetch/Open.",
            ),
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
        self.pending_fetch = true;
        self.status = String::from("Fetching...");
        self.render_status = String::from("Loading rendered document...");
        self.developer_tools.console.push(
            ConsoleSeverity::Quiet,
            ConsoleSource::Browser,
            format!("Navigation queued for {}", self.url_input.value().trim()),
        );
    }

    fn perform_pending_fetch(&mut self) {
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

        let parsed = match ParsedUrl::parse(&normalized) {
            Ok(url) => url,
            Err(err) => {
                self.set_error(format!("{err}"));
                return;
            }
        };

        let request = build_get_request(&parsed);
        self.begin_navigation_session(&normalized, &request);
        match perform_request(parsed, request) {
            Ok(result) => self.apply_result(&normalized, result),
            Err(err) => self.apply_fetch_error(err),
        }
    }

    fn begin_navigation_session(&mut self, requested_url: &str, request: &HttpRequest) {
        let generation = self.document_lifecycle.begin_navigation();
        self.discovered_resources.clear();
        self.resource_queue.clear();
        #[cfg(feature = "dom")]
        {
            self.image_cache = ImageCache::default();
        }
        self.render_scroll = 0;
        self.render_status = String::from("Loading rendered document...");
        #[cfg(feature = "dom")]
        {
            self.render_state = None;
            self.form_state = FormState::new();
        }
        self.developer_tools.network.clear_for_new_page();
        self.developer_tools
            .dom
            .clear_with_message("Waiting for parsed HTML document.");

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

    fn apply_result(&mut self, requested_url: &str, result: RequestResult) {
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
        // Navigation is currently performed from the next local tick, but
        // retain this equality guard so a future asynchronous completion never
        // overwrites a URL the user has begun editing.
        if self.url_input.value().trim() == requested_url {
            self.url_input.set_text(&final_url);
        }
        let content_type = result
            .response
            .header("content-type")
            .map_or("(missing)", |value| value);
        let body_size = result.body.len();
        self.log_heap_stage("after response completion", generation, Some(body_size));

        self.status = format!("HTTP {status_code} {status_text}");
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

        if (400..500).contains(&status_code) {
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
                match parse_html_with_limits(source_text, ParseLimits::default()) {
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
                            let mut all_candidates = candidates;
                            discover_stylesheet_images(
                                &mut all_candidates,
                                &linked_stylesheets,
                                base_url,
                            );
                            self.resource_queue.replace_from_candidates(&all_candidates);
                            self.fetch_images(&all_candidates);
                            self.discovered_resources = all_candidates;
                        }
                        let stylesheets = order_document_stylesheets(
                            &document,
                            embedded_stylesheets,
                            linked_stylesheets,
                        );
                        let style_context = StyleContext::build(&document, &stylesheets);
                        let inspector_document = document.clone();
                        let inspector_styles = style_context.clone();
                        let viewport = self.render_viewport();
                        self.render_scroll = 0;
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
                        self.log_render_scene("initial scene");
                        self.developer_tools
                            .dom
                            .set_document_with_styles(inspector_document, inspector_styles);
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
            let result = match perform_request(parsed_url, request) {
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
    fn fetch_images(&mut self, candidates: &[ResourceCandidate]) {
        for candidate in candidates {
            if candidate.resource_type != ResourceType::Image {
                continue;
            }
            if self.image_cache.get(&candidate.resolved_url).is_some() {
                self.developer_tools.console.push(
                    ConsoleSeverity::Quiet,
                    ConsoleSource::Fetch,
                    format!("[RABBIT][IMAGE] cache hit url={}", candidate.resolved_url),
                );
                continue;
            }
            self.developer_tools.console.push(
                ConsoleSeverity::Quiet,
                ConsoleSource::Fetch,
                format!("[RABBIT][IMAGE] discovered url={}", candidate.resolved_url),
            );
            let parsed_url = match ParsedUrl::parse(&candidate.resolved_url) {
                Ok(url) => url,
                Err(error) => {
                    let reason = format!("invalid image URL: {error}");
                    self.image_cache
                        .insert_failed(candidate.resolved_url.clone(), reason.clone());
                    self.developer_tools.console.push(
                        ConsoleSeverity::Warn,
                        ConsoleSource::Fetch,
                        format!("[RABBIT][IMAGE] failed reason={reason}"),
                    );
                    continue;
                }
            };
            let request = build_get_request(&parsed_url);
            let request_id = self.developer_tools.network.begin_request(
                request.method,
                candidate.resolved_url.clone(),
                request.headers.clone(),
                ResourceType::Image,
                ResourcePriority::Embedded,
                RequestState::Queued,
            );
            self.developer_tools
                .network
                .set_request_state(request_id, RequestState::Connecting);
            self.developer_tools.console.push(
                ConsoleSeverity::Quiet,
                ConsoleSource::Fetch,
                format!(
                    "[RABBIT][IMAGE] request started url={}",
                    candidate.resolved_url
                ),
            );
            match perform_request(parsed_url, request) {
                Ok(result) => {
                    let status_code = result.response.status_code;
                    let status_text = if result.response.status_text.is_empty() {
                        String::from("(no reason phrase)")
                    } else {
                        result.response.status_text.clone()
                    };
                    let final_url = result.final_url.as_ref().map(format_url);
                    let content_type = result.response.header("content-type").map(String::from);
                    let byte_size = result.body.len();
                    self.developer_tools.network.complete_request(
                        request_id,
                        final_url.clone(),
                        status_code,
                        status_text.clone(),
                        result.duration_ms,
                        Some(byte_size),
                        content_type.clone(),
                        result.response.headers.clone(),
                        None,
                        Some(false),
                        Some(false),
                    );
                    if !(200..400).contains(&status_code) {
                        let reason = format!("HTTP {status_code} {status_text}");
                        self.image_cache
                            .insert_failed(candidate.resolved_url.clone(), reason.clone());
                        self.developer_tools.console.push(
                            ConsoleSeverity::Warn,
                            ConsoleSource::Fetch,
                            format!("[RABBIT][IMAGE] failed reason={reason}"),
                        );
                        continue;
                    }
                    if byte_size > MAX_IMAGE_RESPONSE_BYTES {
                        let reason =
                            format!("response exceeds {MAX_IMAGE_RESPONSE_BYTES} byte limit");
                        self.image_cache
                            .insert_failed(candidate.resolved_url.clone(), reason.clone());
                        self.developer_tools.console.push(
                            ConsoleSeverity::Warn,
                            ConsoleSource::Fetch,
                            format!("[RABBIT][IMAGE] failed reason={reason}"),
                        );
                        continue;
                    }
                    match decode_image(
                        &result.body,
                        content_type.as_deref(),
                        final_url
                            .as_deref()
                            .unwrap_or(candidate.resolved_url.as_str()),
                    ) {
                        Ok(decoded) => {
                            self.developer_tools.console.push(
                                ConsoleSeverity::Quiet,
                                ConsoleSource::Fetch,
                                format!(
                                    "[RABBIT][IMAGE] response url={} status={} content_type={} bytes={}",
                                    candidate.resolved_url,
                                    status_code,
                                    content_type.as_deref().unwrap_or("(missing)"),
                                    byte_size,
                                ),
                            );
                            self.developer_tools.console.push(
                                ConsoleSeverity::Quiet,
                                ConsoleSource::Fetch,
                                format!(
                                    "[RABBIT][IMAGE] decode url={} result=ok format={} size={}x{} pixels={}",
                                    candidate.resolved_url,
                                    decoded.format.label(),
                                    decoded.image.width,
                                    decoded.image.height,
                                    decoded.image.pixels.len(),
                                ),
                            );
                            self.image_cache
                                .insert_decoded(candidate.resolved_url.clone(), decoded);
                            self.developer_tools.console.push(
                                ConsoleSeverity::Quiet,
                                ConsoleSource::Fetch,
                                format!(
                                    "[RABBIT][IMAGE] cache key={} state=Decoded",
                                    candidate.resolved_url
                                ),
                            );
                        }
                        Err(reason) => {
                            self.image_cache
                                .insert_failed(candidate.resolved_url.clone(), reason.clone());
                            self.developer_tools.console.push(
                                ConsoleSeverity::Warn,
                                ConsoleSource::Fetch,
                                format!("[RABBIT][IMAGE] failed reason={reason}"),
                            );
                        }
                    }
                }
                Err(error) => {
                    let reason = format!("image request failed: {error}");
                    self.developer_tools.network.fail_request(
                        request_id,
                        None,
                        None,
                        None,
                        reason.clone(),
                    );
                    self.image_cache
                        .insert_failed(candidate.resolved_url.clone(), reason.clone());
                    self.developer_tools.console.push(
                        ConsoleSeverity::Warn,
                        ConsoleSource::Fetch,
                        format!("[RABBIT][IMAGE] failed reason={reason}"),
                    );
                }
            }
        }
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
        let inner = root.inset(PAD);
        let mut root_children = [
            LayoutBox::new(Rect::new(0, 0, 0, TOP_BAR_H))
                .with_sizing(Sizing::new(AxisSizing::Fill, AxisSizing::Fixed(TOP_BAR_H))),
            LayoutBox::new(Rect::default()).with_sizing(fill),
        ];
        let _ = Column::new(inner)
            .with_gap(PAD.max(0) as u32)
            .arrange(&mut root_children);
        let top_bar = root_children[0].bounds();
        let content = root_children[1].bounds();

        let controls = top_bar.inset(8);
        let fixed = |width| {
            LayoutBox::new(Rect::new(0, 0, width, 28))
                .with_sizing(Sizing::new(AxisSizing::Fixed(width), AxisSizing::Fixed(28)))
        };
        let mut toolbar = [
            fixed(METHOD_W),
            LayoutBox::new(Rect::new(0, 0, 0, 28))
                .with_sizing(Sizing::new(AxisSizing::Fill, AxisSizing::Fixed(28))),
            fixed(FETCH_W),
            fixed(VIEW_W),
            fixed(DEVTOOLS_W),
            fixed(STATUS_W),
        ];
        let _ = Row::new(controls).with_gap(8).arrange(&mut toolbar);
        let status_box = toolbar[5].bounds();

        RabbitLayout {
            root,
            top_bar,
            content,
            method: toolbar[0].bounds(),
            url_bar: toolbar[1].bounds(),
            fetch: toolbar[2].bounds(),
            view: toolbar[3].bounds(),
            devtools: toolbar[4].bounds(),
            status: Rect::new(
                status_box.x,
                status_box.y.saturating_sub(1),
                status_box.w,
                30,
            ),
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
        #[cfg(feature = "dom")]
        self.refresh_render_viewport();
        self.clamp_scrolls();
        changed
    }

    fn top_bar_rect(&self) -> Rect {
        self.layout.top_bar
    }

    fn method_rect(&self) -> Rect {
        self.layout.method
    }

    fn status_rect(&self) -> Rect {
        self.layout.status
    }

    fn devtools_button_rect(&self) -> Rect {
        self.layout.devtools
    }

    fn view_button_rect(&self) -> Rect {
        self.layout.view
    }

    fn fetch_rect(&self) -> Rect {
        self.layout.fetch
    }

    fn url_bar_rect(&self) -> Rect {
        self.layout.url_bar
    }

    fn url_site_rect(&self) -> Rect {
        let bar = self.url_bar_rect();
        Rect::new(bar.x + 4, bar.y + 4, URL_SITE_W, bar.h.saturating_sub(8))
    }

    fn url_action_rect(&self) -> Rect {
        let bar = self.url_bar_rect();
        Rect::new(
            bar.right() - URL_ACTION_W as i32 - 4,
            bar.y + 4,
            URL_ACTION_W,
            bar.h.saturating_sub(8),
        )
    }

    fn url_text_rect(&self) -> Rect {
        let bar = self.url_bar_rect();
        let site = self.url_site_rect();
        let action = self.url_action_rect();
        Rect::new(
            site.right() + 5,
            bar.y + 2,
            (action.x - site.right() - 10).max(32) as u32,
            bar.h.saturating_sub(4),
        )
    }

    fn update_url_chrome_hover(&mut self, x: i32, y: i32) -> bool {
        let point = Point::new(x, y);
        let hovered = self.url_bar_rect().contains(point);
        let action_hovered = !self.pending_fetch && self.url_action_rect().contains(point);
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
        DocumentCanvas::new(self.source_panel_rect(), &[])
            .with_presentation(DocumentCanvasPresentation::Browser)
            .viewport_size()
    }

    #[cfg(feature = "dom")]
    fn refresh_render_viewport(&mut self) {
        let viewport = self.render_viewport();
        let changed = if let Some(render_state) = self.render_state.as_mut() {
            if render_state.rebuild_for_viewport(viewport, &RABBIT_FONTS) {
                self.render_scroll = self
                    .render_scroll
                    .min(render_state.current_scene.max_scroll_y(viewport.h));
                true
            } else {
                false
            }
        } else {
            false
        };
        if changed {
            self.patch_all_controls();
        }
        if changed {
            self.log_render_scene("viewport reflow");
        }
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
                        "[RABBIT][IMAGE] scene node={} object={} kind=Image url={} bounds={},{} {}x{}",
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
                        "[RABBIT][IMAGE] scene node={} object={} kind=ImagePlaceholder url={} bounds={},{} {}x{}",
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
        let rect = self.source_panel_rect();
        let canvas = DocumentCanvas::new(rect, &[])
            .with_presentation(DocumentCanvasPresentation::Browser)
            .with_empty_label(self.render_status.as_str())
            .with_fonts(Some(&F_LARGE), Some(&F_SMALL), Some(&F_UI), Some(&F_SMALL))
            .with_scene_font_families(Some(&F_SERIF), Some(&F_MONO));
        #[cfg(feature = "dom")]
        if let Some(render_state) = self.render_state.as_ref() {
            return canvas
                .with_scene(&render_state.current_scene)
                .with_scroll_y(self.render_scroll);
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
            format!(
                "[RABBIT][FORM] submit form={} method=GET action={} controls={}",
                form_id.0,
                target,
                self.form_state
                    .controls
                    .values()
                    .filter(|c| c.form_owner() == Some(form_id) && !c.is_disabled())
                    .count()
            ),
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
                if matches!(
                    control.kind(),
                    FormControlKind::TextInput | FormControlKind::SearchInput
                ) {
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
            Event::Key(ch) if ch.is_ascii_graphic() || ch == ' ' => {
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

    fn draw_top_bar(&mut self, canvas: &mut Canvas, theme: &Theme) {
        let top = self.top_bar_rect();
        Panel::new(top).draw(canvas, theme);

        let mut method = Button::secondary(self.method_rect(), "GET").with_font(&F_UI);
        method.state = ButtonState::Normal;
        method.draw(canvas, theme);

        self.draw_url_bar(canvas, theme);

        let mut fetch = Button::new(self.fetch_rect(), "Fetch/Open").with_font(&F_UI);
        fetch.state = ButtonState::Normal;
        fetch.draw(canvas, theme);

        let tools_label = if self.developer_tools.panel.open {
            "Hide Tools"
        } else {
            "DevTools"
        };
        let mut devtools =
            Button::secondary(self.devtools_button_rect(), tools_label).with_font(&F_UI);
        devtools.state = ButtonState::Normal;
        devtools.draw(canvas, theme);

        let view_label = match self.document_view {
            DocumentView::Source => "Render",
            DocumentView::Render => "Source",
        };
        let mut view = Button::secondary(self.view_button_rect(), view_label).with_font(&F_UI);
        view.state = ButtonState::Normal;
        view.draw(canvas, theme);

        Label::new(self.status_rect(), self.status.as_str())
            .with_font(&F_SMALL)
            .draw(canvas, theme);
    }

    fn draw_url_bar(&mut self, canvas: &mut Canvas, theme: &Theme) {
        let rect = self.url_bar_rect();
        let focused = self.url_input.active;
        let disabled = self.pending_fetch;
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
        if focused {
            canvas.stroke_rounded_rect(
                rect.inset(2),
                URL_BAR_RADIUS.saturating_sub(2),
                1,
                theme.accent.lighten(46),
            );
        }

        let site = self.url_site_rect();
        let secure = self.url_input.value().starts_with("https://");
        let site_color = if secure { theme.ok } else { theme.warn };
        canvas.fill_rounded_rect(site, 5, theme.panel_alt);
        canvas.stroke_rounded_rect(site, 5, 1, theme.border);
        Self::draw_site_indicator(canvas, site, site_color);

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
            disabled,
        );

        self.url_input.rect = self.url_text_rect();
        self.url_input.draw_content(canvas, theme);
    }

    fn draw_site_indicator(canvas: &mut Canvas, rect: Rect, color: Color) {
        let body = Rect::new(rect.x + 8, rect.y + 9, 10, 7);
        canvas.draw_rect(body, color);
        canvas.hbar(body.x + 2, body.y - 3, 6, 1, color);
        canvas.vline(body.x + 2, body.y - 3, 4, color);
        canvas.vline(body.right() - 3, body.y - 3, 4, color);
        canvas.vline(body.x + 4, body.y + 2, 3, color);
    }

    fn draw_reload_indicator(canvas: &mut Canvas, rect: Rect, color: Color, loading: bool) {
        let cx = rect.x + rect.w as i32 / 2;
        let cy = rect.y + rect.h as i32 / 2;
        if loading {
            for offset in [-4, 0, 4] {
                canvas.fill_rect(Rect::new(cx + offset - 1, cy - 1, 2, 2), color);
            }
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
                // Layout box for the node
                if let Some(ln) = rs
                    .layout_tree
                    .nodes
                    .iter()
                    .find(|n| n.owner_node_id.0 == node as u64)
                {
                    extra.push_str("\nLayout Box:\n");
                    extra.push_str(&format!("  content: ({},{}) {}x{}  padding:({},{},{},{})  border:({},{},{},{})  margin:({},{},{},{})\n",
                        ln.content_box.x, ln.content_box.y, ln.content_box.w, ln.content_box.h,
                        ln.padding_box.x, ln.padding_box.y, ln.padding_box.w, ln.padding_box.h,
                        ln.border_box.x, ln.border_box.y, ln.border_box.w, ln.border_box.h,
                        ln.margin_box.x, ln.margin_box.y, ln.margin_box.w, ln.margin_box.h));
                    extra.push_str(&format!(
                        "  border widths: {:?} colors: {:?}\n  corner radii: {:?}\n",
                        ln.paint.border_widths, ln.paint.border_colors, ln.paint.corner_radii
                    ));
                    if let Some(shadow) = ln.paint.box_shadow {
                        extra.push_str(&format!(
                            "  box-shadow: offset=({}, {}) blur={} spread={} color={:?} inset={}\n",
                            shadow.offset_x,
                            shadow.offset_y,
                            shadow.blur,
                            shadow.spread,
                            shadow.color,
                            shadow.inset
                        ));
                    }
                    if ln.float_side != "none" || ln.clear != "none" {
                        extra.push_str(&format!(
                            "  float={} clear={} containing-block={:?}\n",
                            ln.float_side, ln.clear, ln.float_containing_block
                        ));
                    }
                    if let Some((rows, columns)) = ln.table_dimensions {
                        extra.push_str(&format!(
                            "  table: rows={} columns={} final-y={}\n",
                            rows, columns, ln.border_box.y
                        ));
                    }
                    if let Some(m) = &ln.marker {
                        extra.push_str(&format!(
                            "  marker: {:?} {}\n",
                            m.shape,
                            m.label.as_deref().unwrap_or("")
                        ));
                    }
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
        self.source_scroll = self.source_scroll.min(self.source_view().max_scroll());
        #[cfg(feature = "dom")]
        {
            let viewport_h = self.render_viewport().h;
            if let Some(render_state) = self.render_state.as_ref() {
                self.render_scroll = self
                    .render_scroll
                    .min(render_state.current_scene.max_scroll_y(viewport_h));
            }
        }

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
                #[cfg(feature = "dom")]
                {
                    let viewport_h = self.render_viewport().h;
                    if let Some(render_state) = self.render_state.as_ref() {
                        let max_scroll = render_state.current_scene.max_scroll_y(viewport_h);
                        if home {
                            self.render_scroll = 0;
                        } else if end {
                            self.render_scroll = max_scroll;
                        } else {
                            let step = if page {
                                viewport_h.saturating_sub(24)
                            } else {
                                24
                            } as i32;
                            self.render_scroll = if delta < 0 {
                                self.render_scroll.saturating_sub(step as u32)
                            } else {
                                self.render_scroll
                                    .saturating_add(step as u32)
                                    .min(max_scroll)
                            };
                        }
                        return;
                    }
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
fn discover_stylesheet_images(
    candidates: &mut Vec<ResourceCandidate>,
    sheets: &[Option<Vec<rappid_rabbit::css::Stylesheet>>],
    document_url: &ParsedUrl,
) {
    for sheet in sheets.iter().flatten().flatten() {
        let base = match &sheet.source {
            StylesheetSource::External(url) => ParsedUrl::parse(url).ok(),
            _ => Some(document_url.clone()),
        };
        let Some(base) = base else { continue };
        for rule in &sheet.rules {
            for declaration in &rule.declarations {
                let raw = declaration.raw_value.trim();
                let Some(start) = raw.find("url(") else {
                    continue;
                };
                let Some(end_rel) = raw[start + 4..].find(')') else {
                    continue;
                };
                let value = raw[start + 4..start + 4 + end_rel]
                    .trim()
                    .trim_matches(['\"', '\'']);
                if value.is_empty() || value.starts_with("data:") {
                    continue;
                }
                let Ok(resolved) = rappid_rabbit::resources::discovery::resolve_url(&base, value)
                else {
                    continue;
                };
                if !candidates
                    .iter()
                    .any(|candidate| candidate.resolved_url == resolved)
                {
                    candidates.push(ResourceCandidate {
                        raw_url: String::from(value),
                        resolved_url: resolved,
                        resource_type: ResourceType::Image,
                        classification: rappid_rabbit::resources::discovery::DiscoveryClassification::EmbeddedResource,
                        enqueue_for_fetch: false,
                    });
                }
            }
        }
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
        self.draw_top_bar(canvas, theme);

        match self.document_view {
            DocumentView::Source => {
                let source_panel = Panel::with_title(self.source_panel_rect(), "Source");
                source_panel.draw(canvas, theme);
                self.source_view().draw(canvas, theme);
            }
            DocumentView::Render => self.document_canvas().draw(canvas, theme),
        }

        self.draw_developer_tools(canvas, theme);
        // TextInput's menu is a window-local overlay.  It must paint after
        // document and developer-tool surfaces rather than as part of the
        // top-bar child draw, or those surfaces conceal its lower rows.
        self.url_input.draw_context_menu(canvas, theme);
    }

    fn update(&mut self, event: Event) -> bool {
        self.url_input.rect = self.url_text_rect();
        if self.url_input.context_menu_open() {
            return self.url_input.update(event);
        }

        if let Event::Click { x, y } = event {
            if !self.pending_fetch && self.url_action_rect().contains(Point::new(x, y)) {
                self.url_input.active = false;
                self.queue_fetch();
                return true;
            }
        }

        if self.url_input.update(event) {
            return true;
        }

        #[cfg(feature = "dom")]
        if self.document_view == DocumentView::Render && self.handle_form_key(event) {
            return true;
        }

        match event {
            Event::Tick => {
                if self.pending_fetch {
                    self.perform_pending_fetch();
                    self.clamp_scrolls();
                    return true;
                }
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

                let fetch_button = Button::new(self.fetch_rect(), "Fetch/Open");
                if fetch_button.hit_test(x, y) {
                    self.queue_fetch();
                    return true;
                }

                let devtools_label = if self.developer_tools.panel.open {
                    "Hide Tools"
                } else {
                    "DevTools"
                };
                let devtools_button =
                    Button::secondary(self.devtools_button_rect(), devtools_label);
                if devtools_button.hit_test(x, y) {
                    if self.developer_tools.panel.open {
                        self.developer_tools.panel.close();
                    } else {
                        self.developer_tools.panel.open();
                    }
                    #[cfg(feature = "dom")]
                    self.refresh_render_viewport();
                    self.clamp_scrolls();
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
            Event::Key('\n') if self.url_input.active => {
                self.queue_fetch();
                true
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

    fn window_event(&mut self, event: WindowEvent) -> bool {
        let WindowEvent::Resized { width, height } = event;
        self.set_client_bounds(width, height)
    }
}

#[cfg(not(test))]
#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8, _envp: *const *const u8) -> ! {
    unsafe {
        init_heap();
    }
    sunlight_libc::launch_trace::init_from_argv(argc, argv);
    let trace = launch_trace::current().unwrap_or(LaunchTrace::new(0, LaunchSource::Unknown, 0));
    launch_trace::log_phase_now(
        trace,
        "app=rappid-rabbit",
        "app_main_started",
        Some(sunlight_ipc::getpid()),
    );

    let mut app = RabbitApp::new(initial_url_from_argv(argc, argv));
    let Some(mut window) = Window::connect(WindowConfig {
        width: WIN_W,
        height: WIN_H,
        title: "Rappid Rabbit",
        decoration: WindowDecoration::Normal,
    }) else {
        debug_log("[RABBIT] failed to connect window\n");
        loop {
            process_yield();
        }
    };
    window.run(&mut app);
    ProcessExit::exit(0);
}

/// Read only a bounded first HTTP(S) URL from launch argv. Launch-trace
/// metadata and unknown arguments are ignored, keeping regular startup
/// behavior intact while allowing the shell's centralized launcher handoff.
fn initial_url_from_argv(argc: u64, argv: *const *const u8) -> Option<&'static str> {
    let mut raw = [core::ptr::null::<u8>(); 8];
    let count = unsafe { crt0::collect_raw_args(argc, argv, &mut raw) };
    for pointer in raw.iter().take(count).skip(1) {
        let len = unsafe { crt0::cstr_len(*pointer, URL_INPUT_CAP) };
        if len == 0 || len >= URL_INPUT_CAP {
            continue;
        }
        let value: &'static [u8] = unsafe { core::slice::from_raw_parts(*pointer, len) };
        let values = [value];
        if let Some(url) = rappid_rabbit::launch_url::initial_url_from_values(&values) {
            return Some(url);
        }
    }
    None
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
    use super::{RabbitApp, Rect, Size, WIN_H, WIN_W};

    #[test]
    fn toolbar_and_page_area_follow_live_root() {
        let initial = RabbitApp::compute_layout(Rect::new(0, 0, WIN_W, WIN_H));
        let resized = RabbitApp::compute_layout(Rect::new(0, 0, 1360, 880));
        assert_eq!(initial.root.w, WIN_W);
        assert_eq!(resized.root.w, 1360);
        assert_eq!(resized.top_bar.w, 1360 - 24);
        assert_eq!(resized.content.w, resized.top_bar.w);
        assert_eq!(resized.content.bottom(), resized.root.bottom() - 12);
        assert!(resized.content.h > initial.content.h);
    }

    #[test]
    fn address_field_is_the_horizontal_fill_participant() {
        let narrow = RabbitApp::compute_layout(Rect::new(0, 0, 820, 720));
        let wide = RabbitApp::compute_layout(Rect::new(0, 0, 1280, 720));
        assert_eq!(narrow.method.w, wide.method.w);
        assert_eq!(narrow.fetch.w, wide.fetch.w);
        assert_eq!(narrow.status.w, wide.status.w);
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
            assert_eq!(layout.content.size(), Size::new(0, 0));
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
