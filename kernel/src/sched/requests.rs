//! Coalesced, level-triggered scheduling requests. A consumer clears only its
//! own bit at dispatch entry; requests arriving after that remain pending.
use core::sync::atomic::{AtomicU64, Ordering};

pub struct RescheduleRequests(AtomicU64);

impl RescheduleRequests {
    pub const fn new() -> Self {
        Self(AtomicU64::new(0))
    }
    pub fn request(&self, cpu: usize) {
        self.0.fetch_or(1u64 << cpu, Ordering::Release);
    }
    pub fn pending(&self, cpu: usize) -> bool {
        self.0.load(Ordering::Acquire) & (1u64 << cpu) != 0
    }
    pub fn take(&self, cpu: usize) -> bool {
        let bit = 1u64 << cpu;
        self.0.fetch_and(!bit, Ordering::AcqRel) & bit != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_requests_coalesce_without_consuming_another_cpu() {
        let r = RescheduleRequests::new();
        r.request(0);
        r.request(0);
        r.request(11);
        assert!(r.take(0));
        assert!(!r.take(0));
        assert!(r.pending(11));
        assert!(r.take(11));
        assert!(!r.pending(11));
    }

    #[test]
    fn new_request_after_dispatch_entry_survives() {
        let r = RescheduleRequests::new();
        r.request(1);
        assert!(r.take(1));
        // Internal requeue makes no request; a concurrent producer does.
        assert!(!r.pending(1));
        r.request(1);
        assert!(r.pending(1));
        assert!(r.take(1));
        assert!(!r.take(1));
    }
}
