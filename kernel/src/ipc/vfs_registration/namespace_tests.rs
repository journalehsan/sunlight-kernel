//! Production kernel/service components with supplied identities, no transport.
use super::super::vfs_resources::{charged_bytes, TEST_LEDGER_LOCK};
use super::*;
use sunlight_fs::{mode, FileSystem, Tmpfs, TmpfsLimits};
use sunlight_ipc::vfs_rpc::{
    context::{self, LOOKUP_METADATA},
    lifecycle::Failure,
};
use sunlight_vfs_server::{directory::DirectoryNamespace, runtime::Runtime as Service};

fn runtime() -> (
    Runtime,
    AdmissionIdentity,
    Service<64, 64, 4>,
    DirectoryNamespace,
) {
    let mut kernel = Runtime::new();
    let registration = kernel
        .register(EndpointOwner {
            endpoint: 0,
            pid: 3,
            incarnation: 4,
            address_space_generation: 5,
        })
        .unwrap();
    let identity = AdmissionIdentity {
        caller: Caller {
            pid: 10,
            incarnation: 11,
            address_space_generation: 12,
        },
        uid: 1000,
        gid: 100,
        groups: [],
        registration,
    };
    let mut service = Service::new();
    service.bind(registration).unwrap();
    let mut fs = Tmpfs::with_limits(&[], TmpfsLimits::new(0, 4)).unwrap();
    fs.mkdir("/root", 1, 2, mode::DIR_755).unwrap();
    fs.mkdir("/root/cwd", 3, 4, mode::DIR_700).unwrap();
    (
        kernel,
        identity,
        service,
        DirectoryNamespace::new(fs).unwrap(),
    )
}

fn provision(
    kernel: &mut Runtime,
    identity: AdmissionIdentity,
    service: &mut Service<64, 64, 4>,
    namespace: &DirectoryNamespace,
) -> Receipt {
    let frame = kernel.prepare_namespace(identity, LOOKUP_METADATA).unwrap();
    let Frame::Retain { scope, .. } = frame else {
        panic!()
    };
    service
        .install(
            identity.registration,
            scope,
            namespace,
            "/root",
            "/root/cwd",
        )
        .unwrap();
    let Frame::Retained(receipt) = service
        .request(identity.registration, &frame.encode().unwrap())
        .unwrap()
    else {
        panic!()
    };
    receipt
}

fn install(kernel: &mut Runtime, identity: AdmissionIdentity, receipt: Receipt) {
    assert_eq!(
        kernel.install_retained(
            identity.registration,
            receipt.request_id(),
            &Frame::Retained(receipt).encode().unwrap(),
            Some(identity)
        ),
        Ok(None)
    );
}

#[test]
fn receipt_installation_pins_real_service_directories_until_reap_owner_drops() {
    let _serial = TEST_LEDGER_LOCK.lock();
    assert_eq!(charged_bytes(), 0);
    for service_retired in [false, true] {
        let (mut kernel, identity, mut service, namespace) = runtime();
        let r = identity.registration;
        kernel.namespaces.reserve_scope(identity).unwrap(); // An abandoned scope is never reused.
        let receipt = provision(&mut kernel, identity, &mut service, &namespace);
        assert_ne!(receipt.scope().namespace_generation, receipt.generation());
        install(&mut kernel, identity, receipt);
        assert!(kernel
            .released(
                r,
                receipt.request_id(),
                &Frame::Released(receipt).encode().unwrap()
            )
            .is_err());
        let request = kernel.admit_stat(identity, "/file").unwrap().words()[1];
        let request2 = kernel.admit_stat(identity, "relative").unwrap().words()[1];
        let before = service.metadata(r, receipt).unwrap();
        assert_ne!(before[0].0, before[1].0); // Distinct root and cwd backend objects.
        kernel.reconcile_namespaces(); // Live policy prevents early release.
        assert_eq!(kernel.remote_owner(r).unwrap().releases().count(), 0);
        if service_retired {
            kernel.retire_registration(r);
            service.retire_registration(r).unwrap();
        } else {
            kernel.retire_process(identity.caller);
            service.retire_caller(identity.caller);
        }
        assert!(kernel.namespaces.lookup(identity).is_err());
        assert!(kernel.admit_stat(identity, "/denied").is_err());
        assert_eq!(service.metadata(r, receipt).unwrap(), before);
        kernel.reconcile_namespaces();
        assert_eq!(kernel.remote_owner(r).unwrap().releases().count(), 0);
        let retired = kernel.pending.reap(request).unwrap();
        let mut groups = [0; context::MAX_GROUPS];
        let decoded = context::decode(retired.resources.input(), &mut groups).unwrap();
        assert_eq!(decoded.context.namespace.id, receipt.scope().namespace_id);
        assert_eq!(
            decoded.context.namespace.generation,
            receipt.scope().namespace_generation
        );
        assert_eq!(decoded.context.namespace.root, receipt.generation());
        assert_eq!(decoded.context.namespace.directory, receipt.generation());
        assert_eq!(
            retired.resources.revalidate_namespace(),
            Err(ResourceError::StaleNamespace)
        );
        drop(retired);
        kernel.reconcile_namespaces();
        assert_eq!(kernel.remote_owner(r).unwrap().releases().count(), 0);
        let last = kernel.pending.reap(request2).unwrap();
        kernel.reconcile_namespaces(); // Reap transferred ownership, still pinned.
        assert_eq!(kernel.remote_owner(r).unwrap().releases().count(), 0);
        assert!(charged_bytes() > 0);
        drop(last);
        assert_eq!(charged_bytes(), 0);
        kernel.reconcile_namespaces();
        let release = Frame::Release(receipt);
        assert_eq!(
            kernel
                .remote_owner(r)
                .unwrap()
                .releases()
                .collect::<alloc::vec::Vec<_>>(),
            [release]
        );
        for _ in 0..2 {
            // Lost release ACK: service accepts exact retry.
            assert_eq!(
                service.request(r, &release.encode().unwrap()).unwrap(),
                Frame::Released(receipt)
            );
            kernel.reconcile_namespaces();
            assert_eq!(kernel.remote_owner(r).unwrap().outstanding(), 1);
        }
        assert!(service.metadata(r, receipt).is_err());
        kernel
            .released(
                r,
                receipt.request_id(),
                &Frame::Released(receipt).encode().unwrap(),
            )
            .unwrap();
        // Lease settlement alone cannot discard the caller-delivery obligation,
        // including when the registration itself has retired.
        assert_eq!(kernel.remote_owner(r).unwrap().outstanding(), 0);
    }
}

#[test]
fn malformed_or_wrong_identity_reply_preserves_preparation_and_duplicate_cannot_install() {
    let (mut kernel, identity, mut service, namespace) = runtime();
    let receipt = provision(&mut kernel, identity, &mut service, &namespace);
    let bytes = Frame::Retained(receipt).encode().unwrap();
    for len in 0..bytes.len() {
        assert!(kernel
            .install_retained(
                identity.registration,
                receipt.request_id(),
                &bytes[..len],
                Some(identity)
            )
            .is_err());
    }
    for word in 2..13 {
        // Every registration/scope/request word.
        let mut wrong = bytes;
        wrong[word * 8] ^= 16;
        assert!(kernel
            .install_retained(
                identity.registration,
                receipt.request_id(),
                &wrong,
                Some(identity)
            )
            .is_err());
    }
    for field in 0..5 {
        let mut wrong = identity.registration;
        match field {
            0 => wrong.owner.endpoint += 1,
            1 => wrong.owner.pid += 1,
            2 => wrong.owner.incarnation += 1,
            3 => wrong.owner.address_space_generation += 1,
            _ => wrong.epoch += 1,
        }
        assert!(kernel
            .install_retained(wrong, receipt.request_id(), &bytes, Some(identity))
            .is_err());
    }
    assert!(kernel.namespaces.lookup(identity).is_err());
    assert_eq!(
        kernel
            .remote_owner(identity.registration)
            .unwrap()
            .outstanding(),
        1
    );
    install(&mut kernel, identity, receipt);
    assert!(kernel
        .install_retained(
            identity.registration,
            receipt.request_id(),
            &bytes,
            Some(identity)
        )
        .is_err());
    assert_eq!(
        kernel
            .remote_owner(identity.registration)
            .unwrap()
            .releases()
            .count(),
        0
    );
}

#[test]
fn changed_live_snapshot_compensates_without_installing_or_losing_release() {
    for field in 0..9 {
        let (mut kernel, identity, mut service, namespace) = runtime();
        let receipt = provision(&mut kernel, identity, &mut service, &namespace);
        let mut live = identity;
        match field {
            0 => live.caller.pid += 1,
            1 => live.caller.incarnation += 1,
            2 => live.caller.address_space_generation += 1,
            3 => live.uid += 1,
            4 => live.gid += 1,
            5 => live.registration.owner.address_space_generation += 1,
            6 => live.registration.epoch += 1,
            _ => (),
        }
        if field == 8 {
            kernel.retire_process(identity.caller);
        }
        let release = Frame::Release(receipt);
        assert_eq!(
            kernel.install_retained(
                identity.registration,
                receipt.request_id(),
                &Frame::Retained(receipt).encode().unwrap(),
                if field == 7 { None } else { Some(live) }
            ),
            Ok(Some(release))
        );
        assert!(kernel.namespaces.lookup(identity).is_err());
        assert_eq!(
            kernel
                .remote_owner(identity.registration)
                .unwrap()
                .releases()
                .collect::<alloc::vec::Vec<_>>(),
            [release]
        );
    }
}

#[test]
fn installation_failure_compensates_and_refusal_reclaims_preparation() {
    let (mut kernel, identity, mut service, namespace) = runtime();
    let first = provision(&mut kernel, identity, &mut service, &namespace);
    install(&mut kernel, identity, first);
    // Service provisioning has advanced while the old local binding still owns
    // its lease. Kernel installation must reject implicit replacement.
    service
        .withdraw(identity.registration, first.scope())
        .unwrap();
    let duplicate = provision(&mut kernel, identity, &mut service, &namespace);
    assert_eq!(
        kernel.install_retained(
            identity.registration,
            duplicate.request_id(),
            &Frame::Retained(duplicate).encode().unwrap(),
            Some(identity)
        ),
        Err(NamespaceRetentionError::Resource(
            ResourceError::StaleNamespace
        ))
    );
    assert_eq!(
        kernel
            .remote_owner(identity.registration)
            .unwrap()
            .releases()
            .collect::<alloc::vec::Vec<_>>(),
        [Frame::Release(duplicate)]
    );
    assert!(kernel.namespaces.lookup(identity).is_ok());
    let mut other = identity;
    other.caller.pid += 1;
    let frame = kernel.prepare_namespace(other, 0).unwrap();
    let Frame::Retain {
        request_id, scope, ..
    } = frame
    else {
        panic!()
    };
    // No service policy installed: exact refusal clears both bounded tables.
    let refused = service
        .request(identity.registration, &frame.encode().unwrap())
        .unwrap();
    let bytes = refused.encode().unwrap();
    assert!(kernel
        .refused(identity.registration, request_id + 1, &bytes)
        .is_err());
    assert_eq!(
        kernel.refused(identity.registration, request_id, &bytes),
        Ok(Refusal::Resolve)
    );
    let Frame::Retain {
        request_id: next,
        scope: next_scope,
        ..
    } = kernel.prepare_namespace(other, 0).unwrap()
    else {
        panic!()
    };
    assert!(next > request_id);
    assert!(next_scope.namespace_generation > scope.namespace_generation);
}

#[test]
fn namespace_and_lease_capacity_fail_closed_with_retryable_compensation() {
    let (mut kernel, identity, mut service, namespace) = runtime();
    // Fill local admission independently; remote retain still succeeds and must
    // be compensated when there is no local installation capacity.
    for n in 0..64 {
        let mut other = identity;
        other.caller.pid += n + 1;
        kernel.namespaces.install(other, 9, 10, 0).unwrap();
    }
    let receipt = provision(&mut kernel, identity, &mut service, &namespace);
    assert_eq!(
        kernel.install_retained(
            identity.registration,
            receipt.request_id(),
            &Frame::Retained(receipt).encode().unwrap(),
            Some(identity)
        ),
        Err(NamespaceRetentionError::Resource(
            ResourceError::NamespaceCapacity
        ))
    );
    assert!(kernel.namespaces.lookup(identity).is_err());
    assert_eq!(
        kernel
            .remote_owner(identity.registration)
            .unwrap()
            .releases()
            .collect::<alloc::vec::Vec<_>>(),
        [Frame::Release(receipt)]
    );
    for _ in 1..REMOTE_LEASES {
        kernel.prepare_namespace(identity, 0).unwrap();
    }
    assert_eq!(
        kernel.prepare_namespace(identity, 0),
        Err(NamespaceRetentionError::Ownership(OwnershipError::Capacity))
    );
    assert_eq!(
        kernel
            .remote_owner(identity.registration)
            .unwrap()
            .outstanding(),
        REMOTE_LEASES
    );
    assert_eq!(
        kernel.prepare_namespace(identity, LOOKUP_METADATA << 1),
        Err(NamespaceRetentionError::Resource(
            ResourceError::StaleNamespace
        ))
    );
    kernel.retire_registration(identity.registration);
    assert_eq!(
        kernel
            .remote_owner(identity.registration)
            .unwrap()
            .outstanding(),
        REMOTE_LEASES
    );
    assert_eq!(
        kernel
            .remote_owner(identity.registration)
            .unwrap()
            .releases()
            .count(),
        1
    );
}

#[test]
fn late_prepared_receipt_across_replacement_releases_only_old_epoch() {
    let (mut kernel, identity, mut service, namespace) = runtime();
    let old = provision(&mut kernel, identity, &mut service, &namespace);
    kernel.retire_registration(identity.registration);
    service.retire_registration(identity.registration).unwrap();
    let registration = kernel.register(identity.registration.owner).unwrap();
    service.bind(registration).unwrap();
    let new_identity = AdmissionIdentity {
        registration,
        ..identity
    };
    let new = provision(&mut kernel, new_identity, &mut service, &namespace);
    install(&mut kernel, new_identity, new);
    assert_eq!(
        kernel.install_retained(
            identity.registration,
            old.request_id(),
            &Frame::Retained(old).encode().unwrap(),
            Some(identity)
        ),
        Ok(Some(Frame::Release(old)))
    );
    assert!(kernel.namespaces.lookup(new_identity).is_ok());
    assert_eq!(
        kernel
            .remote_owner(registration)
            .unwrap()
            .releases()
            .count(),
        0
    );
    let ack = service
        .request(
            identity.registration,
            &Frame::Release(old).encode().unwrap(),
        )
        .unwrap();
    kernel
        .released(
            identity.registration,
            old.request_id(),
            &ack.encode().unwrap(),
        )
        .unwrap();
    assert_eq!(
        kernel
            .remote_owner(identity.registration)
            .unwrap()
            .outstanding(),
        0
    );
    assert_eq!(kernel.remote_owner(registration).unwrap().outstanding(), 1);
}

#[test]
fn explicit_withdrawal_waits_for_retained_preparation_reference() {
    let (mut kernel, identity, mut service, namespace) = runtime();
    let receipt = provision(&mut kernel, identity, &mut service, &namespace);
    install(&mut kernel, identity, receipt);
    let retained = kernel.namespaces.lookup(identity).unwrap();
    let mut wrong = identity;
    wrong.registration.epoch += 1;
    assert_eq!(
        kernel.namespaces.withdraw(wrong),
        Err(ResourceError::StaleNamespace)
    );
    kernel.reconcile_namespaces();
    assert_eq!(
        kernel
            .remote_owner(identity.registration)
            .unwrap()
            .releases()
            .count(),
        0
    );
    kernel.namespaces.withdraw(identity).unwrap();
    kernel.reconcile_namespaces();
    assert_eq!(
        kernel
            .remote_owner(identity.registration)
            .unwrap()
            .releases()
            .count(),
        0
    );
    drop(retained);
    kernel.reconcile_namespaces();
    assert_eq!(
        kernel
            .remote_owner(identity.registration)
            .unwrap()
            .releases()
            .collect::<alloc::vec::Vec<_>>(),
        [Frame::Release(receipt)]
    );
}

#[test]
fn retired_receipt_drain_survives_replacement_without_releasing_new_binding() {
    let _serial = TEST_LEDGER_LOCK.lock();
    let (mut kernel, identity, mut service, namespace) = runtime();
    let old = provision(&mut kernel, identity, &mut service, &namespace);
    install(&mut kernel, identity, old);
    let request = kernel.admit_stat(identity, "/old").unwrap().words()[1];
    kernel.retire_registration(identity.registration);
    service.retire_registration(identity.registration).unwrap();
    let registration = kernel.register(identity.registration.owner).unwrap();
    service.bind(registration).unwrap();
    let new_identity = AdmissionIdentity {
        registration,
        ..identity
    };
    let new = provision(&mut kernel, new_identity, &mut service, &namespace);
    assert!(new.scope().namespace_generation > old.scope().namespace_generation);
    install(&mut kernel, new_identity, new);
    let new_request = kernel.admit_stat(new_identity, "/new").unwrap().words()[1];
    kernel.retire_registration(identity.registration);
    kernel.reconcile_namespaces();
    assert_eq!(
        kernel
            .remote_owner(identity.registration)
            .unwrap()
            .releases()
            .count(),
        0
    );
    drop(kernel.pending.reap(request).unwrap());
    kernel.reconcile_namespaces();
    assert_eq!(
        kernel
            .remote_owner(identity.registration)
            .unwrap()
            .releases()
            .count(),
        1
    );
    assert_eq!(
        kernel
            .remote_owner(registration)
            .unwrap()
            .releases()
            .count(),
        0
    );
    assert!(kernel.namespaces.lookup(new_identity).is_ok());
    kernel
        .pending
        .fail(new_request, Failure::Cancelled)
        .unwrap();
    drop(kernel.pending.reap(new_request).unwrap());
    kernel.reconcile_namespaces(); // A drained request does not withdraw live policy.
    assert_eq!(
        kernel
            .remote_owner(registration)
            .unwrap()
            .releases()
            .count(),
        0
    );
    assert_eq!(charged_bytes(), 0);
}
