pub use super::common::Args;
use crate::ffi::{CStr, OsStr};
use crate::sync::atomic::{AtomicIsize, AtomicPtr, Ordering};
static ARGC: AtomicIsize = AtomicIsize::new(0);
static ARGV: AtomicPtr<*const u8> = AtomicPtr::new(crate::ptr::null_mut());
pub unsafe fn init(argc: isize, argv: *const *const u8) {
    ARGC.store(argc, Ordering::Relaxed);
    ARGV.store(argv.cast_mut(), Ordering::Release);
}
pub fn args() -> Args {
    let argv = ARGV.load(Ordering::Acquire);
    let argc = if argv.is_null() {
        0
    } else {
        ARGC.load(Ordering::Relaxed)
    };
    let mut args = Vec::new();
    for index in 0..argc {
        let ptr = unsafe { *argv.offset(index) };
        if ptr.is_null() {
            break;
        }
        let bytes = unsafe { CStr::from_ptr(ptr.cast()) }.to_bytes();
        args.push(unsafe { OsStr::from_encoded_bytes_unchecked(bytes) }.to_owned());
    }
    Args::new(args)
}
