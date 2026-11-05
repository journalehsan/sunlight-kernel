//! Test coordination only; no production algorithm consults this state.
use core::sync::atomic::{AtomicU64, Ordering};

pub(crate) const PUBLISH: u64 = 1;
pub(crate) const CLOSE: u64 = 2;
pub(crate) struct Gate(AtomicU64);
impl Gate {
    pub(crate) const fn new() -> Self {
        Self(AtomicU64::new(0))
    }
    pub(crate) fn reset(&self) {
        self.0.store(0, Ordering::Release);
    }
    pub(crate) fn enter(&self, operation: u64) {
        self.0.fetch_or(operation, Ordering::Release);
    }
    pub(crate) fn finish(&self, operation: u64) {
        self.0.fetch_or(operation << 2, Ordering::Release);
    }
    pub(crate) fn may_run(&self, operation: u64, winner: u64) -> bool {
        let bits = self.0.load(Ordering::Acquire);
        bits & 3 == 3 && (operation == winner || bits & (winner << 2) != 0)
    }
    pub(crate) fn done(&self) -> bool {
        self.0.load(Ordering::Acquire) == 15
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn neither_winner_can_start_without_both_entrants() {
        for winner in [PUBLISH, CLOSE] {
            let gate = Gate::new();
            assert!(!gate.may_run(winner, winner));
            gate.enter(PUBLISH);
            assert!(!gate.may_run(winner, winner));
            gate.enter(CLOSE);
            assert!(gate.may_run(winner, winner));
            assert!(!gate.may_run(3 ^ winner, winner));
            gate.finish(winner);
            assert!(gate.may_run(3 ^ winner, winner));
            assert!(!gate.done());
            gate.finish(3 ^ winner);
            assert!(gate.done());
            gate.reset();
            assert!(!gate.done());
            assert!(!gate.may_run(winner, winner));
        }
    }
    #[test]
    fn independent_threads_publish_readiness_and_retirement() {
        for winner in [PUBLISH, CLOSE] {
            let gate = std::sync::Arc::new(Gate::new());
            let other = gate.clone();
            let thread = std::thread::spawn(move || {
                other.enter(CLOSE);
                while !other.may_run(CLOSE, winner) {
                    std::thread::yield_now();
                }
                other.finish(CLOSE);
            });
            gate.enter(PUBLISH);
            while !gate.may_run(PUBLISH, winner) {
                std::thread::yield_now();
            }
            gate.finish(PUBLISH);
            thread.join().unwrap();
            assert!(gate.done());
        }
    }
}
