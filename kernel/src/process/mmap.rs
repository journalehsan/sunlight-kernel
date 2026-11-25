use super::mm2a_plan::{checked_page_layout, DeferredCursor};
use crate::memory::pmm::PhysicalMemoryManager;
use crate::process::address_space::{
    ExpectedMapping, MappingError, OwnershipTransition, ReplacementMapping,
};
use crate::process::region::{
    MappingKind, MappingRegion, RegionBacking, RegionPolicy, RegionProtection,
};
use crate::sched::Scheduler;
use crate::process::fd_table::{CapRights, FileHandle};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};
use x86_64::structures::paging::{Page, PageTableFlags, PhysFrame, Size4KiB};
use x86_64::{PhysAddr, VirtAddr};

// mmap flags
pub const MAP_PRIVATE: u32 = 0x02;
pub const MAP_SHARED: u32 = 0x01;
pub const MAP_ANONYMOUS: u32 = 0x20;
pub const MAP_FIXED: u32 = 0x10;
pub const MAP_FIXED_NOREPLACE: u32 = 0x100000;
/// Native stack/TLS resource storage retains exact allocation boundaries.
/// This flag is native-only and is not a Linux mmap flag extension.
pub const MAP_NATIVE_THREAD: u32 = 0x4000_0000;

// mprotect flags
pub const PROT_NONE: u32 = 0;
pub const PROT_READ: u32 = 0x1;
pub const PROT_WRITE: u32 = 0x2;
pub const PROT_EXEC: u32 = 0x4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MmapError {
    BadFd,
    FileIo,
    InvalidAddress,
    NoMemory,
    InvalidFlags,
    InvalidProt,
    PermissionDenied,
    Protected,
    AlreadyMapped,
    Unsupported,
    SwappedUnsupported,
    InternalInvariant,
}

pub const fn munmap_linux_errno(error: MmapError) -> i32 {
    match error {
        MmapError::NoMemory => 12,
        MmapError::PermissionDenied | MmapError::Protected => 13,
        MmapError::InternalInvariant => 14,
        _ => 22,
    }
}

pub const fn mprotect_linux_errno(error: MmapError) -> i32 {
    match error {
        MmapError::NoMemory => 12,
        MmapError::PermissionDenied | MmapError::Protected => 13,
        MmapError::InternalInvariant => 14,
        MmapError::Unsupported | MmapError::SwappedUnsupported => 95,
        _ => 22,
    }
}

#[cfg(all(debug_assertions, feature = "helios_runtime_mmap"))]
static REJECT_NEXT_EAGER_PUBLICATION: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

#[cfg(all(debug_assertions, feature = "helios_runtime_mmap"))]
pub(crate) fn reject_next_eager_publication_for_test() {
    REJECT_NEXT_EAGER_PUBLICATION.store(true, Ordering::Relaxed);
}

static MUNMAP_REQUESTS: AtomicU64 = AtomicU64::new(0);
static MUNMAP_SUCCESSES: AtomicU64 = AtomicU64::new(0);
static MUNMAP_PAGES: AtomicU64 = AtomicU64::new(0);
static MUNMAP_FULL_REGIONS: AtomicU64 = AtomicU64::new(0);
static MUNMAP_PREFIX_REGIONS: AtomicU64 = AtomicU64::new(0);
static MUNMAP_SUFFIX_REGIONS: AtomicU64 = AtomicU64::new(0);
static MUNMAP_MIDDLE_SPLITS: AtomicU64 = AtomicU64::new(0);
static MUNMAP_HOLE_PAGES: AtomicU64 = AtomicU64::new(0);
static MUNMAP_PROTECTED_REJECTIONS: AtomicU64 = AtomicU64::new(0);
static MUNMAP_CAPACITY_REJECTIONS: AtomicU64 = AtomicU64::new(0);
static MUNMAP_SWAPPED_RELEASED: AtomicU64 = AtomicU64::new(0);
static MUNMAP_INVARIANT_FAILURES: AtomicU64 = AtomicU64::new(0);

static MPROTECT_REQUESTS: AtomicU64 = AtomicU64::new(0);
static MPROTECT_SUCCESSES: AtomicU64 = AtomicU64::new(0);
static MPROTECT_NOOPS: AtomicU64 = AtomicU64::new(0);
static MPROTECT_PAGES_CHANGED: AtomicU64 = AtomicU64::new(0);
static MPROTECT_RW_TO_R: AtomicU64 = AtomicU64::new(0);
static MPROTECT_RW_TO_RX: AtomicU64 = AtomicU64::new(0);
static MPROTECT_R_TO_RW: AtomicU64 = AtomicU64::new(0);
static MPROTECT_R_TO_RX: AtomicU64 = AtomicU64::new(0);
static MPROTECT_RX_TO_R: AtomicU64 = AtomicU64::new(0);
static MPROTECT_RX_TO_RW: AtomicU64 = AtomicU64::new(0);
static MPROTECT_WX_REJECTIONS: AtomicU64 = AtomicU64::new(0);
static MPROTECT_PROTECTED_REJECTIONS: AtomicU64 = AtomicU64::new(0);
static MPROTECT_HOLE_REJECTIONS: AtomicU64 = AtomicU64::new(0);
static MPROTECT_SWAPPED_REJECTIONS: AtomicU64 = AtomicU64::new(0);
static MPROTECT_NONE_REJECTIONS: AtomicU64 = AtomicU64::new(0);
static MPROTECT_CAPACITY_FAILURES: AtomicU64 = AtomicU64::new(0);
static MPROTECT_INVARIANT_FAILURES: AtomicU64 = AtomicU64::new(0);

const MUNMAP_CHUNK_PAGES: u64 = crate::memory::tlb::RANGE_FLUSH_PAGE_THRESHOLD;
const MPROTECT_CHUNK_PAGES: usize = crate::memory::tlb::RANGE_FLUSH_PAGE_THRESHOLD as usize;

#[derive(Clone, Copy)]
struct ProtectionPage {
    address: u64,
    frame: PhysAddr,
    flags: PageTableFlags,
    old: RegionProtection,
}

#[derive(Default)]
struct ProtectionTransitions {
    rw_to_r: u64,
    rw_to_rx: u64,
    r_to_rw: u64,
    r_to_rx: u64,
    rx_to_r: u64,
    rx_to_rw: u64,
}

#[derive(Clone, Copy)]
enum RemovedOwnership {
    Empty,
    Present(PhysAddr),
    Swapped(u64),
}

#[derive(Clone, Copy)]
struct RemovedPage {
    address: u64,
    ownership: RemovedOwnership,
}

#[derive(Clone, Copy)]
struct InstalledAnonymousPage {
    page: Page<Size4KiB>,
    frame: PhysAddr,
    replaced: Option<ExpectedMapping>,
}

impl RemovedPage {
    const EMPTY: Self = Self {
        address: 0,
        ownership: RemovedOwnership::Empty,
    };
}

pub fn diagnostic_report() {
    crate::serial_println!(
        "[MM-2D-DIAG] requests={} successes={} pages={} full={} prefix={} suffix={} middle={} holes={} protected={} capacity={} swapped={} invariant_failures={}",
        MUNMAP_REQUESTS.load(Ordering::Relaxed),
        MUNMAP_SUCCESSES.load(Ordering::Relaxed),
        MUNMAP_PAGES.load(Ordering::Relaxed),
        MUNMAP_FULL_REGIONS.load(Ordering::Relaxed),
        MUNMAP_PREFIX_REGIONS.load(Ordering::Relaxed),
        MUNMAP_SUFFIX_REGIONS.load(Ordering::Relaxed),
        MUNMAP_MIDDLE_SPLITS.load(Ordering::Relaxed),
        MUNMAP_HOLE_PAGES.load(Ordering::Relaxed),
        MUNMAP_PROTECTED_REJECTIONS.load(Ordering::Relaxed),
        MUNMAP_CAPACITY_REJECTIONS.load(Ordering::Relaxed),
        MUNMAP_SWAPPED_RELEASED.load(Ordering::Relaxed),
        MUNMAP_INVARIANT_FAILURES.load(Ordering::Relaxed),
    );
    crate::serial_println!(
        "[MM-2E-DIAG] requests={} successes={} noops={} pages={} rw_r={} rw_rx={} r_rw={} r_rx={} rx_r={} rx_rw={} wx={} protected={} holes={} swapped={} prot_none={} capacity={} invariant_failures={}",
        MPROTECT_REQUESTS.load(Ordering::Relaxed),
        MPROTECT_SUCCESSES.load(Ordering::Relaxed),
        MPROTECT_NOOPS.load(Ordering::Relaxed),
        MPROTECT_PAGES_CHANGED.load(Ordering::Relaxed),
        MPROTECT_RW_TO_R.load(Ordering::Relaxed),
        MPROTECT_RW_TO_RX.load(Ordering::Relaxed),
        MPROTECT_R_TO_RW.load(Ordering::Relaxed),
        MPROTECT_R_TO_RX.load(Ordering::Relaxed),
        MPROTECT_RX_TO_R.load(Ordering::Relaxed),
        MPROTECT_RX_TO_RW.load(Ordering::Relaxed),
        MPROTECT_WX_REJECTIONS.load(Ordering::Relaxed),
        MPROTECT_PROTECTED_REJECTIONS.load(Ordering::Relaxed),
        MPROTECT_HOLE_REJECTIONS.load(Ordering::Relaxed),
        MPROTECT_SWAPPED_REJECTIONS.load(Ordering::Relaxed),
        MPROTECT_NONE_REJECTIONS.load(Ordering::Relaxed),
        MPROTECT_CAPACITY_FAILURES.load(Ordering::Relaxed),
        MPROTECT_INVARIANT_FAILURES.load(Ordering::Relaxed),
    );
}

/// Validate the ABI protection request against the permissions this kernel
/// can represent exactly. x86_64 page tables cannot enforce write-only or
/// execute-only user mappings, so those requests are rejected rather than
/// silently widened to RW or RX. PROT_NONE is represented by a present,
/// supervisor-only NX leaf so ownership remains tracked while Ring 3 has no
/// access.
fn normalize_protection(prot: u32) -> Result<RegionProtection, MmapError> {
    if prot & !(PROT_READ | PROT_WRITE | PROT_EXEC) != 0 {
        return Err(MmapError::InvalidProt);
    }
    if prot == PROT_NONE {
        return Ok(RegionProtection::NONE);
    }
    if prot & PROT_WRITE != 0 && prot & PROT_EXEC != 0 {
        return Err(MmapError::PermissionDenied);
    }
    if prot & PROT_READ == 0 {
        return Err(MmapError::InvalidProt);
    }
    let protection = if prot & PROT_EXEC != 0 {
        RegionProtection::READ_EXECUTE
    } else if prot & PROT_WRITE != 0 {
        RegionProtection::READ_WRITE
    } else {
        RegionProtection::READ_ONLY
    };
    // Keep RegionProtection and the architecture PTE conversion as the
    // authoritative W^X validators even after ABI normalization.
    RegionProtection::new(
        protection.readable(),
        protection.writable(),
        protection.executable(),
    )
    .map_err(|_| MmapError::PermissionDenied)
}

/// Map memory in the current process.
pub fn sys_mmap(
    addr: u64,
    length: u64,
    prot: u32,
    flags: u32,
    fd: i32,
    offset: u64,
    pmm: &mut PhysicalMemoryManager,
    sched: &mut Scheduler,
) -> Result<u64, MmapError> {
    if flags & MAP_NATIVE_THREAD != 0 && sched.current_process().is_linux_compat() {
        return Err(MmapError::InvalidFlags);
    }
    if flags == MAP_SHARED {
        return map_shared_fd(addr, length, prot, fd, offset, pmm, sched);
    }
    if flags & MAP_ANONYMOUS == 0 && sched.current_process().is_linux_compat() {
        if flags & MAP_PRIVATE == 0
            || flags & !(MAP_PRIVATE | MAP_FIXED | MAP_FIXED_NOREPLACE) != 0
        {
            return Err(MmapError::InvalidFlags);
        }
        let (_, span) = checked_page_layout(length).map_err(|_| MmapError::InvalidAddress)?;
        if offset & 4095 != 0
            || offset.checked_add(span).is_none_or(|end| end > i64::MAX as u64)
        {
            return Err(MmapError::InvalidAddress);
        }
        let descriptor = sched
            .current_shared_process()
            .fd_table
            .get(fd)
            .ok_or(MmapError::BadFd)?;
        if !descriptor.handle.is_vfs() {
            return Err(MmapError::Unsupported);
        }
        if !descriptor.rights.contains(CapRights::new(CapRights::READ)) {
            return Err(MmapError::PermissionDenied);
        }
        let handle = sunlight_fs::vfs::FileHandle(descriptor.handle.vfs_handle());
        // Keep the descriptor live under the scheduler lock and serialize
        // reads against VFS writers for the duration of the copy.
        let mut guard = crate::KERNEL_VFS.lock();
        let vfs = guard.as_mut().ok_or(MmapError::FileIo)?;
        let stat = vfs.fstat_handle(handle).map_err(|_| MmapError::FileIo)?;
        if stat.file_type != sunlight_fs::vfs::FileType::File {
            return Err(MmapError::Unsupported);
        }
        let mut populate = |index: u64, frame: PhysAddr, hhdm: VirtAddr| {
            let file_offset = (offset + index * 4096) as usize;
            let count = stat.size.saturating_sub(file_offset).min(4096);
            if count == 0 {
                return Ok(());
            }
            // The frame is sanitized, exclusively owned, and still unpublished.
            let bytes = unsafe {
                core::slice::from_raw_parts_mut((hhdm + frame.as_u64()).as_mut_ptr::<u8>(), count)
            };
            let mut done = 0;
            while done < count {
                let read = vfs.read(handle, file_offset + done, &mut bytes[done..])
                    .map_err(|_| MmapError::FileIo)?;
                if read == 0 {
                    return Err(MmapError::FileIo);
                }
                done += read;
            }
            Ok(())
        };
        let addr = if flags & (MAP_FIXED | MAP_FIXED_NOREPLACE) == 0 {
            0
        } else {
            addr
        };
        return map_private_kind(
            addr,
            length,
            prot,
            flags,
            pmm,
            sched,
            MappingKind::Anonymous,
            false,
            Some(&mut populate),
        );
    }
    if fd != -1 || offset != 0 {
        return Err(MmapError::InvalidFlags);
    }
    // Preserve the existing eager PROT_NONE reservation representation for
    // Linux stacks. Ordinary syscall execution itself uses the private T1
    // supervisor stack, independently of user-stack mapping permissions.
    let demand_backed = sched.current_process().is_linux_compat() && prot != PROT_NONE;
    #[cfg(all(debug_assertions, feature = "helios_runtime_page_allocator"))]
    if length == 3 * 4096
        && (mmap_fault_fixture(sched, "F")
            || mmap_fault_fixture(sched, "T")
            || mmap_fault_fixture(sched, "L"))
    {
        let pid = sched.current_process().pid;
        let counts = |sched: &Scheduler, pmm: &PhysicalMemoryManager| {
            let owner = sched.current_memory_owner();
            (pmm.free_page_count(), pmm.owned_frame_count(pid as u32),
             owner.address_space.region_count(), owner.mmap_next)
        };
        let before = counts(sched, pmm);
        let result = map_private_kind(
            addr, length, prot, flags, pmm, sched, MappingKind::Anonymous, false, None,
        );
        assert!(result.is_err());
        assert_eq!(before, counts(sched, pmm));
        crate::serial_println!(
            "[PAGE-ALLOC-VM] pid={} rollback=exact free={} owned={} regions={} cursor=unchanged",
            pid, before.0, before.1, before.2
        );
        return result;
    }
    map_private_kind(
        addr,
        length,
        prot,
        flags,
        pmm,
        sched,
        MappingKind::Anonymous,
        demand_backed,
        None,
    )
}

pub(crate) const SHARED_PAGE_PATH: &str = "/dev/sunlight-shared-page";
static SHARED_PAGE: spin::Mutex<Option<PhysFrame<Size4KiB>>> = spin::Mutex::new(None);

fn map_shared_fd(
    addr: u64,
    length: u64,
    prot: u32,
    fd: i32,
    offset: u64,
    pmm: &mut PhysicalMemoryManager,
    sched: &mut Scheduler,
) -> Result<u64, MmapError> {
    if addr != 0 || offset & 4095 != 0 || length == 0 {
        return Err(MmapError::InvalidAddress);
    }
    let protection = match prot {
        PROT_READ => RegionProtection::READ_ONLY,
        3 => RegionProtection::READ_WRITE,
        _ => return Err(MmapError::InvalidProt),
    };
    let descriptor = sched.current_shared_process().fd_table.get(fd).ok_or(MmapError::BadFd)?;
    if !matches!(descriptor.handle, FileHandle::SHARED_PAGE | FileHandle::MEMFD) {
        return Err(MmapError::Unsupported);
    }
    let mut required = CapRights::READ | CapRights::MMAP_R;
    if prot & PROT_WRITE != 0 {
        required |= CapRights::WRITE | CapRights::MMAP_W;
    }
    if !descriptor.rights.contains(CapRights::new(required)) {
        return Err(MmapError::PermissionDenied);
    }
    let hhdm_offset = crate::HHDM_REQ
        .response()
        .map(|response| VirtAddr::new(response.offset))
        .ok_or(MmapError::InternalInvariant)?;
    if descriptor.handle == FileHandle::MEMFD {
        let id = descriptor.memfd_view();
        let end = offset.checked_add(length).ok_or(MmapError::InvalidAddress)?;
        let space = &mut sched.current_shared_process_mut().address_space;
        let address = super::fd_table::with_memfd_frames(id, |size, frames| {
            if end > size {
                return Err(MmapError::InvalidAddress);
            }
            unsafe {
                space.map_shared_region_protected(
                    &frames[(offset / 4096) as usize..end.div_ceil(4096) as usize],
                    RegionBacking::Memfd {
                        description: id,
                        first_page: (offset / 4096) as u16,
                    },
                    protection,
                    pmm,
                    hhdm_offset,
                )
            }
            .map_err(shared_mapping_error)
        })?;
        super::fd_table::retain_memfd_view(id);
        return Ok(address.as_u64());
    }
    if offset != 0 || length != 4096 {
        return Err(MmapError::InvalidAddress);
    }
    let mut backing = SHARED_PAGE.lock();
    let frame = match *backing {
        Some(frame) => frame,
        None => {
            let physical = pmm.alloc_frame().ok_or(MmapError::NoMemory)?;
            if !crate::memory::security::sanitize_user_frame(physical, hhdm_offset) {
                pmm.free_frame(physical);
                return Err(MmapError::InternalInvariant);
            }
            PhysFrame::containing_address(physical)
        }
    };
    let process = sched.current_process_mut();
    let mapped = unsafe {
        process.address_space.map_shared_region_protected(
            &[frame],
            RegionBacking::FdSharedPage,
            protection,
            pmm,
            hhdm_offset,
        )
    };
    match mapped {
        Ok(address) => {
            *backing = Some(frame);
            #[cfg(all(debug_assertions, feature = "fd_shared_mmap_test"))]
            crate::serial_println!(
                "[FD-SHARED] pid={} fd={} object=shared-page frame={:#x} va={:#x}",
                process.pid, fd, frame.start_address().as_u64(), address.as_u64()
            );
            Ok(address.as_u64())
        }
        Err(error) => {
            if backing.is_none() {
                pmm.free_frame(frame.start_address());
            }
            match error {
                crate::memory::shared::SharedMemError::OutOfMemory => Err(MmapError::NoMemory),
                crate::memory::shared::SharedMemError::AlreadyMapped => Err(MmapError::AlreadyMapped),
                _ => Err(MmapError::InternalInvariant),
            }
        }
    }
}

fn shared_mapping_error(error: crate::memory::shared::SharedMemError) -> MmapError {
    use crate::memory::shared::SharedMemError;
    match error {
        SharedMemError::OutOfMemory => MmapError::NoMemory,
        SharedMemError::AlreadyMapped => MmapError::AlreadyMapped,
        _ => MmapError::InternalInvariant,
    }
}

pub fn map_brk(
    addr: u64,
    length: u64,
    pmm: &mut PhysicalMemoryManager,
    sched: &mut Scheduler,
) -> Result<u64, MmapError> {
    map_private_kind(
        addr,
        length,
        PROT_READ | PROT_WRITE,
        MAP_FIXED | MAP_PRIVATE | MAP_ANONYMOUS,
        pmm,
        sched,
        MappingKind::Brk,
        false,
        None,
    )
}

type PagePopulator<'a> = dyn FnMut(u64, PhysAddr, VirtAddr) -> Result<(), MmapError> + 'a;

fn map_private_kind(
    addr: u64,
    length: u64,
    prot: u32,
    flags: u32,
    pmm: &mut PhysicalMemoryManager,
    sched: &mut Scheduler,
    kind: MappingKind,
    demand_backed: bool,
    mut populate: Option<&mut PagePopulator<'_>>,
) -> Result<u64, MmapError> {
    let protection = match normalize_protection(prot) {
        Err(MmapError::PermissionDenied) => {
            crate::memory::security::note_rwx_mapping_rejected();
            return Err(MmapError::PermissionDenied);
        }
        result => result?,
    };

    if flags & !(MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED | MAP_FIXED_NOREPLACE | MAP_NATIVE_THREAD)
        != 0
    {
        return Err(MmapError::InvalidFlags);
    }
    if flags & MAP_NATIVE_THREAD != 0
        && (addr != 0 || flags & (MAP_FIXED | MAP_FIXED_NOREPLACE) != 0)
    {
        return Err(MmapError::InvalidFlags);
    }
    if flags & (MAP_FIXED | MAP_FIXED_NOREPLACE) == (MAP_FIXED | MAP_FIXED_NOREPLACE) {
        return Err(MmapError::InvalidFlags);
    }

    if flags & MAP_PRIVATE == 0 || (flags & MAP_ANONYMOUS == 0 && populate.is_none()) {
        return Err(MmapError::InvalidFlags);
    }

    // Base of the anonymous mmap region for hint-less mappings.
    const MMAP_REGION_BASE: u64 = 0x10_0000_0000u64;

    let (page_count, span) = checked_page_layout(length).map_err(|_| MmapError::InvalidAddress)?;

    // Determine the address to map at. For anonymous mappings without a
    // fixed address we hand out fresh VA ranges from the address-space owner's
    // bump cursor, shared with native and Linux borrowers. Independent cursors
    // alias the same range and corrupt userspace allocators (e.g. musl
    // mallocng), which then page-faulted on the first free().
    let fixed = flags & (MAP_FIXED | MAP_FIXED_NOREPLACE) != 0;
    let noreplace = flags & MAP_FIXED_NOREPLACE != 0;
    // `map_brk` reuses this mapper for exact placement, but brk growth is not
    // an mmap replacement operation and must retain its existing collision
    // behavior.
    let replace_fixed = fixed && !noreplace && kind == MappingKind::Anonymous;
    let deferred_cursor = if fixed {
        None
    } else {
        // The native mapper has no safe hint-placement policy. Reject a
        // non-zero hint instead of ignoring it and returning an unrelated
        // range; MAP_FIXED below retains exact no-replacement semantics.
        if addr != 0 {
            return Err(MmapError::InvalidAddress);
        }
        Some(
            DeferredCursor::new(
                sched.current_memory_owner().mmap_next,
                MMAP_REGION_BASE,
                span,
            )
            .map_err(|_| MmapError::InvalidAddress)?,
        )
    };
    let map_addr = if fixed {
        // Use the provided address (must be page-aligned).
        if addr & 0xFFF != 0 {
            return Err(MmapError::InvalidAddress);
        }
        addr
    } else {
        // Cursor publication is deferred until the full mapping commits.
        deferred_cursor.ok_or(MmapError::InternalInvariant)?.base()
    };

    let span_usize = usize::try_from(span).map_err(|_| MmapError::InvalidAddress)?;
    if crate::memory::user::UserRange::new(map_addr, span_usize).is_err() {
        return Err(MmapError::InvalidAddress);
    }

    let page_flags =
        crate::process::address_space::AddressSpace::protection_to_pte_flags(protection)
            .map_err(|_| MmapError::PermissionDenied)?;

    // Map all the pages
    let pid = sched.current_memory_owner().pid;
    let hhdm_offset = crate::HHDM_REQ
        .response()
        .map(|response| VirtAddr::new(response.offset))
        .ok_or(MmapError::InternalInvariant)?;

    if !replace_fixed {
        for i in 0..page_count {
            let page_vaddr = VirtAddr::new(map_addr + i * 4096);
            let page =
                Page::from_start_address(page_vaddr).map_err(|_| MmapError::InvalidAddress)?;
            let process = sched
                .process_mut_by_pid(pid)
                .ok_or(MmapError::InternalInvariant)?;
            if unsafe { process.address_space.is_occupied(page, hhdm_offset) } {
                crate::process::address_space::note_mapping_collision();
                return Err(MmapError::AlreadyMapped);
            }
        }
    }

    let mut policy = match kind {
        MappingKind::Anonymous => RegionPolicy::MAY_UNMAP
            .union(RegionPolicy::MAY_REPLACE)
            .union(RegionPolicy::MAY_CHANGE_PROTECTION)
            .union(RegionPolicy::OWNER_MANAGED),
        MappingKind::Brk => RegionPolicy::MAY_UNMAP
            .union(RegionPolicy::MAY_REPLACE)
            .union(RegionPolicy::OWNER_MANAGED),
        _ => return Err(MmapError::InternalInvariant),
    };
    if demand_backed {
        policy = policy.union(RegionPolicy::DEMAND);
    }
    if flags & MAP_NATIVE_THREAD != 0 {
        policy = policy.union(RegionPolicy::NO_MERGE);
    }
    let region = MappingRegion::new(
        map_addr,
        map_addr
            .checked_add(span)
            .ok_or(MmapError::InvalidAddress)?,
        protection,
        kind,
        policy,
        RegionBacking::AnonymousOwner(pid as u32),
    )
    .map_err(|_| MmapError::InvalidAddress)?;
    if replace_fixed {
        let plan = sched.current_process().address_space.preflight_replace(region)
            .map_err(mapping_error)?;
        return map_fixed_transaction(
            map_addr, page_count, page_flags, plan, pid, pmm, sched, hhdm_offset, populate,
        );
    }
    let reservation = Some(sched.current_process().address_space.preflight_region(region)
        .map_err(mapping_error)?);

    // Hint-less Linux mappings reserve their virtual range now and receive
    // zeroed frames on first access. Fixed mappings retain the eager path so
    // replacing a live range always releases its old pages in one transaction.
    if demand_backed && !fixed {
        if let Some(reservation) = reservation {
            #[cfg(all(debug_assertions, feature = "helios_runtime_mmap"))]
            if length == 4097 && mmap_fault_fixture(sched, "S") {
                sched.current_process().address_space.cancel_region(reservation);
                crate::serial_println!(
                    "[ANON-MMAP-FAULT] pid={} setup scratch publication cancelled requested=4097 effective=8192",
                    pid
                );
            }
            if let Err(error) = sched
                .current_process()
                .address_space
                .commit_region(reservation)
            {
                return Err(mapping_error(error));
            }
        }
        if let Some(cursor) = deferred_cursor {
            cursor.commit(&mut sched.current_memory_owner_mut().mmap_next);
        }
        return Ok(map_addr);
    }

    let page_count_usize = usize::try_from(page_count).map_err(|_| MmapError::InvalidAddress)?;
    let mut installed: Vec<InstalledAnonymousPage> = Vec::new();
    installed.try_reserve_exact(page_count_usize).map_err(|_| {
        if let Some(reservation) = reservation {
            sched
                .current_process()
                .address_space
                .cancel_region(reservation);
        }
        MmapError::NoMemory
    })?;
    if crate::memory::swap::reserve_candidates(page_count_usize).is_err() {
        if let Some(reservation) = reservation {
            sched
                .current_process()
                .address_space
                .cancel_region(reservation);
        }
        return Err(MmapError::NoMemory);
    }
    // One budget for this entire locked mapping transaction. Reclaiming up to
    // 256 pages before every installed page could starve IPC for seconds.
    // If bounded reclaim cannot supply enough frames, normal ENOMEM rollback
    // preserves the original region rather than publishing a partial mapping.
    let mut reclaim_budget = crate::memory::swap::DirectReclaimBudget::new();
    for i in 0..page_count {
        let page_vaddr = VirtAddr::new(
            map_addr
                .checked_add(i.checked_mul(4096).ok_or(MmapError::InvalidAddress)?)
                .ok_or(MmapError::InvalidAddress)?,
        );
        let page = Page::from_start_address(page_vaddr).map_err(|_| MmapError::InvalidAddress)?;

        crate::memory::swap::maybe_reclaim_for_anonymous_allocation(
            sched,
            pmm,
            hhdm_offset,
            Some((pid, page_vaddr)),
            &mut reclaim_budget,
        );
        #[cfg(all(debug_assertions, feature = "helios_runtime_page_allocator"))]
        let reject_frame = length == 3 * 4096 && i == 1 && mmap_fault_fixture(sched, "F");
        #[cfg(all(debug_assertions, feature = "helios_runtime_page_allocator"))]
        if reject_frame {
            pmm.fail_test_allocations_after(0);
        }
        let allocated_frame = pmm.alloc_frame_owned(pid as u32);
        #[cfg(all(debug_assertions, feature = "helios_runtime_page_allocator"))]
        if reject_frame {
            pmm.clear_test_allocation_failure();
            assert!(allocated_frame.is_none());
            crate::serial_println!("[PAGE-ALLOC-VM] pid={} fault=frame page=2", pid);
        }
        let frame_addr = match allocated_frame {
            Some(addr) => addr,
            None => {
                crate::process::address_space::note_frame_allocation_failure();
                rollback_anonymous(&mut installed, pid, pmm, sched, hhdm_offset);
                if let Some(reservation) = reservation {
                    sched
                        .current_process()
                        .address_space
                        .cancel_region(reservation);
                }
                return Err(MmapError::NoMemory);
            }
        };
        if !crate::memory::security::sanitize_user_frame(frame_addr, hhdm_offset) {
            pmm.free_frame(frame_addr);
            rollback_anonymous(&mut installed, pid, pmm, sched, hhdm_offset);
            if let Some(reservation) = reservation {
                sched
                    .current_process()
                    .address_space
                    .cancel_region(reservation);
            }
            return Err(MmapError::NoMemory);
        }
        if let Some(populate) = populate.as_deref_mut() {
            if let Err(error) = populate(i, frame_addr, hhdm_offset) {
                pmm.free_frame(frame_addr);
                rollback_anonymous(&mut installed, pid, pmm, sched, hhdm_offset);
                if let Some(reservation) = reservation {
                    sched.current_process().address_space.cancel_region(reservation);
                }
                return Err(error);
            }
        }
        let frame = unsafe { PhysFrame::from_start_address_unchecked(frame_addr) };

        #[cfg(all(debug_assertions, feature = "helios_runtime_page_allocator"))]
        let reject_table = length == 3 * 4096 && i == 0 && mmap_fault_fixture(sched, "T");
        let proc = match sched.process_mut_by_pid(pid) {
            Some(process) => process,
            None => {
                pmm.free_frame(frame_addr);
                rollback_anonymous(&mut installed, pid, pmm, sched, hhdm_offset);
                if let Some(reservation) = reservation {
                    sched
                        .current_process()
                        .address_space
                        .cancel_region(reservation);
                }
                return Err(MmapError::InternalInvariant);
            }
        };
        let replaced = if replace_fixed {
            match proc.address_space.lookup_region(page_vaddr.as_u64()) {
                Some(region) => match expected_replaceable_leaf(
                    &proc.address_space,
                    page_vaddr.as_u64(),
                    region,
                    pmm,
                    hhdm_offset,
                ) {
                    Ok(expected) => expected,
                    Err(error) => {
                        pmm.free_frame(frame_addr);
                        rollback_anonymous(&mut installed, pid, pmm, sched, hhdm_offset);
                        return Err(error);
                    }
                },
                None if unsafe { proc.address_space.is_occupied(page, hhdm_offset) } => {
                    pmm.free_frame(frame_addr);
                    rollback_anonymous(&mut installed, pid, pmm, sched, hhdm_offset);
                    return Err(MmapError::Protected);
                }
                None => None,
            }
        } else {
            None
        };
        #[cfg(all(debug_assertions, feature = "helios_runtime_page_allocator"))]
        if reject_table {
            pmm.fail_test_allocations_after(0);
        }
        let install_result = match replaced {
            Some(expected) => unsafe {
                proc.address_space.replace_mapping(
                    page,
                    expected,
                    ReplacementMapping::Present {
                        frame,
                        flags: page_flags,
                    },
                    OwnershipTransition::RetainOld,
                    pmm,
                    hhdm_offset,
                )
            },
            None => unsafe {
                proc.address_space
                    .map_page(page, frame, page_flags, pmm, hhdm_offset)
            },
        };
        #[cfg(all(debug_assertions, feature = "helios_runtime_page_allocator"))]
        if reject_table {
            pmm.clear_test_allocation_failure();
            assert!(matches!(install_result, Err(MappingError::PageTableAllocationFailed)));
            crate::serial_println!("[PAGE-ALLOC-VM] pid={} fault=page-table page=1", pid);
        }
        if let Err(error) = install_result {
            pmm.free_frame(frame_addr);
            rollback_anonymous(&mut installed, pid, pmm, sched, hhdm_offset);
            if let Some(reservation) = reservation {
                sched
                    .current_process()
                    .address_space
                    .cancel_region(reservation);
            }
            return Err(match error {
                MappingError::AlreadyMapped => MmapError::AlreadyMapped,
                MappingError::FrameAllocationFailed | MappingError::PageTableAllocationFailed => {
                    MmapError::NoMemory
                }
                MappingError::PermissionRejected => MmapError::PermissionDenied,
                _ => MmapError::InternalInvariant,
            });
        }
        installed.push(InstalledAnonymousPage {
            page,
            frame: frame_addr,
            replaced,
        });
    }

    if let Some(reservation) = reservation {
        #[cfg(all(debug_assertions, feature = "helios_runtime_page_allocator"))]
        if length == 3 * 4096 && mmap_fault_fixture(sched, "L") {
            sched.current_process().address_space.cancel_region(reservation);
            crate::serial_println!("[PAGE-ALLOC-VM] pid={} fault=mapping-publication leaves={}", pid, installed.len());
        }
        #[cfg(all(debug_assertions, feature = "helios_runtime_mmap"))]
        if REJECT_NEXT_EAGER_PUBLICATION.swap(false, Ordering::Relaxed) {
            sched.current_process().address_space.cancel_region(reservation);
        }
        if let Err(error) = sched
            .current_process()
            .address_space
            .commit_region(reservation)
        {
            rollback_anonymous(&mut installed, pid, pmm, sched, hhdm_offset);
            return Err(mapping_error(error));
        }
    }

    for installed_page in &installed {
        if let Some(old) = installed_page.replaced {
            release_replaced_ownership(old, pmm, hhdm_offset);
        }
        crate::memory::swap::track_anon(
            pid,
            installed_page.page.start_address(),
            installed_page.frame,
        );
    }
    if let Some(cursor) = deferred_cursor {
        cursor.commit(&mut sched.current_memory_owner_mut().mmap_next);
    }

    if replace_fixed {
        let identity = sched.current_process().address_space.identity();
        sched.cancel_futex_range(identity, map_addr, map_addr + span);
    }
    Ok(map_addr)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnonymousAccess {
    Read,
    Write,
    Execute,
}

/// Back one absent page of a demand-mapped Linux anonymous region.
///
/// The caller must hold the scheduler lock that serializes address-space
/// mutation. Returns `Ok(false)` when the address is not part of a demand
/// backing, allowing the page-fault path to continue to its terminal handler.
pub unsafe fn fault_in_anonymous_page(
    process: &crate::process::Process,
    address: u64,
    access: AnonymousAccess,
    pmm: &mut PhysicalMemoryManager,
    hhdm_offset: VirtAddr,
) -> Result<bool, MmapError> {
    if address >= crate::memory::user::USER_END_EXCLUSIVE {
        return Ok(false);
    }
    let page_address = address & !0xfff;
    let Some(region) = process.address_space.lookup_region(page_address) else {
        return Ok(false);
    };
    if region.kind != MappingKind::Anonymous || !region.policy.contains(RegionPolicy::DEMAND) {
        return Ok(false);
    }
    let owner = match region.backing {
        RegionBacking::AnonymousOwner(owner) => owner as usize,
        _ => return Err(MmapError::InternalInvariant),
    };
    let allowed = match access {
        AnonymousAccess::Read => region.protection.readable(),
        AnonymousAccess::Write => region.protection.writable(),
        AnonymousAccess::Execute => region.protection.executable(),
    };
    if region.protection == RegionProtection::NONE || !allowed {
        return Err(MmapError::PermissionDenied);
    }

    let page = Page::<Size4KiB>::from_start_address(VirtAddr::new(page_address))
        .map_err(|_| MmapError::InvalidAddress)?;
    if let Some((_, flags)) = unsafe { process.address_space.lookup_entry(page, hhdm_offset) } {
        if flags.contains(PageTableFlags::PRESENT) {
            return Ok(true);
        }
        return Err(MmapError::SwappedUnsupported);
    }

    let frame_addr = pmm
        .alloc_frame_owned(owner as u32)
        .ok_or(MmapError::NoMemory)?;
    if !crate::memory::security::sanitize_user_frame(frame_addr, hhdm_offset) {
        pmm.free_frame(frame_addr);
        return Err(MmapError::NoMemory);
    }
    let flags = crate::process::address_space::AddressSpace::protection_to_pte_flags(
        region.protection,
    )
    .map_err(|_| MmapError::PermissionDenied)?;
    let frame = PhysFrame::from_start_address(frame_addr).map_err(|_| {
        pmm.free_frame(frame_addr);
        MmapError::InternalInvariant
    })?;
    if let Err(error) = unsafe {
        process
            .address_space
            .map_page(page, frame, flags, pmm, hhdm_offset)
    } {
        pmm.free_frame(frame_addr);
        return match error {
            MappingError::AlreadyMapped => Ok(true),
            MappingError::FrameAllocationFailed | MappingError::PageTableAllocationFailed => {
                Err(MmapError::NoMemory)
            }
            MappingError::PermissionRejected => Err(MmapError::PermissionDenied),
            _ => Err(MmapError::InternalInvariant),
        };
    }
    crate::memory::swap::track_anon(owner, page.start_address(), frame_addr);
    Ok(true)
}

fn mapping_error(error: MappingError) -> MmapError {
    match error {
        MappingError::AlreadyMapped | MappingError::LedgerOverlap => MmapError::AlreadyMapped,
        MappingError::FrameAllocationFailed
        | MappingError::PageTableAllocationFailed
        | MappingError::LedgerCapacityExhausted => MmapError::NoMemory,
        MappingError::PermissionRejected => MmapError::PermissionDenied,
        MappingError::ProtectedRegion => MmapError::Protected,
        MappingError::InvalidAddress
        | MappingError::NonCanonical
        | MappingError::Overflow
        | MappingError::Misaligned => MmapError::InvalidAddress,
        _ => MmapError::InternalInvariant,
    }
}

fn map_fixed_transaction(
    start: u64,
    count: u64,
    flags: PageTableFlags,
    plan: crate::process::region::ReplacePlan,
    pid: usize,
    pmm: &mut PhysicalMemoryManager,
    sched: &mut Scheduler,
    hhdm: VirtAddr,
    mut populate: Option<&mut PagePopulator<'_>>,
) -> Result<u64, MmapError> {
    use crate::memory::retirement::{Backing, MappingPage, OWNERS};
    let count = usize::try_from(count).map_err(|_| MmapError::NoMemory)?;
    if populate.is_none() && count > 32 {
        return Err(MmapError::NoMemory);
    }
    let mut pages = Vec::new();
    pages.try_reserve_exact(count).map_err(|_| MmapError::NoMemory)?;
    pages.resize(
        count,
        MappingPage { address: 0, original: None, original_flags: 0, replacement: 0 },
    );
    let identity = sched.current_process().address_space.identity();
    for (index, mapping) in pages.iter_mut().enumerate() {
        mapping.address = start + index as u64 * 4096;
        let process = sched.process_mut_by_pid(pid).ok_or(MmapError::InternalInvariant)?;
        let page = Page::from_start_address(VirtAddr::new(mapping.address)).map_err(|_| MmapError::InvalidAddress)?;
        let expected = match process.address_space.lookup_region(mapping.address) {
            Some(region) => expected_replaceable_leaf(&process.address_space, mapping.address, region, pmm, hhdm)?,
            None if unsafe { process.address_space.is_occupied(page, hhdm) } => return Err(MmapError::Protected),
            None => None,
        };
        match expected {
            Some(ExpectedMapping::Present { frame, flags }) => {
                mapping.original = Some(Backing::Frame(frame.as_u64()));
                mapping.original_flags = flags.bits();
            }
            Some(ExpectedMapping::Swapped { block_id }) => mapping.original = Some(Backing::Swap(block_id)),
            None => {}
        }
    }
    let token = {
        let mut owners = OWNERS.lock();
        if populate.is_some() {
            owners.reserve_mapping_with_capacity(
                identity.pml4_phys, identity.generation, &pages, flags.bits(), usize::MAX,
            )
        } else {
            owners.reserve_mapping(identity.pml4_phys, identity.generation, &pages, flags.bits())
        }
    }.ok_or(MmapError::NoMemory)?;
    let preparation = (|| {
        crate::memory::swap::reserve_candidates(count).map_err(|_| MmapError::NoMemory)?;
        for (index, mapping) in pages.iter().enumerate() {
            let page = Page::from_start_address(VirtAddr::new(mapping.address)).map_err(|_| MmapError::InvalidAddress)?;
            unsafe { sched.process_mut_by_pid(pid).ok_or(MmapError::InternalInvariant)?
                .address_space.prepare_transaction_tables(page, pmm, hhdm).map_err(mapping_error)?; }
            let frame = pmm.alloc_frame_owned(pid as u32).ok_or(MmapError::NoMemory)?;
            assert!(OWNERS.lock().prepare_mapping_frame(&token, index, frame.as_u64()));
            if !crate::memory::security::sanitize_user_frame(frame, hhdm) { return Err(MmapError::NoMemory); }
            if let Some(populate) = populate.as_deref_mut() {
                populate(index as u64, frame, hhdm)?;
            }
        }
        Ok(())
    })();
    if let Err(error) = preparation {
        cancel_fixed_transaction(&token, pid, pmm, sched, hhdm);
        return Err(error);
    }
    assert!(OWNERS.lock().begin_mapping(&token));
    #[cfg(all(debug_assertions, feature = "kfp2a5_transaction_test"))]
    crate::memory::retirement::mapping_receipt(&token, "installing", true);
    for index in 0..count {
        #[cfg(all(debug_assertions, feature = "kfp2a5_transaction_test"))]
        if crate::memory::transaction_gate::fail_install(index) {
            cancel_fixed_transaction(&token, pid, pmm, sched, hhdm);
            return Err(MmapError::NoMemory);
        }
        let result = unsafe { sched.process_mut_by_pid(pid).expect("mapping owner process lost")
            .address_space.write_transaction_page(&token, index, false, hhdm) };
        if let Err(error) = result {
            cancel_fixed_transaction(&token, pid, pmm, sched, hhdm);
            return Err(mapping_error(error));
        }
        assert!(OWNERS.lock().mapping_installed(&token, index));
    }
    crate::memory::tlb::invalidate_retirement_all(identity, &token);
    sched.current_process().address_space.commit_replace(plan);
    assert!(OWNERS.lock().mapping_decide(&token, true));
    #[cfg(all(debug_assertions, feature = "kfp2a5_transaction_test"))]
    crate::memory::retirement::mapping_receipt(&token, "committed", false);
    for index in 0..count {
        let mapping = OWNERS.lock().mapping_page(&token, index).expect("committed mapping descriptor lost");
        crate::memory::swap::track_anon(pid, VirtAddr::new(mapping.address), PhysAddr::new(mapping.replacement));
    }
    resolve_fixed_transaction(&token, pmm, hhdm);
    sched.cancel_futex_range(identity, start, start + count as u64 * 4096);
    Ok(start)
}

fn cancel_fixed_transaction(
    token: &crate::memory::retirement::Token,
    pid: usize,
    pmm: &mut PhysicalMemoryManager,
    sched: &mut Scheduler,
    hhdm: VirtAddr,
) {
    use crate::memory::retirement::OWNERS;
    let installed = OWNERS.lock().cancel_mapping(token).expect("mapping cancellation state");
    #[cfg(all(debug_assertions, feature = "kfp2a5_transaction_test"))]
    crate::memory::retirement::mapping_receipt(token, "cancelling", installed == 0);
    for index in (0..installed).rev() {
        #[cfg(all(debug_assertions, feature = "kfp2a5_transaction_test"))]
        if crate::memory::transaction_gate::fail_restore(index) {
            OWNERS.lock().quarantine_mapping(token);
            crate::memory::retirement::mapping_receipt(token, "quarantined", false);
            panic!("MAP_FIXED injected restoration failure; transaction retained");
        }
        let restored = sched.process_mut_by_pid(pid).is_some_and(|process| unsafe {
            process.address_space.write_transaction_page(token, index, true, hhdm).is_ok()
        });
        if !restored {
            OWNERS.lock().quarantine_mapping(token);
            #[cfg(all(debug_assertions, feature = "kfp2a5_transaction_test"))]
            crate::memory::retirement::mapping_receipt(token, "quarantined", false);
            panic!("MAP_FIXED restoration failed; transaction retained");
        }
        assert!(OWNERS.lock().mapping_restored(token, index));
    }
    let identity = sched.process_mut_by_pid(pid).expect("mapping cancellation owner lost").address_space.identity();
    crate::memory::tlb::invalidate_retirement_all(identity, token);
    assert!(OWNERS.lock().mapping_decide(token, false));
    #[cfg(all(debug_assertions, feature = "kfp2a5_transaction_test"))]
    crate::memory::retirement::mapping_receipt(token, "rolled-back", false);
    resolve_fixed_transaction(token, pmm, hhdm);
}

fn resolve_fixed_transaction(
    token: &crate::memory::retirement::Token,
    pmm: &mut PhysicalMemoryManager,
    hhdm: VirtAddr,
) {
    use crate::memory::retirement::{Backing, OWNERS};
    let count = OWNERS.lock().mapping_entry_count(token).expect("mapping owner lost");
    for index in 0..count {
        let resolution = OWNERS.lock().mapping_release(token, index);
        let Some((entry, release)) = resolution else { continue };
        if release {
            match entry.backing {
                Backing::Frame(frame) => {
                    crate::memory::swap::untrack(PhysAddr::new(frame));
                    pmm.free_frame(PhysAddr::new(frame));
                }
                Backing::Swap(block) => {
                    crate::memory::zram::discard_block(block, pmm, hhdm)
                        .expect("MAP_FIXED transaction lost swap backing");
                }
            }
        }
        assert!(OWNERS.lock().mapping_resolved(token, index, release));
    }
    #[cfg(all(debug_assertions, feature = "kfp2a5_transaction_test"))]
    crate::memory::retirement::mapping_receipt(token, "resolved", false);
    assert!(OWNERS.lock().finish_mapping(token));
}

fn rollback_anonymous(
    installed: &mut Vec<InstalledAnonymousPage>,
    pid: usize,
    pmm: &mut PhysicalMemoryManager,
    sched: &mut Scheduler,
    hhdm_offset: VirtAddr,
) {
    if installed.is_empty() {
        return;
    }
    crate::process::address_space::note_mmap_rollback();
    // Keep the exact descriptor until restoration and release succeed. A
    // failed restore cannot be reported as an ordinary mmap allocation error:
    // the old ledger may no longer describe the installed leaf, and popping
    // here would erase both the old snapshot and the new-frame obligation.
    while let Some(installed_page) = installed.last().copied() {
        let restored = sched.process_mut_by_pid(pid).is_some_and(|process| unsafe {
            match installed_page.replaced {
                Some(expected) => process
                    .address_space
                    .replace_mapping(
                        installed_page.page,
                        ExpectedMapping::Present {
                            frame: installed_page.frame,
                            flags: process
                                .address_space
                                .lookup_entry(installed_page.page, hhdm_offset)
                                .map(|(_, flags)| flags)
                                .unwrap_or(PageTableFlags::empty()),
                        },
                        replacement_for_expected(expected),
                        OwnershipTransition::RetainOld,
                        pmm,
                        hhdm_offset,
                    )
                    .is_ok(),
                None => process
                    .address_space
                    .rollback_mapped_page(
                        installed_page.page,
                        installed_page.frame,
                        pmm,
                        hhdm_offset,
                    )
                    .is_ok(),
            }
        });
        assert!(restored, "anonymous mapping rollback lost exact installed leaf");
        pmm.free_frame(installed_page.frame);
        installed.pop();
    }
}

fn replacement_for_expected(expected: ExpectedMapping) -> ReplacementMapping {
    match expected {
        ExpectedMapping::Present { frame, flags } => ReplacementMapping::Present {
            frame: unsafe { PhysFrame::from_start_address_unchecked(frame) },
            flags,
        },
        ExpectedMapping::Swapped { block_id } => ReplacementMapping::Swapped { block_id },
    }
}

fn release_replaced_ownership(
    expected: ExpectedMapping,
    pmm: &mut PhysicalMemoryManager,
    hhdm_offset: VirtAddr,
) {
    match expected {
        ExpectedMapping::Present { frame, .. } => {
            crate::memory::swap::untrack(frame);
            pmm.free_frame(frame);
        }
        ExpectedMapping::Swapped { block_id } => {
            if crate::memory::zram::discard_block(block_id, pmm, hhdm_offset).is_err() {
                panic!("MAP_FIXED replacement lost a preflighted ZRAM block");
            }
        }
    }
}

/// Validate a leaf which MAP_FIXED is allowed to replace. This is stricter
/// than ordinary occupancy: a ledger record must identify owned, replaceable
/// user memory, and the PTE must still agree with its ownership and policy.
fn expected_replaceable_leaf(
    address_space: &crate::process::address_space::AddressSpace,
    address: u64,
    region: MappingRegion,
    pmm: &PhysicalMemoryManager,
    hhdm_offset: VirtAddr,
) -> Result<Option<ExpectedMapping>, MmapError> {
    let owner = match (region.kind, region.policy, region.backing) {
        (
            MappingKind::Anonymous | MappingKind::Brk,
            policy,
            RegionBacking::AnonymousOwner(owner),
        ) if policy.contains(RegionPolicy::MAY_REPLACE) => owner,
        _ => return Err(MmapError::Protected),
    };
    let page = Page::<Size4KiB>::from_start_address(VirtAddr::new(address))
        .map_err(|_| MmapError::InternalInvariant)?;
    let Some((frame_or_marker, flags)) = (unsafe { address_space.lookup_entry(page, hhdm_offset) })
    else {
        if region.policy.contains(RegionPolicy::DEMAND) {
            return Ok(None);
        }
        return Err(MmapError::InternalInvariant);
    };
    if flags.contains(PageTableFlags::PRESENT) {
        if flags.contains(PageTableFlags::USER_ACCESSIBLE)
            != (region.protection != RegionProtection::NONE)
            || pmm.owner_of(frame_or_marker) != Some(owner)
            || crate::process::address_space::AddressSpace::protection_from_pte_flags(flags).ok()
                != Some(region.protection)
        {
            return Err(MmapError::InternalInvariant);
        }
        Ok(Some(ExpectedMapping::Present {
            frame: frame_or_marker,
            flags,
        }))
    } else {
        let Some(block_id) = (unsafe { address_space.swapped_block_id(page, hhdm_offset) }) else {
            return Err(MmapError::InternalInvariant);
        };
        if !crate::memory::zram::block_exists(block_id) {
            return Err(MmapError::InternalInvariant);
        }
        Ok(Some(ExpectedMapping::Swapped { block_id }))
    }
}

/// Remove policy-authorized anonymous mappings from the current address space.
pub fn sys_munmap(
    addr: u64,
    length: u64,
    pmm: &mut PhysicalMemoryManager,
    sched: &mut Scheduler,
) -> Result<(), MmapError> {
    MUNMAP_REQUESTS.fetch_add(1, Ordering::Relaxed);
    if addr & 0xfff != 0 || length == 0 {
        return Err(MmapError::InvalidAddress);
    }
    let (page_count, span) = checked_page_layout(length).map_err(|_| MmapError::InvalidAddress)?;
    let span_usize = usize::try_from(span).map_err(|_| MmapError::InvalidAddress)?;
    crate::memory::user::UserRange::new(addr, span_usize).map_err(|_| MmapError::InvalidAddress)?;
    let end = addr.checked_add(span).ok_or(MmapError::InvalidAddress)?;
    let hhdm_offset = crate::HHDM_REQ
        .response()
        .map(|response| VirtAddr::new(response.offset))
        .ok_or(MmapError::InternalInvariant)?;

    let space = &mut sched.current_shared_process_mut().address_space;
    if let Some(region) = space.lookup_region(addr) {
        if let RegionBacking::Memfd { description, first_page } = region.backing {
            if region.start != addr || region.end != end {
                return Err(MmapError::Protected);
            }
            super::fd_table::with_memfd_frames(description, |_, frames| {
                for (index, address) in (addr..end).step_by(4096).enumerate() {
                    let page = Page::<Size4KiB>::containing_address(VirtAddr::new(address));
                    assert_eq!(
                        unsafe { space.lookup_phys(page, hhdm_offset) },
                        Some(frames[first_page as usize + index].start_address()),
                        "memfd mapping does not match its backing",
                    );
                }
            });
            for address in (addr..end).step_by(4096) {
                let page = Page::<Size4KiB>::containing_address(VirtAddr::new(address));
                let physical = unsafe { space.unmap_page(page, hhdm_offset) };
                assert!(physical.is_some(), "memfd mapping lost its leaf");
                unsafe { space.reclaim_empty_tables_for_page(page, pmm, hhdm_offset) };
            }
            space.remove_region_exact(region).expect("memfd mapping lost its region");
            super::fd_table::release_memfd_view(description);
            super::fd_table::drain_memfd_frames(pmm);
            MUNMAP_SUCCESSES.fetch_add(1, Ordering::Relaxed);
            MUNMAP_FULL_REGIONS.fetch_add(1, Ordering::Relaxed);
            MUNMAP_PAGES.fetch_add(page_count as u64, Ordering::Relaxed);
            return Ok(());
        }
    }

    let plan = match sched
        .current_process()
        .address_space
        .preflight_unmap(addr, end)
    {
        Ok(plan) => plan,
        Err(MappingError::ProtectedRegion) => {
            MUNMAP_PROTECTED_REJECTIONS.fetch_add(1, Ordering::Relaxed);
            return Err(MmapError::Protected);
        }
        Err(MappingError::LedgerCapacityExhausted) => {
            MUNMAP_CAPACITY_REJECTIONS.fetch_add(1, Ordering::Relaxed);
            return Err(MmapError::NoMemory);
        }
        Err(error) => return Err(mapping_error(error)),
    };
    let effects = plan.effects();

    // Full expected-state/ownership preflight. This is deliberately a second
    // bounded walk after ledger staging so no PTE is cleared before every
    // mapped page in the request is proven safe to release.
    let mut page_address = addr;
    while page_address < end {
        if let Some(region) = sched
            .current_process()
            .address_space
            .lookup_region(page_address)
        {
            let _ = expected_anonymous_leaf(
                &sched.current_process().address_space,
                page_address,
                region,
                pmm,
                hhdm_offset,
            )?;
        }
        page_address = page_address
            .checked_add(4096)
            .ok_or(MmapError::InvalidAddress)?;
    }

    let identity = sched.current_process().address_space.identity();
    // Reserve once for the entire request, before its first destructive write.
    // Capacity cannot fail after an earlier chunk has already been withdrawn.
    let retirement = crate::memory::retirement::OWNERS.lock()
        .reserve(identity.pml4_phys, identity.generation).ok_or(MmapError::NoMemory)?;
    let mut chunk_start = addr;
    while chunk_start < end {
        let remaining_pages = (end - chunk_start) / 4096;
        let chunk_pages = remaining_pages.min(MUNMAP_CHUNK_PAGES);
        let mut removed = [RemovedPage::EMPTY; MUNMAP_CHUNK_PAGES as usize];
        let mut removed_len = 0usize;

        for offset in 0..chunk_pages {
            let address = chunk_start + offset * 4096;
            let Some(region) = sched.current_process().address_space.lookup_region(address) else {
                continue;
            };
            let expected = match expected_anonymous_leaf(
                &sched.current_process().address_space,
                address,
                region,
                pmm,
                hhdm_offset,
            ) {
                Ok(Some(expected)) => expected,
                Ok(None) => continue,
                Err(_) => {
                    panic!("MM-2D munmap invariant failure: leaf changed after complete preflight")
                }
            };
            removed[removed_len] = RemovedPage {
                address,
                ownership: match expected {
                    ExpectedMapping::Present { frame, .. } => RemovedOwnership::Present(frame),
                    ExpectedMapping::Swapped { block_id } => RemovedOwnership::Swapped(block_id),
                },
            };
            removed_len += 1;
        }

        if removed_len != 0 {
            let mut entries = [crate::memory::retirement::Entry {
                address: 0, backing: crate::memory::retirement::Backing::Frame(0), released: false,
            }; MUNMAP_CHUNK_PAGES as usize];
            for (slot, removed_page) in entries.iter_mut().zip(&removed[..removed_len]) {
                slot.address = removed_page.address;
                slot.backing = match removed_page.ownership {
                    RemovedOwnership::Present(frame) => crate::memory::retirement::Backing::Frame(frame.as_u64()),
                    RemovedOwnership::Swapped(block) => crate::memory::retirement::Backing::Swap(block),
                    RemovedOwnership::Empty => munmap_invariant_failure("empty staged retirement"),
                };
            }
            assert!(crate::memory::retirement::OWNERS.lock().stage(&retirement, &entries[..removed_len]));
            #[cfg(all(debug_assertions, feature = "kfp2a2_fault_test"))]
            unsafe { crate::memory::pmm::retirement_track(&entries[..removed_len]); }
            // The journal already owns every obligation before any leaf changes.
            for removed_page in &removed[..removed_len] {
                let address = removed_page.address;
                let region = sched.current_process().address_space.lookup_region(address)
                    .unwrap_or_else(|| munmap_invariant_failure("staged region disappeared"));
                let expected = expected_anonymous_leaf(&sched.current_process().address_space,
                    address, region, pmm, hhdm_offset)
                    .unwrap_or_else(|_| munmap_invariant_failure("staged leaf changed"))
                    .unwrap_or_else(|| munmap_invariant_failure("staged leaf disappeared"));
            let page = Page::<Size4KiB>::from_start_address(VirtAddr::new(address))
                .unwrap_or_else(|_| munmap_invariant_failure("preflighted page lost alignment"));
            if unsafe {
                sched
                    .current_process_mut()
                    .address_space
                    .remove_expected_mapping(page, expected, hhdm_offset)
            }
            .is_err()
            {
                munmap_invariant_failure("leaf changed after complete preflight");
            }
            }
            assert!(crate::memory::retirement::OWNERS.lock().withdrawn(&retirement));
            #[cfg(all(debug_assertions, feature = "kfp2a2_fault_test"))]
            for entry in &entries[..removed_len] {
                let page = Page::<Size4KiB>::from_start_address(VirtAddr::new(entry.address)).unwrap();
                let absent = unsafe { sched.current_process().address_space.lookup_entry(page, hhdm_offset).is_none() };
                crate::serial_println!("[KFP2A2] withdrawal va={} backing={:?} absent={}", entry.address, entry.backing, absent);
                assert!(absent);
            }

            if crate::memory::tlb::invalidate_retirement_range(identity, chunk_start, chunk_pages, &retirement).is_err() {
                munmap_invariant_failure("validated chunk was rejected by shootdown");
            }
            #[cfg(all(debug_assertions, feature = "kfp2a2_fault_test"))]
            crate::memory::tlb::ack_test_receipt(&retirement);

            crate::memory::swap::untrack_range(
                sched.current_memory_owner().pid,
                chunk_start,
                chunk_start + chunk_pages * 4096,
            );

            // The synchronous acknowledgement above is the ownership-release
            // barrier: no CPU may retain a translation to any frame freed here.
            for (release_index, removed_page) in removed[..removed_len].iter().enumerate() {
                match removed_page.ownership {
                    RemovedOwnership::Present(frame) => {
                        pmm.free_frame(frame);
                    }
                    RemovedOwnership::Swapped(block_id) => {
                        let hhdm_offset =
                            VirtAddr::new(crate::HHDM_REQ.response().expect("no hhdm").offset);
                        if crate::memory::zram::discard_block(block_id, pmm, hhdm_offset).is_err() {
                            munmap_invariant_failure("preflighted ZRAM block disappeared");
                        }
                        MUNMAP_SWAPPED_RELEASED.fetch_add(1, Ordering::Relaxed);
                    }
                    RemovedOwnership::Empty => {
                        munmap_invariant_failure("empty bounded removal record");
                    }
                }
                assert!(crate::memory::retirement::OWNERS.lock().released(&retirement, release_index));
            }
            for removed_page in &removed[..removed_len] {
                let page =
                    Page::<Size4KiB>::from_start_address(VirtAddr::new(removed_page.address))
                        .unwrap_or_else(|_| {
                            munmap_invariant_failure("removed page lost alignment")
                        });
                unsafe {
                    sched
                        .current_process_mut()
                        .address_space
                        .reclaim_empty_tables_for_page(page, pmm, hhdm_offset);
                }
            }
            assert!(crate::memory::retirement::OWNERS.lock().next_chunk(&retirement));
        }
        chunk_start += chunk_pages * 4096;
    }
    assert!(crate::memory::retirement::OWNERS.lock().finish(&retirement));

    sched.current_process().address_space.commit_unmap(plan);
    sched.cancel_futex_range(identity, addr, end);
    if !unsafe {
        sched
            .current_process()
            .address_space
            .validate_ledger_ptes(hhdm_offset)
    } {
        munmap_invariant_failure("ledger/PTE validation failed after commit");
    }

    MUNMAP_SUCCESSES.fetch_add(1, Ordering::Relaxed);
    MUNMAP_PAGES.fetch_add(effects.pages_covered, Ordering::Relaxed);
    MUNMAP_FULL_REGIONS.fetch_add(effects.full_regions, Ordering::Relaxed);
    MUNMAP_PREFIX_REGIONS.fetch_add(effects.prefix_regions, Ordering::Relaxed);
    MUNMAP_SUFFIX_REGIONS.fetch_add(effects.suffix_regions, Ordering::Relaxed);
    MUNMAP_MIDDLE_SPLITS.fetch_add(effects.middle_splits, Ordering::Relaxed);
    MUNMAP_HOLE_PAGES.fetch_add(effects.hole_pages, Ordering::Relaxed);
    debug_assert_eq!(effects.pages_covered + effects.hole_pages, page_count);
    Ok(())
}

fn expected_anonymous_leaf(
    address_space: &crate::process::address_space::AddressSpace,
    address: u64,
    region: MappingRegion,
    pmm: &PhysicalMemoryManager,
    hhdm_offset: VirtAddr,
) -> Result<Option<ExpectedMapping>, MmapError> {
    let owner = match (region.kind, region.policy, region.backing) {
        (
            MappingKind::Anonymous | MappingKind::Brk,
            policy,
            RegionBacking::AnonymousOwner(owner),
        ) if policy.contains(RegionPolicy::MAY_UNMAP) => owner,
        _ => return invariant_rejection(),
    };
    let page = Page::<Size4KiB>::from_start_address(VirtAddr::new(address))
        .map_err(|_| MmapError::InternalInvariant)?;
    let Some((frame_or_marker, flags)) = (unsafe { address_space.lookup_entry(page, hhdm_offset) })
    else {
        if region.policy.contains(RegionPolicy::DEMAND) {
            return Ok(None);
        }
        return invariant_rejection();
    };
    if flags.contains(PageTableFlags::PRESENT) {
        let user_accessible = flags.contains(PageTableFlags::USER_ACCESSIBLE);
        if user_accessible != (region.protection != RegionProtection::NONE)
            || pmm.owner_of(frame_or_marker) != Some(owner)
        {
            return invariant_rejection();
        }
        if let Ok(actual) =
            crate::process::address_space::AddressSpace::protection_from_pte_flags(flags)
        {
            if actual != region.protection {
                return invariant_rejection();
            }
        } else {
            return invariant_rejection();
        }
        Ok(Some(ExpectedMapping::Present {
            frame: frame_or_marker,
            flags,
        }))
    } else {
        let Some(block_id) = (unsafe { address_space.swapped_block_id(page, hhdm_offset) }) else {
            return invariant_rejection();
        };
        if !crate::memory::zram::block_exists(block_id) {
            return invariant_rejection();
        }
        Ok(Some(ExpectedMapping::Swapped { block_id }))
    }
}

fn invariant_rejection<T>() -> Result<T, MmapError> {
    MUNMAP_INVARIANT_FAILURES.fetch_add(1, Ordering::Relaxed);
    Err(MmapError::InternalInvariant)
}

fn munmap_invariant_failure(message: &str) -> ! {
    MUNMAP_INVARIANT_FAILURES.fetch_add(1, Ordering::Relaxed);
    panic!("MM-2D munmap invariant failure: {message}");
}

#[cfg(all(debug_assertions, feature = "helios_runtime_mmap"))]
fn mmap_fault_fixture(sched: &Scheduler, marker: &str) -> bool {
    let process = sched.current_process();
    process.name_str() == "mmap-probe" && process.env.get("M") == Some(marker)
}

/// Change protection on fully present, policy-authorized anonymous mappings.
pub fn sys_mprotect(
    addr: u64,
    length: u64,
    prot: u32,
    pmm: &PhysicalMemoryManager,
    sched: &mut Scheduler,
) -> Result<(), MmapError> {
    MPROTECT_REQUESTS.fetch_add(1, Ordering::Relaxed);
    if addr & 0xfff != 0 {
        return Err(MmapError::InvalidAddress);
    }
    crate::memory::user::UserRange::new(addr, 0).map_err(|_| MmapError::InvalidAddress)?;
    let protection = match normalize_protection(prot) {
        Err(MmapError::PermissionDenied) => {
            crate::memory::security::note_rwx_mapping_rejected();
            MPROTECT_WX_REJECTIONS.fetch_add(1, Ordering::Relaxed);
            return Err(MmapError::PermissionDenied);
        }
        result => result?,
    };
    // Linux performs the aligned zero-length fast path without consulting the
    // mapping. We still require a supported protection request above.
    if length == 0 {
        MPROTECT_NOOPS.fetch_add(1, Ordering::Relaxed);
        return Ok(());
    }

    let (page_count, span) = checked_page_layout(length).map_err(|_| MmapError::InvalidAddress)?;
    let span_usize = usize::try_from(span).map_err(|_| MmapError::InvalidAddress)?;
    crate::memory::user::UserRange::new(addr, span_usize).map_err(|_| MmapError::InvalidAddress)?;
    let end = addr.checked_add(span).ok_or(MmapError::InvalidAddress)?;
    let hhdm_offset = crate::HHDM_REQ
        .response()
        .map(|response| VirtAddr::new(response.offset))
        .ok_or(MmapError::InternalInvariant)?;

    // Resolve full ledger coverage and policy before inspecting a leaf. This
    // walk does not retain the terminal ledger lock.
    let mut covered = addr;
    let mut ledger_change = false;
    while covered < end {
        let Some(region) = sched.current_process().address_space.lookup_region(covered) else {
            MPROTECT_HOLE_REJECTIONS.fetch_add(1, Ordering::Relaxed);
            return Err(MmapError::NoMemory);
        };
        if region.kind != MappingKind::Anonymous
            || !region.policy.contains(RegionPolicy::MAY_CHANGE_PROTECTION)
        {
            MPROTECT_PROTECTED_REJECTIONS.fetch_add(1, Ordering::Relaxed);
            return Err(MmapError::Protected);
        }
        ledger_change |= region.protection != protection;
        covered = region.end.min(end);
    }

    // Retain the complete expected leaf image so no PTE changes until every
    // page is proven present, user accessible, owned by the anonymous mapping,
    // and consistent with its ledger protection.
    let page_count_usize = usize::try_from(page_count).map_err(|_| MmapError::InvalidAddress)?;
    let mut pages = Vec::new();
    pages
        .try_reserve_exact(page_count_usize)
        .map_err(|_| MmapError::NoMemory)?;
    let mut address = addr;
    while address < end {
        let region = sched
            .current_process()
            .address_space
            .lookup_region(address)
            .ok_or_else(|| {
                MPROTECT_INVARIANT_FAILURES.fetch_add(1, Ordering::Relaxed);
                MmapError::InternalInvariant
            })?;
        pages.push(preflight_protection_leaf(
            &sched.current_process().address_space,
            address,
            region,
            pmm,
            hhdm_offset,
        )?);
        address = address.checked_add(4096).ok_or(MmapError::InvalidAddress)?;
    }

    // Construct the exact post-change ledger image only after the complete PTE
    // preflight. Capacity and policy errors still occur before the first write.
    let plan = match sched
        .current_process()
        .address_space
        .preflight_protect(addr, end, protection)
    {
        Ok(plan) => plan,
        Err(MappingError::NotMapped) => {
            MPROTECT_HOLE_REJECTIONS.fetch_add(1, Ordering::Relaxed);
            return Err(MmapError::NoMemory);
        }
        Err(MappingError::ProtectedRegion) => {
            MPROTECT_PROTECTED_REJECTIONS.fetch_add(1, Ordering::Relaxed);
            return Err(MmapError::Protected);
        }
        Err(MappingError::LedgerCapacityExhausted) => {
            MPROTECT_CAPACITY_FAILURES.fetch_add(1, Ordering::Relaxed);
            return Err(MmapError::NoMemory);
        }
        Err(_) => return mprotect_invariant_rejection(),
    };

    let changed_pages = pages
        .iter()
        .flatten()
        .filter(|page| page.old != protection)
        .count();
    if !ledger_change {
        MPROTECT_NOOPS.fetch_add(1, Ordering::Relaxed);
        return Ok(());
    }

    #[cfg(all(debug_assertions, feature = "helios_runtime_mmap"))]
    let inject_update = changed_pages == 2
        && length == 8192
        && protection == RegionProtection::READ_ONLY
        && mmap_fault_fixture(sched, "P");
    let identity = sched.current_process().address_space.identity();
    let mut transitions = ProtectionTransitions::default();
    for chunk in pages.chunks(MPROTECT_CHUNK_PAGES) {
        let mut chunk_changed = false;
        for expected in chunk.iter().flatten() {
            if expected.old == protection {
                continue;
            }
            let page = Page::<Size4KiB>::from_start_address(VirtAddr::new(expected.address))
                .unwrap_or_else(|_| mprotect_invariant_failure("preflighted page lost alignment"));
            #[cfg(all(debug_assertions, feature = "helios_runtime_mmap"))]
            if inject_update && chunk_changed {
                // Inject before the second write, with the first already applied
                // and the staged ledger still unpublished.
                let first = chunk.iter().flatten().find(|entry| entry.old != protection).unwrap();
                let first_page = Page::containing_address(VirtAddr::new(first.address));
                let space = &mut sched.current_process_mut().address_space;
                let (_, flags) = unsafe { space.lookup_entry(first_page, hhdm_offset) }
                    .expect("updated leaf must exist");
                let permission_bits =
                    PageTableFlags::USER_ACCESSIBLE | PageTableFlags::WRITABLE | PageTableFlags::NO_EXECUTE;
                let applied = crate::process::address_space::AddressSpace::protection_to_pte_flags(protection)
                    .expect("preflighted protection");
                assert_eq!(flags & permission_bits, applied & permission_bits);
                let restored = unsafe {
                    space.update_permissions_expected(
                        first_page, first.frame, flags, first.old, hhdm_offset,
                    )
                }.expect("fixture rollback must restore the first leaf");
                assert!(restored);
                crate::memory::tlb::invalidate_range(identity, first.address, 1)
                    .expect("fixture rollback shootdown");
                for original in [first, expected] {
                    let page = Page::containing_address(VirtAddr::new(original.address));
                    let (frame, flags) = unsafe { space.lookup_entry(page, hhdm_offset) }
                        .expect("original leaf must exist");
                    assert_eq!(frame, original.frame);
                    assert_eq!(flags & permission_bits, original.flags & permission_bits);
                }
                assert!(unsafe { space.validate_ledger_ptes(hhdm_offset) });
                crate::serial_println!(
                    "[ANON-MMAP-FAULT] pid={} protect leaf=2 applied=1 restored=1 ptes=RW,RW metadata=RW errno=-12",
                    sched.current_process().pid
                );
                return Err(MmapError::NoMemory);
            }
            let updated = unsafe {
                sched
                    .current_process_mut()
                    .address_space
                    .update_permissions_expected(
                        page,
                        expected.frame,
                        expected.flags,
                        protection,
                        hhdm_offset,
                    )
            }
            .unwrap_or_else(|_| {
                mprotect_invariant_failure("leaf changed after complete preflight")
            });
            if !updated {
                mprotect_invariant_failure("changed protection produced identical PTE flags");
            }
            note_transition(&mut transitions, expected.old, protection);
            chunk_changed = true;
        }
        if chunk_changed {
            let first = chunk
                .iter()
                .flatten()
                .next()
                .expect("changed chunk has a present page")
                .address;
            let last = chunk
                .iter()
                .flatten()
                .last()
                .expect("changed chunk has a present page")
                .address;
            let pages = (last - first) / 4096 + 1;
            if crate::memory::tlb::invalidate_range(identity, first, pages).is_err() {
                mprotect_invariant_failure("validated chunk was rejected by shootdown");
            }
        }
    }

    sched.current_process().address_space.commit_protect(plan);
    if !unsafe {
        sched
            .current_process()
            .address_space
            .validate_ledger_ptes(hhdm_offset)
    } {
        mprotect_invariant_failure("ledger/PTE validation failed after commit");
    }

    MPROTECT_SUCCESSES.fetch_add(1, Ordering::Relaxed);
    MPROTECT_PAGES_CHANGED.fetch_add(changed_pages as u64, Ordering::Relaxed);
    MPROTECT_RW_TO_R.fetch_add(transitions.rw_to_r, Ordering::Relaxed);
    MPROTECT_RW_TO_RX.fetch_add(transitions.rw_to_rx, Ordering::Relaxed);
    MPROTECT_R_TO_RW.fetch_add(transitions.r_to_rw, Ordering::Relaxed);
    MPROTECT_R_TO_RX.fetch_add(transitions.r_to_rx, Ordering::Relaxed);
    MPROTECT_RX_TO_R.fetch_add(transitions.rx_to_r, Ordering::Relaxed);
    MPROTECT_RX_TO_RW.fetch_add(transitions.rx_to_rw, Ordering::Relaxed);
    Ok(())
}

fn preflight_protection_leaf(
    address_space: &crate::process::address_space::AddressSpace,
    address: u64,
    region: MappingRegion,
    pmm: &PhysicalMemoryManager,
    hhdm_offset: VirtAddr,
) -> Result<Option<ProtectionPage>, MmapError> {
    let owner = match (region.kind, region.policy, region.backing) {
        (MappingKind::Anonymous, policy, RegionBacking::AnonymousOwner(owner))
            if policy.contains(RegionPolicy::MAY_CHANGE_PROTECTION) =>
        {
            owner
        }
        _ => return mprotect_invariant_rejection(),
    };
    let page = Page::<Size4KiB>::from_start_address(VirtAddr::new(address))
        .map_err(|_| MmapError::InternalInvariant)?;
    let Some((frame_or_marker, flags)) = (unsafe { address_space.lookup_entry(page, hhdm_offset) })
    else {
        if region.policy.contains(RegionPolicy::DEMAND) {
            return Ok(None);
        }
        return mprotect_invariant_rejection();
    };
    if !flags.contains(PageTableFlags::PRESENT) {
        if let Some(block_id) = unsafe { address_space.swapped_block_id(page, hhdm_offset) } {
            if !crate::memory::zram::block_exists(block_id) {
                return mprotect_invariant_rejection();
            }
            MPROTECT_SWAPPED_REJECTIONS.fetch_add(1, Ordering::Relaxed);
            return Err(MmapError::SwappedUnsupported);
        }
        return mprotect_invariant_rejection();
    }
    let user_accessible = flags.contains(PageTableFlags::USER_ACCESSIBLE);
    if user_accessible != (region.protection != RegionProtection::NONE)
        || pmm.owner_of(frame_or_marker) != Some(owner)
    {
        return mprotect_invariant_rejection();
    }
    let actual = crate::process::address_space::AddressSpace::protection_from_pte_flags(flags)
        .map_err(|_| {
            MPROTECT_INVARIANT_FAILURES.fetch_add(1, Ordering::Relaxed);
            MmapError::InternalInvariant
        })?;
    if actual != region.protection {
        return mprotect_invariant_rejection();
    }
    Ok(Some(ProtectionPage {
        address,
        frame: frame_or_marker,
        flags,
        old: actual,
    }))
}

fn note_transition(
    transitions: &mut ProtectionTransitions,
    old: RegionProtection,
    new: RegionProtection,
) {
    match (old, new) {
        (RegionProtection::READ_WRITE, RegionProtection::READ_ONLY) => transitions.rw_to_r += 1,
        (RegionProtection::READ_WRITE, RegionProtection::READ_EXECUTE) => transitions.rw_to_rx += 1,
        (RegionProtection::READ_ONLY, RegionProtection::READ_WRITE) => transitions.r_to_rw += 1,
        (RegionProtection::READ_ONLY, RegionProtection::READ_EXECUTE) => transitions.r_to_rx += 1,
        (RegionProtection::READ_EXECUTE, RegionProtection::READ_ONLY) => transitions.rx_to_r += 1,
        (RegionProtection::READ_EXECUTE, RegionProtection::READ_WRITE) => transitions.rx_to_rw += 1,
        (RegionProtection::NONE, _) | (_, RegionProtection::NONE) => {}
        _ => mprotect_invariant_failure("unsupported protection transition"),
    }
}

fn mprotect_invariant_rejection<T>() -> Result<T, MmapError> {
    MPROTECT_INVARIANT_FAILURES.fetch_add(1, Ordering::Relaxed);
    Err(MmapError::InternalInvariant)
}

fn mprotect_invariant_failure(message: &str) -> ! {
    MPROTECT_INVARIANT_FAILURES.fetch_add(1, Ordering::Relaxed);
    panic!("MM-2E mprotect invariant failure: {message}");
}

/// Remap memory (stub for now)
pub fn sys_mremap(
    _old_addr: u64,
    _old_size: u64,
    _new_size: u64,
    _flags: u32,
) -> Result<u64, MmapError> {
    // TODO: Implement mremap
    Err(MmapError::InvalidFlags)
}
