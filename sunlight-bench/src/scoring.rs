//! Score normalisation helpers shared by the GUI and serial views.
//!
//! # Why v2 scoring exists
//!
//! Raw single-core and multi-core scores are on very different scales. A typical
//! 12-core run might produce ~1.7M single-core raw vs ~26k multi-core raw. The
//! legacy total (`single_raw + multi_raw`) is therefore ~99% single-core; even
//! when N-core improves sharply (e.g. 14k → 26k), the legacy total barely moves
//! and 4-core vs 8-core vs 12-core comparisons look flat.
//!
//! v2 scoring keeps all raw data but normalises each group independently against
//! internal baseline constants (scoring calibration, not hardware truth), then
//! blends the normalised groups with equal weights so SMP scaling affects the
//! headline score.

extern crate alloc;

use alloc::string::String;

pub const SINGLE_COUNT: usize = 5;
pub const MULTI_COUNT: usize = 3;
pub const BENCH_COUNT: usize = SINGLE_COUNT + MULTI_COUNT;

/// Single-core raw sum that maps to normalised 1000. Scoring constant only.
pub const SINGLE_BASELINE: u64 = 1_000_000;

/// Multi-core raw sum that maps to normalised 1000. Scoring constant only.
pub const MULTI_BASELINE: u64 = 10_000;

/// Single-core weight in the v2 final score.
pub const SINGLE_WEIGHT: f64 = 0.50;

/// Multi-core weight in the v2 final score.
pub const MULTI_WEIGHT: f64 = 0.50;

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

/// Separated raw, normalised, legacy, and v2 headline scores.
#[derive(Clone, Copy, Debug, Default)]
pub struct ScoreReport {
    pub single_raw: u64,
    pub multi_raw: u64,
    pub single_normalized: u64,
    pub multi_normalized: u64,
    /// `single_raw + multi_raw` — compatibility/debug only.
    pub legacy_raw_total: u64,
    /// Final v2 score, derived from the normalized single- and multi-core groups.
    pub weighted_final: u64,
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

/// Sum of completed single-core stage raw scores.
pub fn compute_single_raw_score(entries: &[Option<Entry>; BENCH_COUNT]) -> u64 {
    entries[..SINGLE_COUNT]
        .iter()
        .flatten()
        .filter(|entry| entry.class == WorkloadClass::SingleCore)
        .map(|entry| entry.score)
        .sum()
}

/// Sum of completed multi-core stage raw scores.
pub fn compute_multi_raw_score(entries: &[Option<Entry>; BENCH_COUNT]) -> u64 {
    entries[SINGLE_COUNT..]
        .iter()
        .flatten()
        .map(|entry| entry.score)
        .sum()
}

/// Legacy raw total (`single_raw + multi_raw`).
pub fn compute_legacy_raw_total(entries: &[Option<Entry>; BENCH_COUNT]) -> u64 {
    compute_single_raw_score(entries).saturating_add(compute_multi_raw_score(entries))
}

fn normalize_against_baseline(raw: u64, baseline: u64) -> u64 {
    if baseline == 0 {
        return 0;
    }
    let scaled = (raw as u128)
        .saturating_mul(1000)
        .saturating_add((baseline as u128) / 2);
    scaled
        .saturating_div(baseline as u128)
        .max(1)
        .min(u64::MAX as u128) as u64
}

/// `(single_raw * 1000) / SINGLE_BASELINE`
pub fn normalize_single_score(raw: u64) -> u64 {
    normalize_against_baseline(raw, SINGLE_BASELINE)
}

/// `(multi_raw * 1000) / MULTI_BASELINE`
pub fn normalize_multi_score(raw: u64) -> u64 {
    normalize_against_baseline(raw, MULTI_BASELINE)
}

/// Equal-weighted blend of the normalized single- and multi-core scores.
pub fn compute_weighted_final_score(single_normalized: u64, multi_normalized: u64) -> u64 {
    debug_assert!((SINGLE_WEIGHT - 0.50).abs() < f64::EPSILON);
    debug_assert!((MULTI_WEIGHT - 0.50).abs() < f64::EPSILON);

    if single_normalized == 0 && multi_normalized == 0 {
        return 0;
    }

    single_normalized
        .saturating_add(multi_normalized)
        .saturating_add(1)
        / 2
}

pub fn score_report(entries: &[Option<Entry>; BENCH_COUNT]) -> ScoreReport {
    let single_raw = compute_single_raw_score(entries);
    let multi_raw = compute_multi_raw_score(entries);
    let single_normalized = normalize_single_score(single_raw);
    let multi_normalized = normalize_multi_score(multi_raw);
    let legacy_raw_total = compute_legacy_raw_total(entries);
    let weighted_final = compute_weighted_final_score(single_normalized, multi_normalized);
    ScoreReport {
        single_raw,
        multi_raw,
        single_normalized,
        multi_normalized,
        legacy_raw_total,
        weighted_final,
    }
}

/// Serial/debug summary block for the v2 scoring model.
#[allow(dead_code)]
pub fn format_bench_v2_summary(
    report: ScoreReport,
    cores: usize,
    stages_completed: usize,
    speedup: &str,
    efficiency: &str,
    hypervisor: &str,
) -> String {
    alloc::format!(
        "SunLight Bench v2 Summary\n\
         Hypervisor/Profile: {hypervisor}\n\
         Cores: {cores}\n\
         Stages completed: {stages_completed}/{BENCH_COUNT}\n\
         \n\
         Final v2 Score: {weighted_final}\n\
         Single normalized/raw: {single_normalized} / {single_raw}\n\
         Multi normalized/raw: {multi_normalized} / {multi_raw}\n\
         Legacy Raw Total: {legacy_raw_total}\n\
         \n\
         Speedup: {speedup}\n\
         Efficiency: {efficiency}",
        hypervisor = hypervisor,
        cores = cores,
        stages_completed = stages_completed,
        single_raw = report.single_raw,
        multi_raw = report.multi_raw,
        legacy_raw_total = report.legacy_raw_total,
        single_normalized = report.single_normalized,
        multi_normalized = report.multi_normalized,
        weighted_final = report.weighted_final,
        speedup = speedup,
        efficiency = efficiency,
    )
}

/// Legacy aliases kept for compatibility with older call sites.
#[allow(dead_code)]
pub fn single_score(entries: &[Option<Entry>; BENCH_COUNT]) -> u64 {
    compute_single_raw_score(entries)
}

#[allow(dead_code)]
pub fn multi_score(entries: &[Option<Entry>; BENCH_COUNT]) -> u64 {
    compute_multi_raw_score(entries)
}

#[allow(dead_code)]
pub fn multi_fixed_score(entries: &[Option<Entry>; BENCH_COUNT]) -> u64 {
    entries[SINGLE_COUNT..]
        .iter()
        .flatten()
        .filter(|entry| entry.class == WorkloadClass::MultiFixedTotalWork)
        .map(|entry| entry.score)
        .sum()
}

#[allow(dead_code)]
pub fn multi_work_per_core_score(entries: &[Option<Entry>; BENCH_COUNT]) -> u64 {
    entries[SINGLE_COUNT..]
        .iter()
        .flatten()
        .filter(|entry| entry.class == WorkloadClass::MultiWorkPerCore)
        .map(|entry| entry.score)
        .sum()
}

/// Legacy alias for [`compute_legacy_raw_total`].
#[allow(dead_code)]
pub fn total_score(entries: &[Option<Entry>; BENCH_COUNT]) -> u64 {
    compute_legacy_raw_total(entries)
}

/// Spread percentage from min/max relative to the average: `(max - min) * 100 / average`.
pub fn spread_pct_from_scores(scores: &[u64]) -> u64 {
    if scores.is_empty() {
        return 0;
    }
    let min = scores.iter().copied().min().unwrap_or(0);
    let max = scores.iter().copied().max().unwrap_or(0);
    let sum: u128 = scores.iter().map(|score| *score as u128).sum();
    let average = ((sum + (scores.len() as u128 / 2)) / scores.len() as u128) as u64;
    if average == 0 {
        0
    } else {
        let spread = max.saturating_sub(min) as u128;
        ((spread.saturating_mul(100) + (average as u128 / 2)) / average as u128) as u64
    }
}

/// Median of an unsorted score slice.
pub fn median_score(scores: &[u64]) -> u64 {
    if scores.is_empty() {
        return 0;
    }
    let mut sorted = alloc::vec::Vec::with_capacity(scores.len());
    sorted.extend_from_slice(scores);
    sorted.sort_unstable();
    let mid = sorted.len() / 2;
    if sorted.len() % 2 == 0 {
        (sorted[mid - 1] + sorted[mid]).saturating_add(1) / 2
    } else {
        sorted[mid]
    }
}

/// Human-readable stability label for repeat-run spread.
pub fn spread_stability_class(spread_pct: u64) -> &'static str {
    if spread_pct <= 5 {
        "excellent"
    } else if spread_pct <= 10 {
        "good"
    } else if spread_pct <= 15 {
        "acceptable"
    } else if spread_pct <= 20 {
        "noisy"
    } else {
        "unstable"
    }
}

/// True when repeat-run variance is high enough to warn consumers.
pub fn spread_is_unstable(spread_pct: u64) -> bool {
    spread_pct > 20
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(score: u64, class: WorkloadClass) -> Entry {
        Entry {
            score,
            class,
            ..Entry::default()
        }
    }

    #[test]
    fn score_report_keeps_legacy_raw_total_and_v2_blend() {
        let mut entries = [None; BENCH_COUNT];
        entries[0] = Some(entry(1_529_429, WorkloadClass::SingleCore));
        entries[5] = Some(entry(14_703, WorkloadClass::MultiWorkPerCore));

        let report = score_report(&entries);

        assert_eq!(report.single_raw, 1_529_429);
        assert_eq!(report.multi_raw, 14_703);
        assert_eq!(report.legacy_raw_total, 1_544_132);
        assert_eq!(report.single_normalized, 1_529);
        assert_eq!(report.multi_normalized, 1_470);
        assert_eq!(report.weighted_final, 1_500);
        assert_eq!(
            report.legacy_raw_total,
            report.single_raw + report.multi_raw
        );
        assert_eq!(
            report.weighted_final,
            compute_weighted_final_score(report.single_normalized, report.multi_normalized)
        );
    }

    #[test]
    fn spread_and_median_helpers() {
        let scores = [672u64, 835, 1156];
        assert_eq!(median_score(&scores), 835);
        assert_eq!(spread_stability_class(4), "excellent");
        assert_eq!(spread_stability_class(8), "good");
        assert_eq!(spread_stability_class(12), "acceptable");
        assert_eq!(spread_stability_class(18), "noisy");
        assert_eq!(spread_stability_class(58), "unstable");
        assert!(spread_is_unstable(21));
        assert!(!spread_is_unstable(20));
    }

    #[test]
    fn v2_summary_uses_clear_labels() {
        let report = ScoreReport {
            single_raw: 1_529_429,
            multi_raw: 14_703,
            single_normalized: 1_529,
            multi_normalized: 1_470,
            legacy_raw_total: 1_544_132,
            weighted_final: 1_500,
        };

        let summary = format_bench_v2_summary(report, 12, 8, "2.00x", "83%", "QEMU");

        assert!(summary.contains("Final v2 Score: 1500"));
        assert!(summary.contains("Single normalized/raw: 1529 / 1529429"));
        assert!(summary.contains("Multi normalized/raw: 1470 / 14703"));
        assert!(summary.contains("Legacy Raw Total: 1544132"));
    }
}
