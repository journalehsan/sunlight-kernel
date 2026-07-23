//! Sun Shell library surface.
//!
//! Currently exposes the shared calculator engine used by the `=` builtin and
//! by Vortex Shell Search. The interactive shell binary remains `sshl`.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

pub mod calc;
