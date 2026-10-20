use super::*;
use crate::lifecycle_host::{AddressSpace, Foreground, Process, Scheduler};
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
        exiting: false,
        foreground: Foreground {
            incarnation: pid as u64,
        },
        address_space: AddressSpace {
            generation: pid as u64 + 10,
        },
        pending_call: (pid != 6).then_some((pid, 1)),
        ipc_reply_target: (pid == 6).then_some((7, 1)),
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
        s.processes[0].ipc_reply_target = Some((8, 1));
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
                5 => s.processes[0].ipc_reply_target = Some((7, 2)),
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
        s.processes[0].ipc_reply_target = Some((7, 2));
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
    s.processes[0].ipc_reply_target = Some((7, 2));
    let fresh = issue(&s, account).unwrap();
    assert_ne!(fresh, token);
    assert_eq!(inventory(), 1);
    if account {
        assert_eq!(
            CAP_BROKER.lock().account_grants[0]
                .1
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
