//! Native census for publication adapter transactions. Scheduler and runtime guards
//! serialize ownership observations. Global allocator samples are diagnostic.
use super::*;

pub(super) struct AllocatorSample {
    frames_free: usize,
    heap_used: usize,
    process_slots: usize,
    process_capacity: usize,
}

impl AllocatorSample {
    pub(super) fn read_baseline(sched: &Scheduler, mode: u64) -> Self {
        log_heap(sched, "r5-baseline", mode as usize, 0);
        Self::read(sched)
    }

    pub(super) fn read(sched: &Scheduler) -> Self {
        Self {
            frames_free: crate::PMM.lock().free_page_count(),
            heap_used: crate::memory::heap::heap_stats().allocated,
            process_slots: sched.processes.len(),
            process_capacity: sched.processes.capacity(),
        }
    }

    fn log(&self, phase: &str, claim: RemoteClaim) {
        crate::serial_println!(
            "[VFS-ACCT] {} child={} retain={} frames_free={} heap_used={} heap_capacity={} process_slots={} process_capacity={}",
            phase, claim.child.ticket.pid, claim.request_id, self.frames_free,
            self.heap_used, crate::memory::heap::HEAP_SIZE, self.process_slots,
            self.process_capacity,
        );
    }
}

struct Audit {
    claim: RemoteClaim,
    identity: AdmissionIdentity,
    mode: u64,
    disposed: bool,
    #[cfg(feature = "vfs_l1c_parent_lifecycle_test")]
    parent: AdmissionIdentity,
}

static AUDITS: Mutex<[Option<Audit>; 2]> = Mutex::new([None, None]);

pub(super) fn stage(
    runtime: &Runtime,
    sched: &Scheduler,
    claim: RemoteClaim,
    mode: u64,
    baseline: AllocatorSample,
    constructed: AllocatorSample,
) {
    assert!(matches!(mode, 0 | 4)
        || (cfg!(feature = "vfs_l1c_adapter_accounting_test") && matches!(mode, 1..=3 | 5..=8 | 11))
        || (cfg!(feature = "vfs_l1c_parent_lifecycle_test") && matches!(mode, 12..=16)));
    let call = runtime
        .spawn_request(claim.registration, claim.request_id)
        .unwrap();
    let private = sched
        .pending_children
        .private(claim.child)
        .expect("private process box");
    let hhdm = x86_64::VirtAddr::new(crate::HHDM_REQ.response().unwrap().offset);
    // The scheduler guard keeps this unpublished address space quiescent.
    let mapped_user_pages = unsafe { private.address_space.count_user_pages(hhdm) };
    assert!(mapped_user_pages > 0 && private.address_space.region_count() > 0);
    assert_eq!(sched.pending_children.remote(claim.child), Some(claim));
    assert!(sched
        .pending_children
        .pids()
        .any(|pid| pid == claim.child.ticket.pid));
    let remote = runtime
        .remote
        .iter()
        .flatten()
        .find(|remote| remote.registration == claim.registration)
        .expect("staged registration");
    assert!(remote.ownership.contains(claim.request_id));
    assert!(remote
        .local
        .iter()
        .flatten()
        .any(|lease| lease.id() == claim.request_id));
    assert!(remote
        .callers
        .iter()
        .flatten()
        .any(|entry| entry.caller == call.child.caller));
    let descriptors = (0..256)
        .filter(|&fd| private.fd_table.get(fd).is_some())
        .count();
    assert_eq!(descriptors, 3);
    let audit = Audit {
        claim,
        identity: call.child,
        mode,
        disposed: false,
        #[cfg(feature = "vfs_l1c_parent_lifecycle_test")]
        parent: call.parent,
    };
    assert!(AUDITS.lock()[usize::from(mode != 0)]
        .replace(audit)
        .is_none());
    crate::serial_println!(
        "[VFS-ADAPTER] mode={} child={} retain={} epoch={}",
        mode, claim.child.ticket.pid, claim.request_id, claim.registration.epoch
    );
    baseline.log("baseline", claim);
    constructed.log("constructed", claim);
    crate::serial_println!(
        "[VFS-ACCT] staged child={} retain={} epoch={} private_pid=1 process_box=1 fd_table_box=1 descriptors={} mapped_user_pages={} adapter=1 continuation=1 vfs_claim=1",
        claim.child.ticket.pid, claim.request_id, claim.registration.epoch, descriptors, mapped_user_pages,
    );
}

pub(super) fn dispose(claim: RemoteClaim) {
    let mut audits = AUDITS.lock();
    let audit = audits
        .iter_mut()
        .flatten()
        .find(|audit| audit.claim == claim)
        .unwrap();
    assert!(!audit.disposed);
    audit.disposed = true;
}

pub(in crate::ipc::vfs_registration::spawn) fn poll(runtime: &Runtime, sched: &Scheduler) {
    for slot in AUDITS.lock().iter_mut() {
        let Some(audit) = slot.as_ref().filter(|audit| audit.disposed) else {
            continue;
        };
        let claim = audit.claim;
        let pid = claim.child.ticket.pid;
        let child = audit.identity.caller;
        let process = sched.processes.iter().find(|process| {
            process.pid == pid && process.foreground.incarnation == claim.child.incarnation
        });
        if process.is_some_and(|process| {
            process.state != ProcessState::Reaped
                || process.child_wait == crate::process::ChildWait::Pending
        }) || sched
            .pending_children
            .pids()
            .any(|reserved| reserved == pid)
        {
            continue;
        }
        let remote = runtime
            .remote
            .iter()
            .flatten()
            .find(|remote| remote.registration == claim.registration);
        if matches!(audit.mode, 5..=8) {
            if remote.is_some() {
                continue;
            }
            assert_ne!(runtime.registry.active(), Some(claim.registration));
        } else {
            let remote = remote.expect("fixture registration remains active");
            if remote.ownership.contains(claim.request_id)
                || remote.callers.iter().flatten().any(|entry| entry.caller == child)
            {
                continue;
            }
            assert!(remote.local.iter().flatten().all(|lease| lease.id() != claim.request_id));
            assert_ne!(remote.delivered, Some(claim.request_id));
            assert!(remote.unsent.iter().flatten().all(|frame| {
                !matches!(frame, Frame::Retain { request_id, .. } if *request_id == claim.request_id)
            }));
        }
        #[cfg(feature = "vfs_l1c_parent_lifecycle_test")]
        if matches!(audit.mode, 12..=16) && !parent_settled(runtime, sched, audit.parent) {
            continue;
        }
        let published = matches!(audit.mode, 0 | 8 | 14 | 15);
        assert!(published || process.is_none());
        assert!(sched.pending_children.private(claim.child).is_none());
        assert!(runtime.spawn_request(claim.registration, claim.request_id).is_none());
        assert!(runtime.namespaces.lookup(audit.identity).is_err());
        assert_eq!(crate::ipc::vfs_resources::caller_charge(child), (0, 0));
        assert!(runtime
            .publications
            .iter()
            .flatten()
            .all(|op| op.identity.caller != child));
        assert!(runtime
            .file_publications
            .iter()
            .flatten()
            .all(|op| op.identity.caller != child));
        assert!(runtime
            .chdirs
            .iter()
            .flatten()
            .all(|op| op.identity.caller != child && op.waiter != child));
        if let Some(process) = process {
            assert_eq!(process.child_wait, crate::process::ChildWait::Consumed);
            assert!(
                process.address_space.is_reclaimed() && process.address_space.region_count() == 0
            );
            assert!((0..256).all(|fd| process.fd_table.get(fd).is_none()));
            assert!(process.kernel_stack.is_none());
            assert!(process.ipc_endpoint.is_none() && process.pending_call.is_none());
            assert!(process.ipc_queue.is_empty() && process.ipc_reply.is_none());
            assert!(
                process.ipc_reply_target.is_none() && process.deferred_reply_targets.is_empty()
            );
        }
        assert_eq!(crate::PMM.lock().owned_frame_count(pid as u32), 0);
        let mut ipc_refs = 0;
        crate::ipc::for_all_shards(|bus| ipc_refs += bus.pending_count_for_pid(pid));
        assert_eq!(ipc_refs, 0);
        AllocatorSample::read(sched).log("settled", claim);
        log_heap(sched, "r5-settled", usize::from(published), pid);
        crate::serial_println!(
            "[VFS-ACCT] PASS child={} retain={} epoch={} published={} reaped_slot={} fd_table_bytes={} live_frames=0 private_pid=0 adapter=0 continuation=0 vfs_claim=0 ipc_refs=0 stat_bytes=0 stat_requests=0",
            pid, claim.request_id, claim.registration.epoch, usize::from(published),
            usize::from(process.is_some()), core::mem::size_of::<crate::process::fd_table::FdTable>(),
        );
        *slot = None;
    }
}

pub(crate) fn heap_checkpoint(cycle: usize, pid: usize) -> u64 {
    let sched = SCHEDULER.lock();
    if sched.current_process().pid != 1 {
        return u64::MAX;
    }
    if AUDITS.lock().iter().any(Option::is_some) {
        return 0;
    }
    let runtime = RUNTIME.lock();
    if pid != 0 {
        let process = sched
            .processes
            .iter()
            .find(|process| process.pid == pid)
            .expect("ordinary spawn record");
        if process.state != ProcessState::Reaped {
            return 0;
        }
        assert_eq!(process.child_wait, crate::process::ChildWait::Consumed);
        assert!(process.address_space.is_reclaimed());
        assert_eq!(crate::PMM.lock().owned_frame_count(pid as u32), 0);
        assert!(process.kernel_stack.is_none());
        assert!((0..256).all(|fd| process.fd_table.get(fd).is_none()));
        let matches_child = |caller: Caller| {
            caller.pid == pid as u64 && caller.incarnation == process.foreground.incarnation
        };
        // The reaper retires admission synchronously. The VFS service
        // acknowledges caller retirement and retain compensation separately.
        if runtime.remote.iter().flatten().any(|remote| {
            remote.callers.iter().flatten().any(|row| matches_child(row.caller))
                || remote.local.iter().flatten().any(|lease| match lease {
                    LocalLease::Preparing { scope, .. } => matches_child(scope.caller),
                    LocalLease::Bound { receipt, .. } => matches_child(receipt.scope().caller),
                })
                || remote.ownership.releases().any(|frame| {
                    matches!(frame, Frame::Release(receipt) if matches_child(receipt.scope().caller))
                })
        }) {
            return 0;
        }
    }
    if matches!(cycle, 0 | 1 | 10 | 100 | 1000) {
        log_heap(&sched, "ordinary", cycle, pid);
    }
    1
}

fn buffer_bytes<T>(queue: &alloc::collections::VecDeque<T>) -> usize {
    core::mem::size_of::<T>() * queue.capacity()
}

#[cfg(feature = "vfs_l1c_file_ipc_test")]
type FileIpcWitness = (AdmissionIdentity, [u32; 2], [crate::capability::CapabilityToken; 2]);

#[cfg(feature = "vfs_l1c_file_ipc_test")]
static FILE_IPC_WITNESS: Mutex<Option<FileIpcWitness>> = Mutex::new(None);

#[cfg(feature = "vfs_l1c_file_ipc_test")]
pub(crate) fn file_ipc_checkpoint(cycle: usize, pid: usize) -> u64 {
    use crate::capability::CAP_BROKER;
    use crate::memory::shared_quota::{observation, ProcessIdentity};
    use crate::process::fd_table::FdTable;

    let sched = SCHEDULER.lock();
    let current = sched.current_process();
    let runtime = RUNTIME.lock();
    let mut witness = FILE_IPC_WITNESS.lock();
    let registration = runtime.registry.active().expect("live VFS registration");
    let (descriptions, references) = FdTable::description_counts();
    let quota = |identity: AdmissionIdentity| {
        observation(ProcessIdentity {
            pid: identity.caller.pid as usize,
            incarnation: identity.caller.incarnation,
        }, None).0
    };
    if cycle == 0 {
        if current.ppid != 1 || current.name_str() != "vfs-lifecycle-parent" {
            return u64::MAX;
        }
        assert!(witness.is_none());
        let identity = AdmissionIdentity {
            registration,
            caller: process_identity(current),
            uid: current.uid,
            gid: current.gid,
            groups: [],
        };
        if pid == 1 {
            return u64::from(runtime.namespaces.lookup(identity).is_ok());
        }
        assert_eq!(pid, 0);
        assert!(runtime.namespaces.lookup(identity).is_ok());
        let descriptors = (0..256).filter(|&fd| current.fd_table.get(fd).is_some()).count();
        assert_eq!(descriptors, 6);
        assert!((0..3).all(|fd| current.fd_table.get(fd).is_some()));
        assert_eq!(current.owned_shared.len(), 2);
        assert_eq!(current.mapped_shared.len(), 2);
        assert_eq!(quota(identity), 12288);
        let tokens = core::array::from_fn(|index| current.owned_shared[index].token);
        let endpoints = {
            let caps = CAP_BROKER.lock();
            assert!(tokens.iter().all(|token| caps.shared_region_map_count(*token) == Some(1)));
            let ids = caps.endpoints_owned_by(current.pid);
            assert_eq!(ids.len(), 2);
            [ids[0], ids[1]]
        };
        let mut queued = 0;
        let mut queue_bytes = 0;
        for endpoint in endpoints {
            let (messages, bytes) = crate::ipc::with_shard(endpoint, |bus| bus.file_ipc_queue(endpoint))
                .unwrap_or_default();
            queued += messages;
            queue_bytes += bytes;
        }
        let hhdm = x86_64::VirtAddr::new(crate::HHDM_REQ.response().unwrap().offset);
        let pages = unsafe { current.address_space.count_user_pages(hhdm) };
        crate::serial_println!(
            "[VFS-FILE-IPC] live pid={} incarnation={} epoch={} descriptors={} descriptions={} references={} endpoints=2 queued={} queue_bytes={} shm_objects=2 shm_maps=2 shm_bytes={} mapped_pages={} owned_frames={}",
            current.pid, identity.caller.incarnation, registration.epoch, descriptors, descriptions, references,
            queued, queue_bytes, quota(identity), pages, crate::PMM.lock().owned_frame_count(current.pid as u32),
        );
        *witness = Some((identity, endpoints, tokens));
        log_heap(&sched, "file-ipc-live", cycle, current.pid);
        return 1;
    }
    if current.pid != 1 {
        return u64::MAX;
    }
    let phase = if pid == 0 {
        assert!(witness.is_none());
        "baseline"
    } else {
        let (identity, endpoints, tokens) = witness.expect("child live witness");
        assert_eq!(identity.caller.pid as usize, pid);
        if !parent_settled(&runtime, &sched, identity) {
            return 0;
        }
        let process = sched.processes.iter().find(|process| process.pid == pid).unwrap();
        assert!(process.owned_shared.is_empty() && process.mapped_shared.is_empty());
        assert_eq!(quota(identity), 0);
        {
            let caps = CAP_BROKER.lock();
            assert!(!caps.has_owned_endpoints(pid));
            assert!(tokens.iter().all(|token| caps.shared_region_map_count(*token).is_none()));
        }
        assert!(endpoints.iter().all(|&id| crate::ipc::with_shard(id, |bus| bus.file_ipc_queue(id)).is_none()));
        *witness = None;
        "settled"
    };
    crate::serial_println!(
        "[VFS-FILE-IPC] {} cycle={} pid={} epoch={} descriptions={} references={} broker_shm={}",
        phase, cycle, pid, registration.epoch, descriptions, references, CAP_BROKER.lock().shared_region_count(),
    );
    log_heap(&sched, phase, cycle, pid);
    1
}

fn log_heap(sched: &Scheduler, phase: &str, cycle: usize, pid: usize) {
    use core::mem::size_of;

    let process_size = size_of::<crate::process::Process>();
    let table_bytes = sched.processes.capacity() * process_size;
    let fd_box_bytes = size_of::<crate::process::fd_table::FdTable>();
    let mut reaped = 0;
    let mut reaped_ipc = 0;
    let mut live_ipc = 0;
    for process in &sched.processes {
        let ipc = buffer_bytes(&process.ipc_queue) + buffer_bytes(&process.deferred_reply_targets);
        if process.state == ProcessState::Reaped {
            reaped += 1;
            reaped_ipc += ipc;
        } else {
            live_ipc += ipc;
        }
    }
    let mut rr_bytes = 0;
    let mut tier_bytes = 0;
    for (cpu, cell) in crate::sched::CORE_STATES.iter().enumerate() {
        rr_bytes += cell.lock().rr_queue.capacity() * size_of::<usize>();
        let core = &sched.cores[cpu];
        tier_bytes += (core.run_queue_high.capacity()
            + core.run_queue_medium.capacity()
            + core.run_queue_low.capacity())
            * size_of::<usize>();
    }
    let pmm = crate::PMM.lock();
    use crate::memory::accounting::{class_frame_count, PhysicalMemoryClass};
    crate::serial_println!(
        "[VFS-HEAP] phase={} cycle={} pid={} heap={} frames={} user_frames={} page_tables={} rows={} row_bytes={} slots={} table_bytes={} reaped={} fd_box_bytes={} reaped_ipc={} live_ipc={} rr_bytes={} tier_bytes={}",
        phase, cycle, pid, crate::memory::heap::heap_stats().allocated,
        pmm.free_page_count(),
        class_frame_count(PhysicalMemoryClass::UserPrivate),
        class_frame_count(PhysicalMemoryClass::PageTable),
        sched.processes.len(), process_size, sched.processes.capacity(), table_bytes,
        reaped, fd_box_bytes, reaped_ipc, live_ipc, rr_bytes, tier_bytes,
    );
}

#[cfg(feature = "vfs_l1c_adapter_accounting_test")]
pub(in crate::ipc::vfs_registration::spawn) fn before_unsent_cancel(
    runtime: &Runtime,
    claim: RemoteClaim,
) {
    if !AUDITS.lock().iter().flatten().any(|audit| audit.claim == claim && matches!(audit.mode, 1 | 12)) {
        return;
    }
    let remote = runtime.remote.iter().flatten()
        .find(|remote| remote.registration == claim.registration && !remote.retired)
        .expect("active unsent registration");
    assert!(remote.unsent.iter().flatten().any(|frame| {
        matches!(frame, Frame::Retain { request_id, .. } if *request_id == claim.request_id)
    }));
    assert_ne!(remote.delivered, Some(claim.request_id));
    assert!(remote.ownership.contains(claim.request_id));
    crate::serial_println!(
        "[VFS-ADAPTER] unsent child={} retain={} queued=1 delivered=0",
        claim.child.ticket.pid, claim.request_id
    );
}

#[cfg(feature = "vfs_l1c_adapter_accounting_test")]
pub(super) fn before_consumed_cancel(runtime: &Runtime, claim: RemoteClaim, receipt: Receipt) {
    let remote = runtime.remote.iter().flatten()
        .find(|remote| remote.registration == claim.registration && !remote.retired)
        .expect("active retained registration");
    assert_eq!(receipt.request_id(), claim.request_id);
    assert_eq!(receipt.scope().caller, caller(claim.child));
    assert!(remote.ownership.holds(receipt));
    assert_ne!(remote.delivered, Some(claim.request_id));
    crate::serial_println!(
        "[VFS-ADAPTER] consumed child={} retain={} held=1 delivered=0",
        claim.child.ticket.pid, claim.request_id
    );
}

#[cfg(feature = "vfs_l1c_parent_lifecycle_test")]
fn parent_settled(runtime: &Runtime, sched: &Scheduler, parent: AdmissionIdentity) -> bool {
    let pid = parent.caller.pid as usize;
    let process = sched.processes.iter().find(|process| {
        process.pid == pid && process.foreground.incarnation == parent.caller.incarnation
    }).expect("lifecycle parent record");
    if process.state != ProcessState::Reaped
        || process.child_wait != crate::process::ChildWait::Consumed
        || runtime.remote.iter().flatten().any(|remote| {
            remote.callers.iter().flatten().any(|entry| entry.caller == parent.caller)
                || remote.local.iter().flatten().any(|lease| match lease {
                    LocalLease::Preparing { scope, .. } => scope.caller == parent.caller,
                    LocalLease::Bound { receipt, .. } => receipt.scope().caller == parent.caller,
                })
                || remote.ownership.releases().any(|frame| {
                    matches!(frame, Frame::Release(receipt) if receipt.scope().caller == parent.caller)
                })
        })
    {
        return false;
    }
    assert_eq!(runtime.registry.active(), Some(parent.registration));
    assert!(runtime.namespaces.lookup(parent).is_err());
    assert!(runtime.spawns.iter().flatten().all(|call| call.parent.caller != parent.caller));
    assert!(runtime.publications.iter().flatten().all(|op| op.identity.caller != parent.caller));
    assert!(runtime.file_publications.iter().flatten().all(|op| op.identity.caller != parent.caller));
    assert!(runtime.chdirs.iter().flatten().all(|op| op.identity.caller != parent.caller && op.waiter != parent.caller));
    assert_eq!(crate::ipc::vfs_resources::caller_charge(parent.caller), (0, 0));
    assert_eq!(process.exit_code, 37);
    assert!(process.address_space.is_reclaimed() && process.address_space.region_count() == 0);
    assert!(process.kernel_stack.is_none() && !process.exit_cleanup_pending);
    assert!((0..256).all(|fd| process.fd_table.get(fd).is_none()));
    assert!(process.ipc_endpoint.is_none() && process.pending_call.is_none());
    assert!(process.ipc_queue.is_empty() && process.ipc_reply.is_none());
    assert!(process.ipc_reply_target.is_none() && process.deferred_reply_targets.is_empty());
    let mut ipc_refs = 0;
    crate::ipc::for_all_shards(|bus| ipc_refs += bus.pending_count_for_pid(pid));
    assert_eq!(ipc_refs, 0);
    assert_eq!(crate::PMM.lock().owned_frame_count(pid as u32), 0);
    crate::serial_println!(
        "[VFS-PARENT] reaped parent={} incarnation={} status=37 descriptors=0 live_frames=0 stack=0 ipc_refs=0 namespace=0 vfs_caller=0",
        pid, parent.caller.incarnation,
    );
    true
}
