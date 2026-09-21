#![no_std]
#![cfg_attr(not(test), no_main)]

//! `sunlight-terminal` — graphical terminal emulator with multi-tab support.
//!
//! # Tab architecture
//!
//! Each [`TerminalTab`] owns a fully independent terminal session: its own
//! [`PtySession`], its own spawned shell process (`shell_pid`, via
//! `/bin/sshl<id>`), its own [`ModelGrid`] (`sunlight_tty::TerminalGrid`)
//! screen/cursor state, its own [`Footer`] (prompt/line-editor/history) and
//! [`OscParser`] state, and its own [`TabStatus`].
//!
//! [`TerminalApp`] owns the tab collection (`[Option<TerminalTab>; MAX_TABS]`
//! kept compacted at the front, plus a count and an `active` index), routes
//! input to the active tab only, and polls every tab for PTY output once per
//! `Event::Tick`.
//!
//! ## New-tab lifecycle: non-blocking by construction
//!
//! Creating a tab needs two round trips to the `pty` service (`CREATE` then
//! `SET_MODE`) plus a process spawn. Naively doing all of that inline inside
//! the click handler — using the plain, un-timed [`sunlight_ipc::ipc_call`],
//! which retries **forever** on `WouldBlock` — makes the *entire* window
//! event loop hang if the `pty` service is ever slow to answer a second
//! session while the first is already live: `Window::run_with` is a single
//! synchronous loop (poll → `App::update` → redraw), so any unbounded
//! blocking call made from `update()` freezes polling, rendering, *and*
//! input for every tab, not just the new one, with no way to recover.
//!
//! This is exactly the "new tab hangs the whole terminal" failure mode this
//! file guards against. The fix keeps the same request→create→spawn steps
//! but restructures them into a tiny state machine ([`SpawnStep`] /
//! [`PendingSpawn`]) advanced by [`TerminalApp::advance_pending_spawn`], one
//! bounded step per `Event::Tick`:
//!
//! - [`TerminalApp::spawn_tab`] (the click/shortcut handler) does **no
//!   IPC at all**. It only allocates a `TabStatus::Connecting` placeholder
//!   tab (instant, in-memory) and activates it, so the tab visibly appears
//!   immediately — this is what makes tab creation non-blocking from the
//!   UI's perspective (see `tab_state_allocated`/`first_tab_frame` in the
//!   phase log below).
//! - Each subsequent tick, [`TerminalApp::advance_pending_spawn`] attempts
//!   exactly one step (`CREATE`, then `SET_MODE`, then spawn the shell),
//!   using [`sunlight_ipc::ipc_call_timeout`] with a short
//!   [`SPAWN_STEP_TIMEOUT_MS`] budget instead of the unbounded `ipc_call`.
//!   A timeout just retries on the next tick; the render/input loop keeps
//!   running in between.
//! - An overall [`SPAWN_DEADLINE_MS`] wall-clock budget bounds the whole
//!   sequence. If it elapses (or the `pty`/shell step is rejected outright)
//!   the tab flips to `TabStatus::Failed` — a visible, closable tab state —
//!   instead of hanging forever.
//!
//! The very first tab (opened in `_start`, before the window/event loop
//! exists) drives the *same* state machine synchronously in a small bounded
//! loop, so the happy-path timing (visible in well under a second) is
//! unchanged, but a stuck `pty` service can no longer wedge the process
//! before it even opens a window.
//!
//! ### Phase log
//!
//! Every step above is logged via [`log_tab_phase`] with a monotonic
//! timestamp (`sunlight_ipc::monotonic_millis`), tagged with the numeric tab
//! id, to `debug_log` (serial): `tab_create_clicked`, `tab_state_allocated`,
//! `pty_request_sent`, `pty_created`, `shell_spawn_requested`,
//! `shell_spawned`, `tab_attached_to_pty`, `tab_focused`,
//! `first_tab_frame`, and `tab_create_failed` on the failure path.
//!
//! ## Keyboard shortcuts
//!
//! - `Ctrl+Tab` / `Ctrl+Shift+Tab`: next / previous tab.
//! - `Ctrl+T` / `Ctrl+Shift+T`: new tab.
//! - `Ctrl+Shift+W`: close the active tab (no-op if it's the last tab).
//! - `Alt+1..Alt+9`: jump to tab N (best-effort, see limitation below).
//!
//! ### Known input-stack limitation
//!
//! The display server's `Window::poll_event` (`sunlight-ui`) resolves a
//! pressed key to `Event::Key(char)` whenever the keyboard driver produced an
//! ASCII value for it — and that ASCII value is computed independently of
//! the Ctrl modifier (only Shift is factored in). This means `Ctrl+<letter>`
//! and `Ctrl+<digit>` cannot be observed as a distinct `Event::KeyPress` the
//! way `Ctrl+Tab` can (Tab has no ASCII mapping in the printable range, so it
//! always falls through to `Event::KeyPress` with accurate modifiers).
//!
//! To still offer `Ctrl+T`/`Ctrl+Shift+T`/`Alt+1..9`, this file tracks
//! `ctrl`/`alt` state itself from every `Event::KeyPress` it observes
//! (including presses of the modifier keys themselves, which *do* carry
//! accurate modifier bits) and consults that tracked state when a plain
//! `Event::Key(ch)` arrives. This is best-effort: if a modifier key-up event
//! is ever dropped by the input stack, tracked state can desync until the
//! next KeyPress event resynchronizes it.
//!
//! Plain `Ctrl+W` still can't be used here: the display server globally
//! intercepts it (unconditionally) to close the focused window outright,
//! before the event ever reaches this app. `Ctrl+Shift+W`, however, is
//! deliberately left unconsumed by that same interceptor specifically so
//! this app can bind it. See `docs/terminal/tab-support.md` for the full
//! writeup.
//!
//! ## Testing
//!
//! Pure tab-array bookkeeping (`insert_tab`/`remove_tab`/`switch_tab`/
//! `next_tab`/`prev_tab`), per-tab byte routing (`TerminalTab::ingest`), and
//! the non-blocking new-tab allocation path (`spawn_tab`,
//! `advance_pending_spawn`'s cancellation on close) have `#[cfg(test)]` unit
//! tests at the bottom of this file. They run on the host target
//! (`cargo test --target x86_64-unknown-linux-gnu -p sunlight-terminal`);
//! `no_main`/the global allocator/panic handler/`_start` are all gated with
//! `#[cfg(not(test))]` so a normal host test binary can link. Anything that
//! performs a real syscall (`PtySession::create_timeout`/`set_mode_timeout`/
//! `read`/`write`/`close`, `spawn_shell`, `libc::kill`, `libc::try_waitpid`)
//! cannot run on the host and is instead covered by the manual test plan in
//! `docs/terminal/tab-support.md`.
//!
//! ## Deferred / non-goals
//!
//! - Linux `SIGWINCH` delivery remains deferred until Helios can construct and
//!   return from real userspace signal frames. Winsize state itself updates
//!   immediately and is re-queryable after existing input/poll wakes.
//! - OSC window-title escape support — tab titles are `Tab N` today.
//! - Drag-to-reorder tabs, split panes, session restore.
//! - PTY reads/writes use short bounded IPC calls so a delayed PTY server
//!   cannot freeze the window's input and redraw loop.

extern crate alloc;

use sun_font::{self, FontRole, VecFont};
use sunlight_ipc::{
    debug_log, ipc_call_timeout,
    launch_trace::{self, LaunchSource, LaunchTrace},
    monotonic_millis, nameserver_lookup, process_yield, CapabilityToken, IpcCallError, IpcMsg,
    ProcessExit, PtyMsg, TerminalWinsize,
};
use sunlight_libc as libc;
use sunlight_tty::TerminalGrid as ModelGrid;
use sunlight_ui::{
    widgets::{Label, StatusBar},
    App, Canvas, Event, Point, Rect, VecText, Window, WindowConfig, WindowEvent,
};

static F_SMALL: VecFont = VecFont(FontRole::UiSmall);

const WIN_W: u32 = 900;
const WIN_H: u32 = 580;
const TAB_H: u32 = 34;
const FOOTER_H: u32 = 28;
const PAD_X: u32 = 10;
const PAD_Y: u32 = 10;
const CELL_W: u32 = 9;
const CELL_H: u32 = 20;
/// Allocation bounds for six independent grids in the terminal's reclaiming
/// 16 MiB heap. This covers a complete 4K client at the current cell metrics.
const MAX_RENDER_COLUMNS: u32 = 512;
const MAX_RENDER_ROWS: u32 = 128;

/// Client-area layout shared by sizing, rendering, and PTY publication.
/// `content` includes a one-pixel terminal border; only `content.inset(1)` is
/// drawable text space. Right/bottom remainders smaller than a complete cell
/// are intentionally ignored and remain painted with the terminal background.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TerminalLayout {
    client_width: u32,
    client_height: u32,
    content: Rect,
    footer: Rect,
    winsize: TerminalWinsize,
}

impl TerminalLayout {
    fn new(client_width: u32, client_height: u32) -> Self {
        let content_width = client_width.saturating_sub(PAD_X.saturating_mul(2));
        let content_height = client_height
            .saturating_sub(TAB_H)
            .saturating_sub(FOOTER_H)
            .saturating_sub(PAD_Y.saturating_mul(2));
        let grid_width = content_width.saturating_sub(2);
        let grid_height = content_height.saturating_sub(2);
        let columns = (grid_width / CELL_W)
            .max(1)
            .min(MAX_RENDER_COLUMNS)
            .min(TerminalWinsize::MAX_COLUMNS as u32) as u16;
        let rows = (grid_height / CELL_H)
            .max(1)
            .min(MAX_RENDER_ROWS)
            .min(TerminalWinsize::MAX_ROWS as u32) as u16;
        let pixel_width = u16::try_from(u32::from(columns).saturating_mul(CELL_W))
            .unwrap_or(TerminalWinsize::MAX_PIXELS);
        let pixel_height = u16::try_from(u32::from(rows).saturating_mul(CELL_H))
            .unwrap_or(TerminalWinsize::MAX_PIXELS);
        Self {
            client_width,
            client_height,
            content: Rect::new(
                PAD_X as i32,
                TAB_H.saturating_add(PAD_Y) as i32,
                content_width,
                content_height,
            ),
            footer: Rect::new(
                0,
                client_height.saturating_sub(FOOTER_H) as i32,
                client_width,
                FOOTER_H.min(client_height),
            ),
            winsize: TerminalWinsize::new(columns, rows, pixel_width, pixel_height),
        }
    }

    const fn cols(self) -> usize {
        self.winsize.columns as usize
    }

    const fn rows(self) -> usize {
        self.winsize.rows as usize
    }
}

const KEY_BACKSPACE: u8 = 0x0E;
const KEY_ENTER: u8 = 0x1C;
const KEY_UP: u8 = 0x48;
const KEY_DOWN: u8 = 0x50;
const KEY_LEFT: u8 = 0x4B;
const KEY_RIGHT: u8 = 0x4D;
const KEY_HOME: u8 = 0x47;
const KEY_END: u8 = 0x4F;
const KEY_DEL: u8 = 0x53;
/// PS/2 Set-1 scancode for Tab. Unlike letter/digit keys, Tab has no ASCII
/// mapping in `sunlight-kbd`'s `scancode_to_ascii`, so it always reaches this
/// app as `Event::KeyPress` with accurate Ctrl/Shift bits — see the
/// module-level doc comment for why that matters.
const KEY_TAB: u8 = 0x0F;

const INPUT_MAX: usize = 240;
const PROMPT_MAX: usize = 64;
const APP_NAME_MAX: usize = 32;
const HIST_MAX: usize = 32;
const READ_BUF: usize = PtyMsg::BULK_BYTES;
const ANSI_COLORS: [u32; 16] = [
    0xFF15191F, 0xFFE06C75, 0xFF98C379, 0xFFE5C07B, 0xFF61AFEF, 0xFFC678DD, 0xFF56B6C2, 0xFFD7DAE0,
    0xFF697383, 0xFFEF8790, 0xFFB3D994, 0xFFF2D399, 0xFF8BC8FF, 0xFFDDA2F0, 0xFF83D4DD, 0xFFF4F6FA,
];
const TERM_BG: u32 = ANSI_COLORS[0];
const TERM_SURFACE: u32 = 0xFF1D222B;
const TERM_SEPARATOR: u32 = 0xFF2D3541;
const TERM_ACCENT: u32 = 0xFFE8B86D;
const IDLE_POLL_TIMEOUT_MS: u64 = 16;
const STREAM_POLL_TIMEOUT_MS: u64 = 1;
const FAST_POLL_HOLD_MS: u64 = 80;
const PTY_DRAIN_BUDGET_MS: u64 = 4;

/// Signal number for a graceful stop request. Matches the constant used
/// elsewhere in the tree (e.g. `services/sunlight-display`, `sunlightd`).
const SIGTERM: u32 = 15;

/// Upper bound on concurrent tabs *per terminal window*. Kept modest because
/// `pty_server` only maintains `MAX_SESSIONS = 8` sessions for the whole
/// system (see `services/pty_server/src/main.rs`).
const MAX_TABS: usize = 6;
const TAB_TITLE_MAX: usize = 20;

/// Budget for a single `pty` IPC round trip while creating/closing a tab.
/// Chosen to be short enough that a stuck `pty` service only ever stalls the
/// UI loop for a barely-perceptible instant (never indefinitely, unlike the
/// plain `ipc_call` this replaces) while still comfortably covering a normal
/// same-machine reply, which completes in well under a millisecond.
const SPAWN_STEP_TIMEOUT_MS: u64 = 40;
/// Overall wall-clock budget for a tab to go from `Connecting` to `Running`.
/// If this elapses the tab is marked `Failed` instead of retrying forever.
const SPAWN_DEADLINE_MS: u64 = 1500;
/// Budget for the best-effort `PtyMsg::CLOSE` sent when a tab/session goes
/// away. Bounded for the same reason as `SPAWN_STEP_TIMEOUT_MS`.
const CLOSE_TIMEOUT_MS: u64 = 200;
/// Budget for PTY reads and writes performed by the window event loop.
/// These calls must never use the unbounded IPC helper: a delayed PTY reply
/// would otherwise freeze keyboard polling and framebuffer commits while the
/// display server continues queueing keys for the window.
///
/// 100 ms is still imperceptible as a frame stall, but is large enough that a
/// busy `pty_server` (shell hammering READ_SLAVE in a tight loop) does not
/// cause `write()` to give up mid-line — which previously cleared the footer
/// via `take_line` and then dropped the bytes, so Enter looked dead.
const PTY_IO_TIMEOUT_MS: u64 = 100;
/// How many times a single PTY write chunk may retry after a timeout before
/// the rest of the buffer is abandoned.
const PTY_WRITE_RETRIES: usize = 4;

#[cfg(not(test))]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    debug_log("[TERM] panic\n");
    loop {
        process_yield();
    }
}

/// Outcome of a single bounded `pty` IPC attempt. `Timeout` is retryable
/// (the caller tries again next tick); `Rejected` is a hard failure (bad
/// capability, malformed reply, or an explicit `PtyMsg::ERROR`) that should
/// not be retried.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PtyIoError {
    Timeout,
    Rejected,
}

impl From<IpcCallError> for PtyIoError {
    fn from(err: IpcCallError) -> Self {
        match err {
            IpcCallError::Timeout => PtyIoError::Timeout,
            IpcCallError::QueueFull | IpcCallError::Cancelled => PtyIoError::Timeout,
            IpcCallError::PeerClosed => PtyIoError::Rejected,
            _ => PtyIoError::Rejected,
        }
    }
}

struct PtySession {
    id: u64,
    generation: u64,
    service_cap: CapabilityToken,
    master: CapabilityToken,
    control: CapabilityToken,
    output_buffer: Option<sunlight_ipc::pty::PtyBuffer>,
}

impl PtySession {
    /// Request a new session against an already-resolved `pty` service
    /// service capability, bounded by `timeout_ms`. The returned master and
    /// control tokens are role-specific opaque authorities; the numeric ID is
    /// only a generation-qualified locator.
    ///
    /// Uses [`ipc_call_timeout`] rather than the unbounded `ipc_call` — see
    /// the module-level doc comment for why an unbounded call here is what
    /// causes the whole-terminal hang this file fixes.
    fn create_timeout(
        cap: CapabilityToken,
        size: TerminalWinsize,
        timeout_ms: u64,
    ) -> Result<Self, PtyIoError> {
        let reply = ipc_call_timeout(
            cap,
            IpcMsg::with_label(PtyMsg::CREATE)
                .word(0, PtyMsg::FLAG_CANONICAL | PtyMsg::FLAG_ECHO)
                .word(1, size.to_wire()),
            timeout_ms,
        )?;
        if reply.label != PtyMsg::REPLY || reply.cap_count < 2 {
            return Err(PtyIoError::Rejected);
        }
        Ok(Self {
            id: reply.words[0],
            generation: reply.words[1],
            service_cap: cap,
            master: reply.caps[0],
            control: reply.caps[1],
            output_buffer: sunlight_ipc::pty::PtyBuffer::new().ok(),
        })
    }

    fn set_mode_timeout(&self, mode_flags: u64, timeout_ms: u64) -> Result<(), PtyIoError> {
        let reply = ipc_call_timeout(
            self.service_cap,
            IpcMsg::with_label(PtyMsg::SET_MODE)
                .word(0, self.id)
                .word(1, self.generation)
                .word(2, mode_flags)
                .with_cap(0, self.control),
            timeout_ms,
        )?;
        if reply.label != PtyMsg::REPLY {
            return Err(PtyIoError::Rejected);
        }
        Ok(())
    }

    fn set_window_size_timeout(
        &self,
        size: TerminalWinsize,
        timeout_ms: u64,
    ) -> Result<bool, PtyIoError> {
        let reply = ipc_call_timeout(
            self.service_cap,
            IpcMsg::with_label(PtyMsg::SET_WINDOW_SIZE)
                .word(0, self.id)
                .word(1, self.generation)
                .word(2, size.to_wire())
                .with_cap(0, self.control),
            timeout_ms,
        )?;
        if reply.label != PtyMsg::REPLY {
            return Err(PtyIoError::Rejected);
        }
        Ok(reply.words[2] != 0)
    }

    fn set_foreground_process_timeout(&self, pid: u64, timeout_ms: u64) -> Result<(), PtyIoError> {
        let reply = ipc_call_timeout(
            self.service_cap,
            IpcMsg::with_label(PtyMsg::SET_FOREGROUND_PROCESS)
                .word(0, self.id)
                .word(1, self.generation)
                .word(2, pid)
                .with_cap(0, self.control),
            timeout_ms,
        )?;
        if reply.label != PtyMsg::REPLY {
            return Err(PtyIoError::Rejected);
        }
        Ok(())
    }

    fn write(&self, bytes: &[u8]) {
        #[cfg(test)]
        {
            let _ = bytes;
            return;
        }
        #[cfg(not(test))]
        {
            let mut pos = 0;
            while pos < bytes.len() {
                let chunk = (bytes.len() - pos).min(8);
                let mut msg = IpcMsg::with_label(PtyMsg::WRITE_MASTER)
                    .word(0, self.id)
                    .word(1, self.generation)
                    .word(2, chunk as u64)
                    .with_cap(0, self.master);
                let mut word = 0u64;
                for (byte_index, &byte) in bytes[pos..pos + chunk].iter().enumerate() {
                    word |= (byte as u64) << (byte_index * 8);
                }
                msg = msg.word(3, word);
                let mut attempt = 0usize;
                let reply = loop {
                    match ipc_call_timeout(self.service_cap, msg, PTY_IO_TIMEOUT_MS) {
                        Ok(reply) => break reply,
                        Err(IpcCallError::Timeout)
                        | Err(IpcCallError::QueueFull)
                        | Err(IpcCallError::Cancelled) => {
                            attempt += 1;
                            if attempt >= PTY_WRITE_RETRIES {
                                return;
                            }
                            process_yield();
                        }
                        Err(_) => return,
                    }
                };
                if reply.label != PtyMsg::REPLY {
                    break;
                }
                let accepted = (reply.words[2] as usize).min(chunk);
                if accepted == 0 {
                    break;
                }
                pos += accepted;
            }
        }
    }

    fn read(&self, out: &mut [u8]) -> usize {
        #[cfg(test)]
        {
            let _ = out;
            return 0;
        }
        #[cfg(not(test))]
        {
            if let Some(buffer) = self.output_buffer.as_ref() {
                let request = buffer.request(
                    PtyMsg::READ_MASTER_BULK,
                    self.id,
                    self.generation,
                    self.master,
                    out.len(),
                );
                let Ok(reply) = ipc_call_timeout(self.service_cap, request, PTY_IO_TIMEOUT_MS)
                else {
                    return 0;
                };
                if reply.label != PtyMsg::REPLY
                    || reply.words[0] != self.id
                    || reply.words[1] != self.generation
                {
                    return 0;
                }
                return buffer.copy_to(out, reply.words[2] as usize);
            }
            let started = monotonic_millis();
            let mut total = 0;
            while total < out.len() {
                let chunk = (out.len() - total).min(8);
                let reply = match ipc_call_timeout(
                    self.service_cap,
                    IpcMsg::with_label(PtyMsg::READ_MASTER)
                        .word(0, self.id)
                        .word(1, self.generation)
                        .word(2, chunk as u64)
                        .with_cap(0, self.master),
                    PTY_IO_TIMEOUT_MS,
                ) {
                    Ok(reply) => reply,
                    Err(_) => break,
                };
                if reply.label != PtyMsg::REPLY {
                    break;
                }
                let n = (reply.words[2] as usize).min(chunk);
                if n == 0 {
                    break;
                }
                for i in 0..n {
                    out[total + i] = ((reply.words[3] >> (i * 8)) & 0xFF) as u8;
                }
                total += n;
                if n < chunk || monotonic_millis().saturating_sub(started) >= PTY_DRAIN_BUDGET_MS {
                    break;
                }
            }
            total
        }
    }

    fn attach_slave_timeout(&self, timeout_ms: u64) -> Result<CapabilityToken, PtyIoError> {
        let reply = ipc_call_timeout(
            self.service_cap,
            IpcMsg::with_label(PtyMsg::ATTACH_SLAVE)
                .word(0, self.id)
                .word(1, self.generation)
                .word(2, 0)
                .with_cap(0, self.control),
            timeout_ms,
        )?;
        if reply.label != PtyMsg::REPLY || reply.cap_count < 1 {
            return Err(PtyIoError::Rejected);
        }
        Ok(reply.caps[0])
    }

    /// Release this session through its control authority. Best-effort and idempotent; bounded by
    /// [`CLOSE_TIMEOUT_MS`] so a stuck server can't wedge tab close/teardown
    /// either.
    fn close(&self) {
        #[cfg(test)]
        {
            return;
        }
        #[cfg(not(test))]
        let _ = ipc_call_timeout(
            self.service_cap,
            IpcMsg::with_label(PtyMsg::CLOSE_SESSION)
                .word(0, self.id)
                .word(1, self.generation)
                .with_cap(0, self.control),
            CLOSE_TIMEOUT_MS,
        );
    }
}

struct Footer {
    prompt: [u8; PROMPT_MAX],
    prompt_len: usize,
    input: [u8; INPUT_MAX],
    input_len: usize,
    input_cursor: usize,
    history: [[u8; INPUT_MAX]; HIST_MAX],
    history_lens: [usize; HIST_MAX],
    history_head: usize,
    history_count: usize,
    hist_pos: usize,
    hist_stash: [u8; INPUT_MAX],
    hist_stash_len: usize,
    app_mode: bool,
    app_name: [u8; APP_NAME_MAX],
    app_name_len: usize,
}

impl Footer {
    const fn new() -> Self {
        Self {
            prompt: [0; PROMPT_MAX],
            prompt_len: 0,
            input: [0; INPUT_MAX],
            input_len: 0,
            input_cursor: 0,
            history: [[0; INPUT_MAX]; HIST_MAX],
            history_lens: [0; HIST_MAX],
            history_head: 0,
            history_count: 0,
            hist_pos: 0,
            hist_stash: [0; INPUT_MAX],
            hist_stash_len: 0,
            app_mode: false,
            app_name: [0; APP_NAME_MAX],
            app_name_len: 0,
        }
    }

    fn set_prompt(&mut self, text: &[u8]) {
        self.prompt_len = text.len().min(PROMPT_MAX);
        self.prompt[..self.prompt_len].copy_from_slice(&text[..self.prompt_len]);
    }

    fn prompt_str(&self) -> &str {
        if self.prompt_len == 0 {
            "$ "
        } else {
            core::str::from_utf8(&self.prompt[..self.prompt_len]).unwrap_or("$ ")
        }
    }

    fn input_str(&self) -> &str {
        core::str::from_utf8(&self.input[..self.input_len]).unwrap_or("")
    }

    fn app_name_str(&self) -> &str {
        core::str::from_utf8(&self.app_name[..self.app_name_len]).unwrap_or("app")
    }

    fn input_prefix_str(&self) -> &str {
        core::str::from_utf8(&self.input[..self.input_cursor]).unwrap_or("")
    }

    fn input_suffix_str(&self) -> &str {
        core::str::from_utf8(&self.input[self.input_cursor..self.input_len]).unwrap_or("")
    }

    fn insert(&mut self, ch: u8) {
        if self.input_len >= INPUT_MAX {
            return;
        }
        let mut idx = self.input_len;
        while idx > self.input_cursor {
            self.input[idx] = self.input[idx - 1];
            idx -= 1;
        }
        self.input[self.input_cursor] = ch;
        self.input_len += 1;
        self.input_cursor += 1;
        self.hist_pos = 0;
    }

    fn backspace(&mut self) {
        if self.input_cursor == 0 {
            return;
        }
        let mut idx = self.input_cursor - 1;
        while idx + 1 < self.input_len {
            self.input[idx] = self.input[idx + 1];
            idx += 1;
        }
        self.input_len -= 1;
        self.input_cursor -= 1;
        self.hist_pos = 0;
    }

    fn delete_fwd(&mut self) {
        if self.input_cursor >= self.input_len {
            return;
        }
        let mut idx = self.input_cursor;
        while idx + 1 < self.input_len {
            self.input[idx] = self.input[idx + 1];
            idx += 1;
        }
        self.input_len -= 1;
    }

    fn move_left(&mut self) {
        if self.input_cursor > 0 {
            self.input_cursor -= 1;
        }
    }

    fn move_right(&mut self) {
        if self.input_cursor < self.input_len {
            self.input_cursor += 1;
        }
    }

    fn home(&mut self) {
        self.input_cursor = 0;
    }

    fn end(&mut self) {
        self.input_cursor = self.input_len;
    }

    fn history_up(&mut self) {
        if self.history_count == 0 || self.hist_pos >= self.history_count {
            return;
        }
        if self.hist_pos == 0 {
            self.hist_stash[..self.input_len].copy_from_slice(&self.input[..self.input_len]);
            self.hist_stash_len = self.input_len;
        }
        self.hist_pos += 1;
        let slot = (self.history_head + self.history_count - self.hist_pos) % HIST_MAX;
        let len = self.history_lens[slot];
        self.input[..len].copy_from_slice(&self.history[slot][..len]);
        self.input_len = len;
        self.input_cursor = len;
    }

    fn history_down(&mut self) {
        if self.hist_pos == 0 {
            return;
        }
        self.hist_pos -= 1;
        if self.hist_pos == 0 {
            self.input[..self.hist_stash_len]
                .copy_from_slice(&self.hist_stash[..self.hist_stash_len]);
            self.input_len = self.hist_stash_len;
            self.input_cursor = self.hist_stash_len;
            return;
        }
        let slot = (self.history_head + self.history_count - self.hist_pos) % HIST_MAX;
        let len = self.history_lens[slot];
        self.input[..len].copy_from_slice(&self.history[slot][..len]);
        self.input_len = len;
        self.input_cursor = len;
    }

    fn push_history(&mut self) {
        if self.input_len == 0 {
            return;
        }
        let slot = if self.history_count == HIST_MAX {
            let oldest = self.history_head;
            self.history_head = (self.history_head + 1) % HIST_MAX;
            oldest
        } else {
            let next = (self.history_head + self.history_count) % HIST_MAX;
            self.history_count += 1;
            next
        };
        self.history[slot][..self.input_len].copy_from_slice(&self.input[..self.input_len]);
        self.history_lens[slot] = self.input_len;
    }

    fn take_line(&mut self) -> ([u8; INPUT_MAX], usize) {
        self.push_history();
        let mut line = [0u8; INPUT_MAX];
        line[..self.input_len].copy_from_slice(&self.input[..self.input_len]);
        let len = self.input_len;
        self.input_len = 0;
        self.input_cursor = 0;
        self.hist_pos = 0;
        (line, len)
    }

    fn enter_app_mode(&mut self, name: &[u8]) {
        self.app_mode = true;
        self.app_name_len = name.len().min(APP_NAME_MAX);
        self.app_name[..self.app_name_len].copy_from_slice(&name[..self.app_name_len]);
    }

    fn exit_app_mode(&mut self) {
        self.app_mode = false;
        self.app_name_len = 0;
    }
}

struct OscParser {
    state: u8,
    body: [u8; 256],
    body_len: usize,
}

impl OscParser {
    const fn new() -> Self {
        Self {
            state: 0,
            body: [0; 256],
            body_len: 0,
        }
    }

    fn feed<F: FnMut(&[u8])>(
        &mut self,
        bytes: &[u8],
        console_out: &mut [u8],
        console_len: &mut usize,
        mut on_osc: F,
    ) {
        for &b in bytes {
            match self.state {
                0 => {
                    if b == 0x1B {
                        self.state = 1;
                    } else if *console_len < console_out.len() {
                        console_out[*console_len] = b;
                        *console_len += 1;
                    }
                }
                1 => {
                    if b == b']' {
                        self.state = 2;
                        self.body_len = 0;
                    } else {
                        push_console(console_out, console_len, 0x1B);
                        push_console(console_out, console_len, b);
                        self.state = 0;
                    }
                }
                2 => {
                    if b == 0x07 {
                        on_osc(&self.body[..self.body_len]);
                        self.body_len = 0;
                        self.state = 0;
                    } else if b == 0x1B {
                        self.state = 3;
                    } else if self.body_len < self.body.len() {
                        self.body[self.body_len] = b;
                        self.body_len += 1;
                    }
                }
                3 => {
                    if b == b'\\' {
                        on_osc(&self.body[..self.body_len]);
                        self.body_len = 0;
                        self.state = 0;
                    } else {
                        if self.body_len < self.body.len() {
                            self.body[self.body_len] = 0x1B;
                            self.body_len += 1;
                        }
                        if self.body_len < self.body.len() {
                            self.body[self.body_len] = b;
                            self.body_len += 1;
                        }
                        self.state = 2;
                    }
                }
                _ => self.state = 0,
            }
        }
    }
}

fn push_console(console_out: &mut [u8], console_len: &mut usize, byte: u8) {
    if *console_len < console_out.len() {
        console_out[*console_len] = byte;
        *console_len += 1;
    }
}

#[derive(Clone, Copy)]
enum OscCmd<'a> {
    Prompt(&'a [u8]),
    AppStart(&'a [u8]),
    AppDone,
    Unknown,
}

fn parse_osc(body: &[u8]) -> OscCmd<'_> {
    if !body.starts_with(b"9001;") {
        return OscCmd::Unknown;
    }
    let rest = &body[5..];
    if let Some(sep) = rest.iter().position(|&b| b == b';') {
        let op = &rest[..sep];
        let data = &rest[sep + 1..];
        if op == b"prompt" {
            return OscCmd::Prompt(data);
        }
        if op == b"app_start" {
            return OscCmd::AppStart(data);
        }
    } else if rest == b"app_done" {
        return OscCmd::AppDone;
    }
    OscCmd::Unknown
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct PaintedCell {
    cell: sunlight_tty::grid::Cell,
    cursor: bool,
}

struct TerminalViewport {
    rect: Rect,
    painted: alloc::vec::Vec<Option<PaintedCell>>,
}

impl TerminalViewport {
    fn new(rect: Rect) -> Self {
        Self {
            rect,
            painted: alloc::vec::Vec::new(),
        }
    }

    fn invalidate(&mut self, rect: Rect) {
        self.rect = rect;
        self.painted.clear();
    }

    fn draw(
        &mut self,
        canvas: &mut Canvas,
        grid: &ModelGrid,
        scrollback_offset: usize,
        cursor_on: bool,
    ) {
        let cols = grid.cols;
        let rows = grid.rows;
        self.painted.resize(cols * rows, None);
        let cursor_visible =
            grid.cursor_visible() && (scrollback_offset == 0 || grid.in_alt_screen()) && cursor_on;
        let cursor = grid.cursor();
        let mut clipped = canvas.sub_canvas(self.rect.inset(1));
        for row in 0..rows {
            for col in 0..cols {
                let cell = grid.viewport_cell(row, col, scrollback_offset);
                let painted = PaintedCell {
                    cell,
                    cursor: cursor_visible && cursor == (row, col),
                };
                let index = row * cols + col;
                if self.painted[index] == Some(painted) {
                    continue;
                }
                self.painted[index] = Some(painted);
                let foreground = if cell.bold && cell.fg < 8 {
                    cell.fg + 8
                } else {
                    cell.fg
                };
                let mut fg = ANSI_COLORS[foreground as usize % 16];
                let mut bg = ANSI_COLORS[cell.bg as usize % 16];
                if cell.inverse ^ painted.cursor {
                    core::mem::swap(&mut fg, &mut bg);
                }
                let rect = Rect::new(
                    col as i32 * CELL_W as i32,
                    row as i32 * CELL_H as i32,
                    CELL_W,
                    CELL_H,
                );
                let mut target = clipped.sub_canvas(rect);
                target.fill_rect(Rect::new(0, 0, CELL_W, CELL_H), sunlight_ui::Color(bg));
                if cell.ch.is_ascii_graphic() {
                    let bytes = [cell.ch];
                    let text = core::str::from_utf8(&bytes).unwrap_or("?");
                    let role = if cell.bold {
                        FontRole::MonoMedium
                    } else {
                        FontRole::MonoRegular
                    };
                    sun_font::draw_text(
                        &mut target,
                        text,
                        0,
                        (CELL_H.saturating_sub(sun_font::line_height(role)) / 2) as i32,
                        &sun_font::TextStyle::new(role, sunlight_ui::Color(fg)),
                    );
                }
                if cell.underline {
                    target.hbar(0, CELL_H as i32 - 3, CELL_W, 1, sunlight_ui::Color(fg));
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
struct DebugFlags {
    log_pty_stream: bool,
}

impl DebugFlags {
    const fn new() -> Self {
        Self {
            log_pty_stream: false,
        }
    }
}

/// Tracked keyboard-modifier state, refreshed from every `Event::KeyPress`.
/// See the module-level doc comment for why `Event::Key(char)` can't carry
/// this itself.
#[derive(Clone, Copy, Default)]
struct Mods {
    ctrl: bool,
    alt: bool,
}

impl Mods {
    fn clear(&mut self) {
        self.ctrl = false;
        self.alt = false;
    }
}

type TabId = u32;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TabStatus {
    /// Placeholder tab: allocated and focused, but its PTY/shell are still
    /// being brought up by [`TerminalApp::advance_pending_spawn`].
    Connecting,
    Running,
    Exited,
    /// PTY creation or shell spawn did not complete within
    /// [`SPAWN_DEADLINE_MS`], or was rejected outright. Shown distinctly and
    /// closable like any other tab — never a silent hang.
    Failed,
}

/// One step of the new-tab state machine driven by
/// [`TerminalApp::advance_pending_spawn`]. See the module-level doc comment.
enum SpawnStep {
    RequestPty,
    SetMode(PtySession),
    SpawnShell(PtySession),
    RegisterForeground(PtySession, u64),
}

/// The single in-flight tab creation, if any. Only one is allowed at a time
/// (enforced by [`TerminalApp::spawn_tab`]) so tab creation never contends
/// with itself over `pty_server` session ordering.
struct PendingSpawn {
    tab_id: TabId,
    step: SpawnStep,
    started_ms: u64,
}

/// One terminal tab: an independent PTY session, shell process, terminal
/// emulator/grid, and line-editor footer. `pty`/`shell_pid` are `None` while
/// `status == Connecting` (see [`SpawnStep`]) and are populated by
/// [`TerminalApp::attach_tab`] once the shell is actually running.
struct TerminalTab {
    id: TabId,
    title: [u8; TAB_TITLE_MAX],
    title_len: usize,
    pty: Option<PtySession>,
    shell_pid: Option<u64>,
    grid: ModelGrid,
    footer: Footer,
    osc: OscParser,
    status: TabStatus,
    /// Set when a background (non-active) tab receives PTY output the user
    /// hasn't seen yet. Cleared when the tab becomes active.
    dirty: bool,
    /// Set once `App::view` has drawn this tab for the first time, so the
    /// `first_tab_frame` phase is logged exactly once per tab.
    first_frame_logged: bool,
    /// Scrollback viewport offset: 0 = live output, 1..N = history lines
    /// above the visible screen.
    scrollback_offset: usize,
    /// The renderer grid has changed and this exact size still needs to be
    /// acknowledged by the authoritative PTY broker.
    geometry_dirty: bool,
}

impl TerminalTab {
    /// A brand-new placeholder tab with no PTY/shell yet — the immediate,
    /// zero-IPC result of clicking "+"/pressing `Ctrl+T` (see
    /// [`TerminalApp::spawn_tab`]).
    fn connecting(id: TabId, title: &[u8], layout: TerminalLayout) -> Self {
        let mut tab = Self {
            id,
            title: [0; TAB_TITLE_MAX],
            title_len: 0,
            pty: None,
            shell_pid: None,
            grid: ModelGrid::new(layout.cols(), layout.rows()),
            footer: Footer::new(),
            osc: OscParser::new(),
            status: TabStatus::Connecting,
            dirty: false,
            first_frame_logged: false,
            scrollback_offset: 0,
            geometry_dirty: false,
        };
        tab.set_title(title);
        tab
    }

    fn set_title(&mut self, text: &[u8]) {
        self.title_len = text.len().min(TAB_TITLE_MAX);
        self.title[..self.title_len].copy_from_slice(&text[..self.title_len]);
    }

    fn title_str(&self) -> &str {
        core::str::from_utf8(&self.title[..self.title_len]).unwrap_or("tab")
    }

    fn app_owns_input(&self) -> bool {
        self.footer.app_mode || self.grid.in_alt_screen()
    }

    /// Non-blocking liveness check for this tab's shell process. Uses the
    /// same `try_waitpid` primitive as the rest of the tree.
    fn refresh_status(&mut self) {
        if self.status != TabStatus::Running {
            return;
        }
        let Some(pid) = self.shell_pid else { return };
        if let Ok(Some(_exit_code)) = libc::try_waitpid(pid) {
            self.status = TabStatus::Exited;
        }
    }

    /// Drain one round of PTY output into this tab's OSC parser / grid.
    /// A no-op (returns `false`) while `pty` is `None` (i.e. `Connecting`
    /// or `Failed`).
    fn poll_pty(&mut self, read_buf: &mut [u8], console_buf: &mut [u8], debug: DebugFlags) -> bool {
        if self.pty.is_none() {
            return false;
        }
        let mut produced = false;
        let started = monotonic_millis();
        for _ in 0..4 {
            let n = self.pty.as_ref().map(|pty| pty.read(read_buf)).unwrap_or(0);
            if n == 0 {
                break;
            }
            self.ingest(&read_buf[..n], console_buf, debug);
            produced = true;
            if n < read_buf.len()
                || monotonic_millis().saturating_sub(started) >= PTY_DRAIN_BUDGET_MS
            {
                break;
            }
        }
        produced
    }

    fn publish_geometry(&mut self, size: TerminalWinsize) -> bool {
        if !self.geometry_dirty {
            return false;
        }
        let Some(pty) = self.pty.as_ref() else {
            return false;
        };
        if pty
            .set_window_size_timeout(size, SPAWN_STEP_TIMEOUT_MS)
            .is_ok()
        {
            self.geometry_dirty = false;
        }
        false
    }

    /// Feed already-read PTY bytes through this tab's OSC parser and into
    /// its own [`ModelGrid`]/[`Footer`]. Never touches any other tab's
    /// state, which is what keeps per-tab output isolated.
    fn ingest(&mut self, bytes: &[u8], console_buf: &mut [u8], debug: DebugFlags) {
        if debug.log_pty_stream {
            log_pty_bytes(bytes);
        }
        let mut console_len = 0usize;
        self.osc.feed(
            bytes,
            console_buf,
            &mut console_len,
            |body| match parse_osc(body) {
                OscCmd::Prompt(text) => self.footer.set_prompt(text),
                OscCmd::AppStart(name) => self.footer.enter_app_mode(name),
                OscCmd::AppDone => self.footer.exit_app_mode(),
                OscCmd::Unknown => {}
            },
        );
        if console_len > 0 {
            self.grid.feed(&console_buf[..console_len]);
        }
    }

    fn handle_raw_key(&mut self, keycode: u8, pressed: bool) -> bool {
        if !pressed {
            return false;
        }
        if self.app_owns_input() {
            let Some(pty) = self.pty.as_ref() else {
                return false;
            };
            let mut buf = [0u8; 4];
            let n = translate_special_key(keycode, &mut buf);
            if n > 0 {
                pty.write(&buf[..n]);
                return true;
            }
            return false;
        }
        // Shell-mode line editor: resolve special keys by scancode first so
        // keys that sometimes lack ASCII (numpad Enter is E0-prefixed and has
        // no ascii byte) still submit/edit. Matches the pre-UI path that
        // checked KEY_ENTER before the printable-ascii branch.
        // Footer editing does NOT require an attached PTY — only submit does.
        match keycode {
            KEY_ENTER => {
                if self.pty.is_none() {
                    return false;
                }
                self.submit_line();
                true
            }
            KEY_BACKSPACE => {
                self.footer.backspace();
                true
            }
            KEY_UP => {
                self.footer.history_up();
                true
            }
            KEY_DOWN => {
                self.footer.history_down();
                true
            }
            KEY_LEFT => {
                self.footer.move_left();
                true
            }
            KEY_RIGHT => {
                self.footer.move_right();
                true
            }
            KEY_HOME => {
                self.footer.home();
                true
            }
            KEY_END => {
                self.footer.end();
                true
            }
            KEY_DEL => {
                self.footer.delete_fwd();
                true
            }
            KEY_TAB => {
                self.footer.insert(b'\t');
                true
            }
            _ => false,
        }
    }

    fn handle_char(&mut self, ch: char) -> bool {
        if self.app_owns_input() {
            let Some(pty) = self.pty.as_ref() else {
                return false;
            };
            let byte = match ch {
                '\t' => b'\t',
                // POSIX terminals deliver Enter as carriage return in raw
                // mode. Crossterm treats LF as a character while raw mode is
                // active, so forwarding LF made Enter a no-op in Helios Note.
                '\n' => b'\r',
                '\u{8}' => 0x08,
                c if c.is_ascii() => c as u8,
                _ => 0,
            };
            if byte != 0 {
                pty.write(&[byte]);
                return true;
            }
            return false;
        }
        // Local footer line editor: always accept printable input even if the
        // PTY is still connecting, so typing is never "dead" while the shell
        // attaches. Submit still requires a live session.
        if ch == '\n' {
            if self.pty.is_none() {
                return false;
            }
            self.submit_line();
            return true;
        }
        if ch == '\u{8}' {
            self.footer.backspace();
            return true;
        }
        if ch.is_ascii_graphic() || ch == ' ' {
            self.footer.insert(ch as u8);
            return true;
        }
        false
    }

    fn submit_line(&mut self) {
        self.scrollback_offset = 0;
        let Some(pty) = self.pty.as_ref() else {
            return;
        };
        let (line, len) = self.footer.take_line();
        if len > 0 {
            pty.write(&line[..len]);
        }
        pty.write(b"\n");
    }

    /// Clean shutdown for this tab: stop the shell if it's still alive, then
    /// release the PTY session (if either was ever actually created — a
    /// `Connecting`/`Failed` tab may have neither).
    fn close(&self) {
        #[cfg(not(test))]
        if self.status == TabStatus::Running {
            if let Some(pid) = self.shell_pid {
                let _ = libc::kill(pid, SIGTERM);
            }
        }
        if let Some(pty) = &self.pty {
            pty.close();
        }
    }
}

/// The terminal window/app. Owns every open [`TerminalTab`] plus the state
/// shared across tabs (the `pty` service capability, scratch read buffers,
/// debug flags, tracked keyboard modifiers, and the one allowed in-flight
/// [`PendingSpawn`]).
struct TerminalApp {
    tabs: [Option<TerminalTab>; MAX_TABS],
    tab_count: usize,
    active: usize,
    poll_cursor: usize,
    next_tab_id: TabId,
    pty_cap: CapabilityToken,
    read_buf: [u8; READ_BUF],
    console_buf: [u8; READ_BUF + 2],
    debug: DebugFlags,
    mods: Mods,
    /// Compositor keyboard focus for this window. Keys are ignored while false
    /// so a lagging FocusChanged edge cannot inject into a background terminal.
    window_focused: bool,
    pending_spawn: Option<PendingSpawn>,
    layout: TerminalLayout,
    last_output_ms: u64,
    cursor_phase: bool,
    viewport: TerminalViewport,
    painted_view: Option<(TerminalLayout, TabId, TabStatus)>,
}

impl TerminalApp {
    const TAB_W: u32 = 124;
    /// Wider than a single glyph so the new-tab button has a comfortable
    /// click target.
    const NEW_TAB_W: u32 = 34;
    const CLOSE_BTN_SIZE: u32 = 14;

    fn new(pty_cap: CapabilityToken, debug: DebugFlags) -> Self {
        let layout = TerminalLayout::new(WIN_W, WIN_H);
        Self {
            tabs: core::array::from_fn(|_| None),
            tab_count: 0,
            active: 0,
            poll_cursor: 0,
            next_tab_id: 1,
            pty_cap,
            read_buf: [0; READ_BUF],
            console_buf: [0; READ_BUF + 2],
            debug,
            mods: Mods::default(),
            // Optimistic: a newly opened window is raised and focused by the
            // compositor. FocusChanged will correct this if we were wrong.
            window_focused: true,
            pending_spawn: None,
            layout,
            last_output_ms: 0,
            cursor_phase: true,
            viewport: TerminalViewport::new(layout.content),
            painted_view: None,
        }
    }

    fn active_tab_mut(&mut self) -> Option<&mut TerminalTab> {
        self.tabs.get_mut(self.active).and_then(|t| t.as_mut())
    }

    fn clear_tracked_mods(&mut self) {
        self.mods.clear();
    }

    fn set_client_size(&mut self, width: u32, height: u32) -> bool {
        let next = TerminalLayout::new(width, height);
        if next == self.layout {
            return false;
        }
        let dimensions_changed =
            (next.cols(), next.rows()) != (self.layout.cols(), self.layout.rows());
        if dimensions_changed {
            for tab in self.tabs.iter_mut().flatten() {
                if !tab.grid.resize(next.cols(), next.rows()) {
                    debug_log("[TERM] resize rejected: grid allocation failed\n");
                    return false;
                }
                tab.scrollback_offset = 0;
                tab.geometry_dirty = tab.pty.is_some();
            }
        }
        self.layout = next;
        for tab in self.tabs.iter_mut().flatten() {
            let _ = tab.publish_geometry(next.winsize);
        }
        true
    }

    fn tab_index_by_id(&self, id: TabId) -> Option<usize> {
        self.tabs
            .iter()
            .position(|t| t.as_ref().map(|t| t.id) == Some(id))
    }

    /// Request a new tab. Returns `false` (no-op) if at capacity or if a
    /// spawn is already in flight — existing tabs are left untouched either
    /// way.
    ///
    /// This performs **no IPC and no process spawn** — it only allocates an
    /// in-memory `TabStatus::Connecting` placeholder and activates it, then
    /// queues a [`PendingSpawn`] for [`Self::advance_pending_spawn`] to
    /// drive on subsequent ticks. That split is the fix for the
    /// whole-terminal hang described in the module doc comment: the
    /// click/shortcut handler that calls this can never block on `pty_server`
    /// or process spawn, no matter how slow either one is.
    fn spawn_tab(&mut self) -> bool {
        if self.tab_count >= MAX_TABS || self.pending_spawn.is_some() {
            return false;
        }

        let id = self.next_tab_id;
        self.next_tab_id = self.next_tab_id.wrapping_add(1);
        log_tab_phase(id, "tab_create_clicked");

        let mut title = [0u8; TAB_TITLE_MAX];
        let mut len = copy_ascii(b"Tab ", &mut title);
        len += fmt_u64(&mut title[len..], id as u64);

        if !self.insert_tab(TerminalTab::connecting(id, &title[..len], self.layout)) {
            return false;
        }
        self.clear_tracked_mods();
        log_tab_phase(id, "tab_state_allocated");
        // `insert_tab` always activates the tab it just inserted.
        log_tab_phase(id, "tab_focused");
        log_tab_phase(id, "pty_request_sent");

        self.pending_spawn = Some(PendingSpawn {
            tab_id: id,
            step: SpawnStep::RequestPty,
            started_ms: terminal_now_ms(),
        });
        true
    }

    /// Pure array bookkeeping: append `tab` and make it active. Returns
    /// `false` (leaving `self` untouched) if already at [`MAX_TABS`].
    ///
    /// Split out from [`Self::spawn_tab`] so tab-creation bookkeeping can be
    /// unit tested without a live PTY/process (see `tests` below).
    fn insert_tab(&mut self, tab: TerminalTab) -> bool {
        if self.tab_count >= MAX_TABS {
            return false;
        }
        let slot = self.tab_count;
        self.tabs[slot] = Some(tab);
        self.tab_count += 1;
        self.active = slot;
        true
    }

    /// Advance the in-flight tab creation (if any) by exactly one bounded
    /// step. Intended to be called once per `Event::Tick`. Returns `true` if
    /// anything changed (so callers can request a redraw).
    ///
    /// Every IPC step uses [`ipc_call_timeout`] with [`SPAWN_STEP_TIMEOUT_MS`]
    /// instead of an unbounded call, so a single stuck `pty_server` reply
    /// only ever delays this by one short, bounded step — it can retry on
    /// the next tick rather than freezing the caller. The overall elapsed
    /// time since the tab was requested is checked against
    /// [`SPAWN_DEADLINE_MS`] before every step; exceeding it fails the tab
    /// instead of retrying indefinitely.
    fn advance_pending_spawn(&mut self) -> bool {
        let Some(mut pending) = self.pending_spawn.take() else {
            return false;
        };

        if terminal_now_ms().saturating_sub(pending.started_ms) > SPAWN_DEADLINE_MS {
            self.mark_pending_failed(pending.tab_id, pending.step);
            return true;
        }

        let outcome: Result<Option<SpawnStep>, ()> = match pending.step {
            SpawnStep::RequestPty => {
                match PtySession::create_timeout(
                    self.pty_cap,
                    self.layout.winsize,
                    SPAWN_STEP_TIMEOUT_MS,
                ) {
                    Ok(pty) => {
                        log_tab_phase(pending.tab_id, "pty_created");
                        Ok(Some(SpawnStep::SetMode(pty)))
                    }
                    Err(PtyIoError::Timeout) => Ok(Some(SpawnStep::RequestPty)),
                    Err(PtyIoError::Rejected) => Err(()),
                }
            }
            SpawnStep::SetMode(pty) => match pty.set_mode_timeout(0, SPAWN_STEP_TIMEOUT_MS) {
                Ok(()) => Ok(Some(SpawnStep::SpawnShell(pty))),
                Err(PtyIoError::Timeout) => Ok(Some(SpawnStep::SetMode(pty))),
                Err(PtyIoError::Rejected) => {
                    pty.close();
                    Err(())
                }
            },
            SpawnStep::SpawnShell(pty) => {
                log_tab_phase(pending.tab_id, "shell_spawn_requested");
                let shell_id = u64::from(sunlight_ipc::PTY_TTY_TAB_BASE).saturating_add(pty.id);
                let slave = match pty.attach_slave_timeout(SPAWN_STEP_TIMEOUT_MS) {
                    Ok(slave) => slave,
                    Err(PtyIoError::Timeout) => {
                        return {
                            self.pending_spawn = Some(PendingSpawn {
                                tab_id: pending.tab_id,
                                step: SpawnStep::SpawnShell(pty),
                                started_ms: pending.started_ms,
                            });
                            true
                        }
                    }
                    Err(PtyIoError::Rejected) => {
                        pty.close();
                        self.mark_pending_failed_id(pending.tab_id);
                        return true;
                    }
                };
                match spawn_shell(&pty, slave, shell_id) {
                    Ok(shell_pid) => {
                        log_tab_phase(pending.tab_id, "shell_spawned");
                        Ok(Some(SpawnStep::RegisterForeground(pty, shell_pid)))
                    }
                    Err(_) => {
                        pty.close();
                        Err(())
                    }
                }
            }
            SpawnStep::RegisterForeground(pty, shell_pid) => {
                match pty.set_foreground_process_timeout(shell_pid, SPAWN_STEP_TIMEOUT_MS) {
                    Ok(()) => {
                        self.attach_tab(pending.tab_id, pty, shell_pid);
                        Ok(None)
                    }
                    Err(PtyIoError::Timeout) => {
                        Ok(Some(SpawnStep::RegisterForeground(pty, shell_pid)))
                    }
                    Err(PtyIoError::Rejected) => {
                        let _ = libc::kill(shell_pid, SIGTERM);
                        pty.close();
                        Err(())
                    }
                }
            }
        };

        match outcome {
            Ok(Some(step)) => {
                pending.step = step;
                self.pending_spawn = Some(pending);
            }
            Ok(None) => {
                // Attached successfully; nothing left pending.
            }
            Err(()) => self.mark_pending_failed_id(pending.tab_id),
        }
        true
    }

    /// Wire a freshly created PTY/shell into the tab that requested it. If
    /// that tab was closed while the spawn was still in flight, releases the
    /// now-orphaned PTY session instead of leaking a `pty_server` slot.
    fn attach_tab(&mut self, tab_id: TabId, pty: PtySession, shell_pid: u64) {
        if let Some(idx) = self.tab_index_by_id(tab_id) {
            if let Some(tab) = self.tabs[idx].as_mut() {
                tab.pty = Some(pty);
                tab.shell_pid = Some(shell_pid);
                tab.status = TabStatus::Running;
                // Covers a client resize that occurred while the PTY/shell was
                // still held by the pending-spawn state machine.
                tab.geometry_dirty = true;
                let _ = tab.publish_geometry(self.layout.winsize);
                log_tab_phase(tab_id, "tab_attached_to_pty");
                return;
            }
        }
        pty.close();
    }

    fn mark_pending_failed_id(&mut self, tab_id: TabId) {
        if let Some(idx) = self.tab_index_by_id(tab_id) {
            if let Some(tab) = self.tabs[idx].as_mut() {
                tab.status = TabStatus::Failed;
            }
        }
        log_tab_phase(tab_id, "tab_create_failed");
    }

    /// Like [`Self::mark_pending_failed_id`], but also releases a
    /// partially-created PTY session (deadline exceeded mid-`SetMode`/
    /// `SpawnShell`) so it isn't leaked in `pty_server`.
    fn mark_pending_failed(&mut self, tab_id: TabId, step: SpawnStep) {
        match step {
            SpawnStep::SetMode(pty)
            | SpawnStep::SpawnShell(pty)
            | SpawnStep::RegisterForeground(pty, _) => pty.close(),
            SpawnStep::RequestPty => {}
        }
        self.mark_pending_failed_id(tab_id);
    }

    /// Close the tab at `idx`. Running processes are signaled and the PTY
    /// session is released (see [`TerminalTab::close`]). If a tab creation
    /// was still in flight for this tab, it is cancelled and any
    /// partially-created PTY session is released rather than leaked.
    ///
    /// Closing the *last* remaining tab is deliberately a no-op — there is
    /// therefore always at least one tab open; closing the terminal itself
    /// requires closing the window.
    ///
    /// Returns `true` if a tab was actually closed.
    fn close_tab(&mut self, idx: usize) -> bool {
        if self.tab_count <= 1 {
            return false;
        }
        let Some(tab) = self.remove_tab(idx) else {
            return false;
        };
        let tab_id = tab.id;
        tab.close();

        if let Some(pending) = self.pending_spawn.take() {
            if pending.tab_id == tab_id {
                match pending.step {
                    SpawnStep::SetMode(pty)
                    | SpawnStep::SpawnShell(pty)
                    | SpawnStep::RegisterForeground(pty, _) => pty.close(),
                    SpawnStep::RequestPty => {}
                }
            } else {
                self.pending_spawn = Some(pending);
            }
        }
        self.clear_tracked_mods();
        true
    }

    /// Pure array bookkeeping: remove the tab at `idx`, shifting later tabs
    /// down to keep `tabs[0..tab_count]` contiguous and fixing up `active`.
    /// Returns the removed tab (still owning its live PTY/process — callers
    /// that want a clean shutdown must call [`TerminalTab::close`] on it, as
    /// [`Self::close_tab`] does) or `None` if `idx` is out of range.
    fn remove_tab(&mut self, idx: usize) -> Option<TerminalTab> {
        if idx >= self.tab_count {
            return None;
        }
        let removed = self.tabs[idx].take();
        for i in idx..self.tab_count - 1 {
            self.tabs[i] = self.tabs[i + 1].take();
        }
        self.tabs[self.tab_count - 1] = None;
        self.tab_count -= 1;

        if self.active > idx {
            self.active -= 1;
        }
        if self.tab_count > 0 && self.active >= self.tab_count {
            self.active = self.tab_count - 1;
        }
        if self.tab_count == 0 {
            self.poll_cursor = 0;
        } else {
            self.poll_cursor %= self.tab_count;
        }
        removed
    }

    fn switch_tab(&mut self, idx: usize) -> bool {
        if idx >= self.tab_count || idx == self.active {
            return false;
        }
        self.active = idx;
        self.clear_tracked_mods();
        if let Some(tab) = self.tabs[idx].as_mut() {
            tab.dirty = false;
            log_tab_phase(tab.id, "tab_focused");
        }
        true
    }

    fn next_tab(&mut self) -> bool {
        if self.tab_count == 0 {
            return false;
        }
        self.switch_tab((self.active + 1) % self.tab_count)
    }

    fn prev_tab(&mut self) -> bool {
        if self.tab_count == 0 {
            return false;
        }
        self.switch_tab((self.active + self.tab_count - 1) % self.tab_count)
    }

    /// Poll the active tab every tick, plus one background tab per tick in a
    /// round-robin, and advance any in-flight tab creation by one step.
    fn poll_all_tabs(&mut self) -> bool {
        let mut redraw = self.advance_pending_spawn();

        for tab in self.tabs.iter_mut().flatten() {
            let _ = tab.publish_geometry(self.layout.winsize);
        }

        if let Some(tab) = self.tabs.get_mut(self.active).and_then(|t| t.as_mut()) {
            let previous_status = tab.status;
            tab.refresh_status();
            redraw |= previous_status != tab.status;
            if tab.poll_pty(&mut self.read_buf, &mut self.console_buf, self.debug) {
                self.last_output_ms = monotonic_millis();
                redraw = true;
            }
        }
        if self.tab_count <= 1 {
            return redraw;
        }

        if self.poll_cursor >= self.tab_count {
            self.poll_cursor = 0;
        }
        let start = self.poll_cursor;
        let mut idx = start;
        for _ in 0..self.tab_count {
            if idx != self.active {
                self.poll_cursor = (idx + 1) % self.tab_count;
                if let Some(tab) = self.tabs[idx].as_mut() {
                    let previous_status = tab.status;
                    tab.refresh_status();
                    redraw |= previous_status != tab.status;
                    let produced =
                        tab.poll_pty(&mut self.read_buf, &mut self.console_buf, self.debug);
                    if produced && !tab.dirty {
                        tab.dirty = true;
                        redraw = true;
                    }
                }
                return redraw;
            }
            idx = (idx + 1) % self.tab_count;
        }
        self.poll_cursor = (start + 1) % self.tab_count;
        redraw
    }

    /// Release every remaining tab's PTY/process, plus any in-flight spawn.
    /// Called before process exit (window-close).
    fn shutdown_all_tabs(&mut self) {
        for slot in self.tabs.iter_mut() {
            if let Some(tab) = slot.take() {
                tab.close();
            }
        }
        self.tab_count = 0;
        if let Some(pending) = self.pending_spawn.take() {
            match pending.step {
                SpawnStep::SetMode(pty)
                | SpawnStep::SpawnShell(pty)
                | SpawnStep::RegisterForeground(pty, _) => pty.close(),
                SpawnStep::RequestPty => {}
            }
        }
    }

    fn tab_width(&self) -> u32 {
        let reserve = if self.tab_count < MAX_TABS {
            Self::NEW_TAB_W
        } else {
            0
        };
        (self.layout.client_width.saturating_sub(reserve) / self.tab_count.max(1) as u32)
            .min(Self::TAB_W)
    }

    fn tab_rect(&self, index: usize) -> Rect {
        let width = self.tab_width();
        Rect::new((index as u32 * width) as i32, 0, width, TAB_H)
    }

    fn new_tab_rect(&self) -> Rect {
        Rect::new(
            (self.tab_count as u32 * self.tab_width()) as i32,
            0,
            Self::NEW_TAB_W,
            TAB_H,
        )
    }

    fn close_btn_rect(tab: Rect) -> Rect {
        let s = Self::CLOSE_BTN_SIZE as i32;
        Rect::new(
            tab.right() - s - 6,
            tab.y + (TAB_H as i32 - s) / 2,
            Self::CLOSE_BTN_SIZE,
            Self::CLOSE_BTN_SIZE,
        )
    }

    /// Route a click within the tab strip to close/switch/new-tab. Returns
    /// `false` (and does nothing) for clicks outside the tab strip.
    fn handle_click(&mut self, x: i32, y: i32) -> bool {
        if y < 0 || y as u32 >= TAB_H {
            return false;
        }
        let point = Point::new(x, y);

        if self.tab_count > 1 {
            for i in 0..self.tab_count {
                let r = self.tab_rect(i);
                if r.w >= 48 && Self::close_btn_rect(r).contains(point) {
                    self.close_tab(i);
                    return true;
                }
            }
        }
        for i in 0..self.tab_count {
            if self.tab_rect(i).contains(point) {
                return self.switch_tab(i);
            }
        }
        if self.tab_count < MAX_TABS && self.new_tab_rect().contains(point) {
            return self.spawn_tab();
        }
        false
    }

    fn draw_tab_bar(&self, canvas: &mut Canvas, theme: &sunlight_ui::Theme) {
        canvas.fill_rect(
            Rect::new(0, 0, self.layout.client_width, TAB_H),
            sunlight_ui::Color(TERM_SURFACE),
        );
        canvas.hbar(
            0,
            TAB_H as i32 - 1,
            self.layout.client_width,
            1,
            sunlight_ui::Color(TERM_SEPARATOR),
        );

        for i in 0..self.tab_count {
            let Some(tab) = self.tabs[i].as_ref() else {
                continue;
            };
            let r = self.tab_rect(i);
            let active = i == self.active;

            canvas.fill_rect(
                r,
                sunlight_ui::Color(if active { TERM_BG } else { TERM_SURFACE }),
            );

            let text_color = match tab.status {
                TabStatus::Failed | TabStatus::Exited => theme.danger,
                TabStatus::Connecting => theme.warn,
                TabStatus::Running if active => sunlight_ui::Color(TERM_ACCENT),
                TabStatus::Running => theme.text_dim,
            };
            let label = if tab.app_owns_input() && !tab.footer.app_name_str().is_empty() {
                tab.footer.app_name_str()
            } else {
                tab.title_str()
            };
            let mut label_canvas =
                canvas.sub_canvas(Rect::new(r.x + 10, r.y, r.w.saturating_sub(36), r.h));
            F_SMALL.draw_vcenter(&mut label_canvas, label, 0, 0, TAB_H, text_color);

            if active {
                canvas.hbar(r.x, r.bottom() - 2, r.w, 2, sunlight_ui::Color(TERM_ACCENT));
            } else if tab.dirty {
                canvas.fill_rect(
                    Rect::new(r.right() - 10, r.y + 5, 4, 4),
                    sunlight_ui::Color(TERM_ACCENT),
                );
            }

            if self.tab_count > 1 && r.w >= 48 {
                let close_r = Self::close_btn_rect(r);
                F_SMALL.draw_vcenter(
                    canvas,
                    "x",
                    close_r.x + 2,
                    close_r.y,
                    close_r.h,
                    theme.text_dim,
                );
            }

            if i + 1 < self.tab_count {
                canvas.vline(
                    r.right() - 1,
                    r.y + 5,
                    TAB_H.saturating_sub(10),
                    sunlight_ui::Color(TERM_SEPARATOR),
                );
            }
        }

        if self.tab_count < MAX_TABS {
            let nr = self.new_tab_rect();
            let plus_w = sun_font::measure_text("+", FontRole::UiSmall).w as i32;
            let plus_x = nr.x + ((nr.w as i32 - plus_w) / 2).max(0);
            F_SMALL.draw_vcenter(canvas, "+", plus_x, nr.y, TAB_H, theme.text_dim);
        }
    }

    fn footer_center_text(tab: &TerminalTab) -> &'static str {
        match tab.status {
            TabStatus::Connecting => "Starting session...",
            TabStatus::Failed => "Session failed",
            TabStatus::Exited => "Session exited",
            TabStatus::Running => {
                if tab.app_owns_input() {
                    "App input active"
                } else {
                    "Shell input active"
                }
            }
        }
    }

    fn footer_right_text(tab: &TerminalTab, pending_spawn_for_tab: bool) -> &'static str {
        if pending_spawn_for_tab {
            "Starting..."
        } else if tab.scrollback_offset != 0 && !tab.grid.in_alt_screen() {
            "Scrollback"
        } else {
            match tab.status {
                TabStatus::Connecting => "Connecting...",
                TabStatus::Failed => "Failed",
                TabStatus::Exited => "Exited",
                TabStatus::Running if tab.app_owns_input() => "Running",
                TabStatus::Running => "Ready",
            }
        }
    }
}

/// Unit tests for the tab model.
///
/// These target `cargo test --target <host>`, not the kernel target: the
/// crate stays `#![no_std]` but `no_main`/the custom global allocator/panic
/// handler/`_start` are all gated with `#[cfg(not(test))]`.
///
/// Covered here: pure tab-array bookkeeping (`insert_tab`/`remove_tab`/
/// `switch_tab`/`next_tab`/`prev_tab`), per-tab byte routing
/// (`TerminalTab::ingest`), and — most importantly for this fix — that
/// `spawn_tab` allocates its placeholder tab and queues a `PendingSpawn`
/// *without performing any IPC or process spawn*, and that closing a tab
/// whose spawn is still pending cancels it cleanly. Anything that performs a
/// real syscall (`PtySession::create_timeout`/`set_mode_timeout`/`read`/
/// `write`/`close`, `spawn_shell`, `libc::kill`, `libc::try_waitpid`) cannot
/// run on the host and is instead covered by the manual test plan in
/// `docs/terminal/tab-support.md`.
#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;

    fn test_pty(id: u64) -> PtySession {
        PtySession {
            id,
            generation: 1,
            service_cap: CapabilityToken::INVALID,
            master: CapabilityToken::INVALID,
            control: CapabilityToken::INVALID,
            output_buffer: None,
        }
    }

    /// A tab already `Running` with a fake PTY/shell — used by tests that
    /// only exercise array bookkeeping / byte routing, not the connecting
    /// state machine.
    fn test_tab(id: TabId, title: &[u8]) -> TerminalTab {
        let mut tab = TerminalTab::connecting(id, title, TerminalLayout::new(WIN_W, WIN_H));
        tab.pty = Some(test_pty(id as u64));
        tab.shell_pid = Some(0);
        tab.status = TabStatus::Running;
        tab
    }

    fn test_app() -> TerminalApp {
        TerminalApp::new(CapabilityToken::INVALID, DebugFlags::new())
    }

    #[test]
    fn insert_tab_creates_first_tab() {
        let mut app = test_app();
        assert!(app.insert_tab(test_tab(1, b"Tab 1")));
        assert_eq!(app.tab_count, 1);
        assert_eq!(app.active, 0);
        assert_eq!(app.tabs[0].as_ref().unwrap().title_str(), "Tab 1");
    }

    #[test]
    fn insert_tab_adds_additional_tabs_and_activates_newest() {
        let mut app = test_app();
        assert!(app.insert_tab(test_tab(1, b"Tab 1")));
        assert!(app.insert_tab(test_tab(2, b"Tab 2")));
        assert_eq!(app.tab_count, 2);
        assert_eq!(app.active, 1);
        assert_eq!(app.tabs[1].as_ref().unwrap().title_str(), "Tab 2");
    }

    #[test]
    fn insert_tab_respects_max_tabs_capacity() {
        let mut app = test_app();
        for i in 0..MAX_TABS {
            assert!(app.insert_tab(test_tab(i as TabId + 1, b"Tab")));
        }
        assert_eq!(app.tab_count, MAX_TABS);
        assert!(!app.insert_tab(test_tab(99, b"Overflow")));
        assert_eq!(app.tab_count, MAX_TABS);
    }

    #[test]
    fn switch_tab_changes_active_and_clears_dirty_flag() {
        let mut app = test_app();
        app.insert_tab(test_tab(1, b"Tab 1"));
        app.insert_tab(test_tab(2, b"Tab 2"));
        app.tabs[0].as_mut().unwrap().dirty = true;
        assert_eq!(app.active, 1);

        assert!(app.switch_tab(0));
        assert_eq!(app.active, 0);
        assert!(!app.tabs[0].as_ref().unwrap().dirty);

        assert!(!app.switch_tab(0));

        assert!(app.next_tab());
        assert_eq!(app.active, 1);
        assert!(app.prev_tab());
        assert_eq!(app.active, 0);
    }

    #[test]
    fn remove_tab_shifts_later_tabs_and_reindexes_active() {
        let mut app = test_app();
        app.insert_tab(test_tab(1, b"Tab 1"));
        app.insert_tab(test_tab(2, b"Tab 2"));
        app.insert_tab(test_tab(3, b"Tab 3"));
        assert_eq!(app.active, 2);

        let removed = app.remove_tab(0).expect("tab 0 should be removed");
        assert_eq!(removed.title_str(), "Tab 1");
        assert_eq!(app.tab_count, 2);
        assert_eq!(app.tabs[0].as_ref().unwrap().title_str(), "Tab 2");
        assert_eq!(app.tabs[1].as_ref().unwrap().title_str(), "Tab 3");
        assert_eq!(app.active, 1);
    }

    #[test]
    fn remove_tab_out_of_range_is_a_no_op() {
        let mut app = test_app();
        app.insert_tab(test_tab(1, b"Tab 1"));
        assert!(app.remove_tab(5).is_none());
        assert_eq!(app.tab_count, 1);
    }

    #[test]
    fn removing_last_tab_reaches_zero_tabs() {
        let mut app = test_app();
        app.insert_tab(test_tab(1, b"Tab 1"));
        assert!(app.remove_tab(0).is_some());
        assert_eq!(app.tab_count, 0);
        assert!(app.remove_tab(0).is_none());
    }

    #[test]
    fn close_tab_is_a_no_op_on_the_last_remaining_tab() {
        let mut app = test_app();
        app.insert_tab(test_tab(1, b"Tab 1"));
        assert!(!app.close_tab(0));
        assert_eq!(app.tab_count, 1);
        assert_eq!(app.tabs[0].as_ref().unwrap().title_str(), "Tab 1");
    }

    #[test]
    fn ingest_routes_pty_output_into_its_own_grid() {
        let mut tab = test_tab(1, b"Tab 1");
        let mut console_buf = [0u8; READ_BUF];
        tab.ingest(b"hi", &mut console_buf, DebugFlags::new());
        assert_eq!(tab.grid.cell(0, 0).ch, b'h');
        assert_eq!(tab.grid.cell(0, 1).ch, b'i');
    }

    #[test]
    fn inactive_tab_output_does_not_leak_into_other_tabs() {
        let mut app = test_app();
        app.insert_tab(test_tab(1, b"Tab 1"));
        app.insert_tab(test_tab(2, b"Tab 2"));
        assert_eq!(app.active, 1);

        let mut console_buf = [0u8; READ_BUF];
        app.tabs[0]
            .as_mut()
            .unwrap()
            .ingest(b"top", &mut console_buf, DebugFlags::new());

        assert_eq!(app.tabs[0].as_ref().unwrap().grid.cell(0, 0).ch, b't');
        assert_eq!(app.tabs[1].as_ref().unwrap().grid.cell(0, 0).ch, b' ');
    }

    #[test]
    fn translate_special_key_maps_tab_to_tab_byte() {
        let mut buf = [0u8; 4];
        let n = translate_special_key(KEY_TAB, &mut buf);
        assert_eq!(n, 1);
        assert_eq!(buf[0], b'\t');
    }

    #[test]
    fn translate_special_key_maps_enter_to_raw_terminal_carriage_return() {
        let mut buf = [0u8; 4];
        let n = translate_special_key(KEY_ENTER, &mut buf);
        assert_eq!(n, 1);
        assert_eq!(buf[0], b'\r');
    }

    #[test]
    fn client_area_geometry_matches_renderer_grid_exactly() {
        let layout = TerminalLayout::new(WIN_W, WIN_H);
        let drawable = layout.content.inset(1);
        assert_eq!(layout.cols(), (drawable.w / CELL_W) as usize);
        assert_eq!(layout.rows(), (drawable.h / CELL_H) as usize);
        assert_eq!(
            layout.winsize.pixel_width as u32,
            layout.cols() as u32 * CELL_W
        );
        assert_eq!(
            layout.winsize.pixel_height as u32,
            layout.rows() as u32 * CELL_H
        );
        assert!(drawable.w.saturating_sub(layout.winsize.pixel_width as u32) < CELL_W);
        assert!(
            drawable
                .h
                .saturating_sub(layout.winsize.pixel_height as u32)
                < CELL_H
        );
    }

    #[test]
    fn independent_terminal_windows_keep_independent_geometry() {
        let a = TerminalLayout::new(1380, 870);
        let b = TerminalLayout::new(656, 468);
        assert_ne!(a.winsize, b.winsize);
        let resized_a = TerminalLayout::new(1540, 910);
        assert_ne!(resized_a.winsize, a.winsize);
        assert_eq!(b, TerminalLayout::new(656, 468));
    }

    #[test]
    fn resize_updates_every_tab_grid_to_reported_geometry() {
        let mut app = test_app();
        assert!(app.spawn_tab());
        app.pending_spawn = None;
        app.next_tab_id = 2;
        assert!(app.spawn_tab());
        app.pending_spawn = None;
        assert!(app.set_client_size(1380, 870));
        for tab in app.tabs.iter().flatten() {
            assert_eq!(
                (tab.grid.cols, tab.grid.rows),
                (app.layout.cols(), app.layout.rows())
            );
        }
    }

    // ---- Regression coverage for the new-tab hang fix -------------------
    //
    // These are the key regression tests for this bug: `spawn_tab` (the
    // click/Ctrl+T handler) must never touch the network/PTY/process
    // syscalls directly — only `advance_pending_spawn` (driven by
    // `Event::Tick`) may do that, bounded by `ipc_call_timeout`. Because
    // `spawn_tab` performs no IPC, these tests can (and do) call it
    // directly on the host target and assert on its *synchronous* result,
    // which is exactly the property that makes tab creation non-blocking
    // from the UI's perspective.

    #[test]
    fn spawn_tab_allocates_connecting_placeholder_without_any_blocking_call() {
        let mut app = test_app();
        assert!(app.spawn_tab());

        assert_eq!(app.tab_count, 1);
        assert_eq!(app.active, 0);
        let tab = app.tabs[0].as_ref().unwrap();
        assert_eq!(tab.status, TabStatus::Connecting);
        assert!(tab.pty.is_none());
        assert!(tab.shell_pid.is_none());

        let pending = app.pending_spawn.as_ref().expect("spawn should be queued");
        assert_eq!(pending.tab_id, tab.id);
        assert!(matches!(pending.step, SpawnStep::RequestPty));
    }

    #[test]
    fn spawn_tab_is_a_no_op_while_a_spawn_is_already_pending() {
        let mut app = test_app();
        assert!(app.spawn_tab());
        // A second click/shortcut before the first tab finished connecting
        // must not start a second concurrent spawn (and must not panic or
        // corrupt the first one).
        assert!(!app.spawn_tab());
        assert_eq!(app.tab_count, 1);
    }

    #[test]
    fn spawn_tab_respects_max_tabs_capacity() {
        let mut app = test_app();
        for _ in 0..MAX_TABS {
            assert!(app.insert_tab(test_tab(app.next_tab_id, b"Tab")));
            app.next_tab_id = app.next_tab_id.wrapping_add(1);
        }
        assert_eq!(app.tab_count, MAX_TABS);
        assert!(!app.spawn_tab());
        assert!(app.pending_spawn.is_none());
    }

    #[test]
    fn closing_a_tab_with_a_pending_spawn_cancels_it() {
        let mut app = test_app();
        app.insert_tab(test_tab(1, b"Tab 1"));
        assert!(app.spawn_tab());
        assert_eq!(app.tab_count, 2);
        assert!(app.pending_spawn.is_some());

        // Close the still-connecting tab (index 1).
        assert!(app.close_tab(1));
        assert_eq!(app.tab_count, 1);
        assert!(
            app.pending_spawn.is_none(),
            "closing the tab a spawn was targeting must cancel that spawn"
        );
    }

    #[test]
    fn closing_an_unrelated_tab_leaves_a_pending_spawn_intact() {
        let mut app = test_app();
        app.insert_tab(test_tab(1, b"Tab 1"));
        app.insert_tab(test_tab(2, b"Tab 2"));
        app.next_tab_id = 3;
        assert!(app.spawn_tab()); // tab 3, connecting
        assert_eq!(app.tab_count, 3);

        // Close tab 1 (index 0) -- unrelated to the pending spawn for tab 3.
        assert!(app.close_tab(0));
        assert_eq!(app.tab_count, 2);
        let pending = app
            .pending_spawn
            .as_ref()
            .expect("unrelated spawn survives");
        assert_eq!(pending.tab_id, 3);
    }

    #[test]
    fn advance_pending_spawn_is_a_cheap_no_op_when_nothing_is_pending() {
        let mut app = test_app();
        assert!(!app.advance_pending_spawn());
    }

    #[test]
    fn alt_screen_and_app_mode_own_keyboard_input() {
        let mut tab = test_tab(1, b"Tab 1");
        assert!(!tab.app_owns_input());
        let mut console_buf = [0u8; READ_BUF];
        tab.ingest(b"\x1b[?1049h", &mut console_buf, DebugFlags::new());
        assert!(tab.grid.in_alt_screen());
        assert!(tab.app_owns_input());

        tab.ingest(b"\x1b[?1049l", &mut console_buf, DebugFlags::new());
        assert!(!tab.grid.in_alt_screen());
        tab.footer.enter_app_mode(b"top");
        assert!(tab.app_owns_input());
        tab.footer.exit_app_mode();
        assert!(!tab.app_owns_input());
    }

    #[test]
    fn app_mode_does_not_treat_letters_as_new_tab_shortcuts() {
        let mut app = test_app();
        assert!(app.insert_tab(test_tab(1, b"Tab 1")));
        app.tabs[0].as_mut().unwrap().footer.enter_app_mode(b"top");
        app.mods.ctrl = true;
        // Ctrl+T must reach the app, not open a tab.
        let _ = app.update(Event::Key('t'));
        assert_eq!(app.tab_count, 1);
        assert!(app.pending_spawn.is_none());
        app.mods.ctrl = false;
        assert!(app.update(Event::Key('s')));
        assert!(app.update(Event::Key('q')));
        assert_eq!(app.tab_count, 1);
    }

    #[test]
    fn idle_poll_is_short_and_pending_spawn_is_immediate() {
        let app = test_app();
        assert_eq!(app.poll_timeout_ms(), IDLE_POLL_TIMEOUT_MS);
        let mut spawning = test_app();
        assert!(spawning.spawn_tab());
        assert_eq!(spawning.poll_timeout_ms(), 0);
    }

    #[test]
    fn terminal_font_matches_cell_geometry() {
        for role in [FontRole::MonoRegular, FontRole::MonoMedium] {
            assert_eq!(sun_font::measure_text("M", role).w, CELL_W);
            assert!(sun_font::line_height(role) <= CELL_H);
        }
    }

    #[test]
    fn resized_tabs_remain_inside_the_window_and_keep_their_hit_targets() {
        let mut app = test_app();
        for index in 0..MAX_TABS {
            assert!(app.insert_tab(test_tab(index as TabId + 1, b"Shell")));
        }
        app.set_client_size(360, 240);
        for index in 0..MAX_TABS {
            let rect = app.tab_rect(index);
            assert!(rect.right() <= app.layout.client_width as i32);
            let _ = app.handle_click(rect.x + 4, 8);
            assert_eq!(app.active, index);
        }
    }

    #[test]
    fn incremental_paint_matches_full_paint_after_updates_and_cursor_changes() {
        let theme = sunlight_ui::Theme::sunlight_dark();
        let mut app = test_app();
        assert!(app.insert_tab(test_tab(1, b"Shell")));
        let mut pixels = alloc::vec![0; (WIN_W * WIN_H) as usize];
        let sequences: &[&[u8]] = &[
            b"hello\r\n\x1b[1;32mworld\x1b[0m",
            b"\x1b[1;1H\x1b[2Kchanged",
            b"\x1b[?1049h\x1b[4;10H\x1b[4;34munderlined\x1b[0m",
            b"\x1b[?25l\x1b[2J\x1b[Hrefresh",
            b"\x1b[?1049l",
        ];
        for sequence in sequences {
            app.tabs[0].as_mut().unwrap().grid.feed(sequence);
            for phase in [true, false] {
                app.cursor_phase = phase;
                app.view(&mut Canvas::new(&mut pixels, WIN_W, WIN_W, WIN_H), &theme);
                let mut fresh = alloc::vec![0; pixels.len()];
                app.painted_view = None;
                app.view(&mut Canvas::new(&mut fresh, WIN_W, WIN_W, WIN_H), &theme);
                assert_eq!(pixels, fresh);
            }
        }
    }

    #[test]
    fn terminal_preview_and_small_window_clipping() {
        let theme = sunlight_ui::Theme::sunlight_dark();
        let mut app = test_app();
        assert!(app.insert_tab(test_tab(1, b"Shell")));
        let tab = app.tabs[0].as_mut().unwrap();
        tab.footer.set_prompt(b"sunlight@sunlight:~$ ");
        tab.grid.feed(b"\x1b[1;36mWelcome to SunlightOS\x1b[0m\r\n\r\n\x1b[33msunlight@sunlight:~$\x1b[0m ls\r\n\x1b[34mDocuments  Downloads  Pictures  Projects\x1b[0m\r\n\r\n\x1b[33msunlight@sunlight:~$\x1b[0m echo Ready for a new day\r\nReady for a new day\r\n");
        for byte in b"cargo build" {
            tab.footer.insert(*byte);
        }
        for mode in ["shell", "top"] {
            if mode == "top" {
                let tab = app.tabs[0].as_mut().unwrap();
                tab.footer.enter_app_mode(b"sunlight-top");
                tab.grid.feed(b"\x1b[?1049h\x1b[?25l\x1b[1;36m SUNLIGHT TOP\x1b[0m\r\n\r\n Uptime  00:12:45    Tasks  37    Cores  4\r\n\x1b[32m CPU    [||||||||                          ]  24%\r\n\x1b[34m Memory [||||||||||||                      ]  36%\x1b[0m\r\n\r\n\x1b[1;33m PID    STATE       CPU%    MEMORY    NAME\x1b[0m\r\n   1    sleeping     0.0     12 MB    init\r\n   3    sleeping     0.1     46 MB    vfs-server\r\n   4    sleeping     0.0     25 MB    tty-server\r\n  12    running      2.4     32 MB    sunlight-display\r\n  24    running      0.8     16 MB    sunlight-terminal\r\n\r\n\x1b[90m q Quit   s Sort   r Refresh\x1b[0m");
            }
            let mut pixels = alloc::vec![0; (WIN_W * WIN_H) as usize];
            app.painted_view = None;
            app.view(&mut Canvas::new(&mut pixels, WIN_W, WIN_W, WIN_H), &theme);
            if let Ok(prefix) = std::env::var("SUNLIGHT_TERMINAL_PREVIEW") {
                let mut image = alloc::format!("P6\n{} {}\n255\n", WIN_W, WIN_H).into_bytes();
                for pixel in pixels {
                    image.extend_from_slice(&[
                        (pixel >> 16) as u8,
                        (pixel >> 8) as u8,
                        pixel as u8,
                    ]);
                }
                std::fs::write(alloc::format!("{}-{}.ppm", prefix, mode), image).unwrap();
            }
        }
        for (width, height) in [(320, 180), (64, 64), (1, 1)] {
            assert!(app.set_client_size(width, height));
            let mut pixels = alloc::vec![0; (width * height) as usize];
            app.view(&mut Canvas::new(&mut pixels, width, width, height), &theme);
        }
    }
}

impl App for TerminalApp {
    fn view(&mut self, canvas: &mut Canvas, theme: &sunlight_ui::Theme) {
        if canvas.width < PAD_X * 2 + CELL_W + 2
            || canvas.height < TAB_H + FOOTER_H + PAD_Y * 2 + CELL_H + 2
        {
            canvas.fill_rect(
                Rect::new(0, 0, canvas.width, canvas.height),
                sunlight_ui::Color(TERM_BG),
            );
            self.painted_view = None;
            return;
        }
        let view = self.tabs[self.active]
            .as_ref()
            .map(|tab| (self.layout, tab.id, tab.status));
        if self.painted_view != view || view.is_none() {
            canvas.fill_rect(
                Rect::new(0, 0, self.layout.client_width, self.layout.client_height),
                sunlight_ui::Color(TERM_BG),
            );
            self.viewport.invalidate(self.layout.content);
            self.painted_view = view;
        }
        self.draw_tab_bar(canvas, theme);

        let content = self.layout.content;
        let footer = self.layout.footer;

        let Some(tab) = self.tabs[self.active].as_mut() else {
            StatusBar::new(footer, "", "no active tab", "").draw(canvas, theme);
            return;
        };

        if !tab.first_frame_logged {
            tab.first_frame_logged = true;
            log_tab_phase(tab.id, "first_tab_frame");
        }

        self.viewport.draw(
            canvas,
            &tab.grid,
            tab.scrollback_offset,
            self.cursor_phase && self.window_focused && tab.app_owns_input(),
        );

        let status_label = match tab.status {
            TabStatus::Connecting => Some("Connecting..."),
            TabStatus::Failed => Some("Failed to start shell"),
            TabStatus::Running | TabStatus::Exited => None,
        };
        if let Some(text) = status_label {
            Label::new(
                Rect::new(
                    content.x + 8,
                    content.y + 8,
                    content.w.saturating_sub(16),
                    20,
                ),
                text,
            )
            .with_font(&F_SMALL)
            .draw(canvas, theme);
        }

        let pending_spawn_for_tab =
            self.pending_spawn.as_ref().map(|pending| pending.tab_id) == Some(tab.id);
        canvas.fill_rect(footer, sunlight_ui::Color(TERM_SURFACE));
        canvas.hbar(
            footer.x,
            footer.y,
            footer.w,
            1,
            sunlight_ui::Color(TERM_SEPARATOR),
        );
        let status_width = if footer.w >= 480 { 124 } else { 0 };
        if status_width != 0 {
            let status = Rect::new(
                footer.right() - status_width as i32,
                footer.y,
                status_width,
                footer.h,
            );
            let mut status_canvas = canvas.sub_canvas(status);
            F_SMALL.draw_vcenter(
                &mut status_canvas,
                Self::footer_right_text(tab, pending_spawn_for_tab),
                4,
                0,
                footer.h,
                sunlight_ui::Color(ANSI_COLORS[8]),
            );
        }
        let input_area = Rect::new(
            PAD_X as i32,
            footer.y + 4,
            footer.w.saturating_sub(PAD_X * 2 + status_width),
            footer.h.saturating_sub(8),
        );
        let mut input_canvas = canvas.sub_canvas(input_area);
        if tab.app_owns_input() {
            let label = if tab.footer.app_name_str().is_empty() {
                "Terminal"
            } else {
                tab.footer.app_name_str()
            };
            F_SMALL.draw_vcenter(
                &mut input_canvas,
                label,
                0,
                0,
                input_area.h,
                sunlight_ui::Color(TERM_ACCENT),
            );
            if input_area.w > 320 {
                F_SMALL.draw_vcenter(
                    &mut input_canvas,
                    Self::footer_center_text(tab),
                    180,
                    0,
                    input_area.h,
                    sunlight_ui::Color(ANSI_COLORS[8]),
                );
            }
        } else {
            let role = FontRole::MonoRegular;
            let prompt_width = sun_font::measure_text(tab.footer.prompt_str(), role)
                .w
                .min(input_area.w / 2);
            {
                let mut prompt_canvas =
                    input_canvas.sub_canvas(Rect::new(0, 0, prompt_width, input_area.h));
                sun_font::draw_text(
                    &mut prompt_canvas,
                    tab.footer.prompt_str(),
                    0,
                    0,
                    &sun_font::TextStyle::new(role, sunlight_ui::Color(TERM_ACCENT)),
                );
            }
            let edit_rect = Rect::new(
                prompt_width as i32 + 4,
                0,
                input_area.w.saturating_sub(prompt_width + 4),
                input_area.h,
            );
            let mut edit_canvas = input_canvas.sub_canvas(edit_rect);
            let prefix_width = sun_font::measure_text(tab.footer.input_prefix_str(), role).w;
            let scroll = prefix_width.saturating_sub(edit_rect.w.saturating_sub(CELL_W));
            sun_font::draw_text(
                &mut edit_canvas,
                tab.footer.input_str(),
                -(scroll as i32),
                0,
                &sun_font::TextStyle::new(role, sunlight_ui::Color(ANSI_COLORS[7])),
            );
            if self.window_focused && self.cursor_phase {
                let caret = Rect::new(
                    prefix_width.saturating_sub(scroll) as i32,
                    0,
                    CELL_W,
                    edit_rect.h,
                );
                edit_canvas.fill_rect(caret, sunlight_ui::Color(TERM_ACCENT));
                if let Some(character) = tab.footer.input_suffix_str().chars().next() {
                    let mut bytes = [0; 4];
                    sun_font::draw_text(
                        &mut edit_canvas,
                        character.encode_utf8(&mut bytes),
                        caret.x,
                        0,
                        &sun_font::TextStyle::new(role, sunlight_ui::Color(TERM_BG)),
                    );
                }
            }
        }
    }

    fn update(&mut self, event: Event) -> bool {
        let mut dirty = false;
        match event {
            Event::Tick => {
                dirty |= self.poll_all_tabs();
                let now = monotonic_millis();
                let phase = ((now / 530) & 1) == 0;
                let cursor_visible = self.window_focused
                    && self.tabs[self.active].as_ref().is_some_and(|tab| {
                        !tab.app_owns_input()
                            || (tab.grid.cursor_visible() && tab.scrollback_offset == 0)
                    });
                if phase != self.cursor_phase {
                    self.cursor_phase = phase;
                    dirty |= cursor_visible;
                }
            }
            Event::FocusChanged { focused } => {
                dirty |= self.window_focused != focused;
                // Always resync modifiers on focus edges: a dropped key-up
                // while unfocused would otherwise leave ctrl/alt stuck and
                // turn the next plain letter into a shortcut.
                self.clear_tracked_mods();
                self.window_focused = focused;
                if focused {
                    debug_log("[TERM] focus gained\n");
                } else {
                    debug_log("[TERM] focus lost\n");
                }
            }
            Event::KeyPress {
                keycode,
                pressed,
                shift,
                ctrl,
                alt,
                ..
            } => {
                // Modifier tracking always runs so key-up events still clear
                // ctrl/alt even if the window is momentarily unfocused.
                self.mods = Mods { ctrl, alt };
                // Display only queues keys to the focused window. Receiving a
                // key therefore proves focus even if a prior FocusChanged
                // edge was wrong or lost (log showed keys queued to win=2
                // while the app dropped them as unfocused).
                if pressed {
                    self.window_focused = true;
                }
                if !self.window_focused {
                    return dirty;
                }
                if pressed && ctrl && keycode == KEY_TAB {
                    dirty |= if shift {
                        self.prev_tab()
                    } else {
                        self.next_tab()
                    };
                } else if let Some(tab) = self.active_tab_mut() {
                    dirty |= tab.handle_raw_key(keycode, pressed);
                }
            }
            Event::Key(ch) => {
                // Same as KeyPress: key delivery implies compositor focus.
                self.window_focused = true;
                log_term_key(ch, self.mods.ctrl, self.mods.alt);
                let app_input = self
                    .active_tab_mut()
                    .map(|tab| tab.app_owns_input())
                    .unwrap_or(false);
                if !app_input && self.mods.ctrl && (ch == 't' || ch == 'T') {
                    dirty |= self.spawn_tab();
                } else if !app_input && self.mods.ctrl && (ch == 'w' || ch == 'W') {
                    // Plain Ctrl+W never reaches here -- `sunlight-display`
                    // still intercepts it globally to close the window. Only
                    // Ctrl+Shift+W is left unconsumed for apps.
                    dirty |= self.close_tab(self.active);
                } else if !app_input
                    && self.mods.alt
                    && !self.mods.ctrl
                    && ch.is_ascii_digit()
                    && ch != '0'
                {
                    let idx = (ch as u8 - b'1') as usize;
                    dirty |= self.switch_tab(idx);
                } else if let Some(tab) = self.active_tab_mut() {
                    dirty |= tab.handle_char(ch);
                    if dirty {
                        log_term_footer_len(tab.footer.input_len);
                    }
                }
            }
            Event::Click { x, y } => {
                self.clear_tracked_mods();
                // A click that reaches the app implies the compositor focused
                // us; set this optimistically in case FocusChanged was coalesced.
                self.window_focused = true;
                dirty |= self.handle_click(x, y);
            }
            Event::MouseDown { .. } => {
                self.clear_tracked_mods();
                self.window_focused = true;
            }
            Event::MouseUp { .. } | Event::MouseMove { .. } | Event::PointerOwnership { .. } => {}
            Event::MouseWheel { x, y, delta } => {
                let content = self.layout.content;
                if content.contains(sunlight_ui::Point::new(x, y)) {
                    if let Some(tab) = self.active_tab_mut() {
                        if delta > 0 {
                            let max = tab.grid.scrollback_len();
                            if tab.scrollback_offset < max {
                                tab.scrollback_offset = (tab.scrollback_offset + 1).min(max);
                                dirty = true;
                            }
                        } else if delta < 0 {
                            if tab.scrollback_offset > 0 {
                                tab.scrollback_offset -= 1;
                                dirty = true;
                            }
                        }
                    }
                }
            }
        }
        dirty
    }

    fn window_event(&mut self, event: WindowEvent) -> bool {
        let WindowEvent::Resized { width, height } = event;
        self.set_client_size(width, height)
    }

    fn poll_timeout_ms(&self) -> u64 {
        let now = monotonic_millis();
        if self.pending_spawn.is_some() {
            0
        } else if self.last_output_ms != 0
            && now.saturating_sub(self.last_output_ms) < FAST_POLL_HOLD_MS
        {
            STREAM_POLL_TIMEOUT_MS
        } else {
            IDLE_POLL_TIMEOUT_MS
        }
    }
}

/// Log one lifecycle phase for tab `tab_id` with a monotonic timestamp.
/// Format: `[TERM][TAB] tab=<id> phase=<phase> t=<monotonic_ms>ms`.
///
/// This is what makes the new-tab path traceable end to end (see the
/// module-level doc comment for the full phase list): grepping serial output
/// for `tab=<id>` shows exactly how long each step took and where a hang (or
/// failure) occurred, without needing a debugger attached to a `no_std`
/// process.
fn log_tab_phase(tab_id: TabId, phase: &str) {
    #[cfg(test)]
    {
        let _ = (tab_id, phase);
        return;
    }
    let mut buf = [0u8; 96];
    let mut len = 0usize;
    len += copy_ascii(b"[TERM][TAB] tab=", &mut buf[len..]);
    len += fmt_u64(&mut buf[len..], tab_id as u64);
    len += copy_ascii(b" phase=", &mut buf[len..]);
    len += copy_ascii(phase.as_bytes(), &mut buf[len..]);
    len += copy_ascii(b" t=", &mut buf[len..]);
    len += fmt_u64(&mut buf[len..], terminal_now_ms());
    len += copy_ascii(b"ms\n", &mut buf[len..]);
    if let Ok(text) = core::str::from_utf8(&buf[..len]) {
        debug_log(text);
    }
}

#[cfg(not(test))]
fn terminal_now_ms() -> u64 {
    monotonic_millis()
}

#[cfg(test)]
fn terminal_now_ms() -> u64 {
    0
}

#[cfg(not(test))]
#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8, _envp: *const *const u8) -> ! {
    sunlight_libc::launch_trace::init_from_argv(argc, argv);
    let trace = launch_trace::current().unwrap_or(LaunchTrace::new(0, LaunchSource::Unknown, 0));
    launch_trace::log_phase_now(
        trace,
        "app=terminal",
        "app_main_started",
        Some(sunlight_ipc::getpid()),
    );

    let Some(pty_cap) = nameserver_lookup("pty") else {
        loop {
            process_yield();
        }
    };

    let mut app = TerminalApp::new(pty_cap, parse_debug_flags(argc, argv));

    // Drive the very first tab through the same bounded, non-blocking state
    // machine used for every later tab (see `advance_pending_spawn`). In the
    // healthy case (the reported-working launch path) this resolves within
    // a handful of near-instant local IPC round trips, exactly matching the
    // old behavior's timing; the difference is that a stuck `pty` service
    // can no longer hang this loop forever -- `SPAWN_DEADLINE_MS` bounds it,
    // after which the window still opens, showing a failed first tab
    // instead of a process that never becomes visible at all.
    app.spawn_tab();
    while app.pending_spawn.is_some() {
        app.advance_pending_spawn();
        if app.pending_spawn.is_some() {
            process_yield();
        }
    }

    let mut window = match Window::connect(WindowConfig {
        width: WIN_W,
        height: WIN_H,
        title: "Sunlight Terminal",
        decoration: sunlight_ui::WindowDecoration::Normal,
    }) {
        Some(window) => window,
        None => loop {
            process_yield();
        },
    };
    window.run(&mut app);
    // Defensive: under normal operation `close_tab` has already released
    // every tab's PTY/process by the time the window-close flow above
    // returns. This just guards against leaking sessions if that
    // invariant is ever violated.
    app.shutdown_all_tabs();
    ProcessExit::exit(0);
}

fn translate_special_key(keycode: u8, buf: &mut [u8; 4]) -> usize {
    match keycode {
        KEY_ENTER => {
            buf[0] = b'\r';
            1
        }
        KEY_TAB => {
            buf[0] = b'\t';
            1
        }
        KEY_BACKSPACE => {
            buf[0] = 0x08;
            1
        }
        KEY_UP => {
            buf[0] = 0x1B;
            buf[1] = b'[';
            buf[2] = b'A';
            3
        }
        KEY_DOWN => {
            buf[0] = 0x1B;
            buf[1] = b'[';
            buf[2] = b'B';
            3
        }
        KEY_RIGHT => {
            buf[0] = 0x1B;
            buf[1] = b'[';
            buf[2] = b'C';
            3
        }
        KEY_LEFT => {
            buf[0] = 0x1B;
            buf[1] = b'[';
            buf[2] = b'D';
            3
        }
        _ => 0,
    }
}

fn parse_debug_flags(argc: u64, argv: *const *const u8) -> DebugFlags {
    let mut flags = DebugFlags::new();
    let mut raw = [core::ptr::null::<u8>(); 8];
    let count = unsafe { sunlight_libc::crt0::collect_raw_args(argc, argv, &mut raw) };
    for arg in raw[..count].iter().copied() {
        if bytes_eq(arg, b"--debug-pty-stream") {
            flags.log_pty_stream = true;
        }
    }
    flags
}

fn bytes_eq(mut ptr: *const u8, expected: &[u8]) -> bool {
    if ptr.is_null() {
        return false;
    }
    for &byte in expected {
        let actual = unsafe { *ptr };
        if actual != byte {
            return false;
        }
        ptr = unsafe { ptr.add(1) };
    }
    unsafe { *ptr == 0 }
}

// Opt-in diagnostics: serial writes must not be part of ordinary input handling.
const INPUT_DEBUG: bool = false;

fn log_term_key(ch: char, ctrl: bool, alt: bool) {
    if !INPUT_DEBUG {
        return;
    }
    // Rate-limited enough for diagnosis without flooding serial on hold-repeat.
    static mut COUNT: u32 = 0;
    let n = unsafe {
        COUNT = COUNT.wrapping_add(1);
        COUNT
    };
    if n > 32 && n % 16 != 0 {
        return;
    }
    let mut buf = [0u8; 64];
    let mut len = 0usize;
    len += copy_ascii(b"[TERM] key ch=", &mut buf[len..]);
    if ch >= ' ' && ch <= '~' {
        if len < buf.len() {
            buf[len] = ch as u8;
            len += 1;
        }
    } else if ch == '\n' {
        len += copy_ascii(b"\\n", &mut buf[len..]);
    } else if ch == '\u{8}' {
        len += copy_ascii(b"\\b", &mut buf[len..]);
    } else {
        len += copy_ascii(b"?", &mut buf[len..]);
    }
    if ctrl {
        len += copy_ascii(b" ctrl", &mut buf[len..]);
    }
    if alt {
        len += copy_ascii(b" alt", &mut buf[len..]);
    }
    len += copy_ascii(b"\n", &mut buf[len..]);
    if let Ok(text) = core::str::from_utf8(&buf[..len]) {
        debug_log(text);
    }
}

fn log_term_footer_len(input_len: usize) {
    if !INPUT_DEBUG {
        return;
    }
    let mut buf = [0u8; 48];
    let mut len = 0usize;
    len += copy_ascii(b"[TERM] footer_len=", &mut buf[len..]);
    len += fmt_u64(&mut buf[len..], input_len as u64);
    len += copy_ascii(b"\n", &mut buf[len..]);
    if let Ok(text) = core::str::from_utf8(&buf[..len]) {
        debug_log(text);
    }
}

fn log_pty_bytes(bytes: &[u8]) {
    const LOG_LIMIT: usize = 96;
    let mut buf = [0u8; 320];
    let mut len = 0usize;
    len += copy_ascii(b"[TERM][PTY] ", &mut buf[len..]);
    for &byte in bytes.iter().take(LOG_LIMIT) {
        len += escape_byte(byte, &mut buf[len..]);
        if len >= buf.len().saturating_sub(5) {
            break;
        }
    }
    if bytes.len() > LOG_LIMIT {
        len += copy_ascii(b"...", &mut buf[len..]);
    }
    if len < buf.len() {
        buf[len] = b'\n';
        len += 1;
    }
    if let Ok(text) = core::str::from_utf8(&buf[..len]) {
        debug_log(text);
    }
}

fn escape_byte(byte: u8, dst: &mut [u8]) -> usize {
    match byte {
        b'\n' => copy_ascii(b"\\n", dst),
        b'\r' => copy_ascii(b"\\r", dst),
        b'\t' => copy_ascii(b"\\t", dst),
        0x1B => copy_ascii(b"\\x1b", dst),
        0x20..=0x7E => {
            if !dst.is_empty() {
                dst[0] = byte;
                1
            } else {
                0
            }
        }
        _ => {
            if dst.len() < 4 {
                return 0;
            }
            dst[0] = b'\\';
            dst[1] = b'x';
            dst[2] = hex_digit(byte >> 4);
            dst[3] = hex_digit(byte & 0x0F);
            4
        }
    }
}

const fn hex_digit(nibble: u8) -> u8 {
    match nibble & 0x0F {
        0..=9 => b'0' + (nibble & 0x0F),
        _ => b'a' + ((nibble & 0x0F) - 10),
    }
}

fn spawn_shell(pty: &PtySession, slave: CapabilityToken, shell_id: u64) -> Result<u64, ()> {
    let mut path_buf = [0u8; 32];
    let mut arg0 = [0u8; 16];
    let mut arg_session = [0u8; 48];
    let mut arg_generation = [0u8; 48];
    let mut arg_service = [0u8; 48];
    let mut arg_slave = [0u8; 48];

    let mut path_len = copy_ascii(b"/bin/sshl", &mut path_buf);
    path_len += fmt_u64(&mut path_buf[path_len..], shell_id);

    let mut a0_len = copy_ascii(b"sshl", &mut arg0);
    a0_len += fmt_u64(&mut arg0[a0_len..], shell_id);

    let mut aps_len = copy_ascii(b"--pty-session=", &mut arg_session);
    aps_len += fmt_u64(&mut arg_session[aps_len..], pty.id);

    let mut apg_len = copy_ascii(b"--pty-generation=", &mut arg_generation);
    apg_len += fmt_u64(&mut arg_generation[apg_len..], pty.generation);

    let mut apc_len = copy_ascii(b"--pty-service-cap=", &mut arg_service);
    apc_len += fmt_u64(&mut arg_service[apc_len..], pty.service_cap.0);

    let mut apsl_len = copy_ascii(b"--pty-slave-cap=", &mut arg_slave);
    apsl_len += fmt_u64(&mut arg_slave[apsl_len..], slave.0);

    let argv = [
        &arg0[..a0_len],
        &arg_session[..aps_len],
        &arg_generation[..apg_len],
        &arg_service[..apc_len],
        &arg_slave[..apsl_len],
    ];
    libc::spawn(&path_buf[..path_len], &argv, None).map_err(|_| ())
}

fn copy_ascii(src: &[u8], dst: &mut [u8]) -> usize {
    let len = src.len().min(dst.len());
    dst[..len].copy_from_slice(&src[..len]);
    len
}

fn fmt_u64(buf: &mut [u8], mut value: u64) -> usize {
    if value == 0 {
        if !buf.is_empty() {
            buf[0] = b'0';
        }
        return 1;
    }
    let mut tmp = [0u8; 20];
    let mut digits = 0;
    while value > 0 {
        tmp[digits] = b'0' + (value % 10) as u8;
        value /= 10;
        digits += 1;
    }
    for idx in 0..digits.min(buf.len()) {
        buf[idx] = tmp[digits - idx - 1];
    }
    digits.min(buf.len())
}
