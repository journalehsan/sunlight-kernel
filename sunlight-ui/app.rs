//! Application framework — event-loop wrapper around the display server SGP protocol.
//!
//! Provides [`App`] trait and [`Window`] struct that let app developers write:
//!
//! ```ignore
//! struct MyApp { ... }
//!
//! impl App for MyApp {
//!     fn view(&mut self, canvas: &mut Canvas, theme: &Theme) { ... }
//!     fn update(&mut self, event: Event) -> bool { ... }
//! }
//!
//! let mut window = Window::connect(WindowConfig {
//!     width: 640,
//!     height: 480,
//!     title: "My App",
//!     decoration: WindowDecoration::Normal,
//! }).unwrap();
//! window.run();
//! ```

use core::ptr;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use sunlight_ipc::{
    ipc_call, ipc_call_timeout,
    launch_trace::{self, LaunchSource, LaunchTrace},
    monotonic_millis, nameserver_lookup, process_yield, shm_create, shm_free, shm_map,
    CapabilityToken, IpcMsg, SgpMsg,
};

use crate::event::Event;
use crate::paint::Canvas;
use crate::theme::Theme;

/// Default timeout for `EVENT_POLL` (milliseconds).
/// When no event arrives within this window, the loop delivers `Event::Tick`.
const POLL_TIMEOUT_MS: u64 = 200;
const RUNNABLE_EVENT_POLL_TIMEOUT_MS: u64 = 16;
const MAX_LOCAL_TICKS_BEFORE_EVENT_POLL: u8 = 8;
const WINDOW_IPC_TIMEOUT_MS: u64 = 500;
const WINDOW_CREATE_BASE_TIMEOUT_MS: u64 = 2_000;
const WINDOW_CREATE_PER_MIB_TIMEOUT_MS: u64 = 1_000;
const WINDOW_CREATE_MAX_TIMEOUT_MS: u64 = 15_000;
const MIB_BYTES: u64 = 1024 * 1024;
static CLOSE_REQUESTED: AtomicBool = AtomicBool::new(false);
static CLIENT_CURSOR: AtomicU8 = AtomicU8::new(u8::MAX);
static CLIENT_WIDTH: AtomicU32 = AtomicU32::new(0);
static CLIENT_HEIGHT: AtomicU32 = AtomicU32::new(0);

const fn should_deliver_local_tick(timeout_ms: u64, local_tick_streak: u8) -> bool {
    timeout_ms == 0 && local_tick_streak < MAX_LOCAL_TICKS_BEFORE_EVENT_POLL
}

fn window_create_timeout_ms(width: u32, height: u32) -> u64 {
    let surface_bytes = u64::from(width)
        .saturating_mul(u64::from(height))
        .saturating_mul(core::mem::size_of::<u32>() as u64);
    let surface_mib = surface_bytes.saturating_add(MIB_BYTES - 1) / MIB_BYTES;
    WINDOW_CREATE_BASE_TIMEOUT_MS
        .saturating_add(surface_mib.saturating_mul(WINDOW_CREATE_PER_MIB_TIMEOUT_MS))
        .min(WINDOW_CREATE_MAX_TIMEOUT_MS)
}

/// Decode a packed display-server key word into an [`Event`].
///
/// Returns `None` when `key_word == 0` (no key in this poll). Printable ASCII,
/// backspace, and enter become [`Event::Key`]; everything else becomes
/// [`Event::KeyPress`] (including key-up and pure modifiers).
fn decode_key_event_word(key_word: u64) -> Option<Event> {
    if key_word == 0 {
        return None;
    }
    let (keycode, pressed, shift, ctrl, alt, super_key, ascii) =
        sunlight_ipc::unpack_key_event(key_word);
    if pressed {
        if let Some(ch) = ascii {
            match ch {
                0x20..=0x7E => return Some(Event::key(ch as char)),
                0x08 => return Some(Event::key('\u{8}')),
                b'\r' | b'\n' => return Some(Event::key('\n')),
                _ => {}
            }
        }
    }
    Some(Event::key_press(
        keycode, pressed, shift, ctrl, alt, super_key,
    ))
}

/// Yield until `started_ms + budget_ms` (or return immediately if already past).
fn idle_wait_remaining(started_ms: u64, budget_ms: u64) {
    let deadline = started_ms.saturating_add(budget_ms);
    while monotonic_millis() < deadline {
        process_yield();
    }
}

/// Cursor shape the client requests from the compositor.
///
/// Discriminants must stay in sync with the display server's `CursorShape`
/// enum so that the packed u8 discriminant transmitted via [`SgpMsg::SET_CURSOR`]
/// maps to the same shape in both processes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum CursorShape {
    Pointer = 0,
    Hand = 1,
    ResizeHorizontal = 2,
    ResizeVertical = 3,
    ResizeNwse = 4,
    ResizeNesw = 5,
    Move = 6,
    Wait = 7,
    Help = 8,
    Text = 9,
}

/// Request a cursor shape for the active window.  Call this from
/// [`App::update`] to change the cursor when the pointer moves over
/// different document regions.  The Window's event loop applies the
/// request after each update cycle.  If no request is made, the
/// previously-set cursor remains in effect.
///
/// This is a static trampoline because the [`App`] trait does not
/// receive a mutable reference to the [`Window`].  The initial cursor
/// is always [`CursorShape::Pointer`].
pub fn set_client_cursor(shape: CursorShape) {
    if shape == CursorShape::Pointer {
        // Pointer is the fallback. Preserve a more-specific request made by
        // another widget during the same event dispatch (for example when a
        // pointer crosses directly between two text inputs).
        let _ = CLIENT_CURSOR.compare_exchange(
            u8::MAX,
            shape as u8,
            Ordering::Relaxed,
            Ordering::Relaxed,
        );
    } else {
        CLIENT_CURSOR.store(shape as u8, Ordering::Relaxed);
    }
}

impl CursorShape {
    fn from_discriminant(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Pointer),
            1 => Some(Self::Hand),
            2 => Some(Self::ResizeHorizontal),
            3 => Some(Self::ResizeVertical),
            4 => Some(Self::ResizeNwse),
            5 => Some(Self::ResizeNesw),
            6 => Some(Self::Move),
            7 => Some(Self::Wait),
            8 => Some(Self::Help),
            9 => Some(Self::Text),
            _ => None,
        }
    }
}

pub fn request_close() {
    CLOSE_REQUESTED.store(true, Ordering::Relaxed);
}

fn take_requested_cursor() -> Option<CursorShape> {
    let v = CLIENT_CURSOR.swap(u8::MAX, Ordering::Relaxed);
    CursorShape::from_discriminant(v)
}

pub(crate) fn active_client_bounds() -> Option<crate::geom::Rect> {
    let width = CLIENT_WIDTH.load(Ordering::Relaxed);
    let height = CLIENT_HEIGHT.load(Ordering::Relaxed);
    (width > 0 && height > 0).then_some(crate::geom::Rect::new(0, 0, width, height))
}

fn take_close_requested() -> bool {
    CLOSE_REQUESTED.swap(false, Ordering::Relaxed)
}

/// Configuration passed to [`Window::connect`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum WindowDecoration {
    Normal = 0,
    CompactClose = 1,
    CompactCloseMinimize = 2,
    HiddenOverlay = 3,
}

impl WindowDecoration {
    const fn config_flag_bits(self) -> u64 {
        (self as u64) << sunlight_ipc::sgp::SgpMsg::config_flags::DECORATION_SHIFT
    }
}

pub struct WindowConfig {
    pub width: u32,
    pub height: u32,
    pub title: &'static str,
    pub decoration: WindowDecoration,
}

/// Explicit compositor material for a native window surface.
///
/// `Opaque` retains the historical XRGB copy path.
///
/// `WindowGlass` is a **reserved compatibility value**. It still requests
/// straight-alpha ARGB client composition so unused root pixels can be left
/// transparent, but the compositor maps it to **opaque** window chrome (solid
/// charcoal body and titlebar). It is **not** background blur, acrylic, or a
/// live translucent desktop backdrop. Prefer `Opaque` for new windows; keep
/// dense content opaque either way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowMaterial {
    Opaque,
    /// Reserved: straight-alpha client pixels over opaque compositor chrome.
    WindowGlass,
}

impl WindowMaterial {
    const fn config_flag_bits(self) -> u64 {
        match self {
            Self::Opaque => 0,
            Self::WindowGlass => sunlight_ipc::sgp::SgpMsg::config_flags::MATERIAL_WINDOW_GLASS,
        }
    }
}

/// Monotonic client-side evidence for the window event route.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EventPollCounters {
    pub window_id: u64,
    pub display_polls: u64,
    pub events_available: u64,
    pub events_dequeued: u64,
    pub wrong_window_replies: u64,
    pub local_ticks: u64,
    pub interleaved_polls: u64,
    pub active_workspace_id: u8,
    pub integrated_top_panel: bool,
}

/// An event-loop–driven window connected to the display server.
///
/// Owns the SHM-mapped framebuffer and manages the poll–update–commit cycle.
pub struct Window {
    pub width: u32,
    pub height: u32,

    win_id: u64,
    title: &'static str,
    buffer: *mut u32,
    buffer_size: usize,
    display_ep: CapabilityToken,
    shm_cap: CapabilityToken,
    launch_trace: LaunchTrace,

    /// Client-area origin reported by the display server (updated each poll).
    pub client_x: i32,
    pub client_y: i32,

    /// Previous poll button state, for detecting press/release transitions.
    prev_buttons: u8,
    /// Previous cursor position (screen coordinates), for detecting movement.
    prev_mouse_x: i32,
    prev_mouse_y: i32,
    prev_focused: bool,
    prev_pointer_owned: bool,
    prev_pointer_captured: bool,
    pending_event: Option<Event>,
    event_counters: EventPollCounters,
    /// Last cursor shape sent to the compositor, to avoid redundant IPC.
    current_cursor: CursorShape,
    window_valid: bool,
}

// SAFETY: the buffer pointer is valid for the lifetime of the Window.
unsafe impl Send for Window {}
unsafe impl Sync for Window {}

impl Window {
    fn display_call(&self, msg: IpcMsg) -> bool {
        matches!(
            ipc_call_timeout(self.display_ep, msg, WINDOW_IPC_TIMEOUT_MS),
            Ok(reply) if reply.label == SgpMsg::REPLY
        )
    }

    fn frame_len(&self) -> usize {
        self.width as usize * self.height as usize
    }

    fn draw_buffer_offset(&self) -> usize {
        let frame_len = self.frame_len();
        if self.buffer_size / 4 >= frame_len.saturating_mul(2) {
            frame_len
        } else {
            0
        }
    }

    /// Look up the display server, create a window, and map the shared buffer.
    pub fn connect(config: WindowConfig) -> Option<Self> {
        Self::connect_with_flags(config, 0)
    }

    pub fn connect_with_material(config: WindowConfig, material: WindowMaterial) -> Option<Self> {
        Self::connect_with_flags(config, material.config_flag_bits())
    }

    pub fn connect_with_flags(config: WindowConfig, config_flags: u64) -> Option<Self> {
        let trace =
            launch_trace::current().unwrap_or(LaunchTrace::new(0, LaunchSource::Unknown, 0));
        let pid = sunlight_ipc::getpid();
        launch_trace::log_phase_now(trace, config.title, "display_connect_start", Some(pid));
        let display_ep = nameserver_lookup("display_server")?;
        launch_trace::log_phase_now(trace, config.title, "display_connect_done", Some(pid));

        if trace.is_active() {
            let _ = ipc_call_timeout(
                display_ep,
                IpcMsg::with_label(SgpMsg::LAUNCH_TRACE)
                    .word(0, trace.launch_id)
                    .word(1, trace.source as u64)
                    .word(2, pid)
                    .word(3, trace.requested_at_ms),
                50,
            );
        }

        launch_trace::log_phase_now(trace, config.title, "window_create_request_sent", Some(pid));
        let title_bytes = config.title.as_bytes();
        let mut title_words = [0u64; 4];
        for (i, &b) in title_bytes.iter().enumerate().take(32) {
            title_words[i / 8] |= (b as u64) << ((i % 8) * 8);
        }
        let reply = ipc_call_timeout(
            display_ep,
            IpcMsg::with_label(SgpMsg::CREATE_WINDOW)
                .word(0, config.width as u64 | ((config.height as u64) << 32))
                .word(1, config.decoration.config_flag_bits() | config_flags)
                .word(2, pid)
                .word(3, title_words[0]),
            window_create_timeout_ms(config.width, config.height),
        )
        .ok()?;

        if reply.label != SgpMsg::REPLY || reply.cap_count == 0 {
            return None;
        }

        let win_id = reply.words[0];
        let buffer_size = reply.words[1] as usize;
        let shm_cap = reply.caps[0];

        // Map the shared framebuffer
        let buffer = shm_map(shm_cap).ok()? as *mut u32;

        // Set window title via CONFIGURE_WINDOW
        let mut title_cap = CapabilityToken::INVALID;
        let mut cfg = IpcMsg::with_label(SgpMsg::CONFIGURE_WINDOW)
            .word(0, win_id)
            .word(1, 0) // config_flags = 0 (no border changes)
            .word(2, 0) // pid|ppid
            .word(3, title_words[0]);

        if let Ok((title_ptr, cap)) = shm_create(4096, 0) {
            let copy_len = title_bytes.len().min(4095);
            unsafe {
                core::ptr::write_bytes(title_ptr, 0, 4096);
                core::ptr::copy_nonoverlapping(title_bytes.as_ptr(), title_ptr, copy_len);
            }
            cfg.caps[0] = cap;
            cfg.cap_count = 1;
            title_cap = cap;
        }
        let _ = ipc_call_timeout(display_ep, cfg, WINDOW_IPC_TIMEOUT_MS);
        if title_cap != CapabilityToken::INVALID {
            let _ = shm_free(title_cap);
        }
        launch_trace::log_phase_now(trace, config.title, "window_registered", Some(pid));

        CLIENT_WIDTH.store(config.width, Ordering::Relaxed);
        CLIENT_HEIGHT.store(config.height, Ordering::Relaxed);

        Some(Self {
            width: config.width,
            height: config.height,
            win_id,
            title: config.title,
            buffer,
            buffer_size,
            display_ep,
            shm_cap,
            launch_trace: trace,
            client_x: 0,
            client_y: 0,
            prev_buttons: 0,
            prev_mouse_x: 0,
            prev_mouse_y: 0,
            prev_focused: false,
            prev_pointer_owned: false,
            prev_pointer_captured: false,
            pending_event: None,
            event_counters: EventPollCounters {
                window_id: win_id,
                ..EventPollCounters::default()
            },
            current_cursor: CursorShape::Pointer,
            window_valid: true,
        })
    }

    pub const fn id(&self) -> u64 {
        self.win_id
    }

    /// Poll the display server for the next event.
    ///
    /// Uses a bounded timeout so the caller can refresh state even when
    /// no user input is arriving.
    pub fn poll_event(&mut self) -> Event {
        self.poll_event_timeout(POLL_TIMEOUT_MS)
    }

    /// Poll with an application-selected idle timeout. Interactive runtimes
    /// can request short idle waits while work is runnable, then return to the
    /// ordinary low-CPU cadence while blocked for input.
    ///
    /// # Why EVENT_POLL must not use a short `ipc_call_timeout`
    ///
    /// The display server **pops** a pending key into the EVENT_POLL reply.
    /// If the client arms a short IPC deadline and the display is busy
    /// compositing (full desktop redraws can take longer than 16–200 ms), the
    /// kernel late-drops the reply after the deadline — **and the key is
    /// already gone from the window queue**. Serial then shows
    /// `[DISPLAY] queued key win=N` with no matching app-side `Event::Key`.
    /// That is the sunlight-terminal "keyboard is dead" failure mode.
    ///
    /// Display always replies when it dequeues the request, so waiting without
    /// a short client deadline is correct. Idle `Event::Tick` cadence is
    /// implemented with a post-reply yield wait when the snapshot is empty.
    pub fn poll_event_timeout(&mut self, timeout_ms: u64) -> Event {
        if let Some(event) = self.pending_event.take() {
            return event;
        }
        self.event_counters.display_polls = self.event_counters.display_polls.wrapping_add(1);
        let idle_ms = timeout_ms.clamp(1, POLL_TIMEOUT_MS);
        let poll_started = monotonic_millis();
        // Unbounded wait for the display reply — see doc comment above.
        let reply = ipc_call(
            self.display_ep,
            IpcMsg::with_label(SgpMsg::EVENT_POLL).word(0, self.win_id),
        );
        if reply.label != SgpMsg::REPLY {
            idle_wait_remaining(poll_started, idle_ms);
            return Event::Tick;
        }

        if reply.words[3] & SgpMsg::EVENT_FLAG_WINDOW_VALID == 0 {
            self.event_counters.wrong_window_replies =
                self.event_counters.wrong_window_replies.wrapping_add(1);
            self.window_valid = false;
            idle_wait_remaining(poll_started, idle_ms);
            return Event::Tick;
        }
        self.window_valid = true;

        let desktop_state = reply.words[3];
        self.event_counters.active_workspace_id =
            ((desktop_state & SgpMsg::EVENT_DESKTOP_ACTIVE_WORKSPACE_MASK)
                >> SgpMsg::EVENT_DESKTOP_ACTIVE_WORKSPACE_SHIFT) as u8;
        self.event_counters.integrated_top_panel =
            desktop_state & SgpMsg::EVENT_DESKTOP_INTEGRATED_PANEL != 0;

        // words[0]: mouse_x (low 16) | mouse_y (high 16)
        let packed = reply.words[0];
        let mouse_x = (packed & 0xFFFF) as i32;
        let mouse_y = ((packed >> 16) & 0xFFFF) as i32;

        // words[1]: client-area origin (low 32 = clx, high 32 = cly)
        let origin = reply.words[1];
        if origin != 0 {
            self.client_x = (origin & 0xFFFF_FFFF) as i32;
            self.client_y = (origin >> 32) as i32;
        }

        // words[3]: mouse buttons plus focus/input-ownership flags.
        let input_word = reply.words[3];
        let buttons = (input_word & 0xFF) as u8;
        let focused = input_word & SgpMsg::EVENT_FLAG_FOCUSED != 0;
        let pointer_owned = input_word & SgpMsg::EVENT_FLAG_POINTER_OWNED != 0;
        let pointer_captured = input_word & SgpMsg::EVENT_FLAG_POINTER_CAPTURED != 0;
        let focus_press = input_word & SgpMsg::EVENT_FLAG_FOCUS_PRESS != 0;
        let local_x = mouse_x.saturating_sub(self.client_x);
        let local_y = mouse_y.saturating_sub(self.client_y);
        let changed = buttons ^ self.prev_buttons;
        // words[2]: packed key event (0 = none). Read early so a focus-edge
        // transition cannot permanently drop a key that arrived in the same
        // poll — that was a common "terminal has focus but no typing" failure
        // mode for apps that handle FocusChanged and then expect the next
        // poll to still carry the key.
        let key_word = reply.words[2];
        let wheel_event = if input_word & SgpMsg::EVENT_FLAG_WHEEL_VALID != 0 {
            let raw = ((packed & SgpMsg::EVENT_WHEEL_DELTA_MASK) >> SgpMsg::EVENT_WHEEL_DELTA_SHIFT)
                as u16;
            let delta = raw as i16;
            (delta != 0).then_some(Event::mouse_wheel(local_x, local_y, delta))
        } else {
            None
        };

        if focused != self.prev_focused {
            self.prev_focused = focused;
            // Never synthesize a press when focus returns, and never retain a
            // stale physical state after focus is lost.
            if focused && focus_press && changed != 0 {
                for btn in 0..3u8 {
                    let mask = 1u8 << btn;
                    if changed & mask != 0 && buttons & mask != 0 {
                        self.pending_event = Some(Event::mouse_down(local_x, local_y, btn));
                        break;
                    }
                }
            }
            // If no mouse edge was stashed, keep the key for the next poll so
            // FocusChanged does not steal the user's first keystroke.
            if self.pending_event.is_none() {
                if let Some(key_event) = decode_key_event_word(key_word) {
                    self.pending_event = Some(key_event);
                } else if let Some(wheel_event) = wheel_event {
                    self.pending_event = Some(wheel_event);
                }
            }
            self.prev_buttons = buttons;
            return Event::FocusChanged { focused };
        }
        let ownership_changed = pointer_owned != self.prev_pointer_owned
            || pointer_captured != self.prev_pointer_captured;
        if ownership_changed {
            self.prev_pointer_owned = pointer_owned;
            self.prev_pointer_captured = pointer_captured;
        }

        if let Some(key_event) = decode_key_event_word(key_word) {
            return key_event;
        }

        if let Some(wheel_event) = wheel_event {
            // Update tracking so stale movement is not replayed as
            // a redundant MouseMove on the next poll.
            self.prev_mouse_x = mouse_x;
            self.prev_mouse_y = mouse_y;
            return wheel_event;
        }

        // Detect button transitions (press/release) for each of the 3 buttons.
        // Priority: left > right > middle.
        for btn in 0..3u8 {
            let mask = 1u8 << btn;
            if changed & mask != 0 {
                let now_down = buttons & mask != 0;
                self.prev_buttons = buttons;
                self.prev_mouse_x = mouse_x;
                self.prev_mouse_y = mouse_y;
                return if now_down {
                    Event::mouse_down(local_x, local_y, btn)
                } else {
                    // Emit Click on left-button release for backwards compatibility.
                    if btn == 0 {
                        Event::click(local_x, local_y)
                    } else {
                        Event::mouse_up(local_x, local_y, btn)
                    }
                };
            }
        }

        if ownership_changed {
            return Event::PointerOwnership {
                owned: pointer_owned,
                captured: pointer_captured,
            };
        }

        // No button change — report movement if cursor moved.
        if mouse_x != self.prev_mouse_x || mouse_y != self.prev_mouse_y {
            self.prev_mouse_x = mouse_x;
            self.prev_mouse_y = mouse_y;
            return Event::mouse_move(local_x, local_y);
        }

        // Empty snapshot: wait out the idle budget so the app loop does not
        // spin at full CPU (44k+ EVENT_POLLs per session were observed).
        idle_wait_remaining(poll_started, idle_ms);
        Event::Tick
    }

    /// Mark the current framebuffer contents as ready for composition.
    ///
    /// Frame publication waits without the generic short window-operation
    /// deadline. A timed-out state-changing call can be canceled before a busy
    /// compositor dequeues it, leaving a newly created window permanently
    /// non-visible. The display server always acknowledges accepted commits.
    pub fn commit(&mut self) {
        let frame_len = self.frame_len();
        let draw_offset = self.draw_buffer_offset();
        if draw_offset != 0 {
            unsafe {
                // Draw into the hidden half of the shared mapping, then publish
                // the completed frame into the compositor-visible surface.
                ptr::copy_nonoverlapping(self.buffer.add(draw_offset), self.buffer, frame_len);
            }
        }
        let _ = ipc_call(
            self.display_ep,
            IpcMsg::with_label(SgpMsg::COMMIT_FRAME).word(0, self.win_id),
        );
    }

    /// Tell the compositor to remove this window.
    ///
    /// Best-effort and idempotent: the display server simply ignores a
    /// `CLOSE_WINDOW` for a window it no longer tracks, so calling this more
    /// than once (e.g. from the event loop and again from `Drop`) is harmless.
    ///
    /// This must be sent explicitly because apps terminate via
    /// `ProcessExit::exit`, which diverges and therefore never runs the
    /// `Window` destructor. Without this the process dies but its window stays
    /// registered in the compositor — visible, frozen, and unable to receive
    /// input — until something force-closes it (Ctrl+W).
    fn notify_close(&self) {
        let _ = self.display_call(IpcMsg::with_label(SgpMsg::CLOSE_WINDOW).word(0, self.win_id));
    }

    /// Send a `CONFIGURE_WINDOW` message to update window type, state, and border
    /// flags after the window has been created.
    ///
    /// `flags` uses the same bit layout as the SGP `config_flags` field:
    /// - bits [1:0]: window type (0=Normal, 1=Dialog, 2=Desktop, 3=Widget)
    /// - bits [3:2]: state (0=Normal, 1=Minimized, 2=Maximized, 3=Fullscreen)
    /// - bit  [4]:   border (0=Full chrome, 1=None)
    /// - bit  [5]:   z-index type (0=Normal, 1=OnTop)
    /// - bits [12:6]: z-index value 1–100 (0 = keep default)
    /// - bits [18:17]: decoration
    /// - bits [20:19]: explicit surface material
    ///
    /// Passing `flags = 0` is a no-op (the display server interprets zero as
    /// "no flags change").
    pub fn configure_flags(&self, flags: u64) {
        if flags == 0 {
            return;
        }
        let _ = self.display_call(
            IpcMsg::with_label(SgpMsg::CONFIGURE_WINDOW)
                .word(0, self.win_id)
                .word(1, flags)
                .word(2, 0)
                .word(3, 0),
        );
    }

    /// Request a cursor shape for this window's client area.
    ///
    /// Sends [`SgpMsg::SET_CURSOR`] to the compositor.  The compositor
    /// overrides the client-area cursor only when the pointer is inside
    /// the client area; window-chrome cursors (resize borders,
    /// title-bar pointer, etc.) always take priority.
    ///
    /// Callers should avoid spurious re-requests: the compositor itself
    /// caches the last known cursor shape, but the IPC is synchronous
    /// and unnecessary calls waste time.
    pub fn set_cursor(&self, shape: CursorShape) {
        let packed = self.win_id | ((shape as u64) << 32);
        let _ = self.display_call(IpcMsg::with_label(SgpMsg::SET_CURSOR).word(0, packed));
    }

    /// Build a `Canvas` wrapping the shared framebuffer.
    pub fn canvas(&mut self) -> Canvas<'_> {
        let frame_len = self.frame_len();
        let offset = self.draw_buffer_offset();
        let pixels = unsafe { core::slice::from_raw_parts_mut(self.buffer.add(offset), frame_len) };
        Canvas::new(pixels, self.width, self.width, self.height)
    }

    /// Run the application event loop.
    ///
    /// This is a convenience wrapper around [`Window::run_with`] that
    /// uses [`App`] and `Theme::sunlight_dark()`.
    pub fn run<A: App>(&mut self, app: &mut A) {
        let theme = Theme::sunlight_dark();
        self.run_with(app, &theme);
    }

    /// Run the event loop with a custom theme.
    pub fn run_with<A: App>(&mut self, app: &mut A, theme: &Theme) {
        take_close_requested();
        launch_trace::log_phase_now(
            self.launch_trace,
            self.title,
            "first_frame_or_first_draw",
            Some(sunlight_ipc::getpid()),
        );
        // Paint and commit the initial UI shell immediately so the window
        // appears on screen before any deferred/expensive work starts.
        {
            let mut c = self.canvas();
            app.view(&mut c, theme);
            self.commit();
        }

        // First frame is committed and visible. Fire on_ready() so the app
        // can kick off deferred work (filesystem discovery, network probing,
        // metadata refresh) without blocking the initial paint.
        // If on_ready() requests a redraw we honour it right away.
        launch_trace::log_phase_now(
            self.launch_trace,
            self.title,
            "on_ready_start",
            Some(sunlight_ipc::getpid()),
        );
        if app.on_ready() {
            let mut c = self.canvas();
            app.view(&mut c, theme);
            self.commit();
        }
        launch_trace::log_phase_now(
            self.launch_trace,
            self.title,
            "on_ready_done",
            Some(sunlight_ipc::getpid()),
        );

        let mut local_tick_streak = 0u8;
        loop {
            let timeout_ms = app.poll_timeout_ms();
            // Runnable apps receive a bounded local burst, then one bounded
            // display poll. This preserves input-independent progress without
            // allowing continuous execution to starve pointer/key snapshots.
            let event = if should_deliver_local_tick(timeout_ms, local_tick_streak) {
                local_tick_streak += 1;
                self.event_counters.local_ticks = self.event_counters.local_ticks.wrapping_add(1);
                Event::Tick
            } else {
                if timeout_ms == 0 {
                    self.event_counters.interleaved_polls =
                        self.event_counters.interleaved_polls.wrapping_add(1);
                }
                local_tick_streak = 0;
                self.poll_event_timeout(if timeout_ms == 0 {
                    RUNNABLE_EVENT_POLL_TIMEOUT_MS
                } else {
                    timeout_ms
                })
            };
            if !matches!(event, Event::Tick) {
                self.event_counters.events_available =
                    self.event_counters.events_available.wrapping_add(1);
                self.event_counters.events_dequeued =
                    self.event_counters.events_dequeued.wrapping_add(1);
            }
            let event_poll_redraw = app.event_poll_counters(self.event_counters);

            // Redraw requested?
            let needs_redraw = app.update(event) || event_poll_redraw;

            // Apply any cursor shape the app requested during update().
            if let Some(shape) = take_requested_cursor() {
                if shape != self.current_cursor {
                    self.set_cursor(shape);
                    self.current_cursor = shape;
                }
            }

            if take_close_requested() {
                // Remove our window from the compositor before returning. The
                // caller will `ProcessExit::exit`, which skips `Drop`, so this
                // is the only point the close actually reaches the display
                // server on a normal self-close.
                self.notify_close();
                break;
            }

            if !self.window_valid {
                break;
            }

            if needs_redraw {
                let mut c = self.canvas();
                app.view(&mut c, theme);
                self.commit();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        should_deliver_local_tick, window_create_timeout_ms, WindowDecoration,
        MAX_LOCAL_TICKS_BEFORE_EVENT_POLL,
    };

    #[test]
    fn decoration_flag_bits_match_protocol_layout() {
        assert_eq!(WindowDecoration::Normal.config_flag_bits(), 0);
        assert_eq!(WindowDecoration::CompactClose.config_flag_bits(), 1 << 17);
        assert_eq!(
            WindowDecoration::CompactCloseMinimize.config_flag_bits(),
            2 << 17
        );
        assert_eq!(WindowDecoration::HiddenOverlay.config_flag_bits(), 3 << 17);
    }

    #[test]
    fn runnable_local_ticks_are_bounded_by_an_event_poll() {
        for streak in 0..MAX_LOCAL_TICKS_BEFORE_EVENT_POLL {
            assert!(should_deliver_local_tick(0, streak));
        }
        assert!(!should_deliver_local_tick(
            0,
            MAX_LOCAL_TICKS_BEFORE_EVENT_POLL
        ));
        assert!(!should_deliver_local_tick(16, 0));
    }

    #[test]
    fn window_create_timeout_scales_with_surface_allocation() {
        assert_eq!(window_create_timeout_ms(1, 1), 3_000);
        assert_eq!(window_create_timeout_ms(960, 620), 5_000);
        assert_eq!(window_create_timeout_ms(1220, 760), 6_000);
        assert_eq!(window_create_timeout_ms(8192, 8192), 15_000);
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        // Normal window lifecycle cleanup: tell the compositor to forget this
        // window before we release our local SHM mapping. Drop must stay best-effort.
        let _ = self.display_call(IpcMsg::with_label(SgpMsg::CLOSE_WINDOW).word(0, self.win_id));
        // Client-side SHM cleanup still happens even if the display server is
        // already gone or the process is exiting.
        let _ = shm_free(self.shm_cap);
    }
}

/// Application trait — the primary interface for GUI programs.
///
/// Implementations define what to draw each frame and how to respond to
/// user input. The framework handles the event loop, IPC, and commit cycle.
///
/// # Startup responsiveness
///
/// The framework guarantees that `view()` is called and committed **before**
/// `on_ready()` is invoked, so the window shell is always visible to the user
/// before any deferred startup work begins.
///
/// Preferred pattern for apps that need to load data at startup:
///
/// ```ignore
/// fn view(&mut self, canvas: &mut Canvas, theme: &Theme) {
///     // Draw the window skeleton / loading placeholder immediately.
/// }
///
/// fn on_ready(&mut self) -> bool {
///     // Now kick off the expensive part: scan the filesystem, probe network,
///     // refresh metadata, etc.  Return true to request a redraw once done.
///     self.load_directory("/home");
///     true  // redraw with real content
/// }
///
/// fn update(&mut self, event: Event) -> bool {
///     // For work that must be chunked across ticks, do one small piece here
///     // each time Event::Tick fires (every ~200 ms) — this avoids blocking
///     // the event loop for long operations.
///     if matches!(event, Event::Tick) && self.pending_scan {
///         self.scan_next_chunk();
///     }
///     false
/// }
/// ```
pub trait App {
    /// Draw the current application state into the canvas.
    ///
    /// Called immediately at startup (before `on_ready`) and whenever
    /// `update()` or `on_ready()` returns `true`.
    fn view(&mut self, canvas: &mut Canvas, theme: &Theme);

    /// Handle an incoming event.
    ///
    /// Return `true` to request a redraw (your `view()` will be called).
    fn update(&mut self, event: Event) -> bool;

    /// Maximum event-poll sleep between update opportunities. Returning zero
    /// requests an immediate app-local [`Event::Tick`] without a display IPC
    /// round trip. The default preserves the normal low-idle-CPU 200 ms cadence.
    /// Apps should request immediate ticks only for bounded cooperative work.
    fn poll_timeout_ms(&self) -> u64 {
        POLL_TIMEOUT_MS
    }

    /// Receive monotonic evidence from the window event route. The default is
    /// a no-op; runtimes can retain or log it alongside their own input state.
    fn event_poll_counters(&mut self, _counters: EventPollCounters) -> bool {
        false
    }

    /// Called once after the **first frame is committed and visible** on screen.
    ///
    /// Use this hook to defer non-critical startup work — filesystem discovery,
    /// volume probing, network discovery, metadata refresh — so the window
    /// shell appears immediately even when that work is slow.
    ///
    /// Return `true` to request a redraw after this call completes.
    ///
    /// The default implementation is a no-op — existing apps that do not
    /// override this method continue to behave exactly as before.
    fn on_ready(&mut self) -> bool {
        false
    }
}
