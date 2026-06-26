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
