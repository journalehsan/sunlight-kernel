#![no_std]
#![no_main]

extern crate alloc;

use sunlight_ipc::{
    debug_log, ipc_call, nameserver_lookup, process_yield, shm_create, unpack_key_event,
    CapabilityToken, IpcMsg, PtyMsg, SgpMsg,
};
use sunlight_libc as libc;
use sunlight_tty::console::Console;
use sunlight_tui::{framebuffer::Framebuffer, font, ANSI_COLORS};

// ── Window geometry ────────────────────────────────────────────────────────
const WIN_W: u32 = 640;
const WIN_H: u32 = 360;
const TAB_H: u32 = 28;    // top chrome
const FOOTER_H: u32 = 32; // prompt/input bar at the bottom
const PAD_X: u32 = 8;
const PAD_Y: u32 = 4;
const CELL_W: u32 = 8;
const CELL_H: u32 = 16;

// Content area dimensions (derived)
const CONTENT_Y: u32 = TAB_H + PAD_Y;
const CONTENT_H: u32 = WIN_H - TAB_H - FOOTER_H - PAD_Y; // no bottom pad; footer starts right below
const CONTENT_COLS: usize = ((WIN_W - PAD_X * 2) / CELL_W) as usize;
const CONTENT_ROWS: usize = (CONTENT_H / CELL_H) as usize;

// ── Colors ─────────────────────────────────────────────────────────────────
const BG: u32 = 0x00141418;
const PANEL: u32 = 0x001D1F25;
const PANEL_2: u32 = 0x00262630;
const FOOTER_BG: u32 = 0x000E0E12;
const SEP_COLOR: u32 = 0x002A2A35;
const TEXT_DIM: u32 = 0x009A948B;
const TEXT_NORMAL: u32 = 0x00D4D0C8;
const ACCENT: u32 = 0x00FF7A00;
const GREEN: u32 = 0x0050FA7B;
const CURSOR_COL: u32 = 0x00FF7A00;

// ── Key scan codes (PS/2 set-1) ────────────────────────────────────────────
const KEY_BACKSPACE: u8 = 0x0E;
const KEY_ENTER: u8 = 0x1C;
const KEY_UP: u8 = 0x48;
const KEY_DOWN: u8 = 0x50;
const KEY_LEFT: u8 = 0x4B;
const KEY_RIGHT: u8 = 0x4D;
const KEY_HOME: u8 = 0x47;
const KEY_END: u8 = 0x4F;
const KEY_DEL: u8 = 0x53;

// ── Capacity constants ─────────────────────────────────────────────────────
const INPUT_MAX: usize = 240;
const PROMPT_MAX: usize = 64;
const APP_NAME_MAX: usize = 32;
const HIST_MAX: usize = 32;
const READ_BUF: usize = 256;

// ── Bump allocator ─────────────────────────────────────────────────────────
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
#[global_allocator]
static ALLOC: BumpAllocator = BumpAllocator;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    debug_log("[TERM] panic\n");
    loop {
        process_yield();
    }
}

// ── PTY session ────────────────────────────────────────────────────────────
struct PtySession {
    id: u64,
    cap: CapabilityToken,
}

impl PtySession {
    fn open() -> Result<Self, ()> {
        let cap = nameserver_lookup("pty").ok_or(())?;
        let reply = ipc_call(cap, IpcMsg::with_label(PtyMsg::CREATE));
        if reply.label != PtyMsg::REPLY || reply.cap_count < 2 {
            return Err(());
        }
        let session = Self {
            id: reply.words[0],
            cap,
        };
        // Raw mode: no echo, no canonical. The terminal manages input display.
        ipc_call(
            cap,
            IpcMsg::with_label(PtyMsg::SET_MODE)
                .word(0, reply.words[0])
                .word(1, 0),
        );
        Ok(session)
    }

    fn write(&self, bytes: &[u8]) {
        let mut pos = 0;
        while pos < bytes.len() {
            let chunk = (bytes.len() - pos).min(16);
            let mut msg = IpcMsg::with_label(PtyMsg::WRITE_MASTER)
                .word(0, self.id)
                .word(1, chunk as u64);
            for (wi, cb) in bytes[pos..pos + chunk].chunks(8).enumerate() {
                let mut w = 0u64;
                for (bi, &b) in cb.iter().enumerate() {
                    w |= (b as u64) << (bi * 8);
                }
                msg = msg.word(2 + wi, w);
            }
            let r = ipc_call(self.cap, msg);
            if r.label != PtyMsg::REPLY {
                break;
            }
            let accepted = (r.words[1] as usize).min(chunk);
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
            let r = ipc_call(
                self.cap,
                IpcMsg::with_label(PtyMsg::READ_MASTER)
                    .word(0, self.id)
                    .word(1, chunk as u64),
            );
            if r.label != PtyMsg::REPLY {
                break;
            }
            let n = (r.words[1] as usize).min(chunk);
            if n == 0 {
                break;
            }
            for i in 0..n {
                out[total + i] = ((r.words[2 + (i / 8)] >> ((i % 8) * 8)) & 0xFF) as u8;
            }
            total += n;
            if n < chunk {
                break;
            }
        }
        total
    }
}

// ── Footer state ───────────────────────────────────────────────────────────
struct Footer {
    // Prompt text (received via OSC 9001;prompt;TEXT from shell)
    prompt: [u8; PROMPT_MAX],
    prompt_len: usize,

    // Current user input line (managed locally)
    input: [u8; INPUT_MAX],
    input_len: usize,
    input_cursor: usize,

    // History ring
    history: [[u8; INPUT_MAX]; HIST_MAX],
    history_lens: [usize; HIST_MAX],
    history_head: usize,
    history_count: usize,
    hist_pos: usize,
    hist_stash: [u8; INPUT_MAX],
    hist_stash_len: usize,

    // App mode: a long-running foreground app owns keyboard + content
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
        let n = text.len().min(PROMPT_MAX);
        self.prompt[..n].copy_from_slice(&text[..n]);
        self.prompt_len = n;
    }

    fn insert(&mut self, ch: u8) {
        if self.input_len >= INPUT_MAX {
            return;
        }
        let c = self.input_cursor;
        let mut i = self.input_len;
        while i > c {
            self.input[i] = self.input[i - 1];
            i -= 1;
        }
        self.input[c] = ch;
        self.input_len += 1;
        self.input_cursor += 1;
        self.hist_pos = 0;
    }

    fn backspace(&mut self) {
        if self.input_cursor == 0 {
            return;
        }
        let c = self.input_cursor - 1;
        let mut i = c;
        while i + 1 < self.input_len {
            self.input[i] = self.input[i + 1];
            i += 1;
        }
        self.input_len -= 1;
        self.input_cursor -= 1;
        self.hist_pos = 0;
    }

    fn delete_fwd(&mut self) {
        if self.input_cursor >= self.input_len {
            return;
        }
        let c = self.input_cursor;
        let mut i = c;
        while i + 1 < self.input_len {
            self.input[i] = self.input[i + 1];
            i += 1;
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
            let n = self.input_len.min(INPUT_MAX);
            self.hist_stash[..n].copy_from_slice(&self.input[..n]);
            self.hist_stash_len = n;
        }
        self.hist_pos += 1;
        let slot = (self.history_head + self.history_count - self.hist_pos) % HIST_MAX;
        let len = self.history_lens[slot].min(INPUT_MAX);
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
            let n = self.hist_stash_len.min(INPUT_MAX);
            self.input[..n].copy_from_slice(&self.hist_stash[..n]);
            self.input_len = n;
            self.input_cursor = n;
        } else {
            let slot = (self.history_head + self.history_count - self.hist_pos) % HIST_MAX;
            let len = self.history_lens[slot].min(INPUT_MAX);
            self.input[..len].copy_from_slice(&self.history[slot][..len]);
            self.input_len = len;
            self.input_cursor = len;
        }
    }

    fn push_history(&mut self) {
        if self.input_len == 0 {
            return;
        }
        if self.history_count > 0 {
            let last = (self.history_head + self.history_count - 1) % HIST_MAX;
            if self.history_lens[last] == self.input_len
                && self.history[last][..self.input_len] == self.input[..self.input_len]
            {
                return;
            }
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
        let n = self.input_len.min(INPUT_MAX);
        self.history[slot][..n].copy_from_slice(&self.input[..n]);
        self.history_lens[slot] = n;
    }

    /// Consume the input line, return it. Caller must send it to PTY.
    fn take_line(&mut self) -> ([u8; INPUT_MAX], usize) {
        self.push_history();
        let mut buf = [0u8; INPUT_MAX];
        let n = self.input_len;
        buf[..n].copy_from_slice(&self.input[..n]);
        self.input_len = 0;
        self.input_cursor = 0;
        self.hist_pos = 0;
        (buf, n)
    }

    fn enter_app_mode(&mut self, name: &[u8]) {
        self.app_mode = true;
        let n = name.len().min(APP_NAME_MAX);
        self.app_name[..n].copy_from_slice(&name[..n]);
        self.app_name_len = n;
    }

    fn exit_app_mode(&mut self) {
        self.app_mode = false;
        self.app_name_len = 0;
    }
}

// ── OSC stream parser ──────────────────────────────────────────────────────
// Extracts `\x1b]9001;OP;DATA\x07` control sequences emitted by the shell.
// Everything else is passed through to the Console.
struct OscParser {
    state: u8, // 0=normal 1=got_esc 2=got_osc_start 3=in_osc_body
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

    /// Feed bytes from PTY master_out. For each byte:
    ///   - Regular bytes → appended to `console_out` slice (up to `console_cap`)
    ///   - Completed OSC 9001 sequence → calls the provided closure
    /// Returns how many bytes were placed in `console_out`.
    fn feed<F: FnMut(&[u8])>(&mut self, bytes: &[u8], console_out: &mut [u8], console_len: &mut usize, mut on_osc: F) {
        for &b in bytes {
            match self.state {
                0 => {
                    if b == 0x1B {
                        self.state = 1;
                    } else {
                        self.passthrough(b, console_out, console_len);
                    }
                }
                1 => {
                    if b == b']' {
                        self.state = 2;
                        self.body_len = 0;
                    } else {
                        // Not an OSC — pass ESC + this byte through
                        self.passthrough(0x1B, console_out, console_len);
                        self.passthrough(b, console_out, console_len);
                        self.state = 0;
                    }
                }
                2 => {
                    // Accumulate OSC body
                    if b == 0x07 {
                        // BEL: end of OSC
                        on_osc(&self.body[..self.body_len]);
                        self.body_len = 0;
                        self.state = 0;
                    } else if b == 0x1B {
                        // Could be ESC \ (ST) — treat as end
                        on_osc(&self.body[..self.body_len]);
                        self.body_len = 0;
                        self.state = 1;
                    } else {
                        if self.body_len < self.body.len() {
                            self.body[self.body_len] = b;
                            self.body_len += 1;
                        }
                    }
                }
                _ => {
                    self.state = 0;
                    self.passthrough(b, console_out, console_len);
                }
            }
        }
    }

    fn passthrough(&self, b: u8, out: &mut [u8], len: &mut usize) {
        if *len < out.len() {
            out[*len] = b;
            *len += 1;
        }
    }
}

/// Parse a completed OSC body: `9001;OP;DATA` or `9001;OP`
enum OscCmd<'a> {
    Prompt(&'a [u8]),
    AppStart(&'a [u8]),
    AppDone,
    Unknown,
}

fn parse_osc(body: &[u8]) -> OscCmd<'_> {
    // Must start with "9001;"
    let prefix = b"9001;";
    if !body.starts_with(prefix) {
        return OscCmd::Unknown;
    }
    let rest = &body[prefix.len()..];
    // Find next ';'
    if let Some(sep) = rest.iter().position(|&b| b == b';') {
        let op = &rest[..sep];
        let data = &rest[sep + 1..];
        if op == b"prompt" {
            return OscCmd::Prompt(data);
        }
        if op == b"app_start" {
            return OscCmd::AppStart(data);
        }
    } else {
        // No second ';'
        if rest == b"app_done" {
            return OscCmd::AppDone;
        }
    }
    OscCmd::Unknown
}

// ── Helpers ────────────────────────────────────────────────────────────────
fn fmt_u64(buf: &mut [u8], mut v: u64) -> usize {
    if v == 0 {
        if !buf.is_empty() {
            buf[0] = b'0';
        }
        return 1;
    }
    let mut tmp = [0u8; 20];
    let mut n = 0;
    while v > 0 && n < tmp.len() {
        tmp[n] = b'0' + (v % 10) as u8;
        v /= 10;
        n += 1;
    }
    for i in 0..n.min(buf.len()) {
        buf[i] = tmp[n - 1 - i];
    }
    n.min(buf.len())
}

fn spawn_shell(pty: &PtySession, shell_id: u64) -> Result<u64, ()> {
    let mut path_buf = [0u8; 32];
    let mut arg0 = [0u8; 16];
    let mut arg_session = [0u8; 48];
    let mut arg_cap = [0u8; 48];

    let mut path_len = 0;
    for &b in b"/bin/sshl" {
        path_buf[path_len] = b;
        path_len += 1;
    }
    path_len += fmt_u64(&mut path_buf[path_len..], shell_id);

    let mut a0_len = 0;
    for &b in b"sshl" {
        arg0[a0_len] = b;
        a0_len += 1;
    }
    a0_len += fmt_u64(&mut arg0[a0_len..], shell_id);

    let mut aps_len = 0;
    for &b in b"--pty-session=" {
        arg_session[aps_len] = b;
        aps_len += 1;
    }
    aps_len += fmt_u64(&mut arg_session[aps_len..], pty.id);

    let mut apc_len = 0;
    for &b in b"--pty-cap=" {
        arg_cap[apc_len] = b;
        apc_len += 1;
    }
    apc_len += fmt_u64(&mut arg_cap[apc_len..], pty.cap.0);

    let argv = [&arg0[..a0_len], &arg_session[..aps_len], &arg_cap[..apc_len]];
    libc::spawn(&path_buf[..path_len], &argv, None).map_err(|_| ())
}

fn create_window(display: CapabilityToken) -> Result<(u64, *mut u32), ()> {
    let reply = ipc_call(
        display,
        IpcMsg::with_label(SgpMsg::CREATE_WINDOW)
            .word(0, WIN_W as u64 | ((WIN_H as u64) << 32)),
    );
    if reply.label != SgpMsg::REPLY || reply.cap_count == 0 {
        return Err(());
    }
    let buffer = sunlight_ipc::shm_map(reply.caps[0]).map_err(|_| ())? as *mut u32;
    Ok((reply.words[0], buffer))
}

fn set_window_title(display: CapabilityToken, win_id: u64, title: &[u8]) {
    if let Ok((ptr, shm)) = shm_create(4096, 0) {
        let n = title.len().min(4095);
        unsafe {
            core::ptr::copy_nonoverlapping(title.as_ptr(), ptr, n);
            ptr.add(n).write_volatile(0);
        }
        let mut msg = IpcMsg::with_label(SgpMsg::CONFIGURE_WINDOW).word(0, win_id);
        msg.caps[0] = shm;
        msg.cap_count = 1;
        let _ = ipc_call(display, msg);
    }
}

// ── Rendering ──────────────────────────────────────────────────────────────
unsafe fn render(buffer: *mut u32, console: &mut Console, footer: &Footer) {
    let mut fb = Framebuffer::from_limine(buffer, WIN_W, WIN_H, WIN_W * 4);

    // Background
    fb.fill_rect(0, 0, WIN_W, WIN_H, BG);

    // ── Tab bar ──
    fb.fill_rect(0, 0, WIN_W, TAB_H, PANEL);
    fb.hline(0, TAB_H, WIN_W, SEP_COLOR);
    fb.fill_rect(8, 5, 60, TAB_H - 10, PANEL_2);
    font::draw_str(&mut fb, 14, 8, "Tab 1", ACCENT, 1);
    fb.fill_rect(76, 5, 20, TAB_H - 10, PANEL_2);
    font::draw_str(&mut fb, 82, 8, "+", TEXT_DIM, 1);

    // ── Content area (Console) ──
    let rows = CONTENT_ROWS;
    let cols = CONTENT_COLS;
    let cells = console.to_term_cells(&ANSI_COLORS);
    for row in 0..rows {
        let y = CONTENT_Y + row as u32 * CELL_H;
        for col in 0..cols {
            let idx = row * cols + col;
            if idx >= cells.len() {
                break;
            }
            let cell = cells[idx];
            let x = PAD_X + col as u32 * CELL_W;
            fb.fill_rect(x, y, CELL_W, CELL_H, cell.bg);
            if cell.ch >= 0x20 && cell.ch <= 0x7E && cell.ch != b' ' {
                font::draw_char(&mut fb, x, y, cell.ch, cell.fg, 1);
            }
        }
    }

    // ── Footer separator ──
    let footer_y = WIN_H - FOOTER_H;
    fb.hline(0, footer_y, WIN_W, SEP_COLOR);
    fb.fill_rect(0, footer_y + 1, WIN_W, FOOTER_H - 1, FOOTER_BG);

    // ── Footer content ──
    let text_y = footer_y + (FOOTER_H - CELL_H) / 2;
    if footer.app_mode {
        // App mode: show ● APP_NAME [running]
        fb.fill_rect(PAD_X, text_y + 4, 8, 8, GREEN);
        let mut x = PAD_X + 14;
        if footer.app_name_len > 0 {
            if let Ok(s) = core::str::from_utf8(&footer.app_name[..footer.app_name_len]) {
                font::draw_str(&mut fb, x, text_y, s, TEXT_NORMAL, 1);
                x += footer.app_name_len as u32 * CELL_W;
            }
        } else {
            font::draw_str(&mut fb, x, text_y, "app", TEXT_NORMAL, 1);
            x += 3 * CELL_W;
        }
        font::draw_str(&mut fb, x + 8, text_y, "[running]", TEXT_DIM, 1);
    } else {
        // Normal mode: show prompt + input line + cursor
        let mut x = PAD_X;

        // Prompt
        if footer.prompt_len > 0 {
            if let Ok(s) = core::str::from_utf8(&footer.prompt[..footer.prompt_len]) {
                font::draw_str(&mut fb, x, text_y, s, ACCENT, 1);
                x += footer.prompt_len as u32 * CELL_W;
            }
        } else {
            font::draw_str(&mut fb, x, text_y, "$ ", TEXT_DIM, 1);
            x += 2 * CELL_W;
        }

        // Input line (before cursor)
        let cursor = footer.input_cursor;
        if cursor > 0 {
            if let Ok(s) = core::str::from_utf8(&footer.input[..cursor]) {
                font::draw_str(&mut fb, x, text_y, s, TEXT_NORMAL, 1);
                x += cursor as u32 * CELL_W;
            }
        }

        // Cursor block
        fb.fill_rect(x, text_y, CELL_W, CELL_H, CURSOR_COL);
        if cursor < footer.input_len {
            let ch = footer.input[cursor];
            if ch >= 0x20 && ch <= 0x7E {
                font::draw_char(&mut fb, x, text_y, ch, BG, 1);
            }
        }
        x += CELL_W;

        // Input line (after cursor)
        if cursor < footer.input_len {
            if let Ok(s) = core::str::from_utf8(&footer.input[cursor..footer.input_len]) {
                font::draw_str(&mut fb, x, text_y, s, TEXT_NORMAL, 1);
            }
        }
    }
}

// ── Entry point ────────────────────────────────────────────────────────────
#[no_mangle]
pub extern "C" fn _start() -> ! {
    debug_log("[TERM] sunlight-terminal starting\n");

    let display = loop {
        if let Some(cap) = nameserver_lookup("display_server") {
            break cap;
        }
        process_yield();
    };

    let pty = match PtySession::open() {
        Ok(p) => p,
        Err(_) => {
            debug_log("[TERM] failed to open PTY\n");
            loop {
                process_yield();
            }
        }
    };

    let shell_id = (libc::getpid() as u8).max(1) as u64;
    if spawn_shell(&pty, shell_id).is_err() {
        debug_log("[TERM] failed to spawn shell\n");
    }

    let mut console = Console::new(CONTENT_COLS, CONTENT_ROWS);
    let mut footer = Footer::new();
    let mut osc = OscParser::new();

    let (win_id, buffer) = match create_window(display) {
        Ok(v) => v,
        Err(_) => {
            debug_log("[TERM] failed to create window\n");
            loop {
                process_yield();
            }
        }
    };
    set_window_title(display, win_id, b"Sunlight Terminal");

    unsafe {
        render(buffer, &mut console, &footer);
    }
    let _ = ipc_call(
        display,
        IpcMsg::with_label(SgpMsg::COMMIT_FRAME).word(0, win_id),
    );

    let mut read_buf = [0u8; READ_BUF];
    let mut console_buf = [0u8; READ_BUF];

    loop {
        let mut dirty = false;

        // ── Poll keyboard event ──────────────────────────────────────────
        let poll = ipc_call(
            display,
            IpcMsg::with_label(SgpMsg::EVENT_POLL).word(0, win_id),
        );
        if poll.label == SgpMsg::REPLY && poll.words[2] != 0 {
            let (keycode, pressed, _shift, ctrl, _alt, ascii) =
                unpack_key_event(poll.words[2]);

            if pressed {
                if footer.app_mode {
                    // App mode: forward raw sequences to PTY
                    let mut buf = [0u8; 4];
                    let n = translate_key_app(keycode, ctrl, ascii, &mut buf);
                    if n > 0 {
                        pty.write(&buf[..n]);
                    }
                } else {
                    // Normal mode: handle input locally
                    let mut send_line = false;
                    let mut ctrl_c = false;

                    if ctrl {
                        if let Some(ch) = ascii {
                            let lower = if ch.is_ascii_uppercase() { ch + 32 } else { ch };
                            match lower {
                                b'c' => {
                                    ctrl_c = true;
                                    // Clear current input on Ctrl+C
                                    footer.input_len = 0;
                                    footer.input_cursor = 0;
                                    dirty = true;
                                }
                                b'd' => {
                                    pty.write(&[0x04]);
                                }
                                b'l' => {
                                    console.feed(b"\x1b[2J\x1b[H");
                                    dirty = true;
                                }
                                _ => {}
                            }
                        }
                        if ctrl_c {
                            pty.write(&[0x03]);
                        }
                    } else {
                        // Check special keys by keycode first so that control
                        // characters like backspace (ascii=0x08) are always caught.
                        match keycode {
                            KEY_BACKSPACE => {
                                footer.backspace();
                                dirty = true;
                            }
                            KEY_ENTER => {
                                send_line = true;
                            }
                            KEY_LEFT => {
                                footer.move_left();
                                dirty = true;
                            }
                            KEY_RIGHT => {
                                footer.move_right();
                                dirty = true;
                            }
                            KEY_UP => {
                                footer.history_up();
                                dirty = true;
                            }
                            KEY_DOWN => {
                                footer.history_down();
                                dirty = true;
                            }
                            KEY_HOME => {
                                footer.home();
                                dirty = true;
                            }
                            KEY_END => {
                                footer.end();
                                dirty = true;
                            }
                            KEY_DEL => {
                                footer.delete_fwd();
                                dirty = true;
                            }
                            _ => {
                                // Regular printable character from ascii
                                if let Some(ch) = ascii {
                                    if ch == b'\r' || ch == b'\n' {
                                        send_line = true;
                                    } else if ch >= 0x20 && ch <= 0x7E {
                                        footer.insert(ch);
                                        dirty = true;
                                    }
                                }
                            }
                        }
                    }

                    if send_line {
                        let (line, n) = footer.take_line();
                        if n > 0 {
                            // Echo the command into the content area
                            // Format: prompt + command + newline
                            let mut echo = [0u8; 256];
                            let mut ep = 0;
                            for &b in &footer.prompt[..footer.prompt_len] {
                                if ep < echo.len() {
                                    echo[ep] = b;
                                    ep += 1;
                                }
                            }
                            for &b in &line[..n] {
                                if ep < echo.len() {
                                    echo[ep] = b;
                                    ep += 1;
                                }
                            }
                            if ep < echo.len() {
                                echo[ep] = b'\n';
                                ep += 1;
                            }
                            console.feed(&echo[..ep]);
                            // Send line + newline to PTY
                            pty.write(&line[..n]);
                            pty.write(b"\n");
                        } else {
                            // Empty line: just echo prompt + newline
                            let mut echo = [0u8; 128];
                            let mut ep = 0;
                            for &b in &footer.prompt[..footer.prompt_len] {
                                if ep < echo.len() {
                                    echo[ep] = b;
                                    ep += 1;
                                }
                            }
                            if ep < echo.len() {
                                echo[ep] = b'\n';
                                ep += 1;
                            }
                            console.feed(&echo[..ep]);
                            pty.write(b"\n");
                        }
                        dirty = true;
                    }
                }
            }
        }

        // ── Read PTY output ──────────────────────────────────────────────
        let n = pty.read(&mut read_buf);
        if n > 0 {
            let mut console_len = 0usize;
            osc.feed(
                &read_buf[..n],
                &mut console_buf,
                &mut console_len,
                |body| {
                    match parse_osc(body) {
                        OscCmd::Prompt(text) => {
                            footer.set_prompt(text);
                        }
                        OscCmd::AppStart(name) => {
                            footer.enter_app_mode(name);
                        }
                        OscCmd::AppDone => {
                            footer.exit_app_mode();
                        }
                        OscCmd::Unknown => {}
                    }
                },
            );
            if console_len > 0 {
                console.feed(&console_buf[..console_len]);
            }
            dirty = true;
        }

        if dirty || console.dirty {
            unsafe {
                render(buffer, &mut console, &footer);
            }
            console.clear_dirty();
            let _ = ipc_call(
                display,
                IpcMsg::with_label(SgpMsg::COMMIT_FRAME).word(0, win_id),
            );
        }

        process_yield();
    }
}

/// Translate a key event to bytes for forwarding to an app in app mode.
/// Returns byte count; buf must be at least 4 bytes.
fn translate_key_app(keycode: u8, ctrl: bool, ascii: Option<u8>, buf: &mut [u8; 4]) -> usize {
    if ctrl {
        if let Some(ch) = ascii {
            let lower = if ch.is_ascii_uppercase() { ch + 32 } else { ch };
            match lower {
                b'c' => {
                    buf[0] = 0x03;
                    return 1;
                }
                b'd' => {
                    buf[0] = 0x04;
                    return 1;
                }
                b'z' => {
                    buf[0] = 0x1A;
                    return 1;
                }
                b'l' => {
                    buf[0] = 0x0C;
                    return 1;
                }
                _ => {}
            }
        }
        return 0;
    }

    if let Some(ch) = ascii {
        if ch != 0 {
            buf[0] = ch;
            return 1;
        }
    }

    match keycode {
        KEY_ENTER => {
            buf[0] = b'\n';
            1
        }
        KEY_BACKSPACE => {
            buf[0] = 0x08;
            1
        }
        // Arrow keys → VT100 escape sequences
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
        KEY_HOME => {
            buf[0] = 0x1B;
            buf[1] = b'[';
            buf[2] = b'H';
            3
        }
        KEY_END => {
            buf[0] = 0x1B;
            buf[1] = b'[';
            buf[2] = b'F';
            3
        }
        _ => 0,
    }
}
