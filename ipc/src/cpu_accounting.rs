//! Integer-only CPU accounting shared by the kernel and telemetry readers.

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct CpuCounters {
    pub elapsed_ns: u64,
    pub busy_ns: u64,
    pub idle_ns: u64,
    pub switches: u64,
    pub schedules: u64,
    pub wakeups: u64,
    pub idle_entries: u64,
    pub idle_exits: u64,
    pub timer_irqs: u64,
    pub ipc_wakeups: u64,
}

impl CpuCounters {
    pub const ZERO: Self = Self {
        elapsed_ns: 0,
        busy_ns: 0,
        idle_ns: 0,
        switches: 0,
        schedules: 0,
        wakeups: 0,
        idle_entries: 0,
        idle_exits: 0,
        timer_irqs: 0,
        ipc_wakeups: 0,
    };
}

/// Preserve real sub-basis-point work as <0.1% rather than manufacturing zero.
pub fn usage_bp(runtime: u64, capacity: u64) -> u16 {
    if capacity == 0 || runtime == 0 {
        return 0;
    }
    ((runtime as u128 * 10_000 / capacity as u128).clamp(1, 10_000)) as u16
}

pub fn rate(delta: u64, elapsed_ns: u64) -> u64 {
    if elapsed_ns == 0 {
        return 0;
    }
    (delta as u128 * 1_000_000_000 / elapsed_ns as u128).min(u64::MAX as u128) as u64
}

/// Open execution intervals are independent of scheduler state (a task may
/// already be marked Blocked while returning from the blocking syscall).
pub fn runtime_at(committed: u64, since: Option<u64>, now: u64) -> u64 {
    committed.saturating_add(since.map(|start| now.saturating_sub(start)).unwrap_or(0))
}

pub fn checkpoint(committed: &mut u64, since: &mut Option<u64>, now: u64, stop: bool) {
    *committed = runtime_at(*committed, *since, now);
    *since = if stop { None } else { since.map(|_| now) };
}

/// Outcome of charging one execution interval. A bad interval is *reported*,
/// never folded into a plausible-looking total.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Charge {
    /// Interval accepted; this many nanoseconds were committed.
    Ok(u64),
    /// The observed clock precedes the interval start (counter regressed or the
    /// actor was sampled with a different clock epoch).
    Backward { start: u64, now: u64 },
    /// Committing would exceed `u64::MAX`; the accumulator is unchanged.
    Exhausted { committed: u64 },
}

/// One actor's scheduled-execution accumulator with an explicitly open or
/// closed interval.
///
/// The same arithmetic covers migration, preemption and syscall tails: the
/// caller chooses *when* to close and reopen, and the type guarantees a closed
/// interval is charged exactly once. Migration is `close_at` + `open_at`;
/// there is no separate cross-CPU accumulator, so no double charge on the
/// interval that straddles the migration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct ActorTime {
    pub committed_ns: u64,
    pub since_ns: u64,
    pub open: bool,
    pub discontinuities: u64,
    pub switches: u64,
}

impl ActorTime {
    pub const IDLE: Self = Self {
        committed_ns: 0,
        since_ns: 0,
        open: false,
        discontinuities: 0,
        switches: 0,
    };

    /// Begin a scheduled interval. Re-opening an already-open accumulator is a
    /// no-op so a duplicate dispatch notification cannot double-charge.
    pub fn open_at(&mut self, now: u64) {
        if !self.open {
            self.open = true;
            self.since_ns = now;
        }
    }

    /// End a scheduled interval and commit it. A close without an open interval
    /// is a no-op; that is the only silent path, and it cannot invent time.
    pub fn close_at(&mut self, now: u64) -> Charge {
        if !self.open {
            return Charge::Ok(0);
        }
        self.open = false;
        self.switches = self.switches.saturating_add(1);
        let Some(delta) = now.checked_sub(self.since_ns) else {
            self.discontinuities = self.discontinuities.saturating_add(1);
            return Charge::Backward {
                start: self.since_ns,
                now,
            };
        };
        match self.committed_ns.checked_add(delta) {
            Some(total) => {
                self.committed_ns = total;
                Charge::Ok(delta)
            }
            None => {
                self.discontinuities = self.discontinuities.saturating_add(1);
                Charge::Exhausted {
                    committed: self.committed_ns,
                }
            }
        }
    }

    /// Snapshot a *running* actor without ending its interval. The returned
    /// value is what `committed_ns` would become if it stopped at `now`.
    pub fn running_at(&self, now: u64) -> Charge {
        if !self.open {
            return Charge::Ok(self.committed_ns);
        }
        let Some(delta) = now.checked_sub(self.since_ns) else {
            return Charge::Backward {
                start: self.since_ns,
                now,
            };
        };
        match self.committed_ns.checked_add(delta) {
            Some(total) => Charge::Ok(total),
            None => Charge::Exhausted {
                committed: self.committed_ns,
            },
        }
    }
}

/// Result of reconciling per-actor CPU time against an independently observed
/// CPU total for the same window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reconcile {
    /// Every part fit inside the total. `slack` is time not attributed to a
    /// measured actor (idle, unmeasured tasks, or observation gaps).
    Consistent { sum: u64, slack: u64 },
    /// The parts exceed the total: at least one interval was double-charged.
    Inconsistent { sum: u64, total: u64 },
}

/// Sum actor intervals and compare with a CPU total from the same clock domain.
pub fn reconcile(parts: &[u64], total_ns: u64) -> Reconcile {
    let mut sum: u64 = 0;
    for part in parts {
        match sum.checked_add(*part) {
            Some(v) => sum = v,
            None => {
                return Reconcile::Inconsistent {
                    sum: u64::MAX,
                    total: total_ns,
                }
            }
        }
    }
    if sum > total_ns {
        Reconcile::Inconsistent {
            sum,
            total: total_ns,
        }
    } else {
        Reconcile::Consistent {
            sum,
            slack: total_ns - sum,
        }
    }
}

/// Wall time minus charged CPU time, but only when the wall clock is at least
/// as large. This is a *scheduling-gap* figure, not qualified blocked time:
/// without a trustworthy clock domain the subtraction is refused.
pub fn scheduling_gap(wall_ns: u64, charged_ns: u64) -> Option<u64> {
    wall_ns.checked_sub(charged_ns)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn migrating_task_commits_once_per_cpu() {
        let mut total = 0;
        let mut since = Some(0);
        checkpoint(&mut total, &mut since, 40, true); // CPU 0 out
        assert_eq!(runtime_at(total, since, 100), 40);
        since = Some(100); // CPU 3 in
        assert_eq!(runtime_at(total, since, 120), 60);
        checkpoint(&mut total, &mut since, 130, true);
        checkpoint(&mut total, &mut since, 200, true);
        assert_eq!(total, 70);
    }
    #[test]
    fn blocked_syscall_tail_and_running_snapshot_are_counted() {
        let mut total = 0;
        let mut since = Some(10);
        checkpoint(&mut total, &mut since, 20, false); // marked blocked
        assert_eq!(runtime_at(total, since, 25), 15);
        checkpoint(&mut total, &mut since, 30, true); // actually descheduled
        assert_eq!(runtime_at(total, since, 1_000), 20);
    }
    #[test]
    fn long_uptime_saturates_without_wrapping() {
        assert_eq!(runtime_at(u64::MAX - 1, Some(0), 10), u64::MAX);
        assert_eq!(usage_bp(u64::MAX / 4, u64::MAX), 2499);
        assert_eq!(usage_bp(u64::MAX, u64::MAX), 10_000);
        assert_eq!(rate(u64::MAX, 1), u64::MAX);
        assert_eq!(runtime_at(50, Some(100), 90), 50);
    }
    #[test]
    fn tiny_real_work_is_not_rounded_to_zero() {
        assert_eq!(usage_bp(1, 4_000_000_000), 1);
        assert_eq!(usage_bp(0, 4_000_000_000), 0);
    }

    #[test]
    fn actor_time_charges_migration_exactly_once() {
        let mut t = ActorTime::IDLE;
        t.open_at(100);
        assert_eq!(t.close_at(140), Charge::Ok(40)); // running on CPU 0
        t.open_at(180); // resumed on CPU 3
        assert_eq!(t.close_at(200), Charge::Ok(20));
        assert_eq!(t.committed_ns, 60);
        assert_eq!(t.switches, 2);
        assert_eq!(t.discontinuities, 0);
    }

    #[test]
    fn duplicate_open_does_not_double_charge() {
        let mut t = ActorTime::IDLE;
        t.open_at(10);
        t.open_at(12); // duplicate dispatch notification
        assert_eq!(t.close_at(20), Charge::Ok(10));
        assert_eq!(t.close_at(30), Charge::Ok(0)); // close without open is inert
        assert_eq!(t.committed_ns, 10);
    }

    #[test]
    fn backward_clock_is_a_discontinuity_not_a_negative_charge() {
        let mut t = ActorTime::IDLE;
        t.open_at(500);
        assert_eq!(
            t.close_at(400),
            Charge::Backward {
                start: 500,
                now: 400
            }
        );
        assert_eq!(t.committed_ns, 0);
        assert_eq!(t.discontinuities, 1);
    }

    #[test]
    fn exhausted_accumulator_is_reported_and_leaves_state_stable() {
        let mut t = ActorTime::IDLE;
        t.committed_ns = u64::MAX - 1;
        t.open_at(0);
        assert_eq!(
            t.close_at(10),
            Charge::Exhausted {
                committed: u64::MAX - 1
            }
        );
        assert_eq!(t.discontinuities, 1);
    }

    #[test]
    fn reconciliation_accepts_slack_and_rejects_double_charging() {
        assert_eq!(
            reconcile(&[10, 20, 30], 100),
            Reconcile::Consistent { sum: 60, slack: 40 }
        );
        assert_eq!(
            reconcile(&[60, 60], 100),
            Reconcile::Inconsistent {
                sum: 120,
                total: 100
            }
        );
        assert_eq!(
            reconcile(&[u64::MAX, 1], 5),
            Reconcile::Inconsistent {
                sum: u64::MAX,
                total: 5
            }
        );
    }

    #[test]
    fn scheduling_gap_refuses_inverted_wall_and_runtime() {
        assert_eq!(scheduling_gap(1_000, 400), Some(600));
        assert_eq!(scheduling_gap(400, 1_000), None);
    }
}
