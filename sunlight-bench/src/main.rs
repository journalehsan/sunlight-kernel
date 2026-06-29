//! sunlight-bench — SunlightOS performance benchmarking suite.

#![no_std]
#![no_main]

extern crate alloc;

mod bench;
mod matrix;
mod multi;
mod pi;
mod prime_scan;
mod scoring;
mod sieve;
mod thread;

use core::alloc::{GlobalAlloc, Layout};
use core::sync::atomic::{AtomicUsize, Ordering};

use matrix::MatrixRunner;
use multi::{AsyncHandle, ParallelMix};
use pi::PiRunner;
use prime_scan::PrimeRunner;
use scoring::{make_entry, total_score, Entry, BENCH_COUNT};
use sieve::SieveRunner;
use sun_font::{FontRole, VecFont};
use sunlight_ipc::{debug_log, process_yield, ProcessExit};
use sunlight_telemetry::{TelemetryPage, MAX_CORES as TELEMETRY_MAX_CORES, TELEMETRY_MAGIC};
use sunlight_ui::{
    request_close,
    widgets::{
        BadgeKind, Button, ButtonState, Column, Label, Panel, ProgressBar, StatusBadge, StatusBar,
        Table,
    },
    App, Event, HBox, Point, Rect, Window, WindowConfig,
};

static F_UI: VecFont = VecFont(FontRole::UiRegular);
static F_MED: VecFont = VecFont(FontRole::UiMedium);
static F_SMALL: VecFont = VecFont(FontRole::UiSmall);

const HEAP_SIZE: usize = 32 * 1024 * 1024;

const WIN_W: u32 = 880;
const WIN_H: u32 = 560;
const HEADER_H: u32 = 46;
const TOOLBAR_H: u32 = 30;
const SUMMARY_H: u32 = 104;
const STAGE_H: u32 = 116;
const STATUS_H: u32 = 18;
const MARGIN: i32 = 14;

const BUTTON_COUNT: usize = 3;
const BUTTON_WIDTHS: [u32; BUTTON_COUNT] = [112, 112, 112];

const KEY_Q: u8 = 0x10;
const KEY_ENTER: u8 = 0x1C;

const TABLE_COLS: usize = 4;
const CELL_BUF: usize = 64;

const TABLE_COLUMNS: [Column<'static>; TABLE_COLS] = [
    Column {
        header: "Benchmark",
        width: 360,
        right_align: false,
    },
    Column {
        header: "State",
        width: 110,
        right_align: false,
    },
    Column {
        header: "Cycles",
        width: 220,
        right_align: true,
    },
    Column {
        header: "Score",
        width: 120,
        right_align: true,
    },
];

const EMPTY_ROW: [&str; TABLE_COLS] = ["", "", "", ""];

#[repr(C, align(16))]
struct AlignedHeap([u8; HEAP_SIZE]);

static mut HEAP_DATA: AlignedHeap = AlignedHeap([0u8; HEAP_SIZE]);
static HEAP_NEXT: AtomicUsize = AtomicUsize::new(0);

struct BumpAlloc;

unsafe impl GlobalAlloc for BumpAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let align = layout.align();
        let size = layout.size();
        let mut cur = HEAP_NEXT.load(Ordering::Relaxed);
        loop {
            let aligned = (cur + align - 1) & !(align - 1);
            let next = aligned + size;
            if next > HEAP_SIZE {
                return core::ptr::null_mut();
            }
            match HEAP_NEXT.compare_exchange(cur, next, Ordering::SeqCst, Ordering::Relaxed) {
                Ok(_) => {
                    #[allow(static_mut_refs)]
                    return unsafe { HEAP_DATA.0.as_mut_ptr().add(aligned) };
                }
                Err(actual) => cur = actual,
            }
        }
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {}
}

#[global_allocator]
static ALLOC: BumpAlloc = BumpAlloc;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    if let Some(loc) = info.location() {
        debug_log(&alloc::format!(
            "[BENCH] PANIC at {}:{}",
            loc.file(),
            loc.line()
        ));
    } else {
        debug_log("[BENCH] PANIC");
    }
    ProcessExit::exit(1);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Ready,
    Pi,
    Prime,
    Sieve,
    Matrix,
    Multi,
    Finished,
}

impl Stage {
    fn label(self) -> &'static str {
        match self {
            Self::Ready => "Ready",
            Self::Pi => "Pi",
            Self::Prime => "Prime",
            Self::Sieve => "Sieve",
            Self::Matrix => "Matrix",
            Self::Multi => "Parallel Mix",
            Self::Finished => "Finished",
        }
    }
}

struct BenchApp {
    pid: u64,
    ncores: usize,
    stage: Stage,
    running: bool,
    started: bool,
    serial_reported: bool,
    hovered_button: Option<usize>,
    status: [u8; 128],
    status_len: usize,
    detail: [u8; 128],
    detail_len: usize,
    score_text: [u8; 24],
    score_len: usize,
    core_text: [u8; 24],
    core_len: usize,
    phase_text: [u8; 32],
    phase_len: usize,
    results: [Option<Entry>; BENCH_COUNT],
    row_bufs: [[[u8; CELL_BUF]; TABLE_COLS]; BENCH_COUNT],
    row_lens: [[usize; TABLE_COLS]; BENCH_COUNT],
    pi: PiRunner,
    prime: PrimeRunner,
    sieve: SieveRunner,
    matrix: MatrixRunner,
    multi_started: bool,
    multi_handle: Option<AsyncHandle>,
    telemetry_ptr: *const TelemetryPage,
    multi_busy_verified: bool,
    multi_busy_peak: usize,
}

impl BenchApp {
    fn new(pid: u64, ncores: usize) -> Self {
        let mut app = Self {
            pid,
            ncores,
            stage: Stage::Ready,
            running: false,
            started: false,
            serial_reported: false,
            hovered_button: None,
            status: [0; 128],
            status_len: 0,
            detail: [0; 128],
            detail_len: 0,
            score_text: [0; 24],
            score_len: 0,
            core_text: [0; 24],
            core_len: 0,
            phase_text: [0; 32],
            phase_len: 0,
            results: [None; BENCH_COUNT],
            row_bufs: [[[0; CELL_BUF]; TABLE_COLS]; BENCH_COUNT],
            row_lens: [[0; TABLE_COLS]; BENCH_COUNT],
            pi: PiRunner::new(),
            prime: PrimeRunner::new(),
            sieve: SieveRunner::new(),
            matrix: MatrixRunner::new(),
            multi_started: false,
            multi_handle: None,
            telemetry_ptr: map_telemetry_page(),
            multi_busy_verified: false,
            multi_busy_peak: 0,
        };
        app.set_status("Ready to benchmark");
        app.set_detail(
            "Run starts chunked single-core stages and a live multi-core integer mix pass.",
        );
        app.rebuild_summary();
        app.rebuild_rows();
        app
    }

    fn set_status(&mut self, text: &str) {
        self.status_len = copy_str(text, &mut self.status);
    }

    fn status_str(&self) -> &str {
        as_str(&self.status[..self.status_len])
    }

    fn set_detail(&mut self, text: &str) {
        self.detail_len = copy_str(text, &mut self.detail);
    }

    fn detail_str(&self) -> &str {
        as_str(&self.detail[..self.detail_len])
    }

    fn score_str(&self) -> &str {
        as_str(&self.score_text[..self.score_len])
    }

    fn core_str(&self) -> &str {
        as_str(&self.core_text[..self.core_len])
    }

    fn phase_str(&self) -> &str {
        as_str(&self.phase_text[..self.phase_len])
    }

    fn content_rect(&self) -> Rect {
        Rect::new(
            MARGIN,
            MARGIN,
            WIN_W.saturating_sub((MARGIN as u32) * 2),
            WIN_H.saturating_sub((MARGIN as u32) * 2),
        )
    }

    fn header_rect(&self) -> Rect {
        let content = self.content_rect();
        Rect::new(content.x, content.y, content.w, HEADER_H)
    }

    fn toolbar_rect(&self) -> Rect {
        let header = self.header_rect();
        Rect::new(header.x, header.bottom() + 8, header.w, TOOLBAR_H)
    }

    fn summary_rect(&self) -> Rect {
        let toolbar = self.toolbar_rect();
        Rect::new(toolbar.x, toolbar.bottom() + 10, toolbar.w, SUMMARY_H)
    }

    fn stage_rect(&self) -> Rect {
        let summary = self.summary_rect();
        Rect::new(summary.x, summary.bottom() + 10, summary.w, STAGE_H)
    }

    fn results_rect(&self) -> Rect {
        let stage = self.stage_rect();
        let content = self.content_rect();
        Rect::new(
            stage.x,
            stage.bottom() + 10,
            stage.w,
            content
                .bottom()
                .saturating_sub(stage.bottom() + 10 + STATUS_H as i32 + 8) as u32,
        )
    }

    fn status_rect(&self) -> Rect {
        let content = self.content_rect();
        Rect::new(
            content.x,
            content.bottom() - STATUS_H as i32,
            content.w,
            STATUS_H,
        )
    }

    fn button_rects(&self) -> [Rect; BUTTON_COUNT] {
        let mut rects = [Rect::default(); BUTTON_COUNT];
        for (idx, rect) in HBox::new(self.toolbar_rect())
            .with_spacing(8)
            .layout(&BUTTON_WIDTHS)
            .enumerate()
        {
            rects[idx] = rect;
        }
        rects
    }

    fn rebuild_summary(&mut self) {
        self.core_len = copy_bytes(b"Cores ", &mut self.core_text);
        self.core_len += write_num_into(
            self.ncores.min(u32::MAX as usize) as u32,
            &mut self.core_text[self.core_len..],
        );

        let score = total_score(&self.results);
        self.score_len = copy_bytes(b"Score ", &mut self.score_text);
        self.score_len += write_u64_into(score, &mut self.score_text[self.score_len..]);

        let phase = completed_count(&self.results);
        self.phase_len = write_num_into(phase as u32, &mut self.phase_text);
        self.phase_len += copy_bytes(b"/5 stages", &mut self.phase_text[self.phase_len..]);
    }

    fn rebuild_rows(&mut self) {
        let names = [
            pi::NAME,
            prime_scan::NAME,
            sieve::NAME,
            matrix::NAME,
            multi::NAME,
        ];
        for (idx, name) in names.iter().enumerate() {
            copy_cell(name, &mut self.row_bufs[idx][0], &mut self.row_lens[idx][0]);
            copy_cell(
                self.row_state(idx),
                &mut self.row_bufs[idx][1],
                &mut self.row_lens[idx][1],
            );

            if let Some(entry) = self.results[idx] {
                write_u64_cell(
                    entry.cycles,
                    &mut self.row_bufs[idx][2],
                    &mut self.row_lens[idx][2],
                );
                write_u64_cell(
                    entry.score,
                    &mut self.row_bufs[idx][3],
                    &mut self.row_lens[idx][3],
                );
            } else {
                self.row_lens[idx][2] = 0;
                self.row_lens[idx][3] = 0;
            }
        }
    }

    fn row_state(&self, idx: usize) -> &'static str {
        if self.results[idx].is_some() {
            "Done"
        } else if self.running && self.current_index() == Some(idx) {
            "Running"
        } else if self.started {
            "Queued"
        } else {
            "Ready"
        }
    }

    fn current_index(&self) -> Option<usize> {
        match self.stage {
            Stage::Pi => Some(0),
            Stage::Prime => Some(1),
            Stage::Sieve => Some(2),
            Stage::Matrix => Some(3),
            Stage::Multi => Some(4),
            _ => None,
        }
    }

    fn global_progress(&self) -> f32 {
        let completed = completed_count(&self.results) as f32;
        let current = self.stage_progress_bp() as f32 / 10_000.0;
        ((completed + current) / BENCH_COUNT as f32).clamp(0.0, 1.0)
    }

    fn stage_progress_bp(&self) -> u16 {
        match self.stage {
            Stage::Ready => 0,
            Stage::Pi => self.pi.progress_bp(),
            Stage::Prime => self.prime.progress_bp(),
            Stage::Sieve => self.sieve.progress_bp(),
            Stage::Matrix => self.matrix.progress_bp(),
            Stage::Multi => multi::async_progress_bp(),
            Stage::Finished => 10_000,
        }
    }

    fn stage_name(&self) -> &'static str {
        match self.stage {
            Stage::Ready => "Standby",
            Stage::Pi => self.pi.name(),
            Stage::Prime => self.prime.name(),
            Stage::Sieve => self.sieve.name(),
            Stage::Matrix => self.matrix.name(),
            Stage::Multi => multi::NAME,
            Stage::Finished => "Benchmark complete",
        }
    }

    fn stage_note(&self) -> &'static str {
        match self.stage {
            Stage::Ready => "UI stays live while the single-core passes yield between work chunks.",
            Stage::Pi => "Fixed-point Machin iterations measured in chunks, then accumulated.",
            Stage::Prime => {
                "A compact dense sieve gives you a fast prime-focused pass before the heavy one."
            }
            Stage::Sieve => "Segment windows stream across the limit with cache-sized bitsets.",
            Stage::Matrix => {
                "Large integer matmul runs in resumable slices to avoid a frozen window."
            }
            Stage::Multi => {
                "Worker threads run a deterministic integer mixer while the window polls progress."
            }
            Stage::Finished => "Use Serial to mirror the final table into debug_log.",
        }
    }

    fn status_badge(&self) -> BadgeKind {
        match self.stage {
            Stage::Ready => BadgeKind::Dim,
            Stage::Finished => BadgeKind::Ok,
            Stage::Multi => BadgeKind::Warn,
            _ => BadgeKind::Accent,
        }
    }

    fn start(&mut self) {
        if self.started {
            return;
        }
        self.started = true;
        self.running = true;
        self.stage = Stage::Pi;
        sunlight_ipc::set_nice(self.pid, -10);
        self.set_status("Benchmark running");
        self.set_detail("Pi stage started. The process priority is raised to reduce interference.");
        debug_log("[BENCH] GUI benchmark starting");
        self.rebuild_summary();
        self.rebuild_rows();
    }

    fn emit_serial_report(&mut self) {
        debug_log("[BENCH] ============================================");
        debug_log("[BENCH] SunLight-Bench GUI results");
        debug_log("[BENCH] ============================================");
        for entry in self.results.iter().flatten() {
            debug_log(&alloc::format!(
                "[BENCH] {:.<40} {:>14} cycles  score {:>6}",
                entry.name,
                entry.cycles,
                entry.score
            ));
        }
        debug_log(&alloc::format!(
            "[BENCH] TOTAL SCORE {:>32}",
            total_score(&self.results)
        ));
        self.serial_reported = true;
        self.set_status("Results mirrored to serial");
    }

    fn record_result(&mut self, idx: usize, name: &'static str, cycles: u64) {
        let entry = make_entry(name, cycles);
        self.results[idx] = Some(entry);
        debug_log(&alloc::format!(
            "[BENCH] {:.<40} {:>14} cycles  score {:>6}",
            entry.name,
            entry.cycles,
            entry.score
        ));
        self.rebuild_summary();
        self.rebuild_rows();
    }

    fn finish(&mut self) {
        self.stage = Stage::Finished;
        self.running = false;
        sunlight_ipc::set_nice(self.pid, 0);
        self.report_multi_core_activity();
        self.set_detail(
            "SunLight-Bench finished. Close the window or mirror the summary to serial.",
        );
        if !self.serial_reported {
            self.emit_serial_report();
        }
        self.set_status("Benchmark complete");
        self.rebuild_summary();
        self.rebuild_rows();
    }

    fn tick_benchmark(&mut self) -> bool {
        if !self.running {
            return false;
        }

        let changed = match self.stage {
            Stage::Pi => {
                let done = self.pi.step();
                self.set_detail("Pi iterations are chunked to preserve redraw cadence.");
                if done {
                    self.record_result(0, pi::NAME, self.pi.cycles());
                    self.stage = Stage::Prime;
                    self.set_status("Pi stage complete");
                    self.set_detail("Dense 100k prime sieve started.");
                }
                true
            }
            Stage::Prime => {
                let done = self.prime.step();
                self.set_detail(
                    "Dense prime sieve is marking and counting in small UI-friendly chunks.",
                );
                if done {
                    self.record_result(1, prime_scan::NAME, self.prime.cycles());
                    self.stage = Stage::Sieve;
                    self.set_status("Prime stage complete");
                    self.set_detail("Segmented sieve started.");
                }
                true
            }
            Stage::Sieve => {
                let done = self.sieve.step();
                self.set_detail("Marking cache-sized segments and counting primes incrementally.");
                if done {
                    self.record_result(2, sieve::NAME, self.sieve.cycles());
                    self.stage = Stage::Matrix;
                    self.set_status("Sieve stage complete");
                    self.set_detail(
                        "Matrix multiply started. This is typically the longest stage.",
                    );
                }
                true
            }
            Stage::Matrix => {
                let done = self.matrix.step();
                self.set_detail(
                    "Matrix work slices through the ikj loop nest without freezing the UI.",
                );
                if done {
                    self.record_result(3, matrix::NAME, self.matrix.cycles());
                    self.stage = Stage::Multi;
                    self.set_status("Matrix stage complete");
                    self.set_detail("Dispatching multi-core mix workers.");
                }
                true
            }
            Stage::Multi => {
                if !self.multi_started {
                    self.multi_handle = multi::start_async(self.ncores);
                    self.multi_started = self.multi_handle.is_some();
                    if !self.multi_started {
                        self.set_status("Parallel mix failed to start");
                        self.set_detail("Background worker creation failed.");
                        self.running = false;
                        return true;
                    }
                    self.set_status("Parallel mix running");
                    self.set_detail("Worker threads are running a deterministic integer mixer in the background.");
                    self.verify_multi_core_activity();
                    true
                } else if let Some(cycles) = multi::take_async_result() {
                    self.verify_multi_core_activity();
                    self.record_result(4, multi::NAME, cycles);
                    self.multi_handle = None;
                    self.finish();
                    true
                } else {
                    self.verify_multi_core_activity();
                    self.set_detail(
                        "Parallel mix workers are active. The window is polling completion.",
                    );
                    true
                }
            }
            Stage::Finished | Stage::Ready => false,
        };

        self.rebuild_summary();
        self.rebuild_rows();
        changed
    }

    fn verify_multi_core_activity(&mut self) {
        let busy = busy_core_count(self.telemetry_ptr);
        self.multi_busy_peak = self.multi_busy_peak.max(busy);
        if !self.multi_busy_verified && busy >= 2 {
            self.multi_busy_verified = true;
            debug_log(&alloc::format!(
                "[BENCH] telemetry verified {} non-idle cores during Parallel Mix",
                busy
            ));
        }
    }

    fn report_multi_core_activity(&self) {
        if self.multi_busy_peak >= 2 {
            debug_log(&alloc::format!(
                "[BENCH] telemetry peak non-idle cores during Parallel Mix: {}",
                self.multi_busy_peak
            ));
        } else {
            debug_log(&alloc::format!(
                "[BENCH] telemetry peak non-idle cores during Parallel Mix: {} (multi-core activity not observed)",
                self.multi_busy_peak
            ));
        }
    }

    fn button_state(&self, idx: usize) -> ButtonState {
        match idx {
            0 if self.started => ButtonState::Disabled,
            1 if !self.started => ButtonState::Disabled,
            _ if self.hovered_button == Some(idx) => ButtonState::Hovered,
            _ => ButtonState::Normal,
        }
    }

    fn handle_click(&mut self, x: i32, y: i32) -> bool {
        let buttons = self.button_rects();
        for (idx, rect) in buttons.iter().enumerate() {
            if rect.contains(Point::new(x, y)) {
                match idx {
                    0 => self.start(),
                    1 => self.emit_serial_report(),
                    2 => request_close(),
                    _ => {}
                }
                return true;
            }
        }
        false
    }

    fn row_refs<'a>(
        &'a self,
        rows: &'a mut [[&'a str; TABLE_COLS]; BENCH_COUNT],
    ) -> [&'a [&'a str]; BENCH_COUNT] {
        for row in 0..BENCH_COUNT {
            for col in 0..TABLE_COLS {
                rows[row][col] = as_str(&self.row_bufs[row][col][..self.row_lens[row][col]]);
            }
        }
        let mut refs: [&[&str]; BENCH_COUNT] = [&[]; BENCH_COUNT];
        for row in 0..BENCH_COUNT {
            refs[row] = &rows[row];
        }
        refs
    }
}

impl App for BenchApp {
    fn view(&mut self, canvas: &mut sunlight_ui::Canvas, theme: &sunlight_ui::Theme) {
        canvas.fill_rect(Rect::new(0, 0, WIN_W, WIN_H), theme.bg);

        let content = self.content_rect();
        Panel::new(content).draw(canvas, theme);

        let header = self.header_rect();
        canvas.fill_rect(header, theme.panel_alt);
        canvas.hbar(header.x, header.bottom() - 1, header.w, 1, theme.accent);
        Label::new(
            Rect::new(header.x + 12, header.y + 6, 260, 20),
            "SunLight Bench",
        )
        .with_font(&F_MED)
        .draw(canvas, theme);
        Label::new(
            Rect::new(header.x + 12, header.y + 26, 520, 14),
            "Windowed performance suite with live progress, orange telemetry, and chunked long runs.",
        )
        .dim()
        .with_font(&F_SMALL)
        .draw(canvas, theme);
        Label::new(
            Rect::new(header.right() - 118, header.y + 12, 106, 14),
            "SunlightOS GUI",
        )
        .dim()
        .with_font(&F_SMALL)
        .draw(canvas, theme);

        let buttons = self.button_rects();
        let labels = ["Run", "Serial", "Close"];
        for (idx, rect) in buttons.iter().enumerate() {
            let mut button = if idx == 0 {
                Button::new(*rect, labels[idx]).with_font(&F_UI)
            } else {
                Button::secondary(*rect, labels[idx]).with_font(&F_UI)
            };
            button.state = self.button_state(idx);
            button.draw(canvas, theme);
        }

        let summary = self.summary_rect();
        Panel::with_title(summary, "Overview").draw(canvas, theme);
        StatusBadge::new(summary.x + 14, summary.y + 26, self.status_badge())
            .with_label(self.stage.label())
            .draw(canvas, theme);
        Label::new(
            Rect::new(summary.x + 120, summary.y + 20, 180, 16),
            self.score_str(),
        )
        .with_font(&F_UI)
        .draw(canvas, theme);
        Label::new(
            Rect::new(summary.x + 300, summary.y + 20, 140, 16),
            self.core_str(),
        )
        .with_font(&F_UI)
        .draw(canvas, theme);
        Label::new(
            Rect::new(summary.x + 430, summary.y + 20, 140, 16),
            self.phase_str(),
        )
        .with_font(&F_UI)
        .draw(canvas, theme);
        Label::new(
            Rect::new(summary.x + 14, summary.y + 42, 240, 14),
            self.status_str(),
        )
        .with_font(&F_UI)
        .draw(canvas, theme);
        Label::new(
            Rect::new(
                summary.x + 14,
                summary.y + 58,
                summary.w.saturating_sub(28),
                14,
            ),
            self.detail_str(),
        )
        .dim()
        .with_font(&F_SMALL)
        .draw(canvas, theme);
        ProgressBar::new(
            Rect::new(
                summary.x + 14,
                summary.bottom() - 28,
                summary.w.saturating_sub(28),
                14,
            ),
            self.global_progress(),
        )
        .with_pct()
        .draw(canvas, theme);

        let stage = self.stage_rect();
        Panel::with_title(stage, "Current Stage").draw(canvas, theme);
        Label::new(
            Rect::new(stage.x + 14, stage.y + 26, stage.w.saturating_sub(28), 16),
            self.stage_name(),
        )
        .with_font(&F_UI)
        .draw(canvas, theme);
        Label::new(
            Rect::new(stage.x + 14, stage.y + 46, stage.w.saturating_sub(28), 14),
            self.stage_note(),
        )
        .dim()
        .with_font(&F_SMALL)
        .draw(canvas, theme);
        ProgressBar::new(
            Rect::new(
                stage.x + 14,
                stage.bottom() - 34,
                stage.w.saturating_sub(28),
                16,
            ),
            self.stage_progress_bp() as f32 / 10_000.0,
        )
        .with_pct()
        .draw(canvas, theme);

        let results = self.results_rect();
        Panel::with_title(results, "Results").draw(canvas, theme);
        let mut rows = [EMPTY_ROW; BENCH_COUNT];
        let row_refs = self.row_refs(&mut rows);
        Table::new(
            Rect::new(
                results.x + 10,
                results.y + 24,
                results.w.saturating_sub(20),
                results.h.saturating_sub(34),
            ),
            &TABLE_COLUMNS,
            &row_refs,
        )
        .with_selected(self.current_index())
        .with_font(&F_UI)
        .draw(canvas, theme);

        StatusBar::new(
            self.status_rect(),
            self.status_str(),
            self.stage.label(),
            self.core_str(),
        )
        .draw(canvas, theme);
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::Tick => self.tick_benchmark(),
            Event::MouseMove { x, y } => {
                let next = self
                    .button_rects()
                    .iter()
                    .enumerate()
                    .find(|(_, rect)| rect.contains(Point::new(x, y)))
                    .map(|(idx, _)| idx);
                if self.hovered_button != next {
                    self.hovered_button = next;
                    true
                } else {
                    false
                }
            }
            Event::Click { x, y } => self.handle_click(x, y),
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
                if keycode == KEY_ENTER && !self.started {
                    self.start();
                    return true;
                }
                false
            }
            _ => false,
        }
    }
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let pid = sunlight_ipc::getpid();
    let ncores = cpu_count();

    let mut app = BenchApp::new(pid, ncores);
    let mut window = match Window::connect(WindowConfig {
        width: WIN_W,
        height: WIN_H,
        title: "SunLight Bench",
    }) {
        Some(window) => window,
        None => run_headless(pid, ncores),
    };

    window.run(&mut app);
    sunlight_ipc::set_nice(pid, 0);
    ProcessExit::exit(0);
}

fn run_headless(pid: u64, ncores: usize) -> ! {
    let mut results = [None; BENCH_COUNT];
    sunlight_ipc::set_nice(pid, -10);
    debug_log("[BENCH] display unavailable, running headless");

    let mut pi = PiRunner::new();
    while !pi.step() {
        process_yield();
    }
    results[0] = Some(make_entry(pi::NAME, pi.cycles()));
    log_entry(results[0].unwrap());

    let mut prime = PrimeRunner::new();
    while !prime.step() {
        process_yield();
    }
    results[1] = Some(make_entry(prime_scan::NAME, prime.cycles()));
    log_entry(results[1].unwrap());

    let mut sieve = SieveRunner::new();
    while !sieve.step() {
        process_yield();
    }
    results[2] = Some(make_entry(sieve::NAME, sieve.cycles()));
    log_entry(results[2].unwrap());

    let mut matrix = MatrixRunner::new();
    while !matrix.step() {
        process_yield();
    }
    results[3] = Some(make_entry(matrix::NAME, matrix.cycles()));
    log_entry(results[3].unwrap());

    let telemetry_ptr = map_telemetry_page();
    let mut multi_busy_peak = 0usize;
    let mut multi_verified = false;
    let handle = multi::start_async(ncores);
    if let Some(_handle) = handle {
        while results[4].is_none() {
            let busy = busy_core_count(telemetry_ptr);
            multi_busy_peak = multi_busy_peak.max(busy);
            if !multi_verified && busy >= 2 {
                multi_verified = true;
                debug_log(&alloc::format!(
                    "[BENCH] telemetry verified {} non-idle cores during Parallel Mix",
                    busy
                ));
            }
            if let Some(cycles) = multi::take_async_result() {
                results[4] = Some(make_entry(multi::NAME, cycles));
            } else {
                process_yield();
            }
        }
    } else {
        let multi = ParallelMix::new(ncores);
        results[4] = Some(make_entry(multi::NAME, multi.run_sync()));
    }
    debug_log(&alloc::format!(
        "[BENCH] telemetry peak non-idle cores during Parallel Mix: {}{}",
        multi_busy_peak,
        if multi_busy_peak >= 2 {
            ""
        } else {
            " (multi-core activity not observed)"
        }
    ));
    log_entry(results[4].unwrap());

    debug_log(&alloc::format!(
        "[BENCH] TOTAL SCORE {:>32}",
        total_score(&results)
    ));
    sunlight_ipc::set_nice(pid, 0);
    ProcessExit::exit(0);
}

fn log_entry(entry: Entry) {
    debug_log(&alloc::format!(
        "[BENCH] {:.<40} {:>14} cycles  score {:>6}",
        entry.name,
        entry.cycles,
        entry.score
    ));
}

fn cpu_count() -> usize {
    const CPU_COUNT_OFFSET: usize = 76;

    let ptr = sunlight_ipc::map_telemetry();
    if ptr.is_null() {
        return 1;
    }
    let magic = unsafe { core::ptr::read_volatile(ptr as *const u64) };
    if magic != TELEMETRY_MAGIC {
        return 1;
    }
    let count = unsafe { core::ptr::read_volatile(ptr.add(CPU_COUNT_OFFSET)) };
    (count as usize).max(1)
}

fn map_telemetry_page() -> *const TelemetryPage {
    let ptr = sunlight_ipc::map_telemetry() as *const TelemetryPage;
    if ptr.is_null() {
        return core::ptr::null();
    }
    let magic = unsafe { core::ptr::read_volatile(core::ptr::addr_of!((*ptr).magic)) };
    if magic == TELEMETRY_MAGIC {
        ptr
    } else {
        core::ptr::null()
    }
}

fn busy_core_count(page: *const TelemetryPage) -> usize {
    if page.is_null() {
        return 0;
    }

    let sequence = unsafe { core::ptr::read_volatile(core::ptr::addr_of!((*page).sequence)) };
    if sequence & 1 != 0 {
        return 0;
    }

    let count =
        unsafe { core::ptr::read_volatile(core::ptr::addr_of!((*page).core_count)) } as usize;
    let mut busy = 0usize;
    for idx in 0..count.min(TELEMETRY_MAX_CORES) {
        let current_pid = unsafe {
            core::ptr::read_volatile(core::ptr::addr_of!((*page).cores[idx].current_pid))
        };
        if current_pid != 0 {
            busy += 1;
        }
    }
    busy
}

fn completed_count(results: &[Option<Entry>; BENCH_COUNT]) -> usize {
    results.iter().filter(|entry| entry.is_some()).count()
}

fn as_str(bytes: &[u8]) -> &str {
    core::str::from_utf8(bytes).unwrap_or("")
}

fn copy_str(src: &str, dst: &mut [u8]) -> usize {
    copy_bytes(src.as_bytes(), dst)
}

fn copy_bytes(src: &[u8], dst: &mut [u8]) -> usize {
    let len = src.len().min(dst.len());
    dst[..len].copy_from_slice(&src[..len]);
    len
}

fn copy_cell(src: &str, dst: &mut [u8; CELL_BUF], len: &mut usize) {
    *len = copy_str(src, dst);
}

fn write_u64_cell(value: u64, dst: &mut [u8; CELL_BUF], len: &mut usize) {
    *len = write_u64_into(value, dst);
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
    let mut digits = 0usize;
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

fn write_u64_into(mut value: u64, dst: &mut [u8]) -> usize {
    if dst.is_empty() {
        return 0;
    }
    if value == 0 {
        dst[0] = b'0';
        return 1;
    }
    let mut tmp = [0u8; 20];
    let mut digits = 0usize;
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
