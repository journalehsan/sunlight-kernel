#![no_std]
#![no_main]

extern crate alloc;

use core::alloc::{GlobalAlloc, Layout};

use sunlight_ipc::{
    debug_log, ipc_call, ipc_call_timeout, nameserver_lookup, process_yield, shm_create,
    unpack_key_event, CapabilityToken, IpcCallError, IpcMsg, SgpMsg,
};
use sunlight_telemetry::{ProcessState, SystemSnapshot, Telemetry, MAX_PROCESSES};
use sunlight_tui::{font, framebuffer::Framebuffer};

const WIN_W: u32 = 720;
const WIN_H: u32 = 460;

const BG: u32 = 0x00131518;
const PANEL: u32 = 0x001B1E24;
const PANEL_2: u32 = 0x00232830;
const PANEL_3: u32 = 0x002D313B;
const SEP: u32 = 0x00FF7A00;
const TEXT: u32 = 0x00E4E0D8;
const TEXT_DIM: u32 = 0x009C9488;
const TEXT_MUTED: u32 = 0x007A746A;
const GOOD: u32 = 0x0058D68D;
const WARN: u32 = 0x00F4D35E;
const BAD: u32 = 0x00FF6B6B;
const SELECT_BG: u32 = 0x00323A46;
const BTN_BG: u32 = 0x00262A33;
const BTN_BG_HOVER: u32 = 0x00303946;
const BTN_BG_ACTIVE: u32 = 0x00FF7A00;
const BTN_TEXT_ACTIVE: u32 = 0x00110A02;
const STATUS_BG: u32 = 0x0015171C;
const EVENT_TIMEOUT_MS: u64 = 200;

const KEY_BACKSPACE: u8 = 0x0E;
const KEY_ENTER: u8 = 0x1C;
const KEY_UP: u8 = 0x48;
const KEY_DOWN: u8 = 0x50;
const KEY_LEFT: u8 = 0x4B;
const KEY_RIGHT: u8 = 0x4D;
const KEY_HOME: u8 = 0x47;
const KEY_END: u8 = 0x4F;
const KEY_PGUP: u8 = 0x49;
const KEY_PGDN: u8 = 0x51;

struct BumpAllocator;

unsafe impl GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        static mut HEAP: [u8; 3 * 1024 * 1024] = [0; 3 * 1024 * 1024];
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

    unsafe fn dealloc(&self, _: *mut u8, _: Layout) {}
}

#[global_allocator]
static ALLOC: BumpAllocator = BumpAllocator;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    debug_log("[TASKS] panic\n");
    loop {
        process_yield();
    }
}

#[derive(Clone, Copy)]
struct ButtonRect {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
}

impl ButtonRect {
    fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x as i32
            && x < (self.x + self.w) as i32
            && y >= self.y as i32
            && y < (self.y + self.h) as i32
    }
}

struct AppState {
    telemetry: Telemetry,
    snapshot: SystemSnapshot,
    selected_pid: Option<u32>,
    scroll: usize,
    show_system_info: bool,
    status: [u8; 128],
    status_len: usize,
    last_buttons: u8,
    mouse_x: u32,
    mouse_y: u32,
}

impl AppState {
    fn new(telemetry: Telemetry) -> Self {
        Self {
            telemetry,
            snapshot: SystemSnapshot::default(),
            selected_pid: None,
            scroll: 0,
            show_system_info: true,
            status: [0; 128],
            status_len: 0,
            last_buttons: 0,
            mouse_x: 0,
            mouse_y: 0,
        }
    }

    fn set_status(&mut self, msg: &str) {
        self.status = [0; 128];
        self.status_len = copy_ascii(msg.as_bytes(), &mut self.status);
    }

    fn refresh(&mut self, force: bool) -> bool {
        let changed = self.telemetry.poll();
        if changed || force {
            self.snapshot = *self.telemetry.snapshot();
            self.clamp_scroll(visible_rows(self));
        }
        changed || force
    }

    fn clamp_scroll(&mut self, visible: usize) {
        let max_scroll = self.snapshot.proc_count.saturating_sub(visible);
        if self.scroll > max_scroll {
            self.scroll = max_scroll;
        }
    }
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    debug_log("[TASKS] starting\n");

    let display = loop {
        if let Some(cap) = nameserver_lookup("display_server") {
            break cap;
        }
        process_yield();
    };

    let telemetry = match Telemetry::init() {
        Ok(t) => t,
        Err(_) => {
            debug_log("[TASKS] telemetry unavailable\n");
            sunlight_ipc::ProcessExit::exit(1);
        }
    };

    let (win_id, buffer) = match create_window(display) {
        Ok(v) => v,
        Err(_) => {
            debug_log("[TASKS] create_window failed\n");
            loop {
                process_yield();
            }
        }
    };
    set_window_title(display, win_id, b"Tasks Monitor");

    let mut app = AppState::new(telemetry);
    app.set_status("Telemetry ready");
    let _ = app.refresh(true);
    unsafe {
        render(buffer, &app);
    }
    let _ = ipc_call(display, IpcMsg::with_label(SgpMsg::COMMIT_FRAME).word(0, win_id));

    loop {
        let poll = ipc_call_timeout(
            display,
            IpcMsg::with_label(SgpMsg::EVENT_POLL).word(0, win_id),
            EVENT_TIMEOUT_MS,
        );

        let mut dirty = false;
        let mut refresh_now = false;

        match poll {
            Ok(reply) if reply.label == SgpMsg::REPLY => {
                let mouse_x = (reply.words[0] & 0xFFFF) as u32;
                let mouse_y = ((reply.words[0] >> 16) & 0xFFFF) as u32;
                let client_x = (reply.words[1] & 0xFFFF_FFFF) as u32;
                let client_y = (reply.words[1] >> 32) as u32;
                let buttons = (reply.words[3] & 0xFF) as u8;
                let key_word = reply.words[2];

                app.mouse_x = mouse_x.saturating_sub(client_x);
                app.mouse_y = mouse_y.saturating_sub(client_y);

                let click_x = app.mouse_x as i32;
                let click_y = app.mouse_y as i32;

                if key_word != 0 {
                    let _ = handle_key(&mut app, key_word);
                }

                if buttons & 1 != 0 && app.last_buttons & 1 == 0 {
                    let _ = handle_click(&mut app, click_x, click_y, &mut refresh_now);
                }

                app.last_buttons = buttons;
                dirty = true;
            }
            Ok(_) => {}
            Err(IpcCallError::Timeout) => {
                refresh_now = true;
            }
            Err(_) => {}
        }

        if refresh_now {
            let changed = app.refresh(true);
            dirty |= changed;
        } else if app.refresh(false) {
            dirty = true;
        }

        if dirty {
            unsafe {
                render(buffer, &app);
            }
            let _ = ipc_call(display, IpcMsg::with_label(SgpMsg::COMMIT_FRAME).word(0, win_id));
        }
    }
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

unsafe fn render(buffer: *mut u32, app: &AppState) {
    let mut fb = Framebuffer::from_limine(buffer, WIN_W, WIN_H, WIN_W * 4);
    fb.fill_rect(0, 0, WIN_W, WIN_H, BG);

    draw_title(&mut fb);
    let _buttons = draw_toolbar(&mut fb, app);
    draw_status_row(&mut fb, app);

    let mut y = 86u32;
    if app.show_system_info {
        y = draw_info_panel(&mut fb, app, y);
    }

    let table_top = y + 10;
    let table_bottom = WIN_H.saturating_sub(30);
    let visible = ((table_bottom.saturating_sub(table_top)) / 18) as usize;
    draw_table(&mut fb, app, table_top, visible);
    draw_footer(&mut fb, app);
}

fn draw_title(fb: &mut Framebuffer) {
    fb.fill_rect(0, 0, WIN_W, 32, PANEL);
    fb.hline(0, 32, WIN_W, SEP);
    font::draw_str(fb, 18, 10, "Tasks Monitor", TEXT, 1);
    font::draw_str(fb, WIN_W.saturating_sub(160), 10, "SunlightOS", TEXT_DIM, 1);
}

fn draw_toolbar(fb: &mut Framebuffer, app: &AppState) -> [ButtonRect; 4] {
    let labels: [&[u8]; 4] = [b"End Process", b"More Charts", b"System Info", b"Refresh"];
    let mut x = 16u32;
    let y = 42u32;
    let h = 28u32;
    let mut rects = [ButtonRect { x: 0, y, w: 0, h }; 4];
    for (i, label) in labels.iter().enumerate() {
        let text = core::str::from_utf8(label).unwrap_or("");
        let w = font::text_width(text, 1) + 18;
        let rect = ButtonRect { x, y, w, h };
        rects[i] = rect;
        let hovered = rect.contains(app.mouse_x as i32, app.mouse_y as i32);
        let active = matches!(i, 2) && app.show_system_info;
        let fill = if active {
            BTN_BG_ACTIVE
        } else if hovered {
            BTN_BG_HOVER
        } else {
            BTN_BG
        };
        fb.fill_rect(rect.x, rect.y, rect.w, rect.h, fill);
        fb.hline(rect.x, rect.y + rect.h, rect.w, SEP);
        fb.vline(rect.x, rect.y, rect.h, SEP);
        fb.vline(rect.x + rect.w - 1, rect.y, rect.h, SEP);

        let color = if active { BTN_TEXT_ACTIVE } else if hovered { TEXT } else { TEXT_DIM };
        font::draw_str(fb, rect.x + 9, rect.y + 7, text, color, 1);
        x += w + 10;
    }
    rects
}

fn draw_status_row(fb: &mut Framebuffer, app: &AppState) {
    fb.fill_rect(0, 74, WIN_W, 18, STATUS_BG);
    fb.hline(0, 74, WIN_W, PANEL_3);

    let mut status = "Ready";
    if app.status_len > 0 {
        if let Ok(s) = core::str::from_utf8(&app.status[..app.status_len]) {
            status = s;
        }
    }
    font::draw_str(fb, 16, 77, status, TEXT_DIM, 1);
}

fn draw_info_panel(fb: &mut Framebuffer, app: &AppState, y: u32) -> u32 {
    let h = 46u32;
    fb.fill_rect(12, y, WIN_W - 24, h, PANEL_2);
    fb.hline(12, y + h, WIN_W - 24, SEP);

    let snap = &app.snapshot;
    let cpu_pct = (snap.cpu_used_bp / 100).min(100) as u8;
    let ram_pct = if snap.total_ram_kb > 0 {
        ((snap.used_ram_kb.saturating_mul(100)) / snap.total_ram_kb).min(100) as u8
    } else {
        0
    };

    let mut x = 24u32;
    x = draw_kv_u64(fb, x, y + 11, "Uptime", snap.uptime_secs, TEXT, ValueFmt::Uptime);
    x = draw_kv_u64(fb, x, y + 11, "Tasks", snap.proc_count as u64, TEXT, ValueFmt::Plain);
    x = draw_kv_u8(fb, x, y + 11, "CPU", cpu_pct, GOOD, ValueFmt::Pct);
    x = draw_kv_u8(fb, x, y + 11, "RAM", ram_pct, WARN, ValueFmt::Pct);
    if let Some(pid) = app.selected_pid {
        let _ = draw_kv_u64(fb, x, y + 11, "Selected", pid as u64, TEXT, ValueFmt::Plain);
    }
    y + h
}

enum ValueFmt {
    Plain,
    Pct,
    Uptime,
}

fn draw_kv_u64(
    fb: &mut Framebuffer,
    x: u32,
    y: u32,
    key: &str,
    value: u64,
    value_color: u32,
    fmt: ValueFmt,
) -> u32 {
    font::draw_str(fb, x, y, key, TEXT_DIM, 1);
    let key_w = font::text_width(key, 1);
    let mut buf = [0u8; 32];
    let n = match fmt {
        ValueFmt::Plain => push_u64(&mut buf, value),
        ValueFmt::Pct => push_pct(&mut buf, value as u8),
        ValueFmt::Uptime => push_uptime(&mut buf, value),
    };
    let s = core::str::from_utf8(&buf[..n]).unwrap_or("-");
    font::draw_str(fb, x + key_w + 8, y, s, value_color, 1);
    x + key_w + 8 + font::text_width(s, 1) + 20
}

fn draw_kv_u8(
    fb: &mut Framebuffer,
    x: u32,
    y: u32,
    key: &str,
    value: u8,
    value_color: u32,
    fmt: ValueFmt,
) -> u32 {
    draw_kv_u64(fb, x, y, key, value as u64, value_color, fmt)
}

fn draw_table(fb: &mut Framebuffer, app: &AppState, top: u32, visible_rows: usize) {
    draw_table_header(fb, top);

    let order = sorted_order(&app.snapshot);
    let total = app.snapshot.proc_count.min(MAX_PROCESSES);
    let end = core::cmp::min(total, app.scroll + visible_rows);
    let mut row_y = top + 20;
    for visible_idx in app.scroll..end {
        let proc = &app.snapshot.procs[order[visible_idx]];
        let selected = app.selected_pid == Some(proc.pid);
        draw_task_row(fb, proc, row_y, selected);
        row_y += 18;
    }

    let table_end = top + 20 + (visible_rows as u32 * 18);
    fb.fill_rect(12, table_end, WIN_W - 24, 2, PANEL_3);
}

fn draw_table_header(fb: &mut Framebuffer, top: u32) {
    fb.fill_rect(12, top, WIN_W - 24, 20, PANEL);
    fb.hline(12, top + 20, WIN_W - 24, SEP);
    font::draw_str(fb, 22, top + 4, "PID", TEXT_DIM, 1);
    font::draw_str(fb, 78, top + 4, "Name", TEXT_DIM, 1);
    font::draw_str(fb, 332, top + 4, "State", TEXT_DIM, 1);
    font::draw_str(fb, 436, top + 4, "CPU%", TEXT_DIM, 1);
    font::draw_str(fb, 506, top + 4, "RAM", TEXT_DIM, 1);
    font::draw_str(fb, 612, top + 4, "Priority", TEXT_DIM, 1);
}

fn draw_task_row(fb: &mut Framebuffer, proc: &sunlight_telemetry::ProcessSnapshot, y: u32, selected: bool) {
    if selected {
        fb.fill_rect(14, y - 1, WIN_W - 28, 17, SELECT_BG);
    }

    let state = state_text(proc.state);

    let state_color = match proc.state {
        ProcessState::Running => GOOD,
        ProcessState::Blocked => WARN,
        ProcessState::Finished => TEXT_MUTED,
        ProcessState::Ready => TEXT,
    };
    let cpu_color = if proc.cpu_bp >= 7000 {
        BAD
    } else if proc.cpu_bp >= 3000 {
        WARN
    } else {
        GOOD
    };

    let mut pid_buf = [0u8; 12];
    let pid_len = push_u64(&mut pid_buf, proc.pid as u64);
    let pid = core::str::from_utf8(&pid_buf[..pid_len]).unwrap_or("-");
    let mut cpu_buf = [0u8; 8];
    let cpu_len = push_pct(&mut cpu_buf, (proc.cpu_bp / 100).min(100) as u8);
    let cpu = core::str::from_utf8(&cpu_buf[..cpu_len]).unwrap_or("-");
    let mut ram_buf = [0u8; 16];
    let ram_len = push_kib(&mut ram_buf, proc.mem_kb as u64);
    let ram = core::str::from_utf8(&ram_buf[..ram_len]).unwrap_or("-");

    font::draw_str(fb, 22, y + 3, pid, TEXT, 1);
    draw_truncated(fb, 78, y + 3, proc.name_str(), 30, TEXT);
    font::draw_str(fb, 332, y + 3, state, state_color, 1);
    font::draw_str(fb, 436, y + 3, cpu, cpu_color, 1);
    font::draw_str(fb, 506, y + 3, ram, TEXT, 1);
    font::draw_str(fb, 612, y + 3, "-", TEXT_MUTED, 1);
}

fn draw_footer(fb: &mut Framebuffer, app: &AppState) {
    let y = WIN_H - 24;
    fb.fill_rect(0, y, WIN_W, 24, STATUS_BG);
    fb.hline(0, y, WIN_W, SEP);

    let snap = &app.snapshot;
    let cpu_used = (snap.cpu_used_bp / 100).min(100);
    let cpu_idle = (snap.cpu_idle_bp / 100).min(100);
    let ram_mb_used = snap.used_ram_kb / 1024;
    let ram_mb_total = snap.total_ram_kb / 1024;

    let mut buf = [0u8; 128];
    let mut pos = 0usize;

    macro_rules! push_str {
        ($s:expr) => {{
            let b = $s.as_bytes();
            let n = b.len().min(buf.len() - pos);
            buf[pos..pos + n].copy_from_slice(&b[..n]);
            pos += n;
        }};
    }
    macro_rules! push_num {
        ($v:expr) => {{
            let mut tmp = [0u8; 20];
            let n = push_u64(&mut tmp, $v as u64);
            let n = n.min(buf.len() - pos);
            buf[pos..pos + n].copy_from_slice(&tmp[..n]);
            pos += n;
        }};
    }

    push_str!("CPU: ");
    push_num!(cpu_used);
    push_str!("% used  idle: ");
    push_num!(cpu_idle);
    push_str!("%   |   RAM: ");
    push_num!(ram_mb_used);
    push_str!(" / ");
    push_num!(ram_mb_total);
    push_str!(" MB   |   Kernel: ");

    // sum mem of first 5 pids (kernel services) as a rough "kernel footprint"
    let mut kernel_kb: u64 = 0;
    let count = snap.proc_count.min(5);
    for i in 0..count {
        kernel_kb += snap.procs[i].mem_kb as u64;
    }
    push_num!(kernel_kb / 1024);
    push_str!(" MB   |   Tasks: ");
    push_num!(snap.proc_count as u64);

    if let Ok(s) = core::str::from_utf8(&buf[..pos]) {
        font::draw_str(fb, 16, y + 6, s, TEXT_DIM, 1);
    }
}

fn handle_click(app: &mut AppState, x: i32, y: i32, refresh_now: &mut bool) -> bool {
    let buttons = draw_toolbar_layout();
    if buttons[3].contains(x, y) {
        *refresh_now = true;
        app.set_status("Refreshing telemetry");
        return true;
    }
    if buttons[2].contains(x, y) {
        app.show_system_info = !app.show_system_info;
        app.set_status(if app.show_system_info {
            "System info enabled"
        } else {
            "System info hidden"
        });
        return true;
    }
    if buttons[1].contains(x, y) {
        app.set_status("More Charts is a v0.1 placeholder");
        return true;
    }
    if buttons[0].contains(x, y) {
        if app.selected_pid.is_some() {
            app.set_status("End Process is not implemented yet");
        } else {
            app.set_status("Select a process first");
        }
        return true;
    }

    let row_top = table_top(app);
    let row_h = 18i32;
    let visible = visible_rows(app);
    if y >= row_top as i32 && y < (row_top as i32 + (visible as i32 * row_h)) {
        let row_idx = ((y - row_top as i32) / row_h) as usize + app.scroll;
        let order = sorted_order(&app.snapshot);
        if row_idx < app.snapshot.proc_count.min(MAX_PROCESSES) {
            let proc = &app.snapshot.procs[order[row_idx]];
            app.selected_pid = Some(proc.pid);
            app.set_status("Process selected");
            return true;
        }
    }
    false
}

fn handle_key(app: &mut AppState, key_word: u64) -> bool {
    let (keycode, pressed, _shift, ctrl, _alt, ascii) = unpack_key_event(key_word);
    if !pressed {
        return false;
    }

    if ctrl {
        if let Some(ch) = ascii {
            let lower = if ch.is_ascii_uppercase() { ch + 32 } else { ch };
            if lower == b'r' {
                app.set_status("Refreshing telemetry");
                return true;
            }
        }
    }

    match keycode {
        KEY_UP => {
            app.scroll = app.scroll.saturating_sub(1);
            true
        }
        KEY_DOWN => {
            app.scroll = app.scroll.saturating_add(1);
            app.clamp_scroll(visible_rows(app));
            true
        }
        KEY_PGUP => {
            app.scroll = app.scroll.saturating_sub(visible_rows(app).max(1) / 2);
            true
        }
        KEY_PGDN => {
            app.scroll = app.scroll.saturating_add(visible_rows(app).max(1) / 2);
            app.clamp_scroll(visible_rows(app));
            true
        }
        KEY_HOME => {
            app.scroll = 0;
            true
        }
        KEY_END => {
            app.scroll = app.snapshot.proc_count.saturating_sub(visible_rows(app));
            true
        }
        KEY_ENTER => {
            app.set_status("End Process is not implemented yet");
            true
        }
        KEY_LEFT | KEY_RIGHT | KEY_BACKSPACE => false,
        _ => false,
    }
}

fn draw_toolbar_layout() -> [ButtonRect; 4] {
    let labels: [&[u8]; 4] = [b"End Process", b"More Charts", b"System Info", b"Refresh"];
    let mut x = 16u32;
    let y = 42u32;
    let h = 28u32;
    let mut rects = [ButtonRect { x: 0, y, w: 0, h }; 4];
    for (i, label) in labels.iter().enumerate() {
        let text = core::str::from_utf8(label).unwrap_or("");
        let w = font::text_width(text, 1) + 18;
        rects[i] = ButtonRect { x, y, w, h };
        x += w + 10;
    }
    rects
}

fn visible_rows(app: &AppState) -> usize {
    let table_top = table_top(app);
    let max_rows = (WIN_H.saturating_sub(30).saturating_sub(table_top + 20)) / 18;
    max_rows as usize
}

fn table_top(app: &AppState) -> u32 {
    if app.show_system_info {
        140
    } else {
        94
    }
}

fn sorted_order(snapshot: &SystemSnapshot) -> [usize; MAX_PROCESSES] {
    let mut order = [0usize; MAX_PROCESSES];
    let mut len = 0usize;
    for i in 0..snapshot.proc_count.min(MAX_PROCESSES) {
        order[len] = i;
        len += 1;
    }
    for i in 1..len {
        let key = order[i];
        let mut j = i;
        while j > 0 {
            let a = &snapshot.procs[key];
            let b = &snapshot.procs[order[j - 1]];
            let before = if a.cpu_bp != b.cpu_bp {
                a.cpu_bp > b.cpu_bp
            } else {
                a.pid < b.pid
            };
            if before {
                order[j] = order[j - 1];
                j -= 1;
            } else {
                break;
            }
        }
        order[j] = key;
    }
    order
}

fn state_text(state: ProcessState) -> &'static str {
    match state {
        ProcessState::Running => "running",
        ProcessState::Blocked => "blocked",
        ProcessState::Finished => "finished",
        ProcessState::Ready => "ready",
    }
}


fn copy_ascii(src: &[u8], dst: &mut [u8]) -> usize {
    let n = src.len().min(dst.len().saturating_sub(1));
    dst[..n].copy_from_slice(&src[..n]);
    dst[n] = 0;
    n
}

fn push_u64(buf: &mut [u8], mut v: u64) -> usize {
    if v == 0 {
        buf[0] = b'0';
        return 1;
    }

    let mut rev = [0u8; 20];
    let mut n = 0usize;
    while v > 0 && n < rev.len() {
        rev[n] = b'0' + (v % 10) as u8;
        n += 1;
        v /= 10;
    }

    for i in 0..n {
        buf[i] = rev[n - 1 - i];
    }
    n
}

fn push_pct(buf: &mut [u8], pct: u8) -> usize {
    let pct = pct.min(100);
    let mut pos = push_u64(buf, pct as u64);
    if pos < buf.len() {
        buf[pos] = b'%';
        pos += 1;
    }
    pos
}

fn push_uptime(buf: &mut [u8], secs: u64) -> usize {
    let d = secs / 86_400;
    let rem = secs % 86_400;
    let h = rem / 3600;
    let m = (rem % 3600) / 60;
    let s = rem % 60;
    let mut pos = 0usize;

    if d > 0 {
        pos += push_u64(&mut buf[pos..], d);
        buf[pos] = b'd';
        pos += 1;
        buf[pos] = b' ';
        pos += 1;
    }

    if h < 10 {
        buf[pos] = b'0';
        pos += 1;
    }
    pos += push_u64(&mut buf[pos..], h);
    buf[pos] = b':';
    pos += 1;
    if m < 10 {
        buf[pos] = b'0';
        pos += 1;
    }
    pos += push_u64(&mut buf[pos..], m);
    buf[pos] = b':';
    pos += 1;
    if s < 10 {
        buf[pos] = b'0';
        pos += 1;
    }
    pos += push_u64(&mut buf[pos..], s);
    pos
}

fn push_kib(buf: &mut [u8], kib: u64) -> usize {
    let mut pos = 0usize;
    if kib >= 1024 * 1024 {
        pos += push_u64(&mut buf[pos..], kib / (1024 * 1024));
        buf[pos] = b'G';
        pos += 1;
    } else if kib >= 1024 {
        pos += push_u64(&mut buf[pos..], kib / 1024);
        buf[pos] = b'M';
        pos += 1;
    } else {
        pos += push_u64(&mut buf[pos..], kib);
        buf[pos] = b'K';
        pos += 1;
    }
    pos
}

fn draw_truncated(fb: &mut Framebuffer, x: u32, y: u32, text: &str, max_chars: usize, color: u32) {
    let truncated = truncate_chars(text, max_chars);
    font::draw_str(fb, x, y, truncated, color, 1);
}

fn truncate_chars(text: &str, max_chars: usize) -> &str {
    let mut end = text.len();
    let mut count = 0usize;
    for (idx, ch) in text.char_indices() {
        if count == max_chars {
            end = idx;
            break;
        }
        count += 1;
        end = idx + ch.len_utf8();
    }
    &text[..end]
}
