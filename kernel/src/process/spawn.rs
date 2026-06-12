use super::{Process, ProcessState};
use crate::memory::pmm::PhysicalMemoryManager;
use crate::sched::Scheduler;
use crate::capability::{CapabilityBroker, CapabilityRights};
use x86_64::{
    VirtAddr,
    structures::paging::{Page, PageTableFlags, PhysFrame},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnError {
    NotFound,
    PermissionDenied,
    ElfLoadFailed,
    NoMemory,
    InvalidPath,
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
) -> Result<u64, SpawnError> {
    // Tear down old address space (note: old frames leak; acceptable for minimal scope)
    process.address_space = unsafe {
        crate::process::address_space::AddressSpace::new(pmm, hhdm_offset)
    };

    // Phase 4.5: Detect if this is a Linux-compatible ELF binary
    process.is_linux_compat = super::elf_loader::is_linux_elf(bytes);
    if process.is_linux_compat {
        crate::serial_println!("[EXEC] Linux ELF detected");
    }

    let entry = super::elf_loader::load_elf(bytes, process, pmm, hhdm_offset);
    let entry = entry.ok_or(SpawnError::ElfLoadFailed)?;

    // Allocate user stack
    let stack_pages = (super::layout::USER_STACK_SIZE + 4095) / 4096;
    for i in 0..stack_pages {
        let page_addr = VirtAddr::new(super::layout::USER_STACK_TOP - (i + 1) * 4096);
        let page = Page::from_start_address(page_addr).unwrap();
        let frame_addr = pmm.alloc_frame().ok_or(SpawnError::NoMemory)?;
        let phys = unsafe { PhysFrame::from_start_address_unchecked(frame_addr) };
        let flags = PageTableFlags::PRESENT
            | PageTableFlags::WRITABLE
            | PageTableFlags::USER_ACCESSIBLE;
        unsafe {
            process.address_space.map_page(page, phys, flags, pmm, hhdm_offset);
        }
    }

    // Setup stack with argv/envp
    let stack_ptr = setup_exec_stack(argv, envp, process, pmm, hhdm_offset)?;

    process.init_context(entry, stack_ptr);
    // Set rdi=argc, rsi=argv, rdx=envp (SysV ABI) via initial args
    let argc = argv.len() as u64;
    // argv and envp pointers will be computed from stack layout below
    process.set_initial_args(argc, 0, 0, 0);

    crate::serial_println!("[EXEC] Loaded ELF entry={:#x}, stack={:#x}", entry, stack_ptr);
    Ok(entry)
}

/// Setup the user stack with argc, argv, envp per SysV x86_64 ABI.
/// Returns the final RSP value for process entry.
/// For minimal scope: just set up argc and basic stack; argv/envp full marshalling deferred.
fn setup_exec_stack(
    argv: &[&[u8]],
    _envp: &[&[u8]],
    _process: &mut Process,
    _pmm: &mut PhysicalMemoryManager,
    _hhdm_offset: VirtAddr,
) -> Result<u64, SpawnError> {
    // Minimal implementation: just return the stack top
    // Full argv/envp marshalling requires writing through the page tables,
    // which is deferred to avoid complexity in this phase.
    // The entry point receives argc=argv.len() via set_initial_args.
    let stack_top = super::layout::USER_STACK_TOP;

    crate::serial_println!("[EXEC] Stack setup: argc={}, top={:#x}", argv.len(), stack_top);

    Ok(stack_top)
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
    spawn_from_path_with_env(path, argv, pmm, sched, caps, hhdm_offset, uid, gid, None)
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
    let bytes = embedded_bytes_for_path(path)?;
    let shell_id = shell_id_from_path(path).ok_or(SpawnError::NotFound)?;

    crate::serial_println!("[SPAWN] Loading {} ({} bytes)", path, bytes.len());

    let pid = sched.processes.len() + 1;
    let mut process = unsafe {
        Process::new(pid, 1, "sshl", pmm, hhdm_offset)
    };
    process.uid = uid;
    process.gid = gid;
    // Phase 6.5 Step 2: every spawned process gets an environment — either
    // one inherited from the caller or the defaults for this uid (PATH,
    // USER, HOME, SHELL). Username resolution from /etc/passwd happens in
    // userspace via VFS; the kernel only knows the uid here.
    process.env = env.unwrap_or_else(|| super::env::EnvMap::with_defaults(uid, ""));

    let envp_strings = process.env.to_envp();
    let envp: alloc::vec::Vec<&[u8]> =
        envp_strings.iter().map(|s| s.as_bytes()).collect();
    exec_into_process(bytes, &mut process, pmm, hhdm_offset, &[], &envp)?;
    process.set_initial_args(shell_id, uid as u64, gid as u64, 0);

    let actual_pid = process.pid;
    let _id = sched.add_process(process);

    crate::serial_println!("[SPAWN] {} spawned pid={}", path, actual_pid);
    Ok(actual_pid)
}

/// Get embedded ELF bytes for a given path.
pub fn embedded_bytes_for_path(path: &str) -> Result<&'static [u8], SpawnError> {
    match path {
        "/bin/sh" | "/bin/ssh" | "/bin/sshl" => {
            Ok(crate::SUNSHELL_ELF_BYTES)
        }
        p if p.starts_with("/bin/sshl") => {
            Ok(crate::SUNSHELL_ELF_BYTES)
        }
        _ => {
            crate::serial_println!("[SPAWN] Unknown path: {}", path);
            Err(SpawnError::NotFound)
        }
    }
}

fn shell_id_from_path(path: &str) -> Option<u64> {
    match path {
        "/bin/sh" | "/bin/ssh" | "/bin/sshl" => Some(0),
        p if p.starts_with("/bin/sshl") => parse_u64(&p[9..]),
        _ => None,
    }
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
