//! Minimal time support for SunlightOS (Phase 1).
//!
//! `clock_gettime` is backed by the `sysinfo` syscall, which provides
//! `unix_time` (wall-clock seconds) and `uptime_secs` (monotonic seconds).
//! Sub-second precision is not available; `tv_nsec` is always 0.

use crate::errno::{set_errno, EINVAL, EIO};

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
/// - `CLOCK_REALTIME`  → `sysinfo.unix_time` (seconds since Unix epoch)
/// - `CLOCK_MONOTONIC` → `sysinfo.uptime_secs` (seconds since boot)
///
/// `tv_nsec` is always 0 (Phase 1: no sub-second precision).
///
/// Returns 0 on success, -1 and sets errno on failure.
#[no_mangle]
pub unsafe extern "C" fn clock_gettime(clockid: i32, tp: *mut Timespec) -> i32 {
    if tp.is_null() {
        set_errno(EINVAL);
        return -1;
    }
    match crate::sysinfo() {
        Ok(info) => {
            let sec = match clockid {
                CLOCK_REALTIME => info.unix_time as i64,
                CLOCK_MONOTONIC => info.uptime_secs as i64,
                _ => {
                    set_errno(EINVAL);
                    return -1;
                }
            };
            (*tp).tv_sec = sec;
            (*tp).tv_nsec = 0;
            0
        }
        Err(_) => {
            set_errno(EIO);
            -1
        }
    }
}
