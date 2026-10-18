//! Physical outgoing-stack ownership. Slots never move with the process Vec.
//! Pin before publishing an outgoing task under SCHEDULER, release in assembly
//! only after the last access to its stack. Local IRQs must stay disabled.
use core::sync::atomic::{AtomicUsize, Ordering};

pub const EMPTY: usize = usize::MAX;

#[repr(align(64))]
struct Slot(AtomicUsize);

pub struct Handoffs<const N: usize>([Slot; N]);

impl<const N: usize> Handoffs<N> {
    pub const fn new() -> Self {
        Self([const { Slot(AtomicUsize::new(EMPTY)) }; N])
    }

    pub fn pin(&self, cpu: usize, task: usize) {
        assert_ne!(task, EMPTY);
        self.0[cpu]
            .0
            .compare_exchange(EMPTY, task, Ordering::Release, Ordering::Relaxed)
            .expect("nested outgoing stack handoff");
    }

    pub fn is_pinned(&self, cpu: usize, task: usize) -> bool {
        self.0[cpu].0.load(Ordering::Acquire) == task
    }

    /// The x86 epilogue writes EMPTY with one aligned store (Release on x86).
    pub fn slot(&self, cpu: usize) -> &AtomicUsize {
        &self.0[cpu].0
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::sync::{Arc, Barrier};

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
