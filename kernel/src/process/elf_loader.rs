use super::Process;
use crate::memory::pmm::PhysicalMemoryManager;
use x86_64::{
    VirtAddr,
    structures::paging::{
        Page, PageTableFlags, PhysFrame,
    },
};

/// Minimal ELF64 parser: load PT_LOAD segments into the process address space.
pub fn load_elf(
    elf_bytes: &[u8],
    process: &mut Process,
    pmm: &mut PhysicalMemoryManager,
    hhdm_offset: VirtAddr,
) -> Option<u64> {
    // ELF64 header
    if elf_bytes.len() < 64 {
        crate::serial_println!("[ELF] too short: {}", elf_bytes.len());
        return None;
    }
    if elf_bytes[0..4] != [0x7f, b'E', b'L', b'F'] {
        crate::serial_println!("[ELF] bad magic");
        return None;
    }

    // e_entry at offset 0x18
    let entry = u64::from_le_bytes(elf_bytes[0x18..0x20].try_into().ok()?);

    // e_phoff at offset 0x20
    let phoff = u64::from_le_bytes(elf_bytes[0x20..0x28].try_into().ok()?);

    // e_phentsize at offset 0x36
    let phentsize = u16::from_le_bytes(elf_bytes[0x36..0x38].try_into().ok()?);

    // e_phnum at offset 0x38
    let phnum = u16::from_le_bytes(elf_bytes[0x38..0x3A].try_into().ok()?);

    for i in 0..phnum {
        let ph_start = phoff as usize + (i as usize * phentsize as usize);
        let ph_end = ph_start + phentsize as usize;
        if ph_end > elf_bytes.len() {
            crate::serial_println!("[ELF] phdr {} out of bounds", i);
            return None;
        }
        let ph = &elf_bytes[ph_start..ph_end];

        // p_type at offset 0x00
        let p_type = u32::from_le_bytes(ph[0x00..0x04].try_into().ok()?);
        if p_type != 1 {
            // Not PT_LOAD
            continue;
        }

        // p_offset at offset 0x08
        let p_offset = u64::from_le_bytes(ph[0x08..0x10].try_into().ok()?);
        // p_vaddr at offset 0x10
        let p_vaddr = u64::from_le_bytes(ph[0x10..0x18].try_into().ok()?);
        // p_filesz at offset 0x20
        let p_filesz = u64::from_le_bytes(ph[0x20..0x28].try_into().ok()?);
        // p_memsz at offset 0x28
        let p_memsz = u64::from_le_bytes(ph[0x28..0x30].try_into().ok()?);
        // p_flags at offset 0x04
        let p_flags = u32::from_le_bytes(ph[0x04..0x08].try_into().ok()?);

        crate::serial_println!(
            "[ELF] PT_LOAD off={:x} vaddr={:x} filesz={:x} memsz={:x} flags={:x}",
            p_offset, p_vaddr, p_filesz, p_memsz, p_flags
        );

        let flags = if p_flags & 0x1 != 0 {
            // PF_X: executable (read + execute)
            PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE
        } else if p_flags & 0x2 != 0 {
            // PF_W: writable data (read + write)
            PageTableFlags::PRESENT
                | PageTableFlags::WRITABLE
                | PageTableFlags::USER_ACCESSIBLE
                | PageTableFlags::NO_EXECUTE
        } else {
            // Read-only data
            PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE | PageTableFlags::NO_EXECUTE
        };

        let vaddr_start = p_vaddr;
        let vaddr_end = p_vaddr.saturating_add(p_memsz);
        let page_vaddr_start = vaddr_start & !0xFFF;
        let page_vaddr_end = (vaddr_end + 0xFFF) & !0xFFF;
        let memsz_pages = ((page_vaddr_end - page_vaddr_start) / 4096) as usize;

        crate::serial_println!(
            "[ELF] mapping {} pages from {:x} to {:x}",
            memsz_pages, page_vaddr_start, page_vaddr_end
        );

        for page_idx in 0..memsz_pages {
            let page_addr = VirtAddr::new(page_vaddr_start + page_idx as u64 * 4096);
            let page = Page::from_start_address(page_addr).ok()?;

            // Check if this virtual page is already mapped by a previous segment.
            // When two segments share a page (e.g., rodata and GOT both landing in
            // the same 4 KiB page), reuse the existing physical frame instead of
            // allocating a new one that would overwrite the previous segment's data.
            let existing_phys = unsafe {
                process.address_space.lookup_phys(page, hhdm_offset)
            };

            let (frame_addr, is_new) = if let Some(phys) = existing_phys {
                (phys, false)
            } else {
                (pmm.alloc_frame()?, true)
            };

            let phys = unsafe { PhysFrame::from_start_address_unchecked(frame_addr) };
            let hhdm_ptr = (hhdm_offset + frame_addr.as_u64()).as_mut_ptr::<u8>();

            if is_new {
                // Zero the new frame before copying segment data into it.
                unsafe { core::ptr::write_bytes(hhdm_ptr, 0, 4096); }
            }

            // Calculate overlap between this page and the segment
            let page_start = page_addr.as_u64();
            let page_end = page_start + 4096;
            let overlap_start = vaddr_start.max(page_start);
            let overlap_end = vaddr_end.min(page_end);

            if overlap_start < overlap_end {
                let file_offset = p_offset + (overlap_start - vaddr_start);
                let dst_offset = (overlap_start - page_start) as usize;
                let len = (overlap_end - overlap_start) as usize;

                if file_offset as usize + len <= elf_bytes.len() {
                    unsafe {
                        core::ptr::copy_nonoverlapping(
                            elf_bytes.as_ptr().add(file_offset as usize),
                            hhdm_ptr.add(dst_offset),
                            len,
                        );
                    }
                }
            }

            if is_new {
                // SAFETY: mapping a user page into the process address space.
                unsafe {
                    process.address_space.map_page(page, phys, flags, pmm, hhdm_offset);
                }
            }
            // If the page was already mapped, the data was written into the existing
            // frame. No remapping is needed; the original flags are preserved.
        }
    }

    Some(entry)
}
