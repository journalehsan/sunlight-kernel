//! AF_UNIX `SOCK_STREAM` plumbing for the Helios Linux compatibility layer.
//!
//! A `socket(AF_UNIX, SOCK_STREAM, 0)` creates an unbound endpoint. `bind`
//! attaches an internal name and `listen` publishes it so a later
//! `connect` (possibly from another process) can find it. Once connected, both
//! ends are represented by the existing pipe-backed socketpair rings, so
//! ordinary read/write/poll/epoll handling applies unchanged.
//!
//! The implementation is intentionally generic: no application-specific
//! behavior is encoded here, and unsupported domains/types fail closed.

use alloc::collections::VecDeque;
use alloc::vec::Vec;
use spin::Mutex;

use sunlight_compat_linux::abi;

/// Advisory cap on queued, unaccepted connections for one listener.
pub const DEFAULT_BACKLOG: usize = 128;
pub const NATIVE_PREFIX: &[u8] = b"/sunlight/native-unix/";

struct UnixSocket {
    /// Path attached by `bind`, if any (unnamed sockets have no path).
    path: Option<Vec<u8>>,
    /// True after `listen` published the path for connection lookup.
    listening: bool,
    allowed_uid: Option<u32>,
    /// Pending connections as socketpair indices owned by this listener.
    pending: VecDeque<u32>,
    /// Open descriptors (plus queued endpoints) referencing this entry.
    refs: usize,
}

static SOCKETS: Mutex<Vec<Option<UnixSocket>>> = Mutex::new(Vec::new());

/// Allocate an unbound AF_UNIX stream endpoint. Returns its pool index.
pub fn create() -> u32 {
    let mut pool = SOCKETS.lock();
    insert(&mut pool, UnixSocket {
        path: None,
        listening: false,
        allowed_uid: None,
        pending: VecDeque::new(),
        refs: 1,
    })
}

fn insert(pool: &mut Vec<Option<UnixSocket>>, socket: UnixSocket) -> u32 {
    if let Some((idx, slot)) = pool.iter_mut().enumerate().find(|(_, s)| s.is_none()) {
        *slot = Some(socket);
        idx as u32
    } else {
        let idx = pool.len() as u32;
        pool.push(Some(socket));
        idx
    }
}

pub fn register_native(path: Vec<u8>, uid: u32) -> Result<u32, u32> {
    if !path.starts_with(NATIVE_PREFIX) || path.len() == NATIVE_PREFIX.len()
        || path.contains(&0)
    {
        return Err(abi::EINVAL);
    }
    let mut pool = SOCKETS.lock();
    if pool.iter().flatten().any(|socket| socket.path.as_deref() == Some(&path)) {
        return Err(abi::EADDRINUSE);
    }
    Ok(insert(&mut pool, UnixSocket {
        path: Some(path),
        listening: true,
        allowed_uid: Some(uid),
        pending: VecDeque::new(),
        refs: 1,
    }))
}

pub fn is_native_listener(idx: u32) -> bool {
    SOCKETS.lock().get(idx as usize).and_then(Option::as_ref)
        .is_some_and(|socket| socket.allowed_uid.is_some())
}

pub fn retain(idx: u32) {
    let mut pool = SOCKETS.lock();
    if let Some(Some(socket)) = pool.get_mut(idx as usize) {
        socket.refs += 1;
    }
}

/// Drop one descriptor reference. The final reference drains any queued but
/// unaccepted connections and frees the slot.
pub fn release(idx: u32) {
    let mut drained = Vec::new();
    {
        let mut pool = SOCKETS.lock();
        let Some(Some(socket)) = pool.get_mut(idx as usize) else {
            return;
        };
        socket.refs = socket.refs.saturating_sub(1);
        if socket.refs == 0 {
            drained.extend(socket.pending.drain(..));
            socket.listening = false;
            socket.path = None;
            pool[idx as usize] = None;
        }
    }
    for pair in drained {
        super::pipe::socket_close(pair, 1);
    }
}

pub fn is_live(idx: u32) -> bool {
    SOCKETS
        .lock()
        .get(idx as usize)
        .is_some_and(Option::is_some)
}

/// `bind(fd, sockaddr_un)`: attach `path`. Duplicate bound/listening paths fail
/// with `EADDRINUSE`, matching Linux.
pub fn bind(idx: u32, path: Vec<u8>) -> Result<(), u32> {
    if path.is_empty() {
        return Err(abi::EINVAL);
    }
    if path.starts_with(NATIVE_PREFIX) {
        return Err(abi::EACCES);
    }
    let mut pool = SOCKETS.lock();
    if !pool
        .get(idx as usize)
        .is_some_and(Option::is_some)
    {
        return Err(abi::EBADF);
    }
    let in_use = pool.iter().enumerate().any(|(other, slot)| {
        other != idx as usize
            && slot
                .as_ref()
                .is_some_and(|s| s.path.as_deref() == Some(path.as_slice()))
    });
    if in_use {
        return Err(abi::EADDRINUSE);
    }
    let socket = pool[idx as usize].as_mut().expect("live socket");
    if socket.path.is_some() {
        return Err(abi::EINVAL);
    }
    socket.path = Some(path);
    Ok(())
}

/// `listen(fd, backlog)`: publish a bound path for `connect`.
pub fn listen(idx: u32, backlog: i32) -> Result<(), u32> {
    let mut pool = SOCKETS.lock();
    let Some(Some(socket)) = pool.get_mut(idx as usize) else {
        return Err(abi::EBADF);
    };
    if socket.path.is_none() {
        return Err(abi::EINVAL);
    }
    socket.listening = true;
    // The backlog is advisory here: bound the queue to the Linux default.
    let cap = if backlog <= 0 {
        DEFAULT_BACKLOG
    } else {
        (backlog as usize).min(DEFAULT_BACKLOG)
    };
    socket.pending.reserve(cap);
    Ok(())
}

/// Resolve, authorize and enqueue without releasing the name table lock.
pub fn connect(path: &[u8], uid: u32) -> Result<(u32, u32), u32> {
    let mut pool = SOCKETS.lock();
    let (idx, socket) = pool.iter_mut().enumerate().find_map(|(idx, slot)| {
        let socket = slot.as_mut()?;
        (socket.listening && socket.path.as_deref() == Some(path))
            .then_some((idx, socket))
    }).ok_or(abi::ECONNREFUSED)?;
    if socket.allowed_uid.is_some_and(|allowed| allowed != uid) {
        return Err(abi::ECONNREFUSED);
    }
    if socket.pending.len() >= DEFAULT_BACKLOG {
        return Err(abi::EAGAIN);
    }
    let pair = super::pipe::alloc_socketpair();
    socket.pending.push_back(pair);
    Ok((idx as u32, pair))
}

pub fn pending_count(idx: u32) -> usize {
    SOCKETS
        .lock()
        .get(idx as usize)
        .and_then(Option::as_ref)
        .map(|s| s.pending.len())
        .unwrap_or(0)
}

/// Take the oldest queued connection for `accept`.
pub fn dequeue(idx: u32) -> Option<u32> {
    SOCKETS
        .lock()
        .get_mut(idx as usize)
        .and_then(Option::as_mut)
        .and_then(|s| s.pending.pop_front())
}

/// Remove a specific queued connection, rolling back a failed `connect`.
pub fn withdraw(idx: u32, pair: u32) -> bool {
    let mut pool = SOCKETS.lock();
    match pool.get_mut(idx as usize).and_then(Option::as_mut) {
        Some(socket) => match socket.pending.iter().position(|&queued| queued == pair) {
            Some(pos) => {
                socket.pending.remove(pos);
                true
            }
            None => false,
        },
        None => false,
    }
}

/// Bound path for `getsockname`, if any.
pub fn path(idx: u32) -> Option<Vec<u8>> {
    SOCKETS
        .lock()
        .get(idx as usize)
        .and_then(Option::as_ref)
        .and_then(|s| s.path.clone())
}

pub fn is_listening(idx: u32) -> bool {
    SOCKETS
        .lock()
        .get(idx as usize)
        .and_then(Option::as_ref)
        .is_some_and(|s| s.listening)
}
