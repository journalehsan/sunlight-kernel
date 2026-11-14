//! Bounded native TCP transport bridge to the `net` userspace service.
//!
//! SunlightOS keeps the TCP/IP stack in `net_server` (smoltcp over the kernel
//! frame proxy). Native `socket()` still owns a kernel `inet_socket`
//! descriptor; this module adds the missing userspace transport attachment:
//! `bind`/`connect`/`listen`/`accept`/`send`/`recv`/`getsockname`/
//! `getpeername`/`close` are translated into the existing `NetOp` capability
//! IPC protocol. No parallel stack, pipe, file or host-side substitution is
//! used.
//!
//! Scope and limits:
//! * AF_INET/SOCK_STREAM only, matching the kernel's current `socket()`.
//! * `MSG_DONTWAIT` and descriptor `O_NONBLOCK` are honoured; blocking calls
//!   use bounded `NetOp::WAIT` deadlines rather than unbounded sleeps.
//! * `shutdown()` is explicitly unsupported (EOPNOTSUPP); peer closure is
//!   observed as `recv()` returning 0.
//! * Sockets are owned by the calling PID in `net_server`. `fork()` therefore
//!   does not transfer ownership; `dup()` does not copy the bridge mapping.
//! * `poll` and (through poll) `select` use `NetOp::WAIT` readiness for
//!   attached listeners and streams. Native epoll submits observations against
//!   kernel-owned registration tokens through syscall 177.
//!
//! No global lock is held across an IPC call. The fixed fd table lock only
//! protects the fd -> socket-id mapping itself.

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use sunlight_ipc::netop::{NetDiagnostic, NetOp, NetReady, NetStatus, NetWait};
use sunlight_ipc::{
    ipc_call_timeout, nameserver_lookup_timeout, shm_alloc, shm_free, CapabilityToken,
    IpcCallError, IpcMsg,
};

use crate::{errno, sys};

const MAX_BRIDGED_FDS: usize = 256;
const SHM_PAGE: usize = 4096;
const NET_LOOKUP_TIMEOUT_MS: u64 = 2_000;
const NET_CALL_TIMEOUT_MS: u64 = 5_000;
const CONNECT_TIMEOUT_MS: u64 = 10_000;
const ACCEPT_TIMEOUT_MS: u64 = 10_000;
const IO_TIMEOUT_MS: u64 = 10_000;

const AF_INET: u16 = 2;
const SOCK_STREAM: u64 = 1;
const MSG_DONTWAIT: i32 = 0x40;
const MSG_OOB: i32 = 0x01;
const O_NONBLOCK: u32 = 0x800;
const F_GETFL: u64 = 3;
const SYS_FCNTL: u64 = 49;
const SOCKADDR_IN_LEN: u32 = 16;

/// fd -> (net_server socket id + 1); zero means "not bridged".
static IDS: [AtomicU64; MAX_BRIDGED_FDS] = [const { AtomicU64::new(0) }; MAX_BRIDGED_FDS];
/// Kernel inet-endpoint generation recorded when the mapping was installed.
/// Zero means "not a live inet endpoint at install time".
static GENERATIONS: [AtomicU64; MAX_BRIDGED_FDS] = [const { AtomicU64::new(0) }; MAX_BRIDGED_FDS];
static TABLE_LOCK: AtomicBool = AtomicBool::new(false);
static NET_CAP: AtomicU64 = AtomicU64::new(0);

fn lock_table() {
    while TABLE_LOCK
        .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
    }
}

fn unlock_table() {
    TABLE_LOCK.store(false, Ordering::Release);
}

fn slot(fd: i32) -> Option<usize> {
    if (0..MAX_BRIDGED_FDS as i32).contains(&fd) {
        Some(fd as usize)
    } else {
        None
    }
}

/// Current kernel endpoint generation for `fd`. Zero means the fd is not an
/// inet endpoint; a negative value means the fd does not exist.
///
/// This is a non-blocking identity query, never a NetOp IPC. It is performed
/// before taking the table lock.
fn endpoint_generation(fd: i32) -> i64 {
    // SAFETY: generation query (mode 3) takes only the fd and writes no user
    // memory.
    unsafe { sys::syscall6(sys::SYS_EPOLL_BRIDGE, fd as u64, 0, 0, 0, 0, 3) as i64 }
}

fn recorded(fd: i32) -> Option<(u64, u64)> {
    let index = slot(fd)?;
    lock_table();
    let stored = IDS[index].load(Ordering::Acquire);
    let generation = GENERATIONS[index].load(Ordering::Acquire);
    unlock_table();
    (stored != 0 && generation != 0).then_some((stored - 1, generation))
}

/// Endpoint identity captured at the start of an attach operation. Capturing
/// before the `NetOp` means a replacement racing the operation leaves a stale
/// record that [`lookup`] rejects, instead of a record that matches the new
/// endpoint.
fn endpoint_identity(fd: i32) -> u64 {
    endpoint_generation(fd).max(0) as u64
}

/// Return the attached net_server socket id for `fd`, if any.
///
/// A mapping is valid only while the descriptor still names the kernel inet
/// endpoint it was recorded against. `dup2`/`dup3` replacement, or a close
/// that bypassed this crate, rebinds the fd number. The stale attachment must
/// then be unusable: returning it would route transport I/O and readiness to
/// the replaced endpoint. `close` uses [`detach`] instead so the displaced
/// endpoint is still released.
pub fn lookup(fd: i32) -> Option<u64> {
    let (socket_id, generation) = recorded(fd)?;
    let current = endpoint_generation(fd);
    (current > 0 && current as u64 == generation).then_some(socket_id)
}

fn install(fd: i32, socket_id: u64, generation: u64) {
    let Some(index) = slot(fd) else { return };
    lock_table();
    IDS[index].store(socket_id + 1, Ordering::Release);
    GENERATIONS[index].store(generation, Ordering::Release);
    unlock_table();
}

/// Remove and return the attachment and endpoint generation recorded for `fd`.
pub fn detach(fd: i32) -> Option<(u64, u64)> {
    let index = slot(fd)?;
    lock_table();
    let stored = IDS[index].load(Ordering::Acquire);
    let generation = GENERATIONS[index].load(Ordering::Acquire);
    IDS[index].store(0, Ordering::Release);
    GENERATIONS[index].store(0, Ordering::Release);
    unlock_table();
    (stored != 0).then_some((stored - 1, generation))
}

/// Release an attachment removed by [`detach`].
pub fn release_detached(socket_id: u64) {
    if let Some(cap) = net_cap() {
        close_net_socket(cap, socket_id);
    }
}

/// Restore an attachment removed by [`detach`] after a rejected replacement.
/// The original generation is restored, so the record only becomes valid
/// again if the descriptor still names the same endpoint.
pub fn reattach(fd: i32, socket_id: u64, generation: u64) {
    install(fd, socket_id, generation);
}

/// Drop any bridge mapping for `fd` without contacting net_server.
pub fn forget(fd: i32) {
    let Some(index) = slot(fd) else { return };
    lock_table();
    IDS[index].store(0, Ordering::Release);
    GENERATIONS[index].store(0, Ordering::Release);
    unlock_table();
}

fn fail(code: i32) -> i32 {
    errno::set_errno(code);
    -1
}

fn fail_isize(code: i32) -> isize {
    errno::set_errno(code);
    -1
}

fn net_cap() -> Option<CapabilityToken> {
    let cached = NET_CAP.load(Ordering::Acquire);
    if cached != 0 {
        return Some(CapabilityToken(cached));
    }
    let cap = nameserver_lookup_timeout("net", NET_LOOKUP_TIMEOUT_MS)?;
    NET_CAP.store(cap.0, Ordering::Release);
    Some(cap)
}

fn pack_ipv4(ip: [u8; 4]) -> u64 {
    (ip[0] as u64) | ((ip[1] as u64) << 8) | ((ip[2] as u64) << 16) | ((ip[3] as u64) << 24)
}

fn unpack_ipv4(word: u64) -> [u8; 4] {
    [
        (word & 0xff) as u8,
        ((word >> 8) & 0xff) as u8,
        ((word >> 16) & 0xff) as u8,
        ((word >> 24) & 0xff) as u8,
    ]
}

fn status_errno(status: u64) -> i32 {
    match status {
        NetStatus::WOULD_BLOCK => errno::EAGAIN,
        NetStatus::RESET => errno::ECONNRESET,
        NetStatus::CLOSED => errno::EPIPE,
        NetStatus::TIMEOUT => errno::ETIMEDOUT,
        NetStatus::INVALID_SOCKET => errno::EBADF,
        NetStatus::ACCESS_DENIED => errno::EACCES,
        NetStatus::ADDRESS_IN_USE => errno::EADDRINUSE,
        NetStatus::INVALID_STATE => errno::EINVAL,
        NetStatus::NOT_CONNECTED => errno::ECONNREFUSED,
        NetStatus::BACKLOG_FULL | NetStatus::NO_SLOTS => errno::ENOMEM,
        _ => errno::EIO,
    }
}

fn call_error_errno(error: IpcCallError) -> i32 {
    match error {
        IpcCallError::Timeout => errno::ETIMEDOUT,
        IpcCallError::QueueFull => errno::EAGAIN,
        IpcCallError::EndpointNotFound | IpcCallError::InvalidCapability => errno::ENOTCONN,
        _ => errno::EIO,
    }
}

fn alloc_net_socket(cap: CapabilityToken) -> Result<u64, i32> {
    let reply = ipc_call_timeout(
        cap,
        IpcMsg::with_label(NetOp::SOCKET),
        NET_CALL_TIMEOUT_MS,
    )
    .map_err(call_error_errno)?;
    if reply.label != NetOp::SOCKET || reply.words[0] == 0 {
        return Err(status_errno(reply.words[1]));
    }
    Ok(reply.words[0])
}

fn close_net_socket(cap: CapabilityToken, socket_id: u64) {
    let _ = ipc_call_timeout(
        cap,
        IpcMsg::with_label(NetOp::CLOSE).word(0, socket_id),
        NET_CALL_TIMEOUT_MS,
    );
}

/// Wait for readiness bits. Returns the ready mask or an errno.
fn wait_ready(
    cap: CapabilityToken,
    socket_id: u64,
    interest: u64,
    timeout_ms: u64,
) -> Result<u64, i32> {
    let (ptr, token) = shm_alloc().map_err(|_| errno::ENOMEM)?;
    // SAFETY: shm_alloc returns a writable 4 KiB page.
    unsafe {
        *(ptr as *mut u64) = socket_id;
    }
    let reply = ipc_call_timeout(
        cap,
        IpcMsg::with_label(NetOp::WAIT)
            .word(0, 1)
            .word(1, timeout_ms)
            .word(2, interest)
            .with_cap(0, token),
        timeout_ms.saturating_add(100),
    );
    let _ = shm_free(token);
    let reply = reply.map_err(call_error_errno)?;
    if reply.label != NetOp::WAIT {
        return Err(errno::EIO);
    }
    match reply.words[0] {
        NetStatus::OK => {
            if reply.words[1] != socket_id {
                return Err(errno::EIO);
            }
            Ok(reply.words[2])
        }
        NetStatus::WOULD_BLOCK => Err(errno::EAGAIN),
        NetStatus::TIMEOUT => Err(errno::ETIMEDOUT),
        NetStatus::NOT_CONNECTED => Err(errno::ECONNREFUSED),
        other => Err(status_errno(other)),
    }
}

/// Result of a bounded `NetOp::WAIT`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaitOutcome {
    /// One requested identity became ready; carries the identity and bits.
    Ready(u64, u64),
    /// The wait deadline elapsed or a zero-timeout snapshot found nothing.
    Empty,
    /// Every requested identity no longer resolves (reaped/closed).
    Gone,
}

/// Wait on up to [`NetWait::MAX_IDENTITIES`] sockets with one interest mask
/// per identity. This is the readiness primitive used by native `poll`; the
/// caller keeps the descriptor table authoritative and does not hold any lock
/// across the IPC call.
pub fn wait_any_per_interest(
    socket_ids: &[u64],
    interests: &[u32],
    timeout_ms: u64,
) -> Result<WaitOutcome, i32> {
    if socket_ids.is_empty()
        || socket_ids.len() != interests.len()
        || socket_ids.len() > NetWait::MAX_IDENTITIES
    {
        return Err(errno::EINVAL);
    }
    let Some(cap) = net_cap() else {
        return Err(errno::ENOTCONN);
    };
    let (ptr, token) = shm_alloc().map_err(|_| errno::ENOMEM)?;
    unsafe {
        let identities = ptr as *mut u64;
        for (index, socket_id) in socket_ids.iter().enumerate() {
            identities.add(index).write(*socket_id);
        }
        let masks = ptr.add(NetWait::INTEREST_OFFSET) as *mut u32;
        for (index, interest) in interests.iter().enumerate() {
            masks.add(index).write(*interest);
        }
    }
    let reply = ipc_call_timeout(
        cap,
        IpcMsg::with_label(NetOp::WAIT)
            .word(0, socket_ids.len() as u64)
            .word(1, timeout_ms)
            .word(3, NetWait::PER_IDENTITY_INTEREST)
            .with_cap(0, token),
        timeout_ms.saturating_add(100),
    );
    let _ = shm_free(token);
    let reply = match reply {
        Ok(reply) => reply,
        // The IPC deadline is 100 ms beyond the requested wait. If the server
        // reply is late, it is still the caller's poll deadline, not a poll
        // failure. A following snapshot revalidates readiness.
        Err(IpcCallError::Timeout) => return Ok(WaitOutcome::Empty),
        Err(error) => return Err(call_error_errno(error)),
    };
    if reply.label != NetOp::WAIT {
        return Err(errno::EIO);
    }
    match reply.words[0] {
        NetStatus::OK => Ok(WaitOutcome::Ready(reply.words[1], reply.words[2])),
        NetStatus::WOULD_BLOCK | NetStatus::TIMEOUT => Ok(WaitOutcome::Empty),
        NetStatus::NOT_CONNECTED | NetStatus::INVALID_SOCKET => Ok(WaitOutcome::Gone),
        other => Err(status_errno(other)),
    }
}

fn fd_nonblocking(fd: i32) -> bool {
    // SAFETY: F_GETFL takes no user pointer.
    let flags = unsafe { sys::syscall3(SYS_FCNTL, fd as u64, F_GETFL, 0) } as i64;
    flags >= 0 && (flags as u32 & O_NONBLOCK) != 0
}

/// Parse a `sockaddr_in`. Returns (ip, port) or an errno.
unsafe fn parse_sockaddr(addr: *const u8, len: u32) -> Result<([u8; 4], u16), i32> {
    if addr.is_null() {
        return Err(errno::EFAULT);
    }
    if len < SOCKADDR_IN_LEN {
        return Err(errno::EINVAL);
    }
    // SAFETY: caller supplied `len` readable bytes.
    let bytes = core::slice::from_raw_parts(addr, SOCKADDR_IN_LEN as usize);
    let family = u16::from_ne_bytes([bytes[0], bytes[1]]);
    if family != AF_INET {
        return Err(errno::EAFNOSUPPORT);
    }
    let port = u16::from_be_bytes([bytes[2], bytes[3]]);
    Ok(([bytes[4], bytes[5], bytes[6], bytes[7]], port))
}

unsafe fn write_sockaddr(
    addr: *mut u8,
    len_ptr: *mut u32,
    ip: [u8; 4],
    port: u16,
) -> Result<(), i32> {
    if addr.is_null() || len_ptr.is_null() {
        return Err(errno::EFAULT);
    }
    // SAFETY: caller supplied a readable socklen_t.
    let capacity = *len_ptr;
    if capacity < SOCKADDR_IN_LEN {
        return Err(errno::EINVAL);
    }
    // SAFETY: caller supplied a writable 16-byte sockaddr.
    let out = core::slice::from_raw_parts_mut(addr, SOCKADDR_IN_LEN as usize);
    out[0..2].copy_from_slice(&AF_INET.to_ne_bytes());
    out[2..4].copy_from_slice(&port.to_be_bytes());
    out[4..8].copy_from_slice(&ip);
    out[8..16].fill(0);
    *len_ptr = SOCKADDR_IN_LEN;
    Ok(())
}

/// `connect(2)` for AF_INET/SOCK_STREAM.
///
/// # Safety
/// `addr` must designate `len` readable bytes.
pub unsafe extern "C" fn connect(fd: i32, addr: *const u8, len: u32) -> i32 {
    if lookup(fd).is_some() {
        return fail(errno::EISCONN);
    }
    // Asynchronous connect (EINPROGRESS + SO_ERROR completion) is outside this
    // bounded subset. Reject the nonblocking request explicitly instead of
    // silently blocking for the full deadline.
    if fd_nonblocking(fd) {
        return fail(errno::EOPNOTSUPP);
    }
    // Capture the endpoint before the first NetOp so a concurrent
    // replacement cannot make this attachment look current.
    let generation = endpoint_identity(fd);
    let (ip, port) = match parse_sockaddr(addr, len) {
        Ok(value) => value,
        Err(error) => return fail(error),
    };
    if port == 0 {
        return fail(errno::EINVAL);
    }
    let Some(cap) = net_cap() else {
        return fail(errno::ENOTCONN);
    };
    let socket_id = match alloc_net_socket(cap) {
        Ok(id) => id,
        Err(error) => return fail(error),
    };
    let reply = match ipc_call_timeout(
        cap,
        IpcMsg::with_label(NetOp::CONNECT)
            .word(0, socket_id)
            .word(1, pack_ipv4(ip))
            .word(2, port as u64),
        NET_CALL_TIMEOUT_MS,
    ) {
        Ok(reply) => reply,
        Err(error) => {
            close_net_socket(cap, socket_id);
            return fail(call_error_errno(error));
        }
    };
    if reply.label != NetOp::CONNECT || reply.words[1] != NetStatus::OK {
        close_net_socket(cap, socket_id);
        return fail(status_errno(reply.words[1]));
    }
    match wait_ready(cap, socket_id, NetReady::WRITE, CONNECT_TIMEOUT_MS) {
        Ok(ready) if ready & NetReady::WRITE != 0 => {
            install(fd, socket_id, generation);
            0
        }
        Ok(ready) => {
            close_net_socket(cap, socket_id);
            if ready & NetReady::RESET != 0 {
                fail(errno::ECONNRESET)
            } else {
                fail(errno::ECONNREFUSED)
            }
        }
        Err(error) => {
            close_net_socket(cap, socket_id);
            fail(error)
        }
    }
}

/// `bind(2)` for AF_INET/SOCK_STREAM.
///
/// # Safety
/// `addr` must designate `len` readable bytes.
pub unsafe extern "C" fn bind(fd: i32, addr: *const u8, len: u32) -> i32 {
    if lookup(fd).is_some() {
        return fail(errno::EINVAL);
    }
    let generation = endpoint_identity(fd);
    let (ip, port) = match parse_sockaddr(addr, len) {
        Ok(value) => value,
        Err(error) => return fail(error),
    };
    let Some(cap) = net_cap() else {
        return fail(errno::ENOTCONN);
    };
    let socket_id = match alloc_net_socket(cap) {
        Ok(id) => id,
        Err(error) => return fail(error),
    };
    let reply = match ipc_call_timeout(
        cap,
        IpcMsg::with_label(NetOp::BIND)
            .word(0, socket_id)
            .word(1, pack_ipv4(ip))
            .word(2, port as u64),
        NET_CALL_TIMEOUT_MS,
    ) {
        Ok(reply) => reply,
        Err(error) => {
            close_net_socket(cap, socket_id);
            return fail(call_error_errno(error));
        }
    };
    if reply.label != NetOp::BIND || reply.words[1] != NetStatus::OK {
        close_net_socket(cap, socket_id);
        return fail(status_errno(reply.words[1]));
    }
    install(fd, socket_id, generation);
    0
}

/// `listen(2)` for a bridged AF_INET/SOCK_STREAM socket.
pub extern "C" fn listen(fd: i32, backlog: i32) -> i32 {
    let Some(socket_id) = lookup(fd) else {
        return fail(errno::EINVAL);
    };
    if backlog <= 0 {
        return fail(errno::EINVAL);
    }
    let Some(cap) = net_cap() else {
        return fail(errno::ENOTCONN);
    };
    let reply = match ipc_call_timeout(
        cap,
        IpcMsg::with_label(NetOp::LISTEN)
            .word(0, socket_id)
            .word(1, backlog as u64),
        NET_CALL_TIMEOUT_MS,
    ) {
        Ok(reply) => reply,
        Err(error) => return fail(call_error_errno(error)),
    };
    if reply.label != NetOp::LISTEN || reply.words[1] != NetStatus::OK {
        return fail(status_errno(reply.words[1]));
    }
    0
}

/// `accept(2)` for a bridged listener. Returns a new native descriptor.
///
/// # Safety
/// `addr`/`len_ptr` are optional; when present they must be writable.
pub unsafe extern "C" fn accept(fd: i32, addr: *mut u8, len_ptr: *mut u32) -> i32 {
    let Some(socket_id) = lookup(fd) else {
        return fail(errno::EINVAL);
    };
    // Nonblocking accept (EAGAIN when the backlog is empty) is outside this
    // bounded subset and is rejected explicitly.
    if fd_nonblocking(fd) {
        return fail(errno::EOPNOTSUPP);
    }
    let Some(cap) = net_cap() else {
        return fail(errno::ENOTCONN);
    };
    match wait_ready(cap, socket_id, NetReady::ACCEPT, ACCEPT_TIMEOUT_MS) {
        Ok(ready) if ready & NetReady::ACCEPT != 0 => {}
        Ok(_) => return fail(errno::ECONNABORTED),
        Err(error) => return fail(error),
    }
    let reply = match ipc_call_timeout(
        cap,
        IpcMsg::with_label(NetOp::ACCEPT).word(0, socket_id),
        NET_CALL_TIMEOUT_MS,
    ) {
        Ok(reply) => reply,
        Err(error) => return fail(call_error_errno(error)),
    };
    if reply.label != NetOp::ACCEPT || reply.words[0] == 0 {
        return fail(status_errno(reply.words[1]));
    }
    let new_id = reply.words[0];
    // The accepted endpoint still needs a native descriptor for close()/dup()
    // ownership. SYS_SOCKET is the existing creation path.
    let raw = unsafe {
        sys::syscall3(
            sys::SYS_SOCKET,
            AF_INET as u64,
            SOCK_STREAM,
            0,
        )
    } as i64;
    if raw < 0 {
        close_net_socket(cap, new_id);
        return fail((-raw) as i32);
    }
    let new_fd = raw as i32;
    install(new_fd, new_id, endpoint_identity(new_fd));
    if !addr.is_null() && !len_ptr.is_null() {
        if getpeername(new_fd, addr, len_ptr) != 0 {
            // Preserve accept success; the peer address is advisory.
            errno::set_errno(0);
        }
    }
    new_fd
}

/// `getsockname(2)` for a bridged socket.
///
/// # Safety
/// `addr` must designate at least 16 writable bytes and `len_ptr` a writable
/// `socklen_t`.
pub unsafe extern "C" fn getsockname(fd: i32, addr: *mut u8, len_ptr: *mut u32) -> i32 {
    endpoint(NetOp::GET_SOCKNAME, fd, addr, len_ptr)
}

/// `getpeername(2)` for a bridged socket.
///
/// # Safety
/// See `getsockname`.
pub unsafe extern "C" fn getpeername(fd: i32, addr: *mut u8, len_ptr: *mut u32) -> i32 {
    endpoint(NetOp::GET_PEERNAME, fd, addr, len_ptr)
}

unsafe fn endpoint(op: u64, fd: i32, addr: *mut u8, len_ptr: *mut u32) -> i32 {
    let Some(socket_id) = lookup(fd) else {
        return fail(errno::EINVAL);
    };
    let Some(cap) = net_cap() else {
        return fail(errno::ENOTCONN);
    };
    let reply = match ipc_call_timeout(
        cap,
        IpcMsg::with_label(op).word(0, socket_id),
        NET_CALL_TIMEOUT_MS,
    ) {
        Ok(reply) => reply,
        Err(error) => return fail(call_error_errno(error)),
    };
    if reply.label != op || reply.words[2] != NetStatus::OK {
        return fail(status_errno(reply.words[2]));
    }
    match write_sockaddr(addr, len_ptr, unpack_ipv4(reply.words[0]), reply.words[1] as u16) {
        Ok(()) => 0,
        Err(error) => fail(error),
    }
}

/// `send(2)`/`write(2)` over a bridged socket.
///
/// # Safety
/// `buf` must designate `len` readable bytes when `len != 0`.
pub unsafe extern "C" fn send(fd: i32, buf: *const u8, len: usize, flags: i32) -> isize {
    if len != 0 && buf.is_null() {
        return fail_isize(errno::EFAULT);
    }
    if flags & !(MSG_DONTWAIT) != 0 {
        return fail_isize(errno::EINVAL);
    }
    let Some(socket_id) = lookup(fd) else {
        return fail_isize(errno::ENOTCONN);
    };
    if len == 0 {
        return 0;
    }
    let nonblock = flags & MSG_DONTWAIT != 0 || fd_nonblocking(fd);
    let Some(cap) = net_cap() else {
        return fail_isize(errno::ENOTCONN);
    };
    let data = core::slice::from_raw_parts(buf, len);
    let mut offset = 0usize;
    while offset < data.len() {
        let chunk_len = (data.len() - offset).min(SHM_PAGE);
        let (ptr, token) = match shm_alloc() {
            Ok(value) => value,
            Err(_) => return if offset > 0 { offset as isize } else { fail_isize(errno::ENOMEM) },
        };
        // SAFETY: shm_alloc returns a writable 4 KiB page.
        core::ptr::copy_nonoverlapping(data[offset..].as_ptr(), ptr, chunk_len);
        let reply = ipc_call_timeout(
            cap,
            IpcMsg::with_label(NetOp::SEND_SHM)
                .word(0, socket_id)
                .word(1, chunk_len as u64)
                .with_cap(0, token),
            NET_CALL_TIMEOUT_MS,
        );
        let _ = shm_free(token);
        let reply = match reply {
            Ok(reply) => reply,
            Err(error) => {
                return if offset > 0 {
                    offset as isize
                } else {
                    fail_isize(call_error_errno(error))
                }
            }
        };
        if reply.label != NetOp::SEND_SHM {
            return if offset > 0 { offset as isize } else { fail_isize(errno::EIO) };
        }
        let status = reply.words[1];
        if status == NetStatus::WOULD_BLOCK {
            if nonblock {
                return if offset > 0 { offset as isize } else { fail_isize(errno::EAGAIN) };
            }
            match wait_ready(cap, socket_id, NetReady::WRITE, IO_TIMEOUT_MS) {
                Ok(ready) if ready & NetReady::WRITE != 0 => continue,
                Ok(_) => {
                    return if offset > 0 { offset as isize } else { fail_isize(errno::EPIPE) }
                }
                Err(error) => {
                    return if offset > 0 { offset as isize } else { fail_isize(error) }
                }
            }
        }
        if status != NetStatus::OK {
            return if offset > 0 { offset as isize } else { fail_isize(status_errno(status)) };
        }
        let sent = reply.words[0] as usize;
        if sent == 0 {
            return if offset > 0 { offset as isize } else { fail_isize(errno::EPIPE) };
        }
        offset = offset.saturating_add(sent.min(chunk_len));
    }
    offset as isize
}

/// `recv(2)`/`read(2)` over a bridged socket.
///
/// # Safety
/// `buf` must designate `len` writable bytes when `len != 0`.
pub unsafe extern "C" fn recv(fd: i32, buf: *mut u8, len: usize, flags: i32) -> isize {
    if len != 0 && buf.is_null() {
        return fail_isize(errno::EFAULT);
    }
    if flags & !(MSG_DONTWAIT | MSG_OOB) != 0 {
        return fail_isize(errno::EINVAL);
    }
    if flags & MSG_OOB != 0 {
        return fail_isize(errno::EOPNOTSUPP);
    }
    let Some(socket_id) = lookup(fd) else {
        return fail_isize(errno::ENOTCONN);
    };
    if len == 0 {
        return 0;
    }
    let nonblock = flags & MSG_DONTWAIT != 0 || fd_nonblocking(fd);
    let Some(cap) = net_cap() else {
        return fail_isize(errno::ENOTCONN);
    };
    let want = len.min(SHM_PAGE);
    loop {
        let (ptr, token) = match shm_alloc() {
            Ok(value) => value,
            Err(_) => return fail_isize(errno::ENOMEM),
        };
        let reply = ipc_call_timeout(
            cap,
            IpcMsg::with_label(NetOp::RECV_SHM)
                .word(0, socket_id)
                .word(1, want as u64)
                .with_cap(0, token),
            NET_CALL_TIMEOUT_MS,
        );
        let reply = match reply {
            Ok(reply) => reply,
            Err(error) => {
                let _ = shm_free(token);
                return fail_isize(call_error_errno(error));
            }
        };
        if reply.label != NetOp::RECV_SHM {
            let _ = shm_free(token);
            return fail_isize(errno::EIO);
        }
        let status = reply.words[1];
        let count = (reply.words[0] as usize).min(want);
        if status == NetStatus::WOULD_BLOCK {
            let _ = shm_free(token);
            if nonblock {
                return fail_isize(errno::EAGAIN);
            }
            match wait_ready(cap, socket_id, NetReady::READ, IO_TIMEOUT_MS) {
                Ok(ready) if ready & (NetReady::READ | NetReady::EOF) != 0 => continue,
                Ok(_) => return fail_isize(errno::ECONNRESET),
                Err(error) => return fail_isize(error),
            }
        }
        if status != NetStatus::OK && status != NetStatus::EOF {
            let _ = shm_free(token);
            return fail_isize(status_errno(status));
        }
        if count == 0 {
            let _ = shm_free(token);
            return 0;
        }
        // SAFETY: net_server copied `count` bytes into the shared page.
        core::ptr::copy_nonoverlapping(ptr, buf, count);
        let _ = shm_free(token);
        return count as isize;
    }
}

/// `sendto(2)` for a connected TCP socket. A destination is rejected.
///
/// # Safety
/// See `send`.
pub unsafe extern "C" fn sendto(
    fd: i32,
    buf: *const u8,
    len: usize,
    flags: i32,
    addr: *const u8,
    addr_len: u32,
) -> isize {
    if !addr.is_null() && addr_len != 0 {
        return fail_isize(errno::EISCONN);
    }
    send(fd, buf, len, flags)
}

/// `recvfrom(2)` for a connected TCP socket; fills the peer address.
///
/// # Safety
/// See `recv` and `getpeername`.
pub unsafe extern "C" fn recvfrom(
    fd: i32,
    buf: *mut u8,
    len: usize,
    flags: i32,
    addr: *mut u8,
    addr_len: *mut u32,
) -> isize {
    let received = recv(fd, buf, len, flags);
    if received >= 0 && !addr.is_null() && !addr_len.is_null() {
        let _ = getpeername(fd, addr, addr_len);
    }
    received
}

/// `shutdown(2)` is not part of the bounded transport subset.
pub extern "C" fn shutdown(_fd: i32, _how: i32) -> i32 {
    fail(errno::EOPNOTSUPP)
}

/// Release the net_server attachment for `fd`, if any.
///
/// Called by `close()` before the kernel descriptor is consumed. Safe to call
/// for non-bridged descriptors.
pub fn close(fd: i32) {
    // Descriptor destruction must release the attachment that was installed,
    // even if the fd number was rebound (for example by a raw `dup2`) and the
    // current identity no longer matches. Ordinary operations use `lookup`.
    let Some((socket_id, _)) = detach(fd) else {
        return;
    };
    if let Some(cap) = net_cap() {
        close_net_socket(cap, socket_id);
    }
}

/// Read one `net_server` TCP diagnostic counter.
pub fn diagnostic(selector: u64) -> Option<u64> {
    let cap = net_cap()?;
    let reply = ipc_call_timeout(
        cap,
        IpcMsg::with_label(NetOp::GET_DIAGNOSTIC).word(0, selector),
        NET_CALL_TIMEOUT_MS,
    )
    .ok()?;
    (reply.label == NetOp::GET_DIAGNOSTIC).then_some(reply.words[0])
}

/// Non-standard diagnostic selector used by the native networking fixtures.
pub const DIAGNOSTIC_SOCKET_LIVE: u64 = NetDiagnostic::SOCKET_LIVE;
pub const DIAGNOSTIC_LISTENER_LIVE: u64 = NetDiagnostic::LISTENER_LIVE;
pub const DIAGNOSTIC_STREAM_LIVE: u64 = NetDiagnostic::STREAM_LIVE;

#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub extern "C" fn htons(value: u16) -> u16 {
    value.to_be()
}

#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub extern "C" fn ntohs(value: u16) -> u16 {
    u16::from_be(value)
}

#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub extern "C" fn htonl(value: u32) -> u32 {
    value.to_be()
}

#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub extern "C" fn ntohl(value: u32) -> u32 {
    u32::from_be(value)
}
