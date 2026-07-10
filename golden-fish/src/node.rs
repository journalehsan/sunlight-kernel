//! Node types for the Golden Fish DOM.
//!
//! Golden Fish owns its DOM data. `Node` and `Document` contain only
//! owned data and stable indices (`NodeId`). No parser-backend types
//! are exposed.

use alloc::{string::String, vec::Vec};

use crate::attributes::Attribute;

/// Stable identifier for a node within a `Document`.
pub type NodeId = usize;

/// A single node in the Golden Fish DOM tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Node {
    /// The synthetic document root.
    Document {
        /// Child node ids in document order.
        children: Vec<NodeId>,
    },

    /// An element with a tag name, attributes, and children.
    Element {
        /// Tag name (e.g. "html", "div", "p"). Lowercased where the
        /// underlying parser provides case-insensitive names.
        tag_name: String,
        /// Attributes in source order (duplicates preserved if present).
        attributes: Vec<Attribute>,
        /// Child node ids in source order.
        children: Vec<NodeId>,
    },

    /// A text node.
    Text {
        /// The text content.
        content: String,
    },

    /// An HTML comment node.
    Comment {
        /// The comment text (without the `<!--` and `-->` delimiters).
        content: String,
    },
}

impl Node {
    /// Returns true if this node is an element.
    pub fn is_element(&self) -> bool {
        matches!(self, Node::Element { .. })
    }

    /// Returns the tag name if this is an element.
    pub fn tag_name(&self) -> Option<&str> {
        match self {
            Node::Element { tag_name, .. } => Some(tag_name.as_str()),
            _ => None,
        }
    }

    /// Returns a slice of attributes if this is an element.
    pub fn attributes(&self) -> Option<&[Attribute]> {
        match self {
            Node::Element { attributes, .. } => Some(attributes),
            _ => None,
        }
    }

    /// Returns the text content if this is a text node.
    pub fn text_content(&self) -> Option<&str> {
        match self {
            Node::Text { content } => Some(content.as_str()),
            _ => None,
        }
    }

    /// Returns the comment content if this is a comment node.
    pub fn comment_content(&self) -> Option<&str> {
        match self {
            Node::Comment { content } => Some(content.as_str()),
            _ => None,
        }
    }
}
