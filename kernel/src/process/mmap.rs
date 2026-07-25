use super::mm2a_plan::{checked_page_layout, DeferredCursor};
use crate::memory::pmm::PhysicalMemoryManager;
use crate::process::address_space::{
    ExpectedMapping, MappingError, OwnershipTransition, ReplacementMapping,
};
use crate::process::region::{
    MappingKind, MappingRegion, RegionBacking, RegionPolicy, RegionProtection,
};
use crate::sched::Scheduler;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};
use x86_64::structures::paging::{Page, PageTableFlags, PhysFrame, Size4KiB};
use x86_64::{PhysAddr, VirtAddr};

// mmap flags
pub const MAP_PRIVATE: u32 = 0x02;
pub const MAP_ANONYMOUS: u32 = 0x20;
pub const MAP_FIXED: u32 = 0x10;
pub const MAP_FIXED_NOREPLACE: u32 = 0x100000;

// mprotect flags
pub const PROT_NONE: u32 = 0;
pub const PROT_READ: u32 = 0x1;
pub const PROT_WRITE: u32 = 0x2;
pub const PROT_EXEC: u32 = 0x4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MmapError {
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

/// Map anonymous memory in the current process.
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
    // Backed mappings are not implemented.  Reject descriptor/offset use so
    // no caller can mistake this for a file or device mapping interface.
    if fd != -1 || offset != 0 {
        return Err(MmapError::InvalidFlags);
    }
    map_anonymous_kind(
        addr,
        length,
        prot,
        flags,
        pmm,
        sched,
        MappingKind::Anonymous,
    )
}

pub fn map_brk(
    addr: u64,
    length: u64,
    pmm: &mut PhysicalMemoryManager,
    sched: &mut Scheduler,
) -> Result<u64, MmapError> {
    map_anonymous_kind(
        addr,
        length,
        PROT_READ | PROT_WRITE,
        MAP_FIXED | MAP_PRIVATE | MAP_ANONYMOUS,
        pmm,
        sched,
        MappingKind::Brk,
    )
}

fn map_anonymous_kind(
    addr: u64,
    length: u64,
    prot: u32,
    flags: u32,
    pmm: &mut PhysicalMemoryManager,
    sched: &mut Scheduler,
    kind: MappingKind,
) -> Result<u64, MmapError> {
    let protection = match normalize_protection(prot) {
        Err(MmapError::PermissionDenied) => {
            crate::memory::security::note_rwx_mapping_rejected();
            return Err(MmapError::PermissionDenied);
        }
        result => result?,
    };

    if flags & !(MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED | MAP_FIXED_NOREPLACE) != 0 {
        return Err(MmapError::InvalidFlags);
    }
    if flags & (MAP_FIXED | MAP_FIXED_NOREPLACE) == (MAP_FIXED | MAP_FIXED_NOREPLACE) {
        return Err(MmapError::InvalidFlags);
    }

    // The current native surface is deliberately limited to private,
    // anonymous mappings.  Require both bits instead of accepting an
    // incomplete request as private anonymous memory.
    if flags & (MAP_PRIVATE | MAP_ANONYMOUS) != (MAP_PRIVATE | MAP_ANONYMOUS) {
        return Err(MmapError::InvalidFlags);
    }

    // Base of the anonymous mmap region for hint-less mappings.
    const MMAP_REGION_BASE: u64 = 0x10_0000_0000u64;

    let (page_count, span) = checked_page_layout(length).map_err(|_| MmapError::InvalidAddress)?;

    // Determine the address to map at. For anonymous mappings without a
    // fixed address we hand out fresh VA ranges from a per-process bump
    // cursor; returning a fixed base for every call made successive mmaps
    // alias the same range and corrupted userspace allocators (e.g. musl
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
            DeferredCursor::new(sched.current_process().mmap_next, MMAP_REGION_BASE, span)
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
    let pid = sched.current_process().pid;
    let hhdm_offset = crate::HHDM_REQ
        .response()
        .map(|response| VirtAddr::new(response.offset))
        .ok_or(MmapError::InternalInvariant)?;

    if !replace_fixed {
        for i in 0..page_count {
            let page_vaddr = VirtAddr::new(map_addr + i * 4096);
            let page = Page::from_start_address(page_vaddr).map_err(|_| MmapError::InvalidAddress)?;
            let process = sched
                .process_mut_by_pid(pid)
                .ok_or(MmapError::InternalInvariant)?;
            if unsafe { process.address_space.is_occupied(page, hhdm_offset) } {
                crate::process::address_space::note_mapping_collision();
                return Err(MmapError::AlreadyMapped);
            }
        }
    }

    let policy = match kind {
        MappingKind::Anonymous => RegionPolicy::MAY_UNMAP
            .union(RegionPolicy::MAY_REPLACE)
            .union(RegionPolicy::MAY_CHANGE_PROTECTION)
            .union(RegionPolicy::OWNER_MANAGED),
        MappingKind::Brk => RegionPolicy::MAY_REPLACE.union(RegionPolicy::OWNER_MANAGED),
        _ => return Err(MmapError::InternalInvariant),
    };
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
    let replacement_plan = if replace_fixed {
        Some(
            sched
                .current_process()
                .address_space
                .preflight_replace(region)
                .map_err(mapping_error)?,
        )
    } else {
        None
    };
    let reservation = if replacement_plan.is_none() {
        Some(
            sched
                .current_process()
                .address_space
                .preflight_region(region)
                .map_err(mapping_error)?,
        )
    } else {
        None
    };

    let page_count_usize = usize::try_from(page_count).map_err(|_| MmapError::InvalidAddress)?;
    let mut installed: Vec<InstalledAnonymousPage> = Vec::new();
    installed.try_reserve_exact(page_count_usize).map_err(|_| {
        if let Some(reservation) = reservation {
            sched.current_process().address_space.cancel_region(reservation);
        }
        MmapError::NoMemory
    })?;
    if crate::memory::swap::reserve_candidates(page_count_usize).is_err() {
        if let Some(reservation) = reservation {
            sched.current_process().address_space.cancel_region(reservation);
        }
        return Err(MmapError::NoMemory);
    }
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
        );
        let frame_addr = match pmm.alloc_frame_owned(pid as u32) {
            Some(addr) => addr,
            None => {
                crate::process::address_space::note_frame_allocation_failure();
                rollback_anonymous(&mut installed, pid, pmm, sched, hhdm_offset);
                if let Some(reservation) = reservation {
                    sched.current_process().address_space.cancel_region(reservation);
                }
                return Err(MmapError::NoMemory);
            }
        };
        if !crate::memory::security::sanitize_user_frame(frame_addr, hhdm_offset) {
            pmm.free_frame(frame_addr);
            rollback_anonymous(&mut installed, pid, pmm, sched, hhdm_offset);
            if let Some(reservation) = reservation {
                sched.current_process().address_space.cancel_region(reservation);
            }
            return Err(MmapError::NoMemory);
        }
        let frame = unsafe { PhysFrame::from_start_address_unchecked(frame_addr) };

        let proc = match sched.process_mut_by_pid(pid) {
            Some(process) => process,
            None => {
                pmm.free_frame(frame_addr);
                rollback_anonymous(&mut installed, pid, pmm, sched, hhdm_offset);
                if let Some(reservation) = reservation {
                    sched.current_process().address_space.cancel_region(reservation);
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
                    Ok(expected) => Some(expected),
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
            None => unsafe { proc.address_space.map_page(page, frame, page_flags, pmm, hhdm_offset) },
        };
        if let Err(error) = install_result {
            pmm.free_frame(frame_addr);
            rollback_anonymous(&mut installed, pid, pmm, sched, hhdm_offset);
            if let Some(reservation) = reservation {
                sched.current_process().address_space.cancel_region(reservation);
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

    if let Some(plan) = replacement_plan {
        sched.current_process().address_space.commit_replace(plan);
    } else if let Some(reservation) = reservation {
        if let Err(error) = sched.current_process().address_space.commit_region(reservation) {
            rollback_anonymous(&mut installed, pid, pmm, sched, hhdm_offset);
            return Err(mapping_error(error));
        }
    }

    for installed_page in &installed {
        if let Some(old) = installed_page.replaced {
            release_replaced_ownership(old, pmm, hhdm_offset);
        }
        crate::memory::swap::track_anon(pid, installed_page.page.start_address(), installed_page.frame);
    }
    if let Some(cursor) = deferred_cursor {
        cursor.commit(&mut sched.current_process_mut().mmap_next);
    }

    Ok(map_addr)
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
    while let Some(installed_page) = installed.pop() {
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
        if restored {
            pmm.free_frame(installed_page.frame);
        }
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
) -> Result<ExpectedMapping, MmapError> {
    let owner = match (region.kind, region.policy, region.backing) {
        (MappingKind::Anonymous | MappingKind::Brk, policy, RegionBacking::AnonymousOwner(owner))
            if policy.contains(RegionPolicy::MAY_REPLACE) =>
        {
            owner
        }
        _ => return Err(MmapError::Protected),
    };
    let page = Page::<Size4KiB>::from_start_address(VirtAddr::new(address))
        .map_err(|_| MmapError::InternalInvariant)?;
    let Some((frame_or_marker, flags)) = (unsafe { address_space.lookup_entry(page, hhdm_offset) })
    else {
        return Err(MmapError::InternalInvariant);
    };
    if flags.contains(PageTableFlags::PRESENT) {
        if flags.contains(PageTableFlags::USER_ACCESSIBLE) != (region.protection != RegionProtection::NONE)
            || pmm.owner_of(frame_or_marker) != Some(owner)
            || crate::process::address_space::AddressSpace::protection_from_pte_flags(flags)
                .ok()
                != Some(region.protection)
        {
            return Err(MmapError::InternalInvariant);
        }
        Ok(ExpectedMapping::Present {
            frame: frame_or_marker,
            flags,
        })
    } else {
        let Some(block_id) = (unsafe { address_space.swapped_block_id(page, hhdm_offset) }) else {
            return Err(MmapError::InternalInvariant);
        };
        if !crate::memory::zram::block_exists(block_id) {
            return Err(MmapError::InternalInvariant);
        }
        Ok(ExpectedMapping::Swapped { block_id })
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
            expected_anonymous_leaf(
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
                Ok(expected) => expected,
                Err(_) => {
                    panic!("MM-2D munmap invariant failure: leaf changed after complete preflight")
                }
            };
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
            if crate::memory::tlb::invalidate_range(identity, chunk_start, chunk_pages).is_err() {
                munmap_invariant_failure("validated chunk was rejected by shootdown");
            }

            // The synchronous acknowledgement above is the ownership-release
            // barrier: no CPU may retain a translation to any frame freed here.
            for removed_page in &removed[..removed_len] {
                match removed_page.ownership {
                    RemovedOwnership::Present(frame) => {
                        crate::memory::swap::untrack(frame);
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
        }
        chunk_start += chunk_pages * 4096;
    }

    sched.current_process().address_space.commit_unmap(plan);
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
) -> Result<ExpectedMapping, MmapError> {
    let owner = match (region.kind, region.policy, region.backing) {
        (MappingKind::Anonymous, policy, RegionBacking::AnonymousOwner(owner))
            if policy.contains(RegionPolicy::MAY_UNMAP) =>
        {
            owner
        }
        _ => return invariant_rejection(),
    };
    let page = Page::<Size4KiB>::from_start_address(VirtAddr::new(address))
        .map_err(|_| MmapError::InternalInvariant)?;
    let Some((frame_or_marker, flags)) = (unsafe { address_space.lookup_entry(page, hhdm_offset) })
    else {
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
        Ok(ExpectedMapping::Present {
            frame: frame_or_marker,
            flags,
        })
    } else {
        let Some(block_id) = (unsafe { address_space.swapped_block_id(page, hhdm_offset) }) else {
            return invariant_rejection();
        };
        if !crate::memory::zram::block_exists(block_id) {
            return invariant_rejection();
        }
        Ok(ExpectedMapping::Swapped { block_id })
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

    let changed_pages = pages.iter().filter(|page| page.old != protection).count();
    if changed_pages == 0 {
        MPROTECT_NOOPS.fetch_add(1, Ordering::Relaxed);
        return Ok(());
    }

    let identity = sched.current_process().address_space.identity();
    let mut transitions = ProtectionTransitions::default();
    for chunk in pages.chunks(MPROTECT_CHUNK_PAGES) {
        let mut chunk_changed = false;
        for expected in chunk {
            if expected.old == protection {
                continue;
            }
            let page = Page::<Size4KiB>::from_start_address(VirtAddr::new(expected.address))
                .unwrap_or_else(|_| mprotect_invariant_failure("preflighted page lost alignment"));
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
        if chunk_changed
            && crate::memory::tlb::invalidate_range(identity, chunk[0].address, chunk.len() as u64)
                .is_err()
        {
            mprotect_invariant_failure("validated chunk was rejected by shootdown");
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
) -> Result<ProtectionPage, MmapError> {
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
    Ok(ProtectionPage {
        address,
        frame: frame_or_marker,
        flags,
        old: actual,
    })
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
        (RegionProtection::NONE, _)
        | (_, RegionProtection::NONE) => {}
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
