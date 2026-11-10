#![cfg_attr(not(test), no_std)]
#![cfg_attr(not(test), no_main)]

use sun_font::{FontRole, VecFont};
use sunlight_ipc::{
    debug_log,
    launch_trace::{self, LaunchSource, LaunchTrace},
    process_yield, ProcessExit,
};
use sunlight_libc::kill as libc_kill;
use sunlight_telemetry::{ProcessState, SystemSnapshot, Telemetry, MAX_CORES, MAX_PROCESSES};
use sunlight_ui::layout::Column as LayoutColumn;
use sunlight_ui::{
    request_close,
    widgets::{Column, Label, StatusBar, Table},
    App, AxisSizing, Color, Event, LayoutBox, LayoutInvalidation, Material, MaterialPalette, Point,
    Rect, Row, Size, Sizing, Theme, VecText, Window, WindowConfig, WindowEvent, WindowMaterial,
};

static F_UI: VecFont = VecFont(FontRole::UiRegular);
static F_MED: VecFont = VecFont(FontRole::UiMedium);
static F_SMALL: VecFont = VecFont(FontRole::UiSmall);

const WIN_W: u32 = 1024;
const WIN_H: u32 = 600;
const STATUS_H: u32 = 18;
const TITLE_H: u32 = 22;
const TOOLBAR_H: u32 = 32;
/// Extra height for physical-memory accounting breakdown (Phase 1).
const SUMMARY_H: u32 = 148;
const CONTENT_MARGIN: i32 = 12;
const ACTION_GAP: i32 = 8;
const ACTION_WIDTHS: [u32; 4] = [92, 104, 96, 92];
const TABLE_COLS: usize = 8;
const MAX_ROWS: usize = MAX_PROCESSES * 2;
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

#[cfg(not(test))]
struct NoAlloc;

#[cfg(not(test))]
unsafe impl core::alloc::GlobalAlloc for NoAlloc {
    unsafe fn alloc(&self, _layout: core::alloc::Layout) -> *mut u8 {
        core::ptr::null_mut()
    }
    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: core::alloc::Layout) {}
}

#[cfg(not(test))]
#[global_allocator]
static ALLOC: NoAlloc = NoAlloc;

#[cfg(not(test))]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    debug_log("[TASKS] panic\n");
    loop {
        process_yield();
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct TasksLayout {
    root: Rect,
    content: Rect,
    title: Rect,
    brand: Rect,
    toolbar: Rect,
    actions: [Rect; 4],
    summary: Rect,
    cpu_card: Rect,
    ram_card: Rect,
    mem_breakdown: Rect,
    table: Rect,
    status: Rect,
}

fn fill_sizing() -> Sizing {
    Sizing::new(AxisSizing::Fill, AxisSizing::Fill)
}

fn fixed_height_box(height: u32) -> LayoutBox {
    LayoutBox::new(Rect::new(0, 0, 0, height))
        .with_sizing(Sizing::new(AxisSizing::Fill, AxisSizing::Fixed(height)))
}

const TABLE_COLUMNS: [Column<'static>; TABLE_COLS] = [
    Column {
        header: "PID",
        width: 60,
        right_align: true,
    },
    Column {
        header: "Name",
        width: 180,
        right_align: false,
    },
    Column {
        header: "State",
        width: 90,
        right_align: false,
    },
    Column {
        header: "CPU%",
        width: 80,
        right_align: true,
    },
    Column {
        // Mapped present user pages (not unique private; shared counted per map).
        header: "Private",
        width: 120,
        right_align: true,
    },
    Column {
        header: "Shared",
        width: 100,
        right_align: true,
    },
    Column {
        header: "Mapped",
        width: 100,
        right_align: true,
    },
    Column {
        header: "App ID",
        width: 150,
        right_align: false,
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
        header: "CSw/s",
        width: 70,
        right_align: true,
    },
];

const EMPTY_CORE_ROW: [&str; CORE_TABLE_COLS] = ["", "", "", "", "", "", "", ""];

struct TasksApp {
    grouping: u8, // 0: flat, 1: parent tree, 2: application groups
    collapsed: [[u8; 32]; MAX_PROCESSES],
    row_group: [bool; MAX_ROWS],
    sort_col: usize,
    descending: bool,
    query: [u8; 64],
    query_len: usize,
    search_focus: bool,
    interval_ms: u64,
    last_poll_ms: u64,
    last_update_ms: u64,
    paused: bool,
    toast_until_ms: u64,
    confirm_pid: Option<u32>,
    details: bool,
    widths: [u32; TABLE_COLS],
    col_order: [usize; TABLE_COLS],
    drag: Option<(usize, i32, bool)>,
    cpu_history: [[u16; 30]; MAX_PROCESSES],
    mem_history: [[u64; 30]; MAX_PROCESSES],
    history_ids: [(u32, u32); MAX_PROCESSES],
    history_pos: usize,
    warning_bp: u16,
    danger_bp: u16,
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
    order: [usize; MAX_ROWS],
    row_bufs: [[[u8; CELL_BUF]; TABLE_COLS]; MAX_ROWS],
    row_lens: [[usize; TABLE_COLS]; MAX_ROWS],
    row_count: usize,
    core_row_bufs: [[[u8; CELL_BUF]; CORE_TABLE_COLS]; MAX_CORES],
    core_row_lens: [[usize; CORE_TABLE_COLS]; MAX_CORES],
    core_row_count: usize,
    client_bounds: Rect,
    layout_invalidation: LayoutInvalidation,
    layout: TasksLayout,
}

impl TasksApp {
    fn new(telemetry: Telemetry) -> Self {
        let mut app = Self {
            grouping: 0,
            collapsed: [[0; 32]; MAX_PROCESSES],
            row_group: [false; MAX_ROWS],
            sort_col: 3,
            descending: true,
            query: [0; 64],
            query_len: 0,
            search_focus: false,
            interval_ms: 1000,
            last_poll_ms: 0,
            last_update_ms: 0,
            paused: false,
            toast_until_ms: 0,
            confirm_pid: None,
            details: false,
            widths: core::array::from_fn(|i| TABLE_COLUMNS[i].width),
            col_order: core::array::from_fn(|i| i),
            drag: None,
            cpu_history: [[0; 30]; MAX_PROCESSES],
            mem_history: [[0; 30]; MAX_PROCESSES],
            history_ids: [(0, 0); MAX_PROCESSES],
            history_pos: 0,
            warning_bp: 5000,
            danger_bp: 8000,
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
            order: [0; MAX_ROWS],
            row_bufs: [[[0; CELL_BUF]; TABLE_COLS]; MAX_ROWS],
            row_lens: [[0; TABLE_COLS]; MAX_ROWS],
            row_count: 0,
            core_row_bufs: [[[0; CELL_BUF]; CORE_TABLE_COLS]; MAX_CORES],
            core_row_lens: [[0; CORE_TABLE_COLS]; MAX_CORES],
            core_row_count: 0,
            client_bounds: Rect::new(0, 0, WIN_W, WIN_H),
            layout_invalidation: LayoutInvalidation::new(),
            layout: TasksLayout::default(),
        };
        #[cfg(not(test))]
        app.load_columns();
        if let Some(value) =
            sunlight_libc::env::getenv_bytes(b"SUNLIGHT_TASKS_WARN_PERCENT").and_then(parse_percent)
        {
            app.warning_bp = value;
        }
        if let Some(value) = sunlight_libc::env::getenv_bytes(b"SUNLIGHT_TASKS_DANGER_PERCENT")
            .and_then(parse_percent)
        {
            app.danger_bp = value.max(app.warning_bp);
        }
        app.set_status("Telemetry ready");
        app.ensure_layout();
        #[cfg(not(test))]
        app.refresh(true);
        app
    }

    fn compute_layout(root: Rect) -> TasksLayout {
        let content = Rect::new(
            CONTENT_MARGIN,
            CONTENT_MARGIN,
            root.w
                .saturating_sub((CONTENT_MARGIN as u32).saturating_mul(2)),
            root.h
                .saturating_sub((CONTENT_MARGIN as u32).saturating_mul(2)),
        );
        let mut root_children = [
            fixed_height_box(TITLE_H + 6),
            fixed_height_box(TOOLBAR_H),
            fixed_height_box(SUMMARY_H),
            LayoutBox::new(Rect::new(0, 0, 0, 0)).with_sizing(fill_sizing()),
            fixed_height_box(STATUS_H),
        ];
        let _ = LayoutColumn::new(content)
            .with_gap(8)
            .arrange(&mut root_children);
        let title_row = root_children[0].bounds();
        let toolbar = root_children[1].bounds();
        let summary = root_children[2].bounds();
        let table = root_children[3].bounds();
        let status = root_children[4].bounds();

        let brand_w = 76u32;
        let title_w = 220u32;
        let mut title_children = [
            LayoutBox::new(Rect::new(0, 0, title_w, TITLE_H)).with_sizing(Sizing::new(
                AxisSizing::Fixed(title_w),
                AxisSizing::Fixed(TITLE_H),
            )),
            LayoutBox::new(Rect::new(0, 0, 0, TITLE_H))
                .with_sizing(Sizing::new(AxisSizing::Fill, AxisSizing::Fixed(TITLE_H))),
            LayoutBox::new(Rect::new(0, 0, brand_w, TITLE_H)).with_sizing(Sizing::new(
                AxisSizing::Fixed(brand_w),
                AxisSizing::Fixed(TITLE_H),
            )),
        ];
        let title_inner = Rect::new(
            title_row.x + 12,
            title_row.y + 6,
            title_row.w.saturating_sub(24),
            TITLE_H,
        );
        let _ = Row::new(title_inner).arrange(&mut title_children);

        let toolbar_inner = Rect::new(
            toolbar.x + 10,
            toolbar.y,
            toolbar.w.saturating_sub(20),
            toolbar.h,
        );
        let mut action_boxes = [
            LayoutBox::new(Rect::new(0, 0, ACTION_WIDTHS[0], TOOLBAR_H)).with_sizing(Sizing::new(
                AxisSizing::Fixed(ACTION_WIDTHS[0]),
                AxisSizing::Fixed(TOOLBAR_H),
            )),
            LayoutBox::new(Rect::new(0, 0, ACTION_WIDTHS[1], TOOLBAR_H)).with_sizing(Sizing::new(
                AxisSizing::Fixed(ACTION_WIDTHS[1]),
                AxisSizing::Fixed(TOOLBAR_H),
            )),
            LayoutBox::new(Rect::new(0, 0, ACTION_WIDTHS[2], TOOLBAR_H)).with_sizing(Sizing::new(
                AxisSizing::Fixed(ACTION_WIDTHS[2]),
                AxisSizing::Fixed(TOOLBAR_H),
            )),
            LayoutBox::new(Rect::new(0, 0, ACTION_WIDTHS[3], TOOLBAR_H)).with_sizing(Sizing::new(
                AxisSizing::Fixed(ACTION_WIDTHS[3]),
                AxisSizing::Fixed(TOOLBAR_H),
            )),
        ];
        let _ = Row::new(toolbar_inner)
            .with_gap(ACTION_GAP as u32)
            .arrange(&mut action_boxes);

        let cards_h = 56u32;
        let cards_row = Rect::new(
            summary.x + 12,
            summary.y + 32,
            summary.w.saturating_sub(24),
            cards_h,
        );
        let mut card_boxes = [
            LayoutBox::new(Rect::new(0, 0, 0, cards_h)).with_sizing(fill_sizing()),
            LayoutBox::new(Rect::new(0, 0, 0, cards_h)).with_sizing(fill_sizing()),
        ];
        let _ = Row::new(cards_row).with_gap(12).arrange(&mut card_boxes);
        let cpu_card = card_boxes[0].bounds();
        let ram_card = card_boxes[1].bounds();
        let mem_breakdown = Rect::new(
            summary.x + 12,
            ram_card.bottom() + 6,
            summary.w.saturating_sub(24),
            summary
                .bottom()
                .saturating_sub(ram_card.bottom() + 10)
                .max(0) as u32,
        );

        TasksLayout {
            root,
            content,
            title: title_children[0].bounds(),
            brand: title_children[2].bounds(),
            toolbar,
            actions: [
                action_boxes[0].bounds(),
                action_boxes[1].bounds(),
                action_boxes[2].bounds(),
                action_boxes[3].bounds(),
            ],
            summary,
            cpu_card,
            ram_card,
            mem_breakdown,
            table,
            status,
        }
    }

    fn ensure_layout(&mut self) -> bool {
        if !self.layout_invalidation.update(self.client_bounds) {
            return false;
        }
        self.layout = Self::compute_layout(self.client_bounds);
        self.clamp_scroll();
        true
    }

    fn set_client_bounds(&mut self, width: u32, height: u32) -> bool {
        let bounds = Rect::new(0, 0, width, height);
        if bounds == self.client_bounds {
            return false;
        }
        self.client_bounds = bounds;
        self.layout_invalidation.invalidate();
        self.ensure_layout()
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
        if target == 0 {
            self.set_status("Refused: protected pid");
            return;
        }
        if self.confirm_pid != Some(target) {
            self.confirm_pid = Some(target);
            let system = self.snapshot.procs[..self.snapshot.proc_count]
                .iter()
                .find(|p| p.pid == target)
                .is_some_and(|p| {
                    p.pid == 1 || p.name_str().starts_with("sunlight") || p.name_str() == "init"
                });
            self.set_status(if system {
                "System process: Delete confirms termination; Esc cancels"
            } else {
                "Confirm End task: press Delete again (Esc cancels)"
            });
            return;
        }
        self.confirm_pid = None;
        let self_pid = sunlight_ipc::getpid();
        if u64::from(target) == self_pid {
            self.set_status("Refused: cannot end self");
            return;
        }
        self.toast_until_ms = sunlight_ipc::monotonic_millis().saturating_add(3000);
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
        let now = sunlight_ipc::monotonic_millis();
        if self.paused || (!force && now.saturating_sub(self.last_poll_ms) < self.interval_ms) {
            return false;
        }
        self.last_poll_ms = now;
        let changed = self.telemetry.poll();
        if !changed
            && !self.paused
            && now.saturating_sub(self.last_update_ms) > self.interval_ms * 3
        {
            self.snapshot = SystemSnapshot::default();
            self.row_count = 0;
            self.selected_pid = None;
            self.set_status("Telemetry degraded: no current sample");
            return true;
        }
        if changed || force {
            let old_identity = self.selected_pid.and_then(|pid| {
                self.snapshot.procs[..self.snapshot.proc_count]
                    .iter()
                    .find(|p| p.pid == pid)
                    .map(|p| (p.pid, p.generation))
            });
            self.snapshot = *self.telemetry.snapshot();
            if old_identity.is_some_and(|id| {
                !self.snapshot.procs[..self.snapshot.proc_count]
                    .iter()
                    .any(|p| (p.pid, p.generation) == id)
            }) {
                self.selected_pid = None;
                self.confirm_pid = None;
            }
            if changed {
                self.last_update_ms = now;
                self.record_history();
                if self.selected_pid.is_some_and(|pid| {
                    !self.snapshot.procs[..self.snapshot.proc_count]
                        .iter()
                        .any(|p| p.pid == pid)
                }) {
                    self.selected_pid = None;
                    self.confirm_pid = None;
                }
            }
            self.rebuild_rows();
            self.rebuild_core_rows();
            self.rebuild_hw_info();
            self.clamp_scroll();
            return true;
        }
        false
    }

    fn rebuild_rows(&mut self) {
        self.row_count = 0;
        for i in 0..self.snapshot.proc_count.min(MAX_PROCESSES) {
            let p = &self.snapshot.procs[i];
            let mut pid = [0; 16];
            let n = write_num_into(p.pid, &mut pid);
            let query = &self.query[..self.query_len];
            if contains_ascii(p.name_str().as_bytes(), query)
                || contains_ascii(&p.metrics.app_id, query)
                || contains_ascii(&pid[..n], query)
            {
                self.order[self.row_count] = i;
                self.row_count += 1;
            }
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

        self.row_group.fill(false);
        if self.grouping == 2 {
            let sorted = self.order;
            let count = self.row_count;
            self.row_count = 0;
            let mut done = [false; MAX_PROCESSES];
            for i in 0..count {
                let index = sorted[i];
                if done[index] {
                    continue;
                }
                let key = self.snapshot.procs[index].metrics.app_id;
                self.order[self.row_count] = index;
                self.row_group[self.row_count] = true;
                self.row_count += 1;
                let collapsed = self.collapsed.contains(&key);
                for j in i..count {
                    let member = sorted[j];
                    if self.snapshot.procs[member].metrics.app_id == key {
                        done[member] = true;
                        if !collapsed {
                            self.order[self.row_count] = member;
                            self.row_count += 1;
                        }
                    }
                }
            }
        } else if self.grouping == 1 {
            let sorted = self.order;
            let count = self.row_count;
            let mut emitted = [false; MAX_PROCESSES];
            self.row_count = 0;
            // Stable parent-before-child traversal, including orphans/cycles.
            for _ in 0..MAX_PROCESSES {
                let mut progress = false;
                for index in &sorted[..count] {
                    if emitted[*index] {
                        continue;
                    }
                    let p = &self.snapshot.procs[*index];
                    let parent = sorted[..count]
                        .iter()
                        .find(|j| self.snapshot.procs[**j].pid == p.ppid);
                    if parent.is_none_or(|j| emitted[*j]) {
                        self.order[self.row_count] = *index;
                        self.row_count += 1;
                        emitted[*index] = true;
                        progress = true;
                    }
                }
                if !progress {
                    break;
                }
            }
            for index in &sorted[..count] {
                if !emitted[*index] {
                    self.order[self.row_count] = *index;
                    self.row_count += 1;
                }
            }
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
                proc.cpu_bp,
                &mut self.row_bufs[row][3],
                &mut self.row_lens[row][3],
            );
            write_kib(
                (proc.metrics.private_bytes / 1024).min(u32::MAX as u64) as u32,
                &mut self.row_bufs[row][4],
                &mut self.row_lens[row][4],
            );
            write_kib(
                (proc.metrics.shared_bytes / 1024).min(u32::MAX as u64) as u32,
                &mut self.row_bufs[row][5],
                &mut self.row_lens[row][5],
            );
            write_kib(
                (proc.metrics.mapped_bytes / 1024).min(u32::MAX as u64) as u32,
                &mut self.row_bufs[row][6],
                &mut self.row_lens[row][6],
            );
            write_str(
                core::str::from_utf8(trim_zeros(&proc.metrics.app_id)).unwrap_or("?"),
                &mut self.row_bufs[row][7],
                &mut self.row_lens[row][7],
            );
            if self.snapshot.accounting_version < 5 {
                for col in [4, 5, 6, 7] {
                    write_str(
                        "unavailable",
                        &mut self.row_bufs[row][col],
                        &mut self.row_lens[row][col],
                    );
                }
            }
            if self.row_group[row] {
                let mut cpu = 0u32;
                let mut private = 0u64;
                let mut shared = 0u64;
                let mut mapped = 0u64;
                for p in &self.snapshot.procs[..self.snapshot.proc_count] {
                    if p.metrics.app_id == proc.metrics.app_id {
                        cpu += p.cpu_bp as u32;
                        private += p.metrics.private_bytes;
                        shared += p.metrics.shared_bytes;
                        mapped += p.metrics.mapped_bytes;
                    }
                }
                let name = &mut self.row_bufs[row][1];
                let mut n = copy_tail(
                    if self.collapsed.contains(&proc.metrics.app_id) {
                        b"> "
                    } else {
                        b"v "
                    },
                    name,
                );
                n += copy_tail(trim_zeros(&proc.metrics.app_id), &mut name[n..]);
                self.row_lens[row][1] = n;
                write_str(
                    "group",
                    &mut self.row_bufs[row][0],
                    &mut self.row_lens[row][0],
                );
                write_str("", &mut self.row_bufs[row][2], &mut self.row_lens[row][2]);
                write_pct(
                    cpu.min(10000) as u16,
                    &mut self.row_bufs[row][3],
                    &mut self.row_lens[row][3],
                );
                for (col, bytes) in [(4, private), (5, shared), (6, mapped)] {
                    self.row_lens[row][col] = write_bytes_into(bytes, &mut self.row_bufs[row][col]);
                }
            } else if self.grouping != 0 {
                let mut name = [0; CELL_BUF];
                let mut depth = 1;
                if self.grouping == 1 {
                    let mut parent = proc.ppid;
                    for _ in 0..8 {
                        if let Some(p) = self.snapshot.procs[..self.snapshot.proc_count]
                            .iter()
                            .find(|p| p.pid == parent && p.pid != proc.pid)
                        {
                            depth += 1;
                            parent = p.ppid;
                        } else {
                            break;
                        }
                    }
                }
                let indent = depth.min(8) * 2;
                name[..indent].fill(b' ');
                let n = indent + copy_tail(proc.name_str().as_bytes(), &mut name[indent..]);
                self.row_bufs[row][1] = name;
                self.row_lens[row][1] = n;
            }
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
            let context_switches = self.snapshot.cpu_telemetry.cores[i].switches_per_second;

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
                load_bp,
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
        let cmp = match self.sort_col {
            0 => left.pid.cmp(&right.pid),
            1 => left.name_str().cmp(right.name_str()),
            2 => (left.state as u8).cmp(&(right.state as u8)),
            4 => left.metrics.private_bytes.cmp(&right.metrics.private_bytes),
            5 => left.metrics.shared_bytes.cmp(&right.metrics.shared_bytes),
            6 => left.metrics.mapped_bytes.cmp(&right.metrics.mapped_bytes),
            7 => left.metrics.app_id.cmp(&right.metrics.app_id),
            _ => left.cpu_bp.cmp(&right.cpu_bp),
        };
        if cmp == core::cmp::Ordering::Equal {
            left.pid < right.pid
        } else if self.descending {
            cmp.is_gt()
        } else {
            cmp.is_lt()
        }
    }

    fn clamp_scroll(&mut self) {
        let total = match self.view_mode {
            ViewMode::Processes => self.row_count,
            ViewMode::Cores => self.core_row_count,
        };
        self.scroll = Self::clamp_offset(self.scroll, total, self.visible_rows());
    }

    fn clamp_offset(scroll: usize, total: usize, visible: usize) -> usize {
        scroll.min(total.saturating_sub(visible.max(1)))
    }

    fn visible_rows(&self) -> usize {
        visible_rows_in(self.table_rect())
    }

    fn action_rect(&self, index: usize) -> Rect {
        self.layout.actions.get(index).copied().unwrap_or_default()
    }

    fn action_at(&self, x: i32, y: i32) -> Option<usize> {
        (0..ACTION_WIDTHS.len()).find(|index| self.action_rect(*index).contains(Point::new(x, y)))
    }

    fn summary_rect(&self) -> Rect {
        self.layout.summary
    }

    fn summary_card_rects(&self) -> (Rect, Rect) {
        (self.layout.cpu_card, self.layout.ram_card)
    }

    fn mem_breakdown_rect(&self) -> Rect {
        self.layout.mem_breakdown
    }

    fn table_rect(&self) -> Rect {
        let mut rect = self.layout.table;
        if self.details && self.selected_pid.is_some() && self.view_mode == ViewMode::Processes {
            rect.h = rect.h.saturating_sub(112);
        }
        rect
    }

    fn status_bar_rect(&self) -> Rect {
        self.layout.status
    }

    fn process_columns(&self) -> [Column<'static>; TABLE_COLS] {
        core::array::from_fn(|i| {
            let id = self.col_order[i];
            Column {
                header: TABLE_COLUMNS[id].header,
                width: self.widths[id],
                right_align: TABLE_COLUMNS[id].right_align,
            }
        })
    }

    fn core_columns(&self) -> [Column<'static>; CORE_TABLE_COLS] {
        columns_with_fill(CORE_COLUMNS, 2, self.layout.table.w)
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

    fn draw_memory_breakdown(&self, canvas: &mut sunlight_ui::Canvas, theme: &Theme) {
        let rect = self.mem_breakdown_rect();
        if rect.h < 20 {
            return;
        }
        let acct = &self.snapshot.mem_acct;
        let task_n = if acct.active_task_count > 0 {
            acct.active_task_count
        } else {
            self.snapshot.proc_count as u32
        };

        // Line 1: unique task private + shared + kernel + page tables
        let mut line1 = [0u8; 120];
        let mut n = 0usize;
        n += copy_tail(b"Tasks&svc ", &mut line1[n..]);
        n += write_mib_bytes_into(acct.task_private_unique_bytes, &mut line1[n..]);
        n += copy_tail(b"  Shared ", &mut line1[n..]);
        n += write_mib_bytes_into(acct.shared_memory_unique_bytes, &mut line1[n..]);
        n += copy_tail(b"  Kernel ", &mut line1[n..]);
        n += write_mib_bytes_into(acct.kernel_total_bytes(), &mut line1[n..]);
        n += copy_tail(b"  PT ", &mut line1[n..]);
        n += write_mib_bytes_into(acct.page_table_bytes, &mut line1[n..]);
        let s1 = core::str::from_utf8(&line1[..n]).unwrap_or("");
        F_SMALL.draw_vcenter(canvas, s1, rect.x, rect.y, 14, theme.text_muted);

        // Line 2: RAMFS + cache + graphics + other/unclassified + free
        let mut line2 = [0u8; 256];
        n = 0;
        n += copy_tail(b"RAMFS ", &mut line2[n..]);
        n += write_mib_bytes_into(
            acct.ramfs_file_data_bytes
                .saturating_add(if acct.ramfs_metadata_available() {
                    acct.ramfs_metadata_bytes
                } else {
                    0
                }),
            &mut line2[n..],
        );
        n += copy_tail(b"  Cache ", &mut line2[n..]);
        n += write_mib_bytes_into(acct.cache_total_bytes(), &mut line2[n..]);
        n += copy_tail(b"  Gfx/dev ", &mut line2[n..]);
        n += write_mib_bytes_into(acct.graphics_and_device_bytes(), &mut line2[n..]);
        n += copy_tail(b"  Other ", &mut line2[n..]);
        n += write_mib_bytes_into(
            acct.other_accounted_bytes
                .saturating_add(acct.unclassified_bytes),
            &mut line2[n..],
        );
        n += copy_tail(b"  ZRAM ", &mut line2[n..]);
        n += write_mib_bytes_into(acct.zram_physical_bytes, &mut line2[n..]);
        n += copy_tail(b"  Free ", &mut line2[n..]);
        n += write_mib_bytes_into(acct.free_bytes, &mut line2[n..]);
        let s2 = core::str::from_utf8(&line2[..n]).unwrap_or("");
        F_SMALL.draw_vcenter(canvas, s2, rect.x, rect.y + 14, 14, theme.text_dim);

        // A disjoint physical breakdown; shared mapped row bytes never enter it.
        let categories = [
            acct.task_private_unique_bytes,
            acct.shared_memory_unique_bytes,
            acct.kernel_total_bytes(),
            acct.page_table_bytes,
            acct.ramfs_file_data_bytes
                .saturating_add(if acct.ramfs_metadata_available() {
                    acct.ramfs_metadata_bytes
                } else {
                    0
                }),
            acct.cache_total_bytes(),
            acct.graphics_and_device_bytes(),
            acct.zram_physical_bytes,
            acct.other_accounted_bytes
                .saturating_add(acct.unclassified_bytes),
            acct.free_bytes,
        ];
        let colors = [
            theme.accent,
            theme.warn,
            theme.text_muted,
            theme.text_dim,
            theme.accent.darken(80),
            theme.warn.darken(80),
            theme.danger,
            theme.danger.darken(80),
            theme.border,
            theme.bg,
        ];
        let mut x = rect.x;
        for (i, bytes) in categories.iter().enumerate() {
            let w = ((*bytes as u128 * rect.w as u128) / acct.managed_bytes.max(1) as u128)
                .min(rect.right().saturating_sub(x).max(0) as u128) as u32;
            canvas.fill_rect(Rect::new(x, rect.bottom() - 5, w, 4), colors[i]);
            x += w as i32;
        }
        // Line 3: task count + honesty notes
        let mut line3 = [0u8; 120];
        n = 0;
        n += copy_tail(b"Tasks: ", &mut line3[n..]);
        n += write_num_into(task_n, &mut line3[n..]);
        n += copy_tail(
            b"  Unique private physical (shared counted once)",
            &mut line3[n..],
        );
        if acct.unclassified_bytes > 0 {
            n += copy_tail(b"  Unclass>0", &mut line3[n..]);
        }
        let s3 = core::str::from_utf8(&line3[..n]).unwrap_or("");
        F_SMALL.draw_vcenter(canvas, s3, rect.x, rect.y + 28, 14, theme.text_dim);
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
        // Denser card glass over the transparent WindowGlass root.
        canvas.fill_material(rect, Material::card(theme).with_radius(9).without_border());
        canvas.stroke_rounded_rect(rect, 9, 1, theme.chrome.subtle_border);

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
        if self.client_bounds.size() != Size::new(canvas.width, canvas.height) {
            let _ = self.set_client_bounds(canvas.width, canvas.height);
        } else {
            let _ = self.ensure_layout();
        }
        // Root stays transparent so compositor WindowGlass is the base density.
        canvas.clear_transparent(self.layout.root);

        let materials = MaterialPalette::new(theme);

        Label::new(self.layout.title, "Tasks Monitor")
            .with_font(&F_MED)
            .draw(canvas, theme);
        Label::new(
            self.layout.brand,
            match self.grouping {
                1 => "Tree",
                2 => "Apps",
                _ => "Flat",
            },
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

        let mut search = [0; 96];
        let mut n = copy_tail(
            if self.search_focus {
                b"Search: "
            } else {
                b"Ctrl+F: "
            },
            &mut search,
        );
        n += copy_tail(&self.query[..self.query_len], &mut search[n..]);
        let search_x = self.layout.actions[3].right() + 12;
        let search_rect = Rect::new(
            search_x,
            self.layout.toolbar.y,
            self.layout.toolbar.right().saturating_sub(search_x).max(0) as u32,
            self.layout.toolbar.h,
        );
        canvas.fill_rounded_rect(search_rect, 4, theme.bg);
        F_SMALL.draw_vcenter(
            canvas,
            core::str::from_utf8(&search[..n]).unwrap_or(""),
            search_x + 4,
            search_rect.y,
            search_rect.h,
            theme.text_muted,
        );
        let summary = self.summary_rect();
        canvas.fill_material(
            summary,
            materials.card_glass.with_radius(11).without_border(),
        );
        canvas.stroke_rounded_rect(summary, 11, 1, theme.chrome.subtle_border);
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
        cpu_detail_len += copy_tail(b" | ", &mut cpu_detail[cpu_detail_len..]);
        cpu_detail_len += write_num_into(
            self.snapshot.cpu_count as u32,
            &mut cpu_detail[cpu_detail_len..],
        );
        cpu_detail_len += copy_tail(b" cores", &mut cpu_detail[cpu_detail_len..]);
        let cpu_detail_str = core::str::from_utf8(&cpu_detail[..cpu_detail_len]).unwrap_or("");

        let acct = &self.snapshot.mem_acct;
        let usable_kb = if acct.usable_bytes > 0 {
            acct.usable_bytes / 1024
        } else {
            self.snapshot.total_ram_kb
        };
        let used_kb = if acct.managed_bytes > 0 {
            acct.used_bytes() / 1024
        } else {
            self.snapshot.used_ram_kb
        };
        let ram_usage_bp = ram_usage_bp(used_kb, usable_kb);
        let mut ram_value = [0u8; 16];
        let ram_value_len = write_bp_into(ram_usage_bp, &mut ram_value);
        let ram_value_str = core::str::from_utf8(&ram_value[..ram_value_len]).unwrap_or("");
        let mut ram_detail = [0u8; 48];
        let mut ram_detail_len = 0usize;
        ram_detail_len += write_mb_into(used_kb, &mut ram_detail[ram_detail_len..]);
        ram_detail_len += copy_tail(b" used of ", &mut ram_detail[ram_detail_len..]);
        ram_detail_len += write_mb_into(usable_kb, &mut ram_detail[ram_detail_len..]);
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

        // Physical memory accounting breakdown (unique classes, not residual cache).
        self.draw_memory_breakdown(canvas, theme);

        let table_rect = self.table_rect();
        // Dense readable surface for the process/core table.
        canvas.fill_material(
            table_rect,
            materials.tinted_content.with_radius(8).without_border(),
        );
        canvas.stroke_rounded_rect(table_rect, 8, 1, theme.chrome.subtle_border);
        let visible = self.visible_rows();

        match self.view_mode {
            ViewMode::Processes => {
                self.draw_process_table(canvas, theme);
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
                let columns = self.core_columns();
                Table::new(table_rect, &columns, &core_refs[start..end])
                    .with_font(&F_UI)
                    .draw(canvas, theme);
            }
        }

        if self.confirm_pid.is_some() {
            let rect = Rect::new(
                self.layout.content.x + 16,
                self.layout.status.y - 38,
                self.layout.content.w.saturating_sub(32),
                32,
            );
            canvas.fill_rounded_rect(rect, 6, theme.panel_alt);
            canvas.stroke_rounded_rect(rect, 6, 1, theme.danger);
            F_UI.draw_vcenter(
                canvas,
                "End selected task? Delete confirms · Esc cancels",
                rect.x + 8,
                rect.y,
                rect.h,
                theme.danger,
            );
        }
        self.draw_details(canvas, theme);
        if self.toast_until_ms > sunlight_ipc::monotonic_millis() {
            let rect = Rect::new(
                self.layout.content.x + 16,
                self.layout.status.y - 38,
                self.layout.content.w.saturating_sub(32),
                32,
            );
            canvas.fill_rounded_rect(rect, 6, theme.panel_alt);
            canvas.stroke_rounded_rect(rect, 6, 1, theme.accent);
            F_UI.draw_vcenter(
                canvas,
                self.status_str(),
                rect.x + 8,
                rect.y,
                rect.h,
                theme.text,
            );
        }
        // Per-core bars fit between the overview title and the resource cards.
        let core_n = self.snapshot.cpu_telemetry.count.max(1);
        let core_w = self.layout.summary.w.saturating_sub(24) / core_n as u32;
        for i in 0..self.snapshot.cpu_telemetry.count {
            let load = self.snapshot.cpu_telemetry.cores[i].load_bp;
            let rect = Rect::new(
                self.layout.summary.x + 12 + (i as u32 * core_w) as i32,
                self.layout.summary.y + 25,
                core_w.saturating_sub(2),
                4,
            );
            canvas.fill_rect(rect, theme.bg);
            canvas.fill_rect(
                Rect::new(rect.x, rect.y, rect.w * load as u32 / 10000, rect.h),
                self.cell_color(theme, load),
            );
        }
        let mut status_meta = [0; 120];
        let healthy = self.snapshot.accounting_version >= 5
            && self.last_update_ms != 0
            && sunlight_ipc::monotonic_millis().saturating_sub(self.last_update_ms)
                <= self.interval_ms * 3;
        let mut n = copy_tail(
            if healthy || self.paused {
                b"Telemetry ready | "
            } else {
                b"Telemetry degraded | "
            },
            &mut status_meta,
        );
        n += write_num_into(self.interval_ms as u32, &mut status_meta[n..]);
        n += copy_tail(b"ms | update ", &mut status_meta[n..]);
        n += write_num_into((self.last_update_ms / 1000) as u32, &mut status_meta[n..]);
        n += copy_tail(
            if self.paused { b"s | paused" } else { b"s" },
            &mut status_meta[n..],
        );
        StatusBar::new(
            self.status_bar_rect(),
            self.status_str(),
            core::str::from_utf8(&status_meta[..n]).unwrap_or(""),
            self.hw_info_str(),
        )
        .draw(canvas, theme);
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::Key(ch) if self.search_focus && ch.is_ascii() && !ch.is_control() => {
                if self.query_len < self.query.len() {
                    self.query[self.query_len] = ch as u8;
                    self.query_len += 1;
                }
                self.scroll = 0;
                self.rebuild_rows();
                true
            }
            Event::Tick => self.refresh(false),
            Event::MouseMove { x, y } => {
                if let Some((col, origin, resize)) = self.drag {
                    if resize {
                        let id = self.col_order[col];
                        self.widths[id] =
                            (self.widths[id] as i32 + x - origin).clamp(45, 400) as u32;
                        self.drag = Some((col, x, resize));
                        return true;
                    }
                }
                let hovered = self.action_at(x, y);
                if hovered != self.hovered_action {
                    self.hovered_action = hovered;
                    return true;
                }
                false
            }
            Event::MouseDown { x, y, button: 0 } => {
                if let Some((col, edge)) = self.header_at(x, y) {
                    self.drag = Some((col, x, edge));
                    return true;
                }
                let pressed = self.action_at(x, y);
                if pressed.is_some() {
                    self.pressed_action = pressed;
                    self.hovered_action = pressed;
                    return true;
                }
                false
            }
            Event::MouseUp { x, y, .. } => {
                if let Some((col, origin, resize)) = self.drag.take() {
                    if !resize {
                        if let Some((to, _)) = self.header_at(x, y) {
                            if (x - origin).abs() > 8 {
                                self.col_order.swap(col, to);
                            } else {
                                let id = self.col_order[col];
                                self.descending = if id == self.sort_col {
                                    !self.descending
                                } else {
                                    false
                                };
                                self.sort_col = id;
                                self.rebuild_rows();
                            }
                        }
                    }
                    self.save_columns();
                    return true;
                }
                if self.pressed_action.take().is_some() {
                    return true;
                }
                false
            }
            Event::Click { x, y } => {
                if self.layout.brand.contains(Point::new(x, y)) {
                    self.grouping = (self.grouping + 1) % 3;
                    self.scroll = 0;
                    self.rebuild_rows();
                    return true;
                }
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
                            self.details = !self.details;
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

                if x > self.layout.actions[3].right()
                    && y >= self.layout.toolbar.y
                    && y < self.layout.toolbar.bottom()
                {
                    self.search_focus = true;
                    return true;
                }
                if self.view_mode == ViewMode::Processes {
                    let columns = self.process_columns();
                    let table = Table::new(self.table_rect(), &columns, &[]).with_font(&F_UI);
                    if let Some(row) = table.hit_test(x, y) {
                        let idx = self.scroll + row;
                        if idx < self.row_count {
                            let proc = &self.snapshot.procs[self.order[idx]];
                            if self.row_group[idx] {
                                let key = proc.metrics.app_id;
                                if let Some(i) = self.collapsed.iter().position(|v| *v == key) {
                                    self.collapsed[i] = [0; 32];
                                } else if let Some(i) =
                                    self.collapsed.iter().position(|v| *v == [0; 32])
                                {
                                    self.collapsed[i] = key;
                                }
                                self.rebuild_rows();
                                return true;
                            }
                            self.confirm_pid = None;
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
                    KEY_UP => self.move_selection(false),
                    KEY_DOWN => self.move_selection(true),
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
                    KEY_ENTER => {
                        self.details = !self.details;
                        self.search_focus = false;
                    }
                    0x53 => self.end_task(),
                    0x3F => {
                        self.refresh(true);
                    }
                    0x01 => {
                        self.confirm_pid = None;
                        self.search_focus = false;
                        self.query_len = 0;
                        self.rebuild_rows();
                    }
                    0x21 if ctrl => self.search_focus = true,
                    0x22 if ctrl => {
                        self.grouping = (self.grouping + 1) % 3;
                        self.scroll = 0;
                        self.rebuild_rows();
                        self.set_status(match self.grouping {
                            1 => "Parent tree",
                            2 => "App groups: click group to expand/collapse",
                            _ => "All processes",
                        });
                    }
                    0x19 if ctrl => {
                        self.paused = !self.paused;
                        self.telemetry.set_paused(self.paused);
                    }
                    0x0D if ctrl => {
                        self.interval_ms = (self.interval_ms * 2).min(60000);
                        self.telemetry.set_interval_ms(self.interval_ms);
                    }
                    0x0C if ctrl => {
                        self.interval_ms = (self.interval_ms / 2).max(1000);
                        self.telemetry.set_interval_ms(self.interval_ms);
                    }
                    KEY_BACKSPACE if self.search_focus => {
                        self.query_len = self.query_len.saturating_sub(1);
                        self.rebuild_rows();
                    }
                    _ => return false,
                }
                self.clamp_scroll();
                true
            }
            _ => false,
        }
    }

    fn window_event(&mut self, event: WindowEvent) -> bool {
        let WindowEvent::Resized { width, height } = event;
        self.set_client_bounds(width, height)
    }
}

#[cfg(not(test))]
#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8, _envp: *const *const u8) -> ! {
    sunlight_libc::env::init(_envp);
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
            Telemetry::unavailable()
        }
    };

    let mut app = TasksApp::new(telemetry);
    let mut window = match Window::connect_with_material(
        WindowConfig {
            width: WIN_W,
            height: WIN_H,
            title: "Tasks Monitor",
            decoration: sunlight_ui::WindowDecoration::Normal,
        },
        WindowMaterial::WindowGlass,
    ) {
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

fn write_pct(value: u16, dst: &mut [u8; CELL_BUF], len: &mut usize) {
    *len = write_bp_into(value, dst);
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
    if value > 0 && value < 10 {
        return copy_tail(b"<0.1%", dst);
    }
    if dst.len() < 6 {
        return 0;
    }
    let mut n = write_num_into((value / 100) as u32, dst);
    if value % 100 != 0 {
        dst[n] = b'.';
        dst[n + 1] = b'0' + ((value % 100) / 10) as u8;
        n += 2;
    }
    dst[n] = b'%';
    n + 1
}

fn write_mb_into(kb: u64, dst: &mut [u8]) -> usize {
    write_bytes_into(kb.saturating_mul(1024), dst)
}
fn write_bytes_into(bytes: u64, dst: &mut [u8]) -> usize {
    let (unit, label) = if bytes >= 1 << 30 {
        (1 << 30, b" GiB")
    } else if bytes >= 1 << 20 {
        (1 << 20, b" MiB")
    } else {
        (1024, b" KiB")
    };
    let mut n = write_num_into((bytes / unit).min(u32::MAX as u64) as u32, dst);
    if n + 2 <= dst.len() {
        dst[n] = b'.';
        dst[n + 1] = b'0' + ((bytes % unit) * 10 / unit) as u8;
        n += 2;
    }
    n + copy_tail(label, &mut dst[n..])
}
fn write_mib_bytes_into(bytes: u64, dst: &mut [u8]) -> usize {
    write_bytes_into(bytes, dst)
}

fn copy_tail_mb(kb: u64, dst: &mut [u8]) -> usize {
    write_mb_into(kb, dst)
}

fn trim_zeros(bytes: &[u8]) -> &[u8] {
    let len = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    &bytes[..len]
}

fn table_row_metrics() -> (u32, u32) {
    let line_h = F_UI.line_height();
    (line_h.saturating_add(8), line_h.saturating_add(5))
}

fn visible_rows_in(table: Rect) -> usize {
    let (header_h, row_h) = table_row_metrics();
    let usable = table.h.saturating_sub(header_h);
    if row_h == 0 {
        return 1;
    }
    (usable / row_h).max(1) as usize
}

fn columns_with_fill<const N: usize>(
    base: [Column<'static>; N],
    fill_idx: usize,
    table_w: u32,
) -> [Column<'static>; N] {
    let mut cols = base;
    let mut fixed = 0u32;
    for (idx, col) in cols.iter().enumerate() {
        if idx != fill_idx {
            fixed = fixed.saturating_add(col.width);
        }
    }
    if fill_idx < N {
        cols[fill_idx].width = table_w.saturating_sub(fixed);
    }
    cols
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toolbar_follows_root_width_and_body_fills_remaining_height() {
        let layout = TasksApp::compute_layout(Rect::new(0, 0, WIN_W, WIN_H));
        assert_eq!(layout.root, Rect::new(0, 0, WIN_W, WIN_H));
        assert_eq!(layout.toolbar.w, WIN_W.saturating_sub(24));
        assert_eq!(layout.summary.w, layout.toolbar.w);
        assert_eq!(layout.table.w, layout.content.w);
        assert_eq!(layout.status.w, layout.content.w);
        assert_eq!(layout.status.bottom(), layout.content.bottom());
        assert!(layout.table.h > 0);
        assert_eq!(layout.table.y, layout.summary.bottom() + 8);
    }

    #[test]
    fn task_viewport_grows_on_resize() {
        let initial = TasksApp::compute_layout(Rect::new(0, 0, WIN_W, WIN_H));
        let wider = TasksApp::compute_layout(Rect::new(0, 0, WIN_W + 180, WIN_H));
        let taller = TasksApp::compute_layout(Rect::new(0, 0, WIN_W, WIN_H + 120));
        assert_eq!(wider.toolbar.w, initial.toolbar.w + 180);
        assert_eq!(wider.table.w, initial.table.w + 180);
        assert_eq!(taller.table.h, initial.table.h + 120);
        assert_eq!(wider.actions[0].w, ACTION_WIDTHS[0]);
        assert_eq!(taller.toolbar.h, TOOLBAR_H);
        assert_eq!(taller.summary.h, SUMMARY_H);
    }

    #[test]
    fn fixed_columns_stay_stable_and_name_consumes_extra_width() {
        let narrow = columns_with_fill(TABLE_COLUMNS, 1, 1000);
        let wide = columns_with_fill(TABLE_COLUMNS, 1, 1200);
        assert_eq!(narrow[0].width, 60);
        assert_eq!(narrow[2].width, 90);
        assert_eq!(narrow[3].width, 80);
        assert_eq!(narrow[4].width, 120);
        assert_eq!(wide[0].width, 60);
        assert_eq!(wide[2].width, 90);
        assert_eq!(wide[1].width, narrow[1].width + 200);
        let core = columns_with_fill(CORE_COLUMNS, 2, 800);
        assert_eq!(core[0].width, 50);
        assert_eq!(core[3].width, 48);
        assert!(core[2].width > CORE_COLUMNS[2].width);
    }

    #[test]
    fn visible_row_geometry_responds_to_height() {
        let short = TasksApp::compute_layout(Rect::new(0, 0, WIN_W, 420));
        let tall = TasksApp::compute_layout(Rect::new(0, 0, WIN_W, 820));
        assert!(visible_rows_in(tall.table) > visible_rows_in(short.table));
    }

    #[test]
    fn scroll_clamps_to_new_viewport_without_resetting_valid_offset() {
        assert_eq!(TasksApp::clamp_offset(3, 20, 10), 3);
        assert_eq!(TasksApp::clamp_offset(18, 20, 10), 10);
        assert_eq!(TasksApp::clamp_offset(0, 20, 40), 0);
        let tall = TasksApp::compute_layout(Rect::new(0, 0, WIN_W, WIN_H + 200));
        let tiny = TasksApp::compute_layout(Rect::new(0, 0, 320, 180));
        let tall_visible = visible_rows_in(tall.table);
        let tiny_visible = visible_rows_in(tiny.table);
        assert!(tiny_visible <= tall_visible);
        assert_eq!(
            TasksApp::clamp_offset(4, 20, tall_visible),
            4.min(20usize.saturating_sub(tall_visible.max(1)))
        );
    }

    #[test]
    fn tiny_dimensions_are_safe_and_repeated_layout_is_deterministic() {
        for bounds in [
            Rect::new(0, 0, 0, 0),
            Rect::new(0, 0, 1, 1),
            Rect::new(0, 0, 80, 40),
        ] {
            let layout = TasksApp::compute_layout(bounds);
            assert_eq!(layout.root, bounds);
            let _ = visible_rows_in(layout.table);
            let _ = columns_with_fill(TABLE_COLUMNS, 1, layout.table.w);
        }
        let bounds = Rect::new(0, 0, 900, 640);
        assert_eq!(
            TasksApp::compute_layout(bounds),
            TasksApp::compute_layout(bounds)
        );
    }
}
fn contains_ascii(haystack: &[u8], needle: &[u8]) -> bool {
    needle.is_empty()
        || haystack
            .windows(needle.len())
            .any(|s| s.eq_ignore_ascii_case(needle))
}

impl TasksApp {
    fn settings_path(out: &mut [u8; 256]) -> usize {
        let home = sunlight_libc::env::getenv_bytes(b"HOME").unwrap_or(b"/root");
        let n = copy_tail(home, out);
        n + copy_tail(b"/.sunlight-tasks.columns", &mut out[n..])
    }
    #[cfg(not(test))]
    fn load_columns(&mut self) {
        let mut path = [0; 256];
        let n = Self::settings_path(&mut path);
        let Ok(fd) = sunlight_libc::open(&path[..n]) else {
            return;
        };
        let mut bytes = [0; 44];
        let read = sunlight_libc::read(fd, &mut bytes);
        let _ = sunlight_libc::close(fd);
        if read != Ok(bytes.len()) || &bytes[..4] != b"TSK1" {
            return;
        }
        let mut seen = 0u16;
        for i in 0..TABLE_COLS {
            let id = bytes[4 + i] as usize;
            let width = u32::from_le_bytes(bytes[12 + i * 4..16 + i * 4].try_into().unwrap());
            if id >= TABLE_COLS || seen & (1 << id) != 0 || !(45..=400).contains(&width) {
                return;
            }
            seen |= 1 << id;
        }
        for i in 0..TABLE_COLS {
            self.col_order[i] = bytes[4 + i] as usize;
            self.widths[i] = u32::from_le_bytes(bytes[12 + i * 4..16 + i * 4].try_into().unwrap());
        }
    }
    fn save_columns(&self) {
        let mut path = [0; 256];
        let n = Self::settings_path(&mut path);
        let Ok(fd) = sunlight_libc::open_with_flags_mode(
            &path[..n],
            sunlight_libc::O_WRONLY | sunlight_libc::O_CREAT | sunlight_libc::O_TRUNC,
            0o600,
        ) else {
            return;
        };
        let mut bytes = [0; 44];
        bytes[..4].copy_from_slice(b"TSK1");
        for i in 0..TABLE_COLS {
            bytes[4 + i] = self.col_order[i] as u8;
            bytes[12 + i * 4..16 + i * 4].copy_from_slice(&self.widths[i].to_le_bytes());
        }
        let _ = sunlight_libc::write_all(fd, &bytes);
        let _ = sunlight_libc::close(fd);
    }
    fn header_at(&self, x: i32, y: i32) -> Option<(usize, bool)> {
        let rect = self.table_rect();
        if y < rect.y
            || y >= rect.y + table_row_metrics().0 as i32
            || x < rect.x
            || x >= rect.right()
        {
            return None;
        }
        let mut cx = rect.x;
        for i in 0..TABLE_COLS {
            cx += self.widths[self.col_order[i]] as i32;
            if x < cx {
                return Some((i, cx - x < 7));
            }
        }
        None
    }
    fn move_selection(&mut self, down: bool) {
        if self.row_count == 0 {
            return;
        }
        let old = self.selected_pid.and_then(|pid| {
            (0..self.row_count)
                .find(|i| !self.row_group[*i] && self.snapshot.procs[self.order[*i]].pid == pid)
        });
        let candidate = match old {
            Some(i) if down => (i + 1..self.row_count).find(|j| !self.row_group[*j]),
            Some(i) => (0..i).rev().find(|j| !self.row_group[*j]),
            None => (0..self.row_count).find(|j| !self.row_group[*j]),
        };
        let Some(row) = candidate else {
            return;
        };
        self.selected_pid = Some(self.snapshot.procs[self.order[row]].pid);
        self.confirm_pid = None;
        if row < self.scroll {
            self.scroll = row;
        }
        if row >= self.scroll + self.visible_rows() {
            self.scroll = row + 1 - self.visible_rows();
        }
    }
    fn record_history(&mut self) {
        // Keep history by identity, independent of sorting, filtering and PID reuse.
        for slot in 0..MAX_PROCESSES {
            let id = self.history_ids[slot];
            if !self.snapshot.procs[..self.snapshot.proc_count]
                .iter()
                .any(|p| (p.pid, p.generation) == id)
            {
                self.history_ids[slot] = (0, 0);
                self.cpu_history[slot] = [0; 30];
                self.mem_history[slot] = [0; 30];
            }
        }
        for p in &self.snapshot.procs[..self.snapshot.proc_count] {
            let id = (p.pid, p.generation);
            let slot = self
                .history_ids
                .iter()
                .position(|x| *x == id)
                .or_else(|| self.history_ids.iter().position(|x| *x == (0, 0)));
            if let Some(i) = slot {
                self.history_ids[i] = id;
                self.cpu_history[i][self.history_pos] = p.cpu_bp;
                self.mem_history[i][self.history_pos] = p.metrics.private_bytes;
            }
        }
        self.history_pos = (self.history_pos + 1) % 30;
    }
    fn cell_color(&self, theme: &Theme, bp: u16) -> Color {
        if bp >= self.danger_bp {
            theme.danger
        } else if bp >= self.warning_bp {
            theme.warn
        } else {
            theme.text
        }
    }
    fn draw_process_table(&self, canvas: &mut sunlight_ui::Canvas, theme: &Theme) {
        let rect = self.table_rect();
        let (hh, rh) = table_row_metrics();
        canvas.fill_rect(Rect::new(rect.x, rect.y, rect.w, hh), theme.panel_alt);
        let mut x = rect.x;
        for id in self.col_order {
            let width = self.widths[id].min(rect.right().saturating_sub(x).max(0) as u32);
            let header = TABLE_COLUMNS[id].header;
            F_UI.draw_vcenter(canvas, header, x + 4, rect.y, hh, theme.text);
            if id == self.sort_col {
                F_UI.draw_vcenter(
                    canvas,
                    if self.descending { "▼" } else { "▲" },
                    x + width as i32 - 16,
                    rect.y,
                    hh,
                    theme.accent,
                );
            }
            x += width as i32;
        }
        if self.row_count == 0 {
            F_UI.draw_vcenter(
                canvas,
                if self.snapshot.proc_count == 0 {
                    "Telemetry unavailable or no live processes"
                } else {
                    "No matching processes"
                },
                rect.x + 12,
                rect.y + hh as i32 + 12,
                rh,
                theme.text_muted,
            );
        }
        for row in self.scroll..(self.scroll + self.visible_rows()).min(self.row_count) {
            let p = &self.snapshot.procs[self.order[row]];
            let y = rect.y + hh as i32 + ((row - self.scroll) as u32 * rh) as i32;
            canvas.fill_rect(
                Rect::new(rect.x, y, rect.w, rh),
                if self.selected_pid == Some(p.pid) {
                    theme.chrome.selection
                } else if row % 2 == 0 {
                    theme.chrome.window_bg
                } else {
                    theme.chrome.card_bg
                },
            );
            let mut x = rect.x;
            for id in self.col_order {
                let width = self.widths[id].min(rect.right().saturating_sub(x).max(0) as u32);
                if width == 0 {
                    break;
                }
                let text = core::str::from_utf8(&self.row_bufs[row][id][..self.row_lens[row][id]])
                    .unwrap_or("");
                let color = if id == 3 {
                    self.cell_color(theme, p.cpu_bp)
                } else if id == 4 {
                    self.cell_color(
                        theme,
                        ram_usage_bp(
                            p.metrics.private_bytes,
                            self.snapshot.mem_acct.managed_bytes,
                        ),
                    )
                } else {
                    theme.text
                };
                let mut available = width.saturating_sub(if id == 2 { 18 } else { 8 });
                // CPU and private-memory histories: fixed 30 columns, one sample each.
                if (id == 3 || id == 4) && width >= 80 {
                    if let Some(slot) = self
                        .history_ids
                        .iter()
                        .position(|v| *v == (p.pid, p.generation))
                    {
                        let max_mem = self.mem_history[slot]
                            .iter()
                            .copied()
                            .max()
                            .unwrap_or(1)
                            .max(1);
                        for j in 0..30 {
                            let at = (self.history_pos + j) % 30;
                            let bp = if id == 3 {
                                self.cpu_history[slot][at]
                            } else {
                                ram_usage_bp(self.mem_history[slot][at], max_mem)
                            };
                            let h = (bp as u32 * rh.saturating_sub(6) / 10000).max(1);
                            canvas.vline(
                                x + 3 + j as i32,
                                y + rh as i32 - 3 - h as i32,
                                h,
                                theme.accent,
                            );
                        }
                        available = available.saturating_sub(32);
                    }
                }
                let mut end = text.len();
                while end > 0 && F_UI.measure_w(&text[..end]) > available {
                    end -= 1;
                    while !text.is_char_boundary(end) {
                        end -= 1;
                    }
                }
                let text = &text[..end];
                let tx = if TABLE_COLUMNS[id].right_align {
                    x + width as i32 - 4 - F_UI.measure_w(text) as i32
                } else {
                    x + 4
                };
                if id == 2 {
                    let dot = match p.state {
                        ProcessState::Running => theme.accent,
                        ProcessState::Ready => theme.warn,
                        _ => theme.text_muted,
                    };
                    canvas.fill_rounded_rect(
                        Rect::new(x + 4, y + (rh as i32 - 6) / 2, 6, 6),
                        3,
                        dot,
                    );
                    F_UI.draw_vcenter(canvas, text, tx + 10, y, rh, color);
                } else {
                    F_UI.draw_vcenter(canvas, text, tx, y, rh, color);
                }
                x += width as i32;
            }
        }
    }
}

impl TasksApp {
    fn draw_details(&self, canvas: &mut sunlight_ui::Canvas, theme: &Theme) {
        if !self.details || self.view_mode != ViewMode::Processes {
            return;
        }
        let Some(p) = self.snapshot.procs[..self.snapshot.proc_count]
            .iter()
            .find(|p| Some(p.pid) == self.selected_pid)
        else {
            return;
        };
        let rect = Rect::new(
            self.layout.table.x,
            self.table_rect().bottom() + 4,
            self.layout.table.w,
            108,
        );
        canvas.fill_rounded_rect(rect, 6, theme.panel_alt);
        let mut text = [0; 256];
        let mut n = copy_tail(b"PID ", &mut text);
        n += write_num_into(p.pid, &mut text[n..]);
        n += copy_tail(b"  PPID ", &mut text[n..]);
        n += write_num_into(p.ppid, &mut text[n..]);
        n += copy_tail(b"  Name ", &mut text[n..]);
        n += copy_tail(p.name_str().as_bytes(), &mut text[n..]);
        n += copy_tail(b"  App ", &mut text[n..]);
        n += copy_tail(trim_zeros(&p.metrics.app_id), &mut text[n..]);
        F_SMALL.draw_vcenter(
            canvas,
            core::str::from_utf8(&text[..n]).unwrap_or(""),
            rect.x + 8,
            rect.y + 3,
            22,
            theme.text,
        );
        n = copy_tail(b"State ", &mut text);
        n += copy_tail(state_text(p.state).as_bytes(), &mut text[n..]);
        n += copy_tail(b"  CPU ", &mut text[n..]);
        n += write_bp_into(p.cpu_bp, &mut text[n..]);
        n += copy_tail(b"  Threads ", &mut text[n..]);
        n += write_num_into(p.metrics.threads, &mut text[n..]);
        n += copy_tail(b"  Uptime ", &mut text[n..]);
        n += write_num_into(p.metrics.uptime_secs, &mut text[n..]);
        n += copy_tail(b"s", &mut text[n..]);
        F_SMALL.draw_vcenter(
            canvas,
            core::str::from_utf8(&text[..n]).unwrap_or(""),
            rect.x + 8,
            rect.y + 27,
            22,
            theme.text,
        );
        n = copy_tail(b"Private ", &mut text);
        n += write_bytes_into(p.metrics.private_bytes, &mut text[n..]);
        n += copy_tail(b"  Shared ", &mut text[n..]);
        n += write_bytes_into(p.metrics.shared_bytes, &mut text[n..]);
        n += copy_tail(b"  Mapped ", &mut text[n..]);
        n += write_bytes_into(p.metrics.mapped_bytes, &mut text[n..]);
        F_SMALL.draw_vcenter(
            canvas,
            core::str::from_utf8(&text[..n]).unwrap_or(""),
            rect.x + 8,
            rect.y + 51,
            22,
            theme.text,
        );
        n = copy_tail(b"Telemetry sample cost ", &mut text);
        n += write_num_into(
            (self.snapshot.sample_cost_ns / 1000).min(u32::MAX as u64) as u32,
            &mut text[n..],
        );
        n += copy_tail(b" us (", &mut text[n..]);
        n += write_bp_into(self.snapshot.sampling_cpu_bp(), &mut text[n..]);
        n += copy_tail(
            b" capacity) | Ctrl+P pause  Ctrl +/- interval  Ctrl+G group",
            &mut text[n..],
        );
        F_SMALL.draw_vcenter(
            canvas,
            core::str::from_utf8(&text[..n]).unwrap_or(""),
            rect.x + 8,
            rect.y + 75,
            22,
            theme.text_muted,
        );
    }
}

#[cfg(test)]
mod monitor_tests {
    use super::*;
    fn fixture() -> TasksApp {
        let mut app = TasksApp::new(Telemetry::unavailable());
        app.snapshot.proc_count = 3;
        for (i, (pid, name, cpu, memory)) in [
            (41, "melody-mina", 100, 4096),
            (42, "melody-mina", 500, 8192),
            (50, "pdf-viewer", 300, 16384),
        ]
        .into_iter()
        .enumerate()
        {
            let p = &mut app.snapshot.procs[i];
            p.pid = pid;
            p.ppid = if pid == 42 { 41 } else { 1 };
            p.generation = 1;
            p.name = sunlight_ipc::telemetry::executable_name(name.as_bytes());
            p.metrics.app_id = p.name;
            p.cpu_bp = cpu;
            p.metrics.private_bytes = memory;
        }
        app.rebuild_rows();
        app
    }
    #[test]
    fn sorting_search_and_group_totals() {
        let mut app = fixture();
        assert_eq!(app.snapshot.procs[app.order[0]].pid, 42);
        app.sort_col = 4;
        app.rebuild_rows();
        assert_eq!(app.snapshot.procs[app.order[0]].pid, 50);
        app.descending = false;
        app.rebuild_rows();
        assert_eq!(app.snapshot.procs[app.order[0]].pid, 41);
        app.query[..2].copy_from_slice(b"42");
        app.query_len = 2;
        app.rebuild_rows();
        assert_eq!(app.row_count, 1);
        app.query_len = 0;
        app.grouping = 2;
        app.rebuild_rows();
        assert_eq!(app.row_count, 5);
        let group = (0..app.row_count)
            .find(|i| app.row_group[*i] && app.snapshot.procs[app.order[*i]].pid == 41)
            .unwrap();
        assert_eq!(
            core::str::from_utf8(&app.row_bufs[group][4][..app.row_lens[group][4]]).unwrap(),
            "12.0 KiB"
        );
        app.collapsed[0] = sunlight_ipc::telemetry::executable_name(b"melody-mina");
        app.rebuild_rows();
        assert_eq!(app.row_count, 3);
    }
    #[test]
    fn histories_clear_on_exit_and_reuse() {
        let mut app = fixture();
        app.record_history();
        let slot = app
            .history_ids
            .iter()
            .position(|id| *id == (41, 1))
            .unwrap();
        assert_eq!(app.mem_history[slot][0], 4096);
        app.snapshot.procs[0].generation = 2;
        app.snapshot.procs[0].metrics.private_bytes = 1;
        app.record_history();
        assert!(!app.history_ids.contains(&(41, 1)));
        let slot = app
            .history_ids
            .iter()
            .position(|id| *id == (41, 2))
            .unwrap();
        assert_eq!(app.mem_history[slot][0], 0);
    }
    #[test]
    fn units_and_case_insensitive_filter() {
        for (bytes, expected) in [
            (1024, "1.0 KiB"),
            (1572864, "1.5 MiB"),
            (1073741824, "1.0 GiB"),
        ] {
            let mut out = [0; 32];
            let n = write_bytes_into(bytes, &mut out);
            assert_eq!(core::str::from_utf8(&out[..n]).unwrap(), expected);
        }
        assert!(contains_ascii(b"Melody-Mina", b"MINA"));
        assert!(!contains_ascii(b"pdf", b"browser"));
    }
}

fn parse_percent(bytes: &[u8]) -> Option<u16> {
    if bytes.is_empty() || bytes.len() > 3 {
        return None;
    }
    let mut n = 0u16;
    for b in bytes {
        if !b.is_ascii_digit() {
            return None;
        }
        n = n * 10 + (b - b'0') as u16;
    }
    if n <= 100 {
        Some(n * 100)
    } else {
        None
    }
}
