#[cfg(feature = "dom")]
use alloc::format;
#[cfg(feature = "dom")]
use alloc::string::ToString;
use alloc::{string::String, vec::Vec};

#[cfg(feature = "dom")]
use golden_fish::{Document, Node, NodeId};
use sunlight_ui::widgets::{TreeViewRow, TreeViewState};

#[cfg(feature = "dom")]
use super::dom_tree_adapter::{
    default_expanded_nodes, default_selected_node, node_kind_label, tree_label, DomTreeAdapter,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DomInspectorPane {
    Styles,
    Properties,
    Tree,
}

#[derive(Debug, Clone)]
pub struct DomInspectorState {
    selected_node: Option<usize>,
    tree: TreeViewState<usize>,
    properties_scroll: usize,
    styles_scroll: usize,
    focused_pane: DomInspectorPane,
    empty_message: String,
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
            tree: TreeViewState::new(),
            properties_scroll: 0,
            styles_scroll: 0,
            focused_pane: DomInspectorPane::Tree,
            empty_message: String::from("No document parsed yet."),
            #[cfg(feature = "dom")]
            document: None,
        }
    }

    pub fn clear_with_message(&mut self, message: impl Into<String>) {
        self.selected_node = None;
        self.tree.clear();
        self.properties_scroll = 0;
        self.styles_scroll = 0;
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
        self.tree.scroll_offset()
    }

    pub fn set_tree_scroll(&mut self, tree_scroll: usize) {
        self.tree.set_scroll_offset(tree_scroll);
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
        let expanded_ids = default_expanded_nodes(&document);
        let selected_node = default_selected_node(&document);

        self.tree.clear();
        self.properties_scroll = 0;
        self.styles_scroll = 0;
        self.empty_message = String::from("No document parsed yet.");
        for node_id in expanded_ids {
            let _ = self.tree.expand(node_id);
        }
        let _ = self.tree.set_selected(selected_node);
        self.selected_node = selected_node;
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
            let _ = self.tree.set_selected(Some(node_id));
            self.selected_node = Some(node_id);
        } else {
            let _ = self.tree.set_selected(None);
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
        let adapter = DomTreeAdapter::new(document);
        let _ = self.tree.toggle(&adapter, node_id);
        self.selected_node = self.tree.selected_id();
    }

    #[cfg(not(feature = "dom"))]
    pub fn toggle_node(&mut self, _node_id: usize) {}

    #[cfg(feature = "dom")]
    pub fn move_tree_selection(&mut self, delta: i32, visible_rows: usize) -> bool {
        let Some(document) = self.document.as_ref() else {
            return false;
        };
        let adapter = DomTreeAdapter::new(document);
        let rows = self.tree.rebuild_rows(&adapter);
        let changed = self.tree.move_selection(&rows, delta).is_some();
        self.tree.ensure_selected_visible(&rows, visible_rows);
        self.selected_node = self.tree.selected_id();
        changed
    }

    #[cfg(not(feature = "dom"))]
    pub fn move_tree_selection(&mut self, _delta: i32, _visible_rows: usize) -> bool {
        false
    }

    #[cfg(feature = "dom")]
    pub fn tree_left(&mut self, visible_rows: usize) -> bool {
        let Some(document) = self.document.as_ref() else {
            return false;
        };
        let adapter = DomTreeAdapter::new(document);
        let rows = self.tree.rebuild_rows(&adapter);
        let changed = self
            .tree
            .collapse_or_select_parent(&adapter, &rows)
            .is_some();
        self.tree.ensure_selected_visible(&rows, visible_rows);
        self.selected_node = self.tree.selected_id();
        changed
    }

    #[cfg(not(feature = "dom"))]
    pub fn tree_left(&mut self, _visible_rows: usize) -> bool {
        false
    }

    #[cfg(feature = "dom")]
    pub fn tree_right(&mut self, visible_rows: usize) -> bool {
        let Some(document) = self.document.as_ref() else {
            return false;
        };
        let adapter = DomTreeAdapter::new(document);
        let rows = self.tree.rebuild_rows(&adapter);
        let changed = self.tree.expand_or_select_first_child(&adapter).is_some();
        self.tree.ensure_selected_visible(&rows, visible_rows);
        self.selected_node = self.tree.selected_id();
        changed
    }

    #[cfg(not(feature = "dom"))]
    pub fn tree_right(&mut self, _visible_rows: usize) -> bool {
        false
    }

    #[cfg(feature = "dom")]
    pub fn toggle_selected_tree_node(&mut self, visible_rows: usize) -> bool {
        let Some(document) = self.document.as_ref() else {
            return false;
        };
        let adapter = DomTreeAdapter::new(document);
        let rows = self.tree.rebuild_rows(&adapter);
        let changed = self.tree.toggle_selected(&adapter).is_some();
        self.tree.ensure_selected_visible(&rows, visible_rows);
        self.selected_node = self.tree.selected_id();
        changed
    }

    #[cfg(not(feature = "dom"))]
    pub fn toggle_selected_tree_node(&mut self, _visible_rows: usize) -> bool {
        false
    }

    #[cfg(feature = "dom")]
    pub fn clamp_tree_scroll(&mut self, visible_rows: usize) {
        let Some(document) = self.document.as_ref() else {
            self.tree.set_scroll_offset(0);
            self.selected_node = None;
            return;
        };
        let adapter = DomTreeAdapter::new(document);
        let rows = self.tree.rebuild_rows(&adapter);
        self.tree.clamp_scroll(rows.len(), visible_rows);
        self.selected_node = self.tree.selected_id();
    }

    #[cfg(not(feature = "dom"))]
    pub fn clamp_tree_scroll(&mut self, _visible_rows: usize) {
        self.tree.set_scroll_offset(0);
    }

    #[cfg(feature = "dom")]
    pub fn tree_rows(&mut self) -> Vec<TreeViewRow<NodeId>> {
        let Some(document) = self.document.as_ref() else {
            return Vec::new();
        };
        let adapter = DomTreeAdapter::new(document);
        let rows = self.tree.rebuild_rows(&adapter);
        self.selected_node = self.tree.selected_id();
        rows
    }

    #[cfg(not(feature = "dom"))]
    pub fn tree_rows(&mut self) -> Vec<TreeViewRow<usize>> {
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
                out.push_str("Kind: ");
                out.push_str(node_kind_label(document, node_id));
                out.push('\n');
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
                }
                out.push_str("Child Count: ");
                out.push_str(&document.children(node_id).len().to_string());
                out.push('\n');
                out.push_str("Parent: ");
                out.push_str(&parent_label(document, node_id));
                out.push('\n');
                out.push_str("Preview: ");
                out.push_str(&tree_label(document, node_id));
                out.push('\n');
                out.push_str("NodeId: ");
                out.push_str(&node_id.to_string());
                out.push('\n');
                out
            }
            Node::Text { content } => {
                format!(
                    "Node Type: Text\nKind: {}\nPreview: {}\nCharacter Length: {}\nByte Length: {}\nParent: {}\nNodeId: {}\n",
                    node_kind_label(document, node_id),
                    tree_label(document, node_id),
                    content.chars().count(),
                    content.len(),
                    parent_label(document, node_id),
                    node_id
                )
            }
            Node::Comment { .. } => {
                format!(
                    "Node Type: Comment\nPreview: {}\nParent: {}\nNodeId: {}\n",
                    tree_label(document, node_id),
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
            return String::from(
                "Computed styles are not available yet.\nThis node does not expose inline style attributes.",
            );
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
        let mut state = DomInspectorState::new();
        assert_eq!(state.empty_message(), "No document parsed yet.");
        assert!(state.tree_rows().is_empty());
    }

    #[test]
    fn dom_tree_uses_real_parsed_structure() {
        let mut state =
            build_state("<html><head></head><body><header></header><main></main></body></html>");
        let labels: Vec<String> = state.tree_rows().into_iter().map(|row| row.label).collect();
        assert_eq!(labels, vec!["#document", "<html>", "<head>", "<body>"]);
    }

    #[test]
    fn selected_node_is_cleared_when_document_replaced() {
        let mut state = build_state(r#"<div id="main">a</div>"#);
        let document = state.document().unwrap();
        let node_id = document.find_first_element("div").unwrap();
        state.select_node(node_id);
        assert_eq!(state.selected_node(), Some(node_id));

        state.set_document(parse_html("<p>fresh</p>").unwrap());
        assert_ne!(state.selected_node(), Some(node_id));
    }

    #[test]
    fn default_selection_prefers_html_when_present() {
        let state = build_state("<html><body></body></html>");
        let document = state.document().unwrap();
        assert_eq!(state.selected_node(), document.find_first_element("html"));
    }

    #[test]
    fn tree_replacement_resets_old_expansion_state() {
        let mut state = build_state("<html><body><section><p>first</p></section></body></html>");
        let document = state.document().unwrap();
        let body_id = document.find_first_element("body").unwrap();
        let section_id = document.find_first_element("section").unwrap();
        state.toggle_node(body_id);
        state.toggle_node(section_id);
        assert!(state.tree_rows().iter().any(|row| row.label == "<p>"));

        state.set_document(parse_html("<html><body><div>fresh</div></body></html>").unwrap());
        let labels: Vec<String> = state.tree_rows().into_iter().map(|row| row.label).collect();
        assert!(!labels.iter().any(|label| label == "<section>"));
    }
}
