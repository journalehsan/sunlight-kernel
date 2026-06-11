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

/// Syscall dispatch — called from assembly with pointer to saved frame.
/// Returns the value to put in RAX.
/// SAFETY: `frame` must point to a valid SyscallFrame on the stack.
#[no_mangle]
pub extern "C" fn syscall_dispatch(frame: &mut SyscallFrame) -> u64 {
    let num = frame.rax;
    match num {
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
        99 => debug_log(frame.rdi, frame.rsi),
        _ => {
            crate::serial_println!("[SYSCALL] Unknown syscall {}", num);
            u64::MAX
        }
    }
}

// ---------------------------------------------------------------------------
// Individual syscall implementations
// ---------------------------------------------------------------------------

use crate::ipc::{IpcError, IpcMsg, INIT_NAMESERVER_ENDPOINT};
use crate::capability::CapabilityToken;
use crate::capability::CapabilityRights;
use crate::process::ProcessState;
use crate::sched;
use crate::process::layout::is_user_address;

fn ipc_call(frame: &mut SyscallFrame) -> u64 {
    let token = CapabilityToken(frame.rsi);
    let msg = IpcMsg::from_registers(frame);

    // Check for spawn capability (fast path handled by kernel)
    if token == crate::capability::SPAWN_TOKEN {
        return handle_spawn_call(frame, msg);
    }

    let mut bus = crate::ipc::IPC_BUS.lock();
    let caps = crate::capability::CAP_BROKER.lock();
    let mut sched = crate::sched::SCHEDULER.lock();
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
    let _uid = msg.words[4] as u32;
    let _gid = msg.words[5] as u32;

    crate::serial_println!("[SPAWN] Request from pid={} for path={}",
        crate::sched::SCHEDULER.lock().current_process().pid,
        path);

    let mut pmm = crate::PMM.lock();
    let mut sched = crate::sched::SCHEDULER.lock();
    let hhdm = crate::HHDM_REQ.response().expect("no hhdm").offset;

    match crate::process::spawn::spawn_from_path(
        &path,
        &[],
        &mut *pmm,
        &mut *sched,
        &mut crate::capability::CAP_BROKER.lock(),
        VirtAddr::new(hhdm),
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
    let mut bus = crate::ipc::IPC_BUS.lock();
    let mut sched = crate::sched::SCHEDULER.lock();
    let server_pid = sched.current_process().pid;
    match crate::ipc::handle_ipc_reply(server_pid, reply, &mut sched, &mut bus) {
        Ok(()) => 0,
        Err(e) => e as u64,
    }
}

fn ipc_reply_wait(frame: &mut SyscallFrame) -> u64 {
    let endpoint_token = CapabilityToken(frame.rsi);
    let reply = IpcMsg::from_registers(frame);
    let mut bus = crate::ipc::IPC_BUS.lock();
    let caps = crate::capability::CAP_BROKER.lock();
    let mut sched = crate::sched::SCHEDULER.lock();
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
    let mut bus = crate::ipc::IPC_BUS.lock();
    let caps = crate::capability::CAP_BROKER.lock();
    let mut sched = crate::sched::SCHEDULER.lock();
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
    let slice = unsafe {
        core::slice::from_raw_parts(ptr as *const u8, len.min(256) as usize)
    };

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
    let mut pmm = crate::PMM.lock();
    let mut sched = crate::sched::SCHEDULER.lock();
    let hhdm = crate::HHDM_REQ.response().expect("no hhdm").offset;

    // Borrow the parent process momentarily to fork it
    let parent_pid = sched.current_process().pid;
    match crate::process::fork::fork_current_process(
        &mut *pmm,
        &mut *sched,
        VirtAddr::new(hhdm),
    ) {
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
fn sys_exec(_frame: &mut SyscallFrame) -> u64 {
    crate::serial_println!("[SYSCALL] exec requested");
    u64::MAX
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
    // TODO: track uid in process
    0
}

/// Syscall: Getgid (36)
fn sys_getgid() -> u64 {
    // TODO: track gid in process
    0
}

/// Syscall: Setuid (37)
fn sys_setuid(_frame: &mut SyscallFrame) -> u64 {
    // TODO: implement setuid (requires root)
    IpcError::InvalidArgument as u64
}

/// Syscall: Setgid (38)
fn sys_setgid(_frame: &mut SyscallFrame) -> u64 {
    // TODO: implement setgid (requires root)
    IpcError::InvalidArgument as u64
}

// File descriptor syscalls (stubs for now)
fn sys_open(_frame: &mut SyscallFrame) -> u64 {
    crate::serial_println!("[SYSCALL] open requested");
    u64::MAX
}

fn sys_close(_frame: &mut SyscallFrame) -> u64 {
    crate::serial_println!("[SYSCALL] close requested");
    u64::MAX
}

fn sys_read(_frame: &mut SyscallFrame) -> u64 {
    crate::serial_println!("[SYSCALL] read requested");
    u64::MAX
}

fn sys_write(_frame: &mut SyscallFrame) -> u64 {
    crate::serial_println!("[SYSCALL] write requested");
    u64::MAX
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

fn sys_pipe(_frame: &mut SyscallFrame) -> u64 {
    crate::serial_println!("[SYSCALL] pipe requested");
    u64::MAX
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

    let mut pmm = crate::PMM.lock();
    let mut sched = crate::sched::SCHEDULER.lock();

    match crate::process::mmap::sys_mmap(addr, length, prot, flags, fd, offset, &mut *pmm, &mut *sched) {
        Ok(mapped_addr) => {
            crate::serial_println!("[SYSCALL] mmap({:#x}, {:#x}) -> {:#x}", addr, length, mapped_addr);
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
