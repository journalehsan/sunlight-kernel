#![cfg_attr(not(test), no_std)]
#![cfg_attr(feature = "dom", allow(dead_code, unused_imports))]
#![no_main]
// Required for the custom OOM handler below; matches kernel/src/main.rs.
#![feature(alloc_error_handler)]

extern crate alloc;

use alloc::{borrow::Cow, format, string::String, vec::Vec};
// Needed for `write!`/`writeln!` against the stack-buffer writer in the fatal handlers.
use core::fmt::Write as _;
use linked_list_allocator::LockedHeap;

use rappid_rabbit::{
    body_is_probably_text, build_get_request,
    developer_tools::{
        console::{ConsoleSeverity, ConsoleSource},
        dom_inspector::DomInspectorPane,
        network::NetworkPaneFocus,
        panel::{DeveloperPanelLayout, DeveloperPanelState, MIN_MAIN_CONTENT_H},
        state::DeveloperToolsState,
        tabs::DeveloperToolTab,
    },
    document_lifecycle::DocumentLifecycle,
    format_url, normalize_url_input,
    resources::{
        discovery::{ResourceCandidate, ResourceQueue},
        request::RequestState,
    },
};

#[cfg(feature = "dom")]
use golden_fish::{parse_html_with_limits, ParseLimits};
use sun_font::{FontRole, VecFont};
use sunlight_fetch::backend::{perform_request, RequestResult};
use sunlight_fetch::FetchError;
use sunlight_http::{HttpRequest, ParsedUrl};
use sunlight_ipc::{
    debug_log,
    launch_trace::{self, LaunchSource, LaunchTrace},
    process_yield, ProcessExit,
};
use sunlight_ui::widgets::{
    Button, ButtonState, Column, Label, Panel, TabBar, Table, TextInput, TextView, TreeHitTarget,
    TreeView,
};
use sunlight_ui::{
    request_close, App, Canvas, Event, Point, Rect, Theme, VecText, Window, WindowConfig,
    WindowDecoration,
};

static F_UI: VecFont = VecFont(FontRole::UiRegular);
static F_SMALL: VecFont = VecFont(FontRole::UiSmall);
static F_MONO: VecFont = VecFont(FontRole::MonoRegular);

const WIN_W: u32 = 1080;
const WIN_H: u32 = 720;
const PAD: i32 = 12;
const TOP_BAR_H: u32 = 42;
const METHOD_W: u32 = 56;
const FETCH_W: u32 = 104;
const DEVTOOLS_W: u32 = 96;
const STATUS_W: u32 = 220;
const GUTTER: i32 = 12;
const URL_INPUT_CAP: usize = 512;

const KEY_Q: u8 = 0x10;
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
    DeveloperTools,
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
    status: String,
    pending_fetch: bool,
    focus: FocusPane,
    source_scroll: usize,
    source: SourceContent,
    active_navigation_request_id: Option<u64>,
    developer_tools: DeveloperToolsState,
    discovered_resources: Vec<ResourceCandidate>,
    resource_queue: ResourceQueue,
    document_lifecycle: DocumentLifecycle,
}

impl RabbitApp {
    fn new() -> Self {
        let mut url_input = TextInput::new(Rect::default())
            .with_font(&F_UI)
            .with_placeholder("Enter URL (http:// or https://)");
        url_input.set_text("http://example.com/");

        let mut developer_tools = DeveloperToolsState::default();
        developer_tools.console.push(
            ConsoleSeverity::Quiet,
            ConsoleSource::Browser,
            "Developer tools panel ready.",
        );

        Self {
            url_input,
            status: String::from("Idle"),
            pending_fetch: false,
            focus: FocusPane::Source,
            source_scroll: 0,
            source: SourceContent::Placeholder(
                "Enter a URL, then click Fetch/Open to inspect the response body.",
            ),
            active_navigation_request_id: None,
            developer_tools,
            discovered_resources: Vec::new(),
            resource_queue: ResourceQueue::default(),
            document_lifecycle: DocumentLifecycle::default(),
        }
    }

    fn queue_fetch(&mut self) {
        self.pending_fetch = true;
        self.status = String::from("Fetching...");
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
                        if let Some(base_url) = final_url_parsed.as_ref() {
                            let candidates =
                                rappid_rabbit::resources::discovery::discover_resources(
                                    &document, base_url,
                                );
                            self.resource_queue.replace_from_candidates(&candidates);
                            self.discovered_resources = candidates;
                        }
                        self.developer_tools.dom.set_document(document);
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
            }
        }

        #[cfg(not(feature = "dom"))]
        {
            self.developer_tools
                .dom
                .clear_with_message("Golden Fish DOM inspector is unavailable in this build.");
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
        self.source = SourceContent::Message(String::from(
            "No response body was captured for this request.",
        ));
        self.discovered_resources.clear();
        self.resource_queue.clear();
        self.developer_tools
            .dom
            .clear_with_message("No DOM available for the failed request.");
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
        self.source = SourceContent::Message(String::from("Fix the URL and fetch again."));
        self.active_navigation_request_id = None;
        self.developer_tools
            .console
            .push(ConsoleSeverity::Warn, ConsoleSource::Browser, message);
    }

    fn top_bar_rect(&self) -> Rect {
        Rect::new(PAD, PAD, WIN_W.saturating_sub((PAD * 2) as u32), TOP_BAR_H)
    }

    fn method_rect(&self) -> Rect {
        let top = self.top_bar_rect();
        Rect::new(top.x + 8, top.y + 7, METHOD_W, 28)
    }

    fn status_rect(&self) -> Rect {
        let top = self.top_bar_rect();
        Rect::new(top.right() - STATUS_W as i32 - 8, top.y + 6, STATUS_W, 30)
    }

    fn devtools_button_rect(&self) -> Rect {
        let status = self.status_rect();
        Rect::new(
            status.x - DEVTOOLS_W as i32 - 8,
            status.y + 1,
            DEVTOOLS_W,
            28,
        )
    }

    fn fetch_rect(&self) -> Rect {
        let devtools = self.devtools_button_rect();
        Rect::new(devtools.x - FETCH_W as i32 - 8, devtools.y, FETCH_W, 28)
    }

    fn url_rect(&self) -> Rect {
        let method = self.method_rect();
        let fetch = self.fetch_rect();
        Rect::new(
            method.right() + 8,
            method.y,
            (fetch.x - method.right() - 16).max(120) as u32,
            method.h,
        )
    }

    fn content_rect(&self) -> Rect {
        let top = self.top_bar_rect();
        Rect::new(
            PAD,
            top.bottom() + PAD,
            WIN_W.saturating_sub((PAD * 2) as u32),
            (WIN_H as i32 - PAD - top.bottom() - PAD).max(MIN_MAIN_CONTENT_H as i32) as u32,
        )
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

    fn draw_top_bar(&mut self, canvas: &mut Canvas, theme: &Theme) {
        let top = self.top_bar_rect();
        Panel::new(top).draw(canvas, theme);

        let mut method = Button::secondary(self.method_rect(), "GET").with_font(&F_UI);
        method.state = ButtonState::Normal;
        method.draw(canvas, theme);

        self.url_input.rect = self.url_rect();
        self.url_input.draw(canvas, theme);

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

        Label::new(self.status_rect(), self.status.as_str())
            .with_font(&F_SMALL)
            .draw(canvas, theme);
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

        let styles_panel = Panel::with_title(styles_rect, "Styles");
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
            Column {
                header: "Name",
                width: list_panel
                    .content_rect()
                    .w
                    .saturating_sub(92 + 108 + 96 + 88 + 92),
                right_align: false,
            },
            Column {
                header: "Method",
                width: 92,
                right_align: false,
            },
            Column {
                header: "Status",
                width: 108,
                right_align: false,
            },
            Column {
                header: "Type",
                width: 96,
                right_align: false,
            },
            Column {
                header: "Size",
                width: 88,
                right_align: true,
            },
            Column {
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
                        let styles_text = self.developer_tools.dom.styles_text();
                        TextView::new(
                            Panel::with_title(styles_rect, "Styles")
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
                            let styles_text = self.developer_tools.dom.styles_text();
                            let view = TextView::new(
                                Panel::with_title(styles_rect, "Styles")
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
            self.developer_tools
                .dom
                .set_focused_pane(DomInspectorPane::Styles);
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

impl App for RabbitApp {
    fn view(&mut self, canvas: &mut Canvas, theme: &Theme) {
        canvas.fill_rect(Rect::new(0, 0, WIN_W, WIN_H), theme.bg);
        self.draw_top_bar(canvas, theme);

        let source_panel = Panel::with_title(self.source_panel_rect(), "Source");
        source_panel.draw(canvas, theme);
        self.source_view().draw(canvas, theme);

        self.draw_developer_tools(canvas, theme);
    }

    fn update(&mut self, event: Event) -> bool {
        if self.url_input.update(event) {
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
            Event::MouseMove { x: _, y } => {
                let changed = self
                    .developer_tools
                    .panel
                    .update_resize(y, self.content_rect());
                if changed {
                    self.clamp_scrolls();
                }
                changed
            }
            Event::MouseUp { .. } => {
                let was_resizing = self.developer_tools.panel.is_resizing();
                self.developer_tools.panel.finish_resize();
                was_resizing
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
                    self.clamp_scrolls();
                    return true;
                }

                if self.handle_developer_tools_click(point) {
                    return true;
                }

                if self.source_panel_rect().contains(point) {
                    self.focus = FocusPane::Source;
                    return true;
                }
                false
            }
            Event::Key('\n') if self.url_input.active => {
                self.queue_fetch();
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

    let mut app = RabbitApp::new();
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
