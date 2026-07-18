use super::mm2a_plan::{checked_page_layout, DeferredCursor};
use crate::memory::pmm::PhysicalMemoryManager;
use crate::process::address_space::{ExpectedMapping, MappingError};
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

const MUNMAP_CHUNK_PAGES: u64 = crate::memory::tlb::RANGE_FLUSH_PAGE_THRESHOLD;

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
}

fn prot_to_protection(prot: u32) -> Result<RegionProtection, MmapError> {
    RegionProtection::new(
        prot & PROT_READ != 0,
        prot & PROT_WRITE != 0,
        prot & PROT_EXEC != 0,
    )
    .map_err(|_| MmapError::PermissionDenied)
}

/// Map anonymous memory in the current process.
pub fn sys_mmap(
    addr: u64,
    length: u64,
    prot: u32,
    flags: u32,
    _fd: i32,
    _offset: u64,
    pmm: &mut PhysicalMemoryManager,
    sched: &mut Scheduler,
) -> Result<u64, MmapError> {
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
    if (prot & (PROT_WRITE | PROT_EXEC)) == (PROT_WRITE | PROT_EXEC) {
        crate::memory::security::note_rwx_mapping_rejected();
        return Err(MmapError::PermissionDenied);
    }
    if prot & !(PROT_READ | PROT_WRITE | PROT_EXEC) != 0 {
        return Err(MmapError::InvalidProt);
    }

    if flags & !(MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED) != 0 {
        return Err(MmapError::InvalidFlags);
    }

    // Only support anonymous mappings for now
    if (flags & MAP_ANONYMOUS) == 0 {
        return Err(MmapError::InvalidFlags);
    }

    // Base of the anonymous mmap region for hint-less / hinted mappings.
    const MMAP_REGION_BASE: u64 = 0x10_0000_0000u64;

    let (page_count, span) = checked_page_layout(length).map_err(|_| MmapError::InvalidAddress)?;

    // Determine the address to map at. For anonymous mappings without a
    // fixed address we hand out fresh VA ranges from a per-process bump
    // cursor; returning a fixed base for every call made successive mmaps
    // alias the same range and corrupted userspace allocators (e.g. musl
    // mallocng), which then page-faulted on the first free().
    let deferred_cursor = if (flags & MAP_FIXED) != 0 {
        None
    } else {
        Some(
            DeferredCursor::new(sched.current_process().mmap_next, MMAP_REGION_BASE, span)
                .map_err(|_| MmapError::InvalidAddress)?,
        )
    };
    let map_addr = if (flags & MAP_FIXED) != 0 {
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

    let protection = prot_to_protection(prot)?;
    let page_flags =
        crate::process::address_space::AddressSpace::protection_to_pte_flags(protection)
            .map_err(|_| MmapError::PermissionDenied)?;

    // Map all the pages
    let pid = sched.current_process().pid;
    let hhdm_offset = crate::HHDM_REQ
        .response()
        .map(|response| VirtAddr::new(response.offset))
        .ok_or(MmapError::InternalInvariant)?;

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

    let policy = match kind {
        MappingKind::Anonymous => RegionPolicy::MAY_UNMAP
            .union(RegionPolicy::MAY_CHANGE_PROTECTION)
            .union(RegionPolicy::OWNER_MANAGED),
        MappingKind::Brk => RegionPolicy::OWNER_MANAGED,
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
    let reservation = sched
        .current_process()
        .address_space
        .preflight_region(region)
        .map_err(mapping_error)?;

    let page_count_usize = usize::try_from(page_count).map_err(|_| MmapError::InvalidAddress)?;
    let mut installed: Vec<(Page<Size4KiB>, x86_64::PhysAddr)> = Vec::new();
    installed.try_reserve_exact(page_count_usize).map_err(|_| {
        sched
            .current_process()
            .address_space
            .cancel_region(reservation);
        MmapError::NoMemory
    })?;
    if crate::memory::swap::reserve_candidates(page_count_usize).is_err() {
        sched
            .current_process()
            .address_space
            .cancel_region(reservation);
        return Err(MmapError::NoMemory);
    }
    for i in 0..page_count {
        let page_vaddr = VirtAddr::new(
            map_addr
                .checked_add(i.checked_mul(4096).ok_or(MmapError::InvalidAddress)?)
                .ok_or(MmapError::InvalidAddress)?,
        );
        let page = Page::from_start_address(page_vaddr).map_err(|_| MmapError::InvalidAddress)?;

        let frame_addr = match pmm.alloc_frame_owned(pid as u32) {
            Some(addr) => addr,
            None => {
                crate::process::address_space::note_frame_allocation_failure();
                rollback_anonymous(&mut installed, pid, pmm, sched, hhdm_offset);
                sched
                    .current_process()
                    .address_space
                    .cancel_region(reservation);
                return Err(MmapError::NoMemory);
            }
        };
        if !crate::memory::security::sanitize_user_frame(frame_addr, hhdm_offset) {
            pmm.free_frame(frame_addr);
            rollback_anonymous(&mut installed, pid, pmm, sched, hhdm_offset);
            sched
                .current_process()
                .address_space
                .cancel_region(reservation);
            return Err(MmapError::NoMemory);
        }
        let frame = unsafe { PhysFrame::from_start_address_unchecked(frame_addr) };

        let proc = match sched.process_mut_by_pid(pid) {
            Some(process) => process,
            None => {
                pmm.free_frame(frame_addr);
                rollback_anonymous(&mut installed, pid, pmm, sched, hhdm_offset);
                sched
                    .current_process()
                    .address_space
                    .cancel_region(reservation);
                return Err(MmapError::InternalInvariant);
            }
        };
        if let Err(error) = unsafe {
            proc.address_space
                .map_page(page, frame, page_flags, pmm, hhdm_offset)
        } {
            pmm.free_frame(frame_addr);
            rollback_anonymous(&mut installed, pid, pmm, sched, hhdm_offset);
            sched
                .current_process()
                .address_space
                .cancel_region(reservation);
            return Err(match error {
                MappingError::AlreadyMapped => MmapError::AlreadyMapped,
                MappingError::FrameAllocationFailed | MappingError::PageTableAllocationFailed => {
                    MmapError::NoMemory
                }
                MappingError::PermissionRejected => MmapError::PermissionDenied,
                _ => MmapError::InternalInvariant,
            });
        }
        installed.push((page, frame_addr));
    }

    if let Err(error) = sched
        .current_process()
        .address_space
        .commit_region(reservation)
    {
        rollback_anonymous(&mut installed, pid, pmm, sched, hhdm_offset);
        return Err(mapping_error(error));
    }

    for (page, frame) in &installed {
        crate::memory::swap::track_anon(pid, page.start_address(), *frame);
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
    installed: &mut Vec<(Page<Size4KiB>, x86_64::PhysAddr)>,
    pid: usize,
    pmm: &mut PhysicalMemoryManager,
    sched: &mut Scheduler,
    hhdm_offset: VirtAddr,
) {
    if installed.is_empty() {
        return;
    }
    crate::process::address_space::note_mmap_rollback();
    while let Some((page, frame)) = installed.pop() {
        let unmapped = sched.process_mut_by_pid(pid).is_some_and(|process| unsafe {
            process
                .address_space
                .rollback_mapped_page(page, frame, pmm, hhdm_offset)
                .is_ok()
        });
        if unmapped {
            pmm.free_frame(frame);
        }
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
                        if crate::memory::zram::discard_block(block_id as usize).is_err() {
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
        if !flags.contains(PageTableFlags::USER_ACCESSIBLE)
            || pmm.owner_of(frame_or_marker) != Some(owner)
        {
            return invariant_rejection();
        }
        if let Ok(actual) =
            crate::process::address_space::AddressSpace::protection_from_pte_flags(flags)
        {
            if actual.writable() != region.protection.writable()
                || actual.executable() != region.protection.executable()
            {
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
        if !crate::memory::zram::block_exists(block_id as usize) {
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

/// Change memory protection (stub for now)
pub fn sys_mprotect(_addr: u64, _length: u64, _prot: u32) -> Result<(), MmapError> {
    Err(MmapError::Unsupported)
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
