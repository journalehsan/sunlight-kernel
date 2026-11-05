//! Runs the actual production grant methods with synthetic host identities.
//! Native service execution and remote lifecycle progress are separate gates.
//! Account cases explicitly use the host-only unchecked delivery helper; they
//! verify historical metadata mechanics, never current session authorization.
use super::*;

#[path = "session_authority_tests.rs"]
mod session_authority_tests;
#[path = "broker_session_tests.rs"]
mod broker_session_tests;

fn complete(s: &Scheduler, tokens: &[CapabilityToken], delivered: bool) {
    let target = s.current_process().ipc_reply_target.unwrap().call;
    CAP_BROKER.lock().settle_grant_delivery(
        target.pid,
        target.incarnation,
        target.generation,
        delivered.then(|| RecipientIdentity::of(&s.processes[1])),
        &tokens.iter().map(|t| t.0).collect::<std::vec::Vec<_>>(),
    );
}
fn next_request(s: &mut Scheduler) {
    let generation = s.processes[1].pending_call.unwrap().1 + 1;
    s.processes[1].pending_call = Some((7, generation));
    s.processes[0].ipc_reply_target =
        Some(reply(7, generation, s.processes[1].foreground.incarnation));
}
fn proof(s: &mut Scheduler, uid: u32) -> CapabilityToken {
    s.processes[0].trusted_auth_broker = true;
    s.processes[0].trusted_vfs_service = false;
    let token = s.issue_auth_session_grant(7, uid, uid).unwrap();
    complete(s, &[token], true);
    next_request(s);
    token
}
fn vfs(s: &mut Scheduler) {
    next_request(s);
    s.processes[0].trusted_auth_broker = false;
    s.processes[0].trusted_vfs_service = true;
}
fn count() -> usize {
    CAP_BROKER.lock().vfs_caps.len()
}

#[test]
fn production_root_proof_to_vfs_consumption_and_cleanup() {
    let _serial = TEST_LOCK.lock().unwrap();
    let mut s = table();
    // Trusted provenance, rather than legacy numeric slot, authorizes the new
    // operation. Existing syscall 100 retains its stricter historical policy.
    s.processes[0].pid = 42;
    let root = proof(&mut s, 0);
    let tokens = s.delegate_vfs_grants(root, None).unwrap();
    assert_eq!(tokens.len(), 2);
    assert_eq!(inventory(), 0); // password proof consumed atomically
    assert_eq!(count(), 2);
    for token in &tokens {
        assert!(CAP_BROKER.lock().resolve_vfs_capability(*token).is_none());
        assert!(!CAP_BROKER
            .lock()
            .vfs_allows(*token, "/bin/true", AccessFlags::ALL));
    }
    complete(&s, &tokens, true);
    vfs(&mut s);
    assert!(s.consume_vfs_grant(tokens[0], "/bin/true"));
    assert!(!s.consume_vfs_grant(tokens[0], "/bin/true"));
    assert!(s.consume_vfs_grant(tokens[1], "/usr/bin/true"));
    assert_eq!(count(), 0);
    cleanup(&mut s, 7);
    assert_eq!(CAP_BROKER.lock().recipient_grant_count(7), 0);
}

#[test]
fn production_rejects_forged_nonroot_pending_expired_and_withdrawn_proofs() {
    let _serial = TEST_LOCK.lock().unwrap();
    for fault in 0..9 {
        let mut s = table();
        let token = proof(&mut s, if fault == 1 { 1000 } else { 0 });
        match fault {
            0 => {
                assert!(s.delegate_vfs_grants(CapabilityToken(123), None).is_none());
                continue;
            }
            2 => {
                CAP_BROKER.lock().auth_session_grants[0]
                    .delivery
                    .as_mut()
                    .unwrap()
                    .delivered = false
            }
            3 => s.global_tick = 502,
            4 => s.processes[0].withdraw_exec_image_roles(),
            5 => s.processes[0].exiting = true,
            6 => s.processes[1].foreground.incarnation += 1,
            7 => s.processes[1].address_space.generation += 1,
            8 => s.processes[1].exiting = true,
            _ => {}
        }
        assert!(s.delegate_vfs_grants(token, None).is_none());
        assert_eq!(count(), 0);
    }
}

#[test]
fn production_vfs_consumer_requires_live_role_exact_request_and_identity() {
    let _serial = TEST_LOCK.lock().unwrap();
    for fault in 0..8 {
        let mut s = table();
        let root = proof(&mut s, 0);
        let token = s.delegate_vfs_grants(root, Some("/bin/true")).unwrap()[0];
        complete(&s, &[token], true);
        vfs(&mut s);
        match fault {
            0 => s.processes[0].trusted_vfs_service = false,
            1 => s.processes[0].withdraw_exec_image_roles(),
            2 => s.processes[0].exiting = true,
            3 => s.processes[1].pending_call = None,
            4 => s.processes[1].foreground.incarnation += 1,
            5 => s.processes[1].address_space.generation += 1,
            6 => s.processes[1].exiting = true,
            _ => s.global_tick = 502,
        }
        assert!(!s.consume_vfs_grant(token, "/bin/true"));
        CAP_BROKER.lock().retire_recipient_grants(7);
        assert_eq!(count(), 0);
    }
}

#[test]
fn production_exit_exec_and_pid_reuse_before_delivery() {
    let _serial = TEST_LOCK.lock().unwrap();
    for transition in 0..4 {
        let mut s = table();
        let root = proof(&mut s, 0);
        let tokens = s.delegate_vfs_grants(root, None).unwrap();
        match transition {
            0 => cleanup(&mut s, 7),
            1 => {
                CAP_BROKER.lock().retire_recipient_grants(7);
                s.processes[1].address_space.generation += 1;
            }
            2 => {
                cleanup(&mut s, 7);
                s.processes[1] = process(7);
                s.processes[1].foreground.incarnation += 100;
            }
            _ => {
                CAP_BROKER.lock().retire_recipient_grants(6);
                s.processes[0].withdraw_exec_image_roles();
            }
        }
        complete(&s, &tokens, true);
        CAP_BROKER.lock().retire_recipient_grants(7);
        complete(&s, &tokens, true);
        assert_eq!(count(), 0);
        for token in tokens {
            assert!(!CAP_BROKER.lock().consume_vfs_grant(
                token,
                delivered_identity(&s),
                "/bin/true",
                1
            ));
        }
    }
}

#[test]
fn production_failed_omitted_partial_and_ambiguous_delivery() {
    let _serial = TEST_LOCK.lock().unwrap();
    for mode in 0..4 {
        let mut s = table();
        let root = proof(&mut s, 0);
        let tokens = s.delegate_vfs_grants(root, None).unwrap();
        // Early token observation alone is not eligible authority.
        assert!(!CAP_BROKER.lock().consume_vfs_grant(
            tokens[0],
            delivered_identity(&s),
            "/bin/true",
            1
        ));
        match mode {
            0 => complete(&s, &[], false),
            1 => complete(&s, &[], true),
            2 => complete(&s, &tokens[..1], true),
            _ => complete(&s, &tokens, true),
        }
        // A reported failure after publication may not undo delivered authority
        // or recycle a potentially observed token. Only owner cleanup does so.
        complete(&s, &[], false);
        complete(&s, &tokens, true);
        assert_eq!(count(), if mode < 2 { 0 } else { mode - 1 });
        cleanup(&mut s, 7);
        cleanup(&mut s, 7);
        assert_eq!(count(), 0);
    }
}

#[test]
fn production_batch_failure_preserves_proof_and_publishes_nothing() {
    let _serial = TEST_LOCK.lock().unwrap();
    for failure in 0..4 {
        let mut s = table();
        let root = proof(&mut s, 0);
        match failure {
            0 => CAP_BROKER.lock().test_fail_reservation(true),
            1 => CAP_BROKER.lock().test_entropy(&[None]),
            2 => CAP_BROKER.lock().test_entropy(&[Some(111), None]),
            _ => CAP_BROKER.lock().test_entropy(&[Some(111); 17]),
        }
        assert!(s.delegate_vfs_grants(root, None).is_none());
        assert_eq!(count(), 0);
        assert_eq!(inventory(), 1);
        cleanup(&mut s, 7);
        assert_eq!(inventory(), 0);
    }
}

#[test]
fn production_exact_path_not_prefix_and_no_legacy_adoption() {
    let _serial = TEST_LOCK.lock().unwrap();
    let mut s = table();
    let root = proof(&mut s, 0);
    assert!(s
        .delegate_vfs_grants(root, Some("/bin/../etc/shadow"))
        .is_none());
    let token = s.delegate_vfs_grants(root, Some("/bin/true")).unwrap()[0];
    complete(&s, &[token], true);
    vfs(&mut s);
    for path in [
        "/bin/true/sub",
        "/bin/truex",
        "/binary/true",
        "/bin/../etc/shadow",
    ] {
        assert!(!s.consume_vfs_grant(token, path));
    }
    assert!(s.consume_vfs_grant(token, "/bin/true"));
    let legacy = CAP_BROKER
        .lock()
        .grant_vfs_capability(7, vfs_cap())
        .unwrap();
    assert!(!s.consume_vfs_grant(legacy, "/bin/true"));
    cleanup(&mut s, 7);
    assert!(CAP_BROKER.lock().resolve_vfs_capability(legacy).is_some());
}

#[test]
fn production_service_restart_preserves_delivered_recipient_ownership() {
    let _serial = TEST_LOCK.lock().unwrap();
    let mut s = table();
    let root = proof(&mut s, 0);
    let delivered = s.delegate_vfs_grants(root, None).unwrap();
    complete(&s, &delivered, true);
    let root = proof(&mut s, 0);
    let pending = s.delegate_vfs_grants(root, None).unwrap();
    cleanup(&mut s, 6);
    assert_eq!(count(), 2);
    // Numeric service slot reuse installs no trusted image role by itself.
    s.processes[0] = process(6);
    s.processes[0].trusted_auth_broker = false;
    s.processes[0].foreground.incarnation += 100;
    assert!(s.delegate_vfs_grants(pending[0], None).is_none());
    vfs(&mut s);
    assert!(s.consume_vfs_grant(delivered[0], "/bin/true"));
    assert!(!s.consume_vfs_grant(pending[0], "/bin/true"));
    cleanup(&mut s, 7);
    assert_eq!(count(), 0);
}

#[test]
fn production_bounded_cycles_capacity_and_metadata_inventory() {
    let _serial = TEST_LOCK.lock().unwrap();
    let mut s = table();
    let mut capacities = None;
    for cycle in 0..128 {
        let root = proof(&mut s, 0);
        let tokens = s.delegate_vfs_grants(root, None).unwrap();
        complete(&s, &tokens, true);
        vfs(&mut s);
        if cycle % 2 == 0 {
            assert!(s.consume_vfs_grant(tokens[0], "/bin/true"));
        }
        for token in tokens {
            CAP_BROKER
                .lock()
                .cancel_vfs_grant(token, delivered_identity(&s));
            CAP_BROKER
                .lock()
                .cancel_vfs_grant(token, delivered_identity(&s));
        }
        assert_eq!(count(), 0);
        assert_eq!(inventory(), 0);
        let b = CAP_BROKER.lock();
        let current = (b.vfs_caps.capacity(), b.auth_session_grants.capacity());
        assert_eq!(*capacities.get_or_insert(current), current);
    }
    for _ in 0..32 {
        let root = proof(&mut s, 0);
        let tokens = s.delegate_vfs_grants(root, None).unwrap();
        complete(&s, &tokens, true);
    }
    assert_eq!(count(), 64);
    let root = proof(&mut s, 0);
    assert!(s.delegate_vfs_grants(root, None).is_none());
    assert_eq!(count(), 64);
    cleanup(&mut s, 7);
    assert_eq!(CAP_BROKER.lock().recipient_grant_count(7), 0);
}

#[test]
fn production_retirement_vs_consumption_is_single_terminal_operation() {
    let _serial = TEST_LOCK.lock().unwrap();
    for _ in 0..32 {
        let mut s = table();
        let root = proof(&mut s, 0);
        let token = s.delegate_vfs_grants(root, Some("/bin/true")).unwrap()[0];
        complete(&s, &[token], true);
        vfs(&mut s);
        let scheduler = Arc::new(Mutex::new(s));
        let start = Arc::new(Barrier::new(3));
        let consumer = {
            let scheduler = scheduler.clone();
            let start = start.clone();
            std::thread::spawn(move || {
                start.wait();
                scheduler
                    .lock()
                    .unwrap()
                    .consume_vfs_grant(token, "/bin/true")
            })
        };
        let retire = {
            let scheduler = scheduler.clone();
            let start = start.clone();
            std::thread::spawn(move || {
                start.wait();
                cleanup(&mut scheduler.lock().unwrap(), 7);
            })
        };
        start.wait();
        let _consumed = consumer.join().unwrap();
        retire.join().unwrap();
        assert_eq!(count(), 0);
        assert!(!scheduler
            .lock()
            .unwrap()
            .consume_vfs_grant(token, "/bin/true"));
    }
}

#[test]
fn production_commit_holds_lifecycle_serialization_through_proof_exchange() {
    let _serial = TEST_LOCK.lock().unwrap();
    let mut s = table();
    let root = proof(&mut s, 0);
    let scheduler = Arc::new(Mutex::new(s));
    let validated = Arc::new(Barrier::new(2));
    let resume = Arc::new(Barrier::new(2));
    let worker = {
        let scheduler = scheduler.clone();
        let validated = validated.clone();
        let resume = resume.clone();
        std::thread::spawn(move || {
            HOOK.with(|h| {
                *h.borrow_mut() = Some(Box::new(move || {
                    validated.wait();
                    resume.wait();
                }))
            });
            scheduler
                .lock()
                .unwrap()
                .delegate_vfs_grants(root, None)
                .unwrap()
        })
    };
    validated.wait();
    assert!(matches!(
        scheduler.try_lock(),
        Err(std::sync::TryLockError::WouldBlock)
    ));
    assert_eq!(count(), 0);
    assert_eq!(inventory(), 1);
    resume.wait();
    let tokens = worker.join().unwrap();
    let mut s = scheduler.lock().unwrap();
    cleanup(&mut s, 6);
    complete(&s, &tokens, true);
    assert_eq!(count(), 0);
}

#[test]
fn production_invalidation_completes_before_issuance_thread_commits() {
    let _serial = TEST_LOCK.lock().unwrap();
    for transition in 0..4 {
        let mut s = table();
        let root = proof(&mut s, 0);
        let scheduler = Arc::new(Mutex::new(s));
        let invalidated = Arc::new(Barrier::new(2));
        let worker = {
            let scheduler = scheduler.clone();
            let invalidated = invalidated.clone();
            std::thread::spawn(move || {
                invalidated.wait();
                scheduler.lock().unwrap().delegate_vfs_grants(root, None)
            })
        };
        {
            let mut s = scheduler.lock().unwrap();
            match transition {
                0 => cleanup(&mut s, 6),
                1 => s.processes[0].withdraw_exec_image_roles(),
                2 => cleanup(&mut s, 7),
                _ => {
                    CAP_BROKER.lock().retire_recipient_grants(7);
                    s.processes[1].address_space.generation += 1;
                }
            }
        }
        invalidated.wait();
        assert!(worker.join().unwrap().is_none());
        assert_eq!(count(), 0);
        cleanup(&mut scheduler.lock().unwrap(), 7);
        assert_eq!(inventory(), 0);
    }
}

#[test]
fn production_two_issuers_share_one_recipient_vm_without_adopting_ownership() {
    let _serial = TEST_LOCK.lock().unwrap();
    let mut s = table();
    let mut second = process(42);
    second.trusted_auth_broker = true;
    second.ipc_reply_target = s.processes[0].ipc_reply_target;
    s.processes.push(second);
    let scheduler = Arc::new(Mutex::new(s));
    let start = Arc::new(Barrier::new(3));
    let workers: std::vec::Vec<_> = (0..2)
        .map(|issuer| {
            let scheduler = scheduler.clone();
            let start = start.clone();
            std::thread::spawn(move || {
                start.wait();
                let mut s = scheduler.lock().unwrap();
                s.processes[0].ipc_reply_target = Some(reply(
                    7,
                    s.processes[1].pending_call.unwrap().1,
                    s.processes[1].foreground.incarnation,
                ));
                if issuer == 1 {
                    s.processes[3].ipc_reply_target = s.processes[0].ipc_reply_target;
                    s.processes.swap(0, 3);
                }
                let root = proof(&mut s, 0);
                let tokens = s.delegate_vfs_grants(root, None).unwrap();
                complete(&s, &tokens, true);
                if issuer == 1 {
                    s.processes.swap(0, 3);
                }
                tokens
            })
        })
        .collect();
    start.wait();
    let batches: std::vec::Vec<_> = workers.into_iter().map(|w| w.join().unwrap()).collect();
    assert_eq!(count(), 4);
    let mut s = scheduler.lock().unwrap();
    cleanup(&mut s, 42);
    assert_eq!(count(), 4); // already delivered authority belongs to recipient
    vfs(&mut s);
    for tokens in batches {
        assert!(s.consume_vfs_grant(tokens[0], "/bin/true"));
        assert!(s.consume_vfs_grant(tokens[1], "/usr/bin/true"));
    }
    assert_eq!(count(), 0);
    cleanup(&mut s, 7);
    assert_eq!(inventory(), 0);
}

#[test]
fn production_retired_bearer_replay_after_controlled_vfs_slot_reuse() {
    let _serial = TEST_LOCK.lock().unwrap();
    // Exercise real production broker/lifecycle methods. Identity changes here
    // are host-only; they are explicitly not native PID or service restart proof.
    for transition in 0..10 {
        let mut s = table();
        let root = proof(&mut s, 0);
        CAP_BROKER.lock().test_entropy(&[Some(41)]);
        let old = s.delegate_vfs_grants(root, Some("/bin/true")).unwrap()[0];
        let capacity = CAP_BROKER.lock().vfs_caps.capacity();
        if transition != 5 {
            complete(&s, &[old], true);
        }
        match transition {
            0 => {
                vfs(&mut s);
                assert!(s.consume_vfs_grant(old, "/bin/true"));
            }
            1 => CAP_BROKER
                .lock()
                .cancel_vfs_grant(old, delivered_identity(&s)),
            2 | 6 | 7 | 9 => CAP_BROKER.lock().retire_recipient_grants(7),
            3 => cleanup(&mut s, 7),
            4 => {
                CAP_BROKER.lock().retire_recipient_grants(7);
                s.processes[1].address_space.generation += 1;
            }
            5 => cleanup(&mut s, 6), // issuer-owned pending delivery
            8 => {
                s.global_tick = 502;
                assert!(!CAP_BROKER.lock().consume_vfs_grant(
                    old,
                    delivered_identity(&s),
                    "/bin/true",
                    s.global_tick
                ));
                // Expiry is lazy; a following issuance prunes the expired row.
            }
            _ => unreachable!(),
        }
        if transition == 6 {
            s.processes[1] = process(7);
            s.processes[1].foreground.incarnation += 100;
        }
        if transition == 7 {
            s.processes[1].address_space.generation += 100;
        }
        if transition == 9 {
            // Trusted role is installed explicitly in this host shim; actual
            // restart provenance must be tested under SunlightD separately.
            s.processes[0] = process(6);
            s.processes[0].foreground.incarnation += 100;
        }
        s.processes[0].exiting = false;
        s.processes[1].exiting = false;
        next_request(&mut s);
        let recipient = s.processes[1].foreground.incarnation;
        let request = s.processes[1].pending_call.unwrap().1;
        s.processes[0].ipc_reply_target = Some(reply(7, request, recipient));
        CAP_BROKER.lock().test_entropy(&[Some(101)]);
        let root = proof(&mut s, 0);
        // The old raw candidate must be rejected even for a different scope.
        CAP_BROKER.lock().test_entropy(&[Some(41), Some(43)]);
        let replacement = s.delegate_vfs_grants(root, Some("/bin/new")).unwrap()[0];
        assert!(old != replacement);
        assert_eq!(count(), 1);
        assert_eq!(CAP_BROKER.lock().vfs_caps.capacity(), capacity);
        complete(&s, &[replacement], true);
        vfs(&mut s);
        assert!(!s.consume_vfs_grant(old, "/bin/new"));
        assert!(!s.consume_vfs_grant(old, "/bin/true"));
        // Stale cancel and late delivery cannot retire/adopt the replacement.
        s.cancel_vfs_grant(old);
        assert_eq!(count(), 1);
        assert!(s.consume_vfs_grant(replacement, "/bin/new"));
        assert!(!s.consume_vfs_grant(replacement, "/bin/new"));
        cleanup(&mut s, 7);
        cleanup(&mut s, 7);
        assert_eq!(count(), 0);
        assert_eq!(inventory(), 0);
    }
}

#[test]
fn production_failed_mint_retry_cannot_reissue_burned_batch_identity() {
    let _serial = TEST_LOCK.lock().unwrap();
    let mut s = table();
    let root = proof(&mut s, 0);
    CAP_BROKER.lock().test_entropy(&[Some(41), None]);
    assert!(s.delegate_vfs_grants(root, None).is_none());
    assert_eq!(count(), 0);
    assert_eq!(inventory(), 1);
    CAP_BROKER
        .lock()
        .test_entropy(&[Some(41), Some(43), Some(45)]);
    let tokens = s.delegate_vfs_grants(root, None).unwrap();
    assert!(tokens
        .iter()
        .all(|t| *t != CapabilityToken::tagged(41, CapabilityToken::TAG_VFS)));
    complete(&s, &tokens, true);
    cleanup(&mut s, 7);
    assert_eq!(count(), 0);
    assert_eq!(inventory(), 0);
}

#[test]
fn production_auth_account_retirement_cannot_reassign_changed_scope() {
    let _serial = TEST_LOCK.lock().unwrap();
    for account in [false, true] {
        for transition in 0..5 {
            let mut s = table();
            CAP_BROKER.lock().test_entropy(&[Some(41)]);
            let old = issue(&s, account).unwrap();
            if transition != 4 {
                complete(&s, &[old], true);
            }
            match transition {
                0 => {
                    if account {
                        assert!(s
                            .unchecked_account_grant_for_delivery_test(7, scope(), Some(old))
                            .is_some());
                    } else {
                        assert!(CAP_BROKER
                            .lock()
                            .consume_live_auth_grant(old, delivered_identity(&s), 1)
                            .is_some());
                    }
                }
                1 => complete(&s, &[], false),
                2 => cleanup(&mut s, 7),
                3 => {
                    CAP_BROKER.lock().retire_recipient_grants(7);
                    s.processes[1].address_space.generation += 100;
                }
                4 => cleanup(&mut s, 6),
                _ => unreachable!(),
            }
            // Failed delivery cannot revoke already delivered ownership.
            if transition == 1 {
                CAP_BROKER.lock().retire_recipient_grants(7);
            }
            assert_eq!(inventory(), 0);
            s.processes[0].exiting = false;
            s.processes[1].exiting = false;
            next_request(&mut s);
            CAP_BROKER.lock().test_entropy(&[Some(41), Some(43)]);
            let mut changed = scope();
            changed.session_generation += 1;
            changed.target = 1001;
            let new = if account {
                s.unchecked_account_grant_for_delivery_test(7, changed, None)
                    .unwrap()
            } else {
                s.issue_auth_session_grant(7, 1001, 1001).unwrap()
            };
            assert!(old != new);
            complete(&s, &[new], true);
            if account {
                assert!(s
                    .unchecked_account_grant_for_delivery_test(7, changed, Some(old))
                    .is_none());
                assert!(s
                    .unchecked_account_grant_for_delivery_test(7, changed, Some(new))
                    .is_some());
                assert!(s
                    .unchecked_account_grant_for_delivery_test(7, changed, Some(new))
                    .is_none());
            } else {
                let mut b = CAP_BROKER.lock();
                assert!(b
                    .consume_live_auth_grant(old, delivered_identity(&s), 1)
                    .is_none());
                assert_eq!(
                    b.consume_live_auth_grant(new, delivered_identity(&s), 1),
                    Some((1001, 1001))
                );
                assert!(b
                    .consume_live_auth_grant(new, delivered_identity(&s), 1)
                    .is_none());
            }
            cleanup(&mut s, 7);
            cleanup(&mut s, 7);
            assert_eq!(inventory(), 0);
            assert_eq!(CAP_BROKER.lock().token_history.usage().0, 2);
        }
    }
}
