#![no_std]
#![no_main]

use sun_font::{self, FontRole, VecFont};
use sunlight_ipc::{
    debug_log, ipc_call,
    launch_trace::{self, LaunchSource, LaunchTrace},
    nameserver_lookup, process_yield, CapabilityToken, IpcMsg, PtyMsg,
};
use sunlight_libc as libc;
use sunlight_tty::TerminalGrid as ModelGrid;
use sunlight_ui::{
    widgets::{Label, Panel, StatusBar},
    App, Canvas, Event, HBox, Rect, Window, WindowConfig,
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

const INPUT_MAX: usize = 240;
const PROMPT_MAX: usize = 64;
const APP_NAME_MAX: usize = 32;
const HIST_MAX: usize = 32;
const READ_BUF: usize = 256;
const ANSI_COLORS: [u32; 16] = [
    0xFF000000, 0xFFCC241D, 0xFF98971A, 0xFFD79921, 0xFF458588, 0xFFB16286, 0xFF689D6A, 0xFFA89984,
    0xFF928374, 0xFFFB4934, 0xFFB8BB26, 0xFFFABD2F, 0xFF83A598, 0xFFD3869B, 0xFF8EC07C, 0xFFEBDBB2,
];

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
        ipc_call(
            cap,
            IpcMsg::with_label(PtyMsg::SET_MODE)
                .word(0, reply.words[0])
                .word(1, 0),
        );
        Ok(Self {
            id: reply.words[0],
            cap,
        })
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
            let reply = ipc_call(self.cap, msg);
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
            let reply = ipc_call(
                self.cap,
                IpcMsg::with_label(PtyMsg::READ_MASTER)
                    .word(0, self.id)
                    .word(1, chunk as u64),
            );
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
        core::str::from_utf8(&self.prompt[..self.prompt_len]).unwrap_or("$ ")
    }

    fn input_str(&self) -> &str {
        core::str::from_utf8(&self.input[..self.input_len]).unwrap_or("")
    }

    fn app_name_str(&self) -> &str {
        core::str::from_utf8(&self.app_name[..self.app_name_len]).unwrap_or("app")
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

struct TerminalApp {
    pty: PtySession,
    grid: ModelGrid,
    footer: Footer,
    osc: OscParser,
    read_buf: [u8; READ_BUF],
    console_buf: [u8; READ_BUF],
    debug: DebugFlags,
}

impl TerminalApp {
    fn new(pty: PtySession, debug: DebugFlags) -> Self {
        Self {
            pty,
            grid: ModelGrid::new(CONTENT_COLS, CONTENT_ROWS),
            footer: Footer::new(),
            osc: OscParser::new(),
            read_buf: [0; READ_BUF],
            console_buf: [0; READ_BUF],
            debug,
        }
    }

    fn content_rect(&self) -> Rect {
        Rect::new(
            PAD_X as i32,
            TAB_H as i32 + PAD_Y as i32,
            WIN_W - PAD_X * 2,
            WIN_H - TAB_H - FOOTER_H - PAD_Y * 2,
        )
    }

    fn footer_rect(&self) -> Rect {
        Rect::new(0, WIN_H as i32 - FOOTER_H as i32, WIN_W, FOOTER_H)
    }

    fn app_owns_input(&self) -> bool {
        self.footer.app_mode || self.grid.in_alt_screen()
    }

    fn shell_spawn(&self) {
        let shell_id = (libc::getpid() as u8).max(1) as u64;
        let _ = spawn_shell(&self.pty, shell_id);
    }

    fn poll_pty(&mut self) -> bool {
        let n = self.pty.read(&mut self.read_buf);
        if n == 0 {
            return false;
        }
        if self.debug.log_pty_stream {
            log_pty_bytes(&self.read_buf[..n]);
        }
        let mut console_len = 0usize;
        self.osc.feed(
            &self.read_buf[..n],
            &mut self.console_buf,
            &mut console_len,
            |body| match parse_osc(body) {
                OscCmd::Prompt(text) => self.footer.set_prompt(text),
                OscCmd::AppStart(name) => self.footer.enter_app_mode(name),
                OscCmd::AppDone => self.footer.exit_app_mode(),
                OscCmd::Unknown => {}
            },
        );
        if console_len > 0 {
            self.grid.feed(&self.console_buf[..console_len]);
        }
        true
    }

    fn handle_raw_key(&mut self, keycode: u8, pressed: bool) -> bool {
        if !pressed {
            return false;
        }
        if self.app_owns_input() {
            let mut buf = [0u8; 4];
            let n = translate_special_key(keycode, &mut buf);
            if n > 0 {
                self.pty.write(&buf[..n]);
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

    fn submit_line(&mut self) {
        let (line, len) = self.footer.take_line();
        if len > 0 {
            self.pty.write(&line[..len]);
        }
        self.pty.write(b"\n");
    }
}

impl App for TerminalApp {
    fn view(&mut self, canvas: &mut Canvas, theme: &sunlight_ui::Theme) {
        canvas.fill_rect(Rect::new(0, 0, WIN_W, WIN_H), theme.bg);
        Panel::new(Rect::new(0, 0, WIN_W, TAB_H)).draw(canvas, theme);
        Label::new(Rect::new(14, 8, 80, TAB_H - 8), "Tab 1")
            .with_font(&F_UI)
            .draw(canvas, theme);

        TerminalViewport::new(self.content_rect()).draw(canvas, &mut self.grid, theme);

        let footer = self.footer_rect();
        StatusBar::new(footer, "", "", "").draw(canvas, theme);
        if self.app_owns_input() {
            Label::new(
                Rect::new(8, footer.y + 4, 220, FOOTER_H - 8),
                self.footer.app_name_str(),
            )
            .with_font(&F_SMALL)
            .draw(canvas, theme);
        } else {
            let prompt_w =
                sun_font::measure_text(self.footer.prompt_str(), FontRole::UiSmall).w + 4;
            let prompt_widths = [prompt_w, WIN_W - 48];
            let mut prompt_cells = HBox::new(Rect::new(8, footer.y + 4, WIN_W - 16, FOOTER_H - 8))
                .with_spacing(8)
                .layout(&prompt_widths);
            if let Some(prompt_rect) = prompt_cells.next() {
                Label::new(prompt_rect, self.footer.prompt_str())
                    .with_font(&F_SMALL)
                    .draw(canvas, theme);
            }
            if let Some(input_rect) = prompt_cells.next() {
                Label::new(input_rect, self.footer.input_str())
                    .with_font(&F_UI)
                    .draw(canvas, theme);
            }
        }
    }

    fn update(&mut self, event: Event) -> bool {
        let mut dirty = false;
        match event {
            Event::Tick => {
                dirty |= self.poll_pty();
            }
            Event::Key(ch) => {
                if self.app_owns_input() {
                    let byte = match ch {
                        '\n' => b'\n',
                        '\u{8}' => 0x08,
                        c if c.is_ascii() => c as u8,
                        _ => 0,
                    };
                    if byte != 0 {
                        self.pty.write(&[byte]);
                        dirty = true;
                    }
                } else if ch == '\n' {
                    self.submit_line();
                    dirty = true;
                } else if ch == '\u{8}' {
                    self.footer.backspace();
                    dirty = true;
                } else if ch.is_ascii_graphic() || ch == ' ' {
                    self.footer.insert(ch as u8);
                    dirty = true;
                }
            }
            Event::KeyPress {
                keycode, pressed, ..
            } => {
                dirty |= self.handle_raw_key(keycode, pressed);
            }
            Event::Click { .. }
            | Event::MouseDown { .. }
            | Event::MouseUp { .. }
            | Event::MouseMove { .. } => {}
        }
        dirty
    }
}

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
    let pty = match PtySession::open() {
        Ok(pty) => pty,
        Err(_) => loop {
            process_yield();
        },
    };

    let mut app = TerminalApp::new(pty, parse_debug_flags(argc, argv));
    app.shell_spawn();

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
    loop {
        process_yield();
    }
}

fn translate_special_key(keycode: u8, buf: &mut [u8; 4]) -> usize {
    match keycode {
        KEY_ENTER => {
            buf[0] = b'\n';
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
