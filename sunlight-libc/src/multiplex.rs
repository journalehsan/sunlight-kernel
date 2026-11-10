//! C multiplexing uses the native poll ABI without changing personality.
use crate::{errno, sys, PollFd};

fn fail(code: i32) -> i32 {
    errno::set_errno(code);
    -1
}

/// # Safety
/// Nonzero count requires count readable/writable pollfd records. The kernel
/// validates user mappings; no Rust slice is constructed from caller memory.
#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub unsafe extern "C" fn poll(fds: *mut PollFd, count: usize, timeout: i32) -> i32 {
    if count > 256 {
        return fail(errno::EINVAL);
    }
    if count != 0 && fds.is_null() {
        return fail(errno::EFAULT);
    }
    match sys::check(sys::syscall3(
        sys::SYS_POLL,
        fds as u64,
        count as u64,
        timeout as u64,
    )) {
        Ok(n) => n as i32,
        Err(e) => {
            errno::set_from_errno(e);
            -1
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct FdSet {
    bits: [u64; 16],
}
#[repr(C)]
pub struct Timeval {
    sec: i64,
    usec: i64,
}
unsafe fn isset(set: *mut FdSet, fd: usize) -> bool {
    !set.is_null() && (*set).bits[fd / 64] & (1 << (fd % 64)) != 0
}
unsafe fn insert(set: *mut FdSet, fd: usize) {
    if !set.is_null() {
        (*set).bits[fd / 64] |= 1 << (fd % 64);
    }
}

fn selected(p: &PollFd) -> bool {
    (p.events & 1 != 0 && p.revents & (1 | 8 | 16) != 0)
        || (p.events & 4 != 0 && p.revents & (4 | 8) != 0)
        || (p.events & 2 != 0 && p.revents & 2 != 0)
}
unsafe fn monotonic_ms() -> Option<u64> {
    let mut t = crate::time::Timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    if crate::time::clock_gettime(crate::time::CLOCK_MONOTONIC, &mut t) != 0 {
        return None;
    }
    Some(
        (t.tv_sec as u64)
            .saturating_mul(1000)
            .saturating_add(t.tv_nsec as u64 / 1_000_000),
    )
}

/// # Safety
/// Non-NULL set and timeout pointers must designate valid, suitably aligned
/// storage. Set pointers must permit writes. Timeout storage is read only.
#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub unsafe extern "C" fn select(
    nfds: i32,
    read: *mut FdSet,
    write: *mut FdSet,
    except: *mut FdSet,
    timeout: *const Timeval,
) -> i32 {
    if !(0..=256).contains(&nfds) {
        return fail(errno::EINVAL);
    }
    let ms = if timeout.is_null() {
        -1
    } else {
        let t = &*timeout;
        if t.sec < 0 || !(0..1_000_000).contains(&t.usec) {
            return fail(errno::EINVAL);
        }
        let Some(ms) = t
            .sec
            .checked_mul(1000)
            .and_then(|s| s.checked_add((t.usec + 999) / 1000))
        else {
            return fail(errno::EINVAL);
        };
        let Ok(ms) = i32::try_from(ms) else {
            return fail(errno::EINVAL);
        };
        ms
    };
    let mut fds = [PollFd::default(); 256];
    let mut count = 0;
    for fd in 0..nfds as usize {
        let events = i16::from(isset(read, fd))
            | (i16::from(isset(write, fd)) << 2)
            | (i16::from(isset(except, fd)) << 1);
        if events != 0 {
            fds[count] = PollFd {
                fd: fd as i32,
                events,
                revents: 0,
            };
            count += 1;
        }
    }
    let deadline = if ms > 0 {
        let Some(now) = monotonic_ms() else { return -1 };
        Some(now.saturating_add(ms as u64))
    } else {
        None
    };
    let mut remaining = ms;
    loop {
        let n = poll(fds.as_mut_ptr(), count, remaining);
        if n < 0 {
            return -1;
        }
        if fds[..count].iter().any(|p| p.revents & 32 != 0) {
            return fail(errno::EBADF);
        }
        if n == 0 || ms == 0 || fds[..count].iter().any(selected) {
            break;
        }
        // A poll hangup/error is unconditional, but is not an exceptional
        // condition for select. Suppress terminal, nonselected entries so an
        // except-only pipe hangup cannot end a finite wait early or spin.
        for p in &mut fds[..count] {
            if p.revents != 0 {
                p.fd = -1;
                p.revents = 0;
            }
        }
        if let Some(end) = deadline {
            let Some(now) = monotonic_ms() else { return -1 };
            if now >= end {
                break;
            }
            remaining = (end - now).min(i32::MAX as u64) as i32;
        }
    }
    for set in [read, write, except] {
        if !set.is_null() {
            (*set).bits = [0; 16];
        }
    }
    let mut ready = 0;
    for p in &fds[..count] {
        for (set, mask, request) in [(read, 1 | 8 | 16, 1), (write, 4 | 8, 4), (except, 2, 2)] {
            if p.events & request != 0 && p.revents & mask != 0 {
                insert(set, p.fd as usize);
                ready += 1;
            }
        }
    }
    ready
}
