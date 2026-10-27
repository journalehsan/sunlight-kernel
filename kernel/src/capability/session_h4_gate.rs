//! Bounded rendezvous state, shared by the actual native hooks and host tests.
use core::sync::atomic::{AtomicU64, Ordering};

pub(crate) const GRANT_READY: u64 = 1;
pub(crate) const EXIT_READY: u64 = 2;
pub(crate) const GRANT_DONE: u64 = 4;
pub(crate) const RETIRED: u64 = 8;
const ABORTED: u64 = 16;

pub(crate) struct Gate(AtomicU64);
impl Gate {
    pub(crate) const fn new() -> Self {
        Self(AtomicU64::new(0))
    }
    pub(crate) fn publish(&self, bit: u64) {
        self.0.fetch_or(bit, Ordering::Release);
    }
    pub(crate) fn released(&self, case: u64, grant: bool) -> bool {
        let bits = self.0.load(Ordering::Acquire);
        bits & ABORTED != 0
            || (bits & (GRANT_READY | EXIT_READY) == GRANT_READY | EXIT_READY
                && if grant {
                    case != 14 || bits & RETIRED != 0
                } else {
                    case != 13 || bits & GRANT_DONE != 0
                })
    }
    pub(crate) fn abort(&self) {
        self.publish(ABORTED);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn both_ready_and_winner_transition_required() {
        for case in 13..=15 {
            let gate = Gate::new();
            assert!(!gate.released(case, true));
            gate.publish(GRANT_READY);
            assert!(!gate.released(case, false));
            gate.publish(EXIT_READY);
            assert_eq!(gate.released(case, true), case != 14);
            assert_eq!(gate.released(case, false), case != 13);
            gate.publish(GRANT_DONE | RETIRED);
            assert!(gate.released(case, true));
            assert!(gate.released(case, false));
        }
    }
    #[test]
    fn timeout_releases_both_and_late_events_do_not_rearm() {
        let gate = Gate::new();
        gate.publish(GRANT_READY);
        gate.abort();
        assert!(gate.released(13, true));
        assert!(gate.released(14, false));
        gate.publish(EXIT_READY);
        assert!(gate.released(14, true));
    }
    #[test]
    fn cross_cpu_publication_has_no_lost_readiness() {
        let gate = std::sync::Arc::new(Gate::new());
        let other = gate.clone();
        let thread = std::thread::spawn(move || other.publish(EXIT_READY));
        gate.publish(GRANT_READY);
        thread.join().unwrap();
        assert!(gate.released(15, true));
        assert!(gate.released(15, false));
    }
}
