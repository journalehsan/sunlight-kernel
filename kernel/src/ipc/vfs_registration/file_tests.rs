use super::*;
use crate::sched::{AddressSpace, Foreground, Process};

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
        processes: alloc::vec![Process {
            pid: 10,
            trusted_vfs_service: false,
            vfs_root_metadata: false,
            vfs_boot_root: false,
            vfs_root_rebased: false,
            vfs_downloads_cwd: false,
            shared_owner: None,
            vfs_stat_routed: false,
            vfs_file_routed: true,
            cwd: alloc::string::String::from("/"),
            exiting: false,
            uid: 1000,
            gid: 100,
            foreground: Foreground {
                incarnation: 11,
                ..Default::default()
            },
            address_space: AddressSpace { generation: 12 },
            state: ProcessState::Running,
            pending_call: None,
            ipc_reply_target: None,
            ipc_endpoint: None,
        }],
    };
    (runtime, sched, identity)
}

fn insert_acked_caller(runtime: &mut Runtime, caller: Caller) {
    let registration = runtime.registry.active().unwrap();
    let remote = runtime
        .remote
        .iter_mut()
        .flatten()
        .find(|remote| remote.registration == registration)
        .unwrap();
    remote.callers[0] = Some(CallerDelivery {
        caller,
        id: 1,
        retired: false,
        delivered: false,
        metadata_acked: true,
        file_retired: false,
        file_delivered: false,
        file_transfer_pending: false,
    });
    remote.last_caller_id = 1;
}

fn reply_frame(
    identity: AdmissionIdentity,
    request_id: u64,
    op: u64,
    errno: u64,
    result0: u64,
    payload: &[u8],
) -> [u8; api::FRAME_BYTES] {
    api::Reply {
        registration: identity.registration,
        caller: identity.caller,
        request_id,
        op,
        errno,
        result0,
        result1: 0,
        length: payload.len(),
        payload,
    }
    .encode()
    .unwrap()
}

#[test]
fn publishing_parks_the_caller_and_the_exact_reply_resumes_it_once() {
    let (mut runtime, mut sched, identity) = setup();
    runtime
        .publish_file(
            identity,
            FileOperation::Read {
                token: 7,
                length: 8,
            },
            None,
            &mut sched,
        )
        .unwrap();
    assert_eq!(sched.processes[0].state, ProcessState::BlockedOnIpc);
    // One in-flight request per caller, selected by the caller identity.
    assert_eq!(
        runtime.publish_file(
            identity,
            FileOperation::Read {
                token: 7,
                length: 8
            },
            None,
            &mut sched
        ),
        Err(Error::Busy)
    );
    // Pull is idempotent until the exact reply is accepted.
    let mut frame = [0u8; api::FRAME_BYTES];
    let id = runtime
        .next_file(identity.registration, &mut frame)
        .unwrap()
        .unwrap();
    let request = api::Request::decode(&frame).unwrap();
    assert_eq!(request.op, api::READ);
    assert_eq!(request.token, 7);
    assert_eq!(request.arg1, 8);
    assert_eq!(
        runtime
            .next_file(identity.registration, &mut frame)
            .unwrap(),
        Some(id)
    );
    // A foreign epoch cannot pull another owner's work.
    let mut foreign = identity.registration;
    foreign.epoch += 1;
    assert_eq!(runtime.next_file(foreign, &mut frame), Err(Error::Identity));
    // A reply that does not name the exact caller is refused, not delivered.
    let mut wrong = api::Reply {
        registration: identity.registration,
        caller: identity.caller,
        request_id: id,
        op: api::READ,
        errno: 0,
        result0: 3,
        result1: 0,
        length: 3,
        payload: b"abc",
    };
    wrong.caller.incarnation += 1;
    assert_eq!(
        runtime.reply_file(identity.registration, &wrong.encode().unwrap(), &mut sched),
        Err(Error::Identity)
    );
    assert_eq!(sched.processes[0].state, ProcessState::BlockedOnIpc);
    // The exact reply wakes the parked caller and owns the payload once.
    let bytes = reply_frame(identity, id, api::READ, 0, 3, b"abc");
    runtime
        .reply_file(identity.registration, &bytes, &mut sched)
        .unwrap();
    assert_eq!(sched.processes[0].state, ProcessState::Ready);
    let completion = runtime.take_file(identity.caller, api::READ).unwrap();
    assert_eq!((completion.errno, completion.result0), (0, 3));
    assert_eq!(completion.payload.as_slice(), b"abc");
    assert!(runtime.take_file(identity.caller, api::READ).is_none());
    assert_eq!(
        runtime
            .next_file(identity.registration, &mut frame)
            .unwrap(),
        None
    );
}

#[test]
fn expiry_is_terminal_and_wakes_without_retransmission() {
    let (mut runtime, mut sched, identity) = setup();
    runtime
        .publish_file(identity, FileOperation::Stat { token: 5 }, None, &mut sched)
        .unwrap();
    assert_eq!(sched.processes[0].state, ProcessState::BlockedOnIpc);
    sched.global_tick = TIMEOUT_TICKS;
    runtime.expire_files(sched.global_tick);
    runtime.wake_file_callers(&mut sched);
    assert_eq!(sched.processes[0].state, ProcessState::Ready);
    let completion = runtime.take_file(identity.caller, api::STAT).unwrap();
    assert_eq!(completion.errno, 5);
    assert!(completion.payload.is_empty());
    let mut frame = [0u8; api::FRAME_BYTES];
    assert_eq!(
        runtime
            .next_file(identity.registration, &mut frame)
            .unwrap(),
        None
    );
    // No request remains to correlate, so a late reply is refused outright.
    let bytes = reply_frame(identity, 1, api::STAT, 0, 0, &[0u8; api::STAT_BYTES]);
    assert_eq!(
        runtime.reply_file(identity.registration, &bytes, &mut sched),
        Err(Error::Identity)
    );
}

#[test]
fn retirement_clears_caller_and_registration_publications() {
    let (mut runtime, mut sched, identity) = setup();
    let mut frame = [0u8; api::FRAME_BYTES];
    runtime
        .publish_file(
            identity,
            FileOperation::Close { token: 9 },
            None,
            &mut sched,
        )
        .unwrap();
    runtime.retire_file_publications(identity.caller);
    assert_eq!(
        runtime
            .next_file(identity.registration, &mut frame)
            .unwrap(),
        None
    );
    // The caller is still parked; retirement of the publication does not wake.
    sched.processes[0].state = ProcessState::Running;
    runtime
        .publish_file(
            identity,
            FileOperation::Close { token: 9 },
            None,
            &mut sched,
        )
        .unwrap();
    runtime.retire_file_registration(identity.registration);
    assert_eq!(
        runtime
            .next_file(identity.registration, &mut frame)
            .unwrap(),
        None
    );
}

#[test]
fn open_requires_a_retained_receipt_and_carries_exact_correlation() {
    let (mut runtime, mut sched, identity) = setup();
    assert_eq!(
        runtime.publish_file(
            identity,
            FileOperation::Open {
                anchor: 0,
                path: "data",
                access: api::ACCESS_READ,
                mode: 0,
            },
            None,
            &mut sched
        ),
        Err(Error::NotReady)
    );
    let receipt = Receipt::from_parts(
        identity.registration,
        Scope {
            caller: identity.caller,
            namespace_id: 21,
            namespace_generation: 22,
        },
        31,
        32,
    )
    .unwrap();
    runtime
        .publish_file(
            identity,
            FileOperation::Open {
                anchor: 0,
                path: "data",
                access: api::ACCESS_READ,
                mode: 0,
            },
            Some(receipt),
            &mut sched,
        )
        .unwrap();
    let mut frame = [0u8; api::FRAME_BYTES];
    let id = runtime
        .next_file(identity.registration, &mut frame)
        .unwrap()
        .unwrap();
    let request = api::Request::decode(&frame).unwrap();
    assert_eq!(request.op, api::OPEN);
    assert_eq!(request.payload, b"data");
    assert_eq!(request.arg0, api::ACCESS_READ);
    assert_eq!(request.namespace_id, 21);
    assert_eq!(request.namespace_generation, 22);
    assert_eq!(request.receipt_id, 31);
    assert_eq!(request.receipt_generation, 32);
    assert_eq!((request.uid, request.gid), (identity.uid, identity.gid));
    // The reply operation must match the authenticated request.
    let bytes = reply_frame(identity, id, api::CLOSE, 0, 0, &[]);
    assert_eq!(
        runtime.reply_file(identity.registration, &bytes, &mut sched),
        Err(Error::Identity)
    );
}

fn complete_open(runtime: &mut Runtime, sched: &mut Scheduler, identity: AdmissionIdentity) -> u64 {
    let receipt = Receipt::from_parts(
        identity.registration,
        Scope {
            caller: identity.caller,
            namespace_id: 21,
            namespace_generation: 22,
        },
        31,
        32,
    )
    .unwrap();
    sched.processes[0].state = ProcessState::Running;
    runtime
        .publish_file(
            identity,
            FileOperation::Open {
                anchor: 0,
                path: "data",
                access: api::ACCESS_READ,
                mode: 0,
            },
            Some(receipt),
            sched,
        )
        .unwrap();
    let mut bytes = [0; api::FRAME_BYTES];
    let id = runtime
        .next_file(identity.registration, &mut bytes)
        .unwrap()
        .unwrap();
    runtime
        .reply_file(
            identity.registration,
            &reply_frame(identity, id, api::OPEN, 0, id, &[]),
            sched,
        )
        .unwrap();
    id
}

#[test]
fn successful_install_consumes_open_and_retired_epoch_cannot_install() {
    let (mut runtime, mut sched, identity) = setup();
    let id = complete_open(&mut runtime, &mut sched, identity);
    let authority = Some((identity.caller, identity.registration.epoch));
    assert_eq!(runtime.check_descriptor(identity, authority), Ok(()));
    let wrong_image = Caller {
        address_space_generation: identity.caller.address_space_generation + 1,
        ..identity.caller
    };
    assert_eq!(
        runtime.check_descriptor(identity, Some((wrong_image, identity.registration.epoch))),
        Err(Error::NotReady)
    );
    let slot = runtime.caller_publication(identity.caller).unwrap();
    let installed = runtime
        .finish_open(slot, |token, epoch| {
            assert_eq!((token, epoch), (id, identity.registration.epoch));
            Ok(3)
        })
        .unwrap();
    assert_eq!((installed.errno, installed.result0), (0, 3));
    assert!(runtime.file_publications.iter().all(Option::is_none));

    complete_open(&mut runtime, &mut sched, identity);
    let slot = runtime.caller_publication(identity.caller).unwrap();
    runtime.retire_registration(identity.registration);
    let new = runtime.register(identity.registration.owner).unwrap();
    assert_ne!(new.epoch, identity.registration.epoch);
    assert_eq!(
        runtime.check_descriptor(identity, authority),
        Err(Error::NotReady)
    );
    assert_eq!(
        runtime.check_descriptor(
            AdmissionIdentity {
                registration: new,
                ..identity
            },
            authority
        ),
        Err(Error::NotReady)
    );
    assert_eq!(
        runtime
            .finish_open(slot, |_, _| panic!("retired OPEN installed"))
            .unwrap()
            .errno,
        5
    );
    assert!(runtime.file_publications.iter().all(Option::is_none));
}

#[test]
fn failed_install_retries_exact_close_until_ack_without_slot_growth() {
    let (mut runtime, mut sched, identity) = setup();
    for _ in 0..100 {
        let id = complete_open(&mut runtime, &mut sched, identity);
        let slot = runtime.caller_publication(identity.caller).unwrap();
        assert_eq!(runtime.finish_open(slot, |_, _| Err(24)).unwrap().errno, 24);
        assert!(runtime.caller_publication(identity.caller).is_none());
        let mut bytes = [0; api::FRAME_BYTES];
        assert_eq!(
            runtime.next_file(identity.registration, &mut bytes),
            Ok(Some(id))
        );
        let close = api::Request::decode(&bytes).unwrap();
        assert_eq!(
            (close.registration, close.caller, close.op, close.token),
            (identity.registration, identity.caller, api::CLOSE, id)
        );
        let refused = reply_frame(identity, id, api::CLOSE, 5, 0, &[]);
        assert_eq!(
            runtime.reply_file(identity.registration, &refused, &mut sched),
            Err(Error::Protocol)
        );
        sched.global_tick += TIMEOUT_TICKS;
        runtime.expire_files(sched.global_tick);
        let mut retry = [0; api::FRAME_BYTES];
        assert_eq!(
            runtime.next_file(identity.registration, &mut retry),
            Ok(Some(id))
        );
        assert_eq!(retry, bytes);
        let absent = reply_frame(identity, id, api::CLOSE, 9, 0, &[]);
        runtime
            .reply_file(identity.registration, &absent, &mut sched)
            .unwrap();
        assert!(runtime.file_publications.iter().all(Option::is_none));
        assert_eq!(
            runtime.reply_file(identity.registration, &absent, &mut sched),
            Err(Error::Identity)
        );
    }
}

#[test]
fn caller_retirement_preserves_uninstalled_open_cleanup_until_epoch_withdrawal() {
    let (mut runtime, mut sched, identity) = setup();
    let id = complete_open(&mut runtime, &mut sched, identity);
    runtime.retire_process(identity.caller);
    let mut bytes = [0; api::FRAME_BYTES];
    assert_eq!(
        runtime.next_file(identity.registration, &mut bytes),
        Ok(Some(id))
    );
    let close = api::Request::decode(&bytes).unwrap();
    assert_eq!(
        (close.caller, close.op, close.token),
        (identity.caller, api::CLOSE, id)
    );
    runtime.retire_registration(identity.registration);
    let new = runtime.register(identity.registration.owner).unwrap();
    assert_eq!(runtime.next_file(new, &mut bytes), Ok(None));
    assert!(runtime.file_publications.iter().all(Option::is_none));
}

#[test]
fn file_owner_barrier_waits_for_transfer_publication_and_reply() {
    let (mut runtime, mut sched, identity) = setup();
    insert_acked_caller(&mut runtime, identity.caller);
    runtime.expect_file_transfer(identity.caller);
    runtime.retire_process(identity.caller);
    // The retirement event exists, but the publication gap keeps it closed.
    assert_eq!(runtime.next_control(identity.registration), Ok(None));

    let new = Caller {
        address_space_generation: identity.caller.address_space_generation + 1,
        ..identity.caller
    };
    runtime
        .publish_file_transfer(
            identity.registration,
            identity.caller,
            new,
            7,
            sched.global_tick,
        )
        .unwrap();
    runtime.finish_file_transfer(identity.caller);
    // The transfer is published but not yet replied.
    assert_eq!(runtime.next_control(identity.registration), Ok(None));

    let mut frame = [0u8; api::FRAME_BYTES];
    let id = runtime
        .next_file(identity.registration, &mut frame)
        .unwrap()
        .unwrap();
    let request = api::Request::decode(&frame).unwrap();
    assert_eq!(request.op, api::TRANSFER);
    assert_eq!(request.caller, identity.caller);
    assert_eq!(api::decode_caller(request.payload), Ok(new));

    let reply = reply_frame(identity, id, api::TRANSFER, 0, 7, &[]);
    runtime
        .reply_file(identity.registration, &reply, &mut sched)
        .unwrap();
    assert!(matches!(
        runtime.next_control(identity.registration),
        Ok(Some(Frame::FileOwnerRetired { caller, .. })) if caller == identity.caller
    ));
}

#[test]
fn transfer_timeout_releases_the_barrier_without_retransmission() {
    let (mut runtime, mut sched, identity) = setup();
    insert_acked_caller(&mut runtime, identity.caller);
    runtime.expect_file_transfer(identity.caller);
    runtime.retire_process(identity.caller);
    let new = Caller {
        address_space_generation: identity.caller.address_space_generation + 1,
        ..identity.caller
    };
    runtime
        .publish_file_transfer(
            identity.registration,
            identity.caller,
            new,
            7,
            sched.global_tick,
        )
        .unwrap();
    runtime.finish_file_transfer(identity.caller);
    sched.global_tick = TIMEOUT_TICKS;
    runtime.expire_files(sched.global_tick);
    let mut frame = [0u8; api::FRAME_BYTES];
    assert_eq!(
        runtime
            .next_file(identity.registration, &mut frame)
            .unwrap(),
        None
    );
    assert!(matches!(
        runtime.next_control(identity.registration),
        Ok(Some(Frame::FileOwnerRetired { .. }))
    ));
}

#[test]
fn transfer_rejects_an_unrelated_owner_relation() {
    let (mut runtime, sched, identity) = setup();
    let unrelated = Caller {
        pid: identity.caller.pid + 1,
        incarnation: identity.caller.incarnation + 1,
        address_space_generation: identity.caller.address_space_generation + 1,
    };
    assert_eq!(
        runtime.publish_file_transfer(
            identity.registration,
            identity.caller,
            unrelated,
            7,
            sched.global_tick,
        ),
        Err(Error::Identity)
    );
}

#[test]
fn fire_and_forget_close_does_not_park_and_does_not_block_a_later_op() {
    let (mut runtime, mut sched, identity) = setup();
    // The final kernel reference is already gone, so the caller stays runnable:
    // a close that parked would resume and re-execute close(2) on a dead fd.
    runtime
        .publish_file_close(identity.registration, identity.caller, 9, sched.global_tick)
        .unwrap();
    assert_eq!(sched.processes[0].state, ProcessState::Running);
    let mut frame = [0u8; api::FRAME_BYTES];
    let close_id = runtime
        .next_file(identity.registration, &mut frame)
        .unwrap()
        .unwrap();
    let request = api::Request::decode(&frame).unwrap();
    assert_eq!(request.op, api::CLOSE);
    assert_eq!(request.token, 9);
    // The pending close names the caller but does not own it, so a later
    // operation from the same caller is admitted rather than refused Busy.
    runtime
        .publish_file(
            identity,
            FileOperation::Read {
                token: 7,
                length: 8,
            },
            None,
            &mut sched,
        )
        .unwrap();
    assert_eq!(sched.processes[0].state, ProcessState::BlockedOnIpc);
    // The close reply frees its slot with no caller to take the result.
    let bytes = reply_frame(identity, close_id, api::CLOSE, 0, 0, &[]);
    runtime
        .reply_file(identity.registration, &bytes, &mut sched)
        .unwrap();
    assert!(runtime.take_file(identity.caller, api::CLOSE).is_none());
    // Only the read publication now remains pullable.
    let read_id = runtime
        .next_file(identity.registration, &mut frame)
        .unwrap()
        .unwrap();
    assert_eq!(api::Request::decode(&frame).unwrap().op, api::READ);
    let bytes = reply_frame(identity, read_id, api::READ, 0, 2, b"hi");
    runtime
        .reply_file(identity.registration, &bytes, &mut sched)
        .unwrap();
    assert_eq!(sched.processes[0].state, ProcessState::Ready);
    let completion = runtime.take_file(identity.caller, api::READ).unwrap();
    assert_eq!(completion.payload.as_slice(), b"hi");
}

#[test]
fn close_reply_does_not_wake_a_caller_parked_on_another_op() {
    let (mut runtime, mut sched, identity) = setup();
    // Publish the close first so it carries the lower request id and is the
    // next frame the service pulls; then park the caller on a read.
    runtime
        .publish_file_close(identity.registration, identity.caller, 9, sched.global_tick)
        .unwrap();
    runtime
        .publish_file(
            identity,
            FileOperation::Read {
                token: 7,
                length: 8,
            },
            None,
            &mut sched,
        )
        .unwrap();
    assert_eq!(sched.processes[0].state, ProcessState::BlockedOnIpc);
    let mut frame = [0u8; api::FRAME_BYTES];
    let close_id = runtime
        .next_file(identity.registration, &mut frame)
        .unwrap()
        .unwrap();
    assert_eq!(api::Request::decode(&frame).unwrap().op, api::CLOSE);
    // A fire-and-forget close owns no continuation: replying it must not wake
    // the caller that is parked on the still-outstanding read.
    let bytes = reply_frame(identity, close_id, api::CLOSE, 0, 0, &[]);
    runtime
        .reply_file(identity.registration, &bytes, &mut sched)
        .unwrap();
    assert_eq!(sched.processes[0].state, ProcessState::BlockedOnIpc);
    // The read's own reply is the one that resumes it.
    let read_id = runtime
        .next_file(identity.registration, &mut frame)
        .unwrap()
        .unwrap();
    let bytes = reply_frame(identity, read_id, api::READ, 0, 3, b"abc");
    runtime
        .reply_file(identity.registration, &bytes, &mut sched)
        .unwrap();
    assert_eq!(sched.processes[0].state, ProcessState::Ready);
    assert_eq!(
        runtime
            .take_file(identity.caller, api::READ)
            .unwrap()
            .payload
            .as_slice(),
        b"abc"
    );
}

#[test]
fn close_timeout_frees_the_slot_without_a_caller_wake() {
    let (mut runtime, mut sched, identity) = setup();
    runtime
        .publish_file_close(identity.registration, identity.caller, 9, sched.global_tick)
        .unwrap();
    sched.global_tick = TIMEOUT_TICKS;
    runtime.expire_files(sched.global_tick);
    runtime.wake_file_callers(&mut sched);
    // No owner parked, so nothing to wake and nothing to take.
    assert_eq!(sched.processes[0].state, ProcessState::Running);
    assert!(runtime.take_file(identity.caller, api::CLOSE).is_none());
    let mut frame = [0u8; api::FRAME_BYTES];
    assert_eq!(
        runtime
            .next_file(identity.registration, &mut frame)
            .unwrap(),
        None
    );
    // A late reply finds no correlation and is refused, never retransmitted.
    let bytes = reply_frame(identity, 1, api::CLOSE, 0, 0, &[]);
    assert_eq!(
        runtime.reply_file(identity.registration, &bytes, &mut sched),
        Err(Error::Identity)
    );
}

#[test]
fn cancelled_mapping_read_rejects_late_reply_and_preserves_exact_close() {
    let (mut runtime, mut sched, identity) = setup();
    let caller = identity.caller;
    let epoch = identity.registration.epoch;
    let token = 7;
    runtime.publish_file_close(identity.registration, caller, token, 0).unwrap();
    runtime.publish_file(identity, FileOperation::Pread {
        token, offset: 4096, length: 16,
    }, None, &mut sched).unwrap();
    let read_slot = runtime.caller_publication(caller).unwrap();
    let request_id = runtime.file_publications[read_slot].as_ref().unwrap().id;

    runtime.cancel_mapping_read(caller, epoch + 1, token);
    runtime.cancel_mapping_read(caller, epoch, token + 1);
    assert_eq!(runtime.caller_publication(caller), Some(read_slot));

    runtime.retire_file_registration(identity.registration);
    // Epoch withdrawal independently discharges copied Close. The terminal
    // PREAD publication remains until the mapping replay rejects its epoch.
    assert_eq!(runtime.caller_publication(caller), Some(read_slot));
    runtime.cancel_mapping_read(caller, epoch, token);
    assert!(runtime.caller_publication(caller).is_none());
    let late = reply_frame(identity, request_id, api::PREAD, 0, 4, b"late");
    assert!(runtime.reply_file(identity.registration, &late, &mut sched).is_err());

    sched.processes[0].state = ProcessState::Running;
    runtime.publish_file_close(identity.registration, caller, token, 0).unwrap();
    runtime.publish_file(identity, FileOperation::Pread {
        token, offset: 0, length: 16,
    }, None, &mut sched).unwrap();
    runtime.cancel_mapping_read(caller, epoch, token);
    let mut frame = [0; api::FRAME_BYTES];
    assert!(runtime.next_file(identity.registration, &mut frame).unwrap().is_some());
    let close = api::Request::decode(&frame).unwrap();
    assert_eq!((close.op, close.token), (api::CLOSE, token));
}
