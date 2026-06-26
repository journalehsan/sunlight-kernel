#![no_std]
#![no_main]

use sunlight_ipc::{debug_log, process_yield, ProcessExit};
use sunlight_telemetry::{ProcessState, SystemSnapshot, Telemetry, MAX_PROCESSES};
use sunlight_ui::{
    request_close,
    widgets::{Button, ButtonState, Column, Label, Panel, StatusBar, Table},
    App, Event, HBox, Rect, Window, WindowConfig,
};

const WIN_W: u32 = 720;
const WIN_H: u32 = 460;
const STATUS_H: u32 = 18;
const TITLE_H: u32 = 28;
const TOOLBAR_H: u32 = 28;
const INFO_H: u32 = 60;
const CONTENT_MARGIN: i32 = 12;
const TABLE_COLS: usize = 5;
const CELL_BUF: usize = 32;

const KEY_BACKSPACE: u8 = 0x0E;
const KEY_ENTER: u8 = 0x1C;
const KEY_UP: u8 = 0x48;
const KEY_DOWN: u8 = 0x50;
const KEY_HOME: u8 = 0x47;
const KEY_END: u8 = 0x4F;
const KEY_PGUP: u8 = 0x49;
const KEY_PGDN: u8 = 0x51;
const KEY_Q: u8 = 0x10;

struct NoAlloc;

unsafe impl core::alloc::GlobalAlloc for NoAlloc {
    unsafe fn alloc(&self, _layout: core::alloc::Layout) -> *mut u8 {
        core::ptr::null_mut()
    }
    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: core::alloc::Layout) {}
}

#[global_allocator]
static ALLOC: NoAlloc = NoAlloc;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    debug_log("[TASKS] panic\n");
    loop {
        process_yield();
    }
}

const TABLE_COLUMNS: [Column<'static>; TABLE_COLS] = [
    Column {
        header: "PID",
        width: 70,
        right_align: true,
    },
    Column {
        header: "Name",
        width: 260,
        right_align: false,
    },
    Column {
        header: "State",
        width: 120,
        right_align: false,
    },
    Column {
        header: "CPU%",
        width: 90,
        right_align: true,
    },
    Column {
        header: "RAM",
        width: 120,
        right_align: true,
    },
];

const EMPTY_ROW: [&str; TABLE_COLS] = ["", "", "", "", ""];

struct TasksApp {
    telemetry: Telemetry,
    snapshot: SystemSnapshot,
    status: [u8; 96],
    status_len: usize,
    hw_info: [u8; 40],
    hw_info_len: usize,
    selected_pid: Option<u32>,
    scroll: usize,
    show_system_info: bool,
    order: [usize; MAX_PROCESSES],
    row_bufs: [[[u8; CELL_BUF]; TABLE_COLS]; MAX_PROCESSES],
    row_lens: [[usize; TABLE_COLS]; MAX_PROCESSES],
    row_count: usize,
}

impl TasksApp {
    fn new(telemetry: Telemetry) -> Self {
        let mut app = Self {
            telemetry,
            snapshot: SystemSnapshot::default(),
            status: [0; 96],
            status_len: 0,
            hw_info: [0; 40],
            hw_info_len: 0,
            selected_pid: None,
            scroll: 0,
            show_system_info: true,
            order: [0; MAX_PROCESSES],
            row_bufs: [[[0; CELL_BUF]; TABLE_COLS]; MAX_PROCESSES],
            row_lens: [[0; TABLE_COLS]; MAX_PROCESSES],
            row_count: 0,
        };
        app.set_status("Telemetry ready");
        app.refresh(true);
        app
    }

    fn set_status(&mut self, text: &str) {
        let bytes = text.as_bytes();
        self.status_len = bytes.len().min(self.status.len());
        self.status[..self.status_len].copy_from_slice(&bytes[..self.status_len]);
    }

    fn status_str(&self) -> &str {
        core::str::from_utf8(&self.status[..self.status_len]).unwrap_or("")
    }

    fn rebuild_hw_info(&mut self) {
        let mut n = copy_tail(b"Cores: ", &mut self.hw_info);
        n += write_num_into(self.snapshot.cpu_count as u32, &mut self.hw_info[n..]);
        n += copy_tail(b"  GPU: ", &mut self.hw_info[n..]);
        n += write_num_into(self.snapshot.gpu_count as u32, &mut self.hw_info[n..]);
        self.hw_info_len = n;
    }

    fn hw_info_str(&self) -> &str {
        core::str::from_utf8(&self.hw_info[..self.hw_info_len]).unwrap_or("")
    }

    fn refresh(&mut self, force: bool) -> bool {
        let changed = self.telemetry.poll();
        if changed || force {
            self.snapshot = *self.telemetry.snapshot();
            self.rebuild_rows();
            self.rebuild_hw_info();
            self.clamp_scroll();
            return true;
        }
        false
    }

    fn rebuild_rows(&mut self) {
        self.row_count = self.snapshot.proc_count.min(MAX_PROCESSES);
        for i in 0..self.row_count {
            self.order[i] = i;
        }
        for i in 1..self.row_count {
            let key = self.order[i];
            let mut j = i;
            while j > 0 && self.before(key, self.order[j - 1]) {
                self.order[j] = self.order[j - 1];
                j -= 1;
            }
            self.order[j] = key;
        }

        for row in 0..self.row_count {
            let proc = &self.snapshot.procs[self.order[row]];
            write_u32(
                proc.pid,
                &mut self.row_bufs[row][0],
                &mut self.row_lens[row][0],
            );
            write_str(
                proc.name_str(),
                &mut self.row_bufs[row][1],
                &mut self.row_lens[row][1],
            );
            write_str(
                state_text(proc.state),
                &mut self.row_bufs[row][2],
                &mut self.row_lens[row][2],
            );
            write_pct(
                (proc.cpu_bp / 100).min(100) as u32,
                &mut self.row_bufs[row][3],
                &mut self.row_lens[row][3],
            );
            write_kib(
                proc.mem_kb,
                &mut self.row_bufs[row][4],
                &mut self.row_lens[row][4],
            );
        }
    }

    fn before(&self, a: usize, b: usize) -> bool {
        let left = &self.snapshot.procs[a];
        let right = &self.snapshot.procs[b];
        if left.cpu_bp != right.cpu_bp {
            left.cpu_bp > right.cpu_bp
        } else {
            left.pid < right.pid
        }
    }

    fn clamp_scroll(&mut self) {
        let visible = self.visible_rows();
        let max_scroll = self.row_count.saturating_sub(visible);
        if self.scroll > max_scroll {
            self.scroll = max_scroll;
        }
    }

    fn visible_rows(&self) -> usize {
        let content = self.content_rect();
        let info_h = if self.show_system_info { INFO_H + 8 } else { 0 };
        let usable = content
            .h
            .saturating_sub(TITLE_H + TOOLBAR_H + info_h + STATUS_H + 18);
        (usable / 16).max(1) as usize
    }

    fn content_rect(&self) -> Rect {
        Rect::new(
            CONTENT_MARGIN,
            CONTENT_MARGIN,
            WIN_W.saturating_sub((CONTENT_MARGIN as u32) * 2),
            WIN_H.saturating_sub((CONTENT_MARGIN as u32) * 2),
        )
    }

    fn toolbar_rect(&self) -> Rect {
        let content = self.content_rect();
        Rect::new(
            content.x,
            content.y + TITLE_H as i32 + 8,
            content.w,
            TOOLBAR_H,
        )
    }

    fn table_rect(&self) -> Rect {
        let content = self.content_rect();
        let top = content.y
            + TITLE_H as i32
            + 8
            + TOOLBAR_H as i32
            + 8
            + if self.show_system_info {
                INFO_H as i32 + 8
            } else {
                0
            };
        let status_h = STATUS_H + 8;
        Rect::new(
            content.x,
            top,
            content.w,
            content.bottom().saturating_sub(top + status_h as i32) as u32,
        )
    }

    fn status_bar_rect(&self) -> Rect {
        let content = self.content_rect();
        Rect::new(
            content.x,
            content.bottom() - STATUS_H as i32,
            content.w,
            STATUS_H,
        )
    }

    fn toolbar_buttons(&self) -> [Rect; 4] {
        let widths = [120, 120, 120, 120];
        let mut rects = [Rect::default(); 4];
        for (idx, rect) in HBox::new(self.toolbar_rect())
            .with_spacing(8)
            .layout(&widths)
            .enumerate()
        {
            rects[idx] = rect;
        }
        rects
    }

    fn info_strings(
        &self,
        uptime: &mut [u8; 24],
        tasks: &mut [u8; 24],
        cpu: &mut [u8; 40],
        ram: &mut [u8; 40],
    ) {
        let mut n = copy_tail(b"Uptime ", uptime);
        n += write_num_into((self.snapshot.uptime_secs / 3600) as u32, &mut uptime[n..]);
        n += copy_tail(b"h ", &mut uptime[n..]);
        n += write_num_into(
            ((self.snapshot.uptime_secs % 3600) / 60) as u32,
            &mut uptime[n..],
        );
        copy_tail(b"m", &mut uptime[n..]);

        let n = copy_tail(b"Tasks ", tasks);
        write_num_into(self.snapshot.proc_count as u32, &mut tasks[n..]);

        let mut n = copy_tail(b"CPU ", cpu);
        n += write_bp_into(self.snapshot.cpu_used_bp, &mut cpu[n..]);
        n += copy_tail(b" used ", &mut cpu[n..]);
        n += write_bp_into(self.snapshot.cpu_idle_bp, &mut cpu[n..]);
        copy_tail(b" idle", &mut cpu[n..]);

        let mut n = copy_tail(b"RAM ", ram);
        n += write_mb_into(self.snapshot.used_ram_kb, &mut ram[n..]);
        n += copy_tail(b" / ", &mut ram[n..]);
        copy_tail_mb(self.snapshot.total_ram_kb, &mut ram[n..]);
    }
}

impl App for TasksApp {
    fn view(&mut self, canvas: &mut sunlight_ui::Canvas, theme: &sunlight_ui::Theme) {
        canvas.fill_rect(Rect::new(0, 0, WIN_W, WIN_H), theme.bg);

        let content = self.content_rect();
        Panel::new(content).draw(canvas, theme);

        let title_rect = Rect::new(content.x + 12, content.y + 8, 220, TITLE_H);
        Label::new(title_rect, "Tasks Monitor").draw(canvas, theme);
        Label::new(
            Rect::new(content.right() - 88, content.y + 8, 76, TITLE_H),
            "SunlightOS",
        )
        .dim()
        .draw(canvas, theme);

        let buttons = self.toolbar_buttons();
        let labels = ["End", "Charts", "Info", "Refresh"];
        for (idx, rect) in buttons.iter().enumerate() {
            let mut button = if idx == 2 && self.show_system_info {
                Button::new(*rect, labels[idx])
            } else {
                Button::secondary(*rect, labels[idx])
            };
            button.state = ButtonState::Normal;
            button.draw(canvas, theme);
        }

        if self.show_system_info {
            let info_rect = Rect::new(
                content.x,
                self.toolbar_rect().bottom() + 8,
                content.w,
                INFO_H,
            );
            Panel::with_title(info_rect, "System").draw(canvas, theme);

            let row1 = Rect::new(
                info_rect.x + 12,
                info_rect.y + 22,
                info_rect.w.saturating_sub(24),
                14,
            );
            let row2 = Rect::new(
                info_rect.x + 12,
                info_rect.y + 38,
                info_rect.w.saturating_sub(24),
                14,
            );
            let mut top_cells = HBox::new(row1).with_spacing(12).layout(&[150, 120]);
            let mut bottom_cells = HBox::new(row2).with_spacing(12).layout(&[280, 220]);
            let mut uptime = [0u8; 24];
            let mut tasks = [0u8; 24];
            let mut cpu = [0u8; 40];
            let mut ram = [0u8; 40];
            self.info_strings(&mut uptime, &mut tasks, &mut cpu, &mut ram);
            let top_labels = [
                core::str::from_utf8(trim_zeros(&uptime)).unwrap_or(""),
                core::str::from_utf8(trim_zeros(&tasks)).unwrap_or(""),
            ];
            let bottom_labels = [
                core::str::from_utf8(trim_zeros(&cpu)).unwrap_or(""),
                core::str::from_utf8(trim_zeros(&ram)).unwrap_or(""),
            ];
            for label in top_labels.iter() {
                if let Some(rect) = top_cells.next() {
                    Label::new(rect, label).draw(canvas, theme);
                }
            }
            for label in bottom_labels.iter() {
                if let Some(rect) = bottom_cells.next() {
                    Label::new(rect, label).draw(canvas, theme);
                }
            }
        }

        let mut row_cells = [EMPTY_ROW; MAX_PROCESSES];
        let mut row_refs: [&[&str]; MAX_PROCESSES] = [&[]; MAX_PROCESSES];
        for row in 0..self.row_count {
            for col in 0..TABLE_COLS {
                row_cells[row][col] =
                    core::str::from_utf8(&self.row_bufs[row][col][..self.row_lens[row][col]])
                        .unwrap_or("");
            }
        }
        for row in 0..self.row_count {
            row_refs[row] = &row_cells[row];
        }

        let table_rect = self.table_rect();
        let visible = self.visible_rows();
        let end = (self.scroll + visible).min(self.row_count);
        let selected = self.selected_pid.and_then(|pid| {
            (self.scroll..end).position(|idx| {
                let proc = &self.snapshot.procs[self.order[idx]];
                proc.pid == pid
            })
        });
        let table = Table {
            rect: table_rect,
            columns: &TABLE_COLUMNS,
            rows: &row_refs[self.scroll..end],
            selected,
            header_h: 18,
            row_h: 16,
        };
        table.draw(canvas, theme);

        StatusBar::new(self.status_bar_rect(), self.status_str(), "", self.hw_info_str()).draw(canvas, theme);
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::Tick => self.refresh(false),
            Event::Click { x, y } => {
                let buttons = self.toolbar_buttons();
                for (idx, rect) in buttons.iter().enumerate() {
                    if rect.contains(sunlight_ui::Point::new(x, y)) {
                        match idx {
                            0 => self.set_status(if self.selected_pid.is_some() {
                                "End process not implemented"
                            } else {
                                "Select a process first"
                            }),
                            1 => self.set_status("Charts view is not implemented yet"),
                            2 => {
                                self.show_system_info = !self.show_system_info;
                                self.set_status(if self.show_system_info {
                                    "System info shown"
                                } else {
                                    "System info hidden"
                                });
                            }
                            3 => {
                                self.set_status("Refreshing telemetry");
                                let _ = self.refresh(true);
                            }
                            _ => {}
                        }
                        return true;
                    }
                }

                let table = Table {
                    rect: self.table_rect(),
                    columns: &TABLE_COLUMNS,
                    rows: &[],
                    selected: None,
                    header_h: 18,
                    row_h: 16,
                };
                if let Some(row) = table.hit_test(x, y) {
                    let idx = self.scroll + row;
                    if idx < self.row_count {
                        let proc = &self.snapshot.procs[self.order[idx]];
                        self.selected_pid = Some(proc.pid);
                        self.set_status("Process selected");
                        return true;
                    }
                }
                false
            }
            Event::KeyPress {
                keycode,
                pressed: true,
                ctrl,
                ..
            } => {
                if ctrl && keycode == KEY_Q {
                    request_close();
                    return true;
                }
                match keycode {
                    KEY_UP => self.scroll = self.scroll.saturating_sub(1),
                    KEY_DOWN => self.scroll = self.scroll.saturating_add(1),
                    KEY_PGUP => {
                        self.scroll = self.scroll.saturating_sub(self.visible_rows().max(1) / 2)
                    }
                    KEY_PGDN => {
                        self.scroll = self.scroll.saturating_add(self.visible_rows().max(1) / 2)
                    }
                    KEY_HOME => self.scroll = 0,
                    KEY_END => self.scroll = self.row_count.saturating_sub(self.visible_rows()),
                    KEY_ENTER => self.set_status("End process not implemented"),
                    KEY_BACKSPACE => return false,
                    _ => return false,
                }
                self.clamp_scroll();
                true
            }
            _ => false,
        }
    }
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let telemetry = match Telemetry::init() {
        Ok(t) => t,
        Err(_) => {
            debug_log("[TASKS] telemetry unavailable\n");
            ProcessExit::exit(1);
        }
    };

    let mut app = TasksApp::new(telemetry);
    let mut window = match Window::connect(WindowConfig {
        width: WIN_W,
        height: WIN_H,
        title: "Tasks Monitor",
    }) {
        Some(window) => window,
        None => {
            debug_log("[TASKS] failed to connect window\n");
            loop {
                process_yield();
            }
        }
    };
    window.run(&mut app);
    ProcessExit::exit(0);
}

fn state_text(state: ProcessState) -> &'static str {
    match state {
        ProcessState::Ready => "ready",
        ProcessState::Running => "running",
        ProcessState::Blocked => "blocked",
        ProcessState::Finished => "finished",
    }
}

fn write_str(text: &str, dst: &mut [u8; CELL_BUF], len: &mut usize) {
    *len = text.len().min(dst.len());
    dst[..*len].copy_from_slice(&text.as_bytes()[..*len]);
}

fn write_u32(value: u32, dst: &mut [u8; CELL_BUF], len: &mut usize) {
    *len = write_num_into(value, dst);
}

fn write_pct(value: u32, dst: &mut [u8; CELL_BUF], len: &mut usize) {
    *len = write_num_into(value, dst);
    if *len < dst.len() {
        dst[*len] = b'%';
        *len += 1;
    }
}

fn write_kib(value: u32, dst: &mut [u8; CELL_BUF], len: &mut usize) {
    *len = copy_tail_mb(value as u64, dst);
}

fn write_num_into(mut value: u32, dst: &mut [u8]) -> usize {
    if dst.is_empty() {
        return 0;
    }
    if value == 0 {
        dst[0] = b'0';
        return 1;
    }
    let mut tmp = [0u8; 10];
    let mut digits = 0;
    while value > 0 && digits < tmp.len() {
        tmp[digits] = b'0' + (value % 10) as u8;
        value /= 10;
        digits += 1;
    }
    for idx in 0..digits.min(dst.len()) {
        dst[idx] = tmp[digits - idx - 1];
    }
    digits.min(dst.len())
}

fn copy_tail(src: &[u8], dst: &mut [u8]) -> usize {
    let len = src.len().min(dst.len());
    dst[..len].copy_from_slice(&src[..len]);
    len
}

fn write_bp_into(value: u16, dst: &mut [u8]) -> usize {
    if dst.len() < 6 {
        return 0;
    }
    let whole = (value / 100) as u32;
    let frac = (value % 100) as u32;
    let mut n = write_num_into(whole, dst);
    if n + 3 > dst.len() {
        return n;
    }
    dst[n] = b'.';
    dst[n + 1] = b'0' + (frac / 10) as u8;
    dst[n + 2] = b'0' + (frac % 10) as u8;
    n += 3;
    if n < dst.len() {
        dst[n] = b'%';
        n += 1;
    }
    n
}

fn write_mb_into(kb: u64, dst: &mut [u8]) -> usize {
    let mb_whole = kb / 1024;
    let mb_tenths = ((kb % 1024) * 10) / 1024;
    let mut n = write_num_into(mb_whole.min(u32::MAX as u64) as u32, dst);
    if n + 3 > dst.len() {
        return n;
    }
    dst[n] = b'.';
    dst[n + 1] = b'0' + mb_tenths as u8;
    dst[n + 2] = b' ';
    n += 3;
    n + copy_tail(b"MB", &mut dst[n..])
}

fn copy_tail_mb(kb: u64, dst: &mut [u8]) -> usize {
    write_mb_into(kb, dst)
}

fn trim_zeros(bytes: &[u8]) -> &[u8] {
    let len = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    &bytes[..len]
}
