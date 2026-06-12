use super::Process;
use crate::memory::pmm::PhysicalMemoryManager;
use sunlight_elf::{SegmentPlan, SegmentProt};
use x86_64::{
    VirtAddr,
    structures::paging::{
        Page, PageTableFlags, PhysFrame,
    },
};

/// User-address window allowed for PT_LOAD segments. The stack and heap are
/// mapped separately above USER_HI, so a validated binary can never collide
/// with them or reach the kernel higher half.
const USER_LO: u64 = 0x1000;
const USER_HI: u64 = super::layout::USER_HEAP_START;

fn prot_flags(prot: SegmentProt) -> PageTableFlags {
    match prot {
        // Executable (read + execute). W^X is already enforced by validation.
        SegmentProt::ReadExec => PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE,
        SegmentProt::ReadWrite => {
            PageTableFlags::PRESENT
                | PageTableFlags::WRITABLE
                | PageTableFlags::USER_ACCESSIBLE
                | PageTableFlags::NO_EXECUTE
        }
        SegmentProt::Read => {
            PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE | PageTableFlags::NO_EXECUTE
        }
    }
}

/// Combine protections when two segments share a 4 KiB page (e.g. .rodata
/// ending where .data begins): writable if either side is, executable if
/// either side is (NX must drop out of the union).
fn union_flags(old: PageTableFlags, new: PageTableFlags) -> PageTableFlags {
    let mut merged = old | new;
    if !old.contains(PageTableFlags::NO_EXECUTE) || !new.contains(PageTableFlags::NO_EXECUTE) {
        merged.remove(PageTableFlags::NO_EXECUTE);
    }
    merged
}

/// Load a validated ELF64 into the process address space.
/// Returns the entry point, or None if the binary is rejected.
pub fn load_elf(
    elf_bytes: &[u8],
    process: &mut Process,
    pmm: &mut PhysicalMemoryManager,
    hhdm_offset: VirtAddr,
) -> Option<u64> {
    let header = match sunlight_elf::parse_elf_header(elf_bytes) {
        Ok(h) => h,
        Err(e) => {
            crate::serial_println!("[ELF] header rejected: {:?}", e);
            return None;
        }
    };

    // plan_segments validates every segment (user-range bounds, W^X, entry
    // coverage) before emitting anything, so mapping never starts on a
    // binary that will be rejected.
    let mut map_failed = false;
    let planned = sunlight_elf::plan_segments(
        elf_bytes,
        &header,
        USER_LO,
        USER_HI,
        &mut |plan| {
            if !map_failed && map_segment(plan, elf_bytes, process, pmm, hhdm_offset).is_none() {
                map_failed = true;
            }
        },
    );
    if let Err(e) = planned {
        crate::serial_println!("[ELF] segment validation failed: {:?}", e);
        return None;
    }
    if map_failed {
        crate::serial_println!("[ELF] segment mapping failed (out of frames?)");
        return None;
    }

    Some(header.entry)
}

fn map_segment(
    plan: &SegmentPlan,
    elf_bytes: &[u8],
    process: &mut Process,
    pmm: &mut PhysicalMemoryManager,
    hhdm_offset: VirtAddr,
) -> Option<()> {
    let flags = prot_flags(plan.prot);

    crate::serial_println!(
        "[ELF] PT_LOAD off={:x} vaddr={:x} filesz={:x} memsz={:x} prot={:?}",
        plan.file_offset, plan.vaddr, plan.file_size, plan.mem_size, plan.prot
    );

    // Only bytes up to file_size are copied; the rest of the segment
    // (.bss tail) stays zero from the fresh frames.
    let copy_end = plan.vaddr + plan.file_size;

    for page_idx in 0..plan.page_count {
        let page_addr = VirtAddr::new(plan.vaddr_page_start + page_idx as u64 * 4096);
        let page = Page::from_start_address(page_addr).ok()?;

        // When two segments share a page, reuse the existing physical frame
        // instead of allocating a new one that would overwrite the previous
        // segment's data.
        let existing = unsafe {
            process.address_space.lookup_entry(page, hhdm_offset)
        };

        let (frame_addr, existing_flags) = match existing {
            Some((phys, old_flags)) => (phys, Some(old_flags)),
            None => (pmm.alloc_frame()?, None),
        };

        let phys = unsafe { PhysFrame::from_start_address_unchecked(frame_addr) };
        let hhdm_ptr = (hhdm_offset + frame_addr.as_u64()).as_mut_ptr::<u8>();

        if existing_flags.is_none() {
            // Zero the new frame before copying segment data into it.
            unsafe { core::ptr::write_bytes(hhdm_ptr, 0, 4096); }
        }

        // Copy the overlap between this page and the segment's file bytes.
        let page_start = page_addr.as_u64();
        let page_end = page_start + 4096;
        let overlap_start = plan.vaddr.max(page_start);
        let overlap_end = copy_end.min(page_end);

        if overlap_start < overlap_end {
            let file_offset = (plan.file_offset + (overlap_start - plan.vaddr)) as usize;
            let dst_offset = (overlap_start - page_start) as usize;
            let len = (overlap_end - overlap_start) as usize;

            // Validation guarantees file_offset + len <= elf_bytes.len().
            unsafe {
                core::ptr::copy_nonoverlapping(
                    elf_bytes.as_ptr().add(file_offset),
                    hhdm_ptr.add(dst_offset),
                    len,
                );
            }
        }

        match existing_flags {
            None => {
                // SAFETY: mapping a fresh user page into the process address space.
                unsafe {
                    process.address_space.map_page(page, phys, flags, pmm, hhdm_offset);
                }
            }
            Some(old_flags) if old_flags != flags => {
                // Shared page with different protections: union them so e.g.
                // a .data byte in a mostly-.rodata page stays writable.
                unsafe {
                    process.address_space.update_flags(
                        page,
                        union_flags(old_flags, flags),
                        hhdm_offset,
                    );
                }
            }
            Some(_) => {}
        }
    }

    Some(())
}

/// Detect if an ELF binary is a Linux-compatible ELF (Phase 4.5).
/// Returns true if e_ident[EI_OSABI] == ELFOSABI_LINUX (3).
pub fn is_linux_elf(elf_bytes: &[u8]) -> bool {
    // ELF64 e_ident[EI_OSABI] at offset 0x07
    const EI_OSABI: usize = 0x07;

    if elf_bytes.len() < 8 {
        return false;
    }

    // Check ELF magic first
    if elf_bytes[0..4] != [0x7f, b'E', b'L', b'F'] {
        return false;
    }

    // Check OSABI field
    elf_bytes[EI_OSABI] == sunlight_elf::ELFOSABI_LINUX
}
