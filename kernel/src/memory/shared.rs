use crate::capability::{CapabilityBroker, CapabilityToken};
use crate::memory::pmm::PhysicalMemoryManager;
use crate::process::Process;
use x86_64::structures::paging::{Page, PhysFrame, Size4KiB};
use x86_64::VirtAddr;

pub const PAGE_SIZE: usize = 4096;
pub const SHARED_REGION_BASE: u64 = 0x0000_0003_0000_0000;
pub const SHARED_REGION_MAX_SIZE: usize = 0x1_0000_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SharedMemError {
    OutOfMemory,
    InvalidToken,
    InvalidAddress,
    InvalidArgument,
    AlreadyMapped,
    PageTableAllocationFailed,
    InternalInvariant,
}

impl From<crate::process::address_space::MappingError> for SharedMemError {
    fn from(error: crate::process::address_space::MappingError) -> Self {
        use crate::process::address_space::MappingError;
        match error {
            MappingError::AlreadyMapped => Self::AlreadyMapped,
            MappingError::LedgerOverlap => Self::AlreadyMapped,
            MappingError::FrameAllocationFailed | MappingError::LedgerCapacityExhausted => {
                Self::OutOfMemory
            }
            MappingError::PageTableAllocationFailed => Self::PageTableAllocationFailed,
            MappingError::InvalidAddress
            | MappingError::NonCanonical
            | MappingError::Overflow
            | MappingError::Misaligned => Self::InvalidAddress,
            _ => Self::InternalInvariant,
        }
    }
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
    let requested = size.max(1);
    let rounded = requested
        .checked_add(page_size - 1)
        .ok_or(SharedMemError::InvalidArgument)?;
    let num_pages = rounded / page_size;
    if num_pages == 0 {
        return Err(SharedMemError::InvalidArgument);
    }

    let actual_size = num_pages
        .checked_mul(page_size)
        .ok_or(SharedMemError::InvalidArgument)?;
    if actual_size > SHARED_REGION_MAX_SIZE {
        return Err(SharedMemError::InvalidArgument);
    }
    caller
        .owned_shared
        .try_reserve(1)
        .map_err(|_| SharedMemError::OutOfMemory)?;
    caller
        .mapped_shared
        .try_reserve(1)
        .map_err(|_| SharedMemError::OutOfMemory)?;
    caps.reserve_shared_region_slot()
        .map_err(|_| SharedMemError::OutOfMemory)?;

    let mut frames: alloc::vec::Vec<PhysFrame<Size4KiB>> = alloc::vec::Vec::new();
    frames
        .try_reserve_exact(num_pages)
        .map_err(|_| SharedMemError::OutOfMemory)?;
    for _ in 0..num_pages {
        // SharedMemory is the unique global primary class: one charge regardless
        // of how many processes map the object.
        let phys = match pmm.alloc_frame_owned_class(
            caller.pid as u32,
            crate::memory::accounting::PhysicalMemoryClass::SharedMemory,
        ) {
            Some(phys) => phys,
            None => {
                crate::process::address_space::note_frame_allocation_failure();
                for frame in &frames {
                    pmm.free_frame(frame.start_address());
                }
                if !frames.is_empty() {
                    crate::process::address_space::note_shm_create_rollback();
                }
                return Err(SharedMemError::OutOfMemory);
            }
        };
        if !crate::memory::security::sanitize_user_frame(phys, hhdm_offset) {
            pmm.free_frame(phys);
            for frame in &frames {
                pmm.free_frame(frame.start_address());
            }
            crate::process::address_space::note_shm_create_rollback();
            return Err(SharedMemError::OutOfMemory);
        }
        let frame = unsafe { PhysFrame::<Size4KiB>::from_start_address_unchecked(phys) };
        frames.push(frame);
    }

    let virt = match unsafe {
        caller.address_space.map_shared_region(
            &frames,
            crate::process::region::RegionBacking::None,
            pmm,
            hhdm_offset,
        )
    } {
        Ok(virt) => virt,
        Err(error) => {
            for frame in &frames {
                pmm.free_frame(frame.start_address());
            }
            crate::process::address_space::note_shm_create_rollback();
            return Err(error);
        }
    };

    // Publication is the commit point: backing frames and the complete owner
    // mapping exist before the token becomes observable.
    let token = caps.mint_shared_region(frames, actual_size, caller.pid);

    caller
        .address_space
        .update_region_backing(
            virt.as_u64(),
            virt.as_u64() + actual_size as u64,
            crate::process::region::MappingKind::SharedMemory,
            crate::process::region::RegionBacking::None,
            crate::process::region::RegionBacking::SharedMemory(token.as_u64()),
        )
        .expect("committed SHM owner ledger record disappeared");

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
    // Check admission and count overflow before installing even one PTE.
    if caps.resolve_shared_region(token).is_none() {
        return Err(SharedMemError::InvalidToken);
    }
    if !caps.preflight_shared_mapping(token) {
        return Err(SharedMemError::OutOfMemory);
    }
    receiver
        .mapped_shared
        .try_reserve(1)
        .map_err(|_| SharedMemError::OutOfMemory)?;

    let (virt, size) = {
        let obj = caps
            .resolve_shared_region(token)
            .ok_or(SharedMemError::InvalidToken)?;
        let virt = match unsafe {
            receiver.address_space.map_shared_region(
                &obj.frames,
                crate::process::region::RegionBacking::SharedMemory(token.as_u64()),
                pmm,
                hhdm_offset,
            )
        } {
            Ok(virt) => virt,
            Err(error) => {
                crate::process::address_space::note_shm_peer_rollback();
                return Err(error);
            }
        };
        (virt, obj.size)
    };

    // Each new mapping increments the ref count; frames won't be freed until
    // every mapping has been released via free_shared_page / cleanup_shared_pages.
    caps.increment_map_count(token);

    receiver.mapped_shared.push((token, virt, size));

    Ok(virt)
}

pub fn free_shared_page(
    process: &mut Process,
    token: CapabilityToken,
    pmm: &mut PhysicalMemoryManager,
    caps: &mut CapabilityBroker,
    hhdm_offset: VirtAddr,
) -> Result<(), SharedMemError> {
    // Release the newest local view. A temporary map/copy/free must not
    // invalidate an older persistent surface mapped with the same token.
    let pos = process
        .mapped_shared
        .iter()
        .rposition(|(t, _, _)| *t == token)
        .ok_or(SharedMemError::InvalidToken)?;
    {
        let object = caps
            .shared_region_for_cleanup(token)
            .ok_or(SharedMemError::InternalInvariant)?;
        let (_, base_virt, sz) = process.mapped_shared[pos];
        if sz == 0
            || sz % PAGE_SIZE != 0
            || sz != object.size
            || object.frames.len() != sz / PAGE_SIZE
            || caps.shared_region_map_count(token).unwrap_or(0) == 0
        {
            return Err(SharedMemError::InternalInvariant);
        }
        for (index, expected_frame) in object.frames.iter().enumerate() {
            let offset = index
                .checked_mul(PAGE_SIZE)
                .ok_or(SharedMemError::InternalInvariant)?;
            let vaddr = base_virt
                .as_u64()
                .checked_add(offset as u64)
                .ok_or(SharedMemError::InternalInvariant)?;
            let page = Page::<Size4KiB>::from_start_address(VirtAddr::new(vaddr))
                .map_err(|_| SharedMemError::InternalInvariant)?;
            let mapped = unsafe { process.address_space.lookup_entry(page, hhdm_offset) };
            if !matches!(
                mapped,
                Some((physical, flags))
                    if physical == expected_frame.start_address()
                        && flags.contains(x86_64::structures::paging::PageTableFlags::PRESENT)
            ) {
                return Err(SharedMemError::InternalInvariant);
            }
        }
    }

    let (_, expected_base, expected_size) = process.mapped_shared[pos];
    let expected_region = process
        .address_space
        .lookup_region(expected_base.as_u64())
        .filter(|region| {
            region.start == expected_base.as_u64()
                && region.end == expected_base.as_u64() + expected_size as u64
                && region.kind == crate::process::region::MappingKind::SharedMemory
                && region.backing
                    == crate::process::region::RegionBacking::SharedMemory(token.as_u64())
        })
        .ok_or(SharedMemError::InternalInvariant)?;

    {
        let (_, base_virt, sz) = process.mapped_shared[pos];
        let object = caps
            .shared_region_for_cleanup(token)
            .expect("preflighted SHM backing disappeared under broker lock");
        for (i, expected_frame) in object.frames.iter().enumerate() {
            let v = VirtAddr::new(base_virt.as_u64() + (i * PAGE_SIZE) as u64);
            let page = Page::<Size4KiB>::from_start_address(v)
                .expect("preflighted SHM page became unaligned");
            // unmap_page synchronously acknowledges remote invalidations.
            // After preflight, mutation cannot fail under the caller's VM /
            // scheduler lock. Fail-stop on an invariant breach: never publish
            // released references or let exit reclaim an uncertain mapping.
            let removed = unsafe { process.address_space.unmap_page(page, hhdm_offset) };
            assert_eq!(
                removed,
                Some(expected_frame.start_address()),
                "preflighted SHM unmap changed under scheduler lock"
            );
        }
        debug_assert_eq!(sz / PAGE_SIZE, object.frames.len());
        process
            .address_space
            .remove_region_exact(expected_region)
            .expect("preflighted SHM ledger changed under scheduler lock");
        process.mapped_shared.remove(pos);
        // Decrement the mapping ref count; the broker returns the frames only
        // when this was the last live mapping, so the PMM is safe to reclaim them.
        if let Some(frames) = caps.decrement_map_count(token) {
            for f in &frames {
                pmm.free_frame(f.start_address());
            }
        }
    }

    // Remove ownership tracking without freeing frames here — ref counting handles that.
    if !process.mapped_shared.iter().any(|(t, _, _)| *t == token) {
        if let Some(pos) = process.owned_shared.iter().position(|sp| sp.token == token) {
            process.owned_shared.remove(pos);
        }
    }
    Ok(())
}

/// Called on process exit (under sched lock) to release owned frames and unmap views.
pub fn cleanup_shared_pages(
    process: &mut Process,
    pmm: &mut PhysicalMemoryManager,
    caps: &mut CapabilityBroker,
) {
    if process.mapped_shared.is_empty() {
        process.owned_shared.clear();
        return;
    }
    let hhdm_offset = VirtAddr::new(
        crate::HHDM_REQ
            .response()
            .expect("SHM cleanup requires initialized HHDM")
            .offset,
    );
    // Use the same verified path for explicit free and exit. Do not drain the
    // tracking vector first: a failed preflight must keep backing pinned, and
    // the reaper must not proceed to reclaim an uncertain address space.
    while let Some(&(token, _, _)) = process.mapped_shared.last() {
        free_shared_page(process, token, pmm, caps, hhdm_offset)
            .expect("SHM exit cleanup invariant failed; backing remains pinned");
    }
    process.owned_shared.clear();
}
