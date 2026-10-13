//! SunlightOS native ABI. No Linux personality or POSIX emulation is used.
pub mod futex;
use crate::io;

unsafe extern "C" {
    pub fn sl_std_alloc(size: usize, align: usize) -> *mut u8;
    pub fn sl_std_free(ptr: *mut u8, size: usize, align: usize);
    pub fn sl_std_entropy(ptr: *mut u8, len: usize) -> i32;
    pub fn sl_std_clock(clock: u32, out: *mut u64) -> i32;
    pub fn sl_std_read(fd: u32, ptr: *mut u8, len: usize) -> isize;
    pub fn sl_std_write(fd: u32, ptr: *const u8, len: usize) -> isize;
    pub fn sl_std_open(ptr: *const u8, len: usize, flags: u64) -> isize;
    pub fn sl_std_close(fd: u32);
    pub fn sl_std_seek(fd: u32, offset: i64, whence: u32, out: *mut u64) -> i32;
    pub fn sl_std_stat(fd: u32, out: *mut u64) -> i32;
    pub fn sl_std_yield();
    pub fn sl_std_futex(ptr: *const u32, op: u32, value: u32, deadline: u64) -> i32;
    pub fn sl_std_spawn(
        entry: extern "C" fn(*mut u8) -> *mut u8,
        arg: *mut u8,
        stack: usize,
    ) -> *mut u8;
    pub fn sl_std_join(handle: *mut u8);
    pub fn sl_std_detach(handle: *mut u8);
    pub fn sl_std_exit(code: i32) -> !;
    pub fn sl_std_init_tls();
    pub fn sl_std_errno() -> i32;
    pub fn sl_std_env(index: usize) -> *const u8;
    pub fn sl_std_getcwd(ptr: *mut u8) -> isize;
    pub fn sl_std_chdir(ptr: *const u8, len: usize) -> i32;
}

pub unsafe fn init(argc: isize, argv: *const *const u8, _sigpipe: u8) {
    unsafe { crate::sys::args::init(argc, argv) };
}
pub unsafe fn cleanup() {
    unsafe { crate::sys::thread_local::key::run_dtors() };
}
pub fn unsupported<T>() -> io::Result<T> {
    Err(unsupported_err())
}
pub fn unsupported_err() -> io::Error {
    io::Error::UNSUPPORTED_PLATFORM
}
pub fn abort_internal() -> ! {
    unsafe { sl_std_exit(101) }
}
pub fn cvt(result: isize) -> io::Result<usize> {
    if result < 0 {
        Err(io::Error::from_raw_os_error(-result as i32))
    } else {
        Ok(result as usize)
    }
}
