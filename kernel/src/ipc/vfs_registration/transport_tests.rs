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
        global_tick: 0,
        processes: alloc::vec![
            Process {
                pid: 3,
                trusted_vfs_service: true,
                vfs_root_metadata: false,
                cwd: alloc::string::String::from("/"),
                exiting: false,
                state: crate::sched::ProcessState::Running,
                pending_call: None,
                ipc_reply_target: None,
                ipc_endpoint: None,
                uid: 0,
                gid: 0,
                foreground: Foreground { incarnation: 4 },
                address_space: AddressSpace { generation: 5 }
            },
            Process {
                pid: 10,
                trusted_vfs_service: false,
                vfs_root_metadata: false,
                cwd: alloc::string::String::from("/"),
                exiting: false,
                state: crate::sched::ProcessState::Running,
                pending_call: None,
                ipc_reply_target: None,
                ipc_endpoint: None,
                uid: 1000,
                gid: 100,
                foreground: Foreground { incarnation: 11 },
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
    kernel.queue_namespace(identity, LOOKUP_METADATA).unwrap();
    kernel.queue_namespace(identity, LOOKUP_METADATA).unwrap();
    let first = kernel.remote[0].as_ref().unwrap().unsent[0].unwrap();
    let Frame::Retain {
        scope, request_id, ..
    } = first
    else {
        panic!()
    };
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
    kernel.queue_namespace(identity, LOOKUP_METADATA).unwrap();
    for expected in [2, 3] {
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
    for turn in 0..4 {
        service.poll_control(&mut Channel {
            kernel: &mut kernel,
            sched: &sched,
            caps: &caps,
            lose_ack: false,
            old_epoch: reg.epoch,
            attempts: [0; 2],
        });
        if turn < 3 {
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
                Ok(Frame::Released(_) | Frame::CallerRetiredAck { .. })
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
