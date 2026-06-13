//! Anonymous page tracking for the ZRAM swap subsystem (Phase 6.6 Step 2).
//!
//! Tracks which physical frames back anonymous user mappings so a future
//! reclaim pass (Step 3+) has a candidate list to evict via `zram`.

use super::pmm::PhysicalMemoryManager;
use super::zram::{self, ZramError, ZRAM_BLOCK_SIZE};
use crate::process::address_space::AddressSpace;
use alloc::vec::Vec;
use spin::Mutex;
use x86_64::{
    structures::paging::{Page, PageTableFlags, PhysFrame, Size4KiB},
    PhysAddr, VirtAddr,
};

/// A tracked anonymous user frame: who owns it and where it's mapped.
#[derive(Debug, Clone, Copy)]
pub struct AnonFrame {
    pub pid: usize,
    pub vaddr: VirtAddr,
    pub frame: PhysAddr,
}

static CANDIDATES: Mutex<Vec<AnonFrame>> = Mutex::new(Vec::new());

/// Register a freshly mapped anonymous user page as a reclaim candidate.
pub fn track_anon(pid: usize, vaddr: VirtAddr, frame: PhysAddr) {
    CANDIDATES.lock().push(AnonFrame { pid, vaddr, frame });
}

/// Remove tracking for a frame (on unmap or process exit).
pub fn untrack(frame: PhysAddr) {
    CANDIDATES.lock().retain(|c| c.frame != frame);
}

/// Remove all tracked frames belonging to `pid` (on process exit).
pub fn untrack_process(pid: usize) {
    CANDIDATES.lock().retain(|c| c.pid != pid);
}

/// Number of frames currently registered as reclaim candidates.
pub fn candidate_count() -> usize {
    CANDIDATES.lock().len()
}

/// Compress `frame`'s contents into ZRAM, rewrite the owning PTE as a
/// swapped marker, free the physical frame, and drop tracking for it.
///
/// SAFETY: `hhdm_offset` must be the correct HHDM base, `frame` must be the
/// frame currently mapped at `page` in `address_space`, and no other code
/// may read/write `frame` concurrently.
pub unsafe fn swap_out_page(
    address_space: &mut AddressSpace,
    page: Page<Size4KiB>,
    frame: PhysAddr,
    hhdm_offset: VirtAddr,
    pmm: &mut PhysicalMemoryManager,
) -> Result<usize, ZramError> {
    let src = &*((hhdm_offset + frame.as_u64()).as_ptr::<[u8; ZRAM_BLOCK_SIZE]>());
    let block_id = zram::write_page(src)?;

    if !address_space.mark_swapped(page, block_id as u64, hhdm_offset) {
        // Roll back the zram write; the page wasn't actually mapped.
        let _ = zram::discard_block(block_id);
        return Err(ZramError::InvalidBlock);
    }

    pmm.free_frame(frame);
    untrack(frame);
    Ok(block_id)
}

/// Allocate a fresh frame, decompress the ZRAM block backing `page` into it,
/// remap `page` present with `flags`, discard the ZRAM block, and re-track
/// the frame as a reclaim candidate for `pid`.
///
/// SAFETY: `hhdm_offset` must be the correct HHDM base and `page`'s leaf PTE
/// must currently be a swapped marker (see `AddressSpace::swapped_block_id`).
pub unsafe fn swap_in_page(
    address_space: &mut AddressSpace,
    page: Page<Size4KiB>,
    pid: usize,
    flags: PageTableFlags,
    hhdm_offset: VirtAddr,
    pmm: &mut PhysicalMemoryManager,
) -> Result<PhysAddr, ZramError> {
    let block_id = address_space
        .swapped_block_id(page, hhdm_offset)
        .ok_or(ZramError::InvalidBlock)? as usize;

    let frame_addr = pmm.alloc_frame().ok_or(ZramError::OutOfSpace)?;
    let dst = &mut *((hhdm_offset + frame_addr.as_u64()).as_mut_ptr::<[u8; ZRAM_BLOCK_SIZE]>());
    zram::read_block(block_id, dst)?;

    let frame = PhysFrame::from_start_address_unchecked(frame_addr);
    address_space.remap_present(page, frame, flags, hhdm_offset);
    let _ = zram::discard_block(block_id);

    track_anon(pid, page.start_address(), frame_addr);
    Ok(frame_addr)
}

/// Evict up to `max_pages` reclaim candidates to ZRAM. Returns the number of
/// pages actually swapped out. Candidates whose page table entry can't be
/// found (stale tracking) are dropped without retry.
///
/// SAFETY: `hhdm_offset` must be the correct HHDM base.
pub unsafe fn reclaim(
    max_pages: usize,
    address_space_for: impl Fn(usize) -> Option<*mut AddressSpace>,
    hhdm_offset: VirtAddr,
    pmm: &mut PhysicalMemoryManager,
) -> usize {
    let mut evicted = 0;
    while evicted < max_pages {
        let candidate = {
            let mut candidates = CANDIDATES.lock();
            if candidates.is_empty() {
                break;
            }
            candidates.remove(0)
        };

        let Some(as_ptr) = address_space_for(candidate.pid) else {
            continue;
        };
        let address_space = &mut *as_ptr;

        let page = Page::<Size4KiB>::from_start_address(candidate.vaddr)
            .expect("tracked vaddr is page-aligned");

        if swap_out_page(address_space, page, candidate.frame, hhdm_offset, pmm).is_ok() {
            evicted += 1;
        }
    }
    evicted
}
