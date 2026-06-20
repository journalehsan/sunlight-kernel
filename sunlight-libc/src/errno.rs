//! POSIX-compatible errno support for SunlightOS.
//!
//! Phase 1: single global errno backed by an AtomicI32.
//! Not thread-safe by design; when threads are added this must become
//! thread-local storage. All callers should treat the value as volatile
//! (it may change after any libc call).

use core::sync::atomic::{AtomicI32, Ordering};

use crate::sys::Errno;

// Standard POSIX errno constants.
pub const EPERM: i32 = 1;
pub const ENOENT: i32 = 2;
pub const EIO: i32 = 5;
pub const E2BIG: i32 = 7;
pub const EAGAIN: i32 = 11;
pub const ENOMEM: i32 = 12;
pub const EACCES: i32 = 13;
pub const EFAULT: i32 = 14;
pub const EINVAL: i32 = 22;
pub const ENOSYS: i32 = 38;

// Phase 1: single-threaded global errno.
static ERRNO_VAL: AtomicI32 = AtomicI32::new(0);

pub fn set_errno(e: i32) {
    ERRNO_VAL.store(e, Ordering::Relaxed);
}

pub fn get_errno() -> i32 {
    ERRNO_VAL.load(Ordering::Relaxed)
}

/// C ABI entry point: `int *__errno_location(void)`.
/// Rust libc and many C-ABI callers use this to locate the errno cell.
///
/// # Safety
/// Phase 1: returns a pointer to the global errno. Not thread-safe.
#[no_mangle]
pub unsafe extern "C" fn __errno_location() -> *mut i32 {
    ERRNO_VAL.as_ptr() as *mut i32
}

/// Map a `sunlight-libc` `Errno` variant to a POSIX errno code and store it.
pub fn set_from_errno(e: Errno) {
    let code = match e {
        Errno::Failed => EIO,
        Errno::Again => EAGAIN,
        Errno::Inval => EINVAL,
        Errno::TooBig => E2BIG,
    };
    set_errno(code);
}
