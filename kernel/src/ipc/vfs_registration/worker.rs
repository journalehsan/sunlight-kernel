//! Native shared threads and Linux clones use the private-child publication
//! transaction. The creator waits on the existing bounded retention request.
use super::*;
use crate::arch::x86_64::syscall::SyscallFrame;
use crate::capability::CAP_BROKER;
use crate::process::pending::Child;
use crate::sched::ThreadSlot;
use x86_64::VirtAddr;

#[derive(Clone, Copy)]
pub(crate) struct Launch {
    origin: [u64; 8],
    cpu: usize,
    parent_tid: Option<u64>,
    hhdm: VirtAddr,
}

impl Launch {
    fn origin(frame: &SyscallFrame) -> [u64; 8] {
        [
            frame.rax, frame.rcx, frame.rdi, frame.rsi, frame.rdx, frame.r10, frame.r8, frame.r9,
        ]
    }

    pub(crate) fn new(
        frame: &SyscallFrame,
        cpu: usize,
        parent_tid: Option<u64>,
        hhdm: VirtAddr,
    ) -> Self {
        Self {
            origin: Self::origin(frame),
            cpu,
            parent_tid,
            hhdm,
        }
    }
}

pub(crate) fn required(sched: &Scheduler) -> Result<bool, Error> {
    let idx = sched.current_process_index().ok_or(Error::Identity)?;
    if RUNTIME.lock().chdir_busy(idx, sched) {
        return Err(Error::Busy);
    }
    let owner = sched.current_shared_process();
    Ok(owner.vfs_downloads_cwd || owner.vfs_stat_routed || owner.vfs_file_routed)
}

pub(crate) fn stage(
    mut worker: Process,
    slot: ThreadSlot,
    launch: Launch,
    sched: &mut Scheduler,
) -> Result<(), Error> {
    let idx = sched.current_process_index().ok_or(Error::Identity)?;
    let source = sched.shared_process_index(idx);
    let creator = parent(process_identity(&sched.processes[idx]));
    worker.foreground = slot.foreground;
    // Diagnostic text only. Derivation pins the exact source receipt.
    worker.cwd = crate::process::thread_admission::try_string(&sched.processes[source].cwd)
        .map_err(|_| Error::Capacity)?;
    worker.vfs_file_routed = sched.processes[source].vfs_file_routed;
    worker.vfs_stat_routed = sched.processes[source].vfs_stat_routed;
    worker.vfs_root_rebased = sched.processes[source].vfs_root_rebased;
    let owner =
        Child::adopt_worker(creator, worker, sched, launch.hhdm).map_err(|_| Error::Capacity)?;
    let caps = CAP_BROKER.lock();
    let mut runtime = RUNTIME.lock();
    let claim = match runtime.stage_spawn_from(
        idx,
        source,
        owner.private_identity(),
        sched,
        &caps,
        false,
        false,
        true,
    ) {
        Ok(claim) => claim,
        Err(error) => {
            drop(runtime);
            owner.rollback_unstaged_worker(sched);
            return Err(error);
        }
    };
    let row = runtime
        .spawns
        .iter()
        .position(|row| row.as_ref().is_some_and(|call| call.claim == claim))
        .expect("staged worker");
    let call = runtime.spawns[row].as_mut().unwrap();
    call.owner = Some(owner);
    call.worker_launch = Some(launch);
    if runtime.wait_spawn_inner(idx, claim, sched, true) != Ok(None) {
        ordinary::abandon(&mut runtime, row, claim);
        return Err(Error::Identity);
    }
    ordinary::park(sched, idx);
    Ok(())
}

pub(crate) fn resume(
    frame: &SyscallFrame,
    sched: &mut Scheduler,
) -> Option<Result<Option<usize>, Error>> {
    let idx = sched.current_process_index()?;
    let creator = parent(process_identity(&sched.processes[idx]));
    let caps = CAP_BROKER.lock();
    let mut runtime = RUNTIME.lock();
    let row = runtime.spawns.iter().position(|row| {
        row.as_ref().is_some_and(|call| {
            !call.detached
                && call.launch.parent == creator
                && call
                    .worker_launch
                    .is_some_and(|launch| launch.origin == Launch::origin(frame))
        })
    })?;
    let call = runtime.spawns[row].as_ref().unwrap();
    let claim = call.claim;
    let launch = call.worker_launch.unwrap();
    match runtime.wait_spawn_inner(idx, claim, sched, true) {
        Ok(None) => {
            ordinary::park(sched, idx);
            return Some(Ok(None));
        }
        Ok(Some(Terminal::Retained(_))) => {}
        _ => {
            ordinary::abandon(&mut runtime, row, claim);
            return Some(Err(Error::Identity));
        }
    }

    let thread_slot = core::cell::Cell::new(None);
    let result = runtime.publish_spawn(
        idx,
        claim,
        sched,
        &caps,
        |sched| {
            let worker = sched
                .pending_children
                .private(claim.child)
                .ok_or(Error::Identity)?;
            if worker.account_membership != sched.processes[idx].account_membership
                || worker.account_membership.is_some_and(|reference| {
                    !caps.session_allocation_live(reference, sched.global_tick)
                })
            {
                return Err(Error::Identity);
            }
            if let Some(address) = launch.parent_tid {
                crate::memory::user::validate_process_write(
                    &sched.processes[idx],
                    launch.hhdm,
                    address,
                    4,
                )
                .map_err(|_| Error::UserMemory)?;
            }
            thread_slot.set(Some(
                sched.reserve_thread_slot().map_err(|_| Error::Capacity)?,
            ));
            Ok(())
        },
        |sched, identity, _| {
            sched
                .pending_children
                .eligible(identity, true)
                .expect("validated worker");
            let mut worker = sched
                .pending_children
                .transfer(identity, true)
                .expect("exclusive worker publication");
            sched.pending_children.published(identity.ticket);
            worker.vfs_root_metadata = true;
            if let Some(address) = launch.parent_tid {
                crate::memory::user::write_process_atomic_u32(
                    &sched.processes[idx],
                    launch.hhdm,
                    address,
                    worker.pid as u32,
                )
                .expect("validated parent TID store under scheduler guard");
            }
            let mut slot = thread_slot.take().expect("reserved worker slot");
            slot.foreground = worker.foreground;
            let tid = worker.pid;
            let native = worker.native_thread;
            sched.publish_thread(slot, *worker, launch.cpu);
            if native {
                crate::sched::note_native_borrower_created();
            }
            tid
        },
    );
    if result.is_err() {
        ordinary::abandon(&mut runtime, row, claim);
    }
    Some(result.map(Some))
}
