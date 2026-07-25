//! Freestanding C string primitives (`strlen`, `strnlen`, `strcmp`, `strncmp`,
//! `strchr`, `strrchr`).
//!
//! # Symbol ownership
//!
//! Strong C ABI definitions for the baseline string operations required by
//! compiler-generated code, C ABI consumers, and freestanding runtimes.
//! `compiler_builtins` already provides a weak `strlen`; our strong symbol
//! overrides it. The remaining functions are supplied here.
//!
//! # Compiler-recursion safety
//!
//! The engines use explicit byte loops and do not call Rust pointer-copy or
//! slice helpers that could lower to an exported libc primitive. The host
//! proof script inspects optimized freestanding code for such dependencies.
//!
//! # Semantics
//!
//! All comparisons treat bytes as **unsigned** (`unsigned char` in C).
//!
//! # Caller validity
//!
//! - `strlen`: `s` must point to a readable NUL-terminated sequence.
//! - `strnlen`: may read at most `max_len` bytes from `s`; if no NUL appears
//!   within that bound, returns `max_len` without reading further.
//! - `strcmp`: both strings must be readable up through their terminators.
//! - `strncmp`: may read at most `n` bytes from each side; if `n == 0`, no
//!   access is performed.
//! - `strchr` / `strrchr`: `s` must point to a readable NUL-terminated
//!   sequence. Searching for `0` returns the terminator.
//!
//! # Host tests
//!
//! C `#[no_mangle]` exports are freestanding-only (`target_os = "none"`) so host unit tests do not
//! collide with the system libc.

// ── Internal helpers ─────────────────────────────────────────────────────────

/// Number of bytes before the first NUL in `s` (terminator not included).
///
/// # Safety
/// `s` must point to a readable NUL-terminated sequence.
#[inline]
pub unsafe fn strlen_bytes(s: *const u8) -> usize {
    let mut n = 0usize;
    while *s.add(n) != 0 {
        n += 1;
    }
    n
}

/// Bounded length: never reads beyond `max_len` bytes.
///
/// # Safety
/// `s` must be readable for `min(max_len, strlen(s)+1)` bytes.
#[inline]
pub unsafe fn strnlen_bytes(s: *const u8, max_len: usize) -> usize {
    let mut n = 0usize;
    while n < max_len && *s.add(n) != 0 {
        n += 1;
    }
    n
}

/// Unsigned-byte string comparison (unbounded).
///
/// # Safety
/// Both strings must be readable through their NULs.
#[inline]
pub unsafe fn strcmp_bytes(a: *const u8, b: *const u8) -> i32 {
    let mut i = 0usize;
    loop {
        let av = *a.add(i);
        let bv = *b.add(i);
        if av != bv {
            return (av as i32) - (bv as i32);
        }
        if av == 0 {
            return 0;
        }
        i += 1;
    }
}

/// Unsigned-byte string comparison, limited to `n` bytes.
///
/// # Safety
/// Each side must be readable for up to `n` bytes (or through its NUL,
/// whichever comes first). When `n == 0`, no access is performed.
#[inline]
pub unsafe fn strncmp_bytes(a: *const u8, b: *const u8, n: usize) -> i32 {
    let mut i = 0usize;
    while i < n {
        let av = *a.add(i);
        let bv = *b.add(i);
        if av != bv {
            return (av as i32) - (bv as i32);
        }
        if av == 0 {
            return 0;
        }
        i += 1;
    }
    0
}

/// Find the first occurrence of `c`, including a matching terminator.
///
/// # Safety
/// `s` must point to a readable NUL-terminated sequence.
#[inline]
pub unsafe fn strchr_bytes(s: *const u8, c: u8) -> *const u8 {
    let mut i = 0usize;
    loop {
        let current = s.add(i);
        let byte = *current;
        if byte == c {
            return current;
        }
        if byte == 0 {
            return core::ptr::null();
        }
        i += 1;
    }
}

/// Find the last occurrence of `c`, including a matching terminator.
///
/// # Safety
/// `s` must point to a readable NUL-terminated sequence.
#[inline]
pub unsafe fn strrchr_bytes(s: *const u8, c: u8) -> *const u8 {
    let mut i = 0usize;
    let mut last = core::ptr::null();
    loop {
        let current = s.add(i);
        let byte = *current;
        if byte == c {
            last = current;
        }
        if byte == 0 {
            return last;
        }
        i += 1;
    }
}

// ── C ABI exports ────────────────────────────────────────────────────────────

/// C11 `size_t strlen(const char *s);`
///
/// # Safety
/// See module docs.
#[cfg_attr(all(not(test), target_os = "none"), no_mangle)]
pub unsafe extern "C" fn strlen(s: *const u8) -> usize {
    strlen_bytes(s)
}

/// POSIX `size_t strnlen(const char *s, size_t maxlen);`
///
/// # Safety
/// See module docs.
#[cfg_attr(all(not(test), target_os = "none"), no_mangle)]
pub unsafe extern "C" fn strnlen(s: *const u8, max_len: usize) -> usize {
    strnlen_bytes(s, max_len)
}

/// C11 `int strcmp(const char *s1, const char *s2);`
///
/// # Safety
/// See module docs.
#[cfg_attr(all(not(test), target_os = "none"), no_mangle)]
pub unsafe extern "C" fn strcmp(s1: *const u8, s2: *const u8) -> i32 {
    strcmp_bytes(s1, s2)
}

/// C11 `int strncmp(const char *s1, const char *s2, size_t n);`
///
/// # Safety
/// See module docs.
#[cfg_attr(all(not(test), target_os = "none"), no_mangle)]
pub unsafe extern "C" fn strncmp(s1: *const u8, s2: *const u8, n: usize) -> i32 {
    strncmp_bytes(s1, s2, n)
}

/// C11 `char *strchr(const char *s, int c);`
///
/// Searches with unsigned-byte conversion. A search for `0` returns the
/// address of the terminating NUL byte.
///
/// # Safety
/// See module-level validity rules.
#[cfg_attr(all(not(test), target_os = "none"), no_mangle)]
pub unsafe extern "C" fn strchr(s: *const u8, c: i32) -> *mut u8 {
    strchr_bytes(s, c as u8) as *mut u8
}

/// C11 `char *strrchr(const char *s, int c);`
///
/// Searches with unsigned-byte conversion and returns the final match. A
/// search for `0` returns the address of the terminating NUL byte.
///
/// # Safety
/// See module-level validity rules.
#[cfg_attr(all(not(test), target_os = "none"), no_mangle)]
pub unsafe extern "C" fn strrchr(s: *const u8, c: i32) -> *mut u8 {
    strrchr_bytes(s, c as u8) as *mut u8
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;

    #[cfg(target_os = "linux")]
    mod host {
        #[link(name = "c")]
        extern "C" {
            #[link_name = "strlen"]
            pub fn strlen(s: *const u8) -> usize;
            #[link_name = "strnlen"]
            pub fn strnlen(s: *const u8, n: usize) -> usize;
            #[link_name = "strcmp"]
            pub fn strcmp(a: *const u8, b: *const u8) -> i32;
            #[link_name = "strncmp"]
            pub fn strncmp(a: *const u8, b: *const u8, n: usize) -> i32;
            #[link_name = "strchr"]
            pub fn strchr(s: *const u8, c: i32) -> *mut u8;
            #[link_name = "strrchr"]
            pub fn strrchr(s: *const u8, c: i32) -> *mut u8;
        }
    }

    #[test]
    fn strlen_basic() {
        unsafe {
            assert_eq!(strlen(b"\0".as_ptr()), 0);
            assert_eq!(strlen(b"a\0".as_ptr()), 1);
            assert_eq!(strlen(b"hello\0".as_ptr()), 5);
            assert_eq!(strlen(b"hello\0world\0".as_ptr()), 5);
        }
    }

    #[test]
    fn strnlen_bounds() {
        unsafe {
            let s = b"abcdef\0";
            assert_eq!(strnlen(s.as_ptr(), 0), 0);
            assert_eq!(strnlen(s.as_ptr(), 3), 3);
            assert_eq!(strnlen(s.as_ptr(), 6), 6);
            assert_eq!(strnlen(s.as_ptr(), 7), 6);
            assert_eq!(strnlen(s.as_ptr(), 100), 6);

            // No terminator inside the bound: must not read past max_len.
            let unterminated = [b'x', b'y', b'z'];
            assert_eq!(strnlen(unterminated.as_ptr(), 0), 0);
            assert_eq!(strnlen(unterminated.as_ptr(), 2), 2);
            assert_eq!(strnlen(unterminated.as_ptr(), 3), 3);
        }
    }

    #[test]
    fn strcmp_ordering_and_prefixes() {
        unsafe {
            assert_eq!(strcmp(b"\0".as_ptr(), b"\0".as_ptr()), 0);
            assert_eq!(strcmp(b"abc\0".as_ptr(), b"abc\0".as_ptr()), 0);
            assert!(strcmp(b"abc\0".as_ptr(), b"abd\0".as_ptr()) < 0);
            assert!(strcmp(b"abd\0".as_ptr(), b"abc\0".as_ptr()) > 0);
            // prefix: shorter string is less when common prefix matches
            assert!(strcmp(b"ab\0".as_ptr(), b"abc\0".as_ptr()) < 0);
            assert!(strcmp(b"abc\0".as_ptr(), b"ab\0".as_ptr()) > 0);
            // empty
            assert!(strcmp(b"\0".as_ptr(), b"a\0".as_ptr()) < 0);
            assert!(strcmp(b"a\0".as_ptr(), b"\0".as_ptr()) > 0);
        }
    }

    #[test]
    fn strcmp_unsigned_high_bytes() {
        unsafe {
            assert!(strcmp([0x80u8, 0].as_ptr(), [0x7fu8, 0].as_ptr()) > 0);
            assert!(strcmp([0xffu8, 0].as_ptr(), [0x00u8, 0].as_ptr()) > 0);
        }
    }

    #[test]
    fn strncmp_n_zero_and_bounds() {
        unsafe {
            // n == 0: equal, no access
            assert_eq!(strncmp(b"a\0".as_ptr(), b"b\0".as_ptr(), 0), 0);

            assert_eq!(strncmp(b"abc\0".as_ptr(), b"abd\0".as_ptr(), 2), 0);
            assert!(strncmp(b"abc\0".as_ptr(), b"abd\0".as_ptr(), 3) < 0);

            // equal within n even if later bytes differ
            assert_eq!(strncmp(b"abX\0".as_ptr(), b"abY\0".as_ptr(), 2), 0);

            // prefixes within bound
            assert!(strncmp(b"ab\0".as_ptr(), b"abc\0".as_ptr(), 3) < 0);
            assert_eq!(strncmp(b"ab\0".as_ptr(), b"abc\0".as_ptr(), 2), 0);

            // unterminated buffers compared only up to n
            let a = [b'x', b'y', 0x80u8];
            let b = [b'x', b'y', 0x7fu8];
            assert_eq!(strncmp(a.as_ptr(), b.as_ptr(), 2), 0);
            assert!(strncmp(a.as_ptr(), b.as_ptr(), 3) > 0);
        }
    }

    #[test]
    fn strchr_and_strrchr_find_first_last_and_terminator() {
        unsafe {
            let s = b"ab\x80ba\0tail";
            let base = s.as_ptr();
            assert_eq!(strchr(base, b'a' as i32), base as *mut u8);
            assert_eq!(strrchr(base, b'a' as i32), base.add(4) as *mut u8);
            assert_eq!(strchr(base, b'b' as i32), base.add(1) as *mut u8);
            assert_eq!(strrchr(base, b'b' as i32), base.add(3) as *mut u8);
            assert_eq!(strchr(base, -128), base.add(2) as *mut u8);
            assert_eq!(strrchr(base, 0x180), base.add(2) as *mut u8);
            assert_eq!(strchr(base, b'z' as i32), core::ptr::null_mut());
            assert_eq!(strrchr(base, b'z' as i32), core::ptr::null_mut());
            assert_eq!(strchr(base, 0), base.add(5) as *mut u8);
            assert_eq!(strrchr(base, 0), base.add(5) as *mut u8);
        }
    }

    #[test]
    fn helpers_match_c_abi() {
        unsafe {
            let s = b"sunlight\0";
            assert_eq!(strlen(s.as_ptr()), strlen_bytes(s.as_ptr()));
            assert_eq!(strnlen(s.as_ptr(), 4), strnlen_bytes(s.as_ptr(), 4));
            assert_eq!(
                strcmp(s.as_ptr(), b"sun\0".as_ptr()),
                strcmp_bytes(s.as_ptr(), b"sun\0".as_ptr())
            );
            assert_eq!(
                strncmp(s.as_ptr(), b"sun\0".as_ptr(), 3),
                strncmp_bytes(s.as_ptr(), b"sun\0".as_ptr(), 3)
            );
            assert_eq!(
                strchr(s.as_ptr(), b'l' as i32),
                strchr_bytes(s.as_ptr(), b'l') as *mut u8
            );
            assert_eq!(
                strrchr(s.as_ptr(), b'n' as i32),
                strrchr_bytes(s.as_ptr(), b'n') as *mut u8
            );
        }
    }

    /// C ABI signature probe: call through `extern "C"` function pointers so
    /// the SysV x86_64 calling convention and return types are exercised the
    /// same way a C translation unit would.
    #[test]
    fn c_abi_function_pointer_probe() {
        type StrlenFn = unsafe extern "C" fn(*const u8) -> usize;
        type StrnlenFn = unsafe extern "C" fn(*const u8, usize) -> usize;
        type StrcmpFn = unsafe extern "C" fn(*const u8, *const u8) -> i32;
        type StrncmpFn = unsafe extern "C" fn(*const u8, *const u8, usize) -> i32;
        type StrchrFn = unsafe extern "C" fn(*const u8, i32) -> *mut u8;
        type StrrchrFn = unsafe extern "C" fn(*const u8, i32) -> *mut u8;

        let strlen_fp: StrlenFn = strlen;
        let strnlen_fp: StrnlenFn = strnlen;
        let strcmp_fp: StrcmpFn = strcmp;
        let strncmp_fp: StrncmpFn = strncmp;
        let strchr_fp: StrchrFn = strchr;
        let strrchr_fp: StrrchrFn = strrchr;

        unsafe {
            let repeated = b"abca\0";
            assert_eq!(strlen_fp(b"probe\0".as_ptr()), 5);
            assert_eq!(strnlen_fp(b"probe\0".as_ptr(), 3), 3);
            assert_eq!(strcmp_fp(b"aa\0".as_ptr(), b"aa\0".as_ptr()), 0);
            assert!(strcmp_fp(b"ab\0".as_ptr(), b"aa\0".as_ptr()) > 0);
            assert_eq!(strncmp_fp(b"abc\0".as_ptr(), b"abd\0".as_ptr(), 2), 0);
            assert_eq!(
                strchr_fp(repeated.as_ptr(), b'a' as i32),
                repeated.as_ptr() as *mut u8
            );
            assert_eq!(
                strrchr_fp(repeated.as_ptr(), b'a' as i32),
                repeated.as_ptr().add(3) as *mut u8
            );
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn host_libc_differential_string_matrix() {
        let mut seed = 0x3c6e_f372_fe94_f82bu64;
        for _ in 0..256 {
            seed = seed
                .wrapping_mul(2862933555777941757)
                .wrapping_add(3037000493);
            let left_len = (seed as usize) % 96;
            let right_len = ((seed >> 8) as usize) % 96;
            let limit = ((seed >> 16) as usize) % 112;
            let needle = (seed >> 24) as i32;
            let mut left = [0u8; 97];
            let mut right = [0u8; 97];
            for i in 0..left_len {
                left[i] = ((seed >> (i & 31)) as u8).wrapping_rem(255).wrapping_add(1);
            }
            for i in 0..right_len {
                right[i] = ((seed.rotate_left(i as u32) >> 9) as u8)
                    .wrapping_rem(255)
                    .wrapping_add(1);
            }
            left[left_len] = 0;
            right[right_len] = 0;

            unsafe {
                assert_eq!(strlen(left.as_ptr()), host::strlen(left.as_ptr()));
                assert_eq!(
                    strnlen(left.as_ptr(), limit),
                    host::strnlen(left.as_ptr(), limit)
                );
                assert_eq!(
                    strcmp(left.as_ptr(), right.as_ptr()).cmp(&0),
                    host::strcmp(left.as_ptr(), right.as_ptr()).cmp(&0)
                );
                assert_eq!(
                    strncmp(left.as_ptr(), right.as_ptr(), limit).cmp(&0),
                    host::strncmp(left.as_ptr(), right.as_ptr(), limit).cmp(&0)
                );
                let ours_first = strchr(left.as_ptr(), needle);
                let host_first = host::strchr(left.as_ptr(), needle);
                assert_eq!(
                    if ours_first.is_null() {
                        None
                    } else {
                        Some(ours_first as usize - left.as_ptr() as usize)
                    },
                    if host_first.is_null() {
                        None
                    } else {
                        Some(host_first as usize - left.as_ptr() as usize)
                    }
                );
                let ours_last = strrchr(left.as_ptr(), needle);
                let host_last = host::strrchr(left.as_ptr(), needle);
                assert_eq!(
                    if ours_last.is_null() {
                        None
                    } else {
                        Some(ours_last as usize - left.as_ptr() as usize)
                    },
                    if host_last.is_null() {
                        None
                    } else {
                        Some(host_last as usize - left.as_ptr() as usize)
                    }
                );
            }
        }
    }
}
