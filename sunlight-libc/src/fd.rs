//! File descriptor helpers and Phase 1 stubs.
//!
//! `lseek` and `fstat` are not yet implemented in SunlightOS VFS (Phase 1).
//! Both return -1 / ENOSYS so callers get a clean unsupported error rather
//! than a crash or silent misbehavior.

use crate::errno::{set_errno, EINVAL, ENOSYS};
use crate::Stat;

// POSIX lseek `whence` constants.
pub const SEEK_SET: i32 = 0;
pub const SEEK_CUR: i32 = 1;
pub const SEEK_END: i32 = 2;

/// Reposition the read/write offset of an open file descriptor.
/// Phase 1: not implemented — returns -1 / ENOSYS.
#[no_mangle]
pub unsafe extern "C" fn lseek(_fd: i32, _offset: i64, _whence: i32) -> i64 {
    set_errno(ENOSYS);
    -1
}

/// Get file status by open file descriptor.
/// Phase 1: not implemented — returns -1 / ENOSYS.
#[no_mangle]
pub unsafe extern "C" fn fstat(_fd: i32, _out: *mut Stat) -> i32 {
    set_errno(ENOSYS);
    -1
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
