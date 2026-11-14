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
//! control blocks are reclaimed lazily by the next `pthread_create` because a
//! thread cannot unmap its own running stack. Public handles are nonwrapping
//! serial tokens resolved under the reclamation lock, never allocation pointers.

use core::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, AtomicUsize, Ordering};

use crate::sys::{self, Errno};
use crate::tls::{OnceFrame, Specific};

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
    // Immutable before spawn; public identity never aliases a recycled allocation.
    handle: usize,
    // Registry fields are accessed only while DETACHED_LOCK is held.
    next: usize,
    joining: bool,
    published: AtomicBool,
}

static DETACHED_LOCK: AtomicBool = AtomicBool::new(false);
static mut DETACHED: [usize; DETACHED_CAPACITY] = [0; DETACHED_CAPACITY];
// Every live/queued Control occupies registry admission until release_control.
// This bounds all Control allocations, not just entries visible in DETACHED.
static mut CONTROLS: usize = 0;
static mut CONTROL_COUNT: usize = 0;
static mut NEXT_HANDLE: usize = 2;

fn main_thread() -> *mut u8 {
    1usize as *mut u8
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

// The following registry helpers require DETACHED_LOCK. Tokens are opaque
// pointer-sized ABI values; they are never dereferenced, even for invalid input.
unsafe fn find_control(handle: *mut u8) -> *mut Control {
    let mut address = CONTROLS;
    while address != 0 {
        let control = address as *mut Control;
        if (*control).handle == handle as usize {
            return control;
        }
        address = (*control).next;
    }
    core::ptr::null_mut()
}

unsafe fn unlink_control(control: *mut Control) {
    let mut link = core::ptr::addr_of_mut!(CONTROLS);
    while *link != 0 {
        let current = *link as *mut Control;
        if current == control {
            *link = (*current).next;
            CONTROL_COUNT -= 1;
            return;
        }
        link = core::ptr::addr_of_mut!((*current).next);
    }
    panic!("pthread control missing from registry");
}

unsafe fn enqueue_detached(control: *mut Control) {
    let slots = core::ptr::addr_of_mut!(DETACHED) as *mut usize;
    for index in 0..DETACHED_CAPACITY {
        let slot = slots.add(index);
        if *slot == 0 {
            *slot = control as usize;
            return;
        }
    }
    // Admission counts both live and queued controls. Each is listed at most
    // once under this same lock, so queue exhaustion is an invariant failure.
    panic!("pthread detached admission invariant");
}

unsafe fn release_control(control: *mut Control) -> bool {
    let stack_base = (*control).stack_base.load(Ordering::Acquire) as *mut u8;
    let stack_size = (*control).stack_size.load(Ordering::Acquire);
    let tls_base = (*control).tls_base.load(Ordering::Acquire) as *mut u8;
    let tls_size = (*control).tls_size.load(Ordering::Acquire);
    if !stack_base.is_null() {
        if crate::mman::munmap(stack_base, stack_size).is_err() {
            return false;
        }
        (*control).stack_base.store(0, Ordering::Release);
    }
    if !tls_base.is_null() {
        if crate::mman::munmap(tls_base, tls_size).is_err() {
            return false;
        }
        (*control).tls_base.store(0, Ordering::Release);
    }
    unlink_control(control);
    crate::alloc::free(control as *mut u8);
    true
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
        if release_control(control) {
            *slot = 0;
        }
    }
    unlock_detached();
}

unsafe fn finish(control: *mut Control, retval: *mut u8) {
    cleanup_current();
    (*control).retval.store(retval, Ordering::Release);
    (*control).state.store(EXITED, Ordering::Release);
    sys::native_futex(state_word(control), 1, 1, 0);
    lock_detached();
    queue_if_exited(control);
    unlock_detached();
}

/// Publish detach with DETACHED_LOCK held, protecting the observer from sweep.
unsafe fn mark_detached(control: *mut Control) -> bool {
    if (*control)
        .detached
        .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return false;
    }
    queue_if_exited(control);
    true
}

// Called with DETACHED_LOCK. A worker may self-detach/exit before spawn returns;
// only the creator can publish the mappings and TID needed by the reclaimer.
unsafe fn queue_if_exited(control: *mut Control) {
    if (*control).published.load(Ordering::Acquire)
        && (*control).state.load(Ordering::Acquire) == EXITED
        && (*control)
            .detached
            .compare_exchange(1, 2, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    {
        enqueue_detached(control);
    }
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
    // Reserve metadata capacity and a unique token before allocation/spawn.
    lock_detached();
    if CONTROL_COUNT == DETACHED_CAPACITY || NEXT_HANDLE == usize::MAX {
        unlock_detached();
        return EAGAIN;
    }
    CONTROL_COUNT += 1;
    let handle_token = NEXT_HANDLE;
    NEXT_HANDLE += 1;
    unlock_detached();
    let control = crate::alloc::malloc(core::mem::size_of::<Control>()) as *mut Control;
    if control.is_null() {
        lock_detached();
        CONTROL_COUNT -= 1;
        unlock_detached();
        return ENOMEM;
    }
    core::ptr::write_bytes(control as *mut u8, 0, core::mem::size_of::<Control>());
    (*control).start = start;
    (*control).arg = arg;
    (*control).handle = handle_token;
    lock_detached();
    (*control).next = CONTROLS;
    CONTROLS = control as usize;
    unlock_detached();
    let detached = !attr.is_null() && (*attr).detachstate == PTHREAD_CREATE_DETACHED;
    if detached {
        (*control).detached.store(1, Ordering::Release);
    }

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
            lock_detached();
            (*control).published.store(true, Ordering::Release);
            queue_if_exited(control);
            thread.write(handle_token as *mut u8);
            unlock_detached();
            0
        }
        Err(error) => {
            lock_detached();
            unlink_control(control);
            unlock_detached();
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
    lock_detached();
    let control = find_control(thread);
    if control.is_null() || (*control).joining || (*control).detached.load(Ordering::Acquire) != 0 {
        unlock_detached();
        return EINVAL;
    }
    // Exclusive join reservation keeps the control live across blocking waits.
    (*control).joining = true;
    unlock_detached();
    while !(*control).published.load(Ordering::Acquire) {
        crate::yield_now();
    }
    loop {
        if (*control).state.load(Ordering::Acquire) != RUNNING {
            break;
        }
        let result = sys::native_futex(state_word(control), 0, 0, u64::MAX);
        if result > 2 {
            lock_detached();
            (*control).joining = false;
            unlock_detached();
            return EINVAL;
        }
    }
    if (*control)
        .state
        .compare_exchange(EXITED, CLAIMED, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        lock_detached();
        (*control).joining = false;
        unlock_detached();
        return EINVAL;
    }
    let tid = (*control).tid.load(Ordering::Acquire);
    if !wait_reaped(tid) {
        lock_detached();
        (*control).state.store(EXITED, Ordering::Release);
        (*control).joining = false;
        unlock_detached();
        return EINVAL;
    }
    core::sync::atomic::fence(Ordering::Acquire);
    if !retval.is_null() {
        retval.write((*control).retval.load(Ordering::Relaxed));
    }
    lock_detached();
    if !release_control(control) {
        (*control).state.store(EXITED, Ordering::Release);
        (*control).joining = false;
        unlock_detached();
        return EINVAL;
    }
    unlock_detached();
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
        cleanup_current();
        // Architectural limitation: the initial thread owns the address space.
        // Its native exit also terminates borrowers; thread-only initial-thread
        // pthread_exit is not supported by the existing kernel lifecycle.
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
            (*(control as *mut Control)).handle as *mut u8
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
    lock_detached();
    let control = find_control(thread);
    let valid = !control.is_null() && !(*control).joining && mark_detached(control);
    unlock_detached();
    if valid {
        0
    } else {
        EINVAL
    }
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

// Registry objects never move or get freed. All registry access is serialized;
// values belong exclusively to the current TCB. Generations never wrap: slots
// are retired at exhaustion rather than allowing stale handles to alias.
type Destructor = unsafe extern "C" fn(*mut u8);
#[derive(Clone, Copy)]
struct Key {
    generation: usize,
    live: bool,
    destructor: Option<Destructor>,
}
static KEY_LOCK: PthreadMutex = PthreadMutex {
    lock: AtomicU32::new(0),
};
static mut KEYS: [Key; 128] = [Key {
    generation: 0,
    live: false,
    destructor: None,
}; 128];
unsafe fn lock_keys() {
    pthread_mutex_lock(core::ptr::addr_of!(KEY_LOCK) as *mut PthreadMutex);
}
unsafe fn unlock_keys() {
    pthread_mutex_unlock(core::ptr::addr_of!(KEY_LOCK) as *mut PthreadMutex);
}
unsafe fn key_slot(key: usize) -> *mut Key {
    (core::ptr::addr_of_mut!(KEYS) as *mut Key).add(key & 127)
}
unsafe fn valid_key(key: usize) -> bool {
    let slot = key_slot(key);
    (*slot).live && (*slot).generation == key >> 7
}
#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub unsafe extern "C" fn pthread_key_create(
    out: *mut usize,
    destructor: Option<Destructor>,
) -> i32 {
    if out.is_null() {
        return EINVAL;
    }
    lock_keys();
    for index in 0..128 {
        let slot = key_slot(index);
        if !(*slot).live && (*slot).generation < usize::MAX >> 7 {
            (*slot).generation += 1;
            (*slot).live = true;
            (*slot).destructor = destructor;
            out.write(((*slot).generation << 7) | index);
            unlock_keys();
            return 0;
        }
    }
    unlock_keys();
    EAGAIN
}
#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub unsafe extern "C" fn pthread_key_delete(key: usize) -> i32 {
    lock_keys();
    let result = if valid_key(key) {
        (*key_slot(key)).live = false;
        (*key_slot(key)).destructor = None;
        0
    } else {
        EINVAL
    };
    unlock_keys();
    result
}
#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub unsafe extern "C" fn pthread_getspecific(key: usize) -> *mut u8 {
    lock_keys();
    let value = &(*crate::tls::current_tcb()).pthread_values[key & 127];
    let result = if valid_key(key) && value.key == key {
        value.value
    } else {
        core::ptr::null_mut()
    };
    unlock_keys();
    result
}
#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub unsafe extern "C" fn pthread_setspecific(key: usize, value: *const u8) -> i32 {
    lock_keys();
    let result = if valid_key(key) {
        (*crate::tls::current_tcb()).pthread_values[key & 127] = Specific {
            key,
            value: value as *mut u8,
        };
        0
    } else {
        EINVAL
    };
    unlock_keys();
    result
}

/// Stack-linked initialization frames live until normal return or thread exit.
/// No other thread accesses these frames. Control storage must outlive callers.
#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub unsafe extern "C" fn pthread_once(
    control: *const AtomicU32,
    routine: Option<extern "C" fn()>,
) -> i32 {
    if control.is_null() || routine.is_none() {
        return EINVAL;
    }
    let state = &*control;
    if state.load(Ordering::Acquire) == 2 {
        return 0;
    }
    let tcb = crate::tls::current_tcb();
    loop {
        match state.compare_exchange(0, 1, Ordering::Acquire, Ordering::Acquire) {
            Ok(_) => {
                let mut frame = OnceFrame {
                    control,
                    previous: (*tcb).once_active,
                };
                (*tcb).once_active = &mut frame;
                routine.unwrap()();
                (*tcb).once_active = frame.previous;
                state.store(2, Ordering::Release);
                sys::native_futex(control as *const u32, 1, u32::MAX, 0);
                return 0;
            }
            Err(2) => return 0,
            Err(_) => {
                sys::native_futex(control as *const u32, 0, 1, u64::MAX);
            }
        }
    }
}

unsafe fn cleanup_current() {
    let tcb = crate::tls::current_tcb();
    // Explicit pthread_exit abandons all nested initializers before callbacks.
    while !(*tcb).once_active.is_null() {
        let frame = (*tcb).once_active;
        (*tcb).once_active = (*frame).previous;
        (*(*frame).control).store(0, Ordering::Release);
        sys::native_futex((*frame).control as *const u32, 1, u32::MAX, 0);
    }
    for _ in 0..4 {
        let mut invoked = false;
        for index in 0..128 {
            lock_keys();
            let specific = (*tcb).pthread_values[index];
            (*tcb).pthread_values[index] = Specific::EMPTY;
            let destructor = if !specific.value.is_null() && valid_key(specific.key) {
                (*key_slot(specific.key)).destructor
            } else {
                None
            };
            unlock_keys();
            // Selection is serialized against deletion. A callback already
            // selected may finish after deletion; callback code must stay live.
            if let Some(callback) = destructor {
                invoked = true;
                callback(specific.value);
            }
        }
        if !invoked {
            break;
        }
    }
    (*tcb).pthread_values.fill(Specific::EMPTY);
}
