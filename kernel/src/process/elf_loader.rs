use super::Process;
use crate::memory::pmm::PhysicalMemoryManager;
use crate::process::address_space::AddressSpace;
use crate::process::region::{
    MappingKind, MappingRegion, RegionBacking, RegionPolicy, RegionProtection, RegionReservation,
};
use sunlight_elf::{SegmentPlan, SegmentProt};
use x86_64::{
    structures::paging::{Page, PageTableFlags, PhysFrame},
    VirtAddr,
};

/// User-address window allowed for PT_LOAD segments. The stack and heap are
/// mapped separately above USER_HI, so a validated binary can never collide
/// with them or reach the kernel higher half.
const USER_LO: u64 = 0x1000;
const USER_HI: u64 = super::layout::USER_HEAP_START;

const MAX_ELF_SEGMENTS: usize = 8;
const MAX_ELF_REGION_RUNS: usize = MAX_ELF_SEGMENTS * 2;

fn region_protection(prot: SegmentProt) -> RegionProtection {
    match prot {
        SegmentProt::ReadExec => RegionProtection::READ_EXECUTE,
        SegmentProt::ReadWrite => RegionProtection::READ_WRITE,
        SegmentProt::Read => RegionProtection::READ_ONLY,
    }
}

fn prot_flags(prot: SegmentProt) -> PageTableFlags {
    AddressSpace::protection_to_pte_flags(region_protection(prot))
        .expect("validated ELF protection")
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

    // Collect into a fixed plan before publishing PTEs. Normal SunlightOS
    // desktop images have four PT_LOAD entries; excess input is rejected.
    let mut plans = heapless::Vec::<SegmentPlan, MAX_ELF_SEGMENTS>::new();
    let mut too_many_segments = false;
    let planned =
        sunlight_elf::plan_segments(elf_bytes, &header, USER_LO, USER_HI, &mut |plan| {
            if plans.push(*plan).is_err() {
                too_many_segments = true;
            }
        });
    if let Err(e) = planned {
        crate::serial_println!("[ELF] segment validation failed: {:?}", e);
        return None;
    }
    if too_many_segments || plans.is_empty() {
        crate::serial_println!("[ELF] segment plan exceeds bounded MM-2C limit");
        return None;
    }

    // PTE collision preflight covers the union before any allocation. Repeated
    // pages caused by overlapping ELF segments are checked harmlessly twice.
    for plan in &plans {
        for page_idx in 0..plan.page_count {
            let address = plan
                .vaddr_page_start
                .checked_add((page_idx as u64).checked_mul(4096)?)?;
            let page = Page::from_start_address(VirtAddr::new(address)).ok()?;
            if unsafe { process.address_space.is_occupied(page, hhdm_offset) } {
                crate::process::address_space::note_mapping_collision();
                return None;
            }
        }
    }

    let mut reservations = reserve_final_regions(&plans, process)?;
    let mut map_failed = false;
    for plan in &plans {
        if map_segment(plan, elf_bytes, process, pmm, hhdm_offset).is_none() {
            map_failed = true;
            break;
        }
    }
    if map_failed {
        rollback_elf_pages(&plans, process, pmm, hhdm_offset);
        while let Some(reservation) = reservations.pop() {
            process.address_space.cancel_region(reservation);
        }
        crate::serial_println!("[ELF] segment mapping failed (out of frames?)");
        return None;
    }

    for reservation in reservations {
        if process.address_space.commit_region(reservation).is_err() {
            rollback_elf_pages(&plans, process, pmm, hhdm_offset);
            crate::serial_println!("[ELF] ledger commit invariant failed");
            return None;
        }
    }

    Some(header.entry)
}

fn reserve_final_regions(
    plans: &[SegmentPlan],
    process: &Process,
) -> Option<heapless::Vec<RegionReservation, MAX_ELF_REGION_RUNS>> {
    let mut boundaries = heapless::Vec::<u64, MAX_ELF_REGION_RUNS>::new();
    for plan in plans {
        let end = plan
            .vaddr_page_start
            .checked_add((plan.page_count as u64).checked_mul(4096)?)?;
        boundaries.push(plan.vaddr_page_start).ok()?;
        boundaries.push(end).ok()?;
    }
    boundaries.as_mut_slice().sort_unstable();

    let mut reservations = heapless::Vec::<RegionReservation, MAX_ELF_REGION_RUNS>::new();
    for pair in boundaries.windows(2) {
        let start = pair[0];
        let end = pair[1];
        if start == end {
            continue;
        }
        let mut writable = false;
        let mut executable = false;
        let mut covered = false;
        for plan in plans {
            let plan_end = plan.vaddr_page_start + plan.page_count as u64 * 4096;
            if plan.vaddr_page_start < end && start < plan_end {
                covered = true;
                let protection = region_protection(plan.prot);
                writable |= protection.writable();
                executable |= protection.executable();
            }
        }
        if !covered || writable && executable {
            while let Some(reservation) = reservations.pop() {
                process.address_space.cancel_region(reservation);
            }
            return None;
        }
        let protection = RegionProtection::new(true, writable, executable).ok()?;
        let region = MappingRegion::new(
            start,
            end,
            protection,
            MappingKind::ElfSegment,
            RegionPolicy::SYSTEM.union(RegionPolicy::OWNER_MANAGED),
            RegionBacking::ElfImage(process.address_space.identity().generation),
        )
        .ok()?;
        match process.address_space.preflight_region(region) {
            Ok(reservation) => reservations.push(reservation).ok()?,
            Err(_) => {
                while let Some(reservation) = reservations.pop() {
                    process.address_space.cancel_region(reservation);
                }
                return None;
            }
        }
    }
    Some(reservations)
}

fn rollback_elf_pages(
    plans: &[SegmentPlan],
    process: &mut Process,
    pmm: &mut PhysicalMemoryManager,
    hhdm_offset: VirtAddr,
) {
    for plan in plans.iter().rev() {
        for page_idx in (0..plan.page_count).rev() {
            let Some(address) = plan
                .vaddr_page_start
                .checked_add(page_idx as u64 * 4096)
            else {
                continue;
            };
            let Ok(page) = Page::from_start_address(VirtAddr::new(address)) else {
                continue;
            };
            let Some((frame, flags)) = (unsafe {
                process.address_space.lookup_entry(page, hhdm_offset)
            }) else {
                continue;
            };
            if !flags.contains(PageTableFlags::PRESENT) {
                continue;
            }
            if unsafe {
                process
                    .address_space
                    .rollback_mapped_page(page, frame, pmm, hhdm_offset)
            }
            .is_ok()
            {
                pmm.free_frame(frame);
            }
        }
    }
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
        plan.file_offset,
        plan.vaddr,
        plan.file_size,
        plan.mem_size,
        plan.prot
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
        let existing = unsafe { process.address_space.lookup_entry(page, hhdm_offset) };

        let (frame_addr, existing_flags) = match existing {
            Some((phys, old_flags)) => (phys, Some(old_flags)),
            None => (pmm.alloc_frame_owned(process.pid as u32)?, None),
        };

        let phys = unsafe { PhysFrame::from_start_address_unchecked(frame_addr) };
        let hhdm_ptr = (hhdm_offset + frame_addr.as_u64()).as_mut_ptr::<u8>();

        if existing_flags.is_none() {
            // Zero the new frame before copying segment data into it.
            unsafe {
                core::ptr::write_bytes(hhdm_ptr, 0, 4096);
            }
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
                    if process
                        .address_space
                        .map_page(page, phys, flags, pmm, hhdm_offset)
                        .is_err()
                    {
                        pmm.free_frame(frame_addr);
                        return None;
                    }
                }
            }
            Some(old_flags) if old_flags != flags => {
                // Shared page with different protections: union them so e.g.
                // a .data byte in a mostly-.rodata page stays writable.
                let merged = union_flags(old_flags, flags);
                if merged.contains(PageTableFlags::WRITABLE)
                    && !merged.contains(PageTableFlags::NO_EXECUTE)
                {
                    crate::memory::security::note_rwx_mapping_rejected();
                    return None;
                }
                unsafe {
                    if process
                        .address_space
                        .update_flags(page, old_flags, merged, hhdm_offset)
                        .is_err()
                    {
                        return None;
                    }
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
