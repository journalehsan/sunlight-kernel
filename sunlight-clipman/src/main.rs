#![no_std]
#![no_main]
#![allow(static_mut_refs)]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

use sunlight_clipd::{decode_summary_list, ClipError, ClipboardKind, ClipboardSummary};
use sunlight_ipc::{
    debug_log, ipc_call, nameserver_lookup, process_yield, shm_free, shm_map, CapabilityToken,
    ClipMsg, IpcMsg, ProcessExit,
};
use sunlight_ui::{
    request_close, App, Canvas, Color, Event, Point, Rect, Theme, Window, WindowConfig,
    WindowDecoration,
};

const WIN_W: u32 = 360;
const WIN_H: u32 = 420;
const HEADER_H: i32 = 40;
const FOOTER_H: i32 = 24;
const PAD: i32 = 12;
const ROW_H: i32 = 58;
const MAX_ROWS: usize = 32;

const KEY_ESC: u8 = 0x01;
const KEY_ENTER: u8 = 0x1C;
const KEY_UP: u8 = 0x48;
const KEY_DOWN: u8 = 0x50;

struct BumpAllocator;

unsafe impl core::alloc::GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        static mut HEAP: [u8; 192 * 1024] = [0; 192 * 1024];
        static mut NEXT: usize = 0;
        let start = NEXT;
        let align = layout.align();
        let aligned = (start + align - 1) & !(align - 1);
        let end = aligned + layout.size();
        if end > HEAP.len() {
            return core::ptr::null_mut();
        }
        NEXT = end;
        HEAP.as_mut_ptr().add(aligned)
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: core::alloc::Layout) {}
}

#[global_allocator]
static BUMP: BumpAllocator = BumpAllocator;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    debug_log("[CLIPMAN] panic\n");
    loop {
        process_yield();
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LoadState {
    Loading,
    Ready,
    Empty,
    ServiceUnavailable,
    Error,
}

struct ClipmanApp {
    clipd: Option<CapabilityToken>,
    items: Vec<ClipboardSummary>,
    load_state: LoadState,
    selected: usize,
    hover: Option<usize>,
    status: String,
}

impl ClipmanApp {
    fn new() -> Self {
        let clipd = nameserver_lookup("clipd");
        let mut app = Self {
            clipd,
            items: Vec::new(),
            load_state: LoadState::Loading,
            selected: 0,
            hover: None,
            status: String::from("Loading clipboard history"),
        };
        app.reload();
        app
    }

    fn reload(&mut self) {
        self.items.clear();
        self.hover = None;
        let Some(cap) = self.clipd else {
            self.load_state = LoadState::ServiceUnavailable;
            self.status = String::from("Clipboard service unavailable");
            return;
        };
        let reply = ipc_call(cap, IpcMsg::with_label(ClipMsg::LIST_CLIPBOARD_HISTORY));
        if reply.label == ClipMsg::ERROR {
            self.load_state = LoadState::Error;
            self.status = String::from(error_label(reply.words[0]));
            return;
        }
        if reply.words[1] == 0 || reply.caps[0] == CapabilityToken::INVALID {
            self.load_state = LoadState::Empty;
            self.status = String::from("Clipboard history is empty");
            return;
        }
        match take_reply_bytes(&reply).and_then(|bytes| decode_summary_list(&bytes)) {
            Ok(list) if list.is_empty() => {
                self.load_state = LoadState::Empty;
                self.status = String::from("Clipboard history is empty");
            }
            Ok(list) => {
                self.items = list.into_iter().take(MAX_ROWS).collect();
                self.selected = self
                    .items
                    .iter()
                    .position(|item| item.is_current)
                    .unwrap_or(0)
                    .min(self.items.len().saturating_sub(1));
                self.load_state = LoadState::Ready;
                self.status = String::from("Enter or click to select");
            }
            Err(err) => {
                self.load_state = LoadState::Error;
                self.status = String::from(error_label(err.code()));
            }
        }
    }

    fn row_rect(index: usize) -> Rect {
        Rect::new(
            PAD,
            HEADER_H + 8 + index as i32 * ROW_H,
            WIN_W.saturating_sub((PAD as u32) * 2),
            (ROW_H - 6) as u32,
        )
    }

    fn item_at(&self, x: i32, y: i32) -> Option<usize> {
        self.items.iter().enumerate().find_map(|(index, _)| {
            Self::row_rect(index)
                .contains(Point::new(x, y))
                .then_some(index)
        })
    }

    fn select_current(&mut self) -> bool {
        let Some(cap) = self.clipd else {
            self.status = String::from("Clipboard service unavailable");
            self.load_state = LoadState::ServiceUnavailable;
            return true;
        };
        let Some(item) = self.items.get(self.selected) else {
            return false;
        };
        let reply = ipc_call(
            cap,
            IpcMsg::with_label(ClipMsg::SELECT_CLIPBOARD_HISTORY_ITEM)
                .word(0, ClipMsg::SELECT_BY_ID)
                .word(1, item.id as u64),
        );
        if reply.label == ClipMsg::ERROR {
            self.status = String::from(error_label(reply.words[0]));
            self.load_state = LoadState::Error;
            return true;
        }
        request_close();
        true
    }

    fn move_selection(&mut self, delta: i32) -> bool {
        if self.items.is_empty() {
            return false;
        }
        let len = self.items.len() as i32;
        let next = (self.selected as i32 + delta).clamp(0, len - 1) as usize;
        if next != self.selected {
            self.selected = next;
            return true;
        }
        false
    }

    fn kind_marker(kind: ClipboardKind) -> &'static str {
        match kind {
            ClipboardKind::Text => "TXT",
            ClipboardKind::FileList => "FIL",
            ClipboardKind::Binary => "BIN",
        }
    }

    fn age_text(created_at_ms: u64) -> String {
        let now = sunlight_ipc::monotonic_millis();
        let delta = now.saturating_sub(created_at_ms);
        if delta < 1_000 {
            String::from("now")
        } else if delta < 60_000 {
            let mut out = String::new();
            append_u64(&mut out, delta / 1_000);
            out.push('s');
            out
        } else if delta < 3_600_000 {
            let mut out = String::new();
            append_u64(&mut out, delta / 60_000);
            out.push('m');
            out
        } else {
            let mut out = String::new();
            append_u64(&mut out, delta / 3_600_000);
            out.push('h');
            out
        }
    }
}

impl App for ClipmanApp {
    fn view(&mut self, canvas: &mut Canvas, theme: &Theme) {
        canvas.fill_rect(Rect::new(0, 0, WIN_W, WIN_H), Color::rgba(0x12, 0x12, 0x14, 0xE8));
        let panel = Rect::new(0, 0, WIN_W, WIN_H);
        canvas.fill_rect(panel, theme.panel);
        canvas.draw_rect(panel, theme.border);
        canvas.fill_rect(Rect::new(0, 0, WIN_W, HEADER_H as u32), theme.panel_alt);
        canvas.hbar(0, HEADER_H - 1, WIN_W, 1, theme.border);
        canvas.draw_text(12, 14, "Sunlight Clipman", theme.text);
        canvas.draw_text((WIN_W as i32) - 84, 14, "Win+V", theme.accent);

        match self.load_state {
            LoadState::Ready => {
                for (index, item) in self.items.iter().enumerate() {
                    let rect = Self::row_rect(index);
                    let hovered = self.hover == Some(index);
                    let selected = self.selected == index;
                    let bg = if selected {
                        theme.accent.darken(170)
                    } else if hovered {
                        theme.panel_alt.lighten(12)
                    } else if index % 2 == 0 {
                        theme.panel
                    } else {
                        theme.panel_alt
                    };
                    canvas.fill_rect(rect, bg);
                    canvas.draw_rect(rect, if selected { theme.accent } else { theme.border });

                    let marker_rect = Rect::new(rect.x + 8, rect.y + 8, 38, 22);
                    canvas.fill_rect(
                        marker_rect,
                        if selected { theme.accent } else { theme.border },
                    );
                    canvas.draw_text(
                        marker_rect.x + 7,
                        marker_rect.y + 7,
                        Self::kind_marker(item.kind),
                        if selected { theme.bg } else { theme.text_dim },
                    );

                    let summary = fit_text(&item.summary, 34);
                    let mime = fit_text(&item.mime, 18);
                    let meta = {
                        let mut out = String::new();
                        out.push_str(&mime);
                        out.push(' ');
                        out.push('|');
                        out.push(' ');
                        append_u64(&mut out, item.size as u64);
                        out.push('b');
                        out.push(' ');
                        out.push('|');
                        out.push(' ');
                        out.push_str(&Self::age_text(item.created_at_ms));
                        out
                    };
                    canvas.draw_text(rect.x + 56, rect.y + 10, &summary, theme.text);
                    canvas.draw_text(
                        rect.x + 56,
                        rect.y + 30,
                        &meta,
                        if selected {
                            theme.accent_hover
                        } else {
                            theme.text_dim
                        },
                    );
                    if item.is_current {
                        canvas.draw_text(rect.right() - 50, rect.y + 10, "Now", theme.accent_hover);
                    }
                }
            }
            LoadState::Empty => {
                canvas.draw_text(74, 172, "Clipboard history is empty", theme.text);
                canvas.draw_text(88, 194, "Copy something to see it here", theme.text_dim);
            }
            LoadState::ServiceUnavailable => {
                canvas.draw_text(72, 172, "Clipboard service unavailable", theme.danger);
                canvas.draw_text(90, 194, "Start sunlight-clipd and try again", theme.text_dim);
            }
            LoadState::Error => {
                canvas.draw_text(100, 172, "Clipboard error", theme.danger);
                let line = fit_text(&self.status, 46);
                canvas.draw_text(28, 194, &line, theme.text_dim);
            }
            LoadState::Loading => {
                canvas.draw_text(132, 184, "Loading...", theme.text);
            }
        }

        canvas.hbar(0, (WIN_H as i32) - FOOTER_H, WIN_W, 1, theme.border);
        canvas.fill_rect(
            Rect::new(0, (WIN_H as i32) - FOOTER_H + 1, WIN_W, (FOOTER_H - 1) as u32),
            theme.panel_alt,
        );
        let footer = fit_text(&self.status, 52);
        canvas.draw_text(10, (WIN_H as i32) - 16, &footer, theme.text_dim);
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::MouseMove { x, y } => {
                let hover = self.item_at(x, y);
                if hover != self.hover {
                    self.hover = hover;
                    if let Some(index) = hover {
                        self.selected = index;
                    }
                    return true;
                }
            }
            Event::Click { x, y } => {
                if let Some(index) = self.item_at(x, y) {
                    self.selected = index;
                    return self.select_current();
                }
            }
            Event::KeyPress {
                keycode: KEY_ESC,
                pressed: true,
                ..
            } => {
                request_close();
                return true;
            }
            Event::KeyPress {
                keycode: KEY_UP,
                pressed: true,
                ..
            } => return self.move_selection(-1),
            Event::KeyPress {
                keycode: KEY_DOWN,
                pressed: true,
                ..
            } => return self.move_selection(1),
            Event::KeyPress {
                keycode: KEY_ENTER,
                pressed: true,
                ..
            } => return self.select_current(),
            Event::Key('\n') => return self.select_current(),
            _ => {}
        }
        false
    }
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let mut app = ClipmanApp::new();
    let mut window = match Window::connect(WindowConfig {
        width: WIN_W,
        height: WIN_H,
        title: "Sunlight Clipman",
        decoration: WindowDecoration::HiddenOverlay,
    }) {
        Some(window) => window,
        None => loop {
            process_yield();
        },
    };
    window.run(&mut app);
    ProcessExit::exit(0);
}

fn take_reply_bytes(reply: &IpcMsg) -> Result<Vec<u8>, ClipError> {
    let len = reply.words[1] as usize;
    let token = reply.caps[0];
    if len == 0 || token == CapabilityToken::INVALID {
        return Ok(Vec::new());
    }
    let ptr = shm_map(token).map_err(|_| ClipError::Internal)?;
    let bytes = unsafe { core::slice::from_raw_parts(ptr, len) }.to_vec();
    let _ = shm_free(token);
    Ok(bytes)
}

fn error_label(code: u64) -> &'static str {
    match code {
        x if x == ClipError::BadRequest.code() => "Bad clipboard request",
        x if x == ClipError::NotFound.code() => "Clipboard item not found",
        x if x == ClipError::TooLarge.code() => "Clipboard item is too large",
        x if x == ClipError::Unsupported.code() => "Clipboard type is not supported yet",
        x if x == ClipError::Corrupt.code() => "Clipboard data is corrupt",
        _ => "Clipboard service error",
    }
}

fn fit_text(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        return String::from(text);
    }
    let mut out = String::new();
    for ch in text.chars().take(max_chars.saturating_sub(1)) {
        out.push(if ch.is_control() { ' ' } else { ch });
    }
    out.push('…');
    out
}

fn append_u64(out: &mut String, mut value: u64) {
    if value == 0 {
        out.push('0');
        return;
    }
    let mut buf = [0u8; 20];
    let mut len = 0usize;
    while value > 0 {
        buf[len] = b'0' + (value % 10) as u8;
        value /= 10;
        len += 1;
    }
    while len > 0 {
        len -= 1;
        out.push(buf[len] as char);
    }
}
