//! Executes the canonical stdio source with Linux descriptor adapters only.
//! This verifies C ABI/stdio behavior, not SunlightOS kernel execution.
#![feature(c_variadic)]
#![allow(dead_code)]
#[path = "../../../sunlight-libc/src/stdio.rs"]
pub mod stdio;
#[path = "../../../sunlight-libc/src/temp.rs"]
pub mod temp;
pub use sys::Errno;
pub const MAX_PATH: usize = 4096;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fd(pub u32);
pub mod sys {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Errno {
        Again,
        Inval,
        TooBig,
        NoEntry,
        Access,
        BadFd,
        IsDir,
        NotDir,
        BrokenPipe,
        NoSpace,
        NoMemory,
        Failed,
    }
    pub const MAX_IO_COUNT: usize = isize::MAX as usize;
}
pub const O_RDONLY: u64 = 0;
pub const O_WRONLY: u64 = 1;
pub const O_RDWR: u64 = 2;
pub const O_CREAT: u64 = 0x40;
pub const O_EXCL: u64 = 0x80;
pub const O_TRUNC: u64 = 0x200;
pub const O_APPEND: u64 = 0x400;
pub const SEEK_SET: i32 = 0;
pub const SEEK_CUR: i32 = 1;
pub mod errno {
    pub const ENOENT: i32 = 2;
    pub const EIO: i32 = 5;
    pub const E2BIG: i32 = 7;
    pub const EBADF: i32 = 9;
    pub const EAGAIN: i32 = 11;
    pub const ENOMEM: i32 = 12;
    pub const EINVAL: i32 = 22;
    pub const ENOSYS: i32 = 38;
    unsafe extern "C" {
        fn __errno_location() -> *mut i32;
    }
    pub fn set_errno(n: i32) {
        unsafe {
            *__errno_location() = n;
        }
    }
    pub fn get_errno() -> i32 {
        unsafe { *__errno_location() }
    }
    pub fn set_from_errno(e: super::sys::Errno) {
        use super::sys::Errno::*;
        set_errno(match e {
            Again => 11,
            Inval => 22,
            TooBig => 7,
            NoEntry => 2,
            Access => 13,
            BadFd => 9,
            IsDir => 21,
            NotDir => 20,
            BrokenPipe => 32,
            NoSpace => 28,
            NoMemory => 12,
            Failed => 5,
        });
    }
}
fn last_error() -> sys::Errno {
    use sys::Errno::*;
    match errno::get_errno() {
        11 => Again,
        22 => Inval,
        2 => NoEntry,
        13 => Access,
        9 => BadFd,
        21 => IsDir,
        20 => NotDir,
        32 => BrokenPipe,
        28 => NoSpace,
        12 => NoMemory,
        _ => Failed,
    }
}
mod host {
    unsafe extern "C" {
        pub fn read(fd: i32, ptr: *mut u8, len: usize) -> isize;
        pub fn write(fd: i32, ptr: *const u8, len: usize) -> isize;
        pub fn open(path: *const u8, flags: i32, mode: u32) -> i32;
        pub fn close(fd: i32) -> i32;
        pub fn unlink(path: *const u8) -> i32;
        pub fn getrandom(ptr: *mut u8, len: usize, flags: u32) -> isize;
        pub fn lseek(fd: i32, offset: i64, whence: i32) -> i64;
        pub fn malloc(size: usize) -> *mut u8;
        pub fn free(ptr: *mut core::ffi::c_void);
    }
}
pub mod alloc {
    pub unsafe fn malloc(size: usize) -> *mut u8 {
        #[cfg(test)]
        if super::FAULTS.with(|f| f.borrow().allocation_failure) {
            return core::ptr::null_mut();
        }
        let ptr = super::host::malloc(size);
        #[cfg(test)]
        if !ptr.is_null() {
            super::FAULTS.with(|f| f.borrow_mut().allocations += 1);
        }
        ptr
    }
    pub unsafe fn free(ptr: *mut core::ffi::c_void) {
        super::host::free(ptr);
        #[cfg(test)]
        super::FAULTS.with(|f| {
            let mut f = f.borrow_mut();
            f.frees += 1;
            if let Some(n) = f.free_errno {
                super::errno::set_errno(n);
            }
        });
    }
}
pub fn yield_now() {
    std::thread::yield_now();
}
pub fn open_with_flags_mode(path: &[u8], flags: u64, mode: u16) -> Result<Fd, sys::Errno> {
    #[cfg(test)]
    {
        FAULTS.with(|f| f.borrow_mut().opens.push((path.to_vec(), flags, mode)));
        if let Some(error) = FAULTS.with(|f| f.borrow().open_failure) {
            return Err(error);
        }
    }
    let path = std::ffi::CString::new(path).map_err(|_| sys::Errno::Inval)?;
    let fd = unsafe { host::open(path.as_ptr().cast(), flags as i32, mode as u32) };
    if fd < 0 {
        Err(last_error())
    } else {
        #[cfg(test)]
        FAULTS.with(|f| f.borrow_mut().created.push(fd as u32));
        Ok(Fd(fd as u32))
    }
}
pub fn getrandom(bytes: &mut [u8], flags: u32) -> isize {
    #[cfg(test)]
    if let Some(result) = FAULTS.with(|f| f.borrow_mut().random.pop_front()) {
        return match result {
            Ok(value) => {
                bytes.copy_from_slice(&value);
                bytes.len() as isize
            }
            Err(()) => -1,
        };
    }
    unsafe { host::getrandom(bytes.as_mut_ptr(), bytes.len(), flags) }
}
pub fn unlink(path: &[u8]) -> Result<(), sys::Errno> {
    #[cfg(test)]
    {
        FAULTS.with(|f| f.borrow_mut().unlinks.push(path.to_vec()));
        if let Some(error) = FAULTS.with(|f| f.borrow().unlink_failure) {
            return Err(error);
        }
    }
    let path = std::ffi::CString::new(path).map_err(|_| sys::Errno::Inval)?;
    if unsafe { host::unlink(path.as_ptr().cast()) } < 0 {
        Err(last_error())
    } else {
        Ok(())
    }
}
pub fn close(fd: Fd) -> Result<(), sys::Errno> {
    let ret = unsafe { host::close(fd.0 as i32) };
    #[cfg(test)]
    if FAULTS.with(|f| {
        let mut f = f.borrow_mut();
        f.closed.push(fd.0);
        f.close_failure
    }) {
        errno::set_errno(errno::EIO);
        return Err(sys::Errno::Failed);
    }
    if ret < 0 {
        Err(last_error())
    } else {
        Ok(())
    }
}
pub fn lseek(fd: Fd, offset: i64, whence: i32) -> Result<u64, sys::Errno> {
    #[cfg(test)]
    if FAULTS.with(|f| f.borrow().seek_failure) {
        return Err(sys::Errno::Inval);
    }
    let ret = unsafe { host::lseek(fd.0 as i32, offset, whence) };
    if ret < 0 {
        Err(last_error())
    } else {
        Ok(ret as u64)
    }
}
pub fn read(fd: Fd, buf: &mut [u8]) -> Result<usize, sys::Errno> {
    #[cfg(test)]
    let cap = FAULTS
        .with(|f| f.borrow_mut().reads.pop_front())
        .unwrap_or(Ok(usize::MAX))?;
    #[cfg(not(test))]
    let cap = usize::MAX;
    let ret = unsafe { host::read(fd.0 as i32, buf.as_mut_ptr(), buf.len().min(cap)) };
    if ret < 0 {
        Err(last_error())
    } else {
        Ok(ret as usize)
    }
}
pub fn write(fd: Fd, buf: &[u8]) -> Result<usize, sys::Errno> {
    #[cfg(test)]
    let cap = FAULTS
        .with(|f| f.borrow_mut().writes.pop_front())
        .unwrap_or(Ok(usize::MAX))?;
    #[cfg(not(test))]
    let cap = usize::MAX;
    let ret = unsafe { host::write(fd.0 as i32, buf.as_ptr(), buf.len().min(cap)) };
    if ret < 0 {
        Err(last_error())
    } else {
        Ok(ret as usize)
    }
}
#[cfg(test)]
#[derive(Default)]
struct Faults {
    reads: std::collections::VecDeque<Result<usize, sys::Errno>>,
    writes: std::collections::VecDeque<Result<usize, sys::Errno>>,
    close_failure: bool,
    seek_failure: bool,
    allocation_failure: bool,
    allocations: usize,
    frees: usize,
    free_errno: Option<i32>,
    random: std::collections::VecDeque<Result<[u8; 6], ()>>,
    open_failure: Option<sys::Errno>,
    unlink_failure: Option<sys::Errno>,
    opens: Vec<(Vec<u8>, u64, u16)>,
    created: Vec<u32>,
    unlinks: Vec<Vec<u8>>,
    closed: Vec<u32>,
}
#[cfg(test)]
thread_local! { static FAULTS: std::cell::RefCell<Faults> = Default::default(); }
#[cfg(test)]
#[path = "stdio-tests.rs"]
mod tests;
