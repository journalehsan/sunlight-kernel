use core::arch::naked_asm;
use x86_64::VirtAddr;

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
    EndpointCreate = 10,
    EndpointBind = 11,
    ProcessExit = 20,
    ProcessYield = 21,
    ThreadSpawn = 22,

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

    // Signal handling (Phase 4.3)
    Sigaction = 70,
    Sigprocmask = 71,
    Kill = 72,
    Pause = 73,
    Sigreturn = 74,

    // Power management (Phase 5.11)
    PowerCtl = 80,

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

/// Deliver pending signals before returning to user space
fn deliver_pending_signals(process: &mut crate::process::Process) {
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
                        // Default action: terminate
                        crate::serial_println!("[SIG] {} delivered: terminating process", sig_num);
                        process.state = crate::process::ProcessState::Finished;
                        crate::sched::request_reschedule();
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
                -1 => {
                    // exit(code) — store code in rdi for process_exit handler
                    if linux_num == 60 || linux_num == 231 {
                        crate::serial_println!(
                            "[HELIOS] Linux exit({}) pid={}",
                            frame.rdi,
                            sched.current_process().pid
                        );
                        num = 20; // ProcessExit
                    }
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
        10 => endpoint_create(),
        11 => endpoint_bind(frame.rdi),
        20 => process_exit(frame.rdi as i32),
        21 => process_yield(),
        22 => thread_spawn(),
        30 => sys_fork(frame),
        31 => sys_exec(frame),
        32 => sys_waitpid(frame),
        33 => sys_getpid(),
        34 => sys_getppid(frame),
        35 => sys_getuid(),
        36 => sys_getgid(),
        37 => sys_setuid(frame),
        38 => sys_setgid(frame),
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
        70 => sys_sigaction(frame),
        71 => sys_sigprocmask(frame),
        72 => sys_kill(frame),
        73 => sys_pause(),
        74 => sys_sigreturn(frame),
        80 => sys_powerctl(frame.rdi),
        81 => sys_get_time_utc(),
        82 => sys_sysinfo(frame),
        99 => debug_log(frame.rdi, frame.rsi),
        _ => {
            crate::serial_println!("[SYSCALL] Unknown syscall {}", num);
            u64::MAX
        }
    };

    // Deliver pending signals before returning to user space
    crate::sched::with_scheduler(|sched| {
        deliver_pending_signals(sched.current_process_mut());
    });

    result
}

// ---------------------------------------------------------------------------
// Individual syscall implementations
// ---------------------------------------------------------------------------

use crate::capability::CapabilityRights;
use crate::capability::CapabilityToken;
use crate::ipc::{IpcError, IpcMsg, INIT_NAMESERVER_ENDPOINT};
use crate::process::layout::is_user_address;
use crate::process::ProcessState;
use crate::sched;
use alloc::vec::Vec;

/// Read a null-terminated C string from user space.
unsafe fn read_user_cstr(ptr: u64, max_len: usize) -> Option<Vec<u8>> {
    if !is_user_address(ptr) {
        return None;
    }

    let mut result = Vec::new();
    let bytes = ptr as *const u8;

    for i in 0..max_len {
        let byte = *bytes.add(i);
        if byte == 0 {
            return Some(result);
        }
        result.push(byte);
    }

    Some(result)
}

/// Read an array of pointers from user space (null-terminated array of *const u8).
unsafe fn read_user_ptr_array(ptr: u64, max_entries: usize) -> Option<Vec<u64>> {
    if !is_user_address(ptr) {
        return None;
    }

    let mut result = Vec::new();
    let ptrs = ptr as *const u64;

    for i in 0..max_entries {
        let ptr_val = *ptrs.add(i);
        if ptr_val == 0 {
            return Some(result);
        }
        result.push(ptr_val);
    }

    Some(result)
}

fn ipc_call(frame: &mut SyscallFrame) -> u64 {
    let token = CapabilityToken(frame.rsi);
    let msg = IpcMsg::from_registers(frame);

    // Check for spawn capability (fast path handled by kernel)
    if token == crate::capability::SPAWN_TOKEN {
        return handle_spawn_call(frame, msg);
    }

    let mut sched = crate::sched::SCHEDULER.lock();
    let caps = crate::capability::CAP_BROKER.lock();
    let mut bus = crate::ipc::IPC_BUS.lock();
    let sender_pid = sched.current_process().pid;

    match crate::ipc::handle_ipc_call(sender_pid, token, msg, &caps, &mut sched, &mut bus) {
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
}

/// Handle a spawn IPC call directly in the kernel.
/// Extracts path from the message words and spawns a new process.
fn handle_spawn_call(frame: &mut SyscallFrame, msg: IpcMsg) -> u64 {
    let path = decode_path_from_words(&msg.words);
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
    let mut bus = crate::ipc::IPC_BUS.lock();
    let server_pid = sched.current_process().pid;
    match crate::ipc::handle_ipc_reply(server_pid, reply, &mut sched, &mut bus) {
        Ok(()) => 0,
        Err(e) => e as u64,
    }
}

fn ipc_reply_wait(frame: &mut SyscallFrame) -> u64 {
    let endpoint_token = CapabilityToken(frame.rsi);
    let reply = IpcMsg::from_registers(frame);
    let mut sched = crate::sched::SCHEDULER.lock();
    let caps = crate::capability::CAP_BROKER.lock();
    let mut bus = crate::ipc::IPC_BUS.lock();
    let server_pid = sched.current_process().pid;
    let endpoint_id = match caps.check(endpoint_token, CapabilityRights::RECV_ONLY) {
        Ok(id) => id,
        Err(_) => return IpcError::InvalidCapability as u64,
    };
    match crate::ipc::handle_ipc_reply_wait(server_pid, endpoint_id, reply, &mut sched, &mut bus) {
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
}

fn ipc_recv(frame: &mut SyscallFrame) -> u64 {
    let endpoint_token = CapabilityToken(frame.rsi);
    let mut sched = crate::sched::SCHEDULER.lock();
    let caps = crate::capability::CAP_BROKER.lock();
    let mut bus = crate::ipc::IPC_BUS.lock();
    let receiver_pid = sched.current_process().pid;
    let endpoint_id = match caps.check(endpoint_token, CapabilityRights::RECV_ONLY) {
        Ok(id) => id,
        Err(_) => return IpcError::InvalidCapability as u64,
    };
    match crate::ipc::handle_ipc_recv(receiver_pid, endpoint_id, &mut sched, &mut bus) {
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
}

fn ipc_notify_send(_token: u64) -> u64 {
    0
}

fn ipc_notify_wait(_endpoint_token: u64) -> u64 {
    sched::with_scheduler(|s| {
        s.current_process_mut().state = ProcessState::BlockedOnIpc;
        s.current_process_mut().block_start_tick = s.global_tick;
    });
    sched::request_reschedule();
    IpcError::WouldBlock as u64
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
        let caps = crate::capability::CAP_BROKER.lock();
        return caps
            .token_for_endpoint(INIT_NAMESERVER_ENDPOINT, CapabilityRights::SEND)
            .map_or(0, |cap| cap.0);
    }
    token
}

/// Syscall: ProcessExit
/// rdi = exit code
fn process_exit(_code: i32) -> ! {
    sched::with_scheduler(|s| {
        s.current_process_mut().state = ProcessState::Finished;
    });
    sched::request_reschedule();
    loop {
        core::arch::x86_64::_mm_pause()
    }
}

/// Syscall: ProcessYield
fn process_yield() -> u64 {
    sched::with_scheduler(|s| {
        if s.current_process().state == ProcessState::Running {
            s.current_process_mut().state = ProcessState::Ready;
        }
    });
    sched::request_reschedule();
    0
}

fn thread_spawn() -> u64 {
    IpcError::InvalidArgument as u64
}

/// Syscall: DebugLog
/// rdi = pointer to string in user space
/// rsi = length
fn debug_log(ptr: u64, len: u64) -> u64 {
    if ptr == 0 || len == 0 {
        return 0;
    }
    if !is_user_address(ptr) {
        return IpcError::InvalidArgument as u64;
    }

    // SAFETY: ptr is validated to be in user space and len is bounded.
    let slice = unsafe { core::slice::from_raw_parts(ptr as *const u8, len.min(256) as usize) };

    // Print valid UTF-8 prefix
    if let Ok(s) = core::str::from_utf8(slice) {
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
    let mut sched = crate::sched::SCHEDULER.lock();
    let mut pmm = crate::PMM.lock();
    let hhdm = crate::HHDM_REQ.response().expect("no hhdm").offset;

    // Borrow the parent process momentarily to fork it
    let parent_pid = sched.current_process().pid;
    match crate::process::fork::fork_current_process(&mut *pmm, &mut *sched, VirtAddr::new(hhdm)) {
        Ok(child_pid) => {
            crate::serial_println!("[SYSCALL] fork {} -> {}", parent_pid, child_pid);
            child_pid as u64
        }
        Err(_) => {
            crate::serial_println!("[SYSCALL] fork failed for pid={}", parent_pid);
            u64::MAX
        }
    }
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
    let path_bytes = match unsafe { read_user_cstr(path_ptr, 256) } {
        Some(b) => b,
        None => {
            crate::serial_println!("[SYSCALL] exec: bad path pointer");
            return u64::MAX;
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
    let argv_ptrs = match unsafe { read_user_ptr_array(argv_ptr, 16) } {
        Some(a) => a,
        None => {
            crate::serial_println!("[SYSCALL] exec: bad argv pointer");
            return u64::MAX;
        }
    };

    let mut argv_bytes = alloc::vec::Vec::new();
    for &arg_ptr in &argv_ptrs {
        match unsafe { read_user_cstr(arg_ptr, 256) } {
            Some(bytes) => argv_bytes.push(bytes),
            None => {
                crate::serial_println!("[SYSCALL] exec: bad argv[{}] pointer", argv_bytes.len());
                return u64::MAX;
            }
        }
    }

    // Read envp from user space; NULL means "inherit my environment".
    let mut envp_bytes = alloc::vec::Vec::new();
    if envp_ptr != 0 {
        let envp_ptrs = match unsafe { read_user_ptr_array(envp_ptr, 16) } {
            Some(e) => e,
            None => {
                crate::serial_println!("[SYSCALL] exec: bad envp pointer");
                return u64::MAX;
            }
        };
        for &env_ptr in &envp_ptrs {
            match unsafe { read_user_cstr(env_ptr, 256) } {
                Some(bytes) => envp_bytes.push(bytes),
                None => {
                    crate::serial_println!(
                        "[SYSCALL] exec: bad envp[{}] pointer",
                        envp_bytes.len()
                    );
                    return u64::MAX;
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

    // Get embedded ELF bytes for the requested path
    let bytes = match crate::process::spawn::embedded_bytes_for_path(path_str) {
        Ok(b) => b,
        Err(_) => {
            crate::serial_println!("[SYSCALL] exec: path not found: {}", path_str);
            return u64::MAX;
        }
    };

    let mut sched = crate::sched::SCHEDULER.lock();
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
    ) {
        Ok(entry) => {
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
fn sys_waitpid(_frame: &mut SyscallFrame) -> u64 {
    crate::serial_println!("[SYSCALL] waitpid requested");
    u64::MAX
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
        crate::serial_println!("[SYSCALL] setuid: pid={} uid {}→{}", process.pid, current_uid, new_uid);
        0
    } else {
        crate::serial_println!("[SYSCALL] setuid: EPERM (uid {} cannot setuid to {})", current_uid, new_uid);
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
        crate::serial_println!("[SYSCALL] setgid: pid={} gid {}→{}", process.pid, current_gid, new_gid);
        0
    } else {
        crate::serial_println!("[SYSCALL] setgid: EPERM (uid {} cannot setgid to {})", current_uid, new_gid);
        u64::MAX // -1 (EPERM)
    }
}

// File descriptor syscalls (stubs for now)
/// Syscall: open (40)
/// rdi = pathname (user-space pointer)
/// rsi = flags (O_RDONLY, O_WRONLY, O_RDWR, O_CREAT, etc.)
/// rdx = mode (for creation)
fn sys_open(frame: &mut SyscallFrame) -> u64 {
    let _path = frame.rdi as *const u8;
    let _flags = frame.rsi as u32;
    let _mode = frame.rdx as u32;

    // TODO: Implement actual open
    // For now: return stub errno
    crate::serial_println!("[SYSCALL] open requested (stub)");
    u64::MAX // ENOENT
}

/// Syscall: close (41)
/// rdi = fd
fn sys_close(frame: &mut SyscallFrame) -> u64 {
    let fd = frame.rdi as i32;

    let mut sched = crate::sched::SCHEDULER.lock();

    // Check if this is a pipe before closing
    if let Some(fd_entry) = sched.current_process().fd_table.get(fd) {
        if fd_entry.handle.is_pipe() {
            let pipe_idx = fd_entry.handle.pipe_index();
            let is_write = fd_entry.handle.pipe_is_write();
            crate::process::pipe::pipe_close_end(pipe_idx, is_write);
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
    let buf_ptr = frame.rsi as *mut u8;
    let count = frame.rdx as usize;

    let sched = crate::sched::SCHEDULER.lock();

    // Check if fd is valid and has READ right
    match sched.current_process().fd_table.check_rights(
        fd,
        crate::process::fd_table::CapRights::new(crate::process::fd_table::CapRights::READ),
    ) {
        Ok(()) => {
            if let Some(fd_entry) = sched.current_process().fd_table.get(fd) {
                if fd_entry.handle.is_pipe() {
                    let pipe_idx = fd_entry.handle.pipe_index();
                    let mut kernel_buf = [0u8; 4096];
                    let read_size = core::cmp::min(count, 4096);

                    match crate::process::pipe::pipe_read(pipe_idx, &mut kernel_buf[..read_size]) {
                        crate::process::pipe::PipeResult::Ok(n) => {
                            if !is_user_address(buf_ptr as u64)
                                || !is_user_address((buf_ptr as u64) + n as u64)
                            {
                                return u64::MAX;
                            }
                            unsafe {
                                core::ptr::copy_nonoverlapping(kernel_buf.as_ptr(), buf_ptr, n);
                            }
                            n as u64
                        }
                        crate::process::pipe::PipeResult::WouldBlock => EAGAIN,
                        crate::process::pipe::PipeResult::Eof => 0,
                        crate::process::pipe::PipeResult::BrokenPipe => u64::MAX,
                    }
                } else {
                    crate::serial_println!(
                        "[SYSCALL] read fd={} (not a pipe, not implemented)",
                        fd
                    );
                    0
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
    let buf = frame.rsi as *const u8;
    let count = frame.rdx as usize;

    let sched = crate::sched::SCHEDULER.lock();

    // Check if fd is valid and has WRITE right
    match sched.current_process().fd_table.check_rights(
        fd,
        crate::process::fd_table::CapRights::new(crate::process::fd_table::CapRights::WRITE),
    ) {
        Ok(()) => {
            if let Some(fd_entry) = sched.current_process().fd_table.get(fd) {
                if fd_entry.handle.is_pipe() {
                    if !is_user_address(buf as u64) || !is_user_address((buf as u64) + count as u64)
                    {
                        return u64::MAX;
                    }

                    let pipe_idx = fd_entry.handle.pipe_index();
                    let write_size = core::cmp::min(count, 4096);
                    let mut kernel_buf = [0u8; 4096];

                    unsafe {
                        core::ptr::copy_nonoverlapping(buf, kernel_buf.as_mut_ptr(), write_size);
                    }

                    match crate::process::pipe::pipe_write(pipe_idx, &kernel_buf[..write_size]) {
                        crate::process::pipe::PipeResult::Ok(n) => n as u64,
                        crate::process::pipe::PipeResult::WouldBlock => EAGAIN,
                        crate::process::pipe::PipeResult::BrokenPipe => u64::MAX,
                        crate::process::pipe::PipeResult::Eof => u64::MAX,
                    }
                } else {
                    // Handle stdin/stdout/stderr specially
                    match fd {
                        1 | 2 => {
                            // stdout/stderr: write to serial
                            if buf as u64 != 0 && count > 0 {
                                if !is_user_address(buf as u64)
                                    || !is_user_address((buf as u64) + count as u64)
                                {
                                    return u64::MAX;
                                }
                                let slice =
                                    unsafe { core::slice::from_raw_parts(buf, count.min(256)) };
                                if let Ok(s) = core::str::from_utf8(slice) {
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

fn sys_lseek(_frame: &mut SyscallFrame) -> u64 {
    crate::serial_println!("[SYSCALL] lseek requested");
    u64::MAX
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
    let fds_ptr = frame.rdi as *mut i32;

    // Check that the pointer is in user space
    if fds_ptr as u64 >= 0x0000_8000_0000_0000 {
        return u64::MAX; // EFAULT
    }

    let mut sched = crate::sched::SCHEDULER.lock();
    let mut pmm = crate::PMM.lock();

    match crate::process::pipe::create_pipe(&mut pmm, &mut sched) {
        Ok((read_fd, write_fd)) => {
            // Write the fds to user space
            unsafe {
                *fds_ptr = read_fd;
                *fds_ptr.add(1) = write_fd;
            }
            0 // Success
        }
        Err(_) => u64::MAX,
    }
}

fn sys_fstat(_frame: &mut SyscallFrame) -> u64 {
    crate::serial_println!("[SYSCALL] fstat requested");
    u64::MAX
}

fn sys_fcntl(_frame: &mut SyscallFrame) -> u64 {
    crate::serial_println!("[SYSCALL] fcntl requested");
    u64::MAX
}

// ---------------------------------------------------------------------------
// Phase 4.1: Memory management syscalls
// ---------------------------------------------------------------------------

/// Syscall: mmap (50)
/// rdi = addr (hint, 0 = kernel chooses)
/// rsi = length
/// rdx = prot (PROT_READ | PROT_WRITE | PROT_EXEC)
/// rcx = flags (MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED)
/// r8 = fd (-1 for anonymous)
/// r9 = offset
fn sys_mmap(frame: &mut SyscallFrame) -> u64 {
    let addr = frame.rdi;
    let length = frame.rsi;
    let prot = frame.rdx as u32;
    let flags = frame.rcx as u32;
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
        Err(_) => {
            crate::serial_println!("[SYSCALL] mmap failed ({:#x}, {:#x})", addr, length);
            u64::MAX
        }
    }
}

/// Syscall: munmap (51)
/// rdi = addr
/// rsi = length
fn sys_munmap(frame: &mut SyscallFrame) -> u64 {
    let addr = frame.rdi;
    let length = frame.rsi;

    match crate::process::mmap::sys_munmap(addr, length) {
        Ok(()) => 0,
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

    match crate::process::mmap::sys_mprotect(addr, length, prot) {
        Ok(()) => 0,
        Err(_) => u64::MAX,
    }
}

/// Syscall: mremap (53)
/// rdi = old_addr
/// rsi = old_size
/// rdx = new_size
/// rcx = flags
fn sys_mremap(frame: &mut SyscallFrame) -> u64 {
    let old_addr = frame.rdi;
    let old_size = frame.rsi;
    let new_size = frame.rdx;
    let flags = frame.rcx as u32;

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
fn sys_kill(_frame: &mut SyscallFrame) -> u64 {
    crate::serial_println!("[SYSCALL] kill requested");
    u64::MAX
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

/// Syscall: get_time_utc (81)
/// Returns the current Unix timestamp in seconds (RTC + tick advancement).
fn sys_get_time_utc() -> u64 {
    crate::arch::x86_64::rtc::unix_time()
}

/// Syscall: sysinfo (82)
/// rdi = user pointer to four u64s, filled as:
///   [0] total RAM (KiB)   [1] used RAM (KiB)
///   [2] uptime (seconds)  [3] Unix time (seconds)
fn sys_sysinfo(frame: &mut SyscallFrame) -> u64 {
    const SYSINFO_BYTES: u64 = 4 * 8;

    let ptr = frame.rdi;
    if !is_user_address(ptr) || !is_user_address(ptr + SYSINFO_BYTES - 1) {
        return u64::MAX;
    }

    let (total_frames, free_frames) = crate::PMM.lock().stats();
    // 4 KiB frames -> KiB
    let total_kb = (total_frames as u64) * 4;
    let used_kb = (total_frames.saturating_sub(free_frames) as u64) * 4;

    let info = [
        total_kb,
        used_kb,
        crate::arch::x86_64::rtc::uptime_secs(),
        crate::arch::x86_64::rtc::unix_time(),
    ];
    unsafe {
        core::ptr::copy_nonoverlapping(info.as_ptr(), ptr as *mut u64, info.len());
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
