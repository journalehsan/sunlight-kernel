//! Process-private condition variables with intrusive, stack-owned wait records.
//!
//! The gate protects the list and its lifetime, never the blocking wait or the
//! caller's mutex reacquisition. Registration precedes releasing that mutex.
//! Each waiter has its own one-shot futex word, avoiding sequence-counter ABA.
//! A signal marks and wakes one unnotified record under the gate. Timeout and
//! removal serialize with notification; a notification winning that serialization
//! returns success. Records remain listed through mutex reacquisition, so destroy
//! cannot succeed while a waiter still uses the condition variable.
//!
//! As with pthread objects generally, storage must remain valid through every
//! concurrent API invocation. Reinitializing a live object is not supported.
use crate::pthread::{pthread_mutex_lock, pthread_mutex_unlock, PthreadMutex};
use crate::sys;
use crate::time::{clock_gettime, Timespec};
use core::ptr;
use core::sync::atomic::{AtomicU32, Ordering};

const EINVAL: i32 = 22;
const EBUSY: i32 = 16;
const ETIMEDOUT: i32 = 110;

#[repr(C)]
pub struct Cond {
    gate: PthreadMutex,
    dead: u32,
    head: *mut Waiter,
    clock: i32,
}
#[repr(C)]
pub struct Attr {
    clock: i32,
}
struct Waiter {
    notified: AtomicU32,
    next: *mut Waiter,
}

fn aligned<T>(p: *const T) -> bool {
    !p.is_null() && (p as usize) % core::mem::align_of::<T>() == 0
}
#[no_mangle]
pub unsafe extern "C" fn pthread_condattr_init(a: *mut Attr) -> i32 {
    if !aligned(a) {
        return EINVAL;
    }
    (*a).clock = 0;
    0
}
#[no_mangle]
pub unsafe extern "C" fn pthread_condattr_destroy(a: *mut Attr) -> i32 {
    if !aligned(a) || !(0..=1).contains(&(*a).clock) {
        return EINVAL;
    }
    (*a).clock = -1;
    0
}
#[no_mangle]
pub unsafe extern "C" fn pthread_condattr_setclock(a: *mut Attr, clock: i32) -> i32 {
    if !aligned(a) || !(0..=1).contains(&(*a).clock) || !(0..=1).contains(&clock) {
        return EINVAL;
    }
    (*a).clock = clock;
    0
}
#[no_mangle]
pub unsafe extern "C" fn pthread_condattr_getclock(a: *const Attr, clock: *mut i32) -> i32 {
    if !aligned(a) || !aligned(clock) || !(0..=1).contains(&(*a).clock) {
        return EINVAL;
    }
    *clock = (*a).clock;
    0
}
#[no_mangle]
pub unsafe extern "C" fn pthread_cond_init(c: *mut Cond, a: *const Attr) -> i32 {
    if !aligned(c) || (!a.is_null() && (!aligned(a) || !(0..=1).contains(&(*a).clock))) {
        return EINVAL;
    }
    crate::pthread::pthread_mutex_init(ptr::addr_of_mut!((*c).gate), ptr::null());
    (*c).dead = 0;
    (*c).head = ptr::null_mut();
    (*c).clock = if a.is_null() { 0 } else { (*a).clock };
    0
}
#[no_mangle]
pub unsafe extern "C" fn pthread_cond_destroy(c: *mut Cond) -> i32 {
    if !aligned(c) {
        return EINVAL;
    }
    let gate = ptr::addr_of_mut!((*c).gate);
    let r = pthread_mutex_lock(gate);
    if r != 0 {
        return r;
    }
    let result = if (*c).dead != 0 {
        EINVAL
    } else if !(*c).head.is_null() {
        EBUSY
    } else {
        (*c).dead = 1;
        0
    };
    pthread_mutex_unlock(gate);
    result
}
unsafe fn notify(c: *mut Cond, all: bool) -> i32 {
    if !aligned(c) {
        return EINVAL;
    }
    let gate = ptr::addr_of_mut!((*c).gate);
    let r = pthread_mutex_lock(gate);
    if r != 0 {
        return r;
    }
    let result = if (*c).dead != 0 {
        EINVAL
    } else {
        let mut node = (*c).head;
        while !node.is_null() {
            if (*node).notified.load(Ordering::Relaxed) == 0 {
                (*node).notified.store(1, Ordering::Release);
                sys::native_futex(ptr::addr_of!((*node).notified).cast(), 1, 1, 0);
                if !all {
                    break;
                }
            }
            node = (*node).next;
        }
        0
    };
    pthread_mutex_unlock(gate);
    result
}
#[no_mangle]
pub unsafe extern "C" fn pthread_cond_signal(c: *mut Cond) -> i32 {
    notify(c, false)
}
#[no_mangle]
pub unsafe extern "C" fn pthread_cond_broadcast(c: *mut Cond) -> i32 {
    notify(c, true)
}

unsafe fn now(clock: i32) -> Result<u64, i32> {
    let mut t = Timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    if clock_gettime(clock, &mut t) != 0 {
        return Err(EINVAL);
    }
    ns(t)
}
fn ns(t: Timespec) -> Result<u64, i32> {
    if t.tv_sec < 0 || !(0..1_000_000_000).contains(&t.tv_nsec) {
        return Err(EINVAL);
    }
    (t.tv_sec as u64)
        .checked_mul(1_000_000_000)
        .and_then(|n| n.checked_add(t.tv_nsec as u64))
        .filter(|n| *n != u64::MAX)
        .ok_or(EINVAL)
}
unsafe fn wait(c: *mut Cond, m: *mut PthreadMutex, absolute: Option<Timespec>) -> i32 {
    if !aligned(c) || !aligned(m) {
        return EINVAL;
    }
    let limit = match absolute {
        Some(t) => match if (0..1_000_000_000).contains(&t.tv_nsec) && t.tv_sec < 0 {
            Ok(0)
        } else {
            ns(t)
        } {
            Ok(n) => Some(n),
            Err(e) => return e,
        },
        None => None,
    };
    let gate = ptr::addr_of_mut!((*c).gate);
    let r = pthread_mutex_lock(gate);
    if r != 0 {
        return r;
    }
    if (*c).dead != 0 {
        pthread_mutex_unlock(gate);
        return EINVAL;
    }
    let clock = (*c).clock;
    let mut node = Waiter {
        notified: AtomicU32::new(0),
        next: (*c).head,
    };
    (*c).head = &mut node;
    // The caller holds m. Any notifier correctly using m cannot pass this
    // registration before we release m. Our private futex compares before park.
    pthread_mutex_unlock(m);
    pthread_mutex_unlock(gate);
    let mut result = 0;
    while node.notified.load(Ordering::Acquire) == 0 {
        let deadline = if let Some(limit) = limit {
            let current = match now(clock) {
                Ok(n) => n,
                Err(e) => {
                    result = e;
                    break;
                }
            };
            if current >= limit {
                result = ETIMEDOUT;
                break;
            }
            // Re-read realtime after each bounded sleep; never freeze a wall
            // clock deadline into an unbounded monotonic wait.
            let remaining = if clock == 0 {
                (limit - current).min(100_000_000)
            } else {
                limit - current
            };
            match now(1).and_then(|n| {
                n.checked_add(remaining)
                    .filter(|v| *v != u64::MAX)
                    .ok_or(EINVAL)
            }) {
                Ok(n) => n,
                Err(e) => {
                    result = e;
                    break;
                }
            }
        } else {
            u64::MAX
        };
        let r = sys::native_futex(ptr::addr_of!(node.notified).cast(), 0, 0, deadline);
        if r > 2 {
            result = EINVAL;
            break;
        }
        // Wake, compare mismatch, timeout and spurious wake all recheck state.
    }
    let reacquire = pthread_mutex_lock(m);
    pthread_mutex_lock(gate);
    if node.notified.load(Ordering::Acquire) != 0 {
        result = 0;
    }
    let mut link = ptr::addr_of_mut!((*c).head);
    while *link != ptr::addr_of_mut!(node) {
        link = ptr::addr_of_mut!((**link).next);
    }
    *link = node.next;
    pthread_mutex_unlock(gate);
    if reacquire != 0 {
        reacquire
    } else {
        result
    }
}
#[no_mangle]
pub unsafe extern "C" fn pthread_cond_wait(c: *mut Cond, m: *mut PthreadMutex) -> i32 {
    wait(c, m, None)
}
#[no_mangle]
pub unsafe extern "C" fn pthread_cond_timedwait(
    c: *mut Cond,
    m: *mut PthreadMutex,
    t: *const Timespec,
) -> i32 {
    if !aligned(t) {
        return EINVAL;
    }
    wait(c, m, Some(*t))
}
