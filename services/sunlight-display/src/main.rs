#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;
use sunlight_libc as libc;

use sunlight_ipc::debug_log;
use sunlight_ipc::{
    endpoint_create, ipc_recv, ipc_reply, nameserver_register, sgp::SgpMsg, CapabilityToken,
    IpcMsg, MouseMsg,
};
use sunlight_ui::image::TgaImage;
use sunlight_ui::{Canvas, Color, Rect, UiSymbol};

/// Wallpaper asset staged at /var/sunlightos/wallpapers/wallpaper.tga.
/// Embedded directly so the compositor can decode without a VFS read at startup.
static WALLPAPER_TGA_BYTES: &[u8] = include_bytes!("../../../docs/images/wallpaper.tga");

mod backend;
mod blend;
mod dirty;
mod mask;

// ---------------------------------------------------------------------------
// Allocator
// ---------------------------------------------------------------------------

struct BumpAllocator;
unsafe impl core::alloc::GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        static mut HEAP: [u8; 16 * 1024 * 1024] = [0; 16 * 1024 * 1024];
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

fn launch_runner() {
    let _ = libc::spawn(b"/bin/sunlight-runner", &[b"sunlight-runner"], None);
}

fn launch_vortex_shell() -> bool {
    match libc::spawn(
        b"/bin/sunlight-vortex-shell",
        &[b"sunlight-vortex-shell"],
        None,
    ) {
        Ok(_pid) => {
            debug_log("[DISPLAY] launched Vortex Shell\n");
            true
        }
        Err(_) => {
            debug_log("[DISPLAY] failed to launch Vortex Shell\n");
            false
        }
    }
}

// ---------------------------------------------------------------------------
// Fixed-point pointer motion
// ---------------------------------------------------------------------------

const FP_SHIFT: i32 = 16;
const FP_ONE: i32 = 1 << FP_SHIFT;

/// Set to true to log every RAW_MOTION packet (dx/dy, cursor before/after).
const MOUSE_DEBUG: bool = false;

const SENS_DEFAULT_FP: i32 = FP_ONE * 2;
#[allow(dead_code)]
const SENS_MIN_FP: i32 = FP_ONE / 2;
#[allow(dead_code)]
const SENS_MAX_FP: i32 = FP_ONE * 4;

const ACCEL_PRECISE_MAX_DELTA: i32 = 2;
const ACCEL_NORMAL_MAX_DELTA: i32 = 6;
const ACCEL_PRECISE_GAIN_FP: i32 = FP_ONE;
const ACCEL_NORMAL_GAIN_FP: i32 = FP_ONE * 3 / 2;
const ACCEL_FAST_GAIN_FP: i32 = FP_ONE * 9 / 4;
/// On a button press (up->down edge) the pointer is held still for this many
/// motion packets so the cursor stays put while the click lands.
const CLICK_FREEZE_EVENTS: u32 = 4;
const EDGE_MARGIN: i32 = 0;
const COUNTER_LOG_INTERVAL: u64 = 64;
const KEY_TAB: u8 = 0x0F;
const KEY_R: u8 = 0x13;
const KEY_W: u8 = 0x11;
const KEY_SPACE: u8 = 0x39;

// ---------------------------------------------------------------------------
// Window property enums
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
#[repr(u8)]
enum WindowType {
    Normal = 0,
    Dialog = 1,
    Desktop = 2,
    Widget = 3,
}

#[derive(Clone, Copy, PartialEq)]
#[repr(u8)]
enum WindowState {
    Normal = 0,
    Minimized = 1,
    Maximized = 2,
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
    OnTop = 1,
}

#[derive(Clone, Copy, PartialEq)]
#[repr(u8)]
enum ShowType {
    Floating = 0,
    Tiled = 1,
    Scrolling = 2,
}

#[derive(Clone, Copy, PartialEq)]
#[repr(u8)]
enum GroupType {
    None = 0,
    Stacked = 1,
    Tabbed = 2,
}

/// Cursor shapes the compositor can render. Clients request via SGP_SET_CURSOR.
#[derive(Clone, Copy, PartialEq)]
#[repr(u8)]
enum CursorShape {
    Pointer = 0,
    Hand = 1,
    ResizeH = 2,
    ResizeV = 3,
    ResizeCornerNW = 4,
    ResizeCornerNE = 5,
    Moving = 6,
    Waiting = 7,
    Question = 8,
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
    title: [u8; 64],
    window_type: WindowType,
    state: WindowState,
    border: BorderStyle,
    z_index_type: ZIndexType,
    z_index_value: u8,
    show_type: ShowType,
    group_type: GroupType,
    pid: u32,
    ppid: u32,
    group_ids: [u32; 4],
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
            window_type: WindowType::Normal,
            state: WindowState::Normal,
            border: BorderStyle::Full,
            z_index_type: ZIndexType::Normal,
            z_index_value: 50,
            show_type: ShowType::Floating,
            group_type: GroupType::None,
            pid: 0,
            ppid: 0,
            group_ids: [0; 4],
            group_id_count: 0,
        }
    }

    fn from_ipc_words(words: &[u64; 8]) -> Self {
        let flags = words[1];
        let pid = (words[2] & 0xFFFF_FFFF) as u32;
        let ppid = (words[2] >> 32) as u32;

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
        let border = if (flags >> 4) & 1 != 0 {
            BorderStyle::None
        } else {
            BorderStyle::Full
        };
        let z_index_type = if (flags >> 5) & 1 != 0 {
            ZIndexType::OnTop
        } else {
            ZIndexType::Normal
        };
        let z_raw = ((flags >> 6) & 0x7F) as u8;
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
        if shm_ptr.is_null() {
            return;
        }
        let max = shm_len.min(63);
        for i in 0..max {
            let b = unsafe { shm_ptr.add(i).read() };
            if b == 0 {
                self.title[i] = 0;
                break;
            }
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
    window_id: u64,
    edge: ResizeEdge,
    // Mouse and window geometry at drag start
    anchor_mx: i32,
    anchor_my: i32,
    anchor_wx: i32,
    anchor_wy: i32,
    anchor_ww: i32,
    anchor_wh: i32,
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
            HitZone::CloseBtn
            | HitZone::MaximizeBtn
            | HitZone::MinimizeBtn
            | HitZone::KeepOnTopBtn => CursorShape::Pointer,
            HitZone::EdgeLeft | HitZone::EdgeRight => CursorShape::ResizeH,
            HitZone::EdgeBottom => CursorShape::ResizeV,
            HitZone::CornerBL => CursorShape::ResizeCornerNW,
            HitZone::CornerBR => CursorShape::ResizeCornerNE,
        }
    }
}

// ---------------------------------------------------------------------------
// PointerState (unchanged from Phase 1)
// ---------------------------------------------------------------------------

struct PointerState {
    x_fp: i32,
    y_fp: i32,
    buttons: u8,
    fb_width: u32,
    fb_height: u32,
    sensitivity_fp: i32,
    /// Remaining motion packets to suppress after a click edge (stabilization).
    freeze_events: u32,
}

impl PointerState {
    fn new(fb_w: u32, fb_h: u32) -> Self {
        let cx = ((fb_w as i32 / 2).max(0)) << FP_SHIFT;
        let cy = ((fb_h as i32 / 2).max(0)) << FP_SHIFT;
        Self {
            x_fp: cx,
            y_fp: cy,
            buttons: 0,
            fb_width: fb_w,
            fb_height: fb_h,
            sensitivity_fp: SENS_DEFAULT_FP,
            freeze_events: 0,
        }
    }

    fn apply_motion(&mut self, dx: i32, dy: i32, buttons: u8) {
        // Detect a button press edge (any button newly pressed) and start a
        // short freeze so the cursor doesn't drift while the click registers.
        let pressed_edge = buttons & !self.buttons;
        if pressed_edge != 0 {
            self.freeze_events = CLICK_FREEZE_EVENTS;
        }
        if self.freeze_events > 0 {
            self.freeze_events -= 1;
            self.buttons = buttons;
            return;
        }

        let speed = dx.abs().max(dy.abs());
        let accel_fp = if speed <= ACCEL_PRECISE_MAX_DELTA {
            ACCEL_PRECISE_GAIN_FP
        } else if speed <= ACCEL_NORMAL_MAX_DELTA {
            ACCEL_NORMAL_GAIN_FP
        } else {
            ACCEL_FAST_GAIN_FP
        } as i64;
        let gain_fp = ((self.sensitivity_fp as i64) * accel_fp) >> FP_SHIFT;
        self.x_fp += (dx as i64 * gain_fp) as i32;
        self.y_fp += (dy as i64 * gain_fp) as i32;
        self.sync_clamp();
        self.buttons = buttons;
    }

    fn min_fp(&self) -> i32 {
        EDGE_MARGIN << FP_SHIFT
    }
    fn max_x_fp(&self) -> i32 {
        ((self.fb_width as i32 - 1 - EDGE_MARGIN).max(EDGE_MARGIN)) << FP_SHIFT
    }
    fn max_y_fp(&self) -> i32 {
        ((self.fb_height as i32 - 1 - EDGE_MARGIN).max(EDGE_MARGIN)) << FP_SHIFT
    }
    fn sync_clamp(&mut self) {
        self.x_fp = self.x_fp.clamp(self.min_fp(), self.max_x_fp());
        self.y_fp = self.y_fp.clamp(self.min_fp(), self.max_y_fp());
    }

    fn x(&self) -> i32 {
        (self.x_fp >> FP_SHIFT)
            .max(0)
            .min((self.fb_width - 1) as i32)
    }
    fn y(&self) -> i32 {
        (self.y_fp >> FP_SHIFT)
            .max(0)
            .min((self.fb_height - 1) as i32)
    }
}

// ---------------------------------------------------------------------------
// Window
// ---------------------------------------------------------------------------

struct Window {
    id: u64,
    shm_cap: CapabilityToken,
    buffer: *mut u32,
    width: u32,  // current client area width
    height: u32, // current client area height
    x: u32,      // chrome top-left on screen
    y: u32,
    // Saved normal geometry for restore from maximized/fullscreen.
    saved_x: u32,
    saved_y: u32,
    saved_w: u32,
    saved_h: u32,
    owner_pid: u64,
    config: WindowConfig,
    /// Cursor shape the client wants when the pointer is in its client area.
    client_cursor: CursorShape,
    pending_keys: KeyEventQueue,
}

impl Window {
    /// Returns the chrome rectangle (x, y, w, h) accounting for state.
    fn chrome_rect(&self, fb_w: u32, fb_h: u32) -> (u32, u32, u32, u32) {
        match self.config.state {
            WindowState::Fullscreen => (0, 0, fb_w, fb_h),
            WindowState::Maximized => (0, 0, fb_w, fb_h),
            _ => (
                self.x,
                self.y,
                self.width + BORDER_W * 2,
                TITLEBAR_H + self.height + BORDER_W,
            ),
        }
    }
}

const KEY_EVENT_QUEUE_CAP: usize = 32;

#[derive(Clone, Copy)]
struct KeyEventQueue {
    buf: [u64; KEY_EVENT_QUEUE_CAP],
    head: usize,
    len: usize,
}

impl KeyEventQueue {
    const fn new() -> Self {
        Self {
            buf: [0; KEY_EVENT_QUEUE_CAP],
            head: 0,
            len: 0,
        }
    }

    fn push(&mut self, event: u64) {
        if self.len == KEY_EVENT_QUEUE_CAP {
            self.head = (self.head + 1) % KEY_EVENT_QUEUE_CAP;
            self.len -= 1;
        }
        let tail = (self.head + self.len) % KEY_EVENT_QUEUE_CAP;
        self.buf[tail] = event;
        self.len += 1;
    }

    fn pop(&mut self) -> Option<u64> {
        if self.len == 0 {
            return None;
        }
        let event = self.buf[self.head];
        self.head = (self.head + 1) % KEY_EVENT_QUEUE_CAP;
        self.len -= 1;
        Some(event)
    }
}

// ---------------------------------------------------------------------------
// CompositorState
// ---------------------------------------------------------------------------

struct CompositorState {
    windows: Vec<Window>,
    mouse_x: u16,
    mouse_y: u16,
    pointer: PointerState,
    mouse_sensitivity_fp: i32,
    active_drag: ActiveDrag,
    prev_buttons: u8,
    fb: *mut u32,
    fb_width: u32,
    fb_height: u32,
    fb_pitch: u32,
    back_buffer: Vec<u32>,
    /// Display backend: Limine memcpy or VirtIO GPU flush.
    display_backend: backend::DisplayBackend,
    /// When true, the hardware cursor is active (VirtIO GPU backend) and
    /// cursor moves do not repaint the back_buffer.
    hw_cursor_active: bool,
    /// Last cursor shape that was uploaded to the GPU (avoid redundant uploads).
    last_hw_cursor_shape: Option<CursorShape>,
    active_cursor: CursorShape,
    /// Saved-under pixels for the software cursor fast path.
    software_cursor: SoftwareCursorState,
    // True when Desktop session owns the framebuffer; false when TTY/login is active.
    // tty_server sends SESSION_ACTIVATE/SESSION_DEACTIVATE to toggle this.
    session_active: bool,
    /// Precomputed anti-aliased corner coverage for the inner chrome radius.
    inner_corner_mask: mask::CornerMask,
    /// Regions that need to be presented to the framebuffer this frame.
    dirty: dirty::DirtyList,
    /// Focused diagnostics for cursor motion vs. redraw behavior.
    debug_counters: DebugCounters,
    /// Decoded-on-demand wallpaper view.  `None` when the asset was absent or
    /// could not be parsed; the compositor falls back to DESKTOP_COLOR fill.
    wallpaper: Option<TgaImage>,
    /// True after spawning Vortex and before a Desktop-layer window appears.
    vortex_launch_pending: bool,
}

#[derive(Clone, Copy)]
struct SoftwareCursorState {
    pixels: [u32; CURSOR_W * CURSOR_H],
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    shape: CursorShape,
    valid: bool,
}

impl SoftwareCursorState {
    const fn new() -> Self {
        Self {
            pixels: [0; CURSOR_W * CURSOR_H],
            x: 0,
            y: 0,
            w: 0,
            h: 0,
            shape: CursorShape::Pointer,
            valid: false,
        }
    }
}

#[derive(Clone, Copy)]
struct DebugCounters {
    mouse_event_count: u64,
    hw_cursor_move_count: u64,
    sw_cursor_move_count: u64,
    sw_cursor_redraw_count: u64,
    desktop_redraw_count: u64,
    dirty_rect_count: u64,
    /// GPU full-screen flushes (VirtIO present_back_buffer).
    full_present_count: u64,
    /// GPU partial-rect flushes (VirtIO present_rect).
    present_rect_count: u64,
    framebuffer_copy_count: u64,
}

impl DebugCounters {
    const fn new() -> Self {
        Self {
            mouse_event_count: 0,
            hw_cursor_move_count: 0,
            sw_cursor_move_count: 0,
            sw_cursor_redraw_count: 0,
            desktop_redraw_count: 0,
            dirty_rect_count: 0,
            full_present_count: 0,
            present_rect_count: 0,
            framebuffer_copy_count: 0,
        }
    }
}

fn fb_stride(state: &CompositorState) -> usize {
    (state.fb_pitch / 4) as usize
}

fn clip_to_screen(r: Rect, w: u32, h: u32) -> Rect {
    let x0 = r.x.max(0);
    let y0 = r.y.max(0);
    let x1 = r.right().min(w as i32);
    let y1 = r.bottom().min(h as i32);
    if x1 <= x0 || y1 <= y0 {
        Rect::new(0, 0, 0, 0)
    } else {
        Rect::new(x0, y0, (x1 - x0) as u32, (y1 - y0) as u32)
    }
}

fn mark_dirty_rect(state: &mut CompositorState, rect: Rect) {
    let clipped = clip_to_screen(rect, state.fb_width, state.fb_height);
    if clipped.w == 0 || clipped.h == 0 {
        return;
    }
    state.debug_counters.dirty_rect_count += 1;
    state.dirty.mark(clipped);
}

fn mark_dirty_full(state: &mut CompositorState) {
    state.debug_counters.dirty_rect_count += 1;
    state.dirty.mark_full();
}

fn is_focusable_window(win: &Window) -> bool {
    win.config.state != WindowState::Minimized
        && win.config.window_type != WindowType::Desktop
        && win.config.window_type != WindowType::Widget
}

fn focused_window_idx(state: &CompositorState) -> Option<usize> {
    state.windows.iter().rposition(is_focusable_window)
}

fn focused_window_id(state: &CompositorState) -> Option<u64> {
    focused_window_idx(state).map(|idx| state.windows[idx].id)
}

fn has_desktop_window(state: &CompositorState) -> bool {
    state
        .windows
        .iter()
        .any(|win| win.config.window_type == WindowType::Desktop)
}

fn ensure_vortex_shell(state: &mut CompositorState) {
    if has_desktop_window(state) || state.vortex_launch_pending {
        return;
    }
    state.vortex_launch_pending = launch_vortex_shell();
}

fn cycle_focus(state: &mut CompositorState) -> bool {
    let Some(focused_idx) = focused_window_idx(state) else {
        return false;
    };
    let z_group = state.windows[focused_idx].config.z_index_type;
    let candidate_count = state
        .windows
        .iter()
        .filter(|win| is_focusable_window(win) && win.config.z_index_type == z_group)
        .count();
    if candidate_count < 2 {
        return false;
    }

    let win = state.windows.remove(focused_idx);
    let insert_idx = state
        .windows
        .iter()
        .position(|other| is_focusable_window(other) && other.config.z_index_type == z_group)
        .unwrap_or(state.windows.len());
    state.windows.insert(insert_idx, win);
    true
}

fn close_window(state: &mut CompositorState, win_id: u64, requester_pid: Option<u64>) -> bool {
    let Some(pos) = state.windows.iter().position(|w| w.id == win_id) else {
        return false;
    };

    if let Some(pid) = requester_pid {
        if state.windows[pos].owner_pid != pid {
            return false;
        }
    }

    let win = state.windows.remove(pos);
    let was_desktop = win.config.window_type == WindowType::Desktop;

    // The display server owns the SHM object created at CREATE_WINDOW time, so
    // normal close must explicitly release that ownership.
    let _ = sunlight_ipc::shm_free(win.shm_cap);
    // TODO: Harden forced-close paths so SHM revocation is coordinated more
    // explicitly with process-exit cleanup and stale clients.

    let cancel = match &state.active_drag {
        ActiveDrag::Move(d) => d.window_id == win_id,
        ActiveDrag::Resize(d) => d.window_id == win_id,
        ActiveDrag::None => false,
    };
    if cancel {
        state.active_drag = ActiveDrag::None;
    }

    state.active_cursor = cursor_for_scene(state);
    if was_desktop {
        state.vortex_launch_pending = false;
        if state.session_active {
            ensure_vortex_shell(state);
        }
    }
    true
}

// ---------------------------------------------------------------------------
// Chrome constants (Vortex Shell Theme)
// ---------------------------------------------------------------------------

// Safe fallback resolution if neither the VirtIO GPU nor the Limine framebuffer
// report a usable size. Used so the compositor never allocates a 0×0 buffer.
const SAFE_FALLBACK_W: u32 = 1280;
const SAFE_FALLBACK_H: u32 = 800;
// Upper sanity bound on any reported dimension. Guards against a garbage VirtIO
// GPU reply (e.g. uninitialized registers) sizing a multi-GB back buffer.
const MAX_DIM: u32 = 16384;

/// Validate a backend-reported `(width, height)`: both must be non-zero and
/// within `MAX_DIM`. Returns `None` for a zero/garbage size so the caller can
/// fall back instead of allocating a bogus buffer.
fn validate_size(w: u32, h: u32) -> Option<(u32, u32)> {
    if w == 0 || h == 0 || w > MAX_DIM || h > MAX_DIM {
        None
    } else {
        Some((w, h))
    }
}

const DESKTOP_COLOR: u32 = 0x00121214; // Deep dark gray/black
const TITLEBAR_H: u32 = 32; // Taller for Chrome-tab style
const TITLEBAR_COLOR: u32 = 0x002B2B36; // inactive titlebar (dark slate)
const TITLEBAR_ACTIVE: u32 = 0x001E1E26; // active window titlebar (darker base)
const TITLEBAR_ACCENT: u32 = 0x00FF7A00; // Warm/Orange accent line
const TITLE_TEXT_COLOR: u32 = 0x00E0E0E0; // Off-white
const BORDER_W: u32 = 2; // Thinner modern border
const BORDER_COLOR: u32 = 0x00FF7A00; // Active window border glow
const BORDER_INACTIVE: u32 = 0x002B2B36; // Inactive window border
const BTN_HOVER_BG: u32 = 0x003A3A4A; // Hover state background for buttons
const BTN_ICON_COLOR: u32 = 0x00B0B0C0; // Icon color
const BTN_ICON_ACTIVE: u32 = 0x00FFFFFF; // Icon color when active/focused
const BTN_SIZE: u32 = 20; // Size of control buttons
const BTN_SPACING: u32 = 4;
const CHROME_RADIUS: u32 = 10;
const CONTROL_RADIUS: u32 = 6;
const RESIZE_BORDER: u32 = 6; // effective hit-test width for edges/corners
const MIN_WIN_W: u32 = 200;
const MIN_WIN_H: u32 = 100;

#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum WindowControlKind {
    Help,
    Pin,
    Minimize,
    Maximize,
    Restore,
    Close,
}

impl WindowControlKind {
    fn symbol(self) -> UiSymbol {
        match self {
            WindowControlKind::Help => UiSymbol::Help,
            WindowControlKind::Pin => UiSymbol::Pin,
            WindowControlKind::Minimize => UiSymbol::Minimize,
            WindowControlKind::Maximize => UiSymbol::Maximize,
            WindowControlKind::Restore => UiSymbol::Restore,
            WindowControlKind::Close => UiSymbol::Close,
        }
    }
}

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
        CursorShape::Pointer => &CURSOR_PTR,
        CursorShape::Hand => &CURSOR_HAND,
        CursorShape::ResizeH => &CURSOR_RESIZE_H,
        CursorShape::ResizeV => &CURSOR_RESIZE_V,
        CursorShape::ResizeCornerNW => &CURSOR_RESIZE_NW,
        CursorShape::ResizeCornerNE => &CURSOR_RESIZE_NE,
        CursorShape::Moving => &CURSOR_MOVE,
        CursorShape::Waiting => &CURSOR_WAIT,
        CursorShape::Question => &CURSOR_QUESTION,
    }
}

// ---------------------------------------------------------------------------
// Drawing primitives
// ---------------------------------------------------------------------------

fn clear_back_buffer(state: &mut CompositorState) {
    if let Some(wp) = state.wallpaper {
        let fw = state.fb_width as usize;
        let fh = state.fb_height as usize;
        let iw = wp.width as usize;
        let ih = wp.height as usize;
        let stride = fb_stride(state);
        for y in 0..fh {
            let src_y = (y * ih / fh) as u32;
            let row_off = y * stride;
            for x in 0..fw {
                let src_x = (x * iw / fw) as u32;
                state.back_buffer[row_off + x] = wp.pixel_xrgb(src_x, src_y);
            }
        }
    } else {
        for pixel in state.back_buffer.iter_mut() {
            *pixel = DESKTOP_COLOR;
        }
    }
}

fn back_buffer_canvas<'a>(state: &CompositorState, pixels: &'a mut [u32]) -> Canvas<'a> {
    Canvas::new(
        pixels,
        fb_stride(state) as u32,
        state.fb_width,
        state.fb_height,
    )
}

fn present_back_buffer(state: &mut CompositorState) {
    match &state.display_backend {
        backend::DisplayBackend::Limine { fb, .. } => {
            state.debug_counters.framebuffer_copy_count += 1;
            unsafe {
                core::ptr::copy_nonoverlapping(
                    state.back_buffer.as_ptr(),
                    *fb,
                    state.back_buffer.len(),
                );
            }
        }
        backend::DisplayBackend::VirtioGpu { width, height } => {
            state.debug_counters.full_present_count += 1;
            sunlight_ipc::gpu_flush(0, 0, *width, *height);
        }
    }
}

/// Blit only the pixels within `r` from the back buffer to the framebuffer.
fn present_rect(state: &mut CompositorState, r: Rect) {
    let x0 = r.x.max(0) as usize;
    let y0 = r.y.max(0) as usize;
    let x1 = (r.right() as usize).min(state.fb_width as usize);
    let y1 = (r.bottom() as usize).min(state.fb_height as usize);
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    match &state.display_backend {
        backend::DisplayBackend::Limine { fb, pitch_words } => {
            state.debug_counters.framebuffer_copy_count += 1;
            let stride = *pitch_words;
            let len = x1 - x0;
            for y in y0..y1 {
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        state.back_buffer.as_ptr().add(y * stride + x0),
                        fb.add(y * stride + x0),
                        len,
                    );
                }
            }
        }
        backend::DisplayBackend::VirtioGpu { .. } => {
            let x = x0 as u32;
            let y = y0 as u32;
            let w = (x1 - x0) as u32;
            let h = (y1 - y0) as u32;
            state.debug_counters.present_rect_count += 1;
            sunlight_ipc::gpu_flush(x, y, w, h);
        }
    }
}

/// Dirty rect that covers the cursor bitmap plus a 1-px border for clean erasure.
fn cursor_dirty_rect(cx: u32, cy: u32) -> Rect {
    Rect::new(
        cx as i32 - 1,
        cy as i32 - 1,
        CURSOR_W as u32 + 2,
        CURSOR_H as u32 + 2,
    )
}

fn cursor_rect(state: &CompositorState, cx: u32, cy: u32) -> Option<Rect> {
    if cx >= state.fb_width || cy >= state.fb_height {
        return None;
    }
    let w = (CURSOR_W as u32).min(state.fb_width - cx);
    let h = (CURSOR_H as u32).min(state.fb_height - cy);
    if w == 0 || h == 0 {
        None
    } else {
        Some(Rect::new(cx as i32, cy as i32, w, h))
    }
}

fn title_len(title: &[u8; 64]) -> usize {
    title.iter().position(|&b| b == 0).unwrap_or(title.len())
}

fn control_rect(wx: u32, wy: u32, chrome_w: u32, slot_from_right: u32) -> Rect {
    let x = wx + chrome_w.saturating_sub((BTN_SIZE + BTN_SPACING) * (slot_from_right + 1));
    let y = wy as i32 + (TITLEBAR_H.saturating_sub(BTN_SIZE)) as i32 / 2;
    Rect::new(x as i32, y, BTN_SIZE, BTN_SIZE)
}

fn draw_title(canvas: &mut Canvas<'_>, title: &[u8; 64], rect: Rect, color: Color) {
    let len = title_len(title);
    if len == 0 {
        return;
    }

    let glyph_stride = 6i32;
    let max_chars = (rect.w as i32 / glyph_stride).max(0) as usize;
    let ty = rect.y + (rect.h as i32 - 7) / 2;
    let mut tx = rect.x;
    for &b in title.iter().take(len.min(max_chars)) {
        tx = canvas.draw_char(tx, ty, b as char, color);
    }
}

fn draw_window_control(
    canvas: &mut Canvas<'_>,
    rect: Rect,
    control: WindowControlKind,
    icon_color: Color,
    chrome_fill: Color,
    hovered: bool,
    accent_active: bool,
) {
    let fill = if hovered && control == WindowControlKind::Close {
        Color(BORDER_COLOR).darken(24)
    } else if hovered {
        Color(BTN_HOVER_BG)
    } else if accent_active {
        chrome_fill.lighten(10)
    } else {
        chrome_fill
    };

    let border = if hovered {
        Color(TITLEBAR_ACCENT)
    } else if accent_active {
        chrome_fill.lighten(24)
    } else {
        Color(BORDER_INACTIVE)
    };

    canvas.fill_rounded_rect_with_border(rect, CONTROL_RADIUS, fill, border, 1);
    canvas.draw_ui_symbol_centered(rect, control.symbol(), icon_color);
}

// ---------------------------------------------------------------------------
// Hit-testing
// ---------------------------------------------------------------------------

fn hit_test_window(win: &Window, cx: u32, cy: u32, fb_w: u32, fb_h: u32) -> HitZone {
    // Determine effective chrome origin / size based on window state.
    let (wx, wy, chrome_w, chrome_h) = win.chrome_rect(fb_w, fb_h);

    let fullscreen = win.config.state == WindowState::Fullscreen;
    let no_border = win.config.border == BorderStyle::None || fullscreen;

    // Completely outside this window?
    if cx < wx || cy < wy || cx >= wx + chrome_w || cy >= wy + chrome_h {
        return HitZone::Miss;
    }

    let rel_x = cx - wx;
    let rel_y = cy - wy;

    // Title bar zone (not present in Fullscreen or no-border Widget/Desktop).
    if !fullscreen && !no_border {
        if rel_y < TITLEBAR_H {
            let close_rect = control_rect(wx, wy, chrome_w, 0);
            let maximize_rect = control_rect(wx, wy, chrome_w, 1);
            let minimize_rect = control_rect(wx, wy, chrome_w, 2);
            let pin_rect = control_rect(wx, wy, chrome_w, 3);
            let point = sunlight_ui::Point::new(cx as i32, cy as i32);

            if close_rect.contains(point) {
                return HitZone::CloseBtn;
            }
            if maximize_rect.contains(point) {
                return HitZone::MaximizeBtn;
            }
            if minimize_rect.contains(point) {
                return HitZone::MinimizeBtn;
            }
            if pin_rect.contains(point) {
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
    if rel_x < RESIZE_BORDER {
        return HitZone::EdgeLeft;
    }
    if rel_x >= chrome_w - RESIZE_BORDER {
        return HitZone::EdgeRight;
    }
    if rel_y >= TITLEBAR_H + win.height {
        return HitZone::EdgeBottom;
    }

    HitZone::ClientArea
}

/// Compute the cursor shape to display given all visible windows and pointer pos.
fn cursor_for_scene(state: &CompositorState) -> CursorShape {
    let cx = state.mouse_x as u32;
    let cy = state.mouse_y as u32;

    // Walk front-to-back (last window in vec = top of z-order).
    for win in state.windows.iter().rev() {
        if win.config.state == WindowState::Minimized {
            continue;
        }
        let zone = hit_test_window(win, cx, cy, state.fb_width, state.fb_height);
        if zone == HitZone::Miss {
            continue;
        }

        return match zone {
            HitZone::ClientArea => win.client_cursor,
            other => other.default_cursor(),
        };
    }
    CursorShape::Pointer
}

// ---------------------------------------------------------------------------
// Compositing
// ---------------------------------------------------------------------------

fn composite_window(
    state: &CompositorState,
    back_buffer: &mut [u32],
    win: &Window,
    is_focused: bool,
) {
    if win.buffer.is_null() {
        return;
    }
    if win.config.state == WindowState::Minimized {
        return;
    }

    let fullscreen = win.config.state == WindowState::Fullscreen;
    let maximized = win.config.state == WindowState::Maximized;
    let no_chrome = fullscreen || win.config.border == BorderStyle::None;

    let (canvas_x, canvas_y, client_w, client_h) = if fullscreen || maximized {
        let cw = if no_chrome {
            state.fb_width
        } else {
            state.fb_width.saturating_sub(BORDER_W * 2)
        };
        let ch = if no_chrome {
            state.fb_height
        } else {
            state.fb_height.saturating_sub(TITLEBAR_H + BORDER_W)
        };
        let ox = if no_chrome { 0 } else { BORDER_W };
        let oy = if no_chrome { 0 } else { TITLEBAR_H };
        (ox, oy, cw, ch)
    } else {
        (win.x + BORDER_W, win.y + TITLEBAR_H, win.width, win.height)
    };

    if !no_chrome {
        let (wx, wy) = if maximized {
            (0u32, 0u32)
        } else {
            (win.x, win.y)
        };
        let chrome_w = if maximized {
            state.fb_width
        } else {
            win.width + BORDER_W * 2
        };
        let chrome_h = if maximized {
            state.fb_height
        } else {
            TITLEBAR_H + win.height + BORDER_W
        };

        let tb_color = if is_focused {
            TITLEBAR_ACTIVE
        } else {
            TITLEBAR_COLOR
        };
        let bd_color = if is_focused {
            BORDER_COLOR
        } else {
            BORDER_INACTIVE
        };
        let icon_col = if is_focused {
            BTN_ICON_ACTIVE
        } else {
            BTN_ICON_COLOR
        };
        let hover_zone = hit_test_window(
            win,
            state.mouse_x as u32,
            state.mouse_y as u32,
            state.fb_width,
            state.fb_height,
        );
        let outer = Rect::new(wx as i32, wy as i32, chrome_w, chrome_h);
        let inner = outer.inset(BORDER_W as i32);
        let outer_radius = if maximized { 0 } else { CHROME_RADIUS };
        let inner_radius = outer_radius.saturating_sub(BORDER_W);
        let content_backdrop = Rect::new(
            inner.x,
            inner.y + TITLEBAR_H as i32,
            inner.w,
            inner.h.saturating_sub(TITLEBAR_H),
        );
        let pin_rect = control_rect(wx, wy, chrome_w, 3);
        let minimize_rect = control_rect(wx, wy, chrome_w, 2);
        let maximize_rect = control_rect(wx, wy, chrome_w, 1);
        let close_rect = control_rect(wx, wy, chrome_w, 0);
        let control_strip_w = (BTN_SIZE + BTN_SPACING) * 4;
        let title_rect = Rect::new(
            wx as i32 + 14,
            wy as i32 + 2,
            chrome_w.saturating_sub(control_strip_w + 28),
            TITLEBAR_H.saturating_sub(4),
        );

        {
            let mut canvas = back_buffer_canvas(state, back_buffer);
            canvas.fill_rounded_rect(outer, outer_radius, Color(bd_color));
            canvas.fill_rounded_rect(inner, inner_radius, Color(tb_color));
            canvas.fill_rect(content_backdrop, Color(0xFF1A1A20));

            draw_window_control(
                &mut canvas,
                pin_rect,
                WindowControlKind::Pin,
                Color(icon_col),
                Color(tb_color),
                hover_zone == HitZone::KeepOnTopBtn,
                win.config.z_index_type == ZIndexType::OnTop,
            );
            draw_window_control(
                &mut canvas,
                minimize_rect,
                WindowControlKind::Minimize,
                Color(icon_col),
                Color(tb_color),
                hover_zone == HitZone::MinimizeBtn,
                false,
            );
            draw_window_control(
                &mut canvas,
                maximize_rect,
                if maximized {
                    WindowControlKind::Restore
                } else {
                    WindowControlKind::Maximize
                },
                Color(icon_col),
                Color(tb_color),
                hover_zone == HitZone::MaximizeBtn,
                maximized,
            );
            draw_window_control(
                &mut canvas,
                close_rect,
                WindowControlKind::Close,
                Color(icon_col),
                Color(tb_color),
                hover_zone == HitZone::CloseBtn,
                is_focused,
            );
            draw_title(
                &mut canvas,
                &win.config.title,
                title_rect,
                Color(TITLE_TEXT_COLOR),
            );
        }
    }

    // Blit client buffer.
    if canvas_x >= state.fb_width || canvas_y >= state.fb_height {
        return;
    }
    let copy_w = client_w.min(state.fb_width - canvas_x) as usize;
    let copy_h = client_h.min(state.fb_height - canvas_y) as usize;
    let stride = fb_stride(state);
    let back_ptr = back_buffer.as_mut_ptr();
    // Inner content area used for anti-aliased rounded-corner clipping.
    let clip_rect = if !no_chrome && !maximized {
        Some(Rect::new(
            win.x as i32 + BORDER_W as i32,
            win.y as i32 + BORDER_W as i32,
            win.width,
            TITLEBAR_H + win.height,
        ))
    } else {
        None
    };
    for row in 0..copy_h {
        unsafe {
            let src_row = win.buffer.add(row * win.width as usize);
            let dst_row = back_ptr.add((canvas_y as usize + row) * stride + canvas_x as usize);
            if let Some(rect) = clip_rect {
                // Rect-local Y coordinate for this row.
                let ly = canvas_y as i32 + row as i32 - rect.y;
                // Base rect-local X for col 0 of this blit row.
                let lx_base = canvas_x as i32 - rect.x;
                let rw = rect.w as i32;
                let rh = rect.h as i32;
                for col in 0..copy_w {
                    let lx = lx_base + col as i32;
                    let cov = state.inner_corner_mask.coverage(lx, ly, rw, rh);
                    match cov {
                        0 => {}
                        255 => {
                            dst_row.add(col).write(src_row.add(col).read());
                        }
                        a => {
                            let src_px = src_row.add(col).read();
                            let dst_px = dst_row.add(col).read();
                            dst_row
                                .add(col)
                                .write(blend::blend_pixel(src_px, dst_px, a));
                        }
                    }
                }
            } else {
                core::ptr::copy_nonoverlapping(src_row, dst_row, copy_w);
            }
        }
    }
}

fn draw_cursor_bitmap(
    state: &CompositorState,
    back_buffer: &mut [u32],
    shape: CursorShape,
    base_x: u32,
    base_y: u32,
) {
    let base_x = base_x as i32;
    let base_y = base_y as i32;
    let stride = fb_stride(state);
    let bitmap = cursor_bitmap(shape);

    const SHADOW: u32 = 0x00000000;
    const FG: u32 = 0x00F5F5F5;

    for (row, &mask) in bitmap.iter().enumerate() {
        for col in 0..CURSOR_W {
            if (mask & (1 << (7 - col))) == 0 {
                continue;
            }
            let x = base_x + col as i32;
            let y = base_y + row as i32;
            if x < 0 || y < 0 || x >= state.fb_width as i32 || y >= state.fb_height as i32 {
                continue;
            }
            let color = if col == CURSOR_W - 1 || row == CURSOR_H - 1 {
                SHADOW
            } else {
                FG
            };
            back_buffer[y as usize * stride + x as usize] = color;
        }
    }
}

fn save_software_cursor_under(state: &mut CompositorState, back_buffer: &[u32]) -> Option<Rect> {
    let rect = cursor_rect(state, state.mouse_x as u32, state.mouse_y as u32)?;
    let stride = fb_stride(state);
    let saved = &mut state.software_cursor;
    saved.x = rect.x as u32;
    saved.y = rect.y as u32;
    saved.w = rect.w;
    saved.h = rect.h;
    saved.shape = state.active_cursor;
    saved.valid = true;

    for row in 0..rect.h as usize {
        let src = (saved.y as usize + row) * stride + saved.x as usize;
        let dst = row * CURSOR_W;
        saved.pixels[dst..dst + rect.w as usize]
            .copy_from_slice(&back_buffer[src..src + rect.w as usize]);
    }

    Some(rect)
}

fn restore_software_cursor_under(
    state: &mut CompositorState,
    back_buffer: &mut [u32],
) -> Option<Rect> {
    if !state.software_cursor.valid {
        return None;
    }

    let saved = state.software_cursor;
    let stride = fb_stride(state);
    for row in 0..saved.h as usize {
        let dst = (saved.y as usize + row) * stride + saved.x as usize;
        let src = row * CURSOR_W;
        back_buffer[dst..dst + saved.w as usize]
            .copy_from_slice(&saved.pixels[src..src + saved.w as usize]);
    }

    state.software_cursor.valid = false;
    Some(Rect::new(saved.x as i32, saved.y as i32, saved.w, saved.h))
}

fn draw_cursor(state: &mut CompositorState, back_buffer: &mut [u32]) {
    // Software cursor: only used when the hardware cursor overlay is inactive.
    if state.hw_cursor_active {
        state.software_cursor.valid = false;
        return;
    }
    state.debug_counters.sw_cursor_redraw_count += 1;
    let _ = save_software_cursor_under(state, back_buffer);
    draw_cursor_bitmap(
        state,
        back_buffer,
        state.active_cursor,
        state.mouse_x as u32,
        state.mouse_y as u32,
    );
}

fn present_dirty_regions(state: &mut CompositorState) {
    if state.dirty.needs_full_present() {
        present_back_buffer(state);
    } else {
        let count = state.dirty.count;
        let rects = state.dirty.rects;
        for i in 0..count {
            present_rect(state, rects[i]);
        }
    }
    state.dirty.clear();
}

fn move_software_cursor(state: &mut CompositorState) -> bool {
    if state.hw_cursor_active || !state.session_active || !state.software_cursor.valid {
        return false;
    }

    state.debug_counters.sw_cursor_move_count += 1;
    let mut back_buffer = core::mem::take(&mut state.back_buffer);
    if let Some(old_rect) = restore_software_cursor_under(state, &mut back_buffer) {
        mark_dirty_rect(state, old_rect);
    }
    if let Some(new_rect) = save_software_cursor_under(state, &back_buffer) {
        mark_dirty_rect(state, new_rect);
    }
    draw_cursor_bitmap(
        state,
        &mut back_buffer,
        state.active_cursor,
        state.mouse_x as u32,
        state.mouse_y as u32,
    );
    state.back_buffer = back_buffer;
    present_dirty_regions(state);
    true
}

/// Upload a hardware cursor to the GPU if the shape changed.
/// Converts the 1-bit 8×12 cursor bitmap to a 64×64 BGRA pixel image.
fn upload_hw_cursor_if_needed(state: &mut CompositorState) -> bool {
    if !state.hw_cursor_active {
        return false;
    }
    if state.last_hw_cursor_shape == Some(state.active_cursor) {
        return true;
    }

    // Build a 64×64 BGRA buffer (transparent background, white foreground).
    // We upscale the 8×12 bitmap by 4× to fill the 32×48 center of the 64×64 canvas.
    static mut CURSOR_PIXELS: [u32; 64 * 64] = [0; 64 * 64];
    let pixels = unsafe { &mut *(&raw mut CURSOR_PIXELS) };
    for p in pixels.iter_mut() {
        *p = 0x00000000; // transparent
    }

    let bitmap = cursor_bitmap(state.active_cursor);
    let scale = 4usize;
    // Offset within 64×64 canvas to center the upscaled 8×12 glyph (32×48)
    let off_x = (64 - CURSOR_W * scale) / 2;
    let off_y = (64 - CURSOR_H * scale) / 2;

    const FG_BGRA: u32 = 0xFFF5F5F5; // opaque white (BGRA: B=F5, G=F5, R=F5, A=FF)
    const SHADOW_BGRA: u32 = 0xFF303030; // opaque dark shadow

    for (row, &mask) in bitmap.iter().enumerate() {
        for col in 0..CURSOR_W {
            if (mask & (1 << (7 - col))) == 0 {
                continue;
            }
            let color = if col == CURSOR_W - 1 || row == CURSOR_H - 1 {
                SHADOW_BGRA
            } else {
                FG_BGRA
            };
            for dy in 0..scale {
                for dx in 0..scale {
                    let px = off_x + col * scale + dx;
                    let py = off_y + row * scale + dy;
                    if px < 64 && py < 64 {
                        pixels[py * 64 + px] = color;
                    }
                }
            }
        }
    }

    // Hotspot is top-left corner (0, 0) for all cursors currently.
    let ok = sunlight_ipc::gpu_update_cursor(pixels.as_ptr(), 64 * 64, 0, 0);
    if ok {
        state.last_hw_cursor_shape = Some(state.active_cursor);
        true
    } else {
        debug_log(
            "[DISPLAY] hardware cursor UPDATE_CURSOR failed, falling back to software cursor\n",
        );
        state.hw_cursor_active = false;
        state.last_hw_cursor_shape = None;
        false
    }
}

fn move_hw_cursor(state: &mut CompositorState, x: u32, y: u32) -> bool {
    if !state.hw_cursor_active {
        return false;
    }
    // Defensively clamp to the current selected screen size so a stray coordinate
    // can never push the hardware cursor off the scanout. Moving the cursor never
    // touches the back buffer, so this does not trigger a compositor redraw.
    let x = x.min(state.fb_width.saturating_sub(1));
    let y = y.min(state.fb_height.saturating_sub(1));
    let ok = sunlight_ipc::gpu_move_cursor(x, y);
    if ok {
        state.debug_counters.hw_cursor_move_count += 1;
        true
    } else {
        debug_log(
            "[DISPLAY] hardware cursor MOVE_CURSOR failed, falling back to software cursor\n",
        );
        state.hw_cursor_active = false;
        state.last_hw_cursor_shape = None;
        false
    }
}

fn redraw_scene(state: &mut CompositorState) {
    if !state.session_active {
        return;
    }
    state.debug_counters.desktop_redraw_count += 1;
    // Upload a new hardware cursor image if the shape changed (no-op when sw cursor).
    let _ = upload_hw_cursor_if_needed(state);

    clear_back_buffer(state);
    let focused_idx = focused_window_idx(state);
    let windows = &state.windows;
    let mut back_buffer = core::mem::take(&mut state.back_buffer);
    for (i, win) in windows.iter().enumerate() {
        let is_focused = focused_idx == Some(i);
        composite_window(state, &mut back_buffer, win, is_focused);
    }
    // Software cursor only: the GPU backend uses a hardware cursor overlay.
    draw_cursor(state, &mut back_buffer);
    state.back_buffer = back_buffer;
    present_dirty_regions(state);
}

// ---------------------------------------------------------------------------
// Debug helpers
// ---------------------------------------------------------------------------

fn debug_hex(val: u32) {
    let mut buf = [0u8; 10];
    buf[0] = b'0';
    buf[1] = b'x';
    let hex = b"0123456789ABCDEF";
    for i in 0..8 {
        buf[2 + i] = hex[((val >> (28 - i * 4)) & 0xF) as usize];
    }
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
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    let len = 11 - i;
    unsafe {
        core::arch::asm!("syscall",
            inlateout("rax") 99u64 => _,
            in("rdi") buf.as_ptr().add(i) as u64, in("rsi") len as u64,
            lateout("rcx") _, lateout("r11") _, options(nostack));
    }
}

fn debug_dec_u64(val: u64) {
    let mut buf = [0u8; 20];
    let mut n = val;
    let mut i = buf.len();
    loop {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    unsafe {
        core::arch::asm!("syscall",
            inlateout("rax") 99u64 => _,
            in("rdi") buf.as_ptr().add(i) as u64, in("rsi") (buf.len() - i) as u64,
            lateout("rcx") _, lateout("r11") _, options(nostack));
    }
}

#[allow(dead_code)]
fn debug_i32(val: i32) {
    if val < 0 {
        debug_log("-");
        debug_dec((-val) as u32);
    } else {
        debug_dec(val as u32);
    }
}

fn log_debug_counters(state: &CompositorState, reason: &str) {
    debug_log("[DISPLAY] counters ");
    debug_log(reason);
    debug_log(": mouse=");
    debug_dec_u64(state.debug_counters.mouse_event_count);
    debug_log(" hw_move=");
    debug_dec_u64(state.debug_counters.hw_cursor_move_count);
    debug_log(" sw_move=");
    debug_dec_u64(state.debug_counters.sw_cursor_move_count);
    debug_log(" sw_redraw=");
    debug_dec_u64(state.debug_counters.sw_cursor_redraw_count);
    debug_log(" desktop_redraw=");
    debug_dec_u64(state.debug_counters.desktop_redraw_count);
    debug_log(" dirty=");
    debug_dec_u64(state.debug_counters.dirty_rect_count);
    debug_log(" full_present=");
    debug_dec_u64(state.debug_counters.full_present_count);
    debug_log(" rect_present=");
    debug_dec_u64(state.debug_counters.present_rect_count);
    debug_log(" fb_copy=");
    debug_dec_u64(state.debug_counters.framebuffer_copy_count);
    debug_log(" hw_cursor=");
    debug_dec(if state.hw_cursor_active { 1 } else { 0 });
    debug_log("\n");
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
        None => {
            debug_log("[DISPLAY] Failed to map framebuffer\n");
            loop {}
        }
    };

    let fb_width = (packed_wh & 0xffffffff) as u32;
    let fb_height = (packed_wh >> 32) as u32;

    debug_log("[DISPLAY] fb ");
    debug_dec(fb_width);
    debug_log("x");
    debug_dec(fb_height);
    debug_log(" pitch=");
    debug_hex(pitch as u32);
    debug_log(" bpp=");
    debug_dec(bpp as u32);
    debug_log("\n");

    let limine_backend = backend::DisplayBackend::Limine {
        fb: fb_ptr as *mut u32,
        pitch_words: pitch as usize / 4,
    };

    // Try to use VirtIO GPU backend; fall back to Limine framebuffer.
    //
    // TODO(live-resize): the VirtIO GPU size is sampled exactly once here, at
    // startup. If the host window is resized later, the kernel can signal a new
    // scanout size; the compositor would then need to resize back_buffer,
    // recreate the GPU resource, re-attach backing, set scanout, mark the whole
    // screen dirty and redraw. Until that path exists we pin the initial size.
    let detected_backend = match sunlight_ipc::gpu_get_info() {
        Some((gw, gh)) => match validate_size(gw, gh) {
            Some((gw, gh)) => {
                debug_log("[DISPLAY] VirtIO GPU reported ");
                debug_dec(gw);
                debug_log("x");
                debug_dec(gh);
                debug_log("\n");
                backend::DisplayBackend::VirtioGpu {
                    width: gw,
                    height: gh,
                }
            }
            None => {
                // Garbage/zero size — ignore the GPU and use Limine instead.
                debug_log("[DISPLAY] VirtIO GPU reported invalid size ");
                debug_dec(gw);
                debug_log("x");
                debug_dec(gh);
                debug_log(", ignoring GPU\n");
                limine_backend
            }
        },
        None => {
            debug_log("[DISPLAY] VirtIO GPU not found, using Limine framebuffer\n");
            limine_backend
        }
    };

    // Render dimensions. The VirtIO GPU scans out at its own resolution (gw×gh),
    // which often differs from the Limine framebuffer (e.g. QEMU auto-resizes the
    // virtio-gpu to the host window). The back buffer and compositor MUST match
    // the GPU's dimensions, tightly packed (stride = width*4) to match the
    // kernel's resource_create_2d — otherwise the GPU reads past the buffer and
    // the screen is black. Default to the Limine framebuffer dimensions, then to
    // a safe fallback if even those are unusable (never allocate a 0×0 buffer).
    let (limine_render_w, limine_render_h, limine_render_pitch) =
        match validate_size(fb_width, fb_height) {
            Some((w, h)) => (w, h, pitch as u32),
            None => {
                debug_log("[DISPLAY] Limine framebuffer size invalid, using safe fallback\n");
                (SAFE_FALLBACK_W, SAFE_FALLBACK_H, SAFE_FALLBACK_W * 4)
            }
        };
    let mut render_width = limine_render_w;
    let mut render_height = limine_render_h;
    let mut render_pitch = limine_render_pitch;
    if let backend::DisplayBackend::VirtioGpu { width, height } = detected_backend {
        render_width = width;
        render_height = height;
        render_pitch = width * 4;
    }

    let mut back_buffer = {
        let mut pixels = Vec::new();
        pixels.resize(
            (render_pitch as usize / 4) * render_height as usize,
            DESKTOP_COLOR,
        );
        pixels
    };

    let mut display_backend = detected_backend;
    let mut hw_cursor = false;
    if let backend::DisplayBackend::VirtioGpu { .. } = display_backend {
        // Page-align the back_buffer pointer and count pages.
        let buf_start = back_buffer.as_ptr();
        let buf_bytes = back_buffer.len() * 4;
        let num_pages = (buf_bytes + 4095) / 4096;
        let ok = sunlight_ipc::gpu_attach_backing(buf_start, num_pages);
        if ok {
            debug_log("[DISPLAY] gpu_attach_backing OK\n");
            let ok2 = sunlight_ipc::gpu_set_scanout();
            if ok2 {
                debug_log("[DISPLAY] gpu_set_scanout OK\n");
                hw_cursor = true;
            } else {
                debug_log("[DISPLAY] gpu_set_scanout FAILED\n");
                debug_log("[DISPLAY] falling back to Limine framebuffer backend\n");
                display_backend = limine_backend;
            }
        } else {
            debug_log("[DISPLAY] gpu_attach_backing FAILED\n");
            debug_log("[DISPLAY] falling back to Limine framebuffer backend\n");
            display_backend = limine_backend;
        }
        // If GPU setup failed, the back buffer was sized for the GPU; revert it
        // and the render dimensions to the Limine framebuffer for software present.
        if !hw_cursor {
            render_width = limine_render_w;
            render_height = limine_render_h;
            render_pitch = limine_render_pitch;
            back_buffer = {
                let mut pixels = Vec::new();
                pixels.resize(
                    (render_pitch as usize / 4) * render_height as usize,
                    DESKTOP_COLOR,
                );
                pixels
            };
        }
    }

    // Startup summary: single source of truth for the compositor's geometry.
    match display_backend {
        backend::DisplayBackend::VirtioGpu { .. } => debug_log("[DISPLAY] backend=VirtioGpu\n"),
        backend::DisplayBackend::Limine { .. } => debug_log("[DISPLAY] backend=Limine\n"),
    }
    debug_log("[DISPLAY] compositor size ");
    debug_dec(render_width);
    debug_log("x");
    debug_dec(render_height);
    debug_log(" pitch=");
    debug_dec(render_pitch);
    debug_log("\n");
    let expected_back_len = (render_pitch as usize / 4) * render_height as usize;
    debug_log("[DISPLAY] back_buffer expected_len=");
    debug_dec(expected_back_len as u32);
    debug_log(" actual_len=");
    debug_dec(back_buffer.len() as u32);
    debug_log("\n");

    let mut state = CompositorState {
        windows: Vec::new(),
        mouse_x: (render_width / 2) as u16,
        mouse_y: (render_height / 2) as u16,
        pointer: PointerState::new(render_width, render_height),
        mouse_sensitivity_fp: SENS_DEFAULT_FP,
        active_drag: ActiveDrag::None,
        prev_buttons: 0,
        fb: fb_ptr as *mut u32,
        fb_width: render_width,
        fb_height: render_height,
        fb_pitch: render_pitch,
        back_buffer,
        display_backend,
        hw_cursor_active: hw_cursor,
        last_hw_cursor_shape: None,
        active_cursor: CursorShape::Pointer,
        software_cursor: SoftwareCursorState::new(),
        // TTY session owns the framebuffer at boot; tty_server sends
        // SESSION_ACTIVATE to hand the framebuffer to the Desktop session.
        session_active: false,
        inner_corner_mask: mask::CornerMask::new(CHROME_RADIUS.saturating_sub(BORDER_W)),
        dirty: dirty::DirtyList::new(),
        debug_counters: DebugCounters::new(),
        wallpaper: TgaImage::parse(WALLPAPER_TGA_BYTES).ok(),
        vortex_launch_pending: false,
    };
    // Initial cursor position for the hardware cursor.
    if state.hw_cursor_active {
        let init_mouse_x = state.mouse_x as u32;
        let init_mouse_y = state.mouse_y as u32;
        let _ = upload_hw_cursor_if_needed(&mut state);
        let _ = move_hw_cursor(&mut state, init_mouse_x, init_mouse_y);
        if !state.hw_cursor_active {
            debug_log(
                "[DISPLAY] initial hardware cursor setup failed; using software cursor on GPU scanout\n",
            );
        }
    }
    debug_log("[DISPLAY] cursor_mode=");
    debug_log(if state.hw_cursor_active { "hardware\n" } else { "software\n" });
    log_debug_counters(&state, "startup");
    redraw_scene(&mut state);

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
                let surface_bytes = w as usize * h as usize * 4;
                let size = surface_bytes.saturating_mul(2).max(4096);
                let config = WindowConfig::from_ipc_words(&msg.words);
                let is_desktop_window = config.window_type == WindowType::Desktop;
                let owner_pid = msg.badge;

                match sunlight_ipc::shm_create(size, 0) {
                    Ok((_, shm_tok)) => {
                        let our_buf = match sunlight_ipc::shm_map(shm_tok) {
                            Ok(p) => p as *mut u32,
                            Err(_) => core::ptr::null_mut(),
                        };

                        let id = next_win_id;
                        next_win_id += 1;

                        // Initial position: cascade from center of screen.
                        let cascade = ((id.saturating_sub(1)) % 8) as u32 * 28;
                        let win_x = state
                            .fb_width
                            .saturating_sub(w)
                            .saturating_div(2)
                            .saturating_add(cascade);
                        let win_y = (state.fb_height / 4)
                            .saturating_sub(h / 2)
                            .saturating_add(cascade);

                        debug_log("[DISPLAY] create_window id=");
                        debug_dec(id as u32);
                        debug_log(" pos=");
                        debug_dec(win_x);
                        debug_log("x");
                        debug_dec(win_y);
                        debug_log(" size=");
                        debug_dec(w);
                        debug_log("x");
                        debug_dec(h);
                        debug_log("\n");

                        // Raise on-top windows above normal ones.
                        let insert_at = if config.z_index_type == ZIndexType::OnTop {
                            state.windows.len()
                        } else {
                            // Insert before any OnTop windows.
                            state
                                .windows
                                .iter()
                                .position(|w| w.config.z_index_type == ZIndexType::OnTop)
                                .unwrap_or(state.windows.len())
                        };

                        state.windows.insert(
                            insert_at,
                            Window {
                                id,
                                shm_cap: shm_tok,
                                buffer: our_buf,
                                width: w,
                                height: h,
                                x: win_x,
                                y: win_y,
                                saved_x: win_x,
                                saved_y: win_y,
                                saved_w: w,
                                saved_h: h,
                                // Authoritative creator PID from the kernel IPC badge.
                                // TODO: Use this for process-exit-driven cleanup in the
                                // later hardening phase.
                                owner_pid,
                                config,
                                client_cursor: CursorShape::Pointer,
                                pending_keys: KeyEventQueue::new(),
                            },
                        );
                        if is_desktop_window {
                            state.vortex_launch_pending = false;
                        }
                        mark_dirty_full(&mut state);
                        redraw_scene(&mut state);

                        let client_x = win_x + BORDER_W;
                        let client_y = win_y + TITLEBAR_H;
                        let mut reply = IpcMsg::with_label(SgpMsg::REPLY)
                            .word(0, id)
                            .word(1, size as u64)
                            .word(2, (w * 4) as u64)
                            .word(3, client_x as u64 | ((client_y as u64) << 32));
                        reply.caps[0] = shm_tok;
                        reply.cap_count = 1;
                        let _ = ipc_reply(reply);
                    }
                    Err(_) => {
                        let _ = ipc_reply(IpcMsg::with_label(0xA1FE));
                    }
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
                            1 => WindowType::Dialog,
                            2 => WindowType::Desktop,
                            3 => WindowType::Widget,
                            _ => WindowType::Normal,
                        };
                        let new_state = match ((flags >> 2) & 0x3) as u8 {
                            1 => WindowState::Minimized,
                            2 => WindowState::Maximized,
                            3 => WindowState::Fullscreen,
                            _ => WindowState::Normal,
                        };
                        // Save geometry before entering maximized/fullscreen.
                        if new_state == WindowState::Maximized
                            || new_state == WindowState::Fullscreen
                        {
                            if win.config.state == WindowState::Normal {
                                win.saved_x = win.x;
                                win.saved_y = win.y;
                                win.saved_w = win.width;
                                win.saved_h = win.height;
                            }
                        } else if new_state == WindowState::Normal {
                            // Restore saved geometry.
                            win.x = win.saved_x;
                            win.y = win.saved_y;
                            win.width = win.saved_w;
                            win.height = win.saved_h;
                        }
                        win.config.state = new_state;
                        win.config.border = if (flags >> 4) & 1 != 0 {
                            BorderStyle::None
                        } else {
                            BorderStyle::Full
                        };
                        win.config.z_index_type = if (flags >> 5) & 1 != 0 {
                            ZIndexType::OnTop
                        } else {
                            ZIndexType::Normal
                        };
                        let z = ((flags >> 6) & 0x7F) as u8;
                        if z > 0 {
                            win.config.z_index_value = z.min(100);
                        }
                        win.config.show_type = match ((flags >> 13) & 0x3) as u8 {
                            1 => ShowType::Tiled,
                            2 => ShowType::Scrolling,
                            _ => ShowType::Floating,
                        };
                        win.config.group_type = match ((flags >> 15) & 0x3) as u8 {
                            1 => GroupType::Stacked,
                            2 => GroupType::Tabbed,
                            _ => GroupType::None,
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
                    // Re-position windows to their designated screen areas once the title
                    // is known (title arrives via CONFIGURE_WINDOW, not CREATE_WINDOW).
                    if win.config.state == WindowState::Normal {
                        if win.config.title.starts_with(b"Tasks Mo") {
                            // Tasks Monitor → bottom-right
                            let new_x = state.fb_width.saturating_sub(win.width).saturating_sub(24);
                            let new_y = state
                                .fb_height
                                .saturating_sub(win.height)
                                .saturating_sub(48);
                            win.x = new_x;
                            win.saved_x = new_x;
                            win.y = new_y;
                            win.saved_y = new_y;
                        } else if win.config.title.starts_with(b"Sunlight Te") {
                            // Sunlight Terminal → left side, keep y
                            win.x = 24;
                            win.saved_x = 24;
                        }
                    }
                }
                mark_dirty_full(&mut state);
                redraw_scene(&mut state);
                let _ = ipc_reply(IpcMsg::with_label(SgpMsg::REPLY));
            }

            // -------------------------------------------------------------------
            // GET_SCREEN_INFO — reply with physical framebuffer dimensions.
            // No input words needed.
            // Reply: words[0] = fb_width | (fb_height << 32)
            // -------------------------------------------------------------------
            SgpMsg::GET_SCREEN_INFO => {
                let packed = state.fb_width as u64 | ((state.fb_height as u64) << 32);
                let _ = ipc_reply(IpcMsg::with_label(SgpMsg::REPLY).word(0, packed));
            }

            // SET_CURSOR — client declares preferred cursor for its client area.
            // words[0] = (win_id as u32) | ((CursorShape discriminant as u32) << 32)
            // -------------------------------------------------------------------
            SgpMsg::SET_CURSOR => {
                let win_id = msg.words[0] & 0xFFFF_FFFF;
                let shape_byte = ((msg.words[0] >> 32) & 0xFF) as u8;
                if let Some(win) = state.windows.iter_mut().find(|w| w.id == win_id) {
                    win.client_cursor = CursorShape::from_u8(shape_byte);
                }
                // Re-evaluate active cursor immediately.
                state.active_cursor = cursor_for_scene(&state);
                let cursor_rect = cursor_dirty_rect(state.mouse_x as u32, state.mouse_y as u32);
                if state.hw_cursor_active {
                    if !upload_hw_cursor_if_needed(&mut state) {
                        mark_dirty_rect(&mut state, cursor_rect);
                        redraw_scene(&mut state);
                    }
                } else {
                    mark_dirty_rect(&mut state, cursor_rect);
                    redraw_scene(&mut state);
                }
                let _ = ipc_reply(IpcMsg::with_label(SgpMsg::REPLY));
            }

            // -------------------------------------------------------------------
            // COMMIT_FRAME — client finished drawing, ask compositor to re-blit.
            // -------------------------------------------------------------------
            SgpMsg::COMMIT_FRAME => {
                let win_id = msg.words[0];
                if let Some(win) = state.windows.iter().find(|w| w.id == win_id) {
                    let (wx, wy, ww, wh) = win.chrome_rect(state.fb_width, state.fb_height);
                    mark_dirty_rect(&mut state, Rect::new(wx as i32, wy as i32, ww, wh));
                    redraw_scene(&mut state);
                }
                let _ = ipc_reply(IpcMsg::with_label(SgpMsg::REPLY));
            }

            // -------------------------------------------------------------------
            // EVENT_POLL — client polls mouse position + its current client origin.
            // -------------------------------------------------------------------
            SgpMsg::EVENT_POLL => {
                let win_id = msg.words[0];
                let packed = (state.mouse_x as u64) | ((state.mouse_y as u64) << 16);
                let mut wake = IpcMsg::with_label(SgpMsg::REPLY)
                    .word(0, packed)
                    .word(3, state.prev_buttons as u64);
                if let Some(win) = state.windows.iter_mut().find(|w| w.id == win_id) {
                    let (cx, cy) = match win.config.state {
                        WindowState::Fullscreen => (0u64, 0u64),
                        WindowState::Maximized => (BORDER_W as u64, TITLEBAR_H as u64),
                        _ => ((win.x + BORDER_W) as u64, (win.y + TITLEBAR_H) as u64),
                    };
                    wake = wake.word(1, cx | (cy << 32));
                    if let Some(key_event) = win.pending_keys.pop() {
                        wake = wake.word(2, key_event);
                    }
                }
                let _ = ipc_reply(wake);
            }

            // -------------------------------------------------------------------
            // CLOSE_WINDOW / DESTROY_WINDOW
            // -------------------------------------------------------------------
            SgpMsg::CLOSE_WINDOW => {
                let win_id = msg.words[0];
                if close_window(&mut state, win_id, Some(msg.badge)) {
                    mark_dirty_full(&mut state);
                    redraw_scene(&mut state);
                }
                let _ = ipc_reply(IpcMsg::with_label(SgpMsg::REPLY));
            }

            // -------------------------------------------------------------------
            // KEY_EVENT — Global keyboard interceptor.
            // Ctrl + W: close currently active (focused) window.
            // Super + R or Ctrl + Space: launch the Run dialog.
            // Alt + Tab: rotate focus across app windows in the active z-group.
            // -------------------------------------------------------------------
            sunlight_ipc::KbdMsg::KEY_EVENT => {
                let packed = msg.words[0];
                let (keycode, pressed, _, ctrl, alt, super_key, _) =
                    sunlight_ipc::unpack_key_event(packed);

                if pressed && ((super_key && keycode == KEY_R) || (ctrl && keycode == KEY_SPACE)) {
                    launch_runner();
                } else if pressed && alt && keycode == KEY_TAB {
                    if cycle_focus(&mut state) {
                        state.active_cursor = cursor_for_scene(&state);
                        mark_dirty_full(&mut state);
                        redraw_scene(&mut state);
                    }
                } else if pressed && ctrl && keycode == KEY_W {
                    if let Some(focused) = focused_window_id(&state) {
                        if close_window(&mut state, focused, None) {
                            mark_dirty_full(&mut state);
                            redraw_scene(&mut state);
                        }
                    }
                } else if state.session_active {
                    if let Some(focused_idx) = focused_window_idx(&state) {
                        let win = &mut state.windows[focused_idx];
                        let (
                            queued_keycode,
                            queued_pressed,
                            _queued_shift,
                            queued_ctrl,
                            _queued_alt,
                            _queued_super,
                            queued_ascii,
                        ) = sunlight_ipc::unpack_key_event(packed);
                        if queued_pressed {
                            debug_log(&alloc::format!(
                                "[DISPLAY] queued key win={} keycode={:#x} ctrl={} ascii={}\n",
                                win.id,
                                queued_keycode,
                                queued_ctrl,
                                queued_ascii.unwrap_or(0)
                            ));
                        }
                        win.pending_keys.push(packed);
                    }
                }
                let _ = ipc_reply(IpcMsg::with_label(SgpMsg::REPLY));
            }

            // -------------------------------------------------------------------
            // RAW_MOTION — mouse driver delta + button state.
            // words[0] = dx(i16) | dy(i16)<<16 | buttons(u8)<<32
            // -------------------------------------------------------------------
            MouseMsg::RAW_MOTION => {
                let raw = msg.words[0];
                // Driver already inverted Y (PS/2 up→negative screen delta). Do NOT negate again.
                let dx = ((raw & 0xFFFF) as i16) as i32;
                let dy = (((raw >> 16) & 0xFFFF) as i16) as i32;
                let buttons = ((raw >> 32) & 0xFF) as u8;

                let prev_cx = state.pointer.x() as u32;
                let prev_cy = state.pointer.y() as u32;
                let prev_buttons = state.prev_buttons;
                state.debug_counters.mouse_event_count += 1;
                // Snapshot drag state before processing to detect window-geometry changes.
                let had_active_drag = !matches!(state.active_drag, ActiveDrag::None);

                if MOUSE_DEBUG {
                    debug_log("[DISPLAY] raw_motion dx=");
                    debug_i32(dx);
                    debug_log(" dy=");
                    debug_i32(dy);
                    debug_log(" before=(");
                    debug_dec(prev_cx);
                    debug_log(",");
                    debug_dec(prev_cy);
                    debug_log(")\n");
                }
                let left_down = (buttons & 1) != 0;
                let was_left_down = (prev_buttons & 1) != 0;

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
                    let mut hit_id: Option<u64> = None;
                    let mut hit_zone: HitZone = HitZone::Miss;

                    for win in state.windows.iter().rev() {
                        if win.config.state == WindowState::Minimized {
                            continue;
                        }
                        let zone = hit_test_window(win, cx, cy, state.fb_width, state.fb_height);
                        if zone != HitZone::Miss {
                            hit_id = Some(win.id);
                            hit_zone = zone;
                            break;
                        }
                    }

                    if let Some(id) = hit_id {
                        // Raise to front (unless it's a Desktop/Widget type).
                        let win_type = state
                            .windows
                            .iter()
                            .find(|w| w.id == id)
                            .map(|w| w.config.window_type)
                            .unwrap_or(WindowType::Normal);
                        if win_type != WindowType::Desktop && win_type != WindowType::Widget {
                            if let Some(pos) = state.windows.iter().position(|w| w.id == id) {
                                // Keep OnTop windows at the very end; raise normal windows
                                // to just before the first OnTop window.
                                let win = state.windows.remove(pos);
                                let target = if win.config.z_index_type == ZIndexType::OnTop {
                                    state.windows.len()
                                } else {
                                    state
                                        .windows
                                        .iter()
                                        .position(|w| w.config.z_index_type == ZIndexType::OnTop)
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
                                let _ = close_window(&mut state, id, None);
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
                                        win.x = win.saved_x;
                                        win.y = win.saved_y;
                                        win.width = win.saved_w;
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
                                    win.config.z_index_type =
                                        if win.config.z_index_type == ZIndexType::OnTop {
                                            ZIndexType::Normal
                                        } else {
                                            ZIndexType::OnTop
                                        };
                                }
                                // Sorting logic is handled on next click, but we could enforce it here too
                                state.active_drag = ActiveDrag::None;
                            }
                            edge @ (HitZone::EdgeLeft
                            | HitZone::EdgeRight
                            | HitZone::EdgeBottom
                            | HitZone::CornerBL
                            | HitZone::CornerBR) => {
                                if let Some(win) = state.windows.iter().find(|w| w.id == id) {
                                    let re = match edge {
                                        HitZone::EdgeLeft => ResizeEdge::Left,
                                        HitZone::EdgeRight => ResizeEdge::Right,
                                        HitZone::EdgeBottom => ResizeEdge::Bottom,
                                        HitZone::CornerBL => ResizeEdge::CornerBL,
                                        _ => ResizeEdge::CornerBR,
                                    };
                                    state.active_drag = ActiveDrag::Resize(ResizeDrag {
                                        window_id: id,
                                        edge: re,
                                        anchor_mx: cx as i32,
                                        anchor_my: cy as i32,
                                        anchor_wx: win.x as i32,
                                        anchor_wy: win.y as i32,
                                        anchor_ww: win.width as i32,
                                        anchor_wh: win.height as i32,
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
                            let drag_id = d.window_id;
                            let edge = d.edge;
                            let amx = d.anchor_mx;
                            let amy = d.anchor_my;
                            let awx = d.anchor_wx;
                            let _awy = d.anchor_wy; // reserved for future top-edge resize
                            let aww = d.anchor_ww;
                            let awh = d.anchor_wh;

                            if let Some(win) = state.windows.iter_mut().find(|w| w.id == drag_id) {
                                if win.config.state != WindowState::Normal { /* no resize when maximized */
                                } else {
                                    let total_dx = cx as i32 - amx;
                                    let total_dy = cy as i32 - amy;
                                    match edge {
                                        ResizeEdge::Right => {
                                            win.width =
                                                (aww + total_dx).max(MIN_WIN_W as i32) as u32;
                                        }
                                        ResizeEdge::Bottom => {
                                            win.height =
                                                (awh + total_dy).max(MIN_WIN_H as i32) as u32;
                                        }
                                        ResizeEdge::Left => {
                                            let new_w =
                                                (aww - total_dx).max(MIN_WIN_W as i32) as u32;
                                            let dx = aww as u32 - new_w;
                                            win.x = (awx + dx as i32).max(0) as u32;
                                            win.width = new_w;
                                        }
                                        ResizeEdge::CornerBR => {
                                            win.width =
                                                (aww + total_dx).max(MIN_WIN_W as i32) as u32;
                                            win.height =
                                                (awh + total_dy).max(MIN_WIN_H as i32) as u32;
                                        }
                                        ResizeEdge::CornerBL => {
                                            let new_w =
                                                (aww - total_dx).max(MIN_WIN_W as i32) as u32;
                                            let dx = aww as u32 - new_w;
                                            win.x = (awx + dx as i32).max(0) as u32;
                                            win.width = new_w;
                                            win.height =
                                                (awh + total_dy).max(MIN_WIN_H as i32) as u32;
                                        }
                                    }
                                }
                            }
                        }
                        ActiveDrag::None => {}
                    }
                }

                if MOUSE_DEBUG {
                    debug_log("[DISPLAY] cursor=(");
                    debug_dec(state.pointer.x() as u32);
                    debug_log(",");
                    debug_dec(state.pointer.y() as u32);
                    debug_log(")\n");
                }

                state.prev_buttons = buttons;
                state.active_cursor = cursor_for_scene(&state);

                let new_cx = state.mouse_x as u32;
                let new_cy = state.mouse_y as u32;
                let button_changed = buttons != prev_buttons;
                let now_dragging = !matches!(state.active_drag, ActiveDrag::None);
                let window_changed = button_changed || had_active_drag || now_dragging;
                let hw_cursor_shape_changed =
                    state.last_hw_cursor_shape != Some(state.active_cursor);
                let sw_cursor_shape_changed = state.software_cursor.valid
                    && state.software_cursor.shape != state.active_cursor;
                let cursor_pixel_changed = new_cx != prev_cx || new_cy != prev_cy;

                if state.hw_cursor_active && !window_changed {
                    let mut fell_back = false;
                    if hw_cursor_shape_changed && !upload_hw_cursor_if_needed(&mut state) {
                        fell_back = true;
                    }
                    if !fell_back
                        && cursor_pixel_changed
                        && !move_hw_cursor(&mut state, new_cx, new_cy)
                    {
                        fell_back = true;
                    }
                    if fell_back {
                        mark_dirty_rect(&mut state, cursor_dirty_rect(prev_cx, prev_cy));
                        mark_dirty_rect(&mut state, cursor_dirty_rect(new_cx, new_cy));
                        redraw_scene(&mut state);
                    }
                } else if !state.hw_cursor_active
                    && !window_changed
                    && (cursor_pixel_changed || sw_cursor_shape_changed)
                {
                    if !move_software_cursor(&mut state) {
                        mark_dirty_rect(&mut state, cursor_dirty_rect(prev_cx, prev_cy));
                        mark_dirty_rect(&mut state, cursor_dirty_rect(new_cx, new_cy));
                        redraw_scene(&mut state);
                    }
                } else {
                    // Window geometry changed or mixed hw/sw state: full dirty-rect path.
                    if window_changed {
                        mark_dirty_full(&mut state);
                    }
                    if window_changed
                        || state.hw_cursor_active
                        || cursor_pixel_changed
                        || sw_cursor_shape_changed
                    {
                        redraw_scene(&mut state);
                    }
                    // After redraw, move hardware cursor to new position.
                    if state.hw_cursor_active
                        && cursor_pixel_changed
                        && !move_hw_cursor(&mut state, new_cx, new_cy)
                    {
                        mark_dirty_rect(&mut state, cursor_dirty_rect(prev_cx, prev_cy));
                        mark_dirty_rect(&mut state, cursor_dirty_rect(new_cx, new_cy));
                        redraw_scene(&mut state);
                    }
                }
                if state.debug_counters.mouse_event_count % COUNTER_LOG_INTERVAL == 0 {
                    log_debug_counters(&state, "mouse");
                }
                let _ = ipc_reply(IpcMsg::with_label(SgpMsg::REPLY));
            }

            // -------------------------------------------------------------------
            // SESSION_ACTIVATE — tty_server hands framebuffer to Desktop session.
            // -------------------------------------------------------------------
            SgpMsg::SESSION_ACTIVATE => {
                if !state.session_active {
                    debug_log("[DISPLAY] [SESSION] activated — Desktop owns framebuffer\n");
                    state.session_active = true;
                    mark_dirty_full(&mut state);
                    redraw_scene(&mut state);
                }
                // Keep the desktop surface present across logins/restarts.
                ensure_vortex_shell(&mut state);
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
                state.vortex_launch_pending = false;
                let _ = ipc_reply(IpcMsg::with_label(SgpMsg::REPLY));
            }

            _ => {
                let _ = ipc_reply(IpcMsg::with_label(SgpMsg::REPLY));
            }
        }
    }
}
