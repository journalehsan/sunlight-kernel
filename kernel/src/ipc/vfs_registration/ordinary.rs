//! Ordinary adapters share the retained-child transaction and its single owner.
use super::*;
use crate::capability::sessions::LaunchAttachment;
use crate::memory::pmm::PhysicalMemoryManager;
use crate::process::{fd_table::FileDescriptor, pending::Child, spawn::SpawnError};
use x86_64::VirtAddr;

pub(crate) enum Launch {
    Native {
        cpu: usize,
        membership: Option<sunlight_ipc::account_session::SessionRef>,
        streams: [Option<(i32, FileDescriptor)>; 3],
        trace: crate::launch_trace::LaunchTrace,
        path: alloc::string::String,
        #[cfg(feature = "vfs_c2i1_candidate")]
        bootstrap_fstab: bool,
    },
    Capability {
        attachment: Option<LaunchAttachment>,
        tracked: bool,
        vortex: bool,
    },
}

pub(crate) enum Progress {
    Parked,
    Published(usize, alloc::boxed::Box<Launch>),
}

pub(crate) fn stage(
    mut child: Process,
    launch: Launch,
    sched: &mut Scheduler,
    pmm: &mut PhysicalMemoryManager,
    hhdm: VirtAddr,
    caps: &CapabilityBroker,
) -> Result<Progress, SpawnError> {
    let idx = sched.current_process_index().expect("launching task");
    let mut storage = match crate::process::thread_admission::try_box_uninit() {
        Ok(storage) => storage,
        Err(_) => {
            unsafe { crate::process::spawn::rollback_initial_image(&mut child, pmm, hhdm) };
            return Err(SpawnError::NoMemory);
        }
    };
    storage.write(launch);
    let launch = unsafe { storage.assume_init() };
    let exact = process_identity(&sched.processes[idx]);
    let owner = Child::adopt(parent(exact), child, sched, pmm, hhdm)
        .map_err(|_| SpawnError::NamespaceUnavailable)?;
    let mut runtime = RUNTIME.lock();
    let claim =
        match runtime.stage_spawn_inner(idx, owner.private_identity(), sched, caps, true, true) {
            Ok(claim) => claim,
            Err(_) => {
                drop(runtime);
                owner.rollback_unstaged(sched, pmm);
                return Err(SpawnError::NamespaceUnavailable);
            }
        };
    let call = runtime
        .spawns
        .iter_mut()
        .flatten()
        .find(|call| call.claim == claim)
        .expect("staged launch");
    call.owner = Some(owner);
    call.ordinary = Some(launch);
    if runtime.wait_spawn_inner(idx, claim, sched, true) != Ok(None) {
        let row = runtime
            .spawns
            .iter()
            .position(|row| row.as_ref().is_some_and(|call| call.claim == claim))
            .expect("owned staged launch");
        abandon(&mut runtime, row, claim);
        return Err(SpawnError::NamespaceUnavailable);
    }
    park(sched, idx);
    Ok(Progress::Parked)
}

pub(super) fn park(sched: &mut Scheduler, idx: usize) {
    sched.account_and_apply_churn_penalty();
    sched.processes[idx].block_start_tick = sched.global_tick;
    sched.remove_from_ready_queues(idx);
}

pub(crate) fn resume(
    native: bool,
    sched: &mut Scheduler,
    caps: &mut CapabilityBroker,
) -> Option<Result<Progress, SpawnError>> {
    let idx = sched.current_process_index()?;
    let exact = process_identity(&sched.processes[idx]);
    let mut runtime = RUNTIME.lock();
    let row = runtime.spawns.iter().position(|row| {
        row.as_ref().is_some_and(|call| {
            !call.detached
                && call.parent.caller == exact
                && call
                    .ordinary
                    .as_ref()
                    .is_some_and(|launch| matches!(**launch, Launch::Native { .. }) == native)
        })
    })?;
    let claim = runtime.spawns[row].as_ref().unwrap().claim;
    match runtime.wait_spawn_inner(idx, claim, sched, true) {
        Ok(None) => {
            park(sched, idx);
            return Some(Ok(Progress::Parked));
        }
        Ok(Some(Terminal::Retained(_))) => {}
        _ => {
            abandon(&mut runtime, row, claim);
            return Some(Err(SpawnError::NamespaceUnavailable));
        }
    }

    let launch = runtime.spawns[row]
        .as_mut()
        .unwrap()
        .ordinary
        .take()
        .unwrap();
    let tick = sched.global_tick;
    let slot = runtime.publish_spawn(
        idx, claim, sched, caps,
        |sched| {
            let parent = &sched.processes[idx];
            #[cfg(feature = "vfs_c2i1_candidate")]
            if !crate::persistent_boot_mount::ready()
                && !(matches!(&*launch, Launch::Native { bootstrap_fstab: true, .. })
                    && sched.supervision.is_bootstrap(Scheduler::supervision_identity(parent)))
            {
                return Err(Error::Preparation);
            }
            let valid = match &*launch {
                Launch::Native { membership, streams, .. } => {
                    parent.account_membership == *membership
                        && membership.is_none_or(|reference| caps.session_allocation_live(reference, tick))
                        && streams.iter().flatten().all(|(fd, entry)| {
                            parent.fd_table.get(*fd) == Some(entry)
                        })
                }
                Launch::Capability { attachment, tracked, .. } => {
                    let owner = Scheduler::supervision_identity(parent);
                    let watcher = (parent.pid, parent.address_space.identity().generation);
                    let allowed = !parent.trusted_service_manager
                        || (sched.active_manager(parent) && sched.supervision.can_spawn(owner));
                    allowed
                        && (!tracked || sched.processes.iter()
                            .filter(|p| p.service_reap_watcher == Some(watcher)).count() < 32)
                        && attachment.as_ref()
                            .is_none_or(|a| caps.validate_session_launch(a, tick).is_ok())
                }
            };
            #[cfg(all(debug_assertions, feature = "vfs_c1a_test"))]
            if parent.pid == 1 && REJECT_PUBLICATION.swap(false, core::sync::atomic::Ordering::Relaxed) {
                LAST_CHILD.store(claim.child.ticket.pid, core::sync::atomic::Ordering::Relaxed);
                crate::serial_println!(
                    "[VFS-C1A] publication refused adapter={} pid={} incarnation={} image={} epoch={} retain={}",
                    if native { "native" } else { "capability" }, claim.child.ticket.pid,
                    claim.child.incarnation, claim.child.image_generation,
                    claim.registration.epoch, claim.request_id,
                );
                return Err(Error::Preparation);
            }
            if !valid {
                return Err(Error::Preparation);
            }
            sched.reserve_private_publication().map_err(|_| Error::Preparation)
        },
        |sched, identity, attribution| {
            let mut process = take_bound_child(sched, identity, attribution);
            if native {
                process.child_wait = crate::process::ChildWait::Pending;
            }
            #[cfg(all(debug_assertions, feature = "vfs_c1a_test"))]
            if process.name_str() == "vfs-c1a-probe" {
                LAST_CHILD.store(identity.ticket.pid, core::sync::atomic::Ordering::Relaxed);
                crate::serial_println!(
                    "[VFS-C1A] bound adapter={} pid={} incarnation={} image={} epoch={} retain={}",
                    if native { "native" } else { "capability" }, identity.ticket.pid,
                    identity.incarnation, identity.image_generation, claim.registration.epoch,
                    claim.request_id,
                );
            }
            sched.insert_attributed_process(*process)
        },
    );
    match slot {
        Ok(slot) => {
            let mut launch = launch;
            if let Launch::Capability {
                attachment, vortex, ..
            } = &mut *launch
            {
                if let Some(attachment) = attachment.take() {
                    let reference = caps
                        .commit_session_launch(attachment, tick)
                        .expect("session validated under the same guards");
                    let child = &mut sched.processes[slot];
                    child.account_membership = Some(reference);
                    child.restrict_account_member_roles();
                    if *vortex {
                        child.service_identity = sunlight_ipc::service_identity::VORTEX_SHELL;
                    }
                }
            }
            Some(Ok(Progress::Published(slot, launch)))
        }
        Err(_) => {
            runtime.spawns[row].as_mut().unwrap().ordinary = Some(launch);
            abandon(&mut runtime, row, claim);
            Some(Err(SpawnError::NamespaceUnavailable))
        }
    }
}

pub(super) fn abandon(runtime: &mut Runtime, row: usize, claim: RemoteClaim) {
    let call = runtime.spawns[row].as_mut().unwrap();
    call.detached = true;
    if matches!(call.result, None | Some(Terminal::Retained(_))) {
        runtime.finish_spawn(claim, Terminal::Cancelled);
    }
}

#[cfg(all(debug_assertions, feature = "vfs_c1a_test"))]
static REJECT_PUBLICATION: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

#[cfg(all(debug_assertions, feature = "vfs_c1a_test"))]
static LAST_CHILD: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

#[cfg(all(debug_assertions, feature = "vfs_c1a_test"))]
pub(crate) fn checkpoint(mode: u64) -> u64 {
    use core::sync::atomic::{AtomicUsize, Ordering::Relaxed};
    static DESCRIPTIONS: AtomicUsize = AtomicUsize::new(0);
    static REFERENCES: AtomicUsize = AtomicUsize::new(0);
    let sched = crate::sched::SCHEDULER.lock();
    let process = sched.current_process();
    let owned_frames = if mode == 2 && process.pid == 1 {
        crate::PMM
            .lock()
            .owned_frame_count(LAST_CHILD.load(Relaxed) as u32)
    } else {
        0
    };
    let caps = crate::capability::CAP_BROKER.lock();
    let mut runtime = RUNTIME.lock();
    if mode == 3 && process.name_str() == "vfs-c1a-probe" && process.cwd == "/etc" {
        let Some(registration) = live_registration(&mut runtime, &sched, &caps) else {
            return u64::MAX;
        };
        let identity = AdmissionIdentity {
            caller: process_identity(process),
            uid: process.uid,
            gid: process.gid,
            groups: [],
            registration,
        };
        let Some(receipt) = runtime
            .capture_inheritance(identity)
            .and_then(|source| runtime.source_receipt(&source))
        else {
            return u64::MAX;
        };
        crate::serial_println!(
            "[VFS-C1A] user pid={} incarnation={} image={} epoch={} retain={} namespace={} generation={} pair={} cwd=/etc",
            identity.caller.pid, identity.caller.incarnation, identity.caller.address_space_generation,
            registration.epoch, receipt.request_id(), receipt.scope().namespace_id,
            receipt.scope().namespace_generation, receipt.generation(),
        );
        return 1;
    }
    if process.pid != 1 {
        return u64::MAX;
    }
    let outstanding: usize = runtime
        .remote
        .iter()
        .flatten()
        .map(|r| r.ownership.outstanding())
        .sum();
    let pending = sched.pending_children.pids().count();
    let (descriptions, refs) = crate::process::fd_table::FdTable::description_counts();
    match mode {
        // A preceding chdir can still own its old lease on another CPU.
        // Start the census only after that predecessor release settles.
        0 if pending != 0 || outstanding != 1 => 0,
        0 => {
            DESCRIPTIONS.store(descriptions, Relaxed);
            REFERENCES.store(refs, Relaxed);
            crate::serial_println!(
                "[VFS-C1A] baseline leases={} descriptions={} refs={} pending=0 owned_frames=0 service_rows=0 service_refs=0",
                outstanding,
                descriptions,
                refs,
            );
            1
        }
        1 => u64::from(!REJECT_PUBLICATION.swap(true, Relaxed)),
        2 => {
            if pending != 0
                || outstanding != 1
                || owned_frames != 0
                || crate::process::fd_table::FdTable::service_description_counts() != (0, 0)
                || descriptions != DESCRIPTIONS.load(Relaxed)
                || refs != REFERENCES.load(Relaxed)
            {
                return 0;
            }
            crate::serial_println!(
                "[VFS-C1A] settled leases={} descriptions={} refs={} pending=0 owned_frames=0 service_rows=0 service_refs=0",
                outstanding,
                descriptions,
                refs,
            );
            1
        }
        _ => u64::MAX,
    }
}

#[cfg(all(debug_assertions, feature = "vfs_c1a_test"))]
pub(crate) fn observe_dispatch(process: &Process, cpu: usize) {
    use core::sync::atomic::{AtomicU64, Ordering::Relaxed};
    static LAST_DISPATCHED: AtomicU64 = AtomicU64::new(0);
    if process.name_str() != "vfs-c1a-probe" {
        return;
    }
    let identity = process_identity(process);
    if LAST_DISPATCHED.swap(identity.incarnation, Relaxed) != identity.incarnation {
        assert!(process.vfs_root_metadata);
        crate::serial_println!(
            "[VFS-C1A] first dispatch pid={} incarnation={} image={} cpu={}",
            identity.pid,
            identity.incarnation,
            identity.address_space_generation,
            cpu,
        );
    }
}
