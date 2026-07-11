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
//! - Resize correctness (rows/cols are still fixed).
//! - OSC window-title escape support — tab titles are `Tab N` today.
//! - Drag-to-reorder tabs, split panes, session restore.
//! - PTY reads/writes use short bounded IPC calls so a delayed PTY server
//!   cannot freeze the window's input and redraw loop.

use sun_font::{self, FontRole, VecFont};
use sunlight_ipc::{
    debug_log, ipc_call_timeout,
    launch_trace::{self, LaunchSource, LaunchTrace},
    monotonic_millis, nameserver_lookup, process_yield, CapabilityToken, IpcCallError, IpcMsg,
    ProcessExit, PtyMsg,
};
use sunlight_libc as libc;
use sunlight_tty::TerminalGrid as ModelGrid;
use sunlight_ui::{
    widgets::{Label, StatusBar},
    App, Canvas, Event, HBox, Point, Rect, VecText, Window, WindowConfig,
};

static F_UI: VecFont = VecFont(FontRole::UiRegular);
static F_SMALL: VecFont = VecFont(FontRole::UiSmall);

const WIN_W: u32 = 656;
const WIN_H: u32 = 468;
const TAB_H: u32 = 28;
const FOOTER_H: u32 = 32;
const PAD_X: u32 = 8;
const PAD_Y: u32 = 4;
const CELL_W: u32 = 8;
const CELL_H: u32 = 16;
const CONTENT_COLS: usize = ((WIN_W - PAD_X * 2) / CELL_W) as usize;
const CONTENT_ROWS: usize = ((WIN_H - TAB_H - FOOTER_H - PAD_Y * 2) / CELL_H) as usize;

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
const READ_BUF: usize = 256;
const ANSI_COLORS: [u32; 16] = [
    0xFF000000, 0xFFCC241D, 0xFF98971A, 0xFFD79921, 0xFF458588, 0xFFB16286, 0xFF689D6A, 0xFFA89984,
    0xFF928374, 0xFFFB4934, 0xFFB8BB26, 0xFFFABD2F, 0xFF83A598, 0xFFD3869B, 0xFF8EC07C, 0xFFEBDBB2,
];

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
const PTY_IO_TIMEOUT_MS: u64 = 20;

struct BumpAllocator;
unsafe impl core::alloc::GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        static mut HEAP: [u8; 3 * 1024 * 1024] = [0; 3 * 1024 * 1024];
        static mut NEXT: usize = 0;
        let aligned = (NEXT + layout.align() - 1) & !(layout.align() - 1);
        let end = aligned + layout.size();
        if end > HEAP.len() {
            return core::ptr::null_mut();
        }
        NEXT = end;
        HEAP.as_mut_ptr().add(aligned)
    }
    unsafe fn dealloc(&self, _: *mut u8, _: core::alloc::Layout) {}
}
#[cfg(not(test))]
#[global_allocator]
static ALLOC: BumpAllocator = BumpAllocator;

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
            _ => PtyIoError::Rejected,
        }
    }
}

struct PtySession {
    id: u64,
    cap: CapabilityToken,
}

impl PtySession {
    /// Request a new session against an already-resolved `pty` service
    /// capability, bounded by `timeout_ms`. Every tab shares the same
    /// capability token (it is just the service endpoint); sessions are
    /// distinguished server-side by `id`.
    ///
    /// Uses [`ipc_call_timeout`] rather than the unbounded `ipc_call` — see
    /// the module-level doc comment for why an unbounded call here is what
    /// causes the whole-terminal hang this file fixes.
    fn create_timeout(cap: CapabilityToken, timeout_ms: u64) -> Result<Self, PtyIoError> {
        let reply = ipc_call_timeout(cap, IpcMsg::with_label(PtyMsg::CREATE), timeout_ms)?;
        if reply.label != PtyMsg::REPLY || reply.cap_count < 2 {
            return Err(PtyIoError::Rejected);
        }
        Ok(Self {
            id: reply.words[0],
            cap,
        })
    }

    fn set_mode_timeout(&self, mode_flags: u64, timeout_ms: u64) -> Result<(), PtyIoError> {
        let reply = ipc_call_timeout(
            self.cap,
            IpcMsg::with_label(PtyMsg::SET_MODE)
                .word(0, self.id)
                .word(1, mode_flags),
            timeout_ms,
        )?;
        if reply.label != PtyMsg::REPLY {
            return Err(PtyIoError::Rejected);
        }
        Ok(())
    }

    fn write(&self, bytes: &[u8]) {
        let mut pos = 0;
        while pos < bytes.len() {
            let chunk = (bytes.len() - pos).min(16);
            let mut msg = IpcMsg::with_label(PtyMsg::WRITE_MASTER)
                .word(0, self.id)
                .word(1, chunk as u64);
            for (wi, cb) in bytes[pos..pos + chunk].chunks(8).enumerate() {
                let mut word = 0u64;
                for (bi, &b) in cb.iter().enumerate() {
                    word |= (b as u64) << (bi * 8);
                }
                msg = msg.word(2 + wi, word);
            }
            let reply = match ipc_call_timeout(self.cap, msg, PTY_IO_TIMEOUT_MS) {
                Ok(reply) => reply,
                Err(_) => break,
            };
            if reply.label != PtyMsg::REPLY {
                break;
            }
            let accepted = (reply.words[1] as usize).min(chunk);
            if accepted == 0 {
                break;
            }
            pos += accepted;
        }
    }

    fn read(&self, out: &mut [u8]) -> usize {
        let mut total = 0;
        while total < out.len() {
            let chunk = (out.len() - total).min(16);
            let reply = match ipc_call_timeout(
                self.cap,
                IpcMsg::with_label(PtyMsg::READ_MASTER)
                    .word(0, self.id)
                    .word(1, chunk as u64),
                PTY_IO_TIMEOUT_MS,
            ) {
                Ok(reply) => reply,
                Err(_) => break,
            };
            if reply.label != PtyMsg::REPLY {
                break;
            }
            let n = (reply.words[1] as usize).min(chunk);
            if n == 0 {
                break;
            }
            for i in 0..n {
                out[total + i] = ((reply.words[2 + (i / 8)] >> ((i % 8) * 8)) & 0xFF) as u8;
            }
            total += n;
            if n < chunk {
                break;
            }
        }
        total
    }

    /// Release this session back to `pty_server` (frees the server-side
    /// slot for reuse). Best-effort and idempotent; bounded by
    /// [`CLOSE_TIMEOUT_MS`] so a stuck server can't wedge tab close/teardown
    /// either.
    fn close(&self) {
        let _ = ipc_call_timeout(
            self.cap,
            IpcMsg::with_label(PtyMsg::CLOSE).word(0, self.id),
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

struct TerminalViewport {
    rect: Rect,
}

impl TerminalViewport {
    const fn new(rect: Rect) -> Self {
        Self { rect }
    }

    fn draw(&self, canvas: &mut Canvas, grid: &mut ModelGrid, theme: &sunlight_ui::Theme) {
        canvas.fill_rect(self.rect, theme.panel);
        canvas.draw_rect(self.rect, theme.border);

        let cols = grid.cols;
        let rows = grid.rows;
        let cells = grid.to_term_cells(&ANSI_COLORS);
        let mut clipped = canvas.sub_canvas(self.rect.inset(1));
        for row in 0..rows {
            for col in 0..cols {
                let idx = row * cols + col;
                if idx >= cells.len() {
                    break;
                }
                let cell = cells[idx];
                let x = col as i32 * CELL_W as i32;
                let y = row as i32 * CELL_H as i32;
                clipped.fill_rect(Rect::new(x, y, CELL_W, CELL_H), sunlight_ui::Color(cell.bg));
                if cell.ch >= b' ' && cell.ch <= b'~' && cell.ch != b' ' {
                    clipped.draw_char(x, y, cell.ch as char, sunlight_ui::Color(cell.fg));
                }
            }
        }
        if grid.cursor_visible() {
            let (cursor_row, cursor_col) = grid.cursor();
            if cursor_row < rows && cursor_col < cols {
                clipped.draw_rect(
                    Rect::new(
                        cursor_col as i32 * CELL_W as i32,
                        cursor_row as i32 * CELL_H as i32,
                        CELL_W,
                        CELL_H,
                    ),
                    theme.accent,
                );
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
}

impl TerminalTab {
    /// A brand-new placeholder tab with no PTY/shell yet — the immediate,
    /// zero-IPC result of clicking "+"/pressing `Ctrl+T` (see
    /// [`TerminalApp::spawn_tab`]).
    fn connecting(id: TabId, title: &[u8]) -> Self {
        let mut tab = Self {
            id,
            title: [0; TAB_TITLE_MAX],
            title_len: 0,
            pty: None,
            shell_pid: None,
            grid: ModelGrid::new(CONTENT_COLS, CONTENT_ROWS),
            footer: Footer::new(),
            osc: OscParser::new(),
            status: TabStatus::Connecting,
            dirty: false,
            first_frame_logged: false,
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
        let Some(pty) = self.pty.as_ref() else {
            return false;
        };
        let n = pty.read(read_buf);
        if n == 0 {
            return false;
        }
        self.ingest(&read_buf[..n], console_buf, debug);
        true
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
        let Some(pty) = self.pty.as_ref() else {
            return false;
        };
        if self.app_owns_input() {
            let mut buf = [0u8; 4];
            let n = translate_special_key(keycode, &mut buf);
            if n > 0 {
                pty.write(&buf[..n]);
                return true;
            }
            return false;
        }
        match keycode {
            KEY_BACKSPACE => self.footer.backspace(),
            KEY_UP => self.footer.history_up(),
            KEY_DOWN => self.footer.history_down(),
            KEY_LEFT => self.footer.move_left(),
            KEY_RIGHT => self.footer.move_right(),
            KEY_HOME => self.footer.home(),
            KEY_END => self.footer.end(),
            KEY_DEL => self.footer.delete_fwd(),
            _ => return false,
        }
        true
    }

    fn handle_char(&mut self, ch: char) -> bool {
        if self.pty.is_none() {
            return false;
        }
        if self.app_owns_input() {
            let byte = match ch {
                '\t' => b'\t',
                '\n' => b'\n',
                '\u{8}' => 0x08,
                c if c.is_ascii() => c as u8,
                _ => 0,
            };
            if byte != 0 {
                self.pty.as_ref().unwrap().write(&[byte]);
                return true;
            }
            return false;
        }
        if ch == '\n' {
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
    console_buf: [u8; READ_BUF],
    debug: DebugFlags,
    mods: Mods,
    pending_spawn: Option<PendingSpawn>,
}

impl TerminalApp {
    const TAB_W: u32 = 92;
    /// Wider than a single glyph so the new-tab button has a comfortable
    /// click target.
    const NEW_TAB_W: u32 = 34;
    const CLOSE_BTN_SIZE: u32 = 14;

    fn new(pty_cap: CapabilityToken, debug: DebugFlags) -> Self {
        Self {
            tabs: core::array::from_fn(|_| None),
            tab_count: 0,
            active: 0,
            poll_cursor: 0,
            next_tab_id: 1,
            pty_cap,
            read_buf: [0; READ_BUF],
            console_buf: [0; READ_BUF],
            debug,
            mods: Mods::default(),
            pending_spawn: None,
        }
    }

    fn active_tab_mut(&mut self) -> Option<&mut TerminalTab> {
        self.tabs.get_mut(self.active).and_then(|t| t.as_mut())
    }

    fn clear_tracked_mods(&mut self) {
        self.mods.clear();
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

        if !self.insert_tab(TerminalTab::connecting(id, &title[..len])) {
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
            started_ms: monotonic_millis(),
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

        if monotonic_millis().saturating_sub(pending.started_ms) > SPAWN_DEADLINE_MS {
            self.mark_pending_failed(pending.tab_id, pending.step);
            return true;
        }

        let outcome: Result<Option<SpawnStep>, ()> = match pending.step {
            SpawnStep::RequestPty => {
                match PtySession::create_timeout(self.pty_cap, SPAWN_STEP_TIMEOUT_MS) {
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
                let shell_id = (pty.id as u8).max(1) as u64;
                match spawn_shell(&pty, shell_id) {
                    Ok(shell_pid) => {
                        log_tab_phase(pending.tab_id, "shell_spawned");
                        self.attach_tab(pending.tab_id, pty, shell_pid);
                        Ok(None)
                    }
                    Err(_) => {
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
            SpawnStep::SetMode(pty) | SpawnStep::SpawnShell(pty) => pty.close(),
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
                    SpawnStep::SetMode(pty) | SpawnStep::SpawnShell(pty) => pty.close(),
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

        if let Some(tab) = self.tabs.get_mut(self.active).and_then(|t| t.as_mut()) {
            tab.refresh_status();
            if tab.poll_pty(&mut self.read_buf, &mut self.console_buf, self.debug) {
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
                    tab.refresh_status();
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
                SpawnStep::SetMode(pty) | SpawnStep::SpawnShell(pty) => pty.close(),
                SpawnStep::RequestPty => {}
            }
        }
    }

    fn tab_rect(index: usize) -> Rect {
        Rect::new((index as u32 * Self::TAB_W) as i32, 0, Self::TAB_W, TAB_H)
    }

    fn new_tab_rect(tab_count: usize) -> Rect {
        Rect::new(
            (tab_count as u32 * Self::TAB_W) as i32,
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
                let r = Self::tab_rect(i);
                if Self::close_btn_rect(r).contains(point) {
                    self.close_tab(i);
                    return true;
                }
            }
        }
        for i in 0..self.tab_count {
            if Self::tab_rect(i).contains(point) {
                return self.switch_tab(i);
            }
        }
        if self.tab_count < MAX_TABS && Self::new_tab_rect(self.tab_count).contains(point) {
            return self.spawn_tab();
        }
        false
    }

    fn draw_tab_bar(&self, canvas: &mut Canvas, theme: &sunlight_ui::Theme) {
        canvas.fill_rect(Rect::new(0, 0, WIN_W, TAB_H), theme.panel);
        canvas.hbar(0, TAB_H as i32 - 1, WIN_W, 1, theme.border);

        for i in 0..self.tab_count {
            let Some(tab) = self.tabs[i].as_ref() else {
                continue;
            };
            let r = Self::tab_rect(i);
            let active = i == self.active;

            canvas.fill_rect(r, if active { theme.panel_alt } else { theme.panel });

            let text_color = match tab.status {
                TabStatus::Failed | TabStatus::Exited => theme.danger,
                TabStatus::Connecting => theme.warn,
                TabStatus::Running if active => theme.accent,
                TabStatus::Running => theme.text_dim,
            };
            F_SMALL.draw_vcenter(canvas, tab.title_str(), r.x + 8, r.y, TAB_H, text_color);

            if active {
                canvas.hbar(r.x, r.bottom() - 2, r.w, 2, theme.accent);
            } else if tab.dirty {
                canvas.fill_rect(Rect::new(r.right() - 10, r.y + 5, 4, 4), theme.accent);
            }

            if self.tab_count > 1 {
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
                    theme.border,
                );
            }
        }

        if self.tab_count < MAX_TABS {
            let nr = Self::new_tab_rect(self.tab_count);
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
            "new tab pending"
        } else if tab.pty.is_some() {
            "session attached"
        } else {
            "no session"
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
    use super::*;

    fn test_pty(id: u64) -> PtySession {
        PtySession {
            id,
            cap: CapabilityToken::INVALID,
        }
    }

    /// A tab already `Running` with a fake PTY/shell — used by tests that
    /// only exercise array bookkeeping / byte routing, not the connecting
    /// state machine.
    fn test_tab(id: TabId, title: &[u8]) -> TerminalTab {
        let mut tab = TerminalTab::connecting(id, title);
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
}

impl App for TerminalApp {
    fn view(&mut self, canvas: &mut Canvas, theme: &sunlight_ui::Theme) {
        canvas.fill_rect(Rect::new(0, 0, WIN_W, WIN_H), theme.bg);
        self.draw_tab_bar(canvas, theme);

        let content = content_rect();
        let footer = footer_rect();

        let Some(tab) = self.tabs[self.active].as_mut() else {
            StatusBar::new(footer, "", "no active tab", "").draw(canvas, theme);
            return;
        };

        if !tab.first_frame_logged {
            tab.first_frame_logged = true;
            log_tab_phase(tab.id, "first_tab_frame");
        }

        TerminalViewport::new(content).draw(canvas, &mut tab.grid, theme);

        let status_label = match tab.status {
            TabStatus::Connecting => Some("Connecting..."),
            TabStatus::Failed => Some("Failed to start shell"),
            TabStatus::Running | TabStatus::Exited => None,
        };
        if let Some(text) = status_label {
            Label::new(
                Rect::new(content.x + 8, content.y + 8, content.w - 16, 20),
                text,
            )
            .with_font(&F_SMALL)
            .draw(canvas, theme);
        }

        let pending_spawn_for_tab = self.pending_spawn.as_ref().map(|p| p.tab_id) == Some(tab.id);
        StatusBar::new(
            footer,
            "",
            Self::footer_center_text(tab),
            Self::footer_right_text(tab, pending_spawn_for_tab),
        )
        .draw(canvas, theme);
        if tab.app_owns_input() {
            Label::new(
                Rect::new(8, footer.y + 4, 220, FOOTER_H - 8),
                tab.footer.app_name_str(),
            )
            .with_font(&F_SMALL)
            .draw(canvas, theme);
        } else {
            let prompt_w = sun_font::measure_text(tab.footer.prompt_str(), FontRole::UiSmall).w + 4;
            let prompt_area = Rect::new(8, footer.y + 4, WIN_W - 16, FOOTER_H - 8);
            let spacing = 8;
            let input_w = prompt_area
                .w
                .saturating_sub(prompt_w)
                .saturating_sub(spacing);
            let prompt_widths = [prompt_w, input_w];
            let mut prompt_cells = HBox::new(prompt_area)
                .with_spacing(spacing)
                .layout(&prompt_widths);
            if let Some(prompt_rect) = prompt_cells.next() {
                Label::new(prompt_rect, tab.footer.prompt_str())
                    .with_font(&F_SMALL)
                    .draw(canvas, theme);
            }
            if let Some(input_rect) = prompt_cells.next() {
                Label::new(input_rect, tab.footer.input_str())
                    .with_font(&F_UI)
                    .draw(canvas, theme);
                if !tab.app_owns_input() {
                    let prefix_w = sun_font::measure_text(
                        tab.footer.input_prefix_str(),
                        FontRole::UiRegular,
                    )
                    .w as i32;
                    let caret_x = (input_rect.x + prefix_w).min(input_rect.right() - 1);
                    canvas.vline(
                        caret_x,
                        input_rect.y + 2,
                        input_rect.h.saturating_sub(4),
                        theme.accent,
                    );
                    if tab.footer.input_cursor < tab.footer.input_len {
                        let suffix = tab.footer.input_suffix_str();
                        if let Some(ch) = suffix.chars().next() {
                            let mut buf = [0u8; 4];
                            let text = ch.encode_utf8(&mut buf);
                            let char_w = sun_font::measure_text(text, FontRole::UiRegular)
                                .w
                                .min(input_rect.w);
                            canvas.fill_rect(
                                Rect::new(
                                    caret_x,
                                    input_rect.y + 1,
                                    char_w,
                                    input_rect.h.saturating_sub(2),
                                ),
                                theme.accent,
                            );
                            canvas.draw_char(caret_x, input_rect.y, ch, theme.bg);
                        }
                    }
                }
            }
        }
    }

    fn update(&mut self, event: Event) -> bool {
        let mut dirty = false;
        match event {
            Event::Tick => {
                dirty |= self.poll_all_tabs();
            }
            Event::KeyPress {
                keycode,
                pressed,
                shift,
                ctrl,
                alt,
                ..
            } => {
                self.mods = Mods { ctrl, alt };
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
                if self.mods.ctrl && (ch == 't' || ch == 'T') {
                    dirty |= self.spawn_tab();
                } else if self.mods.ctrl && (ch == 'w' || ch == 'W') {
                    // Plain Ctrl+W never reaches here -- `sunlight-display`
                    // still intercepts it globally to close the window. Only
                    // Ctrl+Shift+W is left unconsumed for apps.
                    dirty |= self.close_tab(self.active);
                } else if self.mods.alt && !self.mods.ctrl && ch.is_ascii_digit() && ch != '0' {
                    let idx = (ch as u8 - b'1') as usize;
                    dirty |= self.switch_tab(idx);
                } else if let Some(tab) = self.active_tab_mut() {
                    dirty |= tab.handle_char(ch);
                }
            }
            Event::Click { x, y } => {
                self.clear_tracked_mods();
                dirty |= self.handle_click(x, y);
            }
            Event::MouseDown { .. } => {
                self.clear_tracked_mods();
            }
            Event::MouseUp { .. } | Event::MouseMove { .. } => {}
        }
        dirty
    }
}

fn content_rect() -> Rect {
    Rect::new(
        PAD_X as i32,
        TAB_H as i32 + PAD_Y as i32,
        WIN_W - PAD_X * 2,
        WIN_H - TAB_H - FOOTER_H - PAD_Y * 2,
    )
}

fn footer_rect() -> Rect {
    Rect::new(0, WIN_H as i32 - FOOTER_H as i32, WIN_W, FOOTER_H)
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
    let mut buf = [0u8; 96];
    let mut len = 0usize;
    len += copy_ascii(b"[TERM][TAB] tab=", &mut buf[len..]);
    len += fmt_u64(&mut buf[len..], tab_id as u64);
    len += copy_ascii(b" phase=", &mut buf[len..]);
    len += copy_ascii(phase.as_bytes(), &mut buf[len..]);
    len += copy_ascii(b" t=", &mut buf[len..]);
    len += fmt_u64(&mut buf[len..], monotonic_millis());
    len += copy_ascii(b"ms\n", &mut buf[len..]);
    if let Ok(text) = core::str::from_utf8(&buf[..len]) {
        debug_log(text);
    }
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
            buf[0] = b'\n';
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

fn spawn_shell(pty: &PtySession, shell_id: u64) -> Result<u64, ()> {
    let mut path_buf = [0u8; 32];
    let mut arg0 = [0u8; 16];
    let mut arg_session = [0u8; 48];
    let mut arg_cap = [0u8; 48];

    let mut path_len = copy_ascii(b"/bin/sshl", &mut path_buf);
    path_len += fmt_u64(&mut path_buf[path_len..], shell_id);

    let mut a0_len = copy_ascii(b"sshl", &mut arg0);
    a0_len += fmt_u64(&mut arg0[a0_len..], shell_id);

    let mut aps_len = copy_ascii(b"--pty-session=", &mut arg_session);
    aps_len += fmt_u64(&mut arg_session[aps_len..], pty.id);

    let mut apc_len = copy_ascii(b"--pty-cap=", &mut arg_cap);
    apc_len += fmt_u64(&mut arg_cap[apc_len..], pty.cap.0);

    let argv = [
        &arg0[..a0_len],
        &arg_session[..aps_len],
        &arg_cap[..apc_len],
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
