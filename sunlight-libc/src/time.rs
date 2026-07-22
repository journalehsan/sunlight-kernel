//! Minimal POSIX clock support for SunlightOS.
//!
//! The production ABI intentionally exposes only `clock_gettime`:
//! - `CLOCK_REALTIME` is UTC Unix time from the boot RTC epoch, with one-second
//!   precision.  It is for calendar timestamps and certificate validation.
//! - `CLOCK_MONOTONIC` is the kernel's canonical BSP timekeeper, measured from
//!   boot and quantized to its 100 Hz tick.  It is for intervals and deadlines.
//!
//! Keep the kernel result in a private temporary until all ABI-range checks
//! succeed.  That prevents malformed kernel/service data from partially
//! initializing caller storage.

use core::convert::TryFrom;

use crate::errno::{set_errno, EFAULT, EINVAL};
use crate::sys::{check, syscall2, SYS_CLOCK_GETTIME};

/// POSIX clock IDs supported in Phase 1.
pub const CLOCK_REALTIME: i32 = 0;
pub const CLOCK_MONOTONIC: i32 = 1;

/// POSIX `struct timespec` — seconds + nanoseconds.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timespec {
    pub tv_sec: i64,
    pub tv_nsec: i64,
}

const NANOS_PER_SECOND: u64 = 1_000_000_000;

/// Kernel wire layout.  The syscall fills this private temporary, not caller
/// memory; its unsigned fields match the current kernel protocol exactly.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct KernelTimespec {
    tv_sec: u64,
    tv_nsec: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ClockReadError {
    Invalid,
    Syscall(crate::sys::Errno),
}

#[inline]
fn supported_clock(clockid: i32) -> bool {
    matches!(clockid, CLOCK_REALTIME | CLOCK_MONOTONIC)
}

/// Read and validate a clock value without exposing any result to caller
/// storage.  Keeping this independent of the syscall instruction gives the
/// host proof harness a deterministic, non-exported test seam.
fn read_clock_with<F>(clockid: i32, read: F) -> Result<Timespec, ClockReadError>
where
    F: FnOnce(i32) -> Result<KernelTimespec, crate::sys::Errno>,
{
    if !supported_clock(clockid) {
        return Err(ClockReadError::Invalid);
    }

    let raw = read(clockid).map_err(ClockReadError::Syscall)?;
    let tv_sec = i64::try_from(raw.tv_sec).map_err(|_| ClockReadError::Invalid)?;
    if raw.tv_nsec >= NANOS_PER_SECOND {
        return Err(ClockReadError::Invalid);
    }

    Ok(Timespec {
        tv_sec,
        tv_nsec: raw.tv_nsec as i64,
    })
}

fn read_clock_from_syscall(clockid: i32) -> Result<KernelTimespec, crate::sys::Errno> {
    let mut raw = KernelTimespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    let ret = unsafe {
        syscall2(
            SYS_CLOCK_GETTIME,
            clockid as u64,
            (&mut raw as *mut KernelTimespec) as u64,
        )
    };
    check(ret)?;
    Ok(raw)
}

/// Implement the C output/error contract around an injectable private clock
/// source.  This is the sole host-test seam; production passes the syscall
/// reader below and never allocates or stores global clock state.
unsafe fn clock_gettime_with<F>(clockid: i32, tp: *mut Timespec, read: F) -> i32
where
    F: FnOnce(i32) -> Result<KernelTimespec, crate::sys::Errno>,
{
    if tp.is_null() {
        set_errno(EFAULT);
        return -1;
    }

    match read_clock_with(clockid, read) {
        Ok(value) => {
            // SAFETY: `tp` was checked for null and the C ABI requires a valid,
            // writable Timespec pointer on success.  One write publishes only a
            // fully validated value.
            unsafe { core::ptr::write(tp, value) };
            0
        }
        Err(ClockReadError::Invalid) => {
            set_errno(EINVAL);
            -1
        }
        Err(ClockReadError::Syscall(e)) => {
            crate::errno::set_from_errno(e);
            -1
        }
    }
}

/// Return the current time in `tp` for the given `clockid`.
///
/// Supported clocks:
/// - `CLOCK_REALTIME`  -> UTC seconds since the Unix epoch (nanoseconds are 0)
/// - `CLOCK_MONOTONIC` -> time since boot, never derived from wall clock
///
/// Unsupported IDs and malformed kernel timestamps fail with `EINVAL`; a null
/// output pointer fails locally with `EFAULT`.  Success deliberately leaves
/// `errno` unchanged.
#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn clock_gettime(clockid: i32, tp: *mut Timespec) -> i32 {
    unsafe { clock_gettime_with(clockid, tp, read_clock_from_syscall) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sys::Errno;

    fn sample(sec: u64, nsec: u64) -> Result<KernelTimespec, Errno> {
        Ok(KernelTimespec {
            tv_sec: sec,
            tv_nsec: nsec,
        })
    }

    #[test]
    fn supported_clock_ids_are_forwarded_and_normalized() {
        for id in [CLOCK_REALTIME, CLOCK_MONOTONIC] {
            let got = read_clock_with(id, |seen| {
                assert_eq!(seen, id);
                sample(42, 999_999_999)
            })
            .unwrap();
            assert_eq!(
                got,
                Timespec {
                    tv_sec: 42,
                    tv_nsec: 999_999_999,
                }
            );
        }
    }

    #[test]
    fn invalid_ids_do_not_read_the_kernel_clock() {
        for id in [-1, 2, i32::MIN, i32::MAX] {
            assert_eq!(
                read_clock_with(id, |_| panic!("invalid ID reached source")),
                Err(ClockReadError::Invalid)
            );
        }
    }

    #[test]
    fn rejects_unrepresentable_or_unnormalized_kernel_values() {
        assert_eq!(
            read_clock_with(CLOCK_REALTIME, |_| sample(i64::MAX as u64 + 1, 0)),
            Err(ClockReadError::Invalid)
        );
        assert_eq!(
            read_clock_with(CLOCK_MONOTONIC, |_| sample(0, NANOS_PER_SECOND)),
            Err(ClockReadError::Invalid)
        );
    }

    #[test]
    fn syscall_errors_remain_distinct_from_bad_timestamps() {
        assert_eq!(
            read_clock_with(CLOCK_REALTIME, |_| Err(Errno::Again)),
            Err(ClockReadError::Syscall(Errno::Again))
        );
    }

    #[test]
    fn abi_errors_leave_output_unchanged_and_success_preserves_errno() {
        let sentinel = Timespec {
            tv_sec: -7,
            tv_nsec: -9,
        };
        let mut out = sentinel;

        crate::errno::set_errno(123);
        let ok = unsafe { clock_gettime_with(CLOCK_MONOTONIC, &mut out, |_| sample(9, 10)) };
        assert_eq!(ok, 0);
        assert_eq!(
            out,
            Timespec {
                tv_sec: 9,
                tv_nsec: 10
            }
        );
        assert_eq!(crate::errno::get_errno(), 123);

        out = sentinel;
        let bad_id =
            unsafe { clock_gettime_with(2, &mut out, |_| panic!("bad ID reached source")) };
        assert_eq!(bad_id, -1);
        assert_eq!(out, sentinel);
        assert_eq!(crate::errno::get_errno(), EINVAL);

        let null = unsafe {
            clock_gettime_with(CLOCK_REALTIME, core::ptr::null_mut(), |_| {
                panic!("null output reached source")
            })
        };
        assert_eq!(null, -1);
        assert_eq!(crate::errno::get_errno(), EFAULT);
    }
}
