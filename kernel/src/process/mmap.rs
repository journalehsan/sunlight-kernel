use crate::memory::pmm::PhysicalMemoryManager;
use crate::sched::Scheduler;
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
    // Only support anonymous mappings for now
    if (flags & MAP_ANONYMOUS) == 0 {
        return Err(MmapError::InvalidFlags);
    }

    if length == 0 {
        return Err(MmapError::InvalidAddress);
    }

    // Base of the anonymous mmap region for hint-less / hinted mappings.
    const MMAP_REGION_BASE: u64 = 0x10_0000_0000u64;

    // Calculate number of pages needed
    let page_count = (length + 4095) / 4096;
    let span = page_count * 4096;

    // Determine the address to map at. For anonymous mappings without a
    // fixed address we hand out fresh VA ranges from a per-process bump
    // cursor; returning a fixed base for every call made successive mmaps
    // alias the same range and corrupted userspace allocators (e.g. musl
    // mallocng), which then page-faulted on the first free().
    let map_addr = if (flags & MAP_FIXED) != 0 {
        // Use the provided address (must be page-aligned).
        if addr & 0xFFF != 0 {
            return Err(MmapError::InvalidAddress);
        }
        addr
    } else {
        // addr == 0 or a non-fixed hint: allocate from the bump cursor.
        let cur = sched.current_process_mut();
        if cur.mmap_next < MMAP_REGION_BASE {
            cur.mmap_next = MMAP_REGION_BASE;
        }
        let base = cur.mmap_next;
        cur.mmap_next = base + span;
        base
    };

    // Check that the address is in user space
    if map_addr >= 0x0000_8000_0000_0000 {
        return Err(MmapError::InvalidAddress);
    }

    let page_flags = prot_to_flags(prot);

    // Map all the pages
    let pid = sched.current_process_mut().pid;
    let hhdm_offset = VirtAddr::new(crate::HHDM_REQ.response().expect("no hhdm").offset);
    for i in 0..page_count {
        let page_vaddr = VirtAddr::new(map_addr + i * 4096);
        let page = Page::from_start_address(page_vaddr).map_err(|_| MmapError::InvalidAddress)?;

        let frame_addr = match pmm.alloc_frame() {
            Some(addr) => addr,
            None => {
                // Memory pressure: try to evict one anonymous page to ZRAM
                // and retry the allocation once (Phase 6.6 Step 4).
                let evicted = unsafe {
                    crate::memory::swap::reclaim(
                        1,
                        |pid| {
                            sched
                                .process_mut_by_pid(pid)
                                .map(|p| &mut p.address_space as *mut _)
                        },
                        hhdm_offset,
                        pmm,
                    )
                };
                if evicted == 0 {
                    return Err(MmapError::NoMemory);
                }
                pmm.alloc_frame().ok_or(MmapError::NoMemory)?
            }
        };
        let frame = unsafe { PhysFrame::from_start_address_unchecked(frame_addr) };

        let proc = sched.process_mut_by_pid(pid).ok_or(MmapError::NoMemory)?;
        unsafe {
            proc.address_space
                .map_page(page, frame, page_flags, pmm, hhdm_offset);
        }

        // Track as a swap reclaim candidate (Phase 6.6 Step 2).
        crate::memory::swap::track_anon(pid, page_vaddr, frame_addr);
    }

    Ok(map_addr)
}

/// Unmap memory (stub for now)
pub fn sys_munmap(_addr: u64, _length: u64) -> Result<(), MmapError> {
    // TODO: Implement munmap
    Ok(())
}

/// Change memory protection (stub for now)
pub fn sys_mprotect(_addr: u64, _length: u64, _prot: u32) -> Result<(), MmapError> {
    // TODO: Implement mprotect
    Ok(())
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
