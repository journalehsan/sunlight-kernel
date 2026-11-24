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
    let planned = sunlight_elf::plan_segments(elf_bytes, &header, USER_LO, USER_HI, &mut |plan| {
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

    map_image(&plans, elf_bytes, process, pmm, hhdm_offset)?;
    Some(header.entry)
}

fn map_image(
    plans: &[SegmentPlan],
    elf_bytes: &[u8],
    process: &mut Process,
    pmm: &mut PhysicalMemoryManager,
    hhdm_offset: VirtAddr,
) -> Option<()> {
    // PTE collision preflight covers the union before any allocation. Repeated
    // pages caused by overlapping ELF segments are checked harmlessly twice.
    for plan in plans {
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

    let mut reservations = reserve_final_regions(plans, process)?;
    let mut map_failed = false;
    for plan in plans {
        if map_segment(plan, elf_bytes, process, pmm, hhdm_offset).is_none() {
            map_failed = true;
            break;
        }
    }
    if map_failed {
        rollback_elf_pages(plans, process, pmm, hhdm_offset);
        while let Some(reservation) = reservations.pop() {
            process.address_space.cancel_region(reservation);
        }
        crate::serial_println!("[ELF] segment mapping failed (out of frames?)");
        return None;
    }

    for reservation in reservations {
        if process.address_space.commit_region(reservation).is_err() {
            rollback_elf_pages(plans, process, pmm, hhdm_offset);
            crate::serial_println!("[ELF] ledger commit invariant failed");
            return None;
        }
    }

    Some(())
}

#[cfg(all(debug_assertions, feature = "helios_dynamic_elf_handoff"))]
pub(super) mod handoff {
    use super::*;
    use sunlight_elf::{DynamicImage, ElfError};

    pub(in crate::process) const INTERPRETER: &[u8] = b"/lib/helios/ld-handoff.so.1";
    const RESOURCE: &[u8] = include_bytes!(env!("SUNLIGHT_HELIOS_HANDOFF_INTERPRETER"));
    const PIE_BASE: u64 = 0x2000_0000;
    const INTERP_BASE: u64 = 0x8000_0000;
    const INTERP_LIMIT: u64 = 0x8100_0000;
    type Plans = heapless::Vec<SegmentPlan, MAX_ELF_SEGMENTS>;

    #[derive(Clone, Copy, PartialEq, Eq)]
    pub enum Fault {
        Missing,
        Path,
        Machine,
        Malformed,
        Application,
        Interpreter,
        PageTable,
        Stack,
        Auxv,
        #[cfg(feature = "helios_dynamic_elf_relocation_fixture")]
        RelocationWrite,
        #[cfg(feature = "helios_dynamic_elf_relocation_fixture")]
        PostRelocationAllocation,
    }

    pub struct Handoff {
        pub header: sunlight_elf::ElfHeader,
        pub phdr: u64,
        pub base: u64,
        pub app_entry: u64,
        pub entry: u64,
    }

    fn plan(image: &DynamicImage<'_>, window: (u64, u64), bias: u64) -> Result<Plans, ElfError> {
        let mut plans = Plans::new();
        let mut overflow = false;
        image.plan(window, bias, &mut |segment| {
            overflow |= plans.push(*segment).is_err()
        })?;
        if overflow {
            return Err(ElfError::InvalidSegment);
        }
        Ok(plans)
    }

    pub fn load(
        bytes: &[u8],
        process: &mut Process,
        pmm: &mut PhysicalMemoryManager,
        hhdm: VirtAddr,
        fault: Option<Fault>,
    ) -> Result<Option<Handoff>, &'static str> {
        if !process.is_linux_compat() {
            return Ok(None);
        }
        let app = match DynamicImage::parse(bytes) {
            Ok(app) => app,
            Err(error) => {
                if sunlight_elf::parse_elf_header(bytes).is_ok_and(|header| {
                    sunlight_elf::for_each_load_segment(bytes, &header, |_| Ok(()))
                        != Err(ElfError::HasDynamicSection)
                }) {
                    return Ok(None);
                }
                crate::serial_println!("[HELIOS-HANDOFF] application validation: {:?}", error);
                return Err("ELF type validation / PT_INTERP policy");
            }
        };
        let Some(path) = app.interpreter else {
            return if app.header.elf_type == 2 {
                Ok(None)
            } else {
                Err("PT_INTERP policy")
            };
        };
        if path != INTERPRETER {
            return Err("PT_INTERP policy");
        }
        let mut mutated_header = [0; 64];
        let resource = match fault {
            Some(Fault::Missing) => None,
            Some(Fault::Machine) if RESOURCE.len() >= 64 => {
                mutated_header.copy_from_slice(&RESOURCE[..64]);
                mutated_header[18..20].copy_from_slice(&183u16.to_le_bytes());
                Some(mutated_header.as_slice())
            }
            Some(Fault::Malformed) => Some(&RESOURCE[..RESOURCE.len().min(63)]),
            _ if RESOURCE.is_empty() => None,
            _ => Some(RESOURCE),
        }
        .ok_or("interpreter lookup")?;
        let ld = DynamicImage::parse(resource).map_err(|error| {
            crate::serial_println!("[HELIOS-HANDOFF] interpreter validation: {:?}", error);
            "interpreter validation"
        })?;
        if ld.header.elf_type != 3 || ld.interpreter.is_some() {
            return Err("interpreter validation");
        }
        let ld_plans = plan(&ld, (INTERP_BASE, INTERP_LIMIT), INTERP_BASE)
            .map_err(|_| "interpreter validation")?;
        let bias = if app.header.elf_type == 3 {
            PIE_BASE
        } else {
            0
        };
        let app_plans =
            plan(&app, (USER_LO, INTERP_BASE), bias).map_err(|_| "application mapping plan")?;
        let phdr = app
            .program_headers(&app_plans)
            .map_err(|_| "program-header handoff")?;
        crate::serial_println!(
            "[HELIOS-HANDOFF] interpreter=/lib/helios/ld-handoff.so.1 source=kernel pid={}",
            process.pid
        );
        if let Some(budget) = match fault {
            Some(Fault::PageTable) => Some(2),
            Some(Fault::Application) => Some(4),
            _ => None,
        } {
            pmm.fail_test_allocations_after(budget);
        }
        let mapped = map_image(&app_plans, bytes, process, pmm, hhdm);
        if matches!(fault, Some(Fault::PageTable | Fault::Application)) {
            pmm.clear_test_allocation_failure();
        }
        mapped.ok_or(if fault == Some(Fault::PageTable) {
            "page-table allocation"
        } else {
            "application mapping"
        })?;
        if fault == Some(Fault::Interpreter) {
            pmm.fail_test_allocations_after(3);
        }
        let mapped = map_image(&ld_plans, resource, process, pmm, hhdm);
        if fault == Some(Fault::Interpreter) {
            pmm.clear_test_allocation_failure();
        }
        mapped.ok_or("interpreter mapping")?;
        use sunlight_elf::relative::Error;
        sunlight_elf::relative::apply(&app, &app_plans, phdr, |address, value| {
            let page = Page::containing_address(VirtAddr::new(address));
            let (frame, flags) = unsafe { process.address_space.lookup_entry(page, hhdm) }
                .ok_or(Error::Target(address))?;
            if !flags.contains(PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE) {
                return Err(Error::Permission(address));
            }
            let physical = frame.as_u64().checked_add(address & 4095)
                .ok_or(Error::Target(address))?;
            unsafe { (hhdm + physical).as_mut_ptr::<u64>().write(value.to_le()) };
            crate::serial_println!(
                "[HELIOS-RELOC] pid={} type=R_X86_64_RELATIVE address={:#x} value={:#x}",
                process.pid, address, value
            );
            #[cfg(feature = "helios_dynamic_elf_relocation_fixture")]
            if fault == Some(Fault::RelocationWrite) {
                return Err(Error::Target(address));
            }
            Ok(())
        }).map_err(|error| {
            crate::serial_println!("[HELIOS-RELOC] pid={} rejected: {:?}", process.pid, error);
            match error {
                Error::Discovery => "PT_DYNAMIC discovery",
                Error::Dynamic | Error::Tag(_) => "dynamic-table validation",
                Error::Table => "relocation-table validation",
                Error::Bias => "load-bias calculation",
                Error::Unsupported { .. } => "unsupported relocation",
                Error::Target(_) => "relocation target validation",
                Error::Permission(_) => "write-permission validation",
            }
        })?;
        Ok(Some(Handoff {
            header: app.header,
            phdr,
            base: INTERP_BASE,
            app_entry: app.header.entry + bias,
            entry: ld.header.entry + INTERP_BASE,
        }))
    }
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
        // Valid static toolchain images can leave alignment gaps between
        // PT_LOAD segments. Those intervals remain unmapped and need no ledger
        // reservation; rejecting them prevented musl/MuPDF images from loading.
        if !covered {
            continue;
        }
        if writable && executable {
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
            let Some(address) = plan.vaddr_page_start.checked_add(page_idx as u64 * 4096) else {
                continue;
            };
            let Ok(page) = Page::from_start_address(VirtAddr::new(address)) else {
                continue;
            };
            let Some((frame, flags)) =
                (unsafe { process.address_space.lookup_entry(page, hhdm_offset) })
            else {
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

/// Return the deterministic ABI personality selected by the ELF metadata.
pub fn personality(elf_bytes: &[u8]) -> sunlight_elf::Personality {
    sunlight_elf::classify_personality(elf_bytes)
}

/// Detect if an ELF binary is a Linux-compatible ELF.
pub fn is_linux_elf(elf_bytes: &[u8]) -> bool {
    matches!(personality(elf_bytes), sunlight_elf::Personality::Linux)
}
