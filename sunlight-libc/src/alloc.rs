//! Phase 1 allocator: static bump allocator backed by a BSS region, or
//! dynamically backed via `mmap` if the `dynamic-heap` feature is enabled.
//!
//! # Phase 1 limitations
//! - `free()` is a no-op: memory is never returned to the pool.
//! - `realloc()` (C ABI) copies without knowing the old size — safe for
//!   grow-only patterns but not general use.
//! - Rust `GlobalAlloc::realloc` is correct because the old layout is passed.
//! - Allocation uses atomics for concurrent bumps but is still a simple
//!   bump-only allocator with no reclamation.

// ── Variant A: Static BSS Backing (Legacy/Micro-services) ───────────────────

#[cfg(not(feature = "dynamic-heap"))]
mod backend {
    use core::sync::atomic::{AtomicUsize, Ordering};

    const HEAP_SIZE: usize = 256 * 1024;
    static mut HEAP: [u8; HEAP_SIZE] = [0u8; HEAP_SIZE];
    static HEAP_NEXT: AtomicUsize = AtomicUsize::new(0);

    pub fn bump_alloc(size: usize, align: usize) -> *mut u8 {
        if size == 0 {
            return core::ptr::null_mut();
        }
        loop {
            let current = HEAP_NEXT.load(Ordering::Relaxed);
            let aligned = (current + align - 1) & !(align - 1);
            let end = match aligned.checked_add(size) {
                Some(e) if e <= HEAP_SIZE => e,
                _ => return core::ptr::null_mut(),
            };
            match HEAP_NEXT.compare_exchange(current, end, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => {
                    return unsafe { core::ptr::addr_of_mut!(HEAP).cast::<u8>().add(aligned) };
                }
                Err(_) => continue,
            }
        }
    }
}

// ── Variant B: Dynamic mmap Backing (New Applications) ──────────────────────

#[cfg(feature = "dynamic-heap")]
mod backend {
    use core::sync::atomic::{AtomicUsize, Ordering};

    use crate::mman::{mmap, munmap, MAP_ANONYMOUS, MAP_FAILED, MAP_PRIVATE, PROT_READ, PROT_WRITE};

    const HEAP_SIZE: usize = 1024 * 1024;
    static HEAP_BASE: AtomicUsize = AtomicUsize::new(0);
    static HEAP_NEXT: AtomicUsize = AtomicUsize::new(0);

    pub fn bump_alloc(size: usize, align: usize) -> *mut u8 {
        if size == 0 {
            return core::ptr::null_mut();
        }

        let mut base = HEAP_BASE.load(Ordering::Acquire);
        if base == 0 {
            let ptr = mmap(
                core::ptr::null_mut(),
                HEAP_SIZE,
                PROT_READ | PROT_WRITE,
                MAP_PRIVATE | MAP_ANONYMOUS,
                -1,
                0,
            )
            .unwrap_or(MAP_FAILED);

            if ptr == MAP_FAILED {
                return core::ptr::null_mut();
            }

            match HEAP_BASE.compare_exchange(0, ptr as usize, Ordering::Release, Ordering::Acquire)
            {
                Ok(_) => {
                    base = ptr as usize;
                    HEAP_NEXT.store(base, Ordering::Release);
                }
                Err(existing_base) => {
                    let _ = munmap(ptr, HEAP_SIZE);
                    base = existing_base;
                }
            }
        }

        loop {
            let current = HEAP_NEXT.load(Ordering::Relaxed);
            if current == 0 {
                continue;
            }

            let aligned = (current + align - 1) & !(align - 1);
            let end = match aligned.checked_add(size) {
                Some(e) if e <= base + HEAP_SIZE => e,
                _ => return core::ptr::null_mut(),
            };

            if HEAP_NEXT
                .compare_exchange(current, end, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                return aligned as *mut u8;
            }
        }
    }
}

use backend::bump_alloc;

// ── C ABI allocator symbols ─────────────────────────────────────────────────

/// Allocate at least `size` bytes. Returns null on failure or if `size == 0`.
#[no_mangle]
pub unsafe extern "C" fn malloc(size: usize) -> *mut u8 {
    bump_alloc(size, 16)
}

/// Free a previously allocated pointer.
/// Phase 1: no-op. Memory is not reclaimed.
#[no_mangle]
pub unsafe extern "C" fn free(_ptr: *mut u8) {}

/// Allocate a zero-filled array of `count` × `size` bytes.
/// Returns null on overflow or exhaustion.
#[no_mangle]
pub unsafe extern "C" fn calloc(count: usize, size: usize) -> *mut u8 {
    let total = match count.checked_mul(size) {
        Some(n) => n,
        None => return core::ptr::null_mut(),
    };
    let ptr = bump_alloc(total, 16);
    if !ptr.is_null() {
        core::ptr::write_bytes(ptr, 0, total);
    }
    ptr
}

/// Resize an allocation.
///
/// Phase 1 limitation: the C ABI does not carry the old allocation size, so
/// this implementation always copies `new_size` bytes from `ptr`, which may
/// read past the original allocation if `new_size > old_size`. This is safe
/// only for grow-then-copy patterns (typical Rust Vec growth). Do not use for
/// shrink-in-place patterns.
///
/// `realloc(NULL, n)` behaves like `malloc(n)`.
/// `realloc(p, 0)` frees `p` and returns null.
#[no_mangle]
pub unsafe extern "C" fn realloc(ptr: *mut u8, new_size: usize) -> *mut u8 {
    if ptr.is_null() {
        return malloc(new_size);
    }
    if new_size == 0 {
        free(ptr);
        return core::ptr::null_mut();
    }
    let new_ptr = malloc(new_size);
    if !new_ptr.is_null() {
        // Copy conservatively — caller is responsible for not passing new_size
        // larger than the original allocation in Phase 1.
        core::ptr::copy_nonoverlapping(ptr, new_ptr, new_size);
    }
    new_ptr
}

// ── Rust GlobalAlloc ─────────────────────────────────────────────────────────

/// Enable `extern crate alloc` (Vec, String, Box, …) for binaries that use
/// the `global-alloc` Cargo feature.
///
/// The Rust allocator API always passes the old `Layout` to `realloc`, so
/// the copy-size issue that affects the C ABI version does not apply here.
#[cfg(feature = "global-alloc")]
struct SunlightBumpAlloc;

#[cfg(feature = "global-alloc")]
unsafe impl core::alloc::GlobalAlloc for SunlightBumpAlloc {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        bump_alloc(layout.size(), layout.align())
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: core::alloc::Layout) {
        // Phase 1: no-op.
    }

    unsafe fn realloc(
        &self,
        ptr: *mut u8,
        old_layout: core::alloc::Layout,
        new_size: usize,
    ) -> *mut u8 {
        let new_layout =
            match core::alloc::Layout::from_size_align(new_size, old_layout.align()) {
                Ok(l) => l,
                Err(_) => return core::ptr::null_mut(),
            };
        let new_ptr = self.alloc(new_layout);
        if !new_ptr.is_null() {
            // Copy the minimum of old and new sizes — always correct.
            let copy_len = old_layout.size().min(new_size);
            core::ptr::copy_nonoverlapping(ptr, new_ptr, copy_len);
        }
        new_ptr
    }
}

#[cfg(feature = "global-alloc")]
#[global_allocator]
static GLOBAL_ALLOC: SunlightBumpAlloc = SunlightBumpAlloc;
