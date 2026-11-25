use super::*;

pub(crate) const APP: &[u8] = include_bytes!(concat!(
    env!("SUNLIGHT_DYNAMIC_PROBES"),
    "/elf-tls.sunapp/Program/probe"
));
pub(super) const INIT_APP: &[u8] = include_bytes!(concat!(
    env!("SUNLIGHT_DYNAMIC_PROBES"),
    "/elf-tls-init.sunapp/Program/probe"
));
const PAGE: u64 = super::super::super::layout::USER_STACK_TOP
    - super::super::super::layout::USER_STACK_SIZE
    - 8192;
const TP: u64 = PAGE + 2048;
const DSO_PAGE: u64 = PAGE - 4096;
const RESOLVER_PAGE: u64 = DSO_PAGE - 4096;
const MODULE_PAGE: u64 = RESOLVER_PAGE - 4096;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum StartupFault {
    Module, Missing, Duplicate, Malformed, FileSize, Alignment, Unmapped,
    Allocation, TableAllocation, ResolverAllocation, Relocation, Index, Offset, Stack, Interpreter,
}

pub(super) fn install_shared(
    app_bytes: &[u8],
    app_header: sunlight_elf::ElfHeader,
    app_loads: &[SegmentPlan],
    app_bias: u64,
    library_bytes: &[u8],
    library_header: sunlight_elf::ElfHeader,
    bias: u64,
    loads: &[SegmentPlan],
    process: &mut Process,
    pmm: &mut PhysicalMemoryManager,
    hhdm: VirtAddr,
    index: u64,
    fault: Option<StartupFault>,
) -> Result<u64, &'static str> {
    use sha2::{Digest, Sha256};

    let mut modified;
    let bytes = if let Some(mutation) = match fault {
        Some(StartupFault::Malformed) => Some(Fault::Validation),
        Some(StartupFault::FileSize) => Some(Fault::FileSize),
        Some(StartupFault::Alignment) => Some(Fault::Alignment),
        Some(StartupFault::Unmapped) => Some(Fault::Unmapped),
        _ => None,
    } {
        modified = library_bytes.to_vec();
        mutate(&mut modified, mutation);
        modified.as_slice()
    } else {
        library_bytes
    };
    let app_plan = plan(app_bytes, app_header, app_bias, app_loads, false)?;
    let mut dso = plan(bytes, library_header, bias, loads, true)?;
    let alignment = |bytes, header| -> Option<u64> {
        let tls = headers(bytes, header)?.find(|ph| ph[..4] == [7, 0, 0, 0])?;
        Some(u64::from_le_bytes(tls[48..56].try_into().ok()?))
    };
    if (dso.file_size, dso.mem_size, alignment(bytes, library_header)) != (4, 8, Some(4))
        || (app_plan.file_size, app_plan.mem_size, alignment(app_bytes, app_header)) != (4, 4, Some(4))
    {
        return Err("startup DSO TLS layout");
    }
    dso.vaddr_page_start = DSO_PAGE;
    dso.vaddr = DSO_PAGE;
    let mut map = |segment: &SegmentPlan, data: &[u8], injection, stage| {
        let fail = fault == Some(injection);
        if fail {
            pmm.fail_test_allocations_after(0);
        }
        let mapped = map_image(core::slice::from_ref(segment), data, process, pmm, hhdm);
        if fail {
            pmm.clear_test_allocation_failure();
        }
        mapped.ok_or(stage)
    };
    map(&dso, bytes, StartupFault::Allocation, "startup DSO TLS allocation")?;

    let module_id = match fault {
        Some(StartupFault::Module) => 3,
        Some(StartupFault::Duplicate) => 1,
        Some(StartupFault::Missing) => 0,
        _ => 2,
    };
    let modules = [(1, app_plan, app_bytes), (module_id, dso, library_bytes)];
    let mut table = [0; 224];
    for (position, (id, segment, image)) in modules.iter().enumerate() {
        if *id != position as u64 + 1 {
            return Err("startup TLS module ID");
        }
        let row = &mut table[position * 112..][..112];
        row[..32].copy_from_slice(&Sha256::digest(image));
        for (field, value) in row[32..].chunks_exact_mut(8).zip([
            *id, 1, segment.file_offset, segment.file_size, segment.mem_size, 4,
            segment.vaddr, 0, if position == 0 { u64::MAX } else { 4 }, position as u64 + 1,
        ]) {
            field.copy_from_slice(&value.to_le_bytes());
        }
    }
    let metadata_segment = |address, size| SegmentPlan {
        vaddr: address, vaddr_page_start: address, page_count: 1,
        file_offset: 0, file_size: size, mem_size: size,
        prot: sunlight_elf::SegmentProt::Read,
    };
    map(&metadata_segment(MODULE_PAGE, table.len() as u64), &table,
        StartupFault::TableAllocation, "startup TLS module table allocation")?;
    let mut resolver = [0; 32];
    for (field, value) in resolver.chunks_exact_mut(8).zip([MODULE_PAGE, index, DSO_PAGE, 8]) {
        field.copy_from_slice(&value.to_le_bytes());
    }
    map(&metadata_segment(RESOLVER_PAGE, resolver.len() as u64), &resolver,
        StartupFault::ResolverAllocation, "startup TLS resolver metadata allocation")?;
    if fault == Some(StartupFault::Index) || index == 0 || index % 8 != 0 {
        return Err("invalid startup TLS index");
    }
    let offset = if fault == Some(StartupFault::Offset) { u64::MAX } else { 0 };
    if dso.vaddr.checked_add(offset).is_none() || offset >= dso.mem_size {
        return Err("invalid startup TLS offset");
    }
    super::write_jump_slot(process, hhdm, TP, RESOLVER_PAGE)?;
    crate::serial_println!(
        "[HELIOS-TLS-SHARED] pid={} module=1 image=app block=app-local-exec module=2 image=dso block=dso-local-dynamic filesz=4 memsz=8 symbol_offsets=0,4",
        process.pid
    );
    Ok(module_id)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Fault {
    Validation,
    MissingTail,
    Alignment,
    Unmapped,
    FileSize,
    Overflow,
    Allocation,
    Stack,
    Interpreter,
    Process,
}

pub(super) fn plan(
    bytes: &[u8],
    header: sunlight_elf::ElfHeader,
    bias: u64,
    loads: &[SegmentPlan],
    require_tail: bool,
) -> Result<SegmentPlan, &'static str> {
    const INVALID: &str = "PT_TLS validation";
    let mut headers = headers(bytes, header).ok_or(INVALID)?;
    let tls = headers.find(|ph| ph[..4] == [7, 0, 0, 0]).ok_or(INVALID)?;
    if headers.any(|ph| ph[..4] == [7, 0, 0, 0]) {
        return Err(INVALID);
    }
    let word = |offset| u64::from_le_bytes(tls[offset..offset + 8].try_into().unwrap());
    let (offset, vaddr, filesz, memsz, align) = (word(8), word(16), word(32), word(40), word(48));
    if !align.is_power_of_two() || align > 64 || vaddr % align != 0 || offset % align != 0 {
        return Err("PT_TLS validation: alignment");
    }
    if filesz > memsz || memsz == 0 {
        return Err(INVALID);
    }
    if require_tail && filesz == memsz {
        return Err("PT_TLS validation: missing BSS tail");
    }
    let size = memsz.checked_add(align - 1).ok_or(INVALID)? & !(align - 1);
    if size > 2048 || offset.checked_add(filesz).filter(|end| *end <= bytes.len() as u64).is_none() {
        return Err(INVALID);
    }
    let start = bias.checked_add(vaddr).ok_or(INVALID)?;
    let end = start.checked_add(memsz).ok_or(INVALID)?;
    let mut containing = loads.iter().filter(|load| {
        let Some(delta) = start.checked_sub(load.vaddr) else { return false };
        load.vaddr.checked_add(load.mem_size).is_some_and(|limit| end <= limit)
            && delta.checked_add(filesz).is_some_and(|limit| limit <= load.file_size)
            && load.file_offset.checked_add(delta) == Some(offset)
    });
    if containing.next().is_none() || containing.next().is_some() {
        return Err("TLS image mapping");
    }
    Ok(SegmentPlan {
        vaddr: TP.checked_sub(size).ok_or(INVALID)?,
        vaddr_page_start: PAGE,
        page_count: 1,
        file_offset: offset,
        file_size: filesz,
        mem_size: memsz,
        prot: sunlight_elf::SegmentProt::ReadWrite,
    })
}

fn headers(bytes: &[u8], header: sunlight_elf::ElfHeader) -> Option<core::slice::ChunksExact<'_, u8>> {
    let start = usize::try_from(header.phoff).ok()?;
    let end = start.checked_add(usize::from(header.phnum).checked_mul(56)?)?;
    Some(bytes.get(start..end)?.chunks_exact(56))
}

pub(crate) fn mutate(bytes: &mut [u8], fault: Fault) {
    let header = DynamicImage::parse_bundle_image(bytes).expect("TLS fixture").header;
    let index = headers(bytes, header).unwrap().position(|ph| ph[..4] == [7, 0, 0, 0]).unwrap();
    let start = header.phoff as usize + index * 56;
    let (offset, value): (usize, u64) = match fault {
        Fault::Validation => {
            bytes[start..start + 4].fill(0);
            return;
        }
        Fault::Alignment => (48, 3),
        Fault::Unmapped => (16, 0x1000_0000),
        Fault::MissingTail => (40, u64::from_le_bytes(bytes[start + 32..start + 40].try_into().unwrap())),
        Fault::FileSize => (32, 9),
        Fault::Overflow => (40, u64::MAX),
        _ => return,
    };
    bytes[start + offset..start + offset + 8].copy_from_slice(&value.to_le_bytes());
}

pub(super) fn install(
    plan: SegmentPlan,
    bytes: &[u8],
    process: &mut Process,
    pmm: &mut PhysicalMemoryManager,
    hhdm: VirtAddr,
    fault: Option<super::Fault>,
) -> Result<u64, &'static str> {
    let fail = fault == Some(super::Fault::Tls(Fault::Allocation));
    if fail {
        pmm.fail_test_allocations_after(0);
    }
    let mapped = map_image(core::slice::from_ref(&plan), bytes, process, pmm, hhdm);
    if fail {
        pmm.clear_test_allocation_failure();
    }
    mapped.ok_or("TLS allocation")?;
    crate::serial_println!(
        "[HELIOS-TLS] setup pid={} tp={:#x} image={:#x} filesz={} memsz={} model=local-exec",
        process.pid, TP, plan.vaddr, plan.file_size, plan.mem_size
    );
    Ok(TP)
}
