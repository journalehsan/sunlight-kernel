//! A dispatch slice is independent of cumulative task runtime. Start once on
//! dispatch, finish once at deschedule. A priority interruption can resume the
//! same frozen entitlement; yield/block/exit/expiration retire it.
//! Calibrated elapsed time includes a blocked syscall's still-executing tail.
//! Without TSC calibration only delivered timer callbacks are available.
pub const NS_PER_TICK: u64 = 10_000_000;

pub struct Slice {
    start_ns: Option<u64>,
    budget_ticks: u64,
    fallback_ticks: u64,
    consumed_ns: u64,
}

/// One frozen entitlement, never additive credit. Carries across CPU migration.
#[derive(Debug, PartialEq, Eq)]
pub struct Credit {
    budget_ticks: u32,
    consumed_ns: u64,
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
            consumed_ns: 0,
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
        self.consumed_ns = 0;
    }

    pub fn resume(&mut self, now_ns: u64, budget_ticks: u32, credit: Option<Credit>) {
        self.start(now_ns, budget_ticks);
        if let Some(credit) = credit {
            self.budget_ticks = u64::from(credit.budget_ticks);
            self.consumed_ns = credit.consumed_ns;
            self.fallback_ticks = credit.fallback_ticks;
        }
    }

    fn elapsed_ns(&self, now_ns: u64) -> u64 {
        self.consumed_ns.saturating_add(
            self.start_ns
                .map_or(0, |start| now_ns.saturating_sub(start)),
        )
    }

    pub fn expired(&self, now_ns: u64, calibrated: bool) -> bool {
        self.usage(now_ns, calibrated).expired
    }

    fn usage(&self, now_ns: u64, calibrated: bool) -> Usage {
        let ticks = self.start_ns.map_or(0, |_| {
            if calibrated {
                self.elapsed_ns(now_ns) / NS_PER_TICK
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
        self.interrupt(now_ns, calibrated).map(|(usage, _)| usage)
    }

    /// Close actual execution once; retain only the unused part of this turn.
    pub fn interrupt(&mut self, now_ns: u64, calibrated: bool) -> Option<(Usage, Option<Credit>)> {
        self.start_ns?;
        let usage = self.usage(now_ns, calibrated);
        let credit = (!usage.expired).then(|| Credit {
            budget_ticks: usage.budget_ticks,
            consumed_ns: self.elapsed_ns(now_ns),
            fallback_ticks: self.fallback_ticks,
        });
        self.start_ns = None;
        self.fallback_ticks = 0;
        self.consumed_ns = 0;
        Some((usage, credit))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sub_tick_interruptions_never_refill_and_migration_excludes_wait() {
        let mut cpu0 = Slice::new();
        let mut cpu1 = Slice::new();
        cpu0.start(0, 5);
        let (_, credit) = cpu0.interrupt(500_000, true).unwrap();
        assert!(cpu0.finish(900_000_000, true).is_none());
        cpu1.resume(1_000_000_000, 50, credit); // original budget remains 5
        for fragment in 1..100 {
            let end = 1_000_000_000 + fragment * 500_000;
            let (usage, credit) = cpu1.interrupt(end, true).unwrap();
            assert_eq!(usage.budget_ticks, 5);
            assert_eq!(usage.expired, fragment == 99);
            if fragment < 99 {
                assert!(credit.is_some());
                cpu1.resume(end, 50, credit);
            } else {
                assert!(credit.is_none());
            }
        }
        assert!(!cpu0.expired(9_000_000_000, true));
        // Fresh dispatch after yield/sleep/exit never inherits old entitlement.
        cpu1.resume(9_000_000_000, 5, None);
        assert_eq!(cpu1.timer(9_010_000_000, true).ticks, 1);
    }

    #[test]
    fn fallback_interruption_keeps_delivered_ticks_without_idle_credit() {
        let mut s = Slice::new();
        s.start(0, 3);
        s.timer(0, false);
        let (_, credit) = s.interrupt(0, false).unwrap();
        for _ in 0..20 {
            s.timer(0, false);
        }
        s.resume(0, 50, credit);
        assert!(!s.timer(0, false).expired);
        assert!(s.timer(0, false).expired);
        assert!(s.interrupt(0, false).unwrap().1.is_none());
    }

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
