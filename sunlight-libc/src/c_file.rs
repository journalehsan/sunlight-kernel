//! C entry points for existing native descriptor operations. No Linux backend.
use crate::{errno, sys};
use core::ffi::c_int;

fn identity(selector: u64) -> c_int {
    match sys::check(unsafe { sys::syscall1(sys::SYS_NATIVE_PROCESS_IDENTITY, selector) }) {
        Ok(value) if value <= c_int::MAX as u64 => value as c_int,
        Ok(_) => {
            errno::set_errno(errno::EOVERFLOW);
            -1
        }
        Err(error) => {
            errno::set_from_errno(error);
            -1
        }
    }
}

#[no_mangle]
pub extern "C" fn getpid() -> c_int {
    identity(0)
}

#[no_mangle]
pub extern "C" fn getppid() -> c_int {
    identity(1)
}

#[no_mangle]
pub extern "C" fn gettid() -> c_int {
    identity(2)
}

pub(crate) fn control(fd: c_int, command: c_int, argument: c_int) -> c_int {
    if fd < 0 {
        errno::set_errno(errno::EBADF);
        return -1;
    }
    match sys::check(unsafe {
        sys::syscall3(49, fd as u64, command as u64, argument as u32 as u64)
    }) {
        Ok(n) if n <= c_int::MAX as u64 => n as c_int,
        Ok(_) => {
            errno::set_errno(errno::EOVERFLOW);
            -1
        }
        Err(e) => {
            errno::set_from_errno(e);
            -1
        }
    }
}
#[no_mangle]
pub unsafe extern "C" fn fcntl(fd: c_int, command: c_int, mut arguments: ...) -> c_int {
    let arg = match command {
        1 | 3 => 0,
        0 | 2 | 4 | 1030 => arguments.next_arg::<c_int>(),
        _ => {
            errno::set_errno(errno::EINVAL);
            return -1;
        }
    };
    control(fd, command, arg)
}
#[no_mangle]
pub unsafe extern "C" fn open(path: *const u8, flags: c_int, mut arguments: ...) -> c_int {
    if path.is_null() {
        errno::set_errno(errno::EFAULT);
        return -1;
    }
    // Reject unknown flags rather than let the native backend ignore them.
    const FLAGS: c_int = 3 | 0x40 | 0x80 | 0x200 | 0x400 | 0x800 | 0x20000 | 0x80000;
    if flags & !FLAGS != 0 || flags & 3 == 3 {
        errno::set_errno(errno::EINVAL);
        return -1;
    }
    let mode = if flags & 0x40 != 0 {
        arguments.next_arg::<u32>()
    } else {
        0
    };
    let mut len = 0;
    while *path.add(len) != 0 {
        len += 1;
    }
    if mode & !0o7777 != 0 {
        errno::set_errno(errno::EINVAL);
        return -1;
    }
    match crate::open_with_flags_mode(
        core::slice::from_raw_parts(path, len),
        flags as u64,
        mode as u16,
    ) {
        Ok(fd) => fd.0 as c_int,
        Err(e) => {
            errno::set_from_errno(e);
            -1
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn unlink(path: *const u8) -> c_int {
    if path.is_null() {
        errno::set_errno(errno::EFAULT);
        return -1;
    }
    let mut len = 0;
    while *path.add(len) != 0 {
        len += 1;
    }
    match crate::unlink(core::slice::from_raw_parts(path, len)) {
        Ok(()) => 0,
        Err(e) => {
            errno::set_from_errno(e);
            -1
        }
    }
}

/// Native realpath: caller-owned output must have PATH_MAX (256) bytes.
/// NULL output allocates exactly the result plus NUL through libc malloc.
/// Canonical output is a raced observation, never an authorization token.
#[no_mangle]
pub unsafe extern "C" fn realpath(path: *const u8, output: *mut u8) -> *mut u8 {
    if path.is_null() {
        errno::set_errno(errno::EINVAL);
        return core::ptr::null_mut();
    }
    let mut len = 0;
    while len < crate::MAX_PATH && *path.add(len) != 0 {
        len += 1;
    }
    if len == crate::MAX_PATH {
        errno::set_errno(36); // ENAMETOOLONG
        return core::ptr::null_mut();
    }
    if len == 0 {
        errno::set_errno(errno::ENOENT);
        return core::ptr::null_mut();
    }
    let mut canonical = [0u8; crate::MAX_PATH];
    let n = match crate::resolve_path(core::slice::from_raw_parts(path, len), &mut canonical) {
        Ok(n) => n,
        Err(sys::Errno::TooBig) => {
            errno::set_errno(36);
            return core::ptr::null_mut();
        }
        Err(e) => {
            errno::set_from_errno(e);
            return core::ptr::null_mut();
        }
    };
    let destination = if output.is_null() {
        crate::alloc::malloc(n + 1)
    } else {
        output
    };
    if destination.is_null() {
        return destination;
    }
    core::ptr::copy_nonoverlapping(canonical.as_ptr(), destination, n + 1);
    destination
}

