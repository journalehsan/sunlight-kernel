use super::{Process, ProcessState};
use crate::capability::{CapabilityBroker, CapabilityRights};
use crate::memory::pmm::PhysicalMemoryManager;
use crate::sched::Scheduler;
use x86_64::{
    structures::paging::{Page, PageTableFlags, PhysFrame},
    VirtAddr,
};

/// Extract the basename (the component after the last `/`) from a path.
/// Used to give spawned processes a meaningful name instead of "daemon".
pub fn name_from_path(path: &str) -> &str {
    path.rfind('/').map(|i| &path[i + 1..]).unwrap_or(path)
}

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
    process.address_space =
        unsafe { crate::process::address_space::AddressSpace::new(pmm, hhdm_offset) };

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
        let flags =
            PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE;
        unsafe {
            process
                .address_space
                .map_page(page, phys, flags, pmm, hhdm_offset);
        }
    }

    // Setup stack with argv/envp
    let stack = setup_exec_stack(argv, envp, process, hhdm_offset)?;

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

/// Marshal argc/argv/envp onto the user stack per the SysV x86_64 ABI.
///
/// Layout (high → low): NUL-terminated string data, padding, then the
/// pointer table `[argc][argv0..argvN][NULL][envp0..envpM][NULL]` with the
/// final RSP 16-byte aligned and pointing at argc.
fn setup_exec_stack(
    argv: &[&[u8]],
    envp: &[&[u8]],
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

    let mut argv_addrs = alloc::vec::Vec::with_capacity(argv.len());
    for arg in argv {
        argv_addrs.push(copy_string(&mut cursor, arg)?);
    }
    let mut envp_addrs = alloc::vec::Vec::with_capacity(envp.len());
    for env in envp {
        envp_addrs.push(copy_string(&mut cursor, env)?);
    }

    // Pointer table: argc + argv pointers + NULL + envp pointers + NULL.
    let table_words = 1 + argv.len() + 1 + envp.len() + 1;
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
    table.extend_from_slice(&0u64.to_le_bytes());
    for addr in &envp_addrs {
        table.extend_from_slice(&addr.to_le_bytes());
    }
    table.extend_from_slice(&0u64.to_le_bytes());
    copy_to_user(process, hhdm_offset, rsp, &table)?;

    crate::serial_println!(
        "[EXEC] Stack: argc={} envc={} rsp={:#x}",
        argv.len(),
        envp.len(),
        rsp
    );

    Ok(ExecStack {
        rsp,
        argv_ptr: rsp + 8,
        envp_ptr: rsp + 8 + (argv.len() as u64 + 1) * 8,
    })
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
    let mut process = unsafe { Process::new(pid, 1, proc_name, pmm, hhdm_offset) };
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

    let envp_strings = process.env.to_envp();
    let envp: alloc::vec::Vec<&[u8]> = envp_strings.iter().map(|s| s.as_bytes()).collect();
    exec_into_process(bytes, &mut process, pmm, hhdm_offset, &[], &envp)?;
    process.set_initial_args(shell_id.unwrap_or(0), uid as u64, gid as u64, 0);

    let actual_pid = process.pid;
    let _id = sched.add_process(process);

    crate::serial_println!("[SPAWN] {} spawned pid={}", path, actual_pid);
    Ok(actual_pid)
}

/// Get embedded ELF bytes for a given path.
pub fn embedded_bytes_for_path(path: &str) -> Result<&'static [u8], SpawnError> {
    match path {
        "/bin/sh" | "/bin/ssh" | "/bin/sshl" => Ok(crate::SUNSHELL_ELF_BYTES),
        p if p.starts_with("/bin/sshl") => Ok(crate::SUNSHELL_ELF_BYTES),
        // POSIX-style command paths: standard applets execute from /bin or
        // /usr/bin and dispatch by argv[0] inside the multi-call binaries.
        "/bin/ls" | "/bin/cat" | "/bin/cp" | "/bin/mv" | "/bin/rm" | "/bin/mkdir"
        | "/bin/rmdir" | "/bin/touch" | "/bin/find" | "/bin/grep" | "/bin/head" | "/bin/tail"
        | "/bin/wc" | "/bin/sort" | "/bin/uniq" | "/bin/cut" | "/bin/file" | "/bin/stat"
        | "/bin/pwd" | "/bin/date" | "/bin/whoami" | "/bin/id" | "/bin/uname" | "/bin/echo"
        | "/bin/nice" | "/bin/renice" | "/bin/free" | "/bin/freezram" | "/usr/bin/ls"
        | "/usr/bin/cat" | "/usr/bin/cp" | "/usr/bin/mv" | "/usr/bin/rm" | "/usr/bin/mkdir"
        | "/usr/bin/rmdir" | "/usr/bin/touch" | "/usr/bin/find" | "/usr/bin/grep"
        | "/usr/bin/head" | "/usr/bin/tail" | "/usr/bin/wc" | "/usr/bin/sort" | "/usr/bin/uniq"
        | "/usr/bin/cut" | "/usr/bin/file" | "/usr/bin/stat" | "/usr/bin/pwd" | "/usr/bin/date"
        | "/usr/bin/whoami" | "/usr/bin/id" | "/usr/bin/uname" | "/usr/bin/echo"
        | "/usr/bin/nice" | "/usr/bin/renice" | "/usr/bin/free" | "/usr/bin/freezram" => {
            Ok(crate::SUNLIGHT_UTILS_ELF_BYTES)
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
        "/sbin/net_server" | "/usr/sbin/net_server" => Ok(crate::NET_SERVER_ELF_BYTES),
        "/sbin/sunlightd" | "/usr/sbin/sunlightd" => Ok(crate::SUNLIGHTD_ELF_BYTES),
        // User-level daemons spawned by sunlightd (not hardcoded in kernel boot).
        "/sbin/timezone_service" | "/usr/sbin/timezone_service" => {
            Ok(crate::TIMEZONE_SERVICE_ELF_BYTES)
        }
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
        "/bin/certificatectl" | "/usr/bin/certificatectl" => Ok(crate::CERTIFICATECTL_ELF_BYTES),
        // User Access Control daemon (spawned by sunlightd) + control client.
        "/sbin/uac_service" | "/usr/sbin/uac_service" => Ok(crate::UAC_SERVICE_ELF_BYTES),
        "/bin/capabilityctl" | "/usr/bin/capabilityctl" => Ok(crate::CAPABILITYCTL_ELF_BYTES),
        "/bin/runas" | "/usr/bin/runas" => Ok(crate::RUNAS_ELF_BYTES),
        "/usr/bin/top" | "/bin/top" => Ok(crate::SUNLIGHT_TOP_ELF_BYTES),
        "/usr/bin/sunlightctl" | "/bin/sunlightctl" => Ok(crate::SUNLIGHTCTL_ELF_BYTES),
        "/bin/fetch" | "/usr/bin/fetch" => Ok(crate::SUNLIGHT_FETCH_ELF_BYTES),
        // Storage Manager (whitelisted protected FS writes for services such as sunlight-kv).
        "/sbin/sunlight-sm" | "/usr/sbin/sunlight-sm" => Ok(crate::SUNLIGHT_SM_ELF_BYTES),
        // Phase 6.5 Step 3: PATH entries under these directories are applets
        // of the embedded multi-call binaries (argv[0] picks the applet).
        p if p.starts_with("/sunlight-utils/") => Ok(crate::SUNLIGHT_UTILS_ELF_BYTES),
        p if p.starts_with("/sunlight-net-utils/") => Ok(crate::SUNLIGHT_NET_UTILS_ELF_BYTES),
        _ => Err(SpawnError::NotFound),
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
