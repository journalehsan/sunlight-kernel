use core::arch::naked_asm;
use x86_64::VirtAddr;

/// Syscall numbers for SunlightOS
#[repr(u64)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SunlightSyscall {
    IpcSend = 1,
    IpcRecv = 2,
    IpcCall = 3,
    CapDup = 10,
    CapRevoke = 11,
    ProcessExit = 20,
    ProcessYield = 21,
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
pub struct SyscallFrame {
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

/// Syscall dispatch — called from assembly with pointer to saved frame.
/// Returns the value to put in RAX.
/// SAFETY: `frame` must point to a valid SyscallFrame on the stack.
#[no_mangle]
pub extern "C" fn syscall_dispatch(frame: &mut SyscallFrame) -> u64 {
    let num = frame.rax;
    crate::serial_println!(
        "[SYSCALL] dispatch rax={:#x} rdi={:#x} rsi={:#x} rdx={:#x}",
        num, frame.rdi, frame.rsi, frame.rdx
    );
    match num {
        1 => ipc_send(frame.rdi, frame.rsi, frame.rdx),
        2 => ipc_recv(frame.rdi),
        20 => process_exit(frame.rdi as i32),
        21 => process_yield(),
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

use crate::ipc::IpcError;
use crate::capability::CapabilityToken;
use crate::process::{IpcMessage, ProcessState};
use crate::sched;
use crate::process::layout::is_user_address;

/// Syscall: IpcSend
/// rdi = capability token
/// rsi = pointer to IpcMessage in user space
/// rdx = data length
fn ipc_send(token: u64, msg_ptr: u64, _len: u64) -> u64 {
    // SAFETY: Validate user pointer before dereferencing.
    if !is_user_address(msg_ptr) {
        return IpcError::InvalidArgument as u64;
    }

    // SAFETY: msg_ptr is validated to be in user space.
    let user_msg = unsafe { &*(msg_ptr as *const IpcMessage) };

    let mut msg = IpcMessage::new(user_msg.tag);
    let data_len = (user_msg.len as usize).min(crate::process::IPC_INLINE_MAX);
    msg.len = data_len as u32;
    msg.data[..data_len].copy_from_slice(&user_msg.data[..data_len]);

    let token = CapabilityToken(token);

    // Access IPC bus and capability broker
    let mut bus = crate::ipc::IPC_BUS.lock();
    let caps = crate::capability::CAP_BROKER.lock();
    let sender_pid = sched::with_scheduler(|s| s.current_process().pid);

    match bus.send(token, msg, &caps, sender_pid) {
        Ok(()) => 0,
        Err(e) => e as u64,
    }
}

/// Syscall: IpcRecv
/// rdi = pointer to IpcMessage buffer in user space
fn ipc_recv(msg_ptr: u64) -> u64 {
    if !is_user_address(msg_ptr) {
        return IpcError::InvalidArgument as u64;
    }

    // SAFETY: msg_ptr validated to be in user space.
    let user_msg = unsafe { &mut *(msg_ptr as *mut IpcMessage) };

    let _pid = sched::with_scheduler(|s| s.current_process().pid);

    // Check the current process's own queue first.
    let msg = sched::with_scheduler(|s| s.current_process_mut().ipc_queue.pop_front());

    if let Some(msg) = msg {
        let data_len = (msg.len as usize).min(crate::process::IPC_INLINE_MAX);
        user_msg.sender_pid = msg.sender_pid;
        user_msg.endpoint_id = msg.endpoint_id;
        user_msg.tag = msg.tag;
        user_msg.capability = msg.capability;
        user_msg.len = msg.len;
        user_msg.data[..data_len].copy_from_slice(&msg.data[..data_len]);
        return 0;
    }

    // No message - block.
    sched::with_scheduler(|s| {
        s.current_process_mut().state = ProcessState::BlockedOnIpc;
    });
    IpcError::WouldBlock as u64
}

/// Syscall: ProcessExit
/// rdi = exit code
fn process_exit(_code: i32) -> u64 {
    sched::with_scheduler(|s| {
        s.current_process_mut().state = ProcessState::Finished;
    });
    // Request an immediate reschedule.
    sched::request_reschedule();
    0
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
