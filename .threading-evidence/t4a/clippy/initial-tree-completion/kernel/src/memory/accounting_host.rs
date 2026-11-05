//! Host-only pure tests for physical memory accounting Phase 1.
//! Included from `mm2a_host` so they run under the host `test` crate.

#![allow(dead_code)]

use core::sync::atomic::{AtomicU64, Ordering};

pub const FRAME_BYTES: u64 = 4096;
pub const CONSERVATION_TOLERANCE_BYTES: u64 = 16 * FRAME_BYTES;
pub const CLASS_COUNT: usize = 16;

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhysicalMemoryClass {
    Free = 0,
    ReservedFirmware = 1,
    ReservedKernelImage = 2,
    KernelCore = 3,
    KernelHeap = 4,
    KernelStack = 5,
    PageTable = 6,
    UserPrivate = 7,
    SharedMemory = 8,
    RamFsFileData = 9,
    RamFsMetadata = 10,
    FileSystemCache = 11,
    GraphicsBuffer = 12,
    DeviceDma = 13,
    CompressedMemory = 14,
    OtherAccounted = 15,
}

static CLASS_FRAMES: [AtomicU64; CLASS_COUNT] = [const { AtomicU64::new(0) }; CLASS_COUNT];
static DIAG_UNDERFLOW: AtomicU64 = AtomicU64::new(0);

pub fn note_alloc(class: PhysicalMemoryClass, frames: u64) {
    if frames == 0 {
        return;
    }
    let _ = CLASS_FRAMES[class as usize].fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| {
        v.checked_add(frames)
    });
}

pub fn note_free(class: PhysicalMemoryClass, frames: u64) {
    if frames == 0 {
        return;
    }
    if CLASS_FRAMES[class as usize]
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| {
            v.checked_sub(frames)
        })
        .is_err()
    {
        DIAG_UNDERFLOW.fetch_add(1, Ordering::Relaxed);
    }
}

pub fn note_reclass(from: PhysicalMemoryClass, to: PhysicalMemoryClass, frames: u64) {
    if frames == 0 || from == to {
        return;
    }
    if CLASS_FRAMES[from as usize]
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| {
            v.checked_sub(frames)
        })
        .is_err()
    {
        DIAG_UNDERFLOW.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let _ = CLASS_FRAMES[to as usize].fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| {
        v.checked_add(frames)
    });
}

pub fn class_frames(class: PhysicalMemoryClass) -> u64 {
    CLASS_FRAMES[class as usize].load(Ordering::Relaxed)
}

pub fn class_bytes(class: PhysicalMemoryClass) -> u64 {
    class_frames(class).saturating_mul(FRAME_BYTES)
}

fn reset() {
    for c in CLASS_FRAMES.iter() {
        c.store(0, Ordering::Relaxed);
    }
    DIAG_UNDERFLOW.store(0, Ordering::Relaxed);
}

/// Conservation: managed = free + sum(classes) + unclassified
pub fn conservation_ok(managed: u64, free: u64, accounted_used: u64) -> bool {
    let rhs = free.saturating_add(accounted_used);
    managed.abs_diff(rhs) <= CONSERVATION_TOLERANCE_BYTES
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alloc_increments_one_class() {
        reset();
        note_alloc(PhysicalMemoryClass::UserPrivate, 4);
        assert_eq!(class_frames(PhysicalMemoryClass::UserPrivate), 4);
        assert_eq!(class_frames(PhysicalMemoryClass::SharedMemory), 0);
    }

    #[test]
    fn free_decrements_correct_class() {
        reset();
        note_alloc(PhysicalMemoryClass::PageTable, 3);
        note_free(PhysicalMemoryClass::PageTable, 2);
        assert_eq!(class_frames(PhysicalMemoryClass::PageTable), 1);
    }

    #[test]
    fn failed_zero_frames_noop() {
        reset();
        note_alloc(PhysicalMemoryClass::KernelHeap, 0);
        assert_eq!(class_frames(PhysicalMemoryClass::KernelHeap), 0);
    }

    #[test]
    fn invalid_free_does_not_underflow() {
        reset();
        note_free(PhysicalMemoryClass::UserPrivate, 1);
        assert_eq!(class_frames(PhysicalMemoryClass::UserPrivate), 0);
        assert!(DIAG_UNDERFLOW.load(Ordering::Relaxed) >= 1);
    }

    #[test]
    fn reclass_preserves_total() {
        reset();
        note_alloc(PhysicalMemoryClass::OtherAccounted, 10);
        note_reclass(
            PhysicalMemoryClass::OtherAccounted,
            PhysicalMemoryClass::DeviceDma,
            4,
        );
        assert_eq!(class_frames(PhysicalMemoryClass::OtherAccounted), 6);
        assert_eq!(class_frames(PhysicalMemoryClass::DeviceDma), 4);
    }

    #[test]
    fn shm_unique_not_multiplied() {
        reset();
        note_alloc(PhysicalMemoryClass::SharedMemory, 8);
        // Five mappers would still be 8 frames globally.
        assert_eq!(
            class_bytes(PhysicalMemoryClass::SharedMemory),
            8 * FRAME_BYTES
        );
    }

    #[test]
    fn cache_not_inferred_from_residual() {
        reset();
        note_alloc(PhysicalMemoryClass::UserPrivate, 20);
        let used = 90 * FRAME_BYTES;
        let accounted = class_bytes(PhysicalMemoryClass::UserPrivate);
        let unclassified = used.saturating_sub(accounted);
        let cache = 0u64; // never residual
        assert_eq!(cache, 0);
        assert_eq!(unclassified, 70 * FRAME_BYTES);
        assert!(conservation_ok(
            100 * FRAME_BYTES,
            10 * FRAME_BYTES,
            accounted + unclassified
        ));
    }

    #[test]
    fn zram_logical_excluded_from_physical() {
        reset();
        note_alloc(PhysicalMemoryClass::CompressedMemory, 2);
        let physical = class_bytes(PhysicalMemoryClass::CompressedMemory);
        let logical = 100 * FRAME_BYTES;
        assert_eq!(physical, 2 * FRAME_BYTES);
        assert!(physical < logical);
    }
}
