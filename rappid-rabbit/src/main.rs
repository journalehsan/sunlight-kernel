#![cfg_attr(not(any(test, feature = "dom")), no_std)]
#![cfg_attr(feature = "dom", allow(dead_code, unused_imports))]
#![no_main]

extern crate alloc;

use alloc::{borrow::Cow, format, string::String, vec::Vec};
use core::alloc::GlobalAlloc;

use rappid_rabbit::{
    body_is_probably_text, build_get_request,
    developer_tools::{
        console::{ConsoleSeverity, ConsoleSource},
        dom_inspector::DomInspectorPane,
        panel::{DeveloperPanelLayout, DeveloperPanelState, MIN_MAIN_CONTENT_H},
        state::DeveloperToolsState,
        tabs::DeveloperToolTab,
    },
    format_url, normalize_url_input,
    resources::{
        discovery::{DiscoveryClassification, ResourceCandidate, ResourceQueue},
        request::RequestState,
    },
};

#[cfg(feature = "dom")]
use golden_fish::parse_html;
use sun_font::{FontRole, VecFont};
use sunlight_fetch::backend::{perform_request, RequestResult};
use sunlight_fetch::FetchError;
use sunlight_http::ParsedUrl;
use sunlight_ipc::{
    debug_log,
    launch_trace::{self, LaunchSource, LaunchTrace},
    process_yield, ProcessExit,
};
use sunlight_ui::widgets::{
    Button, ButtonState, Column, Label, Panel, TabBar, Table, TextInput, TextView,
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
const KEY_HOME: u8 = 0x47;
const KEY_END: u8 = 0x4F;
const KEY_PGUP: u8 = 0x49;
const KEY_PGDN: u8 = 0x51;

const DOM_TREE_ROW_H: u32 = 18;
const NETWORK_DETAIL_H: u32 = 110;

struct BumpAllocator;

unsafe impl GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        const HEAP_SIZE: usize = 8 * 1024 * 1024;
        static mut HEAP: [u8; HEAP_SIZE] = [0; HEAP_SIZE];
        static mut NEXT: usize = 0;
        let aligned = (NEXT + layout.align() - 1) & !(layout.align() - 1);
        let end = aligned + layout.size();
        if end > HEAP_SIZE {
            return core::ptr::null_mut();
        }
        NEXT = end;
        core::ptr::addr_of_mut!(HEAP).cast::<u8>().add(aligned)
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: core::alloc::Layout) {}
}

#[cfg(not(any(test, feature = "dom")))]
#[global_allocator]
static ALLOC: BumpAllocator = BumpAllocator;

#[cfg(not(any(test, feature = "dom")))]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    debug_log("[RABBIT] panic\n");
    loop {
        process_yield();
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FocusPane {
    Source,
    Inspector,
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
    inspector_scroll: usize,
    source: SourceContent,
    inspector_text: String,
    developer_tools: DeveloperToolsState,
    discovered_resources: Vec<ResourceCandidate>,
    resource_queue: ResourceQueue,
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
            inspector_scroll: 0,
            source: SourceContent::Placeholder(
                "Enter a URL, then click Fetch/Open to inspect the response body.",
            ),
            inspector_text: String::from(
                "Method: GET\nURL: \nFinal URL: \nStatus: \nContent-Type: \nBody Size: \nDuration: \nHeaders:\nDiscovered Resources:\n",
            ),
            developer_tools,
            discovered_resources: Vec::new(),
            resource_queue: ResourceQueue::default(),
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
        self.inspector_scroll = 0;

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

        self.begin_navigation_session(&normalized);

        let request = build_get_request(&parsed);
        match perform_request(parsed, request) {
            Ok(result) => self.apply_result(&normalized, result),
            Err(err) => self.apply_fetch_error(err),
        }
    }

    fn begin_navigation_session(&mut self, requested_url: &str) {
        self.discovered_resources.clear();
        self.resource_queue.clear();
        self.developer_tools.network.clear_for_new_page();
        self.developer_tools
            .dom
            .clear_with_message("Waiting for parsed HTML document.");

        let request_index = self
            .developer_tools
            .network
            .begin_main_document_request("GET", String::from(requested_url));
        self.developer_tools
            .network
            .set_request_state(request_index, RequestState::Connecting);
        self.developer_tools.console.push(
            ConsoleSeverity::Quiet,
            ConsoleSource::Fetch,
            format!("Main document request started: {requested_url}"),
        );
    }

    fn apply_result(&mut self, requested_url: &str, result: RequestResult) {
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

        self.status = format!("HTTP {status_code} {status_text}");
        self.source = self.build_source_content(content_type, result.body);
        self.developer_tools
            .network
            .set_request_state(0, RequestState::Receiving);

        self.developer_tools.network.complete_request(
            0,
            status_code,
            result.duration_ms,
            Some(body_size),
            Some(String::from(content_type)),
            Some(false),
            Some(false),
        );

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
                match parse_html(source_text) {
                    Ok(document) => {
                        let total_nodes = document.node_count();
                        if let Some(base_url) = final_url_parsed.as_ref() {
                            let candidates =
                                rappid_rabbit::resources::discovery::discover_resources(
                                    &document, base_url,
                                );
                            self.resource_queue.replace_from_candidates(&candidates);
                            self.discovered_resources = candidates;
                        }
                        self.developer_tools.dom.set_document(document);
                        self.developer_tools.console.push(
                            ConsoleSeverity::Quiet,
                            ConsoleSource::Parser,
                            format!("Golden Fish parsed HTML document ({total_nodes} nodes)."),
                        );
                    }
                    Err(err) => {
                        self.developer_tools
                            .dom
                            .clear_with_message(format!("Parser error: {err}"));
                        self.developer_tools.console.push(
                            ConsoleSeverity::Warn,
                            ConsoleSource::Parser,
                            format!("Golden Fish parse error: {err}"),
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

        self.inspector_text = self.build_inspector_text(
            requested_url,
            &final_url,
            status_code,
            &status_text,
            content_type,
            body_size,
            result.duration_ms,
            "GET",
            &result.response.headers,
        );
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

    #[allow(clippy::too_many_arguments)]
    fn build_inspector_text(
        &self,
        requested_url: &str,
        final_url: &str,
        status_code: u16,
        status_text: &str,
        content_type: &str,
        body_size: usize,
        duration_ms: Option<u64>,
        method: &str,
        headers: &[(String, String)],
    ) -> String {
        let mut text = String::new();
        append_line(&mut text, "Method", method);
        append_line(&mut text, "URL", requested_url);
        append_line(&mut text, "Final URL", final_url);
        append_line(&mut text, "Status", &format!("{status_code} {status_text}"));
        append_line(&mut text, "Content-Type", content_type);
        append_line(&mut text, "Body Size", &format!("{body_size} bytes"));
        append_line(
            &mut text,
            "Duration",
            &duration_ms.map_or_else(|| String::from("n/a"), |ms| format!("{ms} ms")),
        );
        text.push_str("Headers:\n");
        if headers.is_empty() {
            text.push_str("  (none)\n");
        } else {
            for (key, value) in headers {
                text.push_str("  ");
                text.push_str(key);
                text.push_str(": ");
                text.push_str(value);
                text.push('\n');
            }
        }

        text.push_str("Discovered Resources:\n");
        if self.discovered_resources.is_empty() {
            text.push_str("  (none)\n");
        } else {
            for resource in &self.discovered_resources {
                text.push_str("  [");
                text.push_str(resource.classification.label());
                text.push_str(" / ");
                text.push_str(resource.resource_type.label());
                text.push_str("] ");
                text.push_str(&resource.resolved_url);
                if resource.enqueue_for_fetch {
                    text.push_str(" (queued)");
                } else if resource.classification == DiscoveryClassification::OrdinaryNavigationLink
                {
                    text.push_str(" (not auto-fetched)");
                }
                text.push('\n');
            }
        }
        text
    }

    fn apply_fetch_error(&mut self, err: FetchError) {
        self.status = String::from("Request failed");
        self.source = SourceContent::Message(String::from(
            "No response body was captured for this request.",
        ));
        self.inspector_text = String::from(
            "Method: GET\nURL: \nFinal URL: \nStatus: failed\nContent-Type: \nBody Size: 0 bytes\nDuration: n/a\nHeaders:\nDiscovered Resources:\n",
        );
        self.discovered_resources.clear();
        self.resource_queue.clear();
        self.developer_tools
            .dom
            .clear_with_message("No DOM available for the failed request.");
        self.developer_tools
            .network
            .fail_request(0, format!("{err}"));
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
        self.inspector_text = String::from(
            "Method: GET\nURL: \nFinal URL: \nStatus: not sent\nContent-Type: \nBody Size: 0 bytes\nDuration: n/a\nHeaders:\nDiscovered Resources:\n",
        );
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
        let main = self.developer_panel_layout().main_rect;
        let source_w = ((main.w as i32 * 62) / 100).max(320) as u32;
        Rect::new(main.x, main.y, source_w, main.h)
    }

    fn inspector_panel_rect(&mut self) -> Rect {
        let main = self.developer_panel_layout().main_rect;
        let source = self.source_panel_rect();
        Rect::new(
            source.right() + GUTTER,
            main.y,
            (main.right() - source.right() - GUTTER).max(240) as u32,
            main.h,
        )
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

    fn inspector_view(&mut self) -> TextView<'_> {
        let content = Panel::with_title(self.inspector_panel_rect(), "Inspector")
            .content_rect()
            .inset(8);
        TextView::new(content, self.inspector_text.as_str())
            .with_scroll_offset(self.inspector_scroll)
            .with_focus(self.focus == FocusPane::Inspector)
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
                TextView::new(
                    console_rect,
                    self.developer_tools.console.rendered_text().as_str(),
                )
                .with_scroll_offset(self.developer_tools.console.scroll_offset())
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
        TextView::new(
            styles_panel.content_rect().inset(8),
            self.developer_tools.dom.styles_text().as_str(),
        )
        .with_scroll_offset(self.developer_tools.dom.styles_scroll())
        .with_focus(
            self.focus == FocusPane::DeveloperTools
                && self.developer_tools.dom.focused_pane() == DomInspectorPane::Styles,
        )
        .with_font(&F_MONO)
        .draw(canvas, theme);

        let properties_panel = Panel::with_title(properties_rect, "Properties");
        properties_panel.draw(canvas, theme);
        TextView::new(
            properties_panel.content_rect().inset(8),
            self.developer_tools.dom.node_properties_text().as_str(),
        )
        .with_scroll_offset(self.developer_tools.dom.properties_scroll())
        .with_focus(
            self.focus == FocusPane::DeveloperTools
                && self.developer_tools.dom.focused_pane() == DomInspectorPane::Properties,
        )
        .with_font(&F_MONO)
        .draw(canvas, theme);

        self.draw_dom_tree(canvas, theme, tree_rect);
    }

    fn draw_dom_tree(&mut self, canvas: &mut Canvas, theme: &Theme, rect: Rect) {
        let panel = Panel::with_title(rect, "DOM Tree");
        panel.draw(canvas, theme);
        let content = panel.content_rect().inset(8);
        let rows = self.developer_tools.dom.tree_rows();

        if rows.is_empty() {
            TextView::new(content, self.developer_tools.dom.empty_message())
                .with_focus(
                    self.focus == FocusPane::DeveloperTools
                        && self.developer_tools.dom.focused_pane() == DomInspectorPane::Tree,
                )
                .with_font(&F_MONO)
                .draw(canvas, theme);
            return;
        }

        let visible = ((content.h.saturating_sub(4)) / DOM_TREE_ROW_H).max(1) as usize;
        let max_scroll = rows.len().saturating_sub(visible);
        let scroll = self.developer_tools.dom.tree_scroll().min(max_scroll);
        let base_x = content.x + 4;
        let selected = self.developer_tools.dom.selected_node();

        for (local_index, row) in rows.iter().skip(scroll).take(visible).enumerate() {
            let row_y = content.y + 4 + (local_index as u32 * DOM_TREE_ROW_H) as i32;
            let row_rect = Rect::new(content.x, row_y - 1, content.w, DOM_TREE_ROW_H);
            if selected == Some(row.node_id) {
                canvas.fill_rect(row_rect, theme.accent.darken(180));
            }

            let indent = (row.depth as i32) * 14;
            let marker_x = base_x + indent;
            if row.has_children {
                let marker = if row.expanded { "[-]" } else { "[+]" };
                F_MONO.draw_vcenter(
                    canvas,
                    marker,
                    marker_x,
                    row_y,
                    DOM_TREE_ROW_H,
                    if selected == Some(row.node_id) {
                        theme.accent
                    } else {
                        theme.text_dim
                    },
                );
            }

            let label_x = marker_x + if row.has_children { 28 } else { 14 };
            let clipped = clip_to_width(&row.label, content.right() - label_x - 8, &F_MONO);
            F_MONO.draw_vcenter(
                canvas,
                clipped.as_str(),
                label_x,
                row_y,
                DOM_TREE_ROW_H,
                if selected == Some(row.node_id) {
                    theme.accent
                } else {
                    theme.text
                },
            );
        }
    }

    fn network_layout(content_rect: Rect) -> (Rect, Rect) {
        let table_h = content_rect
            .h
            .saturating_sub(NETWORK_DETAIL_H)
            .saturating_sub(GUTTER.max(0) as u32);
        let table_rect = Rect::new(
            content_rect.x,
            content_rect.y,
            content_rect.w,
            table_h.max(140),
        );
        let detail_rect = Rect::new(
            content_rect.x,
            table_rect.bottom() + GUTTER,
            content_rect.w,
            content_rect
                .bottom()
                .saturating_sub(table_rect.bottom() + GUTTER) as u32,
        );
        (table_rect, detail_rect)
    }

    fn network_header_h() -> u32 {
        (F_SMALL.line_height() + 8).max(18)
    }

    fn network_row_h() -> u32 {
        (F_SMALL.line_height() + 5).max(16)
    }

    fn draw_network_tab(&mut self, canvas: &mut Canvas, theme: &Theme, content_rect: Rect) {
        let (table_rect, detail_rect) = Self::network_layout(content_rect.inset(8));
        let table_panel = Panel::with_title(table_rect, "Requests");
        table_panel.draw(canvas, theme);
        let detail_panel = Panel::with_title(detail_rect, "Details");
        detail_panel.draw(canvas, theme);

        let rows = self.developer_tools.network.summary_rows();
        let columns = [
            Column {
                header: "Name",
                width: table_panel
                    .content_rect()
                    .w
                    .saturating_sub(100 + 100 + 110 + 95 + 100),
                right_align: false,
            },
            Column {
                header: "Method",
                width: 100,
                right_align: false,
            },
            Column {
                header: "Type",
                width: 100,
                right_align: false,
            },
            Column {
                header: "Status",
                width: 110,
                right_align: false,
            },
            Column {
                header: "Size",
                width: 95,
                right_align: true,
            },
            Column {
                header: "Duration",
                width: 100,
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

        Table::new(table_panel.content_rect(), &columns, &row_slices)
            .with_selected(self.developer_tools.network.selected_entry())
            .with_scroll_offset(self.developer_tools.network.scroll_offset())
            .with_font(&F_SMALL)
            .draw(canvas, theme);

        TextView::new(
            detail_panel.content_rect().inset(8),
            self.developer_tools
                .network
                .selected_entry_detail_text()
                .as_str(),
        )
        .with_focus(self.focus == FocusPane::DeveloperTools)
        .with_font(&F_MONO)
        .draw(canvas, theme);
    }

    fn clamp_scrolls(&mut self) {
        self.source_scroll = self.source_scroll.min(self.source_view().max_scroll());
        self.inspector_scroll = self
            .inspector_scroll
            .min(self.inspector_view().max_scroll());

        let dev_layout = self.developer_panel_layout();
        if let Some(content_rect) = dev_layout.content_rect {
            match self.developer_tools.panel.active_tab {
                DeveloperToolTab::Console => {
                    let console_text = self.developer_tools.console.rendered_text();
                    let max = TextView::new(content_rect.inset(8), console_text.as_str())
                        .with_font(&F_MONO)
                        .max_scroll();
                    self.developer_tools
                        .console
                        .set_scroll_offset(self.developer_tools.console.scroll_offset().min(max));
                }
                DeveloperToolTab::DomInspector => {
                    let (styles_rect, properties_rect, tree_rect) =
                        Self::dom_column_rects(content_rect.inset(8));
                    let styles_text = self.developer_tools.dom.styles_text();
                    let styles_max = TextView::new(
                        Panel::with_title(styles_rect, "Styles")
                            .content_rect()
                            .inset(8),
                        styles_text.as_str(),
                    )
                    .with_font(&F_MONO)
                    .max_scroll();
                    let properties_text = self.developer_tools.dom.node_properties_text();
                    let properties_max = TextView::new(
                        Panel::with_title(properties_rect, "Properties")
                            .content_rect()
                            .inset(8),
                        properties_text.as_str(),
                    )
                    .with_font(&F_MONO)
                    .max_scroll();
                    let tree_content = Panel::with_title(tree_rect, "DOM Tree")
                        .content_rect()
                        .inset(8);
                    let visible =
                        ((tree_content.h.saturating_sub(4)) / DOM_TREE_ROW_H).max(1) as usize;
                    let tree_max = self
                        .developer_tools
                        .dom
                        .tree_rows()
                        .len()
                        .saturating_sub(visible);
                    self.developer_tools.dom.set_styles_scroll(
                        self.developer_tools.dom.styles_scroll().min(styles_max),
                    );
                    self.developer_tools.dom.set_properties_scroll(
                        self.developer_tools
                            .dom
                            .properties_scroll()
                            .min(properties_max),
                    );
                    self.developer_tools
                        .dom
                        .set_tree_scroll(self.developer_tools.dom.tree_scroll().min(tree_max));
                }
                DeveloperToolTab::Network => {
                    let (table_rect, _) = Self::network_layout(content_rect.inset(8));
                    let content = Panel::with_title(table_rect, "Requests").content_rect();
                    let visible = ((content.h.saturating_sub(Self::network_header_h()))
                        / Self::network_row_h())
                    .max(1) as usize;
                    let max = self
                        .developer_tools
                        .network
                        .entries()
                        .len()
                        .saturating_sub(visible);
                    self.developer_tools
                        .network
                        .set_scroll_offset(self.developer_tools.network.scroll_offset().min(max));
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
            FocusPane::Inspector => {
                let (visible, max_scroll) = {
                    let view = self.inspector_view();
                    (view.visible_line_count(), view.max_scroll())
                };
                adjust_scroll(
                    &mut self.inspector_scroll,
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
                let console_text = self.developer_tools.console.rendered_text();
                let view =
                    TextView::new(content_rect.inset(8), console_text.as_str()).with_font(&F_MONO);
                let mut scroll = self.developer_tools.console.scroll_offset();
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
                        let styles_text = self.developer_tools.dom.styles_text();
                        let view = TextView::new(
                            Panel::with_title(styles_rect, "Styles")
                                .content_rect()
                                .inset(8),
                            styles_text.as_str(),
                        )
                        .with_font(&F_MONO);
                        let mut scroll = self.developer_tools.dom.styles_scroll();
                        adjust_scroll(
                            &mut scroll,
                            delta,
                            page,
                            home,
                            end,
                            view.visible_line_count(),
                            view.max_scroll(),
                        );
                        self.developer_tools.dom.set_styles_scroll(scroll);
                    }
                    DomInspectorPane::Properties => {
                        let properties_text = self.developer_tools.dom.node_properties_text();
                        let view = TextView::new(
                            Panel::with_title(properties_rect, "Properties")
                                .content_rect()
                                .inset(8),
                            properties_text.as_str(),
                        )
                        .with_font(&F_MONO);
                        let mut scroll = self.developer_tools.dom.properties_scroll();
                        adjust_scroll(
                            &mut scroll,
                            delta,
                            page,
                            home,
                            end,
                            view.visible_line_count(),
                            view.max_scroll(),
                        );
                        self.developer_tools.dom.set_properties_scroll(scroll);
                    }
                    DomInspectorPane::Tree => {
                        let content = Panel::with_title(tree_rect, "DOM Tree")
                            .content_rect()
                            .inset(8);
                        let visible =
                            ((content.h.saturating_sub(4)) / DOM_TREE_ROW_H).max(1) as usize;
                        let max_scroll = self
                            .developer_tools
                            .dom
                            .tree_rows()
                            .len()
                            .saturating_sub(visible);
                        let mut scroll = self.developer_tools.dom.tree_scroll();
                        adjust_scroll(&mut scroll, delta, page, home, end, visible, max_scroll);
                        self.developer_tools.dom.set_tree_scroll(scroll);
                    }
                }
            }
            DeveloperToolTab::Network => {
                let (table_rect, _) = Self::network_layout(content_rect.inset(8));
                let content = Panel::with_title(table_rect, "Requests").content_rect();
                let visible = ((content.h.saturating_sub(Self::network_header_h()))
                    / Self::network_row_h())
                .max(1) as usize;
                let max_scroll = self
                    .developer_tools
                    .network
                    .entries()
                    .len()
                    .saturating_sub(visible);
                let mut scroll = self.developer_tools.network.scroll_offset();
                adjust_scroll(&mut scroll, delta, page, home, end, visible, max_scroll);
                self.developer_tools.network.set_scroll_offset(scroll);
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
        let rows = self.developer_tools.dom.tree_rows();
        let visible = ((content.h.saturating_sub(4)) / DOM_TREE_ROW_H).max(1) as usize;
        let scroll = self.developer_tools.dom.tree_scroll();
        let local_y = point.y - content.y - 4;
        if local_y < 0 {
            return true;
        }
        let local_row = (local_y as u32 / DOM_TREE_ROW_H) as usize;
        if local_row >= visible {
            return true;
        }
        let Some(row) = rows.get(scroll + local_row) else {
            return true;
        };

        let toggle_zone_end = content.x + 4 + (row.depth as i32) * 14 + 24;
        if row.has_children && point.x <= toggle_zone_end {
            self.developer_tools.dom.toggle_node(row.node_id);
        } else {
            self.developer_tools.dom.select_node(row.node_id);
        }
        self.clamp_scrolls();
        true
    }

    fn handle_network_click(&mut self, point: Point, content_rect: Rect) -> bool {
        let (table_rect, detail_rect) = Self::network_layout(content_rect.inset(8));
        if detail_rect.contains(point) {
            return true;
        }
        if !table_rect.contains(point) {
            return false;
        }

        let content = Panel::with_title(table_rect, "Requests").content_rect();
        let rel_y = point.y - content.y - Self::network_header_h() as i32;
        if rel_y < 0 {
            return true;
        }
        let local_row = (rel_y as u32 / Self::network_row_h()) as usize;
        let row_index = self.developer_tools.network.scroll_offset() + local_row;
        if row_index < self.developer_tools.network.entries().len() {
            self.developer_tools.network.select_entry(Some(row_index));
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

        let inspector_panel = Panel::with_title(self.inspector_panel_rect(), "Inspector");
        inspector_panel.draw(canvas, theme);
        self.inspector_view().draw(canvas, theme);

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
                if self.inspector_panel_rect().contains(point) {
                    self.focus = FocusPane::Inspector;
                    return true;
                }
                false
            }
            Event::Key('\n') if self.url_input.active => {
                self.queue_fetch();
                true
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

#[cfg(not(any(test, feature = "dom")))]
#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8, _envp: *const *const u8) -> ! {
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

fn append_line(out: &mut String, label: &str, value: &str) {
    out.push_str(label);
    out.push_str(": ");
    out.push_str(value);
    out.push('\n');
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

fn clip_to_width(value: &str, max_width: i32, font: &dyn VecText) -> String {
    if max_width <= 0 {
        return String::new();
    }
    if font.measure_w(value) as i32 <= max_width {
        return String::from(value);
    }

    let ellipsis = "...";
    let ellipsis_w = font.measure_w(ellipsis) as i32;
    let mut out = String::new();
    for ch in value.chars() {
        let next_len = {
            let mut candidate = out.clone();
            candidate.push(ch);
            candidate.push_str(ellipsis);
            candidate
        };
        if font.measure_w(next_len.as_str()) as i32 > max_width.max(ellipsis_w) {
            break;
        }
        out.push(ch);
    }
    out.push_str(ellipsis);
    out
}
