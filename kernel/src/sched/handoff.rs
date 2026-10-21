//! Physical outgoing-stack ownership. Slots never move with the process Vec.
//! Pin before publishing an outgoing task under SCHEDULER, release in assembly
//! only after the last access to its stack. Local IRQs must stay disabled.
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

pub const EMPTY: usize = usize::MAX;

/// `pin` must stay at offset 0 and `waiters` at offset 8: the naked epilogues
/// release `pin` with `xchg` and test `waiters` relative to the same pointer.
#[repr(C)]
#[repr(align(64))]
struct Slot {
    pin: AtomicUsize,
    /// CPUs that rejected a task pinned here and returned to idle/run. Drained
    /// by the releasing CPU, which owes each of them a scheduling event.
    waiters: AtomicU64,
}

pub struct Handoffs<const N: usize>([Slot; N]);

impl<const N: usize> Handoffs<N> {
    pub const fn new() -> Self {
        Self(
            [const {
                Slot {
                    pin: AtomicUsize::new(EMPTY),
                    waiters: AtomicU64::new(0),
                }
            }; N],
        )
    }

    pub fn pin(&self, cpu: usize, task: usize) {
        assert_ne!(task, EMPTY);
        self.0[cpu]
            .pin
            .compare_exchange(EMPTY, task, Ordering::Release, Ordering::Relaxed)
            .expect("nested outgoing stack handoff");
    }

    pub fn is_pinned(&self, cpu: usize, task: usize) -> bool {
        self.0[cpu].pin.load(Ordering::Acquire) == task
    }

    /// Register `waiter` for the release of `task` pinned on `owner`, then
    /// re-check. Returns true if the pin is still held, in which case the
    /// releasing CPU is guaranteed to observe the registration (the RMW is a
    /// full barrier and the release is an `xchg` followed by a waiters load).
    /// Returns false if the pin was already released; the caller proceeds.
    pub fn wait_release(&self, owner: usize, waiter: usize, task: usize) -> bool {
        #[cfg(all(debug_assertions, feature = "pin_release_ordering_test"))]
        super::pin_release_test::ordering_checkpoint(owner, waiter, task, false);
        let bit = 1u64 << waiter;
        self.0[owner].waiters.fetch_or(bit, Ordering::SeqCst);
        #[cfg(all(debug_assertions, feature = "pin_release_ordering_test"))]
        super::pin_release_test::ordering_checkpoint(owner, waiter, task, true);
        if self.0[owner].pin.load(Ordering::SeqCst) == task {
            true
        } else {
            self.0[owner].waiters.fetch_and(!bit, Ordering::Relaxed);
            false
        }
    }

    /// Take the waiters registered on `cpu`'s slot (after the pin release).
    pub fn take_waiters(&self, cpu: usize) -> u64 {
        self.0[cpu].waiters.swap(0, Ordering::SeqCst)
    }

    #[cfg(all(debug_assertions, feature = "pin_release_test"))]
    pub fn waiter_snapshot(&self, cpu: usize) -> u64 {
        self.0[cpu].waiters.load(Ordering::SeqCst)
    }

    /// The x86 epilogue releases EMPTY with one aligned `xchg` (full barrier).
    pub fn slot(&self, cpu: usize) -> &AtomicUsize {
        &self.0[cpu].pin
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::sync::{Arc, Barrier};

    #[test]
    fn waiter_registered_while_pinned_is_drained_by_release() {
        let h = Handoffs::<3>::new();
        h.pin(0, 7);
        assert!(h.wait_release(0, 1, 7));
        assert!(h.wait_release(0, 2, 7));
        h.slot(0).swap(EMPTY, Ordering::SeqCst);
        assert_eq!(h.take_waiters(0), 0b110);
        assert_eq!(h.take_waiters(0), 0);
    }

    #[test]
    fn waiter_after_release_proceeds_and_leaves_no_registration() {
        let h = Handoffs::<2>::new();
        h.pin(0, 7);
        h.slot(0).swap(EMPTY, Ordering::SeqCst);
        assert!(!h.wait_release(0, 1, 7));
        assert_eq!(h.take_waiters(0), 0);
        // A different task now pinned on the slot does not hold task 7.
        h.pin(0, 9);
        assert!(!h.wait_release(0, 1, 7));
        assert_eq!(h.take_waiters(0), 0);
    }

    #[test]
    fn no_waiters_means_no_release_work() {
        let h = Handoffs::<2>::new();
        h.pin(1, 3);
        h.slot(1).swap(EMPTY, Ordering::SeqCst);
        assert_eq!(h.take_waiters(1), 0);
    }

    #[test]
    fn racing_register_and_release_never_loses_the_waiter() {
        for _ in 0..20000 {
            let h = Arc::new(Handoffs::<2>::new());
            h.pin(0, 5);
            let start = Arc::new(Barrier::new(2));
            let waiter = {
                let (h, start) = (h.clone(), start.clone());
                std::thread::spawn(move || {
                    start.wait();
                    h.wait_release(0, 1, 5)
                })
            };
            start.wait();
            h.slot(0).swap(EMPTY, Ordering::SeqCst);
            let drained = h.take_waiters(0) & 0b10 != 0;
            let still_pinned_seen = waiter.join().unwrap();
            // Either the waiter saw the release (and proceeds itself) or the
            // releaser drained it. Never both absent.
            assert!(!still_pinned_seen || drained, "stranded waiter");
        }
    }

    #[test]
    fn cpu_release_does_not_release_a_remote_stack_or_reused_index() {
        let h = Handoffs::<2>::new();
        h.pin(0, 7);
        h.pin(1, 8);
        h.slot(0).store(EMPTY, Ordering::Release);
        assert!(!h.is_pinned(0, 7));
        assert!(h.is_pinned(1, 8));
        h.pin(0, 9);
        assert!(!h.is_pinned(0, 7));
        assert!(h.is_pinned(0, 9));
    }

    #[test]
    fn remote_reclaim_and_selection_wait_for_physical_release() {
        let h = Arc::new(Handoffs::<2>::new());
        let entered = Arc::new(Barrier::new(2));
        let checked = Arc::new(Barrier::new(2));
        let remote = {
            let (h, entered, checked) = (h.clone(), entered.clone(), checked.clone());
            std::thread::spawn(move || {
                entered.wait();
                // Metadata may already be detached/Finished: the pin wins.
                assert!(h.is_pinned(0, 41));
                checked.wait();
            })
        };
        h.pin(0, 41);
        entered.wait();
        checked.wait();
        remote.join().unwrap();
        h.slot(0).store(EMPTY, Ordering::Release); // model the post-pivot store
        assert!(!h.is_pinned(0, 41));
    }

    #[test]
    #[should_panic(expected = "nested outgoing stack handoff")]
    fn cannot_overwrite_an_unreleased_context() {
        let h = Handoffs::<1>::new();
        h.pin(0, 3);
        h.pin(0, 4);
    }
}
