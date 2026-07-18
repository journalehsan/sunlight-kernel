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
