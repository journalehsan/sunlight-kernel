//! Stable, allocation-cycle-free ABI used by the pinned native Rust std overlay.
use crate::{sys, Fd};
use core::alloc::{GlobalAlloc, Layout};
use core::sync::atomic::{AtomicBool, Ordering};

core::arch::global_asm!(
    ".global _start",
    "_start:",
    // The stack is the common exec ABI. The legacy service-spawn path
    // overwrites entry registers with shell/uid/gid values after loading it.
    "mov rdi, [rsp]",
    "lea rsi, [rsp + 8]",
    "lea rdx, [rsi + rdi * 8 + 8]",
    "and rsp, -16",
    "call sl_std_start",
    "ud2",
);

unsafe extern "C" {
    fn main(argc: isize, argv: *const *const u8) -> i32;
}

#[no_mangle]
unsafe extern "C" fn sl_std_start(
    argc: isize,
    argv: *const *const u8,
    envp: *const *const u8,
) -> ! {
    sl_std_init_tls();
    crate::env::init(envp);
    let status = main(argc, argv);
    sl_std_exit(status)
}

#[no_mangle]
pub unsafe extern "C" fn sl_std_init_tls() {
    if crate::tls::init_tls().is_err() {
        crate::exit(101);
    }
}

#[no_mangle]
pub extern "C" fn sl_std_errno() -> i32 {
    crate::errno::get_errno()
}

#[no_mangle]
pub unsafe extern "C" fn sl_std_env(index: usize) -> *const u8 {
    crate::env::entry(index)
}
#[no_mangle]
pub unsafe extern "C" fn sl_std_getcwd(ptr: *mut u8) -> isize {
    count(crate::getcwd(&mut *(ptr as *mut [u8; 256])))
}
#[no_mangle]
pub unsafe extern "C" fn sl_std_chdir(ptr: *const u8, len: usize) -> i32 {
    crate::chdir(core::slice::from_raw_parts(ptr, len)).map_or_else(|e| error(e) as i32, |_| 0)
}

#[no_mangle]
pub unsafe extern "C" fn sl_std_alloc(size: usize, align: usize) -> *mut u8 {
    match Layout::from_size_align(size.max(1), align) {
        Ok(layout) => crate::alloc::SunlightAlloc.alloc(layout),
        Err(_) => core::ptr::null_mut(),
    }
}
#[no_mangle]
pub unsafe extern "C" fn sl_std_free(ptr: *mut u8, size: usize, align: usize) {
    if let Ok(layout) = Layout::from_size_align(size.max(1), align) {
        crate::alloc::SunlightAlloc.dealloc(ptr, layout);
    }
}
fn error(error: sys::Errno) -> isize {
    -(match error {
        sys::Errno::Again => 11,
        sys::Errno::Inval => 22,
        sys::Errno::TooBig => 22,
        sys::Errno::NoEntry => 2,
        sys::Errno::Access => 13,
        sys::Errno::BadFd => 9,
        sys::Errno::IsDir => 21,
        sys::Errno::NotDir => 20,
        sys::Errno::Failed => 5,
    })
}
fn count(result: Result<usize, sys::Errno>) -> isize {
    result.map_or_else(error, |n| n as isize)
}

#[no_mangle]
pub unsafe extern "C" fn sl_std_entropy(ptr: *mut u8, len: usize) -> i32 {
    // Hash seeds/bootstrap avoid capability IPC or allocation under std locks.
    // Use only the conditioned approved-source kernel entropy, never a fallback.
    if sys::syscall0(sys::SYS_SECURE_ENTROPY_READY) != 1 {
        return -1;
    }
    for chunk in core::slice::from_raw_parts_mut(ptr, len).chunks_mut(8) {
        let bytes = sys::syscall0(sys::SYS_GET_ENTROPY).to_ne_bytes();
        chunk.copy_from_slice(&bytes[..chunk.len()]);
    }
    0
}
#[no_mangle]
pub unsafe extern "C" fn sl_std_clock(clock: u32, out: *mut u64) -> i32 {
    let mut value = crate::time::Timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    if crate::time::clock_gettime(clock as i32, &mut value) != 0 {
        return -1;
    }
    out.write(value.tv_sec as u64);
    out.add(1).write(value.tv_nsec as u64);
    0
}
#[no_mangle]
pub unsafe extern "C" fn sl_std_read(fd: u32, ptr: *mut u8, len: usize) -> isize {
    count(crate::read(
        Fd(fd),
        core::slice::from_raw_parts_mut(ptr, len),
    ))
}
#[no_mangle]
pub unsafe extern "C" fn sl_std_write(fd: u32, ptr: *const u8, len: usize) -> isize {
    // Keep stderr available before terminal/session routing exists.
    if fd == 2 {
        return count(crate::write_all(Fd(fd), core::slice::from_raw_parts(ptr, len)).map(|_| len));
    }
    count(crate::write(Fd(fd), core::slice::from_raw_parts(ptr, len)))
}
#[no_mangle]
pub unsafe extern "C" fn sl_std_open(ptr: *const u8, len: usize, flags: u64) -> isize {
    crate::open_with_flags_mode(core::slice::from_raw_parts(ptr, len), flags, 0o666)
        .map_or_else(error, |fd| fd.0 as isize)
}
#[no_mangle]
pub extern "C" fn sl_std_close(fd: u32) {
    let _ = crate::close(Fd(fd));
}
#[no_mangle]
pub unsafe extern "C" fn sl_std_seek(fd: u32, offset: i64, whence: u32, out: *mut u64) -> i32 {
    match crate::lseek(Fd(fd), offset, whence as i32) {
        Ok(value) => {
            out.write(value);
            0
        }
        Err(e) => error(e) as i32,
    }
}
#[no_mangle]
pub unsafe extern "C" fn sl_std_stat(fd: u32, out: *mut u64) -> i32 {
    match crate::fstat(Fd(fd)) {
        Ok(value) => {
            out.write(value.size);
            out.add(1).write(value.mode as u64);
            out.add(2).write(value.file_type as u64);
            0
        }
        Err(e) => error(e) as i32,
    }
}
#[no_mangle]
pub extern "C" fn sl_std_yield() {
    crate::yield_now();
}
#[no_mangle]
pub unsafe extern "C" fn sl_std_futex(ptr: *const u32, op: u32, value: u32, deadline: u64) -> i32 {
    let result = sys::syscall4(155, ptr as u64, op as u64, value as u64, deadline);
    if op == 0 && result == 0 && deadline != u64::MAX {
        let mut now = [0u64; 2];
        if sl_std_clock(1, now.as_mut_ptr()) == 0
            && now[0].saturating_mul(1_000_000_000).saturating_add(now[1]) >= deadline
        {
            return 2;
        }
    }
    result as i32
}

// Handles use System's same reclaiming allocator, below std, avoiding cycles.
#[no_mangle]
pub unsafe extern "C" fn sl_std_spawn(
    entry: extern "C" fn(*mut u8) -> *mut u8,
    arg: *mut u8,
    stack: usize,
) -> *mut u8 {
    reap_detached();
    let ptr = sl_std_alloc(
        core::mem::size_of::<crate::thread::JoinHandle>(),
        core::mem::align_of::<crate::thread::JoinHandle>(),
    )
    .cast::<crate::thread::JoinHandle>();
    if ptr.is_null() {
        return core::ptr::null_mut();
    }
    match crate::thread::spawn_with_stack(entry, arg, stack, true) {
        Ok(handle) => {
            ptr.write(handle);
            ptr.cast()
        }
        Err(_) => {
            sl_std_free(
                ptr.cast(),
                core::mem::size_of::<crate::thread::JoinHandle>(),
                core::mem::align_of::<crate::thread::JoinHandle>(),
            );
            core::ptr::null_mut()
        }
    }
}
unsafe fn release(ptr: *mut crate::thread::JoinHandle) {
    let handle = &*ptr;
    if crate::mman::munmap(handle.stack_base, handle.stack_size).is_err()
        || crate::mman::munmap(handle.tls_base, 4096).is_err()
    {
        sl_std_exit(101);
    }
    sl_std_free(
        ptr.cast(),
        core::mem::size_of::<crate::thread::JoinHandle>(),
        core::mem::align_of::<crate::thread::JoinHandle>(),
    );
}
#[no_mangle]
pub unsafe extern "C" fn sl_std_join(ptr: *mut u8) {
    let handle = ptr.cast::<crate::thread::JoinHandle>();
    loop {
        match sys::syscall1(156, (*handle).tid as u64) {
            1 => break,
            0 => crate::yield_now(),
            _ => sl_std_exit(101),
        }
    }
    core::sync::atomic::fence(Ordering::Acquire);
    release(handle);
    reap_detached();
}
// Detach never releases a live stack. Quiescent handles are swept at the next
// spawn/join/detach; final process teardown releases the complete VM.
static DETACHED_LOCK: AtomicBool = AtomicBool::new(false);
static mut DETACHED: [usize; 256] = [0; 256];
unsafe fn reap_detached() {
    while DETACHED_LOCK
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        crate::yield_now();
    }
    for index in 0..256 {
        let address = DETACHED[index];
        if address != 0 {
            let handle = address as *mut crate::thread::JoinHandle;
            if sys::syscall1(156, (*handle).tid as u64) == 1 {
                DETACHED[index] = 0;
                release(handle);
            }
        }
    }
    DETACHED_LOCK.store(false, Ordering::Release);
}
#[no_mangle]
pub unsafe extern "C" fn sl_std_detach(ptr: *mut u8) {
    reap_detached();
    while DETACHED_LOCK
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        crate::yield_now();
    }
    for index in 0..256 {
        if DETACHED[index] == 0 {
            DETACHED[index] = ptr as usize;
            DETACHED_LOCK.store(false, Ordering::Release);
            return;
        }
    }
    DETACHED_LOCK.store(false, Ordering::Release);
    sl_std_exit(101)
}
#[no_mangle]
pub unsafe extern "C" fn sl_std_exit(code: i32) -> ! {
    sys::syscall1(157, code as u64);
    loop {
        core::hint::spin_loop();
    }
}
