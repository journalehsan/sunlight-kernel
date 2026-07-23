#![no_std]
#![no_main]

extern crate alloc;

mod clipboard;
mod request_timing;
mod text_area;

use alloc::{format, string::String, vec, vec::Vec};
use core::alloc::GlobalAlloc;

use clipboard::set_clipboard_text;
use request_timing::RequestTimer;
use sun_font::{FontRole, VecFont};
use sunlight_api_lab::{
    build_request, describe_fetch_error, parse_response, BasicAuthInput, BodyFormat, HttpMethod,
    KeyValueEntry, NoticeSeverity, ParsedResponseDisplay, RequestBuildError, RequestBuildInput,
};
use sunlight_fetch::backend::perform_request;
use sunlight_ipc::{
    debug_log,
    launch_trace::{self, LaunchSource, LaunchTrace},
    process_yield, ProcessExit,
};
use sunlight_ui::widgets::{Button, ButtonState, Label, Panel, TabBar, TextInput, TextView};
use sunlight_ui::{
    request_close, App, Canvas, Event, Point, Rect, Theme, Window, WindowConfig, WindowDecoration,
};
use text_area::TextArea;

static F_UI: VecFont = VecFont(FontRole::UiRegular);
static F_SMALL: VecFont = VecFont(FontRole::UiSmall);
static F_MONO: VecFont = VecFont(FontRole::MonoRegular);

const WIN_W: u32 = 1140;
const WIN_H: u32 = 760;
const PAD: i32 = 12;
const TOP_BAR_H: u32 = 42;
const CONSOLE_H: u32 = 88;
const REQUEST_H: u32 = 272;
const SIDEBAR_W: u32 = 156;
const SEND_W: u32 = 88;
const STATUS_W: u32 = 228;
const METHOD_BAR_W: u32 = 360;
const ACTION_W: u32 = 92;
const ACTION_H: u32 = 28;
const GUTTER: i32 = 10;
const ROW_H: u32 = 28;
const ROW_GAP: i32 = 6;
const URL_INPUT_CAP: usize = 768;
const ROW_KEY_CAP: usize = 96;
const ROW_VALUE_CAP: usize = 256;
const BODY_INPUT_CAP: usize = 16 * 1024;
const AUTH_INPUT_CAP: usize = 128;

const KEY_Q: u8 = 0x10;
const KEY_UP: u8 = 0x48;
const KEY_DOWN: u8 = 0x50;
const KEY_HOME: u8 = 0x47;
const KEY_END: u8 = 0x4F;
const KEY_PGUP: u8 = 0x49;
const KEY_PGDN: u8 = 0x51;

const REQUEST_TABS: [&str; 4] = ["Params", "Headers", "Body", "Auth"];
const METHOD_TABS: [&str; 7] = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];
const BODY_FORMAT_TABS: [&str; 4] = ["Raw", "JSON", "XML", "Form"];

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

impl From<NoticeSeverity> for ConsoleSeverity {
    fn from(value: NoticeSeverity) -> Self {
        match value {
            NoticeSeverity::Quiet => Self::Quiet,
            NoticeSeverity::Warn => Self::Warn,
            NoticeSeverity::Error => Self::Error,
        }
    }
}

struct EditableRow {
    enabled: bool,
    key_input: TextInput<'static, ROW_KEY_CAP>,
    value_input: TextInput<'static, ROW_VALUE_CAP>,
}

impl EditableRow {
    fn new(key_placeholder: &'static str, value_placeholder: &'static str) -> Self {
        Self {
            enabled: true,
            key_input: TextInput::new(Rect::default())
                .with_font(&F_MONO)
                .with_placeholder(key_placeholder),
            value_input: TextInput::new(Rect::default())
                .with_font(&F_MONO)
                .with_placeholder(value_placeholder),
        }
    }

    fn to_entry(&self) -> KeyValueEntry {
        KeyValueEntry {
            enabled: self.enabled,
            key: String::from(self.key_input.value()),
            value: String::from(self.value_input.value()),
        }
    }
}

struct ApiLabApp {
    url_input: TextInput<'static, URL_INPUT_CAP>,
    params: Vec<EditableRow>,
    headers: Vec<EditableRow>,
    body_input: TextArea<'static, BODY_INPUT_CAP>,
    auth_username: TextInput<'static, AUTH_INPUT_CAP>,
    auth_password: TextInput<'static, AUTH_INPUT_CAP>,
    method: HttpMethod,
    body_format: BodyFormat,
    request_tab: usize,
    status: String,
    pending_send: bool,
    focus: FocusPane,
    response_scroll: usize,
    details_scroll: usize,
    console_scroll: usize,
    response_body_text: String,
    response_headers_text: String,
    response_copy_text: String,
    details_text: String,
    console_text: String,
    console_severity: ConsoleSeverity,
}

impl ApiLabApp {
    fn new() -> Self {
        let mut url_input = TextInput::new(Rect::default())
            .with_font(&F_UI)
            .with_placeholder("Enter URL (http:// or https://)");
        url_input.set_text("http://example.com/");

        let body_input = TextArea::new(Rect::default())
            .with_font(&F_MONO)
            .with_placeholder("Request body");

        let auth_username = TextInput::new(Rect::default())
            .with_font(&F_MONO)
            .with_placeholder("Username");
        let auth_password = TextInput::new(Rect::default())
            .with_font(&F_MONO)
            .with_placeholder("Password");

        Self {
            url_input,
            params: vec![Self::blank_param_row()],
            headers: vec![Self::blank_header_row()],
            body_input,
            auth_username,
            auth_password,
            method: HttpMethod::Get,
            body_format: BodyFormat::RawText,
            request_tab: 0,
            status: String::from("Ready"),
            pending_send: false,
            focus: FocusPane::ResponseBody,
            response_scroll: 0,
            details_scroll: 0,
            console_scroll: 0,
            response_body_text: String::from("Send a request to inspect the response body here."),
            response_headers_text: String::from("(none)"),
            response_copy_text: String::new(),
            details_text: Self::placeholder_details(),
            console_text: String::new(),
            console_severity: ConsoleSeverity::Quiet,
        }
    }

    fn blank_param_row() -> EditableRow {
        EditableRow::new("key", "value")
    }

    fn blank_header_row() -> EditableRow {
        EditableRow::new("Header-Name", "Header value")
    }

    fn placeholder_details() -> String {
        String::from(
            "Status Code: \n\
Status Text: \n\
Final URL: \n\
Duration: \n\
Body Size: \n\
Content Type: \n\n\
Response Headers:\n(none)",
        )
    }

    fn clear_request(&mut self) {
        self.url_input.set_text("");
        self.method = HttpMethod::Get;
        self.body_format = BodyFormat::RawText;
        self.request_tab = 0;
        self.params.clear();
        self.params.push(Self::blank_param_row());
        self.headers.clear();
        self.headers.push(Self::blank_header_row());
        self.body_input.set_text("");
        self.auth_username.set_text("");
        self.auth_password.set_text("");
        self.pending_send = false;
        self.clear_response_state();
        self.set_console("Request cleared.", ConsoleSeverity::Quiet);
    }

    fn clear_response(&mut self) {
        self.clear_response_state();
        self.set_console("Response cleared.", ConsoleSeverity::Quiet);
    }

    fn clear_response_state(&mut self) {
        self.response_body_text.clear();
        self.response_headers_text = String::from("(none)");
        self.response_copy_text.clear();
        self.details_text = Self::placeholder_details();
        self.response_scroll = 0;
        self.details_scroll = 0;
        self.console_scroll = 0;
        self.status = String::from("Ready");
    }

    fn mark_response_pending(&mut self) {
        self.response_body_text.clear();
        self.response_headers_text = String::from("(pending)");
        self.response_copy_text.clear();
        self.details_text = String::from(
            "Status Code: pending\n\
Status Text: waiting for response\n\
Final URL: \n\
Duration: pending\n\
Body Size: pending\n\
Content Type: pending\n\n\
Response Headers:\n(pending)",
        );
        self.response_scroll = 0;
        self.details_scroll = 0;
        self.console_scroll = 0;
    }

    fn queue_send(&mut self) {
        if self.pending_send {
            return;
        }
        self.pending_send = true;
        self.status = String::from("Sending...");
        self.set_console("Request in progress...", ConsoleSeverity::Quiet);
        self.mark_response_pending();
    }

    fn perform_pending_send(&mut self) {
        self.pending_send = false;
        self.response_scroll = 0;
        self.details_scroll = 0;
        self.console_scroll = 0;

        let parameters = self.collect_entries(&self.params);
        let headers = self.collect_entries(&self.headers);
        let input = RequestBuildInput {
            method: self.method,
            url_input: self.url_input.value(),
            parameters: &parameters,
            headers: &headers,
            auth: BasicAuthInput {
                username: self.auth_username.value(),
                password: self.auth_password.value(),
            },
            body_format: self.body_format,
            body_text: self.body_input.value(),
        };

        let built = match build_request(input) {
            Ok(built) => built,
            Err(err) => {
                self.apply_request_build_error(err);
                return;
            }
        };

        self.url_input.set_text(&built.normalized_url);
        let timer = RequestTimer::start_now();
        match perform_request(built.parsed_url, built.request) {
            Ok(result) => self.apply_response(parse_response(
                self.method,
                &built.normalized_url,
                result,
                Some(timer.elapsed_ms()),
            )),
            Err(err) => self.apply_response(describe_fetch_error(&err, Some(timer.elapsed_ms()))),
        }
    }

    fn apply_response(&mut self, parsed: ParsedResponseDisplay) {
        self.status = parsed.status_label;
        self.response_body_text = parsed.body_text;
        self.response_headers_text = parsed.headers_text;
        self.response_copy_text = parsed.copy_response_text;
        self.details_text = parsed.details_text;
        self.console_text = parsed.console_text;
        self.console_severity = parsed.console_severity.into();
        self.clamp_scrolls();
    }

    fn apply_request_build_error(&mut self, err: RequestBuildError) {
        self.status = match err {
            RequestBuildError::InvalidUrl(ref message)
                if message.contains("only http:// and https://") =>
            {
                String::from("Unsupported URL scheme")
            }
            RequestBuildError::InvalidUrl(_) => String::from("Invalid URL"),
            RequestBuildError::DuplicateHeader(_) => String::from("Duplicate header"),
            RequestBuildError::ManagedHeader(_) => String::from("Managed header"),
        };
        self.response_body_text.clear();
        self.response_headers_text = String::from("(none)");
        self.response_copy_text = self.response_body_text.clone();
        self.details_text = Self::placeholder_details();
        self.set_console(&format!("{err}"), ConsoleSeverity::Warn);
    }

    fn collect_entries(&self, rows: &[EditableRow]) -> Vec<KeyValueEntry> {
        rows.iter().map(EditableRow::to_entry).collect()
    }

    fn copy_response_body(&mut self) {
        let payload = self.response_body_text.as_bytes().to_vec();
        self.copy_text(&payload, "Response body copied.")
    }

    fn copy_response_headers(&mut self) {
        let payload = self.response_headers_text.as_bytes().to_vec();
        self.copy_text(&payload, "Response headers copied.")
    }

    fn copy_response(&mut self) {
        let payload = if self.response_copy_text.is_empty() {
            self.response_body_text.as_bytes().to_vec()
        } else {
            self.response_copy_text.as_bytes().to_vec()
        };
        self.copy_text(&payload, "Response copied.")
    }

    fn copy_text(&mut self, payload: &[u8], success_message: &str) {
        match set_clipboard_text(payload) {
            Ok(()) => self.set_console(success_message, ConsoleSeverity::Quiet),
            Err(message) => self.set_console(message, ConsoleSeverity::Warn),
        }
    }

    fn set_console(&mut self, message: &str, severity: ConsoleSeverity) {
        self.console_text = String::from(message);
        self.console_severity = severity;
    }

    fn duplicate_header_message(&self) -> Option<&'static str> {
        for (index, row) in self.headers.iter().enumerate() {
            if !row.enabled {
                continue;
            }
            let key = row.key_input.value().trim();
            if key.is_empty() {
                continue;
            }
            if self.headers.iter().skip(index + 1).any(|other| {
                other.enabled && key.eq_ignore_ascii_case(other.key_input.value().trim())
            }) {
                return Some("Duplicate enabled header names will block Send.");
            }
        }
        None
    }

    fn top_bar_rect(&self) -> Rect {
        Rect::new(PAD, PAD, WIN_W.saturating_sub((PAD * 2) as u32), TOP_BAR_H)
    }

    fn method_bar_rect(&self) -> Rect {
        let top = self.top_bar_rect();
        Rect::new(top.x + 8, top.y + 7, METHOD_BAR_W, 28)
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
        let methods = self.method_bar_rect();
        let send = self.send_rect();
        Rect::new(
            methods.right() + 8,
            methods.y,
            (send.x - methods.right() - 16).max(160) as u32,
            methods.h,
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

    fn request_inner_rect(&self) -> Rect {
        Panel::with_title(self.request_panel_rect(), "Request")
            .content_rect()
            .inset(8)
    }

    fn request_action_rect(&self) -> Rect {
        let inner = self.request_inner_rect();
        Rect::new(inner.x, inner.y, inner.w, ACTION_H)
    }

    fn clear_request_rect(&self) -> Rect {
        let row = self.request_action_rect();
        Rect::new(row.right() - ACTION_W as i32, row.y, ACTION_W, ACTION_H)
    }

    fn request_tab_bar_rect(&self) -> Rect {
        let action = self.request_action_rect();
        Rect::new(action.x, action.bottom() + 8, action.w, 28)
    }

    fn request_tab_content_rect(&self) -> Rect {
        let tabs = self.request_tab_bar_rect();
        let inner = self.request_inner_rect();
        Rect::new(
            inner.x,
            tabs.bottom() + 8,
            inner.w,
            (inner.bottom() - tabs.bottom() - 8).max(60) as u32,
        )
    }

    fn entry_add_rect(&self) -> Rect {
        let content = self.request_tab_content_rect();
        Rect::new(
            content.right() - ACTION_W as i32,
            content.y,
            ACTION_W,
            ACTION_H,
        )
    }

    fn rows_area_rect(&self) -> Rect {
        let content = self.request_tab_content_rect();
        Rect::new(
            content.x,
            content.y + ACTION_H as i32 + 8,
            content.w,
            (content.h as i32 - ACTION_H as i32 - 8).max(40) as u32,
        )
    }

    fn body_format_rect(&self) -> Rect {
        let content = self.request_tab_content_rect();
        Rect::new(content.x, content.y, content.w.min(360), 28)
    }

    fn body_note_rect(&self) -> Rect {
        let format = self.body_format_rect();
        Rect::new(
            format.x,
            format.bottom() + 6,
            self.request_tab_content_rect().w,
            18,
        )
    }

    fn body_editor_rect(&self) -> Rect {
        let note = self.body_note_rect();
        let content = self.request_tab_content_rect();
        Rect::new(
            content.x,
            note.bottom() + 6,
            content.w,
            (content.bottom() - note.bottom() - 6).max(48) as u32,
        )
    }

    fn auth_username_rect(&self) -> Rect {
        let content = self.request_tab_content_rect();
        Rect::new(content.x, content.y + 26, content.w, ROW_H)
    }

    fn auth_password_rect(&self) -> Rect {
        let user = self.auth_username_rect();
        Rect::new(user.x, user.bottom() + 32, user.w, ROW_H)
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

    fn response_actions_rect(&self) -> Rect {
        let area = self.response_area_rect();
        Rect::new(area.x, area.y, area.w, ACTION_H)
    }

    fn clear_response_rect(&self) -> Rect {
        let row = self.response_actions_rect();
        Rect::new(row.right() - ACTION_W as i32, row.y, ACTION_W, ACTION_H)
    }

    fn copy_response_rect(&self) -> Rect {
        let clear = self.clear_response_rect();
        Rect::new(clear.x - ACTION_W as i32 - 8, clear.y, ACTION_W, ACTION_H)
    }

    fn copy_headers_rect(&self) -> Rect {
        let copy = self.copy_response_rect();
        Rect::new(copy.x - ACTION_W as i32 - 8, copy.y, ACTION_W, ACTION_H)
    }

    fn copy_body_rect(&self) -> Rect {
        let copy = self.copy_headers_rect();
        Rect::new(copy.x - ACTION_W as i32 - 8, copy.y, ACTION_W, ACTION_H)
    }

    fn response_panels_rect(&self) -> Rect {
        let area = self.response_area_rect();
        Rect::new(
            area.x,
            area.y + ACTION_H as i32 + 8,
            area.w,
            (area.h as i32 - ACTION_H as i32 - 8).max(80) as u32,
        )
    }

    fn response_body_rect(&self) -> Rect {
        let area = self.response_panels_rect();
        let body_w = ((area.w as i32 * 58) / 100).max(280) as u32;
        Rect::new(area.x, area.y, body_w, area.h)
    }

    fn response_details_rect(&self) -> Rect {
        let area = self.response_panels_rect();
        let body = self.response_body_rect();
        Rect::new(
            body.right() + GUTTER,
            area.y,
            (area.right() - body.right() - GUTTER).max(220) as u32,
            area.h,
        )
    }

    fn response_view(&self) -> TextView<'_> {
        let content = Panel::with_title(self.response_body_rect(), "Response Body")
            .content_rect()
            .inset(8);
        TextView::new(content, self.response_body_text.as_str())
            .with_scroll_offset(self.response_scroll)
            .with_focus(self.focus == FocusPane::ResponseBody)
            .with_font(&F_MONO)
    }

    fn details_view(&self) -> TextView<'_> {
        let content = Panel::with_title(self.response_details_rect(), "Response Info")
            .content_rect()
            .inset(8);
        TextView::new(content, self.details_text.as_str())
            .with_scroll_offset(self.details_scroll)
            .with_focus(self.focus == FocusPane::ResponseDetails)
            .with_font(&F_MONO)
    }

    fn console_view(&self, theme: &Theme) -> TextView<'_> {
        let content = Panel::with_title(self.console_panel_rect(), "Console")
            .content_rect()
            .inset(8);
        let mut view = TextView::new(content, self.console_text.as_str())
            .with_scroll_offset(self.console_scroll)
            .with_focus(self.focus == FocusPane::Console)
            .with_font(&F_MONO);
        view = match self.console_severity {
            ConsoleSeverity::Quiet => view,
            ConsoleSeverity::Warn => view.with_text_color(theme.warn),
            ConsoleSeverity::Error => view.with_text_color(theme.danger),
        };
        view
    }

    fn clamp_scrolls(&mut self) {
        self.response_scroll = self.response_scroll.min(self.response_view().max_scroll());
        self.details_scroll = self.details_scroll.min(self.details_view().max_scroll());
        let theme = Theme::sunlight_dark();
        self.console_scroll = self
            .console_scroll
            .min(self.console_view(&theme).max_scroll());
    }

    fn scroll_focused(&mut self, delta: i32, page: bool, home: bool, end: bool) {
        match self.focus {
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
                let theme = Theme::sunlight_dark();
                let (visible, max_scroll) = {
                    let view = self.console_view(&theme);
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

    fn update_active_request_inputs(&mut self, event: Event) -> bool {
        let mut changed = self.url_input.update(event);

        changed |= match self.request_tab {
            0 => self.update_rows(event, false),
            1 => self.update_rows(event, true),
            2 => {
                self.body_input.rect = self.body_editor_rect();
                self.body_input.update(event)
            }
            _ => {
                self.auth_username.rect = self.auth_username_rect();
                let username_changed = self.auth_username.update(event);
                self.auth_password.rect = self.auth_password_rect();
                username_changed | self.auth_password.update(event)
            }
        };
        changed
    }

    fn update_rows(&mut self, event: Event, headers: bool) -> bool {
        let area = self.rows_area_rect();
        let rows = if headers {
            &mut self.headers
        } else {
            &mut self.params
        };

        let mut changed = false;
        for (index, row) in rows.iter_mut().enumerate() {
            let row_rect = entry_row_rect(area, index);
            row.key_input.rect = entry_key_rect(row_rect);
            row.value_input.rect = entry_value_rect(row_rect);
            changed |= row.key_input.update(event);
            changed |= row.value_input.update(event);
        }
        changed
    }

    fn handle_request_click(&mut self, x: i32, y: i32) -> bool {
        let point = Point::new(x, y);

        let mut send = Button::new(self.send_rect(), "Send");
        if self.pending_send {
            send.state = ButtonState::Disabled;
        }
        if send.hit_test(x, y) {
            self.queue_send();
            return true;
        }
        if let Some(index) =
            TabBar::new(self.method_bar_rect(), &METHOD_TABS, self.method.index()).hit_test(x, y)
        {
            self.method = HttpMethod::ALL[index];
            return true;
        }
        if Button::secondary(self.clear_request_rect(), "Clear Req").hit_test(x, y) {
            self.clear_request();
            return true;
        }
        if let Some(tab) =
            TabBar::new(self.request_tab_bar_rect(), &REQUEST_TABS, self.request_tab).hit_test(x, y)
        {
            self.request_tab = tab;
            return true;
        }

        match self.request_tab {
            0 => {
                if self.handle_row_click(x, y, false) {
                    return true;
                }
            }
            1 => {
                if self.handle_row_click(x, y, true) {
                    return true;
                }
            }
            2 => {
                if let Some(index) = TabBar::new(
                    self.body_format_rect(),
                    &BODY_FORMAT_TABS,
                    self.body_format.index(),
                )
                .hit_test(x, y)
                {
                    self.body_format = BodyFormat::ALL[index];
                    return true;
                }
            }
            _ => {}
        }

        if Button::secondary(self.copy_body_rect(), "Copy Body").hit_test(x, y) {
            self.copy_response_body();
            return true;
        }
        if Button::secondary(self.copy_headers_rect(), "Copy Headers").hit_test(x, y) {
            self.copy_response_headers();
            return true;
        }
        if Button::secondary(self.copy_response_rect(), "Copy All").hit_test(x, y) {
            self.copy_response();
            return true;
        }
        if Button::secondary(self.clear_response_rect(), "Clear Resp").hit_test(x, y) {
            self.clear_response();
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

    fn handle_row_click(&mut self, x: i32, y: i32, headers: bool) -> bool {
        if Button::secondary(self.entry_add_rect(), "Add Row").hit_test(x, y) {
            if headers {
                self.headers.push(Self::blank_header_row());
            } else {
                self.params.push(Self::blank_param_row());
            }
            return true;
        }

        let area = self.rows_area_rect();
        let rows = if headers {
            &mut self.headers
        } else {
            &mut self.params
        };

        let mut remove_index = None;
        for (index, row) in rows.iter_mut().enumerate() {
            let row_rect = entry_row_rect(area, index);
            if entry_checkbox_rect(row_rect).contains(Point::new(x, y)) {
                row.enabled = !row.enabled;
                return true;
            }
            if Button::secondary(entry_remove_rect(row_rect), "X").hit_test(x, y) {
                remove_index = Some(index);
                break;
            }
        }

        if let Some(index) = remove_index {
            rows.remove(index);
            if rows.is_empty() {
                rows.push(if headers {
                    Self::blank_header_row()
                } else {
                    Self::blank_param_row()
                });
            }
            return true;
        }

        false
    }

    fn draw_sidebar(&self, canvas: &mut Canvas, theme: &Theme) {
        let sidebar = Panel::with_title(self.sidebar_rect(), "Library");
        sidebar.draw(canvas, theme);
        let content = sidebar.content_rect().inset(8);
        Label::new(
            Rect::new(content.x, content.y, content.w, 20),
            "Collections are out of scope",
        )
        .with_font(&F_SMALL)
        .draw(canvas, theme);
        Label::new(
            Rect::new(content.x, content.y + 28, content.w, 20),
            "Request history arrives later",
        )
        .with_font(&F_SMALL)
        .draw(canvas, theme);
    }

    fn draw_request_panel(&mut self, canvas: &mut Canvas, theme: &Theme) {
        let request_panel = Panel::with_title(self.request_panel_rect(), "Request");
        request_panel.draw(canvas, theme);

        let action = self.request_action_rect();
        let add_note_rect = Rect::new(
            action.x,
            action.y,
            action.w.saturating_sub(ACTION_W + 12),
            ACTION_H,
        );
        Label::new(
            add_note_rect,
            "Stable REST workflow powered by sunlight-fetch",
        )
        .with_font(&F_SMALL)
        .draw(canvas, theme);
        Button::secondary(self.clear_request_rect(), "Clear Req")
            .with_font(&F_SMALL)
            .draw(canvas, theme);

        TabBar::new(self.request_tab_bar_rect(), &REQUEST_TABS, self.request_tab)
            .draw(canvas, theme);

        match self.request_tab {
            0 => self.draw_rows_tab(canvas, theme, false),
            1 => self.draw_rows_tab(canvas, theme, true),
            2 => self.draw_body_tab(canvas, theme),
            _ => self.draw_auth_tab(canvas, theme),
        }
    }

    fn draw_rows_tab(&mut self, canvas: &mut Canvas, theme: &Theme, headers: bool) {
        let content = self.request_tab_content_rect();
        let rows_area = self.rows_area_rect();
        let content_bottom = self.request_tab_content_rect().bottom();
        let note = if headers {
            self.duplicate_header_message()
                .unwrap_or("Host, User-Agent, Connection, and Content-Length are automatic.")
        } else {
            "Enabled parameters are appended to the query string before Send."
        };

        let note_color = if headers && self.duplicate_header_message().is_some() {
            theme.warn
        } else {
            theme.text_dim
        };
        canvas.draw_text(content.x, content.y + 8, note, note_color);
        Button::secondary(self.entry_add_rect(), "Add Row")
            .with_font(&F_SMALL)
            .draw(canvas, theme);

        let rows = if headers {
            &mut self.headers
        } else {
            &mut self.params
        };
        for (index, row) in rows.iter_mut().enumerate() {
            let row_rect = entry_row_rect(rows_area, index);
            if row_rect.bottom() > content_bottom {
                break;
            }

            let checkbox = entry_checkbox_rect(row_rect);
            draw_checkbox(canvas, theme, checkbox, row.enabled);

            row.key_input.rect = entry_key_rect(row_rect);
            row.value_input.rect = entry_value_rect(row_rect);
            row.key_input.draw(canvas, theme);
            row.value_input.draw(canvas, theme);

            Button::secondary(entry_remove_rect(row_rect), "X")
                .with_font(&F_SMALL)
                .draw(canvas, theme);
        }
    }

    fn draw_body_tab(&mut self, canvas: &mut Canvas, theme: &Theme) {
        TabBar::new(
            self.body_format_rect(),
            &BODY_FORMAT_TABS,
            self.body_format.index(),
        )
        .draw(canvas, theme);
        let message = if self.method.allows_body() {
            format!(
                "Content-Type defaults to {} unless you override it in Headers.",
                self.body_format.default_content_type()
            )
        } else {
            String::from("This method sends no request body in API Lab.")
        };
        canvas.draw_text(
            self.body_note_rect().x,
            self.body_note_rect().y + 4,
            message.as_str(),
            if self.method.allows_body() {
                theme.text_dim
            } else {
                theme.warn
            },
        );
        self.body_input.rect = self.body_editor_rect();
        self.body_input.draw(canvas, theme);
    }

    fn draw_auth_tab(&mut self, canvas: &mut Canvas, theme: &Theme) {
        let content = self.request_tab_content_rect();
        canvas.draw_text(
            content.x,
            content.y + 6,
            "Basic auth generates Authorization automatically.",
            theme.text_dim,
        );
        canvas.draw_text(content.x, content.y + 28, "Username", theme.text_dim);
        self.auth_username.rect = self.auth_username_rect();
        self.auth_username.draw(canvas, theme);
        canvas.draw_text(
            content.x,
            self.auth_password_rect().y - 20,
            "Password",
            theme.text_dim,
        );
        self.auth_password.rect = self.auth_password_rect();
        self.auth_password.draw(canvas, theme);
    }

    fn draw_response_area(&self, canvas: &mut Canvas, theme: &Theme) {
        Button::secondary(self.copy_body_rect(), "Copy Body")
            .with_font(&F_SMALL)
            .draw(canvas, theme);
        Button::secondary(self.copy_headers_rect(), "Copy Headers")
            .with_font(&F_SMALL)
            .draw(canvas, theme);
        Button::secondary(self.copy_response_rect(), "Copy All")
            .with_font(&F_SMALL)
            .draw(canvas, theme);
        Button::secondary(self.clear_response_rect(), "Clear Resp")
            .with_font(&F_SMALL)
            .draw(canvas, theme);

        Panel::with_title(self.response_body_rect(), "Response Body").draw(canvas, theme);
        self.response_view().draw(canvas, theme);

        Panel::with_title(self.response_details_rect(), "Response Info").draw(canvas, theme);
        self.details_view().draw(canvas, theme);
    }
}

impl App for ApiLabApp {
    fn view(&mut self, canvas: &mut Canvas, theme: &Theme) {
        canvas.fill_rect(Rect::new(0, 0, WIN_W, WIN_H), theme.bg);

        let top = self.top_bar_rect();
        Panel::new(top).draw(canvas, theme);

        TabBar::new(self.method_bar_rect(), &METHOD_TABS, self.method.index()).draw(canvas, theme);

        self.url_input.rect = self.url_rect();
        self.url_input.draw(canvas, theme);

        let mut send = Button::new(self.send_rect(), "Send").with_font(&F_UI);
        send.state = if self.pending_send {
            ButtonState::Disabled
        } else {
            ButtonState::Normal
        };
        send.draw(canvas, theme);

        Label::new(self.status_rect(), self.status.as_str())
            .with_font(&F_SMALL)
            .draw(canvas, theme);

        self.draw_sidebar(canvas, theme);
        self.draw_request_panel(canvas, theme);
        self.draw_response_area(canvas, theme);

        let console_panel = Panel::with_title(self.console_panel_rect(), "Console");
        console_panel.draw(canvas, theme);
        self.console_view(theme).draw(canvas, theme);
    }

    fn update(&mut self, event: Event) -> bool {
        if self.update_active_request_inputs(event) {
            return true;
        }

        match event {
            Event::Tick => {
                if self.pending_send {
                    self.perform_pending_send();
                    return true;
                }
                false
            }
            Event::Click { x, y } => self.handle_request_click(x, y),
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

fn draw_checkbox(canvas: &mut Canvas, theme: &Theme, rect: Rect, checked: bool) {
    canvas.fill_rect(rect, theme.panel);
    canvas.draw_rect(rect, theme.border);
    if checked {
        canvas.hbar(rect.x + 3, rect.y + 8, 10, 2, theme.accent);
        canvas.vline(rect.x + 8, rect.y + 3, 10, theme.accent);
    }
}

fn entry_row_rect(area: Rect, index: usize) -> Rect {
    Rect::new(
        area.x,
        area.y + index as i32 * (ROW_H as i32 + ROW_GAP),
        area.w,
        ROW_H,
    )
}

fn entry_checkbox_rect(row_rect: Rect) -> Rect {
    Rect::new(row_rect.x, row_rect.y + 5, 18, 18)
}

fn entry_remove_rect(row_rect: Rect) -> Rect {
    Rect::new(row_rect.right() - 28, row_rect.y, 28, ROW_H)
}

fn entry_key_rect(row_rect: Rect) -> Rect {
    let start_x = row_rect.x + 24;
    let remove = entry_remove_rect(row_rect);
    let available = (remove.x - start_x - 8).max(80) as u32;
    let key_w = available.min(((available as i32 * 35) / 100).max(140) as u32);
    Rect::new(start_x, row_rect.y, key_w, ROW_H)
}

fn entry_value_rect(row_rect: Rect) -> Rect {
    let key = entry_key_rect(row_rect);
    let remove = entry_remove_rect(row_rect);
    Rect::new(
        key.right() + 8,
        row_rect.y,
        (remove.x - key.right() - 16).max(60) as u32,
        ROW_H,
    )
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
