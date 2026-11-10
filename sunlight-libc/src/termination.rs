//! C callbacks require no allocation and never run while the registry is locked.
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, Ordering};

type Callback = extern "C" fn();
const CAPACITY: usize = 32;
struct State {
    callbacks: [Option<Callback>; CAPACITY],
    len: usize,
}
struct Registry {
    locked: AtomicBool,
    state: UnsafeCell<State>,
}
// All access to state is serialized by locked; callbacks run after unlock.
unsafe impl Sync for Registry {}
impl Registry {
    const fn new() -> Self {
        Self {
            locked: AtomicBool::new(false),
            state: UnsafeCell::new(State {
                callbacks: [None; CAPACITY],
                len: 0,
            }),
        }
    }
    fn with<T>(&self, f: impl FnOnce(&mut State) -> T) -> T {
        while self
            .locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            crate::yield_now();
        }
        let result = f(unsafe { &mut *self.state.get() });
        self.locked.store(false, Ordering::Release);
        result
    }
    fn register(&self, callback: Callback) -> bool {
        self.with(|state| {
            if state.len == CAPACITY {
                return false;
            }
            state.callbacks[state.len] = Some(callback);
            state.len += 1;
            true
        })
    }
    fn pop(&self) -> Option<Callback> {
        self.with(|state| {
            if state.len == 0 {
                return None;
            }
            state.len -= 1;
            state.callbacks[state.len].take()
        })
    }
    fn run(&self) {
        while let Some(callback) = self.pop() {
            callback();
        }
    }
}
static CALLBACKS: Registry = Registry::new();

#[cfg_attr(all(not(test), target_os = "sunlightos"), no_mangle)]
pub extern "C" fn atexit(callback: Option<Callback>) -> i32 {
    let Some(callback) = callback else {
        crate::errno::set_errno(crate::errno::EINVAL);
        return -1;
    };
    if CALLBACKS.register(callback) {
        return 0;
    }
    crate::errno::set_errno(crate::errno::ENOMEM);
    -1
}

#[cfg(all(feature = "native-c", target_os = "sunlightos"))]
#[no_mangle]
pub unsafe extern "C" fn exit(status: i32) -> ! {
    CALLBACKS.run();
    // stdio is unbuffered and tmpfiles are already unlinked. The canonical
    // group exit/reaper closes every descriptor, including unclosed streams.
    // sl_std_exit deliberately remains a callback-free Rust/runtime operation.
    crate::std_runtime::sl_std_exit(status)
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::sync::atomic::AtomicUsize;
    static CALLED: AtomicUsize = AtomicUsize::new(0);
    extern "C" fn callback() {
        CALLED.fetch_add(1, Ordering::Relaxed);
    }
    extern "C" fn other() {}

    #[test]
    fn full_registry_fails_without_losing_or_replacing_callbacks() {
        let registry = Registry::new();
        for _ in 0..CAPACITY {
            assert!(registry.register(callback));
        }
        assert!(!registry.register(other));
        registry.run();
        assert_eq!(CALLED.load(Ordering::Relaxed), CAPACITY);
        assert!(registry.pop().is_none());
        assert!(registry.register(other));
    }
    #[test]
    fn pop_is_reverse_order_and_frees_space_before_callback() {
        let registry = Registry::new();
        assert!(registry.register(callback));
        assert!(registry.register(other));
        assert_eq!(
            registry.pop().unwrap() as usize,
            other as *const () as usize
        );
        assert!(registry.register(other));
        assert_eq!(
            registry.pop().unwrap() as usize,
            other as *const () as usize
        );
        assert_eq!(
            registry.pop().unwrap() as usize,
            callback as *const () as usize
        );
        assert!(registry.pop().is_none());
    }
    #[test]
    fn null_registration_is_an_explicit_failure() {
        assert_eq!(atexit(None), -1);
        assert_eq!(crate::errno::get_errno(), crate::errno::EINVAL);
    }
}
