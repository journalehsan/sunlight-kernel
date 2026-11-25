//! Exit a disposable ordinary process with a private child still in flight.
use super::*;

pub(super) fn stage(command: u64) -> u64 {
    if command == 18 {
        return u64::from(PRIVATE.load(Relaxed) == 0);
    }
    let mode = if command == 17 { 15 } else { command };
    let (idx, exact, uid, gid, baseline) = {
        let mut sched = SCHEDULER.lock();
        let idx = sched.current_process_index().expect("lifecycle parent");
        let process = &sched.processes[idx];
        if process.ppid != 1 || process.name_str() != "vfs-lifecycle-parent" {
            return u64::MAX;
        }
        let exact = process_identity(process);
        let runtime = RUNTIME.lock();
        let pending_request = *REQUEST.lock();
        if let Some((registration, request)) = pending_request {
            let call = runtime
                .spawn_request(registration, request)
                .expect("unfinished lifecycle continuation");
            let claim = call.claim;
            assert_eq!(call.parent.caller, exact);
            assert_eq!(MODE.load(Relaxed), mode as usize);
            if let Some(Terminal::Retained(receipt)) = call.result {
                if mode == 15 && command != 17 {
                    return 1;
                }
                if matches!(mode, 14 | 15) {
                    let launch = call.launch;
                    assert!(!call.detached && call.owner.is_some());
                    assert_eq!(LaunchAttribution::capture(process), Some(launch));
                    drop(runtime);
                    let pid = publish(idx, claim, &mut sched, &CAP_BROKER.lock())
                        .expect("publish owned lifecycle child");
                    let process = &sched.processes[sched.process_index_by_pid(pid).unwrap()];
                    assert_eq!(process_identity(process), caller(claim.child));
                    assert_eq!(process.ppid, launch.parent.pid);
                    assert_eq!(process.foreground.parent_generation, launch.parent.incarnation);
                    assert_eq!(process.foreground.member, launch.execution);
                    assert!(sched.pending_children.parent(claim.child.ticket).is_err());
                    assert!(RUNTIME.lock().spawn_request(registration, request).is_none());
                    *REQUEST.lock() = None;
                    PRIVATE.store(0, Relaxed);
                    accounting::dispose(claim);
                    if mode == 15 {
                        crate::serial_println!("[VFS-EXEC] rejected exec preserved exact launch and receipt");
                    }
                    crate::serial_println!(
                        "[VFS-LAUNCH] published parent={:?} child={:?} execution={} inline_bytes={}",
                        launch.parent, claim.child, launch.execution,
                        core::mem::size_of::<LaunchAttribution>(),
                    );
                    return pid as u64 + 2;
                }
                accounting::before_consumed_cancel(&runtime, claim, receipt);
                announce_exit(mode, exact, claim);
                return 1;
            }
            assert!(call.result.is_none());
            return 0;
        }
        let Some(registration) = runtime.registry.active() else {
            return 0;
        };
        let parent = AdmissionIdentity {
            caller: exact,
            uid: process.uid,
            gid: process.gid,
            groups: [],
            registration,
        };
        if runtime.capture_inheritance(parent).is_none() {
            return 0;
        }
        assert!(process.vfs_root_metadata);
        drop(runtime);
        let descriptors = (0..256)
            .filter(|&fd| process.fd_table.get(fd).is_some())
            .count();
        crate::serial_println!(
            "[VFS-PARENT] live parent={} incarnation={} vm={} descriptors={} frames={}",
            exact.pid,
            exact.incarnation,
            exact.address_space_generation,
            descriptors,
            crate::PMM.lock().owned_frame_count(exact.pid as u32),
        );
        (
            idx,
            exact,
            process.uid,
            process.gid,
            accounting::AllocatorSample::read_baseline(&sched, mode),
        )
    };

    let child = pending::construct(
        parent(exact),
        Image {
            name: "vfs-l1c-private",
            bytes: crate::SUNLIGHT_UTILS_ELF_BYTES,
            argv: &[b"stat", b"/tmp"],
            envp: &[],
            uid,
            gid,
            stdio: [None; 3],
        },
    )
    .expect("construct lifecycle private child");
    let identity = child.identity().expect("live lifecycle parent");
    let mut sched = SCHEDULER.lock();
    let constructed = accounting::AllocatorSample::read(&sched);
    let claim = super::super::spawn::stage_owned(idx, &mut Some(child), &mut sched, &CAP_BROKER.lock())
        .expect("stage lifecycle retain");
    accounting::stage(&RUNTIME.lock(), &sched, claim, mode, baseline, constructed);
    MODE.store(mode as usize, Relaxed);
    WAITER.store(exact.pid as usize, Relaxed);
    PRIVATE.store(identity.ticket.pid, Relaxed);
    OBSERVER_EXITED.store(0, Relaxed);
    *REQUEST.lock() = Some((claim.registration, claim.request_id));

    if mode == 12 {
        accounting::before_unsent_cancel(&RUNTIME.lock(), claim);
        announce_exit(mode, exact, claim);
        1
    } else {
        0
    }
}

fn announce_exit(mode: u64, parent: Caller, claim: RemoteClaim) {
    crate::serial_println!(
        "[VFS-PARENT] exit ready mode={} parent={} child={} retain={} epoch={}",
        mode,
        parent.pid,
        claim.child.ticket.pid,
        claim.request_id,
        claim.registration.epoch,
    );
}

pub(in crate::ipc::vfs_registration::spawn) fn cancel_detached(runtime: &mut Runtime, sched: &Scheduler) {
    if !matches!(MODE.load(Relaxed), 12 | 13 | 16) {
        return;
    }
    let Some((registration, request)) = *REQUEST.lock() else { return };
    let Some(call) = runtime.spawn_request(registration, request) else { return };
    if !call.detached || !matches!(call.result, None | Some(Terminal::Retained(_))) {
        return;
    }
    assert!(runtime.spawn_live(call, sched));
    assert!(!call.parked && call.owner.is_some());
    let claim = call.claim;
    if MODE.load(Relaxed) == 16 {
        let replacement = &sched.processes[sched.process_index_by_pid(call.launch.parent.pid).unwrap()];
        let current = process_identity(replacement);
        assert_eq!(current.incarnation, call.launch.parent.incarnation);
        assert_ne!(current.address_space_generation, call.launch.parent.image_generation);
        assert!(matches!(call.result, Some(Terminal::Retained(_))));
        assert_ne!(LaunchAttribution::capture(replacement), Some(call.launch));
        crate::serial_println!(
            "[VFS-EXEC] detached retained child={} parent={} incarnation={} old_vm={} new_vm={} source_pinned=1 inline_bytes={}",
            claim.child.ticket.pid, current.pid, current.incarnation, call.launch.parent.image_generation,
            current.address_space_generation, core::mem::size_of::<LaunchAttribution>(),
        );
    }
    crate::serial_println!(
        "[VFS-PARENT] detached parent={} child={} retain={} epoch={} source_pinned=1 launch={:?}",
        call.parent.caller.pid, claim.child.ticket.pid, claim.request_id, claim.registration.epoch,
        call.launch.parent,
    );
    runtime.finish_spawn(claim, Terminal::Cancelled);
}

pub(in crate::ipc::vfs_registration::spawn) fn disposed(claim: RemoteClaim) {
    if !matches!(MODE.load(Relaxed), 12 | 13 | 16) {
        return;
    }
    *REQUEST.lock() = None;
    PRIVATE.store(0, Relaxed);
    crate::serial_println!(
        "[VFS-PARENT] production disposed parent={} child={} retain={} epoch={}",
        WAITER.load(Relaxed),
        claim.child.ticket.pid,
        claim.request_id,
        claim.registration.epoch,
    );
    accounting::dispose(claim);
}
