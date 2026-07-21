//! Freestanding C string primitives (`strlen`, `strnlen`, `strcmp`, `strncmp`).
//!
//! # Symbol ownership
//!
//! Strong C ABI definitions for the baseline string operations required by
//! compiler-generated code, C ABI consumers, and freestanding runtimes.
//! `compiler_builtins` already provides a weak `strlen`; our strong symbol
//! overrides it. `strnlen` / `strcmp` / `strncmp` are not provided by
//! `compiler_builtins` and are supplied here.
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
//!
//! # Host tests
//!
//! `#[no_mangle]` is omitted under `cfg(test)` so host unit tests do not
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

// ── C ABI exports ────────────────────────────────────────────────────────────

/// C11 `size_t strlen(const char *s);`
///
/// # Safety
/// See module docs.
#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn strlen(s: *const u8) -> usize {
    strlen_bytes(s)
}

/// POSIX `size_t strnlen(const char *s, size_t maxlen);`
///
/// # Safety
/// See module docs.
#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn strnlen(s: *const u8, max_len: usize) -> usize {
    strnlen_bytes(s, max_len)
}

/// C11 `int strcmp(const char *s1, const char *s2);`
///
/// # Safety
/// See module docs.
#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn strcmp(s1: *const u8, s2: *const u8) -> i32 {
    strcmp_bytes(s1, s2)
}

/// C11 `int strncmp(const char *s1, const char *s2, size_t n);`
///
/// # Safety
/// See module docs.
#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn strncmp(s1: *const u8, s2: *const u8, n: usize) -> i32 {
    strncmp_bytes(s1, s2, n)
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;

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

        let strlen_fp: StrlenFn = strlen;
        let strnlen_fp: StrnlenFn = strnlen;
        let strcmp_fp: StrcmpFn = strcmp;
        let strncmp_fp: StrncmpFn = strncmp;

        unsafe {
            assert_eq!(strlen_fp(b"probe\0".as_ptr()), 5);
            assert_eq!(strnlen_fp(b"probe\0".as_ptr(), 3), 3);
            assert_eq!(strcmp_fp(b"aa\0".as_ptr(), b"aa\0".as_ptr()), 0);
            assert!(strcmp_fp(b"ab\0".as_ptr(), b"aa\0".as_ptr()) > 0);
            assert_eq!(strncmp_fp(b"abc\0".as_ptr(), b"abd\0".as_ptr(), 2), 0);
        }
    }
}
