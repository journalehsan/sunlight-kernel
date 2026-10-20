//! Opt-in kernel VM prerequisite tests. These are not ring-3 FastLane peers.
use super::pmm::PhysicalMemoryManager;
use super::shared::{
    alloc_shared_region, cleanup_shared_pages, free_shared_page, map_shared_page, SharedMemError,
};
use crate::capability::{CapError, CapabilityBroker, CapabilityRights, CapabilityToken};
use crate::process::Process;
use core::sync::atomic::Ordering;
use x86_64::structures::paging::{Page, PageTableFlags, PhysFrame, Size4KiB};
use x86_64::VirtAddr;

pub fn run(pmm: &mut PhysicalMemoryManager, hhdm: VirtAddr) {
    capability_admission_probe();
    let baseline = pmm.free_page_count();
    let mut previous_identity = None;
    for cycle in 0..32 {
        let mut owner = unsafe { Process::new(0xF101, 0, "f1-owner", pmm, hhdm) };
        let mut peer = unsafe { Process::new(0xF102, 0, "f1-peer", pmm, hhdm) };
        let identity = owner.address_space.identity();
        if let Some(previous) = previous_identity {
            assert_ne!(identity, previous, "address-space identity reused");
        }
        previous_identity = Some(identity);
        let mut caps = CapabilityBroker::new();
        let (owner_base, token) =
            alloc_shared_region(&mut owner, pmm, &mut caps, hhdm, 8192).unwrap();
        let peer_base = map_shared_page(&mut peer, token, pmm, &mut caps, hhdm).unwrap();
        map_shared_page(&mut peer, token, pmm, &mut caps, hhdm).unwrap();
        assert_eq!(caps.shared_region_map_count(token), Some(3));
        let first = caps.resolve_shared_region(token).unwrap().frames[0];
        unsafe {
            (hhdm + first.start_address().as_u64())
                .as_mut_ptr::<u64>()
                .write_volatile(0xF101)
        };

        // Demonstrate the exact failed F1B admission prerequisite. The peer has
        // no recipient grant, yet possession of a legacy token maps it RW/NX.
        // No FastLane transport is built on this legacy authority.
        let peer_page = Page::<Size4KiB>::from_start_address(peer_base).unwrap();
        let (_, peer_flags) = unsafe { peer.address_space.lookup_entry(peer_page, hhdm) }.unwrap();
        assert!(peer_flags.contains(PageTableFlags::WRITABLE | PageTableFlags::NO_EXECUTE));
        if cycle == 0 {
            crate::serial_println!(
                "[FASTLANE-F1] isolation prerequisite: BLOCKED (legacy bearer RW/NX)"
            );
        }

        // Both explicit-token and owner-wide retirement must retain views.
        if cycle % 2 == 0 {
            assert!(caps.revoke_shared(token).is_none());
        } else {
            caps.revoke_all_for(owner.pid);
        }
        assert!(caps.revoke_shared(token).is_none());
        assert!(caps.resolve_shared_region(token).is_none());
        assert_eq!(caps.validate_shared_page(token), Err(CapError::Revoked));
        let free_after_retirement = pmm.free_page_count();
        assert_eq!(
            map_shared_page(&mut peer, token, pmm, &mut caps, hhdm),
            Err(SharedMemError::InvalidToken)
        );
        assert_eq!(pmm.free_page_count(), free_after_retirement);
        assert_eq!(caps.shared_region_map_count(token), Some(3));
        assert_eq!(
            unsafe { peer.address_space.lookup_entry(peer_page, hhdm) }
                .unwrap()
                .0,
            first.start_address()
        );

        // Release a temporary duplicate, then corrupt the second page of the
        // persistent owner view. A whole-view preflight must reject cleanup
        // before removing its first page or releasing any backing reference.
        free_shared_page(&mut peer, token, pmm, &mut caps, hhdm).unwrap();
        let owner_page = Page::<Size4KiB>::from_start_address(owner_base).unwrap();
        let missing_page = Page::<Size4KiB>::from_start_address(owner_base + 4096u64).unwrap();
        let (missing_frame, missing_flags) =
            unsafe { owner.address_space.lookup_entry(missing_page, hhdm) }.unwrap();
        assert_eq!(
            unsafe { owner.address_space.unmap_page(missing_page, hhdm) },
            Some(missing_frame)
        );
        let free_before_failed_release = pmm.free_page_count();
        assert_eq!(
            free_shared_page(&mut owner, token, pmm, &mut caps, hhdm),
            Err(SharedMemError::InternalInvariant)
        );
        assert_eq!(caps.shared_region_map_count(token), Some(2));
        assert_eq!(owner.mapped_shared.len(), 1);
        assert_eq!(pmm.free_page_count(), free_before_failed_release);
        assert_eq!(
            unsafe { owner.address_space.lookup_entry(owner_page, hhdm) }
                .unwrap()
                .0,
            first.start_address()
        );
        unsafe {
            owner
                .address_space
                .map_page(
                    missing_page,
                    PhysFrame::from_start_address_unchecked(missing_frame),
                    missing_flags,
                    pmm,
                    hhdm,
                )
                .unwrap();
        }

        // Exit cleanup releases a retired owner's view, preserving the peer.
        cleanup_shared_pages(&mut owner, pmm, &mut caps);
        assert_eq!(caps.shared_region_map_count(token), Some(1));
        assert_eq!(
            unsafe {
                (hhdm + first.start_address().as_u64())
                    .as_ptr::<u64>()
                    .read_volatile()
            },
            0xF101
        );
        free_shared_page(&mut peer, token, pmm, &mut caps, hhdm).unwrap();
        assert_eq!(
            free_shared_page(&mut peer, token, pmm, &mut caps, hhdm),
            Err(SharedMemError::InvalidToken)
        );
        assert_eq!(caps.shared_region_count(), 0);
        cleanup_shared_pages(&mut owner, pmm, &mut caps);
        cleanup_shared_pages(&mut peer, pmm, &mut caps);
        unsafe {
            peer.address_space.reclaim_user_space(pmm, hhdm, true);
            owner.address_space.reclaim_user_space(pmm, hhdm, true);
        }
        assert_eq!(
            pmm.free_page_count(),
            baseline,
            "SHM frame drift cycle={cycle}"
        );
    }
    crate::serial_println!(
        "[FASTLANE-F1] retired views, failed-unmap preflight, double free, 32 lifecycles: OK"
    );
    remote_release(pmm, hhdm);
    assert_eq!(pmm.free_page_count(), baseline);
    crate::serial_println!(
        "[FASTLANE-F1] prerequisite lifetime gate: OK (channel remains BLOCKED)"
    );
}

fn capability_admission_probe() {
    // The attacker knows only its own returned token. No seed/counter access,
    // credential forgery, or delegation of the private endpoint token occurs.
    // For consecutive counters c and c-1, c XOR (c-1) is a low-bit mask of
    // ones. XORing the same boot seed into both does not hide that relation.
    let mut caps = CapabilityBroker::new();
    let (private_endpoint, _undisclosed) = caps
        .create_endpoint(0xF111)
        .expect("security fixture endpoint issuance");
    let (_, attacker_token) = caps
        .create_endpoint(0xF112)
        .expect("security fixture endpoint issuance");
    let predicted = (1..=62).any(|bits| {
        let guess = CapabilityToken(attacker_token.as_u64() ^ ((1u64 << bits) - 1));
        caps.check(guess, CapabilityRights::SEND_ONLY) == Ok(private_endpoint)
    });
    assert!(
        !predicted,
        "private SEND authority predicted from an adjacent attacker token"
    );
    crate::serial_println!(
        "[FASTLANE-F1] capability issuance regression: adjacent token prediction REJECTED"
    );
}

fn remote_release(pmm: &mut PhysicalMemoryManager, hhdm: VirtAddr) {
    use super::tlb;
    const OLD: u64 = 0xFA57_1A4E_0101;
    const NEW: u64 = 0xFA57_1A4E_0202;
    let baseline = pmm.free_page_count();
    let mut owner = unsafe { Process::new(0xF103, 0, "f1-remote", pmm, hhdm) };
    let mut caps = CapabilityBroker::new();
    let (base, token) = alloc_shared_region(&mut owner, pmm, &mut caps, hhdm, 4096).unwrap();
    let identity = owner.address_space.identity();
    let old_frame = caps.resolve_shared_region(token).unwrap().frames[0].start_address();
    unsafe {
        (hhdm + old_frame.as_u64())
            .as_mut_ptr::<u64>()
            .write_volatile(OLD);
        owner.address_space.activate();
    }
    assert_eq!(unsafe { base.as_ptr::<u64>().read_volatile() }, OLD);
    let cpus = crate::sched::ONLINE_CORES.load(Ordering::Acquire);
    let local = crate::sched::current_cpu_id();
    let targets = ((1u64 << cpus) - 1) & !(1u64 << local);
    tlb::test_activate_and_read(identity, base.as_u64(), targets);
    for cpu in 0..cpus {
        if cpu != local {
            assert_eq!(tlb::test_result(cpu), OLD);
        }
    }
    let remote_before = tlb::test_remote_invalidation_count();
    assert!(caps.revoke_shared(token).is_none());
    free_shared_page(&mut owner, token, pmm, &mut caps, hhdm).unwrap();
    assert_eq!(
        tlb::test_remote_invalidation_count() - remote_before,
        targets.count_ones() as u64
    );
    let guard = pmm.alloc_frame().unwrap();
    assert_eq!(
        guard, old_frame,
        "expected immediate backing reuse for guard"
    );
    let replacement = pmm.alloc_frame().unwrap();
    unsafe {
        (hhdm + guard.as_u64())
            .as_mut_ptr::<u64>()
            .write_volatile(OLD);
        (hhdm + replacement.as_u64())
            .as_mut_ptr::<u64>()
            .write_volatile(NEW);
        owner
            .address_space
            .map_page(
                Page::<Size4KiB>::from_start_address(base).unwrap(),
                PhysFrame::from_start_address_unchecked(replacement),
                PageTableFlags::PRESENT
                    | PageTableFlags::USER_ACCESSIBLE
                    | PageTableFlags::NO_EXECUTE,
                pmm,
                hhdm,
            )
            .unwrap();
    }
    assert_eq!(unsafe { base.as_ptr::<u64>().read_volatile() }, NEW);
    tlb::test_read(identity, base.as_u64(), targets);
    for cpu in 0..cpus {
        if cpu != local {
            assert_eq!(tlb::test_result(cpu), NEW);
        }
    }
    tlb::test_leave(targets);
    unsafe {
        tlb::activate_kernel_root();
        assert_eq!(
            owner
                .address_space
                .unmap_page(Page::from_start_address(base).unwrap(), hhdm),
            Some(replacement)
        );
        owner.address_space.reclaim_user_space(pmm, hhdm, true);
    }
    pmm.free_frame(guard);
    pmm.free_frame(replacement);
    assert_eq!(pmm.free_page_count(), baseline);
    crate::serial_println!(
        "[FASTLANE-F1] remote SHM release: cpus={} acknowledgements={} OK",
        cpus,
        targets.count_ones()
    );
}
