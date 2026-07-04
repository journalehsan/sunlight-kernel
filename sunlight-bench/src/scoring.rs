//! Score normalisation helpers shared by the GUI and serial views.
//!
//! # v2 weighted total
//!
//! Per-stage raw scores use different formulas for single-core (`score_single`) and
//! multi-core (`score_multi`). On typical hardware the single-core group sums to
//! roughly 1.4M while the multi-core group stays in the low thousands, so adding
//! raw group totals (`legacy_total`) makes the headline score almost insensitive to
//! SMP scaling (4-core vs 12-core multi-core gains barely move Total).
//!
//! v2 normalises each group independently against its reference sum (stage count ×
//! 1000 at baseline hardware), then combines with configurable weights so single-
//! and multi-core contributions are comparable.

pub const SINGLE_COUNT: usize = 5;
pub const MULTI_COUNT: usize = 3;
pub const BENCH_COUNT: usize = SINGLE_COUNT + MULTI_COUNT;

/// Per-stage score on reference hardware (see `score_single` / `score_multi`).
pub const REFERENCE_STAGE_SCORE: u64 = 1_000;

/// Expected raw single-core group sum at reference hardware.
pub const SINGLE_GROUP_BASELINE: u64 = (SINGLE_COUNT as u64) * REFERENCE_STAGE_SCORE;

/// Expected raw multi-core group sum at reference hardware.
pub const MULTI_GROUP_BASELINE: u64 = (MULTI_COUNT as u64) * REFERENCE_STAGE_SCORE;

/// Weight (percent) of normalised single-core score in the v2 total. Pair with
/// [`WEIGHT_MULTI_CORE`]; both should sum to 100.
pub const WEIGHT_SINGLE_CORE: u32 = 50;

/// Weight (percent) of normalised multi-core score in the v2 total.
pub const WEIGHT_MULTI_CORE: u32 = 50;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkloadClass {
    SingleCore,
    MultiFixedTotalWork,
    MultiWorkPerCore,
}

impl WorkloadClass {
    pub fn label(self) -> &'static str {
        match self {
            Self::SingleCore => "single-core",
            Self::MultiFixedTotalWork => "fixed-total-work",
            Self::MultiWorkPerCore => "work-per-core",
        }
    }
}

/// Baseline cycles per benchmark (reference: 1 GHz single-core, in-order CPU).
pub const BASELINES: [(&str, u64); BENCH_COUNT] = [
    ("Pi (Machin fixed-pt, 200k iters)", 4_000_000_000),
    ("Prime Sieve 100k (dense)", 120_000_000),
    ("Segmented Sieve (primes <= 10^8)", 2_000_000_000),
    ("Matrix Multiply 1024^2 (i32, ikj)", 8_000_000_000),
    ("CPU Mix (Geekbench-style)", 1_000_000_000),
    ("Parallel Integer Mix (64M ops/core)", 1_500_000_000),
    ("Parallel Matrix Multiply 1024^2", 2_000_000_000),
    ("Parallel SHA-256 (16 MiB/core)", 3_000_000_000),
];

/// Benchmark work units used for throughput scoring and reporting.
pub const WORK_UNITS: [u64; BENCH_COUNT] = [
    200_000,
    100_000,
    100_000_000,
    1_073_741_824,
    192,
    64_000_000,
    1_073_741_824,
    16_777_216,
];

pub const WORKLOAD_CLASSES: [WorkloadClass; BENCH_COUNT] = [
    WorkloadClass::SingleCore,
    WorkloadClass::SingleCore,
    WorkloadClass::SingleCore,
    WorkloadClass::SingleCore,
    WorkloadClass::SingleCore,
    WorkloadClass::MultiWorkPerCore,
    WorkloadClass::MultiFixedTotalWork,
    WorkloadClass::MultiWorkPerCore,
];

#[derive(Clone, Copy, Debug)]
pub struct Entry {
    pub name: &'static str,
    pub start_tick: u64,
    pub end_tick: u64,
    pub start_ms: u64,
    pub end_ms: u64,
    pub work_units: u64,
    pub workers: u32,
    pub cycles: u64,
    pub class: WorkloadClass,
    pub score: u64,
}

impl Default for Entry {
    fn default() -> Self {
        Self {
            name: "",
            start_tick: 0,
            end_tick: 0,
            start_ms: 0,
            end_ms: 0,
            work_units: 0,
            workers: 0,
            cycles: 0,
            class: WorkloadClass::SingleCore,
            score: 0,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct StageMetrics {
    pub start_tick: u64,
    pub end_tick: u64,
    pub start_ms: u64,
    pub end_ms: u64,
    pub work_units: u64,
    pub workers: u32,
    pub cycles: u64,
    pub class: WorkloadClass,
}

/// Separated raw and normalised score groups plus legacy and v2 totals.
#[derive(Clone, Copy, Debug, Default)]
pub struct ScoreReport {
    pub single_raw: u64,
    pub multi_raw: u64,
    pub single_normalized: u64,
    pub multi_normalized: u64,
    /// Raw single + raw multi. Kept for compatibility; dominated by single-core.
    pub legacy_total: u64,
    /// `WEIGHT_SINGLE_CORE` / `WEIGHT_MULTI_CORE` blend of normalised groups.
    pub weighted_total: u64,
}

fn baseline_for(idx: usize) -> u64 {
    BASELINES
        .get(idx)
        .map(|(_, baseline)| *baseline)
        .unwrap_or(1_000_000_000)
}

fn work_units_for(idx: usize) -> u64 {
    WORK_UNITS.get(idx).copied().unwrap_or(1)
}

fn class_for(idx: usize) -> WorkloadClass {
    WORKLOAD_CLASSES
        .get(idx)
        .copied()
        .unwrap_or(WorkloadClass::SingleCore)
}

fn score_single(baseline_cycles: u64, cycles: u64) -> u64 {
    if cycles == 0 {
        0
    } else {
        let scaled = ((baseline_cycles as u128) * 1000 + (cycles as u128 / 2)) / cycles as u128;
        scaled.max(1).min(u64::MAX as u128) as u64
    }
}

fn score_multi(baseline_cycles: u64, baseline_work_units: u64, metrics: &StageMetrics) -> u64 {
    if baseline_work_units == 0 || metrics.work_units == 0 {
        return 0;
    }

    let elapsed_ms = if metrics.end_ms > metrics.start_ms {
        metrics.end_ms - metrics.start_ms
    } else {
        metrics.end_tick.saturating_sub(metrics.start_tick) / 1_000_000
    }
    .max(1);
    let total_work_units = metrics.work_units;
    if total_work_units == 0 {
        return 0;
    }

    let numerator = (total_work_units as u128)
        .saturating_mul(baseline_cycles as u128)
        .saturating_mul(1000);
    let denominator = (baseline_work_units as u128)
        .saturating_mul(elapsed_ms as u128)
        .saturating_mul(1_000_000);
    if denominator == 0 {
        return 0;
    }
    let scaled = (numerator + (denominator / 2)) / denominator;
    scaled.max(1).min(u64::MAX as u128) as u64
}

pub fn make_entry(idx: usize, name: &'static str, metrics: StageMetrics) -> Entry {
    let baseline_cycles = baseline_for(idx);
    let baseline_work_units = work_units_for(idx);
    debug_assert_eq!(class_for(idx), metrics.class);
    let score = match metrics.class {
        WorkloadClass::SingleCore => score_single(baseline_cycles, metrics.cycles),
        WorkloadClass::MultiFixedTotalWork | WorkloadClass::MultiWorkPerCore => {
            score_multi(baseline_cycles, baseline_work_units, &metrics)
        }
    };

    Entry {
        name,
        start_tick: metrics.start_tick,
        end_tick: metrics.end_tick,
        start_ms: metrics.start_ms,
        end_ms: metrics.end_ms,
        work_units: metrics.work_units,
        workers: metrics.workers,
        cycles: metrics.cycles,
        class: metrics.class,
        score,
    }
}

/// Scale a raw group sum to reference hardware (1000 = baseline group performance).
pub fn normalize_group_score(raw: u64, group_baseline: u64) -> u64 {
    if group_baseline == 0 {
        return 0;
    }
    let scaled = (raw as u128)
        .saturating_mul(REFERENCE_STAGE_SCORE as u128)
        .saturating_add((group_baseline as u128) / 2);
    scaled
        .saturating_div(group_baseline as u128)
        .max(1)
        .min(u64::MAX as u128) as u64
}

/// v2 headline score: equal-weighted blend of normalised single- and multi-core groups.
pub fn weighted_total_v2(single_normalized: u64, multi_normalized: u64) -> u64 {
    let weight_sum = (WEIGHT_SINGLE_CORE + WEIGHT_MULTI_CORE).max(1) as u128;
    let blended = (single_normalized as u128)
        .saturating_mul(WEIGHT_SINGLE_CORE as u128)
        .saturating_add((multi_normalized as u128).saturating_mul(WEIGHT_MULTI_CORE as u128));
    (blended.saturating_add(weight_sum / 2) / weight_sum)
        .max(1)
        .min(u64::MAX as u128) as u64
}

pub fn score_report(entries: &[Option<Entry>; BENCH_COUNT]) -> ScoreReport {
    let single_raw = single_score(entries);
    let multi_raw = multi_score(entries);
    let single_normalized = normalize_group_score(single_raw, SINGLE_GROUP_BASELINE);
    let multi_normalized = normalize_group_score(multi_raw, MULTI_GROUP_BASELINE);
    let legacy_total = total_score(entries);
    let weighted_total = weighted_total_v2(single_normalized, multi_normalized);
    ScoreReport {
        single_raw,
        multi_raw,
        single_normalized,
        multi_normalized,
        legacy_total,
        weighted_total,
    }
}

pub fn single_score(entries: &[Option<Entry>; BENCH_COUNT]) -> u64 {
    entries[..SINGLE_COUNT]
        .iter()
        .flatten()
        .filter(|entry| entry.class == WorkloadClass::SingleCore)
        .map(|entry| entry.score)
        .sum()
}

pub fn multi_score(entries: &[Option<Entry>; BENCH_COUNT]) -> u64 {
    entries[SINGLE_COUNT..]
        .iter()
        .flatten()
        .map(|entry| entry.score)
        .sum()
}

pub fn multi_fixed_score(entries: &[Option<Entry>; BENCH_COUNT]) -> u64 {
    entries[SINGLE_COUNT..]
        .iter()
        .flatten()
        .filter(|entry| entry.class == WorkloadClass::MultiFixedTotalWork)
        .map(|entry| entry.score)
        .sum()
}

pub fn multi_work_per_core_score(entries: &[Option<Entry>; BENCH_COUNT]) -> u64 {
    entries[SINGLE_COUNT..]
        .iter()
        .flatten()
        .filter(|entry| entry.class == WorkloadClass::MultiWorkPerCore)
        .map(|entry| entry.score)
        .sum()
}

/// Legacy total: raw single-core plus raw multi-core. Not used for v2 headline score.
pub fn total_score(entries: &[Option<Entry>; BENCH_COUNT]) -> u64 {
    single_score(entries).saturating_add(multi_score(entries))
}
