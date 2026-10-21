//! Coalesced scheduling intent and remote delivery ownership. The target clears
//! both at dispatch entry in one RMW; requests arriving after remain pending.
use core::sync::atomic::{AtomicU8, Ordering};

const REQUESTED: u8 = 1;
const DELIVERY_OWNED: u8 = 2;

pub struct RescheduleRequests([AtomicU8; 64]);

impl RescheduleRequests {
    pub const fn new() -> Self {
        Self([const { AtomicU8::new(0) }; 64])
    }
    pub fn request(&self, cpu: usize) {
        self.0[cpu].fetch_or(REQUESTED, Ordering::Release);
    }
    /// Claim delivery until the target reaches dispatch entry. A local-only
    /// request does not own delivery and must not suppress a later remote kick.
    /// Explicit local handoffs claim this same ownership before publishing a
    /// self-IPI, so a remote producer can share an already-pending local kick.
    /// The caller winning this RMW must publish its dispatch notification.
    pub fn request_delivery(&self, cpu: usize) -> bool {
        self.0[cpu].fetch_or(REQUESTED | DELIVERY_OWNED, Ordering::AcqRel) & DELIVERY_OWNED == 0
    }
    pub fn pending(&self, cpu: usize) -> bool {
        self.0[cpu].load(Ordering::Acquire) & REQUESTED != 0
    }
    pub fn take(&self, cpu: usize) -> bool {
        // One linearization point releases delivery ownership and consumes
        // intent. A publication after this exchange owns a fresh delivery.
        self.0[cpu].swap(0, Ordering::AcqRel) & REQUESTED != 0
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

    #[test]
    fn local_intent_does_not_own_a_remote_delivery() {
        let r = RescheduleRequests::new();
        r.request(1);
        assert!(r.request_delivery(1));
        assert!(!r.request_delivery(1));
        assert!(r.take(1));
        assert!(r.request_delivery(1));
    }

    #[test]
    fn every_supported_target_has_independent_bounded_state() {
        let r = RescheduleRequests::new();
        for cpu in 0..64 {
            assert!(r.request_delivery(cpu));
            assert!(!r.request_delivery(cpu));
        }
        for cpu in 0..64 {
            assert!(r.take(cpu));
            assert!(!r.take(cpu));
        }
    }

    #[test]
    fn two_sources_share_delivery_and_post_ack_publication_survives() {
        use std::sync::{Arc, Barrier};
        let r = Arc::new(RescheduleRequests::new());
        let barrier = Arc::new(Barrier::new(2));
        let producer_r = r.clone();
        let producer_barrier = barrier.clone();
        let producer = std::thread::spawn(move || {
            assert!(producer_r.request_delivery(1));
            producer_barrier.wait();
            producer_barrier.wait();
            assert!(producer_r.request_delivery(1));
        });
        barrier.wait();
        assert!(!r.request_delivery(1));
        assert!(r.take(1));
        barrier.wait();
        producer.join().unwrap();
        assert!(r.pending(1));
        assert!(r.take(1));
    }
}
