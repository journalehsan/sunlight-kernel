use super::*;
use crate::process::pending_state::{Identity, Parent, RemoteClaim};
use crate::sched::ProcessState;
use registration::spawn::{Error, Terminal};
use sunlight_ipc::vfs_rpc::retention::wire::PendingRetain;

use super::super::super as registration;

#[test]
fn captured_spawn_authenticates_ack_after_exact_parent_retirement() {
    for exec in [false, true] {
        let (mut runtime, mut sched, caps, parent, child) = fixture();
        let claim = runtime
            .stage_spawn_inner(1, child, &mut sched, &caps, true, false)
            .unwrap();
        let request = pull(&mut runtime, &sched, &caps, claim);
        assert!(runtime.authorize_derivation(claim.registration, &request, &sched).is_ok());
        let mut stale = parent.caller;
        stale.address_space_generation += 1;
        runtime.retire_process(stale);
        assert_eq!(runtime.wait_spawn(1, claim, &mut sched), Ok(None));

        runtime.retire_process(parent.caller);
        if exec {
            sched.processes[1].address_space.generation += 1;
        } else {
            sched.processes[1].exiting = true;
        }
        assert_eq!(runtime.wait_spawn(1, claim, &mut sched), Err(Error::Identity));
        assert!(runtime.authorize_derivation(claim.registration, &request, &sched).is_err());
        let reply = reply_for(&request, |_, _| {}, false);
        assert!(send(&mut runtime, &sched, &caps, claim, reply));
        let Frame::Retained(receipt) = reply else { unreachable!() };
        assert!(runtime.remote_owner(claim.registration).unwrap().holds(receipt));
        assert!(runtime.validate_derivation(claim.registration, &request, &sched, true).is_err());
        assert!(sched.pending_children.private(child).is_some());
        assert!(runtime.publish_spawn(
            1, claim, &mut sched, &caps,
            |_| panic!("retired reply ticket prepared publication"),
            |_, _, _| panic!("retired reply ticket published child"),
        ).is_err());
    }
}

#[test]
fn captured_spawn_drops_source_pin_after_service_image_proof() {
    let (mut runtime, mut sched, caps, parent, child) = fixture();
    let claim = runtime.stage_spawn_inner(1, child, &mut sched, &caps, true, false).unwrap();
    let _request = pull(&mut runtime, &sched, &caps, claim);
    runtime.retire_process(parent.caller);
    runtime.retire_registration(claim.registration);
    drop(sched.pending_children.cancel(child.ticket, Some(child)).unwrap());
    sched.pending_children.reclaimed(child.ticket);
    runtime.poll_spawns(&mut sched, 0);
    assert_eq!(sched.pending_children.remote(child), Some(claim));
    runtime.complete_image_exit(
        crate::sched::exit_proof(retirement_identity(claim.registration), true, true, true).unwrap(),
    );
    runtime.poll_spawns(&mut sched, 0);
    assert!(sched.pending_children.remote(child).is_none());
    assert!(runtime.spawn_request(claim.registration, claim.request_id).is_none());
    assert!(runtime.remote_owner(claim.registration).is_err());
}

#[test]
fn delivered_spawn_derivation_cannot_pin_its_retired_epoch() {
    let (mut runtime, mut sched, caps, parent, child) = fixture();
    let claim = runtime.stage_spawn(1, child, &mut sched, &caps).unwrap();
    assert_eq!(runtime.wait_spawn(1, claim, &mut sched), Ok(None));
    let _request = pull(&mut runtime, &sched, &caps, claim);
    let independent_user = runtime.namespaces.lookup(parent).unwrap();
    runtime.retire_registration(claim.registration);
    assert!(runtime.remote_owner(claim.registration).unwrap().contains(claim.request_id));
    runtime.complete_image_exit(
        crate::sched::exit_proof(retirement_identity(claim.registration), true, true, true).unwrap(),
    );
    assert!(runtime.remote_owner(claim.registration).is_ok());
    drop(independent_user);
    runtime.collect_remote();
    assert!(runtime.remote_owner(claim.registration).is_err());
    assert_eq!(runtime.wait_spawn(1, claim, &mut sched), Ok(Some(Terminal::AuthorityRetired)));
    assert_eq!(runtime.wait_spawn(1, claim, &mut sched), Err(Error::Identity));
    drop(sched.pending_children.cancel(child.ticket, Some(child)).unwrap());
    sched.pending_children.reclaimed(child.ticket);
    runtime.poll_spawns(&mut sched, 0);
    assert_eq!(sched.pending_children.pids().count(), 0);
}

#[test]
fn publication_mixed_cycles_and_boundary_failures() {
    for cycle in 0..112 {
        let (mut runtime, mut sched, caps, parent, child) = fixture();
        sched.processes[1].foreground.member = 42;
        let claim = runtime.stage_spawn(1, child, &mut sched, &caps).unwrap();
        let request = pull(&mut runtime, &sched, &caps, claim);
        let reply = reply_for(&request, |_, _| {}, false);
        assert!(send(&mut runtime, &sched, &caps, claim, reply));
        let mode = cycle % 8;
        if mode == 1 {
            sched.pending_children.cancel(child.ticket, Some(child)).unwrap();
            runtime.cancel_spawns_for(parent.caller);
            sched.pending_children.reclaimed(child.ticket);
        } else if mode == 2 {
            runtime.retire_registration(claim.registration);
        } else if mode == 3 {
            sched.processes[1].exiting = true;
        } else if mode == 4 {
            sched.global_tick += 5 * crate::timekeeping::TICK_HZ;
        } else if mode == 7 {
            sched.processes[1].foreground.member = 43;
        }
        let mut enqueues = 0;
        let result = runtime.publish_spawn(
            1, claim, &mut sched, &caps,
            |_| if mode == 5 { Err(Error::Preparation) } else { Ok(()) },
            |sched, identity, launch| {
                assert_eq!(launch.parent.pid as u64, parent.caller.pid);
                assert_eq!(launch.parent.incarnation, parent.caller.incarnation);
                assert_eq!(launch.parent.image_generation, parent.caller.address_space_generation);
                assert_eq!(launch.execution, 42);
                sched.pending_children.eligible(identity, true).unwrap();
                let process = sched.pending_children.transfer(identity, true).unwrap();
                sched.processes.push(*process);
                sched.pending_children.published(identity.ticket);
                enqueues += 1;
                identity.ticket.pid
            },
        );
        if mode == 0 || mode == 6 {
            assert_eq!(result, Ok(child.ticket.pid));
            assert_eq!(enqueues, 1);
            assert!(runtime.spawn_request(claim.registration, claim.request_id).is_none());
            assert!(!sched.pending_children.pids().any(|pid| pid == child.ticket.pid));
            assert!(sched.pending_children.cancel(child.ticket, Some(child)).is_err());
            assert!(runtime.publish_spawn(
                1, claim, &mut sched, &caps, |_| panic!("duplicate preparation"),
                |_, _, _| panic!("duplicate enqueue"),
            ).is_err());
            if mode == 6 {
                runtime.retire_registration(claim.registration);
            }
            assert!(sched.processes.iter().any(|process| process.pid == child.ticket.pid));
        } else {
            assert!(result.is_err());
            assert_eq!(enqueues, 0);
            assert!(sched.processes.iter().all(|process| process.pid != child.ticket.pid));
            if mode != 1 {
                drop(sched.pending_children.cancel(child.ticket, Some(child)).unwrap());
                sched.pending_children.reclaimed(child.ticket);
            }
            runtime.cancel_spawns_for(parent.caller);
            let now = sched.global_tick;
            runtime.poll_spawns(&mut sched, now);
            if mode != 3 {
                assert!(matches!(
                    runtime.wait_spawn(1, claim, &mut sched),
                    Ok(Some(Terminal::Cancelled | Terminal::AuthorityRetired))
                ));
            }
            assert!(runtime.spawn_request(claim.registration, claim.request_id).is_none());
            assert!(sched.pending_children.private(child).is_none());
            assert!(sched.pending_children.cancel(child.ticket, Some(child)).is_err());
            assert!(sched.pending_children.pids().any(|pid| pid == child.ticket.pid));
            let remote = runtime.remote.iter().flatten()
                .find(|remote| remote.registration == claim.registration).unwrap();
            assert_eq!(remote.ownership.releases().filter(|frame| {
                matches!(frame, Frame::Release(receipt) if receipt.request_id() == claim.request_id)
            }).count(), 1);
        }
    }
}

fn fixture() -> (
    Runtime,
    Scheduler,
    CapabilityBroker,
    AdmissionIdentity,
    Identity,
) {
    let (mut runtime, mut sched, caps, parent) = setup();
    sched.processes[1].vfs_root_metadata = true;
    let frame = runtime.prepare_namespace(parent, LOOKUP_METADATA).unwrap();
    let Frame::Retain {
        request_id, scope, ..
    } = frame
    else {
        unreachable!()
    };
    let mut leases =
        sunlight_ipc::vfs_rpc::retention::Table::<(), 1>::new(parent.registration).unwrap();
    let reply = leases
        .retain_reply(
            parent.registration,
            PendingRetain::new(parent.registration, scope, request_id).unwrap(),
            || Ok::<(), sunlight_fs::FsError>(()),
        )
        .unwrap();
    runtime
        .install_retained(
            parent.registration,
            request_id,
            &reply.encode().unwrap(),
            Some(parent),
        )
        .unwrap();
    let child = reserve(&mut sched, 11);
    (runtime, sched, caps, parent, child)
}

fn reserve(sched: &mut Scheduler, pid: usize) -> Identity {
    let ticket = sched
        .pending_children
        .reserve(
            pid,
            Parent {
                pid: 10,
                incarnation: 11,
                image_generation: 12,
            },
            false,
        )
        .unwrap();
    let child = Identity {
        ticket,
        incarnation: pid as u64 + 10,
        image_generation: pid as u64 + 100,
    };
    assert!(sched
        .pending_children
        .finish(
            child,
            alloc::boxed::Box::new(Process {
                pid,
                trusted_vfs_service: false,
                exiting: false,
                vfs_root_metadata: false,
                vfs_boot_root: false,
                vfs_file_routed: false,
                vfs_root_rebased: false,
                vfs_stat_routed: false,
                cwd: "/".into(),
                uid: 1000,
                gid: 100,
                state: ProcessState::Ready,
                pending_call: None,
                ipc_reply_target: None,
                ipc_endpoint: None,
                foreground: Foreground {
                    incarnation: child.incarnation,
                    ..Default::default()
                },
                address_space: AddressSpace {
                    generation: child.image_generation
                },
            }),
            |_| true
        )
        .is_ok());
    child
}

fn pull(
    runtime: &mut Runtime,
    sched: &Scheduler,
    caps: &CapabilityBroker,
    claim: RemoteClaim,
) -> [u8; FRAME_BYTES] {
    let request = call(
        runtime,
        sched,
        caps,
        3,
        api::NEXT,
        claim.registration.epoch,
        &[],
    )
    .unwrap()
    .unwrap();
    assert!(matches!(Frame::decode(&request).unwrap(),
        Frame::Retain { registration, request_id, .. }
        if registration == claim.registration && request_id == claim.request_id));
    request
}

fn reply_for(
    request: &[u8; FRAME_BYTES],
    changed: impl FnOnce(&mut Scope, &mut u64),
    refuse: bool,
) -> Frame {
    let Frame::Retain {
        registration,
        mut scope,
        mut request_id,
    } = Frame::decode(request).unwrap()
    else {
        unreachable!()
    };
    changed(&mut scope, &mut request_id);
    sunlight_ipc::vfs_rpc::retention::Table::<(), 1>::new(registration)
        .unwrap()
        .retain_reply(
            registration,
            PendingRetain::new(registration, scope, request_id).unwrap(),
            || {
                if refuse {
                    Err(sunlight_fs::FsError::PermissionDenied)
                } else {
                    Ok(())
                }
            },
        )
        .unwrap()
}

fn send(
    runtime: &mut Runtime,
    sched: &Scheduler,
    caps: &CapabilityBroker,
    claim: RemoteClaim,
    frame: Frame,
) -> bool {
    call(
        runtime,
        sched,
        caps,
        3,
        api::REPLY,
        claim.registration.epoch,
        &frame.encode().unwrap(),
    )
    .is_ok()
}

#[test]
fn spawn_ack_before_or_after_park_is_single_consumption() {
    for delayed in [false, true] {
        let (mut runtime, mut sched, caps, _, child) = fixture();
        let claim = runtime.stage_spawn(1, child, &mut sched, &caps).unwrap();
        let request = pull(&mut runtime, &sched, &caps, claim);
        assert!(call(
            &mut runtime,
            &sched,
            &caps,
            3,
            api::DERIVE_SOURCE,
            claim.registration.epoch,
            &request
        )
        .unwrap()
        .is_some());
        if delayed {
            assert_eq!(runtime.wait_spawn(1, claim, &mut sched), Ok(None));
            assert_eq!(sched.processes[1].state, ProcessState::BlockedOnIpc);
            assert_eq!(sched.processes[0].state, ProcessState::Running);
        }
        let reply = reply_for(&request, |_, _| {}, false);
        assert!(send(&mut runtime, &sched, &caps, claim, reply));
        assert!(!send(&mut runtime, &sched, &caps, claim, reply));
        runtime.poll_spawns(&mut sched, 0);
        assert_eq!(
            sched.processes[1].state,
            if delayed {
                ProcessState::Ready
            } else {
                ProcessState::Running
            }
        );
        assert!(matches!(
            runtime.wait_spawn(1, claim, &mut sched),
            Ok(Some(Terminal::Retained(_)))
        ));
        assert_eq!(
            runtime.wait_spawn(1, claim, &mut sched),
            Err(Error::Identity)
        );
        assert!(runtime.spawns.iter().all(Option::is_none));
        assert!(sched
            .processes
            .iter()
            .all(|process| process.pid != child.ticket.pid));
        assert!(sched.pending_children.private(child).is_some());
    }
}

#[test]
fn spawn_rejects_wrong_request_epoch_incarnation_and_waiter() {
    let (mut runtime, mut sched, caps, _, child) = fixture();
    let mut wrong = child;
    wrong.incarnation += 1;
    assert_eq!(
        runtime.stage_spawn(1, wrong, &mut sched, &caps),
        Err(Error::Identity)
    );
    let claim = runtime.stage_spawn(1, child, &mut sched, &caps).unwrap();
    let request = pull(&mut runtime, &sched, &caps, claim);
    assert_eq!(
        runtime.wait_spawn(0, claim, &mut sched),
        Err(Error::Identity)
    );
    assert!(!send(
        &mut runtime,
        &sched,
        &caps,
        claim,
        reply_for(&request, |_, id| *id += 1, false)
    ));
    assert!(!send(
        &mut runtime,
        &sched,
        &caps,
        claim,
        reply_for(&request, |scope, _| scope.caller.incarnation += 1, false)
    ));
    let reply = reply_for(&request, |_, _| {}, false);
    let mut wrong_epoch = claim;
    wrong_epoch.registration.epoch += 1;
    assert!(!send(&mut runtime, &sched, &caps, wrong_epoch, reply));
    let bytes = reply.encode().unwrap();
    assert!(call(
        &mut runtime,
        &sched,
        &caps,
        10,
        api::REPLY,
        claim.registration.epoch,
        &bytes
    )
    .is_err());
    assert_eq!(runtime.wait_spawn(1, claim, &mut sched), Ok(None));
    sched.processes[1].uid += 1;
    assert!(send(&mut runtime, &sched, &caps, claim, reply));
    runtime.poll_spawns(&mut sched, 0);
    assert_eq!(
        runtime.wait_spawn(1, claim, &mut sched),
        Ok(Some(Terminal::InvalidAcknowledgement))
    );
}

#[test]
fn spawn_terminal_failures_do_not_drain_uncertain_remote_ownership() {
    for expected in [
        Terminal::Refused(Refusal::Resolve),
        Terminal::Transport,
        Terminal::Cancelled,
        Terminal::Timeout,
        Terminal::AuthorityRetired,
    ] {
        let (mut runtime, mut sched, caps, _, child) = fixture();
        let claim = runtime.stage_spawn(1, child, &mut sched, &caps).unwrap();
        let request = pull(&mut runtime, &sched, &caps, claim);
        assert_eq!(runtime.wait_spawn(1, claim, &mut sched), Ok(None));
        match expected {
            Terminal::Refused(_) => assert!(send(
                &mut runtime,
                &sched,
                &caps,
                claim,
                reply_for(&request, |_, _| {}, true)
            )),
            Terminal::Transport => runtime.fail_spawn_copy(claim.registration, claim.request_id),
            Terminal::Cancelled => runtime.cancel_spawns_for(process_identity(&sched.processes[1])),
            Terminal::Timeout => sched.global_tick = 5 * crate::timekeeping::TICK_HZ,
            Terminal::AuthorityRetired => runtime.retire_registration(claim.registration),
            _ => unreachable!(),
        }
        let now = sched.global_tick;
        runtime.poll_spawns(&mut sched, now);
        assert_eq!(sched.processes[1].state, ProcessState::Ready);
        sched.processes[1].state = ProcessState::BlockedOnIpc;
        runtime.poll_spawns(&mut sched, now);
        assert_eq!(sched.processes[1].state, ProcessState::BlockedOnIpc);
        assert_eq!(runtime.wait_spawn(1, claim, &mut sched), Ok(Some(expected)));
        drop(
            sched
                .pending_children
                .cancel(child.ticket, Some(child))
                .unwrap(),
        );
        sched.pending_children.reclaimed(child.ticket);
        runtime.handoff_spawn(claim);
        runtime.poll_spawns(&mut sched, now);
        assert!(sched
            .pending_children
            .pids()
            .any(|pid| pid == child.ticket.pid));
        if !matches!(expected, Terminal::Refused(_)) {
            assert!(send(
                &mut runtime,
                &sched,
                &caps,
                claim,
                reply_for(&request, |_, _| {}, false)
            ));
        }
        let remote = runtime
            .remote
            .iter()
            .flatten()
            .find(|remote| remote.registration == claim.registration)
            .unwrap();
        assert_eq!(
            remote.ownership.releases().filter(|frame| {
                matches!(frame, Frame::Release(receipt) if receipt.request_id() == claim.request_id)
            }).count(),
            usize::from(!matches!(expected, Terminal::Refused(_)))
        );
        assert!(runtime.spawns.iter().all(Option::is_none));
    }
}

#[test]
fn spawn_capacity_is_bounded_and_unsent_cancellation_can_release_pid() {
    let (mut runtime, mut sched, caps, parent, child) = fixture();
    for pid in 12..19 {
        reserve(&mut sched, pid);
    }
    assert_eq!(sched.pending_children.pids().count(), 8);
    assert_eq!(
        sched.pending_children.reserve(
            99,
            Parent {
                pid: 10,
                incarnation: 11,
                image_generation: 12
            },
            false
        ),
        Err(crate::process::pending_state::Error::Capacity)
    );
    let claim = runtime.stage_spawn(1, child, &mut sched, &caps).unwrap();
    runtime.cancel_spawns_for(parent.caller);
    assert_eq!(
        runtime.wait_spawn(1, claim, &mut sched),
        Ok(Some(Terminal::Cancelled))
    );
    drop(
        sched
            .pending_children
            .cancel(child.ticket, Some(child))
            .unwrap(),
    );
    sched.pending_children.reclaimed(child.ticket);
    runtime.poll_spawns(&mut sched, 0);
    assert_eq!(sched.pending_children.pids().count(), 7);
    assert!(runtime.next_control(parent.registration).unwrap().is_none());
    let second = reserve(&mut sched, child.ticket.pid);
    for index in 0..63 {
        let mut identity = parent;
        identity.caller.pid = 1000 + index;
        runtime.queue_namespace(identity, LOOKUP_METADATA).unwrap();
    }
    assert_eq!(
        runtime.stage_spawn(1, second, &mut sched, &caps),
        Err(Error::Capacity)
    );
    assert!(sched.pending_children.remote(second).is_none());
}

#[test]
fn spawn_vfs_policy_and_lease_settlement_release_private_pid() {
    let _serial = crate::ipc::vfs_resources::TEST_LEDGER_LOCK.lock();
    let (mut runtime, mut sched, caps, parent) = setup();
    sched.processes[1].vfs_root_metadata = true;
    sched.processes[1].vfs_boot_root = true;
    let namespace = crate::mounted_cwd_namespace(parent.uid, parent.gid);
    let mut service = Service::<2, 8, 1>::new();
    let poll = |service: &mut Service<2, 8, 1>, runtime: &mut Runtime, sched: &mut Scheduler| {
        service.poll_root_control(
            &mut CwdChannel {
                kernel: runtime,
                sched,
                caps: &caps,
                reject_commit: false,
            },
            &namespace,
        );
    };
    poll(&mut service, &mut runtime, &mut sched);
    let child = reserve(&mut sched, 11);
    let claim = runtime.stage_spawn(1, child, &mut sched, &caps).unwrap();
    assert_eq!(runtime.wait_spawn(1, claim, &mut sched), Ok(None));
    poll(&mut service, &mut runtime, &mut sched);
    let terminal = runtime.wait_spawn(1, claim, &mut sched);
    let Ok(Some(Terminal::Retained(receipt))) = terminal else {
        panic!("expected an authenticated receipt from the real VFS policy: {terminal:?}");
    };
    assert!(service.metadata(parent.registration, receipt).is_ok());
    assert!(runtime
        .namespaces
        .lookup(AdmissionIdentity {
            caller: receipt.scope().caller,
            ..parent
        })
        .is_err());

    drop(
        sched
            .pending_children
            .cancel(child.ticket, Some(child))
            .unwrap(),
    );
    runtime.handoff_spawn(claim);
    sched.pending_children.reclaimed(child.ticket);
    runtime.poll_spawns(&mut sched, 0);
    assert_eq!(sched.pending_children.pids().count(), 1);
    poll(&mut service, &mut runtime, &mut sched);
    runtime.poll_spawns(&mut sched, 0);
    assert_eq!(sched.pending_children.pids().count(), 0);
    assert!(service.metadata(parent.registration, receipt).is_err());
}
