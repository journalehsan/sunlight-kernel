//! Public parsing entry point for Golden Fish.
//!
//! This module deliberately hides all details of the `tl` backend.

use crate::convert::parse_with_tl;
use crate::document::Document;
use crate::error::ParseError;

/// Parse an HTML source string into an owned Golden Fish `Document`.
///
/// This is the primary public API. The implementation uses the `tl`
/// crate internally but never exposes `tl` types.
///
/// Contract:
/// - Input: UTF-8 HTML text (already decoded/decompressed by caller).
/// - Output: Owned DOM tree with a root `Document` node.
/// - Fragments without `<html>`/`<body>` are supported.
/// - Malformed input that `tl` can partially parse yields a usable tree.
/// - Only unrecoverable failures produce `ParseError`.
///
/// # Errors
///
/// Returns `ParseError` only for unrecoverable input length or internal
/// parser failures. Most malformed HTML produces a partial but usable tree.
pub fn parse_html(source: &str) -> Result<Document, ParseError> {
    // Delegate to the conversion layer which knows about tl.
    parse_with_tl(source)
}
