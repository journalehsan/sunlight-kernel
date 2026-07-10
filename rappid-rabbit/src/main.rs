#![cfg_attr(not(any(test, feature = "dom")), no_std)]
#![no_main]

extern crate alloc;

use alloc::{borrow::Cow, format, string::String, vec::Vec};
use core::alloc::GlobalAlloc;

use rappid_rabbit::{
    body_is_probably_text, build_get_request, format_url, looks_like_html, normalize_url_input,
    scan_html_resources,
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
use sunlight_ui::widgets::{Button, ButtonState, Label, Panel, TextInput, TextView};
use sunlight_ui::{request_close, App, Event, Point, Rect, Window, WindowConfig, WindowDecoration};

static F_UI: VecFont = VecFont(FontRole::UiRegular);
static F_SMALL: VecFont = VecFont(FontRole::UiSmall);
static F_MONO: VecFont = VecFont(FontRole::MonoRegular);

const WIN_W: u32 = 1080;
const WIN_H: u32 = 720;
const PAD: i32 = 12;
const TOP_BAR_H: u32 = 42;
const CONSOLE_H: u32 = 96;
const METHOD_W: u32 = 56;
const FETCH_W: u32 = 104;
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
    Console,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ConsoleSeverity {
    Quiet,
    Warn,
    Error,
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
    console_scroll: usize,
    source: SourceContent,
    inspector_text: String,
    console_text: String,
    console_severity: ConsoleSeverity,
}

impl RabbitApp {
    fn new() -> Self {
        let mut url_input = TextInput::new(Rect::default())
            .with_font(&F_UI)
            .with_placeholder("Enter URL (http:// or https://)");
        url_input.set_text("http://example.com/");
        Self {
            url_input,
            status: String::from("Idle"),
            pending_fetch: false,
            focus: FocusPane::Source,
            source_scroll: 0,
            inspector_scroll: 0,
            console_scroll: 0,
            source: SourceContent::Placeholder(
                "Enter a URL, then click Fetch/Open to inspect the response body.",
            ),
            inspector_text: String::from(
                "Method: GET\nURL: \nFinal URL: \nStatus: \nContent-Type: \nBody Size: \nDuration: \nHeaders:\n",
            ),
            console_text: String::new(),
            console_severity: ConsoleSeverity::Quiet,
        }
    }

    fn queue_fetch(&mut self) {
        self.pending_fetch = true;
        self.status = String::from("Fetching...");
        self.console_text.clear();
        self.console_severity = ConsoleSeverity::Quiet;
    }

    fn perform_pending_fetch(&mut self) {
        self.pending_fetch = false;
        self.source_scroll = 0;
        self.inspector_scroll = 0;
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

        let request = build_get_request(&parsed);
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

        self.source = self.build_source_content(content_type, result.body);
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

        // Golden Fish DOM tree (only when dom feature enabled and response looks like HTML)
        #[cfg(feature = "dom")]
        {
            let source_text = self.source.as_str();
            if looks_like_html(Some(content_type), source_text) {
                match parse_html(source_text) {
                    Ok(doc) => {
                        let tree = doc.debug_tree();
                        self.inspector_text
                            .push_str("\n--- DOM Tree (golden-fish) ---\n");
                        self.inspector_text.push_str(&tree);
                    }
                    Err(e) => {
                        self.inspector_text
                            .push_str("\n--- DOM Tree (golden-fish) ---\nParse error: ");
                        self.inspector_text.push_str(&alloc::format!("{e}"));
                        self.inspector_text.push('\n');
                    }
                }
            }
        }
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

        let source_text = self.source.as_str();
        if !source_text.is_empty() && looks_like_html(Some(content_type), source_text) {
            let resources = scan_html_resources(source_text);
            text.push_str("Resources:\n");
            if resources.is_empty() {
                text.push_str("  (none found)\n");
            } else {
                for resource in resources {
                    text.push_str("  [");
                    text.push_str(resource.kind);
                    text.push_str("] ");
                    text.push_str(&resource.url);
                    text.push('\n');
                }
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
            "Method: GET\nURL: \nFinal URL: \nStatus: failed\nContent-Type: \nBody Size: 0 bytes\nDuration: n/a\nHeaders:\n",
        );
        self.console_text = format!("{err}");
        self.console_severity = match err {
            FetchError::InvalidUrl(_) | FetchError::HttpError { .. } => ConsoleSeverity::Warn,
            _ => ConsoleSeverity::Error,
        };
    }

    fn set_error(&mut self, message: String) {
        self.status = String::from("Invalid URL");
        self.source = SourceContent::Message(String::from("Fix the URL and fetch again."));
        self.inspector_text = String::from(
            "Method: GET\nURL: \nFinal URL: \nStatus: not sent\nContent-Type: \nBody Size: 0 bytes\nDuration: n/a\nHeaders:\n",
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

    fn fetch_rect(&self) -> Rect {
        let top = self.top_bar_rect();
        Rect::new(
            top.right() - STATUS_W as i32 - FETCH_W as i32 - 12,
            top.y + 7,
            FETCH_W,
            28,
        )
    }

    fn status_rect(&self) -> Rect {
        let top = self.top_bar_rect();
        Rect::new(top.right() - STATUS_W as i32 - 8, top.y + 6, STATUS_W, 30)
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

    fn console_panel_rect(&self) -> Rect {
        Rect::new(
            PAD,
            WIN_H as i32 - PAD - CONSOLE_H as i32,
            WIN_W.saturating_sub((PAD * 2) as u32),
            CONSOLE_H,
        )
    }

    fn main_area_rect(&self) -> Rect {
        let top = self.top_bar_rect();
        let console = self.console_panel_rect();
        Rect::new(
            PAD,
            top.bottom() + PAD,
            WIN_W.saturating_sub((PAD * 2) as u32),
            (console.y - top.bottom() - PAD * 2).max(120) as u32,
        )
    }

    fn source_panel_rect(&self) -> Rect {
        let main = self.main_area_rect();
        let source_w = ((main.w as i32 * 62) / 100).max(320) as u32;
        Rect::new(main.x, main.y, source_w, main.h)
    }

    fn inspector_panel_rect(&self) -> Rect {
        let main = self.main_area_rect();
        let source = self.source_panel_rect();
        Rect::new(
            source.right() + GUTTER,
            main.y,
            (main.right() - source.right() - GUTTER).max(240) as u32,
            main.h,
        )
    }

    fn source_view(&self) -> TextView<'_> {
        let content = Panel::with_title(self.source_panel_rect(), "Source")
            .content_rect()
            .inset(8);
        TextView::new(content, self.source.as_str())
            .with_scroll_offset(self.source_scroll)
            .with_focus(self.focus == FocusPane::Source)
            .with_font(&F_MONO)
    }

    fn inspector_view(&self) -> TextView<'_> {
        let content = Panel::with_title(self.inspector_panel_rect(), "Inspector")
            .content_rect()
            .inset(8);
        TextView::new(content, self.inspector_text.as_str())
            .with_scroll_offset(self.inspector_scroll)
            .with_focus(self.focus == FocusPane::Inspector)
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
        self.source_scroll = self.source_scroll.min(self.source_view().max_scroll());
        self.inspector_scroll = self
            .inspector_scroll
            .min(self.inspector_view().max_scroll());
        self.console_scroll = self.console_scroll.min(self.console_view().max_scroll());
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

impl App for RabbitApp {
    fn view(&mut self, canvas: &mut sunlight_ui::Canvas, theme: &sunlight_ui::Theme) {
        canvas.fill_rect(Rect::new(0, 0, WIN_W, WIN_H), theme.bg);

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

        Label::new(self.status_rect(), self.status.as_str())
            .with_font(&F_SMALL)
            .draw(canvas, theme);

        let source_panel = Panel::with_title(self.source_panel_rect(), "Source");
        source_panel.draw(canvas, theme);
        self.source_view().draw(canvas, theme);

        let inspector_panel = Panel::with_title(self.inspector_panel_rect(), "Inspector");
        inspector_panel.draw(canvas, theme);
        self.inspector_view().draw(canvas, theme);

        let console_panel = Panel::with_title(self.console_panel_rect(), "Console");
        console_panel.draw(canvas, theme);
        self.console_view().draw(canvas, theme);
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
            Event::Click { x, y } => {
                let point = Point::new(x, y);
                let fetch_button = Button::new(self.fetch_rect(), "Fetch/Open");
                if fetch_button.hit_test(x, y) {
                    self.queue_fetch();
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
                if self.console_panel_rect().contains(point) {
                    self.focus = FocusPane::Console;
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
