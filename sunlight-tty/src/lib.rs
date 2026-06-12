#![no_std]

extern crate alloc;

pub mod console;  // Simple ANSI stream-based terminal
pub mod grid;    // Terminal grid with VT100 support
pub mod login;
pub mod mux;
pub mod session;
pub mod shell;
pub mod vt100;

// Export TerminalGrid from grid module (has public cols/rows for caching)
pub use grid::TerminalGrid;
