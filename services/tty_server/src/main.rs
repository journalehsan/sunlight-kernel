#![no_std]
#![no_main]

extern crate alloc;

struct BumpAllocator;

unsafe impl core::alloc::GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        static mut HEAP: [u8; 4 * 1024 * 1024] = [0; 4 * 1024 * 1024];
        static mut NEXT: usize = 0;
        let start = NEXT;
        let align = layout.align();
        let aligned = (start + align - 1) & !(align - 1);
        let end = aligned + layout.size();
        if end > HEAP.len() {
            debug_log("[ALLOC] HEAP EXHAUSTED! Requested allocation would overflow.");
            return core::ptr::null_mut();
        }
        NEXT = end;
        HEAP.as_mut_ptr().add(aligned)
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: core::alloc::Layout) {
        // NOTE: Bump allocator cannot free memory. The real fix is in render_active_shell_fb()
        // which reuses TerminalGrid instead of allocating a new one every frame.
        // See GRID_REUSE logic below.
    }
}

#[global_allocator]
static BUMP: BumpAllocator = BumpAllocator;

use alloc::boxed::Box;
use sunlight_ipc::{
    debug_log, endpoint_create, get_time_utc, ipc_call, ipc_recv, ipc_reply_and_try_recv,
    nameserver_lookup, process_is_alive, process_yield, sysinfo, tty_stdin_push, tty_stdout_pull,
    unpack_key_event, CapabilityToken, IpcMsg, KbdMsg, SpawnMsg, TzMsg,
};
use sunlight_tty::login::{LoginField, LoginResult, LoginScreen};
use sunlight_tty::TerminalGrid;
use sunlight_tui::ANSI_COLORS;

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

enum TtyState {
    Login,
    Shell,
}

const KBD_LABEL: u64 = 1;
const OUTPUT_LABEL: u64 = 2;
const EXIT_LABEL: u64 = 3;
const DRAIN_LABEL: u64 = 4;
/// Shell→tty reply: an external command was launched as a foreground job;
/// word0 = child pid. tty_server then drives the session (routes keyboard to
/// the child's stdin ring, renders its stdout ring) until the child exits.
const FG_STARTED_LABEL: u64 = 5;
/// tty→shell request: the foreground child has exited; the shell reaps it and
/// redraws the prompt.
const FG_DONE_LABEL: u64 = 6;
const TERM_OUTPUT_MAX: usize = 4096;
const IPC_OUTPUT_BYTES: usize = 16;
const INPUT_LINE_MAX: usize = 256;
const PENDING_INPUT_MAX: usize = 128;
const MAX_TABS: usize = 10;

/// Per-tab scrollback viewport state
#[derive(Clone, Copy)]
struct TabScrollback {
    viewport_offset: usize,
}

impl TabScrollback {
    const fn new() -> Self {
        Self { viewport_offset: 0 }
    }
}

/// Terminal geometry: current dimensions and viewport state
#[derive(Clone, Copy, Debug)]
pub struct TerminalGeometry {
    pub cols: u32,
    pub rows: u32,
    pub viewport_offset: usize,
    pub max_scrollback: usize,
}

impl TerminalGeometry {
    const fn new() -> Self {
        Self {
            cols: 80,
            rows: 24,
            viewport_offset: 0,
            max_scrollback: 256,
        }
    }

    fn update(&mut self, cols: u32, rows: u32, viewport_offset: usize) {
        self.cols = cols;
        self.rows = rows;
        self.viewport_offset = viewport_offset;
    }

    fn set_viewport(&mut self, offset: usize) {
        self.viewport_offset = offset;
    }
}

/// Global terminal geometry state (per tab)
static mut TERMINAL_GEOMETRY: [TerminalGeometry; MAX_TABS] = [TerminalGeometry {
    cols: 80,
    rows: 24,
    viewport_offset: 0,
    max_scrollback: 256,
}; MAX_TABS];

#[derive(Clone, Copy)]
struct ShellTab {
    shell_id: u64,
    pid: u64,
    cap: Option<CapabilityToken>,
    output: [u8; TERM_OUTPUT_MAX],
    output_len: usize,
    input_line: [u8; INPUT_LINE_MAX],
    input_line_len: usize,
    pending: [u8; PENDING_INPUT_MAX],
    pending_len: usize,
    username: [u8; 32],
    username_len: usize,
    /// pid of a foreground command running in this tab, if any. While set,
    /// keyboard goes to this tab's stdin ring and its stdout ring is rendered
    /// live, instead of going through the shell line editor.
    fg_pid: Option<u64>,
    /// Basename of the running foreground app (e.g. "top"), shown in the tab
    /// title. Empty (`fg_app_name_len == 0`) means the tab shows "SHELL".
    fg_app_name: [u8; 24],
    fg_app_name_len: usize,
}

/// Global scrollback state for all tabs (indexed by active_tab)
static mut SCROLLBACK_STATE: [TabScrollback; MAX_TABS] =
    [TabScrollback { viewport_offset: 0 }; MAX_TABS];

/// FIX: Cached TerminalGrid to avoid repeated 400KB+ allocations per frame
/// This single grid is reused across all renders, preventing heap exhaustion
/// See ROOT_CAUSE_FOUND.md for detailed explanation
static mut GRID_CACHE: Option<Box<TerminalGrid>> = None;

impl ShellTab {
    const fn empty() -> Self {
        Self {
            shell_id: 0,
            pid: 0,
            cap: None,
            output: [0; TERM_OUTPUT_MAX],
            output_len: 0,
            input_line: [0; INPUT_LINE_MAX],
            input_line_len: 0,
            pending: [0; PENDING_INPUT_MAX],
            pending_len: 0,
            username: [0; 32],
            username_len: 0,
            fg_pid: None,
            fg_app_name: [0; 24],
            fg_app_name_len: 0,
        }
    }
}

#[no_mangle]
pub extern "C" fn _start(fb_addr: u64, fb_width: u64, fb_height: u64, fb_pitch: u64) -> ! {
    debug_log("[TTY]  TTY server started");

    let has_fb = fb_addr != 0 && fb_width != 0 && fb_height != 0 && fb_pitch != 0;
    let fb32_w = fb_width as u32;
    let fb32_h = fb_height as u32;
    let fb32_p = fb_pitch as u32;

    if has_fb {
        debug_log("[TTY] Framebuffer acquired");
        unsafe {
            sunlight_tui::render_login_screen(fb_addr as *mut u32, fb32_w, fb32_h, fb32_p);
        }
        debug_log("[TTY] Login rendered");
    }

    let ep = endpoint_create();
    debug_log("[TTY]  endpoint created");

    debug_log("[TTY]  Registered as 'tty'");
    debug_log("[TTY]  Login screen ready");

    let mut login = LoginScreen::new();
    prefill_root_login(&mut login);

    let mut state = TtyState::Login;
    let mut spawn_cap: Option<CapabilityToken> = None;
    let mut tabs = [ShellTab::empty(); MAX_TABS];
    let mut tab_count = 0usize;
    let mut active_tab = 0usize;
    let mut next_shell_id = 0u64;
    let mut logged_initial_spawn = false;

    let mut msg = ipc_recv(ep);
    let mut phase3_6_done = false;
    loop {
        match state {
            TtyState::Login => {
                let mut logged_in = false;
                if let Some(ascii) = key_ascii_from_msg(&msg) {
                    if login.focused == LoginField::Password {
                        debug_log_kbd_byte("[TTY] Key received in password field: ", ascii);
                    }
                    let result = login.handle_key_ascii(ascii);
                    match result {
                        LoginResult::Success {
                            username,
                            username_len,
                            uid,
                            gid,
                        } => {
                            debug_log_login_success(&username[..username_len], uid, gid);
                            debug_log("[SunlightOS] Phase 3.7 OK");

                            // Spawn sunshell
                            let cap = match nameserver_lookup("spawn") {
                                Some(c) => c,
                                None => {
                                    debug_log("[TTY]  spawn capability not found");
                                    state = TtyState::Shell;
                                    continue;
                                }
                            };
                            spawn_cap = Some(cap);

                            if spawn_tab(
                                &mut tabs,
                                &mut tab_count,
                                &mut active_tab,
                                &mut next_shell_id,
                                cap,
                                uid,
                                gid,
                            ) {
                                // Store username in the active tab for prompt rendering
                                if let Some(tab) = active_shell_tab_mut(&mut tabs, active_tab) {
                                    let len = username_len.min(tab.username.len() - 1);
                                    tab.username[..len].copy_from_slice(&username[..len]);
                                    tab.username_len = len;
                                }
                                if let Some(tab) = active_shell_tab(&tabs, active_tab) {
                                    debug_log_spawn(&username[..username_len], tab.pid);
                                    logged_initial_spawn = true;
                                }
                            }

                            // Don't spin-poll here — process_yield() doesn't context-switch;
                            // only the timer does. Instead, transition to Shell immediately
                            // and resolve the sshl endpoint lazily on the first keyboard event.
                            state = TtyState::Shell;
                            debug_log("[TTY]  Built-in shell ready");
                            if has_fb {
                                render_active_shell_fb(
                                    fb_addr, fb32_w, fb32_h, fb32_p, &tabs, tab_count, active_tab,
                                    true,
                                );
                            }
                            logged_in = true;
                        }
                        LoginResult::Locked => {
                            debug_log("[TTY]  Login locked");
                        }
                        LoginResult::Pending => {}
                    }
                }
                login.tick();
                if has_fb && !logged_in {
                    render_login_fb(&login, fb_addr, fb32_w, fb32_h, fb32_p);
                }
            }
            TtyState::Shell => {
                let mut needs_render = false;
                let prev_output_len = active_shell_tab(&tabs, active_tab)
                    .map(|tab| tab.output_len)
                    .unwrap_or(0);

                // Lazy lookup: try to find sshl once it registers after being spawned.
                if msg.label == KbdMsg::KEY_EVENT {
                    let (keycode, pressed, shift, ctrl, _alt, ctrl_ascii) =
                        unpack_key_event(msg.words[0]);

                    // Scrollback viewport control
                    // - Ctrl+Up/Down: scroll by 1 line
                    // - Shift+PageUp/Down: scroll by full page (rows at a time)
                    let is_ctrl_scroll = pressed && ctrl && (keycode == 0x48 || keycode == 0x50);
                    let is_shift_page = pressed && shift && (keycode == 0x49 || keycode == 0x51);

                    if is_ctrl_scroll || is_shift_page {
                        unsafe {
                            let scrollback = &mut SCROLLBACK_STATE[active_tab];
                            match keycode {
                                0x48 if ctrl => {
                                    // Ctrl+Up: scroll up by 1 line
                                    scrollback.viewport_offset =
                                        (scrollback.viewport_offset + 1).min(256);
                                    needs_render = true;
                                }
                                0x50 if ctrl => {
                                    // Ctrl+Down: scroll down by 1 line
                                    scrollback.viewport_offset =
                                        scrollback.viewport_offset.saturating_sub(1);
                                    needs_render = true;
                                }
                                0x49 if shift => {
                                    // Shift+PageUp: scroll up by multiple lines (full page)
                                    scrollback.viewport_offset =
                                        (scrollback.viewport_offset + 24).min(256);
                                    needs_render = true;
                                }
                                0x51 if shift => {
                                    // Shift+PageDown: scroll down by multiple lines
                                    if scrollback.viewport_offset >= 24 {
                                        scrollback.viewport_offset -= 24;
                                    } else {
                                        scrollback.viewport_offset = 0;
                                    }
                                    needs_render = true;
                                }
                                _ => {}
                            }
                        }
                    } else if pressed && ctrl {
                        if let Some(a) = ctrl_ascii {
                            if handle_ctrl_key(
                                a,
                                &mut tabs,
                                &mut tab_count,
                                &mut active_tab,
                                &mut next_shell_id,
                                spawn_cap,
                                &mut phase3_6_done,
                            ) {
                                needs_render = true;
                            }
                        }
                    }
                }

                resolve_active_shell(&mut tabs, active_tab, &mut logged_initial_spawn);

                // Check if shell was just resolved and has greeting output to render
                let current_output_len = active_shell_tab(&tabs, active_tab)
                    .map(|tab| tab.output_len)
                    .unwrap_or(0);
                if current_output_len > prev_output_len {
                    needs_render = true;
                }

                if let Some(ascii) = key_ascii_from_msg(&msg) {
                    // Reset scrollback on any normal keypress (return to live view)
                    unsafe {
                        SCROLLBACK_STATE[active_tab].viewport_offset = 0;
                    }
                    if let Some(tab) = active_shell_tab_mut(&mut tabs, active_tab) {
                        if tab.fg_pid.is_some() {
                            // A foreground command owns the screen: route the key
                            // to its stdin ring (keyed by shell_id) instead of the
                            // shell line editor. No prompt echo.
                            let _ = tty_stdin_push(tab.shell_id as u32, &[ascii]);
                        } else {
                            update_input_echo(
                                ascii,
                                &mut tab.output,
                                &mut tab.output_len,
                                &mut tab.input_line,
                                &mut tab.input_line_len,
                                tab.username,
                                tab.username_len,
                            );
                            needs_render = true;

                            if let Some(cap) = tab.cap {
                                match send_key_to_shell(
                                    cap,
                                    ascii,
                                    &mut tab.output,
                                    &mut tab.output_len,
                                ) {
                                    ShellKeyResult::Exited => {
                                        state = TtyState::Login;
                                        reset_login(&mut login);
                                        reset_tabs(&mut tabs, &mut tab_count, &mut active_tab);
                                        spawn_cap = None;
                                        logged_initial_spawn = false;
                                        if has_fb {
                                            render_login_fb(
                                                &login, fb_addr, fb32_w, fb32_h, fb32_p,
                                            );
                                        }
                                        continue;
                                    }
                                    ShellKeyResult::ForegroundStarted(pid, name, name_len) => {
                                        // Enter foreground mode. The app's output
                                        // is drained into scrollback and rendered
                                        // by the idle loop; a full-screen app's
                                        // alt-screen enter resets the buffer.
                                        tab.fg_pid = Some(pid);
                                        tab.fg_app_name = name;
                                        tab.fg_app_name_len = name_len;
                                    }
                                    ShellKeyResult::Continue => {}
                                }
                                // Output was received from shell - ensure immediate render
                                needs_render = true;
                            } else if tab.pending_len < tab.pending.len() {
                                tab.pending[tab.pending_len] = ascii;
                                tab.pending_len += 1;
                            }
                        }
                    }
                }

                // Re-render whenever the title-bar clock minute rolls over
                static mut LAST_CLOCK_MIN: u64 = u64::MAX;
                let now_min = get_time_utc() / 60;
                unsafe {
                    if now_min != LAST_CLOCK_MIN {
                        LAST_CLOCK_MIN = now_min;
                        needs_render = true;
                    }
                }

                // Don't run the shell renderer while a foreground app owns the
                // screen — it would clear the grid and replay the shell buffer,
                // wiping the app's frame. The idle loop renders the app instead.
                let active_fg =
                    active_shell_tab(&tabs, active_tab).map_or(false, |t| t.fg_pid.is_some());
                if has_fb && needs_render && !active_fg {
                    render_active_shell_fb(
                        fb_addr, fb32_w, fb32_h, fb32_p, &tabs, tab_count, active_tab, true,
                    );
                }
            }
        }

        // Wait for the next message, but keep the active tab live while idle:
        // drive a foreground command (drain its output, detect exit) and keep
        // the title-bar clock current.
        let reply = IpcMsg::with_label(0);
        loop {
            if let Some(m) = ipc_reply_and_try_recv(ep, reply) {
                msg = m;
                break;
            }
            if has_fb && matches!(state, TtyState::Shell) {
                let fg = active_shell_tab(&tabs, active_tab).and_then(|t| t.fg_pid);
                if let Some(pid) = fg {
                    // Drain the foreground app's output (kernel stdout ring,
                    // keyed by shell_id) into the tab's scrollback. The existing
                    // replay renderer then shows the live screen; full-screen
                    // apps redraw in place (their alt-screen enter resets the
                    // buffer in append_term), streaming output simply scrolls.
                    let mut drained = false;
                    if let Some(tab) = active_shell_tab_mut(&mut tabs, active_tab) {
                        let mut buf = [0u8; 1024];
                        loop {
                            let n = tty_stdout_pull(tab.shell_id as u32, &mut buf);
                            if n == 0 {
                                break;
                            }
                            append_term(&mut tab.output, &mut tab.output_len, &buf[..n]);
                            drained = true;
                        }
                    }

                    if !process_is_alive(pid) {
                        // The app exited. Final drain to catch any output written
                        // just before exit (between the drain above and now).
                        if let Some(tab) = active_shell_tab_mut(&mut tabs, active_tab) {
                            let mut buf = [0u8; 1024];
                            loop {
                                let n = tty_stdout_pull(tab.shell_id as u32, &mut buf);
                                if n == 0 {
                                    break;
                                }
                                append_term(&mut tab.output, &mut tab.output_len, &buf[..n]);
                            }
                        }
                        // Tell the shell to reap it and record $?; leave
                        // foreground mode and redraw with the prompt.
                        let cap = active_shell_tab(&tabs, active_tab).and_then(|t| t.cap);
                        if let Some(cap) = cap {
                            let _ = ipc_call(cap, IpcMsg::with_label(FG_DONE_LABEL));
                        }
                        if let Some(tab) = active_shell_tab_mut(&mut tabs, active_tab) {
                            tab.fg_pid = None;
                            tab.fg_app_name_len = 0;
                        }
                        render_active_shell_fb(
                            fb_addr, fb32_w, fb32_h, fb32_p, &tabs, tab_count, active_tab, true,
                        );
                    } else if drained {
                        // Foreground app owns the screen: render without a prompt.
                        render_active_shell_fb(
                            fb_addr, fb32_w, fb32_h, fb32_p, &tabs, tab_count, active_tab, false,
                        );
                    }
                } else {
                    static mut LAST_POLL_MIN: u64 = u64::MAX;
                    let now_min = get_time_utc() / 60;
                    // SAFETY: tty_server is single-threaded; no concurrent access.
                    unsafe {
                        if now_min != LAST_POLL_MIN {
                            LAST_POLL_MIN = now_min;
                            render_active_shell_fb(
                                fb_addr, fb32_w, fb32_h, fb32_p, &tabs, tab_count, active_tab, true,
                            );
                        }
                    }
                }
            }
            process_yield();
        }
    }
}

/// Get current terminal geometry for the active tab
pub fn get_terminal_geometry(tab_idx: usize) -> Option<TerminalGeometry> {
    if tab_idx < MAX_TABS {
        unsafe { Some(TERMINAL_GEOMETRY[tab_idx]) }
    } else {
        None
    }
}

/// Get terminal dimensions (cols, rows) for the active tab
pub fn get_terminal_dims(tab_idx: usize) -> Option<(u32, u32)> {
    get_terminal_geometry(tab_idx).map(|g| (g.cols, g.rows))
}

/// Get current viewport offset for the active tab
pub fn get_viewport_offset(tab_idx: usize) -> usize {
    if tab_idx < MAX_TABS {
        unsafe { TERMINAL_GEOMETRY[tab_idx].viewport_offset }
    } else {
        0
    }
}

fn render_login_fb(login: &LoginScreen, fb_addr: u64, fb_w: u32, fb_h: u32, fb_p: u32) {
    unsafe {
        sunlight_tui::render_login_dynamic(
            fb_addr as *mut u32,
            fb_w,
            fb_h,
            fb_p,
            &login.username.buf[..login.username.len],
            login.password.len,
            login.focused == LoginField::Password,
            login.message,
        );
    }
}

fn render_active_shell_fb(
    fb_addr: u64,
    fb_w: u32,
    fb_h: u32,
    fb_p: u32,
    tabs: &[ShellTab; MAX_TABS],
    tab_count: usize,
    active_tab: usize,
    show_prompt: bool,
) {
    // Compute terminal dimensions (8x16 glyphs, accounting for chrome)
    // Header: 48px, Tab bar: 26px, Footer: 32px, margins: 16px top/bottom
    let char_w: u32 = 8;
    let char_h: u32 = 16;
    let chrome_h: u32 = 48 + 26 + 32 + 8; // header + tabbar + footer + gaps
    let avail_h = fb_h.saturating_sub(chrome_h);
    let rows = (avail_h / char_h) as usize;
    let cols = (fb_w / char_w) as usize;

    // Update terminal geometry state
    unsafe {
        let viewport_offset = SCROLLBACK_STATE[active_tab].viewport_offset;
        TERMINAL_GEOMETRY[active_tab].update(cols as u32, rows as u32, viewport_offset);
    }

    // A foreground app owns the screen, so suppress the shell prompt/input line.
    let mut prompt_buf = [0u8; 32];
    let (output, input_line, prompt_slice) = active_shell_tab(tabs, active_tab)
        .map(|tab| {
            if show_prompt {
                let prompt_len = build_prompt(tab, &mut prompt_buf);
                (
                    &tab.output[..tab.output_len],
                    &tab.input_line[..tab.input_line_len],
                    &prompt_buf[..prompt_len],
                )
            } else {
                (&tab.output[..tab.output_len], &[][..], &b""[..])
            }
        })
        .unwrap_or((&[][..], &[][..], b"root@sunlight:/$ "));

    // Parse output into a terminal-sized grid. The framebuffer renderer already
    // offsets this grid below the title/tab chrome, so the VT cursor must stay
    // relative to the terminal content, not the full framebuffer.

    // FIX: Reuse cached grid instead of allocating 400KB+ per frame
    // This prevents bump allocator memory exhaustion that was causing freezes
    let grid = unsafe {
        match &mut GRID_CACHE {
            Some(cached) => {
                // Grid exists - check if dimensions match
                if cached.cols == cols && cached.rows == rows {
                    // Dimensions match - reuse grid, clear for fresh content
                    cached.clear_screen();  // FIX: Clear previous content before reuse
                    cached.as_mut()
                } else {
                    // Dimensions changed - allocate new grid
                    debug_log("[TTY]  Grid dimensions changed, reallocating");
                    *cached = Box::new(TerminalGrid::new(cols, rows));
                    cached.as_mut()
                }
            }
            None => {
                // First render - allocate and cache the grid
                debug_log("[TTY]  First render, caching grid");
                GRID_CACHE = Some(Box::new(TerminalGrid::new(cols, rows)));
                GRID_CACHE.as_mut().unwrap().as_mut()
            }
        }
    };

    grid.feed(output);
    let (cursor_row, cursor_col) = grid.cursor();

    // Get viewport offset for scrollback
    let viewport_offset = unsafe { SCROLLBACK_STATE[active_tab].viewport_offset };

    // Render with scrollback offset if active. Both methods fill the grid's
    // internal term-cell buffer in place and return a borrowed slice — no
    // per-frame allocation (the bump allocator never frees).
    let term_cells = if viewport_offset > 0 {
        grid.to_term_cells_with_offset(&ANSI_COLORS, viewport_offset)
    } else {
        grid.to_term_cells(&ANSI_COLORS)
    };

    // Title-bar stats: "CPU 15% RAM 42%  12:22 AM | 2026/6/12 | eth0".
    // Cached and refreshed at most once per minute (the same cadence the clock
    // re-renders) so the per-keystroke render path never does a sysinfo syscall.
    let (clock_buf, clock_len) = unsafe { titlebar(get_time_utc()) };

    // Build the dynamic tab labels: each tab shows its running app's name
    // (uppercased by the renderer) or "SHELL" when idle, plus a "*" on
    // background tabs that still have a live foreground app.
    let mut labels = [sunlight_tui::TabLabel::empty(); MAX_TABS];
    let n_tabs = tab_count.max(1).min(MAX_TABS);
    for i in 0..n_tabs {
        if let Some(tab) = tabs.get(i).filter(|t| t.pid != 0) {
            let len = tab.fg_app_name_len.min(24);
            labels[i].name[..len].copy_from_slice(&tab.fg_app_name[..len]);
            labels[i].name_len = len;
            labels[i].running = tab.fg_pid.is_some();
        }
    }

    unsafe {
        sunlight_tui::render_terminal_grid(
            fb_addr as *mut u32,
            fb_w,
            fb_h,
            fb_p,
            &labels[..n_tabs],
            active_tab,
            cols,
            rows,
            term_cells,
            cursor_row,
            cursor_col,
            input_line,
            prompt_slice,
            &clock_buf[..clock_len],
        );
    }

    // NOTE: Grid is NOT dropped here - it's cached in GRID_CACHE for reuse on next render
    // This prevents the 400KB+ allocation that was exhausting the bump allocator heap
    // Grid stays alive until dimensions change or process exits
}

fn reset_login(login: &mut LoginScreen) {
    *login = LoginScreen::new();
    prefill_root_login(login);
    login.message = "Logged out. Please log in.";
}

fn prefill_root_login(login: &mut LoginScreen) {
    for &b in b"root" {
        login.username.push(b);
    }
    login.focused = LoginField::Password;
}

fn reset_tabs(tabs: &mut [ShellTab; MAX_TABS], tab_count: &mut usize, active_tab: &mut usize) {
    for tab in tabs.iter_mut() {
        *tab = ShellTab::empty();
    }
    *tab_count = 0;
    *active_tab = 0;
}

fn active_shell_tab(tabs: &[ShellTab; MAX_TABS], active_tab: usize) -> Option<&ShellTab> {
    tabs.get(active_tab).filter(|tab| tab.pid != 0)
}

fn active_shell_tab_mut(
    tabs: &mut [ShellTab; MAX_TABS],
    active_tab: usize,
) -> Option<&mut ShellTab> {
    tabs.get_mut(active_tab).filter(|tab| tab.pid != 0)
}

fn build_prompt(tab: &ShellTab, buf: &mut [u8]) -> usize {
    let username = if tab.username_len > 0 {
        &tab.username[..tab.username_len]
    } else {
        b"root"
    };
    let suffix = b"@sunlight:/$ ";

    let mut pos = 0;
    // Copy username
    for &b in username.iter().take(buf.len()) {
        buf[pos] = b;
        pos += 1;
        if pos >= buf.len() {
            break;
        }
    }
    // Copy suffix
    for &b in suffix.iter().take(buf.len() - pos) {
        buf[pos] = b;
        pos += 1;
    }
    pos
}

fn spawn_tab(
    tabs: &mut [ShellTab; MAX_TABS],
    tab_count: &mut usize,
    active_tab: &mut usize,
    next_shell_id: &mut u64,
    spawn_cap: CapabilityToken,
    uid: u32,
    gid: u32,
) -> bool {
    if *tab_count >= MAX_TABS {
        return false;
    }

    let shell_id = *next_shell_id;
    *next_shell_id += 1;
    let mut path = [0u8; 16];
    let path_len = make_shell_path(shell_id, &mut path);
    let (pw0, pw1, pw2, pw3) = pack_path(&path[..path_len]);
    // The kernel derives the TTY tab (kernel ring key) from shell_id parsed out
    // of this path, so children's fd0/fd1 route to this tab's rings. tty_server
    // uses the same shell_id as the ring key (see the foreground drive loop).
    let spawn_msg = IpcMsg::with_label(SpawnMsg::SPAWN)
        .word(0, pw0)
        .word(1, pw1)
        .word(2, pw2)
        .word(3, pw3)
        .word(4, uid as u64)
        .word(5, gid as u64);
    let spawn_reply = ipc_call(spawn_cap, spawn_msg);
    if spawn_reply.label != SpawnMsg::REPLY {
        debug_log("[TTY]  Spawning /bin/sshl FAILED");
        return false;
    }

    let index = *tab_count;
    tabs[index] = ShellTab::empty();
    tabs[index].shell_id = shell_id;
    tabs[index].pid = spawn_reply.words[0];
    *active_tab = index;
    *tab_count += 1;
    true
}

fn handle_ctrl_key(
    ascii: u8,
    tabs: &mut [ShellTab; MAX_TABS],
    tab_count: &mut usize,
    active_tab: &mut usize,
    next_shell_id: &mut u64,
    spawn_cap: Option<CapabilityToken>,
    phase3_6_done: &mut bool,
) -> bool {
    match ascii {
        b't' | b'T' => {
            if let Some(cap) = spawn_cap {
                if spawn_tab(tabs, tab_count, active_tab, next_shell_id, cap, 0, 0)
                    && !*phase3_6_done
                {
                    debug_log("[TTY]  Ctrl+T test: new tab OK");
                    debug_log("[SunlightOS] Phase 3.6 OK");
                    *phase3_6_done = true;
                }
                return true;
            }
        }
        b'w' | b'W' => {
            close_active_tab(tabs, tab_count, active_tab);
            return true;
        }
        b'1'..=b'9' => {
            let idx = (ascii - b'1') as usize;
            if idx < *tab_count {
                *active_tab = idx;
                return true;
            }
        }
        b'0' => {
            if *tab_count >= 10 {
                *active_tab = 9;
                return true;
            }
        }
        _ => {}
    }
    false
}

fn close_active_tab(
    tabs: &mut [ShellTab; MAX_TABS],
    tab_count: &mut usize,
    active_tab: &mut usize,
) {
    if *tab_count <= 1 {
        return;
    }

    for i in *active_tab..(*tab_count - 1) {
        tabs[i] = tabs[i + 1];
    }
    tabs[*tab_count - 1] = ShellTab::empty();
    *tab_count -= 1;
    if *active_tab >= *tab_count {
        *active_tab = *tab_count - 1;
    }
}

fn resolve_active_shell(
    tabs: &mut [ShellTab; MAX_TABS],
    active_tab: usize,
    logged_initial_spawn: &mut bool,
) {
    let Some(tab) = active_shell_tab_mut(tabs, active_tab) else {
        return;
    };
    if tab.cap.is_some() {
        return;
    }

    let mut name = [0u8; 16];
    let name_len = make_shell_name(tab.shell_id, &mut name);
    let Some(name_str) = core::str::from_utf8(&name[..name_len]).ok() else {
        return;
    };
    if let Some(cap) = nameserver_lookup(name_str) {
        tab.cap = Some(cap);
        if *logged_initial_spawn {
            debug_log("[TTY]  sunshell endpoint found");
            *logged_initial_spawn = false;
        }
        // Trigger the shell's greeting by sending a null byte (ignored by shell)
        // This causes the shell to immediately reply with its greeting output
        let _ = send_key_to_shell(cap, 0x00, &mut tab.output, &mut tab.output_len);

        let pending_len = tab.pending_len;
        for i in 0..pending_len {
            let b = tab.pending[i];
            let _ = send_key_to_shell(cap, b, &mut tab.output, &mut tab.output_len);
        }
        tab.pending_len = 0;
    }
}

fn key_ascii_from_msg(msg: &IpcMsg) -> Option<u8> {
    if msg.label == KbdMsg::KEY_EVENT {
        let (_keycode, pressed, _shift, ctrl, _alt, ascii) = unpack_key_event(msg.words[0]);
        // Suppress ctrl combos: Ctrl+T, Ctrl+1 etc. are handled by tty_server
        // itself and must NOT be forwarded as bare ASCII to the shell (which
        // would corrupt its line buffer, e.g. turning "id" into "1id").
        if pressed && !ctrl {
            ascii
        } else {
            None
        }
    } else {
        None
    }
}

fn update_input_echo(
    byte: u8,
    term_output: &mut [u8; TERM_OUTPUT_MAX],
    term_output_len: &mut usize,
    input_line: &mut [u8; INPUT_LINE_MAX],
    input_line_len: &mut usize,
    username: [u8; 32],
    username_len: usize,
) {
    match byte {
        b'\n' | b'\r' => {
            let mut prompt_buf = [0u8; 32];
            let username_ref = if username_len > 0 {
                &username[..username_len]
            } else {
                b"root"
            };
            let suffix = b"@sunlight:/$ ";
            let mut pos = 0;
            for &b in username_ref.iter().take(prompt_buf.len()) {
                prompt_buf[pos] = b;
                pos += 1;
                if pos >= prompt_buf.len() {
                    break;
                }
            }
            for &b in suffix.iter().take(prompt_buf.len() - pos) {
                prompt_buf[pos] = b;
                pos += 1;
            }
            append_term(term_output, term_output_len, &prompt_buf[..pos]);
            append_term(term_output, term_output_len, &input_line[..*input_line_len]);
            append_term(term_output, term_output_len, b"\n");
            *input_line_len = 0;
        }
        0x08 => {
            if *input_line_len > 0 {
                *input_line_len -= 1;
            }
        }
        c if (0x20..=0x7e).contains(&c) => {
            if *input_line_len < input_line.len() {
                input_line[*input_line_len] = c;
                *input_line_len += 1;
            }
        }
        _ => {}
    }
}

/// Outcome of forwarding a keystroke to the shell.
enum ShellKeyResult {
    /// Normal: the shell handled the key (output already appended).
    Continue,
    /// The shell wants to exit (logout).
    Exited,
    /// The shell launched a foreground command; tty_server now drives it.
    /// Carries (pid, app-name buffer, app-name length).
    ForegroundStarted(u64, [u8; 24], usize),
}

fn send_key_to_shell(
    cap: CapabilityToken,
    byte: u8,
    term_output: &mut [u8; TERM_OUTPUT_MAX],
    term_output_len: &mut usize,
) -> ShellKeyResult {
    let kbd_msg = IpcMsg::with_label(KBD_LABEL).word(0, byte as u64);
    let reply = ipc_call(cap, kbd_msg);
    if reply.label == EXIT_LABEL {
        return ShellKeyResult::Exited;
    }
    if reply.label == FG_STARTED_LABEL {
        // words[0] = pid, words[1] = name length, words[2..] = name bytes
        // packed 8 per word (little-endian).
        let pid = reply.words[0];
        let name_len = (reply.words[1] as usize).min(24);
        let mut name = [0u8; 24];
        let mut ni = 0usize;
        let mut wi = 2usize;
        while ni < name_len && wi < reply.words.len() {
            let bytes = reply.words[wi].to_le_bytes();
            let mut bi = 0usize;
            while bi < 8 && ni < name_len {
                name[ni] = bytes[bi];
                ni += 1;
                bi += 1;
            }
            wi += 1;
        }
        return ShellKeyResult::ForegroundStarted(pid, name, name_len);
    }
    append_shell_reply(cap, term_output, term_output_len, &reply);
    ShellKeyResult::Continue
}

fn append_shell_reply(
    cap: CapabilityToken,
    term_output: &mut [u8; TERM_OUTPUT_MAX],
    term_output_len: &mut usize,
    reply: &IpcMsg,
) {
    if reply.label != OUTPUT_LABEL {
        return;
    }

    let mut remaining = reply.words[1] as usize;
    append_one_chunk(term_output, term_output_len, reply, remaining == 0);

    // Drain additional chunks if the shell has long output pending. IPC replies
    // currently return four register words, so payload bytes live in words 2..4.
    let mut seq: u64 = 1;
    let mut safety: usize = 64; // hard cap to avoid infinite drain loops
    while remaining > 0 && safety > 0 {
        let drain_msg = IpcMsg::with_label(DRAIN_LABEL).word(0, seq);
        let next = ipc_call(cap, drain_msg);
        if next.label != OUTPUT_LABEL {
            break;
        }
        remaining = next.words[1] as usize;
        append_one_chunk(term_output, term_output_len, &next, remaining == 0);
        seq += 1;
        safety -= 1;
    }
}

fn append_one_chunk(
    term_output: &mut [u8; TERM_OUTPUT_MAX],
    term_output_len: &mut usize,
    reply: &IpcMsg,
    append_missing_newline: bool,
) {
    let len = (reply.words[0] as usize).min(IPC_OUTPUT_BYTES);
    if len == 0 {
        return;
    }

    let mut bytes = [0u8; IPC_OUTPUT_BYTES];
    for i in 0..len {
        let word_idx = 2 + i / 8;
        if word_idx >= 4 {
            break;
        }
        let byte_idx = i % 8;
        bytes[i] = ((reply.words[word_idx] >> (byte_idx * 8)) & 0xff) as u8;
    }

    append_term(term_output, term_output_len, &bytes[..len]);
    if append_missing_newline && bytes[len - 1] != b'\n' {
        append_term(term_output, term_output_len, b"\n");
    }
}

/// True if `needle` appears anywhere in `haystack`.
fn slice_contains(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || needle.len() > haystack.len() {
        return false;
    }
    haystack.windows(needle.len()).any(|w| w == needle)
}

fn append_term(output: &mut [u8; TERM_OUTPUT_MAX], output_len: &mut usize, data: &[u8]) {
    if data.is_empty() {
        return;
    }

    // Reset the buffer on a full-screen clear (ESC[2J) or on alt-screen
    // enter/exit (ESC[?1049h / ESC[?1049l). This gives full-screen apps like
    // top a clean screen and returns to a clean prompt when they exit, without
    // clearing scrollback for ordinary streaming commands (ls, cat, ...), whose
    // output simply appends and slides on overflow below.
    let starts_with_clear = data.len() >= 4
        && data[0] == b'\x1B'
        && data[1] == b'['
        && data[2] == b'2'
        && data[3] == b'J';
    if starts_with_clear || slice_contains(data, b"\x1b[?1049") {
        *output_len = 0; // Clear the accumulated output buffer
    }

    if data.len() >= output.len() {
        let start = data.len() - output.len();
        output.copy_from_slice(&data[start..]);
        *output_len = output.len();
        return;
    }

    let overflow = output_len
        .saturating_add(data.len())
        .saturating_sub(output.len());
    if overflow > 0 {
        let keep = *output_len - overflow;
        for i in 0..keep {
            output[i] = output[i + overflow];
        }
        *output_len = keep;
    }

    let start = *output_len;
    output[start..start + data.len()].copy_from_slice(data);
    *output_len += data.len();
}

fn debug_log_kbd_byte(prefix: &str, byte: u8) {
    let mut buf = [0u8; 64];
    let pb = prefix.as_bytes();
    let plen = pb.len().min(60);
    buf[..plen].copy_from_slice(&pb[..plen]);
    let dstart = plen;
    let dlen = if byte < 10 {
        buf[dstart] = b'0' + byte;
        1
    } else if byte < 100 {
        buf[dstart] = b'0' + byte / 10;
        buf[dstart + 1] = b'0' + byte % 10;
        2
    } else {
        buf[dstart] = b'0' + byte / 100;
        buf[dstart + 1] = b'0' + (byte % 100) / 10;
        buf[dstart + 2] = b'0' + byte % 10;
        3
    };
    if let Ok(s) = core::str::from_utf8(&buf[..dstart + dlen]) {
        debug_log(s);
    }
}

fn debug_log_login_success(username: &[u8], uid: u32, gid: u32) {
    let mut buf = [0u8; 128];
    let prefix = b"[TTY]  Login success: ";
    let mut pos = prefix.len();
    buf[..pos].copy_from_slice(prefix);
    let ulen = username.len().min(64);
    buf[pos..pos + ulen].copy_from_slice(&username[..ulen]);
    pos += ulen;
    let mid = b" (uid=";
    buf[pos..pos + mid.len()].copy_from_slice(mid);
    pos += mid.len();
    pos += fmt_u32(&mut buf[pos..], uid);
    let sep = b", gid=";
    buf[pos..pos + sep.len()].copy_from_slice(sep);
    pos += sep.len();
    pos += fmt_u32(&mut buf[pos..], gid);
    buf[pos] = b')';
    pos += 1;
    if let Ok(s) = core::str::from_utf8(&buf[..pos]) {
        debug_log(s);
    }
}

fn debug_log_spawn(_username: &[u8], pid: u64) {
    let mut buf = [0u8; 128];
    let prefix = b"[TTY]  Spawning /bin/sshl (pid=";
    let mut pos = prefix.len();
    buf[..pos].copy_from_slice(prefix);
    pos += fmt_u64(&mut buf[pos..], pid);
    buf[pos] = b')';
    pos += 1;
    let suffix = b"...";
    buf[pos..pos + suffix.len()].copy_from_slice(suffix);
    pos += suffix.len();
    if let Ok(s) = core::str::from_utf8(&buf[..pos]) {
        debug_log(s);
    }
}

fn fmt_u32(buf: &mut [u8], val: u32) -> usize {
    if val == 0 {
        buf[0] = b'0';
        return 1;
    }
    let mut tmp = [0u8; 10];
    let mut n = 0;
    let mut v = val;
    while v > 0 {
        tmp[n] = b'0' + (v % 10) as u8;
        v /= 10;
        n += 1;
    }
    for i in 0..n {
        buf[i] = tmp[n - 1 - i];
    }
    n
}

/// Build the title-bar string shown in the header next to the "TTY" label:
/// `CPU 15% RAM 42%  <clock> | eth0`.
///
/// Tries "tz" TZ_GET_LOCAL_TIME for local time + abbr. Falls back to UTC
/// from kernel + "UTC" label if tz service not yet available.
///
/// SAFETY: touches function-local `static mut` cache; tty_server is
/// single-threaded so there is no concurrent access.
unsafe fn titlebar(ts: u64) -> ([u8; 64], usize) {
    static mut CACHE_BUF: [u8; 64] = [0; 64];
    static mut CACHE_LEN: usize = 0;
    static mut CACHE_MIN: u64 = u64::MAX;

    let now_min = ts / 60;
    if now_min == CACHE_MIN && CACHE_LEN != 0 {
        return (CACHE_BUF, CACHE_LEN);
    }

    let mut buf = [0u8; 64];
    let mut n = 0usize;

    // CPU usage (placeholder until scheduler accounting lands — matches the old
    // banner's behaviour).
    let cpu_percent: u64 = 15;
    n += copy_into(&mut buf[n..], b"CPU ");
    n += fmt_u64(&mut buf[n..], cpu_percent);
    n += copy_into(&mut buf[n..], b"% RAM ");

    let info = sysinfo();
    let total = info.total_ram_kb.max(1);
    let ram_percent = (info.used_ram_kb * 100) / total;
    n += fmt_u64(&mut buf[n..], ram_percent);
    n += copy_into(&mut buf[n..], b"%  ");

    // Local time via tz service if available (sub-phase 9 refactor)
    let (clock_len, used_tz) = try_local_clock(&mut buf[n..], ts);
    n += clock_len;
    if !used_tz {
        // fallback already wrote UTC style inside try_ or here
    }
    n += copy_into(&mut buf[n..], b" | eth0");

    CACHE_BUF = buf;
    CACHE_LEN = n;
    CACHE_MIN = now_min;
    (buf, n)
}

/// Attempt to get local time+abbr from "tz" service. Returns (bytes_written, used_tz).
/// On failure falls back to writing a UTC clock string (using existing fmt).
unsafe fn try_local_clock(dst: &mut [u8], _ts: u64) -> (usize, bool) {
    if let Some(tz_cap) = nameserver_lookup("tz") {
        let req = IpcMsg::with_label(TzMsg::GET_LOCAL_TIME);
        let reply = ipc_call(tz_cap, req);
        if reply.label == TzMsg::REPLY && reply.word_count >= 1 {
            // unpack word(0)
            let w0 = reply.words[0];
            let hour = ((w0 >> 24) & 0xff) as u8;
            let minute = ((w0 >> 16) & 0xff) as u8;
            // abbr from word(3) low 8 bytes
            let abw = if reply.word_count > 3 { reply.words[3] } else { 0 };
            let mut abbr = [0u8; 5]; // short for title e.g. IRDT or UTC
            for i in 0..4 {
                let b = ((abw >> (i*8)) & 0xff) as u8;
                if b == 0 { break; }
                abbr[i] = b;
            }
            // write HH:MM ABBR
            let mut p = 0usize;
            dst[p] = b'0' + hour/10; p+=1;
            dst[p] = b'0' + hour%10; p+=1;
            dst[p] = b':'; p+=1;
            dst[p] = b'0' + minute/10; p+=1;
            dst[p] = b'0' + minute%10; p+=1;
            dst[p] = b' '; p+=1;
            for i in 0..4 {
                if abbr[i] == 0 { break; }
                dst[p] = abbr[i]; p+=1;
            }
            return (p, true);
        }
    }
    // fallback UTC using tui fmt (no abbr change)
    let len = sunlight_tui::fmt::fmt_clock(dst, sunlight_ipc::get_time_utc());
    (len, false)
}

/// Copy `src` into the front of `dst`, returning the number of bytes written
/// (bounded by `dst`'s length).
fn copy_into(dst: &mut [u8], src: &[u8]) -> usize {
    let n = src.len().min(dst.len());
    dst[..n].copy_from_slice(&src[..n]);
    n
}

fn fmt_u64(buf: &mut [u8], val: u64) -> usize {
    if val == 0 {
        buf[0] = b'0';
        return 1;
    }
    let mut tmp = [0u8; 20];
    let mut n = 0;
    let mut v = val;
    while v > 0 {
        tmp[n] = b'0' + (v % 10) as u8;
        v /= 10;
        n += 1;
    }
    for i in 0..n {
        buf[i] = tmp[n - 1 - i];
    }
    n
}

fn pack_bytes(bytes: &[u8]) -> u64 {
    let mut out = 0u64;
    let mut idx = 0;
    while idx < bytes.len() && idx < 8 {
        out |= (bytes[idx] as u64) << (idx * 8);
        idx += 1;
    }
    out
}

fn make_shell_path(shell_id: u64, out: &mut [u8]) -> usize {
    let prefix = b"/bin/sshl";
    out[..prefix.len()].copy_from_slice(prefix);
    prefix.len() + fmt_u64(&mut out[prefix.len()..], shell_id)
}

fn make_shell_name(shell_id: u64, out: &mut [u8]) -> usize {
    let prefix = b"sshl";
    out[..prefix.len()].copy_from_slice(prefix);
    prefix.len() + fmt_u64(&mut out[prefix.len()..], shell_id)
}

/// Pack a path (up to 32 bytes) into four u64 words for IPC transport.
fn pack_path(path: &[u8]) -> (u64, u64, u64, u64) {
    let mut words = [0u64; 4];
    let mut word_idx = 0;
    while word_idx < 4 {
        let start = word_idx * 8;
        if start >= path.len() {
            break;
        }
        let end = (start + 8).min(path.len());
        words[word_idx] = pack_bytes(&path[start..end]);
        word_idx += 1;
    }
    (words[0], words[1], words[2], words[3])
}
