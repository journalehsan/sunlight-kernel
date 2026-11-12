//! File descriptor helpers and C ABI exports.

use crate::errno::{set_errno, EBADF, EFAULT, EINVAL, EIO};
use crate::{fstat as libc_fstat, lseek as libc_lseek, Fd, Stat};

/// Reposition the read/write offset of an open file descriptor.
#[cfg_attr(any(target_os = "none", target_os = "sunlightos"), no_mangle)]
pub unsafe extern "C" fn lseek(fd: i32, offset: i64, whence: i32) -> i64 {
    if fd < 0 {
        set_errno(EBADF);
        return -1;
    }
    if !(0..=2).contains(&whence) {
        set_errno(EINVAL);
        return -1;
    }
    match libc_lseek(Fd(fd as u32), offset, whence) {
        Ok(pos) => match i64::try_from(pos) {
            Ok(pos) => pos,
            Err(_) => {
                set_errno(EIO);
                -1
            }
        },
        Err(e) => {
            crate::errno::set_from_errno(e);
            -1
        }
    }
}

/// Get file status by open file descriptor.
#[cfg_attr(any(target_os = "none", target_os = "sunlightos"), no_mangle)]
pub unsafe extern "C" fn fstat(fd: i32, out: *mut Stat) -> i32 {
    if fd < 0 {
        set_errno(EBADF);
        return -1;
    }
    if out.is_null() {
        set_errno(EFAULT);
        return -1;
    }

    match libc_fstat(Fd(fd as u32)) {
        Ok(stat) => {
            unsafe {
                core::ptr::write(out, stat);
            }
            0
        }
        Err(e) => {
            crate::errno::set_from_errno(e);
            -1
        }
    }
}

/// Resolve the kernel terminal handle, including duplicated descriptors.
#[cfg_attr(any(target_os = "none", target_os = "sunlightos"), no_mangle)]
pub unsafe extern "C" fn isatty(fd: i32) -> i32 {
    i32::from(crate::termios::terminal_operation(fd, 0) == 0)
}
