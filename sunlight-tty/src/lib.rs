#![no_std]

extern crate alloc;

pub mod console;  // Simple ANSI stream-based terminal
pub mod login;
pub mod mux;
pub mod session;
pub mod shell;
pub mod vt100;

// Keep TerminalGrid as an alias for backwards compatibility
pub use console::Console as TerminalGrid;
