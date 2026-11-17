use super::*;
use crate::sched::{AddressSpace, Foreground, Process};
use sunlight_ipc::vfs_rpc::{context::LOOKUP_METADATA, lifecycle::Failure};

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
    runtime
        .namespaces
        .install(identity, 9, 10, LOOKUP_METADATA)
        .unwrap();
    let sched = Scheduler {
        global_tick: 0,
        processes: alloc::vec![Process {
            pid: 10,
            trusted_vfs_service: false,
            vfs_root_metadata: false,
            vfs_boot_root: false,
            vfs_file_routed: false,
            cwd: alloc::string::String::from("/"),
            exiting: false,
            uid: 1000,
            gid: 100,
            foreground: Foreground { incarnation: 11 },
            address_space: AddressSpace { generation: 12 },
            state: ProcessState::Running,
            pending_call: None,
            ipc_reply_target: None,
            ipc_endpoint: None,
        }],
    };
    (runtime, sched, identity)
}
fn reply(id: u64) -> ReplyMessage<'static> {
    ReplyMessage {
        label: sunlight_ipc::vfs_rpc::REPLY,
        words: [1, id, 2, 0],
        word_count: 4,
        cap_count: 0,
        output: &[],
    }
}

#[test]
fn publication_parks_before_delivery_and_completion_resumes_once() {
    let _guard = super::super::super::vfs_resources::TEST_LEDGER_LOCK.lock();
    let (mut runtime, mut sched, identity) = setup();
    let id = runtime
        .publish_stat(identity, "/probe", &mut sched)
        .unwrap()
        .words()[1];
    assert_eq!(sched.processes[0].state, ProcessState::BlockedOnIpc);
    assert_eq!(
        runtime.publish_stat(identity, "/", &mut sched),
        Err(Error::Busy)
    );
    assert_eq!(
        runtime.complete_stat(identity.registration, reply(id)),
        Err(Error::Identity)
    );
    assert_eq!(
        runtime.take_stat(identity.registration, &mut [0; 1]),
        Err(Error::Capacity)
    );
    let mut wrong_registration = identity.registration;
    wrong_registration.epoch += 1;
    assert_eq!(
        runtime.complete_stat(wrong_registration, reply(id)),
        Err(Error::Identity)
    );
    assert_eq!(runtime.pending.state(id), Some(State::Waiting));
    assert!(runtime.stat_ready(identity.registration));
    let mut input = [0; 8192];
    let (_, len) = runtime
        .take_stat(identity.registration, &mut input)
        .unwrap()
        .unwrap();
    let mut groups = [0; 32];
    let dispatch = sunlight_ipc::vfs_rpc::context::decode(&input[..len], &mut groups).unwrap();
    assert_eq!(dispatch.context.caller, identity.caller);
    assert_eq!(dispatch.request_id, id);
    assert!(!runtime.stat_ready(identity.registration));
    assert_eq!(
        runtime.take_stat(identity.registration, &mut input),
        Ok(None)
    );
    runtime
        .complete_stat(identity.registration, reply(id))
        .unwrap();
    runtime.wake_stat_callers(&mut sched);
    assert_eq!(sched.processes[0].state, ProcessState::Ready);
    assert_eq!(
        runtime.complete_stat(identity.registration, reply(id)),
        Err(Error::Identity)
    );
    sched.processes[0].state = ProcessState::BlockedOnIpc;
    runtime.wake_stat_callers(&mut sched);
    assert_eq!(sched.processes[0].state, ProcessState::BlockedOnIpc);
}

#[test]
fn malformed_bound_reply_and_service_loss_are_terminal_and_wake_callers() {
    let _guard = super::super::super::vfs_resources::TEST_LEDGER_LOCK.lock();
    for malformed in [false, true] {
        let (mut runtime, mut sched, identity) = setup();
        let id = runtime
            .publish_stat(identity, "/", &mut sched)
            .unwrap()
            .words()[1];
        runtime
            .take_stat(identity.registration, &mut [0; 8192])
            .unwrap();
        let expected = if malformed {
            let mut bad = reply(id);
            bad.cap_count = 1;
            assert_eq!(
                runtime.complete_stat(identity.registration, bad),
                Err(Error::Protocol)
            );
            Failure::MalformedReply
        } else {
            runtime.retire_registration(identity.registration);
            assert_eq!(
                runtime.complete_stat(identity.registration, reply(id)),
                Err(Error::Identity)
            );
            Failure::ServiceRetired
        };
        assert_eq!(runtime.pending.state(id), Some(State::Failed(expected)));
        runtime.wake_stat_callers(&mut sched);
        assert_eq!(sched.processes[0].state, ProcessState::Ready);
        assert!(runtime.pending.reap(id).is_some());
    }
}

#[test]
fn failed_preparation_does_not_park_and_retired_identity_cannot_wake_replacement() {
    let _guard = super::super::super::vfs_resources::TEST_LEDGER_LOCK.lock();
    let (mut runtime, mut sched, identity) = setup();
    assert!(runtime.publish_stat(identity, "", &mut sched).is_err());
    assert_eq!(sched.processes[0].state, ProcessState::Running);
    assert!(runtime.pending.is_empty());
    let id = runtime
        .publish_stat(identity, "/", &mut sched)
        .unwrap()
        .words()[1];
    runtime.retire_process(identity.caller);
    sched.processes[0].address_space.generation += 1;
    runtime.wake_stat_callers(&mut sched);
    assert_eq!(sched.processes[0].state, ProcessState::BlockedOnIpc);
    assert_eq!(
        runtime.pending.state(id),
        Some(State::Failed(Failure::CallerRetired))
    );
}

#[test]
fn successful_metadata_is_owned_until_exact_caller_takes_result() {
    use sunlight_ipc::vfs_rpc::{
        codec::Request,
        response::{Kind, Metadata, Provenance, Reply, ReplyContract},
    };
    let _guard = super::super::super::vfs_resources::TEST_LEDGER_LOCK.lock();
    let (mut runtime, mut sched, identity) = setup();
    let id = runtime
        .publish_stat(identity, "/", &mut sched)
        .unwrap()
        .words()[1];
    runtime
        .take_stat(identity.registration, &mut [0; 8192])
        .unwrap();
    let mut bytes = [0; 56];
    let metadata = Metadata {
        provenance: Provenance {
            mount_id: 1,
            mount_generation: 1,
            object_id: 1,
            object_generation: 1,
        },
        kind: Kind::File,
        mode: 0o640,
        uid: 1000,
        gid: 100,
        links: 1,
        size: 42,
    };
    ReplyContract::for_request(Request::Stat { path: "/" })
        .unwrap()
        .encode(Reply::Metadata(metadata), &mut bytes)
        .unwrap();
    runtime
        .complete_stat(
            identity.registration,
            ReplyMessage {
                label: sunlight_ipc::vfs_rpc::REPLY,
                words: [1, id, 0, 56],
                word_count: 4,
                cap_count: 0,
                output: &bytes,
            },
        )
        .unwrap();
    assert!(runtime.reap_stat(identity.caller, id).is_none());
    runtime.wake_stat_callers(&mut sched);
    let mut wrong = identity.caller;
    wrong.address_space_generation += 1;
    assert!(runtime.reap_stat(wrong, id).is_none());
    let mut retired = runtime.reap_stat(identity.caller, id).unwrap();
    assert_eq!(retired.resources.output(), bytes);
    assert!(runtime.reap_stat(identity.caller, id).is_none());
    // Taking the result frees the continuation slot for a later call.
    sched.processes[0].state = ProcessState::Running;
    assert!(runtime.publish_stat(identity, "/", &mut sched).is_ok());
}

#[test]
fn server_stat_preserves_inbound_reply_obligation_through_resume() {
    let _guard = super::super::super::vfs_resources::TEST_LEDGER_LOCK.lock();
    let (mut runtime, mut sched, identity) = setup();
    sched.processes[0].ipc_reply_target = Some(());
    let id = runtime
        .publish_stat(identity, "/bin/sunlight-files", &mut sched)
        .unwrap()
        .words()[1];
    assert_eq!(sched.processes[0].state, ProcessState::BlockedOnIpc);
    assert!(sched.processes[0].ipc_reply_target.is_some());
    runtime
        .take_stat(identity.registration, &mut [0; 8192])
        .unwrap()
        .unwrap();
    runtime
        .complete_stat(identity.registration, reply(id))
        .unwrap();
    runtime.wake_stat_callers(&mut sched);
    assert_eq!(sched.processes[0].state, ProcessState::Ready);
    assert!(runtime.reap_stat(identity.caller, id).is_some());
    assert!(sched.processes[0].ipc_reply_target.is_some());
}

#[test]
fn server_stat_withdrawal_wakes_without_consuming_client_reply() {
    let _guard = super::super::super::vfs_resources::TEST_LEDGER_LOCK.lock();
    let (mut runtime, mut sched, identity) = setup();
    sched.processes[0].vfs_root_metadata = true;
    sched.processes[0].ipc_reply_target = Some(());
    let id = runtime
        .publish_stat(identity, "/bin/sunlight-tasks", &mut sched)
        .unwrap()
        .words()[1];
    runtime.withdraw_root_profile(0, &mut sched);
    assert_eq!(sched.processes[0].state, ProcessState::Ready);
    assert!(sched.processes[0].ipc_reply_target.is_some());
    assert!(!sched.processes[0].vfs_root_metadata);
    assert!(runtime.publications.iter().all(Option::is_none));
    drop(runtime.pending.reap(id).unwrap());
}

#[test]
fn nested_ipc_continuation_keeps_its_block_until_its_own_completion() {
    let _guard = super::super::super::vfs_resources::TEST_LEDGER_LOCK.lock();
    let (mut runtime, mut sched, identity) = setup();
    let id = runtime
        .publish_stat(identity, "/", &mut sched)
        .unwrap()
        .words()[1];
    runtime
        .take_stat(identity.registration, &mut [0; 8192])
        .unwrap();
    // Model an interrupting signal handler's nested IPC operation.
    sched.processes[0].pending_call = Some(());
    runtime
        .complete_stat(identity.registration, reply(id))
        .unwrap();
    runtime.wake_stat_callers(&mut sched);
    assert_eq!(sched.processes[0].state, ProcessState::BlockedOnIpc);
    assert!(runtime.reap_stat(identity.caller, id).is_none());
    sched.processes[0].pending_call = None;
    sched.processes[0].state = ProcessState::Running;
    runtime.wake_stat_callers(&mut sched);
    assert_eq!(sched.processes[0].state, ProcessState::Running);
    assert!(runtime.reap_stat(identity.caller, id).is_some());
}

#[test]
fn pathname_snapshot_survives_caller_mutation_and_matches_only_its_own_retry() {
    let _guard = super::super::super::vfs_resources::TEST_LEDGER_LOCK.lock();
    let (mut runtime, mut sched, identity) = setup();
    let mut path = alloc::string::String::from("/original");
    let id = runtime
        .publish_stat(identity, &path, &mut sched)
        .unwrap()
        .words()[1];
    path.clear();
    path.push_str("/different");
    let backing = runtime.pending.resources(id).unwrap();
    assert!(backing.matches_stat_path("/original"));
    assert!(!backing.matches_stat_path(&path));
    assert!(!backing.matches_stat_path("/"));
}

#[test]
fn timeout_bounds_undelivered_and_delivered_work_without_releasing_owned_results() {
    let _guard = super::super::super::vfs_resources::TEST_LEDGER_LOCK.lock();
    for delivered in [false, true] {
        let (mut runtime, mut sched, identity) = setup();
        sched.global_tick = 17;
        let id = runtime
            .publish_stat(identity, "/probe", &mut sched)
            .unwrap()
            .words()[1];
        if delivered {
            runtime
                .take_stat(identity.registration, &mut [0; 8192])
                .unwrap()
                .unwrap();
        }
        runtime.expire_stats(17 + TIMEOUT_TICKS - 1);
        assert_eq!(runtime.pending.state(id), Some(State::Waiting));
        runtime.expire_stats(17 + TIMEOUT_TICKS);
        assert_eq!(
            runtime.pending.state(id),
            Some(State::Failed(Failure::TimedOut))
        );
        runtime.wake_stat_callers(&mut sched);
        assert_eq!(sched.processes[0].state, ProcessState::Ready);
        assert!(!runtime.stat_ready(identity.registration));
        assert_eq!(
            runtime.take_stat(identity.registration, &mut [0; 8192]),
            Ok(None)
        );
        assert_eq!(
            runtime.complete_stat(identity.registration, reply(id)),
            Err(Error::Identity)
        );
        assert!(runtime
            .pending
            .resources(id)
            .unwrap()
            .matches_stat_path("/probe"));
        let mut wrong = identity.caller;
        wrong.address_space_generation += 1;
        assert!(runtime.reap_stat(wrong, id).is_none());
        let mut completion = StatCompletion(runtime.reap_stat(identity.caller, id).unwrap());
        assert_eq!(
            completion.bytes(),
            Err(sunlight_ipc::vfs_rpc::stat_transport::TIMED_OUT)
        );
        drop(completion);
        assert!(runtime.pending.is_empty());
        sched.processes[0].state = ProcessState::Running;
        let next = runtime
            .publish_stat(identity, "/next", &mut sched)
            .unwrap()
            .words()[1];
        assert!(next > id);
        runtime
            .take_stat(identity.registration, &mut [0; 8192])
            .unwrap()
            .unwrap();
        assert_eq!(
            runtime.complete_stat(identity.registration, reply(id)),
            Err(Error::Identity)
        );
        assert_eq!(runtime.pending.state(next), Some(State::Waiting));
        runtime
            .complete_stat(identity.registration, reply(next))
            .unwrap();
    }
}

#[test]
fn timeout_wraps_the_clock_and_preserves_the_first_terminal_event() {
    let _guard = super::super::super::vfs_resources::TEST_LEDGER_LOCK.lock();
    for event in 0..4 {
        let (mut runtime, mut sched, identity) = setup();
        sched.global_tick = u64::MAX - TIMEOUT_TICKS / 2;
        let id = runtime
            .publish_stat(identity, "/", &mut sched)
            .unwrap()
            .words()[1];
        runtime
            .take_stat(identity.registration, &mut [0; 8192])
            .unwrap()
            .unwrap();
        let expected = match event {
            0 => {
                runtime
                    .complete_stat(identity.registration, reply(id))
                    .unwrap();
                runtime.pending.state(id).unwrap()
            }
            1 => {
                runtime.retire_registration(identity.registration);
                State::Failed(Failure::ServiceRetired)
            }
            2 => {
                runtime.retire_process(identity.caller);
                State::Failed(Failure::CallerRetired)
            }
            _ => {
                runtime.expire_stats(sched.global_tick.wrapping_add(TIMEOUT_TICKS - 1));
                assert_eq!(runtime.pending.state(id), Some(State::Waiting));
                State::Failed(Failure::TimedOut)
            }
        };
        runtime.expire_stats(sched.global_tick.wrapping_add(TIMEOUT_TICKS));
        runtime.expire_stats(sched.global_tick.wrapping_add(TIMEOUT_TICKS + 1));
        assert_eq!(runtime.pending.state(id), Some(expected));
    }
}

#[test]
fn caller_retirement_discards_continuations_across_delivery_and_vm_replacement() {
    let _guard = super::super::super::vfs_resources::TEST_LEDGER_LOCK.lock();
    for phase in 0..3 {
        for replacement in 0..3 {
            let (mut runtime, mut sched, identity) = setup();
            let id = runtime
                .publish_stat(identity, "/probe", &mut sched)
                .unwrap()
                .words()[1];
            if phase > 0 {
                runtime
                    .take_stat(identity.registration, &mut [0; 8192])
                    .unwrap()
                    .unwrap();
            }
            if phase == 2 {
                runtime
                    .complete_stat(identity.registration, reply(id))
                    .unwrap();
            }
            // Exit/exec teardown carries the old identity, before a replacement
            // process or VM can run. Completed resources still require reap.
            runtime.retire_process(identity.caller);
            match replacement {
                0 => sched.processes[0].exiting = true,
                1 => sched.processes[0].address_space.generation += 1,
                _ => sched.processes[0].foreground.incarnation += 1,
            }
            runtime.expire_stats(TIMEOUT_TICKS);
            runtime.wake_stat_callers(&mut sched);
            assert_eq!(sched.processes[0].state, ProcessState::BlockedOnIpc);
            assert!(runtime.publications.iter().all(Option::is_none));
            assert!(runtime.reap_stat(identity.caller, id).is_none());
            assert_eq!(
                runtime.complete_stat(identity.registration, reply(id)),
                Err(Error::Identity)
            );
            let retired = runtime.pending.reap(id).unwrap();
            if phase < 2 {
                assert_eq!(retired.state, State::Failed(Failure::CallerRetired));
            } else {
                assert!(matches!(retired.state, State::Completed(_)));
            }
            drop(retired);
            assert!(runtime.pending.is_empty());
        }
    }
}

#[test]
fn profile_withdrawal_discards_each_publication_phase_and_wakes_only_its_continuation() {
    let _guard = super::super::super::vfs_resources::TEST_LEDGER_LOCK.lock();
    for credential in 0..3 {
        for phase in 0..3 {
            for nested in [false, true] {
                let (mut runtime, mut sched, identity) = setup();
                sched.processes[0].vfs_root_metadata = true;
                sched.processes[0].vfs_boot_root = true;
                let id = runtime
                    .publish_stat(identity, "/probe", &mut sched)
                    .unwrap()
                    .words()[1];
                if phase > 0 {
                    runtime
                        .take_stat(identity.registration, &mut [0; 8192])
                        .unwrap()
                        .unwrap();
                }
                if phase == 2 {
                    runtime
                        .complete_stat(identity.registration, reply(id))
                        .unwrap();
                }
                if nested {
                    sched.processes[0].pending_call = Some(());
                }
                // Model the credential commit under the same scheduler guard.
                match credential {
                    1 => sched.processes[0].uid += 1,
                    2 => sched.processes[0].gid += 1,
                    _ => {}
                }
                runtime.withdraw_root_profile(0, &mut sched);
                assert!(!sched.processes[0].vfs_root_metadata);
                assert_eq!(process_identity(&sched.processes[0]), identity.caller);
                assert_eq!(
                    sched.processes[0].state,
                    if nested {
                        ProcessState::BlockedOnIpc
                    } else {
                        ProcessState::Ready
                    }
                );
                assert_eq!(runtime.registry.active(), Some(identity.registration));
                assert!(runtime.namespaces.lookup(identity).is_err());
                assert!(runtime.publications.iter().all(Option::is_none));
                assert!(runtime.reap_stat(identity.caller, id).is_none());
                assert_eq!(
                    runtime.complete_stat(identity.registration, reply(id)),
                    Err(Error::Identity)
                );
                assert!(runtime.admit_stat(identity, "/probe").is_err());
                // The scheduler-return worker owns destruction after unlock.
                drop(runtime.pending.reap(id).unwrap());
                runtime.reconcile_namespaces();
                assert!(runtime.pending.is_empty());
                // Repeated successful chdir does not recreate a continuation.
                runtime.withdraw_root_profile(0, &mut sched);
                assert!(runtime.publications.iter().all(Option::is_none));
            }
        }
    }
}

#[test]
fn native_stat_record_preserves_layout_types_and_rejects_invalid_metadata() {
    use sunlight_ipc::vfs_rpc::{
        codec::Request,
        response::{Kind, Metadata, Provenance, Reply, ReplyContract},
    };
    for kind in [Kind::File, Kind::Directory] {
        let metadata = Metadata {
            provenance: Provenance {
                mount_id: 1,
                mount_generation: 2,
                object_id: 3,
                object_generation: 4,
            },
            kind,
            mode: 0o6751,
            uid: 123,
            gid: 456,
            links: 7,
            size: u64::MAX - 4,
        };
        let mut bytes = [0; 56];
        ReplyContract::for_request(Request::Stat { path: "/" })
            .unwrap()
            .encode(Reply::Metadata(metadata), &mut bytes)
            .unwrap();
        let record = native_record(&bytes).unwrap();
        assert_eq!(
            u64::from_le_bytes(record[..8].try_into().unwrap()),
            metadata.size
        );
        assert_eq!(u32::from_le_bytes(record[8..12].try_into().unwrap()), 123);
        assert_eq!(u32::from_le_bytes(record[12..16].try_into().unwrap()), 456);
        assert_eq!(
            u16::from_le_bytes(record[16..18].try_into().unwrap()),
            metadata.mode
                | if kind == Kind::File {
                    0o100000
                } else {
                    0o040000
                }
        );
        assert_eq!(&record[18..20], &[kind as u8, 0]);
        assert_eq!(u32::from_le_bytes(record[20..24].try_into().unwrap()), 7);
        assert!(native_record(&bytes[..55]).is_err());
        bytes[32..34].copy_from_slice(&3u16.to_le_bytes());
        assert!(native_record(&bytes).is_err());
    }
}

#[test]
fn native_stat_errors_translate_posix_without_exposing_pilot_sentinels() {
    for (errno, offset) in [
        (1, 3),
        (2, 2),
        (5, 8),
        (9, 4),
        (11, 1),
        (12, 12),
        (13, 3),
        (20, 7),
        (21, 6),
        (22, 5),
        (28, 11),
        (36, 5),
        (110, 8),
        (125, 8),
    ] {
        assert_eq!(native_error(0u64.wrapping_sub(errno)), u64::MAX - offset);
    }
}

#[test]
fn linux_stat_record_uses_object_identity_and_checks_signed_size() {
    use sunlight_ipc::vfs_rpc::{
        codec::Request,
        response::{Kind, Metadata, Provenance, Reply, ReplyContract},
    };
    for kind in [Kind::File, Kind::Directory] {
        let mut metadata = Metadata {
            provenance: Provenance {
                mount_id: 7,
                mount_generation: 2,
                object_id: 91,
                object_generation: 4,
            },
            kind,
            mode: 0o6751,
            uid: 123,
            gid: 456,
            links: 7,
            size: 513,
        };
        let encode = |metadata| {
            let mut bytes = [0; 56];
            ReplyContract::for_request(Request::Stat { path: "/" })
                .unwrap()
                .encode(Reply::Metadata(metadata), &mut bytes)
                .unwrap();
            bytes
        };
        let bytes = encode(metadata);
        let record = linux_record(&bytes).unwrap();
        for (offset, expected) in [(0, 7), (8, 91), (16, 7), (48, 513), (56, 4096), (64, 2)] {
            assert_eq!(
                u64::from_le_bytes(record[offset..offset + 8].try_into().unwrap()),
                expected
            );
        }
        let mode = u32::from_le_bytes(record[24..28].try_into().unwrap());
        assert_eq!(
            mode,
            0o6751
                | if kind == Kind::File {
                    0o100000
                } else {
                    0o040000
                }
        );
        assert_eq!(&record[28..32], &123u32.to_le_bytes());
        assert_eq!(&record[32..36], &456u32.to_le_bytes());
        assert!(record[36..48]
            .iter()
            .chain(record[72..].iter())
            .all(|&byte| byte == 0));
        assert_eq!(linux_record(&bytes[..55]), Err((-5i64) as u64));
        metadata.size = i64::MAX as u64;
        assert!(linux_record(&encode(metadata)).is_ok());
        metadata.size += 1;
        assert_eq!(linux_record(&encode(metadata)), Err((-75i64) as u64));
    }
}
