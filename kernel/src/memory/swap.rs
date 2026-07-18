//! Anonymous-page reclaim and the fault-critical ZRAM swap lifecycle.

use super::pmm::PhysicalMemoryManager;
use super::zram::{self, SlotId, ZramError, ZRAM_BLOCK_SIZE};
use crate::process::address_space::{
    AddressSpace, ExpectedMapping, OwnershipTransition, ReplacementMapping,
};
use crate::process::region::{MappingKind, RegionBacking, RegionPolicy};
use crate::sched::Scheduler;
use alloc::collections::VecDeque;
use core::sync::atomic::{AtomicU64, Ordering};
use spin::Mutex;
use x86_64::{
    structures::paging::{Page, PageTableFlags, PhysFrame, Size4KiB},
    PhysAddr, VirtAddr,
};

const MIN_START_WATERMARK_PAGES: usize = 256;
const MAX_START_WATERMARK_PAGES: usize = 16 * 1024;
const MIN_TARGET_WATERMARK_PAGES: usize = 512;
const MAX_TARGET_WATERMARK_PAGES: usize = 32 * 1024;
const MAX_DIRECT_RECLAIM_PAGES: usize = 256;
const MAX_SCAN_SLOP: usize = 32;

static CANDIDATE_SCANS: AtomicU64 = AtomicU64::new(0);
static PAGES_RECLAIMED: AtomicU64 = AtomicU64::new(0);
static WATERMARK_ACTIVATIONS: AtomicU64 = AtomicU64::new(0);
static RECLAIM_NO_PROGRESS: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy)]
pub struct AnonFrame {
    pub pid: usize,
    pub vaddr: VirtAddr,
    pub frame: PhysAddr,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ReclaimDiagnostics {
    pub candidate_scans: u64,
    pub pages_reclaimed: u64,
    pub watermark_activations: u64,
    pub no_progress_events: u64,
    pub candidate_count: usize,
}

static CANDIDATES: Mutex<VecDeque<AnonFrame>> = Mutex::new(VecDeque::new());

pub fn reserve_candidates(additional: usize) -> Result<(), ()> {
    CANDIDATES.lock().try_reserve(additional).map_err(|_| ())
}

pub fn track_anon(pid: usize, vaddr: VirtAddr, frame: PhysAddr) {
    CANDIDATES.lock().push_back(AnonFrame { pid, vaddr, frame });
}

pub fn untrack(frame: PhysAddr) {
    CANDIDATES
        .lock()
        .retain(|candidate| candidate.frame != frame);
}

pub fn untrack_process(pid: usize) {
    CANDIDATES.lock().retain(|candidate| candidate.pid != pid);
}

pub fn candidate_count() -> usize {
    CANDIDATES.lock().len()
}

pub fn diagnostics() -> ReclaimDiagnostics {
    ReclaimDiagnostics {
        candidate_scans: CANDIDATE_SCANS.load(Ordering::Relaxed),
        pages_reclaimed: PAGES_RECLAIMED.load(Ordering::Relaxed),
        watermark_activations: WATERMARK_ACTIVATIONS.load(Ordering::Relaxed),
        no_progress_events: RECLAIM_NO_PROGRESS.load(Ordering::Relaxed),
        candidate_count: candidate_count(),
    }
}

/// Returns `(start_reclaim_below, stop_reclaim_at)` in free 4 KiB pages.
pub fn watermarks(total_pages: usize) -> (usize, usize) {
    let start = (total_pages / 64)
        .max(MIN_START_WATERMARK_PAGES)
        .min(MAX_START_WATERMARK_PAGES)
        .min(total_pages);
    let target = (total_pages / 32)
        .max(MIN_TARGET_WATERMARK_PAGES)
        .min(MAX_TARGET_WATERMARK_PAGES)
        .min(total_pages)
        .max(start);
    (start, target)
}

/// Compress a verified anonymous frame, publish a stale-safe swapped marker,
/// complete the MM-2B shootdown, and only then release the physical frame.
///
/// SAFETY: caller holds the owning scheduler/address-space and PMM locks.
pub unsafe fn swap_out_page(
    address_space: &mut AddressSpace,
    page: Page<Size4KiB>,
    frame: PhysAddr,
    hhdm_offset: VirtAddr,
    pmm: &mut PhysicalMemoryManager,
) -> Result<u64, ZramError> {
    let (_, flags) = match address_space.lookup_entry(page, hhdm_offset) {
        Some(entry) if entry.0 == frame && entry.1.contains(PageTableFlags::PRESENT) => entry,
        _ => return Err(ZramError::InvalidBlock),
    };
    let src = &*((hhdm_offset + frame.as_u64()).as_ptr::<[u8; ZRAM_BLOCK_SIZE]>());
    let identity = address_space.identity();
    let selection_key = identity.generation.wrapping_mul(0x9e37_79b9)
        ^ page.start_address().as_u64().rotate_right(12)
        ^ crate::sched::current_cpu_id() as u64;
    let slot = zram::write_page(src, flags.bits(), selection_key)?;

    if address_space
        .replace_mapping(
            page,
            ExpectedMapping::Present { frame, flags },
            ReplacementMapping::Swapped {
                block_id: slot.raw(),
            },
            OwnershipTransition::ReleaseOldFrame,
            pmm,
            hhdm_offset,
        )
        .is_err()
    {
        let _ = zram::discard(slot);
        return Err(ZramError::InvalidBlock);
    }
    untrack(frame);
    Ok(slot.raw())
}

/// Restore and validate one page before publishing its new present mapping.
/// A read/decompression/checksum/PTE failure leaves the original slot live.
///
/// SAFETY: caller holds the owning scheduler/address-space and PMM locks.
pub unsafe fn swap_in_page(
    address_space: &mut AddressSpace,
    page: Page<Size4KiB>,
    hhdm_offset: VirtAddr,
    pmm: &mut PhysicalMemoryManager,
) -> Result<PhysAddr, ZramError> {
    let raw = address_space
        .swapped_block_id(page, hhdm_offset)
        .ok_or(ZramError::InvalidBlock)?;
    let slot = SlotId::from_raw(raw).ok_or(ZramError::InvalidBlock)?;
    let region = address_space
        .lookup_region(page.start_address().as_u64())
        .ok_or(ZramError::InvalidBlock)?;
    let owner_pid = match (region.kind, region.backing) {
        (MappingKind::Anonymous, RegionBacking::AnonymousOwner(owner)) => owner,
        _ => return Err(ZramError::InvalidBlock),
    };
    let frame_addr = pmm
        .alloc_frame_owned(owner_pid)
        .ok_or(ZramError::OutOfSpace)?;
    if !crate::memory::security::sanitize_user_frame(frame_addr, hhdm_offset) {
        pmm.free_frame(frame_addr);
        return Err(ZramError::OutOfSpace);
    }
    let dst = &mut *((hhdm_offset + frame_addr.as_u64()).as_mut_ptr::<[u8; ZRAM_BLOCK_SIZE]>());
    let stored_bits = match zram::read_page(slot, dst) {
        Ok(bits) => bits,
        Err(error) => {
            dst.fill(0);
            pmm.free_frame(frame_addr);
            return Err(error);
        }
    };
    let stored_flags = PageTableFlags::from_bits_truncate(stored_bits);
    let stored_protection = match AddressSpace::protection_from_pte_flags(stored_flags) {
        Ok(protection) => protection,
        Err(_) => {
            dst.fill(0);
            pmm.free_frame(frame_addr);
            return Err(ZramError::InvalidData);
        }
    };
    if stored_protection != region.protection
        || !stored_flags.contains(PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE)
    {
        dst.fill(0);
        pmm.free_frame(frame_addr);
        return Err(ZramError::InvalidData);
    }

    let frame = PhysFrame::from_start_address_unchecked(frame_addr);
    if address_space
        .replace_mapping(
            page,
            ExpectedMapping::Swapped { block_id: raw },
            ReplacementMapping::Present {
                frame,
                flags: stored_flags,
            },
            OwnershipTransition::RetainOld,
            pmm,
            hhdm_offset,
        )
        .is_err()
    {
        dst.fill(0);
        pmm.free_frame(frame_addr);
        return Err(ZramError::InvalidBlock);
    }
    // Scheduler/address-space serialization makes disappearance impossible.
    // Fail-stop instead of publishing a present page while silently leaking or
    // double-releasing its old ownership token.
    zram::discard(slot).expect("published swap-in lost its live ZRAM slot");
    // Do not enqueue from the page-fault path: that could grow the kernel heap
    // and would make the just-faulted, currently active page an immediate
    // reclaim candidate. A later aging/re-registration mechanism is deferred.
    Ok(frame_addr)
}

fn candidate_is_eligible(
    candidate: AnonFrame,
    sched: &Scheduler,
    pmm: &PhysicalMemoryManager,
    hhdm_offset: VirtAddr,
) -> bool {
    let Some(process) = sched
        .processes
        .iter()
        .find(|process| process.pid == candidate.pid)
    else {
        return false;
    };
    if process.trusted_swap_admin_service || process.trusted_display_service {
        return false;
    }
    let Some(region) = process
        .address_space
        .lookup_region(candidate.vaddr.as_u64())
    else {
        return false;
    };
    if region.kind != MappingKind::Anonymous
        || !region.policy.contains(RegionPolicy::MAY_UNMAP)
        || region.backing != RegionBacking::AnonymousOwner(candidate.pid as u32)
        || pmm.owner_of(candidate.frame) != Some(candidate.pid as u32)
    {
        return false;
    }
    let Ok(page) = Page::<Size4KiB>::from_start_address(candidate.vaddr) else {
        return false;
    };
    unsafe {
        process
            .address_space
            .lookup_entry(page, hhdm_offset)
            .is_some_and(|(frame, flags)| {
                frame == candidate.frame
                    && flags.contains(PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE)
            })
    }
}

/// Bounded direct reclaim. Failed/full/incompressible candidates rotate to the
/// tail; stale or newly ineligible candidates are dropped.
///
/// SAFETY: caller holds the scheduler and PMM locks.
pub unsafe fn reclaim(
    max_pages: usize,
    avoid: Option<(usize, VirtAddr)>,
    sched: &mut Scheduler,
    hhdm_offset: VirtAddr,
    pmm: &mut PhysicalMemoryManager,
) -> usize {
    if max_pages == 0 || zram::policy().is_none() {
        return 0;
    }
    let initial_candidates = candidate_count();
    let scan_limit =
        initial_candidates.min(max_pages.saturating_mul(8).saturating_add(MAX_SCAN_SLOP));
    let mut evicted = 0;
    for _ in 0..scan_limit {
        if evicted >= max_pages {
            break;
        }
        let Some(candidate) = CANDIDATES.lock().pop_front() else {
            break;
        };
        CANDIDATE_SCANS.fetch_add(1, Ordering::Relaxed);
        if avoid.is_some_and(|(pid, vaddr)| pid == candidate.pid && vaddr == candidate.vaddr) {
            CANDIDATES.lock().push_back(candidate);
            continue;
        }
        if !candidate_is_eligible(candidate, sched, pmm, hhdm_offset) {
            continue;
        }
        let Some(process_index) = sched
            .processes
            .iter()
            .position(|process| process.pid == candidate.pid)
        else {
            continue;
        };
        let page = Page::<Size4KiB>::from_start_address(candidate.vaddr)
            .expect("tracked anonymous address lost page alignment");
        let result = swap_out_page(
            &mut sched.processes[process_index].address_space,
            page,
            candidate.frame,
            hhdm_offset,
            pmm,
        );
        if result.is_ok() {
            evicted += 1;
            PAGES_RECLAIMED.fetch_add(1, Ordering::Relaxed);
        } else if candidate_is_eligible(candidate, sched, pmm, hhdm_offset) {
            CANDIDATES.lock().push_back(candidate);
        }
    }
    if evicted == 0 {
        RECLAIM_NO_PROGRESS.fetch_add(1, Ordering::Relaxed);
    }
    evicted
}

/// Trigger direct reclaim before an anonymous allocation reaches PMM
/// exhaustion. Returns pages reclaimed in this bounded activation.
pub fn maybe_reclaim_for_anonymous_allocation(
    sched: &mut Scheduler,
    pmm: &mut PhysicalMemoryManager,
    hhdm_offset: VirtAddr,
    avoid: Option<(usize, VirtAddr)>,
) -> usize {
    let (total, free) = pmm.stats();
    let (start, target) = watermarks(total);
    if free > start || zram::policy().is_none() {
        return 0;
    }
    WATERMARK_ACTIVATIONS.fetch_add(1, Ordering::Relaxed);
    let requested = target.saturating_sub(free).min(MAX_DIRECT_RECLAIM_PAGES);
    unsafe { reclaim(requested.max(1), avoid, sched, hhdm_offset, pmm) }
}

#[cfg(feature = "swap1_test")]
pub fn force_pressure_gate(
    pages: usize,
    sched: &mut Scheduler,
    pmm: &mut PhysicalMemoryManager,
    hhdm_offset: VirtAddr,
) -> usize {
    WATERMARK_ACTIVATIONS.fetch_add(1, Ordering::Relaxed);
    unsafe { reclaim(pages, None, sched, hhdm_offset, pmm) }
}
