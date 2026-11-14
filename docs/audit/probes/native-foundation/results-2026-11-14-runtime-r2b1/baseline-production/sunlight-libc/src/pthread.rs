//! Minimal POSIX threads over the native thread and futex ABI.
//!
//! `pthread_create` allocates a control block, stores the entry point and
//! argument, and wraps `crate::thread::spawn`. The control block doubles as the
//! join channel: the worker publishes `EXITED` and wakes the block's futex
//! word, and `pthread_join` waits on that word before polling the kernel
//! `NativeThreadReaped` (156) query and releasing the stack/TLS mappings.
//!
//! Limits: no cancellation, no scheduling attributes, no robust mutexes, no
//! condition variables, and no process-wide initial-thread join. Detached
//! control blocks are reclaimed lazily by the next `pthread_create` or
//! `pthread_join` because a thread cannot unmap its own running stack.

use core::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, AtomicUsize, Ordering};

use crate::sys::{self, Errno};

const EAGAIN: i32 = 11;
const ENOMEM: i32 = 12;
const EBUSY: i32 = 16;
const EINVAL: i32 = 22;
const EDEADLK: i32 = 35;

const PTHREAD_CREATE_JOINABLE: i32 = 0;
const PTHREAD_CREATE_DETACHED: i32 = 1;

const RUNNING: u32 = 0;
const EXITED: u32 = 1;
const CLAIMED: u32 = 2;

const SYS_NATIVE_THREAD_REAPED: u64 = 156;
const DETACHED_CAPACITY: usize = 64;

/// Default thread attributes. Only the detach state is meaningful here.
#[repr(C)]
pub struct PthreadAttr {
    detachstate: i32,
}

/// Futex-backed mutex: 0 = unlocked, 1 = locked.
#[repr(C)]
pub struct PthreadMutex {
    lock: AtomicU32,
}

type StartRoutine = extern "C" fn(*mut u8) -> *mut u8;

#[repr(C)]
struct Control {
    /// Join futex word: RUNNING, EXITED, or CLAIMED.
    state: AtomicU32,
    /// 0 = joinable, 1 = detached, 2 = detached and listed for reclamation.
    detached: AtomicU32,
    retval: AtomicPtr<u8>,
    start: Option<StartRoutine>,
    arg: *mut u8,
    tid: AtomicUsize,
    stack_base: AtomicUsize,
    stack_size: AtomicUsize,
    tls_base: AtomicUsize,
    tls_size: AtomicUsize,
}

/// Non-null opaque identity for the initial thread.
static MAIN_THREAD: u8 = 0;

static DETACHED_LOCK: AtomicBool = AtomicBool::new(false);
static mut DETACHED: [usize; DETACHED_CAPACITY] = [0; DETACHED_CAPACITY];

fn main_thread() -> *mut u8 {
    &MAIN_THREAD as *const u8 as *mut u8
}

unsafe fn state_word(control: *mut Control) -> *const u32 {
    core::ptr::addr_of!((*control).state) as *const u32
}

unsafe fn lock_detached() {
    while DETACHED_LOCK
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        crate::yield_now();
    }
}

unsafe fn unlock_detached() {
    DETACHED_LOCK.store(false, Ordering::Release);
}

unsafe fn enqueue_detached(control: *mut Control) {
    lock_detached();
    let slots = core::ptr::addr_of_mut!(DETACHED) as *mut usize;
    for index in 0..DETACHED_CAPACITY {
        let slot = slots.add(index);
        if *slot == 0 {
            *slot = control as usize;
            unlock_detached();
            return;
        }
    }
    // Capacity exhaustion leaves the control allocated until process exit.
    unlock_detached();
}

unsafe fn release_control(control: *mut Control) {
    let stack_base = (*control).stack_base.load(Ordering::Acquire) as *mut u8;
    let stack_size = (*control).stack_size.load(Ordering::Acquire);
    let tls_base = (*control).tls_base.load(Ordering::Acquire) as *mut u8;
    let tls_size = (*control).tls_size.load(Ordering::Acquire);
    if !stack_base.is_null() {
        let _ = crate::mman::munmap(stack_base, stack_size);
    }
    if !tls_base.is_null() {
        let _ = crate::mman::munmap(tls_base, tls_size);
    }
    crate::alloc::free(control as *mut u8);
}

/// Wait until the kernel has reaped `tid`. Returns false on an invalid query.
unsafe fn wait_reaped(tid: usize) -> bool {
    loop {
        match sys::syscall1(SYS_NATIVE_THREAD_REAPED, tid as u64) {
            1 => return true,
            0 => crate::yield_now(),
            _ => return false,
        }
    }
}

unsafe fn reap_detached() {
    lock_detached();
    let slots = core::ptr::addr_of_mut!(DETACHED) as *mut usize;
    for index in 0..DETACHED_CAPACITY {
        let slot = slots.add(index);
        let address = *slot;
        if address == 0 {
            continue;
        }
        let control = address as *mut Control;
        if (*control).state.load(Ordering::Acquire) != EXITED {
            continue;
        }
        let tid = (*control).tid.load(Ordering::Acquire);
        if sys::syscall1(SYS_NATIVE_THREAD_REAPED, tid as u64) != 1 {
            continue;
        }
        *slot = 0;
        release_control(control);
    }
    unlock_detached();
}

unsafe fn finish(control: *mut Control, retval: *mut u8) {
    (*control).retval.store(retval, Ordering::Release);
    (*control).state.store(EXITED, Ordering::Release);
    sys::native_futex(state_word(control), 1, 1, 0);
    if (*control)
        .detached
        .compare_exchange(1, 2, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
    {
        enqueue_detached(control);
    }
}

/// Publish the detached state after the control block is fully populated.
unsafe fn mark_detached(control: *mut Control) -> bool {
    if (*control)
        .detached
        .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return false;
    }
    // If the worker already exited before observing the detach, list it now.
    if (*control).state.load(Ordering::Acquire) == EXITED
        && (*control)
            .detached
            .compare_exchange(1, 2, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    {
        enqueue_detached(control);
    }
    true
}

extern "C" fn pthread_trampoline(raw: *mut u8) -> *mut u8 {
    unsafe {
        let control = raw as *mut Control;
        crate::tls::set_pthread_control(raw);
        let start = (*control).start.expect("pthread start routine");
        let retval = start((*control).arg);
        finish(control, retval);
        retval
    }
}

/// Create a joinable or detached thread running `start(arg)`.
///
/// # Safety
/// `thread` and `attr`, when non-null, must be valid writable/readable storage.
#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub unsafe extern "C" fn pthread_create(
    thread: *mut *mut u8,
    attr: *const PthreadAttr,
    start: Option<StartRoutine>,
    arg: *mut u8,
) -> i32 {
    if thread.is_null() || start.is_none() {
        return EINVAL;
    }
    reap_detached();
    let control = crate::alloc::malloc(core::mem::size_of::<Control>()) as *mut Control;
    if control.is_null() {
        return ENOMEM;
    }
    core::ptr::write_bytes(control as *mut u8, 0, core::mem::size_of::<Control>());
    (*control).start = start;
    (*control).arg = arg;
    let detached = !attr.is_null() && (*attr).detachstate == PTHREAD_CREATE_DETACHED;

    match crate::thread::spawn(pthread_trampoline, control as *mut u8) {
        Ok(handle) => {
            (*control).tid.store(handle.tid as usize, Ordering::Release);
            (*control)
                .stack_base
                .store(handle.stack_base as usize, Ordering::Release);
            (*control)
                .stack_size
                .store(handle.stack_size, Ordering::Release);
            (*control)
                .tls_base
                .store(handle.tls_base as usize, Ordering::Release);
            (*control)
                .tls_size
                .store(handle.tls_size, Ordering::Release);
            // Publish detach only after the handle bookkeeping is visible, so
            // a sweep cannot free a partially initialized control block.
            if detached {
                mark_detached(control);
            }
            thread.write(control as *mut u8);
            0
        }
        Err(error) => {
            crate::alloc::free(control as *mut u8);
            match error {
                Errno::Again => EAGAIN,
                _ => ENOMEM,
            }
        }
    }
}

/// Wait for `thread` to exit and reclaim its stack and TLS mapping.
///
/// # Safety
/// `thread` must be a live joinable handle; `retval`, when non-null, must be
/// writable.
#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub unsafe extern "C" fn pthread_join(thread: *mut u8, retval: *mut *mut u8) -> i32 {
    if thread.is_null() || thread == main_thread() {
        return EINVAL;
    }
    if thread == pthread_self() {
        return EDEADLK;
    }
    let control = thread as *mut Control;
    if (*control).detached.load(Ordering::Acquire) != 0 {
        return EINVAL;
    }
    loop {
        if (*control).state.load(Ordering::Acquire) != RUNNING {
            break;
        }
        let result = sys::native_futex(state_word(control), 0, 0, u64::MAX);
        if result > 2 {
            return EINVAL;
        }
    }
    if (*control)
        .state
        .compare_exchange(EXITED, CLAIMED, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return EINVAL;
    }
    let tid = (*control).tid.load(Ordering::Acquire);
    if !wait_reaped(tid) {
        return EINVAL;
    }
    core::sync::atomic::fence(Ordering::Acquire);
    if !retval.is_null() {
        retval.write((*control).retval.load(Ordering::Relaxed));
    }
    release_control(control);
    0
}

/// Terminate the calling thread, publishing `retval` to a waiting join.
#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub extern "C" fn pthread_exit(retval: *mut u8) -> ! {
    unsafe {
        let control = crate::tls::pthread_control() as *mut Control;
        if !control.is_null() {
            finish(control, retval);
        }
        // The initial thread has no join control block; exit it alone. The
        // kernel keeps the process alive while borrower threads remain.
        sys::syscall1(sys::SYS_PROCESS_EXIT, 0);
        loop {
            core::hint::spin_loop();
        }
    }
}

/// Return the calling thread's opaque handle.
#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub extern "C" fn pthread_self() -> *mut u8 {
    unsafe {
        let control = crate::tls::pthread_control();
        if control.is_null() {
            main_thread()
        } else {
            control
        }
    }
}

#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub extern "C" fn pthread_equal(left: *mut u8, right: *mut u8) -> i32 {
    i32::from(left == right)
}

/// Mark `thread` detached. Its resources are reclaimed after it exits.
///
/// # Safety
/// `thread` must be a live handle created by `pthread_create`.
#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub unsafe extern "C" fn pthread_detach(thread: *mut u8) -> i32 {
    if thread.is_null() || thread == main_thread() {
        return EINVAL;
    }
    let control = thread as *mut Control;
    if !mark_detached(control) {
        return EINVAL;
    }
    0
}

#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub unsafe extern "C" fn pthread_attr_init(attr: *mut PthreadAttr) -> i32 {
    if attr.is_null() {
        return EINVAL;
    }
    (*attr).detachstate = PTHREAD_CREATE_JOINABLE;
    0
}

#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub extern "C" fn pthread_attr_destroy(_attr: *mut PthreadAttr) -> i32 {
    0
}

#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub unsafe extern "C" fn pthread_attr_setdetachstate(attr: *mut PthreadAttr, state: i32) -> i32 {
    if attr.is_null() || (state != PTHREAD_CREATE_JOINABLE && state != PTHREAD_CREATE_DETACHED) {
        return EINVAL;
    }
    (*attr).detachstate = state;
    0
}

#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub unsafe extern "C" fn pthread_attr_getdetachstate(
    attr: *const PthreadAttr,
    state: *mut i32,
) -> i32 {
    if attr.is_null() || state.is_null() {
        return EINVAL;
    }
    state.write((*attr).detachstate);
    0
}

#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub unsafe extern "C" fn pthread_mutex_init(mutex: *mut PthreadMutex, _attr: *const u8) -> i32 {
    if mutex.is_null() {
        return EINVAL;
    }
    (*mutex).lock.store(0, Ordering::Relaxed);
    0
}

#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub extern "C" fn pthread_mutex_destroy(_mutex: *mut PthreadMutex) -> i32 {
    0
}

#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub unsafe extern "C" fn pthread_mutex_lock(mutex: *mut PthreadMutex) -> i32 {
    if mutex.is_null() {
        return EINVAL;
    }
    let word = core::ptr::addr_of!((*mutex).lock) as *const u32;
    loop {
        if (*mutex)
            .lock
            .compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            return 0;
        }
        let result = sys::native_futex(word, 0, 1, u64::MAX);
        if result > 2 {
            return EINVAL;
        }
    }
}

#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub unsafe extern "C" fn pthread_mutex_trylock(mutex: *mut PthreadMutex) -> i32 {
    if mutex.is_null() {
        return EINVAL;
    }
    if (*mutex)
        .lock
        .compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed)
        .is_ok()
    {
        0
    } else {
        EBUSY
    }
}

#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub unsafe extern "C" fn pthread_mutex_unlock(mutex: *mut PthreadMutex) -> i32 {
    if mutex.is_null() {
        return EINVAL;
    }
    (*mutex).lock.store(0, Ordering::Release);
    sys::native_futex(core::ptr::addr_of!((*mutex).lock) as *const u32, 1, 1, 0);
    0
}
