//! Chronos milestone-one DOS `.COM` runtime.
//!
//! The runtime deliberately exposes no host pointers, hardware ports, file
//! system, or syscall interfaces to guest code.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

mod cpu;
mod dos;
mod fs;
mod loader;
mod memory;
mod runtime;
mod sample;
mod text_mode;

pub use cpu::CpuState;
pub use fs::{
    is_reserved_device, wildcard_matches, DirectoryEntry, DosDrive, DosEntry, DosError, DosHandle,
    DosHandleTable, DosPath, DriveAccess, DriveTable, OpenMode, ATTR_ARCHIVE, ATTR_DIRECTORY,
    ATTR_HIDDEN, ATTR_READ_ONLY, ATTR_SYSTEM,
};
pub use loader::{load_com, load_com_with_command_tail, LoaderError, PSP_SEGMENT};
pub use memory::{GuestMemory, MemoryError, MEMORY_SIZE};
pub use runtime::{translate_key_press, BiosKey, GuestState, HostKeyEvent, Runtime, Trap};
pub use sample::{CHRONOS_INTERACTIVE_COM, HELLO_CHRONOS_COM};
pub use text_mode::{
    display_char, TextCell, TextModeSurface, DEFAULT_ATTRIBUTE, TEXT_COLUMNS, TEXT_ROWS,
    VIDEO_BYTES, VIDEO_PHYSICAL, VIDEO_SEGMENT,
};
