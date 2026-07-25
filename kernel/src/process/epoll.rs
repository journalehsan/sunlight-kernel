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

pub const EPOLL_CTL_ADD: i32 = 1;
pub const EPOLL_CTL_DEL: i32 = 2;
pub const EPOLL_CTL_MOD: i32 = 3;

/// Packed `struct epoll_event` size on x86_64 Linux (`__EPOLL_PACKED`).
pub const EPOLL_EVENT_SIZE: usize = 12;

#[derive(Clone, Copy)]
struct Interest {
    events: u32,
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
    let process = sched.current_process_mut();
    match process
        .fd_table
        .open(handle, CapRights::new(CapRights::READ | CapRights::WRITE), flags)
    {
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
    let pool = EPOLL_POOL.lock();
    let instance = pool
        .get(epoll_idx as usize)
        .and_then(|s| s.as_ref())
        .ok_or(EpollError::BadFd)?;

    let process = sched.current_process();
    let mut out = Vec::new();

    for (&fd, interest) in instance.interests.iter() {
        if out.len() >= maxevents {
            break;
        }
        let Some(entry) = process.fd_table.get(fd) else {
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

        if interest.events & EPOLLIN != 0 {
            if fd_is_readable(fd, handle, process) {
                revents |= EPOLLIN;
            }
        }
        if interest.events & EPOLLOUT != 0 {
            if fd_is_writable(handle) {
                revents |= EPOLLOUT;
            }
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
    if handle.is_tty_stdout() || handle.is_pipe() && handle.pipe_is_write() {
        return true;
    }
    // stdout/stderr placeholders
    !handle.is_pipe() && !handle.is_vfs() && !handle.is_epoll()
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
