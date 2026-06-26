//! Score normalisation and ASCII results table printed via serial debug_log.
//!
//! Scoring formula:
//!   individual_score = (BASELINE_CYCLES / measured_cycles) * 1000
//!   total_score      = sum of all individual scores
//!
//! Baselines represent expected cycles on a modest single-core 1 GHz reference.

/// Baseline cycles per benchmark (reference: 1 GHz single-core, in-order CPU).
const BASELINES: &[(&str, u64)] = &[
    ("Pi (Machin fixed-pt, 200k iters)", 4_000_000_000),
    ("Segmented Sieve (primes ≤ 10^8)", 2_000_000_000),
    ("Matrix Multiply 1024² (i32, ikj)", 8_000_000_000),
    ("SHA-256 parallel (1 MiB/core)",    1_000_000_000),
];

pub struct Entry {
    pub name: &'static str,
    pub cycles: u64,
    pub score: u64,
}

pub struct Results {
    entries: alloc::vec::Vec<Entry>,
}

impl Results {
    pub fn new() -> Self {
        Self { entries: alloc::vec::Vec::new() }
    }

    pub fn record(&mut self, name: &'static str, cycles: u64) {
        let baseline = BASELINES
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, b)| *b)
            .unwrap_or(1_000_000_000);
        let score = if cycles == 0 {
            0
        } else {
            (baseline / cycles) * 1000
        };
        sunlight_ipc::debug_log(&alloc::format!(
            "[BENCH] {:.<40} {:>14} cycles  score {:>6}",
            name,
            cycles,
            score
        ));
        self.entries.push(Entry { name, cycles, score });
    }

    pub fn print_table(&self) {
        let total: u64 = self.entries.iter().map(|e| e.score).sum();

        sunlight_ipc::debug_log(
            "╔══════════════════════════════════════════════╦═════════════════╦════════╗"
        );
        sunlight_ipc::debug_log(
            "║           SUNLIGHT-BENCH RESULTS             ║     Cycles      ║ Score  ║"
        );
        sunlight_ipc::debug_log(
            "╠══════════════════════════════════════════════╬═════════════════╬════════╣"
        );

        for e in &self.entries {
            let line = alloc::format!(
                "║ {:<44} ║ {:>15} ║ {:>6} ║",
                e.name,
                e.cycles,
                e.score
            );
            sunlight_ipc::debug_log(&line);
        }

        sunlight_ipc::debug_log(
            "╠══════════════════════════════════════════════╬═════════════════╬════════╣"
        );
        let total_line = alloc::format!(
            "║ {:<44} ║ {:>15} ║ {:>6} ║",
            "TOTAL SCORE",
            "",
            total
        );
        sunlight_ipc::debug_log(&total_line);
        sunlight_ipc::debug_log(
            "╚══════════════════════════════════════════════╩═════════════════╩════════╝"
        );
    }
}
