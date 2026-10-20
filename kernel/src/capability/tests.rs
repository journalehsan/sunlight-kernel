use super::*;

fn vfs() -> VfsCapability {
    let mut allowed_prefix = String::new();
    allowed_prefix.push_str("/bin").unwrap();
    VfsCapability {
        allowed_prefix,
        flags: AccessFlags::READ_EXECUTE,
    }
}

#[test]
fn entropy_failure_publishes_nothing_and_preserves_endpoint_id() {
    let mut b = CapabilityBroker::new();
    b.test_entropy(&[None]);
    assert_eq!(b.create_endpoint(7), Err(CapError::EntropyUnavailable));
    assert!(b.endpoints.is_empty() && b.capabilities.is_empty());
    assert_eq!(b.next_endpoint_id, 0);
    b.test_entropy(&[Some(5)]);
    assert_eq!(b.create_endpoint(7).unwrap().0, 0);
}

#[test]
fn collision_retry_does_not_replace_existing_authority() {
    let mut b = CapabilityBroker::new();
    b.test_entropy(&[Some(7), Some(7), Some(9)]);
    let (a, token) = b.create_endpoint(7).unwrap();
    let (c, other) = b.create_endpoint(8).unwrap();
    assert_ne!(token, other);
    assert_eq!(b.check(token, CapabilityRights::SEND), Ok(a));
    assert_eq!(b.check(other, CapabilityRights::SEND), Ok(c));
    assert_eq!(b.issuance_stats().collisions, 1);
}

#[test]
fn stuck_source_and_zero_payload_fail_boundedly() {
    let mut b = CapabilityBroker::new();
    b.test_entropy(&[Some(0); TOKEN_ATTEMPTS + 1]);
    assert_eq!(b.create_endpoint(1), Err(CapError::TokenCollisionLimit));
    assert_eq!(b.test_words.as_ref().unwrap().len(), 1);
    assert_eq!(b.active_tokens(), 0);
    assert_eq!(b.issuance_stats().collisions, TOKEN_ATTEMPTS as u64);
    b.test_entropy(&[Some(3)]);
    let (_, original) = b.create_endpoint(1).unwrap();
    b.test_entropy(&[Some(3); TOKEN_ATTEMPTS]);
    assert_eq!(b.create_endpoint(2), Err(CapError::TokenCollisionLimit));
    assert_eq!(b.check(original, CapabilityRights::RECV_ONLY), Ok(0));
    assert_eq!(b.endpoints.len(), 1);
}

#[test]
fn reserved_selectors_never_become_tokens() {
    let mut b = CapabilityBroker::new();
    b.test_entropy(&[Some(SPAWN_TOKEN.as_u64()), Some(14)]);
    let token = b.grant_vfs_capability(7, vfs()).unwrap();
    // SPAWN_TOKEN has a different tag; exclude its payload in every domain
    // below as well as checking the full reserved word.
    assert_ne!(token, SPAWN_TOKEN);
    assert_ne!(token, CapabilityToken::INVALID);
    assert_eq!(b.issuance_stats().collisions, 1);
}

#[test]
fn cross_domain_collision_including_retired_shm_is_rejected() {
    let mut b = CapabilityBroker::new();
    b.test_entropy(&[Some(7)]);
    let shm = b.mint_shared_page(PhysAddr::new(0x1000), 1).unwrap();
    b.increment_map_count(shm);
    assert!(b.revoke_shared(shm).is_none());
    b.test_entropy(&[Some(7), Some(8)]);
    let cap = b.grant_vfs_capability(2, vfs()).unwrap();
    assert_ne!(shm, cap);
    assert_eq!(b.issuance_stats().collisions, 1);
    b.test_entropy(&[Some(9)]);
    let auth = b.mint_auth_session_grant(1, 0, 0, 10).unwrap();
    b.test_entropy(&[Some(9), Some(10)]);
    let (_, ep) = b.create_endpoint(2).unwrap();
    assert_ne!(auth, ep);
    assert_eq!(b.issuance_stats().collisions, 2);
}

#[test]
fn reservation_and_endpoint_exhaustion_do_not_consume_randomness() {
    let mut b = CapabilityBroker::new();
    b.test_entropy(&[Some(5)]);
    b.test_fail_reservation(true);
    assert_eq!(b.create_endpoint(7), Err(CapError::CapabilityStoreFull));
    assert_eq!(
        b.mint_shared_page(PhysAddr::new(0x1000), 7),
        Err(CapError::CapabilityStoreFull)
    );
    assert_eq!(
        b.grant_vfs_capability(7, vfs()),
        Err(CapError::CapabilityStoreFull)
    );
    assert_eq!(b.active_tokens(), 0);
    assert_eq!(b.test_words.as_ref().unwrap().len(), 1);
    b.test_fail_reservation(false);
    b.next_endpoint_id = u32::MAX;
    assert_eq!(b.create_endpoint(7), Err(CapError::EndpointIdExhausted));
    assert_eq!(b.test_words.as_ref().unwrap().len(), 1);
}

#[test]
fn vfs_batch_rollback_preserves_only_preexisting_authority() {
    let mut b = CapabilityBroker::new();
    b.test_entropy(&[Some(10), Some(11), None]);
    let existing = b.grant_vfs_capability(1, vfs()).unwrap();
    assert_eq!(b.grant_elevated_vfs(2), Err(CapError::EntropyUnavailable));
    assert_eq!(b.vfs_caps.len(), 1);
    assert_eq!(b.vfs_caps[0].0, existing);
}

#[test]
fn preparation_failure_never_takes_shm_frame_ownership() {
    let mut b = CapabilityBroker::new();
    b.test_entropy(&[None]);
    assert_eq!(
        b.mint_shared_page(PhysAddr::new(0x1000), 1),
        Err(CapError::EntropyUnavailable)
    );
    assert_eq!(b.shared_region_count(), 0);
}

#[test]
fn rights_attenuation_type_and_owner_checks_survive_secure_issuance() {
    let mut b = CapabilityBroker::new();
    let (ep, owner) = b.create_endpoint(7).unwrap();
    let public = b.derive(owner, CapabilityRights::SEND).unwrap();
    assert_eq!(b.derive(owner, CapabilityRights::SEND), Ok(public));
    assert_eq!(
        b.derive(public, CapabilityRights::SEND_RECV),
        Err(CapError::InsufficientRights)
    );
    assert_eq!(
        b.check(public, CapabilityRights::RECV_ONLY),
        Err(CapError::InsufficientRights)
    );
    assert_eq!(
        b.destroy_endpoint(8, owner),
        Err(CapError::InsufficientRights)
    );
    assert_eq!(
        b.destroy_endpoint(7, public),
        Err(CapError::InsufficientRights)
    );
    assert_eq!(
        b.check(CapabilityToken::INVALID, CapabilityRights::SEND),
        Err(CapError::InvalidToken)
    );
    assert_eq!(
        b.check(CapabilityToken(u64::MAX), CapabilityRights::SEND),
        Err(CapError::InvalidToken)
    );
    let wrong_type = CapabilityToken::tagged(owner.as_u64(), CapabilityToken::TAG_VFS);
    assert_eq!(
        b.check(wrong_type, CapabilityRights::SEND),
        Err(CapError::InvalidToken)
    );
    assert_eq!(
        b.check(owner, CapabilityRights::VFS_ALL),
        Err(CapError::InsufficientRights)
    );
    assert_eq!(b.destroy_endpoint(7, owner), Ok(ep));
    assert_eq!(
        b.check(public, CapabilityRights::SEND),
        Err(CapError::InvalidToken)
    );
}

#[test]
fn adjacent_prediction_is_rejected_by_actual_broker() {
    let mut b = CapabilityBroker::new();
    for _ in 0..128 {
        let (private, secret) = b.create_endpoint(7).unwrap();
        let (own, attacker) = b.create_endpoint(8).unwrap();
        for bits in 1..=62 {
            let guess = CapabilityToken(attacker.as_u64() ^ ((1u64 << bits) - 1));
            assert_ne!(b.check(guess, CapabilityRights::SEND), Ok(private));
        }
        b.destroy_endpoint(7, secret).unwrap();
        assert!(b.check(secret, CapabilityRights::SEND).is_err());
        b.destroy_endpoint(8, attacker).unwrap();
        assert_ne!(private, own);
    }
}

#[test]
fn exit_revokes_endpoint_and_bound_grants_preserving_transferable_vfs() {
    let mut b = CapabilityBroker::new();
    let (_, owner) = b.create_endpoint(7).unwrap();
    let public = b.derive(owner, CapabilityRights::SEND).unwrap();
    let vfs = b.grant_vfs_capability(7, vfs()).unwrap();
    let auth = b.mint_auth_session_grant(7, 0, 0, 100).unwrap();
    b.revoke_endpoints_owned_by(7);
    assert!(b.check(owner, CapabilityRights::SEND).is_err());
    assert!(b.check(public, CapabilityRights::SEND).is_err());
    assert!(b.resolve_vfs_capability(vfs).is_some());
    assert!(b.consume_auth_session_grant(auth, 7, 10).is_none());
    let (_, new) = b.create_endpoint(7).unwrap();
    assert_ne!(new, owner);
    assert_eq!(b.active_tokens(), 2);
}

#[test]
fn concurrent_mint_revoke_has_no_duplicate_live_token_or_slot_drift() {
    use std::sync::Arc;
    let broker = Arc::new(spin::Mutex::new(CapabilityBroker::new()));
    let mut threads = std::vec::Vec::new();
    for pid in 1..=8 {
        let broker = broker.clone();
        threads.push(std::thread::spawn(move || {
            for _ in 0..128 {
                let token = broker.lock().create_endpoint(pid).unwrap().1;
                let send = broker.lock().derive(token, CapabilityRights::SEND).unwrap();
                assert_eq!(
                    broker.lock().check(send, CapabilityRights::RECV_ONLY),
                    Err(CapError::InsufficientRights)
                );
                broker.lock().destroy_endpoint(pid, token).unwrap();
                assert!(broker.lock().check(send, CapabilityRights::SEND).is_err());
            }
        }));
    }
    for thread in threads {
        thread.join().unwrap();
    }
    assert_eq!(broker.lock().active_tokens(), 0);
    assert_eq!(broker.lock().endpoints.len(), 0);
}

#[test]
fn global_active_limit_is_explicit_and_recovers_after_revoke() {
    let mut b = CapabilityBroker::new();
    let (ep, owner) = b.create_endpoint(1).unwrap();
    for _ in 1..MAX_ACTIVE_TOKENS {
        b.mint(ep, CapabilityRights::SEND).unwrap();
    }
    assert_eq!(
        b.mint(ep, CapabilityRights::SEND),
        Err(CapError::CapabilityStoreFull)
    );
    b.destroy_endpoint(1, owner).unwrap();
    assert_eq!(b.active_tokens(), 0);
    assert!(b.create_endpoint(1).is_ok());
}

#[test]
fn account_and_auth_grants_share_collision_domain_and_remain_single_use() {
    use sunlight_ipc::accounts::{GrantBinding, GrantScope, Operation};
    let binding = GrantBinding {
        pid: 7,
        process_generation: 19,
        scope: GrantScope {
            revision: 1,
            operation: Operation::GroupCreate,
            target: 0,
            member: 0,
            session: 3,
            session_generation: 4,
        },
    };
    let mut b = CapabilityBroker::new();
    b.test_entropy(&[Some(12), Some(12), Some(13)]);
    let account = b.mint_account_grant(binding, 1).unwrap();
    let auth = b.mint_auth_session_grant(7, 0, 0, 100).unwrap();
    assert_ne!(account, auth);
    assert_eq!(b.issuance_stats().collisions, 1);
    let mut stale = binding;
    stale.process_generation += 1;
    assert!(!b.consume_account_grant(account, stale, 2));
    assert!(!b.consume_account_grant(account, binding, 2));
    assert_eq!(b.consume_auth_session_grant(auth, 8, 2), None);
    assert_eq!(b.consume_auth_session_grant(auth, 7, 2), None);
}

#[test]
fn trusted_vfs_issuer_check_preserves_full_kernel_pid() {
    let forged_role = (1usize << 32) | CAPABILITY_BROKER_PID as usize;
    assert_eq!(
        sys_grant_capability(forged_role, vfs()),
        Err(CapError::InvalidCaller)
    );
}
