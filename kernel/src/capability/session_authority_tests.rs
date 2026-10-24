//! Production refusal and bounded capacity tests. No passing session-revocation
//! model is substituted for the missing cross-service authority protocol.
use super::*;
use sunlight_ipc::accounts::Error;

fn burn_until(b: &mut CapabilityBroker, target: usize) {
    let mut word = 1000u64;
    while b.token_history.usage().0 < target {
        let n = (target - b.token_history.usage().0).min(256);
        let mut candidates = [None; 256];
        for c in &mut candidates[..n] {
            *c = Some(word);
            word += 1;
        }
        b.test_entropy(&candidates[..n]);
        for _ in 0..n {
            drop(b.prepare_shared_region().unwrap());
        }
    }
}

#[test]
fn production_account_refuses_all_session_tuples_without_authority() {
    let _serial = TEST_LOCK.lock().unwrap();
    let s = table();
    for session in [0, 1, u64::MAX] {
        for generation in [0, 1, u64::MAX] {
            let mut scope = scope();
            scope.session = session;
            scope.session_generation = generation;
            for consume in [None, Some(CapabilityToken(123))] {
                assert_eq!(
                    s.account_grant(7, scope, consume),
                    Err(Error::SessionAuthorityUnavailable)
                );
            }
        }
    }
    let b = CAP_BROKER.lock();
    assert_eq!(b.active_tokens(), 0);
    assert_eq!(b.token_history.usage().0, 0);
    assert_eq!(b.issuance_stats().prepared, 0);
}

#[test]
fn production_account_restart_stale_delivery_and_expiration_cannot_open_authority() {
    let _serial = TEST_LOCK.lock().unwrap();
    for transition in 0..9 {
        let mut s = table();
        // Seed historical storage only to test that even an old delivered grant
        // cannot be consumed through the current production entry point.
        let old = s
            .unchecked_account_grant_for_delivery_test(7, scope(), None)
            .unwrap();
        complete(&s, &[old], true);
        match transition {
            0 => s.processes[0].exiting = true,
            1 => s.processes[0].foreground.incarnation += 100,
            2 => s.processes[0].withdraw_exec_image_roles(),
            3 => s.processes[1].foreground.incarnation += 100,
            4 => s.processes[1].address_space.generation += 100,
            5 => s.global_tick = u64::MAX,
            6 => next_request(&mut s),
            7 => s.processes[1].pending_call = None,
            _ => {}
        }
        assert!(s.account_grant(7, scope(), None).is_err());
        assert!(s.account_grant(7, scope(), Some(old)).is_err());
        assert_eq!(CAP_BROKER.lock().token_history.usage().0, 1);
        // Refusal creates no grant; ordinary lifecycle retirement remains
        // responsible for cleaning the historical row and is idempotent.
        CAP_BROKER.lock().retire_recipient_grants(7);
        CAP_BROKER.lock().retire_recipient_grants(7);
        assert_eq!(inventory(), 0);
    }
}

#[test]
fn concurrent_account_requests_and_cancellation_publish_no_authority() {
    let _serial = TEST_LOCK.lock().unwrap();
    let scheduler = Arc::new(Mutex::new(table()));
    let start = Arc::new(Barrier::new(3));
    let workers: std::vec::Vec<_> = (0..2)
        .map(|_| {
            let scheduler = scheduler.clone();
            let start = start.clone();
            std::thread::spawn(move || {
                start.wait();
                let s = scheduler.lock().unwrap();
                assert!(s.account_grant(7, scope(), None).is_err());
            })
        })
        .collect();
    start.wait();
    cleanup(&mut scheduler.lock().unwrap(), 7);
    for worker in workers {
        worker.join().unwrap();
    }
    assert_eq!(inventory(), 0);
    assert_eq!(CAP_BROKER.lock().token_history.usage().0, 0);
}

#[test]
fn production_vfs_full_history_rolls_back_batch_and_preserves_existing_grant() {
    let _serial = TEST_LOCK.lock().unwrap();
    let mut s = table();
    let first_proof = proof(&mut s, 0);
    let existing = s
        .try_delegate_vfs_grants(first_proof, Some("/bin/true"))
        .unwrap()[0];
    complete(&s, &[existing], true);
    next_request(&mut s);
    let second_proof = proof(&mut s, 0);
    burn_until(
        &mut CAP_BROKER.lock(),
        token_history::MAX_PREPARED_TOKENS - 1,
    );
    CAP_BROKER
        .lock()
        .test_entropy(&[Some(900_001), Some(900_003)]);
    assert_eq!(
        s.try_delegate_vfs_grants(second_proof, None),
        Err(CapError::TokenHistoryFull)
    );
    assert_eq!(count(), 1); // failed batch contributes zero records
    assert_eq!(inventory(), 1); // delivered proof stays owned by its recipient
    assert_eq!(CAP_BROKER.lock().test_words.as_ref().unwrap().len(), 1);
    assert_eq!(
        s.try_delegate_vfs_grants(second_proof, Some("/bin/true")),
        Err(CapError::TokenHistoryFull)
    );
    vfs(&mut s);
    assert!(s.consume_vfs_grant(existing, "/bin/true"));
    assert!(!s.consume_vfs_grant(existing, "/bin/true"));
    cleanup(&mut s, 7);
    cleanup(&mut s, 7);
    assert_eq!(CAP_BROKER.lock().active_tokens(), 0);
    assert_eq!(
        CAP_BROKER.lock().token_history.usage().0,
        token_history::MAX_PREPARED_TOKENS
    );
}

#[test]
fn concurrent_last_history_slot_has_one_commit_and_one_explicit_failure() {
    let mut b = CapabilityBroker::new();
    burn_until(&mut b, token_history::MAX_PREPARED_TOKENS - 1);
    b.test_entropy(&[Some(900_001), Some(900_003)]);
    let b = Arc::new(Mutex::new(b));
    let start = Arc::new(Barrier::new(3));
    let workers: std::vec::Vec<_> = (1..=2)
        .map(|pid| {
            let b = b.clone();
            let start = start.clone();
            std::thread::spawn(move || {
                start.wait();
                b.lock().unwrap().create_endpoint(pid)
            })
        })
        .collect();
    start.wait();
    let results: std::vec::Vec<_> = workers.into_iter().map(|w| w.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|r| **r == Err(CapError::TokenHistoryFull))
            .count(),
        1
    );
    let mut b = b.lock().unwrap();
    assert_eq!(b.active_tokens(), 1);
    assert_eq!(b.endpoints.len(), 1);
    assert_eq!(b.next_endpoint_id, 1);
    b.revoke_endpoints_owned_by(1);
    b.revoke_endpoints_owned_by(2);
    assert_eq!(b.active_tokens(), 0);
    assert_eq!(
        b.token_history.usage().0,
        token_history::MAX_PREPARED_TOKENS
    );
}
