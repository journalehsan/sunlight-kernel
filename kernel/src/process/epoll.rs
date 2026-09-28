//! Minimal Linux epoll emulation for Helios (Linux-compat processes).
//!
//! Enough for `mio::Poll` as used by crossterm's Unix event source:
//! `epoll_create1` → `epoll_ctl(ADD)` on stdin/pipes → `epoll_wait`/`epoll_pwait`.
//!
//! Ready sources currently recognized:
//! - TTY stdin rings (`FileHandle::is_tty_stdin` / legacy fd 0 with tty tab)
//! - Kernel pipes with readable data (or EOF when writers are gone)

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use spin::Mutex;

use super::fd_table::{CapRights, FileHandle};

/// Max interest entries per epoll instance (crossterm uses 2–3).
const MAX_INTERESTS: usize = 64;
/// Max concurrent epoll instances system-wide.
const MAX_INSTANCES: usize = 32;

pub const EPOLLIN: u32 = 0x001;
pub const EPOLLOUT: u32 = 0x004;
pub const EPOLLERR: u32 = 0x008;
pub const EPOLLHUP: u32 = 0x010;
pub const EPOLLET: u32 = 0x8000_0000;

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
            instance
                .interests
                .insert(target_fd, Interest { events, data, last_ready: 0 });
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
        let Some(entry) = process.fd_table.get(fd) else {
            if trace {
                crate::serial_println!("[HELIOS-EPOLL-TRACE] fd={} missing events={:#x}", fd, interest.events);
            }
            // Closed target: surface as ERR|HUP if caller cares about errors.
            if interest.events & (EPOLLERR | EPOLLHUP) != 0 || interest.events & EPOLLIN != 0 {
                out.push(ReadyEvent {
                    events: EPOLLERR | EPOLLHUP,
                    data: interest.data,
                });
            }
            continue;
        };

        let mut revents = 0u32;
        let handle = entry.handle;
        if trace {
            let (readable, writable) = fd_ready(fd, handle, process);
            crate::serial_println!(
                "[HELIOS-EPOLL-TRACE] fd={} handle={:#x} interest={:#x} readable={} writable={}",
                fd, handle.0, interest.events, readable, writable
            );
        }

        if interest.events & EPOLLIN != 0 {
            if fd_ready(fd, handle, process).0 {
                revents |= EPOLLIN;
            }
        }
        if interest.events & EPOLLOUT != 0 {
            if fd_ready(fd, handle, process).1 {
                revents |= EPOLLOUT;
            }
        }

        if interest.events & EPOLLET != 0 {
            let new_edges = revents & !interest.last_ready;
            interest.last_ready = revents;
            revents = new_edges;
        }
        if revents != 0 {
            out.push(ReadyEvent {
                events: revents,
                data: interest.data,
            });
        }
    }

    Ok(out)
}

/// Called after a read or write on any descriptor referring to `handle`.
/// If I/O drained a readiness condition, a later transition can generate a
/// fresh EPOLLET notification even if no epoll_wait occurred while empty.
pub fn refresh_after_io(handle: FileHandle, process: &crate::process::Process) {
    let mut pool = EPOLL_POOL.lock();
    for instance in pool.iter_mut().filter_map(Option::as_mut) {
        for (&fd, interest) in instance.interests.iter_mut() {
            if interest.events & EPOLLET == 0 ||
                !process.fd_table.get(fd).is_some_and(|entry| entry.handle == handle) {
                continue;
            }
            let (readable, writable) = fd_ready(fd, handle, process);
            if !readable {
                interest.last_ready &= !EPOLLIN;
            }
            if !writable {
                interest.last_ready &= !EPOLLOUT;
            }
        }
    }
}

/// Shared readiness query for poll and epoll, including eventfd counters.
pub fn fd_ready(fd: i32, handle: FileHandle, process: &crate::process::Process) -> (bool, bool) {
    if handle.is_eventfd() {
        return super::eventfd::readiness(handle.eventfd_index());
    }
    if handle.is_socket() {
        return super::pipe::socket_stream(handle.socket_index(), handle.socket_endpoint())
            .map(|(read, write)| (super::pipe::pipe_has_data_or_eof(read),
                                    super::pipe::pipe_has_space_or_broken(write)))
            .unwrap_or((true, true));
    }
    (fd_is_readable(fd, handle, process), fd_is_writable(handle))
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
        return handle.pipe_is_write() &&
            crate::process::pipe::pipe_has_space_or_broken(handle.pipe_index());
    }
    if handle.is_tty_stdout() {
        return true;
    }
    // stdout/stderr placeholders
    !handle.is_pipe() && !handle.is_vfs() && !handle.is_epoll() && !handle.is_eventfd()
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
}
