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
    #[cfg(feature = "helios_gnu_interpreter_probe")]
    const GNU_INTERPRETER: &[u8] = b"/lib64/ld-linux-x86-64.so.2";
    #[cfg(feature = "helios_gnu_interpreter_probe")]
    const GNU_RESOURCE: &[u8] = include_bytes!(env!("SUNLIGHT_HELIOS_GNU_INTERPRETER"));
    const PIE_BASE: u64 = 0x2000_0000;
    const INTERP_BASE: u64 = 0x8000_0000;
    const INTERP_LIMIT: u64 = 0x8100_0000;
    #[cfg(feature = "helios_dynamic_elf_tls_fixture")]
    #[path = "static_tls.rs"]
    pub(in crate::process) mod tls;
    #[cfg(feature = "helios_dynamic_elf_shared_fixture")]
    pub struct TrustedLibrary {
        app: &'static [u8],
        executable: &'static [u8],
        bundle: &'static str,
        relative: &'static str,
        expected: &'static [u8],
    }
    #[cfg(feature = "helios_dynamic_elf_shared_fixture")]
    pub const REAL_APP: &[u8] = include_bytes!(concat!(
        env!("SUNLIGHT_DYNAMIC_PROBES"), "/elf-real.sunapp/Program/probe"
    ));
    #[cfg(feature = "helios_dynamic_elf_shared_fixture")]
    pub const GRAPH_APP: &[u8] = include_bytes!(concat!(
        env!("SUNLIGHT_DYNAMIC_PROBES"), "/elf-graph.sunapp/Program/probe"
    ));
    #[cfg(feature = "helios_dynamic_elf_tls_fixture")]
    pub const TLS_SHARED_APP: &[u8] = include_bytes!(concat!(
        env!("SUNLIGHT_DYNAMIC_PROBES"), "/elf-tls-shared.sunapp/Program/probe"
    ));
    fn is_tls_shared(bytes: &[u8]) -> bool {
        #[cfg(feature = "helios_dynamic_elf_tls_fixture")]
        return bytes == TLS_SHARED_APP;
        #[cfg(not(feature = "helios_dynamic_elf_tls_fixture"))]
        { let _ = bytes; false }
    }
    #[cfg(feature = "helios_dynamic_elf_shared_fixture")]
    pub fn trusted_library(path: &[u8]) -> Option<&'static TrustedLibrary> {
        macro_rules! selection {
            ($bundle:literal, $name:literal) => {
                TrustedLibrary {
                    app: concat!($bundle, "/Program/probe").as_bytes(),
                    executable: include_bytes!(concat!(env!("SUNLIGHT_DYNAMIC_PROBES"),
                        "/", $bundle, "/Program/probe")),
                    bundle: $bundle,
                    relative: concat!("Program/", $name),
                    expected: include_bytes!(concat!(env!("SUNLIGHT_DYNAMIC_PROBES"),
                        "/", $bundle, "/Program/", $name)),
                }
            };
        }
        static TRUSTED: &[TrustedLibrary] = &[
            selection!("elf-shared.sunapp", "libhelios-value.so"),
            selection!("elf-real.sunapp", "libhelios-math.so"),
            #[cfg(feature = "helios_runtime_c_dso")]
            selection!("elf-c-dso.sunapp", "libhelios-util.so"),
            selection!("elf-graph.sunapp", "libhelios_a.so"),
            #[cfg(feature = "helios_dynamic_elf_tls_fixture")]
            selection!("elf-tls-shared.sunapp", "libhelios_tls.so"),
        ];
        let relative = path.strip_prefix(b"/Applications/")?;
        TRUSTED.iter().find(|entry| entry.app == relative)
    }
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
        #[cfg(feature = "helios_runtime_c_startup")]
        CStartupOverflow,
        #[cfg(feature = "helios_runtime_c_startup")]
        CStartupBeforeEntry,
        #[cfg(feature = "helios_runtime_c_startup")]
        CStartupArgc,
        #[cfg(feature = "helios_runtime_c_startup")]
        CStartupArgv,
        #[cfg(feature = "helios_runtime_c_startup")]
        CStartupString,
        #[cfg(feature = "helios_runtime_c_startup")]
        CStartupEnvp,
        #[cfg(feature = "helios_runtime_c_startup")]
        CStartupAuxv,
        #[cfg(feature = "helios_runtime_c_startup")]
        CStartupAlignment,
        #[cfg(feature = "helios_dynamic_elf_tls_fixture")]
        Tls(tls::Fault),
        #[cfg(feature = "helios_dynamic_elf_tls_fixture")]
        StartupTls(tls::StartupFault),
        #[cfg(feature = "helios_dynamic_elf_relocation_fixture")]
        RelocationWrite,
        #[cfg(feature = "helios_dynamic_elf_relocation_fixture")]
        PostRelocationAllocation,
        #[cfg(feature = "helios_dynamic_elf_shared_fixture")]
        Shared(SharedFault),
    }

    #[cfg(feature = "helios_dynamic_elf_shared_fixture")]
    #[derive(Clone, Copy, PartialEq, Eq)]
    pub enum SharedFault {
        Needed, Missing, Identity, Malformed, Machine, Mapping, Dynamic, Symbol, Relocation, Stack, Auxv, Process,
        Metadata(sunlight_elf::shared_fixture::TestFault),
        Graph(GraphFault),
    }

    pub struct Handoff {
        pub header: sunlight_elf::ElfHeader,
        pub phdr: u64,
        pub base: u64,
        pub app_entry: u64,
        pub entry: u64,
        #[cfg(feature = "helios_dynamic_elf_tls_fixture")]
        pub thread_pointer: u64,
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
        #[cfg(feature = "helios_dynamic_elf_shared_fixture")] shared: Option<&TrustedLibrary>,
    ) -> Result<Option<Handoff>, &'static str> {
        if !process.is_linux_compat() {
            return Ok(None);
        }
        #[cfg(feature = "helios_dynamic_elf_shared_fixture")]
        if shared.is_some_and(|entry| entry.executable != bytes) {
            return Err("trusted bundle application identity");
        }
        #[cfg(feature = "helios_dynamic_elf_shared_fixture")]
        let parsed = if shared.is_some() && (bytes == REAL_APP || bytes == GRAPH_APP || is_tls_shared(bytes)) {
            DynamicImage::parse_bundle_image(bytes)
        } else {
            DynamicImage::parse(bytes)
        };
        #[cfg(not(feature = "helios_dynamic_elf_shared_fixture"))]
        let parsed = DynamicImage::parse(bytes);
        let app = match parsed {
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
        let (resource, gnu) = match path {
            INTERPRETER => (RESOURCE, false),
            #[cfg(feature = "helios_gnu_interpreter_probe")]
            GNU_INTERPRETER if fault.is_none() => (GNU_RESOURCE, true),
            _ => return Err("PT_INTERP policy"),
        };
        let mut mutated_header = [0; 64];
        let resource = match fault {
            Some(Fault::Missing) => None,
            Some(Fault::Machine) if RESOURCE.len() >= 64 => {
                mutated_header.copy_from_slice(&RESOURCE[..64]);
                mutated_header[18..20].copy_from_slice(&183u16.to_le_bytes());
                Some(mutated_header.as_slice())
            }
            Some(Fault::Malformed) => Some(&RESOURCE[..RESOURCE.len().min(63)]),
            _ if resource.is_empty() => None,
            _ => Some(resource),
        }
        .ok_or("interpreter lookup")?;
        let ld = if gnu {
            DynamicImage::parse_bundle_image(resource)
        } else {
            DynamicImage::parse(resource)
        }
        .map_err(|error| {
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
        #[cfg(feature = "helios_dynamic_elf_tls_fixture")]
        let tls = if bytes == tls::APP || bytes == tls::INIT_APP || is_tls_shared(bytes) || matches!(fault, Some(Fault::Tls(_))) {
            if app.header.elf_type != 3 || (shared.is_some() != is_tls_shared(bytes)) {
                return Err("TLS model mismatch");
            }
            Some(tls::plan(bytes, app.header, bias, &app_plans, bytes == tls::APP || matches!(fault, Some(Fault::Tls(_))))?)
        } else {
            None
        };
        let phdr = app
            .program_headers(&app_plans)
            .map_err(|_| "program-header handoff")?;
        crate::serial_println!(
            "[HELIOS-HANDOFF] interpreter={} source=kernel pid={}",
            if gnu { "/lib64/ld-linux-x86-64.so.2" } else { "/lib/helios/ld-handoff.so.1" },
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
        #[cfg(feature = "helios_dynamic_elf_tls_fixture")]
        let thread_pointer = match tls {
            Some(plan) => tls::install(plan, bytes, process, pmm, hhdm, fault)?,
            None => 0,
        };
        #[cfg(feature = "helios_dynamic_elf_tls_fixture")]
        if is_tls_shared(bytes) {
            let resolver = INTERP_BASE.checked_add(0x4000).ok_or("startup TLS resolver")?;
            if !ld_plans.iter().any(|segment| segment.prot == sunlight_elf::SegmentProt::ReadExec
                && resolver == segment.vaddr && segment.file_size != 0) {
                return Err("startup TLS resolver");
            }
            load_shared(shared.ok_or("startup TLS trust")?, &app, &app_plans, bias,
                process, pmm, hhdm, fault, INTERP_BASE)?;
        }
        let fail_interpreter = fault == Some(Fault::Interpreter);
        #[cfg(feature = "helios_dynamic_elf_tls_fixture")]
        let fail_interpreter = fail_interpreter || fault == Some(Fault::Tls(tls::Fault::Interpreter))
            || fault == Some(Fault::StartupTls(tls::StartupFault::Interpreter));
        if fail_interpreter {
            pmm.fail_test_allocations_after(3);
        }
        let mapped = map_image(&ld_plans, resource, process, pmm, hhdm);
        if fail_interpreter {
            pmm.clear_test_allocation_failure();
        }
        mapped.ok_or("interpreter mapping")?;
        if gnu {
            return Ok(Some(Handoff {
                header: app.header,
                phdr,
                base: INTERP_BASE,
                app_entry: app.header.entry + bias,
                entry: ld.header.entry + INTERP_BASE,
                #[cfg(feature = "helios_dynamic_elf_tls_fixture")]
                thread_pointer: 0,
            }));
        }
        #[cfg(feature = "helios_dynamic_elf_shared_fixture")]
        if let Some(shared) = shared {
            if bytes == GRAPH_APP {
                load_graph(shared, &app, &app_plans, bias, process, pmm, hhdm, fault)?;
            } else if !is_tls_shared(bytes) {
                load_shared(shared, &app, &app_plans, bias, process, pmm, hhdm, fault, INTERP_BASE)?;
            }
            return Ok(Some(Handoff {
                header: app.header,
                phdr,
                base: INTERP_BASE,
                app_entry: app.header.entry + bias,
                entry: ld.header.entry + INTERP_BASE,
                #[cfg(feature = "helios_dynamic_elf_tls_fixture")]
                thread_pointer,
            }));
        }
        #[cfg(feature = "helios_dynamic_elf_tls_fixture")]
        if thread_pointer != 0 {
            return Ok(Some(Handoff {
                header: app.header,
                phdr,
                base: INTERP_BASE,
                app_entry: app.header.entry + bias,
                entry: ld.header.entry + INTERP_BASE,
                thread_pointer,
            }));
        }
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
            #[cfg(feature = "helios_dynamic_elf_tls_fixture")]
            thread_pointer: 0,
        }))
    }

    #[cfg(feature = "helios_dynamic_elf_shared_fixture")]
    #[derive(Clone, Copy, PartialEq, Eq)]
    pub enum GraphFault {
        Missing(usize), Hash(usize), Undeclared, Cycle, Path, Duplicate,
        Unresolved, Ambiguous, Mapping, LeafRelocation, PageTable, NeededOffset,
    }

    #[cfg(feature = "helios_dynamic_elf_shared_fixture")]
    fn load_graph(
        trusted: &TrustedLibrary, app: &DynamicImage<'_>, app_plans: &[SegmentPlan],
        bias: u64, process: &mut Process, pmm: &mut PhysicalMemoryManager, hhdm: VirtAddr,
        fault: Option<Fault>,
    ) -> Result<(), &'static str> {
        use sha2::{Digest, Sha256};
        use sunlight_elf::shared_fixture::{Tables, TestFault};

        const WINDOW: u64 = 0x0100_0000;
        let approved = [
            (trusted.relative, trusted.expected),
            ("Program/libhelios_b.so", include_bytes!(concat!(
                env!("SUNLIGHT_DYNAMIC_PROBES"), "/elf-graph.sunapp/Program/libhelios_b.so"
            )).as_slice()),
        ];
        let fails = |stage| fault == Some(Fault::Shared(SharedFault::Graph(stage)));
        let root = Tables::parse(app, app_plans, bias, false)?;
        let mut next = root.needed(fails(GraphFault::NeededOffset))?;
        let mut seen = [false; 2];
        let mut images = heapless::Vec::<_, 2>::new();
        let mut owner = "Program/probe";
        while let Some(name) = next {
            let name = if fails(GraphFault::Undeclared) { b"undeclared.so".as_slice() }
                else if fails(GraphFault::Path) { b"../libhelios_a.so".as_slice() }
                else { name };
            sunlight_elf::shared_fixture::validate_dependency_name(name)?;
            let index = approved.iter().position(|(path, _)| path.as_bytes().strip_prefix(b"Program/") == Some(name))
                .ok_or("trusted dependency policy")?;
            if core::mem::replace(&mut seen[index], true) {
                return Err("cycle detection");
            }
            if index != images.len() {
                return Err("dependency graph traversal");
            }
            let (path, expected) = approved[index];
            let bytes = sunlight_fs::INITRAMFS.iter().find(|entry| {
                !fails(GraphFault::Missing(index)) && !entry.is_dir
                    && entry.path.strip_prefix("/Applications/").and_then(|path| path.split_once('/'))
                        == Some((trusted.bundle, path))
            }).map(|entry| entry.data).ok_or("trusted dependency lookup")?;
            let mut digest = Sha256::digest(expected);
            if fails(GraphFault::Hash(index)) {
                digest[0] ^= 1;
            }
            if Sha256::digest(bytes) != digest {
                return Err("hash validation");
            }
            let image = DynamicImage::parse_bundle_image(bytes).map_err(|_| "library ELF validation")?;
            let window = INTERP_LIMIT + WINDOW * (2 * (process.pid as u64 % 8) + index as u64);
            let base = image.library_bias(window).map_err(|_| "load bias")?;
            let mut plans = Plans::new();
            let mut overflow = false;
            image.plan_library((window, window + WINDOW), base, &mut |segment| {
                overflow |= plans.push(*segment).is_err();
            }).map_err(|_| "library segment validation")?;
            if overflow {
                return Err("library segment validation");
            }
            let tables = Tables::parse(&image, &plans, base, true)?;
            tables.identity(true, name)?;
            next = tables.needed(false)?;
            if index == 1 && fails(GraphFault::Cycle) {
                next = approved[0].0.as_bytes().strip_prefix(b"Program/");
            }
            crate::serial_println!(
                "[HELIOS-GRAPH] pid={} owner={} needed={} hash=verified order={} base={:#x} app_base={:#x} interpreter_base={:#x}",
                process.pid, owner, path, index, base, bias, INTERP_BASE
            );
            owner = path;
            images.push((image, plans, base, path, bytes)).map_err(|_| "dependency graph traversal")?;
        }
        if seen != [true; 2] {
            return Err("dependency graph traversal");
        }
        for (index, (_, plans, _, _, bytes)) in images.iter().enumerate() {
            let fail_mapping = index == 1 && fails(GraphFault::Mapping);
            let fail_table = index == 1 && fails(GraphFault::PageTable);
            if fail_mapping {
                pmm.fail_test_allocations_after(3);
            } else if fail_table {
                pmm.fail_test_allocations_after(1);
            }
            let mapped = map_image(plans, bytes, process, pmm, hhdm);
            if fail_mapping || fail_table {
                pmm.clear_test_allocation_failure();
            }
            mapped.ok_or(if fail_table { "page-table allocation" } else { "image mapping" })?;
        }
        let [(first, first_plans, first_bias, first_path, _), (second, second_plans, second_bias, second_path, _)] =
            images.as_slice() else { return Err("dependency graph traversal") };
        let first = Tables::parse(first, first_plans, *first_bias, true)?;
        let second = Tables::parse(second, second_plans, *second_bias, true)?;
        let scope = [("Program/probe", &root), (*first_path, &first), (*second_path, &second)];
        for (owner_index, (owner, tables)) in scope.iter().enumerate() {
            let relocation_fault = (owner_index == 2 && fails(GraphFault::LeafRelocation))
                .then_some(TestFault::RelocationType);
            let (address, index, addend) = tables.jump_slot(relocation_fault)?;
            let symbol = tables.slot_symbol(index, None, true)?;
            let mut provider = None;
            for (candidate_index, (path, candidate)) in scope.iter().enumerate() {
                let test = if fails(GraphFault::Duplicate) && candidate_index == 1 {
                    Some(TestFault::DuplicateExport)
                } else if fails(GraphFault::Unresolved) {
                    Some(TestFault::MissingExport)
                } else if fails(GraphFault::Ambiguous) && owner_index == 0 && candidate_index == 2 {
                    Some(TestFault::AlternateExport)
                } else {
                    None
                };
                if let Some(value) = candidate.find_export(symbol, test)? {
                    if provider.replace((*path, value)).is_some() {
                        return Err("ambiguous symbol");
                    }
                }
            }
            let (path, resolved) = provider.ok_or("unresolved symbol")?;
            let value = resolved.checked_add_signed(addend).ok_or("relocation addend")?;
            write_jump_slot(process, hhdm, address, value)?;
            crate::serial_println!(
                "[HELIOS-GRAPH] pid={} image={} type=R_X86_64_JUMP_SLOT address={:#x} symbol={} provider={} resolved={:#x} addend={} value={:#x}",
                process.pid, owner, address, core::str::from_utf8(symbol).map_err(|_| "symbol name")?,
                path, resolved, addend, value
            );
        }
        Ok(())
    }

    #[cfg(feature = "helios_dynamic_elf_shared_fixture")]
    fn write_jump_slot(process: &Process, hhdm: VirtAddr, address: u64, value: u64) -> Result<(), &'static str> {
        let page = Page::containing_address(VirtAddr::new(address));
        let (frame, flags) = unsafe { process.address_space.lookup_entry(page, hhdm) }
            .ok_or("GOT/PLT")?;
        if !flags.contains(PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE) {
            return Err("GOT/PLT");
        }
        let physical = frame.as_u64().checked_add(address & 4095).ok_or("relocation")?;
        unsafe { (hhdm + physical).as_mut_ptr::<u64>().write(value.to_le()) };
        Ok(())
    }

    #[cfg(feature = "helios_runtime_c_dso")]
    fn load_c_utility(
        app: &sunlight_elf::shared_fixture::Tables<'_, '_>,
        library: &sunlight_elf::shared_fixture::Tables<'_, '_>,
        process: &Process,
        hhdm: VirtAddr,
        fault: Option<Fault>,
    ) -> Result<(), &'static str> {
        use sunlight_elf::shared_fixture::TestFault;
        const NAMES: [&[u8]; 5] = [
            b"helios_memset", b"helios_memcpy", b"helios_strlen", b"helios_write", b"helios_exit",
        ];
        let fails = |stage| fault == Some(Fault::Shared(stage));
        library.utility_exports(&NAMES, fails(SharedFault::Needed))?;
        if fails(SharedFault::Symbol) {
            return Err("utility setup after mapping");
        }
        let metadata = match fault {
            Some(Fault::Shared(SharedFault::Metadata(test))) => Some(test),
            _ => None,
        };
        let mut resolved = [(0, 0, 0); NAMES.len()];
        for (address, index, _) in app.jump_slots::<{ NAMES.len() }>(metadata)? {
            let symbol = app.import(index, None)?;
            let symbol = if metadata == Some(TestFault::UndefinedImport) {
                b"unsupported_utility".as_slice()
            } else {
                symbol
            };
            let index = NAMES.iter().position(|name| *name == symbol).ok_or("unsupported utility import")?;
            if resolved[index].0 != 0 {
                return Err("duplicate utility import");
            }
            resolved[index] = (address, library.export(symbol, metadata)?, index);
        }
        if fails(SharedFault::Relocation) {
            return Err("utility setup after resolution");
        }
        for (address, value, index) in resolved {
            write_jump_slot(process, hhdm, address, value)?;
            crate::serial_println!(
                "[C-DSO-RELOC] pid={} hash=verified type=R_X86_64_JUMP_SLOT symbol={} slot={:#x} target={:#x}",
                process.pid, core::str::from_utf8(NAMES[index]).map_err(|_| "utility symbol")?, address, value
            );
        }
        Ok(())
    }

    #[cfg(feature = "helios_dynamic_elf_shared_fixture")]
    fn load_shared(
        trusted: &TrustedLibrary,
        app: &DynamicImage<'_>, app_plans: &[SegmentPlan], bias: u64,
        process: &mut Process, pmm: &mut PhysicalMemoryManager, hhdm: VirtAddr,
        fault: Option<Fault>, interpreter_base: u64,
    ) -> Result<(), &'static str> {
        use sunlight_elf::shared_fixture::Tables;
        use sha2::{Digest, Sha256};
        const WINDOW_SIZE: u64 = 0x0100_0000;
        let window_start = INTERP_LIMIT + WINDOW_SIZE * (process.pid as u64 % 8);
        let name = trusted.relative.strip_prefix("Program/").filter(|name| {
            !name.is_empty() && *name != "." && *name != ".."
                && name.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
        }).ok_or("trusted library path")?;
        let fails = |stage| fault == Some(Fault::Shared(stage));
        if app.header.elf_type != 3 || fails(SharedFault::Dynamic) {
            return Err("dynamic-table parsing");
        }
        let startup_tls = is_tls_shared(trusted.executable);
        let tables = if startup_tls {
            Tables::parse_startup_tls(app, app_plans, bias, false)?
        } else {
            Tables::parse(app, app_plans, bias, false)?
        };
        if fails(SharedFault::Needed) && trusted.bundle != "elf-c-dso.sunapp" {
            return Err("DT_NEEDED validation");
        }
        tables.identity(false, name.as_bytes())?;
        let resource = sunlight_fs::INITRAMFS.iter().find(|entry| {
            !entry.is_dir && entry.path.strip_prefix("/Applications/")
                .and_then(|path| path.split_once('/')) == Some((trusted.bundle, trusted.relative))
        }).map(|entry| entry.data).ok_or("trusted library lookup")?;
        let mut expected = Sha256::digest(trusted.expected);
        if fails(SharedFault::Identity) {
            expected[0] ^= 1;
        }
        if Sha256::digest(resource) != expected {
            return Err("trusted library hash");
        }
        let mut header = [0; 64];
        let resource = if fails(SharedFault::Missing) {
            return Err("trusted library lookup");
        } else if fails(SharedFault::Malformed) {
            &resource[..63]
        } else if fails(SharedFault::Machine) {
            header.copy_from_slice(&resource[..64]);
            header[18..20].copy_from_slice(&183u16.to_le_bytes());
            &header[..]
        } else {
            resource
        };
        let library = DynamicImage::parse_bundle_image(resource).map_err(|_| "library ELF validation")?;
        let base = library.library_bias(window_start).map_err(|_| "load-bias calculation")?;
        let mut plans = Plans::new();
        let mut overflow = false;
        library.plan_library((window_start, window_start + WINDOW_SIZE), base, &mut |segment| {
            overflow |= plans.push(*segment).is_err();
        }).map_err(|_| "library segment validation")?;
        if overflow {
            return Err("library segment validation");
        }
        if fails(SharedFault::Mapping) {
            pmm.fail_test_allocations_after(3);
        }
        let mapped = map_image(&plans, resource, process, pmm, hhdm);
        if fails(SharedFault::Mapping) {
            pmm.clear_test_allocation_failure();
        }
        mapped.ok_or("library mapping")?;
        crate::serial_println!(
            "[HELIOS-SHARED] pid={} source=embedded-bundle library={} app_base={:#x} interpreter_base={:#x} library_base={:#x}",
            process.pid, trusted.relative, bias, interpreter_base, base
        );
        let exports = if startup_tls {
            Tables::parse_startup_tls(&library, &plans, base, true)?
        } else {
            Tables::parse(&library, &plans, base, true)?
        };
        exports.identity(true, name.as_bytes())?;
        #[cfg(feature = "helios_runtime_c_dso")]
        if trusted.bundle == "elf-c-dso.sunapp" {
            return load_c_utility(&tables, &exports, process, hhdm, fault);
        }
        #[cfg(feature = "helios_dynamic_elf_tls_fixture")]
        if startup_tls {
            let startup_fault = match fault {
                Some(Fault::StartupTls(fault)) => Some(fault),
                _ => None,
            };
            let (index, slot) = exports.startup_tls_relocations(
                startup_fault == Some(tls::StartupFault::Relocation),
            )?;
            let module = tls::install_shared(trusted.executable, app.header, app_plans, bias,
                resource, library.header, base, &plans,
                process, pmm, hhdm, index, startup_fault)?;
            write_jump_slot(process, hhdm, index, module)?;
            write_jump_slot(process, hhdm, slot, interpreter_base.checked_add(0x4000).ok_or("startup TLS resolver")?)?;
            crate::serial_println!("[HELIOS-TLS-SHARED] pid={} hash=verified R_X86_64_DTPMOD64 module=2 offset=0 R_X86_64_JUMP_SLOT symbol=__tls_get_addr", process.pid);
        }
        if fails(SharedFault::Symbol) {
            return Err("symbol-table validation");
        }
        let metadata_fault = match fault {
            Some(Fault::Shared(SharedFault::Metadata(fault))) => Some(fault),
            _ => None,
        };
        let (address, index, addend) = tables.jump_slot(metadata_fault)?;
        let symbol = tables.import(index, metadata_fault)?;
        let resolved = exports.export(symbol, metadata_fault)?;
        let value = resolved.checked_add_signed(addend).ok_or("relocation addend")?;
        write_jump_slot(process, hhdm, address, value)?;
        crate::serial_println!(
            "[HELIOS-SHARED] pid={} image=Program/probe type=R_X86_64_JUMP_SLOT address={:#x} symbol={} resolved={:#x} addend={} value={:#x}",
            process.pid, address, core::str::from_utf8(symbol).map_err(|_| "symbol name")?, resolved, addend, value
        );
        if fails(SharedFault::Relocation) {
            return Err("relocation");
        }
        Ok(())
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
    #[cfg(all(debug_assertions, feature = "helios_dynamic_elf_tls_fixture"))]
    if elf_bytes == handoff::TLS_SHARED_APP {
        return sunlight_elf::Personality::Linux;
    }
    #[cfg(all(debug_assertions, feature = "helios_dynamic_elf_shared_fixture"))]
    if elf_bytes == handoff::REAL_APP || elf_bytes == handoff::GRAPH_APP {
        return sunlight_elf::Personality::Linux;
    }
    sunlight_elf::classify_personality(elf_bytes)
}

/// Detect if an ELF binary is a Linux-compatible ELF.
pub fn is_linux_elf(elf_bytes: &[u8]) -> bool {
    matches!(personality(elf_bytes), sunlight_elf::Personality::Linux)
}
