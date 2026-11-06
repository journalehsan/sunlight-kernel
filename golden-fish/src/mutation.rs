//! Small owned-tree operations used by controlled DOM hosts.

use alloc::{string::String, vec};

use crate::{Document, Node, NodeId};

impl Document {
    /// First connected element with an exact, case-sensitive ID, in tree order.
    pub fn get_element_by_id(&self, id: &str) -> Option<NodeId> {
        if id.is_empty() {
            return None;
        }
        let mut stack = vec![self.root()];
        while let Some(node) = stack.pop() {
            if self.attributes(node).is_some_and(|attrs| {
                attrs
                    .iter()
                    .any(|a| a.name().eq_ignore_ascii_case("id") && a.value() == id)
            }) {
                return Some(node);
            }
            stack.extend(self.children(node).iter().rev().copied());
        }
        None
    }

    /// Concatenated descendant text, excluding comments. Only accepts elements.
    pub fn element_text_content(&self, element: NodeId) -> Option<String> {
        if !self.get(element).is_some_and(Node::is_element) {
            return None;
        }
        let mut text = String::new();
        let mut stack = vec![element];
        while let Some(node) = stack.pop() {
            if let Some(Node::Text { content }) = self.get(node) {
                text.push_str(content);
            }
            stack.extend(self.children(node).iter().rev().copied());
        }
        Some(text)
    }

    /// Replace element children with plain text (never parsed as HTML).
    ///
    /// Returns `None` for a non-element and `Some(false)` for an unchanged leaf.
    /// Removed subtrees remain allocated with stable IDs, but their roots lose
    /// their parents. A sole text child is reused to bound repeated updates.
    pub fn set_element_text_content(&mut self, element: NodeId, text: &str) -> Option<bool> {
        if !self.get(element).is_some_and(Node::is_element) {
            return None;
        }
        let children = self.children(element);
        if children.is_empty() && text.is_empty() {
            return Some(false);
        }
        if children.len() == 1 && !text.is_empty() {
            let child = children[0];
            if let Some(Node::Text { content }) = self.get_mut(child) {
                if content == text {
                    return Some(false);
                }
                *content = String::from(text);
                return Some(true);
            }
        }
        let old = match self.get_mut(element)? {
            Node::Element { children, .. } => core::mem::take(children),
            _ => return None,
        };
        for child in old {
            self.detach_root(child);
        }
        if !text.is_empty() {
            let child = self.alloc_node(Node::Text {
                content: String::from(text),
            });
            self.set_parent(child, element);
            self.append_child(element, child);
        }
        Some(true)
    }
}

#[cfg(test)]
mod tests {
    use crate::parse_html;

    #[test]
    fn replacement_detaches_subtrees_preserves_ids_and_excludes_comments() {
        let mut doc =
            parse_html("<div id='a'>Old<!--skip--><b id='b'>nested</b></div><p id='a'>second</p>")
                .unwrap();
        let a = doc.get_element_by_id("a").unwrap();
        let b = doc.get_element_by_id("b").unwrap();
        assert_eq!(doc.element_text_content(a).as_deref(), Some("Oldnested"));
        assert_eq!(doc.set_element_text_content(a, "<b>New</b>"), Some(true));
        assert_eq!(doc.parent(b), None);
        assert_eq!(doc.get_element_by_id("b"), None);
        assert_eq!(doc.element_text_content(b).as_deref(), Some("nested"));
        let child = doc.children(a)[0];
        assert_eq!(doc.parent(child), Some(a));
        assert_eq!(doc.text_content(child), Some("<b>New</b>"));
        let count = doc.node_count();
        assert_eq!(doc.set_element_text_content(a, "Again"), Some(true));
        assert_eq!(doc.node_count(), count);
        assert_eq!(doc.set_element_text_content(a, "Again"), Some(false));
        assert_eq!(doc.set_element_text_content(a, ""), Some(true));
        assert_eq!(doc.parent(child), None);
        assert!(doc.children(a).is_empty());
        assert_eq!(doc.set_element_text_content(doc.root(), "bad"), None);
        assert_eq!(doc.get_element_by_id("A"), None);
        assert_eq!(doc.get_element_by_id(""), None);
    }
}
