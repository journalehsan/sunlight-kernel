use super::*;
use crate::lifecycle_host::{reply, AddressSpace, Foreground, Process, Scheduler};
use alloc::boxed::Box;
use std::sync::{Arc, Barrier, Mutex};
use sunlight_ipc::accounts::{GrantScope, Operation};

static TEST_LOCK: Mutex<()> = Mutex::new(());
std::thread_local! {
    static HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
}
pub(super) fn before_commit() {
    HOOK.with(|hook| {
        if let Some(f) = hook.borrow_mut().take() {
            f();
        }
    });
}
fn process(pid: usize) -> Process {
    Process {
        pid,
        trusted_auth_broker: pid == 6,
        trusted_vfs_service: false,
        trusted_service_manager: false,
        trusted_tty_session_service: false,
        trusted_lock_service: false,
        exiting: false,
        foreground: Foreground {
            incarnation: pid as u64,
        },
        address_space: AddressSpace {
            generation: pid as u64 + 10,
        },
        pending_call: (pid != 6).then_some((pid, 1)),
        ipc_reply_target: (pid == 6).then_some(reply(7, 1, 7)),
    }
}
fn table() -> Scheduler {
    *CAP_BROKER.lock() = CapabilityBroker::new();
    Scheduler {
        processes: alloc::vec![process(6), process(7), process(8)],
        global_tick: 1,
    }
}
fn scope() -> GrantScope {
    GrantScope {
        revision: 1,
        operation: Operation::GroupCreate,
        target: 0,
        member: 0,
        session: 3,
        session_generation: 4,
    }
}
fn issue(s: &Scheduler, account: bool) -> Option<CapabilityToken> {
    if account {
        s.account_grant(7, scope(), None)
    } else {
        s.issue_auth_session_grant(7, 0, 0)
    }
}
fn inventory() -> usize {
    let b = CAP_BROKER.lock();
    b.auth_session_grants.len() + b.account_grants.len()
}
fn cleanup(s: &mut Scheduler, pid: usize) {
    s.processes
        .iter_mut()
        .find(|p| p.pid == pid)
        .unwrap()
        .exiting = true;
    CAP_BROKER.lock().revoke_endpoints_owned_by(pid);
}
fn commit_vs_cleanup(account: bool) {
    let _serial = TEST_LOCK.lock().unwrap();
    let s = Arc::new(Mutex::new(table()));
    let validated = Arc::new(Barrier::new(2));
    let resume = Arc::new(Barrier::new(2));
    let worker = {
        let s = s.clone();
        let validated = validated.clone();
        let resume = resume.clone();
        std::thread::spawn(move || {
            HOOK.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(move || {
                    validated.wait();
                    resume.wait();
                }))
            });
            issue(&s.lock().unwrap(), account).unwrap()
        })
    };
    validated.wait();
    // Exactly the old boundary: validation complete, insertion not yet done.
    // Cleanup cannot complete while this transaction owns lifecycle serialization.
    assert!(matches!(
        s.try_lock(),
        Err(std::sync::TryLockError::WouldBlock)
    ));
    assert_eq!(inventory(), 0);
    resume.wait();
    let _token = worker.join().unwrap();
    assert_eq!(inventory(), 1);
    let mut s = s.lock().unwrap();
    cleanup(&mut s, 7);
    assert_eq!(inventory(), 0);
    assert!(issue(&s, account).is_none());
    cleanup(&mut s, 7);
    assert_eq!(inventory(), 0);
}
#[test]
fn auth_publication_vs_cleanup() {
    commit_vs_cleanup(false);
}
#[test]
fn account_publication_vs_cleanup() {
    commit_vs_cleanup(true);
}

#[test]
fn preparation_exit_issuer_exit_reuse_and_exec() {
    let _serial = TEST_LOCK.lock().unwrap();
    for account in [false, true] {
        let mut s = table();
        cleanup(&mut s, 7); // cleanup before commit, including external preparation
        assert!(issue(&s, account).is_none());
        assert_eq!(inventory(), 0);
        s.processes[1] = process(7);
        s.processes[1].foreground.incarnation += 100;
        s.processes[1].address_space.generation += 100;
        s.processes[0].ipc_reply_target = Some(reply(7, 1, s.processes[1].foreground.incarnation));
        let token = issue(&s, account).unwrap();
        CAP_BROKER.lock().retire_recipient_grants(7); // successful exec withdrawal
        s.processes[1].address_space.generation += 1;
        assert!(!CAP_BROKER.lock().token_in_use(token));
        cleanup(&mut s, 6);
        assert!(issue(&s, account).is_none());
        cleanup(&mut s, 7);
        assert_eq!(inventory(), 0);
    }
}
#[test]
fn faults_exhaustion_and_multiple_recipients() {
    let _serial = TEST_LOCK.lock().unwrap();
    for account in [false, true] {
        let mut s = table();
        CAP_BROKER.lock().test_fail_reservation(true);
        assert!(issue(&s, account).is_none());
        assert_eq!(inventory(), 0);
        CAP_BROKER.lock().test_fail_reservation(false);
        CAP_BROKER.lock().test_entropy(&[None]);
        assert!(issue(&s, account).is_none());
        assert_eq!(inventory(), 0);
        CAP_BROKER.lock().test_entropy(&[Some(5)]);
        let original = issue(&s, account).unwrap();
        CAP_BROKER.lock().test_entropy(&[Some(5); TOKEN_ATTEMPTS]);
        assert!(issue(&s, account).is_none());
        assert_eq!(inventory(), 1);
        CAP_BROKER.lock().test_words = None;
        for _ in 1..64 {
            issue(&s, account).unwrap();
        }
        assert!(issue(&s, account).is_none());
        assert_eq!(inventory(), 64);
        cleanup(&mut s, 7);
        assert!(!CAP_BROKER.lock().token_in_use(original));
        s.processes[0].ipc_reply_target = Some(reply(
            8,
            1,
            s.processes
                .iter()
                .find(|p| p.pid == 8)
                .unwrap()
                .foreground
                .incarnation,
        ));
        let other = if account {
            s.account_grant(8, scope(), None)
        } else {
            s.issue_auth_session_grant(8, 0, 0)
        }
        .unwrap();
        cleanup(&mut s, 7);
        assert!(CAP_BROKER.lock().token_in_use(other));
        cleanup(&mut s, 8);
        assert_eq!(inventory(), 0);
    }
}
#[test]
fn concurrent_grants_and_cleanup_have_no_late_publication() {
    let _serial = TEST_LOCK.lock().unwrap();
    let s = Arc::new(Mutex::new(table()));
    let start = Arc::new(Barrier::new(9));
    let mut workers = std::vec::Vec::new();
    for n in 0..8 {
        let s = s.clone();
        let start = start.clone();
        workers.push(std::thread::spawn(move || {
            start.wait();
            for _ in 0..64 {
                let _ = issue(&s.lock().unwrap(), n % 2 == 0);
            }
        }));
    }
    start.wait();
    cleanup(&mut s.lock().unwrap(), 7);
    for w in workers {
        w.join().unwrap();
    }
    assert_eq!(inventory(), 0);
}

#[test]
fn invalid_identity_and_stale_ipc_never_publish() {
    let _serial = TEST_LOCK.lock().unwrap();
    for account in [false, true] {
        for fault in 0..7 {
            let mut s = table();
            match fault {
                0 => s.processes[0].trusted_auth_broker = false,
                1 => s.processes[0].foreground.incarnation = 0,
                2 => s.processes[1].foreground.incarnation = 0,
                3 => s.processes[1].address_space.generation = 0,
                4 => s.processes[1].pending_call = None,
                5 => {
                    s.processes[0].ipc_reply_target = Some(reply(
                        7,
                        2,
                        s.processes
                            .iter()
                            .find(|p| p.pid == 7)
                            .unwrap()
                            .foreground
                            .incarnation,
                    ))
                }
                _ => {
                    s.processes.pop();
                    s.processes.pop();
                }
            }
            assert!(issue(&s, account).is_none());
            assert_eq!(inventory(), 0);
        }
    }
}

#[test]
fn concurrent_participant_exit_cannot_resurrect_grants() {
    let _serial = TEST_LOCK.lock().unwrap();
    for _ in 0..16 {
        let s = Arc::new(Mutex::new(table()));
        let start = Arc::new(Barrier::new(4));
        let mut workers = std::vec::Vec::new();
        for role in 0..3 {
            let s = s.clone();
            let start = start.clone();
            workers.push(std::thread::spawn(move || {
                start.wait();
                let mut s = s.lock().unwrap();
                match role {
                    0 => {
                        let _ = issue(&s, false);
                        let _ = issue(&s, true);
                    }
                    1 => cleanup(&mut s, 6),
                    _ => cleanup(&mut s, 7),
                }
            }));
        }
        start.wait();
        for worker in workers {
            worker.join().unwrap();
        }
        let s = s.lock().unwrap();
        assert!(issue(&s, false).is_none() && issue(&s, true).is_none());
        assert_eq!(inventory(), 0);
    }
}

#[test]
fn cleanup_during_synthetic_preparation_rejects_both_grant_types() {
    let _serial = TEST_LOCK.lock().unwrap();
    for account in [false, true] {
        let s = Arc::new(Mutex::new(table()));
        let prepared = Arc::new(Barrier::new(2));
        let resume = Arc::new(Barrier::new(2));
        let worker = {
            let s = s.clone();
            let prepared = prepared.clone();
            let resume = resume.clone();
            std::thread::spawn(move || {
                // Only non-authoritative request data may be prepared without
                // lifecycle serialization. No validated identity is retained.
                let request_scope = scope();
                prepared.wait();
                resume.wait();
                let s = s.lock().unwrap();
                if account {
                    s.account_grant(7, request_scope, None)
                } else {
                    s.issue_auth_session_grant(7, 0, 0)
                }
            })
        };
        prepared.wait();
        cleanup(&mut s.lock().unwrap(), 7);
        assert_eq!(inventory(), 0);
        resume.wait();
        assert!(worker.join().unwrap().is_none());
        assert_eq!(inventory(), 0);
    }
}

#[test]
fn committed_grants_remain_recipient_owned_after_issuer_exit() {
    let _serial = TEST_LOCK.lock().unwrap();
    let mut s = table();
    let auth = issue(&s, false).unwrap();
    let account = issue(&s, true).unwrap();
    assert_eq!(inventory(), 2);
    CAP_BROKER.lock().settle_grant_delivery(
        7,
        7,
        1,
        Some(super::delivery::RecipientIdentity::of(&s.processes[1])),
        &[auth.0, account.0],
    );
    cleanup(&mut s, 6);
    // Issuer withdrawal must not silently adopt or reassign these grants.
    // Their existing contract assigns cleanup responsibility to recipient 7.
    assert!(CAP_BROKER.lock().token_in_use(auth));
    assert!(CAP_BROKER.lock().token_in_use(account));
    assert!(issue(&s, false).is_none() && issue(&s, true).is_none());
    cleanup(&mut s, 7);
    cleanup(&mut s, 6);
    cleanup(&mut s, 7);
    assert_eq!(inventory(), 0);
}

#[test]
fn grant_withdrawal_does_not_permanently_ban_a_live_pid() {
    let _serial = TEST_LOCK.lock().unwrap();
    let s = table();
    let old_auth = issue(&s, false).unwrap();
    let old_account = issue(&s, true).unwrap();
    CAP_BROKER.lock().retire_recipient_grants(7);
    assert_eq!(inventory(), 0);
    assert!(!CAP_BROKER.lock().token_in_use(old_auth));
    assert!(!CAP_BROKER.lock().token_in_use(old_account));
    // Withdrawal alone is not recipient lifecycle invalidation. A live,
    // authenticated request may receive new authority after token revocation.
    issue(&s, false).unwrap();
    issue(&s, true).unwrap();
    assert_eq!(inventory(), 2);
    CAP_BROKER.lock().retire_recipient_grants(7);
    assert_eq!(inventory(), 0);
}

#[test]
fn capacity_failure_then_cleanup_returns_grant_inventory_to_zero() {
    let _serial = TEST_LOCK.lock().unwrap();
    for account in [false, true] {
        let mut s = table();
        for _ in 0..64 {
            issue(&s, account).unwrap();
        }
        assert_eq!(inventory(), 64);
        assert!(issue(&s, account).is_none());
        cleanup(&mut s, 7);
        assert_eq!(inventory(), 0);
        // Reuse the numeric locator with a fresh synthetic incarnation and VM.
        let mut next = process(7);
        next.foreground.incarnation += 1000;
        next.address_space.generation += 1000;
        next.pending_call = Some((7, 2));
        s.processes[1] = next;
        assert!(issue(&s, account).is_none()); // stale reply generation
        s.processes[0].ipc_reply_target = Some(reply(
            7,
            2,
            s.processes
                .iter()
                .find(|p| p.pid == 7)
                .unwrap()
                .foreground
                .incarnation,
        ));
        issue(&s, account).unwrap();
        assert_eq!(inventory(), 1);
        cleanup(&mut s, 7);
        assert_eq!(inventory(), 0);
    }
}

// These barriers exercise the production issuance functions. The host process
// table models exec invalidation; real CR3/stack/reclamation checks are native.
fn commit_vs_exec(account: bool) {
    let _serial = TEST_LOCK.lock().unwrap();
    let s = Arc::new(Mutex::new(table()));
    let validated = Arc::new(Barrier::new(2));
    let resume = Arc::new(Barrier::new(2));
    let worker = {
        let s = s.clone();
        let validated = validated.clone();
        let resume = resume.clone();
        std::thread::spawn(move || {
            HOOK.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(move || {
                    validated.wait();
                    resume.wait();
                }));
            });
            issue(&s.lock().unwrap(), account).unwrap()
        })
    };
    validated.wait();
    assert!(matches!(
        s.try_lock(),
        Err(std::sync::TryLockError::WouldBlock)
    ));
    assert_eq!(inventory(), 0);
    resume.wait();
    let token = worker.join().unwrap();
    let mut s = s.lock().unwrap();
    assert_eq!(inventory(), 1);
    let incarnation = s.processes[1].foreground.incarnation;
    let old_generation = s.processes[1].address_space.generation;
    // Precommit preparation failure preserves the old authority and VM.
    assert!(CAP_BROKER.lock().token_in_use(token));
    assert_eq!(s.processes[1].address_space.generation, old_generation);
    let (_, endpoint) = CAP_BROKER.lock().create_endpoint(7).unwrap();
    CAP_BROKER.lock().retire_recipient_grants(7);
    s.processes[1].pending_call = None;
    s.processes[1].ipc_reply_target = None;
    s.processes[1].address_space.generation += 1;
    assert_eq!(s.processes[1].foreground.incarnation, incarnation);
    assert!(!CAP_BROKER.lock().token_in_use(token));
    assert!(CAP_BROKER.lock().token_in_use(endpoint));
    assert_eq!(inventory(), 0);
    // Exec first: the old IPC relationship cannot authorize new publication.
    assert!(issue(&s, account).is_none());
    assert_eq!(inventory(), 0);
    // A fresh new-image request explicitly reauthorizes against the new VM.
    s.processes[1].pending_call = Some((7, 2));
    s.processes[0].ipc_reply_target = Some(reply(
        7,
        2,
        s.processes
            .iter()
            .find(|p| p.pid == 7)
            .unwrap()
            .foreground
            .incarnation,
    ));
    let fresh = issue(&s, account).unwrap();
    assert_ne!(fresh, token);
    assert_eq!(inventory(), 1);
    if account {
        assert_eq!(
            CAP_BROKER.lock().account_grants[0]
                .1
                .grant
                .binding
                .process_generation,
            old_generation + 1
        );
    }
    cleanup(&mut s, 7);
    assert_eq!(inventory(), 0);
    assert!(!CAP_BROKER.lock().token_in_use(endpoint));
}

#[test]
fn auth_publication_vs_exec() {
    commit_vs_exec(false);
}

#[test]
fn account_publication_vs_exec() {
    commit_vs_exec(true);
}

#[test]
fn exec_withdraws_issuer_image_provenance_after_success_only() {
    let _serial = TEST_LOCK.lock().unwrap();
    let mut s = table();
    s.processes[0].trusted_service_manager = true;
    s.processes[0].trusted_tty_session_service = true;
    s.processes[0].trusted_lock_service = true;
    // Failed preparation does not call the commit helper.
    let before = issue(&s, false).unwrap();
    assert!(s.processes[0].trusted_auth_broker);
    let incarnation = s.processes[0].foreground.incarnation;
    s.processes[0].withdraw_exec_image_roles();
    s.processes[0].address_space.generation += 1;
    assert_eq!(s.processes[0].foreground.incarnation, incarnation);
    assert!(!s.processes[0].trusted_service_manager);
    assert!(!s.processes[0].trusted_tty_session_service);
    assert!(!s.processes[0].trusted_lock_service);
    assert!(issue(&s, false).is_none());
    assert!(issue(&s, true).is_none());
    // A committed recipient-owned grant survives issuer image replacement.
    assert!(CAP_BROKER.lock().token_in_use(before));
    cleanup(&mut s, 7);
    assert_eq!(inventory(), 0);
}

#[test]
fn exec_during_external_preparation_cancels_stale_publication() {
    let _serial = TEST_LOCK.lock().unwrap();
    for account in [false, true] {
        let s = Arc::new(Mutex::new(table()));
        let prepared = Arc::new(Barrier::new(2));
        let resume = Arc::new(Barrier::new(2));
        let worker = {
            let s = s.clone();
            let prepared = prepared.clone();
            let resume = resume.clone();
            std::thread::spawn(move || {
                let request_scope = scope();
                prepared.wait();
                resume.wait();
                let s = s.lock().unwrap();
                if account {
                    s.account_grant(7, request_scope, None)
                } else {
                    s.issue_auth_session_grant(7, 0, 0)
                }
            })
        };
        prepared.wait();
        {
            let mut s = s.lock().unwrap();
            CAP_BROKER.lock().retire_recipient_grants(7);
            s.processes[1].pending_call = None;
            s.processes[1].address_space.generation += 1;
        }
        resume.wait();
        assert!(worker.join().unwrap().is_none());
        assert_eq!(inventory(), 0);
    }
}

fn vfs_cap() -> VfsCapability {
    let mut allowed_prefix = heapless::String::new();
    allowed_prefix.push_str("/bin").unwrap();
    VfsCapability {
        allowed_prefix,
        flags: AccessFlags::READ_EXECUTE,
    }
}

#[test]
fn vfs_numeric_slot_is_not_authority() {
    let _serial = TEST_LOCK.lock().unwrap();
    for fault in 0..6 {
        let mut s = table();
        match fault {
            0 => s.processes[0].trusted_auth_broker = false,
            1 => s.processes[0].foreground.incarnation = 0,
            2 => s.processes[0].address_space.generation = 0,
            3 => s.processes[0].exiting = true,
            4 => s.processes[0].pid = (1usize << 32) | 6,
            _ => s.processes[0].pid = 9,
        }
        assert_eq!(
            s.issue_vfs_capability(vfs_cap()),
            Err(CapError::InvalidCaller)
        );
        assert!(CAP_BROKER.lock().vfs_caps.is_empty());
    }
}

#[test]
fn vfs_preparation_exec_exit_and_slot_reuse_fail_closed() {
    let _serial = TEST_LOCK.lock().unwrap();
    for transition in 0..3 {
        let mut s = table();
        let prepared = vfs_cap();
        let old_incarnation = s.processes[0].foreground.incarnation;
        match transition {
            0 => s.processes[0].withdraw_exec_image_roles(),
            1 => cleanup(&mut s, 6),
            _ => {
                s.processes[0] = process(6);
                s.processes[0].foreground.incarnation = old_incarnation + 1;
                s.processes[0].trusted_auth_broker = false;
            }
        }
        assert_eq!(
            s.issue_vfs_capability(prepared),
            Err(CapError::InvalidCaller)
        );
        assert!(CAP_BROKER.lock().vfs_caps.is_empty());
    }
}

#[test]
fn vfs_commit_serializes_role_withdrawal_and_preserves_bearer_policy() {
    let _serial = TEST_LOCK.lock().unwrap();
    let s = Arc::new(Mutex::new(table()));
    let validated = Arc::new(Barrier::new(2));
    let resume = Arc::new(Barrier::new(2));
    let worker = {
        let s = s.clone();
        let validated = validated.clone();
        let resume = resume.clone();
        std::thread::spawn(move || {
            HOOK.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(move || {
                    validated.wait();
                    resume.wait();
                }))
            });
            s.lock().unwrap().issue_vfs_capability(vfs_cap()).unwrap()
        })
    };
    validated.wait();
    assert!(matches!(
        s.try_lock(),
        Err(std::sync::TryLockError::WouldBlock)
    ));
    assert!(CAP_BROKER.lock().vfs_caps.is_empty());
    resume.wait();
    let token = worker.join().unwrap();
    let mut s = s.lock().unwrap();
    assert_eq!(s.processes[0].foreground.incarnation, 6);
    assert!(s.processes[0].trusted_auth_broker);
    assert_eq!(
        CAP_BROKER.lock().resolve_vfs_capability(token).unwrap().1,
        6
    );
    s.processes[0].withdraw_exec_image_roles();
    assert_eq!(
        s.issue_vfs_capability(vfs_cap()),
        Err(CapError::InvalidCaller)
    );
    cleanup(&mut s, 6);
    cleanup(&mut s, 6);
    // Existing tokens are transferable; issuer death is not bearer revocation.
    assert!(CAP_BROKER.lock().resolve_vfs_capability(token).is_some());
}

#[test]
fn vfs_failed_reservation_or_entropy_has_no_partial_record() {
    let _serial = TEST_LOCK.lock().unwrap();
    let s = table();
    CAP_BROKER.lock().test_fail_reservation(true);
    assert!(s.issue_vfs_capability(vfs_cap()).is_err());
    assert!(CAP_BROKER.lock().vfs_caps.is_empty());
    CAP_BROKER.lock().test_fail_reservation(false);
    CAP_BROKER.lock().test_entropy(&[None]);
    assert!(s.issue_vfs_capability(vfs_cap()).is_err());
    assert!(CAP_BROKER.lock().vfs_caps.is_empty());
}

fn delivered_identity(s: &Scheduler) -> super::delivery::RecipientIdentity {
    super::delivery::RecipientIdentity::of(&s.processes[1])
}
fn settle(s: &Scheduler, tokens: &[CapabilityToken], deliver: bool) {
    CAP_BROKER.lock().settle_grant_delivery(
        7,
        s.processes[1].foreground.incarnation,
        1,
        deliver.then(|| delivered_identity(s)),
        &tokens.iter().map(|t| t.0).collect::<std::vec::Vec<_>>(),
    );
}
#[test]
fn delivery_pending_is_not_usage_eligibility() {
    let _serial = TEST_LOCK.lock().unwrap();
    let s = table();
    let auth = issue(&s, false).unwrap();
    let account = issue(&s, true).unwrap();
    let b = CAP_BROKER.lock();
    assert!(!b.auth_session_grants[0]
        .delivery
        .unwrap()
        .usable(delivered_identity(&s)));
    assert!(!b.account_grants[0].1.delivery.unwrap().delivered);
    assert_eq!(
        b.auth_session_grants[0]
            .delivery
            .unwrap()
            .issuer_incarnation,
        6
    );
    drop(b);
    settle(&s, &[auth, account], true);
    assert_eq!(
        CAP_BROKER
            .lock()
            .consume_live_auth_grant(auth, delivered_identity(&s), 1),
        Some((0, 0))
    );
    assert!(s.account_grant(7, scope(), Some(account)).is_some());
    assert_eq!(inventory(), 0);
}
#[test]
fn delivery_failure_cancellation_and_omission_retire_pending_records() {
    let _serial = TEST_LOCK.lock().unwrap();
    for mode in 0..3 {
        let s = table();
        let a = issue(&s, false).unwrap();
        let b = issue(&s, true).unwrap();
        let listed = [a];
        settle(&s, if mode == 2 { &listed[..] } else { &[] }, mode == 2);
        settle(&s, &[], false); // duplicate cancellation cannot revive/remove delivered authority
        assert!(!CAP_BROKER.lock().token_in_use(b));
        assert_eq!(inventory(), usize::from(mode == 2));
        if mode == 2 {
            assert_eq!(
                CAP_BROKER
                    .lock()
                    .consume_live_auth_grant(a, delivered_identity(&s), 1),
                Some((0, 0))
            );
        }
        assert_eq!(inventory(), 0);
    }
}
#[test]
fn delivery_cleanup_exec_and_vm_mismatch_never_adopt_authority() {
    let _serial = TEST_LOCK.lock().unwrap();
    for mode in 0..4 {
        let mut s = table();
        let a = issue(&s, false).unwrap();
        let b = issue(&s, true).unwrap();
        match mode {
            0 => cleanup(&mut s, 7),
            1 => {
                CAP_BROKER.lock().retire_recipient_grants(7);
                s.processes[1].address_space.generation += 1;
            }
            2 => s.processes[1].address_space.generation += 1, // defense even if transition retirement were missed
            _ => {
                cleanup(&mut s, 6);
            }
        }
        settle(&s, &[a, b], true);
        assert_eq!(inventory(), 0);
        assert!(CAP_BROKER
            .lock()
            .consume_live_auth_grant(a, delivered_identity(&s), 1)
            .is_none());
    }
}
#[test]
fn stale_incarnation_with_identical_pending_generation_is_rejected() {
    let _serial = TEST_LOCK.lock().unwrap();
    let mut s = table();
    let stale = s.processes[0].ipc_reply_target;
    cleanup(&mut s, 7);
    s.processes[1] = process(7);
    s.processes[1].foreground.incarnation += 1;
    s.processes[0].ipc_reply_target = stale;
    assert!(issue(&s, false).is_none() && issue(&s, true).is_none());
    assert_eq!(inventory(), 0);
}
#[test]
fn token_observed_before_delivery_cannot_be_consumed() {
    let _serial = TEST_LOCK.lock().unwrap();
    let s = table();
    let t = issue(&s, false).unwrap();
    assert!(CAP_BROKER
        .lock()
        .consume_live_auth_grant(t, delivered_identity(&s), 1)
        .is_none());
    settle(&s, &[t], true);
    assert_eq!(inventory(), 0);
}
#[test]
fn delivered_authority_survives_issuer_exit_but_not_recipient_exec() {
    let _serial = TEST_LOCK.lock().unwrap();
    let mut s = table();
    let t = issue(&s, false).unwrap();
    settle(&s, &[t], true);
    cleanup(&mut s, 6);
    assert_eq!(inventory(), 1);
    CAP_BROKER.lock().retire_recipient_grants(7);
    s.processes[1].address_space.generation += 1;
    assert_eq!(inventory(), 0);
    assert!(CAP_BROKER
        .lock()
        .consume_live_auth_grant(t, delivered_identity(&s), 1)
        .is_none());
}
#[test]
fn stale_delivered_token_cannot_authorize_replacement_incarnation_or_vm() {
    let _serial = TEST_LOCK.lock().unwrap();
    for vm in [false, true] {
        let s = table();
        let t = issue(&s, false).unwrap();
        settle(&s, &[t], true);
        let mut replacement = delivered_identity(&s);
        if vm {
            replacement.vm_generation += 1;
        } else {
            replacement.incarnation += 1;
        }
        assert!(CAP_BROKER
            .lock()
            .consume_live_auth_grant(t, replacement, 1)
            .is_none());
        assert_eq!(inventory(), 0);
    }
}
#[test]
fn repeated_delivery_cleanup_cycles_have_exact_metadata_inventory() {
    let _serial = TEST_LOCK.lock().unwrap();
    for cycle in 0..64 {
        let mut s = table();
        s.processes[1].foreground.incarnation += cycle;
        s.processes[0].ipc_reply_target = Some(reply(7, 1, s.processes[1].foreground.incarnation));
        let a = issue(&s, false).unwrap();
        let b = issue(&s, true).unwrap();
        assert_eq!(inventory(), 2);
        settle(&s, &[a, b], cycle % 2 == 0);
        cleanup(&mut s, 7);
        settle(&s, &[a, b], true);
        cleanup(&mut s, 7);
        assert_eq!(inventory(), 0);
    }
}

#[test]
fn uac_may_consume_its_own_pending_account_grant_on_the_same_request() {
    let _serial = TEST_LOCK.lock().unwrap();
    let mut s = table();
    let t = issue(&s, true).unwrap();
    assert!(
        !CAP_BROKER.lock().account_grants[0]
            .1
            .delivery
            .unwrap()
            .delivered
    );
    assert_eq!(s.account_grant(7, scope(), Some(t)), Some(t));
    assert_eq!(inventory(), 0);
    let stale = issue(&s, true).unwrap();
    s.processes[1].pending_call = Some((7, 2));
    s.processes[0].ipc_reply_target = Some(reply(7, 2, 7));
    assert!(s.account_grant(7, scope(), Some(stale)).is_none());
    assert_eq!(inventory(), 0);
}

#[path = "production_tests.rs"]
mod production_tests;
