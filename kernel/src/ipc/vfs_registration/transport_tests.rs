use super::*;
use crate::sched::{AddressSpace, Foreground, Process};
use sunlight_ipc::vfs_rpc::context::LOOKUP_METADATA;
use sunlight_vfs_server::{directory::DirectoryNamespace, runtime::Runtime as Service};

fn setup() -> (Runtime, Scheduler, CapabilityBroker, AdmissionIdentity) {
    let mut runtime = Runtime::new();
    let registration = runtime
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
    let sched = Scheduler {
        pending_children: crate::process::pending_state::Pending::new(),
        global_tick: 0,
        processes: alloc::vec![
            Process {
                pid: 3,
                trusted_vfs_service: true,
                vfs_root_metadata: false,
                vfs_boot_root: false,
                vfs_root_rebased: false,
                vfs_downloads_cwd: false,
                shared_owner: None,
                vfs_stat_routed: false,
                vfs_file_routed: false,
                cwd: alloc::string::String::from("/"),
                exiting: false,
                state: crate::sched::ProcessState::Running,
                pending_call: None,
                ipc_reply_target: None,
                ipc_endpoint: None,
                uid: 0,
                gid: 0,
                foreground: Foreground { incarnation: 4, ..Default::default() },
                address_space: AddressSpace { generation: 5 }
            },
            Process {
                pid: 10,
                trusted_vfs_service: false,
                vfs_root_metadata: false,
                vfs_boot_root: false,
                vfs_root_rebased: false,
                vfs_downloads_cwd: false,
                shared_owner: None,
                vfs_stat_routed: false,
                vfs_file_routed: false,
                cwd: alloc::string::String::from("/"),
                exiting: false,
                state: crate::sched::ProcessState::Running,
                pending_call: None,
                ipc_reply_target: None,
                ipc_endpoint: None,
                uid: 1000,
                gid: 100,
                foreground: Foreground { incarnation: 11, ..Default::default() },
                address_space: AddressSpace { generation: 12 }
            },
        ],
    };
    (
        runtime,
        sched,
        CapabilityBroker {
            endpoints: alloc::vec![(0, 3)],
        },
        identity,
    )
}

fn call(
    runtime: &mut Runtime,
    sched: &Scheduler,
    caps: &CapabilityBroker,
    pid: usize,
    op: u64,
    epoch: u64,
    bytes: &[u8],
) -> Result<Option<[u8; FRAME_BYTES]>, IpcError> {
    control_runtime(runtime, pid, op, epoch, bytes, sched, caps)
}

fn retirement_identity(
    registration: Registration,
) -> crate::capability::delivery::RecipientIdentity {
    crate::capability::delivery::RecipientIdentity {
        pid: registration.owner.pid as usize,
        incarnation: registration.owner.incarnation,
        vm_generation: registration.owner.address_space_generation,
    }
}

#[path = "spawn_tests.rs"]
mod spawn_tests;

#[test]
fn last_owner_exit_recovers_unanswered_retains_for_100_replacements() {
    let (mut runtime, mut sched, mut caps, mut identity) = setup();
    let namespace = crate::mounted_namespace();
    let denied = Err(IpcError::InvalidCapability);
    let mut previous_reply: Option<[u8; FRAME_BYTES]> = None;
    for _ in 0..100 {
        let old = identity.registration;
        let image = retirement_identity(old);
        let mut service = Service::<4, 4, 1>::new();
        service.bind(old).unwrap();
        runtime.queue_namespace(identity, LOOKUP_METADATA).unwrap();
        let bytes = call(
            &mut runtime,
            &sched,
            &caps,
            image.pid,
            api::NEXT,
            old.epoch,
            &[],
        )
        .unwrap()
        .unwrap();
        if let Some(previous) = previous_reply {
            // Even with a matching new request number already delivered,
            // the previous image's reply cannot acquire a new binding.
            assert_eq!(
                call(
                    &mut runtime,
                    &sched,
                    &caps,
                    image.pid,
                    api::REPLY,
                    old.epoch,
                    &previous
                ),
                denied
            );
        }
        let Frame::Retain { scope, .. } = Frame::decode(&bytes).unwrap() else {
            panic!("expected retain");
        };
        service.install(old, scope, &namespace, "/", "/").unwrap();
        let reply = service.request(old, &bytes).unwrap().encode().unwrap();
        runtime.queue_namespace(identity, LOOKUP_METADATA).unwrap();
        runtime.retire_registration(old);
        assert_eq!(runtime.remote_owner(old).unwrap().outstanding(), 2);
        for conditions in [
            (false, true, true),
            (true, false, true),
            (true, true, false),
        ] {
            assert!(
                crate::sched::exit_proof(image, conditions.0, conditions.1, conditions.2).is_none()
            );
            runtime.collect_remote();
            assert_eq!(runtime.remote_owner(old).unwrap().outstanding(), 2);
        }
        sched.processes[0].exiting = true;
        caps.endpoints.clear();
        assert_eq!(
            call(
                &mut runtime,
                &sched,
                &caps,
                image.pid,
                api::REPLY,
                old.epoch,
                &reply
            ),
            denied
        );
        drop(service);

        // Reuse numeric PID and endpoint in the host records. The new image
        // can publish before the old completion is delivered.
        sched.processes[0].exiting = false;
        sched.processes[0].foreground.incarnation += 1;
        sched.processes[0].address_space.generation += 1;
        caps.endpoints.push((old.owner.endpoint as u32, image.pid));
        register_service(
            &mut runtime,
            image.pid,
            old.owner.endpoint as u32,
            &sched,
            &caps,
        )
        .unwrap();
        identity.registration = runtime.registry.active().unwrap();
        let snapshot = call(
            &mut runtime,
            &sched,
            &caps,
            image.pid,
            api::SNAPSHOT,
            0,
            &[],
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            api::decode_registration(&snapshot[..api::REGISTRATION_BYTES]),
            Ok(identity.registration),
        );
        for wrong in [
            crate::capability::delivery::RecipientIdentity {
                incarnation: image.incarnation + 1,
                ..image
            },
            crate::capability::delivery::RecipientIdentity {
                vm_generation: image.vm_generation + 1,
                ..image
            },
        ] {
            runtime.complete_image_exit(crate::sched::exit_proof(wrong, true, true, true).unwrap());
            assert_eq!(runtime.remote_owner(old).unwrap().outstanding(), 2);
        }
        let completion = crate::sched::exit_proof(image, true, true, true).unwrap();
        runtime.complete_image_exit(completion);
        runtime.complete_image_exit(completion);
        assert!(runtime.remote_owner(old).is_err());
        assert_eq!(runtime.remote.iter().flatten().count(), 1);
        assert_eq!(runtime.registry.active(), Some(identity.registration));
        assert_eq!(
            call(
                &mut runtime,
                &sched,
                &caps,
                image.pid,
                api::REPLY,
                old.epoch,
                &reply
            ),
            denied
        );
        previous_reply = Some(reply);
    }
}

#[test]
fn mixed_exec_withdrawal_and_exit_recover_private_backing_for_100_replacements() {
    let (mut runtime, mut sched, mut caps, mut identity) = setup();
    let namespace = crate::mounted_namespace();
    let denied = Err(IpcError::InvalidCapability);
    let mut next_endpoint = 1;
    for cycle in 0..100 {
        let old = identity.registration;
        let image = retirement_identity(old);
        let mut service = Service::<4, 4, 1>::new();
        service.bind(old).unwrap();
        runtime.queue_namespace(identity, LOOKUP_METADATA).unwrap();
        let bytes = call(&mut runtime, &sched, &caps, 3, api::NEXT, old.epoch, &[])
            .unwrap()
            .unwrap();
        let Frame::Retain { scope, .. } = Frame::decode(&bytes).unwrap() else {
            panic!("expected retain");
        };
        service.install(old, scope, &namespace, "/", "/").unwrap();
        let reply = service.request(old, &bytes).unwrap().encode().unwrap();
        let hold_binding = cycle % 4 < 2;
        let binding = if hold_binding {
            call(&mut runtime, &sched, &caps, 3, api::REPLY, old.epoch, &reply).unwrap();
            Some(runtime.namespaces.lookup(identity).unwrap())
        } else {
            None
        };

        assert!(crate::process::exec_proof(image, true, true, false).is_none());
        assert_eq!(runtime.registry.active(), Some(old));
        assert_eq!(runtime.remote_owner(old).unwrap().outstanding(), 1);

        if cycle % 4 != 0 {
            caps.endpoints.clear();
            runtime.retire_endpoint(old.owner.endpoint as u32, process_identity(&sched.processes[0]));
            runtime.collect_remote();
            assert_eq!(runtime.registry.active(), None);
            assert_eq!(runtime.remote_owner(old).unwrap().outstanding(), 1);
            assert_eq!(
                call(&mut runtime, &sched, &caps, 3, api::REPLY, old.epoch, &reply),
                denied
            );
        }

        runtime.retire_process(process_identity(&sched.processes[0]));
        if cycle % 4 == 3 {
            sched.processes[0].exiting = true;
            assert!(crate::sched::exit_proof(image, false, true, true).is_none());
            assert!(crate::sched::exit_proof(image, true, false, true).is_none());
        } else {
            sched.processes[0].trusted_vfs_service = false;
            sched.processes[0].address_space.generation += 1;
            assert!(crate::process::exec_proof(image, true, false, false).is_none());
            assert_eq!(
                register_service(&mut runtime, 3, old.owner.endpoint as u32, &sched, &caps),
                Err(IpcError::InvalidCapability)
            );
            assert_eq!(
                call(&mut runtime, &sched, &caps, 3, api::REPLY, old.epoch, &reply),
                denied
            );
        }
        runtime.collect_remote();
        assert_eq!(runtime.remote_owner(old).unwrap().outstanding(), 1);
        drop(service);

        sched.processes[0].exiting = false;
        sched.processes[0].trusted_vfs_service = true;
        sched.processes[0].foreground.incarnation += 1;
        sched.processes[0].address_space.generation += 1;
        caps.endpoints.clear();
        caps.endpoints.push((next_endpoint, 3));
        register_service(&mut runtime, 3, next_endpoint, &sched, &caps).unwrap();
        next_endpoint += 1;
        identity.registration = runtime.registry.active().unwrap();
        runtime.queue_namespace(identity, LOOKUP_METADATA).unwrap();
        assert_eq!(
            call(&mut runtime, &sched, &caps, 3, api::REPLY, identity.registration.epoch, &reply),
            denied
        );
        assert_eq!(runtime.remote_owner(identity.registration).unwrap().outstanding(), 1);

        if cycle % 4 == 3 {
            runtime.complete_image_exit(crate::sched::exit_proof(image, true, true, true).unwrap());
        } else {
            for _ in 0..2 {
                runtime.complete_private_exec(crate::process::exec_proof(image, true, false, true).unwrap());
            }
        }
        assert_eq!(runtime.remote_owner(old).is_ok(), hold_binding);
        drop(binding);
        runtime.collect_remote();
        assert!(runtime.remote_owner(old).is_err());
        assert_eq!(runtime.remote.iter().flatten().count(), 1);
        assert_eq!(runtime.remote_owner(identity.registration).unwrap().outstanding(), 1);
        let request = call(&mut runtime, &sched, &caps, 3, api::NEXT, identity.registration.epoch, &[])
            .unwrap()
            .unwrap();
        let mut replacement = Service::<4, 4, 1>::new();
        replacement.bind(identity.registration).unwrap();
        let refused = replacement.request(identity.registration, &request).unwrap().encode().unwrap();
        call(&mut runtime, &sched, &caps, 3, api::REPLY, identity.registration.epoch, &refused).unwrap();
        assert_eq!(runtime.remote_owner(identity.registration).unwrap().outstanding(), 0);
    }
}

#[test]
fn exec_proof_settles_only_retired_epochs_of_the_exact_old_image() {
    let (mut runtime, mut sched, mut caps, identity) = setup();
    let first = identity.registration;
    let image = retirement_identity(first);
    let second_owner = EndpointOwner { endpoint: 1, ..first.owner };
    runtime.queue_namespace(identity, LOOKUP_METADATA).unwrap();
    caps.endpoints.clear();
    runtime.retire_endpoint(0, process_identity(&sched.processes[0]));
    caps.endpoints.push((1, image.pid));
    let second = runtime.register(second_owner).unwrap();
    runtime.queue_namespace(AdmissionIdentity { registration: second, ..identity }, LOOKUP_METADATA).unwrap();
    runtime.retire_process(process_identity(&sched.processes[0]));
    sched.processes[0].trusted_vfs_service = false;
    sched.processes[0].address_space.generation += 1;

    let replacement = runtime.register(EndpointOwner {
        address_space_generation: first.owner.address_space_generation + 1,
        ..second_owner
    }).unwrap();
    runtime.queue_namespace(AdmissionIdentity { registration: replacement, ..identity }, LOOKUP_METADATA).unwrap();
    runtime.retire_registration(replacement);
    for wrong in [
        crate::capability::delivery::RecipientIdentity { incarnation: image.incarnation + 1, ..image },
        crate::capability::delivery::RecipientIdentity { vm_generation: image.vm_generation + 2, ..image },
    ] {
        runtime.complete_private_exec(crate::process::exec_proof(wrong, true, false, true).unwrap());
        assert_eq!(runtime.remote.iter().flatten().count(), 3);
    }
    assert!(crate::process::exec_proof(image, false, false, true).is_none());
    assert!(crate::process::exec_proof(image, true, true, true).is_none());
    assert!(crate::process::exec_proof(image, true, false, false).is_none());
    runtime.complete_private_exec(crate::process::exec_proof(image, true, false, true).unwrap());
    assert!(runtime.remote_owner(first).is_err());
    assert!(runtime.remote_owner(second).is_err());
    assert_eq!(runtime.remote_owner(replacement).unwrap().outstanding(), 1);
    assert_eq!(runtime.remote.iter().flatten().count(), 1);
    assert_eq!(caps.endpoint_owner(1), Some(image.pid));
    assert_eq!(
        call(&mut runtime, &sched, &caps, image.pid, api::STATUS, second.epoch, &[]),
        Err(IpcError::InvalidCapability)
    );
}

#[test]
fn exit_completion_stays_with_epoch_until_installed_binding_drains() {
    let (mut runtime, sched, caps, identity) = setup();
    let old = identity.registration;
    let namespace = crate::mounted_namespace();
    let mut service = Service::<4, 4, 1>::new();
    service.bind(old).unwrap();
    runtime.queue_namespace(identity, LOOKUP_METADATA).unwrap();
    let bytes = call(&mut runtime, &sched, &caps, 3, api::NEXT, old.epoch, &[])
        .unwrap()
        .unwrap();
    let Frame::Retain { scope, .. } = Frame::decode(&bytes).unwrap() else {
        panic!("expected retain");
    };
    service.install(old, scope, &namespace, "/", "/").unwrap();
    let reply = service.request(old, &bytes).unwrap().encode().unwrap();
    call(
        &mut runtime,
        &sched,
        &caps,
        3,
        api::REPLY,
        old.epoch,
        &reply,
    )
    .unwrap();
    let user = runtime.namespaces.lookup(identity).unwrap();

    runtime.retire_registration(old);
    drop(service);
    runtime.complete_image_exit(
        crate::sched::exit_proof(retirement_identity(old), true, true, true).unwrap(),
    );
    assert_eq!(runtime.remote_owner(old).unwrap().outstanding(), 1);
    // Independent retirement consumes its own evidence while the prior row
    // remains pinned. Observation of the first completion need not recur.
    let second = runtime
        .register(EndpointOwner {
            pid: 4,
            ..old.owner
        })
        .unwrap();
    runtime
        .queue_namespace(
            AdmissionIdentity {
                registration: second,
                ..identity
            },
            LOOKUP_METADATA,
        )
        .unwrap();
    runtime.retire_registration(second);
    runtime.complete_image_exit(
        crate::sched::exit_proof(retirement_identity(second), true, true, true).unwrap(),
    );
    assert!(runtime.remote_owner(second).is_err());
    assert!(runtime.remote_owner(old).is_ok());
    drop(user);
    runtime.collect_remote();
    assert!(runtime.remote_owner(old).is_err());
}

#[test]
fn service_control_authenticates_full_owner_and_exact_epoch() {
    let (mut runtime, mut sched, mut caps, identity) = setup();
    let reg = identity.registration;
    let bytes = call(&mut runtime, &sched, &caps, 3, api::SNAPSHOT, 0, &[])
        .unwrap()
        .unwrap();
    assert_eq!(
        api::decode_registration(&bytes[..api::REGISTRATION_BYTES]),
        Ok(reg)
    );
    let denied = Err(IpcError::InvalidCapability);
    assert_eq!(
        call(&mut runtime, &sched, &caps, 10, api::SNAPSHOT, 0, &[]),
        denied
    );
    assert_eq!(
        call(
            &mut runtime,
            &sched,
            &caps,
            3,
            api::SNAPSHOT,
            reg.epoch,
            &[]
        ),
        denied
    );
    assert_eq!(
        call(&mut runtime, &sched, &caps, 3, api::NEXT, 0, &[]),
        denied
    );
    assert_eq!(
        call(
            &mut runtime,
            &sched,
            &caps,
            3,
            api::NEXT,
            reg.epoch + 1,
            &[]
        ),
        denied
    );
    assert_eq!(
        call(&mut runtime, &sched, &caps, 3, api::NEXT, reg.epoch, &[0]),
        denied
    );
    assert_eq!(
        call(&mut runtime, &sched, &caps, 3, 99, reg.epoch, &[]),
        denied
    );
    for field in 0..6 {
        match field {
            0 => sched.processes[0].trusted_vfs_service = false,
            1 => sched.processes[0].exiting = true,
            2 => sched.processes[0].foreground.incarnation += 1,
            3 => sched.processes[0].address_space.generation += 1,
            4 => caps.endpoints[0].1 = 10,
            _ => caps.endpoints.clear(),
        }
        assert_eq!(
            call(&mut runtime, &sched, &caps, 3, api::NEXT, reg.epoch, &[]),
            denied
        );
        sched.processes[0].trusted_vfs_service = true;
        sched.processes[0].exiting = false;
        sched.processes[0].foreground.incarnation = 4;
        sched.processes[0].address_space.generation = 5;
        caps.endpoints = alloc::vec![(0, 3)];
    }
    assert_eq!(
        call(&mut runtime, &sched, &caps, 3, api::NEXT, reg.epoch, &[]),
        Ok(None)
    );
}

#[test]
fn copied_control_correlates_real_pins_and_compensates_changed_credentials() {
    let (mut kernel, mut sched, caps, identity) = setup();
    let reg = identity.registration;
    let mut service = Service::<4, 4, 2>::new();
    service.bind(reg).unwrap();
    let namespace = DirectoryNamespace::new(sunlight_fs::Tmpfs::new(&[])).unwrap();
    kernel.queue_namespace(identity, LOOKUP_METADATA).unwrap();
    let bytes = call(&mut kernel, &sched, &caps, 3, api::NEXT, reg.epoch, &[])
        .unwrap()
        .unwrap();
    let Frame::Retain { scope, .. } = Frame::decode(&bytes).unwrap() else {
        panic!()
    };
    // Trusted local provisioning is deliberately external to the channel.
    service.install(reg, scope, &namespace, "/", "/").unwrap();
    let reply = service.request(reg, &bytes).unwrap();
    let Frame::Retained(receipt) = reply else {
        panic!()
    };
    // Wrong origin cannot consume a valid receipt, nor can malformed bytes.
    assert!(call(
        &mut kernel,
        &sched,
        &caps,
        10,
        api::REPLY,
        reg.epoch,
        &reply.encode().unwrap()
    )
    .is_err());
    assert!(call(
        &mut kernel,
        &sched,
        &caps,
        3,
        api::REPLY,
        reg.epoch,
        &[0; FRAME_BYTES]
    )
    .is_err());
    sched.processes[1].uid += 1;
    assert_eq!(
        call(
            &mut kernel,
            &sched,
            &caps,
            3,
            api::REPLY,
            reg.epoch,
            &reply.encode().unwrap()
        ),
        Ok(None)
    );
    assert!(kernel.namespaces.lookup(identity).is_err());
    let release = call(&mut kernel, &sched, &caps, 3, api::NEXT, reg.epoch, &[])
        .unwrap()
        .unwrap();
    assert_eq!(Frame::decode(&release), Ok(Frame::Release(receipt)));
    let ack = service.request(reg, &release).unwrap().encode().unwrap();
    // Lost acknowledgement preserves the same retryable release.
    assert_eq!(
        call(&mut kernel, &sched, &caps, 3, api::NEXT, reg.epoch, &[]),
        Ok(Some(release))
    );
    assert_eq!(
        call(&mut kernel, &sched, &caps, 3, api::REPLY, reg.epoch, &ack),
        Ok(None)
    );
    assert_eq!(
        call(&mut kernel, &sched, &caps, 3, api::NEXT, reg.epoch, &[]),
        Ok(None)
    );
    assert_eq!(
        kernel
            .remote
            .iter()
            .flatten()
            .next()
            .unwrap()
            .ownership
            .outstanding(),
        0
    );
}

#[test]
fn failed_copy_never_retransmits_retain_and_late_success_releases_after_retirement() {
    let (mut kernel, sched, caps, identity) = setup();
    let reg = identity.registration;
    let mut service = Service::<4, 4, 2>::new();
    service.bind(reg).unwrap();
    let namespace = DirectoryNamespace::new(sunlight_fs::Tmpfs::new(&[])).unwrap();
    kernel.queue_namespace(identity, LOOKUP_METADATA).unwrap();
    let bytes = call(&mut kernel, &sched, &caps, 3, api::NEXT, reg.epoch, &[])
        .unwrap()
        .unwrap();
    // Discarding the copy leaves ownership pending and cannot publish again.
    assert_eq!(
        call(&mut kernel, &sched, &caps, 3, api::NEXT, reg.epoch, &[]),
        Ok(None)
    );
    assert_eq!(
        kernel
            .remote
            .iter()
            .flatten()
            .next()
            .unwrap()
            .ownership
            .outstanding(),
        1
    );
    let Frame::Retain { scope, .. } = Frame::decode(&bytes).unwrap() else {
        panic!()
    };
    service.install(reg, scope, &namespace, "/", "/").unwrap();
    let reply = service.request(reg, &bytes).unwrap().encode().unwrap();
    kernel.retire_registration(reg);
    let replacement = kernel.register(reg.owner).unwrap();
    assert!(replacement.epoch > reg.epoch);
    // Old epoch replies authenticate against retained owner records, even with
    // a newer active registration. They never install into its namespace.
    assert_eq!(
        call(&mut kernel, &sched, &caps, 3, api::REPLY, reg.epoch, &reply),
        Ok(None)
    );
    assert!(kernel.namespaces.lookup(identity).is_err());
    let release = call(&mut kernel, &sched, &caps, 3, api::NEXT, reg.epoch, &[])
        .unwrap()
        .unwrap();
    let ack = service.request(reg, &release).unwrap().encode().unwrap();
    assert!(call(
        &mut kernel,
        &sched,
        &caps,
        3,
        api::REPLY,
        replacement.epoch,
        &ack
    )
    .is_err());
    assert_eq!(
        call(&mut kernel, &sched, &caps, 3, api::REPLY, reg.epoch, &ack),
        Ok(None)
    );
    assert_eq!(kernel.registry.active(), Some(replacement));
}

#[test]
fn copied_control_installs_once_and_releases_only_after_local_drain() {
    let (mut kernel, sched, caps, identity) = setup();
    let reg = identity.registration;
    let mut service = Service::<4, 4, 2>::new();
    service.bind(reg).unwrap();
    let namespace = DirectoryNamespace::new(sunlight_fs::Tmpfs::new(&[])).unwrap();
    kernel.queue_namespace(identity, LOOKUP_METADATA).unwrap();
    let bytes = call(&mut kernel, &sched, &caps, 3, api::NEXT, reg.epoch, &[])
        .unwrap()
        .unwrap();
    let Frame::Retain { scope, .. } = Frame::decode(&bytes).unwrap() else {
        panic!()
    };
    service.install(reg, scope, &namespace, "/", "/").unwrap();
    let reply = service.request(reg, &bytes).unwrap().encode().unwrap();
    assert_eq!(
        call(&mut kernel, &sched, &caps, 3, api::REPLY, reg.epoch, &reply),
        Ok(None)
    );
    assert!(call(&mut kernel, &sched, &caps, 3, api::REPLY, reg.epoch, &reply).is_err());
    let binding = kernel.namespaces.lookup(identity).unwrap();
    kernel.retire_process(identity.caller);
    let event = call(&mut kernel, &sched, &caps, 3, api::NEXT, reg.epoch, &[])
        .unwrap()
        .unwrap();
    let ack = service
        .control_request(reg, &event)
        .unwrap()
        .encode()
        .unwrap();
    assert_eq!(
        call(&mut kernel, &sched, &caps, 3, api::REPLY, reg.epoch, &ack),
        Ok(None)
    );
    let event = call(&mut kernel, &sched, &caps, 3, api::NEXT, reg.epoch, &[])
        .unwrap()
        .unwrap();
    assert!(matches!(
        Frame::decode(&event).unwrap(),
        Frame::FileOwnerRetired { .. }
    ));
    let ack = service
        .control_request(reg, &event)
        .unwrap()
        .encode()
        .unwrap();
    call(&mut kernel, &sched, &caps, 3, api::REPLY, reg.epoch, &ack).unwrap();
    assert_eq!(
        call(&mut kernel, &sched, &caps, 3, api::NEXT, reg.epoch, &[]),
        Ok(None)
    );
    drop(binding);
    kernel.reconcile_namespaces();
    assert!(
        call(&mut kernel, &sched, &caps, 3, api::NEXT, reg.epoch, &[])
            .unwrap()
            .is_some()
    );
}

#[test]
fn retain_delivery_keeps_allocation_order_after_slot_reuse_and_rejects_unsent_replies() {
    let (mut kernel, sched, caps, identity) = setup();
    let reg = identity.registration;
    let first_id = kernel
        .queue_namespace_inner(identity, LOOKUP_METADATA, None)
        .unwrap();
    let second_id = kernel
        .queue_namespace_inner(identity, LOOKUP_METADATA, None)
        .unwrap();
    let first = kernel.remote[0].as_ref().unwrap().unsent[0].unwrap();
    let Frame::Retain {
        scope, request_id, ..
    } = first
    else {
        panic!()
    };
    assert_eq!(request_id, first_id);
    let refused = Frame::Refused {
        request: sunlight_ipc::vfs_rpc::retention::wire::PendingRetain::new(reg, scope, request_id)
            .unwrap(),
        reason: Refusal::Resolve,
    }
    .encode()
    .unwrap();
    assert!(call(
        &mut kernel,
        &sched,
        &caps,
        3,
        api::REPLY,
        reg.epoch,
        &refused
    )
    .is_err());
    assert_eq!(
        call(&mut kernel, &sched, &caps, 3, api::NEXT, reg.epoch, &[]),
        Ok(Some(first.encode().unwrap()))
    );
    assert_eq!(
        call(
            &mut kernel,
            &sched,
            &caps,
            3,
            api::REPLY,
            reg.epoch,
            &refused
        ),
        Ok(None)
    );
    let third_id = kernel
        .queue_namespace_inner(identity, LOOKUP_METADATA, None)
        .unwrap();
    assert!(first_id < second_id && second_id < third_id);
    for expected in [second_id, third_id] {
        let bytes = call(&mut kernel, &sched, &caps, 3, api::NEXT, reg.epoch, &[])
            .unwrap()
            .unwrap();
        assert!(
            matches!(Frame::decode(&bytes).unwrap(), Frame::Retain { request_id, .. } if request_id == expected)
        );
        assert_eq!(
            call(&mut kernel, &sched, &caps, 3, api::NEXT, reg.epoch, &[]),
            Ok(None)
        );
        let Frame::Retain {
            scope, request_id, ..
        } = Frame::decode(&bytes).unwrap()
        else {
            panic!()
        };
        let refused = Frame::Refused {
            request: sunlight_ipc::vfs_rpc::retention::wire::PendingRetain::new(
                reg, scope, request_id,
            )
            .unwrap(),
            reason: Refusal::Resolve,
        }
        .encode()
        .unwrap();
        assert_eq!(
            call(
                &mut kernel,
                &sched,
                &caps,
                3,
                api::REPLY,
                reg.epoch,
                &refused
            ),
            Ok(None)
        );
    }
    for _ in 0..REMOTE_LEASES {
        kernel.queue_namespace(identity, LOOKUP_METADATA).unwrap();
    }
    assert_eq!(
        kernel.queue_namespace(identity, LOOKUP_METADATA),
        Err(NamespaceRetentionError::Ownership(OwnershipError::Capacity))
    );
    assert_eq!(
        kernel.remote[0].as_ref().unwrap().ownership.outstanding(),
        REMOTE_LEASES
    );
}

struct Channel<'a> {
    kernel: &'a mut Runtime,
    sched: &'a Scheduler,
    caps: &'a CapabilityBroker,
    lose_ack: bool,
    old_epoch: u64,
    attempts: [usize; 2],
}

#[test]
fn caller_barrier_retries_blocks_later_retains_and_compensates_inflight_success() {
    let (mut kernel, mut sched, caps, identity) = setup();
    let reg = identity.registration;
    let mut service = Service::<4, 4, 2>::new();
    service.bind(reg).unwrap();
    kernel.queue_namespace(identity, LOOKUP_METADATA).unwrap();
    let bytes = call(&mut kernel, &sched, &caps, 3, api::NEXT, reg.epoch, &[])
        .unwrap()
        .unwrap();
    let Frame::Retain { scope, .. } = Frame::decode(&bytes).unwrap() else {
        panic!()
    };
    let namespace = DirectoryNamespace::new(sunlight_fs::Tmpfs::new(&[])).unwrap();
    service.install(reg, scope, &namespace, "/", "/").unwrap();
    let success = service.request(reg, &bytes).unwrap();
    let Frame::Retained(receipt) = success else {
        panic!()
    };
    kernel.queue_namespace(identity, LOOKUP_METADATA).unwrap();
    // Wrong incarnation events cannot affect this caller or its reservation.
    kernel.retire_process(Caller {
        incarnation: identity.caller.incarnation + 1,
        ..identity.caller
    });
    assert_eq!(
        call(&mut kernel, &sched, &caps, 3, api::NEXT, reg.epoch, &[]),
        Ok(None)
    );
    sched.processes[1].exiting = true;
    kernel.retire_process(identity.caller);
    kernel.retire_process(identity.caller);
    let event = Frame::CallerRetired {
        registration: reg,
        caller: identity.caller,
        event_id: 1,
    };
    let ack = Frame::CallerRetiredAck {
        registration: reg,
        caller: identity.caller,
        event_id: 1,
    }
    .encode()
    .unwrap();
    // An undelivered acknowledgement cannot open the barrier.
    assert!(call(&mut kernel, &sched, &caps, 3, api::REPLY, reg.epoch, &ack).is_err());
    let event_bytes = event.encode().unwrap();
    assert_eq!(
        call(&mut kernel, &sched, &caps, 3, api::NEXT, reg.epoch, &[]),
        Ok(Some(event_bytes))
    );
    // Lost output copy is retryable, unlike a retain output copy.
    assert_eq!(
        call(&mut kernel, &sched, &caps, 3, api::NEXT, reg.epoch, &[]),
        Ok(Some(event_bytes))
    );
    assert_eq!(
        service
            .control_request(reg, &event_bytes)
            .unwrap()
            .encode()
            .unwrap(),
        ack
    );
    // Losing an acknowledgement leaves the exact barrier ahead of later work.
    assert_eq!(
        call(&mut kernel, &sched, &caps, 3, api::NEXT, reg.epoch, &[]),
        Ok(Some(event_bytes))
    );
    assert_eq!(
        service
            .control_request(reg, &event_bytes)
            .unwrap()
            .encode()
            .unwrap(),
        ack
    );
    for word in 2..10 {
        let mut wrong = ack;
        wrong[word * 8] ^= 32;
        assert!(call(&mut kernel, &sched, &caps, 3, api::REPLY, reg.epoch, &wrong).is_err());
    }
    let mut wrong_id = ack;
    wrong_id[96] = 2;
    assert!(call(
        &mut kernel,
        &sched,
        &caps,
        3,
        api::REPLY,
        reg.epoch,
        &wrong_id
    )
    .is_err());
    assert!(call(&mut kernel, &sched, &caps, 10, api::REPLY, reg.epoch, &ack).is_err());
    // Already fetched success still owns a release and cannot install a namespace.
    call(
        &mut kernel,
        &sched,
        &caps,
        3,
        api::REPLY,
        reg.epoch,
        &success.encode().unwrap(),
    )
    .unwrap();
    assert!(kernel.namespaces.lookup(identity).is_err());
    assert!(service.metadata(reg, receipt).is_ok());
    call(&mut kernel, &sched, &caps, 3, api::REPLY, reg.epoch, &ack).unwrap();
    assert!(call(&mut kernel, &sched, &caps, 3, api::REPLY, reg.epoch, &ack).is_err());
    // The real shared loop refuses the queued retain after dropping policy pins,
    // then settles compensation without losing its retained lease.
    service.poll_control(&mut Channel {
        kernel: &mut kernel,
        sched: &sched,
        caps: &caps,
        lose_ack: false,
        old_epoch: reg.epoch,
        attempts: [0; 2],
    });
    assert_eq!(kernel.remote_owner(reg).unwrap().outstanding(), 0);
    assert!(service.metadata(reg, receipt).is_err());
}

#[test]
fn caller_delivery_capacity_precedes_preparation_and_retired_epoch_collection() {
    let (mut kernel, sched, caps, identity) = setup();
    let reg = identity.registration;
    // Settled/refused namespaces still reserve lifecycle capacity: userspace
    // may have policy pins even though the kernel has no remote lease.
    for n in 0..REMOTE_LEASES {
        let caller = Caller {
            pid: identity.caller.pid + n as u64,
            ..identity.caller
        };
        kernel
            .queue_namespace(AdmissionIdentity { caller, ..identity }, LOOKUP_METADATA)
            .unwrap();
        let bytes = call(&mut kernel, &sched, &caps, 3, api::NEXT, reg.epoch, &[])
            .unwrap()
            .unwrap();
        let Frame::Retain {
            scope, request_id, ..
        } = Frame::decode(&bytes).unwrap()
        else {
            panic!()
        };
        let refusal = Frame::Refused {
            request: sunlight_ipc::vfs_rpc::retention::wire::PendingRetain::new(
                reg, scope, request_id,
            )
            .unwrap(),
            reason: Refusal::Resolve,
        };
        call(
            &mut kernel,
            &sched,
            &caps,
            3,
            api::REPLY,
            reg.epoch,
            &refusal.encode().unwrap(),
        )
        .unwrap();
    }
    assert_eq!(kernel.remote_owner(reg).unwrap().outstanding(), 0);
    assert_eq!(
        kernel.queue_namespace(
            AdmissionIdentity {
                caller: Caller {
                    pid: 999,
                    ..identity.caller
                },
                ..identity
            },
            LOOKUP_METADATA
        ),
        Err(NamespaceRetentionError::Ownership(OwnershipError::Capacity))
    );
    // Every occupied row can retire without allocation, even at capacity.
    for n in 0..REMOTE_LEASES {
        kernel.retire_process(Caller {
            pid: identity.caller.pid + n as u64,
            ..identity.caller
        });
    }
    kernel.retire_registration(reg);
    assert!(kernel.remote_owner(reg).is_ok());
    let mut service = Service::<1, 1, 1>::new();
    service.bind(reg).unwrap();
    for turn in 0..8 {
        service.poll_control(&mut Channel {
            kernel: &mut kernel,
            sched: &sched,
            caps: &caps,
            lose_ack: false,
            old_epoch: reg.epoch,
            attempts: [0; 2],
        });
        if turn < 7 {
            assert!(kernel.remote_owner(reg).is_ok());
        }
    }
    assert!(kernel.remote_owner(reg).is_err());
    // Service replay policy remains retained after the local kernel row is gone.
    assert_eq!(service.registrations(), [Some(reg)]);
}
impl sunlight_vfs_server::runtime::control::Control for Channel<'_> {
    fn snapshot(&mut self) -> Option<Registration> {
        let bytes = call(self.kernel, self.sched, self.caps, 3, api::SNAPSHOT, 0, &[]).ok()??;
        api::decode_registration(&bytes[..api::REGISTRATION_BYTES]).ok()
    }
    fn status(&mut self, epoch: u64) -> Result<(Registration, bool), ()> {
        let bytes = call(
            self.kernel,
            self.sched,
            self.caps,
            3,
            api::STATUS,
            epoch,
            &[],
        )
        .map_err(|_| ())?
        .ok_or(())?;
        api::decode_status(&bytes[..api::STATUS_BYTES]).map_err(|_| ())
    }
    fn next(&mut self, epoch: u64) -> Result<Option<[u8; FRAME_BYTES]>, ()> {
        self.attempts[usize::from(epoch != self.old_epoch)] += 1;
        call(self.kernel, self.sched, self.caps, 3, api::NEXT, epoch, &[]).map_err(|_| ())
    }
    fn reply(&mut self, epoch: u64, bytes: &[u8; FRAME_BYTES]) -> bool {
        if self.lose_ack
            && matches!(
                Frame::decode(bytes),
                Ok(Frame::Released(_)
                    | Frame::CallerRetiredAck { .. }
                    | Frame::FileOwnerRetiredAck { .. })
            )
        {
            return true;
        }
        call(
            self.kernel,
            self.sched,
            self.caps,
            3,
            api::REPLY,
            epoch,
            bytes,
        )
        .is_ok()
    }
}

#[test]
fn shared_poll_retries_caller_barriers_across_epochs_and_preserves_reused_pid() {
    let (mut kernel, mut sched, caps, identity) = setup();
    let old = identity.registration;
    let namespace = DirectoryNamespace::new(sunlight_fs::Tmpfs::new(&[])).unwrap();
    let mut service = Service::<2, 2, 2>::new();
    service.bind(old).unwrap();
    kernel.queue_namespace(identity, LOOKUP_METADATA).unwrap();
    kernel.retire_process(identity.caller);
    kernel.retire_registration(old);
    let new = kernel.register(old.owner).unwrap();
    let next_identity = AdmissionIdentity {
        registration: new,
        caller: Caller {
            incarnation: identity.caller.incarnation + 1,
            ..identity.caller
        },
        ..identity
    };
    sched.processes[1].foreground.incarnation += 1;
    kernel
        .queue_namespace(next_identity, LOOKUP_METADATA)
        .unwrap();
    service.observe_active(new).unwrap();
    let Frame::Retain { scope, .. } = kernel
        .remote
        .iter()
        .flatten()
        .find(|r| r.registration == new)
        .unwrap()
        .unsent[0]
        .unwrap()
    else {
        panic!()
    };
    service.install(new, scope, &namespace, "/", "/").unwrap();
    let mut channel = Channel {
        kernel: &mut kernel,
        sched: &sched,
        caps: &caps,
        lose_ack: true,
        old_epoch: old.epoch,
        attempts: [0; 2],
    };
    service.poll_control(&mut channel);
    assert_eq!(channel.attempts, [16, 2]);
    assert!(channel.kernel.namespaces.lookup(next_identity).is_ok());
    assert_eq!(channel.kernel.remote_owner(old).unwrap().outstanding(), 1);
    channel.lose_ack = false;
    service.poll_control(&mut channel);
    assert!(channel.kernel.remote_owner(old).is_err());
    assert!(channel.kernel.namespaces.lookup(next_identity).is_ok());
    // A new lifetime can reuse the vacated caller event slot. Its identifier
    // never aliases an acknowledged event from the prior lifetime.
    channel.kernel.retire_process(next_identity.caller);
    let event = call(
        channel.kernel,
        channel.sched,
        channel.caps,
        3,
        api::NEXT,
        new.epoch,
        &[],
    )
    .unwrap()
    .unwrap();
    assert!(
        matches!(Frame::decode(&event).unwrap(), Frame::CallerRetired { caller, event_id: 1, .. } if caller == next_identity.caller)
    );
    let ack = service
        .control_request(new, &event)
        .unwrap()
        .encode()
        .unwrap();
    call(
        channel.kernel,
        channel.sched,
        channel.caps,
        3,
        api::REPLY,
        new.epoch,
        &ack,
    )
    .unwrap();
    let third = AdmissionIdentity {
        caller: Caller {
            incarnation: next_identity.caller.incarnation + 1,
            ..next_identity.caller
        },
        ..next_identity
    };
    channel
        .kernel
        .queue_namespace(third, LOOKUP_METADATA)
        .unwrap();
    channel.kernel.retire_process(third.caller);
    assert!(call(
        channel.kernel,
        channel.sched,
        channel.caps,
        3,
        api::REPLY,
        new.epoch,
        &ack
    )
    .is_err());
    let event = call(
        channel.kernel,
        channel.sched,
        channel.caps,
        3,
        api::NEXT,
        new.epoch,
        &[],
    )
    .unwrap()
    .unwrap();
    assert!(
        matches!(Frame::decode(&event).unwrap(), Frame::CallerRetired { caller, event_id: 2, .. } if caller == third.caller)
    );
}

#[test]
fn service_poll_retires_old_policy_retries_release_and_services_replacement() {
    let (mut kernel, sched, caps, identity) = setup();
    let old = identity.registration;
    let mut service = Service::<4, 4, 2>::new();
    service.bind(old).unwrap();
    kernel.queue_namespace(identity, LOOKUP_METADATA).unwrap();
    let retain = kernel.remote[0].as_ref().unwrap().unsent[0].unwrap();
    let Frame::Retain { scope, .. } = retain else {
        panic!()
    };
    let namespace = DirectoryNamespace::new(sunlight_fs::Tmpfs::new(&[])).unwrap();
    service.install(old, scope, &namespace, "/", "/").unwrap();
    let mut channel = Channel {
        kernel: &mut kernel,
        sched: &sched,
        caps: &caps,
        lose_ack: false,
        old_epoch: old.epoch,
        attempts: [0; 2],
    };
    service.poll_control(&mut channel);
    assert!(channel.kernel.namespaces.lookup(identity).is_ok());
    channel.kernel.retire_registration(old);
    let replacement = channel.kernel.register(old.owner).unwrap();
    let new_identity = AdmissionIdentity {
        registration: replacement,
        ..identity
    };
    // Unprovisioned replacement gets an exact refusal, while the old release
    // retries for its full bounded budget after simulated lost acknowledgements.
    channel
        .kernel
        .queue_namespace(new_identity, LOOKUP_METADATA)
        .unwrap();
    channel.lose_ack = true;
    channel.attempts = [0; 2];
    service.poll_control(&mut channel);
    assert_eq!(service.active(), Some(replacement));
    assert_eq!(channel.attempts, [16, 2]);
    assert_eq!(channel.kernel.remote_owner(old).unwrap().outstanding(), 1);
    assert_eq!(
        channel
            .kernel
            .remote_owner(replacement)
            .unwrap()
            .outstanding(),
        0
    );
    assert!(service.install(old, scope, &namespace, "/", "/").is_err());
    channel.lose_ack = false;
    service.poll_control(&mut channel);
    assert_eq!(channel.kernel.remote_owner(old).unwrap().outstanding(), 0);
    // Settlement preserves the live caller's delivery row in the old epoch.
    // Its later exit must still cross an acknowledged lifecycle barrier.
    channel.kernel.retire_process(identity.caller);
    service.poll_control(&mut channel);
    assert!(channel.kernel.remote_owner(old).is_err());
    // Missing kernel status after collection retains the service's replay row.
    service.poll_control(&mut channel);
    assert_eq!(service.registrations(), [Some(old), Some(replacement)]);
    assert_eq!(channel.kernel.registry.active(), Some(replacement));
}

#[test]
fn status_authenticates_retired_epoch_without_an_active_registration() {
    let (mut kernel, mut sched, mut caps, identity) = setup();
    let reg = identity.registration;
    kernel.queue_namespace(identity, LOOKUP_METADATA).unwrap();
    kernel.retire_registration(reg);
    assert!(call(&mut kernel, &sched, &caps, 3, api::SNAPSHOT, 0, &[]).is_err());
    let bytes = call(&mut kernel, &sched, &caps, 3, api::STATUS, reg.epoch, &[])
        .unwrap()
        .unwrap();
    assert_eq!(
        api::decode_status(&bytes[..api::STATUS_BYTES]),
        Ok((reg, true))
    );
    for field in 0..6 {
        match field {
            0 => sched.processes[0].trusted_vfs_service = false,
            1 => sched.processes[0].exiting = true,
            2 => sched.processes[0].foreground.incarnation += 1,
            3 => sched.processes[0].address_space.generation += 1,
            4 => caps.endpoints[0].1 = 10,
            _ => caps.endpoints.clear(),
        }
        assert!(call(&mut kernel, &sched, &caps, 3, api::STATUS, reg.epoch, &[]).is_err());
        sched.processes[0].trusted_vfs_service = true;
        sched.processes[0].exiting = false;
        sched.processes[0].foreground.incarnation = 4;
        sched.processes[0].address_space.generation = 5;
        caps.endpoints = alloc::vec![(0, 3)];
    }
    assert!(call(&mut kernel, &sched, &caps, 10, api::STATUS, reg.epoch, &[]).is_err());
    assert!(call(&mut kernel, &sched, &caps, 3, api::STATUS, 0, &[]).is_err());
    assert!(call(&mut kernel, &sched, &caps, 3, api::STATUS, reg.epoch, &[0]).is_err());
    assert!(call(
        &mut kernel,
        &sched,
        &caps,
        3,
        api::STATUS,
        reg.epoch + 1,
        &[]
    )
    .is_err());
    // Retirement closes service policy admission even with no replacement.
    let mut service = Service::<4, 4, 2>::new();
    service.bind(reg).unwrap();
    service.poll_control(&mut Channel {
        kernel: &mut kernel,
        sched: &sched,
        caps: &caps,
        lose_ack: false,
        old_epoch: reg.epoch,
        attempts: [0; 2],
    });
    assert_eq!(service.active(), None);
}

#[test]
fn root_policy_authorization_requires_staging_delivery_and_live_identity() {
    let (mut kernel, mut sched, mut caps, identity) = setup();
    sched.processes[1].vfs_root_metadata = true;
    sched.processes[1].vfs_boot_root = true;
    let r = identity.registration;
    kernel
        .queue_root_namespace(identity, LOOKUP_METADATA)
        .unwrap();
    let pending = kernel
        .remote
        .iter()
        .flatten()
        .next()
        .unwrap()
        .unsent
        .iter()
        .flatten()
        .next()
        .unwrap()
        .encode()
        .unwrap();
    let denied = Err(IpcError::InvalidCapability);
    assert_eq!(
        call(
            &mut kernel,
            &sched,
            &caps,
            3,
            api::AUTHORIZE_ROOT,
            r.epoch,
            &pending
        ),
        denied
    );
    let bytes = call(&mut kernel, &sched, &caps, 3, api::NEXT, r.epoch, &[])
        .unwrap()
        .unwrap();
    assert_eq!(
        call(
            &mut kernel,
            &sched,
            &caps,
            3,
            api::AUTHORIZE_ROOT,
            r.epoch,
            &bytes
        ),
        Ok(None)
    );
    assert_eq!(
        call(
            &mut kernel,
            &sched,
            &caps,
            10,
            api::AUTHORIZE_ROOT,
            r.epoch,
            &bytes
        ),
        denied
    );
    for word in 2..14 {
        let mut forged = bytes;
        forged[word * 8] ^= 1;
        assert_eq!(
            call(
                &mut kernel,
                &sched,
                &caps,
                3,
                api::AUTHORIZE_ROOT,
                r.epoch,
                &forged
            ),
            denied
        );
    }
    for field in 0..6 {
        match field {
            0 => sched.processes[1].uid += 1,
            1 => sched.processes[1].gid += 1,
            2 => sched.processes[1].foreground.incarnation += 1,
            3 => sched.processes[1].address_space.generation += 1,
            4 => sched.processes[1].exiting = true,
            _ => sched.processes[1].pid += 1,
        }
        assert_eq!(
            call(
                &mut kernel,
                &sched,
                &caps,
                3,
                api::AUTHORIZE_ROOT,
                r.epoch,
                &bytes
            ),
            denied
        );
        sched.processes[1].uid = identity.uid;
        sched.processes[1].gid = identity.gid;
        sched.processes[1].foreground.incarnation = identity.caller.incarnation;
        sched.processes[1].address_space.generation = identity.caller.address_space_generation;
        sched.processes[1].exiting = false;
        sched.processes[1].pid = identity.caller.pid as usize;
    }
    // Endpoint possession and the service role remain independently required.
    caps.endpoints.clear();
    assert_eq!(
        call(
            &mut kernel,
            &sched,
            &caps,
            3,
            api::AUTHORIZE_ROOT,
            r.epoch,
            &bytes
        ),
        denied
    );

    let (mut kernel, sched, caps, identity) = setup();
    kernel.queue_namespace(identity, LOOKUP_METADATA).unwrap();
    let bytes = call(&mut kernel, &sched, &caps, 3, api::NEXT, r.epoch, &[])
        .unwrap()
        .unwrap();
    // Ordinary staging is deliberately not a root-policy grant.
    assert_eq!(
        call(
            &mut kernel,
            &sched,
            &caps,
            3,
            api::AUTHORIZE_ROOT,
            r.epoch,
            &bytes
        ),
        denied
    );
}

struct RootChannel<'a> {
    channel: Channel<'a>,
    checks: usize,
    retire_on_recheck: Option<Caller>,
}

#[test]
fn initial_cwd_handoff_pins_nonroot_object_across_rename_and_mounts() {
    use sunlight_ipc::vfs_rpc::stat_transport;
    let _serial = crate::ipc::vfs_resources::TEST_LEDGER_LOCK.lock();
    let (mut kernel, mut sched, caps, identity) = setup();
    sched.processes[1].vfs_root_metadata = true;
    sched.processes[1].vfs_boot_root = true;
    sched.processes[1].cwd = alloc::string::String::from("/work/cwd");
    let namespace = crate::mounted_cwd_namespace(identity.uid, identity.gid);
    let shared = namespace.shared();
    let mut service = Service::<1, 2, 1>::new();
    {
        let mut control = RootChannel {
            channel: Channel {
                kernel: &mut kernel,
                sched: &sched,
                caps: &caps,
                old_epoch: identity.registration.epoch,
                attempts: [0; 2],
                lose_ack: false,
            },
            checks: 0,
            retire_on_recheck: None,
        };
        service.poll_root_control(&mut control, &namespace);
        assert_eq!(control.checks, 2);
    }
    shared.lock().rename("/work/cwd", "/work/moved").unwrap();
    // A replacement at the initial spelling cannot redirect the retained cwd.
    shared.lock().mkdir("/work/cwd", 0, 0, 0o755).unwrap();
    for (path, errno) in [("data", 0), ("/work/moved/data", 0), ("/work/cwd/data", 2)] {
        sched.processes[1].state = crate::sched::ProcessState::Running;
        let transfer = kernel.publish_stat(identity, path, &mut sched).unwrap();
        let mut input = [0; stat_transport::INPUT_BYTES];
        let (_, length) = kernel
            .take_stat(identity.registration, &mut input)
            .unwrap()
            .unwrap();
        let reply = service
            .stat(identity.registration, &input[..length])
            .unwrap();
        let reply = stat_transport::decode_reply(&reply).unwrap();
        assert_eq!(reply.words[2], errno);
        kernel.complete_stat(identity.registration, reply).unwrap();
        kernel.wake_stat_callers(&mut sched);
        drop(
            kernel
                .reap_stat(identity.caller, transfer.words()[1])
                .unwrap(),
        );
    }
}

#[test]
fn initial_cwd_authorization_and_late_receipt_recheck_exact_staged_directory() {
    let (mut kernel, mut sched, caps, identity) = setup();
    sched.processes[1].vfs_root_metadata = true;
    sched.processes[1].vfs_boot_root = true;
    sched.processes[1].cwd = alloc::string::String::from("/work");
    call(&mut kernel, &sched, &caps, 3, api::SNAPSHOT, 0, &[]).unwrap();
    let bytes = kernel
        .next_control(identity.registration)
        .unwrap()
        .unwrap()
        .encode()
        .unwrap();
    assert_eq!(
        kernel.authorize_root(identity.registration, &bytes, &sched),
        Ok("/work")
    );
    // The old root-only authorization cannot grant a different cwd.
    assert!(call(
        &mut kernel,
        &sched,
        &caps,
        3,
        api::AUTHORIZE_ROOT,
        identity.registration.epoch,
        &bytes
    )
    .is_err());
    let namespace = crate::mounted_namespace();
    namespace
        .shared()
        .lock()
        .mkdir("/work", 0, 0, 0o755)
        .unwrap();
    let mut service = Service::<1, 2, 1>::new();
    service.bind(identity.registration).unwrap();
    let Frame::Retain { scope, .. } = Frame::decode(&bytes).unwrap() else {
        panic!()
    };
    service
        .install(identity.registration, scope, &namespace, "/", "/work")
        .unwrap();
    let reply = service
        .request(identity.registration, &bytes)
        .unwrap()
        .encode()
        .unwrap();
    // Even a missed lifecycle notification cannot install mismatched policy.
    sched.processes[1].cwd = alloc::string::String::from("/");
    assert!(kernel
        .authorize_root(identity.registration, &bytes, &sched)
        .is_err());
    kernel
        .control_reply(identity.registration, &reply, &sched)
        .unwrap();
    assert!(kernel.namespaces.lookup(identity).is_err());
    assert!(matches!(
        kernel.next_control(identity.registration).unwrap(),
        Some(Frame::Release { .. })
    ));
}

#[test]
fn initial_cwd_lookup_refusal_drops_provisional_root_and_is_not_retried() {
    for directory in ["/missing", "/file", "/not/../canonical"] {
        let (mut kernel, mut sched, caps, identity) = setup();
        sched.processes[1].vfs_root_metadata = true;
        sched.processes[1].vfs_boot_root = true;
        sched.processes[1].cwd = alloc::string::String::from(directory);
        let namespace = crate::mounted_namespace();
        let shared = namespace.shared();
        let h = shared
            .lock()
            .create_file_exclusive("/file", 0, 0, 0o644)
            .unwrap();
        shared.lock().close(h).unwrap();
        let initial = shared.lock().open_handle_count();
        let mut service = Service::<1, 2, 1>::new();
        let mut control = RootChannel {
            channel: Channel {
                kernel: &mut kernel,
                sched: &sched,
                caps: &caps,
                old_epoch: identity.registration.epoch,
                attempts: [0; 2],
                lose_ack: false,
            },
            checks: 0,
            retire_on_recheck: None,
        };
        for _ in 0..3 {
            service.poll_root_control(&mut control, &namespace);
        }
        assert_eq!(control.checks, 1);
        assert_eq!(shared.lock().open_handle_count(), initial);
        assert!(control.channel.kernel.namespaces.lookup(identity).is_err());
        assert!(control
            .channel
            .kernel
            .next_control(identity.registration)
            .unwrap()
            .is_none());
    }
}
impl sunlight_vfs_server::runtime::control::Control for RootChannel<'_> {
    fn initial_cwd(
        &mut self,
        epoch: u64,
        bytes: &[u8; FRAME_BYTES],
    ) -> Option<[u8; api::CWD_BYTES]> {
        self.checks += 1;
        if self.checks == 2 {
            if let Some(caller) = self.retire_on_recheck.take() {
                self.channel.kernel.retire_process(caller);
            }
        }
        if epoch != self.channel.old_epoch {
            return None;
        }
        let registration = authenticate(
            self.channel.kernel,
            3,
            epoch,
            self.channel.sched,
            self.channel.caps,
        )
        .ok()?;
        api::encode_cwd(
            self.channel
                .kernel
                .authorize_root(registration, bytes, self.channel.sched)
                .ok()?,
        )
        .ok()
    }
    fn snapshot(&mut self) -> Option<Registration> {
        self.channel.snapshot()
    }
    fn status(&mut self, epoch: u64) -> Result<(Registration, bool), ()> {
        self.channel.status(epoch)
    }
    fn next(&mut self, epoch: u64) -> Result<Option<[u8; FRAME_BYTES]>, ()> {
        self.channel.next(epoch)
    }
    fn reply(&mut self, epoch: u64, bytes: &[u8; FRAME_BYTES]) -> bool {
        self.channel.reply(epoch, bytes)
    }
    fn authorize_root(&mut self, epoch: u64, bytes: &[u8; FRAME_BYTES]) -> bool {
        self.checks += 1;
        if self.checks == 2 {
            if let Some(caller) = self.retire_on_recheck.take() {
                self.channel.kernel.retire_process(caller);
            }
        }
        call(
            self.channel.kernel,
            self.channel.sched,
            self.channel.caps,
            3,
            api::AUTHORIZE_ROOT,
            epoch,
            bytes,
        )
        .is_ok()
    }
}

#[test]
fn copied_root_policy_pins_live_mounts_and_revalidates_before_retaining() {
    for raced in [false, true] {
        let (mut kernel, mut sched, caps, identity) = setup();
        sched.processes[1].vfs_root_metadata = true;
        sched.processes[1].vfs_boot_root = true;
        let r = identity.registration;
        let namespace = crate::mounted_namespace();
        let shared = namespace.shared();
        let initial = shared.lock().open_handle_count();
        let mut service = Service::<1, 2, 1>::new();
        kernel
            .queue_root_namespace(identity, LOOKUP_METADATA)
            .unwrap();
        let mut control = RootChannel {
            channel: Channel {
                kernel: &mut kernel,
                sched: &sched,
                caps: &caps,
                old_epoch: r.epoch,
                attempts: [0; 2],
                lose_ack: false,
            },
            checks: 0,
            retire_on_recheck: raced.then_some(identity.caller),
        };
        // No local fixture installation: real kernel control stages policy,
        // service pins shared mounts, reply installs a correlated kernel binding.
        service.poll_root_control(&mut control, &namespace);
        assert_eq!(control.checks, 2);
        if raced {
            assert!(control.channel.kernel.namespaces.lookup(identity).is_err());
            assert_eq!(shared.lock().open_handle_count(), initial);
        } else {
            assert!(control.channel.kernel.namespaces.lookup(identity).is_ok());
            assert_eq!(shared.lock().open_handle_count(), initial + 2);
            control.channel.kernel.retire_process(identity.caller);
            service.poll_root_control(&mut control, &namespace);
            assert!(control.channel.kernel.namespaces.lookup(identity).is_err());
            assert_eq!(shared.lock().open_handle_count(), initial);
        }
    }
}

#[test]
fn denied_root_profile_settles_without_pins_and_allows_later_work() {
    let (mut kernel, mut sched, caps, identity) = setup();
    let r = identity.registration;
    let namespace = crate::mounted_namespace();
    let shared = namespace.shared();
    let mut service = Service::<1, 2, 1>::new();
    kernel.queue_namespace(identity, LOOKUP_METADATA).unwrap();
    let mut control = RootChannel {
        channel: Channel {
            kernel: &mut kernel,
            sched: &sched,
            caps: &caps,
            old_epoch: r.epoch,
            attempts: [0; 2],
            lose_ack: false,
        },
        checks: 0,
        retire_on_recheck: None,
    };
    service.poll_root_control(&mut control, &namespace);
    assert_eq!(control.checks, 1);
    assert_eq!(shared.lock().open_handle_count(), 0);
    assert!(control.channel.kernel.namespaces.lookup(identity).is_err());
    assert!(control
        .channel
        .kernel
        .remote
        .iter()
        .flatten()
        .all(|r| r.delivered.is_none()));
    drop(control);
    sched.processes[1].vfs_root_metadata = true;
    sched.processes[1].vfs_boot_root = true;
    kernel
        .queue_root_namespace(identity, LOOKUP_METADATA)
        .unwrap();
    let mut control = RootChannel {
        channel: Channel {
            kernel: &mut kernel,
            sched: &sched,
            caps: &caps,
            old_epoch: r.epoch,
            attempts: [0; 2],
            lose_ack: false,
        },
        checks: 1,
        retire_on_recheck: None,
    };
    service.poll_root_control(&mut control, &namespace);
    assert_eq!(control.checks, 3);
    assert!(control.channel.kernel.namespaces.lookup(identity).is_ok());
    assert_eq!(shared.lock().open_handle_count(), 2);
}

#[test]
fn root_policy_refusal_releases_provisional_pins_when_lease_capacity_is_zero() {
    let (mut kernel, mut sched, caps, identity) = setup();
    sched.processes[1].vfs_root_metadata = true;
    sched.processes[1].vfs_boot_root = true;
    let r = identity.registration;
    let namespace = crate::mounted_namespace();
    let shared = namespace.shared();
    let mut service = Service::<1, 0, 1>::new();
    kernel
        .queue_root_namespace(identity, LOOKUP_METADATA)
        .unwrap();
    let mut control = RootChannel {
        channel: Channel {
            kernel: &mut kernel,
            sched: &sched,
            caps: &caps,
            old_epoch: r.epoch,
            attempts: [0; 2],
            lose_ack: false,
        },
        checks: 0,
        retire_on_recheck: None,
    };
    service.poll_root_control(&mut control, &namespace);
    assert_eq!(control.checks, 2);
    assert_eq!(shared.lock().open_handle_count(), 0);
    assert!(control.channel.kernel.namespaces.lookup(identity).is_err());
    assert!(control
        .channel
        .kernel
        .remote
        .iter()
        .flatten()
        .all(|r| r.delivered.is_none()));
}

#[test]
fn production_selection_uses_boot_marker_once_and_captures_credentials() {
    let (mut kernel, mut sched, caps, identity) = setup();
    let r = identity.registration;
    // UID, PID and image-independent ordinary registration do not select callers.
    call(&mut kernel, &sched, &caps, 3, api::SNAPSHOT, 0, &[]).unwrap();
    assert!(kernel.next_control(r).unwrap().is_none());
    sched.processes[1].vfs_root_metadata = true;
    sched.processes[1].vfs_boot_root = true;
    // Unauthenticated snapshot must not stage a selected caller either.
    assert!(call(&mut kernel, &sched, &caps, 10, api::SNAPSHOT, 0, &[]).is_err());
    assert!(kernel.next_control(r).unwrap().is_none());
    for _ in 0..3 {
        call(&mut kernel, &sched, &caps, 3, api::SNAPSHOT, 0, &[]).unwrap();
    }
    let remote = kernel.remote.iter().flatten().next().unwrap();
    assert_eq!(remote.local.iter().flatten().count(), 1);
    assert_eq!(remote.unsent.iter().flatten().count(), 1);
    assert!(remote.local.iter().flatten().any(|l| matches!(l,
        LocalLease::Preparing { identity: actual, grants: LOOKUP_METADATA,
            root_policy: true, .. } if *actual == identity)));
    let bytes = kernel.next_control(r).unwrap().unwrap().encode().unwrap();
    assert!(kernel.authorize_root(r, &bytes, &sched).is_ok());
    // Changed credentials cannot consume the staged decision or cause regrant.
    sched.processes[1].uid += 1;
    assert!(kernel.authorize_root(r, &bytes, &sched).is_err());
    call(&mut kernel, &sched, &caps, 3, api::SNAPSHOT, 0, &[]).unwrap();
    assert_eq!(
        kernel
            .remote
            .iter()
            .flatten()
            .next()
            .unwrap()
            .local
            .iter()
            .flatten()
            .count(),
        1
    );
}

#[test]
fn production_selected_caller_completes_shared_root_handoff_without_fixture_staging() {
    // Admission charges the boot-global ledger; serialize with other tests
    // that assert its exact balance while retaining pending resources.
    let _serial = crate::ipc::vfs_resources::TEST_LEDGER_LOCK.lock();
    let (mut kernel, mut sched, caps, identity) = setup();
    sched.processes[1].vfs_root_metadata = true;
    sched.processes[1].vfs_boot_root = true;
    let namespace = crate::mounted_namespace();
    let shared = namespace.shared();
    let initial = shared.lock().open_handle_count();
    let mut service = Service::<1, 2, 1>::new();
    let mut control = RootChannel {
        channel: Channel {
            kernel: &mut kernel,
            sched: &sched,
            caps: &caps,
            old_epoch: identity.registration.epoch,
            attempts: [0; 2],
            lose_ack: false,
        },
        checks: 0,
        retire_on_recheck: None,
    };
    for _ in 0..3 {
        service.poll_root_control(&mut control, &namespace);
    }
    assert_eq!(control.checks, 2);
    assert_eq!(shared.lock().open_handle_count(), initial + 2);
    assert!(control.channel.kernel.namespaces.lookup(identity).is_ok());
    assert!(control.channel.kernel.admit_stat(identity, "/").is_ok());
    control.channel.kernel.retire_process(identity.caller);
    // Real exit clears live eligibility before a later service turn.
    drop(control);
    sched.processes[1].exiting = true;
    let mut control = RootChannel {
        channel: Channel {
            kernel: &mut kernel,
            sched: &sched,
            caps: &caps,
            old_epoch: identity.registration.epoch,
            attempts: [0; 2],
            lose_ack: false,
        },
        checks: 0,
        retire_on_recheck: None,
    };
    service.poll_root_control(&mut control, &namespace);
    // Pending admission owns the lease until reap, then release drains both pins.
    assert_eq!(shared.lock().open_handle_count(), initial + 2);
    drop(control.channel.kernel.pending.reap_next().unwrap());
    control.channel.kernel.reconcile_namespaces();
    service.poll_root_control(&mut control, &namespace);
    assert_eq!(shared.lock().open_handle_count(), initial);
    assert!(control.channel.kernel.namespaces.lookup(identity).is_err());
}

#[test]
fn production_refusal_is_not_retried_and_replacement_gets_a_new_scope() {
    let (mut kernel, mut sched, caps, identity) = setup();
    sched.processes[1].vfs_root_metadata = true;
    sched.processes[1].vfs_boot_root = true;
    let r = identity.registration;
    call(&mut kernel, &sched, &caps, 3, api::SNAPSHOT, 0, &[]).unwrap();
    let frame = kernel.next_control(r).unwrap().unwrap();
    let Frame::Retain {
        scope, request_id, ..
    } = frame
    else {
        panic!()
    };
    let refusal = Frame::Refused {
        request: sunlight_ipc::vfs_rpc::retention::wire::PendingRetain::new(r, scope, request_id)
            .unwrap(),
        reason: Refusal::Resolve,
    }
    .encode()
    .unwrap();
    kernel.control_reply(r, &refusal, &sched).unwrap();
    for _ in 0..3 {
        call(&mut kernel, &sched, &caps, 3, api::SNAPSHOT, 0, &[]).unwrap();
        assert!(kernel.next_control(r).unwrap().is_none());
    }
    assert!(kernel.namespaces.lookup(identity).is_err());
    kernel.retire_registration(r);
    let next = kernel.register(r.owner).unwrap();
    call(&mut kernel, &sched, &caps, 3, api::SNAPSHOT, 0, &[]).unwrap();
    let Frame::Retain {
        registration,
        scope: new_scope,
        ..
    } = kernel.next_control(next).unwrap().unwrap()
    else {
        panic!()
    };
    assert_eq!(registration, next);
    assert!(next.epoch > r.epoch);
    assert_ne!(scope.namespace_generation, new_scope.namespace_generation);
}

#[test]
fn production_selection_skips_exiting_and_zero_lifetime_records() {
    for field in 0..4 {
        let (mut kernel, mut sched, caps, _) = setup();
        sched.processes[1].vfs_root_metadata = true;
        sched.processes[1].vfs_boot_root = true;
        match field {
            0 => sched.processes[1].exiting = true,
            1 => sched.processes[1].pid = 0,
            2 => sched.processes[1].foreground.incarnation = 0,
            _ => sched.processes[1].address_space.generation = 0,
        }
        call(&mut kernel, &sched, &caps, 3, api::SNAPSHOT, 0, &[]).unwrap();
        assert!(kernel
            .remote
            .iter()
            .flatten()
            .all(|r| r.local.iter().all(Option::is_none)));
    }
}

#[test]
fn copied_stat_round_trip_uses_retained_root_and_owns_result_through_resume() {
    use sunlight_ipc::vfs_rpc::{
        codec::Request,
        response::{Kind, Reply, ReplyContract},
        stat_transport as api, Completion,
    };
    let _serial = crate::ipc::vfs_resources::TEST_LEDGER_LOCK.lock();
    let (mut kernel, mut sched, caps, identity) = setup();
    sched.processes[1].vfs_root_metadata = true;
    sched.processes[1].vfs_boot_root = true;
    let namespace = crate::mounted_namespace();
    let shared = namespace.shared();
    let initial = shared.lock().open_handle_count();
    let mut service = Service::<1, 2, 1>::new();
    {
        let mut control = RootChannel {
            channel: Channel {
                kernel: &mut kernel,
                sched: &sched,
                caps: &caps,
                old_epoch: identity.registration.epoch,
                attempts: [0; 2],
                lose_ack: false,
            },
            checks: 0,
            retire_on_recheck: None,
        };
        service.poll_root_control(&mut control, &namespace);
    }
    let transfer = kernel.publish_stat(identity, "/", &mut sched).unwrap();
    assert_eq!(
        sched.processes[1].state,
        crate::sched::ProcessState::BlockedOnIpc
    );
    // An ordinary caller cannot authenticate as the service even with its epoch.
    assert_eq!(
        authenticate(&mut kernel, 10, identity.registration.epoch, &sched, &caps),
        Err(IpcError::InvalidCapability)
    );
    let authenticated =
        authenticate(&mut kernel, 3, identity.registration.epoch, &sched, &caps).unwrap();
    let mut packet = [0; api::PACKET_BYTES];
    let (_, length) = kernel
        .take_stat(authenticated, &mut packet[8..])
        .unwrap()
        .unwrap();
    packet[..8].copy_from_slice(&(length as u64).to_le_bytes());
    let input = api::input(&packet).unwrap();
    let reply = service.stat(authenticated, input).unwrap();
    kernel
        .complete_stat(authenticated, api::decode_reply(&reply).unwrap())
        .unwrap();
    kernel.wake_stat_callers(&mut sched);
    assert_eq!(sched.processes[1].state, crate::sched::ProcessState::Ready);
    let mut retired = kernel
        .reap_stat(identity.caller, transfer.words()[1])
        .unwrap();
    let metadata = ReplyContract::for_request(Request::Stat { path: "/" })
        .unwrap()
        .validate(
            Completion {
                errno: 0,
                output_bytes: 56,
            },
            retired.resources.output(),
        )
        .unwrap();
    assert!(matches!(metadata, Reply::Metadata(m) if m.kind == Kind::Directory));
    // Numeric context mutation cannot borrow a different caller or retained pair.
    let mut groups = [0; 32];
    let mut dispatch = sunlight_ipc::vfs_rpc::context::decode(input, &mut groups).unwrap();
    dispatch.context.namespace.root += 1;
    dispatch.context.namespace.directory += 1;
    let mut forged = [0; api::INPUT_BYTES];
    let length = dispatch.encode(&mut forged).unwrap();
    let refused = service.stat(authenticated, &forged[..length]).unwrap();
    assert_ne!(api::decode_reply(&refused).unwrap().words[2], 0);
    kernel.retire_process(identity.caller);
    assert_eq!(shared.lock().open_handle_count(), initial + 2);
    drop(retired);
    kernel.reconcile_namespaces();
    let mut control = RootChannel {
        channel: Channel {
            kernel: &mut kernel,
            sched: &sched,
            caps: &caps,
            old_epoch: identity.registration.epoch,
            attempts: [0; 2],
            lose_ack: false,
        },
        checks: 0,
        retire_on_recheck: None,
    };
    service.poll_root_control(&mut control, &namespace);
    assert_eq!(shared.lock().open_handle_count(), initial);
    assert_ne!(
        api::decode_reply(&service.stat(authenticated, input).unwrap())
            .unwrap()
            .words[2],
        0
    );
}

#[test]
fn copied_path_stat_uses_kernel_credentials_and_returns_correlated_posix_errors() {
    use sunlight_ipc::vfs_rpc::{lifecycle::State, stat_transport as api};
    let _serial = crate::ipc::vfs_resources::TEST_LEDGER_LOCK.lock();
    let (mut kernel, mut sched, caps, identity) = setup();
    sched.processes[1].vfs_root_metadata = true;
    sched.processes[1].vfs_boot_root = true;
    let namespace = crate::mounted_namespace();
    {
        let shared = namespace.shared();
        let mut fs = shared.lock();
        fs.mkdir("/allowed", identity.uid, identity.gid, 0o700)
            .unwrap();
        let h = fs
            .create_file_exclusive("/allowed/data", identity.uid, identity.gid, 0)
            .unwrap();
        fs.write(h, 0, b"kernel caller").unwrap();
        fs.close(h).unwrap();
        fs.mkdir("/denied", identity.uid + 1, identity.gid + 1, 0o700)
            .unwrap();
        let h = fs
            .create_file_exclusive("/denied/data", identity.uid, identity.gid, 0o777)
            .unwrap();
        fs.close(h).unwrap();
    }
    let mut service = Service::<1, 2, 1>::new();
    {
        let mut control = RootChannel {
            channel: Channel {
                kernel: &mut kernel,
                sched: &sched,
                caps: &caps,
                old_epoch: identity.registration.epoch,
                attempts: [0; 2],
                lose_ack: false,
            },
            checks: 0,
            retire_on_recheck: None,
        };
        service.poll_root_control(&mut control, &namespace);
    }
    let mut long = alloc::vec![b'/'; api::MAX_LOOKUP_PATH];
    let suffix = b"allowed/data";
    let start = long.len() - suffix.len();
    long[start..].copy_from_slice(suffix);
    let long = core::str::from_utf8(&long).unwrap();
    for (path, errno) in [
        (long, 0),
        ("allowed/./data", 0),
        ("/denied/data", 13),
        ("/not-present", 2),
        ("/allowed/data/..", 20),
    ] {
        sched.processes[1].state = crate::sched::ProcessState::Running;
        let transfer = kernel.publish_stat(identity, path, &mut sched).unwrap();
        let id = transfer.words()[1];
        let authenticated =
            authenticate(&mut kernel, 3, identity.registration.epoch, &sched, &caps).unwrap();
        let mut input = [0; api::INPUT_BYTES];
        let (delivered, length) = kernel
            .take_stat(authenticated, &mut input)
            .unwrap()
            .unwrap();
        assert_eq!(delivered, id);
        let reply = service.stat(authenticated, &input[..length]).unwrap();
        let reply = api::decode_reply(&reply).unwrap();
        assert_eq!(reply.words[1], id);
        assert_eq!(reply.words[2], errno);
        kernel.complete_stat(authenticated, reply).unwrap();
        kernel.wake_stat_callers(&mut sched);
        assert_eq!(sched.processes[1].state, crate::sched::ProcessState::Ready);
        let retired = kernel.reap_stat(identity.caller, id).unwrap();
        assert!(matches!(retired.state, State::Completed(c) if u64::from(c.errno) == errno));
        drop(retired);
    }
}

#[test]
fn cwd_withdrawal_drains_remote_profile_and_snapshot_cannot_regrant() {
    let _serial = crate::ipc::vfs_resources::TEST_LEDGER_LOCK.lock();
    let (mut kernel, mut sched, caps, identity) = setup();
    sched.processes[1].vfs_root_metadata = true;
    sched.processes[1].vfs_boot_root = true;
    let namespace = crate::mounted_namespace();
    let shared = namespace.shared();
    let initial = shared.lock().open_handle_count();
    let mut service = Service::<1, 2, 1>::new();
    let poll = |kernel: &mut Runtime, sched: &Scheduler, service: &mut Service<1, 2, 1>| {
        let mut control = RootChannel {
            channel: Channel {
                kernel,
                sched,
                caps: &caps,
                old_epoch: identity.registration.epoch,
                attempts: [0; 2],
                lose_ack: false,
            },
            checks: 0,
            retire_on_recheck: None,
        };
        service.poll_root_control(&mut control, &namespace);
    };
    poll(&mut kernel, &sched, &mut service);
    assert_eq!(shared.lock().open_handle_count(), initial + 2);
    let id = kernel
        .publish_stat(identity, "/", &mut sched)
        .unwrap()
        .words()[1];
    kernel.withdraw_root_profile(1, &mut sched);
    assert_eq!(sched.processes[1].state, crate::sched::ProcessState::Ready);
    assert!(kernel.namespaces.lookup(identity).is_err());
    // Caller retirement/barrier cannot release pins still held by cleanup.
    poll(&mut kernel, &sched, &mut service);
    assert_eq!(shared.lock().open_handle_count(), initial + 2);
    drop(kernel.pending.reap(id).unwrap());
    kernel.reconcile_namespaces();
    for _ in 0..3 {
        poll(&mut kernel, &sched, &mut service);
    }
    assert_eq!(shared.lock().open_handle_count(), initial);
    assert!(kernel
        .next_control(identity.registration)
        .unwrap()
        .is_none());
    assert!(kernel.admit_stat(identity, "/").is_err());
    assert_eq!(kernel.registry.active(), Some(identity.registration));
}

#[test]
fn cwd_withdrawal_before_or_during_provisioning_cannot_install_a_late_receipt() {
    for retained in [false, true] {
        let (mut kernel, mut sched, caps, identity) = setup();
        let r = identity.registration;
        sched.processes[1].vfs_root_metadata = true;
        sched.processes[1].vfs_boot_root = true;
        let namespace = crate::mounted_namespace();
        let shared = namespace.shared();
        let initial = shared.lock().open_handle_count();
        let mut service = Service::<1, 2, 1>::new();
        call(&mut kernel, &sched, &caps, 3, api::SNAPSHOT, 0, &[]).unwrap();
        let late = if retained {
            let request = kernel.next_control(r).unwrap().unwrap();
            let bytes = request.encode().unwrap();
            assert!(kernel.authorize_root(r, &bytes, &sched).is_ok());
            let Frame::Retain { scope, .. } = request else {
                panic!()
            };
            service.bind(r).unwrap();
            service.install(r, scope, &namespace, "/", "/").unwrap();
            Some(service.request(r, &bytes).unwrap().encode().unwrap())
        } else {
            None
        };
        kernel.withdraw_root_profile(1, &mut sched);
        if let Some(bytes) = late {
            // Identity still lives; ownership retirement must compensate even
            // though this receipt passed the service's final authorization.
            kernel.control_reply(r, &bytes, &sched).unwrap();
        }
        let mut control = RootChannel {
            channel: Channel {
                kernel: &mut kernel,
                sched: &sched,
                caps: &caps,
                old_epoch: r.epoch,
                attempts: [0; 2],
                lose_ack: false,
            },
            checks: 0,
            retire_on_recheck: None,
        };
        for _ in 0..3 {
            service.poll_root_control(&mut control, &namespace);
        }
        assert!(control.channel.kernel.namespaces.lookup(identity).is_err());
        assert!(control.channel.kernel.next_control(r).unwrap().is_none());
        assert_eq!(shared.lock().open_handle_count(), initial);
        assert!(!sched.processes[1].vfs_root_metadata);
    }
}

struct CwdChannel<'a> {
    kernel: &'a mut Runtime,
    sched: &'a mut Scheduler,
    caps: &'a CapabilityBroker,
    reject_commit: bool,
    legacy: Option<&'a mut sunlight_fs::Vfs>,
    file: Option<[u8; sunlight_ipc::vfs_rpc::file_transport::FRAME_BYTES]>,
}
impl sunlight_vfs_server::runtime::control::Control for CwdChannel<'_> {
    #[cfg(feature = "vfs_c2e_candidate")]
    fn file_reply(&mut self, epoch: u64, bytes: &[u8; sunlight_ipc::vfs_rpc::file_transport::FRAME_BYTES]) -> bool {
        authenticate(self.kernel, 3, epoch, self.sched, self.caps).is_ok_and(|registration| {
            self.kernel.reply_file(registration, bytes, self.sched).is_ok()
        })
    }

    fn file_next(&mut self, _: u64) -> Result<Option<[u8; sunlight_ipc::vfs_rpc::file_transport::FRAME_BYTES]>, ()> {
        Ok(self.file)
    }
    fn derive_source(&mut self, epoch: u64, bytes: &[u8; FRAME_BYTES]) -> Option<Receipt> {
        let r = authenticate(self.kernel, 3, epoch, self.sched, self.caps).ok()?;
        self.kernel.authorize_derivation(r, bytes, self.sched).ok()
    }
    fn snapshot(&mut self) -> Option<Registration> {
        let r = self.kernel.registry.active()?;
        self.kernel.stage_selected_roots(r, self.sched);
        Some(r)
    }
    fn status(&mut self, epoch: u64) -> Result<(Registration, bool), ()> {
        let r = self
            .kernel
            .remote
            .iter()
            .flatten()
            .find(|r| r.registration.epoch == epoch)
            .ok_or(())?;
        Ok((r.registration, r.retired))
    }
    fn next(&mut self, epoch: u64) -> Result<Option<[u8; FRAME_BYTES]>, ()> {
        control_runtime(self.kernel, 3, api::NEXT, epoch, &[], self.sched, self.caps)
            .map_err(|_| ())
    }
    fn reply(&mut self, epoch: u64, bytes: &[u8; FRAME_BYTES]) -> bool {
        if self.reject_commit {
            if let Ok(Frame::Retained(receipt)) = Frame::decode(bytes) {
                let derived = self.kernel.remote.iter().flatten().flat_map(|r| r.local.iter().flatten()).any(|l| matches!(l,
                    LocalLease::Preparing { id, derivation: Some(_), .. } if *id == receipt.request_id()));
                if derived {
                    let p = self
                        .sched
                        .processes
                        .iter_mut()
                        .find(|p| process_identity(p) == receipt.scope().caller)
                        .unwrap();
                    p.uid += 1;
                }
            }
        }
        control_runtime(
            self.kernel,
            3,
            api::REPLY,
            epoch,
            bytes,
            self.sched,
            self.caps,
        )
        .is_ok()
    }
    fn initial_cwd(
        &mut self,
        epoch: u64,
        bytes: &[u8; FRAME_BYTES],
    ) -> Option<[u8; api::CWD_BYTES]> {
        let r = authenticate(self.kernel, 3, epoch, self.sched, self.caps).ok()?;
        api::encode_cwd(self.kernel.authorize_root(r, bytes, self.sched).ok()?).ok()
    }
    fn chdir(
        &mut self,
        epoch: u64,
        bytes: &[u8; FRAME_BYTES],
    ) -> Option<[u8; sunlight_ipc::vfs_rpc::chdir_transport::CONTEXT_BYTES]> {
        super::super::chdir::context_runtime(self.kernel, 3, epoch, bytes, self.sched, self.caps)
            .ok()
    }
    fn finish_chdir(
        &mut self,
        epoch: u64,
        bytes: &[u8; FRAME_BYTES],
        errno: u64,
        cwd: &str,
    ) -> bool {
        use sunlight_ipc::vfs_rpc::chdir_transport as cwd_api;
        let mut packet = [0; cwd_api::FINISH_BYTES];
        packet[..FRAME_BYTES].copy_from_slice(bytes);
        packet[FRAME_BYTES..FRAME_BYTES + 8].copy_from_slice(&errno.to_le_bytes());
        packet[FRAME_BYTES + 8..].copy_from_slice(&api::encode_cwd(cwd).unwrap());
        if self.reject_commit {
            self.sched.processes[1].uid += 1;
        }
        let legacy = self.legacy.as_deref_mut();
        super::super::chdir::finish_with_legacy(
            self.kernel, 3, epoch, &packet, self.sched, self.caps,
            |sched, waiter, path, commit| {
                let caller = &sched.processes[waiter];
                let cred = sunlight_fs::Credential { uid: caller.uid, gid: caller.gid };
                legacy.ok_or(5u64)?.with_legacy_cwd(
                    path, &cred, &mut |_| Ok(()), |path| commit(sched, path),
                ).map_err(|error| match error {
                    sunlight_fs::FsError::NotFound => 2u64,
                    sunlight_fs::FsError::PermissionDenied => 13,
                    _ => 5,
                })?
            },
        ).is_ok()
    }
}

#[test]
fn copied_chdir_rotates_scope_commits_cwd_and_preserves_lookup_failures() {
    use super::super::chdir::chdir_call_runtime;
    let _serial = crate::ipc::vfs_resources::TEST_LEDGER_LOCK.lock();
    let (mut kernel, mut sched, caps, identity) = setup();
    sched.processes[1].vfs_root_metadata = true;
    sched.processes[1].vfs_boot_root = true;
    sched.processes[1].cwd = alloc::string::String::from("/work/cwd");
    let namespace = crate::mounted_cwd_namespace(identity.uid, identity.gid);
    let mut service = Service::<1, 8, 1>::new();
    service.poll_root_control(
        &mut CwdChannel {
            kernel: &mut kernel,
            sched: &mut sched,
            caps: &caps,
            reject_commit: false,
            legacy: None,
            file: None,
        },
        &namespace,
    );
    let before = kernel.namespaces.lookup(identity).unwrap();
    let old = before.validate(identity).unwrap();
    for (path, errno) in [("missing", 2), ("data", 20)] {
        assert_eq!(
            chdir_call_runtime(&mut kernel, 10, path, &mut sched, &caps),
            Ok(None)
        );
        assert_eq!(
            sched.processes[1].state,
            crate::sched::ProcessState::BlockedOnIpc
        );
        // Captured request cannot be substituted while parked.
        assert!(chdir_call_runtime(&mut kernel, 10, "/", &mut sched, &caps).is_err());
        service.poll_root_control(
            &mut CwdChannel {
                kernel: &mut kernel,
                sched: &mut sched,
                caps: &caps,
                reject_commit: false,
                legacy: None,
                file: None,
            },
            &namespace,
        );
        assert_eq!(sched.processes[1].state, crate::sched::ProcessState::Ready);
        assert_eq!(
            chdir_call_runtime(&mut kernel, 10, path, &mut sched, &caps),
            Ok(Some(errno))
        );
        assert_eq!(
            kernel
                .namespaces
                .lookup(identity)
                .unwrap()
                .validate(identity),
            Ok(old)
        );
        assert_eq!(sched.processes[1].cwd, "/work/cwd");
    }
    namespace
        .shared()
        .lock()
        .rename("/work/cwd", "/work/moved")
        .unwrap();
    // Resolve from the retained directory object after rename, including mounts.
    for (path, expected) in [(".", "/work/moved"), ("..", "/work"), ("/", "/")] {
        assert_eq!(
            chdir_call_runtime(&mut kernel, 10, path, &mut sched, &caps),
            Ok(None)
        );
        service.poll_root_control(
            &mut CwdChannel {
                kernel: &mut kernel,
                sched: &mut sched,
                caps: &caps,
                reject_commit: false,
                legacy: None,
                file: None,
            },
            &namespace,
        );
        assert_eq!(
            chdir_call_runtime(&mut kernel, 10, path, &mut sched, &caps),
            Ok(Some(0))
        );
        assert_eq!(sched.processes[1].cwd, expected);
        assert!(
            kernel
                .namespaces
                .lookup(identity)
                .unwrap()
                .validate(identity)
                .unwrap()
                .generation
                > old.generation
        );
    }
    assert!(before.validate(identity).is_err());
    drop(before);
    kernel.reconcile_namespaces();
    service.poll_root_control(
        &mut CwdChannel {
            kernel: &mut kernel,
            sched: &mut sched,
            caps: &caps,
            reject_commit: false,
            legacy: None,
            file: None,
        },
        &namespace,
    );
    assert_eq!(namespace.shared().lock().open_handle_count(), 2);
    // The replacement can dispatch from the new pinned cwd immediately.
    let transfer = kernel.admit_stat(identity, "work/moved/data").unwrap();
    let input = kernel
        .pending
        .resources(transfer.words()[1])
        .unwrap()
        .input();
    let reply = service.stat(identity.registration, input).unwrap();
    assert_eq!(
        sunlight_ipc::vfs_rpc::stat_transport::decode_reply(&reply)
            .unwrap()
            .words[2],
        0
    );
    kernel.pending.retire_caller(identity.caller);
    drop(kernel.pending.reap(transfer.words()[1]));
}

#[test]
fn copied_directory_and_root_authentication_lifecycle_timeout_and_no_replay() {
    use super::super::chdir::context_runtime;
    let _serial = crate::ipc::vfs_resources::TEST_LEDGER_LOCK.lock();
    use sunlight_ipc::vfs_rpc::chdir_transport::Kind;
    for kind in [Kind::Directory, Kind::Root] {
        let chdir_call_runtime = |runtime: &mut Runtime,
                                  pid,
                                  path: &str,
                                  sched: &mut Scheduler,
                                  caps: &CapabilityBroker| {
            super::super::chdir::transition_call_runtime(runtime, pid, path, kind, sched, caps)
        };
        for delivered in [false, true] {
            let (mut kernel, mut sched, caps, mut identity) = setup();
            if kind == Kind::Root {
                identity.uid = 0;
                sched.processes[1].uid = 0;
            }
            sched.processes[1].vfs_root_metadata = true;
            sched.processes[1].vfs_boot_root = true;
            let namespace = crate::mounted_namespace();
            let mut service = Service::<1, 8, 1>::new();
            service.poll_root_control(
                &mut CwdChannel {
                    kernel: &mut kernel,
                    sched: &mut sched,
                    caps: &caps,
                    reject_commit: false,
                    legacy: None,
                    file: None,
                },
                &namespace,
            );
            assert_eq!(
                chdir_call_runtime(&mut kernel, 10, "/", &mut sched, &caps),
                Ok(None)
            );
            let mut late = None;
            if delivered {
                let bytes = kernel
                    .next_control(identity.registration)
                    .unwrap()
                    .unwrap()
                    .encode()
                    .unwrap();
                assert!(context_runtime(
                    &mut kernel,
                    10,
                    identity.registration.epoch,
                    &bytes,
                    &sched,
                    &caps
                )
                .is_err());
                assert!(context_runtime(
                    &mut kernel,
                    3,
                    identity.registration.epoch + 1,
                    &bytes,
                    &sched,
                    &caps
                )
                .is_err());
                assert!(context_runtime(
                    &mut kernel,
                    3,
                    identity.registration.epoch,
                    &bytes,
                    &sched,
                    &caps
                )
                .is_ok());
                late = Some(bytes);
                let mut wrong = bytes;
                wrong[FRAME_BYTES - 1] ^= 1;
                assert!(context_runtime(
                    &mut kernel,
                    3,
                    identity.registration.epoch,
                    &wrong,
                    &sched,
                    &caps
                )
                .is_err());
            }
            sched.global_tick = 5 * crate::timekeeping::TICK_HZ;
            kernel.poll_chdirs(&mut sched, 5 * crate::timekeeping::TICK_HZ);
            assert_eq!(sched.processes[1].state, crate::sched::ProcessState::Ready);
            assert_eq!(
                chdir_call_runtime(&mut kernel, 10, "/", &mut sched, &caps),
                Ok(Some(110))
            );
            sched.processes[1].state = crate::sched::ProcessState::Running;
            assert!(chdir_call_runtime(&mut kernel, 10, "/", &mut sched, &caps).is_err());
            assert!(kernel.namespaces.lookup(identity).is_err());
            assert_eq!(sched.processes[1].cwd, "/");
            if let Some(bytes) = late {
                let refusal = service
                    .control_request(identity.registration, &bytes)
                    .unwrap()
                    .encode()
                    .unwrap();
                kernel
                    .control_reply(identity.registration, &refusal, &sched)
                    .unwrap();
            }
            service.poll_root_control(
                &mut CwdChannel {
                    kernel: &mut kernel,
                    sched: &mut sched,
                    caps: &caps,
                    reject_commit: false,
                    legacy: None,
                    file: None,
                },
                &namespace,
            );
            assert_eq!(namespace.shared().lock().open_handle_count(), 0);
            assert_eq!(
                kernel
                    .remote
                    .iter()
                    .flatten()
                    .map(|r| r.ownership.outstanding())
                    .sum::<usize>(),
                0
            );
        }
    }
}

#[test]
fn copied_directory_and_root_rejected_commit_releases_provisional_service_lease() {
    let _serial = crate::ipc::vfs_resources::TEST_LEDGER_LOCK.lock();
    use sunlight_ipc::vfs_rpc::chdir_transport::Kind;
    for kind in [Kind::Directory, Kind::Root] {
        let chdir_call_runtime = |runtime: &mut Runtime,
                                  pid,
                                  path: &str,
                                  sched: &mut Scheduler,
                                  caps: &CapabilityBroker| {
            super::super::chdir::transition_call_runtime(runtime, pid, path, kind, sched, caps)
        };
        let (mut kernel, mut sched, caps, mut identity) = setup();
        if kind == Kind::Root {
            identity.uid = 0;
            sched.processes[1].uid = 0;
        }
        sched.processes[1].vfs_root_metadata = true;
        sched.processes[1].vfs_boot_root = true;
        let namespace = crate::mounted_cwd_namespace(identity.uid, identity.gid);
        let mut service = Service::<1, 8, 1>::new();
        service.poll_root_control(
            &mut CwdChannel {
                kernel: &mut kernel,
                sched: &mut sched,
                caps: &caps,
                reject_commit: false,
                legacy: None,
                file: None,
            },
            &namespace,
        );
        assert_eq!(
            chdir_call_runtime(&mut kernel, 10, "/work/cwd", &mut sched, &caps),
            Ok(None)
        );
        service.poll_root_control(
            &mut CwdChannel {
                kernel: &mut kernel,
                sched: &mut sched,
                caps: &caps,
                reject_commit: true,
                legacy: None,
                file: None,
            },
            &namespace,
        );
        assert_eq!(sched.processes[1].cwd, "/");
        // The original lease remains usable. Rejection completes the waiter
        // without waiting for a timeout that would retire the service.
        sched.processes[1].uid = identity.uid;
        assert_eq!(namespace.shared().lock().open_handle_count(), 2);
        assert_eq!(sched.processes[1].state, crate::sched::ProcessState::Ready);
        assert_eq!(
            chdir_call_runtime(&mut kernel, 10, "/work/cwd", &mut sched, &caps),
            Ok(Some(5))
        );
        let transfer = kernel.admit_stat(identity, "/work/cwd/data").unwrap();
        let input = kernel.pending.resources(transfer.words()[1]).unwrap().input();
        let reply = service.stat(identity.registration, input).unwrap();
        assert_eq!(
            sunlight_ipc::vfs_rpc::stat_transport::decode_reply(&reply).unwrap().words[2],
            0
        );
        kernel.pending.retire_caller(identity.caller);
        drop(kernel.pending.reap(transfer.words()[1]));
    }
}

#[test]
fn copied_directory_and_root_lease_exhaustion_restores_old_policy_and_cwd() {
    use super::super::chdir::resume_runtime;
    let _serial = crate::ipc::vfs_resources::TEST_LEDGER_LOCK.lock();
    use sunlight_ipc::vfs_rpc::chdir_transport::Kind;
    for kind in [Kind::Directory, Kind::Root] {
        let chdir_call_runtime = |runtime: &mut Runtime,
                                  pid,
                                  path: &str,
                                  sched: &mut Scheduler,
                                  caps: &CapabilityBroker| {
            super::super::chdir::transition_call_runtime(runtime, pid, path, kind, sched, caps)
        };
        let (mut kernel, mut sched, caps, mut identity) = setup();
        if kind == Kind::Root {
            identity.uid = 0;
            sched.processes[1].uid = 0;
        }
        sched.processes[1].vfs_root_metadata = true;
        sched.processes[1].vfs_boot_root = true;
        let namespace = crate::mounted_cwd_namespace(identity.uid, identity.gid);
        // The old lease consumes the only slot. A transition may pin provisionally
        // but cannot retain it; both sides must preserve the old directory.
        let mut service = Service::<1, 1, 1>::new();
        service.poll_root_control(
            &mut CwdChannel {
                kernel: &mut kernel,
                sched: &mut sched,
                caps: &caps,
                reject_commit: false,
                legacy: None,
                file: None,
            },
            &namespace,
        );
        let mut path = alloc::string::String::from("/work/cwd");
        assert_eq!(
            chdir_call_runtime(&mut kernel, 10, &path, &mut sched, &caps),
            Ok(None)
        );
        path.clear();
        path.push_str("/different-user-bytes");
        // A nested syscall origin cannot steal the parked operation.
        assert!(resume_runtime(&mut kernel, 10, (1, 2), kind, &mut sched).is_err());
        let other_kind = if kind == Kind::Root {
            Kind::Directory
        } else {
            Kind::Root
        };
        assert!(resume_runtime(&mut kernel, 10, (0, 0), other_kind, &mut sched).is_err());
        service.poll_root_control(
            &mut CwdChannel {
                kernel: &mut kernel,
                sched: &mut sched,
                caps: &caps,
                reject_commit: false,
                legacy: None,
                file: None,
            },
            &namespace,
        );
        // Completion consumes the captured operation without rereading user bytes.
        assert_eq!(
            resume_runtime(&mut kernel, 10, (0, 0), kind, &mut sched),
            Ok(Some(Some(12)))
        );
        assert_eq!(sched.processes[1].cwd, "/");
        assert!(kernel.namespaces.lookup(identity).is_ok());
        assert_eq!(namespace.shared().lock().open_handle_count(), 2);
        let transfer = kernel.admit_stat(identity, "work/cwd/data").unwrap();
        let input = kernel
            .pending
            .resources(transfer.words()[1])
            .unwrap()
            .input();
        let reply = service.stat(identity.registration, input).unwrap();
        assert_eq!(
            sunlight_ipc::vfs_rpc::stat_transport::decode_reply(&reply)
                .unwrap()
                .words[2],
            0
        );
        kernel.pending.retire_caller(identity.caller);
        drop(kernel.pending.reap(transfer.words()[1]));
    }
}

#[test]
fn copied_directory_and_root_service_loss_wakes_once_and_respects_nested_ipc() {
    let _serial = crate::ipc::vfs_resources::TEST_LEDGER_LOCK.lock();
    use sunlight_ipc::vfs_rpc::chdir_transport::Kind;
    for kind in [Kind::Directory, Kind::Root] {
        let chdir_call_runtime = |runtime: &mut Runtime,
                                  pid,
                                  path: &str,
                                  sched: &mut Scheduler,
                                  caps: &CapabilityBroker| {
            super::super::chdir::transition_call_runtime(runtime, pid, path, kind, sched, caps)
        };
        let (mut kernel, mut sched, caps, mut identity) = setup();
        if kind == Kind::Root {
            identity.uid = 0;
            sched.processes[1].uid = 0;
        }
        sched.processes[1].vfs_root_metadata = true;
        sched.processes[1].vfs_boot_root = true;
        let namespace = crate::mounted_namespace();
        let mut service = Service::<1, 8, 1>::new();
        service.poll_root_control(
            &mut CwdChannel {
                kernel: &mut kernel,
                sched: &mut sched,
                caps: &caps,
                reject_commit: false,
                legacy: None,
                file: None,
            },
            &namespace,
        );
        assert_eq!(
            chdir_call_runtime(&mut kernel, 10, "/", &mut sched, &caps),
            Ok(None)
        );
        sched.processes[1].pending_call = Some(());
        kernel.retire_registration(identity.registration);
        kernel.poll_chdirs(&mut sched, 0);
        assert_eq!(
            sched.processes[1].state,
            crate::sched::ProcessState::BlockedOnIpc
        );
        sched.processes[1].pending_call = None;
        kernel.poll_chdirs(&mut sched, 0);
        assert_eq!(sched.processes[1].state, crate::sched::ProcessState::Ready);
        sched.processes[1].state = crate::sched::ProcessState::BlockedOnIpc;
        kernel.poll_chdirs(&mut sched, 0);
        assert_eq!(
            sched.processes[1].state,
            crate::sched::ProcessState::BlockedOnIpc
        );
        assert_eq!(
            chdir_call_runtime(&mut kernel, 10, "/", &mut sched, &caps),
            Ok(Some(5))
        );
    }
}

#[test]
fn copied_root_narrowing_resets_cwd_and_keeps_exact_mounted_root_after_rename() {
    use super::super::chdir::transition_call_runtime;
    use sunlight_ipc::vfs_rpc::chdir_transport::Kind;
    let _serial = crate::ipc::vfs_resources::TEST_LEDGER_LOCK.lock();
    let (mut kernel, mut sched, caps, mut identity) = setup();
    sched.processes[1].vfs_root_metadata = true;
    sched.processes[1].vfs_boot_root = true;
    sched.processes[1].uid = 0;
    identity.uid = 0;
    sched.processes[1].cwd = alloc::string::String::from("/work/cwd");
    let namespace = crate::mounted_cwd_namespace(0, identity.gid);
    let mut service = Service::<1, 8, 1>::new();
    service.poll_root_control(
        &mut CwdChannel {
            kernel: &mut kernel,
            sched: &mut sched,
            caps: &caps,
            reject_commit: false,
            legacy: None,
            file: None,
        },
        &namespace,
    );
    let old = kernel.namespaces.lookup(identity).unwrap();
    for (path, errno) in [("missing", 2), ("data", 20), (".", 0)] {
        assert_eq!(
            transition_call_runtime(&mut kernel, 10, path, Kind::Root, &mut sched, &caps),
            Ok(None)
        );
        // The same pathname cannot substitute a different operation kind.
        assert!(
            transition_call_runtime(&mut kernel, 10, path, Kind::Directory, &mut sched, &caps)
                .is_err()
        );
        service.poll_root_control(
            &mut CwdChannel {
                kernel: &mut kernel,
                sched: &mut sched,
                caps: &caps,
                reject_commit: false,
                legacy: None,
                file: None,
            },
            &namespace,
        );
        assert_eq!(
            transition_call_runtime(&mut kernel, 10, path, Kind::Root, &mut sched, &caps),
            Ok(Some(errno))
        );
        assert_eq!(
            sched.processes[1].cwd,
            if errno == 0 { "/" } else { "/work/cwd" }
        );
        if errno != 0 {
            assert!(old.validate(identity).is_ok());
        }
    }
    assert!(old.validate(identity).is_err());
    drop(old);
    namespace
        .shared()
        .lock()
        .rename("/work/cwd", "/work/moved")
        .unwrap();
    namespace
        .shared()
        .lock()
        .mkdir("/work/cwd", 0, 0, 0o755)
        .unwrap();
    // Absolute and relative requests stay inside the exact retained root.
    for (path, errno) in [("/data", 0), ("../../data", 0), ("/work/moved/data", 2)] {
        let transfer = kernel.admit_stat(identity, path).unwrap();
        let reply = service
            .stat(
                identity.registration,
                kernel
                    .pending
                    .resources(transfer.words()[1])
                    .unwrap()
                    .input(),
            )
            .unwrap();
        assert_eq!(
            sunlight_ipc::vfs_rpc::stat_transport::decode_reply(&reply)
                .unwrap()
                .words[2],
            errno
        );
        kernel.pending.retire_caller(identity.caller);
        drop(kernel.pending.reap(transfer.words()[1]));
    }
    // Live cwd and subsequent root transitions stay beneath the narrowed root.
    for kind in [Kind::Directory, Kind::Root] {
        assert_eq!(
            transition_call_runtime(&mut kernel, 10, "../..", kind, &mut sched, &caps),
            Ok(None)
        );
        service.poll_root_control(
            &mut CwdChannel {
                kernel: &mut kernel,
                sched: &mut sched,
                caps: &caps,
                reject_commit: false,
                legacy: None,
                file: None,
            },
            &namespace,
        );
        assert_eq!(
            transition_call_runtime(&mut kernel, 10, "../..", kind, &mut sched, &caps),
            Ok(Some(0))
        );
        assert_eq!(sched.processes[1].cwd, "/");
    }
    kernel.reconcile_namespaces();
    service.poll_root_control(
        &mut CwdChannel {
            kernel: &mut kernel,
            sched: &mut sched,
            caps: &caps,
            reject_commit: false,
            legacy: None,
            file: None,
        },
        &namespace,
    );
    assert_eq!(namespace.shared().lock().open_handle_count(), 2);
}

#[test]
fn copied_root_requires_selected_profile_and_independent_uid_zero_authority() {
    use super::super::chdir::{context_runtime, transition_call_runtime};
    use sunlight_ipc::vfs_rpc::chdir_transport::{decode_context, Kind};
    let _serial = crate::ipc::vfs_resources::TEST_LEDGER_LOCK.lock();
    let (mut kernel, mut sched, caps, mut identity) = setup();
    sched.processes[1].uid = 0;
    assert!(transition_call_runtime(&mut kernel, 10, "/", Kind::Root, &mut sched, &caps).is_err());
    sched.processes[1].vfs_root_metadata = true;
    sched.processes[1].vfs_boot_root = true;
    sched.processes[1].uid = identity.uid;
    assert_eq!(
        transition_call_runtime(&mut kernel, 10, "/", Kind::Root, &mut sched, &caps),
        Ok(Some(13))
    );
    assert!(kernel.chdirs.iter().all(Option::is_none));
    sched.processes[1].uid = 0;
    identity.uid = 0;
    let namespace = crate::mounted_cwd_namespace(0, identity.gid);
    let mut service = Service::<1, 8, 1>::new();
    service.poll_root_control(
        &mut CwdChannel {
            kernel: &mut kernel,
            sched: &mut sched,
            caps: &caps,
            reject_commit: false,
            legacy: None,
            file: None,
        },
        &namespace,
    );
    assert_eq!(
        transition_call_runtime(&mut kernel, 10, "/work", Kind::Root, &mut sched, &caps),
        Ok(None)
    );
    let retain = kernel
        .next_control(identity.registration)
        .unwrap()
        .unwrap()
        .encode()
        .unwrap();
    assert!(kernel
        .authorize_root(identity.registration, &retain, &sched)
        .is_err());
    assert!(context_runtime(
        &mut kernel,
        10,
        identity.registration.epoch,
        &retain,
        &sched,
        &caps
    )
    .is_err());
    let bytes = context_runtime(
        &mut kernel,
        3,
        identity.registration.epoch,
        &retain,
        &sched,
        &caps,
    )
    .unwrap();
    assert_eq!(decode_context(&bytes).unwrap().kind, Kind::Root);
    sched.processes[1].uid = 1;
    assert!(context_runtime(
        &mut kernel,
        3,
        identity.registration.epoch,
        &retain,
        &sched,
        &caps
    )
    .is_err());
    sched.processes[1].uid = 0;
    let context = decode_context(&bytes).unwrap();
    let Frame::Retain { scope, .. } = Frame::decode(&retain).unwrap() else {
        panic!()
    };
    service
        .change_root(
            identity.registration,
            context.old,
            scope,
            context.path,
            sunlight_fs::Credential {
                uid: context.uid,
                gid: context.gid,
            },
            || true,
        )
        .unwrap();
    let retained = service
        .request(identity.registration, &retain)
        .unwrap()
        .encode()
        .unwrap();
    // Ordinary retention replies cannot bypass the transaction's finish checks.
    assert!(kernel
        .control_reply(identity.registration, &retained, &sched)
        .is_err());
    use sunlight_ipc::vfs_rpc::chdir_transport::FINISH_BYTES;
    let mut packet = [0; FINISH_BYTES];
    packet[..FRAME_BYTES].copy_from_slice(&retained);
    packet[FRAME_BYTES + 8..].copy_from_slice(&api::encode_cwd("/service/global/root").unwrap());
    assert!(super::super::chdir::finish_runtime(
        &mut kernel,
        3,
        identity.registration.epoch,
        &packet,
        &mut sched,
        &caps
    )
    .is_err());
    packet[FRAME_BYTES + 8..].copy_from_slice(&api::encode_cwd("/").unwrap());
    assert!(super::super::chdir::finish_runtime(
        &mut kernel,
        3,
        identity.registration.epoch,
        &packet,
        &mut sched,
        &caps
    )
    .is_ok());
    // Acknowledgement and stale context are single-use.
    assert!(super::super::chdir::finish_runtime(
        &mut kernel,
        3,
        identity.registration.epoch,
        &packet,
        &mut sched,
        &caps
    )
    .is_err());
    assert!(context_runtime(
        &mut kernel,
        3,
        identity.registration.epoch,
        &retain,
        &sched,
        &caps
    )
    .is_err());
    assert_eq!(sched.processes[1].cwd, "/");
}

fn poll_inheritance<const P: usize, const L: usize>(
    kernel: &mut Runtime,
    sched: &mut Scheduler,
    caps: &CapabilityBroker,
    service: &mut Service<P, L, 1>,
    namespace: &DirectoryNamespace,
) {
    service.poll_root_control(
        &mut CwdChannel {
            kernel,
            sched,
            caps,
            reject_commit: false,
            legacy: None,
            file: None,
        },
        namespace,
    );
}

fn bound_receipt(kernel: &Runtime, caller: Caller) -> Receipt {
    kernel
        .remote
        .iter()
        .flatten()
        .flat_map(|r| r.local.iter().flatten())
        .find_map(|l| match l {
            LocalLease::Bound { receipt, .. } if receipt.scope().caller == caller => Some(*receipt),
            _ => None,
        })
        .unwrap()
}

#[test]
fn copied_spawn_snapshot_survives_parent_retirement_rename_and_recreation() {
    let (mut kernel, mut sched, caps, identity) = setup();
    sched.processes[1].vfs_root_metadata = true;
    sched.processes[1].vfs_boot_root = true;
    sched.processes[1].cwd = alloc::string::String::from("/work/cwd");
    let namespace = crate::mounted_cwd_namespace(identity.uid, identity.gid);
    let mut service = Service::<4, 8, 1>::new();
    poll_inheritance(&mut kernel, &mut sched, &caps, &mut service, &namespace);
    let source_receipt = bound_receipt(&kernel, identity.caller);
    let expected = service
        .metadata(identity.registration, source_receipt)
        .unwrap();
    let source = kernel.capture_inheritance(identity).unwrap();
    let mut child = setup().1.processes.remove(1);
    child.pid = 20;
    child.foreground.incarnation = 21;
    child.address_space.generation = 22;
    child.vfs_root_metadata = true;
    child.cwd = alloc::string::String::from("/work/cwd");
    let child_identity = AdmissionIdentity {
        caller: process_identity(&child),
        ..identity
    };
    sched.processes.push(child);
    kernel
        .queue_inheritance(source, child_identity, "/work/cwd")
        .unwrap();
    // The source's policy can disappear before service delivery. Its exact lease
    // must remain held until the child has acquired independent ownership.
    sched.processes[1].exiting = true;
    kernel.retire_process(identity.caller);
    {
        let shared = namespace.shared();
        let mut fs = shared.lock();
        fs.rename("/work/cwd", "/work/moved").unwrap();
        fs.mkdir("/work/cwd", identity.uid, identity.gid, 0o755)
            .unwrap();
    }
    poll_inheritance(&mut kernel, &mut sched, &caps, &mut service, &namespace);
    assert!(kernel.namespaces.lookup(identity).is_err());
    assert!(kernel.namespaces.lookup(child_identity).is_ok());
    let receipt = bound_receipt(&kernel, child_identity.caller);
    assert_eq!(
        service.metadata(identity.registration, receipt).unwrap(),
        expected
    );
    // Reconcile the released snapshot, then prove child exit owns the last pins.
    kernel.reconcile_namespaces();
    poll_inheritance(&mut kernel, &mut sched, &caps, &mut service, &namespace);
    assert!(service
        .metadata(identity.registration, source_receipt)
        .is_err());
    assert_eq!(namespace.shared().lock().open_handle_count(), 2);
    sched.processes[2].exiting = true;
    kernel.retire_process(child_identity.caller);
    poll_inheritance(&mut kernel, &mut sched, &caps, &mut service, &namespace);
    assert_eq!(namespace.shared().lock().open_handle_count(), 0);
}

#[test]
fn copied_exec_derives_new_vm_after_old_caller_barrier_without_reusing_admission() {
    let (mut kernel, mut sched, caps, identity) = setup();
    sched.processes[1].vfs_root_metadata = true;
    sched.processes[1].vfs_boot_root = true;
    let namespace = crate::mounted_namespace();
    let mut service = Service::<1, 8, 1>::new();
    poll_inheritance(&mut kernel, &mut sched, &caps, &mut service, &namespace);
    let source = kernel.capture_inheritance(identity).unwrap();
    let expected = service
        .metadata(
            identity.registration,
            bound_receipt(&kernel, identity.caller),
        )
        .unwrap();
    kernel.retire_process(identity.caller);
    sched.processes[1].address_space.generation += 1;
    let next = AdmissionIdentity {
        caller: process_identity(&sched.processes[1]),
        ..identity
    };
    kernel.queue_inheritance(source, next, "/").unwrap();
    poll_inheritance(&mut kernel, &mut sched, &caps, &mut service, &namespace);
    assert!(kernel.namespaces.lookup(identity).is_err());
    assert!(kernel.namespaces.lookup(next).is_ok());
    assert_eq!(
        service
            .metadata(identity.registration, bound_receipt(&kernel, next.caller))
            .unwrap(),
        expected
    );
    assert!(kernel.capture_inheritance(identity).is_none());
}

#[test]
fn copied_derivation_refuses_changed_destination_and_capacity_without_root_regrant() {
    for fault in 0..5 {
        let (mut kernel, mut sched, caps, identity) = setup();
        sched.processes[1].vfs_root_metadata = true;
        sched.processes[1].vfs_boot_root = true;
        let namespace = crate::mounted_namespace();
        let mut service = Service::<1, 8, 1>::new();
        poll_inheritance(&mut kernel, &mut sched, &caps, &mut service, &namespace);
        let source = kernel.capture_inheritance(identity).unwrap();
        let mut child = setup().1.processes.remove(1);
        child.pid = 20;
        child.vfs_root_metadata = true;
        let next = AdmissionIdentity {
            caller: process_identity(&child),
            ..identity
        };
        sched.processes.push(child);
        kernel.queue_inheritance(source, next, "/").unwrap();
        match fault {
            0 => sched.processes[2].uid += 1,
            1 => sched.processes[2].cwd = alloc::string::String::from("/changed"),
            2 => sched.processes[2].address_space.generation += 1,
            3 => {
                sched.processes[2].exiting = true;
                kernel.retire_process(next.caller);
            }
            _ => (), // live source occupies the sole service policy slot
        }
        poll_inheritance(&mut kernel, &mut sched, &caps, &mut service, &namespace);
        assert!(kernel.namespaces.lookup(next).is_err());
        assert!(kernel.namespaces.lookup(identity).is_ok());
        // Restoring the record cannot silently turn derivation into initial `/`.
        sched.processes[2].uid = identity.uid;
        sched.processes[2].cwd = alloc::string::String::from("/");
        sched.processes[2].address_space.generation = next.caller.address_space_generation;
        poll_inheritance(&mut kernel, &mut sched, &caps, &mut service, &namespace);
        assert!(kernel.namespaces.lookup(next).is_err());
        assert_eq!(namespace.shared().lock().open_handle_count(), 2);
    }
}

#[test]
fn copied_derivation_authenticates_delivery_and_compensates_late_credentials() {
    let (mut kernel, mut sched, caps, identity) = setup();
    sched.processes[1].vfs_root_metadata = true;
    sched.processes[1].vfs_boot_root = true;
    let namespace = crate::mounted_namespace();
    let mut service = Service::<2, 8, 1>::new();
    poll_inheritance(&mut kernel, &mut sched, &caps, &mut service, &namespace);
    let source = kernel.capture_inheritance(identity).unwrap();
    let mut child = setup().1.processes.remove(1);
    child.pid = 20;
    child.vfs_root_metadata = true;
    let next = AdmissionIdentity {
        caller: process_identity(&child),
        ..identity
    };
    sched.processes.push(child);
    kernel.queue_inheritance(source, next, "/").unwrap();
    let request = kernel
        .remote
        .iter()
        .flatten()
        .flat_map(|r| r.unsent.iter().flatten())
        .next()
        .unwrap()
        .encode()
        .unwrap();
    assert!(call(
        &mut kernel,
        &sched,
        &caps,
        3,
        api::DERIVE_SOURCE,
        identity.registration.epoch,
        &request
    )
    .is_err());
    // A later receipt races the independently captured destination credentials.
    service.poll_root_control(
        &mut CwdChannel {
            kernel: &mut kernel,
            sched: &mut sched,
            caps: &caps,
            reject_commit: true,
            legacy: None,
            file: None,
        },
        &namespace,
    );
    assert!(kernel.namespaces.lookup(next).is_err());
    assert!(call(
        &mut kernel,
        &sched,
        &caps,
        20,
        api::DERIVE_SOURCE,
        identity.registration.epoch,
        &request
    )
    .is_err());
    assert!(call(
        &mut kernel,
        &sched,
        &caps,
        3,
        api::DERIVE_SOURCE,
        identity.registration.epoch + 1,
        &request
    )
    .is_err());
    sched.processes[2].exiting = true;
    kernel.retire_process(next.caller);
    poll_inheritance(&mut kernel, &mut sched, &caps, &mut service, &namespace);
    assert_eq!(namespace.shared().lock().open_handle_count(), 2);
    assert!(kernel.namespaces.lookup(identity).is_ok());
}

#[test]
fn copied_derivation_service_retirement_drains_snapshot_without_installing_child() {
    let (mut kernel, mut sched, caps, identity) = setup();
    sched.processes[1].vfs_root_metadata = true;
    sched.processes[1].vfs_boot_root = true;
    let namespace = crate::mounted_namespace();
    let mut service = Service::<2, 8, 1>::new();
    poll_inheritance(&mut kernel, &mut sched, &caps, &mut service, &namespace);
    let source = kernel.capture_inheritance(identity).unwrap();
    let mut child = setup().1.processes.remove(1);
    child.pid = 20;
    child.vfs_root_metadata = true;
    let next = AdmissionIdentity {
        caller: process_identity(&child),
        ..identity
    };
    sched.processes.push(child);
    kernel.queue_inheritance(source, next, "/").unwrap();
    kernel.retire_registration(identity.registration);
    poll_inheritance(&mut kernel, &mut sched, &caps, &mut service, &namespace);
    assert!(kernel.namespaces.lookup(next).is_err());
    assert_eq!(namespace.shared().lock().open_handle_count(), 0);
    assert_eq!(
        kernel
            .remote
            .iter()
            .flatten()
            .next()
            .unwrap()
            .ownership
            .outstanding(),
        0
    );
}

#[test]
fn service_replacement_cannot_reprovision_a_derived_caller_at_boot_root() {
    let (mut kernel, mut sched, caps, identity) = setup();
    sched.processes[1].vfs_root_metadata = true;
    sched.processes[1].vfs_boot_root = true;
    let namespace = crate::mounted_namespace();
    let mut service = Service::<2, 8, 1>::new();
    poll_inheritance(&mut kernel, &mut sched, &caps, &mut service, &namespace);
    let source = kernel.capture_inheritance(identity).unwrap();
    let mut child = setup().1.processes.remove(1);
    child.pid = 20;
    child.vfs_root_metadata = true;
    let next = AdmissionIdentity {
        caller: process_identity(&child),
        ..identity
    };
    assert!(!child.vfs_boot_root);
    sched.processes.push(child);
    kernel.queue_inheritance(source, next, "/").unwrap();
    poll_inheritance(&mut kernel, &mut sched, &caps, &mut service, &namespace);
    assert!(kernel.namespaces.lookup(next).is_ok());
    kernel.retire_registration(identity.registration);
    let replacement = kernel.register(identity.registration.owner).unwrap();
    assert_ne!(replacement.epoch, identity.registration.epoch);
    kernel.stage_selected_roots(replacement, &sched);
    assert!(!kernel
        .remote
        .iter()
        .flatten()
        .filter(|r| r.registration == replacement)
        .flat_map(|r| r.unsent.iter().flatten())
        .any(|f| matches!(f,
            Frame::Retain { scope, .. } if scope.caller == next.caller)));
    assert!(kernel
        .namespaces
        .lookup(AdmissionIdentity {
            registration: replacement,
            ..next
        })
        .is_err());
}

#[test]
fn file_owner_exit_survives_metadata_ack_and_closes_only_exact_lifetime() {
    use sunlight_vfs_server::open::Access;
    let (mut kernel, sched, caps, identity) = setup();
    let reg = identity.registration;
    let mut service = Service::<4, 4, 2>::new();
    service.bind(reg).unwrap();
    let namespace = crate::mounted_namespace();
    {
        let shared = namespace.shared();
        let mut fs = shared.lock();
        let h = fs
            .create_file_exclusive("/data", identity.uid, identity.gid, 0o600)
            .unwrap();
        fs.write(h, 0, b"data").unwrap();
        fs.close(h).unwrap();
    }
    kernel.queue_namespace(identity, LOOKUP_METADATA).unwrap();
    let retain = call(&mut kernel, &sched, &caps, 3, api::NEXT, reg.epoch, &[])
        .unwrap()
        .unwrap();
    let Frame::Retain { scope, .. } = Frame::decode(&retain).unwrap() else {
        panic!()
    };
    service.install(reg, scope, &namespace, "/", "/").unwrap();
    let reply = service.request(reg, &retain).unwrap();
    let Frame::Retained(receipt) = reply else {
        panic!()
    };
    call(
        &mut kernel,
        &sched,
        &caps,
        3,
        api::REPLY,
        reg.epoch,
        &reply.encode().unwrap(),
    )
    .unwrap();
    let token = service
        .open_file(
            reg,
            receipt,
            "data",
            sunlight_fs::Credential {
                uid: identity.uid,
                gid: identity.gid,
            },
            Access::ReadWrite,
            || true,
        )
        .unwrap();
    let duplicate = service.duplicate_file(reg, identity.caller, token).unwrap();

    // Live credential/profile withdrawal delivers only the metadata barrier.
    kernel.retire_caller_profile(identity.caller);
    let event = call(&mut kernel, &sched, &caps, 3, api::NEXT, reg.epoch, &[])
        .unwrap()
        .unwrap();
    assert!(matches!(
        Frame::decode(&event).unwrap(),
        Frame::CallerRetired { .. }
    ));
    let ack = service
        .control_request(reg, &event)
        .unwrap()
        .encode()
        .unwrap();
    call(&mut kernel, &sched, &caps, 3, api::REPLY, reg.epoch, &ack).unwrap();
    assert_eq!(
        service.read_file(reg, identity.caller, token, &mut [0; 1]),
        Ok(1)
    );
    service.poll_control(&mut Channel {
        kernel: &mut kernel,
        sched: &sched,
        caps: &caps,
        lose_ack: false,
        old_epoch: reg.epoch,
        attempts: [0; 2],
    });
    assert_eq!(namespace.shared().lock().open_handle_count(), 1);
    assert_eq!(kernel.next_control(reg), Ok(None));
    // Metadata acknowledgement cannot discard the future exit obligation.
    assert_eq!(
        kernel.remote[0]
            .as_ref()
            .unwrap()
            .callers
            .iter()
            .flatten()
            .count(),
        1
    );
    for caller in [
        Caller {
            pid: identity.caller.pid + 1,
            ..identity.caller
        },
        Caller {
            incarnation: identity.caller.incarnation + 1,
            ..identity.caller
        },
        Caller {
            address_space_generation: identity.caller.address_space_generation + 1,
            ..identity.caller
        },
    ] {
        kernel.retire_process(caller);
        assert_eq!(kernel.next_control(reg), Ok(None));
    }
    kernel.retire_process(identity.caller); // Exit or successful exec's old VM.
    kernel.retire_process(identity.caller);
    let retirement = Frame::FileOwnerRetired {
        registration: reg,
        caller: identity.caller,
        event_id: 1,
    };
    let retirement_ack = Frame::FileOwnerRetiredAck {
        registration: reg,
        caller: identity.caller,
        event_id: 1,
    }
    .encode()
    .unwrap();
    assert!(call(
        &mut kernel,
        &sched,
        &caps,
        3,
        api::REPLY,
        reg.epoch,
        &retirement_ack
    )
    .is_err());
    for _ in 0..2 {
        // Lost event copies retry exactly.
        assert_eq!(kernel.next_control(reg), Ok(Some(retirement)));
    }
    for word in 2..10 {
        // Exact registration, owner PID, incarnation and VM.
        let mut wrong = retirement_ack;
        wrong[word * 8] ^= 32;
        assert!(call(&mut kernel, &sched, &caps, 3, api::REPLY, reg.epoch, &wrong).is_err());
    }
    let mut wrong_id = retirement_ack;
    wrong_id[96] = 2;
    assert!(call(
        &mut kernel,
        &sched,
        &caps,
        3,
        api::REPLY,
        reg.epoch,
        &wrong_id
    )
    .is_err());
    assert!(call(
        &mut kernel,
        &sched,
        &caps,
        10,
        api::REPLY,
        reg.epoch,
        &retirement_ack
    )
    .is_err());
    assert!(call(&mut kernel, &sched, &caps, 3, api::REPLY, reg.epoch, &ack).is_err());
    for _ in 0..2 {
        // Lost acknowledgements repeat idempotent service cleanup.
        assert_eq!(
            service
                .control_request(reg, &retirement.encode().unwrap())
                .unwrap()
                .encode()
                .unwrap(),
            retirement_ack
        );
        assert_eq!(kernel.next_control(reg), Ok(Some(retirement)));
    }
    assert!(service.stat_file(reg, identity.caller, token).is_err());
    assert!(service.stat_file(reg, identity.caller, duplicate).is_err());
    assert_eq!(namespace.shared().lock().open_handle_count(), 0); // Metadata lease already released.
    call(
        &mut kernel,
        &sched,
        &caps,
        3,
        api::REPLY,
        reg.epoch,
        &retirement_ack,
    )
    .unwrap();
    assert!(call(
        &mut kernel,
        &sched,
        &caps,
        3,
        api::REPLY,
        reg.epoch,
        &retirement_ack
    )
    .is_err());
    // Release delivery is separate and remains charged until acknowledged.
    kernel.reconcile_namespaces();
    service.poll_control(&mut Channel {
        kernel: &mut kernel,
        sched: &sched,
        caps: &caps,
        lose_ack: false,
        old_epoch: reg.epoch,
        attempts: [0; 2],
    });
    assert_eq!(namespace.shared().lock().open_handle_count(), 0);
}

#[cfg(feature = "vfs_c2e_candidate")]
#[test]
fn generic_native_and_linux_chdir_cross_downloads_and_settle() {
    use super::super::chdir::{resume_runtime, route_directory, transition_call_runtime, DirectoryRoute};
    use sunlight_fs::Credential;
    use sunlight_ipc::vfs_rpc::{chdir_transport::Kind, file_transport as file};

    const CRED: Credential = Credential { uid: 0, gid: 0 };
    let _serial = crate::ipc::vfs_resources::TEST_LEDGER_LOCK.lock();
    let (mut kernel, mut sched, caps, mut identity) = setup();
    sched.processes[1].uid = CRED.uid;
    sched.processes[1].gid = CRED.gid;
    identity.uid = CRED.uid;
    identity.gid = CRED.gid;

    let (mut legacy, namespace) = crate::downloads_namespaces();
    let mut service = Service::<1, 8, 1>::new();
    let reg = identity.registration;
    let native = 63;
    let linux = sunlight_compat_linux::translate_syscall(sunlight_compat_linux::abi::SYS_CHDIR);
    let mut channel = CwdChannel {
        kernel: &mut kernel, sched: &mut sched, caps: &caps, reject_commit: false,
        legacy: Some(&mut legacy), file: None,
    };
    let chdir = |channel: &mut CwdChannel<'_>, service: &mut Service<1, 8, 1>, number, path: &str| {
        assert_eq!(number, 63);
        let cwd = &channel.sched.processes[1].cwd;
        let route = route_directory(channel.sched, 1, path, || {
            channel.legacy.as_deref_mut().unwrap().resolve_legacy_cwd(cwd, path, &CRED, &mut |_| Ok(()))
        }).unwrap();
        match route {
            DirectoryRoute::Legacy(path) => {
                channel.sched.processes[1].cwd = path;
                Ok(Some(0))
            }
            DirectoryRoute::Service(kind, path) => {
                match transition_call_runtime(channel.kernel, 10, &path, kind, channel.sched, channel.caps) {
                    Ok(None) => {
                        service.poll_root_control(channel, &namespace);
                        resume_runtime(channel.kernel, 10, (0, 0), Kind::Directory, channel.sched)
                            .map(|result| result.expect("parked transition resumes"))
                    }
                    result => result,
                }
            }
        }
    };
    assert_eq!(chdir(&mut channel, &mut service, native, "/root/Downloads"), Ok(Some(0)));
    assert!(channel.sched.processes[1].vfs_downloads_cwd);
    assert_eq!(namespace.shared().lock().open_handle_count(), 2);
    assert_eq!(chdir(&mut channel, &mut service, linux, ".."), Ok(Some(0)));
    assert_eq!(channel.sched.processes[1].cwd, "/root");
    assert!(!channel.sched.processes[1].vfs_downloads_cwd);
    assert_eq!(namespace.shared().lock().open_handle_count(), 2);
    assert_eq!(channel.legacy.as_ref().unwrap().open_handle_count(), 0);

    channel.reject_commit = true;
    assert_eq!(chdir(&mut channel, &mut service, linux, "Downloads"), Ok(Some(5)));
    channel.sched.processes[1].uid = CRED.uid;
    channel.reject_commit = false;
    assert_eq!(channel.sched.processes[1].cwd, "/root");
    assert!(!channel.sched.processes[1].vfs_downloads_cwd);
    assert_eq!(namespace.shared().lock().open_handle_count(), 2);

    assert_eq!(chdir(&mut channel, &mut service, linux, "Downloads"), Ok(Some(0)));
    namespace.shared().lock().mkdir("/root/Downloads/work", 0, 0, 0o755).unwrap();
    assert_eq!(chdir(&mut channel, &mut service, native, "work"), Ok(Some(0)));
    namespace.shared().lock().rename("/root/Downloads/work", "/root/Downloads/renamed").unwrap();
    assert_eq!(channel.sched.processes[1].cwd, "/root/Downloads/work");
    let receipt = channel.kernel.caller_receipt(identity).unwrap();
    let request = file::Request {
        registration: reg, caller: identity.caller, request_id: 1,
        op: file::OPEN, token: 0, arg0: file::ACCESS_WRITE | file::OPEN_CREATE,
        arg1: 0o644, length: 7, payload: b"created",
        namespace_id: receipt.scope().namespace_id,
        namespace_generation: receipt.scope().namespace_generation,
        receipt_id: receipt.request_id(), receipt_generation: receipt.generation(),
        uid: 0, gid: 0,
    };
    let send = |channel: &mut CwdChannel<'_>, service: &mut Service<1, 8, 1>, request: file::Request<'_>| {
        let request = request.encode().unwrap();
        channel.file = Some(request);
        let reply = service.file_request(channel, reg, &request).unwrap();
        channel.file = None;
        reply
    };
    let result = send(&mut channel, &mut service, request);
    let reply = file::Reply::decode(&result).unwrap();
    assert_eq!(reply.errno, 0);
    service.close_file(reg, identity.caller, reply.result0).unwrap();
    let result = send(&mut channel, &mut service, file::Request {
        request_id: 2, arg0: file::ACCESS_READ | file::OPEN_DIRECTORY,
        arg1: 0, length: 1, payload: b".", ..request
    });
    let reply = file::Reply::decode(&result).unwrap();
    assert_eq!(reply.errno, 0);
    let directory = reply.result0;
    let metadata = channel.kernel.admit_stat(identity, "created").unwrap();
    let id = metadata.words()[1];
    let result = service.stat(reg, channel.kernel.pending.resources(id).unwrap().input()).unwrap();
    assert_eq!(sunlight_ipc::vfs_rpc::stat_transport::decode_reply(&result).unwrap().words[2], 0);
    channel.kernel.pending.retire_caller(identity.caller);
    drop(channel.kernel.pending.reap(id));
    assert!(namespace.shared().lock().stat("/root/Downloads/renamed/created").is_ok());

    for (path, errno) in [("missing", 2), ("/absent", 2), ("../../Downloads/missing", 13)] {
        assert_eq!(chdir(&mut channel, &mut service, linux, path), Ok(Some(errno)));
        assert!(channel.sched.processes[1].vfs_downloads_cwd);
        assert_eq!(channel.kernel.caller_receipt(identity), Some(receipt));
    }
    assert_eq!(chdir(&mut channel, &mut service, linux, "/etc"), Ok(Some(0)));
    assert_eq!(channel.sched.processes[1].cwd, "/etc");
    let root_only = channel.kernel.caller_receipt(identity).unwrap();
    let result = send(&mut channel, &mut service, file::Request {
        request_id: 3, op: file::STAT_AT, token: directory, arg0: 0, arg1: 0,
        namespace_id: root_only.scope().namespace_id,
        namespace_generation: root_only.scope().namespace_generation,
        receipt_id: root_only.request_id(), receipt_generation: root_only.generation(),
        ..request
    });
    assert_eq!(file::Reply::decode(&result).unwrap().errno, 0);
    service.close_file(reg, identity.caller, directory).unwrap();
    let token = service.open_file(reg, root_only, "/root/Downloads/renamed/created",
        CRED, sunlight_vfs_server::open::Access::Read, || true).unwrap();
    service.close_file(reg, identity.caller, token).unwrap();
    assert_eq!(namespace.shared().lock().open_handle_count(), 2);
    assert_eq!(chdir(&mut channel, &mut service, native, "/root/Downloads"), Ok(Some(0)));
    assert_eq!(chdir(&mut channel, &mut service, linux, ".."), Ok(Some(0)));
    assert_eq!(chdir(&mut channel, &mut service, native, "Downloads"), Ok(Some(0)));

    channel.kernel.retire_registration(reg);
    assert!(chdir(&mut channel, &mut service, native, "/etc").is_err());
    assert!(channel.sched.processes[1].vfs_downloads_cwd);
    assert_eq!(channel.sched.processes[1].cwd, "/root/Downloads");
    channel.kernel.retire_process(identity.caller);
    service.poll_root_control(&mut channel, &namespace);
    assert!(channel.kernel.namespaces.lookup(identity).is_err());
    assert_eq!(namespace.shared().lock().open_handle_count(), 0);
    assert_eq!(channel.legacy.as_ref().unwrap().open_handle_count(), 0);
    assert!(channel.kernel.remote.iter().flatten().all(|remote| {
        remote.registration != reg || remote.ownership.outstanding() == 0
    }));
}

#[cfg(feature = "vfs_c2e_candidate")]
#[test]
fn generic_downloads_caller_retirement_drains_active_epoch() {
    use super::super::chdir::transition_call_runtime;
    use sunlight_ipc::vfs_rpc::chdir_transport::Kind;
    let _serial = crate::ipc::vfs_resources::TEST_LEDGER_LOCK.lock();
    let (mut kernel, mut sched, caps, identity) = setup();
    let (mut legacy, namespace) = crate::downloads_namespaces();
    let mut service = Service::<1, 8, 1>::new();
    sched.processes[1].uid = 0;
    let mut worker = sched.processes[1].clone();
    worker.pid = 11;
    worker.foreground.incarnation += 1;
    worker.shared_owner = Some(1);
    sched.processes.push(worker);

    // The worker parks and resumes; only its sharing group's owner receives
    // the namespace binding and published cwd.
    assert_eq!(
        transition_call_runtime(&mut kernel, 11, "/root/Downloads",
            Kind::EnterDownloads, &mut sched, &caps),
        Ok(None)
    );
    let mut channel = CwdChannel {
        kernel: &mut kernel, sched: &mut sched, caps: &caps, reject_commit: false,
        legacy: Some(&mut legacy), file: None,
    };
    service.poll_root_control(&mut channel, &namespace);
    use super::super::chdir::resume_runtime;
    assert_eq!(resume_runtime(channel.kernel, 11, (0, 0), Kind::Directory, channel.sched), Ok(Some(Some(0))));
    assert!(channel.sched.processes[1].vfs_downloads_cwd);
    assert!(channel.sched.processes[2].vfs_downloads_cwd);
    assert_eq!(channel.sched.processes[1].cwd, "/root/Downloads");
    assert!(!channel.sched.processes[2].vfs_root_metadata);
    assert_eq!(namespace.shared().lock().open_handle_count(), 2);
    assert_eq!(
        transition_call_runtime(channel.kernel, 11, "..", Kind::Downloads, channel.sched, channel.caps),
        Ok(None)
    );
    service.poll_root_control(&mut channel, &namespace);
    assert_eq!(resume_runtime(channel.kernel, 11, (0, 0), Kind::Directory, channel.sched), Ok(Some(Some(0))));
    assert_eq!(channel.sched.processes[1].cwd, "/root");
    assert!(!channel.sched.processes[1].vfs_downloads_cwd);
    assert!(!channel.sched.processes[2].vfs_downloads_cwd);
    channel.sched.processes[1].exiting = true;
    channel.kernel.retire_process(identity.caller);
    service.poll_root_control(&mut channel, &namespace);
    assert_eq!(channel.kernel.registry.active(), Some(identity.registration));
    assert!(channel.kernel.chdirs.iter().all(Option::is_none));
    assert_eq!(namespace.shared().lock().open_handle_count(), 0);
    assert!(channel.kernel.remote.iter().flatten().all(|remote| remote.ownership.outstanding() == 0));
}

#[cfg(feature = "vfs_c2e_candidate")]
#[test]
fn executable_source_selection_and_loader_input_through_copied_channel() {
    use super::super::{chdir, executable, file};
    use sunlight_elf::{classify_personality, parse_elf_header, plan_segments, Personality};
    use sunlight_ipc::vfs_rpc::{chdir_transport::Kind, file_transport as wire};

    let _serial = crate::ipc::vfs_resources::TEST_LEDGER_LOCK.lock();
    let (mut kernel, mut sched, caps, mut identity) = setup();
    sched.processes[1].uid = 0;
    sched.processes[1].gid = 0;
    identity.uid = 0;
    identity.gid = 0;
    let (mut legacy, namespace) = crate::downloads_namespaces();
    let mut service = Service::<1, 8, 1>::new();
    let registration = identity.registration;
    let mut channel = CwdChannel {
        kernel: &mut kernel, sched: &mut sched, caps: &caps, reject_commit: false,
        legacy: Some(&mut legacy), file: None,
    };
    assert_eq!(
        chdir::transition_call_runtime(channel.kernel, 10, "/root/Downloads", Kind::EnterDownloads,
            channel.sched, channel.caps),
        Ok(None)
    );
    service.poll_root_control(&mut channel, &namespace);
    assert_eq!(chdir::resume_runtime(channel.kernel, 10, (0, 0), Kind::Directory, channel.sched),
        Ok(Some(Some(0))));
    let receipt = channel.kernel.caller_receipt(identity).unwrap();
    let baseline = namespace.shared().lock().open_handle_count();

    let advance = |channel: &mut CwdChannel<'_>, source: &mut executable::Source| {
        channel.sched.processes[1].state = crate::process::ProcessState::Running;
        let mut first = true;
        source.step(|operation, _| {
            if first {
                first = false;
                if let Some(slot) = channel.kernel.caller_publication(identity.caller) {
                    let op = channel.kernel.file_publications[slot].as_ref().unwrap().op;
                    let reply = if op == wire::OPEN {
                        channel.kernel.finish_open(slot, |token, _| Ok(token))?
                    } else {
                        channel.kernel.take_file(identity.caller, op).expect("reply delivered")
                    };
                    return Ok(file::FileCall::Complete(reply));
                }
            }
            channel.kernel.publish_file(identity, operation, Some(receipt), channel.sched)?;
            Ok(file::FileCall::Pending)
        })
    };
    let exchange = |channel: &mut CwdChannel<'_>, service: &mut Service<1, 8, 1>, op: u64| {
        let mut packet = [0; wire::FRAME_BYTES];
        assert!(channel.kernel.next_file(registration, &mut packet).unwrap().is_some());
        assert_eq!(wire::Request::decode(&packet).unwrap().op, op);
        channel.file = Some(packet);
        assert!(service.dispatch_file(channel, registration, &packet));
        channel.file = None;
    };
    let assert_clean = |channel: &mut CwdChannel<'_>| {
        assert_eq!(namespace.shared().lock().open_handle_count(), baseline);
        assert!(channel.kernel.file_publications.iter().all(Option::is_none));
        assert_eq!(channel.legacy.as_ref().unwrap().open_handle_count(), 0);
    };
    assert_eq!(sunlight_compat_linux::translate_syscall(sunlight_compat_linux::abi::SYS_EXECVE), 31);
    assert!(executable::route("/bin/sshl", "/", false).is_none());

    for (path, osabi, mode, expected) in [
        ("native", 0, 0o700, Some(Personality::Native)),
        ("linux", 3, 0o700, Some(Personality::Linux)),
        ("truncated", 0, 0o700, None),
        ("readable", 0, 0o600, None),
        ("short", 0, 0o700, None),
        ("read-error", 0, 0o700, None),
        ("abandoned-open", 0, 0o700, None),
        ("cancel", 0, 0o700, None),
        ("withdrawn", 0, 0o700, None),
    ] {
        let mut bytes = vec![0u8; wire::IO_BYTES + 64];
        bytes[..5].copy_from_slice(b"\x7fELF\x02");
        bytes[7] = osabi;
        bytes[16..20].copy_from_slice(&[2, 0, 0x3e, 0]);
        bytes[24..32].copy_from_slice(&0x400080u64.to_le_bytes());
        bytes[128] = 0xc3;
        bytes[32..40].copy_from_slice(&64u64.to_le_bytes());
        bytes[54..58].copy_from_slice(&[56, 0, 1, 0]);
        bytes[64..72].copy_from_slice(&[1, 0, 0, 0, 5, 0, 0, 0]);
        bytes[80..88].copy_from_slice(&0x400000u64.to_le_bytes());
        let image_len = bytes.len() as u64;
        bytes[96..104].copy_from_slice(&image_len.to_le_bytes());
        bytes[104..112].copy_from_slice(&image_len.to_le_bytes());
        bytes[112..120].copy_from_slice(&4096u64.to_le_bytes());
        if path == "truncated" {
            bytes.truncate(32);
        }
        let absolute = format!("/root/Downloads/{path}");
        {
            let shared = namespace.shared();
            let mut fs = shared.lock();
            let h = fs.create_file_exclusive(&absolute, 0, 0, mode).unwrap();
            fs.write(h, 0, &bytes).unwrap();
            fs.close(h).unwrap();
        }
        assert!(channel.legacy.as_mut().unwrap().open(&absolute).is_err());
        let selected = if path == "linux" {
            executable::route("Downloads/linux", "/root", false).unwrap()
        } else {
            executable::route(path, "/root/Downloads", true).unwrap()
        };
        let mut source = executable::Source::new(selected.into_owned(), (identity, receipt));
        assert_eq!(advance(&mut channel, &mut source), Ok(None));
        exchange(&mut channel, &mut service, wire::OPEN);
        if path == "abandoned-open" {
            assert_eq!(namespace.shared().lock().open_handle_count(), baseline + 1);
            channel.kernel.settle_executable(identity.caller);
            exchange(&mut channel, &mut service, wire::CLOSE);
            assert_clean(&mut channel);
            continue;
        }
        let result = advance(&mut channel, &mut source);
        if path == "readable" {
            assert_eq!(result, Err(file::Error::Protocol));
            assert_clean(&mut channel);
            continue;
        }
        assert_eq!(result, Ok(None));
        assert_eq!(namespace.shared().lock().open_handle_count(), baseline + 1);
        if path == "cancel" {
            channel.kernel.settle_executable(identity.caller);
            exchange(&mut channel, &mut service, wire::CLOSE);
            assert_clean(&mut channel);
            continue;
        }
        if path == "withdrawn" {
            channel.kernel.retire_registration(registration);
            assert_eq!(advance(&mut channel, &mut source), Err(file::Error::Protocol));
            assert!(executable::route(&absolute, "/", false).is_some());
            service.poll_root_control(&mut channel, &namespace);
            assert_eq!(namespace.shared().lock().open_handle_count(), 0);
            break;
        }
        if matches!(path, "short" | "read-error") {
            let mut packet = [0; wire::FRAME_BYTES];
            channel.kernel.next_file(registration, &mut packet).unwrap().unwrap();
            let request = wire::Request::decode(&packet).unwrap();
            let reply = wire::Reply {
                registration, caller: identity.caller, request_id: request.request_id,
                op: wire::PREAD, errno: if path == "short" { 0 } else { 5 },
                result0: 0, result1: 0, length: 0, payload: &[],
            }.encode().unwrap();
            channel.kernel.reply_file(registration, &reply, channel.sched).unwrap();
            assert_eq!(advance(&mut channel, &mut source), Err(file::Error::Protocol));
            channel.kernel.settle_executable(identity.caller);
            exchange(&mut channel, &mut service, wire::CLOSE);
            assert_clean(&mut channel);
            continue;
        }
        exchange(&mut channel, &mut service, wire::PREAD);
        let mut result = advance(&mut channel, &mut source).unwrap();
        if expected.is_some() {
            assert!(result.is_none());
            // The second read occurs after the source is renamed, overwritten,
            // truncated and replaced at its old name.
            let shared = namespace.shared();
            let mut fs = shared.lock();
            let moved = format!("{absolute}.moved");
            fs.rename(&absolute, &moved).unwrap();
            let old = fs.open(&moved).unwrap();
            fs.truncate(old).unwrap();
            fs.write(old, 0, b"mutated").unwrap();
            fs.close(old).unwrap();
            let replacement = fs.create_file_exclusive(&absolute, 0, 0, mode).unwrap();
            fs.write(replacement, 0, b"replacement").unwrap();
            fs.close(replacement).unwrap();
            drop(fs);
            exchange(&mut channel, &mut service, wire::PREAD);
            result = advance(&mut channel, &mut source).unwrap();
        }
        let input = result.expect("complete loader input");
        assert_eq!(input, bytes);
        channel.kernel.settle_executable(identity.caller);
        exchange(&mut channel, &mut service, wire::CLOSE);
        assert_clean(&mut channel);
        if let Some(personality) = expected {
            assert_eq!(classify_personality(&input), personality);
            let header = parse_elf_header(&input).unwrap();
            let mut plans = Vec::new();
            plan_segments(&input, &header, 0x1000, 0x8000_0000_0000, &mut |plan| plans.push(*plan))
                .unwrap();
            assert_eq!(plans.len(), 1);
            assert_eq!(plans[0].prot, sunlight_elf::SegmentProt::ReadExec);
        } else {
            assert!(parse_elf_header(&input).is_err());
        }
    }
}
