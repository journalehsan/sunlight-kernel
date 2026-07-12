//! Chronos milestone-one DOS `.COM` runtime.
//!
//! The runtime deliberately exposes no host pointers, hardware ports, file
//! system, or syscall interfaces to guest code.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

mod cpu;
mod dos;
mod loader;
mod memory;
mod runtime;
mod sample;
mod text_mode;

pub use cpu::CpuState;
pub use loader::{load_com, LoaderError, PSP_SEGMENT};
pub use memory::{GuestMemory, MemoryError, MEMORY_SIZE};
pub use runtime::{GuestState, Runtime, Trap};
pub use sample::HELLO_CHRONOS_COM;
pub use text_mode::{display_char, TextCell, TextModeSurface, TEXT_COLUMNS, TEXT_ROWS};
