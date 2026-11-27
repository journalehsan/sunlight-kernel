#[cfg(all(debug_assertions, feature = "helios_runtime_c_startup"))]
mod c_startup;

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

/// Bootstrap the existing niced High policy before the embedded timer's first
/// Ready publication. Never grant priority by process name or to a non-root
/// launch. Root already has authority to set this nice through setnice.
/// Keep this value aligned with sunlight-niced's protected timer policy.
pub fn initial_embedded_nice(path: &str, uid: u32) -> i8 {
    if uid == 0 && path == "/sbin/timer_server" {
        -5
    } else {
        0
    }
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
        "/bin/sunlight-control-panel"
            | "/usr/bin/sunlight-control-panel"
            | "/bin/control-panel"
            | "/usr/bin/control-panel"
    )
}

/// True only for the exact embedded online-accounts broker path. It is the sole
/// process permitted to read/write the `account-broker:*` KV namespace and the
/// only one allowed to resolve caller roles for the control-panel grant.
pub fn is_trusted_account_broker_path(path: &str) -> bool {
    matches!(
        path,
        "/sbin/sunlight-account-broker" | "/usr/sbin/sunlight-account-broker"
    )
}

/// True only for the exact embedded sunlight-kv daemon path. It may resolve
/// kernel-authenticated IPC caller credentials through the KV credential
/// syscall, mirroring `is_trusted_pty_service_path`.
pub fn is_trusted_kv_service_path(path: &str) -> bool {
    matches!(path, "/sbin/sunlight-kv" | "/usr/sbin/sunlight-kv")
}

/// Kernel-provenance service identity key for a trusted embedded daemon path.
///
/// Returns the canonical service key ([`sunlight_ipc::name_to_u64`] of the
/// service name) for embedded system daemons, or 0 for everything else. This is
/// how the KV daemon classifies a caller as `Caller::Service(key)` without ever
/// trusting a client-supplied service name: the key is stamped by the kernel at
/// load/exec time from the exact embedded executable path.
pub fn trusted_service_identity_for_path(path: &str) -> u64 {
    match path {
        "/sbin/sunlight-kbd" | "/usr/sbin/sunlight-kbd" => sunlight_ipc::service_identity::KEYBOARD,
        "/sbin/sunlight-kv" | "/usr/sbin/sunlight-kv" => sunlight_ipc::service_identity::KV,
        "/sbin/sunlight-tls" | "/usr/sbin/sunlight-tls" => sunlight_ipc::service_identity::TLS,
        "/sbin/uac_service" | "/usr/sbin/uac_service" => {
            sunlight_ipc::service_identity::AUTH_BROKER
        }
        "/sbin/sunlight-sm" | "/usr/sbin/sunlight-sm" => {
            sunlight_ipc::service_identity::STORAGE_MANAGER
        }
        "/sbin/sunlightd" | "/usr/sbin/sunlightd" => {
            sunlight_ipc::service_identity::SERVICE_MANAGER
        }
        "/sbin/sunlight-sessiond" | "/usr/sbin/sunlight-sessiond" => {
            sunlight_ipc::service_identity::SESSION_MANAGER
        }
        "/sbin/sunlight-account-broker" | "/usr/sbin/sunlight-account-broker" => {
            sunlight_ipc::service_identity::ACCOUNT_BROKER
        }
        _ => 0,
    }
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
    PidExhausted,
    SessionRejected,
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
    exec_into_process_inner(
        bytes,
        process,
        pmm,
        hhdm_offset,
        argv,
        envp,
        activate_on_success,
        &mut || Ok(()),
        #[cfg(all(debug_assertions, feature = "helios_dynamic_elf_handoff"))]
        None,
    )
}

pub(super) fn load_pending_image(
    bytes: &[u8],
    process: &mut Process,
    pmm: &mut PhysicalMemoryManager,
    hhdm: VirtAddr,
    argv: &[&[u8]],
    envp: &[&[u8]],
    checkpoint: &mut impl FnMut() -> Result<(), SpawnError>,
) -> Result<u64, SpawnError> {
    exec_into_process_inner(
        bytes,
        process,
        pmm,
        hhdm,
        argv,
        envp,
        false,
        checkpoint,
        #[cfg(all(debug_assertions, feature = "helios_dynamic_elf_handoff"))]
        None,
    )
}

fn exec_into_process_inner(
    bytes: &[u8],
    process: &mut Process,
    pmm: &mut PhysicalMemoryManager,
    hhdm_offset: VirtAddr,
    argv: &[&[u8]],
    envp: &[&[u8]],
    activate_on_success: bool,
    checkpoint: &mut impl FnMut() -> Result<(), SpawnError>,
    #[cfg(all(debug_assertions, feature = "helios_dynamic_elf_handoff"))] fault: Option<
        super::elf_loader::handoff::Fault,
    >,
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
    let old_trusted_kv_service = process.trusted_kv_service;
    let old_trusted_session_service = process.trusted_session_service;
    let old_trusted_wiseowl_braind = process.trusted_wiseowl_braind;
    let old_trusted_wiseowl_console = process.trusted_wiseowl_console;
    let old_trusted_control_panel = process.trusted_control_panel;
    let old_trusted_account_broker = process.trusted_account_broker;
    let old_personality = process.personality;
    let old_linux_tgid = process.linux_tgid;
    let old_linux_sysv_semadj = process.linux_sysv_semadj.take();
    let old_service_identity = process.service_identity;
    process.trusted_display_service = false;
    process.trusted_swap_admin_service = false;
    process.trusted_zram_diagnostic = false;
    process.trusted_pty_service = false;
    process.trusted_kv_service = false;
    process.service_identity = 0;
    process.trusted_session_service = false;
    process.trusted_wiseowl_braind = false;
    process.trusted_wiseowl_console = false;
    process.trusted_control_panel = false;
    process.trusted_account_broker = false;

    process.personality = personality;
    process.linux_tgid = process.is_linux_compat().then_some(process.pid);
    process.linux_sysv_semadj = process
        .is_linux_compat()
        .then_some(crate::process::LinuxSysvSemAdj::default());
    if process.is_linux_compat() {
        crate::serial_println!("[EXEC] Linux ELF detected");
    }

    #[cfg(all(debug_assertions, feature = "helios_dynamic_elf_tls_fixture"))]
    let result = if activate_on_success && bytes == super::elf_loader::handoff::tls::APP {
        Err(SpawnError::ElfLoadFailed)
    } else {
        build_exec_image(bytes, process, pmm, hhdm_offset, argv, envp, checkpoint, fault)
    };
    #[cfg(not(all(debug_assertions, feature = "helios_dynamic_elf_tls_fixture")))]
    let result = build_exec_image(
        bytes,
        process,
        pmm,
        hhdm_offset,
        argv,
        envp,
        checkpoint,
        #[cfg(all(debug_assertions, feature = "helios_dynamic_elf_handoff"))]
        fault,
    );
    let entry = match result {
        Ok(entry) => entry,
        Err(error) => {
            let mut failed_address_space =
                core::mem::replace(&mut process.address_space, old_address_space);
            let _reclaimed =
                unsafe { failed_address_space.reclaim_user_space(pmm, hhdm_offset, true) };
            #[cfg(all(debug_assertions, feature = "helios_dynamic_elf_handoff"))]
            if fault.is_some() {
                crate::serial_println!(
                    "[HELIOS-HANDOFF] rollback pid={} frames={} tables={}",
                    process.pid,
                    _reclaimed.user_frames,
                    _reclaimed.page_tables
                );
            }
            process.trusted_display_service = old_trusted_display;
            process.trusted_swap_admin_service = old_trusted_swap_admin;
            process.trusted_zram_diagnostic = old_trusted_zram_diagnostic;
            process.trusted_pty_service = old_trusted_pty_service;
            process.trusted_kv_service = old_trusted_kv_service;
            process.trusted_session_service = old_trusted_session_service;
            process.trusted_wiseowl_braind = old_trusted_wiseowl_braind;
            process.trusted_wiseowl_console = old_trusted_wiseowl_console;
            process.trusted_control_panel = old_trusted_control_panel;
            process.trusted_account_broker = old_trusted_account_broker;
            process.personality = old_personality;
            process.linux_tgid = old_linux_tgid;
            process.linux_sysv_semadj = old_linux_sysv_semadj;
            process.service_identity = old_service_identity;
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

    // Completion receipts belong to the retired image; failed exec above
    // preserves them along with the original VM.
    process.clear_futex_wait();

    let mut fenced_vfs_exec = None;
    if activate_on_success {
        // The only live-image caller is sys_exec, holding SCHEDULER -> PMM
        // without CAP_BROKER. Unpublished spawn uses false and may already
        // own the broker. Withdraw before activation can replace the syscall
        // stack's VM; failed image preparation above preserves old authority.
        let mut broker = crate::capability::CAP_BROKER.lock();
        let old_caller = sunlight_ipc::vfs_rpc::lifecycle::Caller {
            pid: process.pid as u64,
            incarnation: process.foreground.incarnation,
            address_space_generation: old_address_space.identity().generation,
        };
        let was_trusted_vfs = process.trusted_vfs_service;
        // Hold the file-owner barrier until sys_exec has published transfers
        // for every surviving non-CLOEXEC service descriptor.
        crate::ipc::expect_vfs_file_transfer(old_caller);
        crate::ipc::retire_vfs_process(old_caller);
        #[cfg(any(feature = "cap_issuance_test", feature = "production_grant_test"))]
        let old_grants = broker.recipient_grant_count(process.pid);
        broker.retire_recipient_grants(process.pid);
        process.withdraw_exec_image_roles();
        fenced_vfs_exec = super::exec_retirement::FencedVfsExec::after_withdrawal(
            crate::capability::delivery::RecipientIdentity {
                pid: process.pid,
                incarnation: old_caller.incarnation,
                vm_generation: old_caller.address_space_generation,
            },
            was_trusted_vfs,
            process.trusted_vfs_service,
        );
        process.pending_call = None;
        process.ipc_reply_target = None;
        #[cfg(feature = "production_grant_test")]
        crate::capability::production_gate::exec(
            process,
            old_address_space.identity(),
            old_grants,
            &mut broker,
        );
        #[cfg(feature = "cap_issuance_test")]
        if old_grants != 0 {
            assert_eq!(old_grants, 1);
            assert_eq!(broker.recipient_grant_count(process.pid), 0);
            crate::serial_println!(
                "[CAP-PR] exec authority withdrawn before VM activation records=1 remaining=0 account=0"
            );
        }
    }

    #[cfg(feature = "cap_issuance_test")]
    if activate_on_success {
        crate::capability::gate::assert_exec_authority(process, old_address_space.identity());
    }

    if activate_on_success {
        // FS belongs to the retired image; the replacement runtime initializes TLS.
        process.fs_base = 0;
        #[cfg(feature = "cap_issuance_test")]
        {
            let rsp: u64;
            unsafe {
                core::arch::asm!("mov {}, rsp", out(reg) rsp, options(nomem, nostack));
            }
            crate::serial_println!("[EXEC-R2] commit pid={} incarnation={} old_vm={:?} new_vm={:?} kernel_rsp={:#x} entry={:#x} user_rsp={:#x}",
                process.pid, process.foreground.incarnation, old_address_space.identity(),
                process.address_space.identity(), rsp, entry, process.user_stack_top);
            assert!(rsp >= process.kernel_stack_top - super::KERNEL_STACK_SIZE as u64);
            assert!(rsp < process.kernel_stack_top - 160);
        }
        unsafe {
            process.address_space.activate();
        }
    }
    #[cfg(all(debug_assertions, feature = "quota_lifecycle_test"))]
    crate::memory::quota_lifecycle::process(process, "exec-new-vm");
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
        #[cfg(feature = "cap_issuance_test")]
        if activate_on_success {
            assert_eq!(
                crate::memory::tlb::active_cpu_mask(old_address_space.identity()),
                0
            );
            let stats = old_address_space.reclaim_user_space(pmm, hhdm_offset, true);
            crate::serial_println!(
                "[EXEC-R2] retired VM reclaimed frames={} tables={} swap={} active_cpus=0",
                stats.user_frames,
                stats.page_tables,
                stats.swap_blocks
            );
        } else {
            old_address_space.reclaim_user_space(pmm, hhdm_offset, true);
        }
        #[cfg(not(feature = "cap_issuance_test"))]
        old_address_space.reclaim_user_space(pmm, hhdm_offset, true);
    }

    if let Some(proof) =
        fenced_vfs_exec.and_then(|fence| fence.after_reclaim(old_address_space.is_reclaimed()))
    {
        crate::ipc::complete_vfs_private_exec(proof);
    }

    // Bind this one publication to the actual embedded image. Do not reclassify
    // net_server for general service credentials or trust a path/name supplied
    // by a caller. A successful replacement also withdraws the marker.
    process.trusted_network_discovery =
        process.uid == 0 && core::ptr::eq(bytes, crate::NET_SERVER_ELF_BYTES);
    Ok(entry)
}

fn build_exec_image(
    bytes: &[u8],
    process: &mut Process,
    pmm: &mut PhysicalMemoryManager,
    hhdm_offset: VirtAddr,
    argv: &[&[u8]],
    envp: &[&[u8]],
    checkpoint: &mut impl FnMut() -> Result<(), SpawnError>,
    #[cfg(all(debug_assertions, feature = "helios_dynamic_elf_handoff"))] fault: Option<
        super::elf_loader::handoff::Fault,
    >,
) -> Result<u64, SpawnError> {
    #[cfg(all(debug_assertions, feature = "helios_dynamic_elf_handoff"))]
    let handoff = super::elf_loader::handoff::load(bytes, process, pmm, hhdm_offset, fault,
        #[cfg(feature = "helios_dynamic_elf_shared_fixture")]
        argv.first().and_then(|path| super::elf_loader::handoff::trusted_library(path)),
    )
        .map_err(|stage| {
            crate::serial_println!(
                "[HELIOS-HANDOFF] rejected stage={} pid={}",
                stage,
                process.pid
            );
            SpawnError::ElfLoadFailed
        })?;
    #[cfg(all(debug_assertions, feature = "helios_dynamic_elf_handoff"))]
    let dynamic_entry = handoff.as_ref().map(|image| image.entry);
    #[cfg(not(all(debug_assertions, feature = "helios_dynamic_elf_handoff")))]
    let dynamic_entry = None;
    let entry = match dynamic_entry {
        Some(entry) => entry,
        None => super::elf_loader::load_elf(bytes, process, pmm, hhdm_offset)
            .ok_or(SpawnError::ElfLoadFailed)?,
    };

    #[cfg(all(debug_assertions, feature = "helios_dynamic_elf_relocation_fixture"))]
    let fail_stack = fault == Some(super::elf_loader::handoff::Fault::PostRelocationAllocation);
    #[cfg(all(debug_assertions, feature = "helios_dynamic_elf_shared_fixture"))]
    let fail_stack = fail_stack || fault == Some(super::elf_loader::handoff::Fault::Shared(
        super::elf_loader::handoff::SharedFault::Stack,
    ));
    #[cfg(all(debug_assertions, feature = "helios_dynamic_elf_tls_fixture"))]
    let fail_stack = fail_stack || fault == Some(super::elf_loader::handoff::Fault::Tls(
        super::elf_loader::handoff::tls::Fault::Stack,
    )) || fault == Some(super::elf_loader::handoff::Fault::StartupTls(
        super::elf_loader::handoff::tls::StartupFault::Stack,
    ));
    #[cfg(all(debug_assertions, feature = "helios_dynamic_elf_relocation_fixture"))]
    if fail_stack {
        pmm.fail_test_allocations_after(0);
    }
    let stack_mapping = if cfg!(feature = "demand_main_stack") && !process.is_linux_compat() {
        native_stack_seed(argv, envp)
            .and_then(|seed| map_main_stack(process, pmm, hhdm_offset, Some(seed)))
    } else {
        map_user_stack(process, pmm, hhdm_offset)
    };
    #[cfg(all(debug_assertions, feature = "helios_dynamic_elf_relocation_fixture"))]
    if fail_stack {
        pmm.clear_test_allocation_failure();
    }
    stack_mapping.inspect_err(|_error| {
        if dynamic_entry.is_some() {
            crate::serial_println!(
                "[HELIOS-HANDOFF] rejected stage=initial stack pid={}",
                process.pid
            );
        }
    })?;

    // Build auxv for Linux-compat processes so musl's _start doesn't scan
    // past the stack top looking for AT_NULL and fault at USER_STACK_TOP.
    // AT_PHDR + AT_PHENT + AT_PHNUM are required for musl `__init_tls` to walk
    // program headers and find PT_TLS. Without AT_PHENT (size of each phdr),
    // the walk never advances, TLS stays zeroed, and Rust `OnceLock` treats the
    // all-zero Once state as COMPLETE — e.g. ratatui's layout cache then
    // null-derefs on first `Layout::split` (helios-note page fault at 0x48).
    let mut auxv = heapless::Vec::<(u64, u64), 11>::new();
    if process.is_linux_compat() {
        #[cfg(all(debug_assertions, feature = "helios_dynamic_elf_handoff"))]
        let metadata = handoff
            .as_ref()
            .map(|image| (image.header, image.phdr, image.app_entry));
        #[cfg(not(all(debug_assertions, feature = "helios_dynamic_elf_handoff")))]
        let metadata: Option<(sunlight_elf::ElfHeader, u64, u64)> = None;
        let metadata = metadata.or_else(|| {
            sunlight_elf::parse_elf_header(bytes)
                .ok()
                .map(|header| (header, compute_at_phdr(bytes, &header), header.entry))
        });
        match metadata {
            Some((hdr, at_phdr, app_entry)) => {
                let entries = [
                    (sunlight_compat_linux::abi::AT_PHDR, at_phdr),
                    (sunlight_compat_linux::abi::AT_PHENT, hdr.phentsize as u64),
                    (sunlight_compat_linux::abi::AT_PHNUM, hdr.phnum as u64),
                    (sunlight_compat_linux::abi::AT_PAGESZ, 4096u64),
                    (sunlight_compat_linux::abi::AT_ENTRY, app_entry),
                    (sunlight_compat_linux::abi::AT_UID, process.uid as u64),
                    (sunlight_compat_linux::abi::AT_EUID, process.uid as u64),
                    (sunlight_compat_linux::abi::AT_GID, process.gid as u64),
                    (sunlight_compat_linux::abi::AT_EGID, process.gid as u64),
                    (sunlight_compat_linux::abi::AT_SECURE, 0u64),
                ];
                auxv.extend_from_slice(&entries)
                    .map_err(|_| SpawnError::NoMemory)?;
            }
            None => (),
        }
    }
    #[cfg(all(debug_assertions, feature = "helios_dynamic_elf_handoff"))]
    if let Some(image) = &handoff {
        use super::elf_loader::handoff::Fault;
        let fail_auxv = fault == Some(Fault::Auxv);
        #[cfg(feature = "helios_dynamic_elf_shared_fixture")]
        let fail_auxv = fail_auxv || fault == Some(Fault::Shared(super::elf_loader::handoff::SharedFault::Auxv));
        let capacity = if fail_auxv {
            0
        } else {
            auxv.capacity()
        };
        if auxv.len() >= capacity {
            crate::serial_println!("[HELIOS-HANDOFF] rejected stage=auxv pid={}", process.pid);
            return Err(SpawnError::NoMemory);
        }
        auxv.push((sunlight_compat_linux::abi::AT_BASE, image.base))
            .map_err(|_| SpawnError::NoMemory)?;
        crate::serial_println!(
            "[HELIOS-HANDOFF] pid={} AT_BASE={:#x} AT_PHDR={:#x} AT_PHNUM={} AT_ENTRY={:#x} interpreter_entry={:#x}",
            process.pid, image.base, image.phdr, image.header.phnum, image.app_entry, entry
        );
    }

    // Setup stack with argv/envp/auxv
    let stack_floor = super::layout::USER_STACK_TOP - super::layout::USER_STACK_SIZE;
    #[cfg(all(debug_assertions, feature = "helios_dynamic_elf_handoff"))]
    let stack_floor = if fault == Some(super::elf_loader::handoff::Fault::Stack) {
        super::layout::USER_STACK_TOP - 16
    } else {
        stack_floor
    };
    #[cfg(all(debug_assertions, feature = "helios_runtime_c_startup"))]
    let stack_floor = if fault == Some(super::elf_loader::handoff::Fault::CStartupOverflow) {
        super::layout::USER_STACK_TOP - 64
    } else {
        stack_floor
    };
    let stack = setup_exec_stack(argv, envp, &auxv, process, hhdm_offset, stack_floor, checkpoint)
        .inspect_err(|_error| {
            if dynamic_entry.is_some() {
                crate::serial_println!(
                    "[HELIOS-HANDOFF] rejected stage=initial stack pid={}",
                    process.pid
                );
            }
        })?;

    #[cfg(all(debug_assertions, feature = "helios_runtime_c_startup"))]
    let stack = if bytes == c_startup::IMAGE {
        let mut stack = stack;
        c_startup::validate(process, hhdm_offset, &mut stack, &auxv, fault)?;
        stack
    } else {
        stack
    };

    let args_len = argv.iter().fold(0usize, |len, arg| {
        len.saturating_add(arg.len().saturating_add(1)).min(1024)
    });
    if args_len > process.proc_args.capacity() {
        checkpoint()?;
        process
            .proc_args
            .try_reserve_exact(args_len - process.proc_args.len())
            .map_err(|_| SpawnError::NoMemory)?;
    }
    process.proc_args.clear();
    for arg in argv {
        let remaining = 1024 - process.proc_args.len();
        if remaining == 0 {
            break;
        }
        process
            .proc_args
            .extend_from_slice(&arg[..arg.len().min(remaining - 1)]);
        process.proc_args.push(0);
    }

    process.init_context(entry, stack.rsp);
    #[cfg(all(debug_assertions, feature = "helios_dynamic_elf_tls_fixture"))]
    {
        process.fs_base = handoff.as_ref().map_or(0, |image| image.thread_pointer);
    }
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

#[cfg(all(debug_assertions, feature = "helios_dynamic_elf_handoff"))]
pub(crate) fn test_handoff_rollback() {
    use super::elf_loader::handoff::{Fault, INTERPRETER};
    use crate::memory::accounting::{class_frame_count, PhysicalMemoryClass};

    let fixture = include_bytes!(concat!(
        env!("SUNLIGHT_DYNAMIC_PROBES"),
        "/elf-pie.sunapp/Program/probe"
    ));
    let mut rejected = fixture.to_vec();
    let start = rejected
        .windows(INTERPRETER.len())
        .position(|window| window == INTERPRETER)
        .expect("fixture PT_INTERP");
    rejected[start + 1] = b'x';

    let mut scheduler = crate::sched::SCHEDULER.lock();
    let mut pmm = crate::PMM.lock();
    let hhdm = VirtAddr::new(crate::HHDM_REQ.response().expect("no hhdm").offset);
    let task_count = scheduler.processes.len();
    let pid = scheduler.next_process_pid()
        .expect("rollback test pid");
    let cases = [
        (
            Fault::Missing,
            "missing_interpreter",
            SpawnError::ElfLoadFailed,
        ),
        (Fault::Path, "rejected_path", SpawnError::ElfLoadFailed),
        (Fault::Machine, "wrong_machine", SpawnError::ElfLoadFailed),
        (
            Fault::Malformed,
            "malformed_interpreter",
            SpawnError::ElfLoadFailed,
        ),
        (
            Fault::Application,
            "application_mapping",
            SpawnError::ElfLoadFailed,
        ),
        (
            Fault::Interpreter,
            "interpreter_mapping",
            SpawnError::ElfLoadFailed,
        ),
        (
            Fault::PageTable,
            "page_table_allocation",
            SpawnError::ElfLoadFailed,
        ),
        (Fault::Stack, "initial_stack", SpawnError::NoMemory),
        (Fault::Auxv, "auxv", SpawnError::NoMemory),
    ];
    #[cfg(all(debug_assertions, feature = "helios_dynamic_elf_relocation_fixture"))]
    let relocation_fixture = include_bytes!(concat!(
        env!("SUNLIGHT_DYNAMIC_PROBES"),
        "/elf-relocation.sunapp/Program/probe"
    ));
    #[cfg(all(debug_assertions, feature = "helios_dynamic_elf_relocation_fixture"))]
    let relocation_cases = [
        (Fault::RelocationWrite, "partial_write", SpawnError::ElfLoadFailed),
        (Fault::PostRelocationAllocation, "post_relocation_allocation", SpawnError::NoMemory),
    ];
    #[cfg(not(feature = "helios_dynamic_elf_relocation_fixture"))]
    let relocation_cases = [];
    #[cfg(feature = "helios_dynamic_elf_shared_fixture")]
    use super::elf_loader::handoff::SharedFault;
    #[cfg(feature = "helios_dynamic_elf_shared_fixture")]
    let shared_cases = [
        (SharedFault::Needed, "needed"),
        (SharedFault::Missing, "missing_library"),
        (SharedFault::Identity, "wrong_identity"),
        (SharedFault::Malformed, "malformed_library"),
        (SharedFault::Machine, "wrong_library_machine"),
        (SharedFault::Mapping, "library_mapping"),
        (SharedFault::Dynamic, "dynamic_table"),
        (SharedFault::Symbol, "symbol_table"),
        (SharedFault::Relocation, "jump_slot_write"),
        (SharedFault::Stack, "shared_stack"),
        (SharedFault::Auxv, "shared_auxv"),
        (SharedFault::Process, "process_allocation"),
    ].map(|(fault, label)| {
        let expected = if matches!(fault, SharedFault::Stack | SharedFault::Auxv | SharedFault::Process) {
            SpawnError::NoMemory
        } else {
            SpawnError::ElfLoadFailed
        };
        (Fault::Shared(fault), label, expected)
    });
    #[cfg(not(feature = "helios_dynamic_elf_shared_fixture"))]
    let shared_cases = [];
    #[cfg(feature = "helios_dynamic_elf_shared_fixture")]
    let real_cases = {
        use sunlight_elf::shared_fixture::TestFault;
        [
            (SharedFault::Identity, "real_wrong_hash"),
            (SharedFault::Malformed, "real_malformed"),
            (SharedFault::Mapping, "real_mapping"),
            (SharedFault::Metadata(TestFault::MissingExport), "real_missing_export"),
            (SharedFault::Metadata(TestFault::DuplicateExport), "real_duplicate_export"),
            (SharedFault::Metadata(TestFault::UndefinedImport), "real_bad_undefined_symbol"),
            (SharedFault::Metadata(TestFault::SymbolRange), "real_symbol_range"),
            (SharedFault::Metadata(TestFault::RelocationRange), "real_relocation_range"),
            (SharedFault::Metadata(TestFault::RelocationType), "real_unsupported_relocation"),
        ].map(|(fault, label)| (Fault::Shared(fault), label, SpawnError::ElfLoadFailed))
    };
    #[cfg(not(feature = "helios_dynamic_elf_shared_fixture"))]
    let real_cases = [];
    #[cfg(feature = "helios_dynamic_elf_shared_fixture")]
    let graph_cases = {
        use super::elf_loader::handoff::GraphFault;
        [
            (GraphFault::Missing(0), "graph_missing_a"),
            (GraphFault::Missing(1), "graph_missing_b"),
            (GraphFault::Hash(0), "graph_hash_a"),
            (GraphFault::Hash(1), "graph_hash_b"),
            (GraphFault::Undeclared, "graph_undeclared"),
            (GraphFault::Cycle, "graph_cycle"),
            (GraphFault::Duplicate, "graph_duplicate"),
            (GraphFault::Unresolved, "graph_unresolved"),
            (GraphFault::Ambiguous, "graph_ambiguous"),
            (GraphFault::Path, "graph_path"),
            (GraphFault::NeededOffset, "graph_needed_offset"),
            (GraphFault::Mapping, "graph_mapping"),
            (GraphFault::LeafRelocation, "graph_leaf_relocation"),
            (GraphFault::PageTable, "graph_page_table"),
        ].map(|(fault, label)| (Fault::Shared(SharedFault::Graph(fault)), label, SpawnError::ElfLoadFailed))
        .into_iter().chain([
            (Fault::Shared(SharedFault::Stack), "graph_stack", SpawnError::NoMemory),
            (Fault::Shared(SharedFault::Process), "graph_process", SpawnError::NoMemory),
        ])
    };
    #[cfg(not(feature = "helios_dynamic_elf_shared_fixture"))]
    let graph_cases = [];
    #[cfg(feature = "helios_dynamic_elf_tls_fixture")]
    let tls_cases = {
        use super::elf_loader::handoff::tls::Fault as Tls;
        [
            (Fault::Tls(Tls::Validation), "tls_malformed", SpawnError::ElfLoadFailed),
            (Fault::Tls(Tls::MissingTail), "tls_missing_tail", SpawnError::ElfLoadFailed),
            (Fault::Tls(Tls::Alignment), "tls_alignment", SpawnError::ElfLoadFailed),
            (Fault::Tls(Tls::Unmapped), "tls_unmapped", SpawnError::ElfLoadFailed),
            (Fault::Tls(Tls::FileSize), "tls_filesz", SpawnError::ElfLoadFailed),
            (Fault::Tls(Tls::Overflow), "tls_overflow", SpawnError::ElfLoadFailed),
            (Fault::Tls(Tls::Allocation), "tls_allocation", SpawnError::ElfLoadFailed),
            (Fault::Tls(Tls::Stack), "tls_stack", SpawnError::NoMemory),
            (Fault::Tls(Tls::Interpreter), "tls_interpreter", SpawnError::ElfLoadFailed),
            (Fault::Tls(Tls::Process), "tls_process", SpawnError::NoMemory),
        ]
    };
    #[cfg(not(feature = "helios_dynamic_elf_tls_fixture"))]
    let tls_cases = [];
    #[cfg(feature = "helios_dynamic_elf_tls_fixture")]
    let startup_tls_cases = {
        use super::elf_loader::handoff::tls::StartupFault::*;
        [
            (Module, "startup_tls_module"), (Missing, "startup_tls_missing"),
            (Duplicate, "startup_tls_duplicate"), (Malformed, "startup_tls_malformed"),
            (FileSize, "startup_tls_filesz"), (Alignment, "startup_tls_alignment"),
            (Unmapped, "startup_tls_unmapped"), (Allocation, "startup_tls_allocation"),
            (TableAllocation, "startup_tls_table"), (ResolverAllocation, "startup_tls_resolver"),
            (Relocation, "startup_tls_relocation"), (Index, "startup_tls_index"),
            (Offset, "startup_tls_offset"), (Stack, "startup_tls_stack"),
            (Interpreter, "startup_tls_interpreter"),
        ].map(|(fault, label)| (Fault::StartupTls(fault), label,
            if fault == Stack { SpawnError::NoMemory } else { SpawnError::ElfLoadFailed }))
    };
    #[cfg(not(feature = "helios_dynamic_elf_tls_fixture"))]
    let startup_tls_cases = [];
    #[cfg(feature = "helios_runtime_c_startup")]
    let c_startup_cases = [
        (Fault::CStartupOverflow, "c_startup_overflow", SpawnError::NoMemory),
        (Fault::CStartupBeforeEntry, "c_startup_before_entry", SpawnError::NoMemory),
        (Fault::CStartupArgc, "c_startup_argc", SpawnError::ElfLoadFailed),
        (Fault::CStartupArgv, "c_startup_argv", SpawnError::ElfLoadFailed),
        (Fault::CStartupString, "c_startup_string", SpawnError::ElfLoadFailed),
        (Fault::CStartupEnvp, "c_startup_envp", SpawnError::ElfLoadFailed),
        (Fault::CStartupAuxv, "c_startup_auxv", SpawnError::ElfLoadFailed),
        (Fault::CStartupAlignment, "c_startup_alignment", SpawnError::ElfLoadFailed),
    ];
    #[cfg(not(feature = "helios_runtime_c_startup"))]
    let c_startup_cases = [];
    #[cfg(feature = "helios_runtime_c_dso")]
    let c_dso_cases = {
        use sunlight_elf::shared_fixture::TestFault::*;
        [
            (SharedFault::Metadata(UndefinedImport), "c_dso_import"),
            (SharedFault::Needed, "c_dso_needed"),
            (SharedFault::Metadata(RelocationType), "c_dso_relocation"),
            (SharedFault::Metadata(MissingExport), "c_dso_lookup"),
            (SharedFault::Symbol, "c_dso_after_mapping"),
            (SharedFault::Relocation, "c_dso_after_resolution"),
            (SharedFault::Stack, "c_dso_after_publication"),
        ].map(|(fault, label)| (Fault::Shared(fault), label,
            if fault == SharedFault::Stack { SpawnError::NoMemory } else { SpawnError::ElfLoadFailed }))
    };
    #[cfg(not(feature = "helios_runtime_c_dso"))]
    let c_dso_cases = [];
    for (index, (fault, label, expected)) in cases.into_iter()
        .chain(relocation_cases).chain(shared_cases).chain(real_cases).chain(graph_cases).chain(tls_cases)
        .chain(startup_tls_cases).chain(c_startup_cases).chain(c_dso_cases).enumerate()
    {
        let free = pmm.free_page_count();
        let tables = class_frame_count(PhysicalMemoryClass::PageTable);
        let owned = pmm.owned_frame_count(pid as u32);
        {
            #[cfg(feature = "helios_dynamic_elf_shared_fixture")]
            let fail_process = fault == Fault::Shared(SharedFault::Process);
            #[cfg(not(feature = "helios_dynamic_elf_shared_fixture"))]
            let fail_process = false;
            #[cfg(feature = "helios_dynamic_elf_tls_fixture")]
            let fail_process = fail_process || fault == Fault::Tls(super::elf_loader::handoff::tls::Fault::Process);
            if fail_process {
                pmm.fail_test_allocations_after(0);
            }
            let child = unsafe { Process::try_new(pid, 0, "handoff-rollback", &mut pmm, hhdm) };
            if fail_process {
                pmm.clear_test_allocation_failure();
                assert!(child.is_err(), "rollback fault {label}");
            } else {
            let mut child = child.expect("rollback test process");
            let bytes = if fault == Fault::Path {
                rejected.as_slice()
            } else {
                fixture
            };
            #[cfg(feature = "helios_dynamic_elf_relocation_fixture")]
            let bytes = if index >= cases.len() { relocation_fixture } else { bytes };
            #[cfg(feature = "helios_dynamic_elf_shared_fixture")]
            let bytes = if label.starts_with("graph_") {
                super::elf_loader::handoff::GRAPH_APP
            } else if label.starts_with("real_") {
                super::elf_loader::handoff::REAL_APP
            } else if matches!(fault, Fault::Shared(_)) {
                include_bytes!(concat!(env!("SUNLIGHT_DYNAMIC_PROBES"), "/elf-shared.sunapp/Program/probe"))
            } else { bytes };
            #[cfg(feature = "helios_dynamic_elf_tls_fixture")]
            let bytes = if matches!(fault, Fault::StartupTls(_)) {
                super::elf_loader::handoff::TLS_SHARED_APP
            } else { bytes };
            #[cfg(feature = "helios_dynamic_elf_tls_fixture")]
            let mut tls_bytes;
            #[cfg(feature = "helios_dynamic_elf_tls_fixture")]
            let bytes = if let Fault::Tls(tls_fault) = fault {
                use super::elf_loader::handoff::tls;
                tls_bytes = tls::APP.to_vec();
                tls::mutate(&mut tls_bytes, tls_fault);
                tls_bytes.as_slice()
            } else { bytes };
            #[cfg(feature = "helios_runtime_c_startup")]
            let bytes = if label.starts_with("c_startup_") {
                c_startup::IMAGE
            } else { bytes };
            #[cfg(feature = "helios_runtime_c_dso")]
            let bytes = if label.starts_with("c_dso_") {
                include_bytes!(concat!(env!("SUNLIGHT_DYNAMIC_PROBES"), "/elf-c-dso.sunapp/Program/probe"))
            } else { bytes };
            let argv: &[&[u8]] = &[if label.starts_with("startup_tls_") {
                b"/Applications/elf-tls-shared.sunapp/Program/probe"
            } else if label.starts_with("tls_") {
                b"/Applications/elf-tls.sunapp/Program/probe"
            } else if label.starts_with("graph_") {
                b"/Applications/elf-graph.sunapp/Program/probe"
            } else if label.starts_with("real_") {
                b"/Applications/elf-real.sunapp/Program/probe"
            } else if cfg!(feature = "helios_dynamic_elf_shared_fixture") && index >= cases.len() + relocation_cases.len() {
                b"/Applications/elf-shared.sunapp/Program/probe"
            } else { b"probe" }];
            let (argv, envp): (&[&[u8]], &[&[u8]]) = if label.starts_with("c_startup_") {
                (&[b"c-startup", b"P"], &[])
            } else if label.starts_with("c_dso_") {
                (&[b"/Applications/elf-c-dso.sunapp/Program/probe", b"P"], &[])
            } else {
                (argv, &[b"TEST=1"])
            };
            assert_eq!(
                exec_into_process_inner(
                    bytes,
                    &mut child,
                    &mut pmm,
                    hhdm,
                    argv,
                    envp,
                    false,
                    &mut || Ok(()),
                    Some(fault),
                ),
                Err(expected),
                "rollback fault {label}"
            );
            assert_eq!(
                child.address_space.region_count(),
                0,
                "rollback fault {label}"
            );
            if label.starts_with("c_startup_") || label.starts_with("c_dso_") {
                assert_eq!(child.fs_base, 0);
                assert!(child.proc_args.is_empty());
                assert_eq!(child.entry_point, 0);
            }
            drop(child);
            }
        }
        assert_eq!(pmm.free_page_count(), free, "rollback fault {label}");
        assert_eq!(
            class_frame_count(PhysicalMemoryClass::PageTable),
            tables,
            "rollback fault {label}"
        );
        assert_eq!(
            pmm.owned_frame_count(pid as u32),
            owned,
            "rollback fault {label}"
        );
        assert_eq!(
            scheduler.processes.len(),
            task_count,
            "rollback fault {label}"
        );
        assert!(
            scheduler.process_mut_by_pid(pid).is_none(),
            "rollback fault {label}"
        );
        let family = if index < cases.len() { "HELIOS-HANDOFF" } else { "HELIOS-RELOC" };
        #[cfg(feature = "helios_dynamic_elf_shared_fixture")]
        let family = if matches!(fault, Fault::Shared(_)) { "HELIOS-SHARED" } else { family };
        let family = if label.starts_with("graph_") { "HELIOS-GRAPH" } else { family };
        let family = if label.starts_with("tls_") { "HELIOS-TLS" } else { family };
        let family = if label.starts_with("startup_tls_") { "HELIOS-TLS-SHARED" } else { family };
        let family = if label.starts_with("c_startup_") { "C-STARTUP" } else { family };
        let family = if label.starts_with("c_dso_") { "C-DSO" } else { family };
        crate::serial_println!(
            "[{}] rollback verified case={} pid={} free={} tables={} owned={}",
            family,
            label,
            pid,
            free,
            tables,
            owned
        );
    }
}

pub fn map_user_stack(
    process: &mut Process,
    pmm: &mut PhysicalMemoryManager,
    hhdm_offset: VirtAddr,
) -> Result<(), SpawnError> {
    map_main_stack(process, pmm, hhdm_offset, None)
}

pub fn map_boot_user_stack(
    process: &mut Process,
    pmm: &mut PhysicalMemoryManager,
    hhdm_offset: VirtAddr,
) -> Result<(), SpawnError> {
    map_main_stack(process, pmm, hhdm_offset, cfg!(feature = "demand_main_stack").then_some(4))
}

fn native_stack_seed(argv: &[&[u8]], envp: &[&[u8]]) -> Result<usize, SpawnError> {
    let used = argv.iter().chain(envp).try_fold(16usize, |used, arg| {
        used.checked_add(arg.len())?.checked_add(1)
    }).ok_or(SpawnError::NoMemory)?;
    let words = argv.len().checked_add(envp.len()).and_then(|count| count.checked_add(7))
        .ok_or(SpawnError::NoMemory)?;
    let table = words.checked_mul(8).ok_or(SpawnError::NoMemory)?;
    let used = ((used.checked_add(7).ok_or(SpawnError::NoMemory)? & !7)
        .checked_add(table).and_then(|used| used.checked_add(15))
        .ok_or(SpawnError::NoMemory)?) & !15;
    if used >= super::layout::USER_STACK_SIZE as usize {
        return Err(SpawnError::NoMemory);
    }
    Ok(used.saturating_add(128).div_ceil(4096).clamp(4, 512))
}

fn map_main_stack(
    process: &mut Process,
    pmm: &mut PhysicalMemoryManager,
    hhdm_offset: VirtAddr,
    seed: Option<usize>,
) -> Result<(), SpawnError> {
    use crate::process::region::{
        MappingKind, MappingRegion, RegionBacking, RegionPolicy, RegionProtection,
    };

    let stack_pages = ((super::layout::USER_STACK_SIZE + 4095) / 4096) as usize;
    let stack_start = super::layout::USER_STACK_TOP
        .checked_sub(stack_pages as u64 * 4096)
        .ok_or(SpawnError::NoMemory)?;
    let space = &process.address_space;
    let identity = space.identity();
    let demand = seed.is_some();
    let policy = RegionPolicy::SYSTEM.union(RegionPolicy::OWNER_MANAGED);
    let mut guards = [None, None];
    if demand {
        if !identity.is_valid() || !process.owns_address_space {
            return Err(SpawnError::NoMemory);
        }
        for (index, address) in [stack_start - 4096, super::layout::USER_STACK_TOP].into_iter().enumerate() {
            let page = Page::containing_address(VirtAddr::new(address));
            let guard = MappingRegion::new(address, address + 4096, RegionProtection::NONE,
                MappingKind::UserStackGuard, policy.union(RegionPolicy::NO_MERGE), RegionBacking::None)
                .map_err(|_| SpawnError::NoMemory)?;
            if unsafe { space.is_occupied(page, hhdm_offset) } {
                return Err(SpawnError::NoMemory);
            }
            guards[index] = Some(space.preflight_region(guard).map_err(|_| SpawnError::NoMemory)?);
        }
    }
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
        policy,
        if demand {
            RegionBacking::MainStack { owner: process.pid as u32, generation: identity.generation }
        } else {
            RegionBacking::None
        },
    )
    .map_err(|_| SpawnError::NoMemory)?;
    let reservation = process
        .address_space
        .preflight_region(region)
        .map_err(|_| SpawnError::NoMemory)?;
    let flags = crate::process::address_space::AddressSpace::protection_to_pte_flags(protection)
        .map_err(|_| SpawnError::NoMemory)?;

    let committed = seed.unwrap_or(stack_pages);
    let stack_start = super::layout::USER_STACK_TOP - committed as u64 * 4096;
    if demand {
        let floor = super::layout::USER_STACK_TOP - super::layout::USER_STACK_SIZE;
        unsafe {
            space.prepare_transaction_tables(Page::containing_address(VirtAddr::new(floor)), pmm, hhdm_offset)
        }.map_err(|_| SpawnError::NoMemory)?;
    }
    let mut installed = 0usize;
    while installed < committed {
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
    for guard in guards.into_iter().flatten() {
        process.address_space.commit_region(guard).map_err(|_| SpawnError::NoMemory)?;
    }
    for _ in 0..installed {
        crate::memory::security::note_nx_stack_mapping();
    }
    Ok(())
}

pub(crate) fn demand_stack_owner(process: &Process, address: u64) -> Option<u32> {
    use super::region::{MappingKind, RegionBacking, RegionPolicy, RegionProtection};
    let top = super::layout::USER_STACK_TOP;
    let floor = top - super::layout::USER_STACK_SIZE;
    if !cfg!(feature = "demand_main_stack") || !(floor..top).contains(&address) {
        return None;
    }
    let identity = process.address_space.identity();
    let region = process.address_space.lookup_region(address)?;
    let RegionBacking::MainStack { owner, generation } = region.backing else {
        return None;
    };
    (identity.is_valid()
        && generation == identity.generation
        && region.start == floor
        && region.end == top
        && region.kind == MappingKind::UserStack
        && region.protection == RegionProtection::READ_WRITE
        && region.policy.contains(RegionPolicy::SYSTEM.union(RegionPolicy::OWNER_MANAGED))
        && (!process.owns_address_space || process.pid == owner as usize))
        .then_some(owner)
}

/// Caller serializes VM operations with SCHEDULER before PMM.
pub(crate) fn commit_demand_stack_page(
    process: &Process,
    address: u64,
    pmm: &mut PhysicalMemoryManager,
    hhdm: VirtAddr,
) -> Result<bool, SpawnError> {
    use x86_64::structures::paging::PageTableFlags as Flags;
    let Some(owner) = demand_stack_owner(process, address) else {
        return Ok(false);
    };
    let page = Page::containing_address(VirtAddr::new(address));
    let required = Flags::PRESENT | Flags::USER_ACCESSIBLE | Flags::WRITABLE | Flags::NO_EXECUTE;
    if let Some((frame, flags)) = unsafe { process.address_space.lookup_entry(page, hhdm) } {
        return Ok(flags.contains(required) && pmm.owner_of(frame) == Some(owner));
    }
    let addr = pmm.alloc_frame_owned(owner).ok_or(SpawnError::NoMemory)?;
    if !crate::memory::security::sanitize_user_frame(addr, hhdm) {
        pmm.free_frame(addr);
        return Err(SpawnError::NoMemory);
    }
    if unsafe { process.address_space.install_main_stack_leaf(page, addr, hhdm) }.is_err() {
        pmm.free_frame(addr);
        return Err(SpawnError::NoMemory);
    }
    crate::memory::security::note_nx_stack_mapping();
    Ok(true)
}

#[cfg(feature = "main_stack_test")]
pub(crate) fn test_main_stack(hhdm: VirtAddr) {
    use x86_64::structures::paging::PageTableFlags as Flags;
    let mut pmm = crate::PMM.lock();
    let baseline = pmm.free_page_count();
    let top = super::layout::USER_STACK_TOP;
    let floor = top - super::layout::USER_STACK_SIZE;
    {
        let mut process = unsafe { Process::new(65000, 0, "stack-1-test", &mut pmm, hhdm) };
        let large_argument = [b'x'; 17 * 1024];
        let argv: &[&[u8]] = &[b"stack-probe", &large_argument];
        let envp: &[&[u8]] = &[b"STACK=one"];
        let seed = native_stack_seed(argv, envp).unwrap();
        assert_eq!(seed, 5);
        map_main_stack(&mut process, &mut pmm, hhdm, Some(seed)).unwrap();
        let stack = setup_exec_stack(argv, envp, &[], &mut process, hhdm, floor, &mut || Ok(())).unwrap();
        assert_eq!(stack.rsp % 16, 0);
        let mut words = [0; 80];
        crate::memory::user::copy_from_process_bytes(&process, hhdm, stack.rsp, &mut words).unwrap();
        let word = |index: usize| u64::from_le_bytes(words[index * 8..(index + 1) * 8].try_into().unwrap());
        assert_eq!([word(0), word(3), word(5), word(6), word(8), word(9)], [2, 0, 0, 25, 0, 0]);
        assert_eq!(word(7), top - 16);
        for (pointer, expected) in [(word(1), b"stack-probe\0".as_slice()), (word(4), b"STACK=one\0".as_slice())] {
            let mut bytes = [0; 12];
            let actual = &mut bytes[..expected.len()];
            crate::memory::user::copy_from_process_bytes(&process, hhdm, pointer, actual).unwrap();
            assert_eq!(actual, expected);
        }
        let mut terminator = [1];
        crate::memory::user::copy_from_process_bytes(&process, hhdm, word(2) + large_argument.len() as u64, &mut terminator).unwrap();
        assert_eq!(terminator, [0]);
        let stats = unsafe { process.address_space.reclaim_user_space(&mut pmm, hhdm, true) };
        assert_eq!(stats.user_frames, seed);
        assert_eq!(pmm.free_page_count(), baseline);
    }
    for budget in 0..8 {
        let mut process = unsafe { Process::new(65000, 0, "stack-1-test", &mut pmm, hhdm) };
        pmm.fail_test_allocations_after(budget);
        assert_eq!(map_main_stack(&mut process, &mut pmm, hhdm, Some(4)), Err(SpawnError::NoMemory));
        pmm.clear_test_allocation_failure();
        unsafe { process.address_space.reclaim_user_space(&mut pmm, hhdm, true) };
        assert_eq!(pmm.free_page_count(), baseline);
    }
    for _ in 0..8 {
        let mut process = unsafe { Process::new(65000, 0, "stack-1-test", &mut pmm, hhdm) };
        map_main_stack(&mut process, &mut pmm, hhdm, Some(4)).unwrap();
        for address in [floor - 4096, top, floor - 8192] {
            assert_eq!(commit_demand_stack_page(&process, address, &mut pmm, hhdm), Ok(false));
        }
        for address in [floor - 4096, top] {
            let region = process.address_space.lookup_region(address).unwrap();
            assert_eq!(region.kind, super::region::MappingKind::UserStackGuard);
            assert!(!region.policy.contains(super::region::RegionPolicy::MAY_REPLACE));
        }
        let before = pmm.free_page_count();
        pmm.fail_test_allocations_after(0);
        assert_eq!(commit_demand_stack_page(&process, floor, &mut pmm, hhdm), Err(SpawnError::NoMemory));
        pmm.clear_test_allocation_failure();
        assert_eq!(pmm.free_page_count(), before);
        for address in [top - 5 * 4096, top - 6 * 4096, top - 180 * 4096, floor] {
            let before = pmm.free_page_count();
            assert_eq!(commit_demand_stack_page(&process, address, &mut pmm, hhdm), Ok(true));
            assert_eq!(commit_demand_stack_page(&process, address, &mut pmm, hhdm), Ok(true));
            assert_eq!(pmm.free_page_count(), before - 1);
            let page = Page::containing_address(VirtAddr::new(address));
            let (frame, flags) = unsafe { process.address_space.lookup_entry(page, hhdm) }.unwrap();
            assert!(flags.contains(Flags::PRESENT | Flags::USER_ACCESSIBLE | Flags::WRITABLE | Flags::NO_EXECUTE));
            assert_eq!(pmm.owner_of(frame), Some(65000));
            let bytes = unsafe { core::slice::from_raw_parts((hhdm + frame.as_u64()).as_ptr::<u8>(), 4096) };
            assert!(bytes.iter().all(|byte| *byte == 0));
        }
        let stats = unsafe { process.address_space.reclaim_user_space(&mut pmm, hhdm, true) };
        assert_eq!(stats.user_frames, 8);
        assert_eq!(pmm.free_page_count(), baseline);
    }
    crate::serial_println!("[STACK-1-TEST] startup ABI, rollback, bounds, zero, NX, duplicate, teardown PASS");
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
    stack_floor: u64,
    checkpoint: &mut impl FnMut() -> Result<(), SpawnError>,
) -> Result<ExecStack, SpawnError> {
    let stack_top = super::layout::USER_STACK_TOP;
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

    checkpoint()?;
    let mut argv_addrs = alloc::vec::Vec::new();
    argv_addrs
        .try_reserve_exact(argv.len())
        .map_err(|_| SpawnError::NoMemory)?;
    for arg in argv {
        argv_addrs.push(copy_string(&mut cursor, arg)?);
    }
    checkpoint()?;
    let mut envp_addrs = alloc::vec::Vec::new();
    envp_addrs
        .try_reserve_exact(envp.len())
        .map_err(|_| SpawnError::NoMemory)?;
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

    checkpoint()?;
    let mut table = alloc::vec::Vec::new();
    table
        .try_reserve_exact(table_words * 8)
        .map_err(|_| SpawnError::NoMemory)?;
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
    spawn_with_session_attachment(
        path,
        _argv,
        pmm,
        sched,
        _caps,
        hhdm_offset,
        uid,
        gid,
        env,
        service_lookup_restrictions,
        None,
        None,
    )
}

pub(crate) unsafe fn rollback_initial_image(
    process: &mut Process,
    pmm: &mut PhysicalMemoryManager,
    hhdm: VirtAddr,
) {
    for fd in 0..3 {
        let entry = process.fd_table.take(fd).expect("initial spawn descriptor");
        assert_eq!(entry.handle.0, fd as u32);
    }
    let _reclaimed = process.address_space.reclaim_user_space(pmm, hhdm, true);
    #[cfg(all(debug_assertions, feature = "vfs_l1c_r4_test"))]
    if crate::ipc::vfs_registration::spawn::fixture::tracks_child(process.pid) {
        crate::serial_println!("[VFS-R4] teardown child={} descriptors=3 frames={} tables={} swap={}",
            process.pid, _reclaimed.user_frames, _reclaimed.page_tables, _reclaimed.swap_blocks);
    }
}

#[cfg(all(debug_assertions, feature = "session_launch_test"))]
fn rollback_launch_fault(
    process: &mut Process,
    attachment: Option<&crate::capability::sessions::LaunchAttachment>,
    stage: u8,
    pmm: &mut PhysicalMemoryManager,
    hhdm: VirtAddr,
    free_before: usize,
) -> bool {
    if !attachment.is_some_and(|a| a.fails_at(stage)) {
        return false;
    }
    unsafe {
        rollback_initial_image(process, pmm, hhdm);
    }
    crate::serial_println!("[SL41] event=rollback stage={} pid={} incarnation={} free_before={} free_after={} published=0 runnable=0",
        stage, process.pid, process.foreground.incarnation, free_before, pmm.free_page_count());
    true
}

/// Session launch uses the existing scheduler/PMM/broker lock transaction.
/// The opaque attachment is consumed before publication, never after enqueue.
pub(crate) fn spawn_with_session_attachment(
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
    attachment: Option<crate::capability::sessions::LaunchAttachment>,
    parent_pid: Option<usize>,
) -> Result<usize, SpawnError> {
    sched
        .reserve_task_storage()
        .map_err(|_| SpawnError::NoMemory)?;
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
    let pid = sched.next_process_pid()
        .ok_or(SpawnError::PidExhausted)?;
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
    #[cfg(all(debug_assertions, feature = "session_launch_test"))]
    let free_before = pmm.free_page_count();
    let mut process = unsafe {
        Process::try_new(pid, parent_pid.unwrap_or(1), proc_name, pmm, hhdm_offset)
            .map_err(|_| SpawnError::NoMemory)?
    };
    #[cfg(all(debug_assertions, feature = "session_launch_test"))]
    if rollback_launch_fault(
        &mut process,
        attachment.as_ref(),
        1,
        pmm,
        hhdm_offset,
        free_before,
    ) {
        return Err(SpawnError::SessionRejected);
    }
    process.uid = uid;
    process.gid = gid;
    process.system_allocation = attachment.is_none() && uid == 0;
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
    if let Err(error) = exec_into_process(bytes, &mut process, pmm, hhdm_offset, &[], &envp, false)
    {
        unsafe {
            rollback_initial_image(&mut process, pmm, hhdm_offset);
        }
        return Err(error);
    }
    #[cfg(all(debug_assertions, feature = "session_launch_test"))]
    if rollback_launch_fault(
        &mut process,
        attachment.as_ref(),
        2,
        pmm,
        hhdm_offset,
        free_before,
    ) {
        return Err(SpawnError::SessionRejected);
    }
    process.nice = initial_embedded_nice(path, uid);
    process.trusted_display_service = is_trusted_display_path(path);
    process.trusted_swap_admin_service = is_trusted_swap_admin_path(path);
    process.trusted_zram_diagnostic = is_trusted_zram_diagnostic_path(path);
    process.trusted_pty_service = is_trusted_pty_service_path(path);
    process.trusted_lock_service = is_trusted_lock_service_path(path);
    // The KV credential syscall observes every caller's UID, so the marker is
    // granted only to a root-launched embedded sunlight-kv image.
    process.trusted_kv_service = uid == 0 && is_trusted_kv_service_path(path);
    process.trusted_session_service = is_trusted_session_service_path(path);
    process.service_identity = if uid == 0 {
        trusted_service_identity_for_path(path)
    } else {
        0
    };
    #[cfg(feature = "native_session_race_test")]
    if process.trusted_session_service && sched.online_cores > 1 {
        process.cpu_mask = 2;
    }
    process.trusted_wiseowl_braind = is_trusted_wiseowl_braind_path(path);
    process.trusted_wiseowl_console = is_trusted_wiseowl_console_path(path);
    process.trusted_control_panel = is_trusted_control_panel_path(path);
    // The account broker may resolve caller roles and owns `account-broker:*`.
    process.trusted_account_broker = uid == 0 && is_trusted_account_broker_path(path);
    #[cfg(all(debug_assertions, feature = "session_launch_test"))]
    if rollback_launch_fault(
        &mut process,
        attachment.as_ref(),
        3,
        pmm,
        hhdm_offset,
        free_before,
    ) {
        return Err(SpawnError::SessionRejected);
    }
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

    #[cfg(all(debug_assertions, feature = "session_launch_test"))]
    if rollback_launch_fault(
        &mut process,
        attachment.as_ref(),
        4,
        pmm,
        hhdm_offset,
        free_before,
    ) {
        return Err(SpawnError::SessionRejected);
    }
    if let Some(attachment) = attachment {
        // The caller retains the broker guard throughout construction. Thus
        // revoke/expiry cannot interleave between validation and publication.
        match _caps.commit_session_launch(attachment, crate::timekeeping::global_ticks()) {
            Ok(reference) => {
                process.account_membership = Some(reference);
                process.restrict_account_member_roles();
                // Only SessionD's embedded Vortex child gets this identity.
                // Inherited session launches do not authorize preview reads.
                if path == "/bin/sunlight-vortex-shell"
                    && sched.current_process().trusted_session_service
                {
                    process.service_identity = sunlight_ipc::service_identity::VORTEX_SHELL;
                }
            }
            Err(_) => {
                unsafe {
                    rollback_initial_image(&mut process, pmm, hhdm_offset);
                }
                return Err(SpawnError::SessionRejected);
            }
        }
    }
    #[cfg(all(debug_assertions, feature = "session_launch_test"))]
    if let Some(reference) = process.account_membership {
        crate::serial_println!(
            "[SL41] event=prepublication pid={} incarnation={} vm={} session={} epoch={} uid={}",
            process.pid,
            process.foreground.incarnation,
            process.address_space.identity().generation,
            reference.id,
            reference.epoch,
            process.uid
        );
    }
    // Use the authenticated launching task, never the bookkeeping ppid default.
    let parent_idx = sched
        .current_process_index()
        .map(|idx| sched.shared_process_index(idx));
    let source = parent_idx.and_then(|idx| crate::ipc::capture_vfs_inheritance(idx, sched, _caps));
    if let Some(idx) = parent_idx {
        process.vfs_stat_routed = sched.processes[idx].vfs_stat_routed;
        process.vfs_root_rebased = sched.processes[idx].vfs_root_rebased;
        if process.vfs_stat_routed {
            process.cwd = sched.processes[idx].cwd.clone();
        }
    }
    let actual_pid = process.pid;
    let idx = sched.add_process_after_reaping(process);
    crate::ipc::provision_vfs_inheritance(idx, sched, source);
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
        #[cfg(all(debug_assertions, feature = "vfs_l1c_parent_lifecycle_test"))]
        "/bin/vfs-lifecycle-parent" => Ok(include_bytes!(concat!(env!("OUT_DIR"), "/vfs-stat-caller"))),
        #[cfg(all(debug_assertions, feature = "vfs_d1b_test"))]
        "/bin/vfs-d1b-caller" => Ok(include_bytes!(concat!(env!("OUT_DIR"), "/vfs-stat-caller"))),
        #[cfg(feature = "vfs_stat_failure_test")]
        "/bin/vfs-stat-caller" => Ok(include_bytes!(
            "../../../target/x86_64-unknown-none/release/vfs-stat-caller"
        )),
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
        "/sbin/sunlight-account-broker" | "/usr/sbin/sunlight-account-broker" => {
            Ok(crate::SUNLIGHT_ACCOUNT_BROKER_ELF_BYTES)
        }
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
        #[cfg(feature = "cap_issuance_test")]
        "/test/exec-r2" => Ok(include_bytes!(concat!(env!("OUT_DIR"), "/exec-r2.elf"))),
        "/bin/capabilityctl" | "/usr/bin/capabilityctl" => Ok(crate::CAPABILITYCTL_ELF_BYTES),
        "/bin/runas" | "/usr/bin/runas" => Ok(crate::RUNAS_ELF_BYTES),
        #[cfg(all(debug_assertions, feature = "session_launch_test"))]
        "/test/launch-bad-elf" => Ok(b"not-an-elf"),
        #[cfg(all(debug_assertions, feature = "session_launch_test"))]
        "/bin/session-launch-probe" => Ok(include_bytes!(
            "../../../target/x86_64-unknown-none/release/session-race-probe"
        )),
        #[cfg(feature = "native_session_race_test")]
        "/tmp/session-race-probe" | "/bin/session-racer" => Ok(include_bytes!(
            "../../../target/x86_64-unknown-none/release/session-race-probe"
        )),
        #[cfg(all(debug_assertions, feature = "fastlane_contract_test"))]
        "/test/contract-system"
        | "/test/contract-audio"
        | "/test/contract-protected"
        | "/test/contract-required" => Ok(include_bytes!(
            "../../../target/x86_64-unknown-none/release/session-race-probe"
        )),
        "/usr/bin/top" | "/bin/top" => Ok(crate::SUNLIGHT_TOP_ELF_BYTES),
        "/usr/bin/memoryctl" | "/bin/memoryctl" => Ok(crate::MEMORYCTL_ELF_BYTES),
        "/usr/bin/sunlightctl" | "/bin/sunlightctl" => Ok(crate::SUNLIGHTCTL_ELF_BYTES),
        "/usr/bin/mezzoctl" | "/bin/mezzoctl" => Ok(crate::MEZZOCTL_ELF_BYTES),
        "/usr/bin/sunlight-sessionctl" | "/bin/sunlight-sessionctl" => {
            Ok(crate::SUNLIGHT_SESSIONCTL_ELF_BYTES)
        }
        // Session Configuration Phase 1 fixtures (SpawnRequest path must be ≤31 chars).
        #[cfg(feature = "dependency_test")]
        "/sbin/d3-parent" => Ok(crate::D3_PARENT_ELF_BYTES),
        #[cfg(feature = "dependency_test")]
        "/sbin/d3-leaf" => Ok(crate::D3_LEAF_ELF_BYTES),
        #[cfg(feature = "dependency_test")]
        "/sbin/d3-client" => Ok(crate::D3_CLIENT_ELF_BYTES),
        #[cfg(feature = "idle_test")]
        "/sbin/d2-dependent" => Ok(crate::D2_DEPENDENT_ELF_BYTES),
        #[cfg(feature = "idle_test")]
        "/sbin/d2-fixture" => Ok(crate::D2_FIXTURE_ELF_BYTES),
        #[cfg(feature = "idle_test")]
        "/sbin/d2-driver" => Ok(crate::D2_DRIVER_ELF_BYTES),
        #[cfg(feature = "demand_test")]
        "/sbin/d1-fixture" => Ok(crate::D1_FIXTURE_ELF_BYTES),
        #[cfg(feature = "demand_test")]
        "/sbin/d1-timeout" => Ok(crate::D1_TIMEOUT_ELF_BYTES),
        #[cfg(feature = "demand_test")]
        "/sbin/d1-crash" => Ok(crate::D1_CRASH_ELF_BYTES),
        #[cfg(feature = "demand_test")]
        "/sbin/d1-dependent" => Ok(crate::D1_DEPENDENT_ELF_BYTES),
        #[cfg(feature = "demand_test")]
        "/sbin/d1-driver" => Ok(crate::D1_DRIVER_ELF_BYTES),
        #[cfg(feature = "demand_test")]
        "/sbin/d1-denied" => Ok(crate::D1_DENIED_ELF_BYTES),
        #[cfg(feature = "demand_test")]
        "/sbin/d1-activate-only" => Ok(crate::D1_ACTIVATION_ONLY_ELF_BYTES),
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
        "/bin/sunlight-empires" | "/usr/bin/sunlight-empires" => {
            Ok(crate::SUNLIGHT_EMPIRES_ELF_BYTES)
        }
        // silicon-echoes: native graphical narrative-game vertical slice.
        "/bin/silicon-echoes" | "/usr/bin/silicon-echoes" => Ok(crate::SILICON_ECHOES_ELF_BYTES),
        // sunlight-files: native graphical file manager.
        "/bin/sunlight-files" | "/usr/bin/sunlight-files" => Ok(crate::SUNLIGHT_FILES_ELF_BYTES),
        // light-lens: native graphical image viewer.
        "/bin/light-lens" | "/usr/bin/light-lens" => Ok(crate::LIGHT_LENS_ELF_BYTES),
        "/bin/voice-recorder" | "/usr/bin/voice-recorder" => Ok(crate::VOICE_RECORDER_ELF_BYTES),
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
        #[cfg(feature = "document_service")]
        "/bin/sunlight-docserv" | "/usr/bin/sunlight-docserv" | "/sbin/sunlight-docserv" => {
            Ok(crate::SUNLIGHT_DOCSERV_ELF_BYTES)
        }
        "/bin/sunlight-docview" | "/usr/bin/sunlight-docview" => {
            Ok(crate::SUNLIGHT_DOCVIEW_ELF_BYTES)
        }
        #[cfg(feature = "document_postscript")]
        "/bin/gs" | "/usr/bin/gs" => Ok(crate::DOCUMENT_GHOSTSCRIPT_ELF_BYTES),
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
        "/bin/lsblk" | "/usr/bin/lsblk" => Ok(crate::LSBLK_ELF_BYTES),
        "/bin/storagectl" | "/usr/bin/storagectl" => Ok(crate::STORAGECTL_ELF_BYTES),
        "/bin/storage-unprivileged"
            if option_env!("SUNLIGHT_RUNTIME_STORAGE_TEST") == Some("1")
                || (cfg!(feature = "key_inject")
                    && option_env!("SUNLIGHT_INJECT_PHASE") == Some("storage-fstab")) =>
        {
            Ok(crate::STORAGECTL_ELF_BYTES)
        }
        "/bin/yazi" | "/usr/bin/yazi" => Ok(crate::YAZI_ELF_BYTES),
        #[cfg(feature = "key_inject")]
        "/bin/yazi-baseline" | "/usr/bin/yazi-baseline" => Ok(crate::YAZI_BASELINE_ELF_BYTES),
        "/bin/helios-probe" | "/usr/bin/helios-probe" => Ok(crate::HELIOS_PROBE_ELF_BYTES),
        #[cfg(feature = "sunjs_probe")]
        "/bin/sunjs" => Ok(crate::SUNJS_ELF_BYTES),
        #[cfg(feature = "native_std_probe")]
        "/bin/native-fp-probe" => Ok(crate::NATIVE_FP_PROBE_ELF_BYTES),
        "/bin/linux-thread-probe" | "/usr/bin/linux-thread-probe" => {
            Ok(crate::HELIOS_THREAD_PROBE_ELF_BYTES)
        }
        "/bin/linux-fionbio-pipe-probe" => Ok(crate::HELIOS_FIONBIO_PROBE_ELF_BYTES),
        "/bin/linux-recvfrom-probe" => Ok(crate::HELIOS_RECVFROM_PROBE_ELF_BYTES),
        "/bin/linux-open-largefile-probe" => Ok(crate::HELIOS_OPEN_LARGEFILE_PROBE_ELF_BYTES),
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
