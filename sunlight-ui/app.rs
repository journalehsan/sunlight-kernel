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

use core::sync::atomic::{AtomicBool, Ordering};
use sunlight_ipc::{
    ipc_call, ipc_call_timeout, nameserver_lookup, shm_free, shm_map, CapabilityToken, IpcMsg,
    SgpMsg,
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
    buffer: *mut u32,
    buffer_size: usize,
    display_ep: CapabilityToken,
    shm_cap: CapabilityToken,

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
    /// Look up the display server, create a window, and map the shared buffer.
    pub fn connect(config: WindowConfig) -> Option<Self> {
        let display_ep = nameserver_lookup("display_server")?;

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

        Some(Self {
            width: config.width,
            height: config.height,
            win_id,
            buffer,
            buffer_size,
            display_ep,
            shm_cap,
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
    pub fn commit(&self) {
        let _ = ipc_call(
            self.display_ep,
            IpcMsg::with_label(SgpMsg::COMMIT_FRAME).word(0, self.win_id),
        );
    }

    /// Build a `Canvas` wrapping the shared framebuffer.
    pub fn canvas(&mut self) -> Canvas<'_> {
        let pixels = unsafe { core::slice::from_raw_parts_mut(self.buffer, self.buffer_size / 4) };
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
        {
            let mut c = self.canvas();
            app.view(&mut c, theme);
            self.commit();
        }

        loop {
            let event = self.poll_event();

            // Redraw requested?
            let needs_redraw = app.update(event);
            if take_close_requested() {
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
pub trait App {
    /// Draw the current application state into the canvas.
    ///
    /// Called whenever `update()` returns `true`.
    fn view(&mut self, canvas: &mut Canvas, theme: &Theme);

    /// Handle an incoming event.
    ///
    /// Return `true` to request a redraw (your `view()` will be called).
    fn update(&mut self, event: Event) -> bool;
}
