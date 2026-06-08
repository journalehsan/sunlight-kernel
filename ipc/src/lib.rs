#![no_std]

pub const IPC_MAX_WORDS: usize = 8;
pub const IPC_MAX_CAPS: usize = 2;
pub const INIT_NAMESERVER_ENDPOINT: u64 = 0;

/// Syscall numbers for SunlightOS.
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
    DebugLog = 99,
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapabilityToken(pub u64);

impl CapabilityToken {
    pub const INVALID: Self = Self(0);
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EndpointId(pub u64);

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct IpcMsg {
    pub label: u64,
    pub badge: u64,
    pub word_count: u32,
    pub cap_count: u32,
    pub words: [u64; IPC_MAX_WORDS],
    pub caps: [CapabilityToken; IPC_MAX_CAPS],
}

impl IpcMsg {
    pub const fn empty() -> Self {
        Self {
            label: 0,
            badge: 0,
            word_count: 0,
            cap_count: 0,
            words: [0; IPC_MAX_WORDS],
            caps: [CapabilityToken::INVALID; IPC_MAX_CAPS],
        }
    }

    pub const fn with_label(label: u64) -> Self {
        Self {
            label,
            badge: 0,
            word_count: 0,
            cap_count: 0,
            words: [0; IPC_MAX_WORDS],
            caps: [CapabilityToken::INVALID; IPC_MAX_CAPS],
        }
    }

    pub fn word(mut self, idx: usize, val: u64) -> Self {
        if idx < IPC_MAX_WORDS {
            self.words[idx] = val;
            let count = (idx + 1) as u32;
            if self.word_count < count {
                self.word_count = count;
            }
        }
        self
    }
}

#[allow(non_snake_case)]
pub mod InitMsg {
    pub const REGISTER: u64 = 1;
    pub const LOOKUP: u64 = 2;
    pub const GRANT: u64 = 3;
    pub const DENY: u64 = 4;
}

#[allow(non_snake_case)]
pub mod TimerMsg {
    pub const TICK: u64 = 1;
    pub const GET_TICKS: u64 = 2;
    pub const REPLY: u64 = 3;
    pub const ERROR: u64 = 4;
}

#[allow(non_snake_case)]
pub mod VfsMsg {
    pub const OPEN: u64 = 1;
    pub const READ: u64 = 2;
    pub const CLOSE: u64 = 3;
    pub const STAT: u64 = 4;
    pub const REPLY: u64 = 5;
    pub const ERROR: u64 = 6;
}

/// Errors returned by IPC operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpcError {
    InvalidCapability = 1,
    EndpointNotFound = 2,
    WouldBlock = 3,
    InvalidArgument = 4,
}

#[inline(always)]
unsafe fn raw_syscall(
    num: SunlightSyscall,
    a1: u64,
    a2: u64,
    a3: u64,
    a4: u64,
    a5: u64,
    a6: u64,
    a7: u64,
) -> (u64, IpcMsg) {
    let ret: u64;
    let out_rdi: u64;
    let out_rsi: u64;
    let out_rdx: u64;
    let out_r8: u64;
    let out_r9: u64;
    let out_r10: u64;
    let out_r12: u64;
    let out_r13: u64;
    let out_r14: u64;
    // SAFETY: caller selects a valid syscall number and passes ABI-shaped arguments.
    unsafe {
        core::arch::asm!(
            "syscall",
            inlateout("rax") num as u64 => ret,
            inlateout("rdi") a1 => out_rdi,
            inlateout("rsi") a2 => out_rsi,
            inlateout("rdx") a3 => out_rdx,
            inlateout("r8") a4 => out_r8,
            inlateout("r9") a5 => out_r9,
            inlateout("r10") a6 => out_r10,
            inlateout("r12") a7 => out_r12,
            lateout("r13") out_r13,
            lateout("r14") out_r14,
            lateout("rcx") _,
            lateout("r11") _,
        );
    }
    let mut msg = IpcMsg::with_label(out_rdi);
    msg.badge = out_rsi;
    msg.word_count = (out_rdx & 0xffff_ffff) as u32;
    msg.cap_count = (out_rdx >> 32) as u32;
    msg.words[0] = out_r8;
    msg.words[1] = out_r9;
    msg.words[2] = out_r10;
    msg.words[3] = out_r12;
    msg.caps[0] = CapabilityToken(out_r13);
    msg.caps[1] = CapabilityToken(out_r14);
    (ret, msg)
}

#[inline(always)]
unsafe fn raw_syscall_ipc(num: SunlightSyscall, object: u64, msg: IpcMsg) -> (u64, IpcMsg) {
    let counts = msg.word_count as u64 | ((msg.cap_count as u64) << 32);
    // SAFETY: forwarded to the raw syscall wrapper with register-shaped IPC fields.
    unsafe {
        raw_syscall(
            num,
            msg.label,
            object,
            counts,
            msg.words[0],
            msg.words[1],
            msg.words[2],
            msg.words[3],
        )
    }
}

#[inline(always)]
fn would_block(ret: u64) -> bool {
    ret == IpcError::WouldBlock as u64
}

pub fn endpoint_create() -> EndpointId {
    // SAFETY: EndpointCreate takes no user pointers.
    let (ret, _) = unsafe { raw_syscall(SunlightSyscall::EndpointCreate, 0, 0, 0, 0, 0, 0, 0) };
    EndpointId(ret)
}

pub fn endpoint_bind(endpoint: u64) -> CapabilityToken {
    // SAFETY: EndpointBind accepts an opaque endpoint selector/token.
    let (ret, _) = unsafe {
        raw_syscall(SunlightSyscall::EndpointBind, endpoint, 0, 0, 0, 0, 0, 0)
    };
    CapabilityToken(ret)
}

pub fn get_init_cap() -> CapabilityToken {
    endpoint_bind(INIT_NAMESERVER_ENDPOINT)
}

/// Client: send a message and block until reply.
pub fn ipc_call(cap: CapabilityToken, msg: IpcMsg) -> IpcMsg {
    loop {
        // SAFETY: ipc_call passes a capability token and fixed register IPC message.
        let (ret, reply) = unsafe { raw_syscall_ipc(SunlightSyscall::IpcCall, cap.0, msg) };
        if !would_block(ret) {
            return reply;
        }
        process_yield();
    }
}

/// Server: block waiting for first incoming call.
pub fn ipc_recv(ep: EndpointId) -> IpcMsg {
    loop {
        // SAFETY: ipc_recv passes the endpoint owner token; kernel validates receive rights.
        let (ret, msg) = unsafe { raw_syscall_ipc(SunlightSyscall::IpcRecv, ep.0, IpcMsg::empty()) };
        if !would_block(ret) {
            return msg;
        }
        process_yield();
    }
}

pub fn ipc_reply(reply: IpcMsg) {
    // SAFETY: ipc_reply sends a fixed register IPC message to the current reply waiter.
    unsafe {
        raw_syscall_ipc(SunlightSyscall::IpcReply, 0, reply);
    }
}

/// Server: send reply and block for the next call.
pub fn ipc_reply_and_wait(ep: EndpointId, reply: IpcMsg) -> IpcMsg {
    loop {
        // SAFETY: ipc_reply_and_wait passes the endpoint owner token and fixed reply message.
        let (ret, msg) = unsafe { raw_syscall_ipc(SunlightSyscall::IpcReplyWait, ep.0, reply) };
        if !would_block(ret) {
            return msg;
        }
        process_yield();
    }
}

pub fn notify_send(cap: CapabilityToken) {
    // SAFETY: notify_send passes only an opaque capability token.
    unsafe {
        raw_syscall(SunlightSyscall::IpcNotifySend, cap.0, 0, 0, 0, 0, 0, 0);
    }
}

pub fn notify_wait(ep: EndpointId) {
    loop {
        // SAFETY: notify_wait passes only an opaque endpoint token.
        let (ret, _) = unsafe {
            raw_syscall(SunlightSyscall::IpcNotifyWait, ep.0, 0, 0, 0, 0, 0, 0)
        };
        if !would_block(ret) {
            return;
        }
        process_yield();
    }
}

pub fn nameserver_register(name: &str, ep: EndpointId) {
    let init_cap = get_init_cap();
    let msg = IpcMsg::with_label(InitMsg::REGISTER)
        .word(0, name_to_u64(name))
        .word(1, ep.0);
    let _ = ipc_call(init_cap, msg);
}

pub fn nameserver_lookup(name: &str) -> Option<CapabilityToken> {
    let init_cap = get_init_cap();
    let msg = IpcMsg::with_label(InitMsg::LOOKUP).word(0, name_to_u64(name));
    let reply = ipc_call(init_cap, msg);
    if reply.label == InitMsg::GRANT {
        Some(CapabilityToken(reply.words[0]))
    } else {
        None
    }
}

pub fn name_to_u64(name: &str) -> u64 {
    let bytes = name.as_bytes();
    let mut out = 0u64;
    let mut i = 0;
    while i < bytes.len() && i < 8 {
        out |= (bytes[i] as u64) << (i * 8);
        i += 1;
    }
    out
}

pub fn debug_log(msg: &str) {
    // SAFETY: DebugLog receives a valid string pointer and bounded length.
    unsafe {
        raw_syscall(
            SunlightSyscall::DebugLog,
            msg.as_ptr() as u64,
            msg.len() as u64,
            0,
            0,
            0,
            0,
            0,
        );
    }
}

pub fn process_yield() {
    // SAFETY: ProcessYield takes no user pointers.
    unsafe {
        raw_syscall(SunlightSyscall::ProcessYield, 0, 0, 0, 0, 0, 0, 0);
    }
}

pub struct ProcessExit;
impl ProcessExit {
    pub fn exit(code: i32) -> ! {
        // SAFETY: ProcessExit terminates the current process.
        unsafe {
            raw_syscall(
                SunlightSyscall::ProcessExit,
                code as u64,
                0,
                0,
                0,
                0,
                0,
                0,
            );
        }
        loop {
            // SAFETY: hlt is safe in a non-returning fallback loop.
            unsafe {
                core::arch::asm!("hlt", options(nomem, nostack));
            }
        }
    }
}

pub mod process_exit {
    pub use super::ProcessExit;
}
