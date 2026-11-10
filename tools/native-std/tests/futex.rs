//! Host contract tests for the actual overlay. No native syscalls are issued.
#![allow(dead_code)]

use std::cell::Cell;
use std::sync::atomic::AtomicU32;
pub use std::{sync, time};

#[path = "../overlay/pal/sunlightos/futex.rs"]
mod futex;

thread_local! {
    static RESULT: Cell<i32> = const { Cell::new(0) };
    static CALL: Cell<Option<(usize, u32, u32, u64)>> = const { Cell::new(None) };
}

unsafe fn sl_std_futex(ptr: *const u32, op: u32, value: u32, deadline: u64) -> i32 {
    CALL.set(Some((ptr as usize, op, value, deadline)));
    RESULT.get()
}

unsafe fn sl_std_clock(_: u32, _: *mut u64) -> i32 {
    panic!("untimed futex test must not query a clock")
}

#[test]
fn wake_preserves_count_and_abi() {
    let word = AtomicU32::new(0);
    for count in [0, 1, 42, i32::MAX] {
        RESULT.set(count);
        assert_eq!(futex::futex_wake(&word), count > 0);
        assert_eq!(CALL.get(), Some((word.as_ptr() as usize, 1, 1, 0)));
        futex::futex_wake_all(&word);
        assert_eq!(CALL.get(), Some((word.as_ptr() as usize, 1, u32::MAX, 0)));
    }
}

#[test]
fn wake_errors_fail_fast() {
    let word = AtomicU32::new(0);
    // Include raw kernel failure/inval and the full negative i32 boundary.
    for error in [-1, -6, -22, i32::MIN] {
        RESULT.set(error);
        assert!(std::panic::catch_unwind(|| futex::futex_wake(&word)).is_err());
        assert!(std::panic::catch_unwind(|| futex::futex_wake_all(&word)).is_err());
    }
}

#[test]
fn wait_preserves_completion_contract() {
    let word = AtomicU32::new(7);
    for (result, completed) in [(0, true), (1, true), (2, false)] {
        RESULT.set(result);
        assert_eq!(futex::futex_wait(&word, 7, None), completed);
        assert_eq!(CALL.get(), Some((word.as_ptr() as usize, 0, 7, u64::MAX)));
    }
    RESULT.set(-6);
    assert!(std::panic::catch_unwind(|| futex::futex_wait(&word, 7, None)).is_err());
}
