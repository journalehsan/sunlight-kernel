//! Thread-Local Storage bootstrap for SunlightOS.
//!
//! Call `init_tls()` at the very start of `_start`, before any C-ABI libc
//! calls that may write errno. Allocates one page for the Thread Control Block
//! and points FS_BASE at it so that `fs:0` == `self_ptr` and `fs:8` == `errno`.

use crate::mman::{mmap, MAP_ANONYMOUS, MAP_PRIVATE, PROT_READ, PROT_WRITE};
use crate::sys::{syscall1, SYS_SET_FS_BASE};

/// Thread Control Block layout required by the SysV AMD64 ABI.
/// `fs:0` must be a self-pointer; `fs:8` is our per-thread errno.
#[repr(C)]
pub struct Tcb {
    pub self_ptr: *mut Tcb,
    pub errno: i32,
}

/// Initialize TLS for the current thread. Must be called before any
/// C-ABI libc function that reads or writes errno.
///
/// # Safety
/// Must be called exactly once per thread, before any errno-touching code.
pub unsafe fn init_tls() -> Result<(), ()> {
    let ptr = mmap(
        core::ptr::null_mut(),
        4096,
        PROT_READ | PROT_WRITE,
        MAP_PRIVATE | MAP_ANONYMOUS,
        -1,
        0,
    )
    .map_err(|_| ())?;

    let tcb = ptr as *mut Tcb;
    (*tcb).self_ptr = tcb;
    (*tcb).errno = 0;

    let ret = syscall1(SYS_SET_FS_BASE, tcb as u64);
    if ret != 0 {
        return Err(());
    }

    crate::errno::mark_tls_ready();
    Ok(())
}
