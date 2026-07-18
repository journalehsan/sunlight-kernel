use super::mm2a_plan::{checked_page_layout, DeferredCursor};
use crate::memory::pmm::PhysicalMemoryManager;
use crate::process::address_space::MappingError;
use crate::sched::Scheduler;
use alloc::vec::Vec;
use x86_64::structures::paging::{Page, PageTableFlags, PhysFrame, Size4KiB};
use x86_64::VirtAddr;

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
    AlreadyMapped,
    Unsupported,
    InternalInvariant,
}

/// Convert mprotect flags to x86_64 PageTableFlags
fn prot_to_flags(prot: u32) -> PageTableFlags {
    let mut flags = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE;

    if (prot & PROT_WRITE) != 0 {
        flags |= PageTableFlags::WRITABLE;
    }

    if (prot & PROT_EXEC) == 0 {
        flags |= PageTableFlags::NO_EXECUTE;
    }

    flags
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

    let page_flags = prot_to_flags(prot);

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

    let page_count_usize = usize::try_from(page_count).map_err(|_| MmapError::InvalidAddress)?;
    let mut installed: Vec<(Page<Size4KiB>, x86_64::PhysAddr)> = Vec::new();
    installed
        .try_reserve_exact(page_count_usize)
        .map_err(|_| MmapError::NoMemory)?;
    crate::memory::swap::reserve_candidates(page_count_usize).map_err(|_| MmapError::NoMemory)?;
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
                return Err(MmapError::NoMemory);
            }
        };
        if !crate::memory::security::sanitize_user_frame(frame_addr, hhdm_offset) {
            pmm.free_frame(frame_addr);
            rollback_anonymous(&mut installed, pid, pmm, sched, hhdm_offset);
            return Err(MmapError::NoMemory);
        }
        let frame = unsafe { PhysFrame::from_start_address_unchecked(frame_addr) };

        let proc = match sched.process_mut_by_pid(pid) {
            Some(process) => process,
            None => {
                pmm.free_frame(frame_addr);
                rollback_anonymous(&mut installed, pid, pmm, sched, hhdm_offset);
                return Err(MmapError::InternalInvariant);
            }
        };
        if let Err(error) = unsafe {
            proc.address_space
                .map_page(page, frame, page_flags, pmm, hhdm_offset)
        } {
            pmm.free_frame(frame_addr);
            rollback_anonymous(&mut installed, pid, pmm, sched, hhdm_offset);
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

    for (page, frame) in &installed {
        crate::memory::swap::track_anon(pid, page.start_address(), *frame);
    }
    if let Some(cursor) = deferred_cursor {
        cursor.commit(&mut sched.current_process_mut().mmap_next);
    }

    Ok(map_addr)
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

/// Unmap memory (stub for now)
pub fn sys_munmap(_addr: u64, _length: u64) -> Result<(), MmapError> {
    Err(MmapError::Unsupported)
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
