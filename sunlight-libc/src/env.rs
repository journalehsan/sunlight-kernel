//! Environment variable access from the SysV envp pointer.
//!
//! The kernel passes envp in rdx at _start (SysV x86_64 ABI).
//! Call `init(envp)` once from `_start` before any `getenv` call.

use core::sync::atomic::{AtomicUsize, Ordering};

static ENVP: AtomicUsize = AtomicUsize::new(0);

/// Store the envp pointer received from _start (rdx register).
pub fn init(envp: *const *const u8) {
    ENVP.store(envp as usize, Ordering::Relaxed);
}

/// Look up a `KEY=VALUE` environment entry by key bytes.
/// Returns the value bytes (excluding the `=`), or `None`.
/// Lifetime is `'static` because envp lives for the process lifetime.
pub fn getenv_bytes(key: &[u8]) -> Option<&'static [u8]> {
    let ptr = ENVP.load(Ordering::Relaxed) as *const *const u8;
    if ptr.is_null() {
        return None;
    }
    let mut i = 0;
    loop {
        let entry_ptr = unsafe { *ptr.add(i) };
        if entry_ptr.is_null() {
            return None;
        }
        let mut len = 0;
        while unsafe { *entry_ptr.add(len) } != 0 {
            len += 1;
        }
        let entry = unsafe { core::slice::from_raw_parts(entry_ptr, len) };
        if entry.len() > key.len() && entry[key.len()] == b'=' && &entry[..key.len()] == key {
            return Some(&entry[key.len() + 1..]);
        }
        i += 1;
    }
}

/// Look up an environment variable by name, returning `&str` if valid UTF-8.
pub fn getenv(key: &[u8]) -> Option<&'static str> {
    getenv_bytes(key).and_then(|b| core::str::from_utf8(b).ok())
}
