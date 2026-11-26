//! Independent mmap extents with a bounded aggregate capacity.

use super::{align_up, cmp, ptr, Heap, FREE_BLOCK_SIZE, HEAP_SIZE};
use crate::mman;

const PAGE: usize = 4096;
const QUANTUM: usize = 256 * 1024;
const INITIAL: usize =
    if cfg!(feature = "adaptive-tty-heap") && !cfg!(feature = "adaptive-document-heap") {
        25 * QUANTUM
    } else {
        QUANTUM
    };
const REGIONS: usize = HEAP_SIZE / QUANTUM;

pub(super) struct ProcessHeap {
    regions: [Heap; REGIONS],
    committed: usize,
}

impl ProcessHeap {
    pub(super) const fn new() -> Self {
        Self {
            regions: [const { Heap::new() }; REGIONS],
            committed: 0,
        }
    }

    pub(super) fn capacity(&self) -> usize {
        self.committed
    }

    pub(super) fn region_for(&mut self, address: usize) -> Option<&mut Heap> {
        self.regions
            .iter_mut()
            .find(|region| region.start <= address && address < region.end)
    }

    pub(super) unsafe fn allocate(&mut self, size: usize, align: usize) -> (*mut u8, usize) {
        for region in self.regions.iter_mut().filter(|region| region.initialized) {
            let result = region.allocate_backing(size, align);
            if !result.0.is_null() {
                return result;
            }
        }
        self.grow(size, align).unwrap_or((ptr::null_mut(), 0))
    }

    unsafe fn grow(&mut self, size: usize, align: usize) -> Option<(*mut u8, usize)> {
        let available = HEAP_SIZE - self.committed;
        let padding = align.saturating_sub(PAGE);
        let required = align_up(size.checked_add(padding)?.max(FREE_BLOCK_SIZE), PAGE)?;
        let target = if self.committed == 0 {
            INITIAL
        } else {
            QUANTUM
        };
        let length = required.max(target).min(available);
        if length < required {
            return None;
        }
        let region = self.regions.iter_mut().find(|region| !region.initialized)?;
        let map = |length| {
            mman::mmap(
                ptr::null_mut(),
                length,
                mman::PROT_READ | mman::PROT_WRITE,
                mman::MAP_PRIVATE | mman::MAP_ANONYMOUS,
                -1,
                0,
            )
        };
        let (base, length) = match map(length) {
            Ok(base) => (base, length),
            Err(_) if required < length => (map(required).ok()?, required),
            Err(_) => return None,
        };
        if !region.initialize_region(base, length) {
            return None;
        }
        self.committed += length;
        Some(region.allocate_backing(size, align))
    }

    pub(super) unsafe fn trim(&mut self, address: usize) {
        let Some(region) = self.regions.iter_mut().skip(1).find(|region| {
            region.start <= address && address < region.end && region.head as usize == region.start
        }) else {
            return;
        };
        let length = region.end - region.start;
        if (*region.head).size == length
            && (*region.head).next.is_null()
            && mman::munmap(region.start as *mut u8, length).is_ok()
        {
            *region = Heap::new();
            self.committed -= length;
        }
    }

    pub(super) unsafe fn free_list_summary(&self) -> (usize, usize, usize) {
        let mut summary = (0, 0, 0);
        for region in self.regions.iter().filter(|region| region.initialized) {
            let (free, largest, blocks) = region.free_list_summary();
            summary.0 += free;
            summary.1 = cmp::max(summary.1, largest);
            summary.2 += blocks;
        }
        summary
    }

    #[cfg(test)]
    pub(super) unsafe fn invariants_hold(&self) -> bool {
        self.committed <= HEAP_SIZE
            && self.committed
                == self
                    .regions
                    .iter()
                    .map(|region| region.end - region.start)
                    .sum()
            && self.regions.iter().all(|region| region.invariants_hold())
    }
}
