//! Host harness for sunlight-libc freestanding memory and string primitives.
//!
//! Includes `mem.rs` and `string.rs` directly (same pattern as `alloc-proof.rs`)
//! so host unit tests exercise the real engines without linking the full
//! SunlightOS libc or colliding with the system libc's `memcpy`/`strlen`.

#![no_std]

extern crate std;

#[path = "../sunlight-libc/src/mem.rs"]
mod mem;

#[path = "../sunlight-libc/src/string.rs"]
mod string;
