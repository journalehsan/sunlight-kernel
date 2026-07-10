#[cfg(feature = "dom")]
use alloc::string::ToString;
#[cfg(feature = "dom")]
use alloc::{format, vec};
use alloc::{string::String, vec::Vec};

#[cfg(feature = "dom")]
use golden_fish::{Document, Node, NodeId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DomInspectorPane {
    Styles,
    Properties,
    Tree,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomTreeRow {
    pub node_id: usize,
    pub depth: usize,
    pub has_children: bool,
    pub expanded: bool,
    pub label: String,
}

#[derive(Debug, Clone)]
pub struct DomInspectorState {
    selected_node: Option<usize>,
    tree_scroll: usize,
    properties_scroll: usize,
    styles_scroll: usize,
    focused_pane: DomInspectorPane,
    empty_message: String,
    expanded_node_ids: Vec<usize>,
    #[cfg(feature = "dom")]
    document: Option<Document>,
}

impl Default for DomInspectorState {
    fn default() -> Self {
        Self::new()
    }
}

impl DomInspectorState {
    pub fn new() -> Self {
        Self {
            selected_node: None,
            tree_scroll: 0,
            properties_scroll: 0,
            styles_scroll: 0,
            focused_pane: DomInspectorPane::Tree,
            empty_message: String::from("No document parsed yet."),
            expanded_node_ids: Vec::new(),
            #[cfg(feature = "dom")]
            document: None,
        }
    }

    pub fn clear_with_message(&mut self, message: impl Into<String>) {
        self.selected_node = None;
        self.tree_scroll = 0;
        self.properties_scroll = 0;
        self.styles_scroll = 0;
        self.expanded_node_ids.clear();
        self.empty_message = message.into();
        #[cfg(feature = "dom")]
        {
            self.document = None;
        }
    }

    pub fn selected_node(&self) -> Option<usize> {
        self.selected_node
    }

    pub fn tree_scroll(&self) -> usize {
        self.tree_scroll
    }

    pub fn set_tree_scroll(&mut self, tree_scroll: usize) {
        self.tree_scroll = tree_scroll;
    }

    pub fn properties_scroll(&self) -> usize {
        self.properties_scroll
    }

    pub fn set_properties_scroll(&mut self, properties_scroll: usize) {
        self.properties_scroll = properties_scroll;
    }

    pub fn styles_scroll(&self) -> usize {
        self.styles_scroll
    }

    pub fn set_styles_scroll(&mut self, styles_scroll: usize) {
        self.styles_scroll = styles_scroll;
    }

    pub fn focused_pane(&self) -> DomInspectorPane {
        self.focused_pane
    }

    pub fn set_focused_pane(&mut self, focused_pane: DomInspectorPane) {
        self.focused_pane = focused_pane;
    }

    pub fn empty_message(&self) -> &str {
        &self.empty_message
    }

    #[cfg(feature = "dom")]
    pub fn set_document(&mut self, document: Document) {
        self.selected_node = None;
        self.tree_scroll = 0;
        self.properties_scroll = 0;
        self.styles_scroll = 0;
        self.empty_message = String::from("No document parsed yet.");
        self.expanded_node_ids = default_expanded_nodes(&document);
        self.document = Some(document);
    }

    #[cfg(feature = "dom")]
    pub fn document(&self) -> Option<&Document> {
        self.document.as_ref()
    }

    #[cfg(not(feature = "dom"))]
    pub fn has_document(&self) -> bool {
        false
    }

    #[cfg(feature = "dom")]
    pub fn has_document(&self) -> bool {
        self.document.is_some()
    }

    #[cfg(feature = "dom")]
    pub fn select_node(&mut self, node_id: NodeId) {
        if self
            .document
            .as_ref()
            .and_then(|document| document.get(node_id))
            .is_some()
        {
            self.selected_node = Some(node_id);
        } else {
            self.selected_node = None;
        }
    }

    #[cfg(not(feature = "dom"))]
    pub fn select_node(&mut self, _node_id: usize) {
        self.selected_node = None;
    }

    #[cfg(feature = "dom")]
    pub fn toggle_node(&mut self, node_id: NodeId) {
        let Some(document) = self.document.as_ref() else {
            return;
        };
        if document.children(node_id).is_empty() || node_id == document.root() {
            return;
        }
        if let Some(index) = self
            .expanded_node_ids
            .iter()
            .position(|candidate| *candidate == node_id)
        {
            self.expanded_node_ids.remove(index);
        } else {
            self.expanded_node_ids.push(node_id);
        }
    }

    #[cfg(not(feature = "dom"))]
    pub fn toggle_node(&mut self, _node_id: usize) {}

    #[cfg(feature = "dom")]
    pub fn tree_rows(&self) -> Vec<DomTreeRow> {
        let Some(document) = self.document.as_ref() else {
            return Vec::new();
        };

        let mut rows = Vec::with_capacity(document.node_count());
        let mut stack = vec![(document.root(), 0usize)];

        while let Some((node_id, depth)) = stack.pop() {
            let children = document.children(node_id);
            rows.push(DomTreeRow {
                node_id,
                depth,
                has_children: !children.is_empty(),
                expanded: node_id == document.root() || self.expanded_node_ids.contains(&node_id),
                label: tree_label(document, node_id),
            });

            if node_id == document.root() || self.expanded_node_ids.contains(&node_id) {
                for &child in children.iter().rev() {
                    stack.push((child, depth + 1));
                }
            }
        }

        rows
    }

    #[cfg(not(feature = "dom"))]
    pub fn tree_rows(&self) -> Vec<DomTreeRow> {
        Vec::new()
    }

    #[cfg(feature = "dom")]
    pub fn node_properties_text(&self) -> String {
        let Some(document) = self.document.as_ref() else {
            return self.empty_message.clone();
        };
        let Some(node_id) = self.selected_node else {
            return String::from("Select a DOM node to inspect its properties.");
        };
        let Some(node) = document.get(node_id) else {
            return String::from("Select a DOM node to inspect its properties.");
        };

        match node {
            Node::Document { .. } => {
                format!(
                    "Node Type: Document\nChild Count: {}\nTotal Node Count: {}\nNodeId: {}\n",
                    document.children(node_id).len(),
                    document.node_count(),
                    node_id
                )
            }
            Node::Element {
                tag_name,
                attributes,
                ..
            } => {
                let mut out = String::new();
                out.push_str("Node Type: Element\n");
                out.push_str("Tag Name: ");
                out.push_str(tag_name);
                out.push('\n');
                if let Some(id_attr) = attribute_value(attributes, "id") {
                    out.push_str("id: ");
                    out.push_str(id_attr);
                    out.push('\n');
                }
                if let Some(class_attr) = attribute_value(attributes, "class") {
                    out.push_str("class: ");
                    out.push_str(class_attr);
                    out.push('\n');
                    out.push_str("Class List: ");
                    out.push_str(class_attr);
                    out.push('\n');
                }
                out.push_str("Child Count: ");
                out.push_str(&document.children(node_id).len().to_string());
                out.push('\n');
                out.push_str("Parent: ");
                out.push_str(&parent_label(document, node_id));
                out.push('\n');
                out.push_str("NodeId: ");
                out.push_str(&node_id.to_string());
                out.push('\n');
                out.push_str("Attributes:\n");
                if attributes.is_empty() {
                    out.push_str("  (none)\n");
                } else {
                    for attribute in attributes {
                        out.push_str("  ");
                        out.push_str(attribute.name());
                        out.push_str(" = ");
                        out.push_str(attribute.value());
                        out.push('\n');
                    }
                }
                out
            }
            Node::Text { content } => {
                format!(
                    "Node Type: Text\nText: {}\nCharacter Length: {}\nByte Length: {}\nParent: {}\nNodeId: {}\n",
                    content,
                    content.chars().count(),
                    content.len(),
                    parent_label(document, node_id),
                    node_id
                )
            }
            Node::Comment { content } => {
                format!(
                    "Node Type: Comment\nComment: {}\nParent: {}\nNodeId: {}\n",
                    content,
                    parent_label(document, node_id),
                    node_id
                )
            }
        }
    }

    #[cfg(not(feature = "dom"))]
    pub fn node_properties_text(&self) -> String {
        String::from("Golden Fish DOM support is unavailable in this build.")
    }

    #[cfg(feature = "dom")]
    pub fn styles_text(&self) -> String {
        let Some(document) = self.document.as_ref() else {
            return self.empty_message.clone();
        };
        let Some(node_id) = self.selected_node else {
            return String::from("Select a DOM node to inspect inline style data.");
        };
        let Some(node) = document.get(node_id) else {
            return String::from("Select a DOM node to inspect inline style data.");
        };
        let Node::Element {
            tag_name,
            attributes,
            ..
        } = node
        else {
            return String::from("Computed styles are not available yet.\nThis node does not expose inline style attributes.");
        };

        let mut out = String::new();
        out.push_str("Selected Element: <");
        out.push_str(tag_name);
        out.push_str(">\n");
        if let Some(style_attr) = attribute_value(attributes, "style") {
            out.push_str("Inline style: ");
            out.push_str(style_attr);
            out.push('\n');
        } else {
            out.push_str("Inline style: (none)\n");
        }
        out.push_str("Computed styles are not available yet.\n");
        out
    }

    #[cfg(not(feature = "dom"))]
    pub fn styles_text(&self) -> String {
        String::from("Computed styles are not available yet.")
    }
}

#[cfg(feature = "dom")]
fn default_expanded_nodes(document: &Document) -> Vec<NodeId> {
    let mut expanded = Vec::new();
    let mut stack = vec![(document.root(), 0usize)];
    while let Some((node_id, depth)) = stack.pop() {
        if depth <= 3 && !document.children(node_id).is_empty() {
            expanded.push(node_id);
            for &child in document.children(node_id).iter().rev() {
                stack.push((child, depth + 1));
            }
        }
    }
    expanded
}

#[cfg(feature = "dom")]
fn parent_label(document: &Document, node_id: NodeId) -> String {
    document.parent(node_id).map_or_else(
        || String::from("(none)"),
        |parent_id| format!("{} ({parent_id})", tree_label(document, parent_id)),
    )
}

#[cfg(feature = "dom")]
fn attribute_value<'a>(attributes: &'a [golden_fish::Attribute], name: &str) -> Option<&'a str> {
    attributes
        .iter()
        .find(|attribute| attribute.name().eq_ignore_ascii_case(name))
        .map(golden_fish::Attribute::value)
}

#[cfg(feature = "dom")]
fn tree_label(document: &Document, node_id: NodeId) -> String {
    let Some(node) = document.get(node_id) else {
        return String::from("(missing node)");
    };

    match node {
        Node::Document { .. } => String::from("#document"),
        Node::Element {
            tag_name,
            attributes,
            ..
        } => {
            let mut label = String::from("<");
            label.push_str(tag_name);
            if let Some(id_attr) = attribute_value(attributes, "id") {
                label.push_str(" id=\"");
                label.push_str(&truncate_value(id_attr, 24));
                label.push('"');
            }
            if let Some(class_attr) = attribute_value(attributes, "class") {
                label.push_str(" class=\"");
                label.push_str(&truncate_value(class_attr, 28));
                label.push('"');
            }
            label.push('>');
            label
        }
        Node::Text { content } => {
            let preview = preview_text(content, 32);
            format!("\"{preview}\"")
        }
        Node::Comment { content } => {
            let preview = preview_text(content, 32);
            format!("<!-- {preview} -->")
        }
    }
}

#[cfg(feature = "dom")]
fn preview_text(value: &str, max_chars: usize) -> String {
    let collapsed = value.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_value(&collapsed, max_chars)
}

#[cfg(feature = "dom")]
fn truncate_value(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return String::from(value);
    }
    let mut out = String::new();
    for (index, ch) in value.chars().enumerate() {
        if index >= max_chars.saturating_sub(1) {
            break;
        }
        out.push(ch);
    }
    out.push_str("...");
    out
}

#[cfg(all(test, feature = "dom"))]
mod tests {
    use super::*;
    use golden_fish::parse_html;

    fn build_state(html: &str) -> DomInspectorState {
        let mut state = DomInspectorState::new();
        state.set_document(parse_html(html).unwrap());
        state
    }

    #[test]
    fn empty_state_is_stable_before_document_load() {
        let state = DomInspectorState::new();
        assert_eq!(state.empty_message(), "No document parsed yet.");
        assert!(state.tree_rows().is_empty());
    }

    #[test]
    fn dom_tree_labels_include_document_elements_and_text() {
        let state = build_state(
            r#"<html><body><div id="main" class="content">hello world</div><!-- note --></body></html>"#,
        );
        let labels: Vec<String> = state.tree_rows().into_iter().map(|row| row.label).collect();
        assert!(labels.iter().any(|label| label == "#document"));
        assert!(labels.iter().any(|label| label == "<html>"));
        assert!(labels
            .iter()
            .any(|label| label == "<div id=\"main\" class=\"content\">"));
        assert!(labels.iter().any(|label| label == "\"hello world\""));
        assert!(labels
            .iter()
            .any(|label| label.starts_with("<!--") && label.contains("note")));
    }

    #[test]
    fn element_property_extraction_includes_id_and_class() {
        let mut state =
            build_state(r#"<div id="main" class="content primary" data-role="hero"></div>"#);
        let document = state.document().unwrap();
        let node_id = document.find_first_element("div").unwrap();
        state.select_node(node_id);
        let properties = state.node_properties_text();
        assert!(properties.contains("Node Type: Element"));
        assert!(properties.contains("id: main"));
        assert!(properties.contains("class: content primary"));
        assert!(properties.contains("data-role = hero"));
    }

    #[test]
    fn selected_node_is_cleared_when_document_replaced() {
        let mut state = build_state(r#"<div id="main">a</div>"#);
        let document = state.document().unwrap();
        let node_id = document.find_first_element("div").unwrap();
        state.select_node(node_id);
        assert_eq!(state.selected_node(), Some(node_id));

        state.set_document(parse_html("<p>fresh</p>").unwrap());
        assert_eq!(state.selected_node(), None);
    }
}
