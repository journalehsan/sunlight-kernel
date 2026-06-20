//! Memory utility functions.
//!
//! These are thin Rust wrappers around core's intrinsics. The compiler_builtins
//! crate provides the C ABI `#[no_mangle]` versions for compiler-generated code;
//! these are the safe internal helpers used by the rest of sunlight-libc.

/// Copy `n` bytes from `src` to `dst`. Regions must not overlap.
///
/// # Safety
/// `dst` and `src` must each point to at least `n` valid bytes and must not
/// alias.
#[inline]
pub unsafe fn memcpy_bytes(dst: *mut u8, src: *const u8, n: usize) {
    core::ptr::copy_nonoverlapping(src, dst, n);
}

/// Copy `n` bytes from `src` to `dst`. Handles overlapping regions correctly.
///
/// # Safety
/// `dst` and `src` must each point to at least `n` valid bytes.
#[inline]
pub unsafe fn memmove_bytes(dst: *mut u8, src: *const u8, n: usize) {
    core::ptr::copy(src, dst, n);
}

/// Fill `n` bytes at `dst` with the byte value `c`.
///
/// # Safety
/// `dst` must point to at least `n` writable bytes.
#[inline]
pub unsafe fn memset_bytes(dst: *mut u8, c: u8, n: usize) {
    core::ptr::write_bytes(dst, c, n);
}

/// Compare `n` bytes at `a` and `b`. Returns 0 if equal, negative if `a < b`,
/// positive if `a > b`.
///
/// # Safety
/// `a` and `b` must each point to at least `n` valid readable bytes.
#[inline]
pub unsafe fn memcmp_bytes(a: *const u8, b: *const u8, n: usize) -> i32 {
    for i in 0..n {
        let av = *a.add(i);
        let bv = *b.add(i);
        if av != bv {
            return (av as i32) - (bv as i32);
        }
    }
    0
}
