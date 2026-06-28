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
//! let mut window = Window::connect(WindowConfig { width: 640, height: 480, title: "My App" }).unwrap();
//! window.run();
//! ```

use core::ptr;
use core::sync::atomic::{AtomicBool, Ordering};
use sunlight_ipc::{
    ipc_call, ipc_call_timeout,
    launch_trace::{self, LaunchSource, LaunchTrace},
    nameserver_lookup, shm_free, shm_map, CapabilityToken, IpcMsg, SgpMsg,
};

use crate::event::Event;
use crate::paint::Canvas;
use crate::theme::Theme;

/// Default timeout for `EVENT_POLL` (milliseconds).
/// When no event arrives within this window, the loop delivers `Event::Tick`.
const POLL_TIMEOUT_MS: u64 = 200;
static CLOSE_REQUESTED: AtomicBool = AtomicBool::new(false);

pub fn request_close() {
    CLOSE_REQUESTED.store(true, Ordering::Relaxed);
}

fn take_close_requested() -> bool {
    CLOSE_REQUESTED.swap(false, Ordering::Relaxed)
}

/// Configuration passed to [`Window::connect`].
pub struct WindowConfig {
    pub width: u32,
    pub height: u32,
    pub title: &'static str,
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
}

// SAFETY: the buffer pointer is valid for the lifetime of the Window.
unsafe impl Send for Window {}
unsafe impl Sync for Window {}

impl Window {
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
        let reply = ipc_call(
            display_ep,
            IpcMsg::with_label(SgpMsg::CREATE_WINDOW)
                .word(0, config.width as u64 | ((config.height as u64) << 32)),
        );

        if reply.label != SgpMsg::REPLY || reply.cap_count == 0 {
            return None;
        }

        let win_id = reply.words[0];
        let buffer_size = reply.words[1] as usize;
        let shm_cap = reply.caps[0];

        // Map the shared framebuffer
        let buffer = shm_map(shm_cap).ok()? as *mut u32;

        // Set window title via CONFIGURE_WINDOW
        let title_bytes = config.title.as_bytes();
        let mut title_words = [0u64; 4];
        for (i, &b) in title_bytes.iter().enumerate().take(32) {
            title_words[i / 8] |= (b as u64) << ((i % 8) * 8);
        }
        let cfg = IpcMsg::with_label(SgpMsg::CONFIGURE_WINDOW)
            .word(0, win_id)
            .word(1, 0) // config_flags = 0 (no border changes)
            .word(2, 0) // pid|ppid
            .word(3, title_words[0]);
        let _ = ipc_call(display_ep, cfg);
        launch_trace::log_phase_now(trace, config.title, "window_registered", Some(pid));

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
        })
    }

    /// Poll the display server for the next event.
    ///
    /// Uses a bounded timeout so the caller can refresh state even when
    /// no user input is arriving.
    pub fn poll_event(&mut self) -> Event {
        let reply = match ipc_call_timeout(
            self.display_ep,
            IpcMsg::with_label(SgpMsg::EVENT_POLL).word(0, self.win_id),
            POLL_TIMEOUT_MS,
        ) {
            Ok(r) if r.label == SgpMsg::REPLY => r,
            _ => return Event::Tick,
        };

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

        // words[3]: mouse buttons (low byte)
        let buttons = (reply.words[3] & 0xFF) as u8;

        // words[2]: packed key event (0 = none)
        let key_word = reply.words[2];
        if key_word != 0 {
            let (keycode, pressed, shift, ctrl, alt, super_key, ascii) =
                sunlight_ipc::unpack_key_event(key_word);
            if pressed {
                if let Some(ch) = ascii {
                    match ch {
                        0x20..=0x7E => return Event::key(ch as char),
                        0x08 => return Event::key('\u{8}'),
                        b'\r' | b'\n' => return Event::key('\n'),
                        _ => {}
                    }
                }
            }
            return Event::key_press(keycode, pressed, shift, ctrl, alt, super_key);
        }

        let local_x = mouse_x.saturating_sub(self.client_x);
        let local_y = mouse_y.saturating_sub(self.client_y);
        let changed = buttons ^ self.prev_buttons;

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

        // No button change — report movement if cursor moved.
        if mouse_x != self.prev_mouse_x || mouse_y != self.prev_mouse_y {
            self.prev_mouse_x = mouse_x;
            self.prev_mouse_y = mouse_y;
            return Event::mouse_move(local_x, local_y);
        }

        Event::Tick
    }

    /// Mark the current framebuffer contents as ready for composition.
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
        let _ = ipc_call(
            self.display_ep,
            IpcMsg::with_label(SgpMsg::CLOSE_WINDOW).word(0, self.win_id),
        );
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
    ///
    /// Passing `flags = 0` is a no-op (the display server interprets zero as
    /// "no flags change").
    pub fn configure_flags(&self, flags: u64) {
        if flags == 0 {
            return;
        }
        let _ = ipc_call(
            self.display_ep,
            IpcMsg::with_label(SgpMsg::CONFIGURE_WINDOW)
                .word(0, self.win_id)
                .word(1, flags)
                .word(2, 0)
                .word(3, 0),
        );
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

        loop {
            let event = self.poll_event();

            // Redraw requested?
            let needs_redraw = app.update(event);
            if take_close_requested() {
                // Remove our window from the compositor before returning. The
                // caller will `ProcessExit::exit`, which skips `Drop`, so this
                // is the only point the close actually reaches the display
                // server on a normal self-close.
                self.notify_close();
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

impl Drop for Window {
    fn drop(&mut self) {
        // Normal window lifecycle cleanup: tell the compositor to forget this
        // window before we release our local SHM mapping. Drop must stay best-effort.
        let _ = ipc_call(
            self.display_ep,
            IpcMsg::with_label(SgpMsg::CLOSE_WINDOW).word(0, self.win_id),
        );
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
