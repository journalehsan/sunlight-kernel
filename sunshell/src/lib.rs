//! Sun Shell library surface.
//!
//! Shared quote-aware parser, expansion and scripting interpreter, plus the
//! calculator used by the `=` builtin and Vortex Search. The binary is `sshl`.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

pub mod arithmetic;
pub mod calc;
pub mod capture;

pub mod expansion;
pub mod parser;
pub mod script;
pub mod script_parser;
pub mod shellenv;
