//! Native witness for retirement after publication. Only the published child
//! can enter its checkpoint, and only init can initiate retirement or collect it.
use super::*;
use crate::process::{pending_state, ChildWait};

pub(super) static IMAGE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/sunlight-utils"));

static CLAIM: Mutex<Option<RemoteClaim>> = Mutex::new(None);
static CHECKPOINT: AtomicUsize = AtomicUsize::new(0);
static RELEASED: AtomicUsize = AtomicUsize::new(0);

pub(super) fn publish_child(
    idx: usize,
    claim: RemoteClaim,
    outcome: Terminal,
    sched: &mut Scheduler,
) -> u64 {
    let Terminal::Retained(receipt) = outcome else {
        panic!("R3 registration failed: {outcome:?}");
    };
    let who = caller(claim.child);
    assert_eq!(receipt.registration(), claim.registration);
    assert_eq!(receipt.request_id(), claim.request_id);
    assert_eq!(receipt.scope().caller, who);
    crate::serial_println!(
        "[VFS-R3] retained service={} child={} retain={} epoch={} incarnation={} vm={}",
        claim.registration.owner.pid,
        who.pid,
        claim.request_id,
        claim.registration.epoch,
        who.incarnation,
        who.address_space_generation
    );
    let pid = publish(idx, claim, sched, &CAP_BROKER.lock()).expect("publish R3 child");
    assert_eq!(pid, claim.child.ticket.pid);
    assert_completed(claim, sched);
    *CLAIM.lock() = Some(claim);
    pid as u64
}

fn assert_completed(claim: RemoteClaim, sched: &Scheduler) {
    let pid = claim.child.ticket.pid;
    assert!(sched.pending_children.private(claim.child).is_none());
    assert!(!sched
        .pending_children
        .pids()
        .any(|reserved| reserved == pid));
    assert!(RUNTIME
        .lock()
        .spawn_request(claim.registration, claim.request_id)
        .is_none());
    let process = &sched.processes[sched.process_index_by_pid(pid).expect("scheduler child")];
    assert_eq!(process_identity(process), caller(claim.child));
    assert!(process.process_lifetime_live());
    assert!(process.vfs_stat_routed);
    assert_eq!(process.child_wait, ChildWait::Pending);
    assert_eq!(
        (0..256)
            .filter(|&fd| process.fd_table.get(fd).is_some())
            .count(),
        3
    );
}

pub(super) fn resume(mode: u64) -> Option<u64> {
    if !matches!(mode, 8..=10) {
        return None;
    }
    // Keep the same lock order as the initial fixture entry.
    let mut adapter = CALL.lock();
    let mut sched = SCHEDULER.lock();
    let current = sched.current_process();
    let claim = *CLAIM.lock();
    if mode == 8 && current.pid == 1 && claim.is_none() {
        return None;
    }
    let Some(claim) = claim else {
        return Some(u64::MAX);
    };
    let pid = claim.child.ticket.pid;
    if mode == 9 && process_identity(current) == caller(claim.child) {
        assert_eq!(METADATA.load(Relaxed), pid);
        if CHECKPOINT.swap(pid, Relaxed) == 0 {
            crate::serial_println!("[VFS-R3] checkpoint child={} stat=complete", pid);
        }
        return Some(u64::from(RELEASED.load(Relaxed) == pid));
    }
    if current.pid != 1 || mode == 9 {
        return Some(u64::MAX);
    }
    let service = claim.registration.owner.pid as usize;
    if mode == 8 {
        if CHECKPOINT.load(Relaxed) != pid {
            return Some(0);
        }
        if EXIT_REQUESTED.load(Relaxed) == 0 {
            assert_completed(claim, &sched);
            assert!(RUNTIME.lock().remote.iter().flatten().any(|remote| {
                remote.registration == claim.registration && !remote.retired
                    && remote.local.iter().flatten().any(|lease| {
                        matches!(lease, LocalLease::Bound { receipt, .. }
                            if receipt.request_id() == claim.request_id && receipt.scope().caller == caller(claim.child))
                    })
            }));
            EXIT_REQUESTED.store(service, Relaxed);
            assert!(sched.terminate_process_by_pid(service, 0, "vfs-r3-published-child"));
        }
        if EXIT_PROVEN.load(Relaxed) != service {
            return Some(0);
        }
        assert_completed(claim, &sched);
        let idx = sched.current_process_index().unwrap();
        assert_eq!(
            wait_for_publication(idx, claim, &mut sched),
            Err(Error::Identity)
        );
        assert_eq!(
            publish(idx, claim, &mut sched, &CAP_BROKER.lock()),
            Err(Error::Identity)
        );
        assert_completed(claim, &sched);
        assert_eq!(RELEASED.swap(pid, Relaxed), 0);
        crate::serial_println!(
            "[VFS-R3] continued child={} retain={} continuation=0 private=0 scheduler=1 replay=Identity",
            pid, claim.request_id
        );
        return Some(1);
    }

    assert_eq!(mode, 10);
    assert_eq!(RELEASED.load(Relaxed), pid);
    let process = sched
        .processes
        .iter()
        .find(|process| process.pid == pid)
        .expect("tracked child reaper record");
    if process.state != ProcessState::Reaped {
        return Some(0);
    }
    assert_eq!(process.foreground.incarnation, claim.child.incarnation);
    assert_eq!(process.child_wait, ChildWait::Consumed);
    assert!(process.address_space.is_reclaimed());
    assert!(process.kernel_stack.is_none());
    assert!(!process.exit_cleanup_pending);
    assert!((0..256).all(|fd| process.fd_table.get(fd).is_none()));
    assert_eq!(crate::PMM.lock().owned_frame_count(pid as u32), 0);
    assert!(!sched
        .pending_children
        .pids()
        .any(|reserved| reserved == pid));
    crate::serial_println!(
        "[VFS-R3] reaped child={} wait=consumed pid_reserved=0 root=freed stack=freed owned_frames=0 fd_slots=0",
        pid
    );
    let (child, owned_claim) = adapter.take().expect("one R3 adapter");
    assert_eq!(owned_claim, claim);
    *REQUEST.lock() = None;
    drop(sched);
    assert_eq!(child.identity(), Err(pending_state::Error::Stale));
    assert_eq!(child.cancel(), Err(pending_state::Error::Stale));
    drop(child);
    #[cfg(feature = "vfs_l1c_adapter_accounting_test")]
    accounting::dispose(claim);
    crate::serial_println!(
        "[VFS-R3] disposed child={} retain={} owner=scheduler cancel=Stale",
        pid,
        claim.request_id
    );
    let observer = OBSERVER.load(Relaxed);
    assert_eq!(OTHER.load(Relaxed), observer);
    Some(observer as u64)
}

pub(in crate::ipc::vfs_registration) fn withdrawn(runtime: &Runtime, registration: Registration) {
    let Some(claim) = *CLAIM.lock() else {
        return;
    };
    if registration != claim.registration {
        return;
    }
    assert_eq!(
        EXIT_REQUESTED.load(Relaxed),
        registration.owner.pid as usize
    );
    assert!(runtime
        .spawn_request(registration, claim.request_id)
        .is_none());
    assert_ne!(runtime.registry.active(), Some(registration));
    let remote = runtime
        .remote
        .iter()
        .flatten()
        .find(|remote| remote.registration == registration && remote.retired)
        .expect("retired row before image proof");
    assert!(!remote.image_retired);
    let who = caller(claim.child);
    assert!(remote.ownership.releases().any(|frame| {
        matches!(frame, Frame::Release(receipt)
            if receipt.request_id() == claim.request_id && receipt.scope().caller == who)
    }));
    crate::serial_println!(
        "[VFS-R3] withdrawn service={} child={} retain={} epoch={} release_pending=1 continuation=0",
        registration.owner.pid, who.pid, claim.request_id, registration.epoch
    );
}

pub(super) fn proof(image: crate::capability::delivery::RecipientIdentity) {
    let claim = CLAIM.lock().expect("R3 publication recorded");
    let registration = claim.registration;
    assert_eq!(registration.owner.pid as usize, image.pid);
    assert_eq!(registration.owner.incarnation, image.incarnation);
    assert_eq!(
        registration.owner.address_space_generation,
        image.vm_generation
    );
    let runtime = RUNTIME.lock();
    assert!(runtime
        .spawn_request(registration, claim.request_id)
        .is_none());
    assert!(runtime
        .remote
        .iter()
        .flatten()
        .all(|remote| remote.registration != registration));
    assert_eq!(EXIT_PROVEN.swap(image.pid, Relaxed), 0);
    crate::serial_println!(
        "[VFS-R3] proof service={} incarnation={} vm={} epoch={} retain={} row_retained=false",
        image.pid,
        image.incarnation,
        image.vm_generation,
        registration.epoch,
        claim.request_id
    );
}
