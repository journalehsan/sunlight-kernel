#![no_std]
#![no_main]

use sun_font::{FontRole, VecFont};
use sunlight_ipc::{
    debug_log,
    launch_trace::{self, LaunchSource, LaunchTrace},
    process_yield, ProcessExit,
};
use sunlight_libc::kill as libc_kill;
use sunlight_telemetry::{ProcessState, SystemSnapshot, Telemetry, MAX_CORES, MAX_PROCESSES};
use sunlight_ui::{
    request_close,
    widgets::{Column, Label, Panel, StatusBar, Table},
    App, Color, Event, Point, Rect, Theme, VecText, Window, WindowConfig,
};

static F_UI: VecFont = VecFont(FontRole::UiRegular);
static F_MED: VecFont = VecFont(FontRole::UiMedium);
static F_SMALL: VecFont = VecFont(FontRole::UiSmall);

const WIN_W: u32 = 720;
const WIN_H: u32 = 540;
const STATUS_H: u32 = 18;
const TITLE_H: u32 = 22;
const TOOLBAR_H: u32 = 32;
const SUMMARY_H: u32 = 102;
const CONTENT_MARGIN: i32 = 12;
const ACTION_GAP: i32 = 8;
const ACTION_WIDTHS: [u32; 4] = [92, 104, 96, 92];
const TABLE_COLS: usize = 5;
const CORE_TABLE_COLS: usize = 8;
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum ViewMode {
    Processes,
    Cores,
}

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

const CORE_COLUMNS: [Column<'static>; CORE_TABLE_COLS] = [
    Column {
        header: "Core",
        width: 50,
        right_align: true,
    },
    Column {
        header: "PID",
        width: 60,
        right_align: true,
    },
    Column {
        header: "Name",
        width: 135,
        right_align: false,
    },
    Column {
        header: "Nice",
        width: 48,
        right_align: true,
    },
    Column {
        header: "State",
        width: 80,
        right_align: false,
    },
    Column {
        header: "Load%",
        width: 72,
        right_align: true,
    },
    Column {
        header: "LTimer",
        width: 75,
        right_align: true,
    },
    Column {
        header: "CSw",
        width: 70,
        right_align: true,
    },
];

const EMPTY_ROW: [&str; TABLE_COLS] = ["", "", "", "", ""];
const EMPTY_CORE_ROW: [&str; CORE_TABLE_COLS] = ["", "", "", "", "", "", "", ""];

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
    view_mode: ViewMode,
    hovered_action: Option<usize>,
    pressed_action: Option<usize>,
    order: [usize; MAX_PROCESSES],
    row_bufs: [[[u8; CELL_BUF]; TABLE_COLS]; MAX_PROCESSES],
    row_lens: [[usize; TABLE_COLS]; MAX_PROCESSES],
    row_count: usize,
    core_row_bufs: [[[u8; CELL_BUF]; CORE_TABLE_COLS]; MAX_CORES],
    core_row_lens: [[usize; CORE_TABLE_COLS]; MAX_CORES],
    core_row_count: usize,
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
            view_mode: ViewMode::Processes,
            hovered_action: None,
            pressed_action: None,
            order: [0; MAX_PROCESSES],
            row_bufs: [[[0; CELL_BUF]; TABLE_COLS]; MAX_PROCESSES],
            row_lens: [[0; TABLE_COLS]; MAX_PROCESSES],
            row_count: 0,
            core_row_bufs: [[[0; CELL_BUF]; CORE_TABLE_COLS]; MAX_CORES],
            core_row_lens: [[0; CORE_TABLE_COLS]; MAX_CORES],
            core_row_count: 0,
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

    /// Deliver SIGKILL to the currently-selected process and refresh the
    /// telemetry view. Refuses to kill pid 0 (kernel sentinel), pid 1 (init),
    /// or our own pid — those are protected. The vortex shell and the dock's
    /// running-app indicator observe the pid's death on the next
    /// `process_is_alive` poll (`APP_STATE_POLL_MS = 250ms`) and clear the
    /// indicator. See `docs/GUI/START_MENU.md` (lifecycle rules) and
    /// `sunlight-shell-appstate`.
    fn end_task(&mut self) {
        let Some(target) = self.selected_pid else {
            self.set_status("Select a process first");
            return;
        };
        if target == 0 || target == 1 {
            self.set_status("Refused: protected pid");
            return;
        }
        let self_pid = sunlight_ipc::getpid();
        if u64::from(target) == self_pid {
            self.set_status("Refused: cannot end self");
            return;
        }
        match libc_kill(u64::from(target), 9) {
            Ok(()) => {
                self.set_status("End task sent");
                debug_log("[TASKS] end_task dispatched\n");
                let _ = self.refresh(true);
            }
            Err(_) => {
                self.set_status("End task failed");
            }
        }
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
            self.rebuild_core_rows();
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

    fn rebuild_core_rows(&mut self) {
        let count = self.snapshot.cpu_telemetry.count.min(MAX_CORES);
        self.core_row_count = count;
        for i in 0..count {
            // Copy all core fields to locals before borrowing bufs mutably.
            let core_id = self.snapshot.cpu_telemetry.cores[i].core_id;
            let pid = self.snapshot.cpu_telemetry.cores[i].current_task_pid;
            let nice = self.snapshot.cpu_telemetry.cores[i].nice;
            let load_bp = self.snapshot.cpu_telemetry.cores[i].load_bp;
            let local_timer_ticks = self.snapshot.cpu_telemetry.cores[i].local_timer_ticks;
            let context_switches = self.snapshot.cpu_telemetry.cores[i].context_switches;

            // Look up process name from the process table; the core snapshot
            // is authoritative for state (if current_pid != 0 it IS running).
            let mut pname_buf = [0u8; 32];
            let mut pname_len = 0usize;
            let mut found = false;
            if pid != 0 {
                for j in 0..self.snapshot.proc_count {
                    if self.snapshot.procs[j].pid == pid {
                        let s = self.snapshot.procs[j].name_str().as_bytes();
                        pname_len = s.len().min(32);
                        pname_buf[..pname_len].copy_from_slice(&s[..pname_len]);
                        found = true;
                        break;
                    }
                }
            }

            // Write columns (no snapshot borrow active at this point).
            write_u32(
                core_id as u32,
                &mut self.core_row_bufs[i][0],
                &mut self.core_row_lens[i][0],
            );

            if pid != 0 {
                write_u32(
                    pid,
                    &mut self.core_row_bufs[i][1],
                    &mut self.core_row_lens[i][1],
                );
            } else {
                write_str(
                    "-",
                    &mut self.core_row_bufs[i][1],
                    &mut self.core_row_lens[i][1],
                );
            }

            if pid != 0 {
                let name = core::str::from_utf8(&pname_buf[..pname_len]).unwrap_or("?");
                write_str(
                    if found { name } else { "?" },
                    &mut self.core_row_bufs[i][2],
                    &mut self.core_row_lens[i][2],
                );
            } else {
                write_str(
                    "idle",
                    &mut self.core_row_bufs[i][2],
                    &mut self.core_row_lens[i][2],
                );
            }

            if pid != 0 {
                write_nice(
                    nice,
                    &mut self.core_row_bufs[i][3],
                    &mut self.core_row_lens[i][3],
                );
            } else {
                write_str(
                    "-",
                    &mut self.core_row_bufs[i][3],
                    &mut self.core_row_lens[i][3],
                );
            }

            // State is authoritative from the core snapshot, not the process table.
            write_str(
                if pid != 0 { "running" } else { "idle" },
                &mut self.core_row_bufs[i][4],
                &mut self.core_row_lens[i][4],
            );

            write_pct(
                (load_bp / 100).min(100) as u32,
                &mut self.core_row_bufs[i][5],
                &mut self.core_row_lens[i][5],
            );

            write_compact_count(
                local_timer_ticks,
                &mut self.core_row_bufs[i][6],
                &mut self.core_row_lens[i][6],
            );

            write_compact_count(
                context_switches,
                &mut self.core_row_bufs[i][7],
                &mut self.core_row_lens[i][7],
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
        let max_row = match self.view_mode {
            ViewMode::Processes => self.row_count,
            ViewMode::Cores => self.core_row_count,
        };
        let max_scroll = max_row.saturating_sub(visible);
        if self.scroll > max_scroll {
            self.scroll = max_scroll;
        }
    }

    fn visible_rows(&self) -> usize {
        let content = self.content_rect();
        let header_h = TITLE_H + 6 + TOOLBAR_H + 8 + SUMMARY_H + 8;
        let usable = content.h.saturating_sub(header_h + STATUS_H + 4);
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
            content.x + 10,
            content.y + TITLE_H as i32 + 4,
            content.w.saturating_sub(20),
            TOOLBAR_H,
        )
    }

    fn action_rect(&self, index: usize) -> Rect {
        let toolbar = self.toolbar_rect();
        let x_offset = ACTION_WIDTHS
            .iter()
            .take(index)
            .fold(0i32, |offset, width| offset + *width as i32 + ACTION_GAP);
        Rect::new(
            toolbar.x + x_offset,
            toolbar.y,
            ACTION_WIDTHS.get(index).copied().unwrap_or(0),
            toolbar.h,
        )
    }

    fn action_at(&self, x: i32, y: i32) -> Option<usize> {
        (0..ACTION_WIDTHS.len()).find(|index| self.action_rect(*index).contains(Point::new(x, y)))
    }

    fn summary_rect(&self) -> Rect {
        let content = self.content_rect();
        Rect::new(
            content.x + 10,
            self.toolbar_rect().bottom() + 8,
            content.w.saturating_sub(20),
            SUMMARY_H,
        )
    }

    fn summary_card_rects(&self) -> (Rect, Rect) {
        let summary = self.summary_rect();
        let inner = Rect::new(
            summary.x + 12,
            summary.y + 32,
            summary.w.saturating_sub(24),
            summary.h.saturating_sub(44),
        );
        let gap = 12;
        let card_w = inner.w.saturating_sub(gap) / 2;
        (
            Rect::new(inner.x, inner.y, card_w, inner.h),
            Rect::new(
                inner.x + card_w as i32 + gap as i32,
                inner.y,
                card_w,
                inner.h,
            ),
        )
    }

    fn table_rect(&self) -> Rect {
        let content = self.content_rect();
        let top = self.summary_rect().bottom() + 8;
        let status_h = STATUS_H + 4;
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

    fn overview_strings(&self, uptime: &mut [u8; 24], tasks: &mut [u8; 24]) {
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
    }

    fn draw_action_button(
        &self,
        canvas: &mut sunlight_ui::Canvas,
        theme: &Theme,
        index: usize,
        label: &str,
        active: bool,
        danger: bool,
    ) {
        let rect = self.action_rect(index);
        let hovered = self.hovered_action == Some(index);
        let pressed = self.pressed_action == Some(index);
        let (fill, border, text) = if danger {
            if pressed {
                (theme.danger.darken(125), theme.danger, theme.danger_text)
            } else if hovered {
                (theme.danger.darken(165), theme.danger, theme.danger_text)
            } else {
                (theme.panel_alt, theme.danger.darken(70), theme.text_muted)
            }
        } else if active {
            (
                theme.accent.darken(if pressed { 125 } else { 165 }),
                theme.accent,
                theme.accent_hover,
            )
        } else if pressed {
            (theme.border, theme.accent, theme.text)
        } else if hovered {
            (
                theme.panel_alt.lighten(12),
                theme.accent.darken(45),
                theme.text,
            )
        } else {
            (theme.panel_alt, theme.border, theme.text_muted)
        };

        canvas.fill_rounded_rect(rect, 8, fill);
        canvas.stroke_rounded_rect(rect, 8, 1, border);
        if active {
            canvas.fill_rounded_rect(
                Rect::new(rect.x + 9, rect.bottom() - 5, rect.w.saturating_sub(18), 2),
                1,
                theme.accent,
            );
        }
        let text_w = F_UI.measure_w(label);
        F_UI.draw_vcenter(
            canvas,
            label,
            rect.x + (rect.w as i32 - text_w as i32) / 2,
            rect.y,
            rect.h,
            text,
        );
    }

    fn draw_metric_card(
        &self,
        canvas: &mut sunlight_ui::Canvas,
        theme: &Theme,
        rect: Rect,
        title: &str,
        value: &str,
        detail: &str,
        usage_bp: u16,
    ) {
        canvas.fill_rounded_rect(rect, 9, theme.panel);
        canvas.stroke_rounded_rect(rect, 9, 1, theme.border);

        let title_rect = Rect::new(rect.x + 12, rect.y + 6, rect.w.saturating_sub(24), 16);
        F_MED.draw_vcenter(
            canvas,
            title,
            title_rect.x,
            title_rect.y,
            title_rect.h,
            theme.text,
        );
        let value_w = F_MED.measure_w(value);
        F_MED.draw_vcenter(
            canvas,
            value,
            title_rect.right() - value_w as i32,
            title_rect.y,
            title_rect.h,
            usage_color(theme, usage_bp),
        );

        F_SMALL.draw_vcenter(canvas, detail, rect.x + 12, rect.y + 24, 14, theme.text_dim);
        draw_usage_bar(
            canvas,
            theme,
            Rect::new(
                rect.x + 12,
                rect.bottom() - 15,
                rect.w.saturating_sub(24),
                7,
            ),
            usage_bp,
        );
    }
}

impl App for TasksApp {
    fn view(&mut self, canvas: &mut sunlight_ui::Canvas, theme: &sunlight_ui::Theme) {
        canvas.fill_rect(Rect::new(0, 0, WIN_W, WIN_H), theme.bg);

        let content = self.content_rect();
        Panel::new(content).draw(canvas, theme);

        let title_rect = Rect::new(content.x + 12, content.y + 6, 220, TITLE_H);
        Label::new(title_rect, "Tasks Monitor")
            .with_font(&F_MED)
            .draw(canvas, theme);
        Label::new(
            Rect::new(content.right() - 88, content.y + 6, 76, TITLE_H),
            "SunlightOS",
        )
        .dim()
        .with_font(&F_SMALL)
        .draw(canvas, theme);

        self.draw_action_button(canvas, theme, 0, "End task", false, true);
        self.draw_action_button(
            canvas,
            theme,
            1,
            if self.view_mode == ViewMode::Cores {
                "Processes"
            } else {
                "CPU cores"
            },
            self.view_mode == ViewMode::Cores,
            false,
        );
        self.draw_action_button(
            canvas,
            theme,
            2,
            if self.show_system_info {
                "Hide info"
            } else {
                "Show info"
            },
            self.show_system_info,
            false,
        );
        self.draw_action_button(canvas, theme, 3, "Refresh", false, false);

        let summary = self.summary_rect();
        canvas.fill_rounded_rect(summary, 11, theme.panel_alt);
        canvas.stroke_rounded_rect(summary, 11, 1, theme.border);
        let overview_title = if self.show_system_info {
            "System overview"
        } else {
            "Resource usage"
        };
        F_MED.draw_vcenter(
            canvas,
            overview_title,
            summary.x + 14,
            summary.y + 5,
            20,
            theme.text,
        );
        let mut uptime = [0u8; 24];
        let mut tasks = [0u8; 24];
        self.overview_strings(&mut uptime, &mut tasks);
        let uptime_str = core::str::from_utf8(trim_zeros(&uptime)).unwrap_or("");
        let tasks_str = core::str::from_utf8(trim_zeros(&tasks)).unwrap_or("");
        if self.show_system_info {
            let tasks_w = F_SMALL.measure_w(tasks_str);
            let uptime_w = F_SMALL.measure_w(uptime_str);
            let info_x = summary.right() - 14 - tasks_w as i32;
            F_SMALL.draw_vcenter(
                canvas,
                tasks_str,
                info_x,
                summary.y + 6,
                18,
                theme.text_muted,
            );
            F_SMALL.draw_vcenter(
                canvas,
                uptime_str,
                info_x - uptime_w as i32 - 18,
                summary.y + 6,
                18,
                theme.text_muted,
            );
        }

        let mut cpu_value = [0u8; 16];
        let cpu_value_len = write_bp_into(self.snapshot.cpu_used_bp, &mut cpu_value);
        let cpu_value_str = core::str::from_utf8(&cpu_value[..cpu_value_len]).unwrap_or("");
        let mut cpu_detail = [0u8; 40];
        let mut cpu_detail_len = copy_tail(b"Idle ", &mut cpu_detail);
        cpu_detail_len +=
            write_bp_into(self.snapshot.cpu_idle_bp, &mut cpu_detail[cpu_detail_len..]);
        let cpu_detail_str = core::str::from_utf8(&cpu_detail[..cpu_detail_len]).unwrap_or("");

        let ram_usage_bp = ram_usage_bp(self.snapshot.used_ram_kb, self.snapshot.total_ram_kb);
        let mut ram_value = [0u8; 16];
        let ram_value_len = write_bp_into(ram_usage_bp, &mut ram_value);
        let ram_value_str = core::str::from_utf8(&ram_value[..ram_value_len]).unwrap_or("");
        let mut ram_detail = [0u8; 40];
        let mut ram_detail_len = copy_tail(b"Using ", &mut ram_detail);
        ram_detail_len +=
            write_mb_into(self.snapshot.used_ram_kb, &mut ram_detail[ram_detail_len..]);
        ram_detail_len += copy_tail(b" of ", &mut ram_detail[ram_detail_len..]);
        ram_detail_len += write_mb_into(
            self.snapshot.total_ram_kb,
            &mut ram_detail[ram_detail_len..],
        );
        let ram_detail_str = core::str::from_utf8(&ram_detail[..ram_detail_len]).unwrap_or("");

        let (cpu_card, ram_card) = self.summary_card_rects();
        self.draw_metric_card(
            canvas,
            theme,
            cpu_card,
            "Processor",
            cpu_value_str,
            cpu_detail_str,
            self.snapshot.cpu_used_bp,
        );
        self.draw_metric_card(
            canvas,
            theme,
            ram_card,
            "Memory",
            ram_value_str,
            ram_detail_str,
            ram_usage_bp,
        );

        let table_rect = self.table_rect();
        let visible = self.visible_rows();

        match self.view_mode {
            ViewMode::Processes => {
                let end = (self.scroll + visible).min(self.row_count);
                let mut row_cells = [EMPTY_ROW; MAX_PROCESSES];
                let mut row_refs: [&[&str]; MAX_PROCESSES] = [&[]; MAX_PROCESSES];
                for row in 0..self.row_count {
                    for col in 0..TABLE_COLS {
                        row_cells[row][col] = core::str::from_utf8(
                            &self.row_bufs[row][col][..self.row_lens[row][col]],
                        )
                        .unwrap_or("");
                    }
                }
                for row in 0..self.row_count {
                    row_refs[row] = &row_cells[row];
                }
                let selected = self.selected_pid.and_then(|pid| {
                    (self.scroll..end)
                        .position(|idx| self.snapshot.procs[self.order[idx]].pid == pid)
                });
                Table::new(table_rect, &TABLE_COLUMNS, &row_refs[self.scroll..end])
                    .with_selected(selected)
                    .with_font(&F_UI)
                    .draw(canvas, theme);
            }

            ViewMode::Cores => {
                let count = self.core_row_count;
                let start = self.scroll.min(count.saturating_sub(visible));
                let end = (start + visible).min(count);
                let mut core_cells = [EMPTY_CORE_ROW; MAX_CORES];
                let mut core_refs: [&[&str]; MAX_CORES] = [&[]; MAX_CORES];
                for i in 0..count {
                    for col in 0..CORE_TABLE_COLS {
                        core_cells[i][col] = core::str::from_utf8(
                            &self.core_row_bufs[i][col][..self.core_row_lens[i][col]],
                        )
                        .unwrap_or("");
                    }
                }
                for i in 0..count {
                    core_refs[i] = &core_cells[i];
                }
                Table::new(table_rect, &CORE_COLUMNS, &core_refs[start..end])
                    .with_font(&F_UI)
                    .draw(canvas, theme);
            }
        }

        StatusBar::new(
            self.status_bar_rect(),
            self.status_str(),
            "",
            self.hw_info_str(),
        )
        .draw(canvas, theme);
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::Tick => self.refresh(false),
            Event::MouseMove { x, y } => {
                let hovered = self.action_at(x, y);
                if hovered != self.hovered_action {
                    self.hovered_action = hovered;
                    return true;
                }
                false
            }
            Event::MouseDown { x, y, button: 0 } => {
                let pressed = self.action_at(x, y);
                if pressed.is_some() {
                    self.pressed_action = pressed;
                    self.hovered_action = pressed;
                    return true;
                }
                false
            }
            Event::MouseUp { .. } => {
                if self.pressed_action.take().is_some() {
                    return true;
                }
                false
            }
            Event::Click { x, y } => {
                let released_action = self.action_at(x, y);
                let pressed_action = self.pressed_action.take();
                self.hovered_action = released_action;
                if let Some(pressed_idx) = pressed_action {
                    if released_action != Some(pressed_idx) {
                        return true;
                    }
                }
                if let Some(idx) = released_action {
                    match idx {
                        0 => self.end_task(),
                        1 => {
                            self.view_mode = if self.view_mode == ViewMode::Cores {
                                ViewMode::Processes
                            } else {
                                ViewMode::Cores
                            };
                            self.scroll = 0;
                            self.set_status(if self.view_mode == ViewMode::Cores {
                                "Cores view"
                            } else {
                                "Processes view"
                            });
                        }
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

                if self.view_mode == ViewMode::Processes {
                    let table = Table::new(self.table_rect(), &TABLE_COLUMNS, &[]).with_font(&F_UI);
                    if let Some(row) = table.hit_test(x, y) {
                        let idx = self.scroll + row;
                        if idx < self.row_count {
                            let proc = &self.snapshot.procs[self.order[idx]];
                            self.selected_pid = Some(proc.pid);
                            self.set_status("Process selected");
                            return true;
                        }
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
                let page = self.visible_rows().max(1) / 2;
                match keycode {
                    KEY_UP => self.scroll = self.scroll.saturating_sub(1),
                    KEY_DOWN => self.scroll = self.scroll.saturating_add(1),
                    KEY_PGUP => self.scroll = self.scroll.saturating_sub(page),
                    KEY_PGDN => self.scroll = self.scroll.saturating_add(page),
                    KEY_HOME => self.scroll = 0,
                    KEY_END => {
                        let total = match self.view_mode {
                            ViewMode::Processes => self.row_count,
                            ViewMode::Cores => self.core_row_count,
                        };
                        self.scroll = total.saturating_sub(self.visible_rows());
                    }
                    KEY_ENTER => self.end_task(),
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
pub extern "C" fn _start(argc: u64, argv: *const *const u8, _envp: *const *const u8) -> ! {
    sunlight_libc::launch_trace::init_from_argv(argc, argv);
    let trace = launch_trace::current().unwrap_or(LaunchTrace::new(0, LaunchSource::Unknown, 0));
    launch_trace::log_phase_now(
        trace,
        "app=tasks",
        "app_main_started",
        Some(sunlight_ipc::getpid()),
    );
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
        decoration: sunlight_ui::WindowDecoration::Normal,
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

fn ram_usage_bp(used_kb: u64, total_kb: u64) -> u16 {
    if total_kb == 0 {
        return 0;
    }
    used_kb
        .saturating_mul(10_000)
        .checked_div(total_kb)
        .unwrap_or(0)
        .min(10_000) as u16
}

fn usage_color(theme: &Theme, usage_bp: u16) -> Color {
    if usage_bp >= 8_500 {
        theme.danger
    } else if usage_bp >= 6_500 {
        theme.warn
    } else {
        theme.accent
    }
}

fn draw_usage_bar(canvas: &mut sunlight_ui::Canvas, theme: &Theme, rect: Rect, usage_bp: u16) {
    canvas.fill_rounded_rect(rect, 4, theme.bg);
    canvas.stroke_rounded_rect(rect, 4, 1, theme.border);
    let inner = rect.inset(2);
    let fill_w = inner.w.saturating_mul(usage_bp.min(10_000) as u32) / 10_000;
    if fill_w > 0 {
        canvas.fill_rounded_rect(
            Rect::new(inner.x, inner.y, fill_w, inner.h),
            2,
            usage_color(theme, usage_bp),
        );
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

fn write_compact_count(value: u64, dst: &mut [u8; CELL_BUF], len: &mut usize) {
    if value < 10_000 {
        *len = write_num_into(value as u32, dst);
    } else if value < 10_000_000 {
        let mut n = write_num_into((value / 1000) as u32, dst);
        if n < dst.len() {
            dst[n] = b'K';
            n += 1;
        }
        *len = n;
    } else {
        let mut n = write_num_into((value / 1_000_000) as u32, dst);
        if n < dst.len() {
            dst[n] = b'M';
            n += 1;
        }
        *len = n;
    }
}

fn write_nice(value: i8, dst: &mut [u8; CELL_BUF], len: &mut usize) {
    if value == 0 {
        if !dst.is_empty() {
            dst[0] = b'0';
        }
        *len = if dst.is_empty() { 0 } else { 1 };
        return;
    }
    let mut n = 0usize;
    if value < 0 {
        if !dst.is_empty() {
            dst[0] = b'-';
            n = 1;
        }
        n += write_num_into((-(value as i16)) as u32, &mut dst[n..]);
    } else {
        n += write_num_into(value as u32, dst);
    }
    *len = n;
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
