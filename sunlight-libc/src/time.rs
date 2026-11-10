//! Minimal POSIX clock support for SunlightOS.
//!
//! The production ABI exposes `clock_gettime` and relative `nanosleep`:
//! - `CLOCK_REALTIME` is UTC Unix time from the boot RTC epoch, with one-second
//!   precision.  It is for calendar timestamps and certificate validation.
//! - `CLOCK_MONOTONIC` is the kernel's canonical BSP timekeeper, measured from
//!   boot and quantized to its 100 Hz tick.  It is for intervals and deadlines.
//! - `nanosleep` uses that monotonic source and Rust std's native futex wait
//!   transport. Spurious wakes are retried; signal interruption is unavailable.
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
#[cfg_attr(
    all(not(test), any(target_os = "none", target_os = "sunlightos")),
    no_mangle
)]
pub unsafe extern "C" fn clock_gettime(clockid: i32, tp: *mut Timespec) -> i32 {
    unsafe { clock_gettime_with(clockid, tp, read_clock_from_syscall) }
}

fn nanoseconds(value: Timespec) -> Result<u64, ClockReadError> {
    if value.tv_sec < 0 || !(0..NANOS_PER_SECOND as i64).contains(&value.tv_nsec) {
        return Err(ClockReadError::Invalid);
    }
    (value.tv_sec as u64)
        .checked_mul(NANOS_PER_SECOND)
        .and_then(|n| n.checked_add(value.tv_nsec as u64))
        .filter(|&n| n != u64::MAX) // native futex's indefinite-wait sentinel
        .ok_or(ClockReadError::Invalid)
}

fn sleep_with<R, W>(req: Timespec, mut read: R, mut wait: W) -> Result<(), ClockReadError>
where
    R: FnMut(i32) -> Result<KernelTimespec, crate::sys::Errno>,
    W: FnMut(u64) -> Result<(), crate::sys::Errno>,
{
    let duration = nanoseconds(req)?;
    if duration == 0 {
        return Ok(());
    }
    let mut now = nanoseconds(read_clock_with(CLOCK_MONOTONIC, &mut read)?)?;
    let deadline = now
        .checked_add(duration)
        .filter(|&n| n != u64::MAX)
        .ok_or(ClockReadError::Invalid)?;
    while now < deadline {
        wait(deadline).map_err(ClockReadError::Syscall)?;
        now = nanoseconds(read_clock_with(CLOCK_MONOTONIC, &mut read)?)?;
    }
    Ok(())
}

unsafe fn nanosleep_with<R, W>(req: *const Timespec, read: R, wait: W) -> i32
where
    R: FnMut(i32) -> Result<KernelTimespec, crate::sys::Errno>,
    W: FnMut(u64) -> Result<(), crate::sys::Errno>,
{
    if req.is_null() {
        set_errno(EFAULT);
        return -1;
    }
    // The C contract requires valid readable storage. Copy before waiting;
    // there are no references to caller memory across syscall boundaries.
    let request = unsafe { core::ptr::read(req) };
    match sleep_with(request, read, wait) {
        Ok(()) => 0,
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

/// Relative monotonic sleep, with no signal interruption in the native ABI.
/// `rem` is never accessed: it is only meaningful for an interrupted sleep.
/// Conversion/deadline overflow fails EINVAL rather than wrapping or becoming
/// an indefinite wait. Success leaves errno and request/remainder unchanged.
#[cfg_attr(
    all(not(test), any(target_os = "none", target_os = "sunlightos")),
    no_mangle
)]
pub unsafe extern "C" fn nanosleep(req: *const Timespec, _rem: *mut Timespec) -> i32 {
    let word = core::sync::atomic::AtomicU32::new(0);
    unsafe {
        nanosleep_with(req, read_clock_from_syscall, |deadline| {
            let result = crate::sys::native_futex(word.as_ptr(), 0, 0, deadline);
            match check(result)? {
                0..=2 => Ok(()), // wake, compare mismatch, or timeout: recheck clock
                _ => Err(crate::sys::Errno::Failed),
            }
        })
    }
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

    #[test]
    fn lp64_timespec_layout() {
        assert_eq!(core::mem::size_of::<Timespec>(), 16);
        assert_eq!(core::mem::align_of::<Timespec>(), 8);
        assert_eq!(core::mem::offset_of!(Timespec, tv_sec), 0);
        assert_eq!(core::mem::offset_of!(Timespec, tv_nsec), 8);
        assert_eq!(core::mem::size_of::<KernelTimespec>(), 16);
    }

    #[test]
    fn nanosecond_conversion_boundaries() {
        for (sec, nsec) in [
            (-1, 0),
            (0, -1),
            (0, 1_000_000_000),
            (i64::MAX, 0),
            (18_446_744_073, 709_551_615),
            (18_446_744_073, 709_551_616),
        ] {
            assert_eq!(
                nanoseconds(Timespec {
                    tv_sec: sec,
                    tv_nsec: nsec
                }),
                Err(ClockReadError::Invalid)
            );
        }
        for (sec, nsec, expected) in [
            (0, 0, 0),
            (0, 999_999_999, 999_999_999),
            (1, 1, 1_000_000_001),
            (18_446_744_073, 709_551_614, u64::MAX - 1),
        ] {
            assert_eq!(
                nanoseconds(Timespec {
                    tv_sec: sec,
                    tv_nsec: nsec
                }),
                Ok(expected)
            );
        }
    }

    #[test]
    fn zero_sleep_and_bad_requests_do_not_access_backend() {
        for request in [
            Timespec {
                tv_sec: 0,
                tv_nsec: 0,
            },
            Timespec {
                tv_sec: -1,
                tv_nsec: 0,
            },
            Timespec {
                tv_sec: 0,
                tv_nsec: -1,
            },
            Timespec {
                tv_sec: 0,
                tv_nsec: 1_000_000_000,
            },
            Timespec {
                tv_sec: i64::MAX,
                tv_nsec: 0,
            },
        ] {
            crate::errno::set_errno(123);
            let result = unsafe {
                nanosleep_with(
                    &request,
                    |_| panic!("unexpected clock"),
                    |_| panic!("unexpected wait"),
                )
            };
            assert_eq!(
                result,
                if request.tv_sec == 0 && request.tv_nsec == 0 {
                    0
                } else {
                    -1
                }
            );
            assert_eq!(
                crate::errno::get_errno(),
                if result == 0 { 123 } else { EINVAL }
            );
        }
        assert_eq!(
            unsafe { nanosleep_with(core::ptr::null(), |_| panic!("clock"), |_| panic!("wait")) },
            -1
        );
        assert_eq!(crate::errno::get_errno(), EFAULT);
    }

    #[test]
    fn sleep_retries_early_wakes_with_original_deadline() {
        let mut reads = 0;
        let mut waits = 0;
        crate::errno::set_errno(123);
        let request = Timespec {
            tv_sec: 0,
            tv_nsec: 25_000_000,
        };
        let rc = unsafe {
            nanosleep_with(
                &request,
                |id| {
                    assert_eq!(id, CLOCK_MONOTONIC);
                    let times = [100_000_000, 100_000_000, 110_000_000, 130_000_000];
                    let nsec = times[reads];
                    reads += 1;
                    sample(0, nsec)
                },
                |deadline| {
                    assert_eq!(deadline, 125_000_000);
                    waits += 1;
                    Ok(())
                },
            )
        };
        assert_eq!(rc, 0);
        assert_eq!((reads, waits), (4, 3));
        assert_eq!(crate::errno::get_errno(), 123);
    }

    #[test]
    fn sleep_deadline_overflow_and_indefinite_sentinel_are_rejected() {
        for nsec in [709_551_613, 709_551_614] {
            let request = Timespec {
                tv_sec: 0,
                tv_nsec: 2,
            };
            let rc = unsafe {
                nanosleep_with(
                    &request,
                    |_| sample(18_446_744_073, nsec),
                    |_| panic!("overflow reached wait"),
                )
            };
            assert_eq!(rc, -1);
            assert_eq!(crate::errno::get_errno(), EINVAL);
        }
        let mut reads = 0;
        assert_eq!(
            sleep_with(
                Timespec {
                    tv_sec: 0,
                    tv_nsec: 1
                },
                |_| {
                    reads += 1;
                    sample(
                        18_446_744_073,
                        if reads == 1 { 709_551_613 } else { 709_551_614 },
                    )
                },
                |deadline| {
                    assert_eq!(deadline, u64::MAX - 1);
                    Ok(())
                }
            ),
            Ok(())
        );
    }

    #[test]
    fn sleep_backend_failures_set_errno() {
        let request = Timespec {
            tv_sec: 0,
            tv_nsec: 1,
        };
        assert_eq!(
            unsafe {
                nanosleep_with(
                    &request,
                    |_| Err(Errno::Again),
                    |_| panic!("clock failure reached wait"),
                )
            },
            -1
        );
        assert_eq!(crate::errno::get_errno(), crate::errno::EAGAIN);
        assert_eq!(
            unsafe { nanosleep_with(&request, |_| sample(0, 0), |_| Err(Errno::Failed)) },
            -1
        );
        assert_eq!(crate::errno::get_errno(), crate::errno::EIO);
        for raw in [(i64::MAX as u64 + 1, 0), (0, NANOS_PER_SECOND)] {
            assert_eq!(
                unsafe {
                    nanosleep_with(
                        &request,
                        |_| sample(raw.0, raw.1),
                        |_| panic!("bad clock reached wait"),
                    )
                },
                -1
            );
            assert_eq!(crate::errno::get_errno(), EINVAL);
        }
    }

    #[test]
    fn clock_range_endpoints_and_failures_preserve_output() {
        let sentinel = Timespec {
            tv_sec: -7,
            tv_nsec: -9,
        };
        let mut out = sentinel;
        for raw in [(i64::MAX as u64 + 1, 0), (0, NANOS_PER_SECOND)] {
            assert_eq!(
                unsafe { clock_gettime_with(0, &mut out, |_| sample(raw.0, raw.1)) },
                -1
            );
            assert_eq!(out, sentinel);
            assert_eq!(crate::errno::get_errno(), EINVAL);
        }
        assert_eq!(
            unsafe { clock_gettime_with(0, &mut out, |_| Err(Errno::Again)) },
            -1
        );
        assert_eq!(out, sentinel);
        assert_eq!(crate::errno::get_errno(), crate::errno::EAGAIN);
        assert_eq!(
            unsafe { clock_gettime_with(0, &mut out, |_| sample(i64::MAX as u64, 999_999_999)) },
            0
        );
        assert_eq!(
            out,
            Timespec {
                tv_sec: i64::MAX,
                tv_nsec: 999_999_999
            }
        );
    }
}
