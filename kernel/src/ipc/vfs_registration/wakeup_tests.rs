use super::*;
use crate::sched::{AddressSpace, Foreground, Process};
use sunlight_ipc::vfs_rpc::context::LOOKUP_METADATA;

fn setup() -> (Runtime, Scheduler, AdmissionIdentity) {
    let mut runtime = Runtime::new();
    let registration = runtime
        .register(EndpointOwner {
            endpoint: 0,
            pid: 3,
            incarnation: 4,
            address_space_generation: 5,
        })
        .unwrap();
    let sched = Scheduler {
        global_tick: 0,
        processes: alloc::vec![Process {
            pid: 3,
            trusted_vfs_service: true,
            vfs_root_metadata: false,
            cwd: alloc::string::String::from("/"),
            exiting: false,
            uid: 0,
            gid: 0,
            foreground: Foreground { incarnation: 4 },
            address_space: AddressSpace { generation: 5 },
            state: ProcessState::BlockedOnIpc,
            pending_call: None,
            ipc_reply_target: None,
            ipc_endpoint: Some(0),
        }],
    };
    let identity = AdmissionIdentity {
        registration,
        caller: Caller {
            pid: 10,
            incarnation: 11,
            address_space_generation: 12,
        },
        uid: 1000,
        gid: 100,
        groups: [],
    };
    (runtime, sched, identity)
}

#[test]
fn hints_cover_both_sides_of_blocking_and_retry_without_consuming_work() {
    let (mut runtime, mut sched, identity) = setup();
    wake_control_receivers_runtime(&runtime, &mut sched, 10);
    assert_eq!(sched.processes[0].state, ProcessState::BlockedOnIpc);
    assert!(runtime
        .take_control_wakeup(&sched.processes[0], 0, 10)
        .is_none());

    // Publication after blocking is discovered by the tick. The wake itself
    // consumes no frame or retry budget, so receive can still return its hint.
    runtime.queue_namespace(identity, LOOKUP_METADATA).unwrap();
    wake_control_receivers_runtime(&runtime, &mut sched, 10);
    assert_eq!(sched.processes[0].state, ProcessState::Ready);
    let hint = runtime
        .take_control_wakeup(&sched.processes[0], 0, 10)
        .unwrap();
    assert_eq!(
        (hint.label, hint.badge, hint.word_count, hint.cap_count),
        (CONTROL_WAKEUP, 0, 0, 0)
    );
    assert!(runtime
        .take_control_wakeup(&sched.processes[0], 0, 10)
        .is_none());
    sched.processes[0].state = ProcessState::BlockedOnIpc;
    wake_control_receivers_runtime(&runtime, &mut sched, 10);
    assert_eq!(sched.processes[0].state, ProcessState::BlockedOnIpc);
    wake_control_receivers_runtime(&runtime, &mut sched, 11);
    assert_eq!(sched.processes[0].state, ProcessState::Ready);

    // Publication before blocking is found directly by the final receive check.
    assert!(runtime
        .take_control_wakeup(&sched.processes[0], 0, 11)
        .is_some());
    let frame = runtime
        .next_control(identity.registration)
        .unwrap()
        .unwrap();
    assert!(matches!(frame, Frame::Retain { .. }));
    // Unknown single-send retain ownership cannot be retried by waking again.
    assert!(runtime
        .take_control_wakeup(&sched.processes[0], 0, 12)
        .is_none());
    assert!(runtime
        .next_control(identity.registration)
        .unwrap()
        .is_none());

    runtime.retire_process(identity.caller);
    assert!(runtime
        .take_control_wakeup(&sched.processes[0], 0, 12)
        .is_some());
    let event = runtime
        .next_control(identity.registration)
        .unwrap()
        .unwrap();
    assert!(matches!(event, Frame::CallerRetired { .. }));
    // Discarding the hint/event copy changes no obligation. Retry next tick.
    assert!(runtime
        .take_control_wakeup(&sched.processes[0], 0, 13)
        .is_some());
    assert_eq!(runtime.next_control(identity.registration), Ok(Some(event)));
}

#[test]
fn wakeups_match_full_live_owner_and_only_idle_receivers() {
    let (mut runtime, mut sched, identity) = setup();
    runtime.queue_namespace(identity, LOOKUP_METADATA).unwrap();
    for field in 0..9 {
        match field {
            0 => sched.processes[0].pid += 1,
            1 => sched.processes[0].foreground.incarnation += 1,
            2 => sched.processes[0].address_space.generation += 1,
            3 => sched.processes[0].trusted_vfs_service = false,
            4 => sched.processes[0].exiting = true,
            5 => sched.processes[0].ipc_endpoint = Some(1),
            6 => sched.processes[0].pending_call = Some(()),
            7 => sched.processes[0].ipc_reply_target = Some(()),
            _ => sched.processes[0].state = ProcessState::Running,
        }
        wake_control_receivers_runtime(&runtime, &mut sched, 10);
        assert_ne!(sched.processes[0].state, ProcessState::Ready);
        if field < 5 {
            assert!(runtime
                .take_control_wakeup(&sched.processes[0], 0, 10)
                .is_none());
        }
        let (_, restored, _) = setup();
        sched = restored;
    }
    assert!(runtime
        .take_control_wakeup(&sched.processes[0], 1, 10)
        .is_none());
    // Rejected owners never consume the actual owner's notification budget.
    wake_control_receivers_runtime(&runtime, &mut sched, 10);
    assert_eq!(sched.processes[0].state, ProcessState::Ready);
    assert!(runtime
        .take_control_wakeup(&sched.processes[0], 0, 10)
        .is_some());
}

#[test]
fn old_epoch_work_wakes_and_coalesces_with_replacement_and_release_retries() {
    let (mut runtime, sched, identity) = setup();
    let old = identity.registration;
    runtime.queue_namespace(identity, LOOKUP_METADATA).unwrap();
    let Frame::Retain {
        scope, request_id, ..
    } = runtime.next_control(old).unwrap().unwrap()
    else {
        panic!()
    };
    let receipt = sunlight_ipc::vfs_rpc::retention::Table::<(), 1>::new(old)
        .unwrap()
        .retain(old, request_id, scope, || Ok::<_, ()>(()))
        .unwrap();
    runtime
        .install_retained(
            old,
            request_id,
            &Frame::Retained(receipt).encode().unwrap(),
            Some(identity),
        )
        .unwrap();
    runtime.retire_registration(old);
    let replacement = runtime.register(old.owner).unwrap();
    runtime
        .queue_namespace(
            AdmissionIdentity {
                registration: replacement,
                ..identity
            },
            LOOKUP_METADATA,
        )
        .unwrap();
    // Both epochs belong to the same exact live service. One hint covers both.
    assert!(runtime
        .take_control_wakeup(&sched.processes[0], 0, 10)
        .is_some());
    assert!(runtime
        .remote
        .iter()
        .flatten()
        .all(|r| r.last_wake_tick == Some(10)));
    assert!(runtime
        .take_control_wakeup(&sched.processes[0], 0, 10)
        .is_none());
    assert_eq!(runtime.next_control(old), Ok(Some(Frame::Release(receipt))));
    assert!(runtime
        .take_control_wakeup(&sched.processes[0], 0, 11)
        .is_some());
    runtime
        .released(old, request_id, &Frame::Released(receipt).encode().unwrap())
        .unwrap();
    runtime.next_control(replacement).unwrap().unwrap();
    // Live caller-delivery rows alone are not work and do not cause polling.
    assert!(runtime
        .take_control_wakeup(&sched.processes[0], 0, 12)
        .is_none());
    runtime.retire_process(identity.caller);
    assert!(runtime
        .take_control_wakeup(&sched.processes[0], 0, 12)
        .is_some());
}

#[test]
fn old_work_wakes_a_different_replacement_endpoint_of_the_same_owner() {
    let (mut runtime, mut sched, identity) = setup();
    runtime.queue_namespace(identity, LOOKUP_METADATA).unwrap();
    runtime.retire_registration(identity.registration);
    runtime
        .register(EndpointOwner {
            endpoint: 9,
            ..identity.registration.owner
        })
        .unwrap();
    sched.processes[0].ipc_endpoint = Some(9);
    wake_control_receivers_runtime(&runtime, &mut sched, 10);
    assert_eq!(sched.processes[0].state, ProcessState::Ready);
    assert!(runtime
        .take_control_wakeup(&sched.processes[0], 9, 10)
        .is_some());
    assert!(runtime
        .take_control_wakeup(&sched.processes[0], 0, 10)
        .is_none());
    assert!(runtime
        .take_control_wakeup(&sched.processes[0], 8, 11)
        .is_none());
    assert!(runtime
        .take_control_wakeup(&sched.processes[0], 9, 11)
        .is_some());
}
