//! Testable library surface for `rand_service`.
//!
//! The production binary is `src/main.rs`. This crate root exists so the
//! ChaCha20 engine and its deterministic unit tests can be built with
//! `cargo test -p rand_service --lib --target x86_64-unknown-linux-gnu`.

#![cfg_attr(not(test), no_std)]

pub mod engine;

pub use engine::{
    secure_wipe, ChaCha20, EntropySource, ReseedReason, Stats, BLOCK_BYTES, RESEED_BLOCKS,
};
