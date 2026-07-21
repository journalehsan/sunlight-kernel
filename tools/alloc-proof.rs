//! Native runtime harness for sunlight-libc's allocation contract tests.
//!
//! This includes the allocator module directly rather than linking the full
//! libc, so host `clock_gettime` and other SunlightOS ABI symbols cannot
//! replace the host test runtime's libc symbols.

#![no_std]

extern crate std;

#[path = "../sunlight-libc/src/alloc.rs"]
mod alloc;
