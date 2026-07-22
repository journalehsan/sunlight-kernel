//! Minimal freestanding build of the libc memory/string modules.
//!
//! `test-mem-string-proof.sh` compiles this at `-O` and inspects its native
//! object code, relocations, and undefined symbols for self-recursion.

#![no_std]

#[path = "../sunlight-libc/src/mem.rs"]
mod mem;
#[path = "../sunlight-libc/src/string.rs"]
mod string;
