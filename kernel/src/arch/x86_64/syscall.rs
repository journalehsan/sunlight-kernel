use crate::arch::x86_64::interrupts::now_ns;
use core::arch::naked_asm;
use sunlight_net as sunlight_ipc;
use x86_64::structures::paging::{Page, PageTableFlags, PhysFrame, Size4KiB};
use x86_64::{PhysAddr, VirtAddr};

/// Syscall numbers for SunlightOS
#[repr(u64)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SunlightSyscall {
    IpcCall = 1,
    IpcReply = 2,
    IpcReplyWait = 3,
    IpcRecv = 4,
    IpcNotifySend = 5,
    IpcNotifyWait = 6,
    IpcCancel = 7,
    IpcSetDeadline = 8,
    EndpointCreate = 10,
    EndpointBind = 11,
    ProcessExit = 20,
    ProcessYield = 21,
    ThreadSpawn = 22,
    /// TTY mux (foreground input routing). tty_server pushes keyboard bytes
    /// into a tab's kernel stdin ring; the foreground app reads them via fd0.
    /// rdi=tab, rsi=buf, rdx=len. Returns bytes accepted.
    TtyStdinPush = 23,
    /// tty_server drains a tab's kernel stdout ring (written by the app's fd1)
    /// to render. rdi=tab, rsi=buf, rdx=len. Returns bytes pulled.
    TtyStdoutPull = 24,
    /// Non-reaping liveness probe used by tty_server to detect when the
    /// foreground app exits. rdi=pid. Returns 1 if alive, 0 otherwise.
    ProcessIsAlive = 25,

    // Process management (Phase 4)
    Fork = 30,
    Exec = 31,
    Waitpid = 32,
    Getpid = 33,
    Getppid = 34,
    Getuid = 35,
    Getgid = 36,
    Setuid = 37,
    Setgid = 38,
    /// posix_spawn-style: create a new child process from an ELF path
    /// (Phase 6.5 Step 3). rdi=path, rsi=argv, rdx=fd to install as the
    /// child's stdout (u64::MAX for none).
    Spawn = 39,

    // File descriptor management
    Open = 40,
    Close = 41,
    Read = 42,
    Write = 43,
    Lseek = 44,
    Dup = 45,
    Dup2 = 46,
    Pipe = 47,
    Fstat = 48,
    Fcntl = 49,

    // Memory management (Phase 4.1)
    Mmap = 50,
    Munmap = 51,
    Mprotect = 52,
    Mremap = 53,

    // Kernel-VFS path syscalls (Phase 6.5 Step 3)
    ReadDir = 60,
    StatPath = 61,
    Mkdir = 62,
    Unlink = 65,
    Rename = 66,
    Chmod = 67,
    Chown = 68,

    // Signal handling (Phase 4.3)
    Sigaction = 70,
    Sigprocmask = 71,
    Kill = 72,
    Pause = 73,
    Sigreturn = 74,

    // Power management (Phase 5.11)
    PowerCtl = 80,
    SetNice = 83,
    GetNice = 84,
    /// Raw hardware seed entropy (one u64). RDRAND with a TSC-jitter fallback.
    /// Seed-grade only: userland/rand-service expands it with a CSPRNG.
    GetEntropy = 87,
    ClockGetTime = 88,
    NetInfo = 96,

    // Shared memory grant (Bite 4)
    ShmAlloc = 92,
    ShmMap = 93,
    ShmFree = 94,
    MapTelemetry = 95,
    GrantCapability = 100,
    SetFsBase = 101,

    // Keyboard driver (Ring 3 migration)
    KbdRegister = 110,
    KbdUnregister = 111,
    KbdPopScancode = 112,
    KbdGetStats = 113,

    // Mouse driver (Ring 3)
    MouseRegister = 114,
    MousePopByte = 115,
    MouseInit = 116,
    MousePortRead = 117,

    // GUI / Display (Phase 3+): allow the compositor to map the Limine physical framebuffer
    MapFramebuffer = 118,

    // VirtIO GPU proxy syscalls (Phase VirtIO-GPU). Only display_server (by process name) can call.
    /// Returns (width u32, height u32) from cached GET_DISPLAY_INFO, packed in rax and frame.r8.
    GpuGetInfo = 119,
    /// Walk back_buffer VA → phys pages and send RESOURCE_ATTACH_BACKING for the scanout resource.
    /// rdi = user VA of back_buffer start, rsi = number of 4KiB pages.
    GpuAttachBacking = 120,
    /// Send SET_SCANOUT to wire resource 1 to scanout 0.
    GpuSetScanout = 121,
    /// TRANSFER_TO_HOST_2D + RESOURCE_FLUSH for a dirty rect.
    /// rdi = x | (y << 32), rsi = w | (h << 32).
    GpuFlush = 122,
    /// Upload cursor pixels and issue UPDATE_CURSOR.
    /// rdi = user VA of 64×64 BGRA pixels, rsi = num_pixels (≤4096),
    /// rdx = hot_x | (hot_y << 32).
    GpuUpdateCursor = 123,
    /// MOVE_CURSOR to new position. rdi = x | (y << 32).
    GpuMoveCursor = 124,

    DebugLog = 99,
}

/// Setup SYSCALL/SYSRET MSRs once at boot.
/// SAFETY: Must be called exactly once before any user-space code runs.
pub unsafe fn setup_syscall_msrs(handler: VirtAddr) {
    let star_val: u64 = (0x001Bu64 << 48) | (0x0008u64 << 32);
    // SAFETY: MSRs are safe to write during early boot.
    unsafe {
        // Enable SYSCALL/SYSRET (EFER.SCE = bit 0).
        let efer = x86_64::registers::model_specific::Msr::new(0xC0000080).read();
        x86_64::registers::model_specific::Msr::new(0xC0000080).write(efer | 1);

        x86_64::registers::model_specific::Msr::new(0xC0000081).write(star_val);
        x86_64::registers::model_specific::Msr::new(0xC0000082).write(handler.as_u64());
        // Clear IF (bit 9) on syscall entry so interrupts are disabled in kernel.
        x86_64::registers::model_specific::Msr::new(0xC0000084).write(0x200); // 1 << 9
    }
    crate::serial_println!("[SYSCALL] LSTAR = {:#x}", handler.as_u64());
}

/// Raw syscall entry point (naked).
/// Saves all GPRs, calls dispatch, restores, sysretq.
#[unsafe(naked)]
pub unsafe extern "C" fn syscall_entry() {
    naked_asm!(
        // Disable interrupts for the duration of the syscall.
        "cli",
        // Build a full frame on the current stack (user's stack, valid in kernel mode via HHDM).
        // We must preserve all registers because sysretq only restores RIP and RFLAGS.
        "push r15",
        "push r14",
        "push r13",
        "push r12",
        "push r11",
        "push r10",
        "push r9",
        "push r8",
        "push rbp",
        "push rdi",
        "push rsi",
        "push rdx",
        "push rcx",
        "push rbx",
        "push rax",
        // Pass pointer to saved frame as first argument
        "mov rdi, rsp",
        "call syscall_dispatch",
        // rax now holds the return value. Store it into the rax slot on stack.
        "mov [rsp], rax",
        // Restore all GPRs
        "pop rax",
        "pop rbx",
        "pop rcx",
        "pop rdx",
        "pop rsi",
        "pop rdi",
        "pop rbp",
        "pop r8",
        "pop r9",
        "pop r10",
        "pop r11",
        "pop r12",
        "pop r13",
        "pop r14",
        "pop r15",
        // Ensure IF is set in R11 so user space returns with interrupts enabled.
        "or r11, 0x200",
        "sysretq",
    );
}

/// Saved register frame layout (matches push order in syscall_entry).
#[repr(C)]
pub struct SyscallRegs {
    pub rax: u64,
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rbp: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
}

pub type SyscallFrame = SyscallRegs;

enum SignalPostAction {
    Exit(i32),
}

/// Deliver pending signals before returning to user space
fn deliver_pending_signals(process: &mut crate::process::Process) -> Option<SignalPostAction> {
    use crate::process::signal::{SigHandler, Signal};

    // Check for pending signals (in priority order)
    let pending = process.signal_state.pending_signals();

    // Handle a few critical signals
    for sig_num in [2, 9, 15, 17].iter() {
        // SIGINT, SIGKILL, SIGTERM, SIGCHLD
        if let Some(sig) = Signal::try_from_u32(*sig_num) {
            if pending.contains(sig) && !process.signal_state.is_blocked(sig) {
                process.signal_state.clear_pending(sig);

                let action = process.signal_state.get_handler(sig);
                match action.handler {
                    SigHandler::Ignore => {
                        crate::serial_println!("[SIG] {} ignored", sig_num);
                    }
                    SigHandler::Default => {
                        crate::serial_println!("[SIG] {} delivered: terminating process", sig_num);
                        return Some(SignalPostAction::Exit(sig.default_exit_code()));
                    }
                    SigHandler::UserHandler(_handler_addr) => {
                        // Would need to setup signal frame on user stack
                        crate::serial_println!(
                            "[SIG] {} would call user handler at {:#x}",
                            sig_num,
                            _handler_addr
                        );
                        // TODO: Setup signal frame and jump to handler
                    }
                }
            }
        }
    }

    None
}

fn send_signal(pid: usize, signal: crate::process::signal::Signal) -> Result<(), ()> {
    use crate::process::signal::Signal;

    let mut sched = crate::sched::SCHEDULER.lock();
    let Some(idx) = sched.process_index_by_pid(pid) else {
        return Err(());
    };

    if matches!(
        sched.processes[idx].state,
        crate::process::ProcessState::Finished | crate::process::ProcessState::Reaped
    ) {
        return Err(());
    }

    let current_idx = sched.current_process_index().unwrap_or(usize::MAX);
    if idx == current_idx && matches!(signal, Signal::SIGKILL) {
        drop(sched);
        process_exit(signal.default_exit_code());
    }

    if matches!(signal, Signal::SIGKILL) {
        if !sched.terminate_process_by_pid(pid, signal.default_exit_code(), "signal(SIGKILL)") {
            return Err(());
        }
        return Ok(());
    }

    sched.processes[idx].signal_state.deliver_signal(signal);
    sched.wake_for_signal(pid);
    Ok(())
}

/// Syscall dispatch — called from assembly with pointer to saved frame.
/// Returns the value to put in RAX.
/// SAFETY: `frame` must point to a valid SyscallFrame on the stack.
#[no_mangle]
pub extern "C" fn syscall_dispatch(frame: &mut SyscallFrame) -> u64 {
    let mut num = frame.rax;

    // Phase 4.5: Check if this is a Linux-compat process and translate syscall
    crate::sched::with_scheduler(|sched| {
        if sched.current_process().is_linux_compat {
            // Translate Linux syscall number to SunlightOS number
            let linux_num = num as u64;
            match sunlight_compat_linux::translate_syscall(linux_num) {
                native_num if native_num >= 0 => {
                    num = native_num as u64;
                }
                -2 => {
                    if linux_num == 12 {
                        num = 1000; // Internal code for sys_brk
                    }
                }
                -3 => {
                    if linux_num == 158 {
                        num = 1001; // Internal code for sys_arch_prctl
                    }
                }
                -4 => {
                    if linux_num == 218 {
                        num = 1002; // Internal code for Linux set_tid_address
                    }
                }
                -5 => {
                    if linux_num == 273 {
                        num = 1003; // Internal code for Linux set_robust_list
                    }
                }
                -6 => {
                    if linux_num == 334 {
                        num = 1004; // Internal code for Linux rseq
                    }
                }
                -7 => {
                    if linux_num == 7 {
                        num = 1006; // Internal code for Linux poll
                    }
                }
                -8 => {
                    if linux_num == 13 {
                        num = 1007; // Internal code for Linux rt_sigaction
                    }
                }
                -9 => {
                    if linux_num == 14 {
                        num = 1008; // Internal code for Linux rt_sigprocmask
                    }
                }
                -10 => {
                    if linux_num == 200 {
                        num = 1009; // Internal code for Linux tkill
                    }
                }
                -11 => {
                    if linux_num == 9 {
                        num = 1010; // Internal code for Linux mmap
                    }
                }
                -12 => {
                    if linux_num == 131 {
                        num = 1011; // Internal code for Linux sigaltstack
                    }
                }
                -13 => {
                    if linux_num == 16 {
                        num = 1012; // Internal code for Linux ioctl
                    }
                }
                -14 => {
                    if linux_num == 20 {
                        num = 1013; // Internal code for Linux writev
                    }
                }
                -15 => {
                    if linux_num == 257 {
                        // openat(dirfd, path, flags, mode) → sys_open(path, flags)
                        // Rust std (and musl) prefer openat over open. Dirfd is
                        // almost always AT_FDCWD (-100); we ignore it and treat
                        // all paths as CWD-relative / absolute, matching our VFS.
                        frame.rdi = frame.rsi; // path pointer
                        frame.rsi = frame.rdx; // flags
                        num = 40; // sys_open
                    }
                }
                -16 => {
                    if linux_num == 318 {
                        num = 1014; // Internal code for Linux getrandom
                    }
                }
                -17 => {
                    if linux_num == 58 {
                        num = 1015; // Linux vfork containment
                    }
                }
                -18 => {
                    if linux_num == 56 {
                        num = 1016; // Linux clone containment
                    }
                }
                -38 => {
                    crate::serial_println!("[HELIOS] Unsupported Linux syscall {}", linux_num);
                    num = 1005; // Linux ENOSYS
                }
                _ => {
                    // Unknown or unsupported syscall
                    crate::serial_println!("[HELIOS] Unsupported Linux syscall {}", linux_num);
                    num = u64::MAX;
                }
            }
        }
    });

    let result = match num {
        1 => ipc_call(frame),
        2 => ipc_reply(frame),
        3 => ipc_reply_wait(frame),
        4 => ipc_recv(frame),
        5 => ipc_notify_send(frame.rdi),
        6 => ipc_notify_wait(frame.rdi),
        7 => ipc_cancel(),
        8 => ipc_set_deadline(frame.rdi),
        10 => endpoint_create(),
        11 => endpoint_bind(frame.rdi),
        12 => nameserver_endpoint_validate(frame.rdi, frame.rsi),
        13 => endpoint_destroy(frame.rdi),
        14 => nameserver_diagnostic_event(frame.rdi),
        20 => process_exit(frame.rdi as i32),
        21 => process_yield(),
        22 => thread_spawn(frame),
        23 => sys_tty_stdin_push(frame),
        24 => sys_tty_stdout_pull(frame),
        25 => sys_process_is_alive(frame),
        30 => sys_fork(frame),
        31 => sys_exec(frame),
        32 => sys_waitpid(frame),
        33 => sys_getpid(),
        34 => sys_getppid(frame),
        35 => sys_getuid(),
        36 => sys_getgid(),
        37 => sys_setuid(frame),
        38 => sys_setgid(frame),
        39 => sys_spawn(frame),
        40 => sys_open(frame),
        41 => sys_close(frame),
        42 => sys_read(frame),
        43 => sys_write(frame),
        44 => sys_lseek(frame),
        45 => sys_dup(frame),
        46 => sys_dup2(frame),
        47 => sys_pipe(frame),
        48 => sys_fstat(frame),
        49 => sys_fcntl(frame),
        50 => sys_mmap(frame),
        51 => sys_munmap(frame),
        52 => sys_mprotect(frame),
        53 => sys_mremap(frame),
        60 => sys_readdir(frame),
        61 => sys_stat_path(frame),
        62 => sys_mkdir(frame),
        63 => sys_chdir(frame),
        64 => sys_getcwd(frame),
        65 => sys_unlink(frame),
        66 => sys_rename(frame),
        67 => sys_chmod(frame),
        68 => sys_chown(frame),
        70 => sys_sigaction(frame),
        71 => sys_sigprocmask(frame),
        72 => sys_kill(frame),
        73 => sys_pause(),
        74 => sys_sigreturn(frame),
        80 => sys_powerctl(frame.rdi),
        81 => sys_get_time_utc(),
        86 => sys_monotonic_ms(),
        87 => sys_get_entropy(),
        88 => sys_clock_gettime(frame),
        82 => sys_sysinfo(frame),
        83 => sys_setnice(frame),
        84 => sys_getnice(frame),
        85 => sys_swapctl(frame),
        90 => sys_net_tx(frame),
        91 => sys_net_rx(frame),
        92 => sys_shm_alloc(frame),
        93 => sys_shm_map(frame),
        94 => sys_shm_free(frame),
        95 => sys_map_telemetry(frame),
        96 => sys_net_info(frame),
        100 => sys_grant_capability_syscall(frame),
        101 => sys_set_fs_base(frame),
        110 => sys_kbd_register(frame),
        111 => sys_kbd_unregister(),
        112 => sys_kbd_pop_scancode(),
        113 => sys_kbd_get_stats(frame),
        114 => sys_mouse_register(frame),
        115 => sys_mouse_pop_byte(),
        116 => sys_mouse_init(),
        117 => sys_mouse_port_read(frame),
        118 => sys_map_framebuffer(frame),
        119 => sys_gpu_get_info(frame),
        120 => sys_gpu_attach_backing(frame),
        121 => sys_gpu_set_scanout(frame),
        122 => sys_gpu_flush(frame),
        123 => sys_gpu_update_cursor(frame),
        124 => sys_gpu_move_cursor(frame),
        125 => sys_mouse_get_stats(frame),
        126 => sys_hardware_inventory(frame),
        1000 => sys_brk(frame),
        1001 => sys_arch_prctl(frame),
        1002 => sys_linux_set_tid_address(frame),
        1003 => sys_linux_set_robust_list(frame),
        1004 => sys_linux_rseq(frame),
        1005 => linux_errno(38),
        1006 => sys_linux_poll(frame),
        1007 => sys_linux_rt_sigaction(frame),
        1008 => sys_linux_rt_sigprocmask(frame),
        1009 => sys_linux_tkill(frame),
        1010 => sys_linux_mmap(frame),
        1011 => sys_linux_sigaltstack(frame),
        1012 => sys_linux_ioctl(frame),
        1013 => sys_linux_writev(frame),
        1014 => sys_linux_getrandom(frame),
        1015 => reject_linux_address_space_duplication("vfork"),
        1016 => sys_linux_clone_unsupported(frame),
        99 => debug_log(frame.rdi, frame.rsi),
        _ => {
            crate::serial_println!("[SYSCALL] Unknown syscall {}", num);
            u64::MAX
        }
    };

    // Deliver pending signals before returning to user space
    let post_signal =
        crate::sched::with_scheduler(|sched| deliver_pending_signals(sched.current_process_mut()));
    if let Some(SignalPostAction::Exit(code)) = post_signal {
        process_exit(code);
    }

    result
}

// ---------------------------------------------------------------------------
// Individual syscall implementations
// ---------------------------------------------------------------------------

use crate::capability::CapabilityRights;
use crate::capability::CapabilityToken;
use crate::ipc::{IpcError, IpcMsg, INIT_NAMESERVER_ENDPOINT};
use crate::process::ProcessState;
use crate::sched;
use alloc::vec::Vec;
use heapless::String;

const USER_PATH_MAX: usize = 256;
const USER_ARG_MAX: usize = 256;
const USER_ARG_COUNT_MAX: usize = 16;
const USER_ARG_TOTAL_MAX: usize = 4096;

fn read_user_cstr(
    ptr: u64,
    max_len: usize,
) -> Result<Vec<u8>, crate::memory::user::UserMemoryError> {
    crate::memory::user::read_c_string(ptr, max_len)
}

fn read_user_ptr_array(
    ptr: u64,
    max_entries: usize,
) -> Result<Vec<u64>, crate::memory::user::UserMemoryError> {
    crate::memory::user::read_pointer_array(ptr, max_entries)
}

fn user_memory_failure(error: crate::memory::user::UserMemoryError) -> u64 {
    let is_linux =
        crate::sched::with_scheduler(|scheduler| scheduler.current_process().is_linux_compat);
    user_memory_failure_for(is_linux, error)
}

fn user_memory_failure_for(is_linux: bool, error: crate::memory::user::UserMemoryError) -> u64 {
    if !is_linux {
        return u64::MAX;
    }
    match error {
        crate::memory::user::UserMemoryError::StringTooLong => linux_errno(36),
        crate::memory::user::UserMemoryError::ArrayTooLarge => linux_errno(7),
        _ => linux_errno(14),
    }
}

fn copy_from_user(address: u64, destination: &mut [u8]) -> Result<(), u64> {
    crate::memory::user::copy_from_current(address, destination).map_err(user_memory_failure)
}

fn copy_to_user(address: u64, source: &[u8]) -> Result<(), u64> {
    crate::memory::user::copy_to_current(address, source).map_err(user_memory_failure)
}

/// Reject IpcMsg whose counts exceed the register transport limits. Registers are
/// attacker-controlled, so `IpcMsg::from_registers` clamps these defensively — but
/// a clamp silently truncates instead of telling the caller their message was
/// forged/malformed. This catches the out-of-range case explicitly.
pub fn validate_ipc_msg(msg: &IpcMsg) -> Result<(), IpcError> {
    if msg.word_count as usize > crate::ipc::message::IPC_REG_WORDS {
        return Err(IpcError::InvalidWordCount);
    }
    if msg.cap_count as usize > crate::ipc::message::IPC_MAX_CAPS {
        return Err(IpcError::InvalidCapCount);
    }
    Ok(())
}

fn ipc_call(frame: &mut SyscallFrame) -> u64 {
    let token = CapabilityToken(frame.rsi);
    let msg = IpcMsg::from_registers(frame);

    if let Err(e) = validate_ipc_msg(&msg) {
        let mut sched = crate::sched::SCHEDULER.lock();
        let pid = sched.current_process().pid;
        crate::ipc::clear_next_ipc_deadline(pid, &mut sched);
        crate::serial_println!(
            "[IPC] WARN: invalid msg from pid={} word_count={} cap_count={}",
            pid,
            msg.word_count,
            msg.cap_count
        );
        return e as u64;
    }

    // Check for spawn capability (fast path handled by kernel)
    if token == crate::capability::SPAWN_TOKEN {
        let mut sched = crate::sched::SCHEDULER.lock();
        let pid = sched.current_process().pid;
        crate::ipc::clear_next_ipc_deadline(pid, &mut sched);
        drop(sched);
        return handle_spawn_call(frame, msg);
    }

    let mut sched = crate::sched::SCHEDULER.lock();
    let mut caps = crate::capability::CAP_BROKER.lock();
    let sender_pid = sched.current_process().pid;

    let (endpoint_id, target_owner) = caps
        .token_owner(token, CapabilityRights::SEND)
        .map_err(|_| IpcError::InvalidCapability as u64)
        .unwrap_or((0, 0));

    let result = crate::ipc::with_shard(endpoint_id, |bus| {
        match crate::ipc::handle_ipc_call(sender_pid, token, msg, &mut caps, &mut sched, bus) {
            Ok(reply) => {
                reply.to_registers(frame);
                0
            }
            Err(IpcError::WouldBlock) => {
                sched::request_reschedule();
                IpcError::WouldBlock as u64
            }
            Err(e) => e as u64,
        }
    });
    if result == IpcError::WouldBlock as u64
        && sched.processes.iter().any(|process| {
            process.pid == target_owner
                && process.state == ProcessState::BlockedOnIpc
                && process.ipc_endpoint == Some(endpoint_id)
        })
    {
        sched.wake_pid(target_owner);
    }
    result
}

/// Handle a spawn IPC call directly in the kernel.
/// Extracts path from the message words and spawns a new process.
fn handle_spawn_call(frame: &mut SyscallFrame, msg: IpcMsg) -> u64 {
    let path = decode_path_from_words(&msg.words);
    // NOTE: register IPC only transmits words[0..3] (see IpcMsg::from_registers),
    // so words[4]/[5] arrive as 0 here — uid/gid are effectively always 0 today.
    // The TTY tab is derived from shell_id (parsed from the transmitted path)
    // inside spawn_from_path, not passed as a word.
    let uid = msg.words[4] as u32;
    let gid = msg.words[5] as u32;

    let mut sched = crate::sched::SCHEDULER.lock();
    crate::serial_println!(
        "[SPAWN] Request from pid={} for path={} uid={} gid={}",
        sched.current_process().pid,
        path,
        uid,
        gid
    );

    let mut pmm = crate::PMM.lock();
    let mut caps = crate::capability::CAP_BROKER.lock();
    let hhdm = crate::HHDM_REQ.response().expect("no hhdm").offset;

    match crate::process::spawn::spawn_from_path(
        &path,
        &[],
        &mut *pmm,
        &mut *sched,
        &mut *caps,
        VirtAddr::new(hhdm),
        uid,
        gid,
    ) {
        Ok(pid) => {
            let mut reply = IpcMsg::with_label(crate::ipc::SpawnMsg::REPLY);
            reply.words[0] = pid as u64;
            reply.to_registers(frame);
            0
        }
        Err(e) => {
            crate::serial_println!("[SPAWN] Failed: {:?}", e);
            let mut reply = IpcMsg::with_label(crate::ipc::SpawnMsg::ERROR);
            reply.words[0] = e as u64;
            reply.to_registers(frame);
            0
        }
    }
}

/// Decode a path from the first 4 IPC words (32 bytes max).
fn decode_path_from_words(words: &[u64; 8]) -> alloc::string::String {
    let mut bytes = [0u8; 32];
    for i in 0..4 {
        bytes[i * 8..i * 8 + 8].copy_from_slice(&words[i].to_le_bytes());
    }
    let len = bytes.iter().position(|&b| b == 0).unwrap_or(32);
    // SAFETY: path bytes are ASCII from the caller.
    unsafe { alloc::string::String::from_utf8_unchecked(bytes[..len].to_vec()) }
}

fn ipc_reply(frame: &mut SyscallFrame) -> u64 {
    let reply = IpcMsg::from_registers(frame);
    let mut sched = crate::sched::SCHEDULER.lock();
    let server_pid = sched.current_process().pid;
    let endpoint_id = sched
        .current_process()
        .ipc_reply_target
        .map(|target| target.endpoint_id)
        .unwrap_or(0);
    crate::ipc::with_shard(endpoint_id, |bus| {
        match crate::ipc::handle_ipc_reply(server_pid, reply, &mut sched, bus) {
            Ok(()) => 0,
            Err(e) => e as u64,
        }
    })
}

fn ipc_reply_wait(frame: &mut SyscallFrame) -> u64 {
    let endpoint_token = CapabilityToken(frame.rsi);
    let reply = IpcMsg::from_registers(frame);
    let mut sched = crate::sched::SCHEDULER.lock();
    let caps = crate::capability::CAP_BROKER.lock();
    let server_pid = sched.current_process().pid;
    let endpoint_id = match caps.token_owner(endpoint_token, CapabilityRights::RECV_ONLY) {
        Ok((id, owner_pid)) if owner_pid == server_pid => id,
        _ => {
            if caps
                .check(endpoint_token, CapabilityRights::SEND_ONLY)
                .is_ok()
            {
                crate::ipc::note_send_only_management_reject();
            }
            return IpcError::InvalidCapability as u64;
        }
    };
    crate::ipc::with_shard(endpoint_id, |bus| {
        match crate::ipc::handle_ipc_reply_wait(server_pid, endpoint_id, reply, &mut sched, bus) {
            Ok(next) => {
                next.to_registers(frame);
                0
            }
            Err(IpcError::WouldBlock) => {
                sched::request_reschedule();
                IpcError::WouldBlock as u64
            }
            Err(e) => e as u64,
        }
    })
}

fn ipc_recv(frame: &mut SyscallFrame) -> u64 {
    let endpoint_token = CapabilityToken(frame.rsi);
    let mut sched = crate::sched::SCHEDULER.lock();
    let caps = crate::capability::CAP_BROKER.lock();
    let receiver_pid = sched.current_process().pid;
    let endpoint_id = match caps.token_owner(endpoint_token, CapabilityRights::RECV_ONLY) {
        Ok((id, owner_pid)) if owner_pid == receiver_pid => id,
        _ => {
            if caps
                .check(endpoint_token, CapabilityRights::SEND_ONLY)
                .is_ok()
            {
                crate::ipc::note_send_only_management_reject();
            }
            crate::ipc::clear_next_ipc_deadline(receiver_pid, &mut sched);
            return IpcError::InvalidCapability as u64;
        }
    };
    crate::ipc::with_shard(endpoint_id, |bus| {
        match crate::ipc::handle_ipc_recv(receiver_pid, endpoint_id, &mut sched, bus) {
            Ok(msg) => {
                msg.to_registers(frame);
                0
            }
            Err(IpcError::WouldBlock) => {
                sched::request_reschedule();
                IpcError::WouldBlock as u64
            }
            Err(e) => e as u64,
        }
    })
}

fn ipc_notify_send(_token: u64) -> u64 {
    0
}

fn ipc_notify_wait(_endpoint_token: u64) -> u64 {
    sched::with_scheduler(|s| {
        // Phase 1 + 4: charge + penalize quick block/yield patterns
        s.account_and_apply_churn_penalty();
        s.current_process_mut().state = ProcessState::BlockedOnIpc;
        s.current_process_mut().block_start_tick = s.global_tick;
    });
    sched::request_reschedule();
    IpcError::WouldBlock as u64
}

fn ipc_cancel() -> u64 {
    let mut sched = crate::sched::SCHEDULER.lock();
    let caller_pid = sched.current_process().pid;

    let endpoint_id = sched.current_process().ipc_endpoint.unwrap_or(0);

    crate::ipc::with_shard(endpoint_id, |bus| {
        match crate::ipc::handle_ipc_cancel(caller_pid, &mut sched, bus) {
            Ok(()) => 0,
            Err(e) => e as u64,
        }
    })
}

fn ipc_set_deadline(absolute_deadline_ms: u64) -> u64 {
    let deadline_tick = absolute_deadline_ms
        .saturating_mul(crate::timekeeping::TICK_HZ)
        .saturating_add(999)
        / 1000;
    let mut sched = crate::sched::SCHEDULER.lock();
    let caller_pid = sched.current_process().pid;
    match crate::ipc::arm_ipc_deadline(caller_pid, deadline_tick, &mut sched) {
        Ok(()) => 0,
        Err(error) => error as u64,
    }
}

fn endpoint_create() -> u64 {
    let pid = sched::with_scheduler(|s| s.current_process().pid);
    let (_endpoint_id, token) = {
        let mut caps = crate::capability::CAP_BROKER.lock();
        caps.create_endpoint(pid)
    };
    token.0
}

fn endpoint_bind(token: u64) -> u64 {
    if token == INIT_NAMESERVER_ENDPOINT as u64 {
        let mut caps = crate::capability::CAP_BROKER.lock();
        let Some(owner) = caps.token_for_owner_endpoint(1, CapabilityRights::SEND_RECV) else {
            return 0;
        };
        return caps
            .derive(owner, CapabilityRights::SEND_ONLY)
            .map_or(0, |cap| cap.0);
    }
    let caps = crate::capability::CAP_BROKER.lock();
    if caps
        .check(CapabilityToken(token), CapabilityRights::SEND_ONLY)
        .is_ok()
    {
        crate::ipc::note_send_only_management_reject();
    }
    0
}

/// PID 1-only liveness query used for lazy nameserver registry cleanup.
fn nameserver_endpoint_validate(token: u64, endpoint_id: u64) -> u64 {
    let caller_pid = sched::with_scheduler(|sched| sched.current_process().pid);
    if caller_pid != 1 || endpoint_id > u32::MAX as u64 {
        return 0;
    }
    let caps = crate::capability::CAP_BROKER.lock();
    u64::from(caps.endpoint_is_live(CapabilityToken(token), endpoint_id as u32))
}

fn nameserver_diagnostic_event(event: u64) -> u64 {
    let caller_pid = sched::with_scheduler(|sched| sched.current_process().pid);
    if caller_pid != 1 {
        return IpcError::InvalidCapability as u64;
    }
    crate::ipc::note_nameserver_diagnostic(event);
    0
}

fn endpoint_destroy(token: u64) -> u64 {
    let mut sched = crate::sched::SCHEDULER.lock();
    let caller_pid = sched.current_process().pid;
    let endpoint_id = {
        let mut caps = crate::capability::CAP_BROKER.lock();
        match caps.destroy_endpoint(caller_pid, CapabilityToken(token)) {
            Ok(endpoint_id) => endpoint_id,
            Err(_) => {
                if caps
                    .check(CapabilityToken(token), CapabilityRights::SEND_ONLY)
                    .is_ok()
                {
                    crate::ipc::note_send_only_management_reject();
                }
                return IpcError::InvalidCapability as u64;
            }
        }
    };
    crate::arch::x86_64::keyboard::unregister_kbd_endpoint(endpoint_id);
    crate::arch::x86_64::mouse::unregister_mouse_endpoint(endpoint_id);
    let calls = crate::ipc::with_shard(endpoint_id, |bus| bus.remove_endpoint(endpoint_id));
    crate::ipc::finish_peer_closed_calls(endpoint_id, calls, &mut sched);
    0
}

/// Syscall: ProcessExit
/// rdi = exit code
fn process_exit(code: i32) -> ! {
    let kstack_top = sched::finish_current_process(code, "exit");
    sched::request_reschedule();

    // We currently run on the process's *user* stack (syscall_entry builds
    // its frame there). The timer IRQ that switches away will keep using
    // this stack across the CR3 switch, where it is no longer mapped. Pivot
    // to the process's kernel stack (kernel heap — mapped in every address
    // space), then re-enable interrupts (syscall_entry ran `cli`) and wait
    // to be switched away from; this context is never resumed.
    unsafe {
        if kstack_top != 0 {
            core::arch::asm!("mov rsp, {}", in(reg) kstack_top);
        }
        core::arch::asm!("sti", "2:", "hlt", "jmp 2b", options(noreturn),);
    }
}

/// Syscall: ProcessYield
fn process_yield() -> u64 {
    sched::with_scheduler(|s| {
        // Phase 1 + Phase 4: account + penalize extremely short voluntary yields
        s.account_and_apply_churn_penalty();
        if s.current_process().state == ProcessState::Running {
            s.current_process_mut().state = ProcessState::Ready;
        }
        // The timer path requeues the yielded task on its current core before
        // picking another task, preserving queue ownership for both RR and BORE.
        if crate::sched::SCHEDULER_MODE == crate::sched::SchedulerMode::Bore {
            // current is still the index; enqueue_once is idempotent and state-checked.
            // We don't have idx here easily, but enqueue via scheduler logic will happen on pick.
            // To be safe we can request reschedule; timer path handles queue hygiene.
        }
    });
    sched::request_reschedule();
    0
}

/// Syscall: TtyStdinPush (23). tty_server forwards keyboard bytes to the
/// foreground app's stdin ring. rdi=tab, rsi=buf, rdx=len. Returns bytes pushed.
fn sys_tty_stdin_push(frame: &mut SyscallFrame) -> u64 {
    let tab = frame.rdi as usize;
    let len = (frame.rdx as usize).min(256);
    if len == 0 {
        return 0;
    }
    let mut kbuf = [0u8; 256];
    if let Err(error) = copy_from_user(frame.rsi, &mut kbuf[..len]) {
        return error;
    }
    crate::process::tty_io::push_stdin(tab, &kbuf[..len]) as u64
}

/// Syscall: TtyStdoutPull (24). tty_server drains the foreground app's stdout
/// ring to render it. rdi=tab, rsi=buf, rdx=len. Returns bytes pulled.
fn sys_tty_stdout_pull(frame: &mut SyscallFrame) -> u64 {
    let tab = frame.rdi as usize;
    let len = (frame.rdx as usize).min(4096);
    if len == 0 {
        return 0;
    }
    let mut kbuf = [0u8; 4096];
    let n = crate::process::tty_io::pull_stdout(tab, &mut kbuf[..len]);
    if n > 0 {
        if let Err(error) = copy_to_user(frame.rsi, &kbuf[..n]) {
            return error;
        }
    }
    n as u64
}

/// Syscall: ProcessIsAlive (25). Non-reaping liveness probe. rdi=pid.
/// Returns 1 if a process with that pid exists and is not Finished, else 0.
fn sys_process_is_alive(frame: &mut SyscallFrame) -> u64 {
    let pid = frame.rdi as usize;
    let sched = crate::sched::SCHEDULER.lock();
    let alive = sched
        .processes
        .iter()
        .any(|p| p.pid == pid && !matches!(p.state, ProcessState::Finished | ProcessState::Reaped));
    alive as u64
}

/// Syscall: ThreadSpawn (22)
/// rdi = trampoline entry point (userland thread_trampoline fn ptr)
/// rsi = aligned top of the thread's user stack
/// rdx = TLS base pointer (written to the new thread's FS_BASE MSR)
///
/// The two u64 values at [rsi+0] and [rsi+8] must be the actual function
/// pointer and argument; the kernel reads them and loads them into
/// RDI/RSI of the new thread's initial context so `thread_trampoline`
/// receives them as normal C arguments.
fn thread_spawn(frame: &mut SyscallFrame) -> u64 {
    let current_cpu = crate::sched::current_cpu_id();
    let trampoline = frame.rdi;
    let user_stack_top = frame.rsi;
    let tls_ptr = frame.rdx;

    if crate::memory::user::UserAddress::new(trampoline).is_err()
        || crate::memory::user::UserAddress::new(user_stack_top).is_err()
        || crate::memory::user::UserAddress::new(tls_ptr).is_err()
    {
        return u64::MAX;
    }
    let mut startup = [0u8; 16];
    if let Err(error) = copy_from_user(user_stack_top, &mut startup) {
        return error;
    }
    let func = u64::from_ne_bytes(startup[..8].try_into().unwrap());
    let arg = u64::from_ne_bytes(startup[8..].try_into().unwrap());

    if crate::memory::user::UserAddress::new(func).is_err() {
        crate::serial_println!("[SYSCALL] thread_spawn: func not in user space");
        return u64::MAX;
    }

    let mut sched = crate::sched::SCHEDULER.lock();

    // Collect everything we need from the parent in one borrow scope so
    // we can release it before calling clone_boxed (which also borrows sched).
    let (
        parent_pid,
        shared_address_space,
        uid,
        gid,
        nice,
        env,
        caps,
        tty_tab,
        parent_name,
        parent_cwd,
    ) = {
        let p = sched.current_process();
        (
            p.pid,
            p.address_space.shared_handle(),
            p.uid,
            p.gid,
            p.nice,
            crate::process::env::EnvMap::inherit(&p.env),
            p.capabilities.clone(),
            p.tty_tab,
            p.name,
            p.cwd.clone(),
        )
    };
    let thread_fd = sched.current_process().fd_table.clone_boxed();

    let new_tid = sched.processes.iter().map(|p| p.pid).max().unwrap_or(0) + 1;

    let name_len = parent_name.iter().position(|&b| b == 0).unwrap_or(32);
    let name_str = core::str::from_utf8(&parent_name[..name_len]).unwrap_or("thread");

    let mut thread = crate::process::Process::new_thread(
        new_tid,
        parent_pid,
        name_str,
        shared_address_space,
        thread_fd,
        env,
        uid,
        gid,
        nice,
        caps,
        tty_tab,
    );

    thread.native_thread = true;
    thread.cwd = parent_cwd;
    // Set up the iretq frame: RIP = trampoline, RSP = user_stack_top,
    // then override RDI/RSI so the trampoline receives func and arg.
    thread.init_context(trampoline, user_stack_top);
    thread.set_initial_args(func, arg, 0, 0);
    thread.fs_base = tls_ptr;

    let idx = sched.add_process(thread);
    sched.enqueue_ready_on_cpu(idx, current_cpu);
    crate::sched::note_native_borrower_created();

    crate::serial_println!(
        "[SYSCALL] thread_spawn: parent={} tid={} trampoline={:#x}",
        parent_pid,
        new_tid,
        trampoline
    );
    new_tid as u64
}

/// Syscall: DebugLog
/// rdi = pointer to string in user space
/// rsi = length
fn debug_log(ptr: u64, len: u64) -> u64 {
    if ptr == 0 || len == 0 {
        return 0;
    }
    let mut bytes = [0u8; 256];
    let copy_len = (len as usize).min(bytes.len());
    if copy_from_user(ptr, &mut bytes[..copy_len]).is_err() {
        return IpcError::InvalidArgument as u64;
    }

    // Print valid UTF-8 prefix
    if let Ok(s) = core::str::from_utf8(&bytes[..copy_len]) {
        crate::serial_println!("{}", s);
    } else {
        crate::serial_println!("[SYSCALL] DebugLog: invalid UTF-8");
    }
    0
}

// ---------------------------------------------------------------------------
// Phase 4: Process management syscalls
// ---------------------------------------------------------------------------

/// Syscall: Fork (30)
/// Returns: child_pid (parent), 0 (child)
fn sys_fork(_frame: &mut SyscallFrame) -> u64 {
    crate::memory::security::note_unsafe_fork_rejected();
    if crate::sched::with_scheduler(|sched| sched.current_process().is_linux_compat) {
        linux_errno(38)
    } else {
        u64::MAX
    }
}

fn reject_linux_address_space_duplication(operation: &str) -> u64 {
    crate::memory::security::note_unsafe_fork_rejected();
    crate::serial_println!("[MM-0] rejected unsafe Linux {}", operation);
    linux_errno(38)
}

fn sys_linux_clone_unsupported(frame: &SyscallFrame) -> u64 {
    const CLONE_VM: u64 = 0x0000_0100;
    let operation = if frame.rdi & CLONE_VM != 0 {
        "clone(CLONE_VM)"
    } else {
        "clone(process)"
    };
    reject_linux_address_space_duplication(operation)
}

/// Syscall: Exec (31)
/// rdi = path pointer (C string)
/// rsi = argv pointer (array of *const u8, NULL-terminated)
/// rdx = envp pointer (array of *const u8, NULL-terminated)
fn sys_exec(frame: &mut SyscallFrame) -> u64 {
    let path_ptr = frame.rdi;
    let argv_ptr = frame.rsi;
    let envp_ptr = frame.rdx;

    // Read path from user space
    let path_bytes = match read_user_cstr(path_ptr, USER_PATH_MAX) {
        Ok(bytes) => bytes,
        Err(error) => {
            crate::serial_println!("[SYSCALL] exec: bad path pointer");
            return user_memory_failure(error);
        }
    };

    let path_str = match core::str::from_utf8(&path_bytes) {
        Ok(s) => s,
        Err(_) => {
            crate::serial_println!("[SYSCALL] exec: invalid UTF-8 path");
            return u64::MAX;
        }
    };

    // Read argv from user space
    let argv_ptrs = match read_user_ptr_array(argv_ptr, USER_ARG_COUNT_MAX) {
        Ok(pointers) => pointers,
        Err(error) => {
            crate::serial_println!("[SYSCALL] exec: bad argv pointer");
            return user_memory_failure(error);
        }
    };

    let mut argv_bytes = alloc::vec::Vec::new();
    let mut total_arg_bytes = 0usize;
    for &arg_ptr in &argv_ptrs {
        match read_user_cstr(arg_ptr, USER_ARG_MAX) {
            Ok(bytes) => {
                total_arg_bytes = match total_arg_bytes.checked_add(bytes.len() + 1) {
                    Some(total) if total <= USER_ARG_TOTAL_MAX => total,
                    _ => {
                        return user_memory_failure(
                            crate::memory::user::UserMemoryError::ArrayTooLarge,
                        )
                    }
                };
                argv_bytes.push(bytes);
            }
            Err(error) => {
                crate::serial_println!("[SYSCALL] exec: bad argv[{}] pointer", argv_bytes.len());
                return user_memory_failure(error);
            }
        }
    }

    // Read envp from user space; NULL means "inherit my environment".
    let mut envp_bytes = alloc::vec::Vec::new();
    if envp_ptr != 0 {
        let envp_ptrs = match read_user_ptr_array(envp_ptr, USER_ARG_COUNT_MAX) {
            Ok(pointers) => pointers,
            Err(error) => {
                crate::serial_println!("[SYSCALL] exec: bad envp pointer");
                return user_memory_failure(error);
            }
        };
        for &env_ptr in &envp_ptrs {
            match read_user_cstr(env_ptr, USER_ARG_MAX) {
                Ok(bytes) => {
                    total_arg_bytes = match total_arg_bytes.checked_add(bytes.len() + 1) {
                        Some(total) if total <= USER_ARG_TOTAL_MAX => total,
                        _ => {
                            return user_memory_failure(
                                crate::memory::user::UserMemoryError::ArrayTooLarge,
                            )
                        }
                    };
                    envp_bytes.push(bytes);
                }
                Err(error) => {
                    crate::serial_println!(
                        "[SYSCALL] exec: bad envp[{}] pointer",
                        envp_bytes.len()
                    );
                    return user_memory_failure(error);
                }
            }
        }
    }

    crate::serial_println!(
        "[SYSCALL] exec path={}, argc={}, envc={}",
        path_str,
        argv_bytes.len(),
        envp_bytes.len()
    );

    // Resolve the binary: embedded images first (boot servers, /bin/sshl,
    // the multi-call utils), then the kernel VFS (Phase 6.5 Step 3).
    let vfs_bytes;
    let bytes: &[u8] = match crate::process::spawn::embedded_bytes_for_path(path_str) {
        Ok(b) => b,
        Err(_) => match vfs_read_file(path_str) {
            Some(v) => {
                vfs_bytes = v;
                &vfs_bytes
            }
            None => {
                crate::serial_println!("[SYSCALL] exec: path not found: {}", path_str);
                return u64::MAX;
            }
        },
    };

    // Validate before exec_into_process tears down the current image, so a
    // non-ELF file (e.g. `exec /etc/passwd`) fails cleanly.
    if sunlight_elf::parse_elf_header(bytes).is_err() {
        crate::serial_println!("[SYSCALL] exec: not a valid ELF: {}", path_str);
        return u64::MAX;
    }

    let mut sched = crate::sched::SCHEDULER.lock();
    if sched.current_address_space_has_borrowers() {
        crate::serial_println!("[MM-0] exec rejected while address-space borrowers are live");
        return if sched.current_process().is_linux_compat {
            linux_errno(38)
        } else {
            u64::MAX
        };
    }
    let mut pmm = crate::PMM.lock();
    let hhdm = crate::HHDM_REQ.response().expect("no hhdm").offset;

    let process = sched.current_process_mut();
    let argv_refs: alloc::vec::Vec<&[u8]> = argv_bytes.iter().map(|v| v.as_slice()).collect();

    // No explicit environment: the new image inherits this process's EnvMap.
    let inherited_env;
    let envp_refs: alloc::vec::Vec<&[u8]> = if envp_bytes.is_empty() {
        inherited_env = process.env.to_envp();
        inherited_env.iter().map(|s| s.as_bytes()).collect()
    } else {
        envp_bytes.iter().map(|v| v.as_slice()).collect()
    };

    match crate::process::spawn::exec_into_process(
        bytes,
        process,
        &mut *pmm,
        VirtAddr::new(hhdm),
        &argv_refs,
        &envp_refs,
        true,
    ) {
        Ok(entry) => {
            process.trusted_display_service =
                crate::process::spawn::is_trusted_display_path(path_str)
                    && crate::process::spawn::embedded_bytes_for_path(path_str).is_ok();
            process.trusted_swap_admin_service =
                crate::process::spawn::is_trusted_swap_admin_path(path_str)
                    && crate::process::spawn::embedded_bytes_for_path(path_str).is_ok();
            process.trusted_zram_diagnostic =
                crate::process::spawn::is_trusted_zram_diagnostic_path(path_str)
                    && crate::process::spawn::embedded_bytes_for_path(path_str).is_ok();
            crate::serial_println!("[SYSCALL] exec: success, entry={:#x}", entry);
            // Request immediate reschedule so the next timer tick switches context
            crate::sched::request_reschedule();
            // Return 0; the actual context switch will happen via timer interrupt
            // and the next time this process runs, it will be at the new entry point
            0
        }
        Err(e) => {
            crate::serial_println!("[SYSCALL] exec: failed with error {:?}", e);
            u64::MAX
        }
    }
}

/// Syscall: Waitpid (32)
/// rdi = child pid. Returns the exit code (0..=255) once the child is Finished,
/// EAGAIN while it is still running, and -1 when no such child exists.
///
/// While the child is still running, the caller is parked in `BlockedOnIpc`
/// (recording the awaited child in `wait_child`) so it does NOT busy-spin in
/// the scheduler's ready queue. `process_exit` wakes the parent when the child
/// finishes. Userland still loops over the EAGAIN return with yield, but each
/// iteration de-schedules the blocked parent instead of re-queuing it Ready —
/// this is what prevents multi-tab waitpid livelock/starvation.
fn sys_waitpid(frame: &mut SyscallFrame) -> u64 {
    const EAGAIN: u64 = u64::MAX - 1;
    let pid = frame.rdi as usize;

    let mut sched = crate::sched::SCHEDULER.lock();
    let me = sched.current_process().pid;

    let mut found = false;
    let mut finished_code = None;
    for p in sched.processes.iter() {
        if p.pid == pid && p.ppid == me {
            found = true;
            if matches!(p.state, ProcessState::Finished | ProcessState::Reaped) {
                finished_code = Some((p.exit_code as u8) as u64);
            }
            break;
        }
    }

    if !found {
        // No such child — clear any stale wait and report failure.
        sched.current_process_mut().wait_child = None;
        return u64::MAX;
    }

    if let Some(code) = finished_code {
        sched.current_process_mut().wait_child = None;
        return code;
    }

    // Child still running: park the caller until the child exits.
    let global_tick = sched.global_tick;
    let cur = sched.current_process_index().unwrap_or(0);
    // Phase 1 + 4: account + possible churn penalty for short work before wait
    sched.account_and_apply_churn_penalty();
    {
        let caller = &mut sched.processes[cur];
        caller.wait_child = Some(pid);
        caller.state = ProcessState::BlockedOnIpc;
        caller.block_start_tick = global_tick;
    }
    sched.remove_from_ready_queues(cur);
    drop(sched);
    crate::sched::request_reschedule();
    EAGAIN
}

/// Read a whole file out of the kernel VFS into a heap buffer.
/// Returns None when the VFS is absent, the path does not resolve, or the
/// path is not a regular file.
fn vfs_read_file(path: &str) -> Option<alloc::vec::Vec<u8>> {
    use sunlight_fs::vfs::FileType;

    let mut guard = crate::KERNEL_VFS.lock();
    let vfs = guard.as_mut()?;

    let stat = vfs.stat(path).ok()?;
    if stat.file_type != FileType::File {
        return None;
    }

    let handle = vfs.open(path).ok()?;
    let mut buf = alloc::vec![0u8; stat.size];
    let mut filled = 0usize;
    while filled < buf.len() {
        match vfs.read(handle, filled, &mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(_) => {
                let _ = vfs.close(handle);
                return None;
            }
        }
    }
    let _ = vfs.close(handle);
    buf.truncate(filled);
    Some(buf)
}

/// Syscall: Spawn (39) — posix_spawn-style process creation.
/// rdi = path pointer (C string)
/// rsi = argv pointer (array of *const u8, NULL-terminated)
/// rdx = parent fd to install as the child's stdout, or u64::MAX for none
/// Returns the child pid, or -1 on error.
fn sys_spawn(frame: &mut SyscallFrame) -> u64 {
    // [LAUNCH-TRACE] Point 1: request received
    let current_cpu = crate::sched::current_cpu_id();
    let launch_id = crate::launch_trace::next_launch_id();
    let mut trace = crate::launch_trace::LaunchTrace::new(launch_id, now_ns());

    let path_ptr = frame.rdi;
    let argv_ptr = frame.rsi;
    let stdout_fd = frame.rdx;

    let path_bytes = match read_user_cstr(path_ptr, USER_PATH_MAX) {
        Ok(bytes) => bytes,
        Err(error) => return user_memory_failure(error),
    };
    let path_str = match core::str::from_utf8(&path_bytes) {
        Ok(s) => s,
        Err(_) => return u64::MAX,
    };

    // [LAUNCH-TRACE] Point 2: app resolution started
    trace.resolve_started_ns = now_ns();

    let mut argv_bytes = alloc::vec::Vec::new();
    if argv_ptr != 0 {
        let argv_ptrs = match read_user_ptr_array(argv_ptr, USER_ARG_COUNT_MAX) {
            Ok(pointers) => pointers,
            Err(error) => return user_memory_failure(error),
        };
        let mut total_arg_bytes = 0usize;
        for &arg_ptr in &argv_ptrs {
            match read_user_cstr(arg_ptr, USER_ARG_MAX) {
                Ok(bytes) => {
                    total_arg_bytes = match total_arg_bytes.checked_add(bytes.len() + 1) {
                        Some(total) if total <= USER_ARG_TOTAL_MAX => total,
                        _ => {
                            return user_memory_failure(
                                crate::memory::user::UserMemoryError::ArrayTooLarge,
                            )
                        }
                    };
                    argv_bytes.push(bytes);
                }
                Err(error) => return user_memory_failure(error),
            }
        }
    }

    // Resolve the binary before touching scheduler state: embedded images
    // first, then the kernel VFS. (KERNEL_VFS must not be held across the
    // SCHEDULER/PMM locks below.)
    let vfs_bytes;
    let bytes: &[u8] = match crate::process::spawn::embedded_bytes_for_path(path_str) {
        Ok(b) => b,
        Err(_) => match vfs_read_file(path_str) {
            Some(v) => {
                vfs_bytes = v;
                &vfs_bytes
            }
            None => {
                crate::serial_println!("[SYSCALL] spawn: path not found: {}", path_str);
                trace.emit(path_str, path_str, None, "failed:not_found");
                return u64::MAX;
            }
        },
    };

    // [LAUNCH-TRACE] Point 3: app resolution finished
    trace.resolve_finished_ns = now_ns();

    let mut sched = crate::sched::SCHEDULER.lock();
    let mut pmm = crate::PMM.lock();
    let hhdm = VirtAddr::new(crate::HHDM_REQ.response().expect("no hhdm").offset);

    let parent = sched.current_process();
    let ppid = parent.pid;
    let uid = parent.uid;
    let gid = parent.gid;
    let env = crate::process::env::EnvMap::inherit(&parent.env);
    let capabilities = parent.capabilities.clone();
    let parent_tty_tab = parent.tty_tab;
    let parent_cwd = parent.cwd.clone();
    let stdout_entry = if stdout_fd != u64::MAX {
        parent.fd_table.get(stdout_fd as i32).copied()
    } else {
        None
    };
    if stdout_fd != u64::MAX && stdout_entry.is_none() {
        return u64::MAX; // caller asked to pass an fd it does not own
    }

    let pid = sched.processes.iter().map(|p| p.pid).max().unwrap_or(0) + 1;
    let proc_name = crate::process::spawn::name_from_path(path_str);
    let mut child =
        match unsafe { crate::process::Process::try_new(pid, ppid, proc_name, &mut pmm, hhdm) } {
            Ok(child) => child,
            Err(_) => return u64::MAX,
        };
    child.uid = uid;
    child.gid = gid;
    child.env = env;
    child.cwd = parent_cwd;
    child.capabilities = capabilities;
    // Inherit the TTY tab so a shell-spawned app's stdio routes to that tab's
    // kernel rings (foreground input routing).
    child.tty_tab = parent_tty_tab
        .or_else(|| crate::process::spawn::shell_id_from_path(path_str).map(|id| id as u8));

    let argv_refs: alloc::vec::Vec<&[u8]> = argv_bytes.iter().map(|v| v.as_slice()).collect();
    let envp_strings = child.env.to_envp();
    let envp_refs: alloc::vec::Vec<&[u8]> = envp_strings.iter().map(|s| s.as_bytes()).collect();

    // [LAUNCH-TRACE] Point 4: spawn started (about to exec_into_process)
    trace.spawn_started_ns = now_ns();

    match crate::process::spawn::exec_into_process(
        bytes, &mut child, &mut pmm, hhdm, &argv_refs, &envp_refs, false,
    ) {
        Ok(_) => {
            child.trusted_display_service =
                crate::process::spawn::is_trusted_display_path(path_str)
                    && crate::process::spawn::embedded_bytes_for_path(path_str).is_ok();
            child.trusted_swap_admin_service =
                crate::process::spawn::is_trusted_swap_admin_path(path_str)
                    && crate::process::spawn::embedded_bytes_for_path(path_str).is_ok();
            child.trusted_zram_diagnostic =
                crate::process::spawn::is_trusted_zram_diagnostic_path(path_str)
                    && crate::process::spawn::embedded_bytes_for_path(path_str).is_ok();
            // [LAUNCH-TRACE] Point 5: spawn returned successfully
            trace.spawn_returned_ns = now_ns();
        }
        Err(e) => {
            crate::serial_println!("[SYSCALL] spawn: load failed: {:?}", e);
            trace.emit(path_str, path_str, None, "failed:elf_load");
            return u64::MAX;
        }
    }

    // Wire the child's stdio. If attached to a TTY tab, fd0/fd1 route to that
    // tab's kernel rings so keyboard input reaches the app and its output is
    // rendered by tty_server. An explicit stdout_fd (e.g. a pipe write end for
    // `a | b`) still overrides fd1 below.
    if let Some(tab) = child.tty_tab {
        use crate::process::fd_table::{CapRights, FileHandle};
        let _ = child.fd_table.install_at(
            0,
            FileHandle::tty_stdin(tab),
            CapRights::new(CapRights::READ | CapRights::FSTAT),
            0,
        );
        let _ = child.fd_table.install_at(
            1,
            FileHandle::tty_stdout(tab),
            CapRights::new(CapRights::WRITE | CapRights::FSTAT),
            1,
        );
        // Wire stderr to the same TTY ring as stdout so eprintln!/write!(stderr)
        // output appears in the shell rather than falling through to serial only.
        let _ = child.fd_table.install_at(
            2,
            FileHandle::tty_stdout(tab),
            CapRights::new(CapRights::WRITE | CapRights::FSTAT),
            2,
        );
    }

    if let Some(entry) = stdout_entry {
        // The child's stdout becomes the parent-supplied handle (e.g. a pipe
        // write end). The parent keeps its own copy open, so pipe writer
        // accounting stays balanced.
        let _ = child
            .fd_table
            .install_at(1, entry.handle, entry.rights, entry.flags);
    }

    let child_pid = child.pid;
    // [LAUNCH-TRACE] Point 6: child_process_created (pid assigned)
    trace.child_created_ns = now_ns();

    let idx = sched.add_process(child);
    // add_process leaves queueing to the caller; without this the child sits
    // Ready but is never picked by the BORE queues.
    sched.enqueue_ready_on_cpu(idx, current_cpu);

    // [LAUNCH-TRACE] Point 7: enqueue_finished (child is runnable)
    trace.enqueue_finished_ns = now_ns();
    trace.emit(
        crate::process::spawn::name_from_path(path_str),
        path_str,
        Some(child_pid),
        "ok",
    );

    crate::serial_println!(
        "[SYSCALL] spawn: {} pid={} ppid={}",
        path_str,
        child_pid,
        ppid
    );
    child_pid as u64
}

/// Syscall: Getpid (33)
fn sys_getpid() -> u64 {
    sched::with_scheduler(|s| s.current_process().pid as u64)
}

/// Syscall: Getppid (34)
fn sys_getppid(_frame: &mut SyscallFrame) -> u64 {
    // TODO: implement when ppid is tracked
    crate::serial_println!("[SYSCALL] getppid requested");
    1
}

/// Syscall: Getuid (35)
fn sys_getuid() -> u64 {
    sched::with_scheduler(|s| s.current_process().uid as u64)
}

/// Syscall: Getgid (36)
fn sys_getgid() -> u64 {
    sched::with_scheduler(|s| s.current_process().gid as u64)
}

/// Syscall: Setuid (37)
/// rdi = uid to set
/// Returns 0 on success, -1 on error
fn sys_setuid(frame: &mut SyscallFrame) -> u64 {
    let new_uid = frame.rdi as u32;

    let mut sched = crate::sched::SCHEDULER.lock();
    let process = sched.current_process_mut();
    let current_uid = process.uid;

    // Only root (UID 0) can call setuid for other users
    // Any user can setuid to their own uid
    if current_uid == 0 || new_uid == current_uid {
        process.uid = new_uid;
        crate::serial_println!(
            "[SYSCALL] setuid: pid={} uid {}→{}",
            process.pid,
            current_uid,
            new_uid
        );
        0
    } else {
        crate::serial_println!(
            "[SYSCALL] setuid: EPERM (uid {} cannot setuid to {})",
            current_uid,
            new_uid
        );
        u64::MAX // -1 (EPERM)
    }
}

/// Syscall: Setgid (38)
/// rdi = gid to set
/// Returns 0 on success, -1 on error
fn sys_setgid(frame: &mut SyscallFrame) -> u64 {
    let new_gid = frame.rdi as u32;

    let mut sched = crate::sched::SCHEDULER.lock();
    let process = sched.current_process_mut();
    let current_uid = process.uid;
    let current_gid = process.gid;

    // Only root (UID 0) can call setgid for other groups
    // Any user can setgid to their own gid
    if current_uid == 0 || new_gid == current_gid {
        process.gid = new_gid;
        crate::serial_println!(
            "[SYSCALL] setgid: pid={} gid {}→{}",
            process.pid,
            current_gid,
            new_gid
        );
        0
    } else {
        crate::serial_println!(
            "[SYSCALL] setgid: EPERM (uid {} cannot setgid to {})",
            current_uid,
            new_gid
        );
        u64::MAX // -1 (EPERM)
    }
}

fn clamp_nice(raw: i64) -> i8 {
    raw.clamp(-10, 10) as i8
}

/// Syscall: GetNice (84)
/// rdi = pid (0 means current process)
/// Returns signed nice encoded in u64, or u64::MAX on failure.
fn sys_getnice(frame: &mut SyscallFrame) -> u64 {
    let sched = crate::sched::SCHEDULER.lock();

    let current_pid = sched.current_process().pid;
    let current_uid = sched.current_process().uid;
    let target_pid = if frame.rdi == 0 {
        current_pid
    } else {
        frame.rdi as usize
    };

    let Some(target) = sched.processes.iter().find(|p| p.pid == target_pid) else {
        crate::serial_println!("[SYSCALL] getnice: no such pid {}", target_pid);
        return u64::MAX;
    };

    if current_uid != 0 && target.uid != current_uid {
        crate::serial_println!(
            "[SYSCALL] getnice: EPERM current_uid={} target_uid={} pid={}",
            current_uid,
            target.uid,
            target_pid
        );
        return u64::MAX;
    }

    (target.nice as i64) as u64
}

/// Syscall: SetNice (83)
/// rdi = pid (0 means current process)
/// rsi = absolute nice value (kernel clamps to -10..=10)
/// Returns signed nice encoded in u64, or u64::MAX on failure.
fn sys_setnice(frame: &mut SyscallFrame) -> u64 {
    let mut sched = crate::sched::SCHEDULER.lock();

    let current_pid = sched.current_process().pid;
    let current_uid = sched.current_process().uid;
    let target_pid = if frame.rdi == 0 {
        current_pid
    } else {
        frame.rdi as usize
    };
    let new_nice = clamp_nice(frame.rsi as i64);

    let Some(target_idx) = sched.processes.iter().position(|p| p.pid == target_pid) else {
        crate::serial_println!("[SYSCALL] setnice: no such pid {}", target_pid);
        return u64::MAX;
    };

    let target_uid = sched.processes[target_idx].uid;
    let old_nice = sched.processes[target_idx].nice;

    if current_uid != 0 {
        if target_uid != current_uid {
            crate::serial_println!(
                "[SYSCALL] setnice: EPERM cross-uid current_uid={} target_uid={} pid={}",
                current_uid,
                target_uid,
                target_pid
            );
            return u64::MAX;
        }
        if new_nice < old_nice {
            crate::serial_println!(
                "[SYSCALL] setnice: EPERM raise-priority denied uid={} pid={} old={} new={}",
                current_uid,
                target_pid,
                old_nice,
                new_nice
            );
            return u64::MAX;
        }
    }

    sched.processes[target_idx].nice = new_nice;
    crate::serial_println!(
        "[SYSCALL] setnice: pid={} uid={} {}→{}",
        target_pid,
        target_uid,
        old_nice,
        new_nice
    );
    (new_nice as i64) as u64
}

/// Syscall: open (40) — kernel-VFS backed, read-only for now.
/// rdi = pathname (user-space pointer)
/// rsi = flags (reserved)
/// rdx = mode (reserved)
fn sys_open(frame: &mut SyscallFrame) -> u64 {
    let path_bytes = match read_user_cstr(frame.rdi, USER_PATH_MAX) {
        Ok(bytes) => bytes,
        Err(error) => return user_memory_failure(error),
    };
    let raw = match core::str::from_utf8(&path_bytes) {
        Ok(s) => s,
        Err(_) => return u64::MAX,
    };

    // Resolve relative paths against the process CWD.
    let resolved_buf;
    let path: &str = if raw.starts_with('/') {
        raw
    } else {
        let cwd = crate::sched::with_scheduler(|s| s.current_process().cwd.clone());
        resolved_buf = if cwd == "/" {
            alloc::format!("/{}", raw)
        } else {
            alloc::format!("{}/{}", cwd, raw)
        };
        resolved_buf.as_str()
    };

    let flags = frame.rsi;
    let wants_write = flags & 0b11 != 0;
    let wants_create = flags & 0x40 != 0;

    // Open on the VFS first, then register the fd. KERNEL_VFS is released
    // before SCHEDULER is taken (lock-order invariant).
    let vfs_handle = {
        let (uid, gid, actor) = current_fs_actor();
        if wants_write || wants_create {
            let operation = if wants_create {
                sunlight_fs::FsOperation::Create
            } else {
                sunlight_fs::FsOperation::Write
            };
            crate::serial_println!(
                "[SUNLIGHT-FS] request actor={:?} op={:?} path={}",
                actor,
                operation,
                path
            );
            let decision = sunlight_fs::can_write(actor, path, operation, None, false);
            crate::serial_println!(
                "[SUNLIGHT-FS] decision actor={:?} op={:?} path={} result={} reason={:?} err={:?}",
                actor,
                operation,
                path,
                if decision.allowed { "allow" } else { "deny" },
                decision.reason,
                decision.error
            );
            if !decision.allowed {
                return u64::MAX;
            }
        }
        let mut guard = crate::KERNEL_VFS.lock();
        let Some(vfs) = guard.as_mut() else {
            return u64::MAX;
        };
        if wants_create {
            match vfs.create_file(path, uid, gid, sunlight_fs::vfs::mode::FILE_644) {
                Ok(h) => h,
                Err(_) => return u64::MAX,
            }
        } else {
            match vfs.open(path) {
                Ok(h) => h,
                Err(e) => {
                    crate::serial_println!("[HELIOS] open({}) -> err {:?}", path, e);
                    return u64::MAX;
                }
            }
        }
    };

    let mut sched = crate::sched::SCHEDULER.lock();
    let handle = crate::process::fd_table::FileHandle::vfs(vfs_handle.0);
    let mut rights_bits =
        crate::process::fd_table::CapRights::READ | crate::process::fd_table::CapRights::FSTAT;
    if wants_write || wants_create {
        rights_bits |= crate::process::fd_table::CapRights::WRITE;
    }
    let rights = crate::process::fd_table::CapRights::new(rights_bits);
    match sched
        .current_process_mut()
        .fd_table
        .open(handle, rights, flags as u32)
    {
        Ok(fd) => {
            crate::serial_println!("[HELIOS] open({}) -> ok", path);
            fd as u64
        }
        Err(e) => {
            crate::serial_println!("[HELIOS] open({}) -> fd error {:?}", path, e);
            drop(sched);
            if let Some(vfs) = crate::KERNEL_VFS.lock().as_mut() {
                let _ = vfs.close(vfs_handle);
            }
            u64::MAX
        }
    }
}

fn sys_chdir(frame: &mut SyscallFrame) -> u64 {
    let path_bytes = match read_user_cstr(frame.rdi, USER_PATH_MAX) {
        Ok(bytes) => bytes,
        Err(error) => return user_memory_failure(error),
    };
    let path = match core::str::from_utf8(&path_bytes) {
        Ok(s) => s,
        Err(_) => return u64::MAX,
    };

    // Resolve against current CWD if relative.
    let resolved_buf;
    let abs_path: &str = if path.starts_with('/') {
        path
    } else {
        let cwd = crate::sched::with_scheduler(|s| s.current_process().cwd.clone());
        resolved_buf = if cwd == "/" {
            alloc::format!("/{}", path)
        } else {
            alloc::format!("{}/{}", cwd, path)
        };
        resolved_buf.as_str()
    };

    // Verify the path exists in VFS.
    {
        let mut guard = crate::KERNEL_VFS.lock();
        let Some(vfs) = guard.as_mut() else {
            return u64::MAX;
        };
        match vfs.open(abs_path) {
            Ok(h) => {
                let _ = vfs.close(h);
            }
            Err(_) => {
                crate::serial_println!("[HELIOS] chdir({}) -> not found", abs_path);
                return u64::MAX;
            }
        }
    }

    crate::sched::with_scheduler(|s| {
        s.current_process_mut().cwd = alloc::string::String::from(abs_path);
    });
    crate::serial_println!("[HELIOS] chdir({}) -> ok", abs_path);
    0
}

fn sys_getcwd(frame: &mut SyscallFrame) -> u64 {
    let buf_len = frame.rsi as usize;
    if frame.rdi == 0 || buf_len == 0 {
        return u64::MAX;
    }
    let cwd = crate::sched::with_scheduler(|s| s.current_process().cwd.clone());
    let bytes = cwd.as_bytes();
    let copy_len = bytes.len().min(buf_len - 1);
    let mut output = alloc::vec::Vec::new();
    if output.try_reserve_exact(copy_len + 1).is_err() {
        return u64::MAX;
    }
    output.extend_from_slice(&bytes[..copy_len]);
    output.push(0);
    if let Err(error) = copy_to_user(frame.rdi, &output) {
        return error;
    }
    frame.rdi // return buf pointer per Linux getcwd ABI
}

fn current_fs_actor() -> (u32, u32, sunlight_fs::Actor<'static>) {
    let sched = crate::sched::SCHEDULER.lock();
    let p = sched.current_process();
    let actor = match p.name_str() {
        "sunlight-kv" => sunlight_fs::Actor::Service {
            name: "sunlight-kv",
        },
        "sunlight-tls" => sunlight_fs::Actor::Service {
            name: "sunlight-tls",
        },
        "uac_service" | "sunlight-uac" => sunlight_fs::Actor::Service {
            name: "sunlight-uac",
        },
        "capability-broker" => sunlight_fs::Actor::Service {
            name: "capability-broker",
        },
        _ => sunlight_fs::Actor::User {
            uid: p.uid,
            name: username_for_uid(p.uid),
        },
    };
    (p.uid, p.gid, actor)
}

fn username_for_uid(uid: u32) -> &'static str {
    match uid {
        0 => "root",
        1000 => "user",
        _ => "",
    }
}

/// Syscall: GrantCapability (100) — kernel-side VFS capability minting gate.
/// rdi = pointer to path prefix string (NUL-terminated, <= 63 bytes)
/// rsi = access bits (read=1, write=2, execute=4)
fn sys_grant_capability_syscall(frame: &mut SyscallFrame) -> u64 {
    let caller_pid = crate::sched::with_scheduler(|sched| sched.current_process().pid as u32);

    let path_bytes = match read_user_cstr(frame.rdi, 64) {
        Ok(bytes) => bytes,
        Err(error) => return user_memory_failure(error),
    };
    let path = match core::str::from_utf8(&path_bytes) {
        Ok(s) => s,
        Err(_) => return u64::MAX,
    };

    let mut prefix = heapless::String::<64>::new();
    if prefix.push_str(path).is_err() {
        return u64::MAX;
    }
    let flags = frame.rsi;
    let cap = crate::capability::VfsCapability {
        allowed_prefix: prefix,
        flags: crate::capability::AccessFlags {
            read: flags & 1 != 0,
            write: flags & 2 != 0,
            execute: flags & 4 != 0,
        },
    };

    match crate::capability::sys_grant_capability(caller_pid, cap) {
        Ok(token) => token.0,
        Err(crate::capability::CapError::InvalidCaller) => u64::MAX,
        Err(crate::capability::CapError::CapabilityStoreFull) => u64::MAX,
        Err(_) => u64::MAX,
    }
}

fn sys_set_fs_base(frame: &mut SyscallFrame) -> u64 {
    let base = frame.rdi;
    if crate::memory::user::UserAddress::new(base).is_err() {
        return u64::MAX;
    }

    unsafe {
        x86_64::registers::model_specific::Msr::new(0xC0000100).write(base);
    }

    crate::sched::with_scheduler(|sched| {
        sched.current_process_mut().fs_base = base;
    });

    0
}

/// Emulate Linux `brk(2)` for Linux-compatible binaries.
///
/// Linux returns the current break on success. On failure, it returns the
/// previous break rather than a raw error code.
fn sys_brk(frame: &mut SyscallFrame) -> u64 {
    let requested_brk = frame.rdi;
    let heap_base = crate::process::layout::USER_HEAP_START;

    let mut sched = crate::sched::SCHEDULER.lock();
    let pid = sched.current_process().pid;

    // Lazy heap initialization: the first brk call establishes the process-local
    // Linux heap range.
    {
        let process = sched.current_process_mut();
        if process.brk_base == 0 {
            process.brk_base = heap_base;
            process.brk_current = heap_base;
        }

        if requested_brk == 0 {
            return process.brk_current;
        }

        if requested_brk < process.brk_base {
            return process.brk_current;
        }
    }

    let current_brk = sched.current_process().brk_current;
    let current_page_end = (current_brk + 0xFFF) & !0xFFF;
    let target_page_end = (requested_brk + 0xFFF) & !0xFFF;

    if target_page_end > current_page_end {
        let size_to_map = target_page_end - current_page_end;
        let mut pmm = crate::PMM.lock();
        let result =
            crate::process::mmap::map_brk(current_page_end, size_to_map, &mut *pmm, &mut *sched);

        match result {
            Ok(_) => {
                sched.current_process_mut().brk_current = requested_brk;
            }
            Err(_) => {
                let previous = sched
                    .processes
                    .iter()
                    .find(|p| p.pid == pid)
                    .map(|p| p.brk_current)
                    .unwrap_or(current_brk);
                return previous;
            }
        }
    } else if target_page_end < current_page_end {
        // Real unmapping is deferred; do not claim a shrink that did nothing.
        return current_brk;
    } else {
        sched.current_process_mut().brk_current = requested_brk;
    }

    sched.current_process().brk_current
}

const ARCH_SET_FS: u64 = 0x1002;

fn sys_arch_prctl(frame: &mut SyscallFrame) -> u64 {
    let code = frame.rdi;
    let addr = frame.rsi;

    if code == ARCH_SET_FS {
        frame.rdi = addr;
        return sys_set_fs_base(frame);
    }

    u64::MAX
}

fn linux_errno(errno: u64) -> u64 {
    0u64.wrapping_sub(errno)
}

fn sys_linux_set_tid_address(frame: &mut SyscallFrame) -> u64 {
    let tidptr = frame.rdi;
    if tidptr != 0 && crate::memory::user::UserAddress::new(tidptr).is_err() {
        return linux_errno(14); // EFAULT
    }

    sched::with_scheduler(|s| s.current_process().pid as u64)
}

fn sys_linux_set_robust_list(frame: &mut SyscallFrame) -> u64 {
    let head = frame.rdi;
    if head != 0 && crate::memory::user::UserAddress::new(head).is_err() {
        return linux_errno(14); // EFAULT
    }

    0
}

fn sys_linux_rseq(_frame: &mut SyscallFrame) -> u64 {
    linux_errno(38) // ENOSYS: libc should continue with rseq disabled.
}

fn sys_linux_getrandom(frame: &mut SyscallFrame) -> u64 {
    let len = frame.rsi as usize;
    let _flags = frame.rdx;

    if len == 0 {
        return 0;
    }
    if frame.rdi == 0 || len > isize::MAX as usize {
        return linux_errno(14); // EFAULT
    }

    // Compatibility shim: enough for musl/std hash seeding and similar callers.
    // For now this is backed by the same kernel entropy source that seeds the
    // native libc fast path, so it is always non-blocking.
    let mut written = 0usize;
    while written < len {
        let chunk = sys_get_entropy().to_le_bytes();
        let n = (len - written).min(chunk.len());
        if crate::memory::user::copy_to_current(frame.rdi + written as u64, &chunk[..n]).is_err() {
            return linux_errno(14);
        }
        written += n;
    }

    len as u64
}

fn sys_linux_poll(frame: &mut SyscallFrame) -> u64 {
    let nfds = frame.rsi;

    if nfds == 0 {
        return 0;
    }

    let Some(bytes) = nfds.checked_mul(8) else {
        return linux_errno(22); // EINVAL
    };
    if bytes > 8192 {
        return linux_errno(22); // keep the early compat shim bounded
    }

    if crate::memory::user::UserRange::new(frame.rdi, bytes as usize).is_err() {
        return linux_errno(14); // EFAULT
    }

    for idx in 0..nfds {
        if crate::memory::user::copy_to_current(frame.rdi + idx * 8 + 6, &[0, 0]).is_err() {
            return linux_errno(14);
        }
    }

    0
}

fn sys_linux_rt_sigaction(frame: &mut SyscallFrame) -> u64 {
    let sig = frame.rdi as u32;
    let act = frame.rsi;
    let oldact = frame.rdx;
    let sigset_size = frame.r10;

    if sig == 0 || sig > 64 {
        return linux_errno(22); // EINVAL
    }
    if sigset_size != 8 && sigset_size != 16 {
        return linux_errno(22); // EINVAL
    }
    if act != 0 && crate::memory::user::UserRange::new(act, 32).is_err() {
        return linux_errno(14); // EFAULT
    }
    if oldact != 0 {
        if crate::memory::user::copy_to_current(oldact, &[0u8; 32]).is_err() {
            return linux_errno(14);
        }
    }

    0
}

fn sys_linux_rt_sigprocmask(frame: &mut SyscallFrame) -> u64 {
    let how = frame.rdi;
    let set = frame.rsi;
    let oldset = frame.rdx;
    let sigset_size = frame.r10;

    if how > 2 {
        return linux_errno(22); // EINVAL
    }
    if sigset_size != 8 && sigset_size != 16 {
        return linux_errno(22); // EINVAL
    }
    if set != 0 && crate::memory::user::UserRange::new(set, sigset_size as usize).is_err() {
        return linux_errno(14); // EFAULT
    }
    if oldset != 0 {
        let zeros = [0u8; 16];
        if crate::memory::user::copy_to_current(oldset, &zeros[..sigset_size as usize]).is_err() {
            return linux_errno(14);
        }
    }

    0
}

fn sys_linux_tkill(frame: &mut SyscallFrame) -> u64 {
    let tid = frame.rdi as usize;
    let sig = frame.rsi as i32;
    let current_pid = sched::with_scheduler(|s| s.current_process().pid);

    if tid != current_pid || sig < 0 || sig > 64 {
        return linux_errno(22); // EINVAL
    }
    if sig == 0 {
        return 0;
    }
    let Some(signal) = crate::process::signal::Signal::try_from_u32(sig as u32) else {
        return linux_errno(22);
    };
    match send_signal(tid, signal) {
        Ok(()) => 0,
        Err(()) => linux_errno(22),
    }
}

fn sys_linux_mmap(frame: &mut SyscallFrame) -> u64 {
    const LINUX_MAP_SHARED: u64 = 0x01;
    const LINUX_MAP_PRIVATE: u64 = 0x02;
    const LINUX_MAP_FIXED: u64 = 0x10;
    const LINUX_MAP_ANONYMOUS: u64 = 0x20;

    let mut flags = frame.r10;
    let fd = frame.r8 as i32;

    // Keep only the subset understood by the native mmap implementation.
    // Linux-only flags such as MAP_DENYWRITE, MAP_EXECUTABLE, MAP_GROWSDOWN,
    // MAP_NORESERVE, and MAP_STACK are advisory for this early compat layer.
    flags &= LINUX_MAP_SHARED | LINUX_MAP_PRIVATE | LINUX_MAP_FIXED | LINUX_MAP_ANONYMOUS;

    if flags & LINUX_MAP_ANONYMOUS == 0 && fd < 0 {
        flags |= LINUX_MAP_ANONYMOUS;
    }
    if flags & (LINUX_MAP_SHARED | LINUX_MAP_PRIVATE) == 0 {
        flags |= LINUX_MAP_PRIVATE;
    }

    frame.r10 = flags;
    sys_mmap(frame)
}

fn sys_linux_sigaltstack(frame: &mut SyscallFrame) -> u64 {
    let new_ss = frame.rdi;
    let old_ss = frame.rsi;

    if new_ss != 0 && crate::memory::user::UserRange::new(new_ss, 24).is_err() {
        return linux_errno(14); // EFAULT
    }

    if old_ss != 0 {
        let mut old = [0u8; 24];
        old[8..16].copy_from_slice(&2u64.to_ne_bytes());
        if crate::memory::user::copy_to_current(old_ss, &old).is_err() {
            return linux_errno(14); // EFAULT
        }
    }

    0
}

// ── Linux terminal ioctl constants ──────────────────────────────────────────
const TCGETS: u64 = 0x5401;
const TCSETS: u64 = 0x5402;
const TCSETSW: u64 = 0x5403;
const TCSETSF: u64 = 0x5404;
const TIOCGWINSZ: u64 = 0x5413;
const TIOCSWINSZ: u64 = 0x5414;

pub const ICANON: u32 = 0x00000002;
pub const ECHO: u32 = 0x00000008;

/// Linux `struct termios` (x86-64 ABI).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct LinuxTermios {
    pub c_iflag: u32,
    pub c_oflag: u32,
    pub c_cflag: u32,
    pub c_lflag: u32,
    pub c_line: u8,
    pub c_cc: [u8; 32],
    pub c_ispeed: u32,
    pub c_ospeed: u32,
}

impl LinuxTermios {
    /// Sensible cooked-mode defaults (canonical + echo, 38400 baud).
    pub const fn default_cooked() -> Self {
        let mut cc = [0u8; 32];
        cc[4] = 4; // VEOF  = ^D
        cc[7] = 0; // VSTART
        cc[8] = 0; // VSTOP
        cc[10] = 0; // VEOL
        Self {
            c_iflag: 0x0500,                 // ICRNL | IXON
            c_oflag: 0x0005,                 // OPOST | ONLCR
            c_cflag: 0x00BF,                 // CS8 | CREAD | CLOCAL (B38400)
            c_lflag: ICANON | ECHO | 0x8000, // ICANON | ECHO | ISIG
            c_line: 0,
            c_cc: cc,
            c_ispeed: 15, // B38400
            c_ospeed: 15,
        }
    }
}

/// Linux `struct winsize`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct LinuxWinsize {
    pub ws_row: u16,
    pub ws_col: u16,
    pub ws_xpixel: u16,
    pub ws_ypixel: u16,
}

fn sys_linux_ioctl(frame: &mut SyscallFrame) -> u64 {
    let fd = frame.rdi as i32;
    let request = frame.rsi;
    let argp = frame.rdx;

    match request {
        TCGETS | TCSETS | TCSETSW | TCSETSF => {
            // Only honour on stdin/stdout/stderr; any other fd is not a tty.
            if fd != 0 && fd != 1 && fd != 2 {
                return linux_errno(25); // ENOTTY
            }
            if request == TCGETS {
                let sched = crate::sched::SCHEDULER.lock();
                let termios = sched.current_process().linux_termios;
                drop(sched);
                let mut wire = [0u8; 60];
                wire[0..4].copy_from_slice(&termios.c_iflag.to_ne_bytes());
                wire[4..8].copy_from_slice(&termios.c_oflag.to_ne_bytes());
                wire[8..12].copy_from_slice(&termios.c_cflag.to_ne_bytes());
                wire[12..16].copy_from_slice(&termios.c_lflag.to_ne_bytes());
                wire[16] = termios.c_line;
                wire[17..49].copy_from_slice(&termios.c_cc);
                wire[52..56].copy_from_slice(&termios.c_ispeed.to_ne_bytes());
                wire[56..60].copy_from_slice(&termios.c_ospeed.to_ne_bytes());
                if crate::memory::user::copy_to_current(argp, &wire).is_err() {
                    return linux_errno(14);
                }
            } else {
                let mut wire = [0u8; 60];
                if crate::memory::user::copy_from_current(argp, &mut wire).is_err() {
                    return linux_errno(14);
                }
                let mut new_termios = LinuxTermios::default_cooked();
                new_termios.c_iflag = u32::from_ne_bytes(wire[0..4].try_into().unwrap());
                new_termios.c_oflag = u32::from_ne_bytes(wire[4..8].try_into().unwrap());
                new_termios.c_cflag = u32::from_ne_bytes(wire[8..12].try_into().unwrap());
                new_termios.c_lflag = u32::from_ne_bytes(wire[12..16].try_into().unwrap());
                new_termios.c_line = wire[16];
                new_termios.c_cc.copy_from_slice(&wire[17..49]);
                new_termios.c_ispeed = u32::from_ne_bytes(wire[52..56].try_into().unwrap());
                new_termios.c_ospeed = u32::from_ne_bytes(wire[56..60].try_into().unwrap());
                let mut sched = crate::sched::SCHEDULER.lock();
                let process = sched.current_process_mut();
                let was_raw = (process.linux_termios.c_lflag & ICANON) == 0;
                let is_raw = (new_termios.c_lflag & ICANON) == 0;
                process.linux_termios = new_termios;
                if was_raw != is_raw {
                    crate::serial_println!(
                        "[HELIOS] TTY mode → {} (pid={})",
                        if is_raw { "raw" } else { "cooked" },
                        process.pid
                    );
                }
            }
            0
        }
        TIOCGWINSZ => {
            let mut wire = [0u8; 8];
            wire[0..2].copy_from_slice(&25u16.to_ne_bytes());
            wire[2..4].copy_from_slice(&80u16.to_ne_bytes());
            if crate::memory::user::copy_to_current(argp, &wire).is_err() {
                return linux_errno(14);
            }
            0
        }
        TIOCSWINSZ => {
            // Accept but ignore; we have a fixed 80×25 terminal for now.
            0
        }
        _ => {
            crate::serial_println!("[HELIOS] Unhandled ioctl fd={} req={:#x}", fd, request);
            linux_errno(22) // EINVAL
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct LinuxIovec {
    iov_base: u64,
    iov_len: u64,
}

fn sys_linux_writev(frame: &mut SyscallFrame) -> u64 {
    const EAGAIN: u64 = u64::MAX - 1;

    let fd = frame.rdi as i32;
    let iov_ptr = frame.rsi;
    let iovcnt = frame.rdx as usize;

    if iovcnt == 0 {
        return 0;
    }
    if iovcnt > 1024 || crate::memory::user::UserRange::for_elements(iov_ptr, iovcnt, 16).is_err() {
        return linux_errno(14);
    }

    let mut total_written = 0u64;

    for idx in 0..iovcnt {
        let mut wire = [0u8; 16];
        if crate::memory::user::copy_from_current(iov_ptr + (idx * 16) as u64, &mut wire).is_err() {
            return if total_written == 0 {
                linux_errno(14)
            } else {
                total_written
            };
        }
        let iov = LinuxIovec {
            iov_base: u64::from_ne_bytes(wire[..8].try_into().unwrap()),
            iov_len: u64::from_ne_bytes(wire[8..].try_into().unwrap()),
        };
        if iov.iov_len == 0 {
            continue;
        }

        let mut remaining = iov.iov_len;
        let mut offset = 0u64;

        while remaining > 0 {
            let chunk = remaining.min(4096);
            let base = match iov.iov_base.checked_add(offset) {
                Some(addr) => addr,
                None => {
                    return if total_written == 0 {
                        u64::MAX
                    } else {
                        total_written
                    }
                }
            };
            if crate::memory::user::UserRange::new(base, chunk as usize).is_err() {
                return if total_written == 0 {
                    linux_errno(14)
                } else {
                    total_written
                };
            }

            let mut temp_frame = SyscallFrame {
                rax: 43,
                rbx: frame.rbx,
                rcx: frame.rcx,
                rdx: chunk,
                rsi: base,
                rdi: fd as u64,
                rbp: frame.rbp,
                r8: frame.r8,
                r9: frame.r9,
                r10: frame.r10,
                r11: frame.r11,
                r12: frame.r12,
                r13: frame.r13,
                r14: frame.r14,
                r15: frame.r15,
            };

            let res = sys_write(&mut temp_frame);
            if res == u64::MAX {
                return if total_written == 0 {
                    u64::MAX
                } else {
                    total_written
                };
            }
            if res == EAGAIN {
                return if total_written == 0 {
                    EAGAIN
                } else {
                    total_written
                };
            }
            if res == 0 {
                return total_written;
            }

            total_written = total_written.saturating_add(res);
            if res < chunk {
                return total_written;
            }

            offset = match offset.checked_add(res) {
                Some(next) => next,
                None => return total_written,
            };
            remaining -= res;
        }
    }

    total_written
}

/// Syscall: ReadDir (60)
/// rdi = pathname (user-space pointer)
/// rsi = output buffer (array of 80-byte records)
/// rdx = buffer length in bytes
/// Record layout (repr(C), 80 bytes): name[64], name_len u8, file_type u8
/// (1=file, 2=dir), pad[6], size u64. Returns the number of records written.
fn sys_readdir(frame: &mut SyscallFrame) -> u64 {
    use sunlight_fs::vfs::FileType;
    const RECORD: usize = 80;

    let path_bytes = match read_user_cstr(frame.rdi, USER_PATH_MAX) {
        Ok(bytes) => bytes,
        Err(error) => return user_memory_failure(error),
    };
    let path = match core::str::from_utf8(&path_bytes) {
        Ok(s) => s,
        Err(_) => return u64::MAX,
    };

    let max_entries = ((frame.rdx as usize) / RECORD).min(64);
    if max_entries == 0
        || crate::memory::user::validate_current_write(frame.rsi, max_entries * RECORD).is_err()
    {
        return u64::MAX;
    }

    let mut records = alloc::vec::Vec::<[u8; RECORD]>::new();
    if records.try_reserve(max_entries).is_err() {
        return u64::MAX;
    }
    {
        let mut guard = crate::KERNEL_VFS.lock();
        let Some(vfs) = guard.as_mut() else {
            return u64::MAX;
        };
        if vfs
            .read_dir(path, &mut |entry| {
                if records.len() >= max_entries {
                    return false;
                }
                let mut record = [0u8; RECORD];
                let name = entry.name_bytes();
                let len = name.len().min(64);
                record[..len].copy_from_slice(&name[..len]);
                record[64] = len as u8;
                record[65] = match entry.file_type {
                    FileType::File => 1,
                    FileType::Directory => 2,
                };
                record[72..80].copy_from_slice(&(entry.size as u64).to_le_bytes());
                records.push(record);
                true
            })
            .is_err()
        {
            return u64::MAX;
        }
    }
    for (index, record) in records.iter().enumerate() {
        let Some(address) = (index as u64)
            .checked_mul(RECORD as u64)
            .and_then(|offset| frame.rsi.checked_add(offset))
        else {
            return u64::MAX;
        };
        if let Err(error) = copy_to_user(address, record) {
            return error;
        }
    }
    records.len() as u64
}

/// Syscall: StatPath (61)
/// rdi = pathname (user-space pointer)
/// rsi = output buffer (24 bytes, repr(C)): size u64, uid u32, gid u32,
///       mode u16, file_type u8 (1=file, 2=dir), pad u8, nlinks u32.
fn sys_stat_path(frame: &mut SyscallFrame) -> u64 {
    use sunlight_fs::vfs::FileType;

    let path_bytes = match read_user_cstr(frame.rdi, USER_PATH_MAX) {
        Ok(bytes) => bytes,
        Err(error) => return user_memory_failure(error),
    };
    let path = match core::str::from_utf8(&path_bytes) {
        Ok(s) => s,
        Err(_) => return u64::MAX,
    };

    let stat = {
        let mut guard = crate::KERNEL_VFS.lock();
        let Some(vfs) = guard.as_mut() else {
            return u64::MAX;
        };
        match vfs.stat(path) {
            Ok(s) => s,
            Err(_) => return u64::MAX,
        }
    };

    let mut record = [0u8; 24];
    record[0..8].copy_from_slice(&(stat.size as u64).to_le_bytes());
    record[8..12].copy_from_slice(&stat.uid.to_le_bytes());
    record[12..16].copy_from_slice(&stat.gid.to_le_bytes());
    record[16..18].copy_from_slice(&stat.mode.to_le_bytes());
    record[18] = match stat.file_type {
        FileType::File => 1,
        FileType::Directory => 2,
    };
    record[20..24].copy_from_slice(&stat.nlinks.to_le_bytes());
    if let Err(error) = copy_to_user(frame.rsi, &record) {
        return error;
    }
    0
}

/// Syscall: Mkdir (62)
/// rdi = pathname (user-space pointer)
/// rsi = mode bits
fn sys_mkdir(frame: &mut SyscallFrame) -> u64 {
    let path_bytes = match read_user_cstr(frame.rdi, USER_PATH_MAX) {
        Ok(bytes) => bytes,
        Err(error) => return user_memory_failure(error),
    };
    let path = match core::str::from_utf8(&path_bytes) {
        Ok(s) => s,
        Err(_) => return u64::MAX,
    };
    let mode = frame.rsi as u16;

    let (uid, gid, actor) = current_fs_actor();
    crate::serial_println!(
        "[SUNLIGHT-FS] request actor={:?} op={:?} path={}",
        actor,
        sunlight_fs::FsOperation::Mkdir,
        path
    );
    let decision =
        sunlight_fs::can_write(actor, path, sunlight_fs::FsOperation::Mkdir, None, false);
    crate::serial_println!(
        "[SUNLIGHT-FS] decision actor={:?} op={:?} path={} result={} reason={:?} err={:?}",
        actor,
        sunlight_fs::FsOperation::Mkdir,
        path,
        if decision.allowed { "allow" } else { "deny" },
        decision.reason,
        decision.error
    );
    if !decision.allowed {
        return u64::MAX;
    }

    let mut guard = crate::KERNEL_VFS.lock();
    let Some(vfs) = guard.as_mut() else {
        return u64::MAX;
    };
    match vfs.mkdir(path, uid, gid, sunlight_fs::vfs::mode::S_IFDIR | mode) {
        Ok(()) => 0,
        Err(_) => u64::MAX,
    }
}

/// Syscall: unlink (65) — remove a file.
/// rdi = NUL-terminated path
fn sys_unlink(frame: &mut SyscallFrame) -> u64 {
    let path_bytes = match read_user_cstr(frame.rdi, USER_PATH_MAX) {
        Ok(bytes) => bytes,
        Err(error) => return user_memory_failure(error),
    };
    let path = match core::str::from_utf8(&path_bytes) {
        Ok(s) => s,
        Err(_) => return u64::MAX,
    };

    let (_, _, actor) = current_fs_actor();
    let decision =
        sunlight_fs::can_write(actor, path, sunlight_fs::FsOperation::Delete, None, false);
    if !decision.allowed {
        return u64::MAX;
    }

    let mut guard = crate::KERNEL_VFS.lock();
    let Some(vfs) = guard.as_mut() else {
        return u64::MAX;
    };
    match vfs.unlink(path) {
        Ok(()) => 0,
        Err(_) => u64::MAX,
    }
}

/// Syscall: rename (66) — rename/move a file.
/// rdi = NUL-terminated old path, rsi = NUL-terminated new path
fn sys_rename(frame: &mut SyscallFrame) -> u64 {
    let old_bytes = match read_user_cstr(frame.rdi, USER_PATH_MAX) {
        Ok(bytes) => bytes,
        Err(error) => return user_memory_failure(error),
    };
    let new_bytes = match read_user_cstr(frame.rsi, USER_PATH_MAX) {
        Ok(bytes) => bytes,
        Err(error) => return user_memory_failure(error),
    };
    let old_path = match core::str::from_utf8(&old_bytes) {
        Ok(s) => s,
        Err(_) => return u64::MAX,
    };
    let new_path = match core::str::from_utf8(&new_bytes) {
        Ok(s) => s,
        Err(_) => return u64::MAX,
    };

    let (_, _, actor) = current_fs_actor();
    // Require Delete permission on source and Create permission on destination.
    let del_ok = sunlight_fs::can_write(
        actor,
        old_path,
        sunlight_fs::FsOperation::Delete,
        None,
        false,
    );
    let cre_ok = sunlight_fs::can_write(
        actor,
        new_path,
        sunlight_fs::FsOperation::Create,
        None,
        false,
    );
    if !del_ok.allowed || !cre_ok.allowed {
        return u64::MAX;
    }

    let mut guard = crate::KERNEL_VFS.lock();
    let Some(vfs) = guard.as_mut() else {
        return u64::MAX;
    };
    match vfs.rename(old_path, new_path) {
        Ok(()) => 0,
        Err(_) => u64::MAX,
    }
}

/// Syscall: chmod (67) — change file mode.
/// rdi = NUL-terminated path, rsi = mode (u16)
fn sys_chmod(frame: &mut SyscallFrame) -> u64 {
    let path_bytes = match read_user_cstr(frame.rdi, USER_PATH_MAX) {
        Ok(bytes) => bytes,
        Err(error) => return user_memory_failure(error),
    };
    let path = match core::str::from_utf8(&path_bytes) {
        Ok(s) => s,
        Err(_) => return u64::MAX,
    };
    let mode = frame.rsi as u16;

    // Only owner or root can chmod — policy allows if write is allowed.
    let (_, _, actor) = current_fs_actor();
    let decision =
        sunlight_fs::can_write(actor, path, sunlight_fs::FsOperation::Write, None, false);
    if !decision.allowed {
        return u64::MAX;
    }

    let mut guard = crate::KERNEL_VFS.lock();
    let Some(vfs) = guard.as_mut() else {
        return u64::MAX;
    };
    match vfs.chmod(path, mode) {
        Ok(()) => 0,
        Err(_) => u64::MAX,
    }
}

/// Syscall: chown (68) — change file owner/group.
/// rdi = NUL-terminated path, rsi = uid, rdx = gid
fn sys_chown(frame: &mut SyscallFrame) -> u64 {
    let path_bytes = match read_user_cstr(frame.rdi, USER_PATH_MAX) {
        Ok(bytes) => bytes,
        Err(error) => return user_memory_failure(error),
    };
    let path = match core::str::from_utf8(&path_bytes) {
        Ok(s) => s,
        Err(_) => return u64::MAX,
    };
    let uid = frame.rsi as u32;
    let gid = frame.rdx as u32;

    // Only root can chown.
    let (caller_uid, _, _) = current_fs_actor();
    if caller_uid != 0 {
        return u64::MAX;
    }

    let mut guard = crate::KERNEL_VFS.lock();
    let Some(vfs) = guard.as_mut() else {
        return u64::MAX;
    };
    match vfs.chown(path, uid, gid) {
        Ok(()) => 0,
        Err(_) => u64::MAX,
    }
}

/// Syscall: close (41)
/// rdi = fd
fn sys_close(frame: &mut SyscallFrame) -> u64 {
    let fd = frame.rdi as i32;

    let mut sched = crate::sched::SCHEDULER.lock();

    // Release the underlying object (pipe end or VFS handle) before
    // dropping the fd table entry.
    let handle = sched.current_process().fd_table.get(fd).map(|e| e.handle);
    if let Some(h) = handle {
        if h.is_pipe() {
            crate::process::pipe::pipe_close_end(h.pipe_index(), h.pipe_is_write());
        } else if h.is_vfs() {
            if let Some(vfs) = crate::KERNEL_VFS.lock().as_mut() {
                let _ = vfs.close(sunlight_fs::vfs::FileHandle(h.vfs_handle()));
            }
        }
    }

    match sched.current_process_mut().fd_table.close(fd) {
        Ok(()) => 0,
        Err(_) => u64::MAX, // EBADF
    }
}

/// Syscall: read (42)
/// rdi = fd
/// rsi = buf (user-space pointer)
/// rdx = count
fn sys_read(frame: &mut SyscallFrame) -> u64 {
    const EAGAIN: u64 = u64::MAX - 1;

    let fd = frame.rdi as i32;
    let count = frame.rdx as usize;
    if count == 0 {
        return 0;
    }

    let mut sched = crate::sched::SCHEDULER.lock();
    let hhdm = VirtAddr::new(crate::HHDM_REQ.response().expect("no hhdm").offset);

    // Check if fd is valid and has READ right
    match sched.current_process().fd_table.check_rights(
        fd,
        crate::process::fd_table::CapRights::new(crate::process::fd_table::CapRights::READ),
    ) {
        Ok(()) => {
            if let Some(&fd_entry) = sched.current_process().fd_table.get(fd) {
                if fd_entry.handle.is_pipe() {
                    let pipe_idx = fd_entry.handle.pipe_index();
                    let mut kernel_buf = [0u8; 4096];
                    let read_size = core::cmp::min(count, 4096);

                    match crate::process::pipe::pipe_read(pipe_idx, &mut kernel_buf[..read_size]) {
                        crate::process::pipe::PipeResult::Ok(n) => {
                            let is_linux = sched.current_process().is_linux_compat;
                            if let Err(error) = crate::memory::user::copy_to_process_bytes(
                                sched.current_process(),
                                hhdm,
                                frame.rsi,
                                &kernel_buf[..n],
                            ) {
                                return user_memory_failure_for(is_linux, error);
                            }
                            n as u64
                        }
                        crate::process::pipe::PipeResult::WouldBlock => EAGAIN,
                        crate::process::pipe::PipeResult::Eof => 0,
                        crate::process::pipe::PipeResult::BrokenPipe => u64::MAX,
                    }
                } else if fd_entry.handle.is_vfs() {
                    let vfs_handle = sunlight_fs::vfs::FileHandle(fd_entry.handle.vfs_handle());
                    let mut kernel_buf = [0u8; 4096];
                    let to_read = count.min(4096);
                    let read = {
                        let mut guard = crate::KERNEL_VFS.lock();
                        match guard.as_mut() {
                            Some(vfs) => {
                                vfs.read(vfs_handle, fd_entry.offset, &mut kernel_buf[..to_read])
                            }
                            None => return u64::MAX,
                        }
                    };
                    match read {
                        Ok(n) => {
                            let is_linux = sched.current_process().is_linux_compat;
                            if let Err(error) = crate::memory::user::copy_to_process_bytes(
                                sched.current_process(),
                                hhdm,
                                frame.rsi,
                                &kernel_buf[..n],
                            ) {
                                return user_memory_failure_for(is_linux, error);
                            }
                            if let Some(entry) = sched.current_process_mut().fd_table.get_mut(fd) {
                                entry.offset += n;
                            }
                            n as u64
                        }
                        Err(_) => u64::MAX,
                    }
                } else if fd_entry.handle.is_tty_stdin() {
                    // fd0 wired to a TTY tab's kernel stdin ring. Drain locally —
                    // no IPC to tty_server, so no lock inversion with SCHEDULER.
                    if count == 0 {
                        return 0;
                    }
                    let tab = fd_entry.handle.tty_tab() as usize;
                    let mut kbuf = [0u8; 256];
                    let to_read = count.min(256);
                    let n = crate::process::tty_io::read_stdin(tab, &mut kbuf[..to_read]);
                    if n == 0 {
                        // Return Linux-compatible EAGAIN (errno 11) for compat processes
                        // so musl retries rather than treating it as ENOENT (errno 2).
                        let is_linux = sched.current_process().is_linux_compat;
                        return if is_linux { linux_errno(11) } else { EAGAIN };
                    }
                    let is_linux = sched.current_process().is_linux_compat;
                    if let Err(error) = crate::memory::user::copy_to_process_bytes(
                        sched.current_process(),
                        hhdm,
                        frame.rsi,
                        &kbuf[..n],
                    ) {
                        return user_memory_failure_for(is_linux, error);
                    }
                    n as u64
                } else {
                    // Placeholder stdio fds (0/1/2): return EAGAIN for stdin, error for stdout/stderr
                    let is_linux = sched.current_process().is_linux_compat;
                    match fd {
                        0 => {
                            if is_linux {
                                linux_errno(11)
                            } else {
                                EAGAIN
                            }
                        }
                        _ => 0, // stdout/stderr: invalid for read, silent
                    }
                }
            } else {
                u64::MAX
            }
        }
        Err(_) => {
            crate::serial_println!("[SYSCALL] read fd={} (capability denied)", fd);
            u64::MAX // EACCES
        }
    }
}

/// Syscall: write (43)
/// rdi = fd
/// rsi = buf (user-space pointer)
/// rdx = count
fn sys_write(frame: &mut SyscallFrame) -> u64 {
    const EAGAIN: u64 = u64::MAX - 1;

    let fd = frame.rdi as i32;
    let count = frame.rdx as usize;
    if count == 0 {
        return 0;
    }

    let mut sched = crate::sched::SCHEDULER.lock();
    let hhdm = VirtAddr::new(crate::HHDM_REQ.response().expect("no hhdm").offset);

    // Check if fd is valid and has WRITE right
    match sched.current_process().fd_table.check_rights(
        fd,
        crate::process::fd_table::CapRights::new(crate::process::fd_table::CapRights::WRITE),
    ) {
        Ok(()) => {
            if let Some(&fd_entry) = sched.current_process().fd_table.get(fd) {
                if fd_entry.handle.is_pipe() {
                    let pipe_idx = fd_entry.handle.pipe_index();
                    let write_size = core::cmp::min(count, 4096);
                    let mut kernel_buf = [0u8; 4096];
                    let is_linux = sched.current_process().is_linux_compat;
                    if let Err(error) = crate::memory::user::copy_from_process_bytes(
                        sched.current_process(),
                        hhdm,
                        frame.rsi,
                        &mut kernel_buf[..write_size],
                    ) {
                        return user_memory_failure_for(is_linux, error);
                    }

                    match crate::process::pipe::pipe_write(pipe_idx, &kernel_buf[..write_size]) {
                        crate::process::pipe::PipeResult::Ok(n) => n as u64,
                        crate::process::pipe::PipeResult::WouldBlock => EAGAIN,
                        crate::process::pipe::PipeResult::BrokenPipe => u64::MAX,
                        crate::process::pipe::PipeResult::Eof => u64::MAX,
                    }
                } else if fd_entry.handle.is_vfs() {
                    let vfs_handle = sunlight_fs::vfs::FileHandle(fd_entry.handle.vfs_handle());
                    let write_size = count.min(4096);
                    let mut kernel_buf = [0u8; 4096];
                    let is_linux = sched.current_process().is_linux_compat;
                    if let Err(error) = crate::memory::user::copy_from_process_bytes(
                        sched.current_process(),
                        hhdm,
                        frame.rsi,
                        &mut kernel_buf[..write_size],
                    ) {
                        return user_memory_failure_for(is_linux, error);
                    }
                    let written = {
                        let mut guard = crate::KERNEL_VFS.lock();
                        match guard.as_mut() {
                            Some(vfs) => {
                                vfs.write(vfs_handle, fd_entry.offset, &kernel_buf[..write_size])
                            }
                            None => return u64::MAX,
                        }
                    };
                    match written {
                        Ok(n) => {
                            if let Some(entry) = sched.current_process_mut().fd_table.get_mut(fd) {
                                entry.offset += n;
                            }
                            n as u64
                        }
                        Err(_) => u64::MAX,
                    }
                } else if fd_entry.handle.is_tty_stdout() {
                    // fd1 wired to a TTY tab's kernel stdout ring. tty_server
                    // drains it (TtyStdoutPull) and renders. No serial spam.
                    if count == 0 {
                        return 0;
                    }
                    let tab = fd_entry.handle.tty_tab() as usize;
                    let write_size = count.min(4096);
                    let mut kernel_buf = [0u8; 4096];
                    let is_linux = sched.current_process().is_linux_compat;
                    if let Err(error) = crate::memory::user::copy_from_process_bytes(
                        sched.current_process(),
                        hhdm,
                        frame.rsi,
                        &mut kernel_buf[..write_size],
                    ) {
                        return user_memory_failure_for(is_linux, error);
                    }
                    crate::process::tty_io::write_stdout(tab, &kernel_buf[..write_size]);
                    // Report the full count as written even if the ring dropped
                    // tail bytes under pressure — apps treat short writes as errors.
                    count as u64
                } else {
                    // Handle stdin/stdout/stderr specially
                    match fd {
                        1 | 2 => {
                            // stdout/stderr: write to serial
                            if frame.rsi != 0 {
                                let mut bytes = [0u8; 256];
                                let copy_len = count.min(bytes.len());
                                let is_linux = sched.current_process().is_linux_compat;
                                if let Err(error) = crate::memory::user::copy_from_process_bytes(
                                    sched.current_process(),
                                    hhdm,
                                    frame.rsi,
                                    &mut bytes[..copy_len],
                                ) {
                                    return user_memory_failure_for(is_linux, error);
                                }
                                if let Ok(s) = core::str::from_utf8(&bytes[..copy_len]) {
                                    crate::serial_println!("{}", s);
                                }
                            }
                            count as u64
                        }
                        _ => 0,
                    }
                }
            } else {
                u64::MAX
            }
        }
        Err(_) => {
            crate::serial_println!("[SYSCALL] write fd={} (capability denied)", fd);
            u64::MAX // EACCES
        }
    }
}

/// Syscall: lseek (44)
/// rdi = fd, rsi = offset (i64), rdx = whence (0=SET 1=CUR 2=END)
fn sys_lseek(frame: &mut SyscallFrame) -> u64 {
    use sunlight_fs::vfs::FileHandle as VfsHandle;

    let fd = frame.rdi as i32;
    let offset = frame.rsi as i64;
    let whence = frame.rdx as i32;

    let mut sched = crate::sched::SCHEDULER.lock();

    // Copy the two values we need before releasing the borrow.
    let (current_offset, vfs_handle) = match sched.current_process().fd_table.get(fd) {
        Some(e) if e.handle.is_vfs() => (e.offset, e.handle.vfs_handle()),
        Some(_) => return u64::MAX, // ESPIPE: pipes and TTY fds are not seekable
        None => return u64::MAX,    // EBADF
    };

    match whence {
        // SEEK_SET ─────────────────────────────────────────────────────────
        0 => {
            if offset < 0 {
                return u64::MAX;
            } // EINVAL
            let new_off = offset as usize;
            if let Some(e) = sched.current_process_mut().fd_table.get_mut(fd) {
                e.offset = new_off;
            }
            new_off as u64
        }
        // SEEK_CUR ─────────────────────────────────────────────────────────
        1 => {
            match (current_offset as i64).checked_add(offset) {
                Some(v) if v >= 0 => {
                    let new_off = v as usize;
                    if let Some(e) = sched.current_process_mut().fd_table.get_mut(fd) {
                        e.offset = new_off;
                    }
                    new_off as u64
                }
                _ => u64::MAX, // EINVAL: would underflow
            }
        }
        // SEEK_END ─────────────────────────────────────────────────────────
        // Must release SCHEDULER before taking KERNEL_VFS (lock-order rule).
        2 => {
            drop(sched);

            let file_size = {
                let mut guard = crate::KERNEL_VFS.lock();
                match guard.as_mut() {
                    Some(vfs) => match vfs.fstat_handle(VfsHandle(vfs_handle)) {
                        Ok(s) => s.size,
                        Err(_) => return u64::MAX,
                    },
                    None => return u64::MAX,
                }
            };

            match (file_size as i64).checked_add(offset) {
                Some(v) if v >= 0 => {
                    let new_off = v as usize;
                    let mut sched2 = crate::sched::SCHEDULER.lock();
                    if let Some(e) = sched2.current_process_mut().fd_table.get_mut(fd) {
                        e.offset = new_off;
                    }
                    new_off as u64
                }
                _ => u64::MAX, // EINVAL: negative resulting offset
            }
        }
        _ => u64::MAX, // EINVAL: unknown whence
    }
}

fn sys_dup(_frame: &mut SyscallFrame) -> u64 {
    crate::serial_println!("[SYSCALL] dup requested");
    u64::MAX
}

fn sys_dup2(_frame: &mut SyscallFrame) -> u64 {
    crate::serial_println!("[SYSCALL] dup2 requested");
    u64::MAX
}

/// Syscall: pipe (47)
/// rdi = pointer to int[2] array for (read_fd, write_fd)
fn sys_pipe(frame: &mut SyscallFrame) -> u64 {
    if crate::memory::user::validate_current_write(frame.rdi, 8).is_err() {
        return u64::MAX; // EFAULT
    }

    let mut sched = crate::sched::SCHEDULER.lock();
    let mut pmm = crate::PMM.lock();

    match crate::process::pipe::create_pipe(&mut pmm, &mut sched) {
        Ok((read_fd, write_fd)) => {
            let mut output = [0u8; 8];
            output[..4].copy_from_slice(&read_fd.to_ne_bytes());
            output[4..].copy_from_slice(&write_fd.to_ne_bytes());
            let process = sched.current_process();
            let hhdm = VirtAddr::new(crate::HHDM_REQ.response().expect("no hhdm").offset);
            if crate::memory::user::copy_to_process_bytes(process, hhdm, frame.rdi, &output)
                .is_err()
            {
                return u64::MAX;
            }
            0 // Success
        }
        Err(_) => u64::MAX,
    }
}

/// Syscall: fstat (48)
/// rdi = fd, rsi = pointer to stat buffer in user space.
///
/// For native SunlightOS processes: 24-byte custom layout
///   [0..8]  size u64, [8..12] uid u32, [12..16] gid u32,
///   [16..18] mode u16, [18] file_type u8, [19] pad, [20..24] nlinks u32.
///
/// For Linux-compat processes: standard Linux x86_64 struct stat (144 bytes)
///   [0..8]   st_dev, [8..16] st_ino, [16..24] st_nlink,
///   [24..28] st_mode, [28..32] st_uid, [32..36] st_gid, [36..40] __pad0,
///   [40..48] st_rdev, [48..56] st_size, [56..64] st_blksize, [64..72] st_blocks,
///   [72..120] timestamps (atim/mtim/ctim), [120..144] __unused
fn sys_fstat(frame: &mut SyscallFrame) -> u64 {
    use sunlight_fs::vfs::{FileHandle as VfsHandle, FileType};

    let fd = frame.rdi as i32;
    let buf_ptr = frame.rsi;

    let is_linux = crate::sched::with_scheduler(|s| s.current_process().is_linux_compat);
    // Read vfs_handle from fd table; release scheduler before taking VFS lock.
    let vfs_handle = {
        let sched = crate::sched::SCHEDULER.lock();
        match sched.current_process().fd_table.get(fd) {
            Some(e) if e.handle.is_vfs() => e.handle.vfs_handle(),
            Some(_) => return u64::MAX, // ESPIPE
            None => return u64::MAX,    // EBADF
        }
    };

    let stat = {
        let mut guard = crate::KERNEL_VFS.lock();
        let Some(vfs) = guard.as_mut() else {
            return u64::MAX;
        };
        match vfs.fstat_handle(VfsHandle(vfs_handle)) {
            Ok(s) => s,
            Err(_) => return u64::MAX,
        }
    };

    if is_linux {
        // Linux x86_64 struct stat (144 bytes) — field offsets per ABI.
        let mut record = [0u8; 144];
        // st_dev at 0 (fake device 1)
        record[0..8].copy_from_slice(&1u64.to_le_bytes());
        // st_ino at 8 (use vfs handle as proxy inode)
        record[8..16].copy_from_slice(&(vfs_handle as u64).to_le_bytes());
        // st_nlink at 16
        record[16..24].copy_from_slice(&(stat.nlinks as u64).to_le_bytes());
        // st_mode at 24: set file-type bits S_IFREG/S_IFDIR and permission bits
        let linux_mode: u32 = match stat.file_type {
            FileType::File => 0o100000 | (stat.mode as u32 & 0o7777),
            FileType::Directory => 0o040000 | (stat.mode as u32 & 0o7777),
        };
        record[24..28].copy_from_slice(&linux_mode.to_le_bytes());
        // st_uid at 28, st_gid at 32
        record[28..32].copy_from_slice(&stat.uid.to_le_bytes());
        record[32..36].copy_from_slice(&stat.gid.to_le_bytes());
        // st_rdev at 40: 0 (not a device)
        // st_size at 48
        record[48..56].copy_from_slice(&(stat.size as u64).to_le_bytes());
        // st_blksize at 56: 4096
        record[56..64].copy_from_slice(&4096u64.to_le_bytes());
        // st_blocks at 64: in 512-byte units
        let blocks = (stat.size as u64 + 511) / 512;
        record[64..72].copy_from_slice(&blocks.to_le_bytes());
        // timestamps (atim/mtim/ctim) at 72/88/104: zero (no time tracking yet)
        if let Err(error) = copy_to_user(buf_ptr, &record) {
            return error;
        }
    } else {
        let mut record = [0u8; 24];
        record[0..8].copy_from_slice(&(stat.size as u64).to_le_bytes());
        record[8..12].copy_from_slice(&stat.uid.to_le_bytes());
        record[12..16].copy_from_slice(&stat.gid.to_le_bytes());
        record[16..18].copy_from_slice(&stat.mode.to_le_bytes());
        record[18] = match stat.file_type {
            FileType::File => 1,
            FileType::Directory => 2,
        };
        record[20..24].copy_from_slice(&stat.nlinks.to_le_bytes());
        if let Err(error) = copy_to_user(buf_ptr, &record) {
            return error;
        }
    }
    0
}

/// Syscall: fcntl (49)
/// rdi = fd, rsi = cmd, rdx = arg
///
/// Minimal implementation covering the commands musl/libc issue during
/// startup and stdio setup. Returning -1 for everything (as the old stub
/// did) made `read_to_string` and friends believe the descriptor was
/// broken, so files appeared empty.
fn sys_fcntl(frame: &mut SyscallFrame) -> u64 {
    // Linux fcntl command numbers.
    const F_DUPFD: u64 = 0;
    const F_GETFD: u64 = 1;
    const F_SETFD: u64 = 2;
    const F_GETFL: u64 = 3;
    const F_SETFL: u64 = 4;
    const F_DUPFD_CLOEXEC: u64 = 1030;

    let fd = frame.rdi as i32;
    let cmd = frame.rsi;

    let mut sched = crate::sched::SCHEDULER.lock();
    let proc = sched.current_process_mut();

    // Validate the descriptor exists; read its open flags for F_GETFL.
    let open_flags = match proc.fd_table.get(fd) {
        Some(desc) => desc.flags,
        None => return u64::MAX, // EBADF
    };

    match cmd {
        // No CLOEXEC tracking yet: report cleared and accept sets as no-ops.
        F_GETFD => 0,
        F_SETFD => 0,
        // Access mode lives in the low bits of `flags` (O_RDONLY=0,
        // O_WRONLY=1, O_RDWR=2), matching Linux's O_ACCMODE encoding.
        F_GETFL => open_flags as u64,
        // We don't honour O_NONBLOCK/O_APPEND changes; accept silently.
        F_SETFL => 0,
        // Duplication of descriptors isn't supported yet.
        F_DUPFD | F_DUPFD_CLOEXEC => {
            crate::serial_println!("[SYSCALL] fcntl: F_DUPFD unsupported (fd={})", fd);
            u64::MAX
        }
        other => {
            crate::serial_println!("[SYSCALL] fcntl: unsupported cmd={} (fd={})", other, fd);
            u64::MAX
        }
    }
}

// ---------------------------------------------------------------------------
// Phase 4.1: Memory management syscalls
// ---------------------------------------------------------------------------

/// Syscall: mmap (50)
/// rdi = addr (hint, 0 = kernel chooses)
/// rsi = length
/// rdx = prot (PROT_READ | PROT_WRITE | PROT_EXEC)
/// r10 = flags (MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED)
/// r8 = fd (-1 for anonymous)
/// r9 = offset
fn sys_mmap(frame: &mut SyscallFrame) -> u64 {
    let addr = frame.rdi;
    let length = frame.rsi;
    let prot = frame.rdx as u32;
    let flags = frame.r10 as u32;
    let fd = frame.r8 as i32;
    let offset = frame.r9;

    let mut sched = crate::sched::SCHEDULER.lock();
    let mut pmm = crate::PMM.lock();

    match crate::process::mmap::sys_mmap(
        addr,
        length,
        prot,
        flags,
        fd,
        offset,
        &mut *pmm,
        &mut *sched,
    ) {
        Ok(mapped_addr) => {
            crate::serial_println!(
                "[SYSCALL] mmap({:#x}, {:#x}) -> {:#x}",
                addr,
                length,
                mapped_addr
            );
            mapped_addr
        }
        Err(error) => {
            crate::serial_println!("[SYSCALL] mmap failed ({:#x}, {:#x})", addr, length);
            if crate::sched::with_scheduler(|sched| sched.current_process().is_linux_compat) {
                match error {
                    crate::process::mmap::MmapError::PermissionDenied => linux_errno(13),
                    crate::process::mmap::MmapError::NoMemory => linux_errno(12),
                    crate::process::mmap::MmapError::AlreadyMapped => linux_errno(17),
                    _ => linux_errno(22),
                }
            } else {
                u64::MAX
            }
        }
    }
}

/// Syscall: munmap (51)
/// rdi = addr
/// rsi = length
fn sys_munmap(frame: &mut SyscallFrame) -> u64 {
    let addr = frame.rdi;
    let length = frame.rsi;

    let mut sched = crate::sched::SCHEDULER.lock();
    let linux_compat = sched.current_process().is_linux_compat;
    let mut pmm = crate::PMM.lock();
    let result = crate::process::mmap::sys_munmap(addr, length, &mut pmm, &mut sched);
    drop(pmm);
    drop(sched);

    match result {
        Ok(()) => 0,
        Err(error) if linux_compat => {
            linux_errno(crate::process::mmap::munmap_linux_errno(error) as u64)
        }
        Err(_) => u64::MAX,
    }
}

/// Syscall: mprotect (52)
/// rdi = addr
/// rsi = length
/// rdx = prot (PROT_READ | PROT_WRITE | PROT_EXEC)
fn sys_mprotect(frame: &mut SyscallFrame) -> u64 {
    let addr = frame.rdi;
    let length = frame.rsi;
    let prot = frame.rdx as u32;

    let mut sched = crate::sched::SCHEDULER.lock();
    let linux_compat = sched.current_process().is_linux_compat;
    let pmm = crate::PMM.lock();
    let result = crate::process::mmap::sys_mprotect(addr, length, prot, &pmm, &mut sched);
    drop(pmm);
    drop(sched);

    match result {
        Ok(()) => 0,
        Err(error) if linux_compat => {
            linux_errno(crate::process::mmap::mprotect_linux_errno(error) as u64)
        }
        Err(_) => u64::MAX,
    }
}

/// Syscall: mremap (53)
/// rdi = old_addr
/// rsi = old_size
/// rdx = new_size
/// r10 = flags
fn sys_mremap(frame: &mut SyscallFrame) -> u64 {
    let old_addr = frame.rdi;
    let old_size = frame.rsi;
    let new_size = frame.rdx;
    let flags = frame.r10 as u32;

    match crate::process::mmap::sys_mremap(old_addr, old_size, new_size, flags) {
        Ok(addr) => addr,
        Err(_) => u64::MAX,
    }
}

// ---------------------------------------------------------------------------
// Phase 4.3: Signal handling syscalls
// ---------------------------------------------------------------------------

/// Syscall: sigaction (70)
/// rdi = signal number
/// rsi = pointer to new sigaction
/// rdx = pointer to old sigaction
fn sys_sigaction(_frame: &mut SyscallFrame) -> u64 {
    crate::serial_println!("[SYSCALL] sigaction requested");
    u64::MAX
}

/// Syscall: sigprocmask (71)
/// rdi = how (SIG_BLOCK, SIG_UNBLOCK, SIG_SETMASK)
/// rsi = pointer to new mask
/// rdx = pointer to old mask
fn sys_sigprocmask(_frame: &mut SyscallFrame) -> u64 {
    crate::serial_println!("[SYSCALL] sigprocmask requested");
    u64::MAX
}

/// Syscall: kill (72)
/// rdi = pid
/// rsi = signal number
fn sys_kill(frame: &mut SyscallFrame) -> u64 {
    let pid = frame.rdi as usize;
    let sig = frame.rsi as u32;

    if pid == 0 {
        return u64::MAX;
    }
    if sig == 0 {
        return crate::sched::SCHEDULER.lock().processes.iter().any(|p| {
            p.pid == pid && !matches!(p.state, ProcessState::Finished | ProcessState::Reaped)
        }) as u64;
    }

    let Some(signal) = crate::process::signal::Signal::try_from_u32(sig) else {
        return u64::MAX;
    };
    match send_signal(pid, signal) {
        Ok(()) => 0,
        Err(()) => u64::MAX,
    }
}

/// Syscall: pause (73)
/// Sleep until a signal is delivered
fn sys_pause() -> u64 {
    crate::serial_println!("[SYSCALL] pause requested");
    u64::MAX
}

/// Syscall: sigreturn (74)
/// Return from signal handler
fn sys_sigreturn(_frame: &mut SyscallFrame) -> u64 {
    crate::serial_println!("[SYSCALL] sigreturn requested");
    u64::MAX
}

/// Syscall: net_tx (90) — Phase 3.4 frame proxy.
/// rdi = user pointer to a raw Ethernet frame, rsi = frame length.
/// Restricted to net_server (gated by process name; pid is no longer fixed
/// since init launches it after timer_server), which holds the network capability.
/// Returns 1 on success, 0 on failure (no device / send error), u64::MAX
/// if the calling process is not authorized or the buffer is invalid.
fn sys_net_tx(frame: &mut SyscallFrame) -> u64 {
    const MAX_FRAME: usize = 1514;

    let len = (frame.rsi as usize).min(MAX_FRAME);

    if crate::sched::SCHEDULER.lock().current_process().name_str() != "net_server" {
        return u64::MAX;
    }
    let mut kernel_buf = [0u8; MAX_FRAME];
    if let Err(error) = copy_from_user(frame.rdi, &mut kernel_buf[..len]) {
        return error;
    }

    let mut dev = crate::ACTIVE_NET_DEVICE.lock();
    let Some(device) = dev.as_mut() else {
        return 0;
    };
    if len < 14 || kernel_buf[6..12] != device.mac() {
        crate::serial_println!("[NET] TX rejected: frame source MAC does not match active backend");
        return 0;
    }
    let before = device.counters();
    let backend_kind = device.kind();
    // SAFETY: ACTIVE_NET_DEVICE holds the sole Layer-2 backend, initialized once at
    // boot with valid ring-0 mapped queues; the mutex serializes access.
    let result = match unsafe { device.send(&kernel_buf[..len]) } {
        Ok(()) => 1,
        Err(_) => 0,
    };
    crate::telemetry::record_net_tx(len as u64);
    let after = device.counters();
    if backend_kind == sunlight_net::NetworkBackendKind::Vmxnet3
        && before.tx_submitted == 0
        && after.tx_submitted != 0
    {
        crate::serial_println!("[VMXNET3] first TX frame submitted");
        if let Some(desc) = device.first_tx_descriptor() {
            crate::serial_println!(
                "[VMXNET3] TX desc index={} dma={:#x} len={} flags={:#x} gen={} producer={}",
                desc.index,
                desc.dma_address,
                desc.length,
                desc.flags,
                desc.generation,
                desc.producer
            );
        }
    }
    match dhcp_message_type(&kernel_buf[..len]) {
        Some(1) => {
            crate::serial_println!("[DHCP] discover sent");
        }
        Some(3) => {
            crate::serial_println!("[DHCP] request sent");
        }
        _ => {}
    }
    result
}

/// Syscall: net_rx (91) — Phase 3.4 frame proxy.
/// rdi = user pointer to a buffer, rsi = buffer capacity.
/// Restricted to net_server (gated by process name). Returns the number of bytes copied
/// (0 if no frame is pending or the device is absent), or u64::MAX if the
/// calling process is not authorized or the buffer is invalid.
fn sys_net_rx(frame: &mut SyscallFrame) -> u64 {
    const MAX_FRAME: usize = 1514;

    let cap = (frame.rsi as usize).min(MAX_FRAME);

    if crate::sched::SCHEDULER.lock().current_process().name_str() != "net_server" {
        return u64::MAX;
    }
    if let Err(error) = crate::memory::user::validate_current_write(frame.rdi, cap) {
        return user_memory_failure(error);
    }

    let mut kernel_buf = [0u8; MAX_FRAME];
    let (n, backend_kind, before, after, first_rx) = {
        let mut dev = crate::ACTIVE_NET_DEVICE.lock();
        match dev.as_mut() {
            // SAFETY: see sys_net_tx — single NIC backend, mutex-serialized.
            Some(d) => {
                let before = d.counters();
                let n = unsafe { d.recv(&mut kernel_buf) };
                (n, Some(d.kind()), before, d.counters(), d.first_rx())
            }
            None => (
                0,
                None,
                sunlight_net::NetDeviceCounters::default(),
                sunlight_net::NetDeviceCounters::default(),
                None,
            ),
        }
    };
    if backend_kind == Some(sunlight_net::NetworkBackendKind::Vmxnet3) {
        if before.tx_completed == 0 && after.tx_completed != 0 {
            crate::serial_println!("[VMXNET3] first TX completion");
        }
        if before.rx_completed == 0 && after.rx_completed != 0 {
            crate::serial_println!("[VMXNET3] first RX completion");
        }
        if after.polls != 0
            && after.polls % 1024 == 0
            && !DHCP_ACK_SEEN.load(core::sync::atomic::Ordering::Relaxed)
        {
            crate::serial_println!(
                "[VMXNET3] tx_submitted={} tx_completed={} rx_completed={} rx_delivered={} irq={} polls={}",
                after.tx_submitted, after.tx_completed, after.rx_completed,
                after.rx_delivered, after.interrupts, after.polls
            );
        }
    }
    let n = n.min(cap);
    if n > 0 {
        if let Err(error) = copy_to_user(frame.rdi, &kernel_buf[..n]) {
            return error;
        }
        crate::telemetry::record_net_rx(n as u64);
        if backend_kind == Some(sunlight_net::NetworkBackendKind::Vmxnet3)
            && before.rx_delivered == 0
            && after.rx_delivered != 0
        {
            if let Some((length, ethertype)) = first_rx {
                crate::serial_println!(
                    "[VMXNET3] first RX frame length={} ethertype={:#06x}",
                    length,
                    ethertype
                );
            }
            crate::serial_println!("[NET] first VMXNET3 frame delivered to frame proxy");
        }
        match dhcp_message_type(&kernel_buf[..n]) {
            Some(2) => {
                crate::serial_println!("[DHCP] offer received");
            }
            Some(5) => {
                DHCP_ACK_SEEN.store(true, core::sync::atomic::Ordering::Relaxed);
                crate::serial_println!("[DHCP] ACK received");
            }
            _ => {}
        }
    }
    n as u64
}

fn sys_net_info(frame: &mut SyscallFrame) -> u64 {
    const INFO_WORDS: usize = 19;
    let (authorized, may_mark_frame_proxy, may_mark_interface) = {
        let scheduler = crate::sched::SCHEDULER.lock();
        let name = scheduler.current_process().name_str();
        (
            matches!(name, "net_server" | "networkd" | "networkctl"),
            name == "net_server",
            name == "networkd",
        )
    };
    if !authorized {
        return u64::MAX;
    }
    if frame.rsi as usize != INFO_WORDS {
        return u64::MAX;
    }
    let device = crate::ACTIVE_NET_DEVICE.lock();
    let Some(device) = device.as_ref() else {
        let info = [
            0,
            0,
            0,
            0,
            crate::NET_BACKEND_STATE.load(core::sync::atomic::Ordering::Acquire),
            crate::NET_BACKEND_ERROR.load(core::sync::atomic::Ordering::Acquire),
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            crate::VMXNET3_INIT_STAGE.load(core::sync::atomic::Ordering::Acquire),
            crate::VMXNET3_FAILURE_STAGE.load(core::sync::atomic::Ordering::Acquire),
            crate::VMXNET3_ERROR_DETAIL.load(core::sync::atomic::Ordering::Acquire),
        ];
        let bytes =
            unsafe { core::slice::from_raw_parts(info.as_ptr() as *const u8, INFO_WORDS * 8) };
        if let Err(error) = copy_to_user(frame.rdi, bytes) {
            return error;
        }
        return 1;
    };
    if device.kind() == sunlight_net::NetworkBackendKind::Vmxnet3 {
        let requested_event = frame.rdx;
        if requested_event == sunlight_ipc::NetBackendEvent::FrameProxyRegistered as u64
            && may_mark_frame_proxy
        {
            crate::vmxnet3_transition(sunlight_ipc::Vmxnet3InitStage::FrameProxyRegistered);
            let mac = device.mac();
            crate::serial_println!(
                "[NET] generic frame backend registered kind=VMXNET3 mac={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
            );
        } else if requested_event == sunlight_ipc::NetBackendEvent::InterfacePublished as u64
            && may_mark_interface
        {
            crate::vmxnet3_transition(sunlight_ipc::Vmxnet3InitStage::InterfacePublished);
        }
    }
    let counters = device.counters();
    let mac = device.mac();
    let mut packed = [0u8; 8];
    packed[..6].copy_from_slice(&mac);
    let info = [
        u64::from_le_bytes(packed),
        1 | ((device.link_up() as u64) << 1),
        device.kind() as u64,
        device.mtu() as u64,
        crate::NET_BACKEND_STATE.load(core::sync::atomic::Ordering::Acquire),
        crate::NET_BACKEND_ERROR.load(core::sync::atomic::Ordering::Acquire),
        counters.tx_submitted,
        counters.tx_completed,
        counters.tx_bytes,
        counters.rx_completed,
        counters.rx_delivered,
        counters.rx_bytes,
        counters.tx_ring_full,
        counters.rx_bad_completion,
        counters.interrupts,
        counters.polls,
        crate::VMXNET3_INIT_STAGE.load(core::sync::atomic::Ordering::Acquire),
        crate::VMXNET3_FAILURE_STAGE.load(core::sync::atomic::Ordering::Acquire),
        crate::VMXNET3_ERROR_DETAIL.load(core::sync::atomic::Ordering::Acquire),
    ];
    let bytes = unsafe { core::slice::from_raw_parts(info.as_ptr() as *const u8, INFO_WORDS * 8) };
    if let Err(error) = copy_to_user(frame.rdi, bytes) {
        return error;
    }
    1
}

fn sys_hardware_inventory(frame: &mut SyscallFrame) -> u64 {
    if crate::sched::SCHEDULER.lock().current_process().name_str() != "deviced" {
        return u64::MAX;
    }
    if frame.rdx as usize != core::mem::size_of::<::sunlight_ipc::HardwareInventoryRecord>() {
        return u64::MAX;
    }
    let Some((record, total)) = crate::hardware_inventory::snapshot(frame.rdi as usize) else {
        return u64::MAX;
    };
    let bytes = unsafe {
        core::slice::from_raw_parts(
            &record as *const ::sunlight_ipc::HardwareInventoryRecord as *const u8,
            core::mem::size_of::<::sunlight_ipc::HardwareInventoryRecord>(),
        )
    };
    if let Err(error) = copy_to_user(frame.rsi, bytes) {
        return error;
    }
    total as u64
}

static DHCP_ACK_SEEN: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// Return DHCP option 53 for an unfragmented Ethernet/IPv4/UDP BOOTP frame.
/// This deliberately inspects protocol headers only and never logs payloads.
fn dhcp_message_type(frame: &[u8]) -> Option<u8> {
    if frame.len() < 14 + 20 + 8 || frame[12..14] != [0x08, 0x00] {
        return None;
    }
    let ip = 14;
    let ihl = ((frame[ip] & 0x0f) as usize) * 4;
    if ihl < 20 || frame.len() < ip + ihl + 8 || frame[ip + 9] != 17 {
        return None;
    }
    let udp = ip + ihl;
    let src = u16::from_be_bytes([frame[udp], frame[udp + 1]]);
    let dst = u16::from_be_bytes([frame[udp + 2], frame[udp + 3]]);
    if !((src == 68 && dst == 67) || (src == 67 && dst == 68)) {
        return None;
    }
    let mut option = udp + 8 + 240;
    while option < frame.len() {
        let code = frame[option];
        option += 1;
        if code == 0 {
            continue;
        }
        if code == 255 || option >= frame.len() {
            return None;
        }
        let len = frame[option] as usize;
        option += 1;
        if option + len > frame.len() {
            return None;
        }
        if code == 53 && len == 1 {
            return Some(frame[option]);
        }
        option += len;
    }
    None
}

fn sys_shm_alloc(frame: &mut SyscallFrame) -> u64 {
    let size = frame.rdi as usize; // 0 => 4KiB (compat); >0 for multi-page regions
    let Some(hhdm) = crate::HHDM_REQ.response() else {
        return u64::MAX;
    };
    let hhdm_offset = VirtAddr::new(hhdm.offset);
    let mut sched = crate::sched::SCHEDULER.lock();
    let mut pmm = crate::PMM.lock();
    let mut caps = crate::capability::CAP_BROKER.lock();
    let process = sched.current_process_mut();
    match crate::memory::shared::alloc_shared_region(
        process,
        &mut *pmm,
        &mut *caps,
        hhdm_offset,
        size,
    ) {
        Ok((virt, token)) => {
            // Preserve legacy message for test expectations when using the 1-page path
            if size == 0 || size == 4096 {
                crate::serial_println!("[SHM]  alloc_shared_page: OK");
            } else {
                crate::serial_println!("[SHM]  alloc_shared_region: OK ({} bytes)", size);
            }
            // Return virt in rax, token in rdx + r13 (for direct callers and ipc raw_syscall caps[0])
            frame.rdx = token.0;
            frame.r13 = token.0;
            virt.as_u64()
        }
        Err(_) => u64::MAX,
    }
}

fn sys_shm_map(frame: &mut SyscallFrame) -> u64 {
    let token = crate::capability::CapabilityToken(frame.rdi);
    // rsi may carry prot in future (SHM_READ etc); currently always RW for the mapping
    let Some(hhdm) = crate::HHDM_REQ.response() else {
        return u64::MAX;
    };
    let hhdm_offset = VirtAddr::new(hhdm.offset);
    let mut sched = crate::sched::SCHEDULER.lock();
    let mut pmm = crate::PMM.lock();
    let mut caps = crate::capability::CAP_BROKER.lock();
    let process = sched.current_process_mut();
    match crate::memory::shared::map_shared_page(process, token, &mut *pmm, &mut *caps, hhdm_offset)
    {
        Ok(virt) => {
            crate::serial_println!("[SHM]  map_shared_page: OK");
            virt.as_u64()
        }
        Err(_) => u64::MAX,
    }
}

fn sys_shm_free(frame: &mut SyscallFrame) -> u64 {
    let token = crate::capability::CapabilityToken(frame.rdi);
    let Some(hhdm) = crate::HHDM_REQ.response() else {
        return u64::MAX;
    };
    let hhdm_offset = VirtAddr::new(hhdm.offset);
    let mut sched = crate::sched::SCHEDULER.lock();
    let mut pmm = crate::PMM.lock();
    let mut caps = crate::capability::CAP_BROKER.lock();
    let process = sched.current_process_mut();
    match crate::memory::shared::free_shared_page(
        process,
        token,
        &mut *pmm,
        &mut *caps,
        hhdm_offset,
    ) {
        Ok(()) => {
            crate::serial_println!("[SHM]  shm_free: page unmapped OK");
            0
        }
        Err(_) => u64::MAX,
    }
}

/// Map the physical Limine framebuffer into the current process for the GUI compositor.
/// Returns user virtual address of the start of the framebuffer (or 0 on failure).
/// The caller can read fb dimensions via other means or we pack extra info in rdx/rcx etc.
fn sys_map_framebuffer(frame: &mut SyscallFrame) -> u64 {
    let Some(hhdm) = crate::HHDM_REQ.response() else {
        return u64::MAX;
    };
    let hhdm_offset = VirtAddr::new(hhdm.offset);

    let mut sched = crate::sched::SCHEDULER.lock();
    let caller_pid = sched.current_process().pid;
    {
        let caps = crate::capability::CAP_BROKER.lock();
        if !crate::memory::security::framebuffer_authorized(caller_pid, &caps) {
            crate::memory::security::note_framebuffer_mapping_rejected();
            return u64::MAX;
        }
    }

    let fb_resp = crate::FB_REQ.response();
    let fb = match fb_resp.and_then(|r| r.framebuffers().first()) {
        Some(f) => f,
        None => return 0,
    };

    let fb_addr = fb.address() as u64;
    let fb_pitch = fb.pitch as u64;
    let fb_height = fb.height as u64;
    let fb_width = fb.width as u32;
    let fb_bpp = fb.bpp as u32; // usually 32

    let hhdm = hhdm_offset.as_u64();
    let fb_phys_base = if fb_addr >= hhdm {
        fb_addr - hhdm
    } else {
        fb_addr
    };
    let fb_page_offset = fb_phys_base & 0xfff;
    let bytes_per_row = fb_pitch;
    let total_bytes = match bytes_per_row
        .checked_mul(fb_height)
        .and_then(|bytes| bytes.checked_add(fb_page_offset))
    {
        Some(bytes) => bytes,
        None => return 0,
    };
    let page_count = match total_bytes.checked_add(4095) {
        Some(rounded) => rounded / 4096,
        None => return 0,
    };

    let mut pmm = crate::PMM.lock();
    let process = sched.current_process_mut();

    const DISPLAY_FB_VADDR: u64 = 0x0000_0004_0000_0000; // dedicated region for device FB

    let protection = crate::process::region::RegionProtection::READ_WRITE;
    let flags =
        match crate::process::address_space::AddressSpace::protection_to_pte_flags(protection) {
            Ok(flags) => flags,
            Err(_) => return 0,
        };

    for page_idx in 0..page_count {
        let Some(user_va) = DISPLAY_FB_VADDR.checked_add(page_idx * 4096) else {
            return 0;
        };
        let Ok(user_page) = Page::<Size4KiB>::from_start_address(VirtAddr::new(user_va)) else {
            return 0;
        };
        if unsafe { process.address_space.is_occupied(user_page, hhdm_offset) } {
            crate::process::address_space::note_mapping_collision();
            return u64::MAX;
        }
    }

    let Some(region_end) = DISPLAY_FB_VADDR.checked_add(page_count * 4096) else {
        return 0;
    };
    let region = match crate::process::region::MappingRegion::new(
        DISPLAY_FB_VADDR,
        region_end,
        protection,
        crate::process::region::MappingKind::Framebuffer,
        crate::process::region::RegionPolicy::SYSTEM
            .union(crate::process::region::RegionPolicy::OWNER_MANAGED),
        crate::process::region::RegionBacking::Internal(3),
    ) {
        Ok(region) => region,
        Err(_) => return 0,
    };
    let reservation = match process.address_space.preflight_region(region) {
        Ok(reservation) => reservation,
        Err(_) => return u64::MAX,
    };

    for page_idx in 0..page_count {
        let user_va = DISPLAY_FB_VADDR + page_idx * 4096;
        let user_page = match Page::<Size4KiB>::from_start_address(VirtAddr::new(user_va)) {
            Ok(p) => p,
            Err(_) => return 0,
        };
        let phys = (fb_phys_base & !0xfff) + page_idx * 4096;
        let fb_frame = unsafe { PhysFrame::from_start_address_unchecked(PhysAddr::new(phys)) };
        if unsafe {
            process
                .address_space
                .map_page(user_page, fb_frame, flags, &mut *pmm, hhdm_offset)
        }
        .is_err()
        {
            for rollback_idx in (0..page_idx).rev() {
                let Ok(rollback_page) = Page::<Size4KiB>::from_start_address(VirtAddr::new(
                    DISPLAY_FB_VADDR + rollback_idx * 4096,
                )) else {
                    continue;
                };
                let rollback_phys = PhysAddr::new((fb_phys_base & !0xfff) + rollback_idx * 4096);
                let _ = unsafe {
                    process.address_space.rollback_mapped_page(
                        rollback_page,
                        rollback_phys,
                        &mut *pmm,
                        hhdm_offset,
                    )
                };
            }
            process.address_space.cancel_region(reservation);
            return u64::MAX;
        }
    }

    if process.address_space.commit_region(reservation).is_err() {
        for rollback_idx in (0..page_count).rev() {
            let rollback_page = Page::<Size4KiB>::from_start_address(VirtAddr::new(
                DISPLAY_FB_VADDR + rollback_idx * 4096,
            ))
            .expect("validated framebuffer rollback page");
            let rollback_phys = PhysAddr::new((fb_phys_base & !0xfff) + rollback_idx * 4096);
            let _ = unsafe {
                process.address_space.rollback_mapped_page(
                    rollback_page,
                    rollback_phys,
                    &mut *pmm,
                    hhdm_offset,
                )
            };
        }
        return u64::MAX;
    }

    // Return base VA in rax.
    // Pack info into registers that raw_syscall surfaces in the synthetic IpcMsg words:
    // words[0] (r8) = width | (height << 32)
    // words[1] (r9) = pitch
    // words[2] (r10) = bpp
    frame.r8 = (fb_width as u64) | ((fb_height as u64) << 32);
    frame.r9 = fb_pitch;
    frame.r10 = fb_bpp as u64;

    (DISPLAY_FB_VADDR + fb_page_offset) as u64
}

fn sys_map_telemetry(_frame: &mut SyscallFrame) -> u64 {
    let Some(hhdm) = crate::HHDM_REQ.response() else {
        return 0;
    };
    let hhdm_offset = VirtAddr::new(hhdm.offset);

    const PAGE_SIZE: u64 = 4096;

    let telemetry_virt =
        x86_64::VirtAddr::from_ptr(core::ptr::addr_of!(crate::telemetry::TELEMETRY));
    let telemetry_page = x86_64::structures::paging::Page::containing_address(telemetry_virt);

    let kernel_pml4 = x86_64::registers::control::Cr3::read().0.start_address();
    let kernel_as = crate::process::address_space::AddressSpace::from_pml4(kernel_pml4);
    // SAFETY: we walk the currently active kernel page tables via a valid HHDM mapping.
    let telemetry_phys = match unsafe { kernel_as.lookup_phys(telemetry_page, hhdm_offset) } {
        Some(p) => p,
        None => return 0,
    };
    let telemetry_phys_page = telemetry_phys.as_u64() & !0xFFF;
    let telemetry_page_off = telemetry_virt.as_u64() & 0xFFF;
    let telemetry_bytes = core::mem::size_of::<crate::telemetry::TelemetryPage>() as u64;
    let telemetry_pages = telemetry_page_off
        .checked_add(telemetry_bytes)
        .and_then(|bytes| bytes.checked_add(PAGE_SIZE - 1))
        .map(|bytes| bytes / PAGE_SIZE)
        .filter(|pages| *pages > 0)
        .unwrap_or(0);
    if telemetry_pages == 0 {
        return 0;
    }

    let user_addr = x86_64::VirtAddr::new(0x0000_0000_0080_0000);
    let mut sched = crate::sched::SCHEDULER.lock();
    let mut pmm = crate::PMM.lock();
    let process = sched.current_process_mut();
    let protection = crate::process::region::RegionProtection::READ_ONLY;
    let flags =
        match crate::process::address_space::AddressSpace::protection_to_pte_flags(protection) {
            Ok(flags) => flags,
            Err(_) => return 0,
        };

    for i in 0..telemetry_pages {
        let Ok(page) =
            x86_64::structures::paging::Page::from_start_address(user_addr + i * PAGE_SIZE)
        else {
            return 0;
        };
        if unsafe { process.address_space.is_occupied(page, hhdm_offset) } {
            crate::process::address_space::note_mapping_collision();
            return 0;
        }
    }

    let region = match crate::process::region::MappingRegion::new(
        user_addr.as_u64(),
        user_addr.as_u64() + telemetry_pages * PAGE_SIZE,
        protection,
        crate::process::region::MappingKind::Telemetry,
        crate::process::region::RegionPolicy::SYSTEM
            .union(crate::process::region::RegionPolicy::OWNER_MANAGED),
        crate::process::region::RegionBacking::Internal(4),
    ) {
        Ok(region) => region,
        Err(_) => return 0,
    };
    let reservation = match process.address_space.preflight_region(region) {
        Ok(reservation) => reservation,
        Err(_) => return 0,
    };

    // SAFETY: mapping user-visible read-only pages into current process page tables.
    for i in 0..telemetry_pages {
        let user_page =
            match x86_64::structures::paging::Page::from_start_address(user_addr + i * PAGE_SIZE) {
                Ok(p) => p,
                Err(_) => return 0,
            };
        let phys_frame = unsafe {
            x86_64::structures::paging::PhysFrame::from_start_address_unchecked(
                x86_64::PhysAddr::new(telemetry_phys_page + i * PAGE_SIZE),
            )
        };
        if unsafe {
            process
                .address_space
                .map_page(user_page, phys_frame, flags, &mut pmm, hhdm_offset)
        }
        .is_err()
        {
            for rollback_idx in (0..i).rev() {
                let Ok(rollback_page) = x86_64::structures::paging::Page::from_start_address(
                    user_addr + rollback_idx * PAGE_SIZE,
                ) else {
                    continue;
                };
                let rollback_phys =
                    x86_64::PhysAddr::new(telemetry_phys_page + rollback_idx * PAGE_SIZE);
                let _ = unsafe {
                    process.address_space.rollback_mapped_page(
                        rollback_page,
                        rollback_phys,
                        &mut pmm,
                        hhdm_offset,
                    )
                };
            }
            process.address_space.cancel_region(reservation);
            return 0;
        }
    }
    if process.address_space.commit_region(reservation).is_err() {
        for rollback_idx in (0..telemetry_pages).rev() {
            let rollback_page = x86_64::structures::paging::Page::from_start_address(
                user_addr + rollback_idx * PAGE_SIZE,
            )
            .expect("validated telemetry rollback page");
            let rollback_phys =
                x86_64::PhysAddr::new(telemetry_phys_page + rollback_idx * PAGE_SIZE);
            let _ = unsafe {
                process.address_space.rollback_mapped_page(
                    rollback_page,
                    rollback_phys,
                    &mut pmm,
                    hhdm_offset,
                )
            };
        }
        return 0;
    }
    user_addr.as_u64() + telemetry_page_off
}

/// Syscall: get_time_utc (81)
/// Returns the current Unix timestamp in seconds (RTC + tick advancement).
fn sys_get_time_utc() -> u64 {
    crate::arch::x86_64::rtc::unix_time()
}

/// Syscall: monotonic_ms (86). Milliseconds since boot, derived from the
/// centralized kernel timekeeper (~100 Hz, so ~10 ms resolution). Used for
/// RTT measurement (e.g. ping) where the 1 s resolution of `get_time_utc` is
/// too coarse.
fn sys_monotonic_ms() -> u64 {
    // Legacy coarse monotonic syscall kept intentionally for existing Ring 3 code.
    crate::timekeeping::monotonic_ms()
}

/// Syscall: GetEntropy (87). Returns one u64 of raw hardware seed entropy.
///
/// This is *seed-grade* material only — callers (libc's local generator and the
/// `rand` service's ChaCha20 engine) expand it with a real PRNG/CSPRNG. It is
/// deliberately left ungated: handing a process a single unpredictable u64 to
/// bootstrap its own non-crypto RNG is harmless and is the whole point of the
/// `GRND_NONCRYPTO` fast path.
///
/// RDRAND signals success in the carry flag (CF), NOT via the value (0 is a
/// legal random result), so we retry on carry-clear. If RDRAND is absent or
/// exhausted we fall back to folding two TSC reads — the low bits carry real
/// inter-read jitter.
///
/// IMPORTANT: RDRAND is NOT universally present. The default QEMU CPU (`qemu64`)
/// and some hypervisors don't expose it, and executing `rdrand` there raises
/// #UD — a kernel-mode invalid-opcode fault that wedges the system. We must
/// probe CPUID.01H:ECX[bit 30] before issuing the instruction.
fn sys_get_entropy() -> u64 {
    // CPUID leaf 1 is always available on x86_64; bit 30 of ECX = RDRAND.
    let has_rdrand = core::arch::x86_64::__cpuid(1).ecx & (1 << 30) != 0;
    if has_rdrand {
        let mut val: u64;
        let mut ok: u8;
        for _ in 0..10 {
            // SAFETY: RDRAND is gated by the CPUID probe above; it has no memory
            // operands and does not touch the stack.
            unsafe {
                core::arch::asm!(
                    "rdrand {v}",
                    "setc {o}",
                    v = out(reg) val,
                    o = out(reg_byte) ok,
                    options(nomem, nostack),
                );
            }
            if ok != 0 {
                return val;
            }
        }
    }
    // Fallback (no RDRAND, or it returned carry-clear 10×): fold two TSC reads.
    // SAFETY: RDTSC is always available on x86_64 and reads the cycle counter.
    let a = unsafe { core::arch::x86_64::_rdtsc() };
    let b = unsafe { core::arch::x86_64::_rdtsc() };
    a.rotate_left(17) ^ b ^ (b << 32)
}

/// Syscall: ClockGetTime (88)
/// rdi = clockid (0 realtime, 1 monotonic)
/// rsi = pointer to user timespec { tv_sec, tv_nsec }
fn sys_clock_gettime(frame: &mut SyscallFrame) -> u64 {
    let clockid = frame.rdi as i32;

    let values = match clockid {
        0 => {
            let sec = crate::arch::x86_64::rtc::unix_time();
            [sec, 0]
        }
        1 => {
            let ns = crate::arch::x86_64::interrupts::now_ns();
            let sec = ns / 1_000_000_000;
            let nsec = ns % 1_000_000_000;
            [sec, nsec]
        }
        _ => return u64::MAX,
    };
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&values[0].to_ne_bytes());
    bytes[8..].copy_from_slice(&values[1].to_ne_bytes());
    match copy_to_user(frame.rsi, &bytes) {
        Ok(()) => 0,
        Err(error) => error,
    }
}

/// Syscall: sysinfo (82)
/// rdi = user pointer to seven u64s, filled as:
///   [0] total RAM (KiB)
///   [1] used RAM (KiB)
///   [2] uptime (seconds)
///   [3] Unix time (seconds)
///   [4] swap total (KiB)
///   [5] swap used (KiB)
///   [6] swap used, real compressed size (KiB)
fn sys_sysinfo(frame: &mut SyscallFrame) -> u64 {
    let (total_frames, free_frames) = crate::PMM.lock().stats();
    // 4 KiB frames -> KiB
    let total_kb = (total_frames as u64) * 4;
    let used_kb = (total_frames.saturating_sub(free_frames) as u64) * 4;

    let (swap_total_blocks, swap_used_blocks, swap_used_bytes) = crate::memory::zram::stats();
    let swap_total_kb = (swap_total_blocks as u64) * 4;
    let swap_used_kb = (swap_used_blocks as u64) * 4;
    let swap_compressed_kb = (swap_used_bytes as u64 + 1023) / 1024;

    let info = [
        total_kb,
        used_kb,
        crate::arch::x86_64::rtc::uptime_secs(),
        crate::arch::x86_64::rtc::unix_time(),
        swap_total_kb,
        swap_used_kb,
        swap_compressed_kb,
    ];
    let bytes = unsafe { core::slice::from_raw_parts(info.as_ptr() as *const u8, info.len() * 8) };
    if let Err(error) = copy_to_user(frame.rdi, bytes) {
        return error;
    }
    0
}

/// Syscall: swapctl (85).
///
/// Operation 0 performs a bounded, typed diagnostic swap-out over an anonymous
/// range owned by the exact embedded freezram applet. Op 2 reports online
/// CPUs. Op 3 is the one-shot SwapAdmin configuration request;
/// the kernel authenticates the embedded service identity and recomputes the
/// submitted policy from PMM/scheduler state. Op 4 reports active pool count;
/// ops 5/6 copy bounded aggregate/per-pool health snapshots to userspace.
fn sys_swapctl(frame: &mut SyscallFrame) -> u64 {
    match frame.rdi {
        0 => {
            use ::sunlight_ipc::swap_policy::FillDiagnostics;

            if frame.r8 as usize != core::mem::size_of::<FillDiagnostics>() {
                return u64::MAX;
            }
            let hhdm_offset = VirtAddr::new(crate::HHDM_REQ.response().expect("no hhdm").offset);
            let result = {
                let mut sched = crate::sched::SCHEDULER.lock();
                if !sched.current_process().trusted_zram_diagnostic {
                    return u64::MAX;
                }
                let mut pmm = crate::PMM.lock();
                unsafe {
                    crate::memory::swap::diagnostic_fill(
                        frame.rsi,
                        frame.rdx,
                        &mut sched,
                        hhdm_offset,
                        &mut pmm,
                    )
                }
            };
            let bytes = unsafe {
                core::slice::from_raw_parts(
                    (&result as *const FillDiagnostics).cast::<u8>(),
                    core::mem::size_of::<FillDiagnostics>(),
                )
            };
            if copy_to_user(frame.r10, bytes).is_ok() {
                0
            } else {
                return u64::MAX;
            }
        }
        2 => crate::sched::SCHEDULER.lock().online_cores.max(1) as u64,
        3 => {
            let (authorized, owner_pid, owner_generation, online_cpus) = {
                let sched = crate::sched::SCHEDULER.lock();
                let process = sched.current_process();
                (
                    process.trusted_swap_admin_service,
                    process.pid,
                    process.address_space.identity().generation,
                    sched.online_cores.max(1) as u32,
                )
            };
            if !authorized {
                return u64::MAX;
            }
            let total_frames = crate::PMM.lock().stats().0 as u64;
            let Some(usable_ram_bytes) = total_frames.checked_mul(4096) else {
                return u64::MAX;
            };
            let Ok(policy) = ::sunlight_ipc::swap_policy::calculate(usable_ram_bytes, online_cpus)
            else {
                return u64::MAX;
            };
            if frame.rsi != policy.detected_ram_bytes
                || frame.rdx != u64::from(policy.detected_online_cpus)
                || frame.r8 != policy.total_logical_bytes
                || frame.r9 != policy.pool_count as u64
                || frame.r10 != policy.total_physical_budget_bytes
                || frame.r12 != u64::from(policy.version)
            {
                return u64::MAX;
            }
            match crate::memory::zram::configure(policy, owner_pid, owner_generation) {
                Ok(()) => {
                    crate::serial_println!(
                        "[SWAP-1] enabled pools={} logical_mib={} physical_budget_kib={} admin_pid={}",
                        policy.pool_count,
                        policy.total_logical_bytes / (1024 * 1024),
                        policy.total_physical_budget_bytes / 1024,
                        owner_pid
                    );
                    0
                }
                Err(_) => u64::MAX,
            }
        }
        4 => crate::memory::zram::aggregate_stats().active_pool_count as u64,
        5 => {
            use ::sunlight_ipc::swap_policy::AggregateDiagnostics;

            if frame.rdx as usize != core::mem::size_of::<AggregateDiagnostics>() {
                return u64::MAX;
            }
            let zram = crate::memory::zram::aggregate_stats();
            let reclaim = crate::memory::swap::diagnostics();
            let last_fill = crate::memory::swap::last_fill_result();
            let policy = crate::memory::zram::policy();
            let diagnostics = AggregateDiagnostics {
                active_pool_count: zram.active_pool_count as u64,
                configured_logical_pages: zram.configured_logical_pages,
                configured_physical_budget_bytes: zram.configured_physical_budget_bytes,
                stored_pages: zram.stored_pages,
                compressed_bytes: zram.compressed_bytes,
                allocator_consumed_bytes: zram.allocator_consumed_bytes,
                pages_stored_raw: zram.pages_stored_raw,
                incompressible_rejected: zram.incompressible_rejected,
                swap_out_attempts: zram.swap_out_attempts,
                swap_out_successes: zram.swap_out_successes,
                swap_out_failures: zram.swap_out_failures,
                swap_in_attempts: zram.swap_in_attempts,
                swap_in_successes: zram.swap_in_successes,
                swap_in_failures: zram.swap_in_failures,
                checksum_failures: zram.checksum_failures,
                decompression_failures: zram.decompression_failures,
                full_pool_events: zram.full_pool_events,
                budget_full_events: zram.budget_full_events,
                fallback_to_next_pool: zram.fallback_to_next_pool,
                candidate_scans: reclaim.candidate_scans,
                pages_reclaimed: reclaim.pages_reclaimed,
                watermark_activations: reclaim.watermark_activations,
                service_configured: u64::from(zram.service_configured),
                admin_owner_alive: u64::from(zram.admin_owner_alive),
                detected_ram_bytes: policy.map_or(0, |value| value.detected_ram_bytes),
                detected_online_cpus: policy
                    .map_or(0, |value| u64::from(value.detected_online_cpus)),
                last_fill_requested_pages: last_fill.requested_pages,
                last_fill_stored_pages: last_fill.stored_pages,
                last_fill_error: last_fill.error,
            };
            let bytes = unsafe {
                core::slice::from_raw_parts(
                    (&diagnostics as *const AggregateDiagnostics).cast::<u8>(),
                    core::mem::size_of::<AggregateDiagnostics>(),
                )
            };
            if copy_to_user(frame.rsi, bytes).is_ok() {
                0
            } else {
                u64::MAX
            }
        }
        6 => {
            use ::sunlight_ipc::swap_policy::PoolDiagnostics;

            if frame.r8 as usize != core::mem::size_of::<PoolDiagnostics>() {
                return u64::MAX;
            }
            let Some(pool) = crate::memory::zram::pool_stats(frame.rsi as usize) else {
                return u64::MAX;
            };
            let diagnostics = PoolDiagnostics {
                logical_capacity_pages: pool.logical_capacity_pages,
                physical_budget_bytes: pool.physical_budget_bytes,
                used_logical_pages: pool.used_logical_pages,
                used_compressed_bytes: pool.used_compressed_bytes,
                allocator_consumed_bytes: pool.allocator_consumed_bytes,
                compression_successes: pool.compression_successes,
                compression_failures: pool.compression_failures,
                raw_pages: pool.raw_pages,
                incompressible_rejected: pool.incompressible_rejected,
                swap_out_attempts: pool.swap_out_attempts,
                swap_out_successes: pool.swap_out_successes,
                swap_out_failures: pool.swap_out_failures,
                swap_in_attempts: pool.swap_in_attempts,
                swap_in_successes: pool.swap_in_successes,
                swap_in_failures: pool.swap_in_failures,
                checksum_failures: pool.checksum_failures,
                decompression_failures: pool.decompression_failures,
                full_events: pool.full_events,
                budget_full_events: pool.budget_full_events,
                slot_releases: pool.slot_releases,
            };
            let bytes = unsafe {
                core::slice::from_raw_parts(
                    (&diagnostics as *const PoolDiagnostics).cast::<u8>(),
                    core::mem::size_of::<PoolDiagnostics>(),
                )
            };
            if copy_to_user(frame.rdx, bytes).is_ok() {
                0
            } else {
                u64::MAX
            }
        }
        _ => u64::MAX,
    }
}

// ---------------------------------------------------------------------------
// Keyboard driver syscalls (Ring 3 migration)
// ---------------------------------------------------------------------------

/// Syscall: kbd_register (110)
/// rdi = endpoint_id
/// Register the calling process as the keyboard driver.
fn sys_kbd_register(frame: &mut SyscallFrame) -> u64 {
    // rdi carries the full endpoint capability token. Resolve it to the real
    // internal endpoint id so IRQ1 keyboard events are queued on the same
    // endpoint the driver receives on via ipc_recv. Storing the raw token here
    // (truncated to u32) queues events on a non-existent endpoint, so the
    // driver's ipc_recv never returns and the keyboard appears dead.
    let token = CapabilityToken(frame.rdi);
    let caps = crate::capability::CAP_BROKER.lock();
    let endpoint_id = match caps.check(token, CapabilityRights::RECV_ONLY) {
        Ok(id) => id,
        Err(_) => return u64::MAX,
    };
    drop(caps);
    crate::arch::x86_64::keyboard::register_kbd_driver(endpoint_id);
    crate::hardware_inventory::update_ps2(
        0,
        crate::hardware_inventory::pack_short_name("keyboard"),
        crate::hardware_inventory::pack_short_name("keyboard"),
        ::sunlight_ipc::HardwareState::Active,
        ::sunlight_ipc::HardwareFailureStage::None,
        0,
    );
    0
}

/// Syscall: kbd_unregister (111)
/// Unregister the keyboard driver.
fn sys_kbd_unregister() -> u64 {
    crate::arch::x86_64::keyboard::unregister_kbd_driver();
    0
}

/// Syscall: kbd_pop_scancode (112)
/// Pop one raw scancode from the kernel's ring buffer.
/// Returns the scancode in the low byte, or u64::MAX if none available.
fn sys_kbd_pop_scancode() -> u64 {
    crate::arch::x86_64::keyboard::pop_scancode()
        .map(|sc| sc as u64)
        .unwrap_or(u64::MAX)
}

/// Syscall: kbd_get_stats (113)
/// rdi = pointer to [u64; 3] buffer for stats
/// Returns 0 on success, u64::MAX on error.
fn sys_kbd_get_stats(frame: &mut SyscallFrame) -> u64 {
    let (pending, dropped, capacity) = crate::arch::x86_64::keyboard::get_stats();
    let values = [pending as u64, dropped as u64, capacity as u64];
    let bytes =
        unsafe { core::slice::from_raw_parts(values.as_ptr() as *const u8, values.len() * 8) };
    if let Err(error) = copy_to_user(frame.rdi, bytes) {
        return error;
    }
    0
}

// ---------------------------------------------------------------------------
// Mouse driver syscalls (Ring 3 migration)
// ---------------------------------------------------------------------------

/// Syscall: mouse_register (114)
fn sys_mouse_register(frame: &mut SyscallFrame) -> u64 {
    let token = CapabilityToken(frame.rdi);
    let caps = crate::capability::CAP_BROKER.lock();
    let endpoint_id = match caps.check(token, CapabilityRights::RECV_ONLY) {
        Ok(id) => id,
        Err(_) => return u64::MAX,
    };
    drop(caps);
    crate::arch::x86_64::mouse::register_mouse_driver(endpoint_id);
    crate::hardware_inventory::update_ps2(
        1,
        crate::hardware_inventory::pack_short_name("mouse"),
        crate::hardware_inventory::pack_short_name("mouse"),
        ::sunlight_ipc::HardwareState::Active,
        ::sunlight_ipc::HardwareFailureStage::None,
        0,
    );
    0
}

/// Syscall: mouse_pop_byte (115)
fn sys_mouse_pop_byte() -> u64 {
    crate::arch::x86_64::mouse::pop_mouse_byte()
        .map(|b| b as u64)
        .unwrap_or(u64::MAX)
}

fn current_process_is_mouse_driver() -> bool {
    crate::sched::SCHEDULER.lock().current_process().name_str() == "sunlight-mouse"
}

fn is_mouse_port(port: u16) -> bool {
    port == 0x60 || port == 0x64
}

/// Syscall: mouse_init (116)
/// Restricted to sunlight-mouse. Performs PS/2 controller setup atomically in kernel mode.
fn sys_mouse_init() -> u64 {
    if !current_process_is_mouse_driver() {
        return u64::MAX;
    }
    if crate::arch::x86_64::mouse::init_ps2_mouse() {
        0
    } else {
        crate::hardware_inventory::update_ps2(
            1,
            crate::hardware_inventory::pack_short_name("mouse"),
            0,
            ::sunlight_ipc::HardwareState::ProbeFailed,
            ::sunlight_ipc::HardwareFailureStage::DeviceInitialization,
            1,
        );
        u64::MAX
    }
}

/// Syscall: mouse_port_read (117)
/// Restricted debug helper for sunlight-mouse and PS/2 data/status ports.
fn sys_mouse_port_read(frame: &mut SyscallFrame) -> u64 {
    let port = frame.rdi as u16;
    if !current_process_is_mouse_driver() || !is_mouse_port(port) {
        return u64::MAX;
    }
    unsafe {
        let mut p: x86_64::instructions::port::Port<u8> =
            x86_64::instructions::port::Port::new(port);
        p.read() as u64
    }
}

/// Syscall: mouse_get_stats (125)
/// rdi = pointer to [u64; 3] buffer for stats
/// Returns 0 on success, u64::MAX on error.
fn sys_mouse_get_stats(frame: &mut SyscallFrame) -> u64 {
    if !current_process_is_mouse_driver() {
        return u64::MAX;
    }
    let (pending, dropped, capacity) = crate::arch::x86_64::mouse::get_stats();
    let values = [pending as u64, dropped as u64, capacity as u64];
    let bytes =
        unsafe { core::slice::from_raw_parts(values.as_ptr() as *const u8, values.len() * 8) };
    if let Err(error) = copy_to_user(frame.rdi, bytes) {
        return error;
    }
    0
}

/// Syscall: powerctl (80)
/// Power management: shutdown (0) or reboot (1)
fn sys_powerctl(command: u64) -> u64 {
    match command {
        0 => {
            // Shutdown
            crate::serial_println!("[SYSCALL] shutdown requested");
            crate::arch::x86_64::acpi::shutdown();
        }
        1 => {
            // Reboot
            crate::serial_println!("[SYSCALL] reboot requested");
            crate::arch::x86_64::acpi::reboot();
        }
        _ => {
            crate::serial_println!("[SYSCALL] unknown powerctl command: {}", command);
            return u64::MAX;
        }
    }
}

// ---------------------------------------------------------------------------
// VirtIO GPU proxy syscalls (119-124)
// All gated by process name "display_server".
// ---------------------------------------------------------------------------

fn current_process_is_display_server() -> bool {
    // The display server's *process* name is "sunlight-display" (its binary at
    // /sbin/sunlight-display); "display_server" is only its nameserver
    // registration string. Accept both so the GPU proxy syscalls aren't denied.
    let sched = crate::sched::SCHEDULER.lock();
    let name = sched.current_process().name_str();
    name == "sunlight-display" || name == "display_server"
}

/// Syscall 119: GpuGetInfo
/// Returns 1 in rax if GPU present, 0 otherwise.
/// frame.r8 = width | (height << 32)
fn sys_gpu_get_info(frame: &mut SyscallFrame) -> u64 {
    if !current_process_is_display_server() {
        return 0;
    }
    let dev = crate::GPU_DEVICE.lock();
    match dev.as_ref() {
        Some(gpu) if gpu.width > 0 && gpu.height > 0 => {
            frame.r8 = (gpu.width as u64) | ((gpu.height as u64) << 32);
            1
        }
        _ => 0,
    }
}

// Failure reason codes for the GPU setup syscalls (120/121), reported to
// userspace in frame.r8 (msg.words[0]) when the syscall returns 0.
// Keep numerically in sync with `sunlight_ipc::gpu_proxy`.
// Layout: low 32 bits = reason, high 32 bits = detail (VirtIO resp code,
// failing page index, or entry count depending on the reason).
const GPU_ERR_NOT_DISPLAY_SERVER: u64 = 1;
const GPU_ERR_BAD_ARGS: u64 = 2;
const GPU_ERR_NO_DEVICE: u64 = 3;
const GPU_ERR_UNMAPPED_PAGE: u64 = 4;
const GPU_ERR_SG_OVERFLOW: u64 = 5;
const GPU_ERR_CREATE_FAILED: u64 = 6;
const GPU_ERR_ATTACH_FAILED: u64 = 7;
const GPU_ERR_SCANOUT_FAILED: u64 = 8;

fn gpu_err(frame: &mut SyscallFrame, reason: u64, detail: u32) -> u64 {
    frame.r8 = reason | ((detail as u64) << 32);
    0
}

/// Syscall 120: GpuAttachBacking
/// rdi = user VA of back_buffer start (must be page-aligned),
/// rsi = number of 4KiB pages.
/// Walks the process page table, coalesces physically-contiguous pages into a
/// scatter-gather list, then sends RESOURCE_CREATE_2D + RESOURCE_ATTACH_BACKING
/// for SCANOUT_RESOURCE_ID (1). Returns 1 on success; on failure returns 0 with
/// a reason code in r8 and logs the exact step that failed.
fn sys_gpu_attach_backing(frame: &mut SyscallFrame) -> u64 {
    if !current_process_is_display_server() {
        return gpu_err(frame, GPU_ERR_NOT_DISPLAY_SERVER, 0);
    }
    let user_va = frame.rdi;
    let num_pages = frame.rsi as usize;
    // 4096 pages = 16 MiB backing, comfortably above any sane scanout size.
    if num_pages == 0 || num_pages > 4096 {
        crate::serial_println!("[GPU] attach_backing: bad num_pages={}", num_pages);
        return gpu_err(frame, GPU_ERR_BAD_ARGS, num_pages as u32);
    }
    if user_va & 0xFFF != 0 {
        // A misaligned buffer would silently shift every scanline: the device
        // scans from the start of the first page, not from user_va.
        crate::serial_println!(
            "[GPU] attach_backing: user_va {:#x} not page-aligned",
            user_va
        );
        return gpu_err(frame, GPU_ERR_BAD_ARGS, (user_va & 0xFFF) as u32);
    }
    let Some(byte_len) = num_pages.checked_mul(4096) else {
        return gpu_err(frame, GPU_ERR_BAD_ARGS, num_pages as u32);
    };
    if crate::memory::user::UserRange::new(user_va, byte_len).is_err() {
        return gpu_err(frame, GPU_ERR_BAD_ARGS, 0);
    }

    use x86_64::{structures::paging::Page, VirtAddr};
    let hhdm = crate::HHDM_REQ.response().expect("no hhdm").offset;
    let hhdm_va = VirtAddr::new(hhdm);

    // Build scatter-gather entries from the process's page table, merging
    // physically-contiguous pages so large buffers stay within the device
    // driver's sg buffer capacity (MAX_SG_ENTRIES).
    let mut entries: alloc::vec::Vec<sunlight_virtio::VirtioGpuMemEntry> = alloc::vec::Vec::new();
    {
        let sched = crate::sched::SCHEDULER.lock();
        let process = sched.current_process();
        for i in 0..num_pages {
            let va = match (i as u64)
                .checked_mul(4096)
                .and_then(|offset| user_va.checked_add(offset))
            {
                Some(address) => address,
                None => return gpu_err(frame, GPU_ERR_BAD_ARGS, i as u32),
            };
            let page = Page::containing_address(VirtAddr::new(va));
            let phys = match unsafe { process.address_space.lookup_entry(page, hhdm_va) } {
                Some((physical, flags))
                    if flags.contains(PageTableFlags::PRESENT)
                        && flags.contains(PageTableFlags::USER_ACCESSIBLE) =>
                {
                    physical.as_u64()
                }
                None => {
                    crate::serial_println!(
                        "[GPU] attach_backing: page {}/{} at va {:#x} not mapped",
                        i,
                        num_pages,
                        va
                    );
                    return gpu_err(frame, GPU_ERR_UNMAPPED_PAGE, i as u32);
                }
                Some(_) => return gpu_err(frame, GPU_ERR_UNMAPPED_PAGE, i as u32),
            };
            match entries.last_mut() {
                Some(last) if last.addr + last.length as u64 == phys => {
                    last.length += 4096;
                }
                _ => entries.push(sunlight_virtio::VirtioGpuMemEntry {
                    addr: phys,
                    length: 4096,
                    padding: 0,
                }),
            }
        }
    }
    if entries.len() > sunlight_virtio::gpu::MAX_SG_ENTRIES {
        crate::serial_println!(
            "[GPU] attach_backing: {} pages need {} sg entries, driver max is {}",
            num_pages,
            entries.len(),
            sunlight_virtio::gpu::MAX_SG_ENTRIES
        );
        return gpu_err(frame, GPU_ERR_SG_OVERFLOW, entries.len() as u32);
    }

    let mut dev = crate::GPU_DEVICE.lock();
    let gpu = match dev.as_mut() {
        Some(g) => g,
        None => return gpu_err(frame, GPU_ERR_NO_DEVICE, 0),
    };

    // First create the scanout resource (idempotent if already created, but safe)
    let (w, h) = (gpu.width, gpu.height);
    crate::serial_println!(
        "[GPU] attach_backing: va={:#x} pages={} sg_entries={} resource={}x{}",
        user_va,
        num_pages,
        entries.len(),
        w,
        h
    );
    if let Err(code) = unsafe {
        gpu.resource_create_2d(
            sunlight_virtio::gpu::SCANOUT_RESOURCE_ID,
            // The back_buffer holds packed little-endian 0x00RRGGBB pixels,
            // which is memory byte order B,G,R,X. Using X8R8G8B8 here swaps
            // R/G and drops blue entirely (green-tinted screen).
            sunlight_virtio::gpu::VIRTIO_GPU_FORMAT_B8G8R8X8_UNORM,
            w,
            h,
        )
    } {
        crate::serial_println!(
            "[GPU] RESOURCE_CREATE_2D {}x{} failed: {:#x} ({})",
            w,
            h,
            code,
            sunlight_virtio::gpu::resp_code_name(code)
        );
        return gpu_err(frame, GPU_ERR_CREATE_FAILED, code);
    }

    if let Err(code) =
        unsafe { gpu.resource_attach_backing(sunlight_virtio::gpu::SCANOUT_RESOURCE_ID, &entries) }
    {
        crate::serial_println!(
            "[GPU] RESOURCE_ATTACH_BACKING ({} entries) failed: {:#x} ({})",
            entries.len(),
            code,
            sunlight_virtio::gpu::resp_code_name(code)
        );
        return gpu_err(frame, GPU_ERR_ATTACH_FAILED, code);
    }
    1
}

/// Syscall 121: GpuSetScanout
/// Sends SET_SCANOUT to wire resource 1 to scanout 0. Returns 1 on success;
/// on failure returns 0 with a reason code in r8.
fn sys_gpu_set_scanout(frame: &mut SyscallFrame) -> u64 {
    if !current_process_is_display_server() {
        return gpu_err(frame, GPU_ERR_NOT_DISPLAY_SERVER, 0);
    }
    let mut dev = crate::GPU_DEVICE.lock();
    let gpu = match dev.as_mut() {
        Some(g) => g,
        None => return gpu_err(frame, GPU_ERR_NO_DEVICE, 0),
    };
    let (w, h) = (gpu.width, gpu.height);
    if let Err(code) = unsafe {
        gpu.set_scanout(
            sunlight_virtio::gpu::SCANOUT_ID,
            sunlight_virtio::gpu::SCANOUT_RESOURCE_ID,
            w,
            h,
        )
    } {
        crate::serial_println!(
            "[GPU] SET_SCANOUT {}x{} failed: {:#x} ({})",
            w,
            h,
            code,
            sunlight_virtio::gpu::resp_code_name(code)
        );
        return gpu_err(frame, GPU_ERR_SCANOUT_FAILED, code);
    }
    crate::serial_println!("[GPU] SET_SCANOUT {}x{} OK (scanout 0, resource 1)", w, h);
    1
}

/// Syscall 122: GpuFlush
/// rdi = x | (y << 32), rsi = w | (h << 32)
/// Issues TRANSFER_TO_HOST_2D then RESOURCE_FLUSH for the dirty rect.
fn sys_gpu_flush(frame: &mut SyscallFrame) -> u64 {
    if !current_process_is_display_server() {
        return 0;
    }
    let x = (frame.rdi & 0xFFFF_FFFF) as u32;
    let y = (frame.rdi >> 32) as u32;
    let w = (frame.rsi & 0xFFFF_FFFF) as u32;
    let h = (frame.rsi >> 32) as u32;

    let mut dev = crate::GPU_DEVICE.lock();
    let gpu = match dev.as_mut() {
        Some(g) => g,
        None => return 0,
    };
    let width = gpu.width;
    let ok1 = unsafe {
        gpu.transfer_to_host_2d(sunlight_virtio::gpu::SCANOUT_RESOURCE_ID, x, y, w, h, width)
    };
    let ok2 = unsafe { gpu.resource_flush(sunlight_virtio::gpu::SCANOUT_RESOURCE_ID, x, y, w, h) };
    if ok1.is_ok() && ok2.is_ok() {
        1
    } else {
        0
    }
}

/// Syscall 123: GpuUpdateCursor
/// rdi = user VA of 64×64 BGRA pixels
/// rsi = num_pixels (≤ 64*64 = 4096)
/// rdx = hot_x | (hot_y << 32)
///
/// Copies pixels from user VA into kernel cursor backing, creates the cursor
/// resource (if needed), attaches backing, then calls UPDATE_CURSOR.
fn sys_gpu_update_cursor(frame: &mut SyscallFrame) -> u64 {
    if !current_process_is_display_server() {
        return 0;
    }
    let user_va = frame.rdi;
    let num_pixels = (frame.rsi as usize)
        .min((sunlight_virtio::gpu::CURSOR_W * sunlight_virtio::gpu::CURSOR_H) as usize);
    let hot_x = (frame.rdx & 0xFFFF_FFFF) as u32;
    let hot_y = (frame.rdx >> 32) as u32;

    // Copy pixels from user VA into kernel cursor backing
    let cursor_pixel_bytes = num_pixels * 4;
    let mut pixels = [0u8; 4 * 4096];
    if cursor_pixel_bytes > 0 {
        if let Err(error) = copy_from_user(user_va, &mut pixels[..cursor_pixel_bytes]) {
            return error;
        }
    }
    {
        let mut dev = crate::GPU_DEVICE.lock();
        let gpu = match dev.as_mut() {
            Some(g) => g,
            None => return 0,
        };
        let dst = gpu.cursor_pixels_virt() as *mut u8;
        // Zero the backing first (clear any previous cursor shape)
        unsafe {
            dst.write_bytes(0, 4 * 4096);
        }
        if cursor_pixel_bytes > 0 {
            unsafe {
                core::ptr::copy_nonoverlapping(pixels.as_ptr(), dst, cursor_pixel_bytes);
            }
        }
    }

    let mut dev = crate::GPU_DEVICE.lock();
    let gpu = match dev.as_mut() {
        Some(g) => g,
        None => return 0,
    };

    // Create cursor resource (format B8G8R8A8 for transparency)
    if let Err(code) = unsafe {
        gpu.resource_create_2d(
            sunlight_virtio::gpu::CURSOR_RESOURCE_ID,
            sunlight_virtio::gpu::VIRTIO_GPU_FORMAT_B8G8R8A8_UNORM,
            sunlight_virtio::gpu::CURSOR_W,
            sunlight_virtio::gpu::CURSOR_H,
        )
    } {
        crate::serial_println!(
            "[GPU] cursor resource_create_2d failed: {:#x} ({})",
            code,
            sunlight_virtio::gpu::resp_code_name(code)
        );
    }

    // Attach backing (kernel-allocated cursor pages)
    let cursor_pages = gpu.cursor_pages_phys();
    if let Err(code) = unsafe {
        gpu.resource_attach_backing(sunlight_virtio::gpu::CURSOR_RESOURCE_ID, &cursor_pages)
    } {
        crate::serial_println!(
            "[GPU] cursor resource_attach_backing failed: {:#x} ({})",
            code,
            sunlight_virtio::gpu::resp_code_name(code)
        );
    }

    // Transfer cursor pixels to host
    if let Err(code) = unsafe {
        gpu.transfer_to_host_2d(
            sunlight_virtio::gpu::CURSOR_RESOURCE_ID,
            0,
            0,
            sunlight_virtio::gpu::CURSOR_W,
            sunlight_virtio::gpu::CURSOR_H,
            sunlight_virtio::gpu::CURSOR_W,
        )
    } {
        crate::serial_println!(
            "[GPU] cursor transfer failed: {:#x} ({})",
            code,
            sunlight_virtio::gpu::resp_code_name(code)
        );
    }

    // Update cursor on scanout
    let result = unsafe {
        gpu.update_cursor(
            sunlight_virtio::gpu::SCANOUT_ID,
            sunlight_virtio::gpu::CURSOR_RESOURCE_ID,
            0,
            0, // position — display_server will call move_cursor immediately
            hot_x,
            hot_y,
        )
    };
    match result {
        Ok(()) => 1,
        Err(code) => {
            crate::serial_println!(
                "[GPU] cursor UPDATE_CURSOR command failed: {:#x} ({})",
                code,
                sunlight_virtio::gpu::resp_code_name(code)
            );
            0
        }
    }
}

/// Syscall 124: GpuMoveCursor
/// rdi = x | (y << 32)
fn sys_gpu_move_cursor(frame: &mut SyscallFrame) -> u64 {
    if !current_process_is_display_server() {
        return 0;
    }
    let x = (frame.rdi & 0xFFFF_FFFF) as u32;
    let y = (frame.rdi >> 32) as u32;

    let mut dev = crate::GPU_DEVICE.lock();
    let gpu = match dev.as_mut() {
        Some(g) => g,
        None => return 0,
    };
    match unsafe { gpu.move_cursor(sunlight_virtio::gpu::SCANOUT_ID, x, y) } {
        Ok(()) => 1,
        Err(code) => {
            crate::serial_println!(
                "[GPU] cursor MOVE_CURSOR command failed: {:#x} ({})",
                code,
                sunlight_virtio::gpu::resp_code_name(code)
            );
            0
        }
    }
}
