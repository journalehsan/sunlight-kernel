//! Native kernel prerequisites, not scheduled hostile userspace lifecycle proof.
use super::{pmm::PhysicalMemoryManager, shared::*, shared_quota::*};
use crate::{capability::CapabilityBroker, process::Process};
use core::sync::atomic::Ordering;
use x86_64::VirtAddr;

fn process(pid: usize, pmm: &mut PhysicalMemoryManager, hhdm: VirtAddr) -> Process {
    let mut process = unsafe { Process::new(pid, 0, "quota-prerequisite", pmm, hhdm) };
    process.system_allocation = true; // This is a trusted kernel prerequisite.
    process
}
fn identity(p: &Process) -> ProcessIdentity {
    ProcessIdentity {
        pid: p.pid,
        incarnation: p.foreground.incarnation,
    }
}

pub fn run(pmm: &mut PhysicalMemoryManager, hhdm: VirtAddr) {
    let cpus = crate::sched::ONLINE_CORES.load(Ordering::Acquire);
    crate::serial_println!(
        "[SHM-M53] event=start source={} run={} cpus={} scope=kernel-prerequisite",
        env!("SUNLIGHT_M53_SOURCE"),
        env!("SUNLIGHT_M53_RUN"),
        cpus
    );
    let baseline = pmm.free_page_count();
    let mut owner = process(0xF531, pmm, hhdm);
    let mut peer = process(0xF532, pmm, hhdm);
    owner.system_allocation = false;
    let free_before_denial = pmm.free_page_count();
    let mut admission_caps = CapabilityBroker::new();
    assert!(matches!(alloc_shared_region(&mut owner, pmm, &mut admission_caps, hhdm, 4096), Err(SharedMemError::InvalidArgument)));
    assert_eq!(pmm.free_page_count(), free_before_denial);
    owner.system_allocation = true;
    crate::serial_println!("[SL41] event=missing-membership-denied backing=0");
    let who = identity(&owner);
    let session = Some(SessionIdentity::Pty {
        broker: who.incarnation,
        id: 1,
        generation: 1,
    });
    owner.foreground.session_broker = who.incarnation;
    owner.foreground.session = 1;
    owner.foreground.generation = 1;
    let mut caps = CapabilityBroker::new();
    let free = pmm.free_page_count();
    // Real backing allocation fails AFTER reservation, after one frame succeeds.
    pmm.fail_test_allocations_after(1);
    assert_eq!(
        alloc_shared_region(&mut owner, pmm, &mut caps, hhdm, 8192),
        Err(SharedMemError::OutOfMemory)
    );
    pmm.clear_test_allocation_failure();
    assert_eq!(pmm.free_page_count(), free);
    assert_eq!(observation(who, session), (0, 0));
    assert_eq!(caps.shared_region_count(), 0);
    assert_eq!(
        alloc_shared_region(&mut owner, pmm, &mut caps, hhdm, usize::MAX),
        Err(SharedMemError::InvalidArgument)
    );
    assert_eq!(
        alloc_shared_region(&mut owner, pmm, &mut caps, hhdm, PROCESS_LIMIT + 4096),
        Err(SharedMemError::OutOfMemory)
    );
    assert_eq!(pmm.free_page_count(), free);
    crate::serial_println!("[SHM-M53] event=rollback pid={} incarnation={} vm={} quota_before=0 quota_after=0 free_before={} free_after={} partial_frames=1 overflow=denied over_budget=denied objects=0", who.pid, who.incarnation, owner.address_space.identity().generation, free, pmm.free_page_count());
    // Exercise full ceilings without claiming physical allocation at the ceiling.
    let r = reserve(who, session, PROCESS_LIMIT).unwrap();
    assert!(reserve(who, session, 4096).is_none());
    assert_eq!(observation(who, session), (PROCESS_LIMIT, PROCESS_LIMIT));
    drop(r);
    crate::serial_println!(
        "[SHM-M53] event=ledger-boundary process_limit={} quota_after=0 scope=reservation-only",
        PROCESS_LIMIT
    );
    // Exercise the production ceiling with actual backing on the smoke CPU.
    // Other CPU counts exercise the same small lifecycle + remote-TLB path.
    if cpus == 1 {
        let before = pmm.free_page_count();
        let (_, token) =
            alloc_shared_region(&mut owner, pmm, &mut caps, hhdm, PROCESS_LIMIT).unwrap();
        assert_eq!(observation(who, session), (PROCESS_LIMIT, PROCESS_LIMIT));
        let charged_free = pmm.free_page_count();
        assert_eq!(
            alloc_shared_region(&mut owner, pmm, &mut caps, hhdm, 4096),
            Err(SharedMemError::OutOfMemory)
        );
        assert_eq!(pmm.free_page_count(), charged_free);
        free_shared_page(&mut owner, token, pmm, &mut caps, hhdm).unwrap();
        assert_eq!(observation(who, session), (0, 0));
        assert_eq!(pmm.free_page_count(), before);
        crate::serial_println!("[SHM-M53] event=physical-boundary bytes={} frames_allocated={} frames_released={} quota_live={} quota_after=0 over_budget=denied free_before={} free_after={}", PROCESS_LIMIT, PROCESS_LIMIT/PAGE_SIZE, PROCESS_LIMIT/PAGE_SIZE, PROCESS_LIMIT, before, pmm.free_page_count());
        let chunk = PROCESS_LIMIT / 64;
        for _ in 0..64 {
            alloc_shared_region(&mut owner, pmm, &mut caps, hhdm, chunk).unwrap();
        }
        assert_eq!(observation(who, session), (PROCESS_LIMIT, PROCESS_LIMIT));
        let charged_free = pmm.free_page_count();
        assert_eq!(
            alloc_shared_region(&mut owner, pmm, &mut caps, hhdm, chunk),
            Err(SharedMemError::OutOfMemory)
        );
        assert_eq!(pmm.free_page_count(), charged_free);
        cleanup_shared_pages(&mut owner, pmm, &mut caps);
        assert_eq!(observation(who, session), (0, 0));
        assert_eq!(pmm.free_page_count(), before);
        crate::serial_println!("[SHM-M53] event=small-aggregate allocations=64 chunk={} bytes={} frames_allocated={} frames_released={} quota_after=0 over_budget=denied free_before={} free_after={}", chunk, PROCESS_LIMIT, PROCESS_LIMIT/PAGE_SIZE, PROCESS_LIMIT/PAGE_SIZE, before, pmm.free_page_count());
    }
    for cycle in 0..16 {
        let before = pmm.free_page_count();
        let (_, token) = alloc_shared_region(&mut owner, pmm, &mut caps, hhdm, 8192).unwrap();
        map_shared_page(&mut peer, token, pmm, &mut caps, hhdm).unwrap();
        assert_eq!(observation(who, session), (8192, 8192));
        assert!(caps.revoke_shared(token).is_none());
        cleanup_shared_pages(&mut owner, pmm, &mut caps);
        assert_eq!(observation(who, session), (8192, 8192));
        assert_eq!(caps.shared_region_map_count(token), Some(1));
        free_shared_page(&mut peer, token, pmm, &mut caps, hhdm).unwrap();
        assert_eq!(
            free_shared_page(&mut peer, token, pmm, &mut caps, hhdm),
            Err(SharedMemError::InvalidToken)
        );
        assert!(caps.revoke_shared(token).is_none());
        assert_eq!(observation(who, session), (0, 0));
        assert_eq!(caps.shared_region_count(), 0);
        assert_eq!(pmm.free_page_count(), before);
        crate::serial_println!("[SHM-M53] event=cycle cycle={} pid={} incarnation={} vm={} peer_pid={} peer_incarnation={} peer_vm={} quota_before=0 quota_live=8192 quota_owner_unmapped=8192 quota_after=0 frames_allocated=2 frames_released=2 views_retired=2 objects=0 free_before={} free_after={}", cycle, who.pid, who.incarnation, owner.address_space.identity().generation, peer.pid, peer.foreground.incarnation, peer.address_space.identity().generation, before, pmm.free_page_count());
    }
    unsafe {
        owner.address_space.reclaim_user_space(pmm, hhdm, true);
        peer.address_space.reclaim_user_space(pmm, hhdm, true);
    }
    assert_eq!(pmm.free_page_count(), baseline);
    if cpus > 1 {
        let remote_before = super::tlb::test_remote_invalidation_count();
        super::shared_lifetime_gate::remote_release(pmm, hhdm);
        let acknowledgements = super::tlb::test_remote_invalidation_count() - remote_before;
        assert_eq!(acknowledgements, (cpus - 1) as u64);
        crate::serial_println!("[SHM-M53] event=remote acknowledgements={} expected={} free_before={} free_after={} scope=kernel-shm-prerequisite", acknowledgements, cpus-1, baseline, pmm.free_page_count());
    } else {
        crate::serial_println!(
            "[SHM-M53] event=remote acknowledgements=0 expected=0 scope=no-remote-cpu"
        );
    }
    assert_eq!(pmm.free_page_count(), baseline);
    crate::serial_println!("[SHM-M53] event=complete source={} run={} cpus={} cycles=16 quota_after=0 free_before={} free_after={}", env!("SUNLIGHT_M53_SOURCE"), env!("SUNLIGHT_M53_RUN"), cpus, baseline, pmm.free_page_count());
}
