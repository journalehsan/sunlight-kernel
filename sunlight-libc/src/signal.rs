//! Synchronous signals mediated by the native kernel. No libc callback table.
use crate::{errno, sys};
#[repr(C)]
pub struct SigAction {
    pub handler: usize,
    pub mask: u64,
    pub flags: u64,
}
fn fail(code: i32) -> i32 {
    errno::set_errno(code);
    -1
}
fn status(code: u64) -> i32 {
    if code == 0 {
        0
    } else {
        fail(code as i32)
    }
}
fn bit(sig: i32) -> Option<u64> {
    match sig {
        10 | 12 => Some(1 << (sig - 1)),
        _ => None,
    }
}
#[cfg(all(not(test), target_os = "sunlightos"))]
core::arch::global_asm!(
    ".global __sunlight_signal_return",
    ".hidden __sunlight_signal_return",
    "__sunlight_signal_return:",
    "mov rax, 174",
    "mov rdi, 2",
    "syscall",
    "ud2",
);
#[cfg(all(not(test), target_os = "sunlightos"))]
extern "C" {
    fn __sunlight_signal_return();
}
fn trampoline() -> u64 {
    #[cfg(all(not(test), target_os = "sunlightos"))]
    {
        __sunlight_signal_return as *const () as u64
    }
    #[cfg(test)]
    {
        0
    }
}
#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub unsafe extern "C" fn sigaction(sig: i32, new: *const SigAction, old: *mut SigAction) -> i32 {
    status(sys::syscall6(
        174,
        0,
        sig as u64,
        new as u64,
        old as u64,
        trampoline(),
        0,
    ))
}
#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub unsafe extern "C" fn signal(sig: i32, handler: usize) -> usize {
    let new = SigAction {
        handler,
        mask: 0,
        flags: 0,
    };
    let mut old = SigAction {
        handler: 0,
        mask: 0,
        flags: 0,
    };
    if sigaction(sig, &new, &mut old) == 0 {
        old.handler
    } else {
        usize::MAX
    }
}
#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub unsafe extern "C" fn raise(sig: i32) -> i32 {
    status(sys::syscall2(174, 1, sig as u64))
}
#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub extern "C" fn kill(_pid: i32, _sig: i32) -> i32 {
    fail(errno::ENOSYS)
}
#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub extern "C" fn sigprocmask(_how: i32, _new: *const u64, _old: *mut u64) -> i32 {
    fail(errno::ENOSYS)
}
#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub unsafe extern "C" fn sigemptyset(set: *mut u64) -> i32 {
    if set.is_null() {
        return fail(errno::EFAULT);
    }
    *set = 0;
    0
}
#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub unsafe extern "C" fn sigfillset(set: *mut u64) -> i32 {
    if set.is_null() {
        return fail(errno::EFAULT);
    }
    *set = (1 << 9) | (1 << 11);
    0
}
#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub unsafe extern "C" fn sigaddset(set: *mut u64, sig: i32) -> i32 {
    if set.is_null() {
        return fail(errno::EFAULT);
    }
    let Some(b) = bit(sig) else {
        return fail(errno::EINVAL);
    };
    *set |= b;
    0
}
#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub unsafe extern "C" fn sigdelset(set: *mut u64, sig: i32) -> i32 {
    if set.is_null() {
        return fail(errno::EFAULT);
    }
    let Some(b) = bit(sig) else {
        return fail(errno::EINVAL);
    };
    *set &= !b;
    0
}
#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub unsafe extern "C" fn sigismember(set: *const u64, sig: i32) -> i32 {
    if set.is_null() {
        return fail(errno::EFAULT);
    }
    let Some(b) = bit(sig) else {
        return fail(errno::EINVAL);
    };
    (*set & b != 0) as i32
}
