#![no_std]
#![no_main]

extern crate alloc;

use sunlight_ipc::{
    debug_log, ipc_call, nameserver_lookup, process_yield, shm_create, unpack_key_event,
    CapabilityToken, IpcMsg, PtyMsg, SgpMsg,
};
use sunlight_libc as libc;
use sunlight_tty::console::Console;
use sunlight_tui::{
    framebuffer::Framebuffer,
    font,
    ANSI_COLORS,
};

const WIN_W: u32 = 640;
const WIN_H: u32 = 360;
const TAB_H: u32 = 28;
const STATUS_H: u32 = 20;
const PAD_X: u32 = 8;
const PAD_Y: u32 = 6;
const CELL_W: u32 = 8;
const CELL_H: u32 = 18;
const BG: u32 = 0x00141418;
const PANEL: u32 = 0x001D1F25;
const PANEL_2: u32 = 0x00262630;
const TEXT_DIM: u32 = 0x009A948B;
const ACCENT: u32 = 0x00FF7A00;
const ACCENT_DIM: u32 = 0x00C85F00;
const READ_BUF: usize = 128;
const KEY_BACKSPACE: u8 = 0x0E;
const KEY_TAB: u8 = 0x0F;
const KEY_ENTER: u8 = 0x1C;

struct BumpAllocator;
unsafe impl core::alloc::GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        static mut HEAP: [u8; 2 * 1024 * 1024] = [0; 2 * 1024 * 1024];
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

#[global_allocator]
static ALLOC: BumpAllocator = BumpAllocator;

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    debug_log("[TERM] panic\n");
    loop {
        process_yield();
    }
}

struct PtySession {
    id: u64,
    master_cap: CapabilityToken,
    slave_cap: CapabilityToken,
}

impl PtySession {
    fn open() -> Result<Self, ()> {
        let Some(pty_cap) = nameserver_lookup("pty") else {
            return Err(());
        };
        let reply = ipc_call(pty_cap, IpcMsg::with_label(PtyMsg::CREATE));
        if reply.label != PtyMsg::REPLY || reply.cap_count < 2 {
            return Err(());
        }
        let session = Self {
            id: reply.words[0],
            master_cap: reply.caps[0],
            slave_cap: reply.caps[1],
        };
        // Raw mode: the shell handles line editing/prompting over the PTY.
        let _ = ipc_call(
            session.master_cap,
            IpcMsg::with_label(PtyMsg::SET_MODE)
                .word(0, session.id)
                .word(1, 0),
        );
        Ok(session)
    }

    fn write_input(&self, bytes: &[u8]) -> usize {
        let mut written = 0usize;
        while written < bytes.len() {
            let chunk = (bytes.len() - written).min(16);
            let mut msg = IpcMsg::with_label(PtyMsg::WRITE_MASTER)
                .word(0, self.id)
                .word(1, chunk as u64);
            for (word_idx, chunk_bytes) in bytes[written..written + chunk].chunks(8).enumerate() {
                let mut word = 0u64;
                for (byte_idx, &b) in chunk_bytes.iter().enumerate() {
                    word |= (b as u64) << (byte_idx * 8);
                }
                msg = msg.word(2 + word_idx, word);
            }
            let reply = ipc_call(self.master_cap, msg);
            if reply.label != PtyMsg::REPLY {
                break;
            }
            let accepted = (reply.words[1] as usize).min(chunk);
            if accepted == 0 {
                break;
            }
            written += accepted;
        }
        written
    }

    fn read_available(&self, out: &mut [u8]) -> usize {
        let mut total = 0usize;
        while total < out.len() {
            let chunk = (out.len() - total).min(16);
            let reply = ipc_call(
                self.master_cap,
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
                let word = reply.words[2 + (i / 8)];
                out[total + i] = ((word >> ((i % 8) * 8)) & 0xFF) as u8;
            }
            total += n;
            if n < chunk {
                break;
            }
        }
        total
    }

}

struct TerminalTab {
    title: [u8; 16],
    title_len: usize,
    pty: PtySession,
    console: Console,
}

fn fmt_u64_into(buf: &mut [u8], mut val: u64) -> usize {
    if val == 0 {
        if !buf.is_empty() {
            buf[0] = b'0';
        }
        return 1;
    }
    let mut tmp = [0u8; 20];
    let mut n = 0usize;
    while val > 0 && n < tmp.len() {
        tmp[n] = b'0' + (val % 10) as u8;
        val /= 10;
        n += 1;
    }
    for i in 0..n.min(buf.len()) {
        buf[i] = tmp[n - 1 - i];
    }
    n.min(buf.len())
}

fn make_prefixed_arg<'a>(prefix: &[u8], value: u64, buf: &'a mut [u8; 48]) -> &'a [u8] {
    let mut len = 0usize;
    for &b in prefix {
        buf[len] = b;
        len += 1;
    }
    len += fmt_u64_into(&mut buf[len..], value);
    &buf[..len]
}

fn spawn_shell(pty: &PtySession, shell_id: u64) -> Result<u64, ()> {
    let mut path_buf = [0u8; 32];
    let mut arg0 = [0u8; 16];
    let mut arg_session = [0u8; 48];
    let mut arg_cap = [0u8; 48];

    let mut path_len = 0usize;
    for &b in b"/bin/sshl" {
        path_buf[path_len] = b;
        path_len += 1;
    }
    path_len += fmt_u64_into(&mut path_buf[path_len..], shell_id);

    let mut arg0_len = 0usize;
    for &b in b"sshl" {
        arg0[arg0_len] = b;
        arg0_len += 1;
    }
    arg0_len += fmt_u64_into(&mut arg0[arg0_len..], shell_id);

    let arg_session = make_prefixed_arg(b"--pty-session=", pty.id, &mut arg_session);
    let arg_cap = make_prefixed_arg(b"--pty-cap=", pty.slave_cap.0, &mut arg_cap);
    let argv = [&arg0[..arg0_len], arg_session, arg_cap];

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

fn translate_key(packed: u64, out: &mut [u8; 4]) -> usize {
    let (keycode, pressed, _shift, ctrl, _alt, ascii) = unpack_key_event(packed);
    if !pressed {
        return 0;
    }
    if ctrl {
        if let Some(ch) = ascii {
            let lower = if ch.is_ascii_uppercase() { ch + 32 } else { ch };
            return match lower {
                b'c' | b'd' | b'l' => {
                    out[0] = lower & 0x1F;
                    1
                }
                _ => 0,
            };
        }
        return 0;
    }
    if let Some(ch) = ascii {
        out[0] = ch;
        return 1;
    }
    match keycode {
        KEY_ENTER => {
            out[0] = b'\n';
            1
        }
        KEY_BACKSPACE => {
            out[0] = 0x08;
            1
        }
        KEY_TAB => {
            out[0] = b'\t';
            1
        }
        _ => 0,
    }
}

unsafe fn render(buffer: *mut u32, tab: &mut TerminalTab) {
    let mut fb = Framebuffer::from_limine(buffer, WIN_W, WIN_H, WIN_W * 4);
    fb.fill_rect(0, 0, WIN_W, WIN_H, BG);
    fb.fill_rect(0, 0, WIN_W, TAB_H, PANEL);
    fb.hline(0, TAB_H, WIN_W, ACCENT_DIM);
    fb.fill_rect(0, WIN_H - STATUS_H, WIN_W, STATUS_H, PANEL);
    fb.hline(0, WIN_H - STATUS_H, WIN_W, ACCENT_DIM);

    let title = core::str::from_utf8(&tab.title[..tab.title_len]).unwrap_or("Tab 1");
    fb.fill_rect(8, 5, 60, TAB_H - 10, PANEL_2);
    font::draw_str(&mut fb, 14, 8, title, ACCENT, 1);
    fb.fill_rect(76, 5, 20, TAB_H - 10, PANEL_2);
    font::draw_str(&mut fb, 82, 8, "+", TEXT_DIM, 1);
    font::draw_str(&mut fb, 8, WIN_H - STATUS_H + 3, "PTY backend active", TEXT_DIM, 1);

    let content_y = TAB_H + PAD_Y;
    let rows = ((WIN_H - TAB_H - STATUS_H - PAD_Y * 2) / CELL_H) as usize;
    let cols = ((WIN_W - PAD_X * 2) / CELL_W) as usize;
    let cells = tab.console.to_term_cells(&ANSI_COLORS);

    for row in 0..rows {
        let y = content_y + row as u32 * CELL_H;
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

    let (cursor_row, cursor_col) = tab.console.cursor();
    if cursor_row < rows && cursor_col < cols {
        let idx = cursor_row * cols + cursor_col;
        let x = PAD_X + cursor_col as u32 * CELL_W;
        let y = content_y + cursor_row as u32 * CELL_H;
        let cells = tab.console.to_term_cells(&ANSI_COLORS);
        let cell = cells[idx];
        fb.fill_rect(x, y, CELL_W, CELL_H, ACCENT);
        if cell.ch >= 0x20 && cell.ch <= 0x7E && cell.ch != b' ' {
            font::draw_char(&mut fb, x, y, cell.ch, BG, 1);
        }
    }
}

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
        Ok(pty) => pty,
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

    let (cols, rows) = (
        ((WIN_W - PAD_X * 2) / CELL_W) as usize,
        ((WIN_H - TAB_H - STATUS_H - PAD_Y * 2) / CELL_H) as usize,
    );
    let mut tab = TerminalTab {
        title: *b"Tab 1\0\0\0\0\0\0\0\0\0\0\0",
        title_len: 5,
        pty,
        console: Console::new(cols, rows),
    };

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
        render(buffer, &mut tab);
    }
    let _ = ipc_call(display, IpcMsg::with_label(SgpMsg::COMMIT_FRAME).word(0, win_id));

    let mut read_buf = [0u8; READ_BUF];
    let mut key_buf = [0u8; 4];

    loop {
        let mut dirty = false;
        let poll = ipc_call(display, IpcMsg::with_label(SgpMsg::EVENT_POLL).word(0, win_id));
        if poll.label == SgpMsg::REPLY && poll.words[2] != 0 {
            let n = translate_key(poll.words[2], &mut key_buf);
            if n > 0 {
                let _ = tab.pty.write_input(&key_buf[..n]);
            }
        }

        let n = tab.pty.read_available(&mut read_buf);
        if n > 0 {
            tab.console.feed(&read_buf[..n]);
            dirty = true;
        }

        if dirty || tab.console.dirty {
            unsafe {
                render(buffer, &mut tab);
            }
            tab.console.clear_dirty();
            let _ = ipc_call(display, IpcMsg::with_label(SgpMsg::COMMIT_FRAME).word(0, win_id));
        }

        process_yield();
    }
}
