#![no_std]

pub const IPC_MAX_WORDS: usize = 8;
/// Register IPC currently transports only `words[0..4)` via r8/r9/r10/r12.
pub const IPC_REGISTER_WORDS: usize = 4;
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
    IpcCancel = 7,
    EndpointCreate = 10,
    EndpointBind = 11,
    ProcessExit = 20,
    ProcessYield = 21,
    ThreadSpawn = 22,
    // TTY mux for foreground input routing — see kernel process::tty_io.
    TtyStdinPush = 23,
    TtyStdoutPull = 24,
    ProcessIsAlive = 25,
    Kill = 72,
    // NOTE: 50 belongs to sys_mmap in the kernel dispatcher — GetTimeUtc
    // previously sat there and silently invoked mmap.
    GetTimeUtc = 81,
    MonotonicMs = 86,
    SysInfo = 82,
    SetNice = 83,
    GetNice = 84,
    // Phase 3.4: net_server (pid 5) frame proxy — kernel owns the virtio-net
    // device (ring-0 port I/O); these exchange raw Ethernet frames.
    NetTx = 90,
    NetRx = 91,
    // Shared memory grant for large zero-copy IPC (Bite 4)
    ShmAlloc = 92,
    ShmMap = 93,
    ShmFree = 94,
    MapTelemetry = 95,
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
    pub const GET_TIME: u64 = 1; // Query current UTC time
    pub const GET_STATE: u64 = 2; // Get full TimeState (back-compat: tz fields are 0)
    pub const SET_TIMEZONE: u64 = 3; // No-op (timezone moved to "tz")
    pub const SYNC_NTP: u64 = 4; // Trigger NTP sync
    pub const GET_UTC: u64 = 5; // Preferred alias for GET_TIME (pure UTC)
    pub const REPLY: u64 = 100;
    pub const ERROR: u64 = 101;
}

/// Timezone service opcodes (registered as "tz")
#[allow(non_snake_case)]
pub mod TzMsg {
    pub const GET_LOCAL_TIME: u64 = 0x7001;
    pub const GET_ZONE: u64 = 0x7002;
    pub const SET_ZONE: u64 = 0x7003; // arg: zone id in data[0..64]
    pub const LIST_ZONES: u64 = 0x7004; // arg: page in word(0), 8 per page (but one zone per reply for packing)
    pub const NOTIFY_CHANGED: u64 = 0x7005; // sent TO timed after SET_ZONE (best effort)
    pub const REPLY: u64 = 0x70FF;
    pub const ERROR: u64 = 0x70FE;
}

/// Random service opcodes (registered as "rand").
///
/// The crypto path is intentionally chunked: each GET asks for up to 32 bytes
/// (`words[0..3]`, the real register-IPC inline budget — see `raw_syscall_ipc`)
/// and the reply packs that many bytes back into `words[0..3]`. Callers wanting
/// more than 32 bytes loop. This avoids any shared-memory cap-transfer.
#[allow(non_snake_case)]
pub mod RandMsg {
    /// Request random bytes. `words[0]` = requested length (clamped to 32).
    pub const GET: u64 = 0x7201;
    /// Reply carrying exactly the requested byte count in `words[0..3]`.
    pub const REPLY: u64 = 0x72FF;
    pub const ERROR: u64 = 0x72FE;
    /// Maximum bytes returned per GET (4 transiting words).
    pub const MAX_CHUNK: usize = 32;
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
    pub const GETPWNAM: u64 = 11; // Get user info by username
    pub const GETGRGID: u64 = 12; // Get group info by gid
    pub const GETPWUID: u64 = 13; // Get user info by uid
    pub const FSTAT: u64 = 14; // Stat an open file handle
    pub const UNLINK: u64 = 15; // Remove a file (path in words[0..3])
    pub const RENAME: u64 = 16; // Rename: src path words[0..3], dst path words[4..7]
    pub const DATA_SHARED: u64 = 31; // large read reply carries cap in caps[0]
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

/// sunlight-sm (Storage Manager) opcodes. Registered as "sm".
/// Uses shm for payload (path + optional content) to support file sizes > inline.
#[allow(non_snake_case)]
pub mod SmMsg {
    pub const WRITE_FILE: u64 = 1;
    pub const MKDIR_ALL: u64 = 2;
    pub const REMOVE: u64 = 3;
    pub const READ_FILE: u64 = 4;
    // OP 5 batch left for future; current bite keeps simple per-op IPC.

    pub const REPLY_OK: u64 = 1;
    pub const REPLY_ERR: u64 = 0xff;

    /// Max bytes for path+content in one shm page grant.
    pub const PAGE_CAPACITY: usize = 4096;

    // Error codes returned in reply.words[0] when label==REPLY_ERR
    pub const ERR_OK: u64 = 0;
    pub const ERR_DENIED: u64 = 1;
    pub const ERR_INVALID_PATH: u64 = 2;
    pub const ERR_PAYLOAD_TOO_LARGE: u64 = 3;
    pub const ERR_NOT_FOUND: u64 = 4;
    pub const ERR_IO: u64 = 5;
    pub const ERR_UNSUPPORTED: u64 = 6;
}

/// Structured spawn request that carries both a binary path and an explicit
/// process name. Designed to fit within the register IPC budget.
///
/// Wire layout (register IPC — 4 available words via r8/r9/r10/r12):
///   words[0..3]: path, NUL-terminated, up to 32 bytes
///   words[4..5]: name hint, NUL-terminated, up to 16 bytes
///                (NOT transmitted by the register-based IPC path today;
///                 the kernel derives the name from the path basename instead.
///                 Reserved for future memory-mapped IPC channel.)
#[derive(Debug, Clone, Copy)]
pub struct SpawnRequest {
    /// Binary path, e.g. b"/sbin/timezone_service\0..."
    pub path: [u8; 32],
    /// Short process name, e.g. b"timezone_service\0..."
    pub name: [u8; 16],
}

impl SpawnRequest {
    pub fn new(path: &str, name: &str) -> Self {
        let mut req = Self {
            path: [0; 32],
            name: [0; 16],
        };
        let pb = path.as_bytes();
        let plen = pb.len().min(31);
        req.path[..plen].copy_from_slice(&pb[..plen]);
        let nb = name.as_bytes();
        let nlen = nb.len().min(15);
        req.name[..nlen].copy_from_slice(&nb[..nlen]);
        req
    }

    /// Pack into an IpcMsg. Path occupies words[0..3]; name hint in words[4..5].
    pub fn pack_into(&self, msg: &mut IpcMsg) {
        msg.label = SpawnMsg::SPAWN;
        for i in 0..4 {
            let mut w = 0u64;
            for j in 0..8usize {
                w |= (self.path[i * 8 + j] as u64) << (j * 8);
            }
            msg.words[i] = w;
        }
        for i in 0..2 {
            let mut w = 0u64;
            for j in 0..8usize {
                w |= (self.name[i * 8 + j] as u64) << (j * 8);
            }
            msg.words[4 + i] = w;
        }
    }

    /// Unpack from an IpcMsg.
    pub fn unpack(msg: &IpcMsg) -> Self {
        let mut req = Self {
            path: [0; 32],
            name: [0; 16],
        };
        for i in 0..4 {
            let w = msg.words[i];
            for j in 0..8usize {
                req.path[i * 8 + j] = ((w >> (j * 8)) & 0xff) as u8;
            }
        }
        for i in 0..2 {
            let w = msg.words[4 + i];
            for j in 0..8usize {
                req.name[i * 8 + j] = ((w >> (j * 8)) & 0xff) as u8;
            }
        }
        req
    }

    pub fn path_str(&self) -> &str {
        let len = self.path.iter().position(|&b| b == 0).unwrap_or(32);
        core::str::from_utf8(&self.path[..len]).unwrap_or("")
    }

    pub fn name_str(&self) -> &str {
        let len = self.name.iter().position(|&b| b == 0).unwrap_or(16);
        core::str::from_utf8(&self.name[..len]).unwrap_or("")
    }
}

/// Pack a key event into a single u64 word for IPC transport.
/// Layout: keycode(u8) | pressed(u8) << 8 | mods_byte(u8) << 16 | ascii(u8) << 24
pub fn pack_key_event(
    keycode: u8,
    pressed: bool,
    shift: bool,
    ctrl: bool,
    alt: bool,
    ascii: Option<u8>,
) -> u64 {
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
    let ascii = if (val >> 24) & 0xFF != 0 {
        Some(((val >> 24) & 0xFF) as u8)
    } else {
        None
    };
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

    // SAFETY: fixed register IPC ABI. Only words[0..4) cross in registers:
    // r8/r9/r10/r12. r13/r14 carry capability tokens; the
    // generic syscall wrapper cannot do that because normal syscalls do not
    // pass cap registers.
    //
    // TRANSPORT LIMIT: IpcMsg has 8 logical words but this ABI only carries
    // words[0..4] (32 bytes). words[4..7] are silently dropped. Protocols
    // that pack strings into register words (e.g. KV keys at word 2) are
    // therefore limited to 16 transmitted bytes for the key. Extend via
    // shared memory (shm_alloc/shm_map) or add new SHM-key opcodes if
    // longer keys are needed.
    unsafe {
        core::arch::asm!(
            "syscall",
            inlateout("rax") num as u64 => ret,
            inlateout("rdi") msg.label => out_rdi,
            inlateout("rsi") object => out_rsi,
            inlateout("rdx") counts => out_rdx,
            inlateout("r8") msg.words[0] => out_r8,
            inlateout("r9") msg.words[1] => out_r9,
            inlateout("r10") msg.words[2] => out_r10,
            inlateout("r12") msg.words[3] => out_r12,
            inlateout("r13") msg.caps[0].0 => out_r13,
            inlateout("r14") msg.caps[1].0 => out_r14,
            lateout("rcx") _,
            lateout("r11") _,
        );
    }
    let mut reply = IpcMsg::with_label(out_rdi);
    reply.badge = out_rsi;
    reply.word_count = (out_rdx & 0xffff_ffff) as u32;
    reply.cap_count = (out_rdx >> 32) as u32;
    reply.words[0] = out_r8;
    reply.words[1] = out_r9;
    reply.words[2] = out_r10;
    reply.words[3] = out_r12;
    reply.caps[0] = CapabilityToken(out_r13);
    reply.caps[1] = CapabilityToken(out_r14);
    (ret, reply)
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
    let (ret, _) =
        unsafe { raw_syscall(SunlightSyscall::EndpointBind, endpoint, 0, 0, 0, 0, 0, 0) };
    CapabilityToken(ret)
}

pub fn get_init_cap() -> CapabilityToken {
    endpoint_bind(INIT_NAMESERVER_ENDPOINT)
}

/// Client: send a message and block until reply.
///
/// WARNING: This is a blocking call with no timeout. It will loop
/// (yield + retry) forever until a reply arrives or the target endpoint
/// is destroyed. Use only for trusted boot-time or server-to-server flows
/// where the peer is known to be alive and responsive.
///
/// For interactive shell paths or best-effort clients (e.g. calculator
/// history talking to sunlight-kv), use `ipc_call_timeout` instead.
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

/// Errors returned by `ipc_call_timeout`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpcCallError {
    /// The call did not complete within the provided deadline.
    Timeout,
    /// The capability token was invalid or lacked SEND rights.
    InvalidCapability,
    /// The nameserver (or target) could not resolve the endpoint.
    EndpointNotFound,
    /// Invalid argument (e.g. malformed message counts).
    InvalidArgument,
    /// Other/unknown error code from the kernel.
    Unknown(u64),
}

/// Client: send a message and wait for reply up to `timeout_ms`.
///
/// This is the safe primitive for interactive/client paths. It uses
/// `monotonic_millis()` to enforce a deadline and yields between
/// WouldBlock retries.
///
/// Semantics:
/// - The kernel IpcCall syscall returns WouldBlock (in rax) to userspace
///   rather than blocking inside the kernel (confirmed by inspection of
///   handle_ipc_call + syscall dispatch). Therefore a userspace deadline
///   loop is sufficient and correct.
/// - On success (ret==0) the assembled reply IpcMsg is returned.
/// - Known transport errors are mapped to `IpcCallError`.
/// - Server-side semantic errors (e.g. KV_ERROR label in reply) are **not**
///   turned into IpcCallError; the caller must inspect `reply.label`.
///
/// The original `msg` is re-submitted on each retry. The kernel's pending_call
/// state machine tolerates re-submission of the same call.
pub fn ipc_call_timeout(
    cap: CapabilityToken,
    msg: IpcMsg,
    timeout_ms: u64,
) -> Result<IpcMsg, IpcCallError> {
    let start = monotonic_millis();
    loop {
        let (ret, reply) = unsafe { raw_syscall_ipc(SunlightSyscall::IpcCall, cap.0, msg) };
        if !would_block(ret) {
            if ret == 0 {
                return Ok(reply);
            } else if ret == IpcError::InvalidCapability as u64 {
                return Err(IpcCallError::InvalidCapability);
            } else if ret == IpcError::EndpointNotFound as u64 {
                return Err(IpcCallError::EndpointNotFound);
            } else if ret == IpcError::InvalidArgument as u64 {
                return Err(IpcCallError::InvalidArgument);
            } else {
                // Also catch word/cap count validation errors etc.
                return Err(IpcCallError::Unknown(ret));
            }
        }
        let elapsed = monotonic_millis().saturating_sub(start);
        if elapsed >= timeout_ms {
            ipc_cancel_pending();
            // Log without allocation (this crate must remain usable by
            // early no_std binaries like sunlight-init that have no allocator).
            debug_log("[IPC] call timeout");
            return Err(IpcCallError::Timeout);
        }
        process_yield();
    }
}

/// Server: block waiting for first incoming call.
pub fn ipc_recv(ep: EndpointId) -> IpcMsg {
    loop {
        // SAFETY: ipc_recv passes the endpoint owner token; kernel validates receive rights.
        let (ret, msg) =
            unsafe { raw_syscall_ipc(SunlightSyscall::IpcRecv, ep.0, IpcMsg::empty()) };
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

fn ipc_cancel_pending() {
    // SAFETY: IpcCancel has no user pointers or additional arguments.
    unsafe {
        raw_syscall(SunlightSyscall::IpcCancel, 0, 0, 0, 0, 0, 0, 0);
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
        let (ret, _) =
            unsafe { raw_syscall(SunlightSyscall::IpcNotifyWait, ep.0, 0, 0, 0, 0, 0, 0) };
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

/// Nameserver lookup with a bounded timeout.
///
/// Returns `Some(cap)` only when the nameserver replies with `InitMsg::GRANT`.
/// Returns `None` on timeout, lookup failure (DENY or non-GRANT label),
/// IPC transport error, or any other failure.
///
/// This must be used by interactive shell code (e.g. calc history) so that
/// a broken or slow init/nameserver cannot freeze the shell.
pub fn nameserver_lookup_timeout(name: &str, timeout_ms: u64) -> Option<CapabilityToken> {
    let init_cap = get_init_cap();
    let msg = IpcMsg::with_label(InitMsg::LOOKUP).word(0, name_to_u64(name));
    match ipc_call_timeout(init_cap, msg, timeout_ms) {
        Ok(reply) if reply.label == InitMsg::GRANT => Some(CapabilityToken(reply.words[0])),
        _ => None,
    }
}

pub fn name_to_u64(name: &str) -> u64 {
    let bytes = name.as_bytes();
    let mut hash = 0xcbf29ce484222325u64;
    let mut i = 0;
    while i < bytes.len() {
        hash ^= bytes[i] as u64;
        hash = hash.wrapping_mul(0x100000001b3);
        i += 1;
    }
    if hash == 0 {
        1
    } else {
        hash
    }
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

/// Push keyboard bytes into tab `tab`'s kernel stdin ring so the foreground
/// app can read them via fd0. Returns the number of bytes accepted.
pub fn tty_stdin_push(tab: u32, bytes: &[u8]) -> usize {
    if bytes.is_empty() {
        return 0;
    }
    // SAFETY: passes a read-only pointer/length describing `bytes`; the kernel
    // copies it into the tab's stdin ring before returning.
    let (ret, _) = unsafe {
        raw_syscall(
            SunlightSyscall::TtyStdinPush,
            tab as u64,
            bytes.as_ptr() as u64,
            bytes.len() as u64,
            0,
            0,
            0,
            0,
        )
    };
    ret as usize
}

/// Drain tab `tab`'s kernel stdout ring into `buf`. Returns bytes pulled.
pub fn tty_stdout_pull(tab: u32, buf: &mut [u8]) -> usize {
    if buf.is_empty() {
        return 0;
    }
    // SAFETY: passes a writable pointer/length describing `buf`; the kernel
    // writes at most `buf.len()` bytes and returns the count.
    let (ret, _) = unsafe {
        raw_syscall(
            SunlightSyscall::TtyStdoutPull,
            tab as u64,
            buf.as_mut_ptr() as u64,
            buf.len() as u64,
            0,
            0,
            0,
            0,
        )
    };
    ret as usize
}

/// Non-reaping check whether `pid` is still alive (used to detect when a
/// foreground command exits).
pub fn process_is_alive(pid: u64) -> bool {
    // SAFETY: ProcessIsAlive takes no user pointers.
    let (ret, _) = unsafe { raw_syscall(SunlightSyscall::ProcessIsAlive, pid, 0, 0, 0, 0, 0, 0) };
    ret == 1
}

pub fn kill(pid: u64, sig: u32) -> bool {
    let (ret, _) = unsafe { raw_syscall(SunlightSyscall::Kill, pid, sig as u64, 0, 0, 0, 0, 0) };
    ret == 0
}

/// Set the nice value (-10..=10) for `pid` (0 = current process).
/// Wraps syscall 83 (SetNice), kernel clamps to NICE_MIN..=NICE_MAX.
pub fn set_nice(pid: u64, nice: i8) -> bool {
    // SAFETY: SetNice takes pid and nice value as plain integers, no pointers.
    let (ret, _) = unsafe {
        raw_syscall(
            SunlightSyscall::SetNice,
            pid,
            nice as i64 as u64,
            0,
            0,
            0,
            0,
            0,
        )
    };
    ret != u64::MAX
}

/// Get the current nice value for `pid` (0 = current process).
/// Wraps syscall 84 (GetNice). Returns 0 on failure (best-effort).
pub fn get_nice(pid: u64) -> i8 {
    // SAFETY: GetNice takes pid as plain integer, no pointers.
    let (ret, _) = unsafe { raw_syscall(SunlightSyscall::GetNice, pid, 0, 0, 0, 0, 0, 0) };
    ret as i64 as i8
}

pub fn get_time_utc() -> u64 {
    // SAFETY: GetTimeUtc takes no user pointers.
    let (ret, _) = unsafe { raw_syscall(SunlightSyscall::GetTimeUtc, 0, 0, 0, 0, 0, 0, 0) };
    ret
}

/// Milliseconds since boot (~10 ms resolution, PIT-derived). Suitable for
/// RTT/elapsed measurement where `get_time_utc`'s 1 s resolution is too coarse.
pub fn monotonic_millis() -> u64 {
    // SAFETY: MonotonicMs takes no user pointers.
    let (ret, _) = unsafe { raw_syscall(SunlightSyscall::MonotonicMs, 0, 0, 0, 0, 0, 0, 0) };
    ret
}

/// Phase 3.4: hand a raw Ethernet frame to the kernel-owned virtio-net
/// device for transmission. Returns `true` on success. Restricted to the
/// net_server process (pid 5) by the kernel.
pub fn net_tx(frame: &[u8]) -> bool {
    // SAFETY: passes a read-only pointer/length describing `frame` to the
    // kernel, which copies it into its own TX buffer before returning.
    let (ret, _) = unsafe {
        raw_syscall(
            SunlightSyscall::NetTx,
            frame.as_ptr() as u64,
            frame.len() as u64,
            0,
            0,
            0,
            0,
            0,
        )
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
        raw_syscall(
            SunlightSyscall::NetRx,
            buf.as_mut_ptr() as u64,
            buf.len() as u64,
            0,
            0,
            0,
            0,
            0,
        )
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
            raw_syscall(SunlightSyscall::ProcessExit, code as u64, 0, 0, 0, 0, 0, 0);
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

/// Map the kernel telemetry page into this process; returns null on failure.
pub fn map_telemetry() -> *const u8 {
    // SAFETY: MapTelemetry takes no pointers and returns a virtual address in rax.
    let (addr, _) = unsafe { raw_syscall(SunlightSyscall::MapTelemetry, 0, 0, 0, 0, 0, 0, 0) };
    addr as *const u8
}
