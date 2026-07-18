use core::sync::atomic::{AtomicU64, Ordering};
use x86_64::structures::paging::PageTableFlags;
use x86_64::{PhysAddr, VirtAddr};

static UNSAFE_FORK_REJECTED: AtomicU64 = AtomicU64::new(0);
static RWX_MAPPING_REJECTED: AtomicU64 = AtomicU64::new(0);
static FRAMEBUFFER_MAPPING_REJECTED: AtomicU64 = AtomicU64::new(0);
static USER_FRAMES_SANITIZED: AtomicU64 = AtomicU64::new(0);
static NX_STACK_MAPPINGS: AtomicU64 = AtomicU64::new(0);
static NX_SHM_MAPPINGS: AtomicU64 = AtomicU64::new(0);
static DISPLAY_AUTHORITY: AtomicU64 = AtomicU64::new(0);

fn display_authority(owner_pid: usize, endpoint_id: u32) -> Option<u64> {
    let owner_pid = u32::try_from(owner_pid).ok()?;
    Some(((owner_pid as u64) << 32) | endpoint_id as u64 + 1)
}

pub fn note_unsafe_fork_rejected() {
    UNSAFE_FORK_REJECTED.fetch_add(1, Ordering::Relaxed);
}

pub fn note_rwx_mapping_rejected() {
    RWX_MAPPING_REJECTED.fetch_add(1, Ordering::Relaxed);
}

pub fn note_framebuffer_mapping_rejected() {
    FRAMEBUFFER_MAPPING_REJECTED.fetch_add(1, Ordering::Relaxed);
}

pub fn note_nx_stack_mapping() {
    NX_STACK_MAPPINGS.fetch_add(1, Ordering::Relaxed);
}

pub fn note_nx_shm_mapping() {
    NX_SHM_MAPPINGS.fetch_add(1, Ordering::Relaxed);
}

pub fn user_stack_flags() -> PageTableFlags {
    PageTableFlags::PRESENT
        | PageTableFlags::WRITABLE
        | PageTableFlags::USER_ACCESSIBLE
        | PageTableFlags::NO_EXECUTE
}

pub fn user_shm_flags() -> PageTableFlags {
    user_stack_flags()
}

pub fn register_display_authority(
    owner_pid: usize,
    endpoint_id: u32,
    caps: &crate::capability::CapabilityBroker,
) {
    let current = DISPLAY_AUTHORITY.load(Ordering::Acquire);
    if current != 0 {
        let current_pid = (current >> 32) as usize;
        let current_endpoint = (current as u32).wrapping_sub(1);
        if caps.endpoint_owner(current_endpoint) == Some(current_pid) {
            return;
        }
    }
    if let Some(authority) = display_authority(owner_pid, endpoint_id) {
        DISPLAY_AUTHORITY.store(authority, Ordering::Release);
    }
}

pub fn revoke_display_authority_for_endpoint(endpoint_id: u32) {
    let current = DISPLAY_AUTHORITY.load(Ordering::Acquire);
    if current as u32 == endpoint_id.wrapping_add(1) {
        let _ = DISPLAY_AUTHORITY.compare_exchange(current, 0, Ordering::AcqRel, Ordering::Acquire);
    }
}

pub fn revoke_display_authority_for_owner(owner_pid: usize) {
    let current = DISPLAY_AUTHORITY.load(Ordering::Acquire);
    if current >> 32 == owner_pid as u64 {
        let _ = DISPLAY_AUTHORITY.compare_exchange(current, 0, Ordering::AcqRel, Ordering::Acquire);
    }
}

pub fn framebuffer_authorized(
    caller_pid: usize,
    caps: &crate::capability::CapabilityBroker,
) -> bool {
    let authority = DISPLAY_AUTHORITY.load(Ordering::Acquire);
    let owner_pid = (authority >> 32) as usize;
    let endpoint = authority as u32;
    authority != 0
        && caller_pid == owner_pid
        && caps.endpoint_owner(endpoint - 1) == Some(owner_pid)
}

pub fn sanitize_user_frame(frame: PhysAddr, hhdm_offset: VirtAddr) -> bool {
    if frame.as_u64() & 0xFFF != 0 || frame.as_u64().checked_add(4095).is_none() {
        return false;
    }
    unsafe {
        core::ptr::write_bytes((hhdm_offset + frame.as_u64()).as_mut_ptr::<u8>(), 0, 4096);
    }
    USER_FRAMES_SANITIZED.fetch_add(1, Ordering::Relaxed);
    true
}

pub fn diagnostic_report() {
    crate::serial_println!(
        "[MM-0-DIAG] fork_rejected={} rwx_rejected={} framebuffer_denied={} user_frames_sanitized={} nx_stacks={} nx_shm={}",
        UNSAFE_FORK_REJECTED.load(Ordering::Relaxed),
        RWX_MAPPING_REJECTED.load(Ordering::Relaxed),
        FRAMEBUFFER_MAPPING_REJECTED.load(Ordering::Relaxed),
        USER_FRAMES_SANITIZED.load(Ordering::Relaxed),
        NX_STACK_MAPPINGS.load(Ordering::Relaxed),
        NX_SHM_MAPPINGS.load(Ordering::Relaxed),
    );
    crate::process::address_space::diagnostic_report();
    crate::process::mmap::diagnostic_report();
    crate::memory::tlb::diagnostic_report();
    let user = crate::memory::user::diagnostics();
    crate::serial_println!(
        "[MM-1-DIAG] noncanonical={} overflow={} kernel_range={} unmapped={} readonly_write={} string_limit={} array_limit={} multipage={}",
        user[0], user[1], user[2], user[3], user[4], user[5], user[6], user[7],
    );
}

pub fn run_boot_self_tests(pmm: &mut crate::memory::pmm::PhysicalMemoryManager, hhdm: VirtAddr) {
    crate::memory::user::run_address_policy_self_tests();
    crate::memory::user::run_mapping_self_tests(pmm, hhdm);
    assert!(crate::arch::x86_64::cpu::nxe_active());
    assert!(user_stack_flags().contains(PageTableFlags::NO_EXECUTE));
    assert!(user_shm_flags().contains(PageTableFlags::NO_EXECUTE));
    crate::serial_println!("[MM-0] NXE and NX mapping flags: OK");

    let free_before = pmm.free_page_count();
    let frame = pmm
        .alloc_frame()
        .expect("MM-0 sanitization test allocation");
    unsafe {
        core::ptr::write_bytes((hhdm + frame.as_u64()).as_mut_ptr::<u8>(), 0xA5, 4096);
    }
    pmm.free_frame(frame);
    let reused = pmm.alloc_frame().expect("MM-0 sanitization test reuse");
    assert!(sanitize_user_frame(reused, hhdm));
    let bytes = unsafe { &*((hhdm + reused.as_u64()).as_ptr::<[u8; 4096]>()) };
    assert!(bytes.iter().all(|byte| *byte == 0));
    pmm.free_frame(reused);
    assert_eq!(pmm.free_page_count(), free_before);
    crate::serial_println!("[MM-0] user-frame reuse sanitization: OK");

    let mmap_free_before = pmm.free_page_count();
    let mmap_process = unsafe { crate::process::Process::new(0xB001, 0, "mm0-mmap", pmm, hhdm) };
    let mut mmap_sched = crate::sched::Scheduler::new();
    mmap_sched.processes.push(mmap_process);
    let rwx_result = crate::process::mmap::sys_mmap(
        0,
        4096,
        crate::process::mmap::PROT_READ
            | crate::process::mmap::PROT_WRITE
            | crate::process::mmap::PROT_EXEC,
        crate::process::mmap::MAP_PRIVATE | crate::process::mmap::MAP_ANONYMOUS,
        -1,
        0,
        pmm,
        &mut mmap_sched,
    );
    assert_eq!(
        rwx_result,
        Err(crate::process::mmap::MmapError::PermissionDenied)
    );
    assert_eq!(pmm.free_page_count(), mmap_free_before - 1);

    let rw_addr = crate::process::mmap::sys_mmap(
        0,
        4096,
        crate::process::mmap::PROT_READ | crate::process::mmap::PROT_WRITE,
        crate::process::mmap::MAP_PRIVATE | crate::process::mmap::MAP_ANONYMOUS,
        -1,
        0,
        pmm,
        &mut mmap_sched,
    )
    .expect("MM-0 anonymous RW mmap");
    let rw_page = x86_64::structures::paging::Page::from_start_address(VirtAddr::new(rw_addr))
        .expect("MM-0 mmap address alignment");
    let (rw_phys, rw_flags) = unsafe {
        mmap_sched.processes[0]
            .address_space
            .lookup_entry(rw_page, hhdm)
            .expect("MM-0 mmap PTE")
    };
    assert!(rw_flags.contains(PageTableFlags::WRITABLE));
    assert!(rw_flags.contains(PageTableFlags::NO_EXECUTE));
    let rw_bytes = unsafe { &*((hhdm + rw_phys.as_u64()).as_ptr::<[u8; 4096]>()) };
    assert!(rw_bytes.iter().all(|byte| *byte == 0));
    unsafe {
        mmap_sched.processes[0]
            .address_space
            .reclaim_user_space(pmm, hhdm, true);
    }
    assert_eq!(pmm.free_page_count(), mmap_free_before);
    crate::serial_println!("[MM-0] anonymous mmap W^X, NX, and zero-fill: OK");

    let shm_free_before = pmm.free_page_count();
    let mut shm_owner =
        unsafe { crate::process::Process::new(0xB101, 0, "mm0-shm-owner", pmm, hhdm) };
    let mut shm_peer =
        unsafe { crate::process::Process::new(0xB102, 0, "mm0-shm-peer", pmm, hhdm) };
    let mut shm_caps = crate::capability::CapabilityBroker::new();
    let (owner_virt, token) =
        crate::memory::shared::alloc_shared_region(&mut shm_owner, pmm, &mut shm_caps, hhdm, 8192)
            .expect("MM-0 SHM allocation");
    let shm_first_frame = {
        let shm = shm_caps
            .resolve_shared_region(token)
            .expect("MM-0 SHM capability");
        for frame in &shm.frames {
            let bytes =
                unsafe { &*((hhdm + frame.start_address().as_u64()).as_ptr::<[u8; 4096]>()) };
            assert!(bytes.iter().all(|byte| *byte == 0));
        }
        let first = shm.frames[0].start_address();
        unsafe {
            (hhdm + first.as_u64())
                .as_mut_ptr::<u8>()
                .write_volatile(0x5A);
        }
        first
    };
    let peer_virt =
        crate::memory::shared::map_shared_page(&mut shm_peer, token, pmm, &mut shm_caps, hhdm)
            .expect("MM-0 SHM peer map");
    for (process, virt) in [(&shm_owner, owner_virt), (&shm_peer, peer_virt)] {
        let page = x86_64::structures::paging::Page::from_start_address(virt)
            .expect("MM-0 SHM address alignment");
        let (_, flags) = unsafe {
            process
                .address_space
                .lookup_entry(page, hhdm)
                .expect("MM-0 SHM PTE")
        };
        assert!(flags.contains(PageTableFlags::WRITABLE));
        assert!(flags.contains(PageTableFlags::NO_EXECUTE));
    }
    assert_eq!(
        unsafe {
            (hhdm + shm_first_frame.as_u64())
                .as_ptr::<u8>()
                .read_volatile()
        },
        0x5A
    );
    crate::memory::shared::cleanup_shared_pages(&mut shm_peer, pmm, &mut shm_caps);
    crate::memory::shared::cleanup_shared_pages(&mut shm_owner, pmm, &mut shm_caps);
    unsafe {
        shm_peer.address_space.reclaim_user_space(pmm, hhdm, true);
        shm_owner.address_space.reclaim_user_space(pmm, hhdm, true);
    }
    assert_eq!(pmm.free_page_count(), shm_free_before);
    crate::serial_println!("[MM-0] SHM NX, zero-fill, and peer preservation: OK");

    let mut caps = crate::capability::CapabilityBroker::new();
    let (endpoint_id, _) = caps.create_endpoint(0xD15A);
    register_display_authority(0xD15A, endpoint_id, &caps);
    assert!(framebuffer_authorized(0xD15A, &caps));
    assert!(!framebuffer_authorized(0xD15B, &caps));
    caps.revoke_endpoints_owned_by(0xD15A);
    assert!(!framebuffer_authorized(0xD15A, &caps));
    let (restart_endpoint, _) = caps.create_endpoint(0xD15C);
    assert_ne!(restart_endpoint, endpoint_id);
    register_display_authority(0xD15C, restart_endpoint, &caps);
    assert!(framebuffer_authorized(0xD15C, &caps));
    caps.revoke_endpoints_owned_by(0xD15C);
    crate::serial_println!("[MM-0] framebuffer authority lifecycle: OK");

    run_mm2a_self_tests(pmm, hhdm);
}

#[cfg(feature = "mm2c_ledger_test")]
pub fn run_mm2c_ledger_gate(pmm: &mut crate::memory::pmm::PhysicalMemoryManager, hhdm: VirtAddr) {
    use crate::process::region::{MappingKind, RegionPolicy};

    let free_before = pmm.free_page_count();
    let process = unsafe { crate::process::Process::new(0xB2C0, 0, "mm2c-ledger", pmm, hhdm) };
    let mut sched = crate::sched::Scheduler::new();
    sched.processes.push(process);
    let address = crate::process::mmap::sys_mmap(
        0,
        8192,
        crate::process::mmap::PROT_READ | crate::process::mmap::PROT_WRITE,
        crate::process::mmap::MAP_PRIVATE | crate::process::mmap::MAP_ANONYMOUS,
        -1,
        0,
        pmm,
        &mut sched,
    )
    .expect("MM-2C anonymous mapping");
    let region = sched.processes[0]
        .address_space
        .lookup_region(address)
        .expect("MM-2C anonymous ledger record");
    assert_eq!(region.kind, MappingKind::Anonymous);
    assert_eq!(region.start, address);
    assert_eq!(region.end, address + 8192);
    assert!(region.policy.contains(RegionPolicy::MAY_UNMAP));
    assert!(region.policy.contains(RegionPolicy::MAY_CHANGE_PROTECTION));
    assert!(unsafe { sched.processes[0].address_space.validate_ledger_ptes(hhdm) });
    crate::serial_println!("[MM-2C] transactional ledger/PTE consistency: OK");

    unsafe {
        sched.processes[0]
            .address_space
            .reclaim_user_space(pmm, hhdm, true);
    }
    assert_eq!(pmm.free_page_count(), free_before);
    crate::serial_println!("[MM-2C] teardown and PMM accounting: OK");
}

#[cfg(feature = "mm2d_munmap_test")]
pub fn run_mm2d_munmap_gate(pmm: &mut crate::memory::pmm::PhysicalMemoryManager, hhdm: VirtAddr) {
    use crate::process::mmap::{
        MmapError, MAP_ANONYMOUS, MAP_FIXED, MAP_PRIVATE, PROT_READ, PROT_WRITE,
    };
    use crate::process::region::{
        MappingKind, MappingRegion, RegionBacking, RegionPolicy, RegionProtection,
        MAX_REGIONS_PER_ADDRESS_SPACE,
    };
    use x86_64::structures::paging::{Page, PageTableFlags, Size4KiB};

    const FLAGS: u32 = MAP_FIXED | MAP_PRIVATE | MAP_ANONYMOUS;
    const BASE: u64 = 0x0000_0020_0000_0000;
    const STRIDE: u64 = 0x10_000;
    const PAGE_SIZE: u64 = 4096;
    const OLD_VALUE: u64 = 0x4d4d_3244_4f4c_4421;
    const NEW_VALUE: u64 = 0x4d4d_3244_4e45_5721;

    fn map_fixed(
        sched: &mut crate::sched::Scheduler,
        pmm: &mut crate::memory::pmm::PhysicalMemoryManager,
        address: u64,
        pages: u64,
    ) {
        assert_eq!(
            crate::process::mmap::sys_mmap(
                address,
                pages * 4096,
                PROT_READ | PROT_WRITE,
                FLAGS,
                -1,
                0,
                pmm,
                sched,
            ),
            Ok(address)
        );
    }

    fn mapped_entry(
        sched: &crate::sched::Scheduler,
        address: u64,
        hhdm: VirtAddr,
    ) -> Option<(x86_64::PhysAddr, PageTableFlags)> {
        let page = Page::<Size4KiB>::from_start_address(VirtAddr::new(address)).ok()?;
        unsafe {
            sched
                .current_process()
                .address_space
                .lookup_entry(page, hhdm)
        }
    }

    fn assert_consistent(sched: &crate::sched::Scheduler, hhdm: VirtAddr) {
        assert!(unsafe {
            sched
                .current_process()
                .address_space
                .validate_ledger_ptes(hhdm)
        });
    }

    let baseline = pmm.free_page_count();
    let process = unsafe { crate::process::Process::new(0xB2D0, 0, "mm2d-munmap", pmm, hhdm) };
    let mut sched = crate::sched::Scheduler::new();
    sched.processes.push(process);

    // Full, prefix, suffix, and middle removal share one P1 table so PMM
    // deltas reflect exactly the released anonymous data frames.
    for index in 0..4 {
        map_fixed(&mut sched, pmm, BASE + index * STRIDE, 4);
    }

    let free_before = pmm.free_page_count();
    assert_eq!(
        crate::process::mmap::sys_munmap(BASE, 4 * PAGE_SIZE, pmm, &mut sched),
        Ok(())
    );
    assert_eq!(pmm.free_page_count(), free_before + 4);
    assert!(mapped_entry(&sched, BASE, hhdm).is_none());
    assert!(sched
        .current_process()
        .address_space
        .lookup_region(BASE)
        .is_none());
    assert_consistent(&sched, hhdm);

    let prefix = BASE + STRIDE;
    assert_eq!(
        crate::process::mmap::sys_munmap(prefix, PAGE_SIZE, pmm, &mut sched),
        Ok(())
    );
    assert_eq!(
        sched
            .current_process()
            .address_space
            .lookup_region(prefix + PAGE_SIZE)
            .unwrap()
            .start,
        prefix + PAGE_SIZE
    );
    assert_consistent(&sched, hhdm);

    let suffix = BASE + 2 * STRIDE;
    assert_eq!(
        crate::process::mmap::sys_munmap(suffix + 3 * PAGE_SIZE, PAGE_SIZE, pmm, &mut sched,),
        Ok(())
    );
    assert_eq!(
        sched
            .current_process()
            .address_space
            .lookup_region(suffix)
            .unwrap()
            .end,
        suffix + 3 * PAGE_SIZE
    );
    assert_consistent(&sched, hhdm);

    let middle = BASE + 3 * STRIDE;
    assert_eq!(
        crate::process::mmap::sys_munmap(middle + PAGE_SIZE, 2 * PAGE_SIZE, pmm, &mut sched,),
        Ok(())
    );
    assert!(sched
        .current_process()
        .address_space
        .lookup_region(middle)
        .is_some());
    assert!(sched
        .current_process()
        .address_space
        .lookup_region(middle + PAGE_SIZE)
        .is_none());
    assert!(sched
        .current_process()
        .address_space
        .lookup_region(middle + 3 * PAGE_SIZE)
        .is_some());
    assert_consistent(&sched, hhdm);
    crate::serial_println!("[MM-2D] full/prefix/suffix/middle + exact PMM release: OK");

    // Holes are safe no-ops, including a request containing a hole followed by
    // an anonymous mapping.
    let mixed = BASE + 4 * STRIDE + PAGE_SIZE;
    map_fixed(&mut sched, pmm, mixed, 1);
    assert_eq!(
        crate::process::mmap::sys_munmap(mixed - PAGE_SIZE, 2 * PAGE_SIZE, pmm, &mut sched),
        Ok(())
    );
    assert_eq!(
        crate::process::mmap::sys_munmap(BASE + 5 * STRIDE, 3 * PAGE_SIZE, pmm, &mut sched),
        Ok(())
    );
    assert_consistent(&sched, hhdm);
    crate::serial_println!("[MM-2D] hole-only and mixed hole/anonymous ranges: OK");

    // Every non-anonymous kind remains protected. The first request includes
    // a real anonymous PTE before the protected record, proving atomic reject.
    let protected_kinds = [
        MappingKind::ElfSegment,
        MappingKind::UserStack,
        MappingKind::Brk,
        MappingKind::Framebuffer,
        MappingKind::Telemetry,
        MappingKind::BootSharedData,
        MappingKind::InternalUserMapping,
    ];
    for (index, kind) in protected_kinds.into_iter().enumerate() {
        let protected = BASE + 0x20_0000 + index as u64 * 4 * PAGE_SIZE;
        map_fixed(&mut sched, pmm, protected - PAGE_SIZE, 1);
        let record = MappingRegion::new(
            protected,
            protected + PAGE_SIZE,
            RegionProtection::READ_ONLY,
            kind,
            RegionPolicy::SYSTEM.union(RegionPolicy::OWNER_MANAGED),
            RegionBacking::Internal(0x2D00 + index as u64),
        )
        .unwrap();
        let reservation = sched
            .current_process()
            .address_space
            .preflight_region(record)
            .unwrap();
        sched
            .current_process()
            .address_space
            .commit_region(reservation)
            .unwrap();
        let before = mapped_entry(&sched, protected - PAGE_SIZE, hhdm);
        assert_eq!(
            crate::process::mmap::sys_munmap(protected - PAGE_SIZE, 2 * PAGE_SIZE, pmm, &mut sched,),
            Err(MmapError::Protected)
        );
        assert_eq!(mapped_entry(&sched, protected - PAGE_SIZE, hhdm), before);
        assert_eq!(
            sched
                .current_process()
                .address_space
                .lookup_region(protected),
            Some(record)
        );
        sched
            .current_process()
            .address_space
            .remove_region_exact(record)
            .unwrap();
        assert_eq!(
            crate::process::mmap::sys_munmap(protected - PAGE_SIZE, PAGE_SIZE, pmm, &mut sched,),
            Ok(())
        );
    }

    let mut caps = crate::capability::CapabilityBroker::new();
    let (shm_address, token) = crate::memory::shared::alloc_shared_region(
        sched.current_process_mut(),
        pmm,
        &mut caps,
        hhdm,
        PAGE_SIZE as usize,
    )
    .unwrap();
    let shm_count = caps.shared_region_map_count(token);
    let shm_entry = mapped_entry(&sched, shm_address.as_u64(), hhdm);
    assert_eq!(
        crate::process::mmap::sys_munmap(shm_address.as_u64(), PAGE_SIZE, pmm, &mut sched,),
        Err(MmapError::Protected)
    );
    assert_eq!(caps.shared_region_map_count(token), shm_count);
    assert_eq!(mapped_entry(&sched, shm_address.as_u64(), hhdm), shm_entry);
    crate::serial_println!("[MM-2D] ELF/stack/brk/SHM/device/internal policy rejection: OK");

    // Invalid native inputs map to EINVAL for Helios, protected policy to
    // EACCES, and split capacity exhaustion to ENOMEM.
    assert_eq!(
        crate::process::mmap::sys_munmap(BASE + 1, PAGE_SIZE, pmm, &mut sched),
        Err(MmapError::InvalidAddress)
    );
    assert_eq!(
        crate::process::mmap::sys_munmap(BASE, 0, pmm, &mut sched),
        Err(MmapError::InvalidAddress)
    );
    assert_eq!(
        crate::process::mmap::sys_munmap(u64::MAX - 0xfff, 0x2000, pmm, &mut sched),
        Err(MmapError::InvalidAddress)
    );
    assert_eq!(
        crate::process::mmap::sys_munmap(
            crate::memory::user::USER_END_EXCLUSIVE - PAGE_SIZE,
            2 * PAGE_SIZE,
            pmm,
            &mut sched,
        ),
        Err(MmapError::InvalidAddress)
    );
    assert_eq!(
        crate::process::mmap::munmap_linux_errno(MmapError::InvalidAddress),
        22
    );
    assert_eq!(
        crate::process::mmap::munmap_linux_errno(MmapError::Protected),
        13
    );
    assert_eq!(
        crate::process::mmap::munmap_linux_errno(MmapError::NoMemory),
        12
    );
    crate::serial_println!("[MM-2D] native failure convention and Helios errno classes: OK");

    // Fill the bounded ledger around a real three-page anonymous mapping. A
    // middle split must fail before its PTE or ledger record changes.
    let capacity_target = BASE + 0x40_0000;
    map_fixed(&mut sched, pmm, capacity_target, 3);
    let capacity_entry = mapped_entry(&sched, capacity_target + PAGE_SIZE, hhdm);
    let mut fillers = alloc::vec::Vec::new();
    let mut filler_index = 0u64;
    while sched.current_process().address_space.region_count() < MAX_REGIONS_PER_ADDRESS_SPACE {
        let start = BASE + 0x1000_0000 + filler_index * 2 * PAGE_SIZE;
        filler_index += 1;
        let filler = MappingRegion::new(
            start,
            start + PAGE_SIZE,
            RegionProtection::READ_ONLY,
            MappingKind::InternalUserMapping,
            RegionPolicy::SYSTEM,
            RegionBacking::Internal(0xCA00 + filler_index),
        )
        .unwrap();
        let reservation = sched
            .current_process()
            .address_space
            .preflight_region(filler)
            .unwrap();
        sched
            .current_process()
            .address_space
            .commit_region(reservation)
            .unwrap();
        fillers.push(filler);
    }
    assert_eq!(
        crate::process::mmap::sys_munmap(capacity_target + PAGE_SIZE, PAGE_SIZE, pmm, &mut sched,),
        Err(MmapError::NoMemory)
    );
    assert_eq!(
        mapped_entry(&sched, capacity_target + PAGE_SIZE, hhdm),
        capacity_entry
    );
    assert_eq!(
        sched
            .current_process()
            .address_space
            .lookup_region(capacity_target)
            .unwrap()
            .end,
        capacity_target + 3 * PAGE_SIZE
    );
    for filler in fillers {
        sched
            .current_process()
            .address_space
            .remove_region_exact(filler)
            .unwrap();
    }
    assert_eq!(
        crate::process::mmap::sys_munmap(capacity_target, 3 * PAGE_SIZE, pmm, &mut sched,),
        Ok(())
    );
    crate::serial_println!("[MM-2D] middle-split capacity rejection is atomic: OK");

    // Swapped leaves are removed directly: no swap-in, one ZRAM discard, no
    // stale PTE or candidate frame.
    let swapped = BASE + 0x50_0000;
    map_fixed(&mut sched, pmm, swapped, 1);
    let swapped_page = Page::<Size4KiB>::from_start_address(VirtAddr::new(swapped)).unwrap();
    let (swapped_frame, _) = mapped_entry(&sched, swapped, hhdm).unwrap();
    let block_id = unsafe {
        crate::memory::swap::swap_out_page(
            &mut sched.current_process_mut().address_space,
            swapped_page,
            swapped_frame,
            hhdm,
            pmm,
        )
    }
    .unwrap();
    assert!(crate::memory::zram::block_exists(block_id));
    assert_eq!(
        crate::process::mmap::sys_munmap(swapped, PAGE_SIZE, pmm, &mut sched),
        Ok(())
    );
    assert!(!crate::memory::zram::block_exists(block_id));
    assert!(mapped_entry(&sched, swapped, hhdm).is_none());
    assert_consistent(&sched, hhdm);
    crate::serial_println!("[MM-2D] swapped anonymous page released without swap-in: OK");

    // Freed fixed ranges are reusable. All online CPUs pre-touch the old
    // translation; munmap must acknowledge their shootdowns before the old
    // frame is reused as a guard and a different frame is remapped at the VA.
    let remote = BASE + 0x70_000;
    map_fixed(&mut sched, pmm, remote, 1);
    let old_frame = mapped_entry(&sched, remote, hhdm).unwrap().0;
    unsafe {
        (hhdm + old_frame.as_u64())
            .as_mut_ptr::<u64>()
            .write_volatile(OLD_VALUE);
        sched.current_process().address_space.activate();
    }
    assert_eq!(unsafe { (remote as *const u64).read_volatile() }, OLD_VALUE);
    let online = crate::sched::ONLINE_CORES.load(Ordering::Acquire);
    let local_cpu = crate::sched::current_cpu_id();
    let remote_cpus = ((1u64 << online) - 1) & !(1u64 << local_cpu);
    crate::memory::tlb::test_activate_and_read(
        sched.current_process().address_space.identity(),
        remote,
        remote_cpus,
    );
    for cpu_id in 0..online {
        if cpu_id != local_cpu {
            assert_eq!(crate::memory::tlb::test_result(cpu_id), OLD_VALUE);
        }
    }
    assert_eq!(
        crate::process::mmap::sys_munmap(remote, PAGE_SIZE, pmm, &mut sched),
        Ok(())
    );
    let guard = pmm.alloc_frame().unwrap();
    assert_eq!(guard, old_frame, "MM-2D gate expects immediate frame reuse");
    map_fixed(&mut sched, pmm, remote, 1);
    let new_frame = mapped_entry(&sched, remote, hhdm).unwrap().0;
    assert_ne!(new_frame, old_frame);
    unsafe {
        (hhdm + new_frame.as_u64())
            .as_mut_ptr::<u64>()
            .write_volatile(NEW_VALUE);
    }
    assert_eq!(unsafe { (remote as *const u64).read_volatile() }, NEW_VALUE);
    crate::memory::tlb::test_read(
        sched.current_process().address_space.identity(),
        remote,
        remote_cpus,
    );
    for cpu_id in 0..online {
        if cpu_id != local_cpu {
            assert_eq!(crate::memory::tlb::test_result(cpu_id), NEW_VALUE);
        }
    }
    crate::memory::tlb::test_leave(remote_cpus);
    unsafe {
        crate::memory::tlb::activate_kernel_root();
    }
    pmm.free_frame(guard);
    crate::serial_println!("[MM-2D] all CPUs dropped translation before frame reuse: OK");

    crate::memory::shared::cleanup_shared_pages(sched.current_process_mut(), pmm, &mut caps);
    unsafe {
        sched
            .current_process_mut()
            .address_space
            .reclaim_user_space(pmm, hhdm, true);
    }
    assert_eq!(pmm.free_page_count(), baseline);
    crate::process::mmap::diagnostic_report();
    crate::serial_println!("[MM-2D] focused munmap/shootdown gate: OK");
}

#[cfg(feature = "mm2e_mprotect_test")]
pub fn run_mm2e_mprotect_gate(pmm: &mut crate::memory::pmm::PhysicalMemoryManager, hhdm: VirtAddr) {
    use crate::memory::user::UserMemoryError;
    use crate::process::mmap::{
        MmapError, MAP_ANONYMOUS, MAP_FIXED, MAP_PRIVATE, PROT_EXEC, PROT_NONE, PROT_READ,
        PROT_WRITE,
    };
    use crate::process::region::{
        MappingKind, MappingRegion, RegionBacking, RegionPolicy, RegionProtection,
        MAX_REGIONS_PER_ADDRESS_SPACE,
    };
    use x86_64::structures::paging::{Page, PageTableFlags, Size4KiB};

    const FLAGS: u32 = MAP_FIXED | MAP_PRIVATE | MAP_ANONYMOUS;
    const BASE: u64 = 0x0000_0028_0000_0000;
    const STRIDE: u64 = 0x20_000;
    const PAGE_SIZE: u64 = 4096;
    const VALUE: u64 = 0x4d4d_3245_5052_4f54;

    fn map_fixed(
        sched: &mut crate::sched::Scheduler,
        pmm: &mut crate::memory::pmm::PhysicalMemoryManager,
        address: u64,
        pages: u64,
        prot: u32,
    ) {
        assert_eq!(
            crate::process::mmap::sys_mmap(
                address,
                pages * PAGE_SIZE,
                prot,
                FLAGS,
                -1,
                0,
                pmm,
                sched,
            ),
            Ok(address)
        );
    }

    fn entry(
        sched: &crate::sched::Scheduler,
        address: u64,
        hhdm: VirtAddr,
    ) -> (x86_64::PhysAddr, PageTableFlags) {
        let page = Page::<Size4KiB>::from_start_address(VirtAddr::new(address)).unwrap();
        unsafe {
            sched
                .current_process()
                .address_space
                .lookup_entry(page, hhdm)
                .unwrap()
        }
    }

    fn protect(
        sched: &mut crate::sched::Scheduler,
        pmm: &crate::memory::pmm::PhysicalMemoryManager,
        address: u64,
        pages: u64,
        prot: u32,
    ) -> Result<(), MmapError> {
        crate::process::mmap::sys_mprotect(address, pages * PAGE_SIZE, prot, pmm, sched)
    }

    fn assert_protection(
        sched: &crate::sched::Scheduler,
        address: u64,
        hhdm: VirtAddr,
        writable: bool,
        executable: bool,
    ) -> x86_64::PhysAddr {
        let (frame, flags) = entry(sched, address, hhdm);
        assert_eq!(flags.contains(PageTableFlags::WRITABLE), writable);
        assert_eq!(!flags.contains(PageTableFlags::NO_EXECUTE), executable);
        assert!(flags.contains(PageTableFlags::PRESENT));
        assert!(flags.contains(PageTableFlags::USER_ACCESSIBLE));
        assert!(!(writable && executable));
        frame
    }

    let baseline = pmm.free_page_count();
    let process = unsafe { crate::process::Process::new(0xB2E0, 0, "mm2e-mprotect", pmm, hhdm) };
    let mut sched = crate::sched::Scheduler::new();
    sched.processes.push(process);

    // All six real transitions, same-protection no-op, content/frame
    // preservation, and MM-1 copy permission integration.
    let transitions = BASE;
    map_fixed(&mut sched, pmm, transitions, 2, PROT_READ | PROT_WRITE);
    let free_before = pmm.free_page_count();
    let original_frame = assert_protection(&sched, transitions, hhdm, true, false);
    crate::memory::user::copy_to_process_bytes(
        sched.current_process(),
        hhdm,
        transitions,
        &VALUE.to_ne_bytes(),
    )
    .unwrap();
    assert_eq!(protect(&mut sched, pmm, transitions, 1, PROT_READ), Ok(()));
    assert_eq!(
        crate::memory::user::copy_to_process_bytes(
            sched.current_process(),
            hhdm,
            transitions,
            b"x",
        ),
        Err(UserMemoryError::NotWritable)
    );
    let mut contents = [0u8; 8];
    crate::memory::user::copy_from_process_bytes(
        sched.current_process(),
        hhdm,
        transitions,
        &mut contents,
    )
    .unwrap();
    assert_eq!(contents, VALUE.to_ne_bytes());
    assert_eq!(
        original_frame,
        assert_protection(&sched, transitions, hhdm, false, false)
    );
    let before_noop = entry(&sched, transitions, hhdm);
    assert_eq!(protect(&mut sched, pmm, transitions, 1, PROT_READ), Ok(()));
    assert_eq!(entry(&sched, transitions, hhdm), before_noop);
    assert_eq!(protect(&mut sched, pmm, transitions, 1, PROT_WRITE), Ok(()));
    assert_eq!(
        original_frame,
        assert_protection(&sched, transitions, hhdm, true, false)
    );
    crate::memory::user::copy_to_process_bytes(sched.current_process(), hhdm, transitions, b"y")
        .unwrap();
    assert_eq!(protect(&mut sched, pmm, transitions, 1, PROT_EXEC), Ok(()));
    assert_eq!(
        original_frame,
        assert_protection(&sched, transitions, hhdm, false, true)
    );
    assert_eq!(protect(&mut sched, pmm, transitions, 1, PROT_WRITE), Ok(()));
    assert_eq!(
        original_frame,
        assert_protection(&sched, transitions, hhdm, true, false)
    );
    assert_eq!(protect(&mut sched, pmm, transitions, 1, PROT_READ), Ok(()));
    assert_eq!(
        protect(&mut sched, pmm, transitions, 1, PROT_READ | PROT_EXEC),
        Ok(())
    );
    assert_eq!(protect(&mut sched, pmm, transitions, 1, PROT_READ), Ok(()));
    assert_eq!(
        protect(&mut sched, pmm, transitions, 1, PROT_READ | PROT_EXEC),
        Ok(())
    );
    assert_eq!(protect(&mut sched, pmm, transitions, 1, PROT_WRITE), Ok(()));
    assert_eq!(pmm.free_page_count(), free_before);
    crate::serial_println!("[MM-2E] R/RW/RX transitions, frames, contents, and MM-1 copies: OK");

    // Middle/prefix/suffix splits merge back to one compatible record.
    let splits = BASE + STRIDE;
    map_fixed(&mut sched, pmm, splits, 5, PROT_READ | PROT_WRITE);
    assert_eq!(
        protect(&mut sched, pmm, splits + 2 * PAGE_SIZE, 1, PROT_READ),
        Ok(())
    );
    let region_count = sched.current_process().address_space.region_count();
    assert_eq!(protect(&mut sched, pmm, splits, 2, PROT_READ), Ok(()));
    assert_eq!(
        protect(&mut sched, pmm, splits + 3 * PAGE_SIZE, 2, PROT_READ),
        Ok(())
    );
    assert_eq!(
        sched.current_process().address_space.region_count(),
        region_count - 2
    );
    assert_eq!(protect(&mut sched, pmm, splits, 5, PROT_WRITE), Ok(()));
    let merged = sched
        .current_process()
        .address_space
        .lookup_region(splits)
        .unwrap();
    assert_eq!(merged.start, splits);
    assert_eq!(merged.end, splits + 5 * PAGE_SIZE);
    assert_eq!(merged.protection, RegionProtection::READ_WRITE);
    crate::serial_println!("[MM-2E] middle/prefix/suffix splits and compatible merge: OK");

    // Holes, protected kinds, invalid flags, PROT_NONE, zero length, and
    // ledger-capacity exhaustion all reject before PTE mutation.
    let hole = BASE + 2 * STRIDE;
    map_fixed(&mut sched, pmm, hole, 1, PROT_READ | PROT_WRITE);
    map_fixed(
        &mut sched,
        pmm,
        hole + 2 * PAGE_SIZE,
        1,
        PROT_READ | PROT_WRITE,
    );
    let hole_before = entry(&sched, hole, hhdm);
    assert_eq!(
        protect(&mut sched, pmm, hole, 3, PROT_READ),
        Err(MmapError::NoMemory)
    );
    assert_eq!(entry(&sched, hole, hhdm), hole_before);

    for (index, kind) in [
        MappingKind::ElfSegment,
        MappingKind::UserStack,
        MappingKind::Brk,
        MappingKind::SharedMemory,
        MappingKind::Framebuffer,
        MappingKind::Telemetry,
        MappingKind::BootSharedData,
        MappingKind::InternalUserMapping,
    ]
    .into_iter()
    .enumerate()
    {
        let protected = BASE + 3 * STRIDE + index as u64 * 3 * PAGE_SIZE;
        map_fixed(
            &mut sched,
            pmm,
            protected - PAGE_SIZE,
            1,
            PROT_READ | PROT_WRITE,
        );
        let record = MappingRegion::new(
            protected,
            protected + PAGE_SIZE,
            RegionProtection::READ_ONLY,
            kind,
            RegionPolicy::SYSTEM.union(RegionPolicy::OWNER_MANAGED),
            if kind == MappingKind::SharedMemory {
                RegionBacking::SharedMemory(0x2E00 + index as u64)
            } else {
                RegionBacking::Internal(0x2E00 + index as u64)
            },
        )
        .unwrap();
        let reservation = sched
            .current_process()
            .address_space
            .preflight_region(record)
            .unwrap();
        sched
            .current_process()
            .address_space
            .commit_region(reservation)
            .unwrap();
        let before = entry(&sched, protected - PAGE_SIZE, hhdm);
        assert_eq!(
            protect(&mut sched, pmm, protected - PAGE_SIZE, 2, PROT_READ),
            Err(MmapError::Protected)
        );
        assert_eq!(entry(&sched, protected - PAGE_SIZE, hhdm), before);
        sched
            .current_process()
            .address_space
            .remove_region_exact(record)
            .unwrap();
        crate::process::mmap::sys_munmap(protected - PAGE_SIZE, PAGE_SIZE, pmm, &mut sched)
            .unwrap();
    }
    assert_eq!(
        protect(&mut sched, pmm, transitions, 1, PROT_WRITE | PROT_EXEC),
        Err(MmapError::PermissionDenied)
    );
    assert_eq!(
        protect(&mut sched, pmm, transitions, 1, PROT_NONE),
        Err(MmapError::Unsupported)
    );
    assert_eq!(
        crate::process::mmap::sys_mprotect(transitions, 0, PROT_READ, pmm, &mut sched),
        Ok(())
    );
    assert_eq!(
        crate::process::mmap::sys_mprotect(transitions + 1, 0, PROT_READ, pmm, &mut sched),
        Err(MmapError::InvalidAddress)
    );
    assert_eq!(
        crate::process::mmap::mprotect_linux_errno(MmapError::Protected),
        13
    );
    assert_eq!(
        crate::process::mmap::mprotect_linux_errno(MmapError::NoMemory),
        12
    );
    assert_eq!(
        crate::process::mmap::mprotect_linux_errno(MmapError::InvalidProt),
        22
    );
    assert_eq!(
        crate::process::mmap::mprotect_linux_errno(MmapError::SwappedUnsupported),
        95
    );

    let capacity = BASE + 0x80_0000;
    map_fixed(&mut sched, pmm, capacity, 3, PROT_READ | PROT_WRITE);
    let capacity_before = entry(&sched, capacity + PAGE_SIZE, hhdm);
    let mut fillers = alloc::vec::Vec::new();
    let mut filler_index = 0u64;
    while sched.current_process().address_space.region_count() < MAX_REGIONS_PER_ADDRESS_SPACE {
        let start = BASE + 0x1000_0000 + filler_index * 2 * PAGE_SIZE;
        filler_index += 1;
        let filler = MappingRegion::new(
            start,
            start + PAGE_SIZE,
            RegionProtection::READ_ONLY,
            MappingKind::InternalUserMapping,
            RegionPolicy::SYSTEM,
            RegionBacking::Internal(0xCE00 + filler_index),
        )
        .unwrap();
        let reservation = sched
            .current_process()
            .address_space
            .preflight_region(filler)
            .unwrap();
        sched
            .current_process()
            .address_space
            .commit_region(reservation)
            .unwrap();
        fillers.push(filler);
    }
    assert_eq!(
        protect(&mut sched, pmm, capacity + PAGE_SIZE, 1, PROT_READ),
        Err(MmapError::NoMemory)
    );
    assert_eq!(entry(&sched, capacity + PAGE_SIZE, hhdm), capacity_before);
    for filler in fillers {
        sched
            .current_process()
            .address_space
            .remove_region_exact(filler)
            .unwrap();
    }
    crate::serial_println!("[MM-2E] holes, policy, flags, errors, and capacity are atomic: OK");

    // Present-only policy rejects a swapped marker without swap-in.
    let swapped = BASE + 0x90_0000;
    map_fixed(&mut sched, pmm, swapped, 1, PROT_READ | PROT_WRITE);
    let swapped_page = Page::<Size4KiB>::from_start_address(VirtAddr::new(swapped)).unwrap();
    let swapped_frame = entry(&sched, swapped, hhdm).0;
    let block_id = unsafe {
        crate::memory::swap::swap_out_page(
            &mut sched.current_process_mut().address_space,
            swapped_page,
            swapped_frame,
            hhdm,
            pmm,
        )
    }
    .unwrap();
    assert_eq!(
        protect(&mut sched, pmm, swapped, 1, PROT_READ),
        Err(MmapError::SwappedUnsupported)
    );
    assert!(crate::memory::zram::block_exists(block_id));
    crate::process::mmap::sys_munmap(swapped, PAGE_SIZE, pmm, &mut sched).unwrap();
    crate::serial_println!("[MM-2E] swapped anonymous page rejected without swap-in: OK");

    // Four active CPUs pre-touch the mapping. Each protection reduction and
    // increase must synchronously run the MM-2B remote invalidation handler.
    let remote = BASE + 0xA0_0000;
    map_fixed(&mut sched, pmm, remote, 1, PROT_READ | PROT_WRITE);
    let remote_frame = entry(&sched, remote, hhdm).0;
    unsafe {
        (hhdm + remote_frame.as_u64())
            .as_mut_ptr::<u64>()
            .write_volatile(VALUE);
        sched.current_process().address_space.activate();
    }
    assert_eq!(unsafe { (remote as *const u64).read_volatile() }, VALUE);
    let online = crate::sched::ONLINE_CORES.load(Ordering::Acquire);
    assert_eq!(online, 4, "MM-2E gate requires exactly four online CPUs");
    let local_cpu = crate::sched::current_cpu_id();
    let remote_cpus = ((1u64 << online) - 1) & !(1u64 << local_cpu);
    crate::memory::tlb::test_activate_and_read(
        sched.current_process().address_space.identity(),
        remote,
        remote_cpus,
    );
    let invalidations_before = crate::memory::tlb::test_remote_invalidation_count();
    assert_eq!(protect(&mut sched, pmm, remote, 1, PROT_READ), Ok(()));
    assert_eq!(
        protect(&mut sched, pmm, remote, 1, PROT_READ | PROT_EXEC),
        Ok(())
    );
    let invalidations_after = crate::memory::tlb::test_remote_invalidation_count();
    assert!(invalidations_after - invalidations_before >= remote_cpus.count_ones() as u64 * 2);
    crate::memory::tlb::test_read(
        sched.current_process().address_space.identity(),
        remote,
        remote_cpus,
    );
    for cpu_id in 0..online {
        if cpu_id != local_cpu {
            assert_eq!(crate::memory::tlb::test_result(cpu_id), VALUE);
        }
    }
    crate::memory::tlb::test_leave(remote_cpus);
    unsafe {
        crate::memory::tlb::activate_kernel_root();
    }
    crate::serial_println!("[MM-2E] four CPUs acknowledged writable/NX permission shootdowns: OK");

    // MM-2D remains composable after protection fragmentation, and fixed
    // remapping followed by protection works again.
    let remap = BASE + 0xB0_0000;
    map_fixed(&mut sched, pmm, remap, 4, PROT_READ | PROT_WRITE);
    protect(&mut sched, pmm, remap + PAGE_SIZE, 2, PROT_READ).unwrap();
    crate::process::mmap::sys_munmap(remap, 4 * PAGE_SIZE, pmm, &mut sched).unwrap();
    map_fixed(&mut sched, pmm, remap, 4, PROT_READ | PROT_WRITE);
    protect(&mut sched, pmm, remap, 4, PROT_READ).unwrap();
    crate::process::mmap::sys_munmap(remap, 4 * PAGE_SIZE, pmm, &mut sched).unwrap();

    unsafe {
        sched
            .current_process_mut()
            .address_space
            .reclaim_user_space(pmm, hhdm, true);
    }
    assert_eq!(pmm.free_page_count(), baseline);
    crate::process::mmap::diagnostic_report();
    crate::memory::tlb::diagnostic_report();
    crate::serial_println!("[MM-2E] munmap/remap composition and PMM accounting: OK");
    crate::serial_println!("[MM-2E] focused mprotect/permission shootdown gate: OK");
}

fn run_mm2a_self_tests(pmm: &mut crate::memory::pmm::PhysicalMemoryManager, hhdm: VirtAddr) {
    use crate::process::address_space::{
        ExpectedMapping, MappingError, OwnershipTransition, ReplacementMapping,
    };
    use x86_64::structures::paging::{Page, PhysFrame, Size4KiB};

    let free_before = pmm.free_page_count();
    let mut collision_process =
        unsafe { crate::process::Process::new(0xB301, 0, "mm2a-map", pmm, hhdm) };
    let page = Page::<Size4KiB>::from_start_address(VirtAddr::new(0x0000_0002_2000_0000))
        .expect("MM-2A collision page");
    let original = pmm.alloc_frame().expect("MM-2A original frame");
    let rejected = pmm.alloc_frame().expect("MM-2A rejected frame");
    assert!(sanitize_user_frame(original, hhdm));
    assert!(sanitize_user_frame(rejected, hhdm));
    let original_flags = user_stack_flags();
    unsafe {
        collision_process
            .address_space
            .map_page(
                page,
                PhysFrame::from_start_address_unchecked(original),
                original_flags,
                pmm,
                hhdm,
            )
            .expect("MM-2A free page mapping");
        assert_eq!(
            collision_process.address_space.map_page(
                page,
                PhysFrame::from_start_address_unchecked(rejected),
                PageTableFlags::PRESENT
                    | PageTableFlags::USER_ACCESSIBLE
                    | PageTableFlags::NO_EXECUTE,
                pmm,
                hhdm,
            ),
            Err(MappingError::AlreadyMapped)
        );
        assert_eq!(
            collision_process.address_space.lookup_entry(page, hhdm),
            Some((original, original_flags))
        );
        collision_process
            .address_space
            .rollback_mapped_page(page, original, pmm, hhdm)
            .expect("MM-2A collision cleanup");
    }
    pmm.free_frame(original);
    pmm.free_frame(rejected);
    unsafe {
        collision_process
            .address_space
            .reclaim_user_space(pmm, hhdm, true);
    }
    assert_eq!(pmm.free_page_count(), free_before);

    let mut mmap_process =
        unsafe { crate::process::Process::new(0xB302, 0, "mm2a-mmap", pmm, hhdm) };
    const MMAP_BASE: u64 = 0x10_0000_0000;
    mmap_process.mmap_next = MMAP_BASE;
    let occupied_page = Page::<Size4KiB>::from_start_address(VirtAddr::new(MMAP_BASE + 4096))
        .expect("MM-2A mmap collision page");
    let occupied_frame = pmm.alloc_frame().expect("MM-2A mmap collision frame");
    assert!(sanitize_user_frame(occupied_frame, hhdm));
    unsafe {
        mmap_process
            .address_space
            .map_page(
                occupied_page,
                PhysFrame::from_start_address_unchecked(occupied_frame),
                original_flags,
                pmm,
                hhdm,
            )
            .expect("MM-2A mmap collision setup");
    }
    let mmap_free_before = pmm.free_page_count();
    let mut mmap_sched = crate::sched::Scheduler::new();
    mmap_sched.processes.push(mmap_process);
    assert_eq!(
        crate::process::mmap::sys_mmap(
            0,
            3 * 4096,
            crate::process::mmap::PROT_READ | crate::process::mmap::PROT_WRITE,
            crate::process::mmap::MAP_PRIVATE | crate::process::mmap::MAP_ANONYMOUS,
            -1,
            0,
            pmm,
            &mut mmap_sched,
        ),
        Err(crate::process::mmap::MmapError::AlreadyMapped)
    );
    assert_eq!(mmap_sched.processes[0].mmap_next, MMAP_BASE);
    assert_eq!(pmm.free_page_count(), mmap_free_before);
    assert_eq!(
        unsafe {
            mmap_sched.processes[0]
                .address_space
                .lookup_phys(occupied_page, hhdm)
        },
        Some(occupied_frame)
    );
    unsafe {
        mmap_sched.processes[0]
            .address_space
            .rollback_mapped_page(occupied_page, occupied_frame, pmm, hhdm)
            .expect("MM-2A mmap setup cleanup");
        mmap_sched.processes[0]
            .address_space
            .reclaim_user_space(pmm, hhdm, true);
    }
    pmm.free_frame(occupied_frame);
    assert_eq!(pmm.free_page_count(), free_before);

    let mut shm_owner =
        unsafe { crate::process::Process::new(0xB303, 0, "mm2a-shm-owner", pmm, hhdm) };
    let mut shm_peer =
        unsafe { crate::process::Process::new(0xB304, 0, "mm2a-shm-peer", pmm, hhdm) };
    let peer_collision_page = Page::<Size4KiB>::from_start_address(VirtAddr::new(
        crate::memory::shared::SHARED_REGION_BASE,
    ))
    .expect("MM-2A SHM collision page");
    let peer_collision_frame = pmm.alloc_frame().expect("MM-2A SHM collision frame");
    assert!(sanitize_user_frame(peer_collision_frame, hhdm));
    unsafe {
        shm_peer
            .address_space
            .map_page(
                peer_collision_page,
                PhysFrame::from_start_address_unchecked(peer_collision_frame),
                original_flags,
                pmm,
                hhdm,
            )
            .expect("MM-2A SHM collision setup");
    }
    let mut shm_caps = crate::capability::CapabilityBroker::new();
    let (_, token) =
        crate::memory::shared::alloc_shared_region(&mut shm_owner, pmm, &mut shm_caps, hhdm, 8192)
            .expect("MM-2A SHM owner map");
    assert_eq!(shm_caps.shared_region_count(), 1);
    assert_eq!(shm_caps.shared_region_map_count(token), Some(1));
    assert_eq!(
        crate::memory::shared::map_shared_page(&mut shm_peer, token, pmm, &mut shm_caps, hhdm,),
        Err(crate::memory::shared::SharedMemError::AlreadyMapped)
    );
    assert_eq!(shm_caps.shared_region_map_count(token), Some(1));
    assert_eq!(
        unsafe {
            shm_peer
                .address_space
                .lookup_phys(peer_collision_page, hhdm)
        },
        Some(peer_collision_frame)
    );
    unsafe {
        shm_peer
            .address_space
            .rollback_mapped_page(peer_collision_page, peer_collision_frame, pmm, hhdm)
            .expect("MM-2A SHM collision cleanup");
    }
    pmm.free_frame(peer_collision_frame);
    let object_first_frame = shm_caps
        .resolve_shared_region(token)
        .expect("MM-2A SHM object")
        .frames[0]
        .start_address();
    unsafe {
        (hhdm + object_first_frame.as_u64())
            .as_mut_ptr::<u8>()
            .write_volatile(0xA7);
    }
    crate::memory::shared::map_shared_page(&mut shm_peer, token, pmm, &mut shm_caps, hhdm)
        .expect("MM-2A successful SHM peer map");
    assert_eq!(shm_caps.shared_region_map_count(token), Some(2));
    assert_eq!(
        unsafe {
            (hhdm + object_first_frame.as_u64())
                .as_ptr::<u8>()
                .read_volatile()
        },
        0xA7
    );
    crate::memory::shared::cleanup_shared_pages(&mut shm_peer, pmm, &mut shm_caps);
    crate::memory::shared::cleanup_shared_pages(&mut shm_owner, pmm, &mut shm_caps);
    unsafe {
        shm_peer.address_space.reclaim_user_space(pmm, hhdm, true);
        shm_owner.address_space.reclaim_user_space(pmm, hhdm, true);
    }
    assert_eq!(pmm.free_page_count(), free_before);

    let mut replacement_process =
        unsafe { crate::process::Process::new(0xB305, 0, "mm2a-replace", pmm, hhdm) };
    let replacement_page =
        Page::<Size4KiB>::from_start_address(VirtAddr::new(0x0000_0002_3000_0000))
            .expect("MM-2A replacement page");
    let old_frame = pmm.alloc_frame().expect("MM-2A old replacement frame");
    let new_frame = pmm.alloc_frame().expect("MM-2A new replacement frame");
    assert!(sanitize_user_frame(old_frame, hhdm));
    assert!(sanitize_user_frame(new_frame, hhdm));
    let old_flags =
        PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE | PageTableFlags::NO_EXECUTE;
    let new_flags = old_flags | PageTableFlags::WRITABLE;
    unsafe {
        replacement_process
            .address_space
            .map_page(
                replacement_page,
                PhysFrame::from_start_address_unchecked(old_frame),
                old_flags,
                pmm,
                hhdm,
            )
            .expect("MM-2A replacement setup");
        replacement_process
            .address_space
            .replace_mapping(
                replacement_page,
                ExpectedMapping::Present {
                    frame: old_frame,
                    flags: old_flags,
                },
                ReplacementMapping::Present {
                    frame: PhysFrame::from_start_address_unchecked(new_frame),
                    flags: new_flags,
                },
                OwnershipTransition::ReleaseOldFrame,
                pmm,
                hhdm,
            )
            .expect("MM-2A explicit replacement");
        assert_eq!(
            replacement_process
                .address_space
                .lookup_entry(replacement_page, hhdm),
            Some((new_frame, new_flags))
        );
        replacement_process
            .address_space
            .rollback_mapped_page(replacement_page, new_frame, pmm, hhdm)
            .expect("MM-2A replacement cleanup");
        replacement_process
            .address_space
            .reclaim_user_space(pmm, hhdm, true);
    }
    pmm.free_frame(new_frame);
    assert_eq!(pmm.free_page_count(), free_before);
    #[cfg(feature = "mm2a_test_injection")]
    run_mm2a_injected_failure_tests(pmm, hhdm);
    crate::serial_println!(
        "[MM-2A] collision, cursor, SHM commit, live-content, and NX replacement: OK"
    );
}

#[cfg(feature = "mm2a_test_injection")]
fn run_mm2a_injected_failure_tests(
    pmm: &mut crate::memory::pmm::PhysicalMemoryManager,
    hhdm: VirtAddr,
) {
    let free_before = pmm.free_page_count();

    // One user frame plus three page-table frames succeed; the second user
    // frame then fails after the first page is visible and must be rolled back.
    let mmap_process =
        unsafe { crate::process::Process::new(0xB311, 0, "mm2a-mmap-oom", pmm, hhdm) };
    let mut mmap_sched = crate::sched::Scheduler::new();
    mmap_sched.processes.push(mmap_process);
    let mmap_free_before = pmm.free_page_count();
    pmm.fail_test_allocations_after(4);
    let mmap_result = crate::process::mmap::sys_mmap(
        0,
        8192,
        crate::process::mmap::PROT_READ | crate::process::mmap::PROT_WRITE,
        crate::process::mmap::MAP_PRIVATE | crate::process::mmap::MAP_ANONYMOUS,
        -1,
        0,
        pmm,
        &mut mmap_sched,
    );
    pmm.clear_test_allocation_failure();
    assert_eq!(mmap_result, Err(crate::process::mmap::MmapError::NoMemory));
    assert_eq!(mmap_sched.processes[0].mmap_next, 0);
    assert_eq!(pmm.free_page_count(), mmap_free_before);
    unsafe {
        mmap_sched.processes[0]
            .address_space
            .reclaim_user_space(pmm, hhdm, true);
    }
    assert_eq!(pmm.free_page_count(), free_before);

    // A first-page page-table allocation failure must also return normally,
    // free the already allocated user frame, and keep the cursor untouched.
    let pt_process = unsafe { crate::process::Process::new(0xB312, 0, "mm2a-pt-oom", pmm, hhdm) };
    let mut pt_sched = crate::sched::Scheduler::new();
    pt_sched.processes.push(pt_process);
    let pt_free_before = pmm.free_page_count();
    pmm.fail_test_allocations_after(1);
    let pt_result = crate::process::mmap::sys_mmap(
        0,
        4096,
        crate::process::mmap::PROT_READ | crate::process::mmap::PROT_WRITE,
        crate::process::mmap::MAP_PRIVATE | crate::process::mmap::MAP_ANONYMOUS,
        -1,
        0,
        pmm,
        &mut pt_sched,
    );
    pmm.clear_test_allocation_failure();
    assert_eq!(pt_result, Err(crate::process::mmap::MmapError::NoMemory));
    assert_eq!(pt_sched.processes[0].mmap_next, 0);
    assert_eq!(pmm.free_page_count(), pt_free_before);
    unsafe {
        pt_sched.processes[0]
            .address_space
            .reclaim_user_space(pmm, hhdm, true);
    }
    assert_eq!(pmm.free_page_count(), free_before);

    let mut shm_process =
        unsafe { crate::process::Process::new(0xB313, 0, "mm2a-shm-oom", pmm, hhdm) };
    let mut shm_caps = crate::capability::CapabilityBroker::new();
    let shm_free_before = pmm.free_page_count();
    pmm.fail_test_allocations_after(1);
    let shm_result = crate::memory::shared::alloc_shared_region(
        &mut shm_process,
        pmm,
        &mut shm_caps,
        hhdm,
        3 * 4096,
    );
    pmm.clear_test_allocation_failure();
    assert_eq!(
        shm_result,
        Err(crate::memory::shared::SharedMemError::OutOfMemory)
    );
    assert_eq!(shm_caps.shared_region_count(), 0);
    assert!(shm_process.owned_shared.is_empty());
    assert!(shm_process.mapped_shared.is_empty());
    assert_eq!(pmm.free_page_count(), shm_free_before);

    // Both SHM frames allocate, then owner page-table allocation fails. No
    // object/token is published and all backing frames are returned.
    pmm.fail_test_allocations_after(2);
    let shm_map_result = crate::memory::shared::alloc_shared_region(
        &mut shm_process,
        pmm,
        &mut shm_caps,
        hhdm,
        8192,
    );
    pmm.clear_test_allocation_failure();
    assert_eq!(
        shm_map_result,
        Err(crate::memory::shared::SharedMemError::PageTableAllocationFailed)
    );
    assert_eq!(shm_caps.shared_region_count(), 0);
    assert_eq!(pmm.free_page_count(), shm_free_before);
    unsafe {
        shm_process
            .address_space
            .reclaim_user_space(pmm, hhdm, true);
    }
    assert_eq!(pmm.free_page_count(), free_before);

    let mut owner =
        unsafe { crate::process::Process::new(0xB314, 0, "mm2a-peer-owner", pmm, hhdm) };
    let mut peer = unsafe { crate::process::Process::new(0xB315, 0, "mm2a-peer-oom", pmm, hhdm) };
    let mut peer_caps = crate::capability::CapabilityBroker::new();
    let (_, token) =
        crate::memory::shared::alloc_shared_region(&mut owner, pmm, &mut peer_caps, hhdm, 4096)
            .expect("MM-2A injected peer owner setup");
    assert_eq!(peer_caps.shared_region_map_count(token), Some(1));
    pmm.fail_test_allocations_after(0);
    let peer_result =
        crate::memory::shared::map_shared_page(&mut peer, token, pmm, &mut peer_caps, hhdm);
    pmm.clear_test_allocation_failure();
    assert_eq!(
        peer_result,
        Err(crate::memory::shared::SharedMemError::PageTableAllocationFailed)
    );
    assert_eq!(peer_caps.shared_region_map_count(token), Some(1));
    let peer_virt =
        crate::memory::shared::map_shared_page(&mut peer, token, pmm, &mut peer_caps, hhdm)
            .expect("MM-2A peer retry");
    assert_eq!(
        peer_virt.as_u64(),
        crate::memory::shared::SHARED_REGION_BASE
    );
    assert_eq!(peer_caps.shared_region_map_count(token), Some(2));
    crate::memory::shared::cleanup_shared_pages(&mut peer, pmm, &mut peer_caps);
    crate::memory::shared::cleanup_shared_pages(&mut owner, pmm, &mut peer_caps);
    unsafe {
        peer.address_space.reclaim_user_space(pmm, hhdm, true);
        owner.address_space.reclaim_user_space(pmm, hhdm, true);
    }
    assert_eq!(pmm.free_page_count(), free_before);
    crate::serial_println!("[MM-2A] injected mmap/SHM allocation rollback: OK");
}
