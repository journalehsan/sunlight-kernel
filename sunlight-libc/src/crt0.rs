//! Program startup ABI documentation and raw argv helpers.
//!
//! # SunlightOS `_start` ABI
//!
//! The kernel places `argc` / `argv` / `envp` in two complementary ways at
//! process entry.  Userland programs may use either; the register form is the
//! native Sunlight convention.
//!
//! ## Register convention (native Sunlight)
//!
//! | Register | Value                                            |
//! |----------|--------------------------------------------------|
//! | `rdi`    | `argc` as `u64`                                 |
//! | `rsi`    | `argv`: `*const *const u8`, NULL-terminated      |
//! | `rdx`    | `envp`: `*const *const u8`, NULL-terminated, or 0 |
//!
//! ## Stack layout (SysV x86_64 ABI)
//!
//! At entry `rsp` points to:
//!
//! ```text
//! [rsp +  0]              argc              (u64)
//! [rsp +  8]              argv[0]           (*const u8, NUL-terminated)
//! ...
//! [rsp + 8*(argc+1)]      NULL              (argv terminator)
//! [rsp + 8*(argc+2)]      envp[0]           (*const u8, NUL-terminated)
//! ...
//! [rsp + ...]             NULL              (envp terminator)
//! ```
//!
//! `rsp` is 16-byte aligned on entry; `argc` at `[rsp]` disturbs alignment
//! by 8 bytes relative to a CALL frame, which is the expected SysV layout.
//!
//! ## String encoding
//!
//! All `argv` and `envp` strings are NUL-terminated UTF-8 byte slices.
//! The kernel owns the memory; programs must **not** free or mutate it.
//!
//! ## Minimal `_start` pattern
//!
//! ```rust
//! #[no_mangle]
//! pub extern "C" fn _start(argc: u64, argv: *const *const u8) -> ! {
//!     // parse args, call application logic, then:
//!     sunlight_libc::exit(0);
//! }
//! ```
//!
//! Do **not** return from `_start`. The kernel does not provide a return
//! address; returning would jump to garbage and fault.

/// Collect the raw `argv` pointers into `out`.
///
/// Returns the number of entries written (≤ `out.len()` and ≤ `argc`).
/// Stops early if a null pointer is encountered inside the array.
///
/// # Safety
/// `argc` and `argv` must be the values received by `_start` from the kernel.
/// The returned pointers remain valid for the lifetime of the process.
pub unsafe fn collect_raw_args(argc: u64, argv: *const *const u8, out: &mut [*const u8]) -> usize {
    if argv.is_null() {
        return 0;
    }
    let count = (argc as usize).min(out.len());
    for i in 0..count {
        let ptr = *argv.add(i);
        if ptr.is_null() {
            return i;
        }
        out[i] = ptr;
    }
    count
}

/// Collect bounded UTF-8 argv strings into `out`.
///
/// Invalid UTF-8 is represented as an empty string, matching the historical
/// native utility startup convention. `max_len` bounds each NUL-terminated
/// string scan so a malformed pointer cannot cause an unbounded read.
///
/// # Safety
/// `argc` and `argv` must be the values received by `_start` from the kernel,
/// and each advertised argv pointer must be readable for at least `max_len`
/// bytes or until its terminating NUL.
pub unsafe fn collect_utf8_args<'a>(
    argc: u64,
    argv: *const *const u8,
    out: &mut [&'a str],
    max_len: usize,
) -> usize {
    if argv.is_null() {
        return 0;
    }

    let count = (argc as usize).min(out.len());
    for i in 0..count {
        let ptr = *argv.add(i);
        if ptr.is_null() {
            return i;
        }
        let len = cstr_len(ptr, max_len);
        let bytes = core::slice::from_raw_parts(ptr, len);
        out[i] = core::str::from_utf8(bytes).unwrap_or("");
    }
    count
}

/// Return the byte length of a NUL-terminated C string, up to `max` bytes.
///
/// # Safety
/// `ptr` must point to a readable region of at least `max` bytes (or be
/// NUL-terminated before `max` bytes are reached).
pub unsafe fn cstr_len(ptr: *const u8, max: usize) -> usize {
    let mut len = 0;
    while len < max && *ptr.add(len) != 0 {
        len += 1;
    }
    len
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_bounded_utf8_arguments() {
        let argv0 = b"echo\0";
        let argv1 = "sun☀".as_bytes();
        let mut argv1_nul = [0u8; 8];
        argv1_nul[..argv1.len()].copy_from_slice(argv1);
        argv1_nul[argv1.len()] = 0;
        let mut pointers = [argv0.as_ptr(), argv1_nul.as_ptr(), core::ptr::null()];
        let mut out = [""; 3];

        let count = unsafe {
            collect_utf8_args(
                pointers.len() as u64,
                pointers.as_mut_ptr(),
                &mut out,
                argv1_nul.len(),
            )
        };

        assert_eq!(count, 2);
        assert_eq!(&out[..count], &["echo", "sun☀"]);
    }

    #[test]
    fn stops_at_null_and_replaces_invalid_utf8() {
        let argv0 = b"echo\0";
        let invalid = [0xff, 0];
        let mut pointers = [argv0.as_ptr(), invalid.as_ptr(), core::ptr::null()];
        let mut out = [""; 3];

        let count = unsafe {
            collect_utf8_args(
                pointers.len() as u64,
                pointers.as_mut_ptr(),
                &mut out,
                argv0.len(),
            )
        };

        assert_eq!(count, 2);
        assert_eq!(&out[..count], &["echo", ""]);
    }
}
