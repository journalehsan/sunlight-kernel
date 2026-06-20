//! POSIX time support for SunlightOS.

use crate::errno::{set_errno, EFAULT};
use crate::sys::{check, syscall2, SYS_CLOCK_GETTIME};

/// POSIX clock IDs supported in Phase 1.
pub const CLOCK_REALTIME: i32 = 0;
pub const CLOCK_MONOTONIC: i32 = 1;

/// POSIX `struct timespec` — seconds + nanoseconds.
#[repr(C)]
pub struct Timespec {
    pub tv_sec: i64,
    pub tv_nsec: i64,
}

/// Return the current time in `tp` for the given `clockid`.
///
/// Supported clocks:
/// - `CLOCK_REALTIME`  -> Seconds since Unix epoch
/// - `CLOCK_MONOTONIC` -> Nanoseconds since boot
#[no_mangle]
pub unsafe extern "C" fn clock_gettime(clockid: i32, tp: *mut Timespec) -> i32 {
    if tp.is_null() {
        set_errno(EFAULT);
        return -1;
    }

    let ret = unsafe { syscall2(SYS_CLOCK_GETTIME, clockid as u64, tp as u64) };

    match check(ret) {
        Ok(_) => 0,
        Err(e) => {
            crate::errno::set_from_errno(e);
            -1
        }
    }
}
