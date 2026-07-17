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
mod profile;
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
    make_entry, median_score, score_report, spread_is_unstable, spread_pct_from_scores,
    spread_stability_class, Entry, ScoreReport, StageMetrics, WorkloadClass, BENCH_COUNT,
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
const WIN_H: u32 = 670;
const HEADER_H: u32 = 36;
const SUMMARY_H: u32 = 100;
const STAGE_H: u32 = 100;
const STATUS_H: u32 = 18;
const MARGIN: i32 = 14;

const BUTTON_COUNT: usize = 3;
const BUTTON_WIDTHS: [u32; BUTTON_COUNT] = [90, 90, 90];

const KEY_Q: u8 = 0x10;
const KEY_ENTER: u8 = 0x1C;
const KEY_PGUP: u8 = 0x49;
const KEY_PGDN: u8 = 0x51;
const KEY_UP: u8 = 0x48;
const KEY_DOWN: u8 = 0x50;

const TABLE_COLS: usize = 4;
const CELL_BUF: usize = 64;
const DEFAULT_RUNS: usize = 3;
const WARMUP_ENABLED: bool = true;
const RUN_COOLDOWN_MS: u64 = 50;
const RUN_COOLDOWN_YIELDS: u32 = 32;
const SPREAD_WARNING: &str =
    "Warning: benchmark run variance is high; results may not be reliable.";

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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum CooldownTarget {
    #[default]
    None,
    FirstMeasuredRun,
    NextMeasuredRun,
}

#[derive(Clone, Copy, Debug, Default)]
struct AggregateSummary {
    best: u64,
    average: u64,
    median: u64,
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
    detail: [u8; 256],
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
    profile_text: [u8; 48],
    profile_len: usize,
    run_text: [u8; 72],
    run_len: usize,
    phase_text: [u8; 128],
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
    warming_up: bool,
    multi_timing_pending: bool,
    cooldown_target: CooldownTarget,
    cooldown_until_ms: u64,
    cooldown_yields_left: u32,
    table_scroll_offset: usize,
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
            detail: [0; 256],
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
            profile_text: [0; 48],
            profile_len: 0,
            run_text: [0; 72],
            run_len: 0,
            phase_text: [0; 128],
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
            warming_up: false,
            multi_timing_pending: false,
            cooldown_target: CooldownTarget::None,
            cooldown_until_ms: 0,
            cooldown_yields_left: 0,
            table_scroll_offset: 0,
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

    #[allow(dead_code)]
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

    fn profile_str(&self) -> &str {
        as_str(&self.profile_text[..self.profile_len])
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

    fn summary_rect(&self) -> Rect {
        let header = self.header_rect();
        Rect::new(header.x, header.bottom() + 4, header.w, SUMMARY_H)
    }

    fn stage_rect(&self) -> Rect {
        let summary = self.summary_rect();
        if self.stage == Stage::Finished {
            Rect::new(summary.x, summary.bottom(), summary.w, 0)
        } else {
            Rect::new(summary.x, summary.bottom() + 4, summary.w, STAGE_H)
        }
    }

    fn results_rect(&self) -> Rect {
        let stage = self.stage_rect();
        let content = self.content_rect();
        let stage_end = if stage.h == 0 {
            stage.y
        } else {
            stage.bottom() + 4
        };
        let status_y = content.bottom() - STATUS_H as i32;
        let available = (status_y - 4).saturating_sub(stage_end);
        Rect::new(stage.x, stage_end, stage.w, available.max(0) as u32)
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
        let header = self.header_rect();
        let mut rects = [Rect::default(); BUTTON_COUNT];
        let btn_area = Rect::new(header.right() - 300, header.y + 4, 290, HEADER_H - 8);
        for (idx, rect) in HBox::new(btn_area)
            .with_spacing(6)
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

    fn stage_metrics_from_times(
        &self,
        start_tick: u64,
        start_ms: u64,
        end_tick: u64,
        end_ms: u64,
        work_units: u64,
        workers: u32,
        cycles: u64,
        class: WorkloadClass,
    ) -> StageMetrics {
        StageMetrics {
            start_tick,
            end_tick,
            start_ms,
            end_ms,
            work_units,
            workers,
            cycles,
            class,
        }
    }

    fn stage_metrics(
        &self,
        work_units: u64,
        workers: u32,
        cycles: u64,
        class: WorkloadClass,
    ) -> StageMetrics {
        self.stage_metrics_from_times(
            self.stage_started_tick,
            self.stage_started_ms,
            bench::rdtsc(),
            monotonic_millis(),
            work_units,
            workers,
            cycles,
            class,
        )
    }

    fn worker_count(&self) -> usize {
        parallel_workers(self.ncores, self.ncores)
    }

    fn current_run_number(&self) -> usize {
        self.completed_runs + 1
    }

    fn run_scores(&self) -> alloc::vec::Vec<u64> {
        let count = self.completed_runs.min(DEFAULT_RUNS);
        let mut scores = alloc::vec::Vec::with_capacity(count);
        for idx in 0..count {
            scores.push(self.run_summaries[idx].report.weighted_final);
        }
        scores
    }

    fn current_stage_counts(&self) -> (usize, usize, usize) {
        let completed = if !self.running
            && self.cooldown_target == CooldownTarget::FirstMeasuredRun
            && self.completed_runs == 0
        {
            0
        } else {
            completed_count(&self.results)
        };
        (completed, 0, 0)
    }

    fn scores(&self) -> ScoreReport {
        score_report(&self.results)
    }

    fn current_run_score(&self) -> u64 {
        self.scores().weighted_final
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
        let scores = self.run_scores();
        if scores.is_empty() {
            return None;
        }

        let best = scores.iter().copied().max().unwrap_or(0);
        let min = scores.iter().copied().min().unwrap_or(0);
        let max = best;
        let sum: u128 = scores.iter().map(|score| *score as u128).sum();
        let average = ((sum + (scores.len() as u128 / 2)) / scores.len() as u128) as u64;
        let median = median_score(&scores);
        let spread_pct = spread_pct_from_scores(&scores);

        Some(AggregateSummary {
            best,
            average,
            median,
            min,
            max,
            spread_pct,
        })
    }

    fn headline_score(&self) -> u64 {
        if let Some(summary) = self.aggregate_summary() {
            if self.completed_runs >= 2 {
                return summary.median;
            }
        }
        self.current_run_score()
    }

    fn rebuild_summary(&mut self) {
        let report = self.scores();
        let current_score = self.current_run_score();
        let workers = self.worker_count();
        let (completed, skipped, failed) = self.current_stage_counts();
        let headline = if self.stage == Stage::Finished && self.completed_runs >= 2 {
            self.headline_score()
        } else {
            current_score
        };

        self.final_len = if self.warming_up
            || (self.completed_runs == 0
                && self.cooldown_target == CooldownTarget::FirstMeasuredRun)
        {
            copy_bytes(b"Warmup (excluded): ", &mut self.final_text)
        } else if self.stage == Stage::Finished && self.completed_runs >= 2 {
            copy_bytes(b"Median v2 Score: ", &mut self.final_text)
        } else if self.started {
            copy_bytes(b"Current v2 Score: ", &mut self.final_text)
        } else {
            copy_bytes(b"Final v2 Score: ", &mut self.final_text)
        };
        self.final_len += write_u64_into(headline, &mut self.final_text[self.final_len..]);

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

        self.profile_len = copy_bytes(b"Profile: ", &mut self.profile_text);
        self.profile_len += copy_str(
            profile::detect_profile(),
            &mut self.profile_text[self.profile_len..],
        );

        self.core_len = copy_bytes(b"Cores: ", &mut self.core_text);
        self.core_len += write_num_into(
            workers.min(u32::MAX as usize) as u32,
            &mut self.core_text[self.core_len..],
        );

        self.run_len = copy_bytes(b"Runs: ", &mut self.run_text);
        self.run_len += write_num_into(
            self.completed_runs.min(u32::MAX as usize) as u32,
            &mut self.run_text[self.run_len..],
        );
        self.run_len += copy_bytes(b"/", &mut self.run_text[self.run_len..]);
        self.run_len += write_num_into(
            self.run_target.min(u32::MAX as usize) as u32,
            &mut self.run_text[self.run_len..],
        );
        if self.warming_up {
            self.run_len += copy_bytes(b"  Warmup", &mut self.run_text[self.run_len..]);
        } else if self.running && self.completed_runs < self.run_target {
            self.run_len += copy_bytes(b"  Current: ", &mut self.run_text[self.run_len..]);
            self.run_len += write_num_into(
                self.current_run_number().min(u32::MAX as usize) as u32,
                &mut self.run_text[self.run_len..],
            );
        }
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
        self.phase_len = if self.stage == Stage::Finished {
            if let Some(summary) = self.aggregate_summary() {
                let stability = spread_stability_class(summary.spread_pct);
                let mut len = copy_bytes(b"Current: ", &mut self.phase_text);
                len += write_u64_into(current_score, &mut self.phase_text[len..]);
                len += copy_bytes(b"  Median: ", &mut self.phase_text[len..]);
                len += write_u64_into(summary.median, &mut self.phase_text[len..]);
                len += copy_bytes(b"  Best: ", &mut self.phase_text[len..]);
                len += write_u64_into(summary.best, &mut self.phase_text[len..]);
                len += copy_bytes(b"  Avg: ", &mut self.phase_text[len..]);
                len += write_u64_into(summary.average, &mut self.phase_text[len..]);
                len += copy_bytes(b"  Spread: ", &mut self.phase_text[len..]);
                len += write_u64_into(summary.spread_pct, &mut self.phase_text[len..]);
                if len < self.phase_text.len() {
                    self.phase_text[len] = b'%';
                    len += 1;
                }
                len += copy_bytes(b"  ", &mut self.phase_text[len..]);
                len += copy_bytes(stability.as_bytes(), &mut self.phase_text[len..]);
                len
            } else {
                copy_bytes(b"Speedup: n/a", &mut self.phase_text)
            }
        } else {
            let mut len = copy_bytes(b"Speedup: ", &mut self.phase_text);
            len += write_optional_ratio_into(speedup, &mut self.phase_text[len..]);
            len += copy_bytes(b"  Eff: ", &mut self.phase_text[len..]);
            len += write_optional_pct_into(efficiency, &mut self.phase_text[len..]);
            len
        };
    }

    fn aggregate_summary_text(
        summary: AggregateSummary,
        runs: usize,
        current_score: u64,
    ) -> alloc::string::String {
        let stability = spread_stability_class(summary.spread_pct);
        let mut text = alloc::format!(
            "Aggregate over {runs} runs: current={current_score} median={median} best={best} average={average} min={min} max={max} spread={spread_pct}% ({stability})",
            runs = runs,
            current_score = current_score,
            median = summary.median,
            best = summary.best,
            average = summary.average,
            min = summary.min,
            max = summary.max,
            spread_pct = summary.spread_pct,
            stability = stability,
        );
        if spread_is_unstable(summary.spread_pct) {
            text.push_str("  ");
            text.push_str(SPREAD_WARNING);
        }
        text
    }

    fn profile_label(workers: usize) -> alloc::string::String {
        alloc::format!("{}-core", workers)
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
        if WARMUP_ENABLED {
            self.begin_warmup_run();
        } else {
            self.begin_measured_run(1);
        }
        debug_log(&alloc::format!(
            "[BENCH] GUI benchmark starting (runs={}, warmup={})",
            self.run_target,
            WARMUP_ENABLED
        ));
    }

    fn reset_run_state(&mut self) {
        self.release_memory();
        self.results = [None; BENCH_COUNT];
        self.stage = Stage::Pi;
        self.running = true;
        self.multi_started = false;
        self.multi_handle = None;
        self.multi_workload = 0;
        self.multi_timing_pending = false;
        self.stage_started_tick = 0;
        self.stage_started_ms = 0;
        self.multi_busy_verified = false;
        self.multi_busy_peak = 0;
        self.cooldown_target = CooldownTarget::None;
        self.cooldown_until_ms = 0;
        self.cooldown_yields_left = 0;
        self.table_scroll_offset = 0;
        self.pi = PiRunner::new();
        self.prime = PrimeRunner::new();
        self.sieve = SieveRunner::new();
        self.matrix = MatrixRunner::new();
        self.cpu = CpuRunner::new();
        multi::reset_async();
        self.begin_stage_timing();
        self.rebuild_summary();
        self.rebuild_rows();
    }

    fn begin_warmup_run(&mut self) {
        self.warming_up = true;
        sunlight_ipc::set_nice(self.pid, -10);
        self.reset_run_state();
        self.set_status("Warmup pass running");
        self.set_detail("Warmup is excluded from scoring and repeat-run aggregates.");
    }

    fn begin_measured_run(&mut self, run_number: usize) {
        self.warming_up = false;
        sunlight_ipc::set_nice(self.pid, -10);
        self.reset_run_state();
        self.set_status(&alloc::format!(
            "Run {}/{} running",
            run_number,
            self.run_target
        ));
        self.set_detail("Pi stage started. The process priority is raised to reduce interference.");
    }

    fn enter_cooldown(&mut self, target: CooldownTarget) {
        self.running = false;
        self.cooldown_target = target;
        self.cooldown_until_ms = monotonic_millis().saturating_add(RUN_COOLDOWN_MS);
        self.cooldown_yields_left = RUN_COOLDOWN_YIELDS;
        self.set_status("Cooldown between runs");
        self.set_detail("Yielding briefly so QEMU and the OS can settle.");
        self.rebuild_summary();
        self.rebuild_rows();
    }

    fn tick_cooldown(&mut self) -> bool {
        if self.cooldown_yields_left > 0 {
            self.cooldown_yields_left -= 1;
            process_yield();
            return true;
        }
        if monotonic_millis() < self.cooldown_until_ms {
            process_yield();
            return true;
        }

        let target = self.cooldown_target;
        self.cooldown_target = CooldownTarget::None;
        match target {
            CooldownTarget::FirstMeasuredRun => self.begin_measured_run(1),
            CooldownTarget::NextMeasuredRun => self.begin_measured_run(self.current_run_number()),
            CooldownTarget::None => {}
        }
        true
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

    fn finish_current_run(&mut self) {
        let snapshot = self.capture_run_snapshot();
        let run_idx = self.completed_runs.min(DEFAULT_RUNS - 1);
        self.run_summaries[run_idx] = snapshot;
        self.completed_runs = self.completed_runs.saturating_add(1);
    }

    fn complete_current_pass(&mut self) {
        sunlight_ipc::set_nice(self.pid, 0);
        if self.warming_up {
            self.warming_up = false;
            debug_log("[BENCH] Warmup pass complete (excluded from scoring)");
            self.enter_cooldown(CooldownTarget::FirstMeasuredRun);
            return;
        }

        self.finish_current_run();
        if self.completed_runs < self.run_target {
            self.enter_cooldown(CooldownTarget::NextMeasuredRun);
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
            .map(|summary| {
                Self::aggregate_summary_text(summary, self.completed_runs, self.current_run_score())
            })
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
        let (stages_completed, skipped_stages, failed_stages) = self.current_stage_counts();
        let profile = Self::profile_label(workers);
        let report = self.scores();
        let summary = self.aggregate_summary();
        let current = report.weighted_final;
        let speedup = self.multi_speedup_ratio();
        let efficiency = self.multi_efficiency_pct();
        let mut speed_buf = [0u8; 16];
        let speed_len = write_optional_ratio_into(speedup, &mut speed_buf);
        let speed = as_str(&speed_buf[..speed_len]);
        let mut eff_buf = [0u8; 8];
        let eff_len = write_optional_pct_into(efficiency, &mut eff_buf);
        let eff = as_str(&eff_buf[..eff_len]);
        let (median, best, average, min, max, spread, stability) = summary
            .map(|summary| {
                (
                    summary.median,
                    summary.best,
                    summary.average,
                    summary.min,
                    summary.max,
                    summary.spread_pct,
                    spread_stability_class(summary.spread_pct),
                )
            })
            .unwrap_or((0, 0, 0, 0, 0, 0, "n/a"));

        debug_log("[BENCH] ============================================");
        debug_log("[BENCH] SunLight Bench v2 Stable Report");
        debug_log("[BENCH] Environment/Profile:");
        debug_log(&alloc::format!(
            "[BENCH]   Hypervisor: {}",
            profile::detect_profile()
        ));
        debug_log(&alloc::format!("[BENCH]   Profile: {}", profile));
        debug_log(&alloc::format!("[BENCH]   Cores: {}", workers));
        debug_log(&alloc::format!(
            "[BENCH]   Runs: {}/{}",
            completed_runs,
            self.run_target
        ));
        debug_log(&alloc::format!(
            "[BENCH]   Stages: {}/{}",
            stages_completed,
            BENCH_COUNT
        ));
        debug_log(&alloc::format!("[BENCH]   Skipped: {}", skipped_stages));
        debug_log(&alloc::format!("[BENCH]   Failed: {}", failed_stages));
        debug_log(&alloc::format!("[BENCH]   Median v2 Score: {}", median));
        debug_log(&alloc::format!("[BENCH]   Current: {}", current));
        debug_log(&alloc::format!("[BENCH]   Best: {}", best));
        debug_log(&alloc::format!("[BENCH]   Average: {}", average));
        debug_log(&alloc::format!("[BENCH]   Min: {}", min));
        debug_log(&alloc::format!("[BENCH]   Max: {}", max));
        debug_log(&alloc::format!("[BENCH]   Spread: {}%", spread));
        debug_log(&alloc::format!("[BENCH]   Stability: {}", stability));
        if spread_is_unstable(spread) {
            debug_log(&alloc::format!("[BENCH]   {}", SPREAD_WARNING));
        }
        debug_log(&alloc::format!(
            "[BENCH]   Single normalized/raw: {} / {}",
            report.single_normalized,
            report.single_raw
        ));
        debug_log(&alloc::format!(
            "[BENCH]   Multi normalized/raw: {} / {}",
            report.multi_normalized,
            report.multi_raw
        ));
        debug_log(&alloc::format!(
            "[BENCH]   Legacy Raw Total: {}",
            report.legacy_raw_total
        ));
        debug_log(&alloc::format!("[BENCH]   Speedup: {}", speed));
        debug_log(&alloc::format!("[BENCH]   Efficiency: {}", eff));
        debug_log("[BENCH] hypervisor,profile,cores,runs,median,current,best,average,min,max,spread,stability,single_norm,single_raw,multi_norm,multi_raw,legacy_raw_total,speedup,efficiency");
        debug_log(&alloc::format!(
            "[BENCH] {hypervisor},{profile},{cores},{runs},{median},{current},{best},{average},{min},{max},{spread},{stability},{single_norm},{single_raw},{multi_norm},{multi_raw},{legacy_raw_total},{speedup},{efficiency}",
            hypervisor = profile::detect_profile(),
            profile = profile,
            cores = workers,
            runs = completed_runs,
            median = median,
            current = current,
            best = best,
            average = average,
            min = min,
            max = max,
            spread = spread,
            stability = stability,
            single_norm = report.single_normalized,
            single_raw = report.single_raw,
            multi_norm = report.multi_normalized,
            multi_raw = report.multi_raw,
            legacy_raw_total = report.legacy_raw_total,
            speedup = speed,
            efficiency = eff,
        ));

        debug_log("[BENCH] ");
        debug_log("[BENCH] Per-run summary:");

        for run_idx in 0..completed_runs {
            let snapshot = self.run_summaries[run_idx];
            let mut run_speed_buf = [0u8; 16];
            let run_speed_len = write_optional_ratio_into(snapshot.speedup, &mut run_speed_buf);
            let run_speed = as_str(&run_speed_buf[..run_speed_len]);
            let mut run_eff_buf = [0u8; 8];
            let run_eff_len = write_optional_pct_into(snapshot.efficiency, &mut run_eff_buf);
            let run_eff = as_str(&run_eff_buf[..run_eff_len]);
            debug_log(&alloc::format!("[BENCH] Run {}:", run_idx + 1));
            debug_log(&alloc::format!(
                "[BENCH]   Final v2 Score: {}",
                snapshot.report.weighted_final
            ));
            debug_log(&alloc::format!(
                "[BENCH]   Single normalized/raw: {} / {}",
                snapshot.report.single_normalized,
                snapshot.report.single_raw
            ));
            debug_log(&alloc::format!(
                "[BENCH]   Multi normalized/raw: {} / {}",
                snapshot.report.multi_normalized,
                snapshot.report.multi_raw
            ));
            debug_log(&alloc::format!(
                "[BENCH]   Legacy Raw Total: {}",
                snapshot.report.legacy_raw_total
            ));
            debug_log(&alloc::format!("[BENCH]   Speedup: {}", run_speed));
            debug_log(&alloc::format!("[BENCH]   Efficiency: {}", run_eff));
            debug_log(&alloc::format!(
                "[BENCH]   Stages: {}/{} skipped={} failed={}",
                snapshot.completed_stages,
                BENCH_COUNT,
                snapshot.skipped_stages,
                snapshot.failed_stages
            ));
            debug_log(&alloc::format!("[BENCH]   Cores: {}", workers));
            debug_log(&alloc::format!(
                "[BENCH]   Hypervisor/Profile: {}",
                profile::detect_profile()
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
                let workers = entry.map(|entry| entry.workers).unwrap_or(0);
                let wall_ms = entry.map(Self::elapsed_ms).unwrap_or(0);
                debug_log(&alloc::format!(
                    "[BENCH]     stage={} state={} workers={} wall_ms={} cycles={} raw_score={}",
                    name,
                    state,
                    workers,
                    wall_ms,
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
                self.begin_stage_timing();
                self.set_status("Parallel matrix started");
                self.set_detail(
                    "Fixed-total-work: all workers split one 1024^2 matrix multiply to measure speedup.",
                );
            }
            2 => {
                self.stage = Stage::MultiSha;
                self.begin_stage_timing();
                self.set_status("Parallel SHA-256 started");
                self.set_detail(
                    "Work-per-core: each worker hashes 16 MiB so throughput scaling stays visible.",
                );
            }
            _ => {
                self.complete_current_pass();
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
            self.multi_timing_pending = true;
            self.verify_multi_core_activity();
            true
        } else if let Some(result) = multi::take_async_result() {
            let cycles = match result {
                Ok(cycles) => cycles,
                Err(()) => {
                    leave_parallel_phase();
                    self.set_status("Parallel workload failed");
                    self.set_detail("A native worker could not be created; the batch was aborted.");
                    self.running = false;
                    return true;
                }
            };
            self.verify_multi_core_activity();
            if self.multi_timing_pending {
                self.stage_started_tick = multi::measured_start_tick(workers);
                self.stage_started_ms = multi::measured_start_ms(workers);
            }
            self.multi_timing_pending = false;
            let metrics = self.stage_metrics_from_times(
                self.stage_started_tick,
                self.stage_started_ms,
                multi::measured_end_tick(workers),
                multi::measured_end_ms(workers),
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

        // ── Combined header + toolbar ────────────────────────────────────────
        let header = self.header_rect();
        canvas.fill_rect(header, theme.panel_alt);
        canvas.hbar(header.x, header.bottom() - 1, header.w, 1, theme.accent);
        Label::new(
            Rect::new(header.x + 12, header.y + 3, 120, HEADER_H - 6),
            "SunLight Bench",
        )
        .with_font(&F_MED)
        .draw(canvas, theme);
        Label::new(
            Rect::new(header.x + 116, header.y + 4, 280, HEADER_H - 8),
            self.profile_str(),
        )
        .dim()
        .with_font(&F_SMALL)
        .draw(canvas, theme);

        let buttons = self.button_rects();
        let labels = ["Run", "Serial", "Close"];
        for (idx, rect) in buttons.iter().enumerate() {
            let mut button = if idx == 0 {
                Button::new(*rect, labels[idx]).with_font(&F_SMALL)
            } else {
                Button::secondary(*rect, labels[idx]).with_font(&F_SMALL)
            };
            button.state = self.button_state(idx);
            button.draw(canvas, theme);
        }

        // ── Overview (compact) ──────────────────────────────────────────────
        let summary = self.summary_rect();
        Panel::with_title(summary, "Overview").draw(canvas, theme);

        let ov = summary.y + 22;
        StatusBadge::new(summary.x + 14, ov, self.status_badge())
            .with_label(self.stage.label())
            .draw(canvas, theme);
        Label::new(
            Rect::new(summary.x + 120, ov - 2, 260, 16),
            self.final_str(),
        )
        .with_font(&F_MED)
        .draw(canvas, theme);
        Label::new(Rect::new(summary.x + 370, ov + 1, 140, 14), self.core_str())
            .with_font(&F_UI)
            .draw(canvas, theme);
        Label::new(
            Rect::new(summary.x + 460, ov + 1, summary.w.saturating_sub(474), 14),
            self.run_str(),
        )
        .with_font(&F_UI)
        .draw(canvas, theme);

        let ov2 = ov + 18;
        Label::new(Rect::new(summary.x + 14, ov2, 390, 14), self.single_str())
            .with_font(&F_UI)
            .draw(canvas, theme);
        Label::new(Rect::new(summary.x + 400, ov2, 260, 14), self.multi_str())
            .with_font(&F_UI)
            .draw(canvas, theme);
        Label::new(Rect::new(summary.x + 660, ov2, 180, 14), self.legacy_str())
            .dim()
            .with_font(&F_SMALL)
            .draw(canvas, theme);

        let ov3 = ov2 + 16;
        Label::new(
            Rect::new(summary.x + 14, ov3, summary.w.saturating_sub(28), 14),
            self.phase_str(),
        )
        .with_font(&F_UI)
        .draw(canvas, theme);

        ProgressBar::new(
            Rect::new(
                summary.x + 14,
                summary.bottom() - 20,
                summary.w.saturating_sub(28),
                12,
            ),
            self.global_progress(),
        )
        .with_pct()
        .draw(canvas, theme);

        // ── Current Stage (collapsed when finished) ──────────────────────────
        let stage = self.stage_rect();
        if stage.h > 0 {
            Panel::with_title(stage, "Current Stage").draw(canvas, theme);
            Label::new(
                Rect::new(stage.x + 14, stage.y + 22, stage.w.saturating_sub(28), 16),
                self.stage_name(),
            )
            .with_font(&F_UI)
            .draw(canvas, theme);
            Label::new(
                Rect::new(stage.x + 14, stage.y + 40, stage.w.saturating_sub(28), 14),
                self.stage_note(),
            )
            .dim()
            .with_font(&F_SMALL)
            .draw(canvas, theme);
            ProgressBar::new(
                Rect::new(
                    stage.x + 14,
                    stage.bottom() - 28,
                    stage.w.saturating_sub(28),
                    14,
                ),
                self.stage_progress_bp() as f32 / 10_000.0,
            )
            .with_pct()
            .draw(canvas, theme);
        }

        // ── Results ─────────────────────────────────────────────────────────
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
        .with_scroll_offset(self.table_scroll_offset)
        .with_font(&F_UI)
        .draw(canvas, theme);

        StatusBar::new(
            self.status_rect(),
            self.status_str(),
            self.stage.label(),
            self.profile_str(),
        )
        .draw(canvas, theme);
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::Tick => {
                if self.running {
                    self.tick_benchmark()
                } else if self.cooldown_target != CooldownTarget::None {
                    self.tick_cooldown()
                } else {
                    false
                }
            }
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
                // Table scrolling with arrow keys / page keys
                if self.completed_runs > 0 || self.results.iter().any(|e| e.is_some()) {
                    let results = self.results_rect();
                    let table_h = results.h.saturating_sub(34);
                    let row_h = (sun_font::line_height(FontRole::UiRegular) + 5).max(16);
                    let visible = (table_h.saturating_sub(24) / row_h) as usize;
                    let max_offset = BENCH_COUNT.saturating_sub(visible);
                    match keycode {
                        KEY_DOWN => {
                            if self.table_scroll_offset < max_offset {
                                self.table_scroll_offset += 1;
                                return true;
                            }
                        }
                        KEY_UP => {
                            if self.table_scroll_offset > 0 {
                                self.table_scroll_offset -= 1;
                                return true;
                            }
                        }
                        KEY_PGDN => {
                            let step = visible.saturating_sub(1).max(1);
                            let new_offset = self
                                .table_scroll_offset
                                .saturating_add(step)
                                .min(max_offset);
                            if new_offset != self.table_scroll_offset {
                                self.table_scroll_offset = new_offset;
                                return true;
                            }
                        }
                        KEY_PGUP => {
                            let step = visible.saturating_sub(1).max(1);
                            let new_offset = self.table_scroll_offset.saturating_sub(step);
                            if new_offset != self.table_scroll_offset {
                                self.table_scroll_offset = new_offset;
                                return true;
                            }
                        }
                        _ => {}
                    }
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
        if app.running {
            app.tick_benchmark();
        } else if app.cooldown_target != CooldownTarget::None {
            app.tick_cooldown();
        }
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
