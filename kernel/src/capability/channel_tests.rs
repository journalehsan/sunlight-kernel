//! Real broker methods with synthetic host identities. These supplement,
//! and do not claim, native userspace authentication.
use super::*;
use crate::capability::GrantDelivery;
use sunlight_ipc::account_session::{ACTIVATE, REVOKE, SUSPEND};
fn id(pid: usize) -> RecipientIdentity {
    RecipientIdentity {
        pid,
        incarnation: pid as u64 + 100,
        vm_generation: pid as u64 + 200,
    }
}
fn setup() -> (CapabilityBroker, SessionRef) {
    let mut b = CapabilityBroker::new();
    let login = b.mint_auth_session_grant(7, 0, 0, 100).unwrap();
    b.auth_session_grants.last_mut().unwrap().delivery = Some(GrantDelivery {
        recipient: id(7),
        issuer_pid: 6,
        issuer_incarnation: id(6).incarnation,
        issuer_vm_generation: id(6).vm_generation,
        request: 1,
        delivered: true,
    });
    let delivery = GrantDelivery {
        recipient: id(7),
        issuer_pid: 8,
        issuer_incarnation: id(8).incarnation,
        issuer_vm_generation: id(8).vm_generation,
        request: 2,
        delivered: false,
    };
    let (s, _, _) = b
        .register_account_session(id(8), id(6), delivery, login, 1, 10000)
        .unwrap();
    b.transition_account_session(id(8), s, ACTIVATE, 1).unwrap();
    (b, s)
}
fn pair(b: &mut CapabilityBroker, s: SessionRef) -> Handle {
    let h = b.channel_propose(id(6), id(10), s, 0, 2, 100).unwrap();
    let receipt = b.channel_consent(h, id(6), id(11), s, 0, 2).unwrap();
    b.channel_create(h, id(10), receipt, 2).unwrap();
    h
}

#[test]
fn hybrid_reply_revocation_and_completion_share_channel_lifetime() {
    use crate::process::{IpcCallId, IpcCallOutcome, IpcReplyTarget};
    use super::super::channel_contract::{Phase, Service};
    for completion_first in [false, true] {
        let (mut b, s) = setup();
        let h = pair(&mut b, s);
        let target = IpcReplyTarget { endpoint_id: 12, call: IpcCallId {
            pid: 10, incarnation: id(10).incarnation, generation: 42,
        }};
        let policy = b.channels.policies.for_service(id(11), Service::RequiredStream).unwrap();
        let c = &mut b.channels.row_mut(h).unwrap().contract;
        c.begin(Service::RequiredStream, 0x10202, false).unwrap();
        c.finish().unwrap();
        c.policy_identity = Some(policy);
        c.hybrid_started = true;
        c.hybrid_reply = Some(target);
        assert!(b.hybrid_reply_allowed(target));
        if completion_first {
            b.hybrid_reply_finished(target, IpcCallOutcome::ReplyDelivered(42));
        }
        b.channel_revoke(h, id(10)).unwrap();
        if !completion_first {
            assert!(!b.hybrid_reply_allowed(target));
            // A different call generation is not accidentally channel-bound.
            let unrelated = IpcReplyTarget { call: IpcCallId { generation: 43, ..target.call }, ..target };
            assert!(b.hybrid_reply_allowed(unrelated));
            // Undrained reply metadata prevents channel-row reuse.
            let next = b.channel_propose(id(6), id(12), s, 0, 3, 100).unwrap();
            assert_eq!(b.channels.rows.len(), 2);
            b.channel_revoke(next, id(12)).unwrap();
            b.hybrid_reply_finished(target, IpcCallOutcome::PeerClosed(42));
        }
        let c = b.channels.row_mut(h).unwrap().contract;
        assert!(c.hybrid_started);
        assert!(c.hybrid_reply.is_none());
        assert_eq!(c.phase, Phase::Closed);
    }
}

#[test]
fn hybrid_policy_reuse_cannot_restore_original_reply() {
    use super::super::channel_contract::Service;
    use crate::process::{IpcCallId, IpcCallOutcome, IpcReplyTarget};
    let (mut b, s) = setup();
    let h = pair(&mut b, s);
    let old = b.channels.policies.for_service(id(11), Service::RequiredStream).unwrap();
    let target = IpcReplyTarget { endpoint_id: 12, call: IpcCallId {
        pid: 10, incarnation: id(10).incarnation, generation: 42,
    }};
    let c = &mut b.channels.row_mut(h).unwrap().contract;
    c.begin(Service::RequiredStream, 0x10202, false).unwrap();
    c.finish().unwrap();
    c.policy_identity = Some(old);
    c.hybrid_started = true;
    c.hybrid_reply = Some(target);
    b.channels.policies.retire_process(11);
    let new = b.channels.policies.for_service(id(11), Service::RequiredStream).unwrap();
    assert_eq!(old.policy_id, new.policy_id);
    assert_ne!(old.generation, new.generation);
    assert!(!b.hybrid_reply_allowed(target));
    b.hybrid_reply_finished(target, IpcCallOutcome::ExplicitlyCancelled(42));
    assert!(!b.channels.row_mut(h).unwrap().live());
}

#[test]
fn authenticated_bell_roles_ordering_terminal_and_reuse() {
    let (mut b, s) = setup();
    for cycle in 0..1000 {
        let h = pair(&mut b, s);
        assert_eq!(b.channel_bell(h, id(10), 10, 0), Err(Error::Incomplete));
        b.channel_attach(h, id(10), 0, 2).unwrap();
        b.channel_attach(h, id(11), 1, 2).unwrap();
        for (op, caller) in [(10, id(11)), (11, id(10)), (12, id(10)), (14, id(11))] {
            assert_eq!(b.channel_bell(h, caller, op, 0), Err(Error::Denied));
        }
        for op in [10, 11, 12, 14] {
            for who in [id(12), RecipientIdentity { incarnation: 999, ..id(10) }, RecipientIdentity { vm_generation: 999, ..id(11) }] {
                assert_eq!(b.channel_bell(h, who, op, 0), Err(Error::Denied));
            }
        }
        let stale = Handle { generation: h.generation + 1, ..h };
        assert_eq!(b.channel_bell(stale, id(10), 10, 0), Err(Error::Stale));
        assert_eq!(b.channel_bell(h, id(10), 10, 0), Ok((1, None)));
        assert_eq!(b.channel_bell(h, id(11), 12, 0), Err(Error::Changed));
        b.channel_bell(h, id(11), 12, 1).unwrap();
        assert_eq!(b.channel_bell(h, id(10), 14, 0), Ok((0, Some(super::super::channel_doorbell::identity(id(11))))));
        assert_eq!(b.channel_bell(h, id(10), 14, 0), Ok((0, None)));
        b.channel_bell(h, id(11), 12, 1).unwrap();
        b.channel_revoke(h, id(10)).unwrap();
        assert_eq!(b.channel_bell(h, id(11), 12, 1), Err(Error::Terminal));
        let row = b.channels.row_mut(h).unwrap();
        assert!(!row.bell.as_ref().unwrap().has_waiter());
        assert_eq!(row.bell_pending.take(), Some(super::super::channel_doorbell::identity(id(11))));
        assert_eq!(b.channels.live_count(), 0);
        assert_eq!(b.channels.rows.len(), 1);
        assert_eq!(h.generation, cycle + 1);
    }
}

#[test]
fn bell_exhaustion_never_wraps_and_retirement_preserves_wake() {
    let (mut b, s) = setup();
    let h = pair(&mut b, s);
    b.channel_attach(h, id(10), 0, 2).unwrap();
    b.channel_attach(h, id(11), 1, 2).unwrap();
    b.channel_bell(h, id(11), 11, 0).unwrap();
    b.channels.row_mut(h).unwrap().bell.as_mut().unwrap().fixture_sequence(u64::MAX - 1);
    assert_eq!(b.channel_bell(h, id(10), 10, 0), Ok((u64::MAX, None)));
    b.channel_bell(h, id(11), 12, u64::MAX).unwrap();
    assert_eq!(b.channel_bell(h, id(10), 10, 0).unwrap().1, Some(super::super::channel_doorbell::identity(id(11))));
    assert_eq!(b.channel_bell(h, id(11), 11, 0), Err(Error::Exhausted));
    assert_eq!(b.channel_bell(h, id(10), 10, 0), Err(Error::Exhausted));
    assert!(!b.channels.row_mut(h).unwrap().bell.as_ref().unwrap().has_waiter());
}
#[test]
fn independent_roles_are_recipient_bound_and_once_only() {
    let (mut b, s) = setup();
    let h = pair(&mut b, s);
    assert_eq!(
        b.channel_create(h, id(10), CapabilityToken::INVALID, 2),
        Err(Error::Duplicate)
    );
    for (who, role) in [
        (id(11), 0),
        (id(10), 1),
        (id(12), 0),
        (id(12), 1),
        (id(10), 2),
    ] {
        assert_eq!(b.channel_attach(h, who, role, 2), Err(Error::Denied));
    }
    for role in 0..2 {
        let who = id(10 + role as usize);
        for stale in [
            RecipientIdentity {
                incarnation: who.incarnation + 1,
                ..who
            },
            RecipientIdentity {
                vm_generation: who.vm_generation + 1,
                ..who
            },
        ] {
            assert_eq!(b.channel_attach(h, stale, role, 2), Err(Error::Denied));
        }
        assert_eq!(b.channel_attach(h, who, role, 2), Ok(()));
        assert_eq!(b.channel_attach(h, who, role, 2), Err(Error::Duplicate));
    }
    assert_eq!(b.channels.rows[0].state, State::Attached);
    assert!(!h.token.is_ipc());
    assert!(!h.token.is_vfs());
    assert!(b.channels.vm_member(id(11).vm_generation));
    assert_eq!(
        b.channel_status(h, id(10), 2).unwrap() & 0xff,
        State::Attached as u64
    );
}
#[test]
fn pair_creation_rejects_foreign_issuer_session_uid_and_reassignment() {
    let (mut b, s) = setup();
    for (issuer, uid, session) in [
        (id(5), 0, s),
        (id(6), 1, s),
        (
            id(6),
            0,
            SessionRef {
                epoch: s.epoch + 1,
                ..s
            },
        ),
    ] {
        assert_eq!(
            b.channel_propose(issuer, id(10), session, uid, 2, 100),
            Err(Error::Denied)
        );
    }
    let h = b.channel_propose(id(6), id(10), s, 0, 2, 100).unwrap();
    assert_eq!(
        b.channel_create(h, id(10), CapabilityToken::INVALID, 2),
        Err(Error::Incomplete)
    );
    assert_eq!(b.channel_attach(h, id(10), 0, 2), Err(Error::Incomplete));
    assert_eq!(
        b.channel_create(h, id(11), CapabilityToken::INVALID, 2),
        Err(Error::Denied)
    );
    assert_eq!(
        b.channel_propose(id(6), id(10), s, 0, 2, 100),
        Err(Error::Duplicate)
    );
    b.channel_consent(h, id(6), id(11), s, 0, 2).unwrap();
    assert_eq!(
        b.channel_consent(h, id(6), id(12), s, 0, 2),
        Err(Error::Duplicate)
    );
    assert_eq!(b.channels.rows[0].consumer, Some(id(11)));
}
#[test]
fn first_terminal_wins_exit_exec_revocation_and_expiry() {
    for mode in 0..8 {
        let (mut b, s) = setup();
        let h = pair(&mut b, s);
        match mode {
            0 => {
                b.channel_revoke(h, id(10)).unwrap();
            }
            1..=3 => b.retire_recipient_grants([10, 11, 6][mode - 1]),
            4 => {
                b.transition_account_session(id(8), s, REVOKE, 3).unwrap();
            }
            5 => {
                b.transition_account_session(id(8), s, SUSPEND, 3).unwrap();
            }
            6 => b.channels.expire(102),
            _ => b.expire_sessions(10000),
        }
        let reason = b.channels.rows[0].terminal;
        assert!(reason.is_some());
        assert_eq!(b.channel_attach(h, id(11), 1, 3), Err(Error::Terminal));
        b.channel_revoke(h, id(10)).unwrap();
        b.retire_recipient_grants(11);
        assert_eq!(b.channels.rows[0].terminal, reason);
        assert_eq!(b.channels.live_count(), 0);
    }
}
#[test]
fn slot_reuse_never_adopts_old_handle_and_incomplete_expires() {
    let (mut b, s) = setup();
    let old = pair(&mut b, s);
    b.channel_revoke(old, id(10)).unwrap();
    let new = pair(&mut b, s);
    assert_ne!(old, new);
    assert_ne!(old.generation, new.generation);
    assert_eq!(b.channels.rows.len(), 1);
    assert_eq!(b.channel_attach(old, id(10), 0, 2), Err(Error::Stale));
    let forged = Handle {
        token: new.token,
        generation: old.generation,
    };
    assert_eq!(b.channel_attach(forged, id(10), 0, 2), Err(Error::Stale));
    b.channel_revoke(new, id(10)).unwrap();
    let abandoned = b.channel_propose(id(6), id(10), s, 0, 2, 5).unwrap();
    assert_eq!(
        b.channel_create(abandoned, id(10), CapabilityToken::INVALID, 7),
        Err(Error::Terminal)
    );
}
#[test]
fn bounded_metadata_and_failure_atomicity() {
    let (mut b, s) = setup();
    b.test_fail_reservation(true);
    assert_eq!(
        b.channel_propose(id(6), id(10), s, 0, 2, 100),
        Err(Error::Capacity)
    );
    assert_eq!(b.channels.rows.len(), 0);
    b.test_fail_reservation(false);
    b.test_entropy(&[None]);
    assert_eq!(
        b.channel_propose(id(6), id(10), s, 0, 2, 100),
        Err(Error::Capacity)
    );
    assert_eq!(b.channels.live_count(), 0);
    for n in 0..LIMIT {
        b.test_entropy(&[Some(5000 + n as u64)]);
        b.channel_propose(id(6), id(20 + n), s, 0, 2, 100).unwrap();
    }
    let prepared = b.issuance_stats().prepared;
    assert_eq!(
        b.channel_propose(id(6), id(100), s, 0, 2, 100),
        Err(Error::Capacity)
    );
    assert_eq!(b.issuance_stats().prepared, prepared);
    assert_eq!(b.channels.rows.len(), LIMIT);
    assert_eq!(b.channels.rows.capacity(), LIMIT);
}
#[test]
fn generation_exhaustion_never_wraps_or_refunds_identity() {
    let (mut b, s) = setup();
    b.channels.next_generation = u64::MAX - 1;
    let h = pair(&mut b, s);
    assert_eq!(h.generation, u64::MAX - 1);
    b.channel_revoke(h, id(10)).unwrap();
    let prepared = b.issuance_stats().prepared;
    for _ in 0..2 {
        assert_eq!(
            b.channel_propose(id(6), id(10), s, 0, 2, 100),
            Err(Error::Exhausted)
        );
    }
    assert_eq!(b.issuance_stats().prepared, prepared);
    assert_eq!(b.channels.next_generation, u64::MAX);
}
#[test]
fn attach_revocation_races_linearize_under_existing_broker_guard() {
    use std::sync::{Arc, Barrier, Mutex};
    for _ in 0..128 {
        let (mut b, s) = setup();
        let h = pair(&mut b, s);
        let broker = Arc::new(Mutex::new(b));
        let barrier = Arc::new(Barrier::new(2));
        let other = broker.clone();
        let ready = barrier.clone();
        let worker = std::thread::spawn(move || {
            ready.wait();
            other.lock().unwrap().channel_attach(h, id(11), 1, 2)
        });
        barrier.wait();
        broker.lock().unwrap().channel_revoke(h, id(10)).unwrap();
        assert!(matches!(
            worker.join().unwrap(),
            Ok(()) | Err(Error::Terminal)
        ));
        let mut b = broker.lock().unwrap();
        assert_eq!(b.channels.live_count(), 0);
        assert_eq!(b.channel_attach(h, id(11), 1, 2), Err(Error::Terminal));
        assert_eq!(b.channels.rows[0].terminal, Some(Terminal::Revoked));
    }
}

#[test]
fn pair_confirmation_receipt_is_exact_one_use_and_failure_atomic() {
    let (mut b, s) = setup();
    let h = b.channel_propose(id(6), id(10), s, 0, 2, 100).unwrap();
    b.test_fail_reservation(true);
    assert_eq!(
        b.channel_consent(h, id(6), id(11), s, 0, 2),
        Err(Error::Capacity)
    );
    assert_eq!(b.channels.rows[0].consumer, None);
    assert_eq!(b.channels.rows[0].state, State::Proposed);
    b.test_fail_reservation(false);
    let receipt = b.channel_consent(h, id(6), id(11), s, 0, 2).unwrap();
    assert_eq!(b.channels.token_count(), 2);
    // A leaked proposal, guessed receipt or IPC/VFS bearer cannot finalize.
    for wrong in [
        h.token,
        CapabilityToken::INVALID,
        CapabilityToken(receipt.0 ^ 1),
    ] {
        assert_eq!(b.channel_create(h, id(10), wrong, 2), Err(Error::Denied));
    }
    assert_eq!(b.channel_create(h, id(12), receipt, 2), Err(Error::Denied));
    b.channel_create(h, id(10), receipt, 2).unwrap();
    assert_eq!(b.channels.token_count(), 1);
    assert_eq!(
        b.channel_create(h, id(10), receipt, 2),
        Err(Error::Duplicate)
    );
}

#[test]
fn channel_tag_preserves_reserved_spawn_marker_and_class_isolation() {
    use crate::capability::{CapKind, SPAWN_TOKEN};
    assert_eq!(SPAWN_TOKEN.kind(), CapKind::Untagged);
    assert!(SPAWN_TOKEN.is_ipc() && SPAWN_TOKEN.is_vfs());
    let (mut b, s) = setup();
    let h = pair(&mut b, s);
    assert_eq!(h.token.kind(), CapKind::FastLane);
    assert!(!h.token.is_ipc() && !h.token.is_vfs());
    assert_ne!(h.token, SPAWN_TOKEN);
}

#[test]
fn mapping_requires_exact_attached_role_and_asymmetric_region_rights() {
    let (mut b, s) = setup();
    let h = pair(&mut b, s);
    assert_eq!(b.channel_mapping_authorize(h, id(10), 0), Err(Error::Incomplete));
    b.channel_attach(h, id(10), 0, 2).unwrap();
    b.channel_attach(h, id(11), 1, 2).unwrap();
    for role in 0..2 {
        for region in 0..3 {
            let arg = (role * 3 + region) as u64;
            let who = id(10 + role);
            assert_eq!(b.channel_mapping_authorize(h, who, arg), Ok((role, region, if region == 1 {role == 1} else {role == 0})));
            for bad in [id(12), id(11 - role), RecipientIdentity {vm_generation: who.vm_generation+1, ..who}, RecipientIdentity {incarnation: who.incarnation+1, ..who}] {
                assert_eq!(b.channel_mapping_authorize(h, bad, arg), Err(Error::Denied));
            }
            b.channels.row_mut(h).unwrap().mapped[role][region] = true;
            assert_eq!(b.channel_mapping_authorize(h, who, arg), Err(Error::Duplicate));
        }
    }
    assert_eq!(b.channel_mapping_authorize(h, id(10), 6), Err(Error::Denied));
    b.channel_revoke(h, id(10)).unwrap();
    assert_eq!(b.channel_mapping_authorize(h, id(10), 0), Err(Error::Terminal));
    let new = pair(&mut b, s);
    assert_ne!(h.generation, new.generation);
    assert_eq!(b.channel_mapping_authorize(h, id(10), 0), Err(Error::Stale));
}

#[test]
fn undrained_backing_prevents_slot_reuse_and_vm_borrowers() {
    let (mut b, s) = setup();
    let h = pair(&mut b, s);
    b.channels.row_mut(h).unwrap().backing[0] = Some(CapabilityToken(123));
    b.channel_revoke(h, id(10)).unwrap();
    assert!(b.channels.vm_member(id(10).vm_generation));
    let fresh = pair(&mut b, s);
    assert_ne!(h.generation, fresh.generation);
    assert_eq!(b.channels.rows.len(), 2);
    assert_eq!(b.channel_status(h, id(10), 2).unwrap() & 255, State::Retired as u64);
}

#[test]
fn restricted_shm_is_not_a_legacy_bearer_even_with_the_exact_token() {
    let mut b = CapabilityBroker::new();
    let token = b.mint_shared_page(x86_64::PhysAddr::new(0x1000), 10).unwrap();
    b.shared_regions.iter_mut().find(|e| e.token == token).unwrap().restricted = true;
    assert!(b.restricted_shared(token));
    assert!(b.resolve_shared_region(token).is_none());
    assert!(b.resolve_shared_page(token).is_none());
    assert_eq!(b.validate_shared_page(token), Err(super::super::CapError::InsufficientRights));
    assert!(b.shared_region_for_cleanup(token).is_some());
}
