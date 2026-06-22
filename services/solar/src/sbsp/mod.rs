//! SBSP (Solar Basic Server Pages) Engine
//!
//! Phase 3-4: Scripting language lexer, parser, and runtime
//! Executes `.sbsp` files with embedded Rust-like expressions and control flow.

pub mod lexer;
pub mod token;
pub mod value;

pub use lexer::SbspLexer;
pub use token::SbspToken;
pub use value::SbspValue;
