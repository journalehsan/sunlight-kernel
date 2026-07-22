//! POSIX-compatible errno support for SunlightOS.
//!
//! Uses TLS via the FS segment once `tls::init_tls()` has been called.
//! Falls back to a global AtomicI32 before that (early _start only).

use core::sync::atomic::{AtomicBool, AtomicI32, Ordering};

use crate::sys::Errno;

pub const EPERM: i32 = 1;
pub const ENOENT: i32 = 2;
pub const EIO: i32 = 5;
pub const E2BIG: i32 = 7;
pub const EBADF: i32 = 9;
pub const EAGAIN: i32 = 11;
pub const ENOMEM: i32 = 12;
pub const EACCES: i32 = 13;
pub const EFAULT: i32 = 14;
pub const EINVAL: i32 = 22;
pub const ENOSYS: i32 = 38;

static ERRNO_FALLBACK: AtomicI32 = AtomicI32::new(0);
static TLS_READY: AtomicBool = AtomicBool::new(false);

/// Called by `tls::init_tls()` after FS_BASE is set up.
pub(crate) fn mark_tls_ready() {
    TLS_READY.store(true, Ordering::Release);
}

/// C ABI entry point: `int *__errno_location(void)`.
///
/// Returns the per-thread errno cell via `fs:8` when TLS is ready,
/// or the global fallback before that.
#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn __errno_location() -> *mut i32 {
    if !TLS_READY.load(Ordering::Acquire) {
        return ERRNO_FALLBACK.as_ptr() as *mut i32;
    }
    // Read the TCB self-pointer from fs:0, then return the errno cell at +8.
    let tcb_ptr: usize;
    core::arch::asm!(
        "mov {}, fs:0",
        out(reg) tcb_ptr,
        options(readonly, nostack, preserves_flags)
    );
    (tcb_ptr + 8) as *mut i32
}

pub fn set_errno(e: i32) {
    unsafe {
        *__errno_location() = e;
    }
}

pub fn get_errno() -> i32 {
    unsafe { *__errno_location() }
}

pub fn set_from_errno(e: Errno) {
    let code = match e {
        Errno::Failed => EIO,
        Errno::Again => EAGAIN,
        Errno::Inval => EINVAL,
        Errno::TooBig => E2BIG,
    };
    set_errno(code);
}
