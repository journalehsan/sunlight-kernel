#![no_std]
#![no_main]

extern crate alloc;

use alloc::{borrow::Cow, format, string::String, vec::Vec};
use core::alloc::GlobalAlloc;

use sun_font::{FontRole, VecFont};
use sunlight_api_lab::{
    body_is_probably_text, build_request, format_url, normalize_url_input, HttpMethod,
};
use sunlight_fetch::backend::{perform_request, RequestResult};
use sunlight_fetch::FetchError;
use sunlight_http::ParsedUrl;
use sunlight_ipc::{
    debug_log,
    launch_trace::{self, LaunchSource, LaunchTrace},
    process_yield, ProcessExit,
};
use sunlight_ui::widgets::{Button, ButtonState, Label, Panel, TabBar, TextInput, TextView};
use sunlight_ui::{request_close, App, Event, Point, Rect, Window, WindowConfig, WindowDecoration};

static F_UI: VecFont = VecFont(FontRole::UiRegular);
static F_SMALL: VecFont = VecFont(FontRole::UiSmall);
static F_MONO: VecFont = VecFont(FontRole::MonoRegular);

const WIN_W: u32 = 1140;
const WIN_H: u32 = 760;
const PAD: i32 = 12;
const TOP_BAR_H: u32 = 42;
const CONSOLE_H: u32 = 88;
const REQUEST_H: u32 = 168;
const SIDEBAR_W: u32 = 156;
const METHOD_W: u32 = 64;
const SEND_W: u32 = 88;
const STATUS_W: u32 = 220;
const GUTTER: i32 = 10;
const URL_INPUT_CAP: usize = 512;
const BODY_INPUT_CAP: usize = 2048;
const CONTENT_TYPE_CAP: usize = 128;

const KEY_Q: u8 = 0x10;
const KEY_UP: u8 = 0x48;
const KEY_DOWN: u8 = 0x50;
const KEY_HOME: u8 = 0x47;
const KEY_END: u8 = 0x4F;
const KEY_PGUP: u8 = 0x49;
const KEY_PGDN: u8 = 0x51;

const REQUEST_TABS: [&str; 4] = ["Params", "Headers", "Body", "Auth"];

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

#[global_allocator]
static ALLOC: BumpAllocator = BumpAllocator;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    debug_log("[API-LAB] panic\n");
    loop {
        process_yield();
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FocusPane {
    Request,
    ResponseBody,
    ResponseDetails,
    Console,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ConsoleSeverity {
    Quiet,
    Warn,
    Error,
}

enum ResponseContent {
    Placeholder(&'static str),
    Utf8(Vec<u8>),
    Text(String),
    Message(String),
}

impl ResponseContent {
    fn as_str(&self) -> &str {
        match self {
            Self::Placeholder(text) => text,
            Self::Utf8(bytes) => core::str::from_utf8(bytes).unwrap_or(""),
            Self::Text(text) | Self::Message(text) => text.as_str(),
        }
    }
}

struct ApiLabApp {
    url_input: TextInput<'static, URL_INPUT_CAP>,
    body_input: TextInput<'static, BODY_INPUT_CAP>,
    content_type_input: TextInput<'static, CONTENT_TYPE_CAP>,
    method: HttpMethod,
    request_tab: usize,
    status: String,
    pending_send: bool,
    focus: FocusPane,
    response_scroll: usize,
    details_scroll: usize,
    console_scroll: usize,
    request_scroll: usize,
    response: ResponseContent,
    details_text: String,
    console_text: String,
    console_severity: ConsoleSeverity,
    request_tab_display: String,
}

impl ApiLabApp {
    fn new() -> Self {
        let mut url_input = TextInput::new(Rect::default())
            .with_font(&F_UI)
            .with_placeholder("Enter URL (http:// or https://)");
        url_input.set_text("http://example.com/");

        let body_input = TextInput::new(Rect::default())
            .with_font(&F_MONO)
            .with_placeholder("Raw request body (ignored for GET)");
        let content_type_input = TextInput::new(Rect::default())
            .with_font(&F_MONO)
            .with_placeholder("Content-Type (defaults to text/plain when body is set)");

        Self {
            url_input,
            body_input,
            content_type_input,
            method: HttpMethod::Get,
            request_tab: 2,
            status: String::from("Ready"),
            pending_send: false,
            focus: FocusPane::ResponseBody,
            response_scroll: 0,
            details_scroll: 0,
            console_scroll: 0,
            request_scroll: 0,
            response: ResponseContent::Placeholder(
                "Send a request to inspect the response body here.",
            ),
            details_text: String::from(
                "Status: \nFinal URL: \nDuration: \nBody Size: \nResponse Headers:\n",
            ),
            console_text: String::new(),
            console_severity: ConsoleSeverity::Quiet,
            request_tab_display: String::new(),
        }
    }

    fn refresh_request_tab_display(&mut self) {
        self.request_tab_display = match self.request_tab {
            0 => String::from(
                "Query parameters\n\n\
                 Params table coming soon.\n\
                 Use the URL path for query strings in this MVP.",
            ),
            1 => {
                let mut text = String::from(
                    "Request headers\n\n\
                     Custom header rows coming soon.\n\
                     Content-Type (POST):\n  ",
                );
                let content_type = self.content_type_input.value().trim();
                if content_type.is_empty() {
                    text.push_str("(defaults to text/plain when body is set)");
                } else {
                    text.push_str(content_type);
                }
                text.push_str("\n\nEdit Content-Type in the Body tab field below.");
                text
            }
            2 => {
                let mut text = String::from("Request body\n\n");
                let body = self.body_input.value().trim();
                if body.is_empty() {
                    text.push_str("(empty — POST sends no body)");
                } else {
                    text.push_str(body);
                }
                text.push_str("\n\nNote: single-line raw body editor for this MVP.");
                text
            }
            _ => String::from(
                "Auth: None\n\n\
                 Basic Auth and Bearer token support coming soon.",
            ),
        };
    }

    fn queue_send(&mut self) {
        self.pending_send = true;
        self.status = String::from("Sending...");
        self.console_text.clear();
        self.console_severity = ConsoleSeverity::Quiet;
    }

    fn perform_pending_send(&mut self) {
        self.pending_send = false;
        self.response_scroll = 0;
        self.details_scroll = 0;
        self.console_scroll = 0;

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

        let request = build_request(
            self.method,
            &parsed,
            self.content_type_input.value(),
            self.body_input.value(),
        );
        match perform_request(parsed, request) {
            Ok(result) => self.apply_result(&normalized, result),
            Err(err) => self.apply_fetch_error(err),
        }
    }

    fn apply_result(&mut self, requested_url: &str, result: RequestResult) {
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
            .unwrap_or_else(|| String::from(requested_url));
        let content_type = result
            .response
            .header("content-type")
            .map_or("(missing)", |value| value);
        let body_size = result.body.len();

        self.status = format!("HTTP {status_code} {status_text}");
        self.response = self.build_response_content(content_type, result.body);
        self.details_text = self.build_details_text(
            status_code,
            &status_text,
            &final_url,
            content_type,
            body_size,
            result.duration_ms,
            &result.response.headers,
        );

        if (400..500).contains(&status_code) {
            self.console_text = format!("Client error: HTTP {status_code} {status_text}");
            self.console_severity = ConsoleSeverity::Warn;
        } else if status_code >= 500 {
            self.console_text = format!("Server error: HTTP {status_code} {status_text}");
            self.console_severity = ConsoleSeverity::Error;
        } else {
            self.console_text.clear();
            self.console_severity = ConsoleSeverity::Quiet;
        }
    }

    fn build_response_content(&self, content_type: &str, body: Vec<u8>) -> ResponseContent {
        if body.is_empty() {
            return ResponseContent::Message(String::from("Empty response body."));
        }

        if core::str::from_utf8(&body).is_ok() {
            return ResponseContent::Utf8(body);
        }

        if body_is_probably_text(Some(content_type), &body) {
            let lossy: Cow<'_, str> = String::from_utf8_lossy(&body);
            return ResponseContent::Text(lossy.into_owned());
        }

        ResponseContent::Message(format!(
            "Binary response body ({} bytes).\nContent-Type: {content_type}",
            body.len()
        ))
    }

    #[allow(clippy::too_many_arguments)]
    fn build_details_text(
        &self,
        status_code: u16,
        status_text: &str,
        final_url: &str,
        content_type: &str,
        body_size: usize,
        duration_ms: Option<u64>,
        headers: &[(String, String)],
    ) -> String {
        let mut text = String::new();
        append_line(&mut text, "Method", self.method.as_str());
        append_line(&mut text, "Status", &format!("{status_code} {status_text}"));
        append_line(&mut text, "Final URL", final_url);
        append_line(&mut text, "Content-Type", content_type);
        append_line(&mut text, "Body Size", &format!("{body_size} bytes"));
        append_line(
            &mut text,
            "Duration",
            &duration_ms.map_or_else(|| String::from("n/a"), |ms| format!("{ms} ms")),
        );
        text.push_str("Response Headers:\n");
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
        text
    }

    fn apply_fetch_error(&mut self, err: FetchError) {
        self.status = String::from("Request failed");
        self.response = ResponseContent::Message(String::from(
            "No response body was captured for this request.",
        ));
        self.details_text = String::from(
            "Status: failed\nFinal URL: \nDuration: n/a\nBody Size: 0 bytes\nResponse Headers:\n",
        );
        self.console_text = format!("{err}");
        self.console_severity = match err {
            FetchError::InvalidUrl(_) | FetchError::HttpError { .. } => ConsoleSeverity::Warn,
            _ => ConsoleSeverity::Error,
        };
    }

    fn set_error(&mut self, message: String) {
        self.status = String::from("Invalid URL");
        self.response = ResponseContent::Message(String::from("Fix the URL and send again."));
        self.details_text = String::from(
            "Status: not sent\nFinal URL: \nDuration: n/a\nBody Size: 0 bytes\nResponse Headers:\n",
        );
        self.console_text = message;
        self.console_severity = ConsoleSeverity::Warn;
    }

    fn top_bar_rect(&self) -> Rect {
        Rect::new(PAD, PAD, WIN_W.saturating_sub((PAD * 2) as u32), TOP_BAR_H)
    }

    fn method_rect(&self) -> Rect {
        let top = self.top_bar_rect();
        Rect::new(top.x + 8, top.y + 7, METHOD_W, 28)
    }

    fn send_rect(&self) -> Rect {
        let top = self.top_bar_rect();
        Rect::new(
            top.right() - STATUS_W as i32 - SEND_W as i32 - 12,
            top.y + 7,
            SEND_W,
            28,
        )
    }

    fn status_rect(&self) -> Rect {
        let top = self.top_bar_rect();
        Rect::new(top.right() - STATUS_W as i32 - 8, top.y + 6, STATUS_W, 30)
    }

    fn url_rect(&self) -> Rect {
        let method = self.method_rect();
        let send = self.send_rect();
        Rect::new(
            method.right() + 8,
            method.y,
            (send.x - method.right() - 16).max(120) as u32,
            method.h,
        )
    }

    fn sidebar_rect(&self) -> Rect {
        let top = self.top_bar_rect();
        let console = self.console_panel_rect();
        Rect::new(
            PAD,
            top.bottom() + PAD,
            SIDEBAR_W,
            (console.y - top.bottom() - PAD * 2).max(200) as u32,
        )
    }

    fn console_panel_rect(&self) -> Rect {
        Rect::new(
            PAD,
            WIN_H as i32 - PAD - CONSOLE_H as i32,
            WIN_W.saturating_sub((PAD * 2) as u32),
            CONSOLE_H,
        )
    }

    fn content_rect(&self) -> Rect {
        let top = self.top_bar_rect();
        let sidebar = self.sidebar_rect();
        let console = self.console_panel_rect();
        Rect::new(
            sidebar.right() + GUTTER,
            top.bottom() + PAD,
            (WIN_W as i32 - sidebar.right() - GUTTER - PAD).max(400) as u32,
            (console.y - top.bottom() - PAD * 2).max(240) as u32,
        )
    }

    fn request_panel_rect(&self) -> Rect {
        let content = self.content_rect();
        Rect::new(content.x, content.y, content.w, REQUEST_H.min(content.h))
    }

    fn response_area_rect(&self) -> Rect {
        let content = self.content_rect();
        let request = self.request_panel_rect();
        Rect::new(
            content.x,
            request.bottom() + GUTTER,
            content.w,
            (content.bottom() - request.bottom() - GUTTER).max(120) as u32,
        )
    }

    fn request_tab_bar_rect(&self) -> Rect {
        let panel = self.request_panel_rect();
        Rect::new(panel.x + 8, panel.y + 28, panel.w.saturating_sub(16), 28)
    }

    fn request_tab_content_rect(&self) -> Rect {
        let panel = self.request_panel_rect();
        let tabs = self.request_tab_bar_rect();
        Rect::new(
            panel.x + 8,
            tabs.bottom() + 4,
            panel.w.saturating_sub(16),
            (panel.bottom() - tabs.bottom() - 12).max(48) as u32,
        )
    }

    fn body_editor_rect(&self) -> Rect {
        let content = self.request_tab_content_rect();
        Rect::new(content.x, content.bottom() - 30, content.w, 28)
    }

    fn content_type_editor_rect(&self) -> Rect {
        let body = self.body_editor_rect();
        Rect::new(body.x, body.y - 34, body.w, 28)
    }

    fn response_body_rect(&self) -> Rect {
        let area = self.response_area_rect();
        let body_w = ((area.w as i32 * 62) / 100).max(300) as u32;
        Rect::new(area.x, area.y, body_w, area.h)
    }

    fn response_details_rect(&self) -> Rect {
        let area = self.response_area_rect();
        let body = self.response_body_rect();
        Rect::new(
            body.right() + GUTTER,
            area.y,
            (area.right() - body.right() - GUTTER).max(220) as u32,
            area.h,
        )
    }

    fn request_tab_view(&self) -> TextView<'_> {
        let content = self.request_tab_content_rect();
        TextView::new(content, self.request_tab_display.as_str())
            .with_scroll_offset(self.request_scroll)
            .with_focus(self.focus == FocusPane::Request)
            .with_font(&F_MONO)
    }

    fn response_view(&self) -> TextView<'_> {
        let content = Panel::with_title(self.response_body_rect(), "Response Body")
            .content_rect()
            .inset(8);
        TextView::new(content, self.response.as_str())
            .with_scroll_offset(self.response_scroll)
            .with_focus(self.focus == FocusPane::ResponseBody)
            .with_font(&F_MONO)
    }

    fn details_view(&self) -> TextView<'_> {
        let content = Panel::with_title(self.response_details_rect(), "Response")
            .content_rect()
            .inset(8);
        TextView::new(content, self.details_text.as_str())
            .with_scroll_offset(self.details_scroll)
            .with_focus(self.focus == FocusPane::ResponseDetails)
            .with_font(&F_MONO)
    }

    fn console_view(&self) -> TextView<'_> {
        let content = Panel::with_title(self.console_panel_rect(), "Console")
            .content_rect()
            .inset(8);
        let color = match self.console_severity {
            ConsoleSeverity::Quiet => None,
            ConsoleSeverity::Warn => Some(sunlight_ui::Theme::sunlight_dark().warn),
            ConsoleSeverity::Error => Some(sunlight_ui::Theme::sunlight_dark().danger),
        };
        let mut view = TextView::new(content, self.console_text.as_str())
            .with_scroll_offset(self.console_scroll)
            .with_focus(self.focus == FocusPane::Console)
            .with_font(&F_MONO);
        if let Some(color) = color {
            view = view.with_text_color(color);
        }
        view
    }

    fn clamp_scrolls(&mut self) {
        self.request_scroll = self.request_scroll.min(self.request_tab_view().max_scroll());
        self.response_scroll = self.response_scroll.min(self.response_view().max_scroll());
        self.details_scroll = self.details_scroll.min(self.details_view().max_scroll());
        self.console_scroll = self.console_scroll.min(self.console_view().max_scroll());
    }

    fn scroll_focused(&mut self, delta: i32, page: bool, home: bool, end: bool) {
        match self.focus {
            FocusPane::Request => {
                let (visible, max_scroll) = {
                    let view = self.request_tab_view();
                    (view.visible_line_count(), view.max_scroll())
                };
                adjust_scroll(
                    &mut self.request_scroll,
                    delta,
                    page,
                    home,
                    end,
                    visible,
                    max_scroll,
                );
            }
            FocusPane::ResponseBody => {
                let (visible, max_scroll) = {
                    let view = self.response_view();
                    (view.visible_line_count(), view.max_scroll())
                };
                adjust_scroll(
                    &mut self.response_scroll,
                    delta,
                    page,
                    home,
                    end,
                    visible,
                    max_scroll,
                );
            }
            FocusPane::ResponseDetails => {
                let (visible, max_scroll) = {
                    let view = self.details_view();
                    (view.visible_line_count(), view.max_scroll())
                };
                adjust_scroll(
                    &mut self.details_scroll,
                    delta,
                    page,
                    home,
                    end,
                    visible,
                    max_scroll,
                );
            }
            FocusPane::Console => {
                let (visible, max_scroll) = {
                    let view = self.console_view();
                    (view.visible_line_count(), view.max_scroll())
                };
                adjust_scroll(
                    &mut self.console_scroll,
                    delta,
                    page,
                    home,
                    end,
                    visible,
                    max_scroll,
                );
            }
        }
    }
}

impl App for ApiLabApp {
    fn view(&mut self, canvas: &mut sunlight_ui::Canvas, theme: &sunlight_ui::Theme) {
        self.refresh_request_tab_display();
        canvas.fill_rect(Rect::new(0, 0, WIN_W, WIN_H), theme.bg);

        let top = self.top_bar_rect();
        Panel::new(top).draw(canvas, theme);

        let mut method = Button::secondary(self.method_rect(), self.method.as_str()).with_font(&F_UI);
        method.state = ButtonState::Normal;
        method.draw(canvas, theme);

        self.url_input.rect = self.url_rect();
        self.url_input.draw(canvas, theme);

        let mut send = Button::new(self.send_rect(), "Send").with_font(&F_UI);
        send.state = ButtonState::Normal;
        send.draw(canvas, theme);

        Label::new(self.status_rect(), self.status.as_str())
            .with_font(&F_SMALL)
            .draw(canvas, theme);

        let sidebar = Panel::with_title(self.sidebar_rect(), "Library");
        sidebar.draw(canvas, theme);
        let sidebar_content = sidebar.content_rect().inset(8);
        Label::new(
            Rect::new(sidebar_content.x, sidebar_content.y, sidebar_content.w, 20),
            "Collections (coming soon)",
        )
        .with_font(&F_SMALL)
        .draw(canvas, theme);
        Label::new(
            Rect::new(
                sidebar_content.x,
                sidebar_content.y + 28,
                sidebar_content.w,
                20,
            ),
            "Recent Requests (coming soon)",
        )
        .with_font(&F_SMALL)
        .draw(canvas, theme);

        let request_panel = Panel::with_title(self.request_panel_rect(), "Request");
        request_panel.draw(canvas, theme);
        TabBar::new(self.request_tab_bar_rect(), &REQUEST_TABS, self.request_tab)
            .draw(canvas, theme);

        if self.request_tab == 2 {
            self.content_type_input.rect = self.content_type_editor_rect();
            self.content_type_input.draw(canvas, theme);
            self.body_input.rect = self.body_editor_rect();
            self.body_input.draw(canvas, theme);
        }

        self.request_tab_view().draw(canvas, theme);

        let response_panel = Panel::with_title(self.response_body_rect(), "Response Body");
        response_panel.draw(canvas, theme);
        self.response_view().draw(canvas, theme);

        let details_panel = Panel::with_title(self.response_details_rect(), "Response");
        details_panel.draw(canvas, theme);
        self.details_view().draw(canvas, theme);

        let console_panel = Panel::with_title(self.console_panel_rect(), "Console");
        console_panel.draw(canvas, theme);
        self.console_view().draw(canvas, theme);
    }

    fn update(&mut self, event: Event) -> bool {
        if self.url_input.update(event) {
            return true;
        }
        if self.request_tab == 2 {
            if self.content_type_input.update(event) {
                return true;
            }
            if self.body_input.update(event) {
                return true;
            }
        }

        match event {
            Event::Tick => {
                if self.pending_send {
                    self.perform_pending_send();
                    self.clamp_scrolls();
                    return true;
                }
                false
            }
            Event::Click { x, y } => {
                let point = Point::new(x, y);
                if Button::new(self.send_rect(), "Send").hit_test(x, y) {
                    self.queue_send();
                    return true;
                }
                if Button::secondary(self.method_rect(), self.method.as_str()).hit_test(x, y) {
                    self.method = self.method.next();
                    return true;
                }
                if let Some(tab) = TabBar::new(self.request_tab_bar_rect(), &REQUEST_TABS, 0)
                    .hit_test(x, y)
                {
                    self.request_tab = tab;
                    self.request_scroll = 0;
                    self.refresh_request_tab_display();
                    return true;
                }
                if self.request_panel_rect().contains(point) {
                    self.focus = FocusPane::Request;
                    return true;
                }
                if self.response_body_rect().contains(point) {
                    self.focus = FocusPane::ResponseBody;
                    return true;
                }
                if self.response_details_rect().contains(point) {
                    self.focus = FocusPane::ResponseDetails;
                    return true;
                }
                if self.console_panel_rect().contains(point) {
                    self.focus = FocusPane::Console;
                    return true;
                }
                false
            }
            Event::Key('\n') if self.url_input.active => {
                self.queue_send();
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

#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8, _envp: *const *const u8) -> ! {
    sunlight_libc::launch_trace::init_from_argv(argc, argv);
    let trace = launch_trace::current().unwrap_or(LaunchTrace::new(0, LaunchSource::Unknown, 0));
    launch_trace::log_phase_now(
        trace,
        "app=sunlight-api-lab",
        "app_main_started",
        Some(sunlight_ipc::getpid()),
    );

    let mut app = ApiLabApp::new();
    let Some(mut window) = Window::connect(WindowConfig {
        width: WIN_W,
        height: WIN_H,
        title: "Sunlight API Lab",
        decoration: WindowDecoration::Normal,
    }) else {
        debug_log("[API-LAB] failed to connect window\n");
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