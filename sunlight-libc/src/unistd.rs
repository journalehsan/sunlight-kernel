//! First C unistd slice. Preserve single-transfer progress and native errors.
use crate::{errno, sys, Fd};
use core::ffi::{c_int, c_void};

#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub unsafe extern "C" fn write(fd: c_int, ptr: *const c_void, len: usize) -> isize {
    write_with(fd, ptr, len, |fd, ptr, len| crate::write_raw(fd, ptr, len))
}

fn write_with(
    fd: c_int,
    ptr: *const c_void,
    len: usize,
    transfer: impl FnOnce(Fd, *const u8, usize) -> Result<usize, sys::Errno>,
) -> isize {
    let invalid = if fd < 0 {
        Some(errno::EBADF)
    } else if len > sys::MAX_IO_COUNT {
        Some(errno::EINVAL)
    } else if ptr.is_null() && len != 0 {
        Some(errno::EFAULT)
    } else {
        None
    };
    if let Some(error) = invalid {
        errno::set_errno(error);
        return -1;
    }
    match transfer(Fd(fd as u32), ptr.cast(), len) {
        Ok(count) if count <= len => count as isize,
        Ok(_) => {
            errno::set_errno(errno::EIO);
            -1
        }
        Err(error) => {
            errno::set_from_errno(error);
            -1
        }
    }
}

/// Close consumes the descriptor even if backend cleanup reports an error.
#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub extern "C" fn close(fd: c_int) -> c_int {
    if fd < 0 {
        errno::set_errno(errno::EBADF);
        return -1;
    }
    match crate::close(Fd(fd as u32)) {
        Ok(()) => 0,
        Err(error) => {
            errno::set_from_errno(error);
            -1
        }
    }
}

#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub unsafe extern "C" fn read(fd: c_int, ptr: *mut c_void, len: usize) -> isize {
    write_with(fd, ptr, len, |fd, ptr, len| {
        sys::check_io_count(
            sys::syscall3(sys::SYS_READ, fd.0 as u64, ptr as u64, len as u64),
            len,
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_short_and_zero_counts_are_single_transfers() {
        for count in [0, 3, 8] {
            errno::set_errno(77);
            let result = write_with(2, b"12345678".as_ptr().cast(), 8, |fd, ptr, len| {
                assert_eq!(fd.0, 2);
                assert_eq!(ptr, b"12345678".as_ptr());
                assert_eq!(len, 8);
                Ok(count)
            });
            assert_eq!(result, count as isize);
            assert_eq!(errno::get_errno(), 77);
        }
        assert_eq!(
            write_with(1, core::ptr::null(), 0, |_, p, n| {
                assert!(p.is_null());
                assert_eq!(n, 0);
                Ok(0)
            }),
            0
        );
    }

    #[test]
    fn invalid_arguments_never_reach_the_runtime() {
        for (fd, len, expected) in [
            (-1, 1, errno::EBADF),
            (1, 1, errno::EFAULT),
            (1, usize::MAX, errno::EINVAL),
        ] {
            assert_eq!(
                write_with(fd, core::ptr::null(), len, |_, _, _| {
                    panic!("invalid arguments reached runtime")
                }),
                -1
            );
            assert_eq!(errno::get_errno(), expected);
        }
    }

    #[test]
    fn failure_mapping_and_impossible_counts_are_not_progress() {
        use sys::Errno::*;
        for (error, expected) in [
            (BadFd, errno::EBADF),
            (Access, errno::EACCES),
            (Again, errno::EAGAIN),
            (BrokenPipe, errno::EPIPE),
            (NoSpace, errno::ENOSPC),
            (Failed, errno::EIO),
        ] {
            assert_eq!(
                write_with(1, b"x".as_ptr().cast(), 1, |_, _, _| Err(error)),
                -1
            );
            assert_eq!(errno::get_errno(), expected);
        }
        assert_eq!(write_with(1, b"x".as_ptr().cast(), 1, |_, _, _| Ok(2)), -1);
        assert_eq!(errno::get_errno(), errno::EIO);
    }
}
