use crate::ffi::CStr;
use crate::io;
use crate::num::NonZero;
use crate::sys::sunlightos::*;
use crate::thread::ThreadInit;
use crate::time::Duration;
pub const DEFAULT_MIN_STACK_SIZE: usize = 2 * 1024 * 1024;
pub struct Thread(*mut u8);
unsafe impl Send for Thread {}
unsafe impl Sync for Thread {}
impl Thread {
    pub unsafe fn new(stack: usize, init: Box<ThreadInit>) -> io::Result<Thread> {
        extern "C" fn entry(ptr: *mut u8) -> *mut u8 {
            unsafe {
                let init = Box::from_raw(ptr.cast::<ThreadInit>());
                let main = init.init();
                main();
                crate::sys::thread_local::key::run_dtors();
            }
            crate::ptr::null_mut()
        }
        let ptr = Box::into_raw(init);
        let handle = unsafe { sl_std_spawn(entry, ptr.cast(), stack.max(DEFAULT_MIN_STACK_SIZE)) };
        if handle.is_null() {
            unsafe { drop(Box::from_raw(ptr)) };
            Err(io::Error::from_raw_os_error(12))
        } else {
            Ok(Thread(handle))
        }
    }
    pub fn join(self) {
        unsafe { sl_std_join(self.0) };
        crate::mem::forget(self);
    }
}
impl Drop for Thread {
    fn drop(&mut self) {
        unsafe { sl_std_detach(self.0) };
    }
}
pub fn yield_now() {
    unsafe { sl_std_yield() };
}
pub fn set_name(_name: &CStr) {}
pub fn current_os_id() -> Option<u64> {
    None
}
pub fn available_parallelism() -> io::Result<NonZero<usize>> {
    Err(io::Error::UNKNOWN_THREAD_COUNT)
}
pub fn sleep(duration: Duration) {
    // A scheduler wake may be spurious. std sleep must still wait for the full
    // interval, unlike the raw futex primitive.
    let started = crate::time::Instant::now();
    let word = crate::sync::atomic::AtomicU32::new(0);
    while let Some(remaining) = duration.checked_sub(started.elapsed()) {
        if remaining.is_zero() {
            break;
        }
        crate::sys::futex::futex_wait(&word, 0, Some(remaining));
    }
}
