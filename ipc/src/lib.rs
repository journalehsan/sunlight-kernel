#![no_std]

/// Maximum inline message payload: 240 bytes (fits in 3 cache lines with header)
pub const IPC_INLINE_MAX: usize = 240;

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

/// An IPC message sent between processes
#[repr(C)]
#[derive(Clone, Copy)]
pub struct IpcMessage {
    pub sender_pid: u32,
    pub endpoint_id: u32,
    pub tag: u64,
    pub capability: u64, // 0 means none
    pub len: u32,
    pub data: [u8; IPC_INLINE_MAX],
}

impl IpcMessage {
    pub const fn new(tag: u64) -> Self {
        Self {
            sender_pid: 0,
            endpoint_id: 0,
            tag,
            capability: 0,
            len: 0,
            data: [0; IPC_INLINE_MAX],
        }
    }
}

/// Timer message tags
pub struct TimerMessage;
impl TimerMessage {
    pub const TICK: u64 = 0x1;
}

/// Errors returned by IPC operations
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpcError {
    InvalidCapability = 1,
    EndpointNotFound = 2,
    WouldBlock = 3,
    InvalidArgument = 4,
}

/// Raw syscall instruction wrapper.
/// SAFETY: `num` must be a valid syscall number. Arguments must be valid for the syscall.
#[inline(always)]
pub unsafe fn raw_syscall(num: u64, a1: u64, a2: u64, a3: u64) -> u64 {
    let ret: u64;
    core::arch::asm!(
        "syscall",
        in("rax") num,
        in("rdi") a1,
        in("rsi") a2,
        in("rdx") a3,
        lateout("rax") ret,
        lateout("rcx") _,
        lateout("r11") _,
    );
    ret
}

/// Send a message via SYSCALL IpcSend.
/// `cap` is the capability token for the endpoint.
pub fn ipc_send(cap: u64, tag: u64, data: &[u8]) -> Result<(), IpcError> {
    let len = data.len().min(IPC_INLINE_MAX);
    let mut msg = IpcMessage::new(tag);
    msg.len = len as u32;
    msg.data[..len].copy_from_slice(data);

    // SAFETY: IpcSend syscall with valid capability and message pointer.
    let ret = unsafe {
        raw_syscall(
            SunlightSyscall::IpcSend as u64,
            cap,
            &msg as *const _ as u64,
            len as u64,
        )
    };
    if ret == 0 {
        Ok(())
    } else {
        Err(IpcError::InvalidCapability)
    }
}

/// Block until a message arrives via SYSCALL IpcRecv.
/// Returns the received message.
pub fn ipc_recv() -> IpcMessage {
    let mut msg = IpcMessage::new(0);
    // SAFETY: IpcRecv syscall with valid message pointer.
    unsafe {
        raw_syscall(
            SunlightSyscall::IpcRecv as u64,
            &mut msg as *mut _ as u64,
            0,
            0,
        );
    }
    msg
}

/// Write a debug string to kernel serial log via SYSCALL DebugLog.
pub fn debug_log(msg: &str) {
    // SAFETY: DebugLog syscall with valid string pointer and length.
    unsafe {
        raw_syscall(
            SunlightSyscall::DebugLog as u64,
            msg.as_ptr() as u64,
            msg.len() as u64,
            0,
        );
    }
}

/// Voluntarily yield CPU via SYSCALL ProcessYield.
pub fn process_yield() {
    // SAFETY: ProcessYield syscall has no arguments.
    unsafe {
        raw_syscall(SunlightSyscall::ProcessYield as u64, 0, 0, 0);
    }
}

/// Terminate current process via SYSCALL ProcessExit.
pub struct ProcessExit;
impl ProcessExit {
    pub fn exit(code: i32) -> ! {
        // SAFETY: ProcessExit syscall terminates the process; never returns.
        unsafe {
            raw_syscall(SunlightSyscall::ProcessExit as u64, code as u64, 0, 0);
        }
        // Fallback infinite loop in case syscall returns (should not happen).
        loop {
            // SAFETY: hlt is safe in a fallback loop.
            unsafe {
                core::arch::asm!("hlt", options(nomem, nostack));
            }
        }
    }
}

/// Re-export for services that use the old name.
pub mod process_exit {
    pub use super::ProcessExit;
}
