//! Bounded append-only receipts. No reset or slot reuse during a boot.
//! Lifecycle mutation writers hold SCHEDULER; pre-lock probe and post-drop
//! receipts use only atomics. Reservation is one atomic instruction, publication
//! is Release, and readers require Acquire publication. Causal edges also use
//! ordinary program order, the native scheduler guard, and probe acknowledgment.
use core::sync::atomic::{
    AtomicU64,
    Ordering::{AcqRel, Acquire, Relaxed, Release},
};

pub const FIELDS: usize = 14;
pub struct Receipts<const N: usize> {
    generation: AtomicU64,
    context: [AtomicU64; 8],
    next: AtomicU64,
    overflow: AtomicU64,
    published: [AtomicU64; N],
    data: [[AtomicU64; FIELDS]; N],
}
impl<const N: usize> Receipts<N> {
    pub const fn new() -> Self {
        Self {
            generation: AtomicU64::new(0),
            context: [const { AtomicU64::new(0) }; 8],
            next: AtomicU64::new(0),
            overflow: AtomicU64::new(0),
            published: [const { AtomicU64::new(0) }; N],
            data: [const { [const { AtomicU64::new(0) }; FIELDS] }; N],
        }
    }
    // context: test generation, operation, PID, incarnation, VM, session, epoch, channel.
    pub fn arm(&self, context: [u64; 8]) -> bool {
        if context.iter().any(|v| *v == 0) || context[0] == u64::MAX {
            return false;
        }
        if self
            .generation
            .compare_exchange(0, u64::MAX, AcqRel, Acquire)
            .is_err()
        {
            return false;
        }
        for (dst, value) in self.context.iter().zip(context) {
            dst.store(value, Relaxed);
        }
        self.generation.store(context[0], Release);
        true
    }
    pub fn context(&self) -> Option<[u64; 8]> {
        let generation = self.generation.load(Acquire);
        if generation == 0 || generation == u64::MAX {
            return None;
        }
        Some(core::array::from_fn(|i| self.context[i].load(Relaxed)))
    }
    pub fn append(&self, generation: u64, tail: [u64; 6]) -> bool {
        let Some(context) = self.context() else {
            return false;
        };
        if context[0] != generation {
            return false;
        }
        if self.next.load(Relaxed) >= N as u64 {
            self.overflow.store(1, Release);
            return false;
        }
        let index = self.next.fetch_add(1, Relaxed) as usize;
        if index >= N {
            self.overflow.store(1, Release);
            return false;
        }
        for (dst, value) in self.data[index].iter().zip(context.into_iter().chain(tail)) {
            dst.store(value, Relaxed);
        }
        self.published[index].store(index as u64 + 1, Release);
        true
    }
    pub fn read(&self, index: usize) -> Option<[u64; FIELDS]> {
        if index >= N || self.published[index].load(Acquire) != index as u64 + 1 {
            return None;
        }
        Some(core::array::from_fn(|i| self.data[index][i].load(Relaxed)))
    }
    pub fn overflowed(&self) -> bool {
        self.overflow.load(Acquire) != 0
    }
}

/// Reaped processes retain the old VM in the native reap receipt, while their
/// now-empty address-space object has a different identity. Incarnation must
/// still match; this never authorizes a successor with a reused PID.
pub fn same_owner(expected: [u64; 3], current: [u64; 3], reaped_vm: Option<u64>) -> bool {
    expected[0] == current[0]
        && expected[1] == current[1]
        && expected[2] == reaped_vm.unwrap_or(current[2])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reaped_owner_never_adopts_successor() {
        assert!(same_owner([40, 85, 98], [40, 85, 98], None));
        assert!(same_owner([40, 85, 98], [40, 85, 0], Some(98)));
        assert!(!same_owner([40, 85, 98], [40, 86, 0], Some(98)));
        assert!(!same_owner([40, 85, 98], [40, 85, 99], None));
        assert!(!same_owner([40, 85, 98], [40, 85, 0], Some(99)));
    }
    #[test]
    fn identity_and_stale_generation() {
        let r = Receipts::<2>::new();
        assert!(!r.append(1, [9; 6]));
        assert!(!r.arm([0; 8]));
        assert!(r.arm([1, 2, 3, 4, 5, 6, 7, 8]));
        assert!(!r.arm([2; 8]));
        assert!(!r.append(2, [9; 6]));
        assert!(r.append(1, [9, 10, 11, 12, 13, 14]));
        assert_eq!(
            r.read(0),
            Some([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14])
        );
        assert_eq!(r.read(1), None);
    }
    #[test]
    fn bounded_no_overwrite() {
        let r = Receipts::<1>::new();
        assert!(r.arm([1; 8]));
        assert!(r.append(1, [2; 6]));
        let first = r.read(0);
        for _ in 0..1000 {
            assert!(!r.append(1, [3; 6]));
        }
        assert_eq!(r.read(0), first);
        assert!(r.overflowed());
        assert_eq!(r.read(1), None);
    }
    #[test]
    fn release_acquire_publication() {
        use std::{sync::Arc, thread};
        let r = Arc::new(Receipts::<128>::new());
        assert!(r.arm([1, 2, 3, 4, 5, 6, 7, 8]));
        let writer = r.clone();
        let t = thread::spawn(move || {
            for i in 0..128 {
                assert!(writer.append(1, [i; 6]));
            }
        });
        for i in 0..128 {
            loop {
                if let Some(row) = r.read(i) {
                    assert_eq!(&row[..8], &[1, 2, 3, 4, 5, 6, 7, 8]);
                    assert_eq!(&row[8..], &[i as u64; 6]);
                    break;
                }
                thread::yield_now();
            }
        }
        t.join().unwrap();
        assert!(!r.overflowed());
    }
}
