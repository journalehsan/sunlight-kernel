use super::{Process, ProcessState};
use crate::capability::{CapabilityBroker, CapabilityRights};
use crate::memory::pmm::PhysicalMemoryManager;
use crate::sched::Scheduler;
use x86_64::{
    structures::paging::{Page, PhysFrame},
    VirtAddr,
};

/// Extract the basename (the component after the last `/`) from a path.
/// Used to give spawned processes a meaningful name instead of "daemon".
pub fn name_from_path(path: &str) -> &str {
    path.rfind('/').map(|i| &path[i + 1..]).unwrap_or(path)
}

pub fn is_trusted_display_path(path: &str) -> bool {
    matches!(
        path,
        "/sbin/sunlight-display" | "/usr/sbin/sunlight-display"
    )
}

pub fn is_trusted_swap_admin_path(path: &str) -> bool {
    matches!(path, "/sbin/sunlight-swapd" | "/usr/sbin/sunlight-swapd")
}

pub fn is_trusted_zram_diagnostic_path(path: &str) -> bool {
    matches!(path, "/bin/freezram" | "/usr/bin/freezram")
}

pub fn is_trusted_pty_service_path(path: &str) -> bool {
    matches!(path, "/sbin/pty_server" | "/usr/sbin/pty_server")
}

pub fn is_trusted_lock_service_path(path: &str) -> bool {
    matches!(path, "/sbin/mezzo" | "/usr/sbin/mezzo")
}

pub fn is_trusted_session_service_path(path: &str) -> bool {
    matches!(
        path,
        "/sbin/sunlight-sessiond" | "/usr/sbin/sunlight-sessiond"
    )
}

pub fn is_trusted_wiseowl_braind_path(path: &str) -> bool {
    matches!(path, "/sbin/wiseowl-braind" | "/usr/sbin/wiseowl-braind")
}

pub fn is_trusted_wiseowl_console_path(path: &str) -> bool {
    matches!(path, "/bin/wiseowl" | "/usr/bin/wiseowl")
}

pub fn is_trusted_control_panel_path(path: &str) -> bool {
    matches!(
        path,
        "/bin/sunlight-control-panel" | "/usr/bin/sunlight-control-panel"
    )
}

const LOCK_PRESENTER_ENTRY_MAGIC: u64 = 0x4C4F_434B_5052_4553;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnError {
    NotFound,
    PermissionDenied,
    ElfLoadFailed,
    NoMemory,
    EntropyUnavailable,
    InvalidPath,
    UnsupportedPersonality,
}

/// Execute an ELF binary into the current process (re-exec semantics).
/// Tears down the old address space and loads a new binary.
/// Marshals argv/envp onto the new stack in SysV ABI format.
pub fn exec_into_process(
    bytes: &[u8],
    process: &mut Process,
    pmm: &mut PhysicalMemoryManager,
    hhdm_offset: VirtAddr,
    argv: &[&[u8]],
    envp: &[&[u8]],
    activate_on_success: bool,
) -> Result<u64, SpawnError> {
    let personality = match super::elf_loader::personality(bytes) {
        sunlight_elf::Personality::Native => super::ProcessPersonality::Native,
        sunlight_elf::Personality::Linux => {
            super::ProcessPersonality::Linux(super::LinuxProcessState::new())
        }
        sunlight_elf::Personality::Unknown => return Err(SpawnError::UnsupportedPersonality),
    };
    let new_address_space = unsafe {
        crate::process::address_space::AddressSpace::try_new(pmm, hhdm_offset)
            .map_err(|_| SpawnError::NoMemory)?
    };
    let mut old_address_space = core::mem::replace(&mut process.address_space, new_address_space);
    let old_trusted_display = process.trusted_display_service;
    let old_trusted_swap_admin = process.trusted_swap_admin_service;
    let old_trusted_zram_diagnostic = process.trusted_zram_diagnostic;
    let old_trusted_pty_service = process.trusted_pty_service;
    let old_trusted_session_service = process.trusted_session_service;
    let old_trusted_wiseowl_braind = process.trusted_wiseowl_braind;
    let old_trusted_wiseowl_console = process.trusted_wiseowl_console;
    let old_trusted_control_panel = process.trusted_control_panel;
    let old_personality = process.personality;
    process.trusted_display_service = false;
    process.trusted_swap_admin_service = false;
    process.trusted_zram_diagnostic = false;
    process.trusted_pty_service = false;
    process.trusted_session_service = false;
    process.trusted_wiseowl_braind = false;
    process.trusted_wiseowl_console = false;
    process.trusted_control_panel = false;

    process.personality = personality;
    if process.is_linux_compat() {
        crate::serial_println!("[EXEC] Linux ELF detected");
    }

    let result = build_exec_image(bytes, process, pmm, hhdm_offset, argv, envp);
    let entry = match result {
        Ok(entry) => entry,
        Err(error) => {
            let mut failed_address_space =
                core::mem::replace(&mut process.address_space, old_address_space);
            unsafe {
                failed_address_space.reclaim_user_space(pmm, hhdm_offset, true);
            }
            process.trusted_display_service = old_trusted_display;
            process.trusted_swap_admin_service = old_trusted_swap_admin;
            process.trusted_zram_diagnostic = old_trusted_zram_diagnostic;
            process.trusted_pty_service = old_trusted_pty_service;
            process.trusted_session_service = old_trusted_session_service;
            process.trusted_wiseowl_braind = old_trusted_wiseowl_braind;
            process.trusted_wiseowl_console = old_trusted_wiseowl_console;
            process.trusted_control_panel = old_trusted_control_panel;
            process.personality = old_personality;
            if !activate_on_success {
                unsafe {
                    process
                        .address_space
                        .reclaim_user_space(pmm, hhdm_offset, true);
                }
            }
            return Err(error);
        }
    };

    if activate_on_success {
        unsafe {
            process.address_space.activate();
        }
    }
    if !process.mapped_shared.is_empty() || !process.owned_shared.is_empty() {
        let new_address_space = core::mem::replace(&mut process.address_space, old_address_space);
        {
            let mut caps = crate::capability::CAP_BROKER.lock();
            crate::memory::shared::cleanup_shared_pages(process, pmm, &mut caps);
        }
        old_address_space = core::mem::replace(&mut process.address_space, new_address_space);
    }
    if old_trusted_swap_admin {
        crate::memory::zram::revoke_admin(process.pid, old_address_space.identity().generation);
    }
    unsafe {
        old_address_space.reclaim_user_space(pmm, hhdm_offset, true);
    }

    Ok(entry)
}

fn build_exec_image(
    bytes: &[u8],
    process: &mut Process,
    pmm: &mut PhysicalMemoryManager,
    hhdm_offset: VirtAddr,
    argv: &[&[u8]],
    envp: &[&[u8]],
) -> Result<u64, SpawnError> {
    let entry = super::elf_loader::load_elf(bytes, process, pmm, hhdm_offset)
        .ok_or(SpawnError::ElfLoadFailed)?;

    map_user_stack(process, pmm, hhdm_offset)?;

    // Build auxv for Linux-compat processes so musl's _start doesn't scan
    // past the stack top looking for AT_NULL and fault at USER_STACK_TOP.
    // AT_PHDR + AT_PHENT + AT_PHNUM are required for musl `__init_tls` to walk
    // program headers and find PT_TLS. Without AT_PHENT (size of each phdr),
    // the walk never advances, TLS stays zeroed, and Rust `OnceLock` treats the
    // all-zero Once state as COMPLETE — e.g. ratatui's layout cache then
    // null-derefs on first `Layout::split` (helios-note page fault at 0x48).
    let auxv: alloc::vec::Vec<(u64, u64)> = if process.is_linux_compat() {
        match sunlight_elf::parse_elf_header(bytes) {
            Ok(hdr) => {
                let at_phdr = compute_at_phdr(bytes, &hdr);
                alloc::vec![
                    (sunlight_compat_linux::abi::AT_PHDR, at_phdr),
                    (sunlight_compat_linux::abi::AT_PHENT, hdr.phentsize as u64),
                    (sunlight_compat_linux::abi::AT_PHNUM, hdr.phnum as u64),
                    (sunlight_compat_linux::abi::AT_PAGESZ, 4096u64),
                    (sunlight_compat_linux::abi::AT_ENTRY, hdr.entry),
                    (sunlight_compat_linux::abi::AT_UID, process.uid as u64),
                    (sunlight_compat_linux::abi::AT_EUID, process.uid as u64),
                    (sunlight_compat_linux::abi::AT_GID, process.gid as u64),
                    (sunlight_compat_linux::abi::AT_EGID, process.gid as u64),
                    (sunlight_compat_linux::abi::AT_SECURE, 0u64),
                ]
            }
            Err(_) => alloc::vec![],
        }
    } else {
        alloc::vec![]
    };

    // Setup stack with argv/envp/auxv
    let stack = setup_exec_stack(argv, envp, &auxv, process, hhdm_offset)?;

    process.init_context(entry, stack.rsp);
    // SysV-style register convenience on top of the canonical stack layout:
    // rdi=argc, rsi=argv, rdx=envp. _start can use either.
    process.set_initial_args(argv.len() as u64, stack.argv_ptr, stack.envp_ptr, 0);

    crate::serial_println!(
        "[EXEC] Loaded ELF entry={:#x}, stack={:#x}",
        entry,
        stack.rsp
    );
    Ok(entry)
}

pub fn map_user_stack(
    process: &mut Process,
    pmm: &mut PhysicalMemoryManager,
    hhdm_offset: VirtAddr,
) -> Result<(), SpawnError> {
    use crate::process::region::{
        MappingKind, MappingRegion, RegionBacking, RegionPolicy, RegionProtection,
    };

    let stack_pages = ((super::layout::USER_STACK_SIZE + 4095) / 4096) as usize;
    let stack_start = super::layout::USER_STACK_TOP
        .checked_sub(stack_pages as u64 * 4096)
        .ok_or(SpawnError::NoMemory)?;
    for index in 0..stack_pages {
        let address = stack_start + index as u64 * 4096;
        let page =
            Page::from_start_address(VirtAddr::new(address)).map_err(|_| SpawnError::NoMemory)?;
        if unsafe { process.address_space.is_occupied(page, hhdm_offset) } {
            crate::process::address_space::note_mapping_collision();
            return Err(SpawnError::NoMemory);
        }
    }
    let protection = RegionProtection::READ_WRITE;
    let region = MappingRegion::new(
        stack_start,
        super::layout::USER_STACK_TOP,
        protection,
        MappingKind::UserStack,
        RegionPolicy::SYSTEM.union(RegionPolicy::OWNER_MANAGED),
        RegionBacking::None,
    )
    .map_err(|_| SpawnError::NoMemory)?;
    let reservation = process
        .address_space
        .preflight_region(region)
        .map_err(|_| SpawnError::NoMemory)?;
    let flags = crate::process::address_space::AddressSpace::protection_to_pte_flags(protection)
        .map_err(|_| SpawnError::NoMemory)?;

    let mut installed = 0usize;
    while installed < stack_pages {
        let address = stack_start + installed as u64 * 4096;
        let page =
            Page::from_start_address(VirtAddr::new(address)).map_err(|_| SpawnError::NoMemory)?;
        let Some(frame_addr) = pmm.alloc_frame_owned(process.pid as u32) else {
            rollback_user_stack(process, stack_start, installed, pmm, hhdm_offset);
            process.address_space.cancel_region(reservation);
            return Err(SpawnError::NoMemory);
        };
        if !crate::memory::security::sanitize_user_frame(frame_addr, hhdm_offset) {
            pmm.free_frame(frame_addr);
            rollback_user_stack(process, stack_start, installed, pmm, hhdm_offset);
            process.address_space.cancel_region(reservation);
            return Err(SpawnError::NoMemory);
        }
        let frame = unsafe { PhysFrame::from_start_address_unchecked(frame_addr) };
        if unsafe {
            process
                .address_space
                .map_page(page, frame, flags, pmm, hhdm_offset)
        }
        .is_err()
        {
            pmm.free_frame(frame_addr);
            rollback_user_stack(process, stack_start, installed, pmm, hhdm_offset);
            process.address_space.cancel_region(reservation);
            return Err(SpawnError::NoMemory);
        }
        installed += 1;
    }
    if process.address_space.commit_region(reservation).is_err() {
        rollback_user_stack(process, stack_start, installed, pmm, hhdm_offset);
        return Err(SpawnError::NoMemory);
    }
    for _ in 0..installed {
        crate::memory::security::note_nx_stack_mapping();
    }
    Ok(())
}

fn rollback_user_stack(
    process: &mut Process,
    stack_start: u64,
    installed: usize,
    pmm: &mut PhysicalMemoryManager,
    hhdm_offset: VirtAddr,
) {
    for index in (0..installed).rev() {
        let Ok(page) = Page::from_start_address(VirtAddr::new(stack_start + index as u64 * 4096))
        else {
            continue;
        };
        let Some((frame, _)) = (unsafe { process.address_space.lookup_entry(page, hhdm_offset) })
        else {
            continue;
        };
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

/// Final stack state handed to the new process image.
struct ExecStack {
    rsp: u64,
    argv_ptr: u64,
    envp_ptr: u64,
}

/// Copy `bytes` into the process address space at user `vaddr`, walking the
/// page tables and writing through the HHDM. The target pages must already
/// be mapped (the user stack is mapped just before this runs).
fn copy_to_user(
    process: &Process,
    hhdm_offset: VirtAddr,
    vaddr: u64,
    bytes: &[u8],
) -> Result<(), SpawnError> {
    let mut written = 0usize;
    while written < bytes.len() {
        let current = vaddr + written as u64;
        let page_base = current & !0xFFF;
        let page = Page::from_start_address(VirtAddr::new(page_base))
            .map_err(|_| SpawnError::ElfLoadFailed)?;
        // SAFETY: hhdm_offset is the boot HHDM base.
        let phys = unsafe { process.address_space.lookup_phys(page, hhdm_offset) }
            .ok_or(SpawnError::NoMemory)?;

        let in_page = (current - page_base) as usize;
        let chunk = (4096 - in_page).min(bytes.len() - written);
        // SAFETY: phys is a mapped user frame; the HHDM window covers it.
        unsafe {
            core::ptr::copy_nonoverlapping(
                bytes.as_ptr().add(written),
                (hhdm_offset + phys.as_u64() + in_page as u64).as_mut_ptr::<u8>(),
                chunk,
            );
        }
        written += chunk;
    }
    Ok(())
}

/// Marshal argc/argv/envp/auxv onto the user stack per the SysV x86_64 ABI.
///
/// Layout (high → low): 16 random bytes, NUL-terminated string data, then
/// the pointer table `[argc][argv..][NULL][envp..][NULL][auxv pairs][AT_RANDOM][AT_NULL]`
/// with RSP 16-byte aligned pointing at argc.
///
/// `auxv` contains the caller-provided (type, value) pairs (not including
/// AT_RANDOM or AT_NULL, which are appended here).
fn setup_exec_stack(
    argv: &[&[u8]],
    envp: &[&[u8]],
    auxv: &[(u64, u64)],
    process: &mut Process,
    hhdm_offset: VirtAddr,
) -> Result<ExecStack, SpawnError> {
    let stack_top = super::layout::USER_STACK_TOP;
    let stack_floor = stack_top - super::layout::USER_STACK_SIZE;
    let mut cursor = stack_top;

    let copy_string = |cursor: &mut u64, s: &[u8]| -> Result<u64, SpawnError> {
        *cursor = cursor
            .checked_sub(s.len() as u64 + 1)
            .filter(|&c| c > stack_floor)
            .ok_or(SpawnError::NoMemory)?;
        copy_to_user(process, hhdm_offset, *cursor, s)?;
        copy_to_user(process, hhdm_offset, *cursor + s.len() as u64, &[0])?;
        Ok(*cursor)
    };

    // Write 16 bytes for AT_RANDOM just below the top of the stack.
    cursor = cursor
        .checked_sub(16)
        .filter(|&c| c >= stack_floor)
        .ok_or(SpawnError::NoMemory)?;
    let mut at_random = [0u8; 16];
    if !crate::entropy::fill(&mut at_random) {
        return Err(SpawnError::EntropyUnavailable);
    }
    copy_to_user(process, hhdm_offset, cursor, &at_random)?;
    let at_random_ptr = cursor;

    // Align cursor down to 8 bytes before the string data.
    cursor &= !0x7;

    let mut argv_addrs = alloc::vec::Vec::with_capacity(argv.len());
    for arg in argv {
        argv_addrs.push(copy_string(&mut cursor, arg)?);
    }
    let mut envp_addrs = alloc::vec::Vec::with_capacity(envp.len());
    for env in envp {
        envp_addrs.push(copy_string(&mut cursor, env)?);
    }

    // Pointer table: argc + argv ptrs + NULL + envp ptrs + NULL
    //                + caller auxv pairs + AT_EXECFN + AT_RANDOM + AT_NULL.
    // AT_EXECFN is emitted only when the caller already supplied a Linux auxv.
    let emit_execfn = !auxv.is_empty();
    let extra_auxv = 2 + usize::from(emit_execfn); // RANDOM, NULL, optional EXECFN
    let auxv_words = (auxv.len() + extra_auxv) * 2;
    let table_words = 1 + argv.len() + 1 + envp.len() + 1 + auxv_words;
    let mut rsp = (cursor & !0x7)
        .checked_sub(table_words as u64 * 8)
        .ok_or(SpawnError::NoMemory)?;
    rsp &= !0xF; // ABI: RSP ≡ 0 (mod 16) at entry, argc at (%rsp)
    if rsp <= stack_floor {
        return Err(SpawnError::NoMemory);
    }

    let mut table = alloc::vec::Vec::with_capacity(table_words * 8);
    table.extend_from_slice(&(argv.len() as u64).to_le_bytes());
    for addr in &argv_addrs {
        table.extend_from_slice(&addr.to_le_bytes());
    }
    table.extend_from_slice(&0u64.to_le_bytes()); // argv NULL
    for addr in &envp_addrs {
        table.extend_from_slice(&addr.to_le_bytes());
    }
    table.extend_from_slice(&0u64.to_le_bytes()); // envp NULL
                                                  // caller-supplied auxv entries
    for &(atype, aval) in auxv {
        table.extend_from_slice(&atype.to_le_bytes());
        table.extend_from_slice(&aval.to_le_bytes());
    }
    if emit_execfn {
        let execfn_ptr = argv_addrs.first().copied().unwrap_or(0);
        table.extend_from_slice(&sunlight_compat_linux::abi::AT_EXECFN.to_le_bytes());
        table.extend_from_slice(&execfn_ptr.to_le_bytes());
    }
    // AT_RANDOM (25)
    table.extend_from_slice(&sunlight_compat_linux::abi::AT_RANDOM.to_le_bytes());
    table.extend_from_slice(&at_random_ptr.to_le_bytes());
    // AT_NULL (0) — musl stops scanning auxv here
    table.extend_from_slice(&sunlight_compat_linux::abi::AT_NULL.to_le_bytes());
    table.extend_from_slice(&0u64.to_le_bytes());
    copy_to_user(process, hhdm_offset, rsp, &table)?;

    crate::serial_println!(
        "[EXEC] Stack: argc={} envc={} auxv={} rsp={:#x}",
        argv.len(),
        envp.len(),
        auxv.len() + extra_auxv,
        rsp
    );

    Ok(ExecStack {
        rsp,
        argv_ptr: rsp + 8,
        envp_ptr: rsp + 8 + (argv.len() as u64 + 1) * 8,
    })
}

/// Find the virtual address of the ELF program headers in the loaded image.
///
/// Scans PT_LOAD segments for the one that covers `phoff` in the file.
/// Returns `p_vaddr + (phoff - p_offset)` for that segment, or 0 on failure.
/// Used to populate AT_PHDR in the auxv so musl can locate PT_TLS.
fn compute_at_phdr(elf_bytes: &[u8], header: &sunlight_elf::ElfHeader) -> u64 {
    let phoff = header.phoff;
    let phentsize = header.phentsize as usize;
    for i in 0..header.phnum as usize {
        let ph_start = header.phoff as usize + i * phentsize;
        let ph_end = ph_start + phentsize;
        if ph_end > elf_bytes.len() || phentsize < 48 {
            break;
        }
        let p_type = u32::from_le_bytes(elf_bytes[ph_start..ph_start + 4].try_into().unwrap());
        if p_type != 1 {
            continue; // PT_LOAD = 1
        }
        let p_offset =
            u64::from_le_bytes(elf_bytes[ph_start + 8..ph_start + 16].try_into().unwrap());
        let p_vaddr =
            u64::from_le_bytes(elf_bytes[ph_start + 16..ph_start + 24].try_into().unwrap());
        let p_filesz =
            u64::from_le_bytes(elf_bytes[ph_start + 32..ph_start + 40].try_into().unwrap());
        if p_offset <= phoff && phoff < p_offset + p_filesz {
            return p_vaddr + (phoff - p_offset);
        }
    }
    0
}

/// Spawn a new process from a static ELF binary on the filesystem.
/// For the kernel, we embed the sunshell binary and look it up by path.
/// The process receives the default environment for `uid`.
pub fn spawn_from_path(
    path: &str,
    argv: &[&str],
    pmm: &mut PhysicalMemoryManager,
    sched: &mut Scheduler,
    caps: &mut CapabilityBroker,
    hhdm_offset: VirtAddr,
    uid: u32,
    gid: u32,
) -> Result<usize, SpawnError> {
    spawn_from_path_with_restrictions(
        path,
        argv,
        pmm,
        sched,
        caps,
        hhdm_offset,
        uid,
        gid,
        None,
        None,
    )
}

/// Spawn with an explicit base environment (e.g. inherited from a parent via
/// `EnvMap::inherit`). `None` falls back to `EnvMap::with_defaults(uid)`.
pub fn spawn_from_path_with_env(
    path: &str,
    _argv: &[&str],
    pmm: &mut PhysicalMemoryManager,
    sched: &mut Scheduler,
    _caps: &mut CapabilityBroker,
    hhdm_offset: VirtAddr,
    uid: u32,
    gid: u32,
    env: Option<super::env::EnvMap>,
) -> Result<usize, SpawnError> {
    spawn_from_path_with_restrictions(
        path,
        _argv,
        pmm,
        sched,
        _caps,
        hhdm_offset,
        uid,
        gid,
        env,
        None,
    )
}

pub fn spawn_from_path_with_restrictions(
    path: &str,
    _argv: &[&str],
    pmm: &mut PhysicalMemoryManager,
    sched: &mut Scheduler,
    _caps: &mut CapabilityBroker,
    hhdm_offset: VirtAddr,
    uid: u32,
    gid: u32,
    env: Option<super::env::EnvMap>,
    service_lookup_restrictions: Option<u64>,
) -> Result<usize, SpawnError> {
    let bytes = embedded_bytes_for_path(path)?;
    // Shells map a tty tab via their shell_id; non-shell paths (e.g. user-level
    // daemons such as timezone_service/niced/gcd spawned by sunlightd) have no
    // shell_id and run detached from any TTY tab.
    let shell_id = shell_id_from_path(path);

    crate::serial_println!("[SPAWN] Loading {} ({} bytes)", path, bytes.len());

    // Allocate a collision-free pid the same way the spawn syscall does:
    // highest existing pid + 1. Using processes.len()+1 here is unsafe because
    // boot services hardcode non-contiguous pids (e.g. niced=10, gcd=11), so
    // len()+1 can alias an existing pid — duplicate pids then break every
    // pid-keyed lookup (wake_pid/waitpid/process_is_alive find the wrong
    // process, leaving the real one blocked forever).
    let pid = sched
        .processes
        .iter()
        .filter_map(|p| p.pid.checked_add(1))
        .max()
        .unwrap_or(1);
    // net_server is no longer spawned at a fixed pid (init launches it after
    // timer_server), so the kernel frame-proxy syscalls (net_tx/net_rx) can no
    // longer gate on pid==5. Give it a stable name here and gate on that.
    // For all other binaries derive the name from the path basename so that
    // monitoring tools (top) can display meaningful names instead of "daemon".
    let proc_name = if shell_id.is_some() {
        "sshl"
    } else {
        name_from_path(path)
    };
    let mut process = unsafe {
        Process::try_new(pid, 1, proc_name, pmm, hhdm_offset).map_err(|_| SpawnError::NoMemory)?
    };
    process.uid = uid;
    process.gid = gid;
    // Attach this shell to a TTY tab keyed by its shell_id (parsed from the
    // path above). Children spawned by the shell inherit this, so their fd0/fd1
    // route to the tab's kernel stdin/stdout rings (foreground input routing).
    // tty_server uses the same shell_id as the ring key. Daemons stay detached.
    process.tty_tab = shell_id.map(|id| id as u8);
    // Phase 6.5 Step 2: every spawned process gets an environment — either
    // one inherited from the caller or the defaults for this uid (PATH,
    // USER, HOME, SHELL). Username resolution from /etc/passwd happens in
    // userspace via VFS; the kernel only knows the uid here.
    process.env = env.unwrap_or_else(|| super::env::EnvMap::with_defaults(uid, ""));
    process.service_lookup_restrictions = service_lookup_restrictions;

    let envp_strings = process.env.to_envp();
    let envp: alloc::vec::Vec<&[u8]> = envp_strings.iter().map(|s| s.as_bytes()).collect();
    exec_into_process(bytes, &mut process, pmm, hhdm_offset, &[], &envp, false)?;
    process.trusted_display_service = is_trusted_display_path(path);
    process.trusted_swap_admin_service = is_trusted_swap_admin_path(path);
    process.trusted_zram_diagnostic = is_trusted_zram_diagnostic_path(path);
    process.trusted_pty_service = is_trusted_pty_service_path(path);
    process.trusted_lock_service = is_trusted_lock_service_path(path);
    process.trusted_session_service = is_trusted_session_service_path(path);
    process.trusted_wiseowl_braind = is_trusted_wiseowl_braind_path(path);
    process.trusted_wiseowl_console = is_trusted_wiseowl_console_path(path);
    process.trusted_control_panel = is_trusted_control_panel_path(path);
    process.set_initial_args(
        if matches!(
            path,
            "/bin/vortex-lock-presenter" | "/usr/bin/vortex-lock-presenter"
        ) {
            LOCK_PRESENTER_ENTRY_MAGIC
        } else {
            shell_id.unwrap_or(0)
        },
        uid as u64,
        gid as u64,
        0,
    );

    let actual_pid = process.pid;
    let idx = sched.add_process_after_reaping(process);
    sched.enqueue_ready(idx);

    crate::serial_println!("[SPAWN] {} spawned pid={}", path, actual_pid);
    Ok(actual_pid)
}

/// Get embedded ELF bytes for a given path.
pub fn embedded_bytes_for_path(path: &str) -> Result<&'static [u8], SpawnError> {
    match path {
        "/bin/sh" | "/bin/ssh" | "/bin/sshl" => Ok(crate::SUNSHELL_ELF_BYTES),
        p if p.starts_with("/bin/sshl") => Ok(crate::SUNSHELL_ELF_BYTES),
        // POSIX-style command paths: the remaining standard applets execute
        // from /bin or /usr/bin and dispatch by argv[0] inside the multi-call
        // binary. Native-libc cat and pwd are mapped just below.
        "/bin/ls" | "/bin/cp" | "/bin/mv" | "/bin/rm" | "/bin/mkdir" | "/bin/rmdir"
        | "/bin/touch" | "/bin/tail" | "/bin/file" | "/bin/stat" | "/bin/date" | "/bin/whoami"
        | "/bin/id" | "/bin/uname" | "/bin/nice" | "/bin/renice" | "/bin/free"
        | "/bin/freezram" | "/bin/kill" | "/bin/killall" | "/bin/pkill" | "/usr/bin/ls"
        | "/usr/bin/cp" | "/usr/bin/mv" | "/usr/bin/rm" | "/usr/bin/mkdir" | "/usr/bin/rmdir"
        | "/usr/bin/touch" | "/usr/bin/tail" | "/usr/bin/file" | "/usr/bin/stat"
        | "/usr/bin/date" | "/usr/bin/whoami" | "/usr/bin/id" | "/usr/bin/uname"
        | "/usr/bin/nice" | "/usr/bin/renice" | "/usr/bin/free" | "/usr/bin/freezram"
        | "/usr/bin/kill" | "/usr/bin/killall" | "/usr/bin/pkill" => {
            Ok(crate::SUNLIGHT_UTILS_ELF_BYTES)
        }
        "/bin/cat" | "/usr/bin/cat" | "/sunlight-utils/cat" => Ok(crate::SUNLIGHT_CAT_ELF_BYTES),
        "/bin/pwd" | "/usr/bin/pwd" | "/sunlight-utils/pwd" => Ok(crate::SUNLIGHT_PWD_ELF_BYTES),
        "/bin/echo" | "/usr/bin/echo" => Ok(crate::SUNLIGHT_ECHO_ELF_BYTES),
        "/bin/true" | "/usr/bin/true" | "/sunlight-utils/true" => {
            Ok(crate::SUNLIGHT_TRUE_ELF_BYTES)
        }
        "/bin/false" | "/usr/bin/false" | "/sunlight-utils/false" => {
            Ok(crate::SUNLIGHT_FALSE_ELF_BYTES)
        }
        "/bin/basename" | "/usr/bin/basename" | "/sunlight-utils/basename" => {
            Ok(crate::SUNLIGHT_BASENAME_ELF_BYTES)
        }
        "/bin/dirname" | "/usr/bin/dirname" | "/sunlight-utils/dirname" => {
            Ok(crate::SUNLIGHT_DIRNAME_ELF_BYTES)
        }
        "/bin/head" | "/usr/bin/head" | "/sunlight-utils/head" => {
            Ok(crate::SUNLIGHT_HEAD_ELF_BYTES)
        }
        "/bin/cmp" | "/usr/bin/cmp" | "/sunlight-utils/cmp" => Ok(crate::SUNLIGHT_CMP_ELF_BYTES),
        "/bin/wc" | "/usr/bin/wc" | "/sunlight-utils/wc" => Ok(crate::SUNLIGHT_WC_ELF_BYTES),
        "/bin/cut" | "/usr/bin/cut" | "/sunlight-utils/cut" => Ok(crate::SUNLIGHT_CUT_ELF_BYTES),
        "/bin/fold" | "/usr/bin/fold" | "/sunlight-utils/fold" => {
            Ok(crate::SUNLIGHT_FOLD_ELF_BYTES)
        }
        "/bin/expand" | "/usr/bin/expand" | "/sunlight-utils/expand" => {
            Ok(crate::SUNLIGHT_EXPAND_ELF_BYTES)
        }
        "/bin/cksum" | "/usr/bin/cksum" | "/sunlight-utils/cksum" => {
            Ok(crate::SUNLIGHT_CKSUM_ELF_BYTES)
        }
        "/bin/grep" | "/usr/bin/grep" | "/sunlight-utils/grep" => {
            Ok(crate::SUNLIGHT_GREP_ELF_BYTES)
        }
        "/bin/sort" | "/usr/bin/sort" | "/sunlight-utils/sort" => {
            Ok(crate::SUNLIGHT_SORT_ELF_BYTES)
        }
        "/bin/uniq" | "/usr/bin/uniq" | "/sunlight-utils/uniq" => {
            Ok(crate::SUNLIGHT_UNIQ_ELF_BYTES)
        }
        "/bin/comm" | "/usr/bin/comm" | "/sunlight-utils/comm" => {
            Ok(crate::SUNLIGHT_COMM_ELF_BYTES)
        }
        "/bin/tr" | "/usr/bin/tr" | "/sunlight-utils/tr" => Ok(crate::SUNLIGHT_TR_ELF_BYTES),
        "/bin/paste" | "/usr/bin/paste" | "/sunlight-utils/paste" => {
            Ok(crate::SUNLIGHT_PASTE_ELF_BYTES)
        }
        "/bin/join" | "/usr/bin/join" | "/sunlight-utils/join" => {
            Ok(crate::SUNLIGHT_JOIN_ELF_BYTES)
        }
        "/bin/printf" | "/usr/bin/printf" | "/sunlight-utils/printf" => {
            Ok(crate::SUNLIGHT_PRINTF_ELF_BYTES)
        }
        "/bin/tee" | "/usr/bin/tee" | "/sunlight-utils/tee" => Ok(crate::SUNLIGHT_TEE_ELF_BYTES),
        "/bin/nl" | "/usr/bin/nl" | "/sunlight-utils/nl" => Ok(crate::SUNLIGHT_NL_ELF_BYTES),
        "/bin/od" | "/usr/bin/od" | "/sunlight-utils/od" => Ok(crate::SUNLIGHT_OD_ELF_BYTES),
        "/bin/split" | "/usr/bin/split" | "/sunlight-utils/split" => {
            Ok(crate::SUNLIGHT_SPLIT_ELF_BYTES)
        }
        "/bin/find" | "/usr/bin/find" | "/sunlight-utils/find" => {
            Ok(crate::SUNLIGHT_FIND_ELF_BYTES)
        }
        "/bin/xargs" | "/usr/bin/xargs" | "/sunlight-utils/xargs" => {
            Ok(crate::SUNLIGHT_XARGS_ELF_BYTES)
        }
        "/bin/ping"
        | "/bin/ifconfig"
        | "/bin/wget"
        | "/bin/curl"
        | "/bin/dig"
        | "/bin/nslookup"
        | "/bin/hostname"
        | "/bin/netstat"
        | "/bin/ss"
        | "/bin/traceroute"
        | "/bin/arp"
        | "/bin/dhclient"
        | "/usr/bin/ping"
        | "/usr/bin/ifconfig"
        | "/usr/bin/wget"
        | "/usr/bin/curl"
        | "/usr/bin/dig"
        | "/usr/bin/nslookup"
        | "/usr/bin/hostname"
        | "/usr/bin/netstat"
        | "/usr/bin/ss"
        | "/usr/bin/traceroute"
        | "/usr/bin/arp"
        | "/usr/bin/dhclient" => Ok(crate::SUNLIGHT_NET_UTILS_ELF_BYTES),
        // Base servers spawned by init (pid=1), not hardcoded in kernel boot.
        // These need no privileged memory setup (unlike vfs/tty).
        "/sbin/timer_server" | "/usr/sbin/timer_server" => Ok(crate::TIMER_SERVER_ELF_BYTES),
        "/sbin/sunlight-swapd" | "/usr/sbin/sunlight-swapd" => Ok(crate::SUNLIGHT_SWAPD_ELF_BYTES),
        "/sbin/sunlight-kbd" | "/usr/sbin/sunlight-kbd" => Ok(crate::SUNLIGHT_KBD_ELF_BYTES),
        "/sbin/sunlight-mouse" | "/usr/sbin/sunlight-mouse" => Ok(crate::SUNLIGHT_MOUSE_ELF_BYTES),
        "/sbin/sunlight-usb-mouse" | "/usr/sbin/sunlight-usb-mouse" => {
            Ok(crate::SUNLIGHT_USB_MOUSE_ELF_BYTES)
        }
        "/sbin/deviced" | "/usr/sbin/deviced" => Ok(crate::DEVICED_ELF_BYTES),
        "/sbin/networkd" | "/usr/sbin/networkd" => Ok(crate::NETWORKD_ELF_BYTES),
        "/bin/networkctl" | "/usr/bin/networkctl" => Ok(crate::NETWORKCTL_ELF_BYTES),
        "/sbin/resolved" | "/usr/sbin/resolved" => Ok(crate::RESOLVED_ELF_BYTES),
        "/bin/resolvectl" | "/usr/bin/resolvectl" => Ok(crate::RESOLVECTL_ELF_BYTES),
        "/sbin/powerd" | "/usr/sbin/powerd" => Ok(crate::POWERD_ELF_BYTES),
        "/sbin/thermald" | "/usr/sbin/thermald" => Ok(crate::THERMALD_ELF_BYTES),
        "/sbin/audiod" | "/usr/sbin/audiod" => Ok(crate::AUDIOD_ELF_BYTES),
        "/bin/audioctl" | "/usr/bin/audioctl" => Ok(crate::AUDIOCTL_ELF_BYTES),
        "/sbin/pty_server" | "/usr/sbin/pty_server" => Ok(crate::PTY_SERVER_ELF_BYTES),
        "/sbin/net_server" | "/usr/sbin/net_server" => Ok(crate::NET_SERVER_ELF_BYTES),
        "/sbin/sunlightd" | "/usr/sbin/sunlightd" => Ok(crate::SUNLIGHTD_ELF_BYTES),
        // User-level daemons spawned by sunlightd (not hardcoded in kernel boot).
        "/sbin/timezone_service" | "/usr/sbin/timezone_service" => {
            Ok(crate::TIMEZONE_SERVICE_ELF_BYTES)
        }
        "/sbin/timed" | "/usr/sbin/timed" => Ok(crate::TIMED_ELF_BYTES),
        "/bin/tzutils" | "/usr/bin/tzutils" => Ok(crate::TZUTILS_ELF_BYTES),
        "/sbin/rand_service" | "/usr/sbin/rand_service" => Ok(crate::RAND_SERVICE_ELF_BYTES),
        "/sbin/niced" | "/usr/sbin/niced" => Ok(crate::SUNLIGHT_NICED_ELF_BYTES),
        "/bin/nicectl" | "/usr/bin/nicectl" => Ok(crate::NICECTL_ELF_BYTES),
        "/sbin/gcd" | "/usr/sbin/gcd" => Ok(crate::SUNLIGHT_GCD_ELF_BYTES),
        // Key-value storage daemon (spawned by sunlightd).
        "/sbin/sunlight-kv" | "/usr/sbin/sunlight-kv" => Ok(crate::SUNLIGHT_KV_ELF_BYTES),
        // Key-value control CLI.
        "/bin/sunlight-kvctl" | "/usr/bin/sunlight-kvctl" => Ok(crate::SUNLIGHT_KVCTL_ELF_BYTES),
        // TLS service (sunlightd-launched) + certificate control CLI.
        "/sbin/sunlight-tls" | "/usr/sbin/sunlight-tls" => Ok(crate::SUNLIGHT_TLS_ELF_BYTES),
        "/sbin/secret_store_test" | "/usr/sbin/secret_store_test" => {
            Ok(crate::SECRET_STORE_TEST_ELF_BYTES)
        }
        "/bin/certificatectl" | "/usr/bin/certificatectl" => Ok(crate::CERTIFICATECTL_ELF_BYTES),
        // User Access Control daemon (spawned by sunlightd) + control client.
        "/sbin/uac_service" | "/usr/sbin/uac_service" => Ok(crate::UAC_SERVICE_ELF_BYTES),
        "/sbin/sunlight-sessiond" | "/usr/sbin/sunlight-sessiond" => {
            Ok(crate::SUNLIGHT_SESSIOND_ELF_BYTES)
        }
        "/sbin/mezzo" | "/usr/sbin/mezzo" => Ok(crate::MEZZO_ELF_BYTES),
        "/bin/capabilityctl" | "/usr/bin/capabilityctl" => Ok(crate::CAPABILITYCTL_ELF_BYTES),
        "/bin/runas" | "/usr/bin/runas" => Ok(crate::RUNAS_ELF_BYTES),
        "/usr/bin/top" | "/bin/top" => Ok(crate::SUNLIGHT_TOP_ELF_BYTES),
        "/usr/bin/memoryctl" | "/bin/memoryctl" => Ok(crate::MEMORYCTL_ELF_BYTES),
        "/usr/bin/sunlightctl" | "/bin/sunlightctl" => Ok(crate::SUNLIGHTCTL_ELF_BYTES),
        "/usr/bin/mezzoctl" | "/bin/mezzoctl" => Ok(crate::MEZZOCTL_ELF_BYTES),
        "/usr/bin/sunlight-sessionctl" | "/bin/sunlight-sessionctl" => {
            Ok(crate::SUNLIGHT_SESSIONCTL_ELF_BYTES)
        }
        // Session Configuration Phase 1 fixtures (SpawnRequest path must be ≤31 chars).
        "/bin/su1" | "/usr/bin/su1" => Ok(crate::SU1_ELF_BYTES),
        "/bin/su2" | "/usr/bin/su2" => Ok(crate::SU2_ELF_BYTES),
        "/usr/bin/devicectl" | "/bin/devicectl" => Ok(crate::DEVICECTL_ELF_BYTES),
        "/usr/bin/sunlight-hwinfo" | "/bin/sunlight-hwinfo" => Ok(crate::SUNLIGHT_HWINFO_ELF_BYTES),
        "/usr/bin/powerctl" | "/bin/powerctl" => Ok(crate::POWERCTL_ELF_BYTES),
        "/usr/bin/thermalctl" | "/bin/thermalctl" => Ok(crate::THERMALCTL_ELF_BYTES),
        "/bin/fetch" | "/usr/bin/fetch" => Ok(crate::SUNLIGHT_FETCH_ELF_BYTES),
        // Storage Manager (whitelisted protected FS writes for services such as sunlight-kv).
        "/sbin/sunlight-sm" | "/usr/sbin/sunlight-sm" => Ok(crate::SUNLIGHT_SM_ELF_BYTES),
        // Solar HTTP server with SBSP scripting engine.
        "/sbin/solar" | "/usr/sbin/solar" => Ok(crate::SOLAR_ELF_BYTES),
        // GUI Phase 3+ : Display server (compositor) + eyes tracker demo client.
        "/sbin/sunlight-display" | "/usr/sbin/sunlight-display" => {
            Ok(crate::SUNLIGHT_DISPLAY_ELF_BYTES)
        }
        "/bin/eyes" | "/usr/bin/eyes" => Ok(crate::EYES_ELF_BYTES),
        "/bin/sunlight-runner" | "/usr/bin/sunlight-runner" => Ok(crate::SUNLIGHT_RUNNER_ELF_BYTES),
        "/bin/sun-exec" | "/usr/bin/sun-exec" => Ok(crate::SUN_EXEC_ELF_BYTES),
        "/bin/sun-open" | "/usr/bin/sun-open" => Ok(crate::SUN_OPEN_ELF_BYTES),
        "/bin/sunlight-terminal" | "/usr/bin/sunlight-terminal" => {
            Ok(crate::SUNLIGHT_TERMINAL_ELF_BYTES)
        }
        // Chronos: native DOS `.COM` compatibility window.
        "/bin/sunlight-chronos" | "/usr/bin/sunlight-chronos" => {
            Ok(crate::SUNLIGHT_CHRONOS_ELF_BYTES)
        }
        "/bin/sunlight-tasks" | "/usr/bin/sunlight-tasks" => Ok(crate::SUNLIGHT_TASKS_ELF_BYTES),
        // sunlight-sunsay: native Rust proof-of-life binary (Phase 1 std smoke test).
        "/bin/sunlight-sunsay" | "/usr/bin/sunlight-sunsay" | "/usr/local/bin/sunlight-sunsay" => {
            Ok(crate::SUNLIGHT_SUNSAY_ELF_BYTES)
        }
        // sunlight-zoxide: directory jump utility (Phase 2 std validation).
        "/bin/z" | "/usr/bin/z" | "/usr/local/bin/z" => Ok(crate::SUNLIGHT_ZOXIDE_ELF_BYTES),
        // sunlight-dict: offline dictionary lookup (Phase 3 std validation).
        "/bin/dict" | "/usr/bin/dict" | "/usr/local/bin/dict" => Ok(crate::SUNLIGHT_DICT_ELF_BYTES),
        // sunlight-hangman: interactive libc smoke test.
        "/bin/hangman" | "/usr/bin/hangman" | "/usr/local/bin/hangman" => {
            Ok(crate::SUNLIGHT_HANGMAN_ELF_BYTES)
        }
        // sunbench: SunLight-Bench CPU/multi-core performance benchmarking suite.
        "/bin/sunbench" | "/usr/bin/sunbench" => Ok(crate::SUNLIGHT_BENCH_ELF_BYTES),
        // calculator: lightweight graphical calculator.
        "/bin/calculator" | "/usr/bin/calculator" => Ok(crate::SUNLIGHT_CALCULATOR_ELF_BYTES),
        // welcome: SunlightOS Welcome Wizard (startup-eligible .sunapp helper).
        "/bin/welcome" | "/usr/bin/welcome" => Ok(crate::SUNLIGHT_WELCOME_ELF_BYTES),
        // widget-gallery: developer-only reusable widget preview.
        "/bin/widget-gallery" | "/usr/bin/widget-gallery" => {
            Ok(crate::SUNLIGHT_WIDGET_GALLERY_ELF_BYTES)
        }
        // silicon-echoes: native graphical narrative-game vertical slice.
        "/bin/silicon-echoes" | "/usr/bin/silicon-echoes" => Ok(crate::SILICON_ECHOES_ELF_BYTES),
        // sunlight-files: native graphical file manager.
        "/bin/sunlight-files" | "/usr/bin/sunlight-files" => Ok(crate::SUNLIGHT_FILES_ELF_BYTES),
        // light-lens: native graphical image viewer.
        "/bin/light-lens" | "/usr/bin/light-lens" => Ok(crate::LIGHT_LENS_ELF_BYTES),
        // melody-mina: UI-only native music-player frontend.
        "/bin/melody-mina" | "/usr/bin/melody-mina" => Ok(crate::MELODY_MINA_ELF_BYTES),
        // sunlight-edit: native graphical text editor.
        "/bin/sunlight-edit"
        | "/usr/bin/sunlight-edit"
        | "/bin/sunlight-text"
        | "/usr/bin/sunlight-text" => Ok(crate::SUNLIGHT_EDIT_ELF_BYTES),
        // sunlight-writer: professional document shell.
        "/bin/sunlight-writer" | "/usr/bin/sunlight-writer" => Ok(crate::SUNLIGHT_WRITER_ELF_BYTES),
        // sunlight-calendar: graphical calendar application.
        "/bin/sunlight-calendar" | "/usr/bin/sunlight-calendar" => {
            Ok(crate::SUNLIGHT_CALENDAR_ELF_BYTES)
        }
        // sunlight-reminders: personal tasks and reminders application.
        "/bin/sunlight-reminders" | "/usr/bin/sunlight-reminders" => {
            Ok(crate::SUNLIGHT_REMINDERS_ELF_BYTES)
        }
        // sunlight-devices: read-only graphical hardware inventory viewer.
        "/bin/sunlight-devices" | "/usr/bin/sunlight-devices" => {
            Ok(crate::SUNLIGHT_DEVICES_ELF_BYTES)
        }
        // rappid-rabbit: native HTTP inspection application.
        "/bin/rappid-rabbit" | "/usr/bin/rappid-rabbit" => Ok(crate::RAPPID_RABBIT_ELF_BYTES),
        // sunlight-api-lab: native REST/API testing application.
        "/bin/sunlight-api-lab" | "/usr/bin/sunlight-api-lab" => {
            Ok(crate::SUNLIGHT_API_LAB_ELF_BYTES)
        }
        "/sbin/sunlight-dialogd" | "/usr/sbin/sunlight-dialogd" => {
            Ok(crate::SUNLIGHT_DIALOGD_ELF_BYTES)
        }
        "/bin/sunlight-vortex-shell"
        | "/usr/bin/sunlight-vortex-shell"
        | "/bin/vortex-lock-presenter"
        | "/usr/bin/vortex-lock-presenter" => Ok(crate::SUNLIGHT_VORTEX_SHELL_ELF_BYTES),
        // control-panel: System Preferences (Mouse + Monitor settings).
        "/bin/control-panel" | "/usr/bin/control-panel" => {
            Ok(crate::SUNLIGHT_CONTROL_PANEL_ELF_BYTES)
        }
        // sunlight-thumbd: thumbnail daemon for async File Manager previews.
        "/bin/sunlight-thumbd" | "/usr/bin/sunlight-thumbd" | "/sbin/sunlight-thumbd" => {
            Ok(crate::SUNLIGHT_THUMBD_ELF_BYTES)
        }
        "/sbin/sunlight-clipd" | "/usr/sbin/sunlight-clipd" => Ok(crate::SUNLIGHT_CLIPD_ELF_BYTES),
        "/bin/sunlight-clip" | "/usr/bin/sunlight-clip" => Ok(crate::SUNLIGHT_CLIP_ELF_BYTES),
        "/sbin/wiseowl-memoryd" | "/usr/sbin/wiseowl-memoryd" => {
            Ok(crate::WISEOWL_MEMORYD_ELF_BYTES)
        }
        "/bin/wiseowl-memoryctl" | "/usr/bin/wiseowl-memoryctl" => {
            Ok(crate::WISEOWL_MEMORYCTL_ELF_BYTES)
        }
        "/sbin/wiseowl-memorydb" | "/usr/sbin/wiseowl-memorydb" => {
            Ok(crate::WISEOWL_MEMORYDB_ELF_BYTES)
        }
        "/bin/wiseowl-memorydbctl" | "/usr/bin/wiseowl-memorydbctl" => {
            Ok(crate::WISEOWL_MEMORYDBCTL_ELF_BYTES)
        }
        "/sbin/wiseowl-indexd" | "/usr/sbin/wiseowl-indexd" => Ok(crate::WISEOWL_INDEXD_ELF_BYTES),
        "/bin/wiseowl-indexctl" | "/usr/bin/wiseowl-indexctl" => {
            Ok(crate::WISEOWL_INDEXCTL_ELF_BYTES)
        }
        "/sbin/wiseowl-braind" | "/usr/sbin/wiseowl-braind" => Ok(crate::WISEOWL_BRAIND_ELF_BYTES),
        "/bin/wiseowl-brainctl" | "/usr/bin/wiseowl-brainctl" => {
            Ok(crate::WISEOWL_BRAINCTL_ELF_BYTES)
        }
        "/bin/wiseowl" | "/usr/bin/wiseowl" => Ok(crate::WISEOWL_CONSOLE_ELF_BYTES),
        "/bin/sunlight-clipman" | "/usr/bin/sunlight-clipman" => {
            Ok(crate::SUNLIGHT_CLIPMAN_ELF_BYTES)
        }
        "/bin/emoji-picker" | "/usr/bin/emoji-picker" => Ok(crate::EMOJI_PICKER_ELF_BYTES),
        // cpufeat: x86-64 microarchitecture level detection (v2/v3 capability reporting).
        "/bin/cpufeat" | "/usr/bin/cpufeat" => Ok(crate::CPUFEAT_ELF_BYTES),
        // hello-linux: musl Rust binary for Helios Linux-compat smoke test.
        "/bin/hello-linux" | "/usr/bin/hello-linux" => Ok(crate::HELLO_LINUX_ELF_BYTES),
        "/bin/helios-probe" | "/usr/bin/helios-probe" => Ok(crate::HELIOS_PROBE_ELF_BYTES),
        "/bin/helios-probe-runtime"
        | "/usr/bin/helios-probe-runtime"
        | "/bin/linux-uname"
        | "/usr/bin/linux-uname"
        | "/bin/linux-ids"
        | "/usr/bin/linux-ids"
        | "/bin/linux-gettimeofday"
        | "/usr/bin/linux-gettimeofday"
        | "/bin/linux-mkdir"
        | "/usr/bin/linux-mkdir"
        | "/bin/linux-getdents64"
        | "/usr/bin/linux-getdents64"
        | "/bin/linux-pread-pwrite"
        | "/usr/bin/linux-pread-pwrite"
        | "/bin/linux-access"
        | "/usr/bin/linux-access"
        | "/bin/linux-open-flags"
        | "/usr/bin/linux-open-flags"
        | "/bin/linux-dup3"
        | "/usr/bin/linux-dup3"
        | "/bin/linux-stat-metadata"
        | "/usr/bin/linux-stat-metadata" => Ok(crate::HELIOS_PROBE_RUNTIME_ELF_BYTES),
        "/bin/linux-echo" | "/usr/bin/linux-echo" => Ok(crate::SBASE_ECHO_ELF_BYTES),
        // helios-note: std+libc terminal note editor (Helios Linux compat).
        "/bin/note" | "/usr/bin/note" => Ok(crate::HELIOS_NOTE_ELF_BYTES),
        // Phase 6.5 Step 3: PATH entries under these directories are applets
        // of the embedded multi-call binaries (argv[0] picks the applet).
        "/sunlight-utils/echo" => Ok(crate::SUNLIGHT_ECHO_ELF_BYTES),
        p if p.starts_with("/sunlight-utils/") => Ok(crate::SUNLIGHT_UTILS_ELF_BYTES),
        p if p.starts_with("/sunlight-net-utils/") => Ok(crate::SUNLIGHT_NET_UTILS_ELF_BYTES),
        _ => Err(SpawnError::NotFound),
    }
}

pub fn shell_id_from_path(path: &str) -> Option<u64> {
    match path {
        "/bin/sh" | "/bin/ssh" | "/bin/sshl" => Some(0),
        p if p.starts_with("/bin/sshl") => {
            let encoded = parse_leading_u64(&p[9..])?;
            Some(encoded & 0xff)
        }
        _ => None,
    }
}

pub fn shell_credentials_from_path(path: &str) -> Option<(u32, u32)> {
    let encoded = match path {
        "/bin/sh" | "/bin/ssh" | "/bin/sshl" => return Some((0, 0)),
        p if p.starts_with("/bin/sshl") => parse_leading_u64(&p[9..])?,
        _ => return None,
    };
    let uid = ((encoded >> 8) & 0x0fff_ffff) as u32;
    let gid = ((encoded >> 36) & 0x0fff_ffff) as u32;
    Some((uid, gid))
}

fn parse_u64(s: &str) -> Option<u64> {
    if s.is_empty() {
        return None;
    }
    let mut result = 0u64;
    for &b in s.as_bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        result = result.checked_mul(10)?.checked_add((b - b'0') as u64)?;
    }
    Some(result)
}

fn parse_leading_u64(s: &str) -> Option<u64> {
    let (value, _) = split_leading_u64(s)?;
    Some(value)
}

fn split_leading_u64(s: &str) -> Option<(u64, &str)> {
    if s.is_empty() {
        return None;
    }
    let end = s.bytes().take_while(|b| b.is_ascii_digit()).count();
    if end == 0 {
        return None;
    }
    Some((parse_u64(&s[..end])?, &s[end..]))
}
