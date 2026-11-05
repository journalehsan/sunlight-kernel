//! Key based TLS in the native per-thread TCB page, following errno at offset 8.
use crate::sync::atomic::{AtomicUsize, Ordering};
pub type Key = usize;
type Dtor = unsafe extern "C" fn(*mut u8);
// 0 = free, 1 = reserved/no destructor, otherwise destructor address.
static KEYS: [AtomicUsize; 256] = [const { AtomicUsize::new(0) }; 256];
fn table() -> *mut *mut u8 {
    let base: usize;
    unsafe {
        core::arch::asm!("mov {}, fs:0", out(reg) base, options(nostack, preserves_flags, readonly))
    };
    assert!(base != 0, "native TLS must be initialized before std");
    (base + 32) as *mut *mut u8
}
pub fn create(dtor: Option<Dtor>) -> Key {
    for index in 1..KEYS.len() {
        if KEYS[index]
            .compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            KEYS[index].store(dtor.map_or(1, |f| f as usize), Ordering::Release);
            return index;
        }
    }
    panic!("native TLS key capacity exceeded")
}
pub unsafe fn destroy(key: Key) {
    KEYS[key].store(0, Ordering::Release);
}
pub unsafe fn get(key: Key) -> *mut u8 {
    unsafe { table().add(key).read() }
}
pub unsafe fn set(key: Key, value: *mut u8) {
    unsafe { table().add(key).write(value) };
}
pub unsafe fn run_dtors() {
    for _ in 0..5 {
        let mut ran = false;
        for index in 1..KEYS.len() {
            let dtor = KEYS[index].load(Ordering::Acquire);
            let value = unsafe { get(index) };
            if dtor > 1 && !value.is_null() {
                unsafe { set(index, crate::ptr::null_mut()) };
                let dtor: Dtor = unsafe { crate::mem::transmute(dtor) };
                unsafe { dtor(value) };
                ran = true;
            }
        }
        if !ran {
            break;
        }
    }
}
