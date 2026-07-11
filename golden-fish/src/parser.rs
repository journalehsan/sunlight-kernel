//! Public parsing entry point for Golden Fish.
//!
//! This module deliberately hides all details of the parser backend.

#[cfg(not(target_os = "none"))]
use crate::convert::parse_with_tl;
use crate::document::Document;
use crate::error::ParseError;
#[cfg(target_os = "none")]
use crate::simple_parser::parse_with_fallback;

/// Safety rails for the current browser-stage DOM.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParseLimits {
    pub max_nodes: usize,
    pub max_depth: usize,
    pub max_attributes_per_element: usize,
    pub max_text_node_bytes: usize,
    pub max_total_text_bytes: usize,
    pub iteration_multiplier: usize,
    pub iteration_slack: usize,
}

impl Default for ParseLimits {
    fn default() -> Self {
        Self {
            max_nodes: 16_384,
            max_depth: 256,
            max_attributes_per_element: 128,
            max_text_node_bytes: 512 * 1024,
            max_total_text_bytes: 4 * 1024 * 1024,
            iteration_multiplier: 16,
            iteration_slack: 1_024,
        }
    }
}

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
    parse_html_with_limits(source, ParseLimits::default())
}

/// Parse HTML with explicit resource and progress limits.
pub fn parse_html_with_limits(source: &str, limits: ParseLimits) -> Result<Document, ParseError> {
    #[cfg(not(target_os = "none"))]
    {
        let document = parse_with_tl(source)?;
        validate_document(&document, limits)?;
        Ok(document)
    }

    #[cfg(target_os = "none")]
    {
        parse_with_fallback(source, limits)
    }
}

#[cfg(not(target_os = "none"))]
fn validate_document(document: &Document, limits: ParseLimits) -> Result<(), ParseError> {
    let stats = document.stats();
    if stats.node_count > limits.max_nodes {
        return Err(ParseError::new(alloc::format!(
            "DOM limit exceeded: {} nodes (maximum {})",
            stats.node_count,
            limits.max_nodes
        )));
    }
    if stats.max_depth > limits.max_depth {
        return Err(ParseError::new(alloc::format!(
            "DOM limit exceeded: depth {} (maximum {})",
            stats.max_depth,
            limits.max_depth
        )));
    }
    if stats.total_text_bytes > limits.max_total_text_bytes {
        return Err(ParseError::new(alloc::format!(
            "DOM limit exceeded: {} stored text bytes (maximum {})",
            stats.total_text_bytes,
            limits.max_total_text_bytes
        )));
    }
    let mut stack = alloc::vec![document.root()];
    while let Some(node_id) = stack.pop() {
        if let Some(crate::Node::Element {
            attributes,
            children,
            ..
        }) = document.get(node_id)
        {
            if attributes.len() > limits.max_attributes_per_element {
                return Err(ParseError::new(alloc::format!(
                    "DOM limit exceeded: {} attributes on node {} (maximum {})",
                    attributes.len(),
                    node_id,
                    limits.max_attributes_per_element
                )));
            }
            for &child in children.iter().rev() {
                stack.push(child);
            }
        } else if let Some(crate::Node::Document { children }) = document.get(node_id) {
            for &child in children.iter().rev() {
                stack.push(child);
            }
        } else if let Some(crate::Node::Text { content } | crate::Node::Comment { content }) =
            document.get(node_id)
        {
            if content.len() > limits.max_text_node_bytes {
                return Err(ParseError::new(alloc::format!(
                    "DOM limit exceeded: text node {} has {} bytes (maximum {})",
                    node_id,
                    content.len(),
                    limits.max_text_node_bytes
                )));
            }
        }
    }
    Ok(())
}
