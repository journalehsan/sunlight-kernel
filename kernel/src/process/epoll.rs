//! Shared native and Linux-compatible descriptor event queues.
//!
//! Enough for `mio::Poll` as used by crossterm's Unix event source:
//! `epoll_create1` → `epoll_ctl(ADD)` on stdin/pipes → `epoll_wait`/`epoll_pwait`.
//!
//! Native records use aligned LP64 layout; Linux records retain packed layout.
//! Readiness is shared with poll for pipes, stream sockets, TTY and eventfd.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use spin::Mutex;

use super::fd_table::{CapRights, FileHandle};

/// Max interest entries per epoll instance (crossterm uses 2–3).
const MAX_INTERESTS: usize = 64;
/// Max concurrent epoll instances system-wide.
const MAX_INSTANCES: usize = 32;

pub const EPOLLIN: u32 = 0x001;
pub const EPOLLPRI: u32 = 0x002;
pub const EPOLLOUT: u32 = 0x004;
pub const EPOLLERR: u32 = 0x008;
pub const EPOLLHUP: u32 = 0x010;
pub const EPOLLONESHOT: u32 = 0x4000_0000;
pub const EPOLLET: u32 = 0x8000_0000;

pub const POLLIN: i16 = 0x001;
pub const POLLPRI: i16 = 0x002;
pub const POLLOUT: i16 = 0x004;
pub const POLLERR: i16 = 0x008;
pub const POLLHUP: i16 = 0x010;
pub const POLLNVAL: i16 = 0x020;

pub const EPOLL_CTL_ADD: i32 = 1;
pub const EPOLL_CTL_DEL: i32 = 2;
pub const EPOLL_CTL_MOD: i32 = 3;

/// Packed `struct epoll_event` size on x86_64 Linux (`__EPOLL_PACKED`).
pub const EPOLL_EVENT_SIZE: usize = 12;

#[derive(Clone, Copy)]
struct Interest {
    events: u32,
    /// Last observed readiness for edge-triggered interests. An I/O call
    /// that drains readiness clears this mask through refresh_after_io.
    last_ready: u32,
    disabled: bool,
    /// `epoll_data_t` as raw little-endian bytes (union of ptr/fd/u32/u64).
    data: [u8; 8],
}

struct EpollInstance {
    /// Target fd → interest.
    interests: BTreeMap<i32, Interest>,
}

static EPOLL_POOL: Mutex<Vec<Option<EpollInstance>>> = Mutex::new(Vec::new());

fn alloc_instance() -> Option<u32> {
    let mut pool = EPOLL_POOL.lock();
    let instance = EpollInstance {
        interests: BTreeMap::new(),
    };
    for (idx, slot) in pool.iter_mut().enumerate() {
        if slot.is_none() {
            *slot = Some(instance);
            return Some(idx as u32);
        }
    }
    if pool.len() >= MAX_INSTANCES {
        return None;
    }
    let idx = pool.len() as u32;
    pool.push(Some(instance));
    Some(idx)
}

pub fn free_instance(idx: u32) {
    let mut pool = EPOLL_POOL.lock();
    if let Some(slot) = pool.get_mut(idx as usize) {
        *slot = None;
    }
}

/// Create an epoll fd in the current process fd table.
pub fn create_epoll_fd(
    sched: &mut crate::sched::Scheduler,
    cloexec: bool,
) -> Result<i32, EpollError> {
    let idx = alloc_instance().ok_or(EpollError::NoSpace)?;
    let handle = FileHandle::epoll(idx);
    let mut flags = 0u32;
    if cloexec {
        // Linux O_CLOEXEC
        flags |= 0x0008_0000;
    }
    let process = sched.current_shared_process_mut();
    match process.fd_table.open(
        handle,
        CapRights::new(CapRights::READ | CapRights::WRITE),
        flags,
    ) {
        Ok(fd) => Ok(fd),
        Err(_) => {
            free_instance(idx);
            Err(EpollError::NoSpace)
        }
    }
}

pub fn ctl(
    epoll_idx: u32,
    op: i32,
    target_fd: i32,
    events: u32,
    data: [u8; 8],
) -> Result<(), EpollError> {
    if target_fd < 0 {
        return Err(EpollError::BadFd);
    }
    let mut pool = EPOLL_POOL.lock();
    let instance = pool
        .get_mut(epoll_idx as usize)
        .and_then(|s| s.as_mut())
        .ok_or(EpollError::BadFd)?;

    match op {
        EPOLL_CTL_ADD => {
            if instance.interests.contains_key(&target_fd) {
                return Err(EpollError::Exist);
            }
            if instance.interests.len() >= MAX_INTERESTS {
                return Err(EpollError::NoSpace);
            }
            instance.interests.insert(
                target_fd,
                Interest {
                    events,
                    data,
                    last_ready: 0,
                    disabled: false,
                },
            );
            Ok(())
        }
        EPOLL_CTL_MOD => {
            let entry = instance
                .interests
                .get_mut(&target_fd)
                .ok_or(EpollError::Noent)?;
            entry.events = events;
            entry.data = data;
            entry.last_ready = 0;
            entry.disabled = false;
            Ok(())
        }
        EPOLL_CTL_DEL => {
            if instance.interests.remove(&target_fd).is_none() {
                return Err(EpollError::Noent);
            }
            Ok(())
        }
        _ => Err(EpollError::Inval),
    }
}

/// Snapshot of a ready event for userspace copy-out.
pub struct ReadyEvent {
    pub events: u32,
    pub data: [u8; 8],
}

/// Collect ready interests without sleeping.
pub fn collect_ready(
    epoll_idx: u32,
    maxevents: usize,
    sched: &crate::sched::Scheduler,
) -> Result<Vec<ReadyEvent>, EpollError> {
    #[cfg(not(test))]
    let trace = if option_env!("SUNLIGHT_INJECT_PHASE") == Some("yazi-phase1") {
        use core::sync::atomic::{AtomicUsize, Ordering};
        static TRACE_COUNT: AtomicUsize = AtomicUsize::new(0);
        TRACE_COUNT.fetch_add(1, Ordering::Relaxed) < 12
    } else {
        false
    };
    #[cfg(test)]
    let trace = false;
    let mut pool = EPOLL_POOL.lock();
    let instance = pool
        .get_mut(epoll_idx as usize)
        .and_then(|s| s.as_mut())
        .ok_or(EpollError::BadFd)?;

    // Linux CLONE_FILES workers use the group owner's FD table. Looking in
    // the worker's private placeholder table reports live interests as HUP.
    let process = sched.current_shared_process();
    let mut out = Vec::new();

    for (&fd, interest) in instance.interests.iter_mut() {
        if out.len() >= maxevents {
            break;
        }
        if interest.disabled {
            continue;
        }
        let Some(entry) = process.fd_table.get(fd) else {
            if trace {
                crate::serial_println!(
                    "[HELIOS-EPOLL-TRACE] fd={} missing events={:#x}",
                    fd,
                    interest.events
                );
            }
            // Closed targets are always reported as ERR|HUP by Linux epoll.
            out.push(ReadyEvent {
                events: EPOLLERR | EPOLLHUP,
                data: interest.data,
            });
            continue;
        };

        let handle = entry.handle;
        // Nested epoll descriptors are not part of the current readiness
        // graph; reporting them ready would require a second pool lock.
        let readiness = if handle.is_epoll() {
            0
        } else {
            descriptor_events_depth(fd, handle, process, 0)
        };
        let mut revents = (readiness & (POLLIN | POLLPRI | POLLOUT)) as u32 & interest.events;
        revents |= (readiness & (POLLERR | POLLHUP)) as u32;
        if trace {
            crate::serial_println!(
                "[HELIOS-EPOLL-TRACE] fd={} handle={:#x} interest={:#x} readiness={:#x}",
                fd,
                handle.0,
                interest.events,
                readiness
            );
        }

        if interest.events & EPOLLET != 0 {
            let new_edges = revents & !interest.last_ready;
            interest.last_ready = revents;
            revents = new_edges;
        }
        if revents != 0 {
            if interest.events & EPOLLONESHOT != 0 {
                interest.disabled = true;
            }
            out.push(ReadyEvent {
                events: revents,
                data: interest.data,
            });
        }
    }

    Ok(out)
}

/// Remove a closed descriptor only from queues held by its descriptor table.
pub fn detach_fd(fd: i32, process: &crate::process::Process) {
    let mut pool = EPOLL_POOL.lock();
    for number in 0..256 {
        let Some(entry) = process.fd_table.get(number) else {
            continue;
        };
        if !entry.handle.is_epoll() {
            continue;
        }
        if let Some(instance) = pool
            .get_mut(entry.handle.epoll_index() as usize)
            .and_then(Option::as_mut)
        {
            instance.interests.remove(&fd);
        }
    }
}

/// Called after a read or write on any descriptor referring to `handle`.
/// If I/O drained a readiness condition, a later transition can generate a
/// fresh EPOLLET notification even if no epoll_wait occurred while empty.
pub fn refresh_after_io(handle: FileHandle, process: &crate::process::Process) {
    let mut pool = EPOLL_POOL.lock();
    for number in 0..256 {
        let Some(entry) = process.fd_table.get(number) else {
            continue;
        };
        if !entry.handle.is_epoll() {
            continue;
        }
        let Some(instance) = pool
            .get_mut(entry.handle.epoll_index() as usize)
            .and_then(Option::as_mut)
        else {
            continue;
        };
        for (&fd, interest) in instance.interests.iter_mut() {
            if interest.events & EPOLLET == 0
                || !process
                    .fd_table
                    .get(fd)
                    .is_some_and(|entry| entry.handle == handle)
            {
                continue;
            }
            let readiness = descriptor_events_depth(fd, handle, process, 0) as u32;
            interest.last_ready &= readiness;
        }
    }
}

/// Shared readiness query for poll and epoll, including eventfd counters.
pub fn fd_ready(fd: i32, handle: FileHandle, process: &crate::process::Process) -> (bool, bool) {
    let events = descriptor_events(fd, handle, process);
    (events & POLLIN != 0, events & POLLOUT != 0)
}

/// Complete poll-style readiness for a descriptor. ERR, HUP and PRI are
/// produced by the backing object; callers decide which requested bits to
/// expose. This is shared by native poll and epoll so their state cannot drift.
pub fn descriptor_events(fd: i32, handle: FileHandle, process: &crate::process::Process) -> i16 {
    descriptor_events_depth(fd, handle, process, 0)
}

fn descriptor_events_depth(
    fd: i32,
    handle: FileHandle,
    process: &crate::process::Process,
    depth: usize,
) -> i16 {
    if handle.is_eventfd() {
        let (readable, writable) = super::eventfd::readiness(handle.eventfd_index());
        return (if readable { POLLIN } else { 0 }) | (if writable { POLLOUT } else { 0 });
    }
    if handle.is_inet_socket() {
        // Both I/O directions immediately report ENOTCONN. This is an
        // error state, never evidence of a connected/writable stream.
        return POLLERR;
    }
    if handle.is_unix_socket() {
        let idx = handle.unix_socket_index();
        let readable =
            super::unix_socket::is_listening(idx) && super::unix_socket::pending_count(idx) > 0;
        return if readable { POLLIN } else { 0 };
    }
    if handle.is_socket() {
        let index = handle.socket_index();
        let endpoint = handle.socket_endpoint();
        let Some((read, write)) = super::pipe::socket_stream(index, endpoint) else {
            return POLLERR | POLLHUP;
        };
        let mut events = 0;
        if super::pipe::pipe_has_data_or_eof(read) {
            events |= POLLIN;
        }
        if super::pipe::pipe_has_space_or_broken(write) {
            events |= POLLOUT;
        }
        // An orderly full-duplex peer close is HUP/EOF, not a pending
        // socket error. A subsequent write reports the broken stream.
        if super::pipe::pipe_state(read).is_some_and(|(_, _, writers)| writers == 0) {
            events |= POLLHUP;
        }
        if super::pipe::socket_has_oob(index, endpoint) {
            events |= POLLPRI;
        }
        return events;
    }
    if handle.is_pipe() {
        let mut events = 0;
        if let Some((_, readers, writers)) = super::pipe::pipe_state(handle.pipe_index()) {
            if handle.pipe_is_write() {
                if super::pipe::pipe_has_space_or_broken(handle.pipe_index()) {
                    events |= POLLOUT;
                }
                if readers == 0 {
                    events |= POLLERR;
                }
            } else {
                if super::pipe::pipe_has_data_or_eof(handle.pipe_index()) {
                    events |= POLLIN;
                }
                if writers == 0 {
                    events |= POLLHUP;
                }
            }
        } else {
            events |= POLLERR | POLLHUP;
        }
        return events;
    }
    if handle.is_epoll() {
        if depth >= 4 {
            return 0;
        }
        return epoll_poll_events(handle.epoll_index(), process, depth + 1);
    }
    (if fd_is_readable(fd, handle, process) {
        POLLIN
    } else {
        0
    }) | (if fd_is_writable(handle) { POLLOUT } else { 0 })
}

fn epoll_poll_events(epoll_idx: u32, process: &crate::process::Process, depth: usize) -> i16 {
    let interests = {
        let pool = EPOLL_POOL.lock();
        let Some(instance) = pool.get(epoll_idx as usize).and_then(Option::as_ref) else {
            return POLLNVAL;
        };
        instance
            .interests
            .iter()
            .map(|(&fd, interest)| (fd, interest.events))
            .collect::<Vec<_>>()
    };
    let mut inner = 0u32;
    for (fd, requested) in interests {
        let Some(entry) = process.fd_table.get(fd) else {
            inner |= EPOLLERR | EPOLLHUP;
            continue;
        };
        let readiness = descriptor_events_depth(fd, entry.handle, process, depth);
        inner |= ((readiness & (POLLIN | POLLPRI | POLLOUT)) as u32 & requested)
            | (readiness & (POLLERR | POLLHUP)) as u32;
    }
    if inner == 0 {
        return 0;
    }
    let mut events = POLLIN;
    if inner & EPOLLPRI != 0 {
        events |= POLLPRI;
    }
    if inner & EPOLLOUT != 0 {
        events |= POLLOUT;
    }
    events |= (inner & (EPOLLERR | EPOLLHUP)) as i16;
    events
}

fn fd_is_readable(fd: i32, handle: FileHandle, process: &crate::process::Process) -> bool {
    if handle.is_tty_stdin() {
        return crate::process::tty_io::has_stdin(handle.tty_tab() as usize);
    }
    if handle.is_pipe() && !handle.pipe_is_write() {
        return pipe_readable(handle.pipe_index());
    }
    // Legacy stdio fd0 (not yet tagged as tty_stdin): use process tab 0.
    if fd == 0 && !handle.is_pipe() && !handle.is_vfs() {
        let tab = process
            .fd_table
            .get(0)
            .map(|e| e.handle.tty_tab() as usize)
            .unwrap_or(0);
        // Only treat as TTY if the handle looks like a tty or plain stdio slot.
        if handle.is_tty_stdin() || handle.0 < 3 {
            return crate::process::tty_io::has_stdin(tab);
        }
    }
    false
}

fn fd_is_writable(handle: FileHandle) -> bool {
    if handle.is_pipe() {
        return handle.pipe_is_write()
            && crate::process::pipe::pipe_has_space_or_broken(handle.pipe_index());
    }
    if handle.is_tty_stdout() {
        return true;
    }
    // stdout/stderr placeholders
    !handle.is_pipe()
        && !handle.is_vfs()
        && !handle.is_epoll()
        && !handle.is_eventfd()
        && !handle.is_unix_socket()
        && !handle.is_inet_socket()
}

fn pipe_readable(pool_idx: u32) -> bool {
    // Reuse pipe pool internals via a small helper on the pipe module.
    crate::process::pipe::pipe_has_data_or_eof(pool_idx)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EpollError {
    BadFd,
    NoSpace,
    Exist,
    Noent,
    Inval,
}

impl EpollError {
    pub fn to_linux_errno(self) -> u64 {
        match self {
            EpollError::BadFd => 9,    // EBADF
            EpollError::NoSpace => 24, // EMFILE
            EpollError::Exist => 17,   // EEXIST
            EpollError::Noent => 2,    // ENOENT
            EpollError::Inval => 22,   // EINVAL
        }
    }

    pub fn to_native_result(self) -> u64 {
        match self {
            EpollError::BadFd => u64::MAX - 4, // ERR_EBADF
            EpollError::Inval => u64::MAX - 5, // ERR_EINVAL
            EpollError::NoSpace | EpollError::Exist | EpollError::Noent => u64::MAX,
        }
    }
}
