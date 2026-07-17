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
}

pub fn run_boot_self_tests(pmm: &mut crate::memory::pmm::PhysicalMemoryManager, hhdm: VirtAddr) {
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
    let shm = shm_caps
        .resolve_shared_region(token)
        .expect("MM-0 SHM capability");
    for frame in &shm.frames {
        let bytes = unsafe { &*((hhdm + frame.start_address().as_u64()).as_ptr::<[u8; 4096]>()) };
        assert!(bytes.iter().all(|byte| *byte == 0));
    }
    unsafe {
        (hhdm + shm.frames[0].start_address().as_u64())
            .as_mut_ptr::<u8>()
            .write_volatile(0x5A);
    }
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
            (hhdm + shm.frames[0].start_address().as_u64())
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
}
