//! Owned document and tree representation.
//!
//! `Document` owns a flat arena of nodes and provides stable `NodeId`
//! based access. Parents are tracked in a parallel array to avoid
//! self-referential structures and to support future mutation.

use alloc::{
    string::{String, ToString},
    vec::Vec,
};
use core::fmt;

use crate::attributes::Attribute;
use crate::node::{Node, NodeId};

/// An owned HTML document tree.
///
/// Always contains at least a root `Node::Document`.
#[derive(Clone, Debug)]
pub struct Document {
    root: NodeId,
    nodes: Vec<Node>,
    /// Parallel to `nodes`: parent of each node, if any.
    parents: Vec<Option<NodeId>>,
}

impl Document {
    /// Create a new empty document with a root document node.
    pub fn new() -> Self {
        let mut doc = Self {
            root: 0,
            nodes: Vec::new(),
            parents: Vec::new(),
        };
        let root_id = doc.alloc_node(Node::Document {
            children: Vec::new(),
        });
        // Root has no parent
        doc.parents[root_id] = None;
        doc.root = root_id;
        doc
    }

    /// Allocate a new node and return its id.
    pub(crate) fn alloc_node(&mut self, node: Node) -> NodeId {
        let id = self.nodes.len();
        self.nodes.push(node);
        self.parents.push(None);
        id
    }

    /// Set the parent of a node (used during tree construction).
    pub(crate) fn set_parent(&mut self, child: NodeId, parent: NodeId) {
        if child < self.parents.len() {
            self.parents[child] = Some(parent);
        }
    }

    /// Append a child id to a parent's children list.
    pub(crate) fn append_child(&mut self, parent: NodeId, child: NodeId) {
        match &mut self.nodes[parent] {
            Node::Document { children } | Node::Element { children, .. } => {
                children.push(child);
            }
            _ => {}
        }
    }

    /// The root node id. This is always a `Node::Document`.
    pub fn root(&self) -> NodeId {
        self.root
    }

    /// Number of nodes currently allocated in the document (including root).
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Borrow a node by id.
    pub fn get(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id)
    }

    /// Borrow a mutable node by id (supports future mutation).
    pub fn get_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes.get_mut(id)
    }

    /// Children of a node, in source order. Returns empty slice for leaves.
    pub fn children(&self, id: NodeId) -> &[NodeId] {
        match self.nodes.get(id) {
            Some(Node::Document { children } | Node::Element { children, .. }) => children,
            _ => &[],
        }
    }

    /// Parent of a node, if any.
    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.parents.get(id).and_then(|p| *p)
    }

    /// Tag name for an element node.
    pub fn tag_name(&self, id: NodeId) -> Option<&str> {
        self.get(id).and_then(|n| n.tag_name())
    }

    /// Attributes for an element node.
    pub fn attributes(&self, id: NodeId) -> Option<&[Attribute]> {
        self.get(id).and_then(|n| n.attributes())
    }

    /// Text content for a text node.
    pub fn text_content(&self, id: NodeId) -> Option<&str> {
        self.get(id).and_then(|n| n.text_content())
    }

    /// Find the first element with the given tag name (depth-first).
    pub fn find_first_element(&self, tag: &str) -> Option<NodeId> {
        self.find_first_element_from(self.root, tag)
    }

    fn find_first_element_from(&self, start: NodeId, tag: &str) -> Option<NodeId> {
        if let Some(node) = self.get(start) {
            if let Some(t) = node.tag_name() {
                if t.eq_ignore_ascii_case(tag) {
                    return Some(start);
                }
            }
            for &child in self.children(start) {
                if let Some(found) = self.find_first_element_from(child, tag) {
                    return Some(found);
                }
            }
        }
        None
    }

    /// Produce a deterministic, human-readable tree dump.
    ///
    /// Example:
    /// ```text
    /// #document
    /// └── html
    ///     └── body
    ///         └── "Hello"
    /// ```
    pub fn debug_tree(&self) -> String {
        let mut out = String::new();
        self.write_tree(self.root, 0, true, &mut out);
        out
    }

    fn write_tree(&self, id: NodeId, depth: usize, is_last: bool, out: &mut String) {
        use core::fmt::Write;

        let prefix = if depth == 0 {
            String::new()
        } else {
            let mut p = String::new();
            for _ in 0..(depth.saturating_sub(1)) {
                p.push_str("│   ");
            }
            if depth >= 1 {
                if is_last {
                    p.push_str("└── ");
                } else {
                    p.push_str("├── ");
                }
            }
            p
        };

        if let Some(node) = self.get(id) {
            match node {
                Node::Document { .. } => {
                    let _ = writeln!(out, "{prefix}#document");
                }
                Node::Element {
                    tag_name,
                    attributes,
                    ..
                } => {
                    if attributes.is_empty() {
                        let _ = writeln!(out, "{prefix}{tag_name}");
                    } else {
                        let attrs: Vec<String> = attributes
                            .iter()
                            .map(|a| {
                                if a.value().is_empty() {
                                    a.name().to_string()
                                } else {
                                    alloc::format!("{}=\"{}\"", a.name(), escape_attr(a.value()))
                                }
                            })
                            .collect();
                        let joined = attrs.join(" ");
                        let _ = writeln!(out, "{prefix}{tag_name} {joined}");
                    }
                }
                Node::Text { content } => {
                    let trimmed = content.trim();
                    if trimmed.is_empty() {
                        let _ = writeln!(out, "{prefix}(whitespace)");
                    } else {
                        let escaped = escape_text(trimmed);
                        let _ = writeln!(out, "{prefix}\"{escaped}\"");
                    }
                }
                Node::Comment { content } => {
                    let escaped = escape_text(content);
                    let _ = writeln!(out, "{prefix}<!-- {escaped} -->");
                }
            }

            let kids = self.children(id);
            for (i, &child) in kids.iter().enumerate() {
                let last = i + 1 == kids.len();
                self.write_tree(child, depth + 1, last, out);
            }
        }
    }
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

#[allow(clippy::collapsible_str_replace)]
fn escape_text(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

#[allow(clippy::collapsible_str_replace)]
fn escape_attr(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', " ")
        .replace('\r', " ")
        .replace('\t', " ")
}

impl fmt::Display for Document {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.debug_tree())
    }
}
