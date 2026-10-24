//! Bounded boundary exercise of the real broker, shared by host and opt-in
//! native verification. The isolated object never replaces CAP_BROKER and never
//! publishes its deterministic candidates to userspace. No normal-build hook.
use super::{token_history::MAX_PREPARED_TOKENS, CapError, CapabilityBroker, CapabilityRights};

pub(super) fn run() {
    let mut broker = CapabilityBroker::new();
    broker.test_entropy(&[Some(41), Some(43)]);
    let (_, oldest) = broker.create_endpoint(1).unwrap();
    broker.destroy_endpoint(1, oldest).unwrap();
    let (live_endpoint, live) = broker.create_endpoint(2).unwrap();
    let mut next = 100u64;
    // Real dropped preparations consume the same finite identity budget as
    // committed grants. Batch entropy storage is fixed at 256 candidates.
    while broker.token_history.usage().0 < MAX_PREPARED_TOKENS - 1 {
        let count = (MAX_PREPARED_TOKENS - 1 - broker.token_history.usage().0).min(256);
        let mut candidates = [None; 256];
        for candidate in &mut candidates[..count] {
            *candidate = Some(next);
            next += 1;
        }
        broker.test_entropy(&candidates[..count]);
        for _ in 0..count {
            drop(broker.prepare_shared_region().unwrap());
        }
    }
    assert_eq!(broker.active_tokens(), 1);
    broker.test_entropy(&[Some(next), Some(next + 1)]);
    let (_, newest) = broker.create_endpoint(3).unwrap();
    broker.destroy_endpoint(3, newest).unwrap();
    assert_eq!(broker.token_history.usage().0, MAX_PREPARED_TOKENS);
    assert_eq!(broker.create_endpoint(4), Err(CapError::TokenHistoryFull));
    assert_eq!(broker.test_words.as_ref().unwrap().len(), 1);
    assert_eq!(
        broker.check(oldest, CapabilityRights::SEND),
        Err(CapError::InvalidToken)
    );
    assert_eq!(
        broker.check(newest, CapabilityRights::SEND),
        Err(CapError::InvalidToken)
    );
    assert_eq!(
        broker.check(live, CapabilityRights::SEND),
        Ok(live_endpoint)
    );
    broker.destroy_endpoint(2, live).unwrap();
    assert_eq!(broker.active_tokens(), 0);
    assert_eq!(broker.create_endpoint(5), Err(CapError::TokenHistoryFull));
    assert_eq!(broker.token_history.usage().0, MAX_PREPARED_TOKENS);
    #[cfg(all(feature = "cap_issuance_test", target_os = "none"))]
    crate::serial_println!("[HISTORY-E] boundary capacity={} history={} active=0 oldest=denied newest=denied extra=TokenHistoryFull existing=usable partial=0 entropy_after_full=unchanged restart=UNSUPPORTED scope=isolated-production-broker cpu={}",
        MAX_PREPARED_TOKENS, broker.token_history.usage().0, crate::sched::current_cpu_id());
}

#[cfg(test)]
mod tests {
    #[test]
    fn actual_broker_exact_history_boundary_preserves_live_authority() {
        super::run();
    }
}
