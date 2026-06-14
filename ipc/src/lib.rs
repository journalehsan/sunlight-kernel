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
    // NOTE: 50 belongs to sys_mmap in the kernel dispatcher — GetTimeUtc
    // previously sat there and silently invoked mmap.
    GetTimeUtc = 81,
    SysInfo = 82,
    // Phase 3.4: net_server (pid 5) frame proxy — kernel owns the virtio-net
    // device (ring-0 port I/O); these exchange raw Ethernet frames.
    NetTx = 90,
    NetRx = 91,
    // Shared memory grant for large zero-copy IPC (Bite 4)
    ShmAlloc = 92,
    ShmMap = 93,
    ShmFree = 94,
    DebugLog = 99,
}

/// System statistics filled by the SysInfo syscall (kernel writes four u64s).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemInfo {
    pub total_ram_kb: u64,
    pub used_ram_kb: u64,
    pub uptime_secs: u64,
    pub unix_time: u64,
    pub swap_total_kb: u64,
    pub swap_used_kb: u64,
    pub swap_compressed_kb: u64,
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

    pub fn with_cap(mut self, idx: usize, val: CapabilityToken) -> Self {
        if idx < IPC_MAX_CAPS {
            self.caps[idx] = val;
            let count = (idx + 1) as u32;
            if self.cap_count < count {
                self.cap_count = count;
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
pub mod TimeMsg {
    pub const GET_TIME: u64 = 1;           // Query current UTC time
    pub const GET_STATE: u64 = 2;          // Get full TimeState (back-compat: tz fields are 0)
    pub const SET_TIMEZONE: u64 = 3;       // No-op (timezone moved to "tz")
    pub const SYNC_NTP: u64 = 4;           // Trigger NTP sync
    pub const GET_UTC: u64 = 5;            // Preferred alias for GET_TIME (pure UTC)
    pub const REPLY: u64 = 100;
    pub const ERROR: u64 = 101;
}

/// Timezone service opcodes (registered as "tz")
#[allow(non_snake_case)]
pub mod TzMsg {
    pub const GET_LOCAL_TIME: u64 = 0x7001;
    pub const GET_ZONE:       u64 = 0x7002;
    pub const SET_ZONE:       u64 = 0x7003;  // arg: zone id in data[0..64]
    pub const LIST_ZONES:     u64 = 0x7004;  // arg: page in word(0), 8 per page (but one zone per reply for packing)
    pub const NOTIFY_CHANGED: u64 = 0x7005;  // sent TO timed after SET_ZONE (best effort)
    pub const REPLY:          u64 = 0x70FF;
    pub const ERROR:          u64 = 0x70FE;
}

#[allow(non_snake_case)]
pub mod VfsMsg {
    pub const OPEN: u64 = 1;
    pub const READ: u64 = 2;
    pub const CLOSE: u64 = 3;
    pub const STAT: u64 = 4;
    pub const REPLY: u64 = 5;
    pub const ERROR: u64 = 6;
    pub const WRITE: u64 = 7;
    pub const MKDIR: u64 = 8;
    pub const CHMOD: u64 = 9;
    pub const CHOWN: u64 = 10;
    pub const GETPWNAM: u64 = 11;  // Get user info by username
    pub const GETGRGID: u64 = 12;  // Get group info by gid
    pub const GETPWUID: u64 = 13;  // Get user info by uid
    pub const DATA_SHARED: u64 = 31;  // large read reply carries cap in caps[0]
}

#[allow(non_snake_case)]
pub mod KbdMsg {
    pub const KEY_EVENT: u64 = 1;
}

#[allow(non_snake_case)]
pub mod SpawnMsg {
    pub const SPAWN: u64 = 1;
    pub const REPLY: u64 = 2;
    pub const ERROR: u64 = 3;
}

/// Pack a key event into a single u64 word for IPC transport.
/// Layout: keycode(u8) | pressed(u8) << 8 | mods_byte(u8) << 16 | ascii(u8) << 24
pub fn pack_key_event(keycode: u8, pressed: bool, shift: bool, ctrl: bool, alt: bool, ascii: Option<u8>) -> u64 {
    let mut val = keycode as u64;
    val |= (pressed as u64) << 8;
    let mods = ((shift as u64) << 0) | ((ctrl as u64) << 1) | ((alt as u64) << 2);
    val |= mods << 16;
    val |= (ascii.unwrap_or(0) as u64) << 24;
    val
}

/// Unpack a key event from a u64 word.
pub fn unpack_key_event(val: u64) -> (u8, bool, bool, bool, bool, Option<u8>) {
    let keycode = (val & 0xFF) as u8;
    let pressed = ((val >> 8) & 0xFF) != 0;
    let mods = (val >> 16) & 0xFF;
    let shift = (mods & 1) != 0;
    let ctrl = (mods & 2) != 0;
    let alt = (mods & 4) != 0;
    let ascii = if (val >> 24) & 0xFF != 0 { Some(((val >> 24) & 0xFF) as u8) } else { None };
    (keycode, pressed, shift, ctrl, alt, ascii)
}

/// Errors returned by IPC operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpcError {
    InvalidCapability = 1,
    EndpointNotFound = 2,
    WouldBlock = 3,
    InvalidArgument = 4,
}

/// Errors from shared memory grant syscalls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShmError {
    OutOfMemory = 1,
    InvalidToken = 2,
    InvalidArgument = 3,
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

/// Server: send reply, then make a single non-blocking attempt to receive the
/// next call. Returns `None` (instead of yield-looping forever) if no call is
/// pending yet, so callers can do periodic work (e.g. clock refresh) while
/// waiting for the next message.
pub fn ipc_reply_and_try_recv(ep: EndpointId, reply: IpcMsg) -> Option<IpcMsg> {
    // SAFETY: ipc_reply_and_try_recv passes the endpoint owner token and fixed reply message.
    let (ret, msg) = unsafe { raw_syscall_ipc(SunlightSyscall::IpcReplyWait, ep.0, reply) };
    if would_block(ret) {
        None
    } else {
        Some(msg)
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

pub fn get_time_utc() -> u64 {
    // SAFETY: GetTimeUtc takes no user pointers.
    let (ret, _) = unsafe { raw_syscall(SunlightSyscall::GetTimeUtc, 0, 0, 0, 0, 0, 0, 0) };
    ret
}

/// Phase 3.4: hand a raw Ethernet frame to the kernel-owned virtio-net
/// device for transmission. Returns `true` on success. Restricted to the
/// net_server process (pid 5) by the kernel.
pub fn net_tx(frame: &[u8]) -> bool {
    // SAFETY: passes a read-only pointer/length describing `frame` to the
    // kernel, which copies it into its own TX buffer before returning.
    let (ret, _) = unsafe {
        raw_syscall(SunlightSyscall::NetTx, frame.as_ptr() as u64, frame.len() as u64, 0, 0, 0, 0, 0)
    };
    ret == 1
}

/// Phase 3.4: poll the kernel-owned virtio-net device's RX queue for one
/// frame, copying up to `buf.len()` bytes in. Returns the frame length, or
/// `0` if no frame is pending.
pub fn net_rx(buf: &mut [u8]) -> usize {
    // SAFETY: passes a writable pointer/capacity for `buf`; the kernel
    // bounds-checks and copies at most `buf.len()` bytes into it.
    let (ret, _) = unsafe {
        raw_syscall(SunlightSyscall::NetRx, buf.as_mut_ptr() as u64, buf.len() as u64, 0, 0, 0, 0, 0)
    };
    ret as usize
}

pub fn sysinfo() -> SystemInfo {
    let mut info = SystemInfo::default();
    // SAFETY: passes a pointer to a properly sized and aligned SystemInfo that
    // the kernel fills with four u64s.
    unsafe {
        raw_syscall(
            SunlightSyscall::SysInfo,
            &mut info as *mut SystemInfo as u64,
            0,
            0,
            0,
            0,
            0,
            0,
        );
    }
    info
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

/// Allocate a shared physical page. Returns (local virtual ptr in caller AS, capability token to send to receiver).
pub fn shm_alloc() -> Result<(*mut u8, CapabilityToken), ShmError> {
    let (ret, msg) = unsafe { raw_syscall(SunlightSyscall::ShmAlloc, 0, 0, 0, 0, 0, 0, 0) };
    if ret == u64::MAX || msg.caps[0] == CapabilityToken::INVALID {
        return Err(ShmError::OutOfMemory);
    }
    Ok((ret as *mut u8, msg.caps[0]))
}

/// Map a shared page into the caller's AS using a received token. Returns local ptr.
pub fn shm_map(token: CapabilityToken) -> Result<*mut u8, ShmError> {
    let (ret, _) = unsafe { raw_syscall(SunlightSyscall::ShmMap, token.0, 0, 0, 0, 0, 0, 0) };
    if ret == u64::MAX {
        return Err(ShmError::InvalidToken);
    }
    Ok(ret as *mut u8)
}

/// Unmap and (if owner) release the shared page grant.
pub fn shm_free(token: CapabilityToken) -> Result<(), ShmError> {
    let (ret, _) = unsafe { raw_syscall(SunlightSyscall::ShmFree, token.0, 0, 0, 0, 0, 0, 0) };
    if ret == 0 {
        Ok(())
    } else {
        Err(ShmError::InvalidToken)
    }
}
