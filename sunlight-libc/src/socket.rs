//! Native AF_UNIX pairs, unconnected IPv4 streams, and epoll readiness.
use crate::{errno, sys};

pub const AF_UNIX: i32 = 1;
pub const AF_INET: i32 = 2;
pub const IPPROTO_TCP: i32 = 6;

#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub extern "C" fn socket(domain: i32, kind: i32, protocol: i32) -> i32 {
    let result =
        unsafe { sys::syscall3(sys::SYS_SOCKET, domain as u64, kind as u64, protocol as u64) }
            as i64;
    if result < 0 {
        errno::set_errno((-result) as i32);
        -1
    } else {
        // A reused descriptor must never inherit a stale transport mapping.
        crate::net_bridge::forget(result as i32);
        result as i32
    }
}
pub const SOCK_STREAM: i32 = 1;
pub const SOCK_NONBLOCK: i32 = 0x0000_0800;
pub const SOCK_CLOEXEC: i32 = 0x0008_0000;
pub const MSG_OOB: i32 = 0x01;
pub const MSG_DONTWAIT: i32 = 0x40;

pub const EPOLLIN: u32 = 0x001;
pub const EPOLLPRI: u32 = 0x002;
pub const EPOLLOUT: u32 = 0x004;
pub const EPOLLERR: u32 = 0x008;
pub const EPOLLHUP: u32 = 0x010;
pub const EPOLLONESHOT: u32 = 0x4000_0000;
pub const EPOLLET: u32 = 0x8000_0000;
pub const EPOLL_CTL_ADD: i32 = 1;
pub const EPOLL_CTL_DEL: i32 = 2;
pub const EPOLL_CTL_MOD: i32 = 3;

#[repr(C, align(8))]
#[derive(Clone, Copy)]
pub struct EpollEvent {
    pub events: u32,
    pub data: u64,
}

fn fail(code: i32) -> isize {
    errno::set_errno(code);
    -1
}

fn fail_i32(code: i32) -> i32 {
    errno::set_errno(code);
    -1
}

/// # Safety
/// `fds` must point to two writable `int` slots.
#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub unsafe extern "C" fn socketpair(
    domain: i32,
    socket_type: i32,
    protocol: i32,
    fds: *mut i32,
) -> i32 {
    if fds.is_null() {
        return fail_i32(errno::EFAULT);
    }
    match sys::check(sys::syscall4(
        sys::SYS_SOCKETPAIR,
        domain as u64,
        socket_type as u64,
        protocol as u64,
        fds as u64,
    )) {
        Ok(_) => 0,
        Err(error) => {
            errno::set_from_errno(error);
            -1
        }
    }
}

/// # Safety
/// `buf` must designate `len` readable bytes.
#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub unsafe extern "C" fn send(fd: i32, buf: *const u8, len: usize, flags: i32) -> isize {
    if len != 0 && buf.is_null() {
        return fail(errno::EFAULT);
    }
    if crate::net_bridge::lookup(fd).is_some() {
        return crate::net_bridge::send(fd, buf, len, flags);
    }
    match sys::check(sys::syscall4(
        sys::SYS_SEND,
        fd as u64,
        buf as u64,
        len as u64,
        flags as u64,
    )) {
        Ok(n) => n as isize,
        Err(error) => {
            errno::set_from_errno(error);
            -1
        }
    }
}

/// # Safety
/// `buf` must designate `len` writable bytes.
#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub unsafe extern "C" fn recv(fd: i32, buf: *mut u8, len: usize, flags: i32) -> isize {
    if len != 0 && buf.is_null() {
        return fail(errno::EFAULT);
    }
    if crate::net_bridge::lookup(fd).is_some() {
        return crate::net_bridge::recv(fd, buf, len, flags);
    }
    match sys::check(sys::syscall4(
        sys::SYS_RECV,
        fd as u64,
        buf as u64,
        len as u64,
        flags as u64,
    )) {
        Ok(n) => n as isize,
        Err(error) => {
            errno::set_from_errno(error);
            -1
        }
    }
}

#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub extern "C" fn epoll_create1(flags: i32) -> i32 {
    match sys::check(unsafe { sys::syscall1(sys::SYS_EPOLL_CREATE1, flags as u64) }) {
        Ok(fd) => fd as i32,
        Err(error) => {
            errno::set_from_errno(error);
            -1
        }
    }
}

/// # Safety
/// ADD/MOD require a valid `event`; DEL may pass null.
#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub unsafe extern "C" fn epoll_ctl(epfd: i32, op: i32, fd: i32, event: *mut EpollEvent) -> i32 {
    // Capture the kernel endpoint generation before reading the bridge map.
    // A close/reuse between these operations makes ctl fail, never attach an
    // old transport identity to a newly allocated placeholder.
    let generation = sys::syscall6(sys::SYS_EPOLL_BRIDGE, fd as u64, 0, 0, 0, 0, 3) as i64;
    if generation < 0 {
        return fail_i32((-generation) as i32);
    }
    let identity = crate::net_bridge::lookup(fd).unwrap_or(u64::MAX);
    let result = sys::syscall6(
        sys::SYS_EPOLL_BRIDGE,
        epfd as u64,
        op as u64,
        fd as u64,
        event as u64,
        identity,
        2 | ((generation as u64) << 8),
    ) as i64;
    if result < 0 {
        return fail_i32((-result) as i32);
    }
    0
}

/// # Safety
/// `events` must designate `maxevents` writable records.
#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub unsafe extern "C" fn epoll_wait(
    epfd: i32,
    events: *mut EpollEvent,
    maxevents: i32,
    timeout_ms: i32,
) -> i32 {
    if maxevents <= 0 {
        return fail_i32(errno::EINVAL);
    }
    if events.is_null() {
        return fail_i32(errno::EFAULT);
    }
    // One deadline covers snapshots, IPC, and every internal wait slice.
    let Some(started) = crate::multiplex::monotonic_ms() else {
        return -1;
    };
    let deadline = (timeout_ms > 0).then(|| started.saturating_add(timeout_ms as u64));
    loop {
        let mut observations = [ExternalObservation::default(); 64];
        let count = sys::syscall6(
            sys::SYS_EPOLL_BRIDGE,
            epfd as u64,
            observations.as_mut_ptr() as u64,
            64,
            0,
            0,
            0,
        ) as i64;
        if count < 0 {
            return fail_i32((-count) as i32);
        }
        for observation in &mut observations[..count as usize] {
            let Some(identity) = crate::net_bridge::lookup(observation.fd) else {
                continue;
            };
            if observation.identity == u64::MAX {
                observation.identity = identity;
            } else if identity != observation.identity {
                continue;
            }
            let interest = crate::multiplex::requested_interest(observation.events as i16);
            let ready = match crate::net_bridge::wait_any_per_interest(
                &[observation.identity],
                &[interest],
                0,
            ) {
                Ok(crate::net_bridge::WaitOutcome::Ready(id, bits))
                    if id == observation.identity =>
                {
                    crate::multiplex::map_ready(bits, observation.events as i16)
                }
                Ok(crate::net_bridge::WaitOutcome::Gone) => (EPOLLERR | EPOLLHUP) as i16,
                Ok(_) => 0,
                Err(error) => return fail_i32(error),
            };
            if crate::net_bridge::lookup(observation.fd) == Some(observation.identity) {
                observation.ready = ready as u32;
            }
        }
        // Kernel tokens reject observations made before MOD, DEL, or close.
        let result = sys::syscall6(
            sys::SYS_EPOLL_BRIDGE,
            epfd as u64,
            observations.as_ptr() as u64,
            count as u64,
            events as u64,
            (maxevents as u64).min(128),
            1,
        ) as i64;
        if result < 0 {
            return fail_i32((-result) as i32);
        }
        if result != 0 || timeout_ms == 0 {
            return result as i32;
        }
        let Some(now) = crate::multiplex::monotonic_ms() else {
            return -1;
        };
        let remaining = deadline.map_or(20, |end| end.saturating_sub(now).min(20));
        if remaining == 0 {
            return 0;
        }
        // Bounded sleep revisits every registration, including sets larger
        // than NetWait's batch capacity. No registration lock spans IPC/sleep.
        let delay = crate::time::Timespec {
            tv_sec: 0,
            tv_nsec: (remaining * 1_000_000) as i64,
        };
        if crate::time::nanosleep(&delay, core::ptr::null_mut()) != 0 {
            return -1;
        }
    }
}

use sunlight_ipc::native_epoll::ExternalObservation;

/// Attach an AF_INET/SOCK_STREAM descriptor to the `net` service.
///
/// # Safety
/// `addr` must designate `len` readable bytes.
#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub unsafe extern "C" fn connect(fd: i32, addr: *const u8, len: u32) -> i32 {
    crate::net_bridge::connect(fd, addr, len)
}

/// # Safety
/// `addr` must designate `len` readable bytes.
#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub unsafe extern "C" fn bind(fd: i32, addr: *const u8, len: u32) -> i32 {
    crate::net_bridge::bind(fd, addr, len)
}

#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub extern "C" fn listen(fd: i32, backlog: i32) -> i32 {
    crate::net_bridge::listen(fd, backlog)
}

/// # Safety
/// `addr`/`len_ptr` are optional writable out-parameters.
#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub unsafe extern "C" fn accept(fd: i32, addr: *mut u8, len_ptr: *mut u32) -> i32 {
    crate::net_bridge::accept(fd, addr, len_ptr)
}

/// # Safety
/// `addr` must designate at least 16 writable bytes and `len_ptr` a writable
/// `socklen_t`.
#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub unsafe extern "C" fn getsockname(fd: i32, addr: *mut u8, len_ptr: *mut u32) -> i32 {
    crate::net_bridge::getsockname(fd, addr, len_ptr)
}

/// # Safety
/// See `getsockname`.
#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub unsafe extern "C" fn getpeername(fd: i32, addr: *mut u8, len_ptr: *mut u32) -> i32 {
    crate::net_bridge::getpeername(fd, addr, len_ptr)
}

/// # Safety
/// `buf` must designate `len` readable bytes when `len != 0`.
#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub unsafe extern "C" fn sendto(
    fd: i32,
    buf: *const u8,
    len: usize,
    flags: i32,
    addr: *const u8,
    addr_len: u32,
) -> isize {
    crate::net_bridge::sendto(fd, buf, len, flags, addr, addr_len)
}

/// # Safety
/// See `recv` and `getpeername`.
#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub unsafe extern "C" fn recvfrom(
    fd: i32,
    buf: *mut u8,
    len: usize,
    flags: i32,
    addr: *mut u8,
    addr_len: *mut u32,
) -> isize {
    crate::net_bridge::recvfrom(fd, buf, len, flags, addr, addr_len)
}

#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub extern "C" fn shutdown(fd: i32, how: i32) -> i32 {
    crate::net_bridge::shutdown(fd, how)
}

/// Non-standard bounded diagnostic accessor for native networking fixtures.
#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub extern "C" fn sunlight_net_diagnostic(selector: i32) -> i64 {
    match crate::net_bridge::diagnostic(selector as u64) {
        Some(value) => value as i64,
        None => -1,
    }
}
