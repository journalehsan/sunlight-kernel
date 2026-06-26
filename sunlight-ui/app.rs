//! Application framework — event-loop wrapper around the display server SGP protocol.
//!
//! Provides [`App`] trait and [`Window`] struct that let app developers write:
//!
//! ```ignore
//! struct MyApp { ... }
//!
//! impl App for MyApp {
//!     fn view(&self, canvas: &mut Canvas, theme: &Theme) { ... }
//!     fn update(&mut self, event: Event) -> bool { ... }
//! }
//!
//! let mut window = Window::connect(WindowConfig { width: 640, height: 480, title: "My App" }).unwrap();
//! window.run();
//! ```

use sunlight_ipc::{
    ipc_call, ipc_call_timeout, nameserver_lookup, shm_free, shm_map,
    CapabilityToken, IpcMsg, SgpMsg,
};

use crate::event::Event;
use crate::paint::Canvas;
use crate::theme::Theme;

/// Default timeout for `EVENT_POLL` (milliseconds).
/// When no event arrives within this window, the loop delivers `Event::Tick`.
const POLL_TIMEOUT_MS: u64 = 200;

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
        let buffer_size = reply.words[2] as usize;
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
            let keycode = (key_word & 0xFF) as u8;
            let pressed = ((key_word >> 8) & 0xFF) != 0;
            let ascii = if ((key_word >> 24) & 0xFF) != 0 {
                Some(((key_word >> 24) & 0xFF) as u8)
            } else {
                None
            };
            return Event::key(keycode, pressed, ascii);
        }

        // If left button was pressed, deliver a click event
        if buttons & 1 != 0 {
            let local_x = mouse_x.saturating_sub(self.client_x);
            let local_y = mouse_y.saturating_sub(self.client_y);
            return Event::click(local_x, local_y);
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
        let pixels = unsafe {
            core::slice::from_raw_parts_mut(self.buffer, self.buffer_size / 4)
        };
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
        loop {
            let event = self.poll_event();

            // Redraw requested?
            let needs_redraw = app.update(event);

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
        // Unmap the shared buffer
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
    fn view(&self, canvas: &mut Canvas, theme: &Theme);

    /// Handle an incoming event.
    ///
    /// Return `true` to request a redraw (your `view()` will be called).
    fn update(&mut self, event: Event) -> bool;
}
