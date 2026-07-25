//! Freestanding memory primitives (`memcpy`, `memmove`, `memset`, `memcmp`,
//! `memchr`).
//!
//! # Symbol ownership
//!
//! These C ABI symbols are **strong** definitions. They override the weak
//! hidden implementations supplied by `compiler_builtins` for freestanding
//! targets, so userspace has a single intentional owner for memory ops.
//!
//! # Compiler-recursion safety
//!
//! Implementations use explicit byte loops. They deliberately **do not** call
//! `core::ptr::copy`, `copy_nonoverlapping`, or `write_bytes`, because those
//! lower to LLVM memory intrinsics that can become calls back into these
//! symbols. Generated assembly is verified not to call `memcpy`/`memmove`/
//! `memset`/`memcmp`/`memchr` from within themselves.
//!
//! # Caller validity (nonzero `n`)
//!
//! For `n > 0`, the caller must ensure:
//! - pointer arguments are non-null and correctly aligned for byte access
//!   (any address is fine for `u8`);
//! - each range `[ptr, ptr+n)` is readable (and writable for destinations);
//! - for `memcpy` only, source and destination ranges do not overlap
//!   (overlap is undefined behavior for `memcpy`; use `memmove`).
//!
//! For `n == 0`, implementations perform no memory access and return
//! immediately. Null pointers are therefore accepted when `n == 0`.
//!
//! # Host tests
//!
//! C `#[no_mangle]` exports are emitted only on freestanding SunlightOS
//! (`target_os = "none"`, and not under `cfg(test)`). Host-linked dependents
//! and `cargo test -p sunlight-libc` must not interpose system libc symbols.

// ── Internal portable engines ────────────────────────────────────────────────

/// Copy `n` bytes from `src` to `dst`. Regions must not overlap.
///
/// # Safety
/// See module docs. Overlapping ranges are UB.
#[inline]
pub unsafe fn memcpy_bytes(dst: *mut u8, src: *const u8, n: usize) {
    // Forward byte copy. No `core::ptr::copy*` — those can lower to `memcpy`.
    let mut i = 0usize;
    while i < n {
        *dst.add(i) = *src.add(i);
        i += 1;
    }
}

/// Copy `n` bytes from `src` to `dst`, correctly handling every overlap case.
///
/// # Safety
/// See module docs. Both ranges must be valid for `n` bytes.
#[inline]
pub unsafe fn memmove_bytes(dst: *mut u8, src: *const u8, n: usize) {
    if n == 0 || dst as usize == src as usize {
        return;
    }
    if (dst as usize) < (src as usize) {
        // Destination starts before source: forward copy is always safe.
        let mut i = 0usize;
        while i < n {
            *dst.add(i) = *src.add(i);
            i += 1;
        }
    } else {
        // Destination starts after source: may overlap; copy backward.
        let mut i = n;
        while i > 0 {
            i -= 1;
            *dst.add(i) = *src.add(i);
        }
    }
}

/// Fill `n` bytes at `dst` with the byte value `c`.
///
/// # Safety
/// See module docs. `dst` must be writable for `n` bytes when `n > 0`.
#[inline]
pub unsafe fn memset_bytes(dst: *mut u8, c: u8, n: usize) {
    let mut i = 0usize;
    while i < n {
        *dst.add(i) = c;
        i += 1;
    }
}

/// Compare `n` bytes at `a` and `b` as unsigned values.
///
/// Returns `0` if equal, negative if `a < b` at the first difference, positive
/// if `a > b`. Stops at the first differing byte.
///
/// # Safety
/// See module docs. Both ranges must be readable for `n` bytes when `n > 0`.
#[inline]
pub unsafe fn memcmp_bytes(a: *const u8, b: *const u8, n: usize) -> i32 {
    let mut i = 0usize;
    while i < n {
        let av = *a.add(i);
        let bv = *b.add(i);
        if av != bv {
            // Cast through u8→i32 so high bytes (0x80..=0xff) compare unsigned.
            return (av as i32) - (bv as i32);
        }
        i += 1;
    }
    0
}

/// Find the first byte equal to `c` in the first `n` bytes of `s`.
///
/// # Safety
/// `s` must be readable for `n` bytes when `n > 0`. This performs one byte
/// load per iteration and never reads a word or a byte beyond that range.
#[inline]
pub unsafe fn memchr_bytes(s: *const u8, c: u8, n: usize) -> *const u8 {
    let mut i = 0usize;
    while i < n {
        let current = s.add(i);
        if *current == c {
            return current;
        }
        i += 1;
    }
    core::ptr::null()
}

// ── C ABI exports ────────────────────────────────────────────────────────────

/// C11 `void *memcpy(void *dest, const void *src, size_t n);`
///
/// Copies exactly `n` bytes. Returns the original `dest`. Overlapping ranges
/// are caller UB. Supports unaligned addresses. Does not allocate.
///
/// # Safety
/// See module-level validity rules.
#[cfg_attr(all(not(test), target_os = "none"), no_mangle)]
pub unsafe extern "C" fn memcpy(dest: *mut u8, src: *const u8, n: usize) -> *mut u8 {
    memcpy_bytes(dest, src, n);
    dest
}

/// C11 `void *memmove(void *dest, const void *src, size_t n);`
///
/// Copies exactly `n` bytes and handles every overlap direction without a
/// temporary heap buffer. Returns the original `dest`. Supports unaligned
/// addresses.
///
/// # Safety
/// See module-level validity rules.
#[cfg_attr(all(not(test), target_os = "none"), no_mangle)]
pub unsafe extern "C" fn memmove(dest: *mut u8, src: *const u8, n: usize) -> *mut u8 {
    memmove_bytes(dest, src, n);
    dest
}

/// C11 `void *memset(void *s, int c, size_t n);`
///
/// Writes the low unsigned 8 bits of `c` to exactly `n` bytes. Returns the
/// original `s`. Supports unaligned addresses.
///
/// # Safety
/// See module-level validity rules.
#[cfg_attr(all(not(test), target_os = "none"), no_mangle)]
pub unsafe extern "C" fn memset(s: *mut u8, c: i32, n: usize) -> *mut u8 {
    // Truncate to the low 8 bits (matches C conversion to `unsigned char`).
    memset_bytes(s, c as u8, n);
    s
}

/// C11 `int memcmp(const void *s1, const void *s2, size_t n);`
///
/// Compares bytes as unsigned values. Returns less than, equal to, or greater
/// than zero. Stops at the first differing byte.
///
/// # Safety
/// See module-level validity rules.
#[cfg_attr(all(not(test), target_os = "none"), no_mangle)]
pub unsafe extern "C" fn memcmp(s1: *const u8, s2: *const u8, n: usize) -> i32 {
    memcmp_bytes(s1, s2, n)
}

/// C11 `void *memchr(const void *s, int c, size_t n);`
///
/// Searches exactly the first `n` bytes using the low unsigned 8 bits of `c`.
/// Returns the first matching address, or null when no byte matches. Supports
/// unaligned addresses and does not read past `n`.
///
/// # Safety
/// See module-level validity rules.
#[cfg_attr(all(not(test), target_os = "none"), no_mangle)]
pub unsafe extern "C" fn memchr(s: *const u8, c: i32, n: usize) -> *mut u8 {
    memchr_bytes(s, c as u8, n) as *mut u8
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use std::vec::Vec;

    #[cfg(target_os = "linux")]
    mod host {
        use core::ffi::c_void;

        #[link(name = "c")]
        extern "C" {
            #[link_name = "getpagesize"]
            pub fn getpagesize() -> i32;
            #[link_name = "mmap"]
            pub fn mmap(
                addr: *mut c_void,
                length: usize,
                prot: i32,
                flags: i32,
                fd: i32,
                offset: isize,
            ) -> *mut c_void;
            #[link_name = "mprotect"]
            pub fn mprotect(addr: *mut c_void, length: usize, prot: i32) -> i32;
            #[link_name = "munmap"]
            pub fn munmap(addr: *mut c_void, length: usize) -> i32;
            #[link_name = "memcpy"]
            pub fn memcpy(dst: *mut u8, src: *const u8, n: usize) -> *mut u8;
            #[link_name = "memmove"]
            pub fn memmove(dst: *mut u8, src: *const u8, n: usize) -> *mut u8;
            #[link_name = "memset"]
            pub fn memset(dst: *mut u8, c: i32, n: usize) -> *mut u8;
            #[link_name = "memcmp"]
            pub fn memcmp(a: *const u8, b: *const u8, n: usize) -> i32;
            #[link_name = "memchr"]
            pub fn memchr(s: *const u8, c: i32, n: usize) -> *mut u8;
        }

        pub const PROT_NONE: i32 = 0;
        pub const PROT_READ: i32 = 1;
        pub const PROT_WRITE: i32 = 2;
        pub const MAP_PRIVATE: i32 = 2;
        pub const MAP_ANONYMOUS: i32 = 0x20;
        pub const MAP_FAILED: *mut c_void = !0usize as *mut c_void;
    }

    fn canary_buf(len: usize, fill: u8) -> Vec<u8> {
        // 16-byte canaries on each side.
        let mut v = Vec::with_capacity(len + 32);
        v.extend(std::iter::repeat(0xA5).take(16));
        v.extend(std::iter::repeat(fill).take(len));
        v.extend(std::iter::repeat(0x5A).take(16));
        v
    }

    fn assert_canaries(buf: &[u8], len: usize) {
        assert!(buf[..16].iter().all(|&b| b == 0xA5));
        assert!(buf[16 + len..].iter().all(|&b| b == 0x5A));
    }

    fn region(buf: &mut [u8], len: usize) -> &mut [u8] {
        &mut buf[16..16 + len]
    }

    #[test]
    fn memcpy_length_zero_no_access() {
        unsafe {
            let mut dst = [0x11u8; 4];
            let src = [0x22u8; 4];
            let ret = memcpy(dst.as_mut_ptr(), src.as_ptr(), 0);
            assert_eq!(ret, dst.as_mut_ptr());
            assert_eq!(dst, [0x11; 4]);
        }
    }

    #[test]
    fn memcpy_lengths_and_alignment() {
        for &len in &[
            0usize, 1, 7, 8, 9, 15, 16, 17, 31, 32, 33, 63, 64, 65, 4095, 4096, 4097,
        ] {
            for src_off in 0..8usize {
                for dst_off in 0..8usize {
                    let mut src_storage = canary_buf(len + 16, 0);
                    let mut dst_storage = canary_buf(len + 16, 0xEE);
                    // Fill source pattern.
                    for i in 0..(len + 16) {
                        src_storage[16 + i] = (i.wrapping_mul(17).wrapping_add(src_off)) as u8;
                    }
                    let src_ptr = unsafe { src_storage.as_ptr().add(16 + src_off) };
                    let dst_ptr = unsafe { dst_storage.as_mut_ptr().add(16 + dst_off) };
                    unsafe {
                        let ret = memcpy(dst_ptr, src_ptr, len);
                        assert_eq!(ret, dst_ptr);
                        for i in 0..len {
                            assert_eq!(
                                *dst_ptr.add(i),
                                *src_ptr.add(i),
                                "len={len} src_off={src_off} dst_off={dst_off} i={i}"
                            );
                        }
                    }
                    // Canaries on the outer padded buffers (only check when
                    // offsets leave the canary region intact for exact-fit).
                    if src_off == 0 && dst_off == 0 {
                        assert_canaries(&src_storage, len + 16);
                        // Destination canary: only the leading 16 bytes are
                        // guaranteed untouched when dst_off == 0 and we wrote
                        // into [16, 16+len). Trailing canary starts at 16+len+16.
                        assert!(dst_storage[..16].iter().all(|&b| b == 0xA5));
                        assert!(dst_storage[16 + len + 16..].iter().all(|&b| b == 0x5A));
                    }
                }
            }
        }
    }

    #[test]
    fn memcpy_same_pointer_zero_and_one() {
        // n==0 is always safe; n>0 with identical pointers is technically
        // overlapping (UB for memcpy) — we only exercise n==0 here.
        unsafe {
            let mut buf = [1u8, 2, 3];
            let p = buf.as_mut_ptr();
            assert_eq!(memcpy(p, p, 0), p);
            assert_eq!(buf, [1, 2, 3]);
        }
    }

    #[test]
    fn memmove_no_overlap_and_same_pointer() {
        unsafe {
            let src = [1u8, 2, 3, 4, 5, 6, 7, 8];
            let mut dst = [0u8; 8];
            let ret = memmove(dst.as_mut_ptr(), src.as_ptr(), 8);
            assert_eq!(ret, dst.as_mut_ptr());
            assert_eq!(dst, src);

            let mut same = [9u8, 8, 7, 6];
            let p = same.as_mut_ptr();
            assert_eq!(memmove(p, p, 4), p);
            assert_eq!(same, [9, 8, 7, 6]);
        }
    }

    #[test]
    fn memmove_overlap_matrix() {
        // Representative lengths and every practical overlap distance.
        for &len in &[1usize, 2, 3, 7, 8, 9, 15, 16, 17, 32, 64, 100] {
            // Destination after source (must copy backward when overlapping).
            for dist in 1..=len {
                let mut buf = (0..len * 2)
                    .map(|i| (i as u8).wrapping_add(1))
                    .collect::<Vec<_>>();
                let expected: Vec<u8> = buf[..len].to_vec();
                unsafe {
                    let src = buf.as_ptr();
                    let dst = buf.as_mut_ptr().add(dist);
                    let ret = memmove(dst, src, len);
                    assert_eq!(ret, dst);
                    for i in 0..len {
                        assert_eq!(
                            *dst.add(i),
                            expected[i],
                            "dest-after src len={len} dist={dist} i={i}"
                        );
                    }
                }
            }
            // Destination before source (forward copy).
            for dist in 1..=len {
                let mut buf = (0..len * 2)
                    .map(|i| (i as u8).wrapping_mul(3))
                    .collect::<Vec<_>>();
                // Place source at offset `dist`, dest at 0.
                for i in 0..len {
                    buf[dist + i] = (i as u8).wrapping_add(0x40);
                }
                let expected: Vec<u8> = buf[dist..dist + len].to_vec();
                unsafe {
                    let src = buf.as_ptr().add(dist);
                    let dst = buf.as_mut_ptr();
                    let ret = memmove(dst, src, len);
                    assert_eq!(ret, dst);
                    for i in 0..len {
                        assert_eq!(
                            *dst.add(i),
                            expected[i],
                            "dest-before src len={len} dist={dist} i={i}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn memmove_one_byte_and_full_overlap() {
        unsafe {
            let mut buf = [1u8, 2, 3, 4, 5];
            // one-byte overlap forward-style (dest after src by 1)
            memmove(buf.as_mut_ptr().add(1), buf.as_ptr(), 4);
            assert_eq!(buf, [1, 1, 2, 3, 4]);

            let mut buf2 = [10u8, 20, 30, 40, 50];
            // full overlap same pointer
            memmove(buf2.as_mut_ptr(), buf2.as_ptr(), 5);
            assert_eq!(buf2, [10, 20, 30, 40, 50]);
        }
    }

    #[test]
    fn memset_values_and_canaries() {
        for &len in &[0usize, 1, 8, 16, 32, 64, 4096] {
            for &c in &[0i32, 0x7f, 0x80, 0xff, 0x100, 0x1ff, -1, -128, 256 + 0x5a] {
                let mut storage = canary_buf(len, 0x00);
                let expected = c as u8;
                unsafe {
                    let p = region(&mut storage, len).as_mut_ptr();
                    let ret = memset(p, c, len);
                    assert_eq!(ret, p);
                    for i in 0..len {
                        assert_eq!(*p.add(i), expected);
                    }
                }
                assert_canaries(&storage, len);
            }
        }
    }

    #[test]
    fn memcmp_equal_and_differences() {
        unsafe {
            let a = [1u8, 2, 3, 4];
            let b = [1u8, 2, 3, 4];
            assert_eq!(memcmp(a.as_ptr(), b.as_ptr(), 4), 0);
            assert_eq!(memcmp(a.as_ptr(), b.as_ptr(), 0), 0);

            // first-byte difference
            let c = [0u8, 2, 3, 4];
            assert!(memcmp(a.as_ptr(), c.as_ptr(), 4) > 0);
            assert!(memcmp(c.as_ptr(), a.as_ptr(), 4) < 0);

            // middle difference
            let d = [1u8, 2, 9, 4];
            assert!(memcmp(a.as_ptr(), d.as_ptr(), 4) < 0);

            // last-byte difference
            let e = [1u8, 2, 3, 0];
            assert!(memcmp(a.as_ptr(), e.as_ptr(), 4) > 0);
        }
    }

    #[test]
    fn memcmp_unsigned_high_bytes() {
        unsafe {
            // 0x80 must compare greater than 0x7f as unsigned, not as signed char.
            let hi = [0x80u8];
            let lo = [0x7fu8];
            assert!(memcmp(hi.as_ptr(), lo.as_ptr(), 1) > 0);
            assert!(memcmp(lo.as_ptr(), hi.as_ptr(), 1) < 0);

            let ff = [0xffu8];
            let zero = [0x00u8];
            assert!(memcmp(ff.as_ptr(), zero.as_ptr(), 1) > 0);

            // multi-byte with high bytes
            let a = [0x00u8, 0xff];
            let b = [0x00u8, 0x7f];
            assert!(memcmp(a.as_ptr(), b.as_ptr(), 2) > 0);
        }
    }

    #[test]
    fn memchr_bounds_unsigned_values_and_canaries() {
        for &len in &[0usize, 1, 7, 8, 9, 15, 16, 17, 63, 64, 65, 4096] {
            let mut storage = canary_buf(len, 0x11);
            let bytes = region(&mut storage, len);
            if len != 0 {
                bytes[0] = 0x11;
            }
            if len > 1 {
                bytes[len - 1] = 0xff;
            }
            let base = bytes.as_ptr();
            unsafe {
                assert_eq!(memchr(base, 0x22, len), core::ptr::null_mut());
                assert_eq!(
                    memchr(base, 0x11, len),
                    if len == 0 {
                        core::ptr::null_mut()
                    } else {
                        base as *mut u8
                    }
                );
                assert_eq!(
                    memchr(base, -1, len),
                    if len <= 1 {
                        core::ptr::null_mut()
                    } else {
                        base.add(len - 1) as *mut u8
                    }
                );
                assert_eq!(memchr(base, 0x11, 0), core::ptr::null_mut());
            }
            assert_canaries(&storage, len);
        }
    }

    #[test]
    fn zero_length_accepts_null_without_access() {
        unsafe {
            let null = core::ptr::null_mut::<u8>();
            assert_eq!(memcpy(null, core::ptr::null(), 0), null);
            assert_eq!(memmove(null, core::ptr::null(), 0), null);
            assert_eq!(memset(null, 0x5a, 0), null);
            assert_eq!(memcmp(core::ptr::null(), core::ptr::null(), 0), 0);
            assert_eq!(memchr(core::ptr::null(), 0x5a, 0), null);
        }
    }

    #[test]
    fn helpers_match_c_abi() {
        unsafe {
            let src = [9u8, 8, 7, 6];
            let mut d1 = [0u8; 4];
            let mut d2 = [0u8; 4];
            memcpy(d1.as_mut_ptr(), src.as_ptr(), 4);
            memcpy_bytes(d2.as_mut_ptr(), src.as_ptr(), 4);
            assert_eq!(d1, d2);

            memset(d1.as_mut_ptr(), 0xAB, 4);
            memset_bytes(d2.as_mut_ptr(), 0xAB, 4);
            assert_eq!(d1, d2);

            assert_eq!(
                memcmp(d1.as_ptr(), d2.as_ptr(), 4),
                memcmp_bytes(d1.as_ptr(), d2.as_ptr(), 4)
            );
            assert_eq!(
                memchr(d1.as_ptr(), 0xAB, 4),
                memchr_bytes(d1.as_ptr(), 0xAB, 4) as *mut u8
            );
        }
    }

    /// C ABI signature probe: call through `extern "C"` function pointers so
    /// the SysV x86_64 calling convention and return types are exercised.
    #[test]
    fn c_abi_function_pointer_probe() {
        type MemcpyFn = unsafe extern "C" fn(*mut u8, *const u8, usize) -> *mut u8;
        type MemmoveFn = unsafe extern "C" fn(*mut u8, *const u8, usize) -> *mut u8;
        type MemsetFn = unsafe extern "C" fn(*mut u8, i32, usize) -> *mut u8;
        type MemcmpFn = unsafe extern "C" fn(*const u8, *const u8, usize) -> i32;
        type MemchrFn = unsafe extern "C" fn(*const u8, i32, usize) -> *mut u8;

        let memcpy_fp: MemcpyFn = memcpy;
        let memmove_fp: MemmoveFn = memmove;
        let memset_fp: MemsetFn = memset;
        let memcmp_fp: MemcmpFn = memcmp;
        let memchr_fp: MemchrFn = memchr;

        unsafe {
            let src = [1u8, 2, 3, 4, 5, 6, 7, 8];
            let mut dst = [0u8; 8];
            assert_eq!(
                memcpy_fp(dst.as_mut_ptr(), src.as_ptr(), 8),
                dst.as_mut_ptr()
            );
            assert_eq!(dst, src);

            let mut overlap = [10u8, 20, 30, 40, 50];
            assert_eq!(
                memmove_fp(overlap.as_mut_ptr().add(1), overlap.as_ptr(), 4),
                overlap.as_mut_ptr().add(1)
            );
            assert_eq!(overlap, [10, 10, 20, 30, 40]);

            let mut fill = [0u8; 4];
            assert_eq!(memset_fp(fill.as_mut_ptr(), 0x1ff, 4), fill.as_mut_ptr());
            assert_eq!(fill, [0xff; 4]);

            assert_eq!(memcmp_fp(src.as_ptr(), src.as_ptr(), 8), 0);
            assert!(memcmp_fp([0x80u8].as_ptr(), [0x7fu8].as_ptr(), 1) > 0);
            assert_eq!(
                memchr_fp(src.as_ptr(), 5, 8),
                src.as_ptr().add(4) as *mut u8
            );
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn guard_pages_prevent_memory_overreads() {
        unsafe {
            let page = host::getpagesize() as usize;
            assert!(page >= 1024);
            let mapping = host::mmap(
                core::ptr::null_mut(),
                page * 2,
                host::PROT_READ | host::PROT_WRITE,
                host::MAP_PRIVATE | host::MAP_ANONYMOUS,
                -1,
                0,
            );
            assert_ne!(mapping, host::MAP_FAILED);
            assert_eq!(
                host::mprotect(
                    (mapping as *mut u8).add(page) as *mut _,
                    page,
                    host::PROT_NONE
                ),
                0
            );

            let end = (mapping as *mut u8).add(page - 3);
            *end = b'A';
            *end.add(1) = 0x80;
            *end.add(2) = 0;

            assert_eq!(memchr(end, 0x80, 2), end.add(1));
            assert_eq!(memchr(end, 0, 2), core::ptr::null_mut());
            assert_eq!(memcmp(end, end, 2), 0);
            assert_eq!(crate::string::strlen(end), 2);
            assert_eq!(crate::string::strnlen(end, 0), 0);
            assert_eq!(crate::string::strnlen(end, 2), 2);
            assert_eq!(crate::string::strnlen(end, 3), 2);
            assert_eq!(crate::string::strchr(end, 0), end.add(2));
            assert_eq!(crate::string::strrchr(end, 0x80), end.add(1));

            assert_eq!(host::munmap(mapping, page * 2), 0);
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn host_libc_differential_memory_matrix() {
        let mut seed = 0x4d59_5df4_d0f3_3173u64;
        for _ in 0..256 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let len = (seed as usize) % 129;
            let src_off = ((seed >> 8) as usize) & 7;
            let dst_off = ((seed >> 16) as usize) & 7;
            let move_src = ((seed >> 24) as usize) % 32;
            let move_dst = ((seed >> 32) as usize) % 32;

            let mut source = [0u8; 160];
            let mut ours = [0xCCu8; 160];
            let mut expected = [0xCCu8; 160];
            for (i, byte) in source.iter_mut().enumerate() {
                *byte = (i as u8).wrapping_mul(29).wrapping_add(seed as u8);
            }

            unsafe {
                memcpy(
                    ours.as_mut_ptr().add(16 + dst_off),
                    source.as_ptr().add(16 + src_off),
                    len,
                );
                host::memcpy(
                    expected.as_mut_ptr().add(16 + dst_off),
                    source.as_ptr().add(16 + src_off),
                    len,
                );
            }
            assert_eq!(
                ours, expected,
                "memcpy len={len} src_off={src_off} dst_off={dst_off}"
            );

            let mut ours_move = source;
            let mut expected_move = source;
            unsafe {
                memmove(
                    ours_move.as_mut_ptr().add(move_dst),
                    ours_move.as_ptr().add(move_src),
                    len,
                );
                host::memmove(
                    expected_move.as_mut_ptr().add(move_dst),
                    expected_move.as_ptr().add(move_src),
                    len,
                );
            }
            assert_eq!(
                ours_move, expected_move,
                "memmove len={len} src={move_src} dst={move_dst}"
            );

            let fill = (seed >> 40) as i32;
            unsafe {
                memset(ours.as_mut_ptr().add(16 + dst_off), fill, len);
                host::memset(expected.as_mut_ptr().add(16 + dst_off), fill, len);
            }
            assert_eq!(ours, expected, "memset len={len} dst_off={dst_off}");

            unsafe {
                let a = source.as_ptr().add(16 + src_off);
                let b = expected_move.as_ptr().add(16 + dst_off);
                assert_eq!(memcmp(a, b, len).cmp(&0), host::memcmp(a, b, len).cmp(&0));
                let needle = (seed >> 48) as i32;
                let ours_at = memchr(a, needle, len);
                let host_at = host::memchr(a, needle, len);
                assert_eq!(
                    if ours_at.is_null() {
                        None
                    } else {
                        Some(ours_at as usize - a as usize)
                    },
                    if host_at.is_null() {
                        None
                    } else {
                        Some(host_at as usize - a as usize)
                    }
                );
            }
        }
    }
}
