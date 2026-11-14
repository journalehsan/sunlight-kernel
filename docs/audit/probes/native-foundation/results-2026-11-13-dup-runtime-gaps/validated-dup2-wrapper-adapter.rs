#![no_std]
// Test object only. The production rlib/archive supplies both the wrapper and
// the bridge used by C socket/read/write; this object owns no bridge state.
use sunlight_libc::{Fd, Errno};

#[no_mangle]
#[inline(never)]
pub extern "C" fn coverage_replace(old: i32, new: i32) -> i64 {
    match sunlight_libc::dup2(Fd(old as u32), Fd(new as u32)) {
        Ok(fd) => fd.0 as i64,
        Err(Errno::BadFd) => -5,
        Err(_) => -999,
    }
}

#[no_mangle]
pub extern "C" fn coverage_bridge_id(fd: i32) -> u64 {
    sunlight_libc::net_bridge::lookup(fd).map_or(0, |id| id + 1)
}

#[no_mangle]
pub extern "C" fn coverage_generation(fd: i32) -> u64 {
    unsafe { sunlight_libc::sys::syscall6(177, fd as u64, 0, 0, 0, 0, 3) }
}
