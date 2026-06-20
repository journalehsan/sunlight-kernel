//! File descriptor helpers and C ABI exports.

use crate::errno::{set_errno, EINVAL};
use crate::{fstat as libc_fstat, lseek as libc_lseek, Fd, Stat};

/// Reposition the read/write offset of an open file descriptor.
#[no_mangle]
pub unsafe extern "C" fn lseek(fd: i32, offset: i64, whence: i32) -> i64 {
    match libc_lseek(Fd(fd as u32), offset, whence as u64) {
        Ok(pos) => pos as i64,
        Err(e) => {
            crate::errno::set_from_errno(e);
            -1
        }
    }
}

/// Get file status by open file descriptor.
#[no_mangle]
pub unsafe extern "C" fn fstat(fd: i32, out: *mut Stat) -> i32 {
    if out.is_null() {
        set_errno(EINVAL);
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

/// Test whether an open file descriptor refers to a terminal.
/// Phase 1: fds 0 / 1 / 2 (stdin / stdout / stderr) are console-connected and
/// report as terminals.  All other fds return 0 (not a terminal).
#[no_mangle]
pub unsafe extern "C" fn isatty(fd: i32) -> i32 {
    if fd == 0 || fd == 1 || fd == 2 {
        1
    } else {
        set_errno(EINVAL);
        0
    }
}
