//! Isolated host proof for sunlight-libc time validation.
//!
//! Include the implementation modules directly so the production
//! `clock_gettime` symbol is not exported into the host Rust test runtime.

#![no_std]
#![allow(dead_code)]

extern crate std;

#[path = "../sunlight-libc/src/sys.rs"]
mod sys;

#[path = "../sunlight-libc/src/errno.rs"]
mod errno;

#[path = "../sunlight-libc/src/time.rs"]
mod time;
