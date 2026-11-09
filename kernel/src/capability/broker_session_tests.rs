//! Actual broker/Scheduler primitives; synthetic host identities, not native
//! service authentication or SMP evidence.
use super::*;
use sunlight_ipc::account_session::{SessionRef, ACTIVATE, REVOKE, SUSPEND};

fn registered(deadline: u64, activate: bool) -> (Scheduler, GrantScope, RecipientIdentity) {
    let mut s = table();
    s.processes[1].trusted_tty_session_service = true;
    s.processes[2].trusted_session_service = true;
    let token = proof(&mut s, 0);
    s.processes.swap(0, 2);
    s.processes[0].ipc_reply_target = Some(reply(7, 2, 7));
    let registered = s.register_account_session(token, 7, deadline).unwrap();
    // Retrying registration in the same synchronous call returns its result
    // despite the consumed proof, and allocates no replacement identity.
    assert_eq!(
        s.register_account_session(token, 7, deadline),
        Ok(registered)
    );
    let reference = registered.0;
    if activate { s.transition_account_session(reference, ACTIVATE).unwrap(); }
    let owner = RecipientIdentity::of(&s.processes[0]);
    s.processes.swap(0, 2);
    next_request(&mut s);
    let mut scope = scope();
    scope.session = reference.id;
    scope.session_generation = reference.epoch;
    (s, scope, owner)
}

fn active(deadline: u64) -> (Scheduler, GrantScope, RecipientIdentity) {
    registered(deadline, true)
}

fn reference(scope: GrantScope) -> SessionRef {
    SessionRef {
        id: scope.session,
        epoch: scope.session_generation,
    }
}

#[test]
fn failed_delivery_retires_only_its_exact_receipt_and_preserves_committed_reply() {
    let _serial = TEST_LOCK.lock().unwrap();
    for delivered in [false, true] {
        let (s, scope, _) = active(100);
        let token = s.account_grant(7, scope, None).unwrap();
        let delivery = s.grant_delivery_for_test();
        let mut broker = CAP_BROKER.lock();
        for (pid, incarnation, generation) in [
            (8, delivery.recipient.incarnation, delivery.request),
            (7, delivery.recipient.incarnation + 1, delivery.request),
            (7, delivery.recipient.incarnation, delivery.request + 1),
        ] {
            broker.settle_grant_delivery(pid, incarnation, generation, None, &[]);
            assert_eq!(broker.session_inventory(), (1, 1, 1));
        }
        broker.settle_grant_delivery(
            7, delivery.recipient.incarnation, delivery.request,
            delivered.then_some(delivery.recipient), &[token.0],
        );
        assert_eq!(broker.session_inventory(), if delivered { (1, 1, 1) } else { (1, 0, 0) });
        // A late/duplicate failure cannot revoke an already delivered grant.
        broker.settle_grant_delivery(7, delivery.recipient.incarnation, delivery.request, None, &[]);
        assert_eq!(broker.session_inventory(), if delivered { (1, 1, 1) } else { (1, 0, 0) });
    }
}

#[test]
fn s1_s3_s8_normal_commit_retry_consumption_and_revocation_policy() {
    let _serial = TEST_LOCK.lock().unwrap();
    let (mut s, scope, owner) = active(100);
    let token = s.account_grant(7, scope, None).unwrap();
    assert_eq!(s.account_grant(7, scope, None), Ok(token));
    assert_eq!(CAP_BROKER.lock().session_inventory(), (1, 1, 1));
    assert_eq!(s.account_grant(7, scope, Some(token)), Ok(token));
    assert_eq!(
        s.account_grant(7, scope, None),
        Err(sunlight_ipc::accounts::Error::ConcurrentChange)
    );
    next_request(&mut s);
    let unconsumed = s.account_grant(7, scope, None).unwrap();
    CAP_BROKER
        .lock()
        .transition_account_session(owner, reference(scope), REVOKE, 1)
        .unwrap();
    assert!(s.account_grant(7, scope, Some(unconsumed)).is_err());
    assert_eq!(CAP_BROKER.lock().session_inventory(), (0, 0, 0));
}

#[test]
fn s2_s5_s7_revocation_and_logout_before_commit_are_terminal_and_idempotent() {
    let _serial = TEST_LOCK.lock().unwrap();
    let (s, scope, owner) = active(100);
    assert_eq!(CAP_BROKER.lock().session_accounting()[2], 1);
    let mut b = CAP_BROKER.lock();
    for _ in 0..2 {
        b.transition_account_session(owner, reference(scope), REVOKE, 1)
            .unwrap();
    }
    assert!(b
        .transition_account_session(owner, reference(scope), ACTIVATE, 1)
        .is_err());
    drop(b);
    assert!(s.account_grant(7, scope, None).is_err());
    assert_eq!(CAP_BROKER.lock().session_inventory(), (0, 0, 0));
}

#[test]
fn s4_s15_s16_concurrent_commit_revoke_and_cancel_choose_one_order() {
    let _serial = TEST_LOCK.lock().unwrap();
    for cancellation in [false, true] {
        let (s, scope, owner) = active(100);
        let delivery = s.grant_delivery_for_test();
        let issuer = RecipientIdentity::of(&s.processes[0]);
        let binding = sunlight_ipc::accounts::GrantBinding {
            pid: 7,
            process_generation: 17,
            scope,
        };
        let token = s.account_grant(7, scope, None).unwrap();
        complete(&s, &[token], true);
        let start = Arc::new(Barrier::new(3));
        let a = start.clone();
        let consume = std::thread::spawn(move || {
            a.wait();
            CAP_BROKER.lock().commit_account_session_grant(
                issuer,
                delivery,
                0,
                binding,
                Some(token),
                1,
            )
        });
        let a = start.clone();
        let retire = std::thread::spawn(move || {
            a.wait();
            let mut b = CAP_BROKER.lock();
            if cancellation {
                b.retire_recipient_grants(7);
            } else {
                b.transition_account_session(owner, reference(scope), REVOKE, 1)
                    .unwrap();
            }
        });
        start.wait();
        let consumed = consume.join().unwrap();
        retire.join().unwrap();
        assert!(consumed.is_err() || consumed == Ok(token));
        assert!(s.account_grant(7, scope, Some(token)).is_err());
        CAP_BROKER.lock().retire_sessions_for(owner.pid);
        assert_eq!(CAP_BROKER.lock().session_inventory(), (0, 0, 0));
    }
    // Genuine overlapping attempts on the actual commit/revoke broker guard.
    let (s, scope, owner) = active(100);
    let delivery = s.grant_delivery_for_test();
    let issuer = RecipientIdentity::of(&s.processes[0]);
    let binding = sunlight_ipc::accounts::GrantBinding {
        pid: 7,
        process_generation: 17,
        scope,
    };
    let start = Arc::new(Barrier::new(3));
    let a = start.clone();
    let commit = std::thread::spawn(move || {
        a.wait();
        CAP_BROKER
            .lock()
            .commit_account_session_grant(issuer, delivery, 0, binding, None, 1)
    });
    let a = start.clone();
    let revoke = std::thread::spawn(move || {
        a.wait();
        CAP_BROKER
            .lock()
            .transition_account_session(owner, reference(scope), REVOKE, 1)
            .unwrap();
    });
    start.wait();
    let result = commit.join().unwrap();
    revoke.join().unwrap();
    assert!(
        result.is_ok() || result == Err(sunlight_ipc::accounts::Error::SessionAuthorityUnavailable)
    );
    assert_eq!(CAP_BROKER.lock().session_inventory(), (0, 0, 0));
}

#[test]
fn s6_expiration_at_boundary_fences_mint_consume_and_activation() {
    let _serial = TEST_LOCK.lock().unwrap();
    let (mut s, scope, owner) = active(2);
    let token = s.account_grant(7, scope, None).unwrap();
    s.global_tick = 2;
    assert!(s.account_grant(7, scope, Some(token)).is_err());
    assert!(s.account_grant(7, scope, None).is_err());
    assert!(CAP_BROKER
        .lock()
        .transition_account_session(owner, reference(scope), ACTIVATE, 2)
        .is_err());
    assert_eq!(CAP_BROKER.lock().session_inventory(), (0, 0, 0));
}

#[test]
fn s9_s17_exhaustion_and_reinitialized_broker_do_not_alias() {
    let _serial = TEST_LOCK.lock().unwrap();
    let (s, scope, _) = active(100);
    let old_epoch = scope.session_generation;
    drop(s);
    let (s, new, _) = active(100);
    assert_ne!(old_epoch, new.session_generation);
    assert!(s.account_grant(7, scope, None).is_err());
    CAP_BROKER.lock().exhaust_session_ids_for_test();
    assert!(CAP_BROKER.lock().reserve_session_for_test().is_err());
}

#[test]
fn s10_s11_s12_s13_s14_stale_service_roles_incarnations_and_vm_rejected() {
    let _serial = TEST_LOCK.lock().unwrap();
    for fault in 0..8 {
        let (mut s, scope, _) = active(100);
        match fault {
            0 => s.processes[0].exiting = true,
            1 => s.processes[0].foreground.incarnation += 100,
            2 => s.processes[0].address_space.generation += 100,
            3 => s.processes[2].exiting = true,
            4 => s.processes[2].foreground.incarnation += 100,
            5 => s.processes[2].address_space.generation += 100,
            6 => s.processes[1].foreground.incarnation += 100,
            _ => {
                s.processes[1].address_space.generation += 100;
                // A real successful exec withdraws the old pending IPC call.
                s.processes[1].pending_call = None;
            }
        }
        assert!(s.account_grant(7, scope, None).is_err(), "fault={fault}");
        CAP_BROKER.lock().retire_sessions_for(8);
        assert_eq!(CAP_BROKER.lock().session_inventory(), (0, 0, 0));
    }
}

#[test]
fn s18_entropy_and_delivery_failure_publish_no_authority() {
    let _serial = TEST_LOCK.lock().unwrap();
    let (s, scope, _) = active(100);
    CAP_BROKER.lock().test_entropy(&[None]);
    assert!(s.account_grant(7, scope, None).is_err());
    assert_eq!(CAP_BROKER.lock().session_inventory(), (1, 0, 0));
    CAP_BROKER.lock().test_entropy(&[Some(900_001)]);
    let token = s.account_grant(7, scope, None).unwrap();
    complete(&s, &[], false);
    assert!(s.account_grant(7, scope, Some(token)).is_err());
    assert!(s.account_grant(7, scope, None).is_err());
    // Failed transport retires the exact retry receipt along with its grant.
    assert_eq!(CAP_BROKER.lock().session_inventory(), (1, 0, 0));
}

#[test]
fn suspension_denies_consumption_without_reviving_cancelled_grants() {
    let _serial = TEST_LOCK.lock().unwrap();
    let (s, scope, owner) = active(100);
    let token = s.account_grant(7, scope, None).unwrap();
    CAP_BROKER
        .lock()
        .transition_account_session(owner, reference(scope), SUSPEND, 1)
        .unwrap();
    assert!(s.account_grant(7, scope, Some(token)).is_err());
    CAP_BROKER
        .lock()
        .transition_account_session(owner, reference(scope), ACTIVATE, 1)
        .unwrap();
    assert!(s.account_grant(7, scope, Some(token)).is_err());
    assert!(s.account_grant(7, scope, None).is_err());
}

#[test]
fn registration_requires_live_roles_exact_login_request_and_original_issuer_vm() {
    let _serial = TEST_LOCK.lock().unwrap();
    for fault in 0..7 {
        let mut s = table();
        s.processes[1].trusted_tty_session_service = true;
        s.processes[2].trusted_session_service = true;
        let token = proof(&mut s, 0);
        match fault {
            0 => s.processes[0].trusted_auth_broker = false,
            1 => s.processes[0].foreground.incarnation += 100,
            2 => s.processes[0].address_space.generation += 100,
            3 => s.processes[1].trusted_tty_session_service = false,
            4 => s.processes[1].pending_call = None,
            5 => s.processes[2].trusted_session_service = false,
            _ => s.processes[0].exiting = true,
        }
        s.processes.swap(0, 2);
        s.processes[0].ipc_reply_target = Some(reply(7, 2, 7));
        assert!(s.register_account_session(token, 7, 100).is_err());
        assert_eq!(CAP_BROKER.lock().session_inventory(), (0, 0, 0));
    }
}

#[test]
fn live_owner_control_principal_policy_and_bounded_receipts() {
    let _serial = TEST_LOCK.lock().unwrap();
    let (mut s, scope, owner) = active(1000);
    assert!(s
        .transition_account_session(reference(scope), REVOKE)
        .is_err());
    let intruder = RecipientIdentity {
        incarnation: owner.incarnation + 1,
        ..owner
    };
    assert!(CAP_BROKER
        .lock()
        .transition_account_session(intruder, reference(scope), REVOKE, 1)
        .is_err());
    s.processes[1].uid = 1000;
    assert!(s.account_grant(7, scope, None).is_err());
    s.processes[1].uid = 0;
    for _ in 0..256 {
        let token = s.account_grant(7, scope, None).unwrap();
        assert!(s.account_grant(7, scope, Some(token)).is_ok());
        next_request(&mut s);
        assert_eq!(CAP_BROKER.lock().session_inventory(), (1, 1, 0));
    }
    CAP_BROKER.lock().retire_recipient_grants(7);
    assert_eq!(CAP_BROKER.lock().session_inventory(), (1, 0, 0));
    CAP_BROKER.lock().retire_sessions_for(owner.pid);
    assert_eq!(CAP_BROKER.lock().session_inventory(), (0, 0, 0));
}

#[test]
fn session_capacity_failure_preserves_proof_and_allocation_failure_preserves_receipt() {
    let _serial = TEST_LOCK.lock().unwrap();
    let (mut s, scope, owner) = active(1000);
    CAP_BROKER.lock().test_fail_reservation(true);
    assert!(s.account_grant(7, scope, None).is_err());
    assert_eq!(CAP_BROKER.lock().session_inventory(), (1, 0, 0));
    CAP_BROKER.lock().test_fail_reservation(false);
    let issuer = RecipientIdentity::of(&s.processes[0]);
    for index in 0..16 {
        let token = proof(&mut s, 0);
        let mut request = s.grant_delivery_for_test();
        request.issuer_pid = owner.pid;
        request.issuer_incarnation = owner.incarnation;
        request.issuer_vm_generation = owner.vm_generation;
        let mut b = CAP_BROKER.lock();
        let result = b.register_account_session(owner, issuer, request, token, 1, 1000);
        if index < 15 {
            assert!(result.is_ok());
        } else {
            assert!(result.is_err());
            assert!(b.auth_session_grants.iter().any(|g| g.token == token));
        }
    }
    CAP_BROKER.lock().retire_sessions_for(owner.pid);
    CAP_BROKER.lock().retire_recipient_grants(7);
    assert_eq!(CAP_BROKER.lock().session_inventory(), (0, 0, 0));
}

#[test]
fn launch_attachment_is_owner_and_generation_qualified() {
    let _serial = TEST_LOCK.lock().unwrap();
    let (_s, scope, owner) = active(100);
    let mut broker = CAP_BROKER.lock();
    let session = reference(scope);
    let (permit, uid, gid) = broker.authorize_session_launch(owner, session, 1).unwrap();
    assert_eq!((uid, gid), (0, 0));
    assert_eq!(broker.commit_session_launch(permit, 1), Ok(session));
    for foreign in [
        RecipientIdentity {
            pid: owner.pid + 1,
            ..owner
        },
        RecipientIdentity {
            incarnation: owner.incarnation + 1,
            ..owner
        },
        RecipientIdentity {
            vm_generation: owner.vm_generation + 1,
            ..owner
        },
    ] {
        assert!(broker
            .authorize_session_launch(foreign, session, 1)
            .is_err());
    }
    for forged in [
        SessionRef {
            id: session.id + 1,
            ..session
        },
        SessionRef {
            epoch: session.epoch + 1,
            ..session
        },
    ] {
        assert!(broker.authorize_session_launch(owner, forged, 1).is_err());
    }
    assert!(broker
        .authorize_session_launch(owner, session, 100)
        .is_err());
    broker
        .transition_account_session(owner, session, SUSPEND, 1)
        .unwrap();
    assert!(broker.authorize_session_launch(owner, session, 1).is_err());
}

#[test]
fn retirement_fences_attachment_commit_and_new_quota_without_refunding_backing() {
    let _serial = TEST_LOCK.lock().unwrap();
    let (_s, scope, owner) = active(100);
    let mut broker = CAP_BROKER.lock();
    let session = reference(scope);
    let (permit, _, _) = broker.authorize_session_launch(owner, session, 1).unwrap();
    assert!(broker.session_allocation_live(session, 1));
    use crate::memory::shared_quota::{self, ProcessIdentity, SessionIdentity};
    let process = ProcessIdentity {
        pid: 9876,
        incarnation: 8765,
    };
    let key = Some(SessionIdentity::Account {
        id: session.id,
        epoch: session.epoch,
    });
    let receipt = shared_quota::reserve(process, key, 4096).unwrap();
    broker
        .transition_account_session(owner, session, REVOKE, 1)
        .unwrap();
    assert!(!broker.session_allocation_live(session, 1));
    assert!(broker.authorize_session_launch(owner, session, 1).is_err());
    assert!(broker.commit_session_launch(permit, 1).is_err());
    assert_eq!(shared_quota::observation(process, key), (4096, 4096));
    drop(receipt);
    assert_eq!(shared_quota::observation(process, key), (0, 0));
}

#[test]
fn attachment_commit_before_retirement_keeps_identity_but_fences_future_allocation() {
    let _serial = TEST_LOCK.lock().unwrap();
    let (_s, scope, owner) = active(100);
    let mut broker = CAP_BROKER.lock();
    let session = reference(scope);
    let (permit, _, _) = broker.authorize_session_launch(owner, session, 1).unwrap();
    let membership = broker.commit_session_launch(permit, 1).unwrap();
    broker
        .transition_account_session(owner, session, REVOKE, 1)
        .unwrap();
    assert_eq!(membership, session);
    assert!(!broker.session_allocation_live(membership, 1));
}

#[test]
fn concurrent_attachment_and_retirement_have_one_broker_order() {
    let _serial = TEST_LOCK.lock().unwrap();
    let (_s, scope, owner) = active(100);
    let session = reference(scope);
    let (permit, _, _) = CAP_BROKER
        .lock()
        .authorize_session_launch(owner, session, 1)
        .unwrap();
    let start = std::sync::Arc::new(std::sync::Barrier::new(3));
    let launch_start = start.clone();
    let launch = std::thread::spawn(move || {
        launch_start.wait();
        CAP_BROKER.lock().commit_session_launch(permit, 1)
    });
    let retire_start = start.clone();
    let retire = std::thread::spawn(move || {
        retire_start.wait();
        CAP_BROKER
            .lock()
            .transition_account_session(owner, session, REVOKE, 1)
    });
    start.wait();
    let result = launch.join().unwrap();
    assert!(result == Ok(session) || result == Err(sunlight_ipc::accounts::Error::SessionChanged));
    retire.join().unwrap().unwrap();
    assert!(!CAP_BROKER.lock().session_allocation_live(session, 1));
}

#[test]
fn initial_launch_is_one_use_and_requires_fresh_owner_authority() {
    let _serial = TEST_LOCK.lock().unwrap();
    let (_s, scope, owner) = registered(100, false);
    let session = reference(scope);
    let mut b = CAP_BROKER.lock();
    assert!(b.authorize_session_launch(owner, session, 1).is_err());
    assert!(!b.session_allocation_live(session, 1));
    for foreign in [RecipientIdentity { pid: owner.pid + 1, ..owner },
        RecipientIdentity { incarnation: owner.incarnation + 1, ..owner },
        RecipientIdentity { vm_generation: owner.vm_generation + 1, ..owner }] {
        assert!(b.authorize_initial_session_launch(foreign, session, 1).is_err());
    }
    assert!(b.authorize_initial_session_launch(owner,
        SessionRef { epoch: session.epoch + 1, ..session }, 1).is_err());
    let (first, _, _) = b.authorize_initial_session_launch(owner, session, 1).unwrap();
    let (second, _, _) = b.authorize_initial_session_launch(owner, session, 1).unwrap();
    assert_eq!(b.commit_session_launch(first, 1), Ok(session));
    assert!(b.commit_session_launch(second, 1).is_err());
    assert!(b.session_allocation_live(session, 1));
    assert!(b.authorize_initial_session_launch(owner, session, 1).is_err());
}

#[test]
fn suspension_and_retirement_fence_bootstrap_and_old_active_permits() {
    let _serial = TEST_LOCK.lock().unwrap();
    for operation in [SUSPEND, REVOKE] {
        let (_s, scope, owner) = registered(100, false);
        let session = reference(scope);
        let mut b = CAP_BROKER.lock();
        let (permit, _, _) = b.authorize_initial_session_launch(owner, session, 1).unwrap();
        b.transition_account_session(owner, session, operation, 1).unwrap();
        assert!(b.commit_session_launch(permit, 1).is_err());
        assert!(b.authorize_initial_session_launch(owner, session, 1).is_err());
    }
    let (_s, scope, owner) = active(100);
    let session = reference(scope);
    let mut b = CAP_BROKER.lock();
    let (permit, _, _) = b.authorize_session_launch(owner, session, 1).unwrap();
    b.transition_account_session(owner, session, SUSPEND, 1).unwrap();
    b.transition_account_session(owner, session, ACTIVATE, 1).unwrap();
    assert!(b.commit_session_launch(permit, 1).is_err());
    let (permit, _, _) = b.authorize_session_launch(owner, session, 1).unwrap();
    assert!(b.commit_session_launch(permit, 100).is_err());
}
