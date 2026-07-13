//! Chronos milestone-one DOS `.COM` runtime.
//!
//! The runtime deliberately exposes no host pointers, hardware ports, file
//! system, or syscall interfaces to guest code.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

mod arena;
mod cpu;
mod dos;
mod fs;
mod io;
mod loader;
mod memory;
mod mouse;
mod mz;
mod runtime;
mod sample;
mod text_mode;
mod video;

pub use arena::{
    ArenaError, BlockState, DosMemoryArena, MemoryBlock, DOS_ARENA_END_SEGMENT,
    DOS_ARENA_START_SEGMENT,
};
pub use cpu::CpuState;
pub use fs::{
    is_reserved_device, wildcard_matches, DirectoryEntry, DosDrive, DosEntry, DosError, DosHandle,
    DosHandleTable, DosPath, DriveAccess, DriveTable, OpenMode, ATTR_ARCHIVE, ATTR_DIRECTORY,
    ATTR_HIDDEN, ATTR_READ_ONLY, ATTR_SYSTEM,
};
pub use io::{
    GuestIoDevice, IoOperation, IoTrap, IoWidth, VgaDac, VGA_DAC_DATA_PORT,
    VGA_DAC_READ_INDEX_PORT, VGA_DAC_WRITE_INDEX_PORT,
};
pub use loader::{
    build_psp, load_com, load_com_with_command_tail, load_program, LoadedProgram, LoaderError,
    PSP_SEGMENT,
};
pub use memory::{GuestMemory, MemoryError, MEMORY_SIZE};
pub use mouse::{
    viewport_pixel, DosMouse, MouseButtons, MouseRangeError, MouseViewport, DOS_MOUSE_BUTTON_COUNT,
    DOS_MOUSE_DEFAULT_MAX_X, DOS_MOUSE_DEFAULT_MAX_Y,
};
pub use mz::{
    classify_executable, parse_mz, relocations, ExecutableFormat, MzError, MzHeader, MzRelocation,
    UnsupportedExecutable,
};
pub use runtime::{
    translate_key_press, BiosKey, ChildResult, CpuProfile, DosProcess, GuestDate, GuestState,
    GuestStateKind, GuestTime, GuestWaitReason, GuestWakeSource, HostKeyEvent, Runtime,
    SliceStopReason, TerminationType, Trap,
};
pub use sample::{CHRONOS_INTERACTIVE_COM, HELLO_CHRONOS_COM};
pub use text_mode::{
    display_char, TextCell, TextModeSurface, DEFAULT_ATTRIBUTE, TEXT_COLUMNS, TEXT_ROWS,
    VIDEO_BYTES, VIDEO_PHYSICAL, VIDEO_SEGMENT,
};
pub use video::{
    convert_indexed_rows, dac6_to_host8, default_vga_dac_entries, default_vga_palette,
    GuestVideoMode, Rgb8, VgaDacEntry, DEFAULT_VGA_PALETTE, VGA_FRAMEBUFFER_BYTES,
    VGA_FRAMEBUFFER_PHYSICAL, VGA_FRAMEBUFFER_SEGMENT, VGA_HEIGHT, VGA_WIDTH,
};
