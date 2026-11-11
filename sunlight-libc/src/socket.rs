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
    match sys::check(sys::syscall4(
        sys::SYS_EPOLL_CTL,
        epfd as u64,
        op as u64,
        fd as u64,
        event as u64,
    )) {
        Ok(_) => 0,
        Err(error) => {
            errno::set_from_errno(error);
            -1
        }
    }
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
    match sys::check(sys::syscall4(
        sys::SYS_EPOLL_WAIT,
        epfd as u64,
        events as u64,
        maxevents as u64,
        timeout_ms as u64,
    )) {
        Ok(n) => n as i32,
        Err(error) => {
            errno::set_from_errno(error);
            -1
        }
    }
}
