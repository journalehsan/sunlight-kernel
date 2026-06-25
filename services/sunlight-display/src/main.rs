#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;

use sunlight_ipc::debug_log;
use sunlight_ipc::{
    endpoint_create, ipc_recv, ipc_reply, nameserver_register,
    sgp::SgpMsg,
    CapabilityToken, IpcMsg, MouseMsg,
};

// ---------------------------------------------------------------------------
// Allocator
// ---------------------------------------------------------------------------

struct BumpAllocator;
unsafe impl core::alloc::GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        static mut HEAP: [u8; 8 * 1024 * 1024] = [0; 8 * 1024 * 1024];
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

// ---------------------------------------------------------------------------
// Fixed-point pointer acceleration (unchanged from Phase 1)
// ---------------------------------------------------------------------------

const FP_SHIFT: i32 = 16;
const FP_ONE: i32 = 1 << FP_SHIFT;

const SENS_DEFAULT_FP: i32 = (FP_ONE * 3) / 2;
#[allow(dead_code)]
const SENS_MIN_FP: i32 = FP_ONE / 2;
#[allow(dead_code)]
const SENS_MAX_FP: i32 = FP_ONE * 3;

const ACCEL_LOW: i32 = 5;
const ACCEL_HIGH: i32 = 20;
const ACCEL_SLOPE_FP: i32 = FP_ONE / 20;
const ACCEL_MAX_FP: i32 = FP_ONE * 2;

const SMOOTH_SNAP_SPEED: i32 = 3;
const SMOOTH_ALPHA_FP: i32 = (FP_ONE * 45) / 100;

const EDGE_MARGIN: i32 = 2;

// ---------------------------------------------------------------------------
// Window property enums
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
#[repr(u8)]
enum WindowType {
    Normal  = 0,
    Dialog  = 1,
    Desktop = 2,
    Widget  = 3,
}

#[derive(Clone, Copy, PartialEq)]
#[repr(u8)]
enum WindowState {
    Normal     = 0,
    Minimized  = 1,
    Maximized  = 2,
    Fullscreen = 3,
}

#[derive(Clone, Copy, PartialEq)]
#[repr(u8)]
enum BorderStyle {
    Full = 0,
    None = 1,
}

#[derive(Clone, Copy, PartialEq)]
#[repr(u8)]
enum ZIndexType {
    Normal = 0,
    OnTop  = 1,
}

#[derive(Clone, Copy, PartialEq)]
#[repr(u8)]
enum ShowType {
    Floating  = 0,
    Tiled     = 1,
    Scrolling = 2,
}

#[derive(Clone, Copy, PartialEq)]
#[repr(u8)]
enum GroupType {
    None    = 0,
    Stacked = 1,
    Tabbed  = 2,
}

/// Cursor shapes the compositor can render. Clients request via SGP_SET_CURSOR.
#[derive(Clone, Copy, PartialEq)]
#[repr(u8)]
enum CursorShape {
    Pointer       = 0,
    Hand          = 1,
    ResizeH       = 2,
    ResizeV       = 3,
    ResizeCornerNW = 4,
    ResizeCornerNE = 5,
    Moving        = 6,
    Waiting       = 7,
    Question      = 8,
}

impl CursorShape {
    fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Hand,
            2 => Self::ResizeH,
            3 => Self::ResizeV,
            4 => Self::ResizeCornerNW,
            5 => Self::ResizeCornerNE,
            6 => Self::Moving,
            7 => Self::Waiting,
            8 => Self::Question,
            _ => Self::Pointer,
        }
    }
}

// ---------------------------------------------------------------------------
// WindowConfig — binary representation exchanged over IPC
//
// IPC encoding (CREATE_WINDOW / CONFIGURE_WINDOW):
//   words[0] = (client_w as u32) | ((client_h as u32) << 32)   [CREATE only]
//   words[1] = config_flags  (see SgpMsg::config_flags bit layout)
//   words[2] = (pid as u32)  | ((ppid as u32) << 32)
//   words[3] = title bytes [0..8]  (first 8 ASCII chars, LE u64)
//
// Only words[0..3] are transported via register IPC (IPC_REGISTER_WORDS = 4).
// A follow-up SGP_CONFIGURE_WINDOW with a SHM page at caps[0] provides the
// full null-terminated title (up to 255 chars) and extended group_ids.
// ---------------------------------------------------------------------------

#[allow(dead_code)]
struct WindowConfig {
    title:         [u8; 64],
    window_type:   WindowType,
    state:         WindowState,
    border:        BorderStyle,
    z_index_type:  ZIndexType,
    z_index_value: u8,
    show_type:     ShowType,
    group_type:    GroupType,
    pid:           u32,
    ppid:          u32,
    group_ids:     [u32; 4],
    group_id_count: u8,
}

impl WindowConfig {
    #[allow(dead_code)]
    fn default_titled(fallback: &[u8]) -> Self {
        let mut title = [0u8; 64];
        let n = fallback.len().min(63);
        title[..n].copy_from_slice(&fallback[..n]);
        Self {
            title,
            window_type:    WindowType::Normal,
            state:          WindowState::Normal,
            border:         BorderStyle::Full,
            z_index_type:   ZIndexType::Normal,
            z_index_value:  50,
            show_type:      ShowType::Floating,
            group_type:     GroupType::None,
            pid:            0,
            ppid:           0,
            group_ids:      [0; 4],
            group_id_count: 0,
        }
    }

    fn from_ipc_words(words: &[u64; 8]) -> Self {
        let flags = words[1];
        let pid   = (words[2] & 0xFFFF_FFFF) as u32;
        let ppid  = (words[2] >> 32) as u32;

        let window_type = match (flags & 0x3) as u8 {
            1 => WindowType::Dialog,
            2 => WindowType::Desktop,
            3 => WindowType::Widget,
            _ => WindowType::Normal,
        };
        let state = match ((flags >> 2) & 0x3) as u8 {
            1 => WindowState::Minimized,
            2 => WindowState::Maximized,
            3 => WindowState::Fullscreen,
            _ => WindowState::Normal,
        };
        let border       = if (flags >> 4) & 1 != 0 { BorderStyle::None } else { BorderStyle::Full };
        let z_index_type = if (flags >> 5) & 1 != 0 { ZIndexType::OnTop } else { ZIndexType::Normal };
        let z_raw        = ((flags >> 6) & 0x7F) as u8;
        let z_index_value = if z_raw == 0 { 50 } else { z_raw.min(100) };
        let show_type = match ((flags >> 13) & 0x3) as u8 {
            1 => ShowType::Tiled,
            2 => ShowType::Scrolling,
            _ => ShowType::Floating,
        };
        let group_type = match ((flags >> 15) & 0x3) as u8 {
            1 => GroupType::Stacked,
            2 => GroupType::Tabbed,
            _ => GroupType::None,
        };

        // words[3] carries the first 8 title bytes (little-endian u64).
        let mut title = [0u8; 64];
        let w3 = words[3];
        for i in 0..8usize {
            title[i] = ((w3 >> (i * 8)) & 0xFF) as u8;
        }
        // Fallback title when client sent nothing.
        if title[0] == 0 {
            let fb = b"SunlightOS Application";
            title[..fb.len()].copy_from_slice(fb);
        }

        Self {
            title,
            window_type,
            state,
            border,
            z_index_type,
            z_index_value,
            show_type,
            group_type,
            pid,
            ppid,
            group_ids: [0; 4],
            group_id_count: 0,
        }
    }

    /// Apply a SHM page update: the page starts with a null-terminated title string.
    fn apply_shm_title(&mut self, shm_ptr: *const u8, shm_len: usize) {
        if shm_ptr.is_null() { return; }
        let max = shm_len.min(63);
        for i in 0..max {
            let b = unsafe { shm_ptr.add(i).read() };
            if b == 0 { self.title[i] = 0; break; }
            self.title[i] = b;
        }
        self.title[63] = 0;
    }
}

// ---------------------------------------------------------------------------
// Drag / resize state
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum ResizeEdge {
    Left,
    Right,
    Bottom,
    CornerBL,
    CornerBR,
}

#[derive(Clone, Copy)]
struct MoveDrag {
    window_id: u64,
}

#[derive(Clone, Copy)]
struct ResizeDrag {
    window_id:  u64,
    edge:       ResizeEdge,
    // Mouse and window geometry at drag start
    anchor_mx:  i32,
    anchor_my:  i32,
    anchor_wx:  i32,
    anchor_wy:  i32,
    anchor_ww:  i32,
    anchor_wh:  i32,
}

enum ActiveDrag {
    None,
    Move(MoveDrag),
    Resize(ResizeDrag),
}

// ---------------------------------------------------------------------------
// Hit-testing
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum HitZone {
    Miss,
    TitleBar,
    CloseBtn,
    MaximizeBtn,
    MinimizeBtn,
    KeepOnTopBtn,
    ClientArea,
    EdgeLeft,
    EdgeRight,
    EdgeBottom,
    CornerBL,
    CornerBR,
}

impl HitZone {
    fn default_cursor(self) -> CursorShape {
        match self {
            HitZone::TitleBar | HitZone::ClientArea | HitZone::Miss => CursorShape::Pointer,
            HitZone::CloseBtn | HitZone::MaximizeBtn | HitZone::MinimizeBtn | HitZone::KeepOnTopBtn => CursorShape::Pointer,
            HitZone::EdgeLeft | HitZone::EdgeRight                   => CursorShape::ResizeH,
            HitZone::EdgeBottom                                      => CursorShape::ResizeV,
            HitZone::CornerBL                                        => CursorShape::ResizeCornerNW,
            HitZone::CornerBR                                        => CursorShape::ResizeCornerNE,
        }
    }
}

// ---------------------------------------------------------------------------
// PointerState (unchanged from Phase 1)
// ---------------------------------------------------------------------------

struct PointerState {
    x_fp: i32,
    y_fp: i32,
    target_x_fp: i32,
    target_y_fp: i32,
    buttons: u8,
    fb_width: u32,
    fb_height: u32,
    sensitivity_fp: i32,
}

impl PointerState {
    fn new(fb_w: u32, fb_h: u32) -> Self {
        let cx = ((fb_w as i32 / 2).max(0)) << FP_SHIFT;
        let cy = ((fb_h as i32 / 2).max(0)) << FP_SHIFT;
        Self {
            x_fp: cx,
            y_fp: cy,
            target_x_fp: cx,
            target_y_fp: cy,
            buttons: 0,
            fb_width: fb_w,
            fb_height: fb_h,
            sensitivity_fp: SENS_DEFAULT_FP,
        }
    }

    fn apply_motion(&mut self, dx: i32, dy: i32, buttons: u8) {
        let speed = dx.abs().max(dy.abs());
        let accel_fp = if speed <= ACCEL_LOW {
            FP_ONE
        } else if speed <= ACCEL_HIGH {
            (FP_ONE + (speed - ACCEL_LOW) * ACCEL_SLOPE_FP).min(ACCEL_MAX_FP)
        } else {
            ACCEL_MAX_FP
        };
        let gain_fp = ((self.sensitivity_fp as i64 * accel_fp as i64) >> FP_SHIFT) as i64;
        self.target_x_fp = (self.target_x_fp as i64 + (dx as i64 * gain_fp)) as i32;
        self.target_y_fp = (self.target_y_fp as i64 + (dy as i64 * gain_fp)) as i32;
        self.clamp_target();
        let alpha_fp = if speed <= SMOOTH_SNAP_SPEED { FP_ONE } else { SMOOTH_ALPHA_FP };
        self.x_fp += (((self.target_x_fp - self.x_fp) as i64 * alpha_fp as i64) >> FP_SHIFT) as i32;
        self.y_fp += (((self.target_y_fp - self.y_fp) as i64 * alpha_fp as i64) >> FP_SHIFT) as i32;
        self.buttons = buttons;
    }

    fn min_fp(&self) -> i32 { EDGE_MARGIN << FP_SHIFT }
    fn max_x_fp(&self) -> i32 { ((self.fb_width as i32 - 1 - EDGE_MARGIN).max(EDGE_MARGIN)) << FP_SHIFT }
    fn max_y_fp(&self) -> i32 { ((self.fb_height as i32 - 1 - EDGE_MARGIN).max(EDGE_MARGIN)) << FP_SHIFT }

    fn clamp_target(&mut self) {
        self.target_x_fp = self.target_x_fp.clamp(self.min_fp(), self.max_x_fp());
        self.target_y_fp = self.target_y_fp.clamp(self.min_fp(), self.max_y_fp());
    }

    fn sync_clamp(&mut self) {
        self.x_fp = self.x_fp.clamp(self.min_fp(), self.max_x_fp());
        self.y_fp = self.y_fp.clamp(self.min_fp(), self.max_y_fp());
    }

    fn x(&self) -> i32 { (self.x_fp >> FP_SHIFT).max(0).min((self.fb_width - 1) as i32) }
    fn y(&self) -> i32 { (self.y_fp >> FP_SHIFT).max(0).min((self.fb_height - 1) as i32) }
}

// ---------------------------------------------------------------------------
// Window
// ---------------------------------------------------------------------------

struct Window {
    id:        u64,
    _shm_cap:  CapabilityToken,
    buffer:    *mut u32,
    width:     u32,   // current client area width
    height:    u32,   // current client area height
    x:         u32,   // chrome top-left on screen
    y:         u32,
    // Saved normal geometry for restore from maximized/fullscreen.
    saved_x:   u32,
    saved_y:   u32,
    saved_w:   u32,
    saved_h:   u32,
    config:    WindowConfig,
    /// Cursor shape the client wants when the pointer is in its client area.
    client_cursor: CursorShape,
}

impl Window {
    /// Returns the chrome rectangle (x, y, w, h) accounting for state.
    fn chrome_rect(&self, fb_w: u32, fb_h: u32) -> (u32, u32, u32, u32) {
        match self.config.state {
            WindowState::Fullscreen => (0, 0, fb_w, fb_h),
            WindowState::Maximized  => (0, 0, fb_w, fb_h),
            _                       => (
                self.x, self.y,
                self.width + BORDER_W * 2,
                TITLEBAR_H + self.height + BORDER_W,
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// CompositorState
// ---------------------------------------------------------------------------

struct CompositorState {
    windows:            Vec<Window>,
    mouse_x:            u16,
    mouse_y:            u16,
    pointer:            PointerState,
    mouse_sensitivity_fp: i32,
    active_drag:        ActiveDrag,
    prev_buttons:       u8,
    fb:                 *mut u32,
    fb_width:           u32,
    fb_height:          u32,
    fb_pitch:           u32,
    active_cursor:      CursorShape,
    // True when Desktop session owns the framebuffer; false when TTY/login is active.
    // tty_server sends SESSION_ACTIVATE/SESSION_DEACTIVATE to toggle this.
    session_active:     bool,
}

fn fb_stride(state: &CompositorState) -> usize {
    (state.fb_pitch / 4) as usize
}

// ---------------------------------------------------------------------------
// Chrome constants (Vortex Shell Theme)
// ---------------------------------------------------------------------------

const DESKTOP_COLOR:      u32 = 0x00121214; // Deep dark gray/black
const TITLEBAR_H:         u32 = 32;         // Taller for Chrome-tab style
const TITLEBAR_COLOR:     u32 = 0x002B2B36; // inactive titlebar (dark slate)
const TITLEBAR_ACTIVE:    u32 = 0x001E1E26; // active window titlebar (darker base)
const TITLEBAR_ACCENT:    u32 = 0x00FF7A00; // Warm/Orange accent line
const TITLE_TEXT_COLOR:   u32 = 0x00E0E0E0; // Off-white
const BORDER_W:           u32 = 2;          // Thinner modern border
const BORDER_COLOR:       u32 = 0x00FF7A00; // Active window border glow
const BORDER_INACTIVE:    u32 = 0x002B2B36; // Inactive window border
const BTN_HOVER_BG:       u32 = 0x003A3A4A; // Hover state background for buttons
const BTN_ICON_COLOR:     u32 = 0x00B0B0C0; // Icon color
const BTN_ICON_ACTIVE:    u32 = 0x00FFFFFF; // Icon color when active/focused
const BTN_SIZE:           u32 = 20;         // Size of control buttons
const BTN_SPACING:        u32 = 4;
const RESIZE_BORDER:      u32 = 6; // effective hit-test width for edges/corners
const MIN_WIN_W:          u32 = 200;
const MIN_WIN_H:          u32 = 100;

// ---------------------------------------------------------------------------
// Cursor bitmaps — 8 px wide × 12 rows, one u8 per row, MSB = leftmost pixel
// ---------------------------------------------------------------------------

const CURSOR_W: usize = 8;
const CURSOR_H: usize = 12;

// Default arrow
const CURSOR_PTR: [u8; CURSOR_H] = [
    0b1000_0000,
    0b1100_0000,
    0b1110_0000,
    0b1111_0000,
    0b1111_1000,
    0b1111_1100,
    0b1111_1110,
    0b1111_1000,
    0b1101_0000,
    0b1001_0000,
    0b0001_0000,
    0b0000_0000,
];

// Hand (pointing finger)
const CURSOR_HAND: [u8; CURSOR_H] = [
    0b0010_0000, // index finger tip
    0b0010_0000,
    0b0010_1010, // grip shape
    0b0010_1010,
    0b1111_1110, // palm top
    0b1111_1110,
    0b1111_1100,
    0b0111_1100,
    0b0011_1000,
    0b0011_1000,
    0b0001_0000,
    0b0000_0000,
];

// Horizontal resize ←→
const CURSOR_RESIZE_H: [u8; CURSOR_H] = [
    0b0000_0000,
    0b0000_0000,
    0b0001_1000,
    0b0011_1100,
    0b0111_1110,
    0b1111_1111, // full bar
    0b0111_1110,
    0b0011_1100,
    0b0001_1000,
    0b0000_0000,
    0b0000_0000,
    0b0000_0000,
];

// Vertical resize ↑↓
const CURSOR_RESIZE_V: [u8; CURSOR_H] = [
    0b0001_1000, // up arrowhead
    0b0011_1100,
    0b0001_1000,
    0b0001_1000,
    0b0001_1000, // center bar
    0b0001_1000,
    0b0001_1000,
    0b0011_1100, // down arrowhead
    0b0001_1000,
    0b0000_0000,
    0b0000_0000,
    0b0000_0000,
];

// NW-SE diagonal corner (↖↘)
const CURSOR_RESIZE_NW: [u8; CURSOR_H] = [
    0b1111_0000,
    0b1100_0000,
    0b1010_0000,
    0b1001_0000,
    0b0000_1001,
    0b0000_1010,
    0b0000_1100,
    0b0000_1111,
    0b0000_0000,
    0b0000_0000,
    0b0000_0000,
    0b0000_0000,
];

// NE-SW diagonal corner (↗↙)
const CURSOR_RESIZE_NE: [u8; CURSOR_H] = [
    0b0000_1111,
    0b0000_0110,
    0b0000_1010,
    0b0001_0010,
    0b0100_1000,
    0b0101_0000,
    0b0110_0000,
    0b1111_0000,
    0b0000_0000,
    0b0000_0000,
    0b0000_0000,
    0b0000_0000,
];

// 4-way move (✛)
const CURSOR_MOVE: [u8; CURSOR_H] = [
    0b0001_1000,
    0b0011_1100,
    0b0001_1000,
    0b1001_1001,
    0b1111_1111,
    0b1001_1001,
    0b0001_1000,
    0b0011_1100,
    0b0001_1000,
    0b0000_0000,
    0b0000_0000,
    0b0000_0000,
];

// Hourglass wait
const CURSOR_WAIT: [u8; CURSOR_H] = [
    0b1111_1110,
    0b0111_1100,
    0b0011_1000,
    0b0001_1000,
    0b0001_1000, // waist
    0b0001_1000,
    0b0011_1100,
    0b0111_1110,
    0b1111_1111,
    0b0000_0000,
    0b0000_0000,
    0b0000_0000,
];

// Question mark with arrow base
const CURSOR_QUESTION: [u8; CURSOR_H] = [
    0b1100_0000, // arrow tip
    0b1110_0000,
    0b1001_1000, // ?
    0b0000_1100,
    0b0001_1000,
    0b0001_0000,
    0b0000_0000,
    0b0001_0000, // dot
    0b0001_0000,
    0b0000_0000,
    0b0000_0000,
    0b0000_0000,
];

fn cursor_bitmap(shape: CursorShape) -> &'static [u8; CURSOR_H] {
    match shape {
        CursorShape::Pointer        => &CURSOR_PTR,
        CursorShape::Hand           => &CURSOR_HAND,
        CursorShape::ResizeH        => &CURSOR_RESIZE_H,
        CursorShape::ResizeV        => &CURSOR_RESIZE_V,
        CursorShape::ResizeCornerNW => &CURSOR_RESIZE_NW,
        CursorShape::ResizeCornerNE => &CURSOR_RESIZE_NE,
        CursorShape::Moving         => &CURSOR_MOVE,
        CursorShape::Waiting        => &CURSOR_WAIT,
        CursorShape::Question       => &CURSOR_QUESTION,
    }
}

// ---------------------------------------------------------------------------
// 5×7 pixel font for title text (column format, LSB = top row)
// 96 glyphs starting at ASCII 0x20 (space).
// ---------------------------------------------------------------------------

static FONT_5X7: [[u8; 5]; 96] = [
    [0x00,0x00,0x00,0x00,0x00], // ' '
    [0x00,0x00,0x5F,0x00,0x00], // '!'
    [0x00,0x07,0x00,0x07,0x00], // '"'
    [0x14,0x7F,0x14,0x7F,0x14], // '#'
    [0x24,0x2A,0x7F,0x2A,0x12], // '$'
    [0x23,0x13,0x08,0x64,0x62], // '%'
    [0x36,0x49,0x55,0x22,0x50], // '&'
    [0x00,0x05,0x03,0x00,0x00], // '\''
    [0x00,0x1C,0x22,0x41,0x00], // '('
    [0x00,0x41,0x22,0x1C,0x00], // ')'
    [0x14,0x08,0x3E,0x08,0x14], // '*'
    [0x08,0x08,0x3E,0x08,0x08], // '+'
    [0x00,0x50,0x30,0x00,0x00], // ','
    [0x08,0x08,0x08,0x08,0x08], // '-'
    [0x00,0x60,0x60,0x00,0x00], // '.'
    [0x20,0x10,0x08,0x04,0x02], // '/'
    [0x3E,0x51,0x49,0x45,0x3E], // '0'
    [0x00,0x42,0x7F,0x40,0x00], // '1'
    [0x42,0x61,0x51,0x49,0x46], // '2'
    [0x21,0x41,0x45,0x4B,0x31], // '3'
    [0x18,0x14,0x12,0x7F,0x10], // '4'
    [0x27,0x45,0x45,0x45,0x39], // '5'
    [0x3C,0x4A,0x49,0x49,0x30], // '6'
    [0x01,0x71,0x09,0x05,0x03], // '7'
    [0x36,0x49,0x49,0x49,0x36], // '8'
    [0x06,0x49,0x49,0x29,0x1E], // '9'
    [0x00,0x36,0x36,0x00,0x00], // ':'
    [0x00,0x56,0x36,0x00,0x00], // ';'
    [0x08,0x14,0x22,0x41,0x00], // '<'
    [0x14,0x14,0x14,0x14,0x14], // '='
    [0x00,0x41,0x22,0x14,0x08], // '>'
    [0x02,0x01,0x51,0x09,0x06], // '?'
    [0x32,0x49,0x79,0x41,0x3E], // '@'
    [0x7E,0x11,0x11,0x11,0x7E], // 'A'
    [0x7F,0x49,0x49,0x49,0x36], // 'B'
    [0x3E,0x41,0x41,0x41,0x22], // 'C'
    [0x7F,0x41,0x41,0x22,0x1C], // 'D'
    [0x7F,0x49,0x49,0x49,0x41], // 'E'
    [0x7F,0x09,0x09,0x09,0x01], // 'F'
    [0x3E,0x41,0x49,0x49,0x7A], // 'G'
    [0x7F,0x08,0x08,0x08,0x7F], // 'H'
    [0x00,0x41,0x7F,0x41,0x00], // 'I'
    [0x20,0x40,0x41,0x3F,0x01], // 'J'
    [0x7F,0x08,0x14,0x22,0x41], // 'K'
    [0x7F,0x40,0x40,0x40,0x40], // 'L'
    [0x7F,0x02,0x0C,0x02,0x7F], // 'M'
    [0x7F,0x04,0x08,0x10,0x7F], // 'N'
    [0x3E,0x41,0x41,0x41,0x3E], // 'O'
    [0x7F,0x09,0x09,0x09,0x06], // 'P'
    [0x3E,0x41,0x51,0x21,0x5E], // 'Q'
    [0x7F,0x09,0x19,0x29,0x46], // 'R'
    [0x46,0x49,0x49,0x49,0x31], // 'S'
    [0x01,0x01,0x7F,0x01,0x01], // 'T'
    [0x3F,0x40,0x40,0x40,0x3F], // 'U'
    [0x1F,0x20,0x40,0x20,0x1F], // 'V'
    [0x3F,0x40,0x38,0x40,0x3F], // 'W'
    [0x63,0x14,0x08,0x14,0x63], // 'X'
    [0x07,0x08,0x70,0x08,0x07], // 'Y'
    [0x61,0x51,0x49,0x45,0x43], // 'Z'
    [0x00,0x7F,0x41,0x41,0x00], // '['
    [0x02,0x04,0x08,0x10,0x20], // '\\'
    [0x00,0x41,0x41,0x7F,0x00], // ']'
    [0x04,0x02,0x01,0x02,0x04], // '^'
    [0x40,0x40,0x40,0x40,0x40], // '_'
    [0x00,0x01,0x02,0x04,0x00], // '`'
    [0x20,0x54,0x54,0x54,0x78], // 'a'
    [0x7F,0x48,0x44,0x44,0x38], // 'b'
    [0x38,0x44,0x44,0x44,0x20], // 'c'
    [0x38,0x44,0x44,0x48,0x7F], // 'd'
    [0x38,0x54,0x54,0x54,0x18], // 'e'
    [0x08,0x7E,0x09,0x01,0x02], // 'f'
    [0x0C,0x52,0x52,0x52,0x3E], // 'g'
    [0x7F,0x08,0x04,0x04,0x78], // 'h'
    [0x00,0x44,0x7D,0x40,0x00], // 'i'
    [0x20,0x40,0x44,0x3D,0x00], // 'j'
    [0x7F,0x10,0x28,0x44,0x00], // 'k'
    [0x00,0x41,0x7F,0x40,0x00], // 'l'
    [0x7C,0x04,0x18,0x04,0x78], // 'm'
    [0x7C,0x08,0x04,0x04,0x78], // 'n'
    [0x38,0x44,0x44,0x44,0x38], // 'o'
    [0x7C,0x14,0x14,0x14,0x08], // 'p'
    [0x08,0x14,0x14,0x18,0x7C], // 'q'
    [0x7C,0x08,0x04,0x04,0x08], // 'r'
    [0x48,0x54,0x54,0x54,0x20], // 's'
    [0x04,0x3F,0x44,0x40,0x20], // 't'
    [0x3C,0x40,0x40,0x20,0x7C], // 'u'
    [0x1C,0x20,0x40,0x20,0x1C], // 'v'
    [0x3C,0x40,0x30,0x40,0x3C], // 'w'
    [0x44,0x28,0x10,0x28,0x44], // 'x'
    [0x0C,0x50,0x50,0x50,0x3C], // 'y'
    [0x44,0x64,0x54,0x4C,0x44], // 'z'
    [0x00,0x08,0x36,0x41,0x00], // '{'
    [0x00,0x00,0x7F,0x00,0x00], // '|'
    [0x00,0x41,0x36,0x08,0x00], // '}'
    [0x10,0x08,0x08,0x10,0x08], // '~'
    [0x00,0x00,0x00,0x00,0x00], // DEL
];

// ---------------------------------------------------------------------------
// Drawing primitives
// ---------------------------------------------------------------------------

fn draw_rect(state: &CompositorState, x: u32, y: u32, w: u32, h: u32, color: u32) {
    if x >= state.fb_width || y >= state.fb_height || w == 0 || h == 0 { return; }
    let stride  = fb_stride(state);
    let x_end   = (x + w).min(state.fb_width) as usize;
    let y_end   = (y + h).min(state.fb_height) as usize;
    for row in y as usize..y_end {
        for col in x as usize..x_end {
            unsafe { state.fb.add(row * stride + col).write(color); }
        }
    }
}

fn clear_framebuffer(state: &CompositorState) {
    let stride = fb_stride(state);
    for y in 0..state.fb_height as usize {
        for x in 0..state.fb_width as usize {
            unsafe { state.fb.add(y * stride + x).write(DESKTOP_COLOR); }
        }
    }
}

/// Draw a single 5×7 glyph. Each column byte: LSB = top pixel.
fn draw_char(state: &CompositorState, x: i32, y: i32, ch: u8, fg: u32) {
    let idx = ch.saturating_sub(0x20).min(95) as usize;
    let glyph = &FONT_5X7[idx];
    let stride = fb_stride(state);
    for col in 0..5usize {
        for row in 0..7usize {
            if (glyph[col] >> row) & 1 == 0 { continue; }
            let px = x + col as i32;
            let py = y + row as i32;
            if px < 0 || py < 0 || px >= state.fb_width as i32 || py >= state.fb_height as i32 {
                continue;
            }
            unsafe { state.fb.add(py as usize * stride + px as usize).write(fg); }
        }
    }
}

/// Draw a null-terminated title string centered vertically in the title bar.
fn draw_title(state: &CompositorState, title: &[u8; 64], bar_x: u32, bar_y: u32, bar_w: u32) {
    // Measure length (stop at null).
    let mut len = 0usize;
    for &b in title.iter() { if b == 0 { break; } len += 1; }
    if len == 0 { return; }

    // Each glyph is 5px wide + 1px gap = 6px; clamp to available bar width.
    let glyph_stride = 6i32;
    let text_w       = len as i32 * glyph_stride;

    // Reserve left margin for close button (BTN_SIZE + 6) + 4px pad.
    let left_margin  = (BTN_SIZE + 10) as i32;
    let avail_w      = bar_w as i32 - left_margin - 4;
    if avail_w <= 0 { return; }

    // Center horizontally in the available space.
    let text_start_x = bar_x as i32 + left_margin + (avail_w - text_w).max(0) / 2;
    let text_start_y = bar_y as i32 + (TITLEBAR_H as i32 - 7) / 2;

    let max_chars = (avail_w / glyph_stride) as usize;
    for (i, &b) in title.iter().take(len.min(max_chars)).enumerate() {
        draw_char(state, text_start_x + i as i32 * glyph_stride, text_start_y, b, TITLE_TEXT_COLOR);
    }
}

// ---------------------------------------------------------------------------
// Hit-testing
// ---------------------------------------------------------------------------

fn hit_test_window(win: &Window, cx: u32, cy: u32, fb_w: u32, fb_h: u32) -> HitZone {
    // Determine effective chrome origin / size based on window state.
    let (wx, wy, chrome_w, chrome_h) = win.chrome_rect(fb_w, fb_h);

    let fullscreen = win.config.state == WindowState::Fullscreen;
    let no_border  = win.config.border == BorderStyle::None || fullscreen;

    // Completely outside this window?
    if cx < wx || cy < wy || cx >= wx + chrome_w || cy >= wy + chrome_h {
        return HitZone::Miss;
    }

    let rel_x = cx - wx;
    let rel_y = cy - wy;

    // Title bar zone (not present in Fullscreen or no-border Widget/Desktop).
    if !fullscreen && !no_border {
        if rel_y < TITLEBAR_H {
            let mut btn_x = chrome_w.saturating_sub(BTN_SIZE + BTN_SPACING);

            // Close button
            if rel_x >= btn_x && rel_x < btn_x + BTN_SIZE && rel_y >= (TITLEBAR_H - BTN_SIZE) / 2 && rel_y < (TITLEBAR_H + BTN_SIZE) / 2 {
                return HitZone::CloseBtn;
            }
            btn_x = btn_x.saturating_sub(BTN_SIZE + BTN_SPACING);

            // Maximize/Restore button
            if rel_x >= btn_x && rel_x < btn_x + BTN_SIZE && rel_y >= (TITLEBAR_H - BTN_SIZE) / 2 && rel_y < (TITLEBAR_H + BTN_SIZE) / 2 {
                return HitZone::MaximizeBtn;
            }
            btn_x = btn_x.saturating_sub(BTN_SIZE + BTN_SPACING);

            // Minimize button
            if rel_x >= btn_x && rel_x < btn_x + BTN_SIZE && rel_y >= (TITLEBAR_H - BTN_SIZE) / 2 && rel_y < (TITLEBAR_H + BTN_SIZE) / 2 {
                return HitZone::MinimizeBtn;
            }
            btn_x = btn_x.saturating_sub(BTN_SIZE + BTN_SPACING);

            // Keep On Top button
            if rel_x >= btn_x && rel_x < btn_x + BTN_SIZE && rel_y >= (TITLEBAR_H - BTN_SIZE) / 2 && rel_y < (TITLEBAR_H + BTN_SIZE) / 2 {
                return HitZone::KeepOnTopBtn;
            }

            return HitZone::TitleBar;
        }
    }

    // If no border, everything below the title bar is client area.
    if no_border {
        return HitZone::ClientArea;
    }

    // Corner zones (checked before edge zones — larger grab target wins).
    let corner_size = RESIZE_BORDER + 4;
    let bottom_zone = rel_y >= TITLEBAR_H + win.height.saturating_sub(corner_size);

    if bottom_zone {
        if rel_x < corner_size {
            return HitZone::CornerBL;
        }
        if rel_x >= chrome_w.saturating_sub(corner_size) {
            return HitZone::CornerBR;
        }
        if rel_y >= TITLEBAR_H + win.height {
            return HitZone::EdgeBottom;
        }
    }

    // Edge zones.
    if rel_x < RESIZE_BORDER               { return HitZone::EdgeLeft;   }
    if rel_x >= chrome_w - RESIZE_BORDER   { return HitZone::EdgeRight;  }
    if rel_y >= TITLEBAR_H + win.height    { return HitZone::EdgeBottom; }

    HitZone::ClientArea
}

/// Compute the cursor shape to display given all visible windows and pointer pos.
fn cursor_for_scene(state: &CompositorState) -> CursorShape {
    let cx = state.mouse_x as u32;
    let cy = state.mouse_y as u32;

    // Walk front-to-back (last window in vec = top of z-order).
    for win in state.windows.iter().rev() {
        if win.config.state == WindowState::Minimized { continue; }
        let zone = hit_test_window(win, cx, cy, state.fb_width, state.fb_height);
        if zone == HitZone::Miss { continue; }

        return match zone {
            HitZone::ClientArea => win.client_cursor,
            other               => other.default_cursor(),
        };
    }
    CursorShape::Pointer
}

// ---------------------------------------------------------------------------
// Compositing
// ---------------------------------------------------------------------------

fn composite_window(state: &CompositorState, win: &Window, is_focused: bool) {
    if win.buffer.is_null() { return; }
    if win.config.state == WindowState::Minimized { return; }

    let fullscreen = win.config.state == WindowState::Fullscreen;
    let maximized  = win.config.state == WindowState::Maximized;
    let no_chrome  = fullscreen || win.config.border == BorderStyle::None;

    let (canvas_x, canvas_y, client_w, client_h) = if fullscreen || maximized {
        let cw = if no_chrome { state.fb_width  } else { state.fb_width.saturating_sub(BORDER_W * 2) };
        let ch = if no_chrome { state.fb_height } else { state.fb_height.saturating_sub(TITLEBAR_H + BORDER_W) };
        let ox = if no_chrome { 0 } else { BORDER_W };
        let oy = if no_chrome { 0 } else { TITLEBAR_H };
        (ox, oy, cw, ch)
    } else {
        (win.x + BORDER_W, win.y + TITLEBAR_H, win.width, win.height)
    };

    if !no_chrome {
        let (wx, wy) = if maximized { (0u32, 0u32) } else { (win.x, win.y) };
        let chrome_w = if maximized { state.fb_width } else { win.width + BORDER_W * 2 };
        let chrome_h = if maximized { state.fb_height } else { TITLEBAR_H + win.height + BORDER_W };

        let tb_color = if is_focused { TITLEBAR_ACTIVE } else { TITLEBAR_COLOR };
        let bd_color = if is_focused { BORDER_COLOR } else { BORDER_INACTIVE };
        let icon_col = if is_focused { BTN_ICON_ACTIVE } else { BTN_ICON_COLOR };

        // Title bar
        draw_rect(state, wx, wy, chrome_w, TITLEBAR_H, tb_color);

        if is_focused {
            // Accent line below title bar
            draw_rect(state, wx, wy + TITLEBAR_H - 2, chrome_w, 2, TITLEBAR_ACCENT);
        }

        // --- Controls (Right-aligned) ---
        let mut btn_x = wx + chrome_w.saturating_sub(BTN_SIZE + BTN_SPACING);
        let btn_y = wy + (TITLEBAR_H.saturating_sub(BTN_SIZE)) / 2;

        // Close button (no background, orange X)
        draw_rect(state, btn_x, btn_y, BTN_SIZE, BTN_SIZE, tb_color);
        let cx = btn_x as i32 + (BTN_SIZE as i32) / 2;
        let cy = btn_y as i32 + (BTN_SIZE as i32) / 2;
        let csize = 4;
        for i in -csize..=csize {
            draw_rect(state, (cx + i) as u32, (cy + i) as u32, 2, 2, BORDER_COLOR);
            draw_rect(state, (cx + i) as u32, (cy - i) as u32, 2, 2, BORDER_COLOR);
        }

        btn_x = btn_x.saturating_sub(BTN_SIZE + BTN_SPACING);

        // Maximize/Restore button
        draw_rect(state, btn_x, btn_y, BTN_SIZE, BTN_SIZE, tb_color); // Transparentish bg
        let cx = btn_x as i32 + (BTN_SIZE as i32) / 2;
        let cy = btn_y as i32 + (BTN_SIZE as i32) / 2;
        if maximized {
            // Restore: <->
            draw_rect(state, (cx - 4) as u32, (cy) as u32, 8, 1, icon_col);
            draw_rect(state, (cx - 4) as u32, (cy - 2) as u32, 2, 5, icon_col);
            draw_rect(state, (cx + 3) as u32, (cy - 2) as u32, 2, 5, icon_col);
        } else {
            // Maximize: >-<
            draw_rect(state, (cx - 4) as u32, (cy) as u32, 8, 1, icon_col);
            draw_rect(state, (cx - 2) as u32, (cy - 3) as u32, 4, 1, icon_col);
            draw_rect(state, (cx - 2) as u32, (cy + 2) as u32, 4, 1, icon_col);
        }

        btn_x = btn_x.saturating_sub(BTN_SIZE + BTN_SPACING);

        // Minimize button (V shape)
        draw_rect(state, btn_x, btn_y, BTN_SIZE, BTN_SIZE, tb_color);
        let cx = btn_x as i32 + (BTN_SIZE as i32) / 2;
        let cy = btn_y as i32 + (BTN_SIZE as i32) / 2;
        for i in 0..4 {
            draw_rect(state, (cx - 3 + i) as u32, (cy - 1 + i) as u32, 2, 2, icon_col);
            draw_rect(state, (cx + 3 - i) as u32, (cy - 1 + i) as u32, 2, 2, icon_col);
        }

        btn_x = btn_x.saturating_sub(BTN_SIZE + BTN_SPACING);

        // Keep On Top button (circle with dot)
        draw_rect(state, btn_x, btn_y, BTN_SIZE, BTN_SIZE, tb_color);
        let cx = btn_x as i32 + (BTN_SIZE as i32) / 2;
        let cy = btn_y as i32 + (BTN_SIZE as i32) / 2;
        // Outer ring (approximate)
        draw_rect(state, (cx - 3) as u32, (cy - 4) as u32, 6, 1, icon_col);
        draw_rect(state, (cx - 3) as u32, (cy + 3) as u32, 6, 1, icon_col);
        draw_rect(state, (cx - 4) as u32, (cy - 3) as u32, 1, 6, icon_col);
        draw_rect(state, (cx + 3) as u32, (cy - 3) as u32, 1, 6, icon_col);
        
        let is_on_top = win.config.z_index_type == ZIndexType::OnTop;
        let dot_color = if is_on_top { TITLEBAR_ACCENT } else { icon_col };
        // Inner dot
        draw_rect(state, (cx - 1) as u32, (cy - 1) as u32, 2, 2, dot_color);


        // Title text (Centered in remaining space)
        let avail_w = chrome_w.saturating_sub((BTN_SIZE + BTN_SPACING) * 4);
        draw_title(state, &win.config.title, wx, wy, avail_w);

        // Borders (top, left, right, bottom)
        draw_rect(state, wx, wy, chrome_w, BORDER_W, bd_color);
        draw_rect(state, wx,                       wy,  BORDER_W, chrome_h, bd_color);
        draw_rect(state, wx + BORDER_W + win.width, wy, BORDER_W, chrome_h, bd_color);
        draw_rect(state, wx, wy + TITLEBAR_H + win.height, chrome_w, BORDER_W, bd_color);
    }

    // Blit client buffer.
    if canvas_x >= state.fb_width || canvas_y >= state.fb_height { return; }
    let copy_w = client_w.min(state.fb_width  - canvas_x) as usize;
    let copy_h = client_h.min(state.fb_height - canvas_y) as usize;
    let stride = fb_stride(state);
    for row in 0..copy_h {
        unsafe {
            let src = win.buffer.add(row * win.width as usize);
            let dst = state.fb.add((canvas_y as usize + row) * stride + canvas_x as usize);
            core::ptr::copy_nonoverlapping(src, dst, copy_w);
        }
    }
}

fn draw_cursor(state: &CompositorState) {
    let bitmap = cursor_bitmap(state.active_cursor);
    let base_x = state.mouse_x as i32;
    let base_y = state.mouse_y as i32;
    let stride = fb_stride(state);

    const SHADOW: u32 = 0x00000000;
    const FG:     u32 = 0x00F5F5F5;

    for (row, &mask) in bitmap.iter().enumerate() {
        for col in 0..CURSOR_W {
            if (mask & (1 << (7 - col))) == 0 { continue; }
            let x = base_x + col as i32;
            let y = base_y + row as i32;
            if x < 0 || y < 0 || x >= state.fb_width as i32 || y >= state.fb_height as i32 { continue; }
            // Outermost ring pixel = shadow for contrast against any background.
            let color = if col == CURSOR_W - 1 || row == CURSOR_H - 1 { SHADOW } else { FG };
            unsafe { state.fb.add(y as usize * stride + x as usize).write(color); }
        }
    }
}

fn redraw_scene(state: &CompositorState) {
    if !state.session_active {
        return;
    }
    clear_framebuffer(state);
    let last_idx = state.windows.len().saturating_sub(1);
    for (i, win) in state.windows.iter().enumerate() {
        let is_focused = i == last_idx;
        composite_window(state, win, is_focused);
    }
    draw_cursor(state);
}

// ---------------------------------------------------------------------------
// Debug helpers
// ---------------------------------------------------------------------------

fn debug_hex(val: u32) {
    let mut buf = [0u8; 10];
    buf[0] = b'0'; buf[1] = b'x';
    let hex = b"0123456789ABCDEF";
    for i in 0..8 { buf[2 + i] = hex[((val >> (28 - i * 4)) & 0xF) as usize]; }
    unsafe {
        core::arch::asm!("syscall",
            inlateout("rax") 99u64 => _,
            in("rdi") buf.as_ptr() as u64, in("rsi") 10u64,
            lateout("rcx") _, lateout("r11") _, options(nostack));
    }
}

fn debug_dec(val: u32) {
    let mut buf = [0u8; 11];
    let mut n = val;
    let mut i = 11;
    loop {
        i -= 1; buf[i] = b'0' + (n % 10) as u8; n /= 10;
        if n == 0 { break; }
    }
    let len = 11 - i;
    unsafe {
        core::arch::asm!("syscall",
            inlateout("rax") 99u64 => _,
            in("rdi") buf.as_ptr().add(i) as u64, in("rsi") len as u64,
            lateout("rcx") _, lateout("r11") _, options(nostack));
    }
}

#[allow(dead_code)]
fn debug_i32(val: i32) {
    if val < 0 { debug_log("-"); debug_dec((-val) as u32); } else { debug_dec(val as u32); }
}

// ---------------------------------------------------------------------------
// Panic handler
// ---------------------------------------------------------------------------

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    debug_log("[DISPLAY] PANIC\n");
    loop {}
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn _start() -> ! {
    debug_log("[DISPLAY] sunlight-display v2 (window manager) starting\n");

    let my_ep = endpoint_create();
    nameserver_register("display_server", my_ep);
    debug_log("[DISPLAY] registered as display_server\n");

    let (fb_ptr, packed_wh, pitch, bpp) = match sunlight_ipc::map_framebuffer() {
        Some(v) => v,
        None => { debug_log("[DISPLAY] Failed to map framebuffer\n"); loop {} }
    };

    let fb_width  = (packed_wh & 0xffffffff) as u32;
    let fb_height = (packed_wh >> 32) as u32;

    debug_log("[DISPLAY] fb ");
    debug_dec(fb_width); debug_log("x"); debug_dec(fb_height);
    debug_log(" pitch="); debug_hex(pitch as u32);
    debug_log(" bpp="); debug_dec(bpp as u32); debug_log("\n");

    let mut state = CompositorState {
        windows:              Vec::new(),
        mouse_x:              (fb_width  / 2) as u16,
        mouse_y:              (fb_height / 2) as u16,
        pointer:              PointerState::new(fb_width, fb_height),
        mouse_sensitivity_fp: SENS_DEFAULT_FP,
        active_drag:          ActiveDrag::None,
        prev_buttons:         0,
        fb:                   fb_ptr as *mut u32,
        fb_width,
        fb_height,
        fb_pitch:             pitch as u32,
        active_cursor:        CursorShape::Pointer,
        // Display server is spawned into an already-active Desktop session.
        // tty_server sends SESSION_DEACTIVATE if the user switches to TTY.
        session_active:       true,
    };
    redraw_scene(&state);

    let mut next_win_id: u64 = 1;

    loop {
        let msg = ipc_recv(my_ep);

        match msg.label {
            // -------------------------------------------------------------------
            // CREATE_WINDOW
            // words[0] = client_w | client_h<<32
            // words[1] = config_flags
            // words[2] = pid | ppid<<32
            // words[3] = title[0..8]
            // -------------------------------------------------------------------
            SgpMsg::CREATE_WINDOW => {
                let w = (msg.words[0] & 0xffffffff) as u32;
                let h = (msg.words[0] >> 32) as u32;
                let size = (w as usize * h as usize * 4).max(4096);
                let config = WindowConfig::from_ipc_words(&msg.words);

                match sunlight_ipc::shm_create(size, 0) {
                    Ok((_, shm_tok)) => {
                        let our_buf = match sunlight_ipc::shm_map(shm_tok) {
                            Ok(p) => p as *mut u32,
                            Err(_) => core::ptr::null_mut(),
                        };

                        let id = next_win_id;
                        next_win_id += 1;

                        // Initial position: cascade from center.
                        let cascade = ((id.saturating_sub(1)) % 8) as u32 * 28;
                        let win_x   = state.fb_width.saturating_sub(w).saturating_div(2).saturating_add(cascade);
                        let win_y   = (state.fb_height / 4).saturating_sub(h / 2).saturating_add(cascade);

                        debug_log("[DISPLAY] create_window id=");
                        debug_dec(id as u32);
                        debug_log(" pos="); debug_dec(win_x); debug_log("x"); debug_dec(win_y);
                        debug_log(" size="); debug_dec(w); debug_log("x"); debug_dec(h);
                        debug_log("\n");

                        // Raise on-top windows above normal ones.
                        let insert_at = if config.z_index_type == ZIndexType::OnTop {
                            state.windows.len()
                        } else {
                            // Insert before any OnTop windows.
                            state.windows.iter().position(|w| w.config.z_index_type == ZIndexType::OnTop)
                                .unwrap_or(state.windows.len())
                        };

                        state.windows.insert(insert_at, Window {
                            id,
                            _shm_cap: shm_tok,
                            buffer:   our_buf,
                            width:    w,
                            height:   h,
                            x:        win_x,
                            y:        win_y,
                            saved_x:  win_x,
                            saved_y:  win_y,
                            saved_w:  w,
                            saved_h:  h,
                            config,
                            client_cursor: CursorShape::Pointer,
                        });
                        redraw_scene(&state);

                        let client_x = win_x + BORDER_W;
                        let client_y = win_y + TITLEBAR_H;
                        let mut reply = IpcMsg::with_label(SgpMsg::REPLY)
                            .word(0, id)
                            .word(1, size as u64)
                            .word(2, (w * 4) as u64)
                            .word(3, client_x as u64 | ((client_y as u64) << 32));
                        reply.caps[0]  = shm_tok;
                        reply.cap_count = 1;
                        let _ = ipc_reply(reply);
                    }
                    Err(_) => { let _ = ipc_reply(IpcMsg::with_label(0xA1FE)); }
                }
            }

            // -------------------------------------------------------------------
            // CONFIGURE_WINDOW — update title / state / flags post-creation.
            // words[0] = win_id
            // words[1] = config_flags  (0 = no flags change)
            // words[2] = pid|ppid<<32  (0 = no change)
            // words[3] = title[0..8]   (0 = no change)
            // caps[0]  = SHM page with full null-terminated title (optional)
            // -------------------------------------------------------------------
            SgpMsg::CONFIGURE_WINDOW => {
                let win_id = msg.words[0];
                if let Some(win) = state.windows.iter_mut().find(|w| w.id == win_id) {
                    // Update flags if non-zero.
                    if msg.words[1] != 0 {
                        let flags = msg.words[1];
                        win.config.window_type = match (flags & 0x3) as u8 {
                            1 => WindowType::Dialog, 2 => WindowType::Desktop,
                            3 => WindowType::Widget, _ => WindowType::Normal,
                        };
                        let new_state = match ((flags >> 2) & 0x3) as u8 {
                            1 => WindowState::Minimized,
                            2 => WindowState::Maximized,
                            3 => WindowState::Fullscreen,
                            _ => WindowState::Normal,
                        };
                        // Save geometry before entering maximized/fullscreen.
                        if new_state == WindowState::Maximized || new_state == WindowState::Fullscreen {
                            if win.config.state == WindowState::Normal {
                                win.saved_x = win.x;
                                win.saved_y = win.y;
                                win.saved_w = win.width;
                                win.saved_h = win.height;
                            }
                        } else if new_state == WindowState::Normal {
                            // Restore saved geometry.
                            win.x      = win.saved_x;
                            win.y      = win.saved_y;
                            win.width  = win.saved_w;
                            win.height = win.saved_h;
                        }
                        win.config.state       = new_state;
                        win.config.border      = if (flags >> 4) & 1 != 0 { BorderStyle::None } else { BorderStyle::Full };
                        win.config.z_index_type = if (flags >> 5) & 1 != 0 { ZIndexType::OnTop } else { ZIndexType::Normal };
                        let z = ((flags >> 6) & 0x7F) as u8;
                        if z > 0 { win.config.z_index_value = z.min(100); }
                        win.config.show_type = match ((flags >> 13) & 0x3) as u8 {
                            1 => ShowType::Tiled, 2 => ShowType::Scrolling, _ => ShowType::Floating,
                        };
                        win.config.group_type = match ((flags >> 15) & 0x3) as u8 {
                            1 => GroupType::Stacked, 2 => GroupType::Tabbed, _ => GroupType::None,
                        };
                    }
                    // Update title from words[3] inline bytes.
                    if msg.words[3] != 0 {
                        for i in 0..8usize {
                            win.config.title[i] = ((msg.words[3] >> (i * 8)) & 0xFF) as u8;
                        }
                    }
                    // Update title from SHM cap if provided.
                    if msg.cap_count > 0 && msg.caps[0] != CapabilityToken::INVALID {
                        if let Ok(p) = sunlight_ipc::shm_map(msg.caps[0]) {
                            win.config.apply_shm_title(p as *const u8, 4096);
                        }
                    }
                }
                redraw_scene(&state);
                let _ = ipc_reply(IpcMsg::with_label(SgpMsg::REPLY));
            }

            // -------------------------------------------------------------------
            // SET_CURSOR — client declares preferred cursor for its client area.
            // words[0] = (win_id as u32) | ((CursorShape discriminant as u32) << 32)
            // -------------------------------------------------------------------
            SgpMsg::SET_CURSOR => {
                let win_id      = msg.words[0] & 0xFFFF_FFFF;
                let shape_byte  = ((msg.words[0] >> 32) & 0xFF) as u8;
                if let Some(win) = state.windows.iter_mut().find(|w| w.id == win_id) {
                    win.client_cursor = CursorShape::from_u8(shape_byte);
                }
                // Re-evaluate active cursor immediately.
                state.active_cursor = cursor_for_scene(&state);
                redraw_scene(&state);
                let _ = ipc_reply(IpcMsg::with_label(SgpMsg::REPLY));
            }

            // -------------------------------------------------------------------
            // COMMIT_FRAME — client finished drawing, ask compositor to re-blit.
            // -------------------------------------------------------------------
            SgpMsg::COMMIT_FRAME => {
                let win_id = msg.words[0];
                if state.windows.iter().any(|w| w.id == win_id) {
                    redraw_scene(&state);
                }
                let _ = ipc_reply(IpcMsg::with_label(SgpMsg::REPLY));
            }

            // -------------------------------------------------------------------
            // EVENT_POLL — client polls mouse position + its current client origin.
            // -------------------------------------------------------------------
            SgpMsg::EVENT_POLL => {
                let win_id = msg.words[0];
                let packed = (state.mouse_x as u64) | ((state.mouse_y as u64) << 16);
                let mut wake = IpcMsg::with_label(SgpMsg::REPLY).word(0, packed);
                if let Some(win) = state.windows.iter().find(|w| w.id == win_id) {
                    let (cx, cy) = match win.config.state {
                        WindowState::Fullscreen => (0u64, 0u64),
                        WindowState::Maximized  => (BORDER_W as u64, TITLEBAR_H as u64),
                        _                       => ((win.x + BORDER_W) as u64, (win.y + TITLEBAR_H) as u64),
                    };
                    wake = wake.word(1, cx | (cy << 32));
                }
                let _ = ipc_reply(wake);
            }

            // -------------------------------------------------------------------
            // DESTROY_WINDOW
            // -------------------------------------------------------------------
            SgpMsg::DESTROY_WINDOW => {
                let win_id = msg.words[0];
                state.windows.retain(|w| w.id != win_id);
                // Cancel any drag on the destroyed window.
                let cancel = match &state.active_drag {
                    ActiveDrag::Move(d)   => d.window_id == win_id,
                    ActiveDrag::Resize(d) => d.window_id == win_id,
                    ActiveDrag::None      => false,
                };
                if cancel { state.active_drag = ActiveDrag::None; }
                state.active_cursor = cursor_for_scene(&state);
                redraw_scene(&state);
                let _ = ipc_reply(IpcMsg::with_label(SgpMsg::REPLY));
            }

            // -------------------------------------------------------------------
            // KEY_EVENT — Global keyboard interceptor.
            // Ctrl + W: close currently active (focused) window.
            // -------------------------------------------------------------------
            sunlight_ipc::KbdMsg::KEY_EVENT => {
                let (keycode, pressed, _, ctrl, _, _) =
                    sunlight_ipc::unpack_key_event(msg.words[0]);

                if pressed && ctrl && keycode == 0x11 { // 0x11 is 'W' in scancode set 1
                    if let Some(focused) = state.windows.last().map(|w| w.id) {
                        state.windows.retain(|w| w.id != focused);
                        
                        // Cancel any active drag on the closed window
                        let cancel = match &state.active_drag {
                            ActiveDrag::Move(d)   => d.window_id == focused,
                            ActiveDrag::Resize(d) => d.window_id == focused,
                            ActiveDrag::None      => false,
                        };
                        if cancel { state.active_drag = ActiveDrag::None; }
                        
                        state.active_cursor = cursor_for_scene(&state);
                        redraw_scene(&state);
                    }
                }
                let _ = ipc_reply(IpcMsg::with_label(SgpMsg::REPLY));
            }

            // -------------------------------------------------------------------
            // RAW_MOTION — mouse driver delta + button state.
            // words[0] = dx(i16) | dy(i16)<<16 | buttons(u8)<<32
            // -------------------------------------------------------------------
            MouseMsg::RAW_MOTION => {
                let raw      = msg.words[0];
                let dx       =  ((raw & 0xFFFF) as i16) as i32;
                let dy       = -((((raw >> 16) & 0xFFFF) as i16) as i32); // PS/2 Y inverted
                let buttons  = ((raw >> 32) & 0xFF) as u8;

                let prev_cx      = state.pointer.x() as u32;
                let prev_cy      = state.pointer.y() as u32;
                let prev_buttons = state.prev_buttons;
                let left_down    = (buttons     & 1) != 0;
                let was_left_down= (prev_buttons & 1) != 0;

                state.pointer.sensitivity_fp = state.mouse_sensitivity_fp;
                state.pointer.apply_motion(dx, dy, buttons);
                state.pointer.sync_clamp();

                state.mouse_x = state.pointer.x() as u16;
                state.mouse_y = state.pointer.y() as u16;

                let cx = state.pointer.x() as u32;
                let cy = state.pointer.y() as u32;

                // ── Left button just pressed ────────────────────────────────
                if state.session_active && left_down && !was_left_down {
                    // Hit-test windows front-to-back to find what was clicked.
                    let mut hit_id:   Option<u64> = None;
                    let mut hit_zone: HitZone     = HitZone::Miss;

                    for win in state.windows.iter().rev() {
                        if win.config.state == WindowState::Minimized { continue; }
                        let zone = hit_test_window(win, cx, cy, state.fb_width, state.fb_height);
                        if zone != HitZone::Miss {
                            hit_id   = Some(win.id);
                            hit_zone = zone;
                            break;
                        }
                    }

                    if let Some(id) = hit_id {
                        // Raise to front (unless it's a Desktop/Widget type).
                        let win_type = state.windows.iter().find(|w| w.id == id)
                            .map(|w| w.config.window_type).unwrap_or(WindowType::Normal);
                        if win_type != WindowType::Desktop && win_type != WindowType::Widget {
                            if let Some(pos) = state.windows.iter().position(|w| w.id == id) {
                                // Keep OnTop windows at the very end; raise normal windows
                                // to just before the first OnTop window.
                                let win = state.windows.remove(pos);
                                let target = if win.config.z_index_type == ZIndexType::OnTop {
                                    state.windows.len()
                                } else {
                                    state.windows.iter().position(|w| w.config.z_index_type == ZIndexType::OnTop)
                                        .unwrap_or(state.windows.len())
                                };
                                state.windows.insert(target, win);
                            }
                        }

                        match hit_zone {
                            HitZone::TitleBar => {
                                state.active_drag = ActiveDrag::Move(MoveDrag { window_id: id });
                            }
                            HitZone::CloseBtn => {
                                // Destroy the window immediately on close button click.
                                state.windows.retain(|w| w.id != id);
                                state.active_drag = ActiveDrag::None;
                            }
                            HitZone::MaximizeBtn => {
                                if let Some(win) = state.windows.iter_mut().find(|w| w.id == id) {
                                    if win.config.state == WindowState::Normal {
                                        win.saved_x = win.x;
                                        win.saved_y = win.y;
                                        win.saved_w = win.width;
                                        win.saved_h = win.height;
                                        win.config.state = WindowState::Maximized;
                                    } else if win.config.state == WindowState::Maximized {
                                        win.x      = win.saved_x;
                                        win.y      = win.saved_y;
                                        win.width  = win.saved_w;
                                        win.height = win.saved_h;
                                        win.config.state = WindowState::Normal;
                                    }
                                }
                                state.active_drag = ActiveDrag::None;
                            }
                            HitZone::MinimizeBtn => {
                                if let Some(win) = state.windows.iter_mut().find(|w| w.id == id) {
                                    win.config.state = WindowState::Minimized;
                                }
                                state.active_drag = ActiveDrag::None;
                            }
                            HitZone::KeepOnTopBtn => {
                                if let Some(win) = state.windows.iter_mut().find(|w| w.id == id) {
                                    win.config.z_index_type = if win.config.z_index_type == ZIndexType::OnTop {
                                        ZIndexType::Normal
                                    } else {
                                        ZIndexType::OnTop
                                    };
                                }
                                // Sorting logic is handled on next click, but we could enforce it here too
                                state.active_drag = ActiveDrag::None;
                            }
                            edge @ (HitZone::EdgeLeft | HitZone::EdgeRight |
                                    HitZone::EdgeBottom |
                                    HitZone::CornerBL | HitZone::CornerBR) => {
                                if let Some(win) = state.windows.iter().find(|w| w.id == id) {
                                    let re = match edge {
                                        HitZone::EdgeLeft   => ResizeEdge::Left,
                                        HitZone::EdgeRight  => ResizeEdge::Right,
                                        HitZone::EdgeBottom => ResizeEdge::Bottom,
                                        HitZone::CornerBL   => ResizeEdge::CornerBL,
                                        _                   => ResizeEdge::CornerBR,
                                    };
                                    state.active_drag = ActiveDrag::Resize(ResizeDrag {
                                        window_id:  id,
                                        edge:       re,
                                        anchor_mx:  cx as i32,
                                        anchor_my:  cy as i32,
                                        anchor_wx:  win.x as i32,
                                        anchor_wy:  win.y as i32,
                                        anchor_ww:  win.width  as i32,
                                        anchor_wh:  win.height as i32,
                                    });
                                }
                            }
                            _ => {}
                        }
                    }
                }

                // ── Left button released ─────────────────────────────────────
                if !left_down {
                    state.active_drag = ActiveDrag::None;
                }

                // ── Drag / resize in progress ───────────────────────────────
                if left_down {
                    let dcx = cx as i32 - prev_cx as i32;
                    let dcy = cy as i32 - prev_cy as i32;

                    match &state.active_drag {
                        ActiveDrag::Move(d) => {
                            let drag_id = d.window_id;
                            if let Some(win) = state.windows.iter_mut().find(|w| w.id == drag_id) {
                                if win.config.state == WindowState::Normal {
                                    win.x = (win.x as i32 + dcx).max(0) as u32;
                                    win.y = (win.y as i32 + dcy).max(0) as u32;
                                }
                            }
                        }
                        ActiveDrag::Resize(d) => {
                            let drag_id  = d.window_id;
                            let edge     = d.edge;
                            let amx      = d.anchor_mx;
                            let amy      = d.anchor_my;
                            let awx      = d.anchor_wx;
                            let _awy     = d.anchor_wy; // reserved for future top-edge resize
                            let aww      = d.anchor_ww;
                            let awh      = d.anchor_wh;

                            if let Some(win) = state.windows.iter_mut().find(|w| w.id == drag_id) {
                                if win.config.state != WindowState::Normal { /* no resize when maximized */ }
                                else {
                                    let total_dx = cx as i32 - amx;
                                    let total_dy = cy as i32 - amy;
                                    match edge {
                                        ResizeEdge::Right => {
                                            win.width = (aww + total_dx).max(MIN_WIN_W as i32) as u32;
                                        }
                                        ResizeEdge::Bottom => {
                                            win.height = (awh + total_dy).max(MIN_WIN_H as i32) as u32;
                                        }
                                        ResizeEdge::Left => {
                                            let new_w = (aww - total_dx).max(MIN_WIN_W as i32) as u32;
                                            let dx    = aww as u32 - new_w;
                                            win.x     = (awx + dx as i32).max(0) as u32;
                                            win.width = new_w;
                                        }
                                        ResizeEdge::CornerBR => {
                                            win.width  = (aww + total_dx).max(MIN_WIN_W as i32) as u32;
                                            win.height = (awh + total_dy).max(MIN_WIN_H as i32) as u32;
                                        }
                                        ResizeEdge::CornerBL => {
                                            let new_w = (aww - total_dx).max(MIN_WIN_W as i32) as u32;
                                            let dx    = aww as u32 - new_w;
                                            win.x     = (awx + dx as i32).max(0) as u32;
                                            win.width = new_w;
                                            win.height = (awh + total_dy).max(MIN_WIN_H as i32) as u32;
                                        }
                                    }
                                }
                            }
                        }
                        ActiveDrag::None => {}
                    }
                }

                state.prev_buttons  = buttons;
                state.active_cursor = cursor_for_scene(&state);
                redraw_scene(&state);
                let _ = ipc_reply(IpcMsg::with_label(SgpMsg::REPLY));
            }

            // -------------------------------------------------------------------
            // SESSION_ACTIVATE — tty_server hands framebuffer to Desktop session.
            // -------------------------------------------------------------------
            SgpMsg::SESSION_ACTIVATE => {
                if !state.session_active {
                    debug_log("[DISPLAY] [SESSION] activated — Desktop owns framebuffer\n");
                    state.session_active = true;
                    redraw_scene(&state);
                }
                let _ = ipc_reply(IpcMsg::with_label(SgpMsg::REPLY));
            }

            // -------------------------------------------------------------------
            // SESSION_DEACTIVATE — tty_server takes framebuffer for TTY/Login.
            // -------------------------------------------------------------------
            SgpMsg::SESSION_DEACTIVATE => {
                if state.session_active {
                    debug_log("[DISPLAY] [SESSION] deactivated — TTY owns framebuffer\n");
                    state.session_active = false;
                }
                let _ = ipc_reply(IpcMsg::with_label(SgpMsg::REPLY));
            }

            _ => { let _ = ipc_reply(IpcMsg::with_label(SgpMsg::REPLY)); }
        }
    }
}
