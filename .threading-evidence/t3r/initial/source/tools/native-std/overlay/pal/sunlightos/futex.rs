use crate::sync::atomic::AtomicU32;
use crate::time::Duration;
pub type Futex = AtomicU32;
pub type Primitive = u32;
pub type SmallFutex = AtomicU32;
pub type SmallPrimitive = u32;
pub fn futex_wait(word: &Futex, expected: u32, timeout: Option<Duration>) -> bool {
    let deadline = timeout
        .map(|duration| {
            let mut now = [0u64; 2];
            assert_eq!(unsafe { super::sl_std_clock(1, now.as_mut_ptr()) }, 0);
            let nanos = now[0].saturating_mul(1_000_000_000).saturating_add(now[1]);
            nanos.saturating_add(duration.as_nanos().min(u64::MAX as u128) as u64)
        })
        .unwrap_or(u64::MAX);
    let result = unsafe { super::sl_std_futex(word.as_ptr(), 0, expected, deadline) };
    assert!(result >= 0, "native futex wait failed");
    result != 2
}
pub fn futex_wake(word: &Futex) -> bool {
    unsafe { super::sl_std_futex(word.as_ptr(), 1, 1, 0) > 0 }
}
pub fn futex_wake_all(word: &Futex) {
    unsafe { super::sl_std_futex(word.as_ptr(), 1, u32::MAX, 0) };
}
