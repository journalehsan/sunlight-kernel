//! A dispatch slice is independent of cumulative task runtime. Start once on
//! dispatch, finish once at deschedule (including yield/block/same-task renewal).
//! Calibrated elapsed time includes a blocked syscall's still-executing tail.
//! Without TSC calibration only delivered timer callbacks are available.
pub const NS_PER_TICK: u64 = 10_000_000;

pub struct Slice {
    start_ns: Option<u64>,
    budget_ticks: u64,
    fallback_ticks: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Usage {
    pub ticks: u64,
    pub expired: bool,
    pub budget_ticks: u32,
}

impl Slice {
    pub const fn new() -> Self {
        Self {
            start_ns: None,
            budget_ticks: 0,
            fallback_ticks: 0,
        }
    }

    pub fn start(&mut self, now_ns: u64, budget_ticks: u32) {
        assert!(
            self.start_ns.is_none(),
            "slice started without descheduling"
        );
        self.start_ns = Some(now_ns);
        self.budget_ticks = u64::from(budget_ticks.max(1));
        self.fallback_ticks = 0;
    }

    fn usage(&self, now_ns: u64, calibrated: bool) -> Usage {
        let ticks = self.start_ns.map_or(0, |start| {
            if calibrated {
                now_ns.saturating_sub(start) / NS_PER_TICK
            } else {
                self.fallback_ticks
            }
        });
        Usage {
            ticks,
            expired: self.start_ns.is_some() && ticks >= self.budget_ticks,
            budget_ticks: self.budget_ticks as u32,
        }
    }

    pub fn timer(&mut self, now_ns: u64, calibrated: bool) -> Usage {
        if self.start_ns.is_some() {
            self.fallback_ticks = self.fallback_ticks.saturating_add(1);
        }
        self.usage(now_ns, calibrated)
    }

    pub fn finish(&mut self, now_ns: u64, calibrated: bool) -> Option<Usage> {
        self.start_ns?;
        let usage = self.usage(now_ns, calibrated);
        self.start_ns = None;
        self.fallback_ticks = 0;
        Some(usage)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_callbacks_and_previous_task_never_charge_new_dispatch() {
        let mut s = Slice::new();
        for _ in 0..100 {
            assert_eq!(s.timer(0, false).ticks, 0);
        }
        s.start(0, 5);
        assert_eq!(s.timer(0, false).ticks, 1);
        assert_eq!(
            s.finish(0, false),
            Some(Usage {
                ticks: 1,
                expired: false,
                budget_ticks: 5
            })
        );
        s.start(0, 5);
        assert_eq!(s.timer(0, false).ticks, 1);
    }

    #[test]
    fn elapsed_time_handles_delayed_and_duplicate_callbacks() {
        let mut s = Slice::new();
        s.start(100, 5);
        assert_eq!(
            s.timer(100 + 60_000_000, true),
            Usage {
                ticks: 6,
                expired: true,
                budget_ticks: 5
            }
        );
        assert_eq!(s.timer(100 + 60_000_000, true).ticks, 6);
        assert_eq!(s.finish(100 + 65_000_000, true).unwrap().ticks, 6);
        assert_eq!(s.finish(900_000_000, true), None);
    }

    #[test]
    fn yield_block_and_migration_end_the_slice_once() {
        let mut cpu0 = Slice::new();
        let mut cpu1 = Slice::new();
        cpu0.start(0, 5);
        assert_eq!(cpu0.finish(15_000_000, true).unwrap().ticks, 1);
        assert!(cpu0.finish(20_000_000, true).is_none());
        // Blocked interval belongs to neither slice; destination starts fresh.
        cpu1.start(900_000_000, 5);
        assert_eq!(cpu1.timer(910_000_000, true).ticks, 1);
        assert_eq!(cpu0.timer(910_000_000, true).ticks, 0);
    }

    #[test]
    fn fallback_budget_and_same_task_renewal() {
        let mut s = Slice::new();
        s.start(0, 2);
        assert!(!s.timer(0, false).expired);
        assert!(s.timer(0, false).expired);
        assert!(s.finish(0, false).unwrap().expired);
        s.start(0, 2);
        assert!(!s.timer(0, false).expired);
    }
}
