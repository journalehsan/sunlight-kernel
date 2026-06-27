use crate::capability::{CapabilityBroker, CapabilityToken};
use crate::memory::pmm::PhysicalMemoryManager;
use crate::process::Process;
use x86_64::structures::paging::{Page, PhysFrame, Size4KiB};
use x86_64::VirtAddr;

pub const PAGE_SIZE: usize = 4096;
pub const SHARED_REGION_BASE: u64 = 0x0000_0003_0000_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SharedMemError {
    OutOfMemory,
    InvalidToken,
    InvalidAddress,
    InvalidArgument,
}

/// A shared memory region grant (1 or more pages). Owner tracks for exit cleanup.
pub struct SharedRegion {
    pub token: CapabilityToken,
    pub owner: usize,
    pub size: usize,
}

pub fn alloc_shared_region(
    caller: &mut Process,
    pmm: &mut PhysicalMemoryManager,
    caps: &mut CapabilityBroker,
    hhdm_offset: VirtAddr,
    size: usize,
) -> Result<(VirtAddr, CapabilityToken), SharedMemError> {
    let page_size = PAGE_SIZE;
    let num_pages = if size == 0 {
        1
    } else {
        (size + page_size - 1) / page_size
    };
    if num_pages == 0 {
        return Err(SharedMemError::InvalidArgument);
    }

    let mut frames: alloc::vec::Vec<PhysFrame<Size4KiB>> =
        alloc::vec::Vec::with_capacity(num_pages);
    for _ in 0..num_pages {
        let phys = pmm
            .alloc_frame_owned(caller.pid as u32)
            .ok_or(SharedMemError::OutOfMemory)?;
        let frame = unsafe { PhysFrame::<Size4KiB>::from_start_address_unchecked(phys) };
        frames.push(frame);
    }

    let actual_size = num_pages * page_size;
    let token = caps.mint_shared_region(frames.clone(), actual_size, caller.pid);

    let virt = unsafe {
        caller
            .address_space
            .map_shared_region(&frames, pmm, hhdm_offset)
    }?;

    // Track this mapping so the ref count starts at 1 (owner's own mapping).
    caps.increment_map_count(token);

    caller.owned_shared.push(SharedRegion {
        token,
        owner: caller.pid,
        size: actual_size,
    });
    caller.mapped_shared.push((token, virt, actual_size));

    Ok((virt, token))
}

/// Backward-compat wrapper for single-page (4 KiB) allocations used by existing bulk IPC.
pub fn alloc_shared_page(
    caller: &mut Process,
    pmm: &mut PhysicalMemoryManager,
    caps: &mut CapabilityBroker,
    hhdm_offset: VirtAddr,
) -> Result<(VirtAddr, CapabilityToken), SharedMemError> {
    alloc_shared_region(caller, pmm, caps, hhdm_offset, PAGE_SIZE)
}

pub fn map_shared_page(
    receiver: &mut Process,
    token: CapabilityToken,
    pmm: &mut PhysicalMemoryManager,
    caps: &mut CapabilityBroker,
    hhdm_offset: VirtAddr,
) -> Result<VirtAddr, SharedMemError> {
    let obj = caps
        .resolve_shared_region(token)
        .ok_or(SharedMemError::InvalidToken)?;

    let virt = unsafe {
        receiver
            .address_space
            .map_shared_region(&obj.frames, pmm, hhdm_offset)
    }?;

    // Each new mapping increments the ref count; frames won't be freed until
    // every mapping has been released via free_shared_page / cleanup_shared_pages.
    caps.increment_map_count(token);

    receiver.mapped_shared.push((token, virt, obj.size));

    Ok(virt)
}

pub fn free_shared_page(
    process: &mut Process,
    token: CapabilityToken,
    pmm: &mut PhysicalMemoryManager,
    caps: &mut CapabilityBroker,
    hhdm_offset: VirtAddr,
) {
    // Unmap any local mapping(s) for this token (multi-page aware).
    if let Some(pos) = process
        .mapped_shared
        .iter()
        .position(|(t, _, _)| *t == token)
    {
        let (_, base_virt, sz) = process.mapped_shared.remove(pos);
        let num_pages = if sz == 0 {
            1
        } else {
            (sz + PAGE_SIZE - 1) / PAGE_SIZE
        };
        for i in 0..num_pages {
            let v = VirtAddr::new(base_virt.as_u64() + (i * PAGE_SIZE) as u64);
            unsafe {
                if let Ok(page) = Page::<Size4KiB>::from_start_address(v) {
                    let _ = process.address_space.unmap_page(page, hhdm_offset);
                }
            }
        }
        // Decrement the mapping ref count; the broker returns the frames only
        // when this was the last live mapping, so the PMM is safe to reclaim them.
        if let Some(frames) = caps.decrement_map_count(token) {
            for f in &frames {
                pmm.free_frame(f.start_address());
            }
        }
    }

    // Remove ownership tracking without freeing frames here — ref counting handles that.
    if let Some(pos) = process.owned_shared.iter().position(|sp| sp.token == token) {
        process.owned_shared.remove(pos);
    }
}

/// Called on process exit (under sched lock) to release owned frames and unmap views.
pub fn cleanup_shared_pages(
    process: &mut Process,
    pmm: &mut PhysicalMemoryManager,
    caps: &mut CapabilityBroker,
) {
    let hhdm_offset = VirtAddr::new(crate::HHDM_REQ.response().map(|r| r.offset).unwrap_or(0));

    // Unmap all views this process had (owned + received). Decrement the ref
    // count for each; if this was the last mapping, free the physical frames.
    let mapped: alloc::vec::Vec<_> = process.mapped_shared.drain(..).collect();
    for (token, base_virt, sz) in mapped {
        let num_pages = if sz == 0 {
            1
        } else {
            (sz + PAGE_SIZE - 1) / PAGE_SIZE
        };
        for i in 0..num_pages {
            let v = VirtAddr::new(base_virt.as_u64() + (i * PAGE_SIZE) as u64);
            unsafe {
                if let Ok(page) = Page::<Size4KiB>::from_start_address(v) {
                    let _ = process.address_space.unmap_page(page, hhdm_offset);
                }
            }
        }
        if let Some(frames) = caps.decrement_map_count(token) {
            for f in &frames {
                pmm.free_frame(f.start_address());
            }
        }
    }

    // Ownership records are now stale (frames freed via ref count above).
    process.owned_shared.clear();
}
