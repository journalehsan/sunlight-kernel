//! C multiplexing uses the native poll ABI without changing personality.
use crate::{errno, sys, PollFd};
use sunlight_ipc::netop::NetWait;

const POLLIN: i16 = 0x001;
const POLLOUT: i16 = 0x004;
const POLLERR: i16 = 0x008;
const POLLHUP: i16 = 0x010;
const POLLNVAL: i16 = 0x020;

const MAX_POLL_FDS: usize = 256;
/// Mixed local+network sets cannot share one kernel wait primitive. Recheck
/// at the net_server progress tick rather than spinning.
const MIXED_WAIT_SLICE_MS: u64 = 20;
/// An infinite poll is implemented as repeated bounded waits so IPC deadlines
/// never overflow and descriptor lifetimes are revalidated.
const INFINITE_WAIT_SLICE_MS: u64 = 1_000;

fn fail(code: i32) -> i32 {
    errno::set_errno(code);
    -1
}

#[derive(Clone, Copy, Default)]
struct NetworkTarget {
    index: usize,
    fd: i32,
    socket_id: u64,
    interest: u32,
}

pub(crate) fn requested_interest(events: i16) -> u32 {
    use sunlight_ipc::netop::NetReady;
    let mut interest = (NetReady::RESET | NetReady::CLOSED | NetReady::ERROR) as u32;
    if events & POLLIN != 0 {
        interest |= (NetReady::ACCEPT | NetReady::READ | NetReady::EOF) as u32;
    }
    if events & POLLOUT != 0 {
        interest |= NetReady::WRITE as u32;
    }
    // POLLPRI is not implemented for TCP; terminal bits are still explicit.
    interest
}

pub(crate) fn map_ready(bits: u64, events: i16) -> i16 {
    use sunlight_ipc::netop::NetReady;
    let mut revents = 0;
    if bits & (NetReady::ACCEPT | NetReady::READ | NetReady::EOF) != 0 {
        revents |= POLLIN;
    }
    if bits & NetReady::WRITE != 0 {
        revents |= POLLOUT;
    }
    if bits & (NetReady::RESET | NetReady::ERROR) != 0 {
        revents |= POLLERR;
    }
    if bits & NetReady::CLOSED != 0 {
        revents |= POLLHUP;
    }
    revents & (events | POLLERR | POLLHUP)
}

fn ready_count(records: &[PollFd], count: usize) -> i32 {
    records[..count].iter().filter(|fd| fd.revents != 0).count() as i32
}

/// Collect the current bridged descriptors after a kernel snapshot. The
/// kernel result stays authoritative for `POLLNVAL` and negative entries.
fn network_targets(
    records: &[PollFd],
    count: usize,
    targets: &mut [NetworkTarget; MAX_POLL_FDS],
) -> usize {
    let mut length = 0;
    for (index, record) in records[..count].iter().enumerate() {
        if record.fd < 0 || record.revents & POLLNVAL != 0 {
            continue;
        }
        let Some(socket_id) = crate::net_bridge::lookup(record.fd) else {
            continue;
        };
        targets[length] = NetworkTarget {
            index,
            fd: record.fd,
            socket_id,
            interest: requested_interest(record.events),
        };
        length += 1;
    }
    length
}

/// Replace the kernel's placeholder `POLLERR` for bridged `inet_socket` fds
/// with real net_server readiness. Returns `Ok(true)` when a descriptor was
/// closed/reused during the scan and the caller must rebuild its snapshot.
fn collect_network(records: &mut [PollFd], targets: &[NetworkTarget]) -> Result<bool, i32> {
    for target in targets {
        if crate::net_bridge::lookup(target.fd) != Some(target.socket_id) {
            return Ok(true);
        }
    }
    for target in targets {
        let record = &mut records[target.index];
        if record.revents & POLLNVAL != 0 {
            continue;
        }
        match crate::net_bridge::wait_any_per_interest(&[target.socket_id], &[target.interest], 0)?
        {
            crate::net_bridge::WaitOutcome::Ready(_, bits) => {
                record.revents = map_ready(bits, record.events);
            }
            crate::net_bridge::WaitOutcome::Empty => record.revents = 0,
            // All requested identities were reaped: terminal error/hangup,
            // not invented readability.
            crate::net_bridge::WaitOutcome::Gone => {
                use sunlight_ipc::netop::NetReady;
                record.revents = map_ready(NetReady::RESET | NetReady::CLOSED, record.events);
            }
        }
    }
    Ok(false)
}

fn targets_current(targets: &[NetworkTarget]) -> bool {
    targets
        .iter()
        .all(|target| crate::net_bridge::lookup(target.fd) == Some(target.socket_id))
}

/// # Safety
/// Nonzero count requires count readable/writable pollfd records. The kernel
/// validates user mappings; no Rust slice is constructed from caller memory.
#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub unsafe extern "C" fn poll(fds: *mut PollFd, count: usize, timeout: i32) -> i32 {
    if count > MAX_POLL_FDS {
        return fail(errno::EINVAL);
    }
    if count != 0 && fds.is_null() {
        return fail(errno::EFAULT);
    }
    // Preserve the kernel's validation and non-network behavior with a
    // zero-timeout snapshot. This also fills revents for local descriptors.
    let snapshot = match sys::check(sys::syscall3(sys::SYS_POLL, fds as u64, count as u64, 0)) {
        Ok(n) => n as i32,
        Err(e) => {
            errno::set_from_errno(e);
            return -1;
        }
    };
    if count == 0 {
        if timeout == 0 {
            return 0;
        }
        return match sys::check(sys::syscall3(sys::SYS_POLL, 0, 0, timeout as u64)) {
            Ok(n) => n as i32,
            Err(e) => {
                errno::set_from_errno(e);
                -1
            }
        };
    }

    let mut records = [PollFd::default(); MAX_POLL_FDS];
    core::ptr::copy_nonoverlapping(fds, records.as_mut_ptr(), count);
    let mut targets = [NetworkTarget::default(); MAX_POLL_FDS];
    let mut target_count = network_targets(&records, count, &mut targets);

    // No bridged descriptor: retain the original full kernel wait.
    if target_count == 0 {
        if timeout == 0 {
            return snapshot;
        }
        return match sys::check(sys::syscall3(
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
        };
    }

    let Some(started) = monotonic_now() else {
        return -1;
    };
    let deadline = (timeout > 0).then(|| started.saturating_add(timeout as u64));
    let mut refresh = true;
    let mut stale_retries = 0u32;
    loop {
        if refresh {
            // Re-read the kernel table so close/reuse, new local readiness and
            // POLLNVAL are never taken from a stale record.
            match sys::check(sys::syscall3(sys::SYS_POLL, fds as u64, count as u64, 0)) {
                Ok(_) => {}
                Err(e) => {
                    errno::set_from_errno(e);
                    return -1;
                }
            }
            core::ptr::copy_nonoverlapping(fds, records.as_mut_ptr(), count);
            target_count = network_targets(&records, count, &mut targets);
            if target_count == 0 {
                // Every bridged fd was closed/reused away during the wait.
                if timeout == 0 {
                    return ready_count(&records, count);
                }
                let wait_timeout = if timeout < 0 {
                    timeout as u64
                } else {
                    let remaining = remaining_ms(deadline);
                    if remaining == 0 {
                        core::ptr::copy_nonoverlapping(records.as_ptr(), fds, count);
                        return 0;
                    }
                    remaining.min(i32::MAX as u64)
                };
                return match sys::check(sys::syscall3(
                    sys::SYS_POLL,
                    fds as u64,
                    count as u64,
                    wait_timeout,
                )) {
                    Ok(n) => n as i32,
                    Err(e) => {
                        errno::set_from_errno(e);
                        -1
                    }
                };
            }
        }
        match collect_network(&mut records, &targets[..target_count]) {
            Ok(false) => {}
            Ok(true) => {
                refresh = true;
                continue;
            }
            Err(error) => return fail(error),
        }
        let ready = ready_count(&records, count);
        if ready != 0 || timeout == 0 {
            if !targets_current(&targets[..target_count]) {
                stale_retries += 1;
                if stale_retries > 8 {
                    return fail(errno::EAGAIN);
                }
                refresh = true;
                continue;
            }
            core::ptr::copy_nonoverlapping(records.as_ptr(), fds, count);
            return ready;
        }
        let remaining = remaining_ms(deadline);
        if remaining == 0 {
            if !targets_current(&targets[..target_count]) {
                stale_retries += 1;
                if stale_retries > 8 {
                    return fail(errno::EAGAIN);
                }
                refresh = true;
                continue;
            }
            core::ptr::copy_nonoverlapping(records.as_ptr(), fds, count);
            return 0;
        }

        // Build the local-only set. A bridged fd must never enter this buffer:
        // the kernel would report its placeholder POLLERR and return instantly.
        let mut network_index = [false; MAX_POLL_FDS];
        for target in &targets[..target_count] {
            network_index[target.index] = true;
        }
        let mut local = [PollFd::default(); MAX_POLL_FDS];
        let mut local_count = 0;
        for (index, record) in records[..count].iter().enumerate() {
            if record.fd < 0 || network_index[index] || record.revents & POLLNVAL != 0 {
                continue;
            }
            local[local_count] = PollFd {
                fd: record.fd,
                events: record.events,
                revents: 0,
            };
            local_count += 1;
        }

        if local_count != 0 || target_count > NetWait::MAX_IDENTITIES {
            // Wake promptly on local readiness (or recheck oversized network
            // sets) at the bounded mixed-wait slice. This parks in the kernel,
            // it does not drain CPU.
            let slice = remaining.min(MIXED_WAIT_SLICE_MS).min(i32::MAX as u64) as i32;
            let records_ptr = if local_count != 0 {
                local.as_mut_ptr()
            } else {
                core::ptr::null_mut()
            };
            match sys::check(sys::syscall3(
                sys::SYS_POLL,
                records_ptr as u64,
                local_count as u64,
                slice as u64,
            )) {
                Ok(_) => {}
                Err(e) => {
                    errno::set_from_errno(e);
                    return -1;
                }
            }
            refresh = true;
            continue;
        }

        let mut identities = [0u64; NetWait::MAX_IDENTITIES];
        let mut interests = [0u32; NetWait::MAX_IDENTITIES];
        for (slot, target) in targets[..target_count].iter().enumerate() {
            identities[slot] = target.socket_id;
            interests[slot] = target.interest;
        }
        let wait_ms = match deadline {
            Some(_) => remaining,
            None => INFINITE_WAIT_SLICE_MS,
        };
        match crate::net_bridge::wait_any_per_interest(
            &identities[..target_count],
            &interests[..target_count],
            wait_ms,
        ) {
            Ok(crate::net_bridge::WaitOutcome::Ready(_, _))
            | Ok(crate::net_bridge::WaitOutcome::Empty) => {
                refresh = true;
            }
            Ok(crate::net_bridge::WaitOutcome::Gone) => {
                if !targets_current(&targets[..target_count]) {
                    stale_retries += 1;
                    if stale_retries > 8 {
                        return fail(errno::EAGAIN);
                    }
                    refresh = true;
                    continue;
                }
                use sunlight_ipc::netop::NetReady;
                for target in &targets[..target_count] {
                    let record = &mut records[target.index];
                    if record.revents & POLLNVAL == 0 {
                        record.revents =
                            map_ready(NetReady::RESET | NetReady::CLOSED, record.events);
                    }
                }
                core::ptr::copy_nonoverlapping(records.as_ptr(), fds, count);
                return ready_count(&records, count);
            }
            Err(error) => return fail(error),
        }
    }
}

fn remaining_ms(deadline: Option<u64>) -> u64 {
    let Some(deadline) = deadline else {
        return INFINITE_WAIT_SLICE_MS;
    };
    let Some(now) = monotonic_now() else {
        return 0;
    };
    deadline.saturating_sub(now)
}

fn monotonic_now() -> Option<u64> {
    // SAFETY: clock_gettime only reads the clock through a valid stack value.
    unsafe { monotonic_ms() }
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
pub(crate) unsafe fn monotonic_ms() -> Option<u64> {
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
