//! A bounded, reclaiming userspace heap shared by Rust and the C ABI.
//!
//! Every returned pointer has an [`AllocationHeader`] immediately before it.
//! That header records the raw free-list block used for the allocation, so C
//! `free` and `realloc` do not need a Rust `Layout` argument.

use core::cell::UnsafeCell;
use core::cmp;
use core::mem::{align_of, size_of};
use core::ptr;
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

#[cfg(not(feature = "dynamic-heap"))]
const STATIC_HEAP_SIZE: usize = 256 * 1024;
#[cfg(feature = "dynamic-heap")]
const DYNAMIC_HEAP_SIZE: usize = if cfg!(feature = "dynamic-heap-8m") {
    8 * 1024 * 1024
} else {
    1024 * 1024
};
#[cfg(not(feature = "dynamic-heap"))]
const HEAP_SIZE: usize = STATIC_HEAP_SIZE;
#[cfg(feature = "dynamic-heap")]
const HEAP_SIZE: usize = DYNAMIC_HEAP_SIZE;
#[cfg(all(test, not(feature = "dynamic-heap")))]
const REUSE_TEST_HEAP_SIZE: usize = STATIC_HEAP_SIZE;
#[cfg(all(test, feature = "dynamic-heap"))]
const REUSE_TEST_HEAP_SIZE: usize = DYNAMIC_HEAP_SIZE;
const ALLOCATION_MAGIC: usize = 0x534C_414C_4C4F_4331; // "SLALLOC1"
const ALLOCATED: usize = 1;
const FREED: usize = 2;

/// The largest alignment supported directly by the static backing array.  A
/// request with a larger power-of-two alignment still works: the allocator
/// leaves enough prefix padding before its raw block.
#[cfg(not(feature = "dynamic-heap"))]
#[repr(align(4096))]
struct HeapStorage([u8; STATIC_HEAP_SIZE]);

#[cfg(not(feature = "dynamic-heap"))]
static mut STATIC_HEAP: HeapStorage = HeapStorage([0; STATIC_HEAP_SIZE]);

/// Stored at the start of every free block.  The list is sorted by address so
/// insertion can coalesce immediately with both neighbours.
#[repr(C)]
struct FreeBlock {
    size: usize,
    next: *mut FreeBlock,
}

/// Stored immediately before each user pointer.  `backing_*` is the precise
/// block layout returned by the internal free-list allocator; the user layout
/// is retained separately for C `realloc` copy sizing and validation.  The
/// magic/state fields are diagnostic checks only: once a block is returned to
/// the intrusive list, its header may be overwritten, so they cannot promise
/// double-free detection after reuse or a use-after-free write.
#[repr(C)]
struct AllocationHeader {
    magic: usize,
    state: usize,
    backing_addr: usize,
    backing_size: usize,
    backing_align: usize,
    user_size: usize,
    user_align: usize,
}

const FREE_BLOCK_ALIGN: usize = align_of::<FreeBlock>();
const FREE_BLOCK_SIZE: usize = size_of::<FreeBlock>();
const HEADER_ALIGN: usize = align_of::<AllocationHeader>();
const HEADER_SIZE: usize = size_of::<AllocationHeader>();

struct Heap {
    head: *mut FreeBlock,
    initialized: bool,
    start: usize,
    end: usize,
}

/// A fixed-size, allocation-free view of the native allocator's state.
///
/// [`heap_stats`] acquires the allocator lock only long enough to copy
/// free-list totals into this value. It never formats, allocates, or performs
/// IPC, so callers may safely sample it from diagnostics paths.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HeapStats {
    pub heap_capacity: usize,
    pub allocated_backing_bytes: usize,
    pub requested_user_bytes: usize,
    pub free_bytes: usize,
    pub largest_free_block: usize,
    pub allocation_count: u64,
    pub free_count: u64,
    pub realloc_count: u64,
    pub failed_allocation_count: u64,
    pub live_allocation_count: u64,
    pub high_water_allocated_bytes: usize,
    pub free_list_block_count: usize,
}

impl Heap {
    const fn new() -> Self {
        Self {
            head: ptr::null_mut(),
            initialized: false,
            start: 0,
            end: 0,
        }
    }

    /// Initialize the sole bounded heap while the external lock is held.
    unsafe fn initialize(&mut self) -> bool {
        if self.initialized {
            return true;
        }

        #[cfg(not(feature = "dynamic-heap"))]
        let base = ptr::addr_of_mut!(STATIC_HEAP.0).cast::<u8>();

        #[cfg(feature = "dynamic-heap")]
        let base = match crate::mman::mmap(
            ptr::null_mut(),
            DYNAMIC_HEAP_SIZE,
            crate::mman::PROT_READ | crate::mman::PROT_WRITE,
            crate::mman::MAP_PRIVATE | crate::mman::MAP_ANONYMOUS,
            -1,
            0,
        ) {
            Ok(base) => base,
            Err(_) => return false,
        };

        let start = base as usize;
        let heap_size = HEAP_SIZE;
        let Some(end) = start.checked_add(heap_size) else {
            #[cfg(feature = "dynamic-heap")]
            let _ = crate::mman::munmap(base, DYNAMIC_HEAP_SIZE);
            return false;
        };
        if start & (FREE_BLOCK_ALIGN - 1) != 0 {
            #[cfg(feature = "dynamic-heap")]
            let _ = crate::mman::munmap(base, DYNAMIC_HEAP_SIZE);
            return false;
        }

        // Both backends begin on at least page alignment.  Keep the lock
        // state separate from this memory, then publish the list atomically by
        // setting `initialized` last while holding the lock.
        let first = base.cast::<FreeBlock>();
        ptr::write(
            first,
            FreeBlock {
                size: heap_size,
                next: ptr::null_mut(),
            },
        );
        self.head = first;
        self.start = start;
        self.end = end;
        self.initialized = true;
        #[cfg(any(test, debug_assertions))]
        debug_assert!(self.invariants_hold());
        true
    }

    unsafe fn free_list_summary(&self) -> (usize, usize, usize) {
        if !self.initialized {
            return (HEAP_SIZE, HEAP_SIZE, 1);
        }

        let mut free_bytes = 0usize;
        let mut largest_free_block = 0usize;
        let mut free_list_block_count = 0usize;
        let mut current = self.head;
        while !current.is_null() {
            let size = (*current).size;
            free_bytes = free_bytes.saturating_add(size);
            largest_free_block = cmp::max(largest_free_block, size);
            free_list_block_count = free_list_block_count.saturating_add(1);
            current = (*current).next;
        }
        (free_bytes, largest_free_block, free_list_block_count)
    }

    /// Allocate a raw backing block.  `align` is a validated power of two and
    /// at least `FreeBlock` alignment.  All arithmetic is checked.
    unsafe fn allocate_backing(&mut self, size: usize, align: usize) -> (*mut u8, usize) {
        if size == 0 || !valid_align(align) || !self.initialize() {
            return (ptr::null_mut(), 0);
        }

        let mut previous: *mut FreeBlock = ptr::null_mut();
        let mut current = self.head;
        while !current.is_null() {
            let block_start = current as usize;
            let block_size = (*current).size;
            let Some(block_end) = block_start.checked_add(block_size) else {
                return (ptr::null_mut(), 0);
            };

            let Some(mut allocation_start) = align_up(block_start, align) else {
                current = (*current).next;
                continue;
            };
            let prefix = allocation_start - block_start;
            // Do not create a prefix too small to hold a FreeBlock.  Advance
            // one aligned slot so the prefix remains representable instead.
            if prefix != 0 && prefix < FREE_BLOCK_SIZE {
                let Some(after_minimum_prefix) = block_start.checked_add(FREE_BLOCK_SIZE) else {
                    current = (*current).next;
                    continue;
                };
                let Some(next_start) = align_up(after_minimum_prefix, align) else {
                    current = (*current).next;
                    continue;
                };
                allocation_start = next_start;
            }

            let Some(requested_end) = allocation_start.checked_add(size) else {
                current = (*current).next;
                continue;
            };
            if requested_end > block_end {
                previous = current;
                current = (*current).next;
                continue;
            }

            let prefix = allocation_start - block_start;
            let trailing = block_end - requested_end;
            let (backing_size, replacement) = if trailing >= FREE_BLOCK_SIZE {
                let tail = requested_end as *mut FreeBlock;
                ptr::write(
                    tail,
                    FreeBlock {
                        size: trailing,
                        next: (*current).next,
                    },
                );
                (size, tail)
            } else {
                // Consume an unrepresentable tail and record its real size in
                // the allocation metadata so it is returned on free.
                (block_end - allocation_start, (*current).next)
            };

            if prefix == 0 {
                if previous.is_null() {
                    self.head = replacement;
                } else {
                    (*previous).next = replacement;
                }
            } else {
                // `allocation_start` was chosen so this prefix is a valid
                // existing free block; retain it and link its possible tail.
                (*current).size = prefix;
                (*current).next = replacement;
            }
            debug_assert!(backing_size >= size);
            #[cfg(any(test, debug_assertions))]
            debug_assert!(self.invariants_hold());
            return (allocation_start as *mut u8, backing_size);
        }
        (ptr::null_mut(), 0)
    }

    unsafe fn release_backing(&mut self, address: usize, size: usize, align: usize) -> bool {
        if !self.initialized
            || size < FREE_BLOCK_SIZE
            || !valid_align(align)
            || address & (align - 1) != 0
            || address < self.start
        {
            return false;
        }
        let Some(end) = address.checked_add(size) else {
            return false;
        };
        if end > self.end {
            return false;
        }

        let mut previous: *mut FreeBlock = ptr::null_mut();
        let mut current = self.head;
        while !current.is_null() && (current as usize) < address {
            previous = current;
            current = (*current).next;
        }

        // An overlap means a duplicate/invalid free.  C makes that undefined,
        // but rejecting it avoids corrupting our heap in debug or accidental
        // misuse cases.
        let previous_overlaps = if previous.is_null() {
            false
        } else {
            match (previous as usize).checked_add((*previous).size) {
                Some(previous_end) => address < previous_end,
                None => true,
            }
        };
        if previous_overlaps || (!current.is_null() && end > current as usize) {
            return false;
        }

        let merge_left = !previous.is_null()
            && (previous as usize)
                .checked_add((*previous).size)
                .is_some_and(|previous_end| previous_end == address);
        let merged_address = if merge_left {
            previous as usize
        } else {
            address
        };
        let merged_size = if merge_left {
            let Some(value) = (*previous).size.checked_add(size) else {
                return false;
            };
            value
        } else {
            size
        };
        let merge_right = !current.is_null()
            && merged_address
                .checked_add(merged_size)
                .is_some_and(|merged_end| merged_end == current as usize);
        let final_size = if merge_right {
            let Some(value) = merged_size.checked_add((*current).size) else {
                return false;
            };
            value
        } else {
            merged_size
        };
        let next = if merge_right {
            (*current).next
        } else {
            current
        };

        if merge_left {
            (*previous).size = final_size;
            (*previous).next = next;
        } else {
            let inserted = address as *mut FreeBlock;
            ptr::write(
                inserted,
                FreeBlock {
                    size: final_size,
                    next,
                },
            );
            if previous.is_null() {
                self.head = inserted;
            } else {
                (*previous).next = inserted;
            }
        }
        #[cfg(any(test, debug_assertions))]
        debug_assert!(self.invariants_hold());
        true
    }

    /// Check the structural properties that make the intrusive list safe to
    /// traverse.  This deliberately does not allocate or format diagnostics,
    /// so it remains safe to use from allocator tests and debug assertions.
    #[cfg(any(test, debug_assertions))]
    unsafe fn invariants_hold(&self) -> bool {
        if !self.initialized {
            return self.head.is_null() && self.start == 0 && self.end == 0;
        }
        if self.start >= self.end || self.start & (FREE_BLOCK_ALIGN - 1) != 0 {
            return false;
        }
        let Some(capacity) = self.end.checked_sub(self.start) else {
            return false;
        };
        if capacity != HEAP_SIZE {
            return false;
        }
        let mut total = 0usize;
        let mut previous_end = self.start;
        let mut current = self.head;
        // A valid list cannot contain more nodes than the number of minimum
        // sized blocks in the heap.  The bound also detects a cycle.
        let mut remaining_nodes = capacity / FREE_BLOCK_SIZE + 1;
        while !current.is_null() {
            if remaining_nodes == 0 {
                return false;
            }
            remaining_nodes -= 1;
            let address = current as usize;
            let size = (*current).size;
            if address & (FREE_BLOCK_ALIGN - 1) != 0
                || address < previous_end
                || size < FREE_BLOCK_SIZE
            {
                return false;
            }
            let Some(end) = address.checked_add(size) else {
                return false;
            };
            if end > self.end {
                return false;
            }
            let Some(next_total) = total.checked_add(size) else {
                return false;
            };
            total = next_total;
            if total > capacity {
                return false;
            }
            previous_end = end;
            current = (*current).next;
        }
        true
    }
}

struct HeapCell(UnsafeCell<Heap>);
unsafe impl Sync for HeapCell {}

static HEAP: HeapCell = HeapCell(UnsafeCell::new(Heap::new()));
static HEAP_LOCK: AtomicBool = AtomicBool::new(false);
static ALLOCATED_BACKING_BYTES: AtomicUsize = AtomicUsize::new(0);
static REQUESTED_USER_BYTES: AtomicUsize = AtomicUsize::new(0);
static HIGH_WATER_ALLOCATED_BYTES: AtomicUsize = AtomicUsize::new(0);
static ALLOCATION_COUNT: AtomicU64 = AtomicU64::new(0);
static FREE_COUNT: AtomicU64 = AtomicU64::new(0);
static REALLOC_COUNT: AtomicU64 = AtomicU64::new(0);
static FAILED_ALLOCATION_COUNT: AtomicU64 = AtomicU64::new(0);
static LIVE_ALLOCATION_COUNT: AtomicU64 = AtomicU64::new(0);

struct HeapGuard;

impl HeapGuard {
    fn lock() -> Self {
        while HEAP_LOCK
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            while HEAP_LOCK.load(Ordering::Relaxed) {
                core::hint::spin_loop();
            }
        }
        Self
    }

    unsafe fn heap(&self) -> &mut Heap {
        &mut *HEAP.0.get()
    }
}

impl Drop for HeapGuard {
    fn drop(&mut self) {
        HEAP_LOCK.store(false, Ordering::Release);
    }
}

fn valid_align(mut align: usize) -> bool {
    if align == 0 {
        return false;
    }

    // Keep the allocator baseline-x86 safe. `usize::is_power_of_two()` is
    // normally ideal, but LLVM emits POPCNT for it under the workspace's
    // x86-64-v2 service profile. Some supported QEMU CPU models do not expose
    // that instruction, so validate the same property with shifts instead.
    while align & 1 == 0 {
        align >>= 1;
    }
    align == 1
}

fn align_up(value: usize, align: usize) -> Option<usize> {
    debug_assert!(valid_align(align));
    value
        .checked_add(align - 1)
        .map(|value| value & !(align - 1))
}

fn record_high_water(current: usize) {
    let mut observed = HIGH_WATER_ALLOCATED_BYTES.load(Ordering::Relaxed);
    while current > observed {
        match HIGH_WATER_ALLOCATED_BYTES.compare_exchange_weak(
            observed,
            current,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => return,
            Err(next) => observed = next,
        }
    }
}

/// Return a coherent, fixed-size heap telemetry snapshot.
///
/// The free-list traversal is performed while holding the existing heap lock;
/// all atomic counters are copied after that lock is released.
pub fn heap_stats() -> HeapStats {
    let (free_bytes, largest_free_block, free_list_block_count) = {
        let guard = HeapGuard::lock();
        unsafe { guard.heap().free_list_summary() }
    };
    HeapStats {
        heap_capacity: HEAP_SIZE,
        allocated_backing_bytes: ALLOCATED_BACKING_BYTES.load(Ordering::Relaxed),
        requested_user_bytes: REQUESTED_USER_BYTES.load(Ordering::Relaxed),
        free_bytes,
        largest_free_block,
        allocation_count: ALLOCATION_COUNT.load(Ordering::Relaxed),
        free_count: FREE_COUNT.load(Ordering::Relaxed),
        realloc_count: REALLOC_COUNT.load(Ordering::Relaxed),
        failed_allocation_count: FAILED_ALLOCATION_COUNT.load(Ordering::Relaxed),
        live_allocation_count: LIVE_ALLOCATION_COUNT.load(Ordering::Relaxed),
        high_water_allocated_bytes: HIGH_WATER_ALLOCATED_BYTES.load(Ordering::Relaxed),
        free_list_block_count,
    }
}

fn allocation_layout(user_size: usize, user_align: usize) -> Option<(usize, usize, usize)> {
    if user_size == 0 || !valid_align(user_align) {
        return None;
    }
    let backing_align = cmp::max(user_align, cmp::max(HEADER_ALIGN, FREE_BLOCK_ALIGN));
    let user_offset = align_up(HEADER_SIZE, backing_align)?;
    let required = user_offset.checked_add(user_size)?;
    // The raw block must leave future free-block starts correctly aligned.
    let backing_size = align_up(required, FREE_BLOCK_ALIGN)?;
    Some((backing_size, backing_align, user_offset))
}

struct AllocationInfo {
    backing_addr: usize,
    backing_size: usize,
    backing_align: usize,
    user_size: usize,
}

/// Validate an allocation header before trusting its backing-block fields.
///
/// The caller holds the heap lock.  Checking that the immediate header lies
/// inside the managed range first avoids dereferencing obvious foreign
/// pointers.  This cannot make invalid-pointer use defined, but it rejects
/// malformed allocator metadata without touching the free list.
unsafe fn validated_header(
    heap: &Heap,
    user: *mut u8,
) -> Option<(*mut AllocationHeader, AllocationInfo)> {
    if user.is_null() || !heap.initialized {
        return None;
    }
    let user_addr = user as usize;
    let header_addr = user_addr.checked_sub(HEADER_SIZE)?;
    let header_end = header_addr.checked_add(HEADER_SIZE)?;
    if header_addr & (HEADER_ALIGN - 1) != 0 || header_addr < heap.start || header_end > heap.end {
        return None;
    }

    let header = header_addr as *mut AllocationHeader;
    if (*header).magic != ALLOCATION_MAGIC || (*header).state != ALLOCATED {
        return None;
    }
    let info = AllocationInfo {
        backing_addr: (*header).backing_addr,
        backing_size: (*header).backing_size,
        backing_align: (*header).backing_align,
        user_size: (*header).user_size,
    };
    let (required_backing_size, expected_backing_align, user_offset) =
        allocation_layout(info.user_size, (*header).user_align)?;
    if info.backing_align != expected_backing_align
        || !valid_align(info.backing_align)
        || info.backing_addr & (info.backing_align - 1) != 0
        || info.backing_size < required_backing_size
        || info.backing_size & (FREE_BLOCK_ALIGN - 1) != 0
    {
        return None;
    }
    let backing_end = info.backing_addr.checked_add(info.backing_size)?;
    let expected_user_addr = info.backing_addr.checked_add(user_offset)?;
    let user_end = user_addr.checked_add(info.user_size)?;
    if info.backing_addr < heap.start
        || backing_end > heap.end
        || expected_user_addr != user_addr
        || user_end > backing_end
    {
        return None;
    }
    Some((header, info))
}

unsafe fn allocate(user_size: usize, user_align: usize) -> *mut u8 {
    let Some((backing_size, backing_align, user_offset)) = allocation_layout(user_size, user_align)
    else {
        FAILED_ALLOCATION_COUNT.fetch_add(1, Ordering::Relaxed);
        return ptr::null_mut();
    };
    let guard = HeapGuard::lock();
    let (backing, actual_backing_size) = guard.heap().allocate_backing(backing_size, backing_align);
    if backing.is_null() {
        FAILED_ALLOCATION_COUNT.fetch_add(1, Ordering::Relaxed);
        return ptr::null_mut();
    }
    let backing_addr = backing as usize;
    let user_addr = match backing_addr.checked_add(user_offset) {
        Some(address) => address,
        None => return ptr::null_mut(), // unreachable after checked sizing
    };
    let header_addr = user_addr - HEADER_SIZE;
    ptr::write(
        header_addr as *mut AllocationHeader,
        AllocationHeader {
            magic: ALLOCATION_MAGIC,
            state: ALLOCATED,
            backing_addr,
            backing_size: actual_backing_size,
            backing_align,
            user_size,
            user_align,
        },
    );
    let allocated = ALLOCATED_BACKING_BYTES.fetch_add(actual_backing_size, Ordering::Relaxed)
        + actual_backing_size;
    REQUESTED_USER_BYTES.fetch_add(user_size, Ordering::Relaxed);
    ALLOCATION_COUNT.fetch_add(1, Ordering::Relaxed);
    LIVE_ALLOCATION_COUNT.fetch_add(1, Ordering::Relaxed);
    record_high_water(allocated);
    user_addr as *mut u8
}

unsafe fn release(ptr: *mut u8) -> bool {
    if ptr.is_null() {
        return true;
    }
    let guard = HeapGuard::lock();
    let Some((header, info)) = validated_header(guard.heap(), ptr) else {
        return false;
    };
    // Invalidate the header while the block is still unavailable.  If the
    // release is rejected, restore the state before dropping the lock.
    (*header).state = FREED;
    if guard
        .heap()
        .release_backing(info.backing_addr, info.backing_size, info.backing_align)
    {
        ALLOCATED_BACKING_BYTES.fetch_sub(info.backing_size, Ordering::Relaxed);
        REQUESTED_USER_BYTES.fetch_sub(info.user_size, Ordering::Relaxed);
        FREE_COUNT.fetch_add(1, Ordering::Relaxed);
        LIVE_ALLOCATION_COUNT.fetch_sub(1, Ordering::Relaxed);
        true
    } else {
        // A rejected release must leave a valid allocation valid.
        (*header).state = ALLOCATED;
        false
    }
}

unsafe fn requested_size(ptr: *mut u8) -> Option<usize> {
    let guard = HeapGuard::lock();
    validated_header(guard.heap(), ptr).map(|(_, info)| info.user_size)
}

// ── C ABI allocator symbols ─────────────────────────────────────────────────
//
// Export the C names only on freestanding SunlightOS (`target_os = "none"`).
// Host-linked unit tests (and any accidental Linux link of this crate) must
// not interpose glibc `malloc`/`free` — that replaces the process heap with
// the bounded Sunlight free-list and hangs or OOMs the test harness.
// `cfg(test)` covers `cargo test -p sunlight-libc` itself.

/// Allocate at least `size` bytes. Returns null on failure or if `size == 0`.
#[cfg_attr(all(not(test), target_os = "none"), no_mangle)]
pub unsafe extern "C" fn malloc(size: usize) -> *mut u8 {
    allocate(size, 16)
}

/// Release a pointer returned by this allocator. `free(NULL)` is a no-op.
#[cfg_attr(all(not(test), target_os = "none"), no_mangle)]
pub unsafe extern "C" fn free(ptr: *mut u8) {
    let _ = release(ptr);
}

/// Allocate a zero-filled array of `count` × `size` bytes.
#[cfg_attr(all(not(test), target_os = "none"), no_mangle)]
pub unsafe extern "C" fn calloc(count: usize, size: usize) -> *mut u8 {
    let Some(total) = count.checked_mul(size) else {
        return ptr::null_mut();
    };
    let allocation = malloc(total);
    if !allocation.is_null() {
        ptr::write_bytes(allocation, 0, total);
    }
    allocation
}

/// Resize an allocation while preserving its prefix.  Allocation happens
/// before the old block is released, so a failure leaves the old block intact.
#[cfg_attr(all(not(test), target_os = "none"), no_mangle)]
pub unsafe extern "C" fn realloc(ptr: *mut u8, new_size: usize) -> *mut u8 {
    REALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
    if ptr.is_null() {
        return malloc(new_size);
    }
    if new_size == 0 {
        free(ptr);
        return ptr::null_mut();
    }
    let Some(old_size) = requested_size(ptr) else {
        return ptr::null_mut();
    };
    let replacement = malloc(new_size);
    if replacement.is_null() {
        return ptr::null_mut();
    }
    ptr::copy_nonoverlapping(ptr, replacement, cmp::min(old_size, new_size));
    free(ptr);
    replacement
}

// ── Rust GlobalAlloc ─────────────────────────────────────────────────────────

/// Enable `extern crate alloc` (Vec, String, Box, …) for binaries that use
/// the `global-alloc` Cargo feature.
#[cfg(feature = "global-alloc")]
struct SunlightAlloc;

#[cfg(feature = "global-alloc")]
unsafe impl core::alloc::GlobalAlloc for SunlightAlloc {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        allocate(layout.size(), layout.align())
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: core::alloc::Layout) {
        if layout.size() != 0 {
            let _ = release(ptr);
        }
    }

    unsafe fn realloc(
        &self,
        ptr: *mut u8,
        old_layout: core::alloc::Layout,
        new_size: usize,
    ) -> *mut u8 {
        REALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        if old_layout.size() == 0 {
            return allocate(new_size, old_layout.align());
        }
        if new_size == 0 {
            let _ = release(ptr);
            return ptr::null_mut();
        }
        let Some(old_user_size) = requested_size(ptr) else {
            return ptr::null_mut();
        };
        let replacement = allocate(new_size, old_layout.align());
        if replacement.is_null() {
            return ptr::null_mut();
        }
        ptr::copy_nonoverlapping(ptr, replacement, cmp::min(old_user_size, new_size));
        let _ = release(ptr);
        replacement
    }
}

#[cfg(feature = "global-alloc")]
#[global_allocator]
static GLOBAL_ALLOC: SunlightAlloc = SunlightAlloc;

#[cfg(test)]
mod tests {
    #[cfg(feature = "global-alloc")]
    extern crate alloc as rust_alloc;
    extern crate std;

    use super::*;

    fn assert_heap_invariants() {
        let guard = HeapGuard::lock();
        assert!(unsafe { guard.heap().invariants_hold() });
    }

    #[test]
    fn alignment_validator_accepts_powers_of_two() {
        for align in [1, 2, 4, 8, 16, 64, 4096, 65536] {
            assert!(valid_align(align));
        }
        for align in [0, 3, 6, 12, 24, 48, 1023] {
            assert!(!valid_align(align));
        }
    }

    #[test]
    fn c_reuses_blocks_in_non_lifo_order() {
        unsafe {
            let first = malloc(64);
            let middle = malloc(96);
            let last = malloc(64);
            assert!(!first.is_null() && !middle.is_null() && !last.is_null());
            free(middle);
            free(first);
            let reused = malloc(128);
            assert!(!reused.is_null());
            free(last);
            free(reused);
            assert_heap_invariants();
        }
    }

    #[test]
    fn c_reuses_repeated_mixed_size_blocks() {
        unsafe {
            for _ in 0..512 {
                let small = malloc(7);
                let medium = malloc(129);
                let large = malloc(1023);
                assert!(!small.is_null() && !medium.is_null() && !large.is_null());
                free(medium);
                free(large);
                free(small);
            }
            assert_heap_invariants();
        }
    }

    #[test]
    fn heap_stats_track_live_bytes_and_recovery() {
        let baseline = heap_stats();
        unsafe {
            let value = malloc(96);
            assert!(!value.is_null());
            let live = heap_stats();
            assert!(live.allocated_backing_bytes > baseline.allocated_backing_bytes);
            assert!(live.requested_user_bytes >= baseline.requested_user_bytes + 96);
            assert_eq!(
                live.live_allocation_count,
                baseline.live_allocation_count + 1
            );
            assert_eq!(live.allocation_count, baseline.allocation_count + 1);

            free(value);
        }
        let recovered = heap_stats();
        assert_eq!(
            recovered.allocated_backing_bytes,
            baseline.allocated_backing_bytes
        );
        assert_eq!(
            recovered.requested_user_bytes,
            baseline.requested_user_bytes
        );
        assert_eq!(
            recovered.live_allocation_count,
            baseline.live_allocation_count
        );
        assert_eq!(recovered.free_count, baseline.free_count + 1);
    }

    #[test]
    fn c_coalesces_with_each_neighbor_direction() {
        unsafe {
            // Freeing the middle block after its left neighbor exercises the
            // previous-neighbor merge.  Freeing the last block then merges
            // the combined previous block and the free tail (both sides).
            let first = malloc(64);
            let middle = malloc(64);
            let last = malloc(64);
            assert!(!first.is_null() && !middle.is_null() && !last.is_null());
            free(first);
            free(middle);
            free(last);
            assert_heap_invariants();

            // Here the next block is free while the previous block remains
            // allocated, so the middle free is a next-neighbor merge only.
            let first = malloc(64);
            let middle = malloc(64);
            let last = malloc(64);
            assert!(!first.is_null() && !middle.is_null() && !last.is_null());
            free(last);
            free(middle);
            free(first);
            assert_heap_invariants();
        }
    }

    #[test]
    fn c_calloc_and_realloc_contract() {
        unsafe {
            assert!(malloc(0).is_null());
            assert!(calloc(usize::MAX, 2).is_null());
            assert!(calloc(0, 4).is_null());
            let zeroed = calloc(32, 4);
            assert!(!zeroed.is_null());
            assert!((0..128).all(|index| *zeroed.add(index) == 0));
            free(zeroed);

            let grown_from_null = realloc(ptr::null_mut(), 32);
            assert!(!grown_from_null.is_null());
            for index in 0..32 {
                *grown_from_null.add(index) = index as u8;
            }
            let grown = realloc(grown_from_null, 96);
            assert!(!grown.is_null());
            assert!((0..32).all(|index| *grown.add(index) == index as u8));
            let shrunk = realloc(grown, 12);
            assert!(!shrunk.is_null());
            assert!((0..12).all(|index| *shrunk.add(index) == index as u8));
            assert!(realloc(shrunk, 0).is_null());
            free(ptr::null_mut());
            assert_heap_invariants();
        }
    }

    #[cfg(not(feature = "dynamic-heap"))]
    #[test]
    fn c_alignment_and_failed_realloc_keep_old_block() {
        unsafe {
            let aligned = malloc(1);
            assert_eq!((aligned as usize) & 15, 0);
            free(aligned);

            let value = malloc(32);
            assert!(!value.is_null());
            *value = 0xA5;
            assert!(realloc(value, STATIC_HEAP_SIZE).is_null());
            assert_eq!(*value, 0xA5);
            free(value);
            assert_heap_invariants();
        }
    }

    #[test]
    fn raw_layouts_are_reclaimed_and_honor_large_alignment() {
        unsafe {
            for &align in &[1, 2, 8, 16, 64, 4096, 65536] {
                let allocation = allocate(37, align);
                assert!(!allocation.is_null());
                assert_eq!((allocation as usize) & (align - 1), 0);
                let _ = release(allocation);
            }
            // Total traffic is much greater than the bounded heap, but never
            // live concurrently; this proves reuse rather than page release.
            for _ in 0..(REUSE_TEST_HEAP_SIZE / 64 + 64) {
                let allocation = allocate(64, 16);
                assert!(!allocation.is_null());
                let _ = release(allocation);
            }
            assert_heap_invariants();
        }
    }

    #[cfg(all(not(feature = "global-alloc"), not(feature = "dynamic-heap")))]
    #[test]
    fn free_list_exact_fit_unsplittable_tail_and_full_recovery() {
        unsafe {
            let guard = HeapGuard::lock();
            let heap = guard.heap();
            assert!(heap.initialize());
            assert!(heap.invariants_hold());
        }

        // A full backing block is an exact fit.  Returning it must make the
        // complete range available again rather than losing its tail.
        let (_, _, user_offset) = allocation_layout(1, 16).unwrap();
        let exact_user_size = REUSE_TEST_HEAP_SIZE - user_offset;
        unsafe {
            let exact = malloc(exact_user_size);
            assert!(!exact.is_null());
            free(exact);
            assert_heap_invariants();

            // Leave fewer than FreeBlock bytes at the end.  allocate_backing
            // consumes that tail and records the larger backing size in the
            // header; free must recover the entire heap.
            let tail_user_size = REUSE_TEST_HEAP_SIZE - user_offset - FREE_BLOCK_SIZE / 2;
            let tail = malloc(tail_user_size);
            assert!(!tail.is_null());
            free(tail);
            assert_heap_invariants();

            let recovered = malloc(exact_user_size);
            assert!(!recovered.is_null());
            free(recovered);
        }
    }

    #[cfg(feature = "global-alloc")]
    #[test]
    fn rust_global_alloc_supports_core_alloc_types_and_threads() {
        use self::rust_alloc::boxed::Box;
        use self::rust_alloc::collections::BTreeMap;
        use self::rust_alloc::format;
        use self::rust_alloc::rc::Rc;
        use self::rust_alloc::string::String;
        use self::rust_alloc::sync::Arc;
        use self::rust_alloc::vec::Vec;
        use core::alloc::{GlobalAlloc, Layout};

        let boxed = Box::new(7u64);
        assert_eq!(*boxed, 7);
        drop(boxed);

        let mut values = Vec::new();
        for value in 0..1024u32 {
            values.push(value);
        }
        assert_eq!(values.len(), 1024);
        drop(values);

        let message: String = format!("sunlight allocator {}", 42);
        assert_eq!(message.as_bytes(), b"sunlight allocator 42");
        drop(message);

        let mut map = BTreeMap::new();
        for value in 0..32u32 {
            map.insert(value, value * value);
        }
        assert_eq!(map.get(&9), Some(&81));
        drop(map);

        let rc = Rc::new(11u8);
        assert_eq!(*Rc::clone(&rc), 11);
        drop(rc);
        let arc = Arc::new(13u8);
        assert_eq!(*Arc::clone(&arc), 13);
        drop(arc);

        unsafe {
            for &align in &[1, 2, 8, 16, 64, 4096, 65536] {
                let layout = Layout::from_size_align(128, align).unwrap();
                let value = GLOBAL_ALLOC.alloc(layout);
                assert!(!value.is_null());
                assert_eq!((value as usize) & (align - 1), 0);
                GLOBAL_ALLOC.dealloc(value, layout);
            }
        }

        let workers = (0..2)
            .map(|_| {
                std::thread::spawn(|| {
                    for _ in 0..128 {
                        let value = Box::new([0x5Au8; 128]);
                        assert_eq!(value[0], 0x5A);
                        drop(value);
                    }
                })
            })
            .collect::<Vec<_>>();
        for worker in workers {
            worker.join().unwrap();
        }
        assert_heap_invariants();
    }

    #[cfg(feature = "global-alloc")]
    #[test]
    fn rust_realloc_uses_recorded_size_not_disagreeing_old_layout() {
        use core::alloc::{GlobalAlloc, Layout};

        unsafe {
            let actual = Layout::from_size_align(8, 16).unwrap();
            let incorrect_old_layout = Layout::from_size_align(1024, 16).unwrap();
            let value = GLOBAL_ALLOC.alloc(actual);
            assert!(!value.is_null());
            for index in 0..actual.size() {
                *value.add(index) = index as u8;
            }
            let replacement = GLOBAL_ALLOC.realloc(value, incorrect_old_layout, 32);
            assert!(!replacement.is_null());
            assert!((0..actual.size()).all(|index| *replacement.add(index) == index as u8));
            GLOBAL_ALLOC.dealloc(replacement, Layout::from_size_align(32, 16).unwrap());
            assert_heap_invariants();
        }
    }

    #[cfg(all(feature = "global-alloc", not(feature = "dynamic-heap")))]
    #[test]
    fn failed_allocation_does_not_damage_live_rust_allocation() {
        use self::rust_alloc::boxed::Box;

        let live = Box::new([0xA5u8; 256]);
        unsafe {
            let impossible = allocate(STATIC_HEAP_SIZE, 16);
            assert!(impossible.is_null());
        }
        assert_eq!(live[0], 0xA5);
        assert_eq!(live[255], 0xA5);
        drop(live);
    }
}
