use crate::capability::{CapabilityBroker, CapabilityToken};
use crate::memory::pmm::PhysicalMemoryManager;
use crate::process::Process;
use x86_64::structures::paging::{Page, PageTableFlags, PhysFrame, Size4KiB};
use x86_64::{PhysAddr, VirtAddr};

pub const PAGE_SIZE: usize = 4096;
pub const SHARED_REGION_BASE: u64 = 0x0000_0003_0000_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SharedMemError {
    OutOfMemory,
    InvalidToken,
    InvalidAddress,
}

/// A shared physical page grant. Owner tracks it for exit cleanup.
pub struct SharedPage {
    pub phys: PhysAddr,
    pub token: CapabilityToken,
    pub owner: usize,
    pub size: usize,
}

pub fn alloc_shared_page(
    caller: &mut Process,
    pmm: &mut PhysicalMemoryManager,
    caps: &mut CapabilityBroker,
    hhdm_offset: VirtAddr,
) -> Result<(VirtAddr, CapabilityToken), SharedMemError> {
    let phys = pmm.alloc_frame().ok_or(SharedMemError::OutOfMemory)?;

    let virt = unsafe { caller.address_space.map_shared_page(phys, pmm, hhdm_offset) }?;

    let token = caps.mint_shared_page(phys, caller.pid);

    caller.owned_shared.push(SharedPage {
        phys,
        token,
        owner: caller.pid,
        size: PAGE_SIZE,
    });
    caller.mapped_shared.push((token, virt));

    Ok((virt, token))
}

pub fn map_shared_page(
    receiver: &mut Process,
    token: CapabilityToken,
    pmm: &mut PhysicalMemoryManager,
    caps: &mut CapabilityBroker,
    hhdm_offset: VirtAddr,
) -> Result<VirtAddr, SharedMemError> {
    let phys = caps
        .resolve_shared_page(token)
        .ok_or(SharedMemError::InvalidToken)?;

    let virt = unsafe { receiver.address_space.map_shared_page(phys, pmm, hhdm_offset) }?;

    receiver.mapped_shared.push((token, virt));

    Ok(virt)
}

pub fn free_shared_page(
    process: &mut Process,
    token: CapabilityToken,
    pmm: &mut PhysicalMemoryManager,
    caps: &mut CapabilityBroker,
    hhdm_offset: VirtAddr,
) {
    // Unmap any local mapping for this token
    if let Some(pos) = process.mapped_shared.iter().position(|(t, _)| *t == token) {
        let (_, virt) = process.mapped_shared.remove(pos);
        unsafe {
            if let Ok(page) = Page::<Size4KiB>::from_start_address(virt) {
                let _ = process.address_space.unmap_page(page, hhdm_offset);
            }
        }
    }

    // If owner of this grant, revoke and free the frame
    if let Some(pos) = process.owned_shared.iter().position(|sp| sp.token == token) {
        let sp = process.owned_shared.remove(pos);
        caps.revoke_shared(sp.token);
        pmm.free_frame(sp.phys);
    }
}

/// Called on process exit (under sched lock) to release owned frames and unmap views.
pub fn cleanup_shared_pages(
    process: &mut Process,
    pmm: &mut PhysicalMemoryManager,
    caps: &mut CapabilityBroker,
) {
    let hhdm_offset = VirtAddr::new(
        crate::HHDM_REQ
            .response()
            .map(|r| r.offset)
            .unwrap_or(0),
    );

    // Unmap all views this process had (owned + received)
    for &(_, virt) in &process.mapped_shared {
        unsafe {
            if let Ok(page) = Page::<Size4KiB>::from_start_address(virt) {
                let _ = process.address_space.unmap_page(page, hhdm_offset);
            }
        }
    }
    process.mapped_shared.clear();

    // Free frames and revoke tokens for anything we owned
    for sp in process.owned_shared.drain(..) {
        caps.revoke_shared(sp.token);
        pmm.free_frame(sp.phys);
    }
}
