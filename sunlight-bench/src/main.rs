//! sunlight-bench — SunlightOS performance benchmarking suite.

#![no_std]
#![cfg_attr(not(test), no_main)]

extern crate alloc;

mod bench;
mod bench_mode;
mod cpu;
mod matrix;
mod multi;
mod pi;
mod prime_scan;
mod scoring;
mod sha256;
mod sieve;
mod thread;

use core::alloc::{GlobalAlloc, Layout};
use core::sync::atomic::{AtomicUsize, Ordering};

use bench_mode::{enter_parallel_phase, leave_parallel_phase, parallel_workers};
use cpu::CpuRunner;
use matrix::MatrixRunner;
use multi::{AsyncHandle, WorkloadId};
use pi::PiRunner;
use prime_scan::PrimeRunner;
use scoring::{
    make_entry, score_report, Entry, ScoreReport, StageMetrics, WorkloadClass, BENCH_COUNT,
};
use sieve::SieveRunner;
use sun_font::{FontRole, VecFont};
use sunlight_ipc::{debug_log, monotonic_millis, process_yield, ProcessExit};
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
const WIN_H: u32 = 620;
const HEADER_H: u32 = 46;
const TOOLBAR_H: u32 = 30;
const SUMMARY_H: u32 = 148;
const STAGE_H: u32 = 116;
const STATUS_H: u32 = 18;
const MARGIN: i32 = 14;

const BUTTON_COUNT: usize = 3;
const BUTTON_WIDTHS: [u32; BUTTON_COUNT] = [112, 112, 112];

const KEY_Q: u8 = 0x10;
const KEY_ENTER: u8 = 0x1C;

const TABLE_COLS: usize = 4;
const CELL_BUF: usize = 64;
const DEFAULT_RUNS: usize = 3;

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

// ── Heap allocator ─────────────────────────────────────────────────────────

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
                    return core::ptr::addr_of_mut!(HEAP_DATA.0)
                        .cast::<u8>()
                        .add(aligned);
                }
                Err(actual) => cur = actual,
            }
        }
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {}
}

#[global_allocator]
static ALLOC: BumpAlloc = BumpAlloc;

#[cfg(not(test))]
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

// ── Stage state machine ────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Ready,
    Pi,
    Prime,
    Sieve,
    Matrix,
    Cpu,
    MultiInt,
    MultiMatrix,
    MultiSha,
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
            Self::Cpu => "CPU Mix",
            Self::MultiInt => "Multi-1",
            Self::MultiMatrix => "Multi-2",
            Self::MultiSha => "Multi-3",
            Self::Finished => "Finished",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StageOutcome {
    Completed,
    Skipped,
    Failed,
}

impl StageOutcome {
    fn label(self) -> &'static str {
        match self {
            Self::Completed => "done",
            Self::Skipped => "skipped",
            Self::Failed => "failed",
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct RunSnapshot {
    entries: [Option<Entry>; BENCH_COUNT],
    stage_states: [StageOutcome; BENCH_COUNT],
    report: ScoreReport,
    speedup: Option<(u64, u64)>,
    efficiency: Option<u64>,
    completed_stages: usize,
    skipped_stages: usize,
    failed_stages: usize,
}

impl Default for RunSnapshot {
    fn default() -> Self {
        Self {
            entries: [None; BENCH_COUNT],
            stage_states: [StageOutcome::Skipped; BENCH_COUNT],
            report: ScoreReport::default(),
            speedup: None,
            efficiency: None,
            completed_stages: 0,
            skipped_stages: 0,
            failed_stages: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct AggregateSummary {
    best: u64,
    average: u64,
    min: u64,
    max: u64,
    spread_pct: u64,
}

// ── Application ────────────────────────────────────────────────────────────

struct BenchApp {
    pid: u64,
    ncores: usize,
    run_target: usize,
    completed_runs: usize,
    stage: Stage,
    running: bool,
    started: bool,
    hovered_button: Option<usize>,
    status: [u8; 128],
    status_len: usize,
    detail: [u8; 160],
    detail_len: usize,
    single_text: [u8; 48],
    single_len: usize,
    multi_text: [u8; 48],
    multi_len: usize,
    final_text: [u8; 32],
    final_len: usize,
    legacy_text: [u8; 36],
    legacy_len: usize,
    core_text: [u8; 24],
    core_len: usize,
    run_text: [u8; 72],
    run_len: usize,
    phase_text: [u8; 32],
    phase_len: usize,
    results: [Option<Entry>; BENCH_COUNT],
    run_summaries: [RunSnapshot; DEFAULT_RUNS],
    row_bufs: [[[u8; CELL_BUF]; TABLE_COLS]; BENCH_COUNT],
    row_lens: [[usize; TABLE_COLS]; BENCH_COUNT],
    pi: PiRunner,
    prime: PrimeRunner,
    sieve: SieveRunner,
    matrix: MatrixRunner,
    cpu: CpuRunner,
    multi_started: bool,
    multi_handle: Option<AsyncHandle>,
    multi_workload: u8,
    stage_started_tick: u64,
    stage_started_ms: u64,
    telemetry_ptr: *const TelemetryPage,
    multi_busy_verified: bool,
    multi_busy_peak: usize,
}

impl BenchApp {
    fn new(pid: u64, ncores: usize) -> Self {
        let mut app = Self {
            pid,
            ncores,
            run_target: DEFAULT_RUNS,
            completed_runs: 0,
            stage: Stage::Ready,
            running: false,
            started: false,
            hovered_button: None,
            status: [0; 128],
            status_len: 0,
            detail: [0; 160],
            detail_len: 0,
            single_text: [0; 48],
            single_len: 0,
            multi_text: [0; 48],
            multi_len: 0,
            final_text: [0; 32],
            final_len: 0,
            legacy_text: [0; 36],
            legacy_len: 0,
            core_text: [0; 24],
            core_len: 0,
            run_text: [0; 72],
            run_len: 0,
            phase_text: [0; 32],
            phase_len: 0,
            results: [None; BENCH_COUNT],
            run_summaries: [RunSnapshot::default(); DEFAULT_RUNS],
            row_bufs: [[[0; CELL_BUF]; TABLE_COLS]; BENCH_COUNT],
            row_lens: [[0; TABLE_COLS]; BENCH_COUNT],
            pi: PiRunner::new(),
            prime: PrimeRunner::new(),
            sieve: SieveRunner::new(),
            matrix: MatrixRunner::new(),
            cpu: CpuRunner::new(),
            multi_started: false,
            multi_handle: None,
            multi_workload: 0,
            stage_started_tick: 0,
            stage_started_ms: 0,
            telemetry_ptr: map_telemetry_page(),
            multi_busy_verified: false,
            multi_busy_peak: 0,
        };
        app.set_status("Ready to benchmark");
        app.set_detail(
            "Run starts chunked single-core stages, then three parallel multi-core workloads.",
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

    fn single_str(&self) -> &str {
        as_str(&self.single_text[..self.single_len])
    }

    fn multi_str(&self) -> &str {
        as_str(&self.multi_text[..self.multi_len])
    }

    fn final_str(&self) -> &str {
        as_str(&self.final_text[..self.final_len])
    }

    fn legacy_str(&self) -> &str {
        as_str(&self.legacy_text[..self.legacy_len])
    }

    fn core_str(&self) -> &str {
        as_str(&self.core_text[..self.core_len])
    }

    fn run_str(&self) -> &str {
        as_str(&self.run_text[..self.run_len])
    }

    fn phase_str(&self) -> &str {
        as_str(&self.phase_text[..self.phase_len])
    }

    fn elapsed_ms(entry: Entry) -> u64 {
        if entry.end_ms > entry.start_ms {
            entry.end_ms - entry.start_ms
        } else {
            (entry.end_tick.saturating_sub(entry.start_tick) / 1_000_000).max(1)
        }
    }

    fn multi_speedup_ratio(&self) -> Option<(u64, u64)> {
        let single = self.results[3]?;
        let multi = self.results[6]?;
        let single_ms = Self::elapsed_ms(single);
        let multi_ms = Self::elapsed_ms(multi);
        if single_ms == 0 || multi_ms == 0 {
            return None;
        }
        Some((single_ms, multi_ms))
    }

    fn multi_efficiency_pct(&self) -> Option<u64> {
        let (single_ms, multi_ms) = self.multi_speedup_ratio()?;
        let cores = self.worker_count().max(1) as u64;
        if multi_ms == 0 {
            None
        } else {
            Some(single_ms.saturating_mul(100) / (multi_ms.saturating_mul(cores)))
        }
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

    fn begin_stage_timing(&mut self) {
        self.stage_started_tick = bench::rdtsc();
        self.stage_started_ms = monotonic_millis();
    }

    fn stage_metrics(
        &self,
        work_units: u64,
        workers: u32,
        cycles: u64,
        class: WorkloadClass,
    ) -> StageMetrics {
        let end_tick = bench::rdtsc();
        let end_ms = monotonic_millis();
        StageMetrics {
            start_tick: self.stage_started_tick,
            end_tick,
            start_ms: self.stage_started_ms,
            end_ms,
            work_units,
            workers,
            cycles,
            class,
        }
    }

    fn worker_count(&self) -> usize {
        parallel_workers(self.ncores, self.ncores)
    }

    fn current_run_number(&self) -> usize {
        self.completed_runs + 1
    }

    fn display_run_number(&self) -> usize {
        if !self.started {
            0
        } else if self.running {
            self.current_run_number()
        } else {
            self.completed_runs.max(1)
        }
    }

    fn current_stage_counts(&self) -> (usize, usize, usize) {
        let completed = completed_count(&self.results);
        (completed, 0, 0)
    }

    fn scores(&self) -> ScoreReport {
        score_report(&self.results)
    }

    fn capture_run_snapshot(&self) -> RunSnapshot {
        let mut snapshot = RunSnapshot::default();
        snapshot.entries = self.results;
        snapshot.report = self.scores();
        snapshot.speedup = self.multi_speedup_ratio();
        snapshot.efficiency = self.multi_efficiency_pct();
        snapshot.completed_stages = completed_count(&self.results);
        snapshot.skipped_stages = 0;
        snapshot.failed_stages = BENCH_COUNT.saturating_sub(snapshot.completed_stages);
        for idx in 0..BENCH_COUNT {
            snapshot.stage_states[idx] = if snapshot.entries[idx].is_some() {
                StageOutcome::Completed
            } else {
                StageOutcome::Failed
            };
        }
        snapshot
    }

    fn aggregate_summary(&self) -> Option<AggregateSummary> {
        if self.completed_runs == 0 {
            return None;
        }

        let mut best = 0u64;
        let mut min = u64::MAX;
        let mut max = 0u64;
        let mut sum = 0u128;

        for idx in 0..self.completed_runs.min(DEFAULT_RUNS) {
            let score = self.run_summaries[idx].report.weighted_final;
            best = best.max(score);
            min = min.min(score);
            max = max.max(score);
            sum = sum.saturating_add(score as u128);
        }

        let runs = self.completed_runs.min(DEFAULT_RUNS) as u128;
        if runs == 0 {
            return None;
        }

        let average = ((sum + (runs / 2)) / runs) as u64;
        let spread_pct = if average == 0 {
            0
        } else {
            let spread = max.saturating_sub(min) as u128;
            ((spread.saturating_mul(100) + (average as u128 / 2)) / average as u128) as u64
        };

        Some(AggregateSummary {
            best,
            average,
            min,
            max,
            spread_pct,
        })
    }

    fn rebuild_summary(&mut self) {
        let report = self.scores();
        let workers = self.worker_count();
        let (completed, skipped, failed) = self.current_stage_counts();

        self.final_len = copy_bytes(b"Final v2 Score: ", &mut self.final_text);
        self.final_len += write_u64_into(
            report.weighted_final,
            &mut self.final_text[self.final_len..],
        );

        self.single_len = copy_bytes(b"Single normalized/raw: ", &mut self.single_text);
        self.single_len += write_u64_into(
            report.single_normalized,
            &mut self.single_text[self.single_len..],
        );
        self.single_len += copy_bytes(b" / ", &mut self.single_text[self.single_len..]);
        self.single_len +=
            write_u64_into(report.single_raw, &mut self.single_text[self.single_len..]);

        self.multi_len = copy_bytes(b"Multi normalized/raw: ", &mut self.multi_text);
        self.multi_len += write_u64_into(
            report.multi_normalized,
            &mut self.multi_text[self.multi_len..],
        );
        self.multi_len += copy_bytes(b" / ", &mut self.multi_text[self.multi_len..]);
        self.multi_len += write_u64_into(report.multi_raw, &mut self.multi_text[self.multi_len..]);

        self.legacy_len = copy_bytes(b"Legacy Raw Total: ", &mut self.legacy_text);
        self.legacy_len += write_u64_into(
            report.legacy_raw_total,
            &mut self.legacy_text[self.legacy_len..],
        );

        self.core_len = copy_bytes(b"Cores: ", &mut self.core_text);
        self.core_len += write_num_into(
            workers.min(u32::MAX as usize) as u32,
            &mut self.core_text[self.core_len..],
        );

        self.run_len = copy_bytes(b"Runs: ", &mut self.run_text);
        self.run_len += write_num_into(
            self.display_run_number().min(u32::MAX as usize) as u32,
            &mut self.run_text[self.run_len..],
        );
        self.run_len += copy_bytes(b"/", &mut self.run_text[self.run_len..]);
        self.run_len += write_num_into(
            self.run_target.min(u32::MAX as usize) as u32,
            &mut self.run_text[self.run_len..],
        );
        self.run_len += copy_bytes(b"  Stages: ", &mut self.run_text[self.run_len..]);
        self.run_len += write_num_into(
            completed.min(u32::MAX as usize) as u32,
            &mut self.run_text[self.run_len..],
        );
        self.run_len += copy_bytes(b"/", &mut self.run_text[self.run_len..]);
        self.run_len += write_num_into(
            BENCH_COUNT.min(u32::MAX as usize) as u32,
            &mut self.run_text[self.run_len..],
        );
        self.run_len += copy_bytes(b"  Skipped: ", &mut self.run_text[self.run_len..]);
        self.run_len += write_num_into(
            skipped.min(u32::MAX as usize) as u32,
            &mut self.run_text[self.run_len..],
        );
        self.run_len += copy_bytes(b"  Failed: ", &mut self.run_text[self.run_len..]);
        self.run_len += write_num_into(
            failed.min(u32::MAX as usize) as u32,
            &mut self.run_text[self.run_len..],
        );

        let speedup = self.multi_speedup_ratio();
        let efficiency = self.multi_efficiency_pct();
        self.phase_len = copy_bytes(b"Speedup: ", &mut self.phase_text);
        self.phase_len +=
            write_optional_ratio_into(speedup, &mut self.phase_text[self.phase_len..]);
        self.phase_len += copy_bytes(b"  Eff: ", &mut self.phase_text[self.phase_len..]);
        self.phase_len +=
            write_optional_pct_into(efficiency, &mut self.phase_text[self.phase_len..]);
    }

    fn run_summary_text(
        snapshot: &RunSnapshot,
        run_number: usize,
        workers: usize,
    ) -> alloc::string::String {
        let mut speed_buf = [0u8; 16];
        let speed_len = write_optional_ratio_into(snapshot.speedup, &mut speed_buf);
        let speed = as_str(&speed_buf[..speed_len]);
        let mut eff_buf = [0u8; 8];
        let eff_len = write_optional_pct_into(snapshot.efficiency, &mut eff_buf);
        let eff = as_str(&eff_buf[..eff_len]);
        alloc::format!(
            "run={run_number} final={final_score} single={single_norm}/{single_raw} multi={multi_norm}/{multi_raw} legacy={legacy_raw} speedup={speed} efficiency={eff} stages={completed}/{expected} skipped={skipped} failed={failed} cores={workers}",
            run_number = run_number,
            final_score = snapshot.report.weighted_final,
            single_norm = snapshot.report.single_normalized,
            single_raw = snapshot.report.single_raw,
            multi_norm = snapshot.report.multi_normalized,
            multi_raw = snapshot.report.multi_raw,
            legacy_raw = snapshot.report.legacy_raw_total,
            speed = speed,
            eff = eff,
            completed = snapshot.completed_stages,
            expected = BENCH_COUNT,
            skipped = snapshot.skipped_stages,
            failed = snapshot.failed_stages,
            workers = workers,
        )
    }

    fn aggregate_summary_text(summary: AggregateSummary, runs: usize) -> alloc::string::String {
        alloc::format!(
            "Aggregate over {runs} runs: best={best} average={average} min={min} max={max} spread={spread_pct}%",
            runs = runs,
            best = summary.best,
            average = summary.average,
            min = summary.min,
            max = summary.max,
            spread_pct = summary.spread_pct,
        )
    }

    fn rebuild_rows(&mut self) {
        let names = [
            pi::NAME,
            prime_scan::NAME,
            sieve::NAME,
            matrix::NAME,
            cpu::NAME,
            multi::NAME_INTEGER,
            multi::NAME_MATRIX,
            multi::NAME_SHA256,
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
            Stage::Cpu => Some(4),
            Stage::MultiInt => Some(5),
            Stage::MultiMatrix => Some(6),
            Stage::MultiSha => Some(7),
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
            Stage::Cpu => self.cpu.progress_bp(),
            Stage::MultiInt | Stage::MultiMatrix | Stage::MultiSha => multi::async_progress_bp(),
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
            Stage::Cpu => self.cpu.name(),
            Stage::MultiInt => multi::NAME_INTEGER,
            Stage::MultiMatrix => multi::NAME_MATRIX,
            Stage::MultiSha => multi::NAME_SHA256,
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
            Stage::Cpu => {
                "Geekbench-style integer mix and SHA-256 stay bounded and avoid raw-pointer tricks."
            }
            Stage::MultiInt => {
                "Work-per-core: each worker runs 64M integer ops so throughput scaling stays visible."
            }
            Stage::MultiMatrix => {
                "Fixed-total-work: all workers split one 1024^2 matrix multiply to measure speedup."
            }
            Stage::MultiSha => {
                "Work-per-core: each worker hashes 16 MiB so throughput scaling stays visible."
            }
            Stage::Finished => "Use Serial to mirror the repeat-run report into debug_log.",
        }
    }

    fn status_badge(&self) -> BadgeKind {
        match self.stage {
            Stage::Ready => BadgeKind::Dim,
            Stage::Finished => BadgeKind::Ok,
            Stage::MultiInt | Stage::MultiMatrix | Stage::MultiSha => BadgeKind::Warn,
            _ => BadgeKind::Accent,
        }
    }

    fn start(&mut self) {
        if self.started {
            return;
        }
        self.started = true;
        self.begin_run(1);
        debug_log(&alloc::format!(
            "[BENCH] GUI benchmark starting (runs={})",
            self.run_target
        ));
    }

    fn release_memory(&mut self) {
        self.matrix.release();
        self.cpu.release();
        self.prime.release();
        self.multi_handle = None;
        multi::reset_async();
        HEAP_NEXT.store(0, Ordering::Relaxed);
        debug_log("[BENCH] Memory released");
    }

    fn begin_run(&mut self, run_number: usize) {
        sunlight_ipc::set_nice(self.pid, -10);
        if run_number > 1 {
            self.release_memory();
        }

        self.results = [None; BENCH_COUNT];
        self.stage = Stage::Pi;
        self.running = true;
        self.multi_started = false;
        self.multi_handle = None;
        self.multi_workload = 0;
        self.stage_started_tick = 0;
        self.stage_started_ms = 0;
        self.pi = PiRunner::new();
        self.prime = PrimeRunner::new();
        self.sieve = SieveRunner::new();
        self.matrix = MatrixRunner::new();
        self.cpu = CpuRunner::new();
        self.begin_stage_timing();
        self.set_status(&alloc::format!(
            "Run {}/{} running",
            run_number,
            self.run_target
        ));
        self.set_detail("Pi stage started. The process priority is raised to reduce interference.");
        self.rebuild_summary();
        self.rebuild_rows();
    }

    fn finish_current_run(&mut self) {
        let snapshot = self.capture_run_snapshot();
        let run_idx = self.completed_runs.min(DEFAULT_RUNS - 1);
        self.run_summaries[run_idx] = snapshot;
        self.completed_runs = self.completed_runs.saturating_add(1);
    }

    fn start_next_run_or_finish(&mut self) {
        self.finish_current_run();
        sunlight_ipc::set_nice(self.pid, 0);

        if self.completed_runs < self.run_target {
            let next_run = self.current_run_number();
            self.begin_run(next_run);
        } else {
            self.finish_all_runs();
        }
    }

    fn finish_all_runs(&mut self) {
        self.stage = Stage::Finished;
        self.running = false;
        sunlight_ipc::set_nice(self.pid, 0);
        self.report_multi_core_activity();
        let detail = self
            .aggregate_summary()
            .map(|summary| Self::aggregate_summary_text(summary, self.completed_runs))
            .unwrap_or_else(|| alloc::string::String::from("Aggregate: n/a"));
        self.set_detail(&detail);
        self.set_status("Benchmark complete");
        self.rebuild_summary();
        self.rebuild_rows();
        self.emit_serial_report();
    }

    fn emit_serial_report(&self) {
        let completed_runs = self.completed_runs.min(DEFAULT_RUNS);
        let workers = self.worker_count();
        debug_log("[BENCH] ============================================");
        debug_log("[BENCH] SunLight Bench v2 Report");
        debug_log(&alloc::format!("[BENCH] Cores: {}", workers));
        debug_log(&alloc::format!(
            "[BENCH] Runs: {}/{}",
            completed_runs,
            self.run_target
        ));
        debug_log(&alloc::format!(
            "[BENCH] Stage count: {}/{} (skipped=0 failed=0)",
            BENCH_COUNT,
            BENCH_COUNT
        ));
        debug_log("[BENCH] ");
        debug_log("[BENCH] Per-run summary:");

        for run_idx in 0..completed_runs {
            let snapshot = self.run_summaries[run_idx];
            debug_log(&alloc::format!(
                "[BENCH]   {}",
                Self::run_summary_text(&snapshot, run_idx + 1, workers)
            ));
        }

        if let Some(summary) = self.aggregate_summary() {
            debug_log("[BENCH] Aggregate:");
            debug_log(&alloc::format!(
                "[BENCH]   best={} average={} min={} max={} spread={}%",
                summary.best,
                summary.average,
                summary.min,
                summary.max,
                summary.spread_pct
            ));
        }

        debug_log("[BENCH] CSV:");
        debug_log(
            "[BENCH] run,final,single_norm,single_raw,multi_norm,multi_raw,legacy_raw,speedup,efficiency,completed,skipped,failed",
        );
        for run_idx in 0..completed_runs {
            let snapshot = self.run_summaries[run_idx];
            let mut speed_buf = [0u8; 16];
            let speed_len = write_optional_ratio_into(snapshot.speedup, &mut speed_buf);
            let speed = as_str(&speed_buf[..speed_len]);
            let mut eff_buf = [0u8; 8];
            let eff_len = write_optional_pct_into(snapshot.efficiency, &mut eff_buf);
            let eff = as_str(&eff_buf[..eff_len]);
            debug_log(&alloc::format!(
                "[BENCH] {run},{final_score},{single_norm},{single_raw},{multi_norm},{multi_raw},{legacy_raw},{speed},{eff},{completed},{skipped},{failed}",
                run = run_idx + 1,
                final_score = snapshot.report.weighted_final,
                single_norm = snapshot.report.single_normalized,
                single_raw = snapshot.report.single_raw,
                multi_norm = snapshot.report.multi_normalized,
                multi_raw = snapshot.report.multi_raw,
                legacy_raw = snapshot.report.legacy_raw_total,
                speed = speed,
                eff = eff,
                completed = snapshot.completed_stages,
                skipped = snapshot.skipped_stages,
                failed = snapshot.failed_stages,
            ));
        }

        debug_log("[BENCH] Per-stage table:");
        for run_idx in 0..completed_runs {
            let snapshot = self.run_summaries[run_idx];
            debug_log(&alloc::format!("[BENCH]   run={}", run_idx + 1));
            for (idx, entry) in snapshot.entries.iter().enumerate() {
                let entry = *entry;
                let name = entry.map(|entry| entry.name).unwrap_or(match idx {
                    0 => pi::NAME,
                    1 => prime_scan::NAME,
                    2 => sieve::NAME,
                    3 => matrix::NAME,
                    4 => cpu::NAME,
                    5 => multi::NAME_INTEGER,
                    6 => multi::NAME_MATRIX,
                    7 => multi::NAME_SHA256,
                    _ => "",
                });
                let state = snapshot.stage_states[idx].label();
                let cycles = entry.map(|entry| entry.cycles).unwrap_or(0);
                let raw_score = entry.map(|entry| entry.score).unwrap_or(0);
                debug_log(&alloc::format!(
                    "[BENCH]     stage={} state={} cycles={} raw_score={}",
                    name,
                    state,
                    cycles,
                    raw_score
                ));
            }
        }
        debug_log("[BENCH] ============================================");
    }

    fn record_result(&mut self, idx: usize, name: &'static str, metrics: StageMetrics) {
        let entry = make_entry(idx, name, metrics);
        self.results[idx] = Some(entry);
        log_entry(entry);
        self.rebuild_summary();
        self.rebuild_rows();
    }

    fn advance_to_next_multi_workload(&mut self) {
        self.multi_workload += 1;
        self.multi_started = false;
        self.multi_handle = None;
        multi::reset_async();

        match self.multi_workload {
            1 => {
                self.stage = Stage::MultiMatrix;
                self.set_status("Parallel matrix started");
                self.set_detail(
                    "Fixed-total-work: all workers split one 1024^2 matrix multiply to measure speedup.",
                );
                self.begin_stage_timing();
            }
            2 => {
                self.stage = Stage::MultiSha;
                self.set_status("Parallel SHA-256 started");
                self.set_detail(
                    "Work-per-core: each worker hashes 16 MiB so throughput scaling stays visible.",
                );
                self.begin_stage_timing();
            }
            _ => {
                self.start_next_run_or_finish();
            }
        }
    }

    fn tick_multi_workload(
        &mut self,
        idx: usize,
        workload: WorkloadId,
        name: &'static str,
    ) -> bool {
        let workers = self.worker_count();
        if !self.multi_started {
            enter_parallel_phase(workers);
            self.multi_handle = multi::start_async(workers, workload);
            self.multi_started = self.multi_handle.is_some();
            if !self.multi_started {
                leave_parallel_phase();
                self.set_status("Parallel workload failed to start");
                self.set_detail("Background worker creation failed.");
                self.running = false;
                return true;
            }
            self.verify_multi_core_activity();
            true
        } else if let Some(cycles) = multi::take_async_result() {
            self.verify_multi_core_activity();
            let metrics = self.stage_metrics(
                workload.total_work_units(workers),
                workers as u32,
                cycles,
                workload.class(),
            );
            self.record_result(idx, name, metrics);
            leave_parallel_phase();
            self.advance_to_next_multi_workload();
            true
        } else {
            self.verify_multi_core_activity();
            true
        }
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
                    let metrics = self.stage_metrics(
                        scoring::WORK_UNITS[0],
                        1,
                        self.pi.cycles(),
                        WorkloadClass::SingleCore,
                    );
                    self.record_result(0, pi::NAME, metrics);
                    self.stage = Stage::Prime;
                    self.begin_stage_timing();
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
                    let metrics = self.stage_metrics(
                        scoring::WORK_UNITS[1],
                        1,
                        self.prime.cycles(),
                        WorkloadClass::SingleCore,
                    );
                    self.record_result(1, prime_scan::NAME, metrics);
                    self.stage = Stage::Sieve;
                    self.begin_stage_timing();
                    self.set_status("Prime stage complete");
                    self.set_detail("Segmented sieve started.");
                }
                true
            }
            Stage::Sieve => {
                let done = self.sieve.step();
                self.set_detail("Marking cache-sized segments and counting primes incrementally.");
                if done {
                    let metrics = self.stage_metrics(
                        scoring::WORK_UNITS[2],
                        1,
                        self.sieve.cycles(),
                        WorkloadClass::SingleCore,
                    );
                    self.record_result(2, sieve::NAME, metrics);
                    self.stage = Stage::Matrix;
                    self.begin_stage_timing();
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
                    let metrics = self.stage_metrics(
                        scoring::WORK_UNITS[3],
                        1,
                        self.matrix.cycles(),
                        WorkloadClass::SingleCore,
                    );
                    self.record_result(3, matrix::NAME, metrics);
                    self.stage = Stage::Cpu;
                    self.begin_stage_timing();
                    self.set_status("Matrix stage complete");
                    self.set_detail("Geekbench-style CPU mix started.");
                }
                true
            }
            Stage::Cpu => {
                let done = self.cpu.step();
                self.set_detail(
                    "CPU mix combines bounded integer churn with SHA-256 over a fixed working set.",
                );
                if done {
                    let metrics = self.stage_metrics(
                        scoring::WORK_UNITS[4],
                        1,
                        self.cpu.cycles(),
                        WorkloadClass::SingleCore,
                    );
                    self.record_result(4, cpu::NAME, metrics);
                    self.release_memory();
                    self.stage = Stage::MultiInt;
                    self.begin_stage_timing();
                    self.multi_workload = 0;
                    self.multi_started = false;
                    self.set_status("CPU mix complete");
                    self.set_detail("Dispatching multi-core integer mix workers.");
                }
                true
            }
            Stage::MultiInt => {
                self.set_detail(
                    "Worker threads are running a deterministic integer mixer in the background.",
                );
                self.tick_multi_workload(5, WorkloadId::Integer, multi::NAME_INTEGER)
            }
            Stage::MultiMatrix => {
                self.set_detail(
                    "Each core processes rows via ikj-loop. Progress is updated per row.",
                );
                self.tick_multi_workload(6, WorkloadId::Matrix, multi::NAME_MATRIX)
            }
            Stage::MultiSha => {
                self.set_detail(
                    "Each core hashes independent data. Progress is reported per block.",
                );
                self.tick_multi_workload(7, WorkloadId::Sha256, multi::NAME_SHA256)
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
                "[BENCH] telemetry verified {} non-idle cores during multi-core pass",
                busy
            ));
        }
    }

    fn report_multi_core_activity(&self) {
        if self.multi_busy_peak >= 2 {
            debug_log(&alloc::format!(
                "[BENCH] telemetry peak non-idle cores during multi-core passes: {}",
                self.multi_busy_peak
            ));
        } else {
            debug_log(&alloc::format!(
                "[BENCH] telemetry peak non-idle cores during multi-core passes: {} (multi-core activity not observed)",
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
                    2 => {
                        self.release_memory();
                        request_close();
                    }
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
            Rect::new(summary.x + 120, summary.y + 20, 420, 16),
            self.final_str(),
        )
        .with_font(&F_MED)
        .draw(canvas, theme);
        Label::new(
            Rect::new(summary.x + 560, summary.y + 22, 200, 14),
            self.legacy_str(),
        )
        .dim()
        .with_font(&F_SMALL)
        .draw(canvas, theme);

        Label::new(
            Rect::new(summary.x + 14, summary.y + 42, 400, 14),
            self.single_str(),
        )
        .with_font(&F_UI)
        .draw(canvas, theme);
        Label::new(
            Rect::new(summary.x + 420, summary.y + 42, 400, 14),
            self.multi_str(),
        )
        .with_font(&F_UI)
        .draw(canvas, theme);
        Label::new(
            Rect::new(summary.x + 14, summary.y + 60, 120, 14),
            self.core_str(),
        )
        .with_font(&F_UI)
        .draw(canvas, theme);
        Label::new(
            Rect::new(summary.x + 110, summary.y + 60, 690, 14),
            self.run_str(),
        )
        .with_font(&F_UI)
        .draw(canvas, theme);
        Label::new(
            Rect::new(summary.x + 14, summary.y + 78, 280, 14),
            self.phase_str(),
        )
        .with_font(&F_UI)
        .draw(canvas, theme);
        Label::new(
            Rect::new(
                summary.x + 14,
                summary.y + 96,
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
                    self.release_memory();
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

#[cfg(not(test))]
#[no_mangle]
pub extern "C" fn _start() -> ! {
    let pid = sunlight_ipc::getpid();
    let ncores = cpu_count();

    let mut app = BenchApp::new(pid, ncores);
    let mut window = match Window::connect(WindowConfig {
        width: WIN_W,
        height: WIN_H,
        title: "SunLight Bench",
        decoration: sunlight_ui::WindowDecoration::Normal,
    }) {
        Some(window) => window,
        None => run_headless(pid, ncores),
    };

    window.run(&mut app);
    sunlight_ipc::set_nice(pid, 0);
    app.release_memory();
    ProcessExit::exit(0);
}

fn run_headless(pid: u64, ncores: usize) -> ! {
    debug_log("[BENCH] display unavailable, running headless");
    let mut app = BenchApp::new(pid, ncores);
    app.start();
    while app.stage != Stage::Finished {
        app.tick_benchmark();
        process_yield();
    }
    sunlight_ipc::set_nice(pid, 0);
    app.release_memory();
    ProcessExit::exit(0);
}

fn log_entry(entry: Entry) {
    debug_log(&alloc::format!(
        "[BENCH] {:.<40} start={:>10} end={:>10} wall={:>5}ms work={:>12} workers={:>2} cycles={:>14} class={:<17} score={:>6}",
        entry.name,
        entry.start_tick,
        entry.end_tick,
        BenchApp::elapsed_ms(entry),
        entry.work_units,
        entry.workers,
        entry.cycles,
        entry.class.label(),
        entry.score
    ));
}

fn write_optional_ratio_into(value: Option<(u64, u64)>, dst: &mut [u8]) -> usize {
    match value {
        Some((numer, denom)) => write_ratio_into(numer, denom, dst),
        None => copy_bytes(b"n/a", dst),
    }
}

fn write_ratio_into(numer: u64, denom: u64, dst: &mut [u8]) -> usize {
    if dst.is_empty() || denom == 0 {
        return copy_bytes(b"n/a", dst);
    }

    let whole = numer / denom;
    let rem = numer % denom;
    let mut len = write_u64_into(whole, dst);
    if len < dst.len() {
        dst[len] = b'.';
        len += 1;
    } else {
        return len;
    }

    let frac = ((rem as u128).saturating_mul(100) + (denom as u128 / 2)) / denom as u128;
    let frac = frac.min(99) as u8;
    if len < dst.len() {
        dst[len] = b'0' + (frac / 10);
        len += 1;
    }
    if len < dst.len() {
        dst[len] = b'0' + (frac % 10);
        len += 1;
    }
    if len < dst.len() {
        dst[len] = b'x';
        len += 1;
    }
    len
}

fn write_optional_pct_into(value: Option<u64>, dst: &mut [u8]) -> usize {
    match value {
        Some(pct) => {
            let mut len = write_u64_into(pct, dst);
            if len < dst.len() {
                dst[len] = b'%';
                len += 1;
            }
            len
        }
        None => copy_bytes(b"n/a", dst),
    }
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
