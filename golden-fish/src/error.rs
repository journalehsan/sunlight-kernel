//! Error types for Golden Fish HTML parsing.
//!
//! Golden Fish converts parser failures into a small, structured error.
//! The underlying parser backend (tl) is an implementation detail and
//! its types are never exposed through this API.

use alloc::string::String;
use core::fmt;

/// Error returned when HTML parsing fails in a non-recoverable way.
///
/// If the underlying parser produces a partial tree, Golden Fish will
/// typically return a usable `Document` rather than this error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    message: String,
}

impl ParseError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// Human-readable description of the parse failure.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

#[cfg(not(target_os = "none"))]
impl std::error::Error for ParseError {}
