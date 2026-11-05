//! Thread admission is a transaction under the existing SCHEDULER guard.
//! No reservation can outlive that guard: before insertion all resources are
//! local RAII values; after insertion the unreaped Process is the quota owner.
//! Count Finished/pinned tasks until Reaped, using the full VM incarnation.
use alloc::{boxed::Box, string::String, vec::Vec};
use core::{alloc::Layout, mem::MaybeUninit};

pub const MAX_VM_WORKERS: usize = 32;
pub const MAX_GLOBAL_WORKERS: usize = 96;
pub const MAX_THREAD_TASK_SLOTS: usize = 512;
pub const MAX_INHERITED_BYTES: usize = 16 * 1024;
// Thread creation leaves this much allocator space for existing services.
// Fallible allocation remains mandatory: free bytes do not prove contiguity.
pub const THREAD_HEAP_HEADROOM: usize = 2 * 1024 * 1024;
pub const MAX_ENV_ENTRIES: usize = 128;
pub const MAX_CAPABILITIES: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Capacity,
    NoMemory,
    Cancelled,
}

/// Pure accounting boundary shared by host tests and native admission.
pub fn check_counts(global: usize, vm: usize, slots: usize) -> Result<(), Error> {
    if global
        .checked_add(1)
        .filter(|n| *n <= MAX_GLOBAL_WORKERS)
        .is_none()
        || vm.checked_add(1).filter(|n| *n <= MAX_VM_WORKERS).is_none()
        || slots > MAX_THREAD_TASK_SLOTS
    {
        return Err(Error::Capacity);
    }
    Ok(())
}

/// Allocate directly into heap storage, avoiding large stack temporaries.
/// The caller must initialize every field before assuming initialization.
pub fn checkpoint(stage: &'static str) -> Result<(), Error> {
    #[cfg(all(debug_assertions, feature = "thread_admission_test", not(test)))]
    crate::sched::t2_fixture::allocation(stage)?;
    #[cfg(test)]
    host_fault::allocation(stage)?;
    let _ = stage;
    Ok(())
}

pub fn try_box_uninit<T>() -> Result<Box<MaybeUninit<T>>, Error> {
    checkpoint("box")?;
    let layout = Layout::new::<MaybeUninit<T>>();
    assert!(layout.size() != 0);
    let ptr = unsafe { alloc::alloc::alloc(layout) } as *mut MaybeUninit<T>;
    if ptr.is_null() {
        return Err(Error::NoMemory);
    }
    // SAFETY: allocation has exactly the Box layout and unique ownership.
    Ok(unsafe { Box::from_raw(ptr) })
}

pub fn try_string(value: &str) -> Result<String, Error> {
    checkpoint("string")?;
    let mut result = String::new();
    result
        .try_reserve_exact(value.len())
        .map_err(|_| Error::NoMemory)?;
    result.push_str(value);
    Ok(result)
}

pub fn try_copy<T: Copy>(value: &[T]) -> Result<Vec<T>, Error> {
    checkpoint("capabilities")?;
    let mut result = Vec::new();
    result
        .try_reserve_exact(value.len())
        .map_err(|_| Error::NoMemory)?;
    result.extend_from_slice(value);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quota_boundaries_and_overflow() {
        assert_eq!(check_counts(95, 31, 512), Ok(()));
        for (global, vm, slots) in [
            (96, 0, 1),
            (0, 32, 1),
            (0, 0, 513),
            (usize::MAX, 0, 1),
            (0, usize::MAX, 1),
        ] {
            assert_eq!(check_counts(global, vm, slots), Err(Error::Capacity));
        }
    }
}

#[cfg(test)]
pub mod host_fault {
    use super::Error;
    use core::cell::Cell;
    std::thread_local! { static REMAINING: Cell<usize> = const { Cell::new(0) }; }
    pub fn arm(ordinal: usize) {
        REMAINING.with(|n| n.set(ordinal));
    }
    pub fn allocation(_: &str) -> Result<(), Error> {
        REMAINING.with(|n| {
            let value = n.get();
            if value == 0 {
                return Ok(());
            }
            n.set(value - 1);
            if value == 1 {
                Err(Error::NoMemory)
            } else {
                Ok(())
            }
        })
    }
}
