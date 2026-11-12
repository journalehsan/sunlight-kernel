//! Native terminal capability boundary and userspace-only speed selectors.
use crate::{errno, sys};

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Termios {
    pub c_iflag: u32,
    pub c_oflag: u32,
    pub c_cflag: u32,
    pub c_lflag: u32,
    pub c_cc: [u8; 2],
    pub reserved: [u8; 2],
    pub c_ispeed: u32,
    pub c_ospeed: u32,
}

fn fail(code: i32) -> i32 {
    errno::set_errno(code);
    -1
}

pub(crate) fn terminal_operation(fd: i32, op: u64) -> i32 {
    if fd < 0 {
        return fail(errno::EBADF);
    }
    let result = unsafe { sys::syscall2(sys::SYS_TERMINAL, fd as u64, op) } as i64;
    if result < 0 {
        fail((-result) as i32)
    } else {
        result as i32
    }
}

// Pointers are intentionally never accessed by unsupported descriptor APIs.
#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub unsafe extern "C" fn tcgetattr(fd: i32, _out: *mut Termios) -> i32 {
    terminal_operation(fd, 1)
}

#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub unsafe extern "C" fn tcsetattr(fd: i32, action: i32, _value: *const Termios) -> i32 {
    if !(0..=2).contains(&action) {
        return fail(errno::EINVAL);
    }
    terminal_operation(fd, 2)
}

#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub unsafe extern "C" fn cfgetispeed(value: *const Termios) -> u32 {
    (*value).c_ispeed
}
#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub unsafe extern "C" fn cfgetospeed(value: *const Termios) -> u32 {
    (*value).c_ospeed
}
#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub unsafe extern "C" fn cfsetispeed(value: *mut Termios, speed: u32) -> i32 {
    if value.is_null() {
        return fail(errno::EFAULT);
    }
    if speed > 4 {
        return fail(errno::EINVAL);
    }
    let speed = if speed == 0 { (*value).c_ospeed } else { speed };
    if speed > 4 {
        return fail(errno::EINVAL);
    }
    (*value).c_ispeed = speed;
    0
}
#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub unsafe extern "C" fn cfsetospeed(value: *mut Termios, speed: u32) -> i32 {
    if value.is_null() {
        return fail(errno::EFAULT);
    }
    if speed > 4 {
        return fail(errno::EINVAL);
    }
    (*value).c_ospeed = speed;
    0
}
#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub extern "C" fn tcdrain(fd: i32) -> i32 {
    terminal_operation(fd, 3)
}
#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub extern "C" fn tcflush(fd: i32, selector: i32) -> i32 {
    if !(0..=2).contains(&selector) {
        return fail(errno::EINVAL);
    }
    terminal_operation(fd, 4)
}
#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub extern "C" fn tcflow(fd: i32, action: i32) -> i32 {
    if !(0..=3).contains(&action) {
        return fail(errno::EINVAL);
    }
    terminal_operation(fd, 5)
}
#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub extern "C" fn tcsendbreak(fd: i32, _duration: i32) -> i32 {
    terminal_operation(fd, 6)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn speed_edits_and_rejection_are_atomic() {
        let mut t = Termios::default();
        unsafe {
            assert_eq!(cfsetospeed(&mut t, 4), 0);
            assert_eq!(cfsetispeed(&mut t, 0), 0);
            assert_eq!(cfgetispeed(&t), 4);
            let before = t;
            assert_eq!(cfsetospeed(&mut t, 5), -1);
            assert_eq!(cfsetispeed(&mut t, u32::MAX), -1);
            assert_eq!(t, before);
            assert_eq!(cfsetispeed(core::ptr::null_mut(), 1), -1);
        }
        assert_eq!(core::mem::size_of::<Termios>(), 28);
        assert_eq!(core::mem::align_of::<Termios>(), 4);
    }
}
