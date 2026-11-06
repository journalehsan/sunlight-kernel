//! Strict, explicitly bounded JavaScript subset. See docs/javascript.md.
#![forbid(unsafe_code)]
mod ast;
mod lexer;
mod parser;
mod runtime;
mod value;
pub use runtime::{Context, GcStats, HostRoot, Limits, PropertyDescriptor};
pub use value::{Error, ErrorKind, Handle, JsResult, Value};
