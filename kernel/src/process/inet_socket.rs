//! Bounded native IPv4 stream endpoints in the unconnected state.
//!
//! Transport attachment to net_server is deliberately not implemented. No
//! endpoint can claim a connection or consume network data in this phase.
use spin::Mutex;

// Fixed storage makes exhaustion and rollback independent of heap allocation.
static REFS: Mutex<[usize; 128]> = Mutex::new([0; 128]);

pub fn create() -> Option<u32> {
    let mut refs = REFS.lock();
    let index = refs.iter().position(|n| *n == 0)?;
    refs[index] = 1;
    Some(index as u32)
}

pub fn retain(index: u32) {
    let mut refs = REFS.lock();
    let count = &mut refs[index as usize];
    assert!(*count != 0);
    *count = count.checked_add(1).expect("socket reference overflow");
}

pub fn release(index: u32) {
    let mut refs = REFS.lock();
    let count = &mut refs[index as usize];
    assert!(*count != 0);
    *count -= 1;
}

#[cfg(feature = "native_c_guest")]
pub fn references() -> usize {
    REFS.lock().iter().sum()
}
