#![no_std]
#![cfg_attr(not(test), no_main)]

#[cfg(test)]
extern crate std;

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use sunlight_libc as libc;

use sunlight_ipc::debug_log;
use sunlight_ipc::{
    endpoint_create, ipc_recv, ipc_recv_timeout, ipc_reply, kill,
    launch_trace::{self, LaunchSource, LaunchTrace},
    monotonic_millis, nameserver_register,
    sgp::SgpMsg,
    validate_size, CapabilityToken, DisplayMetrics, IpcMsg, MouseMsg, NotificationKind,
    PixelFormat, ScreenBackend, SAFE_FALLBACK_H, SAFE_FALLBACK_W,
};
use sunlight_ui::image::TgaImage;
use sunlight_ui::{Canvas, Color, Point, Rect, UiSymbol};

/// Wallpaper asset staged at /var/sunlightos/wallpapers/wallpaper.tga.
/// Embedded directly so the compositor can decode without a VFS read at startup.
static WALLPAPER_TGA_BYTES: &[u8] = include_bytes!("../../../docs/images/wallpaper.tga");
static ICON_SYM_CLOSE_TGA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_close.tga"));

mod app_lifecycle;
mod backend;
mod blend;
mod dirty;
mod mask;
mod surface;

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

#[cfg(not(test))]
#[global_allocator]
static BUMP: BumpAllocator = BumpAllocator;

fn launch_runner(state: &mut CompositorState) {
    if libc::spawn(b"/bin/sunlight-runner", &[b"sunlight-runner"], None).is_err() {
        debug_log("[DISPLAY] failed to launch runner\n");
        push_notification(
            state,
            NotificationKind::Error,
            String::from("Launch failed"),
            String::from("Could not start /bin/sunlight-runner"),
            NOTIFICATION_TIMEOUT_MS,
        );
        mark_dirty_full(state);
        redraw_scene(state);
    }
}

fn find_window_by_title_prefix(state: &CompositorState, prefix: &[u8]) -> Option<(usize, u64)> {
    state.windows.iter().enumerate().find_map(|(idx, win)| {
        win.config
            .title
            .starts_with(prefix)
            .then_some((idx, win.id))
    })
}

fn launch_clipman(state: &mut CompositorState) {
    if libc::spawn(b"/bin/sunlight-clipman", &[b"sunlight-clipman"], None).is_err() {
        debug_log("[DISPLAY] failed to launch clipman\n");
        push_notification(
            state,
            NotificationKind::Error,
            String::from("Launch failed"),
            String::from("Could not start /bin/sunlight-clipman"),
            NOTIFICATION_TIMEOUT_MS,
        );
        mark_dirty_full(state);
        redraw_scene(state);
    }
}

fn toggle_clipman(state: &mut CompositorState) {
    if let Some((idx, win_id)) = find_window_by_title_prefix(state, b"Sunlight Cl") {
        if focused_window_id(state) == Some(win_id) {
            if close_window(state, win_id, None) {
                mark_dirty_full(state);
                redraw_scene(state);
            }
            return;
        }
        if state.windows[idx].config.state == WindowState::Minimized {
            state.windows[idx].config.state = WindowState::Normal;
        }
        let _ = activate_window(state, win_id);
        return;
    }
    launch_clipman(state);
}

fn launch_emoji_picker(state: &mut CompositorState) {
    if libc::spawn(b"/bin/emoji-picker", &[b"emoji-picker"], None).is_err() {
        debug_log("[DISPLAY] failed to launch emoji-picker\n");
        push_notification(
            state,
            NotificationKind::Error,
            String::from("Launch failed"),
            String::from("Could not start /bin/emoji-picker"),
            NOTIFICATION_TIMEOUT_MS,
        );
        mark_dirty_full(state);
        redraw_scene(state);
    }
}

fn toggle_emoji_picker(state: &mut CompositorState) {
    if let Some((idx, win_id)) = find_window_by_title_prefix(state, b"Sunlight Emoji") {
        if focused_window_id(state) == Some(win_id) {
            if close_window(state, win_id, None) {
                mark_dirty_full(state);
                redraw_scene(state);
            }
            return;
        }
        if state.windows[idx].config.state == WindowState::Minimized {
            state.windows[idx].config.state = WindowState::Normal;
        }
        let _ = activate_window(state, win_id);
        return;
    }
    launch_emoji_picker(state);
}

fn launch_vortex_shell(state: &mut CompositorState) -> bool {
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
            push_notification(
                state,
                NotificationKind::Error,
                String::from("Launch failed"),
                String::from("Could not start /bin/sunlight-vortex-shell"),
                NOTIFICATION_TIMEOUT_MS,
            );
            mark_dirty_full(state);
            redraw_scene(state);
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
const INPUT_DEBUG: bool = false;

// Keep these cursor-motion constants in sync with sunlight-mouse until the
// pointer policy is shared by future input backends.
const POINTER_SENSITIVITY_FP: i32 = FP_ONE * 9 / 8;
const POINTER_ACCELERATION_ENABLED: bool = true;
const POINTER_ACCEL_START_MAGNITUDE: i32 = 2;
const POINTER_ACCEL_FACTOR_FP: i32 = FP_ONE / 20;
const POINTER_MAX_ACCEL_GAIN_FP: i32 = FP_ONE * 11 / 8;
const POINTER_MAX_DELTA_PX: i32 = 24;
const EDGE_MARGIN: i32 = 0;

/// Manhattan pixel distance before a pending window-move drag is confirmed.
const DRAG_THRESHOLD_PX: i32 = 4;
const COUNTER_LOG_INTERVAL: u64 = 64;
const KEY_TAB: u8 = 0x0F;
const KEY_CTRL: u8 = 0x1D;
const KEY_ALT: u8 = 0x38;
const KEY_R: u8 = 0x13;
const KEY_V: u8 = 0x2F;
const KEY_W: u8 = 0x11;
const KEY_PERIOD: u8 = 0x34;
const KEY_SPACE: u8 = 0x39;
const KEY_LEFT_SUPER: u8 = 0x5B;
const KEY_RIGHT_SUPER: u8 = 0x5C;
const ALT_TAB_REPEAT_MS: u64 = 120;
const NOTIFICATION_MAX_COUNT: usize = 4;
const NOTIFICATION_WIDTH: u32 = 320;
const NOTIFICATION_HEIGHT: u32 = 78;
const NOTIFICATION_MARGIN_X: i32 = 12;
const NOTIFICATION_MARGIN_Y: i32 = 52;
const NOTIFICATION_GAP: i32 = 8;
const NOTIFICATION_TIMEOUT_MS: u64 = 30_000;
const NOTIFICATION_MIN_TIMEOUT_MS: u64 = 5_000;
const NOTIFICATION_POLL_MS: u64 = 100;
const OVERLAY_DECORATION_POLL_MS: u64 = 100;
const OVERLAY_DECORATION_IDLE_TIMEOUT_MS: u64 = 2_500;
const NOTIFICATION_TEXT_MARGIN_X: i32 = 14;
const NOTIFICATION_TEXT_MARGIN_Y: i32 = 12;
const NOTIFICATION_CLOSE_SIZE: u32 = 18;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
enum WindowDecoration {
    Normal = 0,
    CompactClose = 1,
    CompactCloseMinimize = 2,
    HiddenOverlay = 3,
}

impl WindowDecoration {
    fn from_flags(flags: u64) -> Self {
        match ((flags & SgpMsg::config_flags::DECORATION_MASK)
            >> SgpMsg::config_flags::DECORATION_SHIFT) as u8
        {
            1 => Self::CompactClose,
            2 => Self::CompactCloseMinimize,
            3 => Self::HiddenOverlay,
            _ => Self::Normal,
        }
    }

    const fn titlebar_height(self) -> u32 {
        match self {
            Self::Normal => TITLEBAR_H,
            Self::CompactClose | Self::CompactCloseMinimize | Self::HiddenOverlay => {
                COMPACT_TITLEBAR_H
            }
        }
    }
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

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
#[allow(dead_code)]
/// Future System Preferences hook for Window Behavior.
///
/// Keep existing shade/roll-up semantics wired until the compositor UI can
/// expose a stable user-facing setting.
enum TitlebarDoubleClickAction {
    MaximizeRestore = 0,
    Minimize = 1,
    WindowShade = 2,
    None = 3,
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
    Text = 9,
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
            9 => Self::Text,
            _ => Self::Pointer,
        }
    }
}

// ---------------------------------------------------------------------------
// WindowConfig — binary representation exchanged over IPC
//
// IPC encoding (CREATE_WINDOW / CONFIGURE_WINDOW):
//   words[0] = (client_w as u32) | ((client_h as u32) << 32)   [CREATE only]
//   words[1] = config_flags  (see SgpMsg::config_flags bit layout, including
//              decoration style/defaulting)
//   words[2] = (pid as u32)  | ((ppid as u32) << 32)
//   words[3] = title bytes [0..8]  (first 8 ASCII chars, LE u64)
//
// Only words[0..3] are transported via register IPC (IPC_REGISTER_WORDS = 4).
// A follow-up SGP_CONFIGURE_WINDOW with a SHM page at caps[0] provides the
// full null-terminated title (up to 255 chars) and extended group_ids.
// ---------------------------------------------------------------------------

#[allow(dead_code)]
#[derive(Clone, Copy)]
struct WindowConfig {
    title: [u8; 64],
    window_type: WindowType,
    state: WindowState,
    decoration: WindowDecoration,
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
            decoration: WindowDecoration::Normal,
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
        let decoration = WindowDecoration::from_flags(flags);
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
            decoration,
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
        self.title = [0; 64];
        let max = shm_len.min(63);
        for i in 0..max {
            let b = unsafe { shm_ptr.add(i).read() };
            if b == 0 {
                break;
            }
            self.title[i] = b;
        }
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

#[derive(Clone, Copy, Debug, PartialEq)]
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
// PointerPolicy — fixed-point cursor accumulation with a stateless,
// capped acceleration curve.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct PointerMotionConfig {
    sensitivity_fp: i32,
    acceleration_enabled: bool,
    acceleration_factor_fp: i32,
    accel_start_magnitude: i32,
    max_accel_gain_fp: i32,
    max_delta_fp: i32,
}

impl PointerMotionConfig {
    const fn moderate_default() -> Self {
        Self {
            sensitivity_fp: POINTER_SENSITIVITY_FP,
            acceleration_enabled: POINTER_ACCELERATION_ENABLED,
            acceleration_factor_fp: POINTER_ACCEL_FACTOR_FP,
            accel_start_magnitude: POINTER_ACCEL_START_MAGNITUDE,
            max_accel_gain_fp: POINTER_MAX_ACCEL_GAIN_FP,
            max_delta_fp: POINTER_MAX_DELTA_PX << FP_SHIFT,
        }
    }
}

#[derive(Clone, Copy)]
struct MotionOutcome {
    final_dx: i32,
    final_dy: i32,
    delta_capped: bool,
    position_clamped: bool,
}

struct PointerPolicy {
    x_fp: i32,
    y_fp: i32,
    buttons: u8,
    fb_width: u32,
    fb_height: u32,
    motion: PointerMotionConfig,
}

impl PointerPolicy {
    fn new(fb_w: u32, fb_h: u32) -> Self {
        let cx = ((fb_w as i32 / 2).max(0)) << FP_SHIFT;
        let cy = ((fb_h as i32 / 2).max(0)) << FP_SHIFT;
        Self {
            x_fp: cx,
            y_fp: cy,
            buttons: 0,
            fb_width: fb_w,
            fb_height: fb_h,
            motion: PointerMotionConfig::moderate_default(),
        }
    }

    fn acceleration_gain_fp(&self, magnitude: i32) -> i32 {
        // The curve is intentionally stateless: each batch is scaled only from
        // its current |dx|+|dy|. Small motion stays close to 1:1, while larger
        // sweeps get a modest linear boost up to a hard ceiling. Because the
        // gain uses no velocity history, movement stops immediately when fresh
        // hardware deltas stop.
        if !self.motion.acceleration_enabled || magnitude <= self.motion.accel_start_magnitude {
            return FP_ONE;
        }

        let extra = magnitude.saturating_sub(self.motion.accel_start_magnitude) as i64;
        let max_bonus = (self.motion.max_accel_gain_fp - FP_ONE).max(0) as i64;
        let bonus_fp = (extra * self.motion.acceleration_factor_fp as i64).min(max_bonus);
        (FP_ONE as i64 + bonus_fp) as i32
    }

    fn apply_motion(&mut self, dx: i32, dy: i32, buttons: u8) -> MotionOutcome {
        let prev_x = self.x();
        let prev_y = self.y();
        self.buttons = buttons;

        if dx == 0 && dy == 0 {
            return MotionOutcome {
                final_dx: 0,
                final_dy: 0,
                delta_capped: false,
                position_clamped: false,
            };
        }

        let magnitude = dx.abs().saturating_add(dy.abs());
        let accel_gain_fp = self.acceleration_gain_fp(magnitude) as i64;
        let total_gain_fp = ((self.motion.sensitivity_fp as i64) * accel_gain_fp) >> FP_SHIFT;
        let mut move_x_fp = (dx as i64) * total_gain_fp;
        let mut move_y_fp = (dy as i64) * total_gain_fp;

        let max_delta_fp = self.motion.max_delta_fp as i64;
        let mut delta_capped = false;
        if move_x_fp > max_delta_fp {
            move_x_fp = max_delta_fp;
            delta_capped = true;
        } else if move_x_fp < -max_delta_fp {
            move_x_fp = -max_delta_fp;
            delta_capped = true;
        }
        if move_y_fp > max_delta_fp {
            move_y_fp = max_delta_fp;
            delta_capped = true;
        } else if move_y_fp < -max_delta_fp {
            move_y_fp = -max_delta_fp;
            delta_capped = true;
        }

        self.x_fp = self.x_fp.saturating_add(move_x_fp as i32);
        self.y_fp = self.y_fp.saturating_add(move_y_fp as i32);
        let position_clamped = self.sync_clamp();

        MotionOutcome {
            final_dx: self.x() - prev_x,
            final_dy: self.y() - prev_y,
            delta_capped,
            position_clamped,
        }
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
    fn sync_clamp(&mut self) -> bool {
        let before_x = self.x_fp;
        let before_y = self.y_fp;
        self.x_fp = self.x_fp.clamp(self.min_fp(), self.max_x_fp());
        self.y_fp = self.y_fp.clamp(self.min_fp(), self.max_y_fp());
        self.x_fp != before_x || self.y_fp != before_y
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
    surface_width_pixels: u32,
    surface_height_rows: u32,
    surface_stride_bytes: usize,
    surface_len_bytes: usize,
    x: u32, // chrome top-left on screen
    y: u32,
    // Saved normal geometry for restore from maximized/fullscreen.
    saved_x: u32,
    saved_y: u32,
    saved_w: u32,
    saved_h: u32,
    parent_focus_window_id: u64,
    owner_pid: u64,
    /// Workspace assignment. Workspace switching UI is a later layer.
    workspace_id: u32,
    hidden: bool,
    config: WindowConfig,
    /// Cursor shape the client wants when the pointer is in its client area.
    client_cursor: CursorShape,
    pending_keys: KeyEventQueue,
    /// Last mouse state delivered to this window while it owned the pointer.
    /// Non-target windows keep their own cached state so they do not synthesize
    /// pointer transitions from someone else's clicks.
    last_mouse_x: u16,
    last_mouse_y: u16,
    last_buttons: u8,
    /// One-poll marker distinguishing a real focus click from focus gained by
    /// keyboard/window management while a physical button happens to be held.
    focus_press_pending: bool,
    /// When true the window is collapsed to its titlebar only (shade/roll-up).
    /// Client content is not blitted. A second titlebar double-click restores it.
    rolled_up: bool,
    /// Client height saved before rolling up so it can be restored on un-roll.
    saved_unrolled_h: u32,
    overlay_decorations_visible: bool,
    overlay_last_motion_ms: u64,
    overlay_pointer_inside: bool,
    first_present_logged: bool,
}

#[derive(Clone, Copy)]
struct LaunchTraceRecord {
    pid: u64,
    trace: LaunchTrace,
}

impl Window {
    fn decoration(&self) -> WindowDecoration {
        self.config.decoration
    }

    fn titlebar_height(&self) -> u32 {
        self.decoration().titlebar_height()
    }

    fn decorations_visible(&self) -> bool {
        self.decoration() != WindowDecoration::HiddenOverlay || self.overlay_decorations_visible
    }

    fn control_layout(&self) -> WindowControlLayout {
        match self.decoration() {
            WindowDecoration::Normal => WindowControlLayout::Normal,
            WindowDecoration::CompactClose => WindowControlLayout::CloseOnly,
            WindowDecoration::CompactCloseMinimize | WindowDecoration::HiddenOverlay => {
                WindowControlLayout::CloseMinimize
            }
        }
    }

    fn control_buttons(&self) -> &'static [WindowControlKind] {
        const NORMAL: &[WindowControlKind] = &[
            WindowControlKind::Pin,
            WindowControlKind::Minimize,
            WindowControlKind::Maximize,
            WindowControlKind::Close,
        ];
        const CLOSE_ONLY: &[WindowControlKind] = &[WindowControlKind::Close];
        const CLOSE_MINIMIZE: &[WindowControlKind] =
            &[WindowControlKind::Minimize, WindowControlKind::Close];

        match self.control_layout() {
            WindowControlLayout::Normal => NORMAL,
            WindowControlLayout::CloseOnly => CLOSE_ONLY,
            WindowControlLayout::CloseMinimize => CLOSE_MINIMIZE,
        }
    }

    fn client_origin(&self) -> (u32, u32) {
        (self.x + BORDER_W, self.y + self.titlebar_height())
    }

    /// Returns the chrome rectangle (x, y, w, h) accounting for state.
    fn chrome_rect(&self, fb_w: u32, fb_h: u32) -> (u32, u32, u32, u32) {
        match self.config.state {
            WindowState::Fullscreen => (0, 0, fb_w, fb_h),
            // Maximized windows are confined below the top panel so the panel
            // remains accessible and visually unobscured.
            WindowState::Maximized => (
                0,
                PANEL_TOP_RESERVED_H,
                fb_w,
                fb_h.saturating_sub(PANEL_TOP_RESERVED_H),
            ),
            _ => {
                // Rolled-up (shaded) windows collapse to titlebar + border only.
                let client_h = if self.rolled_up { 0 } else { self.height };
                (
                    self.x,
                    self.y,
                    self.width + BORDER_W * 2,
                    self.titlebar_height() + client_h + BORDER_W,
                )
            }
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

#[derive(Clone)]
struct Notification {
    id: u64,
    title: String,
    body: String,
    kind: NotificationKind,
    created_at: u64,
    timeout_ms: u64,
}

#[repr(C)]
struct NotificationWire {
    kind: u8,
    _pad0: [u8; 7],
    timeout_ms: u64,
    title_len: u32,
    body_len: u32,
    title: [u8; 96],
    body: [u8; 256],
}

#[derive(Clone, Copy)]
struct KeyboardState {
    pressed: [bool; 256],
    alt_tab_chord_active: bool,
    alt_tab_next_repeat_ms: u64,
}

impl KeyboardState {
    const fn new() -> Self {
        Self {
            pressed: [false; 256],
            alt_tab_chord_active: false,
            alt_tab_next_repeat_ms: 0,
        }
    }

    fn update_key(&mut self, keycode: u8, pressed: bool) -> bool {
        let idx = keycode as usize;
        let was_down = self.pressed[idx];
        self.pressed[idx] = pressed;
        was_down
    }

    fn is_down(&self, keycode: u8) -> bool {
        self.pressed[keycode as usize]
    }

    fn ctrl_down(&self) -> bool {
        self.is_down(KEY_CTRL)
    }

    fn alt_down(&self) -> bool {
        self.is_down(KEY_ALT)
    }

    fn super_down(&self) -> bool {
        self.is_down(KEY_LEFT_SUPER) || self.is_down(KEY_RIGHT_SUPER)
    }

    fn clear_alt_tab_repeat(&mut self) {
        self.alt_tab_chord_active = false;
        self.alt_tab_next_repeat_ms = 0;
    }
}

// ---------------------------------------------------------------------------
// CompositorState
// ---------------------------------------------------------------------------

struct CompositorState {
    windows: Vec<Window>,
    launch_traces: Vec<LaunchTraceRecord>,
    /// Current workspace. Workspace switcher / dock integration comes later.
    active_workspace_id: u32,
    mouse_x: u16,
    mouse_y: u16,
    pointer: PointerPolicy,
    keyboard: KeyboardState,
    active_drag: ActiveDrag,
    /// Pending window-move drag: (window_id, press_x, press_y).
    /// Set when a TitleBar click lands; promoted to ActiveDrag::Move once
    /// the cursor travels more than DRAG_THRESHOLD_PX from the press point.
    pending_move_drag: Option<(u64, i32, i32)>,
    /// Generic client-area drag capture. It only affects delivery to the
    /// originating window and always ends on final physical button release or
    /// focus loss; applications still decide whether a nested viewport uses it.
    client_pointer_capture: Option<u64>,
    prev_buttons: u8,
    #[allow(dead_code)]
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
    /// True once SET_SCANOUT has wired the VirtIO resource to the display.
    /// Kept false until the first SESSION_ACTIVATE so the VGA/TTY login
    /// screen stays visible during boot.
    virtio_scanout_enabled: bool,
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
    notifications: Vec<Notification>,
    next_notification_id: u64,
    /// True after spawning Vortex and before a Desktop-layer window appears.
    vortex_launch_pending: bool,
    last_mouse_generation: u32,
    /// Window ID that received the most recent titlebar click (for double-click detection).
    last_titlebar_click_win_id: u64,
    /// Monotonic timestamp (ms) of the most recent titlebar click.
    last_titlebar_click_ms: u64,
    /// Future user preference placeholder. Existing shade behavior stays wired.
    #[allow(dead_code)]
    titlebar_double_click_action: TitlebarDoubleClickAction,
    /// Application lifecycle tracker: maps pid -> AppInstance, enforces
    /// termination policy on last-window-close, and periodically sweeps
    /// zombie processes.
    app_tracker: app_lifecycle::AppTracker,
}

impl CompositorState {
    fn display_metrics(&self) -> DisplayMetrics {
        let backend = match self.display_backend {
            backend::DisplayBackend::VirtioGpu { .. } => ScreenBackend::VirtioGpu,
            backend::DisplayBackend::Limine { .. } => ScreenBackend::LimineFramebuffer,
        };
        DisplayMetrics::new(
            self.fb_width,
            self.fb_height,
            self.fb_pitch,
            PixelFormat::Xrgb8888,
            backend,
        )
    }
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
    raw_mouse_packet_count: u64,
    raw_dx_min: i32,
    raw_dx_max: i32,
    raw_dy_min: i32,
    raw_dy_max: i32,
    final_dx_min: i32,
    final_dx_max: i32,
    final_dy_min: i32,
    final_dy_max: i32,
    clamped_motion_count: u64,
    delta_capped_count: u64,
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
    /// Window move drags that started after exceeding the drag threshold.
    drag_started_count: u64,
    alt_tab_trigger_count: u64,
    alt_tab_repeat_count: u64,
    display_poll_count: u64,
    events_available_count: u64,
    wrong_window_poll_count: u64,
    pointer_other_window_count: u64,
}

impl DebugCounters {
    const fn new() -> Self {
        Self {
            mouse_event_count: 0,
            raw_mouse_packet_count: 0,
            raw_dx_min: i32::MAX,
            raw_dx_max: i32::MIN,
            raw_dy_min: i32::MAX,
            raw_dy_max: i32::MIN,
            final_dx_min: i32::MAX,
            final_dx_max: i32::MIN,
            final_dy_min: i32::MAX,
            final_dy_max: i32::MIN,
            clamped_motion_count: 0,
            delta_capped_count: 0,
            hw_cursor_move_count: 0,
            sw_cursor_move_count: 0,
            sw_cursor_redraw_count: 0,
            desktop_redraw_count: 0,
            dirty_rect_count: 0,
            full_present_count: 0,
            present_rect_count: 0,
            framebuffer_copy_count: 0,
            drag_started_count: 0,
            alt_tab_trigger_count: 0,
            alt_tab_repeat_count: 0,
            display_poll_count: 0,
            events_available_count: 0,
            wrong_window_poll_count: 0,
            pointer_other_window_count: 0,
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

/// Desktop/Widget windows (the shell's desktop surface: wallpaper, top panel,
/// dock) are workspace-agnostic and stay visible on every workspace. Normal and
/// Dialog windows are only visible on their owning workspace.
fn window_visible_on_workspace(win: &Window, active_workspace_id: u32) -> bool {
    win.config.window_type == WindowType::Desktop
        || win.config.window_type == WindowType::Widget
        || win.workspace_id == active_workspace_id
}

fn is_window_visible(state: &CompositorState, win: &Window) -> bool {
    !win.hidden
        && win.config.state != WindowState::Minimized
        && window_visible_on_workspace(win, state.active_workspace_id)
}

fn is_focusable_window(win: &Window) -> bool {
    win.config.state != WindowState::Minimized
        && win.config.window_type != WindowType::Desktop
        && win.config.window_type != WindowType::Widget
}

fn focused_window_idx(state: &CompositorState) -> Option<usize> {
    state
        .windows
        .iter()
        .rposition(|win| is_focusable_window(win) && is_window_visible(state, win))
}

fn focused_window_id(state: &CompositorState) -> Option<u64> {
    focused_window_idx(state).map(|idx| state.windows[idx].id)
}

fn event_poll_window_idx(state: &CompositorState, requested_id: u64) -> Option<usize> {
    state.windows.iter().position(|win| win.id == requested_id)
}

fn pointer_eligible_window(state: &CompositorState, win: &Window) -> bool {
    win.config.state != WindowState::Minimized
        && !win.hidden
        && window_visible_on_workspace(win, state.active_workspace_id)
}

fn topmost_window_idx_at(state: &CompositorState, cx: u32, cy: u32) -> Option<usize> {
    state
        .windows
        .iter()
        .enumerate()
        .rev()
        .find_map(|(idx, win)| {
            if !pointer_eligible_window(state, win) {
                return None;
            }
            let zone = hit_test_window(win, cx, cy, state.fb_width, state.fb_height);
            if zone == HitZone::Miss {
                None
            } else {
                Some(idx)
            }
        })
}

fn topmost_window_id_at(state: &CompositorState, cx: u32, cy: u32) -> Option<u64> {
    topmost_window_idx_at(state, cx, cy).map(|idx| state.windows[idx].id)
}

fn mouse_poll_words_for_window(state: &mut CompositorState, win_idx: usize) -> (u64, u64) {
    let mouse_x = state.mouse_x;
    let mouse_y = state.mouse_y;
    let target_id = topmost_window_id_at(&state, mouse_x as u32, mouse_y as u32);
    let focused_id = focused_window_id(state);
    if state.client_pointer_capture.is_some() && state.client_pointer_capture != focused_id {
        state.client_pointer_capture = None;
    }
    let captured_id = state.client_pointer_capture;
    let win = &mut state.windows[win_idx];
    let pointer_owned = target_id == Some(win.id) || captured_id == Some(win.id);
    let mut flags = 0;
    if focused_id == Some(win.id) {
        flags |= SgpMsg::EVENT_FLAG_FOCUSED;
    }
    if pointer_owned {
        flags |= SgpMsg::EVENT_FLAG_POINTER_OWNED;
    }
    if captured_id == Some(win.id) {
        flags |= SgpMsg::EVENT_FLAG_POINTER_CAPTURED;
    }
    if win.focus_press_pending {
        flags |= SgpMsg::EVENT_FLAG_FOCUS_PRESS;
        win.focus_press_pending = false;
    }
    if pointer_owned {
        win.last_mouse_x = mouse_x;
        win.last_mouse_y = mouse_y;
        win.last_buttons = state.prev_buttons;
        (
            (mouse_x as u64) | ((mouse_y as u64) << 16),
            state.prev_buttons as u64 | flags,
        )
    } else {
        (
            (win.last_mouse_x as u64) | ((win.last_mouse_y as u64) << 16),
            win.last_buttons as u64 | flags,
        )
    }
}

fn raise_window_by_id(state: &mut CompositorState, id: u64) {
    let Some(pos) = state.windows.iter().position(|w| w.id == id) else {
        return;
    };

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
    state.vortex_launch_pending = launch_vortex_shell(state);
}

fn notification_color(kind: NotificationKind) -> Color {
    match kind {
        NotificationKind::Info => Color(TITLEBAR_ACCENT),
        NotificationKind::Warning => Color(0x00FFC107),
        NotificationKind::Error => Color(0x00F44336),
    }
}

fn notification_text(bytes: &[u8], len: usize) -> String {
    let len = len.min(bytes.len());
    String::from_utf8_lossy(&bytes[..len]).into_owned()
}

fn notification_fit(text: &str, max_chars: usize) -> String {
    let chars = text.chars().count();
    if chars <= max_chars {
        return String::from(text);
    }
    let keep = max_chars.saturating_sub(3);
    let mut out = String::with_capacity(max_chars);
    for ch in text.chars().take(keep) {
        out.push(ch);
    }
    out.push('.');
    out.push('.');
    out.push('.');
    out
}

fn notification_toast_rect(canvas_w: u32, idx_from_top: usize) -> Option<Rect> {
    let max_width =
        NOTIFICATION_WIDTH.min(canvas_w.saturating_sub(2 * NOTIFICATION_MARGIN_X as u32));
    if max_width <= 120 {
        return None;
    }
    let x = canvas_w as i32 - NOTIFICATION_MARGIN_X - max_width as i32;
    let y = NOTIFICATION_MARGIN_Y
        + idx_from_top as i32 * (NOTIFICATION_HEIGHT as i32 + NOTIFICATION_GAP);
    Some(Rect::new(x, y, max_width, NOTIFICATION_HEIGHT))
}

fn notification_close_rect(toast: Rect) -> Rect {
    Rect::new(
        toast.right() - NOTIFICATION_CLOSE_SIZE as i32 - 8,
        toast.y + 8,
        NOTIFICATION_CLOSE_SIZE,
        NOTIFICATION_CLOSE_SIZE,
    )
}

fn dismiss_notification_at_point(state: &mut CompositorState, point: Point) -> bool {
    if state.notifications.is_empty() {
        return false;
    }
    let canvas_w = state.fb_width;
    let mut dismiss_idx = None;
    for (idx_from_top, note) in state.notifications.iter().rev().enumerate() {
        let Some(toast) = notification_toast_rect(canvas_w, idx_from_top) else {
            break;
        };
        let close = notification_close_rect(toast);
        if close.contains(point) {
            dismiss_idx = state
                .notifications
                .iter()
                .position(|candidate| candidate.id == note.id);
            break;
        }
        if toast.contains(point) {
            return true;
        }
    }
    if let Some(idx) = dismiss_idx {
        state.notifications.remove(idx);
        return true;
    }
    false
}

fn register_launch_trace(
    state: &mut CompositorState,
    launch_id: u64,
    source: LaunchSource,
    pid: u64,
    requested_at_ms: u64,
) {
    let trace = LaunchTrace::new(launch_id, source, requested_at_ms);
    if let Some(existing) = state
        .launch_traces
        .iter_mut()
        .find(|entry| entry.pid == pid)
    {
        existing.trace = trace;
        return;
    }
    state.launch_traces.push(LaunchTraceRecord { pid, trace });
}

fn trace_for_pid(state: &CompositorState, pid: u64) -> LaunchTrace {
    state
        .launch_traces
        .iter()
        .find(|entry| entry.pid == pid)
        .map(|entry| entry.trace)
        .unwrap_or_else(|| LaunchTrace::new(0, LaunchSource::Unknown, 0))
}

fn launch_source_from_u64(value: u64) -> LaunchSource {
    match value {
        1 => LaunchSource::Dock,
        2 => LaunchSource::Runner,
        3 => LaunchSource::Shortcut,
        4 => LaunchSource::Boot,
        _ => LaunchSource::Unknown,
    }
}

fn window_title_str(title: &[u8; 64]) -> &str {
    let len = title.iter().position(|&b| b == 0).unwrap_or(title.len());
    core::str::from_utf8(&title[..len]).unwrap_or("window")
}

fn push_notification(
    state: &mut CompositorState,
    kind: NotificationKind,
    title: String,
    body: String,
    timeout_ms: u64,
) {
    if state.notifications.len() >= NOTIFICATION_MAX_COUNT {
        state.notifications.remove(0);
    }
    state.notifications.push(Notification {
        id: state.next_notification_id,
        title,
        body,
        kind,
        created_at: monotonic_millis(),
        timeout_ms,
    });
    state.next_notification_id = state.next_notification_id.saturating_add(1);
}

fn prune_notifications(state: &mut CompositorState, now: u64) -> bool {
    let before = state.notifications.len();
    state
        .notifications
        .retain(|n| now.saturating_sub(n.created_at) < n.timeout_ms);
    before != state.notifications.len()
}

fn ingest_notification(state: &mut CompositorState, msg: &IpcMsg) {
    if msg.cap_count == 0 || msg.caps[0] == CapabilityToken::INVALID {
        return;
    }
    let Ok(ptr) = sunlight_ipc::shm_map(msg.caps[0]) else {
        return;
    };

    let wire = unsafe { &*(ptr as *const NotificationWire) };
    let title_len = (wire.title_len as usize).min(wire.title.len());
    let body_len = (wire.body_len as usize).min(wire.body.len());
    let title = notification_text(&wire.title, title_len);
    let body = notification_text(&wire.body, body_len);
    let kind = match wire.kind {
        1 => NotificationKind::Warning,
        2 => NotificationKind::Error,
        _ => NotificationKind::Info,
    };
    let timeout_ms = if wire.timeout_ms == 0 {
        NOTIFICATION_TIMEOUT_MS
    } else {
        wire.timeout_ms
            .clamp(NOTIFICATION_MIN_TIMEOUT_MS, NOTIFICATION_TIMEOUT_MS)
    };
    push_notification(state, kind, title, body, timeout_ms);
}

fn draw_notifications(canvas: &mut Canvas<'_>, state: &CompositorState) {
    if state.notifications.is_empty() {
        return;
    }

    let max_width = NOTIFICATION_WIDTH.min(
        canvas
            .width
            .saturating_sub(2 * NOTIFICATION_MARGIN_X as u32),
    );
    if max_width <= 120 {
        return;
    }

    let text_width = max_width as i32 - NOTIFICATION_TEXT_MARGIN_X * 2 - 28;
    let title_max_chars = (text_width / 7).max(8) as usize;
    let body_max_chars = ((max_width as i32 - NOTIFICATION_TEXT_MARGIN_X * 2) / 7).max(12) as usize;
    let title_y = NOTIFICATION_TEXT_MARGIN_Y;
    let body_y = title_y + 22;
    let time_y = body_y + 22;
    let close_icon = TgaImage::parse(ICON_SYM_CLOSE_TGA).ok();

    for (idx, note) in state.notifications.iter().rev().enumerate() {
        let Some(rect) = notification_toast_rect(canvas.width, idx) else {
            break;
        };
        if rect.y >= canvas.height as i32 {
            break;
        }

        let _ = note.id;
        let accent = notification_color(note.kind);
        let panel = if idx == 0 {
            Color(0x001E1E26)
        } else {
            Color(0x00181820)
        };
        let border = accent.darken(30);
        canvas.fill_rounded_rect(rect, 8, panel);
        canvas.stroke_rounded_rect(rect, 8, 1, border);
        canvas.fill_rect(Rect::new(rect.x, rect.y, 5, rect.h), accent);
        let close = notification_close_rect(rect);
        canvas.fill_rounded_rect(close, 4, Color(0x002A2A34));
        canvas.stroke_rounded_rect(close, 4, 1, border);
        if let Some(icon) = close_icon {
            canvas.draw_tga_icon_tinted(&icon, close, Color(0x00CCCCD8));
        } else {
            canvas.draw_text(close.x + 5, close.y + 3, "x", Color(0x00CCCCD8));
        }

        let title = notification_fit(&note.title, title_max_chars);
        let body = notification_fit(&note.body, body_max_chars);
        sun_font::draw_text(
            canvas,
            &title,
            rect.x + NOTIFICATION_TEXT_MARGIN_X,
            rect.y + title_y,
            &sun_font::TextStyle::new(sun_font::FontRole::UiMedium, Color(TITLE_TEXT_COLOR)),
        );
        sun_font::draw_text(
            canvas,
            &body,
            rect.x + NOTIFICATION_TEXT_MARGIN_X,
            rect.y + body_y,
            &sun_font::TextStyle::new(sun_font::FontRole::UiSmall, Color(0x00AAAAB6)),
        );
        sun_font::draw_text(
            canvas,
            "just now",
            rect.x + NOTIFICATION_TEXT_MARGIN_X,
            rect.y + time_y,
            &sun_font::TextStyle::new(sun_font::FontRole::UiSmall, Color(0x00747482)),
        );
    }
}

fn cycle_focus(state: &mut CompositorState) -> bool {
    let Some(focused_idx) = focused_window_idx(state) else {
        return false;
    };
    let z_group = state.windows[focused_idx].config.z_index_type;
    let active_workspace_id = state.active_workspace_id;
    let candidate_count = state
        .windows
        .iter()
        .filter(|win| {
            is_focusable_window(win)
                && !win.hidden
                && win.workspace_id == active_workspace_id
                && win.config.z_index_type == z_group
        })
        .count();
    if candidate_count < 2 {
        return false;
    }

    let win = state.windows.remove(focused_idx);
    let insert_idx = state
        .windows
        .iter()
        .position(|other| {
            is_focusable_window(other)
                && !other.hidden
                && other.workspace_id == active_workspace_id
                && other.config.z_index_type == z_group
        })
        .unwrap_or(state.windows.len());
    state.windows.insert(insert_idx, win);
    true
}

#[derive(Clone, Copy)]
enum AltTabTriggerSource {
    Keydown,
    Repeat,
}

fn trigger_alt_tab(state: &mut CompositorState, source: AltTabTriggerSource) {
    if cycle_focus(state) {
        state.active_cursor = cursor_for_scene(state);
        mark_dirty_full(state);
        redraw_scene(state);
    }

    state.debug_counters.alt_tab_trigger_count += 1;
    if matches!(source, AltTabTriggerSource::Repeat) {
        state.debug_counters.alt_tab_repeat_count += 1;
    }

    if INPUT_DEBUG {
        debug_log("[DISPLAY] alt_tab source=");
        debug_log(match source {
            AltTabTriggerSource::Keydown => "keydown",
            AltTabTriggerSource::Repeat => "repeat",
        });
        debug_log(" count=");
        debug_dec_u64(state.debug_counters.alt_tab_trigger_count);
        debug_log("\n");
    }
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
    if state.client_pointer_capture == Some(win_id) {
        state.client_pointer_capture = None;
    }
    let was_desktop = win.config.window_type == WindowType::Desktop;
    let restore_focus_id = if win.config.window_type == WindowType::Dialog {
        (win.parent_focus_window_id != 0).then_some(win.parent_focus_window_id)
    } else {
        None
    };

    let _ = sunlight_ipc::shm_free(win.shm_cap);

    let now = monotonic_millis();
    let action = state.app_tracker.unregister_window(win_id, now);
    match action {
        app_lifecycle::AppAction::Terminate(pid) => {
            let _ = kill(pid, 15);
        }
        app_lifecycle::AppAction::None => {}
        app_lifecycle::AppAction::SystemProtected => {}
    }

    let cancel = match &state.active_drag {
        ActiveDrag::Move(d) => d.window_id == win_id,
        ActiveDrag::Resize(d) => d.window_id == win_id,
        ActiveDrag::None => false,
    };
    if cancel {
        state.active_drag = ActiveDrag::None;
    }

    state.active_cursor = cursor_for_scene(state);
    if let Some(parent_id) = restore_focus_id {
        let _ = activate_window(state, parent_id);
    }
    if was_desktop {
        state.vortex_launch_pending = false;
        if state.session_active {
            ensure_vortex_shell(state);
        }
    }
    true
}

fn sweep_app_zombies(state: &mut CompositorState, now: u64) -> bool {
    if now < state.app_tracker.next_zombie_sweep_ms {
        return false;
    }
    let result = state.app_tracker.sweep_zombies(now);
    let mut dirty = false;
    for win_id in &result.window_ids_to_cleanup {
        close_window(state, *win_id, None);
        dirty = true;
    }
    for _pid in &result.pids_killed {
        debug_log(&alloc::format!(
            "[APP_LIFECYCLE] zombie_app_reaped pid={}\n",
            _pid
        ));
    }
    dirty
}

/// Prune any windows whose owner_pid no longer has a live process.
/// This ensures that on crash or external kill, remaining windows/surfaces
/// for that pid are removed even if app_tracker missed an instance.
/// Minimized windows are left alone if the process is still alive.
fn prune_dead_owner_windows(state: &mut CompositorState) -> bool {
    // Snapshot pids that have windows.
    let owner_pids: alloc::vec::Vec<u64> = state
        .windows
        .iter()
        .map(|w| w.owner_pid)
        .collect::<alloc::collections::BTreeSet<_>>()
        .into_iter()
        .collect();

    let mut to_close: alloc::vec::Vec<u64> = alloc::vec::Vec::new();
    for pid in &owner_pids {
        if *pid <= 1 {
            continue; // system/boot pids
        }
        if !sunlight_ipc::process_is_alive(*pid) {
            for w in state.windows.iter() {
                if w.owner_pid == *pid {
                    to_close.push(w.id);
                }
            }
        }
    }

    if to_close.is_empty() {
        // Still prune tracker instances for dead pids even with no remaining windows.
        let mut did_remove = false;
        for pid in &owner_pids {
            if *pid > 1 && !sunlight_ipc::process_is_alive(*pid) {
                if state.app_tracker.remove_instance_for_pid(*pid) {
                    did_remove = true;
                }
            }
        }
        return did_remove;
    }

    let mut dirty = false;
    let count = to_close.len();
    for id in to_close {
        let _ = close_window(state, id, None);
        dirty = true;
    }
    if count > 0 {
        debug_log(&alloc::format!(
            "[DISPLAY] display_windows_reaped_for_process count={} (dead_owner_prune)\n",
            count
        ));
    }
    // Also drop any AppInstance entries for dead pids so future sweeps are clean.
    for pid in &owner_pids {
        if *pid <= 1 {
            continue;
        }
        if !sunlight_ipc::process_is_alive(*pid) {
            let _ = state.app_tracker.remove_instance_for_pid(*pid);
        }
    }
    dirty
}

fn list_window_reply(win: &Window, active_ws: u32) -> IpcMsg {
    let rolled_up = if win.rolled_up { 1u64 } else { 0u64 };
    let window_type = win.config.window_type as u64;
    let state = win.config.state as u64;
    let mut title0 = 0u64;
    let mut title1 = 0u64;
    for i in 0..8usize {
        title0 |= (win.config.title[i] as u64) << (i * 8);
    }
    for i in 0..8usize {
        title1 |= (win.config.title[8 + i] as u64) << (i * 8);
    }
    IpcMsg::with_label(SgpMsg::REPLY)
        .word(0, win.id)
        .word(1, win.owner_pid)
        .word(2, state)
        .word(
            3,
            window_type | (rolled_up << 8) | ((active_ws as u64) << 16),
        )
        .word(4, win.workspace_id as u64)
        .word(5, win.hidden as u64)
        .word(6, title0)
        .word(7, title1)
}

fn list_window_at(state: &CompositorState, idx: usize) -> IpcMsg {
    state
        .windows
        .get(idx)
        .map(|win| list_window_reply(win, state.active_workspace_id))
        .unwrap_or_else(|| IpcMsg::with_label(SgpMsg::REPLY))
}

fn activate_window(state: &mut CompositorState, win_id: u64) -> bool {
    let Some(pos) = state.windows.iter().position(|w| w.id == win_id) else {
        return false;
    };

    if state.windows[pos].config.state == WindowState::Minimized {
        state.windows[pos].config.state = WindowState::Normal;
    }

    raise_window_by_id(state, win_id);
    state.active_cursor = cursor_for_scene(state);
    mark_dirty_full(state);
    redraw_scene(state);
    true
}

// ---------------------------------------------------------------------------
// Chrome constants (Vortex Shell Theme)
// ---------------------------------------------------------------------------

const DESKTOP_COLOR: u32 = 0x00121214; // Deep dark gray/black
const TITLEBAR_H: u32 = 32; // Taller for Chrome-tab style
const COMPACT_TITLEBAR_H: u32 = 24;
const TITLEBAR_COLOR: u32 = 0x002B2B36; // inactive titlebar (dark slate)
const TITLEBAR_ACTIVE: u32 = 0x001E1E26; // active window titlebar (darker base)
const TITLEBAR_ACCENT: u32 = 0x00FF7A00; // Warm/Orange accent line
const TITLE_TEXT_COLOR: u32 = 0x00E0E0E0; // Off-white
const BORDER_W: u32 = 1;
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

// Top panel reserved height: Vortex Shell top-bar occupies approximately
// y=6..42 (TOP_Y=6, TOP_H=36). We reserve 50px so normal windows cannot
// be placed or dragged over the panel, and maximized windows start below it.
// Fullscreen windows are intentionally exempt (they cover everything).
const PANEL_TOP_RESERVED_H: u32 = 50;

// Maximum time between two titlebar clicks to be treated as a double-click
// (roll-up / shade gesture, like old FVWM/CDE desktops).
const DOUBLE_CLICK_MS: u64 = 400;

// TODO(snap): Super+Left / Super+Right window snapping is planned but currently
// disabled because the app-side resize protocol is not yet robust — sending a
// new window size without a handshake causes some apps to hang or break their
// layout. Re-enable once resize is negotiated properly (e.g. a RESIZE_ACK IPC
// round-trip or the client polls the new geometry on the next EVENT_POLL).
//
// Bottom panel: Vortex Shell's bottom dock panels are drawn as part of the
// Desktop-layer window and therefore sit behind normal app windows in z-order.
// This is acceptable for now; a future Widget-type overlay panel would fix it.

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

#[derive(Clone, Copy, PartialEq, Eq)]
enum WindowControlLayout {
    Normal,
    CloseOnly,
    CloseMinimize,
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
// Cursor sprites — 32×32 TGA assets (type 2, 32bpp BGRA, top-down) generated
// from docs/images/cursors/*.svg by tools/gen_cursors.sh into assets/cursors/.
// Each shape has a hotspot: the sprite pixel that sits on the mouse coordinate.
// ---------------------------------------------------------------------------

const CURSOR_W: usize = 32;
const CURSOR_H: usize = 32;

static CURSOR_TGA_POINTER: &[u8] = include_bytes!("../../../assets/cursors/pointer.tga");
static CURSOR_TGA_HAND: &[u8] = include_bytes!("../../../assets/cursors/hand.tga");
static CURSOR_TGA_RESIZE_H: &[u8] = include_bytes!("../../../assets/cursors/resize-h.tga");
static CURSOR_TGA_RESIZE_V: &[u8] = include_bytes!("../../../assets/cursors/resize-v.tga");
static CURSOR_TGA_RESIZE_NWSE: &[u8] = include_bytes!("../../../assets/cursors/resize-nwse.tga");
static CURSOR_TGA_RESIZE_NESW: &[u8] = include_bytes!("../../../assets/cursors/resize-nesw.tga");
static CURSOR_TGA_MOVE: &[u8] = include_bytes!("../../../assets/cursors/move.tga");
static CURSOR_TGA_WAIT: &[u8] = include_bytes!("../../../assets/cursors/wait.tga");
static CURSOR_TGA_QUESTION: &[u8] = include_bytes!("../../../assets/cursors/question.tga");
static CURSOR_TGA_TEXT: &[u8] = include_bytes!("../../../assets/cursors/text.tga");

/// TGA bytes and hotspot (x, y) for a cursor shape.
fn cursor_asset(shape: CursorShape) -> (&'static [u8], i32, i32) {
    match shape {
        CursorShape::Pointer => (CURSOR_TGA_POINTER, 3, 2),
        CursorShape::Hand => (CURSOR_TGA_HAND, 14, 3),
        CursorShape::ResizeH => (CURSOR_TGA_RESIZE_H, 16, 16),
        CursorShape::ResizeV => (CURSOR_TGA_RESIZE_V, 16, 16),
        CursorShape::ResizeCornerNW => (CURSOR_TGA_RESIZE_NWSE, 16, 16),
        CursorShape::ResizeCornerNE => (CURSOR_TGA_RESIZE_NESW, 16, 16),
        CursorShape::Moving => (CURSOR_TGA_MOVE, 16, 16),
        CursorShape::Waiting => (CURSOR_TGA_WAIT, 16, 16),
        CursorShape::Question => (CURSOR_TGA_QUESTION, 2, 2),
        CursorShape::Text => (CURSOR_TGA_TEXT, 16, 15),
    }
}

/// Parsed sprite + hotspot for a shape. TGA parsing only reads the 18-byte
/// header (pixels decode on demand), so parsing per call is cheap.
fn cursor_sprite(shape: CursorShape) -> Option<(TgaImage, i32, i32)> {
    let (bytes, hx, hy) = cursor_asset(shape);
    TgaImage::parse(bytes).ok().map(|img| (img, hx, hy))
}

/// Top-left screen position of the sprite for a given mouse position.
fn cursor_origin(shape: CursorShape, cx: u32, cy: u32) -> (i32, i32) {
    let (_, hx, hy) = cursor_asset(shape);
    (cx as i32 - hx, cy as i32 - hy)
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
        backend::DisplayBackend::Limine { fb, pitch_words } => {
            state.debug_counters.framebuffer_copy_count += 1;
            let stride = *pitch_words;
            let fw = state.fb_width as usize;
            let fh = state.fb_height as usize;
            for y in 0..fh {
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        state.back_buffer.as_ptr().add(y * stride),
                        fb.add(y * stride),
                        fw,
                    );
                }
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

/// Dirty rect that covers any cursor sprite placement around (cx, cy).
/// Hotspots lie inside the sprite, so every possible placement (old or new
/// shape) fits in a (2W+2)×(2H+2) box centred on the hotspot pixel; using the
/// conservative box keeps this correct across shape changes without needing
/// to know which shape was drawn previously.
fn cursor_dirty_rect(cx: u32, cy: u32) -> Rect {
    Rect::new(
        cx as i32 - CURSOR_W as i32 - 1,
        cy as i32 - CURSOR_H as i32 - 1,
        CURSOR_W as u32 * 2 + 2,
        CURSOR_H as u32 * 2 + 2,
    )
}

/// On-screen rect the active cursor sprite occupies for mouse position
/// (cx, cy), offset by the shape's hotspot and clipped to the screen.
fn cursor_rect(state: &CompositorState, cx: u32, cy: u32) -> Option<Rect> {
    let (ox, oy) = cursor_origin(state.active_cursor, cx, cy);
    let r = clip_to_screen(
        Rect::new(ox, oy, CURSOR_W as u32, CURSOR_H as u32),
        state.fb_width,
        state.fb_height,
    );
    if r.w == 0 || r.h == 0 {
        None
    } else {
        Some(r)
    }
}

fn title_len(title: &[u8; 64]) -> usize {
    title.iter().position(|&b| b == 0).unwrap_or(title.len())
}

fn control_rect(wx: u32, wy: u32, chrome_w: u32, titlebar_h: u32, slot_from_right: u32) -> Rect {
    let x = wx + chrome_w.saturating_sub((BTN_SIZE + BTN_SPACING) * (slot_from_right + 1));
    let y = wy as i32 + (titlebar_h.saturating_sub(BTN_SIZE)) as i32 / 2;
    Rect::new(x as i32, y, BTN_SIZE, BTN_SIZE)
}

fn control_rect_for_kind(
    win: &Window,
    wx: u32,
    wy: u32,
    chrome_w: u32,
    control: WindowControlKind,
) -> Option<Rect> {
    let titlebar_h = win.titlebar_height();
    let controls = win.control_buttons();
    controls
        .iter()
        .position(|kind| *kind == control)
        .map(|index| {
            let slot_from_right = controls.len().saturating_sub(1).saturating_sub(index) as u32;
            control_rect(wx, wy, chrome_w, titlebar_h, slot_from_right)
        })
}

fn draw_title(canvas: &mut Canvas<'_>, title: &[u8; 64], rect: Rect, color: Color) {
    let len = title_len(title);
    if len == 0 {
        return;
    }
    let s = core::str::from_utf8(&title[..len]).unwrap_or("");
    let style = sun_font::TextStyle::new(sun_font::FontRole::UiMedium, color);
    sun_font::draw_text_vcenter(canvas, s, rect.x + 4, rect.y, rect.h, &style);
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
    let titlebar_h = win.titlebar_height();

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
        if win.decorations_visible() && rel_y < titlebar_h {
            let point = sunlight_ui::Point::new(cx as i32, cy as i32);

            for control in win.control_buttons() {
                if let Some(rect) = control_rect_for_kind(win, wx, wy, chrome_w, *control) {
                    if rect.contains(point) {
                        return match control {
                            WindowControlKind::Close => HitZone::CloseBtn,
                            WindowControlKind::Maximize | WindowControlKind::Restore => {
                                HitZone::MaximizeBtn
                            }
                            WindowControlKind::Minimize => HitZone::MinimizeBtn,
                            WindowControlKind::Pin => HitZone::KeepOnTopBtn,
                            WindowControlKind::Help => HitZone::TitleBar,
                        };
                    }
                }
            }

            return HitZone::TitleBar;
        }
    }

    // If no border, everything below the title bar is client area.
    if no_border {
        return HitZone::ClientArea;
    }

    // Rolled-up windows have no visible client area; treat any hit below the
    // titlebar strip (e.g. the 2-px border bottom) as titlebar so the user
    // can still drag, and suppress all edge-resize zones.
    if win.rolled_up {
        return HitZone::TitleBar;
    }

    // Corner zones (checked before edge zones — larger grab target wins).
    let corner_size = RESIZE_BORDER + 4;
    let bottom_zone = rel_y >= titlebar_h + win.height.saturating_sub(corner_size);

    if bottom_zone {
        if rel_x < corner_size {
            return HitZone::CornerBL;
        }
        if rel_x >= chrome_w.saturating_sub(corner_size) {
            return HitZone::CornerBR;
        }
        if rel_y >= titlebar_h + win.height {
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
    if rel_y >= titlebar_h + win.height {
        return HitZone::EdgeBottom;
    }

    HitZone::ClientArea
}

/// Compute the cursor shape to display given all visible windows and pointer pos.
fn cursor_for_scene(state: &CompositorState) -> CursorShape {
    let cx = state.mouse_x as u32;
    let cy = state.mouse_y as u32;

    let Some(idx) = topmost_window_idx_at(state, cx, cy) else {
        return CursorShape::Pointer;
    };
    let win = &state.windows[idx];
    let zone = hit_test_window(win, cx, cy, state.fb_width, state.fb_height);
    match zone {
        HitZone::ClientArea => win.client_cursor,
        other => other.default_cursor(),
    }
}

fn refresh_cursor_after_scene_change(state: &mut CompositorState) {
    let old_cursor = state.active_cursor;
    state.active_cursor = cursor_for_scene(state);
    if old_cursor == state.active_cursor {
        return;
    }
    let cursor_rect = cursor_dirty_rect(state.mouse_x as u32, state.mouse_y as u32);
    if state.hw_cursor_active {
        if !upload_hw_cursor_if_needed(state) {
            mark_dirty_rect(state, cursor_rect);
        }
    } else {
        mark_dirty_rect(state, cursor_rect);
    }
}

fn overlay_decoration_timeout_pending(state: &CompositorState) -> bool {
    state.windows.iter().any(|win| {
        win.decoration() == WindowDecoration::HiddenOverlay
            && !win.hidden
            && win.config.state != WindowState::Minimized
    })
}

fn update_overlay_window_visibility(
    state: &mut CompositorState,
    now: u64,
    pointer_moved: bool,
    pointer_pressed: bool,
) -> bool {
    let cx = state.mouse_x as u32;
    let cy = state.mouse_y as u32;
    let mut changed = false;

    for idx in 0..state.windows.len() {
        let is_overlay = state.windows[idx].decoration() == WindowDecoration::HiddenOverlay;
        if !is_overlay {
            continue;
        }

        let inside = {
            let win = &state.windows[idx];
            pointer_eligible_window(state, win)
                && hit_test_window(win, cx, cy, state.fb_width, state.fb_height) != HitZone::Miss
        };

        let dirty_rect = {
            let win = &mut state.windows[idx];
            let mut next_visible = win.overlay_decorations_visible;
            if inside && (!win.overlay_pointer_inside || pointer_moved || pointer_pressed) {
                win.overlay_last_motion_ms = now;
                next_visible = true;
            } else if !inside && win.overlay_pointer_inside {
                next_visible = false;
            } else if inside
                && win.overlay_decorations_visible
                && now.saturating_sub(win.overlay_last_motion_ms)
                    >= OVERLAY_DECORATION_IDLE_TIMEOUT_MS
            {
                next_visible = false;
            }

            let rect = if next_visible != win.overlay_decorations_visible {
                win.overlay_decorations_visible = next_visible;
                let (wx, wy, ww, wh) = win.chrome_rect(state.fb_width, state.fb_height);
                Some(Rect::new(wx as i32, wy as i32, ww, wh))
            } else {
                None
            };
            win.overlay_pointer_inside = inside;
            rect
        };
        if let Some(rect) = dirty_rect {
            mark_dirty_rect(state, rect);
            changed = true;
        }
    }

    if changed {
        refresh_cursor_after_scene_change(state);
    }
    changed
}

fn compositor_poll_timeout_ms(state: &CompositorState) -> Option<u64> {
    let now = monotonic_millis();
    let notification_timeout = state
        .notifications
        .iter()
        .map(|note| {
            let age = now.saturating_sub(note.created_at);
            note.timeout_ms
                .saturating_sub(age)
                .min(NOTIFICATION_POLL_MS)
        })
        .min();
    let overlay_timeout = if overlay_decoration_timeout_pending(state) {
        Some(OVERLAY_DECORATION_POLL_MS)
    } else {
        None
    };
    let base = match (notification_timeout, overlay_timeout) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    };
    // Always wake up periodically to run the zombie app sweeper.
    match base {
        Some(ms) => Some(ms.min(500)),
        None => Some(500),
    }
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
    if win.buffer.is_null() || !is_window_visible(state, win) {
        return;
    }

    let fullscreen = win.config.state == WindowState::Fullscreen;
    let maximized = win.config.state == WindowState::Maximized;
    let no_chrome = fullscreen || win.config.border == BorderStyle::None;
    let titlebar_h = win.titlebar_height();

    // Rolled-up windows render chrome only; skip client blit entirely.
    let skip_client = win.rolled_up && !fullscreen && !maximized;

    let (canvas_x, canvas_y, client_w, client_h) = if fullscreen {
        // Fullscreen: truly full-screen, covers panel intentionally.
        let cw = if no_chrome {
            state.fb_width
        } else {
            state.fb_width.saturating_sub(BORDER_W * 2)
        };
        let ch = if no_chrome {
            state.fb_height
        } else {
            state.fb_height.saturating_sub(titlebar_h + BORDER_W)
        };
        let ox = if no_chrome { 0 } else { BORDER_W };
        let oy = if no_chrome { 0 } else { titlebar_h };
        (ox, oy, cw, ch)
    } else if maximized {
        // Maximized: confined below the top panel so it doesn't cover the shell bar.
        let cw = state.fb_width.saturating_sub(BORDER_W * 2);
        let ch = state
            .fb_height
            .saturating_sub(PANEL_TOP_RESERVED_H)
            .saturating_sub(titlebar_h + BORDER_W);
        (BORDER_W, PANEL_TOP_RESERVED_H + titlebar_h, cw, ch)
    } else {
        (win.x + BORDER_W, win.y + titlebar_h, win.width, win.height)
    };

    if !no_chrome {
        let (wx, wy) = if maximized {
            (0u32, PANEL_TOP_RESERVED_H)
        } else {
            (win.x, win.y)
        };
        let chrome_w = if maximized {
            state.fb_width
        } else {
            win.width + BORDER_W * 2
        };
        let chrome_h = if maximized {
            state.fb_height.saturating_sub(PANEL_TOP_RESERVED_H)
        } else if win.rolled_up {
            titlebar_h + BORDER_W
        } else {
            titlebar_h + win.height + BORDER_W
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
            inner.y
                + if win.decorations_visible() {
                    titlebar_h as i32
                } else {
                    0
                },
            inner.w,
            inner.h.saturating_sub(if win.decorations_visible() {
                titlebar_h
            } else {
                0
            }),
        );
        let controls = win.control_buttons();
        let control_strip_w = if controls.is_empty() {
            0
        } else {
            (BTN_SIZE + BTN_SPACING) * controls.len() as u32
        };
        let title_rect = Rect::new(
            wx as i32 + 12,
            wy as i32 + 1,
            chrome_w.saturating_sub(control_strip_w + 24),
            titlebar_h.saturating_sub(2),
        );

        {
            let mut canvas = back_buffer_canvas(state, back_buffer);
            canvas.fill_top_rounded_rect(outer, outer_radius, Color(bd_color));
            canvas.fill_top_rounded_rect(
                inner,
                inner_radius,
                Color(if win.decorations_visible() {
                    tb_color
                } else {
                    0xFF1A1A20
                }),
            );
            canvas.fill_rect(content_backdrop, Color(0xFF1A1A20));
            if win.decorations_visible() {
                for control in controls {
                    let Some(rect) = control_rect_for_kind(win, wx, wy, chrome_w, *control) else {
                        continue;
                    };
                    let (zone, accent_active, draw_kind) = match control {
                        WindowControlKind::Pin => (
                            HitZone::KeepOnTopBtn,
                            win.config.z_index_type == ZIndexType::OnTop,
                            *control,
                        ),
                        WindowControlKind::Minimize => {
                            (HitZone::MinimizeBtn, false, WindowControlKind::Minimize)
                        }
                        WindowControlKind::Maximize | WindowControlKind::Restore => (
                            HitZone::MaximizeBtn,
                            maximized,
                            if maximized {
                                WindowControlKind::Restore
                            } else {
                                WindowControlKind::Maximize
                            },
                        ),
                        WindowControlKind::Close => {
                            (HitZone::CloseBtn, is_focused, WindowControlKind::Close)
                        }
                        WindowControlKind::Help => continue,
                    };
                    draw_window_control(
                        &mut canvas,
                        rect,
                        draw_kind,
                        Color(icon_col),
                        Color(tb_color),
                        hover_zone == zone,
                        accent_active,
                    );
                }
                draw_title(
                    &mut canvas,
                    &win.config.title,
                    title_rect,
                    Color(TITLE_TEXT_COLOR),
                );
            }
        }
    }

    // Blit client buffer — skipped for rolled-up (shaded) windows.
    if skip_client {
        return;
    }
    if canvas_x >= state.fb_width || canvas_y >= state.fb_height {
        return;
    }
    let copy_w = client_w.min(state.fb_width - canvas_x) as usize;
    let copy_h = client_h.min(state.fb_height - canvas_y) as usize;
    if copy_w == 0 || copy_h == 0 {
        return;
    }
    let Ok(layout) = surface::SurfaceLayout::validate(
        win.surface_width_pixels,
        win.surface_height_rows,
        win.surface_stride_bytes,
        win.surface_len_bytes,
    ) else {
        return;
    };
    let Ok(source) = layout.readable_rect(0, 0, copy_w as u32, copy_h as u32) else {
        return;
    };
    if source.width == 0 || source.height == 0 {
        return;
    }
    let copy_w = source.width as usize;
    let copy_h = source.height as usize;
    let stride = fb_stride(state);
    let back_ptr = back_buffer.as_mut_ptr();
    // Inner content area used for anti-aliased rounded-corner clipping.
    let clip_rect = if !no_chrome && !maximized {
        Some(Rect::new(
            win.x as i32 + BORDER_W as i32,
            win.y as i32 + BORDER_W as i32,
            win.width,
            titlebar_h + win.height,
        ))
    } else {
        None
    };
    for row in 0..copy_h {
        unsafe {
            let src_row = win
                .buffer
                .add((source.y as usize + row) * layout.stride_pixels + source.x as usize);
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
                                .write(blend::blend_xrgb_with_coverage(src_px, dst_px, a));
                        }
                    }
                }
            } else {
                core::ptr::copy_nonoverlapping(src_row, dst_row, copy_w);
            }
        }
    }
}

/// Alpha-blend the cursor sprite for `shape` into the back buffer with its
/// hotspot on the mouse position (mouse_x, mouse_y).
fn draw_cursor_sprite(
    state: &CompositorState,
    back_buffer: &mut [u32],
    shape: CursorShape,
    mouse_x: u32,
    mouse_y: u32,
) {
    let Some((img, hx, hy)) = cursor_sprite(shape) else {
        return;
    };
    let base_x = mouse_x as i32 - hx;
    let base_y = mouse_y as i32 - hy;
    let stride = fb_stride(state);
    let w = img.width.min(CURSOR_W as u32);
    let h = img.height.min(CURSOR_H as u32);

    for row in 0..h {
        let y = base_y + row as i32;
        if y < 0 || y >= state.fb_height as i32 {
            continue;
        }
        for col in 0..w {
            let x = base_x + col as i32;
            if x < 0 || x >= state.fb_width as i32 {
                continue;
            }
            let src = img.pixel_argb(col, row);
            let a = src >> 24;
            if a == 0 {
                continue;
            }
            let idx = y as usize * stride + x as usize;
            if a == 0xFF {
                back_buffer[idx] = src & 0x00FF_FFFF;
            } else {
                let dst = back_buffer[idx];
                let inv = 255 - a;
                let r = (((src >> 16) & 0xFF) * a + ((dst >> 16) & 0xFF) * inv + 127) / 255;
                let g = (((src >> 8) & 0xFF) * a + ((dst >> 8) & 0xFF) * inv + 127) / 255;
                let b = ((src & 0xFF) * a + (dst & 0xFF) * inv + 127) / 255;
                back_buffer[idx] = (r << 16) | (g << 8) | b;
            }
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
    draw_cursor_sprite(
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
    draw_cursor_sprite(
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
/// Renders the 32×32 TGA cursor sprite into the top-left of the 64×64 BGRA
/// cursor plane image, with the shape's real hotspot.
///
/// NOTE: the hardware cursor plane is currently never activated (see
/// activate_virtio_scanout): QEMU UIs map the VirtIO cursor sprite onto the
/// host pointer, which is hidden while a relative-pointer (PS/2) grab is
/// active — making the cursor invisible. The compositor blends the sprite in
/// software instead. This path is kept functional for future absolute-pointer
/// (virtio-tablet) setups.
fn upload_hw_cursor_if_needed(state: &mut CompositorState) -> bool {
    if !state.hw_cursor_active {
        return false;
    }
    if state.last_hw_cursor_shape == Some(state.active_cursor) {
        return true;
    }

    let Some((img, hot_x, hot_y)) = cursor_sprite(state.active_cursor) else {
        debug_log("[DISPLAY] hardware cursor sprite parse failed, falling back to software\n");
        state.hw_cursor_active = false;
        state.last_hw_cursor_shape = None;
        return false;
    };

    // 64×64 BGRA plane image; u32 0xAARRGGBB little-endian == BGRA bytes.
    static mut CURSOR_PIXELS: [u32; 64 * 64] = [0; 64 * 64];
    let pixels = unsafe { &mut *(&raw mut CURSOR_PIXELS) };
    for p in pixels.iter_mut() {
        *p = 0x00000000; // transparent
    }
    let w = img.width.min(64) as usize;
    let h = img.height.min(64) as usize;
    for y in 0..h {
        for x in 0..w {
            pixels[y * 64 + x] = img.pixel_argb(x as u32, y as u32);
        }
    }

    let ok = sunlight_ipc::gpu_update_cursor(
        pixels.as_ptr(),
        64 * 64,
        hot_x.max(0) as u32,
        hot_y.max(0) as u32,
    );
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

/// Re-blit the top panel strip from the Desktop window onto the back buffer
/// after all normal windows have been composited.  This ensures the Vortex Shell
/// top bar is never visually obscured even when a normal window manages to
/// overlap that region.
///
/// The Desktop window is a full-screen no-chrome surface so its pixel buffer
/// maps directly to screen coordinates; we just copy the first PANEL_TOP_RESERVED_H
/// rows back on top.
fn reblit_desktop_panel_strip(state: &CompositorState, back_buffer: &mut [u32]) {
    let desktop = match state.windows.iter().find(|w| {
        w.config.window_type == WindowType::Desktop
            && !w.buffer.is_null()
            && is_window_visible(state, w)
    }) {
        Some(w) => w,
        None => return,
    };
    let Ok(layout) = surface::SurfaceLayout::validate(
        desktop.surface_width_pixels,
        desktop.surface_height_rows,
        desktop.surface_stride_bytes,
        desktop.surface_len_bytes,
    ) else {
        return;
    };
    let Ok(source) = layout.readable_rect(
        0,
        0,
        state.fb_width,
        PANEL_TOP_RESERVED_H.min(state.fb_height),
    ) else {
        return;
    };
    let strip_rows = source.height as usize;
    let blit_w = source.width as usize;
    let stride = fb_stride(state);
    for row in 0..strip_rows {
        let src = unsafe {
            core::slice::from_raw_parts(desktop.buffer.add(row * layout.stride_pixels), blit_w)
        };
        let dst_start = row * stride;
        back_buffer[dst_start..dst_start + blit_w].copy_from_slice(src);
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
    // Re-paint the Desktop window's top panel strip after all normal windows so
    // the Vortex Shell bar is always visible regardless of window z-order.
    reblit_desktop_panel_strip(state, &mut back_buffer);
    {
        let mut canvas = back_buffer_canvas(state, &mut back_buffer);
        draw_notifications(&mut canvas, state);
    }
    // Software cursor only: the GPU backend uses a hardware cursor overlay.
    draw_cursor(state, &mut back_buffer);
    state.back_buffer = back_buffer;
    present_dirty_regions(state);
}

// ---------------------------------------------------------------------------
// Display backend setup helpers
// ---------------------------------------------------------------------------

/// Small fixed buffer for composing a log line and emitting it with a single
/// DebugLog syscall, so it cannot be interleaved with other services' output.
struct LogLine {
    buf: [u8; 192],
    len: usize,
}

impl LogLine {
    fn new() -> Self {
        Self {
            buf: [0; 192],
            len: 0,
        }
    }

    fn push_str(&mut self, s: &str) {
        for b in s.bytes() {
            if self.len < self.buf.len() {
                self.buf[self.len] = b;
                self.len += 1;
            }
        }
    }

    fn push_dec(&mut self, v: u32) {
        let mut tmp = [0u8; 10];
        let mut n = v;
        let mut i = tmp.len();
        loop {
            i -= 1;
            tmp[i] = b'0' + (n % 10) as u8;
            n /= 10;
            if n == 0 {
                break;
            }
        }
        for &b in &tmp[i..] {
            if self.len < self.buf.len() {
                self.buf[self.len] = b;
                self.len += 1;
            }
        }
    }

    fn push_hex(&mut self, v: u32) {
        self.push_str("0x");
        let hex = b"0123456789ABCDEF";
        for i in 0..8 {
            let nibble = ((v >> (28 - i * 4)) & 0xF) as usize;
            if self.len < self.buf.len() {
                self.buf[self.len] = hex[nibble];
                self.len += 1;
            }
        }
    }

    fn push_dim(&mut self, w: u32, h: u32) {
        self.push_dec(w);
        self.push_str("x");
        self.push_dec(h);
    }

    fn flush(&self) {
        if let Ok(s) = core::str::from_utf8(&self.buf[..self.len]) {
            debug_log(s);
        }
    }
}

/// Allocate a page-aligned, tightly-packed pixel buffer. VirtIO GPU backing
/// must start on a page boundary (the device scans whole pages, and the kernel
/// rejects misaligned buffers). Returns an empty Vec on allocation failure.
fn alloc_page_aligned_pixels(words: usize, fill: u32) -> Vec<u32> {
    if words == 0 {
        return Vec::new();
    }
    let layout = match core::alloc::Layout::from_size_align(words * 4, 4096) {
        Ok(l) => l,
        Err(_) => return Vec::new(),
    };
    // SAFETY: layout is non-zero; the bump allocator never reclaims, so
    // reconstructing the Vec with a u32 layout is safe (dealloc is a no-op).
    let ptr = unsafe { alloc::alloc::alloc(layout) as *mut u32 };
    if ptr.is_null() {
        return Vec::new();
    }
    let mut pixels = unsafe { Vec::from_raw_parts(ptr, words, words) };
    for p in pixels.iter_mut() {
        *p = fill;
    }
    pixels
}

/// Log a kernel GPU proxy failure with its reason and step-specific detail
/// word (VirtIO response code, failing page index, or sg entry count).
fn log_gpu_proxy_error(step: &str, e: sunlight_ipc::gpu_proxy::GpuProxyError) {
    let mut line = LogLine::new();
    line.push_str("[DISPLAY] ");
    line.push_str(step);
    line.push_str(" FAILED reason=");
    line.push_str(e.reason_str());
    line.push_str(" detail=");
    line.push_hex(e.detail);
    line.push_str("\n");
    line.flush();
}

/// Prepare the VirtIO GPU backend: allocate a page-aligned back buffer at the
/// GPU's scanout size and attach it as the scanout resource backing.
///
/// Deliberately does NOT issue SET_SCANOUT: QEMU's virtio-vga keeps showing
/// the VGA-compat output (the Limine framebuffer with the TTY login screen)
/// until the first non-zero SET_SCANOUT. Wiring the scanout is deferred to
/// SESSION_ACTIVATE so the login screen stays visible until the user logs in.
///
/// Returns the attached buffer on success or a diagnostic reason string on
/// failure (each step logs its exact error).
fn setup_virtio_backend(gw: u32, gh: u32) -> Result<Vec<u32>, &'static str> {
    let gpu_buffer = alloc_page_aligned_pixels((gw as usize) * (gh as usize), DESKTOP_COLOR);
    if gpu_buffer.is_empty() {
        debug_log("[DISPLAY] VirtIO back buffer allocation failed (");
        debug_dim(gw, gh);
        debug_log(")\n");
        return Err("virtio-buffer-alloc-failed");
    }
    let num_pages = (gpu_buffer.len() * 4 + 4095) / 4096;
    if let Err(e) = sunlight_ipc::gpu_attach_backing(gpu_buffer.as_ptr(), num_pages) {
        log_gpu_proxy_error("gpu_attach_backing", e);
        return Err("virtio-attach-backing-failed");
    }
    debug_log("[DISPLAY] gpu_attach_backing OK\n");
    Ok(gpu_buffer)
}

/// Wire the prepared VirtIO resource to scanout 0 and switch to the hardware
/// cursor. Called on the first SESSION_ACTIVATE (after login) so the VGA/TTY
/// login screen stays visible until then. Idempotent via `virtio_scanout_enabled`.
fn activate_virtio_scanout(state: &mut CompositorState) {
    if state.virtio_scanout_enabled
        || !matches!(
            state.display_backend,
            backend::DisplayBackend::VirtioGpu { .. }
        )
    {
        return;
    }
    match sunlight_ipc::gpu_set_scanout() {
        Ok(()) => {
            debug_log("[DISPLAY] gpu_set_scanout OK — VirtIO output active\n");
            state.virtio_scanout_enabled = true;
            // Software cursor: the compositor alpha-blends the cursor sprite
            // into the back buffer, so it is always visible. The VirtIO
            // hardware cursor plane is NOT used — QEMU UIs map that sprite to
            // the host pointer, which relative-pointer (PS/2) grabs hide,
            // leaving no visible cursor at all.
            debug_log("[DISPLAY] cursor_mode=software (composited sprite)\n");
        }
        Err(e) => {
            // The resource keeps its backing; QEMU keeps showing the VGA
            // output. Do NOT swap render dimensions here — clients may already
            // hold the VirtIO-sized metrics.
            log_gpu_proxy_error("gpu_set_scanout", e);
            debug_log("[DISPLAY] VirtIO output unavailable; VGA output remains\n");
        }
    }
}

// ---------------------------------------------------------------------------
// Debug helpers
// ---------------------------------------------------------------------------

/// Print `WxH` (e.g. `1280x720`).
fn debug_dim(w: u32, h: u32) {
    debug_dec(w);
    debug_log("x");
    debug_dec(h);
}

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
    debug_log(": batches=");
    debug_dec_u64(state.debug_counters.mouse_event_count);
    if state.debug_counters.mouse_event_count != 0 {
        debug_log(" raw_packets=");
        debug_dec_u64(state.debug_counters.raw_mouse_packet_count);
        debug_log(" raw_dx=[");
        debug_i32(state.debug_counters.raw_dx_min);
        debug_log("..");
        debug_i32(state.debug_counters.raw_dx_max);
        debug_log("] raw_dy=[");
        debug_i32(state.debug_counters.raw_dy_min);
        debug_log("..");
        debug_i32(state.debug_counters.raw_dy_max);
        debug_log("] final_dx=[");
        debug_i32(state.debug_counters.final_dx_min);
        debug_log("..");
        debug_i32(state.debug_counters.final_dx_max);
        debug_log("] final_dy=[");
        debug_i32(state.debug_counters.final_dy_min);
        debug_log("..");
        debug_i32(state.debug_counters.final_dy_max);
        debug_log("]");
    }
    debug_log(" cursor_x=");
    debug_dec(state.mouse_x as u32);
    debug_log(" cursor_y=");
    debug_dec(state.mouse_y as u32);
    debug_log(" clamped=");
    debug_dec_u64(state.debug_counters.clamped_motion_count);
    debug_log(" delta_capped=");
    debug_dec_u64(state.debug_counters.delta_capped_count);
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
    debug_log(" drag_started=");
    debug_dec_u64(state.debug_counters.drag_started_count);
    debug_log(" alt_tab=");
    debug_dec_u64(state.debug_counters.alt_tab_trigger_count);
    debug_log(" alt_tab_repeat=");
    debug_dec_u64(state.debug_counters.alt_tab_repeat_count);
    debug_log(" event_polls=");
    debug_dec_u64(state.debug_counters.display_poll_count);
    debug_log(" events_available=");
    debug_dec_u64(state.debug_counters.events_available_count);
    debug_log(" wrong_window=");
    debug_dec_u64(state.debug_counters.wrong_window_poll_count);
    debug_log(" pointer_other_window=");
    debug_dec_u64(state.debug_counters.pointer_other_window_count);
    debug_log("\n");
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn test_window(
        id: u64,
        x: u32,
        y: u32,
        w: u32,
        h: u32,
        window_type: WindowType,
        state: WindowState,
        z_index_type: ZIndexType,
    ) -> Window {
        Window {
            id,
            shm_cap: CapabilityToken::INVALID,
            buffer: core::ptr::null_mut(),
            width: w,
            height: h,
            surface_width_pixels: w,
            surface_height_rows: h,
            surface_stride_bytes: w as usize * surface::BYTES_PER_PIXEL as usize,
            surface_len_bytes: w as usize * h as usize * surface::BYTES_PER_PIXEL as usize,
            x,
            y,
            saved_x: x,
            saved_y: y,
            saved_w: w,
            saved_h: h,
            parent_focus_window_id: 0,
            owner_pid: 1,
            config: WindowConfig {
                title: [0; 64],
                window_type,
                state,
                decoration: WindowDecoration::Normal,
                border: BorderStyle::Full,
                z_index_type,
                z_index_value: 50,
                show_type: ShowType::Floating,
                group_type: GroupType::None,
                pid: 1,
                ppid: 0,
                group_ids: [0; 4],
                group_id_count: 0,
            },
            client_cursor: CursorShape::Pointer,
            pending_keys: KeyEventQueue::new(),
            last_mouse_x: 0,
            last_mouse_y: 0,
            last_buttons: 0,
            focus_press_pending: false,
            rolled_up: false,
            saved_unrolled_h: h,
            workspace_id: 0,
            hidden: false,
            overlay_decorations_visible: false,
            overlay_last_motion_ms: 0,
            overlay_pointer_inside: false,
            first_present_logged: false,
        }
    }

    fn test_state(windows: Vec<Window>) -> CompositorState {
        CompositorState {
            windows,
            launch_traces: Vec::new(),
            active_workspace_id: 1,
            mouse_x: 0,
            mouse_y: 0,
            pointer: PointerPolicy::new(800, 600),
            keyboard: KeyboardState::new(),
            active_drag: ActiveDrag::None,
            pending_move_drag: None,
            client_pointer_capture: None,
            prev_buttons: 0,
            fb: core::ptr::null_mut(),
            fb_width: 800,
            fb_height: 600,
            fb_pitch: 800 * 4,
            back_buffer: Vec::new(),
            display_backend: backend::DisplayBackend::Limine {
                fb: core::ptr::null_mut(),
                pitch_words: 800,
            },
            hw_cursor_active: false,
            virtio_scanout_enabled: false,
            last_hw_cursor_shape: None,
            active_cursor: CursorShape::Pointer,
            software_cursor: SoftwareCursorState::new(),
            session_active: true,
            inner_corner_mask: mask::CornerMask::new(CHROME_RADIUS.saturating_sub(BORDER_W)),
            dirty: dirty::DirtyList::new(),
            debug_counters: DebugCounters::new(),
            wallpaper: None,
            notifications: Vec::new(),
            next_notification_id: 1,
            vortex_launch_pending: false,
            last_mouse_generation: 0,
            last_titlebar_click_win_id: 0,
            last_titlebar_click_ms: 0,
            titlebar_double_click_action: TitlebarDoubleClickAction::WindowShade,
            app_tracker: app_lifecycle::AppTracker::new(),
        }
    }

    #[test]
    fn event_poll_resolves_the_requested_content_window_not_parent_or_shell() {
        let mut shell = test_window(
            1,
            0,
            0,
            800,
            600,
            WindowType::Desktop,
            WindowState::Normal,
            ZIndexType::Normal,
        );
        shell.owner_pid = 25;
        let mut mines = test_window(
            2,
            120,
            80,
            840,
            592,
            WindowType::Normal,
            WindowState::Normal,
            ZIndexType::Normal,
        );
        mines.owner_pid = 26;
        mines.parent_focus_window_id = 1;
        let state = test_state(vec![shell, mines]);

        assert_eq!(event_poll_window_idx(&state, 2), Some(1));
        assert_eq!(state.windows[1].owner_pid, 26);
        assert_eq!(event_poll_window_idx(&state, 1), Some(0));
        assert_eq!(event_poll_window_idx(&state, 26), None);
        assert_eq!(event_poll_window_idx(&state, 999), None);
    }

    #[test]
    fn topmost_hit_test_prefers_frontmost_visible_window() {
        let mut state = test_state(vec![
            test_window(
                1,
                40,
                40,
                200,
                180,
                WindowType::Normal,
                WindowState::Normal,
                ZIndexType::Normal,
            ),
            test_window(
                2,
                60,
                60,
                200,
                180,
                WindowType::Normal,
                WindowState::Normal,
                ZIndexType::Normal,
            ),
        ]);

        assert_eq!(topmost_window_id_at(&state, 100, 100), Some(2));

        state.windows[1].config.state = WindowState::Minimized;
        assert_eq!(topmost_window_id_at(&state, 100, 100), Some(1));
    }

    #[test]
    fn mouse_poll_isolated_to_topmost_window() {
        let mut state = test_state(vec![
            test_window(
                1,
                20,
                20,
                220,
                180,
                WindowType::Normal,
                WindowState::Normal,
                ZIndexType::Normal,
            ),
            test_window(
                2,
                40,
                40,
                220,
                180,
                WindowType::Normal,
                WindowState::Normal,
                ZIndexType::Normal,
            ),
        ]);
        state.mouse_x = 120;
        state.mouse_y = 90;
        state.prev_buttons = 1;

        state.windows[0].last_mouse_x = 7;
        state.windows[0].last_mouse_y = 9;
        state.windows[0].last_buttons = 2;
        state.windows[1].last_mouse_x = 11;
        state.windows[1].last_mouse_y = 13;
        state.windows[1].last_buttons = 4;

        let bottom = mouse_poll_words_for_window(&mut state, 0);
        assert_eq!(bottom.0, 7 | (9 << 16));
        assert_eq!(bottom.1, 2);
        assert_eq!(state.windows[0].last_mouse_x, 7);
        assert_eq!(state.windows[0].last_buttons, 2);

        let top = mouse_poll_words_for_window(&mut state, 1);
        assert_eq!(top.0, 120 | (90 << 16));
        assert_eq!(
            top.1,
            1 | SgpMsg::EVENT_FLAG_FOCUSED | SgpMsg::EVENT_FLAG_POINTER_OWNED
        );
        assert_eq!(state.windows[1].last_mouse_x, 120);
        assert_eq!(state.windows[1].last_mouse_y, 90);
        assert_eq!(state.windows[1].last_buttons, 1);
    }

    #[test]
    fn client_capture_delivers_outside_motion_and_focus_loss_ends_it() {
        let mut state = test_state(vec![test_window(
            7,
            40,
            40,
            220,
            180,
            WindowType::Normal,
            WindowState::Normal,
            ZIndexType::Normal,
        )]);
        state.mouse_x = 700;
        state.mouse_y = 500;
        state.prev_buttons = 1;
        state.client_pointer_capture = Some(7);
        let captured = mouse_poll_words_for_window(&mut state, 0);
        assert_eq!(captured.0, 700 | (500 << 16));
        assert_eq!(
            captured.1,
            1 | SgpMsg::EVENT_FLAG_FOCUSED
                | SgpMsg::EVENT_FLAG_POINTER_OWNED
                | SgpMsg::EVENT_FLAG_POINTER_CAPTURED
        );

        state.windows.push(test_window(
            8,
            0,
            0,
            100,
            100,
            WindowType::Normal,
            WindowState::Normal,
            ZIndexType::Normal,
        ));
        let old = mouse_poll_words_for_window(&mut state, 0);
        assert_eq!(state.client_pointer_capture, None);
        assert_eq!(old.1 & SgpMsg::EVENT_FLAG_FOCUSED, 0);
        assert_eq!(old.1 & SgpMsg::EVENT_FLAG_POINTER_CAPTURED, 0);
    }

    #[test]
    fn inactive_workspace_windows_do_not_receive_hit_tests() {
        let mut state = test_state(vec![
            test_window(
                1,
                20,
                20,
                220,
                180,
                WindowType::Normal,
                WindowState::Normal,
                ZIndexType::Normal,
            ),
            test_window(
                2,
                40,
                40,
                220,
                180,
                WindowType::Normal,
                WindowState::Normal,
                ZIndexType::Normal,
            ),
        ]);
        state.windows[1].workspace_id = 1;

        assert_eq!(topmost_window_id_at(&state, 100, 100), Some(1));
        assert_eq!(focused_window_id(&state), Some(1));
    }

    #[test]
    fn window_config_missing_decoration_defaults_to_normal() {
        let cfg = WindowConfig::from_ipc_words(&[0; 8]);
        assert_eq!(cfg.decoration, WindowDecoration::Normal);
    }

    #[test]
    fn compact_close_hit_test_only_exposes_close_button() {
        let mut win = test_window(
            1,
            40,
            40,
            220,
            160,
            WindowType::Normal,
            WindowState::Normal,
            ZIndexType::Normal,
        );
        win.config.decoration = WindowDecoration::CompactClose;

        let (wx, wy, chrome_w, _) = win.chrome_rect(800, 600);
        let close = control_rect_for_kind(&win, wx, wy, chrome_w, WindowControlKind::Close)
            .expect("close rect");
        let close_pt = sunlight_ui::Point::new(close.x + 2, close.y + 2);
        assert_eq!(
            hit_test_window(&win, close_pt.x as u32, close_pt.y as u32, 800, 600),
            HitZone::CloseBtn
        );

        let normal_min_slot = control_rect(wx, wy, chrome_w, win.titlebar_height(), 2);
        let min_pt = sunlight_ui::Point::new(normal_min_slot.x + 2, normal_min_slot.y + 2);
        assert_eq!(
            hit_test_window(&win, min_pt.x as u32, min_pt.y as u32, 800, 600),
            HitZone::TitleBar
        );
    }

    #[test]
    fn compact_close_minimize_hit_test_exposes_minimize_and_close_only() {
        let mut win = test_window(
            1,
            40,
            40,
            220,
            160,
            WindowType::Normal,
            WindowState::Normal,
            ZIndexType::Normal,
        );
        win.config.decoration = WindowDecoration::CompactCloseMinimize;

        let (wx, wy, chrome_w, _) = win.chrome_rect(800, 600);
        let min = control_rect_for_kind(&win, wx, wy, chrome_w, WindowControlKind::Minimize)
            .expect("min rect");
        let min_pt = sunlight_ui::Point::new(min.x + 2, min.y + 2);
        assert_eq!(
            hit_test_window(&win, min_pt.x as u32, min_pt.y as u32, 800, 600),
            HitZone::MinimizeBtn
        );

        let normal_max_slot = control_rect(wx, wy, chrome_w, win.titlebar_height(), 1);
        let max_pt = sunlight_ui::Point::new(normal_max_slot.x + 2, normal_max_slot.y + 2);
        assert_eq!(
            hit_test_window(&win, max_pt.x as u32, max_pt.y as u32, 800, 600),
            HitZone::TitleBar
        );
    }

    #[test]
    fn initial_window_origin_stays_within_screen_bounds() {
        let metrics = DisplayMetrics::new(
            1366,
            768,
            1366 * 4,
            PixelFormat::Xrgb8888,
            ScreenBackend::VirtioGpu,
        );
        let client_w = 900;
        let client_h = 650;
        let chrome_w = client_w + BORDER_W * 2;
        let chrome_h = TITLEBAR_H + client_h + BORDER_W;
        let (x, y) = metrics.initial_window_origin(
            5,
            client_w,
            client_h,
            chrome_w,
            chrome_h,
            PANEL_TOP_RESERVED_H,
        );
        assert!(x + chrome_w <= 1366);
        assert!(y + chrome_h <= 768);
        assert!(y >= PANEL_TOP_RESERVED_H);
    }

    #[test]
    fn hidden_overlay_transitions_on_enter_idle_and_leave() {
        let mut state = test_state(vec![test_window(
            1,
            40,
            40,
            220,
            160,
            WindowType::Normal,
            WindowState::Normal,
            ZIndexType::Normal,
        )]);
        state.windows[0].config.decoration = WindowDecoration::HiddenOverlay;

        state.mouse_x = 100;
        state.mouse_y = 80;
        assert!(update_overlay_window_visibility(
            &mut state, 10, true, false
        ));
        assert!(state.windows[0].overlay_decorations_visible);

        assert!(update_overlay_window_visibility(
            &mut state,
            10 + OVERLAY_DECORATION_IDLE_TIMEOUT_MS,
            false,
            false
        ));
        assert!(!state.windows[0].overlay_decorations_visible);

        state.mouse_x = 10;
        state.mouse_y = 10;
        assert!(!update_overlay_window_visibility(
            &mut state, 20, false, false
        ));

        state.mouse_x = 120;
        state.mouse_y = 90;
        assert!(update_overlay_window_visibility(
            &mut state, 30, true, false
        ));
        assert!(state.windows[0].overlay_decorations_visible);

        state.mouse_x = 5;
        state.mouse_y = 5;
        assert!(update_overlay_window_visibility(
            &mut state, 40, true, false
        ));
        assert!(!state.windows[0].overlay_decorations_visible);
    }
}

// ---------------------------------------------------------------------------
// Panic handler
// ---------------------------------------------------------------------------

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    debug_log("[DISPLAY] PANIC\n");
    loop {}
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[cfg(not(test))]
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
    debug_log("[display] available mode 0: ");
    debug_dec(fb_width);
    debug_log("x");
    debug_dec(fb_height);
    debug_log(" pitch=");
    debug_dec(pitch as u32);
    debug_log(" bpp=");
    debug_dec(bpp as u32);
    debug_log(" format=limine-framebuffer current=yes\n");
    debug_log("[display] current mode: ");
    debug_dec(fb_width);
    debug_log("x");
    debug_dec(fb_height);
    debug_log("\n");

    let limine_backend = backend::DisplayBackend::Limine {
        fb: fb_ptr as *mut u32,
        pitch_words: pitch as usize / 4,
    };

    // Probe the VirtIO GPU. The scanout reported by GET_DISPLAY_INFO carries
    // the host's requested resolution (e.g. QEMU -device virtio-vga,xres=,yres=
    // or a host window resize) — it is the mode we try to honor.
    //
    // TODO(live-resize): the VirtIO GPU size is sampled exactly once here, at
    // startup. If the host window is resized later, the kernel can signal a new
    // scanout size; the compositor would then need to resize back_buffer,
    // recreate the GPU resource, re-attach backing, set scanout, mark the whole
    // screen dirty and redraw. Until that path exists we pin the initial size.
    let mut diag_reason: &'static str = "no-virtio-gpu";
    let virtio_scanout: Option<(u32, u32)> = match sunlight_ipc::gpu_get_info() {
        Some((gw, gh)) => match validate_size(gw, gh) {
            Some((gw, gh)) => {
                debug_log("[DISPLAY] VirtIO GPU reported ");
                debug_dim(gw, gh);
                debug_log("\n");
                Some((gw, gh))
            }
            None => {
                // Garbage/zero size — ignore the GPU and use Limine instead.
                debug_log("[DISPLAY] VirtIO GPU reported invalid size ");
                debug_dim(gw, gh);
                debug_log(", ignoring GPU\n");
                diag_reason = "virtio-invalid-size";
                None
            }
        },
        None => {
            debug_log("[DISPLAY] VirtIO GPU not found, using Limine framebuffer\n");
            None
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
                if virtio_scanout.is_none() {
                    diag_reason = "limine-invalid-size-safe-fallback";
                }
                (SAFE_FALLBACK_W, SAFE_FALLBACK_H, SAFE_FALLBACK_W * 4)
            }
        };

    // Backend selection is explicit: VirtIO GPU is only the active backend once
    // resource create + backing attach succeed (SET_SCANOUT itself is deferred
    // to SESSION_ACTIVATE so the TTY login stays visible). Any failure logs its
    // exact reason and reverts every render dimension to the Limine mode, so
    // VirtIO and Limine dimensions are never mixed.
    let mut render_width = limine_render_w;
    let mut render_height = limine_render_h;
    let mut render_pitch = limine_render_pitch;
    let mut display_backend = limine_backend;
    let mut back_buffer: Vec<u32> = Vec::new();

    if let Some((gw, gh)) = virtio_scanout {
        match setup_virtio_backend(gw, gh) {
            Ok(gpu_buffer) => {
                display_backend = backend::DisplayBackend::VirtioGpu {
                    width: gw,
                    height: gh,
                };
                render_width = gw;
                render_height = gh;
                render_pitch = gw * 4;
                back_buffer = gpu_buffer;
                diag_reason = "virtio-attach-ok-scanout-deferred";
                debug_log(
                    "[DISPLAY] VirtIO scanout deferred until session activation (login stays on VGA output)\n",
                );
            }
            Err(reason) => {
                diag_reason = reason;
                debug_log("[DISPLAY] falling back to Limine framebuffer backend at ");
                debug_dim(limine_render_w, limine_render_h);
                debug_log("\n");
            }
        }
    }

    // Limine (or fallback) path: allocate the software back buffer now that the
    // final render dimensions are known.
    if back_buffer.is_empty() {
        back_buffer = alloc_page_aligned_pixels(
            (render_pitch as usize / 4) * render_height as usize,
            DESKTOP_COLOR,
        );
        if back_buffer.is_empty() {
            debug_log("[DISPLAY] FATAL: back buffer allocation failed\n");
            loop {}
        }
    }

    // Startup summary: single source of truth for the compositor's geometry.
    // `requested` is the mode the host asked for via the VirtIO scanout report
    // (QEMU xres/yres override or window size); without a VirtIO GPU the Limine
    // framebuffer mode is the only request we can honor.
    let requested = virtio_scanout.unwrap_or((limine_render_w, limine_render_h));
    let mut diag = LogLine::new();
    diag.push_str("[DISPLAY] display_backend=");
    diag.push_str(match display_backend {
        backend::DisplayBackend::VirtioGpu { .. } => "VirtIO",
        backend::DisplayBackend::Limine { .. } => "Limine",
    });
    diag.push_str(" requested=");
    diag.push_dim(requested.0, requested.1);
    diag.push_str(" virtio_scanout=");
    match virtio_scanout {
        Some((vw, vh)) => diag.push_dim(vw, vh),
        None => diag.push_str("none"),
    }
    diag.push_str(" final=");
    diag.push_dim(render_width, render_height);
    diag.push_str(" reason=");
    diag.push_str(diag_reason);
    diag.push_str("\n");
    diag.flush();

    let mut size_line = LogLine::new();
    size_line.push_str("[DISPLAY] compositor size ");
    size_line.push_dim(render_width, render_height);
    size_line.push_str(" pitch=");
    size_line.push_dec(render_pitch);
    size_line.push_str("\n");
    size_line.flush();
    let expected_back_len = (render_pitch as usize / 4) * render_height as usize;
    debug_log("[DISPLAY] back_buffer expected_len=");
    debug_dec(expected_back_len as u32);
    debug_log(" actual_len=");
    debug_dec(back_buffer.len() as u32);
    debug_log("\n");

    let mut state = CompositorState {
        windows: Vec::new(),
        launch_traces: Vec::new(),
        active_workspace_id: 1,
        mouse_x: (render_width / 2) as u16,
        mouse_y: (render_height / 2) as u16,
        pointer: PointerPolicy::new(render_width, render_height),
        keyboard: KeyboardState::new(),
        active_drag: ActiveDrag::None,
        pending_move_drag: None,
        client_pointer_capture: None,
        prev_buttons: 0,
        fb: fb_ptr as *mut u32,
        fb_width: render_width,
        fb_height: render_height,
        fb_pitch: render_pitch,
        back_buffer,
        display_backend,
        // Hardware cursor and scanout are enabled together on the first
        // SESSION_ACTIVATE (see activate_virtio_scanout).
        hw_cursor_active: false,
        virtio_scanout_enabled: false,
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
        notifications: Vec::new(),
        next_notification_id: 1,
        vortex_launch_pending: false,
        last_mouse_generation: 0,
        last_titlebar_click_win_id: 0,
        last_titlebar_click_ms: 0,
        titlebar_double_click_action: TitlebarDoubleClickAction::WindowShade,
        app_tracker: app_lifecycle::AppTracker::new(),
    };
    log_debug_counters(&state, "startup");
    // Prime the back_buffer content (wallpaper/desktop color) but do NOT
    // present it: the TTY session owns the display until SESSION_ACTIVATE.
    // Presenting here used to overwrite the login screen on the Limine
    // backend, and on VirtIO the (deferred) scanout is not wired yet.
    clear_back_buffer(&mut state);

    let mut next_win_id: u64 = 1;

    loop {
        let msg = if let Some(timeout_ms) = compositor_poll_timeout_ms(&state) {
            if let Some(msg) = ipc_recv_timeout(my_ep, timeout_ms) {
                msg
            } else {
                let now = monotonic_millis();
                let mut needs_redraw = false;
                if prune_notifications(&mut state, now) {
                    mark_dirty_full(&mut state);
                    needs_redraw = true;
                }
                if update_overlay_window_visibility(&mut state, now, false, false) {
                    needs_redraw = true;
                }
                if sweep_app_zombies(&mut state, now) {
                    needs_redraw = true;
                }
                if prune_dead_owner_windows(&mut state) {
                    needs_redraw = true;
                }
                if needs_redraw {
                    redraw_scene(&mut state);
                }
                continue;
            }
        } else {
            ipc_recv(my_ep)
        };

        match msg.label {
            // -------------------------------------------------------------------
            // LAUNCH_TRACE — launcher/runner tells us which pid belongs to which
            // user-visible launch request so window creation/presentation logs can
            // be correlated later.
            // words[0] = launch_id, words[1] = source, words[2] = pid,
            // words[3] = requested_at_ms
            // -------------------------------------------------------------------
            SgpMsg::LAUNCH_TRACE => {
                let launch_id = msg.words[0];
                let source = launch_source_from_u64(msg.words[1]);
                let pid = msg.words[2];
                let requested_at_ms = msg.words[3];
                register_launch_trace(&mut state, launch_id, source, pid, requested_at_ms);
                let trace = LaunchTrace::new(launch_id, source, requested_at_ms);
                launch_trace::log_phase_now(trace, "trace", "launch_trace_registered", Some(pid));
                let _ = ipc_reply(IpcMsg::with_label(SgpMsg::REPLY));
            }

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
                let Ok(layout) = surface::SurfaceLayout::for_new_surface(w, h) else {
                    let _ = ipc_reply(IpcMsg::with_label(0xA1FE));
                    continue;
                };
                let size = layout.surface_len_bytes;
                let config = WindowConfig::from_ipc_words(&msg.words);
                let is_desktop_window = config.window_type == WindowType::Desktop;
                let owner_pid = msg.badge;
                let trace = trace_for_pid(&state, owner_pid);
                let window_subject = String::from(window_title_str(&config.title));
                launch_trace::log_phase_now(
                    trace,
                    window_subject.as_str(),
                    "display_server_received_window_create",
                    Some(owner_pid),
                );

                match sunlight_ipc::shm_create(size, 0) {
                    Ok((buf, shm_tok)) => {
                        let our_buf = buf as *mut u32;

                        let id = next_win_id;
                        next_win_id += 1;

                        let titlebar_h =
                            WindowDecoration::from_flags(msg.words[1]).titlebar_height();
                        let chrome_w = w.saturating_add(BORDER_W * 2);
                        let chrome_h = titlebar_h + h + BORDER_W;
                        let metrics = state.display_metrics();
                        let (win_x, win_y) = if is_desktop_window {
                            (0, 0)
                        } else {
                            metrics.initial_window_origin(
                                id,
                                w,
                                h,
                                chrome_w,
                                chrome_h,
                                PANEL_TOP_RESERVED_H,
                            )
                        };

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
                                surface_width_pixels: layout.width,
                                surface_height_rows: layout.height,
                                surface_stride_bytes: layout.stride_bytes,
                                surface_len_bytes: layout.surface_len_bytes,
                                x: win_x,
                                y: win_y,
                                saved_x: win_x,
                                saved_y: win_y,
                                saved_w: w,
                                saved_h: h,
                                parent_focus_window_id: if config.window_type == WindowType::Dialog
                                {
                                    focused_window_id(&state).unwrap_or(0)
                                } else {
                                    0
                                },
                                // Authoritative creator PID from the kernel IPC badge.
                                // TODO: Use this for process-exit-driven cleanup in the
                                // later hardening phase.
                                owner_pid,
                                config,
                                client_cursor: CursorShape::Pointer,
                                pending_keys: KeyEventQueue::new(),
                                last_mouse_x: 0,
                                last_mouse_y: 0,
                                last_buttons: 0,
                                focus_press_pending: false,
                                rolled_up: false,
                                saved_unrolled_h: h,
                                workspace_id: state.active_workspace_id,
                                hidden: false,
                                overlay_decorations_visible: false,
                                overlay_last_motion_ms: 0,
                                overlay_pointer_inside: false,
                                first_present_logged: false,
                            },
                        );
                        if is_desktop_window {
                            state.vortex_launch_pending = false;
                        }
                        state.app_tracker.register_window(
                            owner_pid,
                            id,
                            window_title_str(&config.title),
                            is_desktop_window,
                        );
                        mark_dirty_full(&mut state);
                        redraw_scene(&mut state);
                        launch_trace::log_phase_now(
                            trace,
                            window_subject.as_str(),
                            "window_registered",
                            Some(owner_pid),
                        );

                        let client_x = win_x + BORDER_W;
                        let client_y = win_y + config.decoration.titlebar_height();
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
            // This operation changes logical geometry/state only; the attached
            // surface metadata is intentionally not resized or replaced. The
            // compositor clips logical geometry to that validated allocation.
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
                                // Unroll before expanding so client area is fully visible.
                                if win.rolled_up {
                                    win.height = win.saved_unrolled_h;
                                    win.rolled_up = false;
                                }
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
                        win.config.decoration = WindowDecoration::from_flags(flags);
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
                    state
                        .app_tracker
                        .update_window_title(win_id, window_title_str(&win.config.title));
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
                        } else if win.config.title.starts_with(b"Sunlight Cl") {
                            // Sunlight Clipman → near the mouse cursor, clamped on-screen.
                            let margin = 16i32;
                            let cursor_x = state.mouse_x as i32;
                            let cursor_y = state.mouse_y as i32;
                            let max_x =
                                (state.fb_width as i32 - win.width as i32 - margin).max(margin);
                            let max_y =
                                (state.fb_height as i32 - win.height as i32 - margin).max(margin);
                            let preferred_x = cursor_x - (win.width as i32 / 2);
                            let preferred_y = cursor_y + 18;
                            let new_x = preferred_x.clamp(margin, max_x) as u32;
                            let new_y = preferred_y.clamp(margin, max_y) as u32;
                            win.x = new_x;
                            win.saved_x = new_x;
                            win.y = new_y;
                            win.saved_y = new_y;
                        }
                    }
                }
                mark_dirty_full(&mut state);
                redraw_scene(&mut state);
                let _ = ipc_reply(IpcMsg::with_label(SgpMsg::REPLY));
            }

            // -------------------------------------------------------------------
            // LIST_WINDOWS — enumerate compositor-managed windows for shell state
            // tracking. words[0] = 0-based index into the current window stack.
            // Reply includes the first 16 title bytes in words[6..7].
            // -------------------------------------------------------------------
            SgpMsg::LIST_WINDOWS => {
                let idx = msg.words[0] as usize;
                let reply = list_window_at(&state, idx);
                let _ = ipc_reply(reply);
            }

            // -------------------------------------------------------------------
            // ACTIVATE_WINDOW — raise a window and restore it if minimized.
            // words[0] = win_id
            // -------------------------------------------------------------------
            SgpMsg::ACTIVATE_WINDOW => {
                let win_id = msg.words[0];
                if activate_window(&mut state, win_id) {
                    debug_log("[DISPLAY] activate_window id=");
                    debug_dec(win_id as u32);
                    debug_log("\n");
                }
                let _ = ipc_reply(IpcMsg::with_label(SgpMsg::REPLY));
            }

            // -------------------------------------------------------------------
            // SET_WORKSPACE — switch the active workspace (1..=4).
            // words[0] = workspace id. Hides/filters windows whose workspace_id
            // differs; Desktop/Widget windows stay visible on all workspaces.
            // -------------------------------------------------------------------
            SgpMsg::SET_WORKSPACE => {
                let ws = msg.words[0] as u32;
                if (1..=4).contains(&ws) && state.active_workspace_id != ws {
                    state.active_workspace_id = ws;
                    mark_dirty_full(&mut state);
                    redraw_scene(&mut state);
                }
                let _ = ipc_reply(IpcMsg::with_label(SgpMsg::REPLY));
            }

            // -------------------------------------------------------------------
            // GET_SCREEN_INFO — reply with compositor display metrics.
            // No input words needed.
            // -------------------------------------------------------------------
            SgpMsg::GET_SCREEN_INFO => {
                let words = state.display_metrics().pack_reply_words();
                let mut reply = IpcMsg::with_label(SgpMsg::REPLY);
                for (i, word) in words.iter().enumerate() {
                    reply = reply.word(i, *word);
                }
                let _ = ipc_reply(reply);
            }

            // -------------------------------------------------------------------
            // SHOW_NOTIFICATION — transient top-right toast overlay.
            // words[0] = NotificationKind discriminant
            // words[1] = timeout_ms
            // caps[0]  = SHM page containing the notification payload
            // -------------------------------------------------------------------
            SgpMsg::SHOW_NOTIFICATION => {
                ingest_notification(&mut state, &msg);
                mark_dirty_full(&mut state);
                redraw_scene(&mut state);
                let _ = ipc_reply(IpcMsg::with_label(SgpMsg::REPLY));
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
                if let Some(win_idx) = state.windows.iter().position(|w| w.id == win_id) {
                    let (chrome_rect, owner_pid, should_log_visible, trace, subject) = {
                        let win = &state.windows[win_idx];
                        let trace = trace_for_pid(&state, win.owner_pid);
                        (
                            win.chrome_rect(state.fb_width, state.fb_height),
                            win.owner_pid,
                            !win.first_present_logged,
                            trace,
                            String::from(window_title_str(&win.config.title)),
                        )
                    };
                    let (wx, wy, ww, wh) = chrome_rect;
                    mark_dirty_rect(&mut state, Rect::new(wx as i32, wy as i32, ww, wh));
                    redraw_scene(&mut state);
                    if should_log_visible {
                        launch_trace::log_phase_now(
                            trace,
                            subject.as_str(),
                            "window_visible_on_screen",
                            Some(owner_pid),
                        );
                        if let Some(win) = state.windows.get_mut(win_idx) {
                            win.first_present_logged = true;
                        }
                    }
                }
                let _ = ipc_reply(IpcMsg::with_label(SgpMsg::REPLY));
            }

            // -------------------------------------------------------------------
            // EVENT_POLL — client polls mouse position + its current client origin.
            // -------------------------------------------------------------------
            SgpMsg::EVENT_POLL => {
                let win_id = msg.words[0];
                let mut wake = IpcMsg::with_label(SgpMsg::REPLY);
                state.debug_counters.display_poll_count =
                    state.debug_counters.display_poll_count.wrapping_add(1);
                if let Some(win_idx) = event_poll_window_idx(&state, win_id) {
                    let (cx, cy) = {
                        let win = &state.windows[win_idx];
                        match win.config.state {
                            WindowState::Fullscreen => (0u64, 0u64),
                            WindowState::Maximized => {
                                (BORDER_W as u64, win.titlebar_height() as u64)
                            }
                            _ => {
                                let (cx, cy) = win.client_origin();
                                (cx as u64, cy as u64)
                            }
                        }
                    };
                    let (previous_mouse_x, previous_mouse_y, previous_buttons, focus_press) = {
                        let win = &state.windows[win_idx];
                        (
                            win.last_mouse_x,
                            win.last_mouse_y,
                            win.last_buttons,
                            win.focus_press_pending,
                        )
                    };
                    let key_event = {
                        let win = &mut state.windows[win_idx];
                        win.pending_keys.pop()
                    };
                    let (mouse_word, button_word) =
                        mouse_poll_words_for_window(&mut state, win_idx);
                    let delivered_mouse_x = (mouse_word & 0xffff) as u16;
                    let delivered_mouse_y = ((mouse_word >> 16) & 0xffff) as u16;
                    let delivered_buttons = (button_word & 0xff) as u8;
                    let pointer_owned = button_word & SgpMsg::EVENT_FLAG_POINTER_OWNED != 0;
                    let event_available = key_event.is_some()
                        || focus_press
                        || (pointer_owned
                            && (delivered_mouse_x != previous_mouse_x
                                || delivered_mouse_y != previous_mouse_y
                                || delivered_buttons != previous_buttons));
                    if event_available {
                        state.debug_counters.events_available_count =
                            state.debug_counters.events_available_count.wrapping_add(1);
                    }
                    if !pointer_owned {
                        state.debug_counters.pointer_other_window_count = state
                            .debug_counters
                            .pointer_other_window_count
                            .wrapping_add(1);
                    }
                    wake = wake
                        .word(0, mouse_word)
                        .word(1, cx | (cy << 32))
                        .word(3, button_word | SgpMsg::EVENT_FLAG_WINDOW_VALID);
                    if let Some(key_event) = key_event {
                        wake = wake.word(2, key_event);
                    }
                } else {
                    state.debug_counters.wrong_window_poll_count =
                        state.debug_counters.wrong_window_poll_count.wrapping_add(1);
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
            // Super + . or Ctrl + .: toggle Emoji Picker.
            // Alt + Tab: rotate focus across app windows in the active z-group.
            // -------------------------------------------------------------------
            sunlight_ipc::KbdMsg::KEY_EVENT => {
                let packed = msg.words[0];
                let (keycode, pressed, _, ctrl, alt, super_key, _) =
                    sunlight_ipc::unpack_key_event(packed);
                let was_down = state.keyboard.update_key(keycode, pressed);
                let now = monotonic_millis();
                let ctrl_down = state.keyboard.ctrl_down() || ctrl;
                let alt_down = state.keyboard.alt_down() || alt;
                let super_down = state.keyboard.super_down() || super_key;
                let mut consumed = false;

                if keycode == KEY_TAB {
                    if pressed && alt_down {
                        state.keyboard.alt_tab_chord_active = true;
                        if !was_down {
                            trigger_alt_tab(&mut state, AltTabTriggerSource::Keydown);
                            state.keyboard.alt_tab_next_repeat_ms =
                                now.saturating_add(ALT_TAB_REPEAT_MS);
                        } else if now >= state.keyboard.alt_tab_next_repeat_ms {
                            trigger_alt_tab(&mut state, AltTabTriggerSource::Repeat);
                            state.keyboard.alt_tab_next_repeat_ms =
                                now.saturating_add(ALT_TAB_REPEAT_MS);
                        }
                        consumed = true;
                    } else if !pressed && state.keyboard.alt_tab_chord_active {
                        state.keyboard.clear_alt_tab_repeat();
                        consumed = true;
                    }
                } else if keycode == KEY_ALT && !pressed && state.keyboard.alt_tab_chord_active {
                    state.keyboard.clear_alt_tab_repeat();
                    consumed = true;
                } else if pressed && !was_down && super_down && keycode == KEY_R {
                    launch_runner(&mut state);
                    consumed = true;
                } else if pressed && !was_down && super_down && keycode == KEY_V {
                    toggle_clipman(&mut state);
                    consumed = true;
                } else if pressed && !was_down && (super_down || ctrl_down) && keycode == KEY_PERIOD
                {
                    toggle_emoji_picker(&mut state);
                    consumed = true;
                } else if pressed && !was_down && ctrl_down && keycode == KEY_SPACE {
                    launch_runner(&mut state);
                    consumed = true;
                } else if pressed && !was_down && ctrl_down && keycode == KEY_W {
                    if let Some(focused) = focused_window_id(&state) {
                        if close_window(&mut state, focused, None) {
                            mark_dirty_full(&mut state);
                            redraw_scene(&mut state);
                        }
                    }
                    consumed = true;
                } else if pressed && !was_down && super_down {
                    // Super + 1..4 → switch to workspace 1..4. PC/AT Set 1
                    // scancodes for the top row digits.
                    let ws = match keycode {
                        0x02 => Some(1u32), // KEY_1
                        0x03 => Some(2),    // KEY_2
                        0x04 => Some(3),    // KEY_3
                        0x05 => Some(4),    // KEY_4
                        _ => None,
                    };
                    if let Some(ws) = ws {
                        if state.active_workspace_id != ws {
                            state.active_workspace_id = ws;
                            mark_dirty_full(&mut state);
                            redraw_scene(&mut state);
                        }
                        consumed = true;
                    }
                }

                if !consumed && state.session_active {
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
                let metadata = msg.words[1];
                // Driver already inverted Y (PS/2 up→negative screen delta). Do NOT negate again.
                let dx = ((raw & 0xFFFF) as i16) as i32;
                let dy = (((raw >> 16) & 0xFFFF) as i16) as i32;
                let buttons = ((raw >> 32) & 0xFF) as u8;
                let packet_count = ((metadata & 0xFFFF_FFFF) as u32).max(1);
                let generation = (metadata >> 32) as u32;

                if generation != 0 && generation <= state.last_mouse_generation {
                    let _ = ipc_reply(IpcMsg::with_label(SgpMsg::REPLY));
                    continue;
                }
                if generation != 0 {
                    state.last_mouse_generation = generation;
                }

                let prev_cx = state.pointer.x() as u32;
                let prev_cy = state.pointer.y() as u32;
                let prev_buttons = state.prev_buttons;
                state.debug_counters.mouse_event_count += 1;
                state.debug_counters.raw_mouse_packet_count += packet_count as u64;
                // Snapshot drag state before processing to detect window-geometry changes.
                let had_active_drag = !matches!(state.active_drag, ActiveDrag::None);

                if INPUT_DEBUG {
                    debug_log("[DISPLAY] raw_motion dx=");
                    debug_i32(dx);
                    debug_log(" dy=");
                    debug_i32(dy);
                    debug_log(" packets=");
                    debug_dec(packet_count);
                    debug_log(" before=(");
                    debug_dec(prev_cx);
                    debug_log(",");
                    debug_dec(prev_cy);
                    debug_log(")\n");
                }
                let left_down = (buttons & 1) != 0;
                let was_left_down = (prev_buttons & 1) != 0;

                state.debug_counters.raw_dx_min = state.debug_counters.raw_dx_min.min(dx);
                state.debug_counters.raw_dx_max = state.debug_counters.raw_dx_max.max(dx);
                state.debug_counters.raw_dy_min = state.debug_counters.raw_dy_min.min(dy);
                state.debug_counters.raw_dy_max = state.debug_counters.raw_dy_max.max(dy);
                let motion = state.pointer.apply_motion(dx, dy, buttons);
                if motion.position_clamped {
                    state.debug_counters.clamped_motion_count += 1;
                }
                if motion.delta_capped {
                    state.debug_counters.delta_capped_count += 1;
                }
                state.debug_counters.final_dx_min =
                    state.debug_counters.final_dx_min.min(motion.final_dx);
                state.debug_counters.final_dx_max =
                    state.debug_counters.final_dx_max.max(motion.final_dx);
                state.debug_counters.final_dy_min =
                    state.debug_counters.final_dy_min.min(motion.final_dy);
                state.debug_counters.final_dy_max =
                    state.debug_counters.final_dy_max.max(motion.final_dy);

                state.mouse_x = state.pointer.x() as u16;
                state.mouse_y = state.pointer.y() as u16;

                let cx = state.pointer.x() as u32;
                let cy = state.pointer.y() as u32;
                let motion_now = monotonic_millis();
                let overlay_changed = update_overlay_window_visibility(
                    &mut state,
                    motion_now,
                    cx != prev_cx || cy != prev_cy,
                    left_down && !was_left_down,
                );

                // Capture any client-area drag at the generic compositor
                // boundary. Chronos applies its stricter graphics-viewport
                // rule before exposing motion to a DOS guest.
                if prev_buttons == 0 && buttons != 0 {
                    if let Some(hit_idx) = topmost_window_idx_at(&state, cx, cy) {
                        let win = &state.windows[hit_idx];
                        if hit_test_window(win, cx, cy, state.fb_width, state.fb_height)
                            == HitZone::ClientArea
                        {
                            state.client_pointer_capture = Some(win.id);
                        }
                    }
                }
                if buttons == 0 {
                    state.client_pointer_capture = None;
                }

                // ── Left button just pressed ────────────────────────────────
                if state.session_active && left_down && !was_left_down {
                    if dismiss_notification_at_point(&mut state, Point::new(cx as i32, cy as i32)) {
                        state.active_drag = ActiveDrag::None;
                        state.pending_move_drag = None;
                        mark_dirty_full(&mut state);
                        redraw_scene(&mut state);
                        let _ = ipc_reply(IpcMsg::with_label(SgpMsg::REPLY));
                        continue;
                    }
                    if let Some(hit_idx) = topmost_window_idx_at(&state, cx, cy) {
                        let id = state.windows[hit_idx].id;
                        let hit_zone = hit_test_window(
                            &state.windows[hit_idx],
                            cx,
                            cy,
                            state.fb_width,
                            state.fb_height,
                        );

                        // Raise to front (unless it's a Desktop/Widget type).
                        let win_type = state.windows[hit_idx].config.window_type;
                        if win_type != WindowType::Desktop && win_type != WindowType::Widget {
                            raise_window_by_id(&mut state, id);
                        }
                        if let Some(win) = state.windows.iter_mut().find(|win| win.id == id) {
                            win.focus_press_pending = true;
                        }

                        let click_now = motion_now;
                        match hit_zone {
                            HitZone::TitleBar => {
                                // Double-click on the titlebar rolls up / unrolls the window
                                // (shade gesture, like FVWM/Afterstep desktops).
                                let is_dbl = state.last_titlebar_click_win_id == id
                                    && click_now.saturating_sub(state.last_titlebar_click_ms)
                                        < DOUBLE_CLICK_MS;
                                if is_dbl {
                                    if let Some(win) = state.windows.iter_mut().find(|w| w.id == id)
                                    {
                                        if win.config.state == WindowState::Normal {
                                            if win.rolled_up {
                                                win.height = win.saved_unrolled_h;
                                                win.rolled_up = false;
                                            } else {
                                                win.saved_unrolled_h = win.height;
                                                win.rolled_up = true;
                                            }
                                        }
                                    }
                                    // Cancel any pending drag so the roll-up click doesn't
                                    // accidentally move the window.
                                    state.pending_move_drag = None;
                                    state.last_titlebar_click_win_id = 0;
                                    state.last_titlebar_click_ms = 0;
                                } else {
                                    // First click of a potential double-click: record it and
                                    // start a pending drag (drag only fires if cursor moves
                                    // more than DRAG_THRESHOLD_PX, so clicking alone is safe).
                                    state.last_titlebar_click_win_id = id;
                                    state.last_titlebar_click_ms = click_now;
                                    state.pending_move_drag = Some((id, cx as i32, cy as i32));
                                }
                            }
                            HitZone::CloseBtn => {
                                let _ = close_window(&mut state, id, None);
                                state.active_drag = ActiveDrag::None;
                            }
                            HitZone::MaximizeBtn => {
                                if let Some(win) = state.windows.iter_mut().find(|w| w.id == id) {
                                    if win.config.state == WindowState::Normal {
                                        // Unroll before maximizing so the client area is restored.
                                        if win.rolled_up {
                                            win.height = win.saved_unrolled_h;
                                            win.rolled_up = false;
                                        }
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

                // ── Promote pending move drag once threshold is exceeded ─────
                if left_down {
                    if let Some((win_id, press_x, press_y)) = state.pending_move_drag {
                        let dist = (cx as i32 - press_x).abs() + (cy as i32 - press_y).abs();
                        if dist > DRAG_THRESHOLD_PX {
                            state.active_drag = ActiveDrag::Move(MoveDrag { window_id: win_id });
                            state.pending_move_drag = None;
                            state.debug_counters.drag_started_count += 1;
                        }
                    }
                }

                // ── Left button released ─────────────────────────────────────
                if !left_down {
                    state.active_drag = ActiveDrag::None;
                    state.pending_move_drag = None;
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
                                    // Clamp y so the titlebar cannot be dragged behind the
                                    // top panel; the panel reserved area is always accessible.
                                    win.y = (win.y as i32 + dcy).max(PANEL_TOP_RESERVED_H as i32)
                                        as u32;
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

                if INPUT_DEBUG {
                    debug_log("[DISPLAY] cursor=(");
                    debug_dec(state.pointer.x() as u32);
                    debug_log(",");
                    debug_dec(state.pointer.y() as u32);
                    debug_log(") final_dx=");
                    debug_i32(motion.final_dx);
                    debug_log(" final_dy=");
                    debug_i32(motion.final_dy);
                    debug_log(" capped=");
                    debug_dec(if motion.delta_capped { 1 } else { 0 });
                    debug_log(")\n");
                }

                state.prev_buttons = buttons;
                state.active_cursor = cursor_for_scene(&state);

                let new_cx = state.mouse_x as u32;
                let new_cy = state.mouse_y as u32;
                let button_changed = buttons != prev_buttons;
                let now_dragging = !matches!(state.active_drag, ActiveDrag::None);
                let window_changed =
                    button_changed || had_active_drag || now_dragging || overlay_changed;
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
            // SET_MOUSE_SETTINGS — control panel adjusts pointer sensitivity/accel.
            // words[0] = sensitivity_fp (i32 as u64)
            // words[1] = acceleration_enabled (0=off, 1=on)
            // -------------------------------------------------------------------
            SgpMsg::SET_MOUSE_SETTINGS => {
                let sens = msg.words[0] as i32;
                let accel = msg.words[1] != 0;
                if sens > 0 {
                    state.pointer.motion.sensitivity_fp = sens;
                }
                state.pointer.motion.acceleration_enabled = accel;
                let _ = ipc_reply(IpcMsg::with_label(SgpMsg::REPLY).word(0, 0));
            }

            // -------------------------------------------------------------------
            // SESSION_ACTIVATE — tty_server hands framebuffer to Desktop session.
            // -------------------------------------------------------------------
            SgpMsg::SESSION_ACTIVATE => {
                if !state.session_active {
                    debug_log("[DISPLAY] [SESSION] activated — Desktop owns framebuffer\n");
                    // First activation on the VirtIO backend: wire the scanout
                    // now, taking the display over from the VGA/TTY output.
                    activate_virtio_scanout(&mut state);
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
