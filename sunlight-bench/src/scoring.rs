//! Score normalisation helpers shared by the GUI and serial views.

pub const SINGLE_COUNT: usize = 5;
pub const MULTI_COUNT: usize = 3;
pub const BENCH_COUNT: usize = SINGLE_COUNT + MULTI_COUNT;

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

#[derive(Clone, Copy, Debug, Default)]
pub struct Entry {
    pub name: &'static str,
    pub cycles: u64,
    pub score: u64,
}

pub fn score_for(name: &str, cycles: u64) -> u64 {
    let baseline = BASELINES
        .iter()
        .find(|(bench_name, _)| *bench_name == name)
        .map(|(_, baseline)| *baseline)
        .unwrap_or(1_000_000_000);
    if cycles == 0 {
        0
    } else {
        let scaled = ((baseline as u128) * 1000 + (cycles as u128 / 2)) / cycles as u128;
        scaled.max(1).min(u64::MAX as u128) as u64
    }
}

pub fn make_entry(name: &'static str, cycles: u64) -> Entry {
    Entry {
        name,
        cycles,
        score: score_for(name, cycles),
    }
}

pub fn single_score(entries: &[Option<Entry>; BENCH_COUNT]) -> u64 {
    entries[..SINGLE_COUNT]
        .iter()
        .flatten()
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

pub fn total_score(entries: &[Option<Entry>; BENCH_COUNT]) -> u64 {
    single_score(entries).saturating_add(multi_score(entries))
}
