use super::*;
use sunlight_fs::{mode, FileSystem, Tmpfs, TmpfsLimits};
use sunlight_ipc::vfs_rpc::{lifecycle::Caller, registration::EndpointOwner};

fn registration() -> Registration {
    Registration {
        owner: EndpointOwner {
            endpoint: 0,
            pid: 2,
            incarnation: 3,
            address_space_generation: 4,
        },
        epoch: 5,
    }
}

fn scope() -> Scope {
    Scope {
        caller: Caller {
            pid: 6,
            incarnation: 7,
            address_space_generation: 8,
        },
        namespace_id: 9,
        namespace_generation: 10,
    }
}

fn namespace() -> DirectoryNamespace {
    let mut fs = Tmpfs::with_limits(&[], TmpfsLimits::new(0, 4)).unwrap();
    fs.mkdir("/root", 1, 2, mode::DIR_755).unwrap();
    fs.mkdir("/root/cwd", 3, 4, mode::DIR_700).unwrap();
    DirectoryNamespace::new(fs).unwrap()
}

fn retain(scope: Scope, id: u64) -> [u8; wire::FRAME_BYTES] {
    wire::PendingRetain::new(registration(), scope, id)
        .unwrap()
        .request()
        .encode()
        .unwrap()
}

#[test]
fn exact_scope_and_registration_are_required_before_policy_resolution() {
    let registration = registration();
    let scope = scope();
    let mut policy = Policy::<1, 2>::new(registration).unwrap();
    policy
        .install(scope, &namespace(), "/root", "/root/cwd")
        .unwrap();
    for field in 0..6 {
        let mut wrong = registration;
        match field {
            0 => wrong.owner.endpoint += 1,
            1 => wrong.owner.pid += 1,
            2 => wrong.owner.incarnation += 1,
            3 => wrong.owner.address_space_generation += 1,
            4 => wrong.epoch += 1,
            _ => wrong.owner.pid = 0,
        }
        assert_eq!(
            policy.request(wrong, &retain(scope, 1)),
            Err(Error::Identity)
        );
        let frame = wire::PendingRetain::new(wrong, scope, 1);
        if let Ok(frame) = frame {
            assert_eq!(
                policy.request(registration, &frame.request().encode().unwrap()),
                Err(Error::Retention(retention::Error::Identity))
            );
        }
    }
    for field in 0..5 {
        let mut wrong = scope;
        match field {
            0 => wrong.caller.pid += 1,
            1 => wrong.caller.incarnation += 1,
            2 => wrong.caller.address_space_generation += 1,
            3 => wrong.namespace_id += 1,
            _ => wrong.namespace_generation += 1,
        }
        assert_eq!(policy.withdraw(wrong), Err(Error::Scope));
        assert!(matches!(
            policy
                .request(registration, &retain(wrong, field + 1))
                .unwrap(),
            wire::Frame::Refused {
                reason: wire::Refusal::Resolve,
                ..
            }
        ));
    }
    // Refusals burn request IDs, but failed authentication did not burn ID 1.
    assert_eq!(
        policy.request(registration, &retain(scope, 1)),
        Err(Error::Retention(retention::Error::Replay))
    );
    let wire::Frame::Retained(receipt) = policy.request(registration, &retain(scope, 6)).unwrap()
    else {
        panic!()
    };
    assert_eq!(policy.metadata(registration, receipt).unwrap()[1].1.uid, 3);
    for bytes in [&[][..], &retain(scope, 7)[..111]] {
        assert_eq!(
            policy.request(registration, bytes),
            Err(Error::Protocol(ProtocolError::Encoding))
        );
    }
    assert_eq!(
        policy.request(
            registration,
            &wire::Frame::Retained(receipt).encode().unwrap()
        ),
        Err(Error::Protocol(ProtocolError::Operation))
    );
    assert_eq!(
        policy.request(registration, &retain(scope, 6)),
        Err(Error::Retention(retention::Error::Replay))
    );
}

#[test]
fn withdrawal_replacement_and_lost_acknowledgements_preserve_exact_pins() {
    let registration = registration();
    let scope = scope();
    let mut policy = Policy::<1, 2>::new(registration).unwrap();
    policy
        .install(scope, &namespace(), "/root", "/root/cwd")
        .unwrap();
    let old = Arc::downgrade(&policy.bindings[0].as_ref().unwrap().pair);
    let wire::Frame::Retained(receipt) = policy.request(registration, &retain(scope, 1)).unwrap()
    else {
        panic!()
    };
    policy.withdraw(scope).unwrap();
    assert!(old.upgrade().is_some());
    assert_eq!(
        policy.install(scope, &namespace(), "/root", "/root/cwd"),
        Err(Error::Generation)
    );
    assert!(matches!(
        policy.request(registration, &retain(scope, 2)).unwrap(),
        wire::Frame::Refused {
            reason: wire::Refusal::Resolve,
            ..
        }
    ));
    let mut replacement = scope;
    replacement.namespace_generation += 1;
    policy
        .install(replacement, &namespace(), "/root", "/root/cwd")
        .unwrap();
    assert_eq!(policy.withdraw(scope), Err(Error::Scope));
    let wire::Frame::Retained(new_receipt) = policy
        .request(registration, &retain(replacement, 3))
        .unwrap()
    else {
        panic!()
    };
    assert!(matches!(
        policy
            .request(registration, &retain(replacement, 4))
            .unwrap(),
        wire::Frame::Refused {
            reason: wire::Refusal::Capacity,
            ..
        }
    ));
    let release = wire::Frame::Release(receipt).encode().unwrap();
    assert_eq!(
        policy.request(registration, &release),
        Ok(wire::Frame::Released(receipt))
    );
    assert!(old.upgrade().is_none());
    // Lost ack/stale release must not drop replacement handles or policy.
    assert_eq!(
        policy.request(registration, &release),
        Ok(wire::Frame::Released(receipt))
    );
    assert_eq!(
        policy.metadata(registration, new_receipt).unwrap()[1].1.uid,
        3
    );
    let new = Arc::downgrade(&policy.bindings[0].as_ref().unwrap().pair);
    policy.withdraw(replacement).unwrap();
    assert!(new.upgrade().is_some());
    drop(policy); // Explicit model destruction, not evidence of runtime drain.
    assert!(new.upgrade().is_none());
}

#[test]
fn failed_installations_burn_generations_and_policy_capacity_is_independent() {
    let registration = registration();
    let scope = scope();
    let namespace = namespace();
    let mut policy = Policy::<1, 0>::new(registration).unwrap();
    assert_eq!(
        policy.install(scope, &namespace, "/root", "/missing"),
        Err(Error::Filesystem(FsError::PermissionDenied))
    );
    assert_eq!(
        policy.install(scope, &namespace, "/root", "/root/cwd"),
        Err(Error::Generation)
    );
    let mut next = scope;
    next.namespace_generation += 1;
    policy
        .install(next, &namespace, "/root", "/root/cwd")
        .unwrap();
    let mut other = next;
    other.namespace_generation += 1;
    assert_eq!(
        policy.install(other, &namespace, "/root", "/root/cwd"),
        Err(Error::Scope)
    );
    other.caller.pid += 1;
    assert_eq!(
        policy.install(other, &namespace, "/root", "/root/cwd"),
        Err(Error::Capacity)
    );
    assert!(matches!(
        policy.request(registration, &retain(next, 1)).unwrap(),
        wire::Frame::Refused {
            reason: wire::Refusal::Capacity,
            ..
        }
    ));
    policy.withdraw(next).unwrap();
    other.namespace_generation = u64::MAX;
    policy
        .install(other, &namespace, "/root", "/root/cwd")
        .unwrap();
    policy.withdraw(other).unwrap();
    assert_eq!(
        policy.install(scope, &namespace, "/root", "/root/cwd"),
        Err(Error::Generation)
    );
}

#[test]
fn explicit_refusal_preserves_replay_protection_and_existing_policy() {
    let r = registration();
    let s = scope();
    let mut policy = Policy::<1, 2>::new(r).unwrap();
    policy
        .install(s, &namespace(), "/root", "/root/cwd")
        .unwrap();
    // Even an existing binding cannot grant a lease on this denied request.
    assert!(matches!(
        policy.refuse(r, &retain(s, 1)).unwrap(),
        wire::Frame::Refused {
            reason: wire::Refusal::Resolve,
            ..
        }
    ));
    assert!(policy.request(r, &retain(s, 1)).is_err());
    let wire::Frame::Retained(receipt) = policy.request(r, &retain(s, 2)).unwrap() else {
        panic!("new request must still use original policy")
    };
    // A replay can refer to an unknown successful lease: never emit a refusal.
    assert!(policy.refuse(r, &retain(s, 2)).is_err());
    assert!(policy.metadata(r, receipt).is_ok());
}
