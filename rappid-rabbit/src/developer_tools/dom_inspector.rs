#[cfg(feature = "dom")]
use alloc::format;
use alloc::string::String;
#[cfg(feature = "dom")]
use alloc::string::ToString;
#[cfg(feature = "dom")]
use alloc::vec::Vec;

#[cfg(feature = "dom")]
use crate::css::{collect_embedded_stylesheets, Property, StyleContext};
#[cfg(feature = "dom")]
use golden_fish::{Document, Node, NodeId};
use sunlight_ui::widgets::{TreeViewRow, TreeViewState};

pub const MAX_INSPECTOR_PROJECTION_ROWS: usize = 8_192;

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
    properties_text_cache: String,
    styles_text_cache: String,
    properties_text_dirty: bool,
    styles_text_dirty: bool,
    #[cfg(feature = "dom")]
    tree_rows_cache: Vec<TreeViewRow<NodeId>>,
    #[cfg(feature = "dom")]
    tree_rows_dirty: bool,
    #[cfg(feature = "dom")]
    tree_rebuild_count: usize,
    #[cfg(feature = "dom")]
    document: Option<Document>,
    #[cfg(feature = "dom")]
    style_context: Option<StyleContext>,
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
            properties_text_cache: String::new(),
            styles_text_cache: String::new(),
            properties_text_dirty: true,
            styles_text_dirty: true,
            #[cfg(feature = "dom")]
            tree_rows_cache: Vec::new(),
            #[cfg(feature = "dom")]
            tree_rows_dirty: true,
            #[cfg(feature = "dom")]
            tree_rebuild_count: 0,
            #[cfg(feature = "dom")]
            document: None,
            #[cfg(feature = "dom")]
            style_context: None,
        }
    }

    pub fn clear_with_message(&mut self, message: impl Into<String>) {
        self.selected_node = None;
        self.tree.clear();
        self.properties_scroll = 0;
        self.styles_scroll = 0;
        self.empty_message = message.into();
        self.properties_text_cache.clear();
        self.styles_text_cache.clear();
        self.properties_text_dirty = true;
        self.styles_text_dirty = true;
        #[cfg(feature = "dom")]
        {
            self.tree_rows_cache.clear();
            self.tree_rows_dirty = true;
            self.document = None;
            self.style_context = None;
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
        let stylesheets = collect_embedded_stylesheets(&document);
        let style_context = StyleContext::build(&document, &stylesheets);
        self.set_document_with_styles(document, style_context);
    }

    #[cfg(feature = "dom")]
    pub fn set_document_with_styles(&mut self, document: Document, style_context: StyleContext) {
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
        self.properties_text_dirty = true;
        self.styles_text_dirty = true;
        self.tree_rows_dirty = true;
        self.document = Some(document);
        self.style_context = Some(style_context);
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
            self.invalidate_cached_views();
        } else {
            let _ = self.tree.set_selected(None);
            self.selected_node = None;
            self.invalidate_cached_views();
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
        self.invalidate_cached_views();
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
        if changed {
            self.invalidate_cached_views();
        }
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
        if changed {
            self.invalidate_cached_views();
        }
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
        if changed {
            self.invalidate_cached_views();
        }
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
        if changed {
            self.invalidate_cached_views();
        }
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
    pub fn tree_rows(&mut self) -> &[TreeViewRow<NodeId>] {
        self.refresh_tree_rows_cache();
        &self.tree_rows_cache
    }

    #[cfg(not(feature = "dom"))]
    pub fn tree_rows(&mut self) -> &[TreeViewRow<usize>] {
        &[]
    }

    #[cfg(feature = "dom")]
    pub fn node_properties_text(&mut self) -> &str {
        if self.properties_text_dirty {
            self.properties_text_cache = self.build_node_properties_text();
            self.properties_text_dirty = false;
        }
        self.properties_text_cache.as_str()
    }

    #[cfg(not(feature = "dom"))]
    pub fn node_properties_text(&mut self) -> &str {
        "Golden Fish DOM support is unavailable in this build."
    }

    #[cfg(feature = "dom")]
    pub fn styles_text(&mut self) -> &str {
        if self.styles_text_dirty {
            self.styles_text_cache = self.build_styles_text();
            self.styles_text_dirty = false;
        }
        self.styles_text_cache.as_str()
    }

    #[cfg(not(feature = "dom"))]
    pub fn styles_text(&mut self) -> &str {
        "Computed styles are not available yet."
    }
}

#[cfg(feature = "dom")]
impl DomInspectorState {
    fn invalidate_cached_views(&mut self) {
        self.properties_text_dirty = true;
        self.styles_text_dirty = true;
        self.tree_rows_dirty = true;
    }

    fn refresh_tree_rows_cache(&mut self) {
        if !self.tree_rows_dirty {
            return;
        }
        let Some(document) = self.document.as_ref() else {
            self.tree_rows_cache.clear();
            self.selected_node = None;
            self.tree_rows_dirty = false;
            return;
        };
        let adapter = DomTreeAdapter::new(document);
        let rows = self.tree.rebuild_rows(&adapter);
        if rows.len() > MAX_INSPECTOR_PROJECTION_ROWS {
            self.tree_rows_cache.clear();
            self.empty_message = format!(
                "DOM Inspector limit exceeded: {} visible rows (maximum {}).",
                rows.len(),
                MAX_INSPECTOR_PROJECTION_ROWS
            );
        } else {
            self.tree_rows_cache = rows;
        }
        self.selected_node = self.tree.selected_id();
        self.tree_rows_dirty = false;
        self.tree_rebuild_count = self.tree_rebuild_count.saturating_add(1);
    }

    pub fn visible_row_count(&mut self) -> usize {
        self.refresh_tree_rows_cache();
        self.tree_rows_cache.len()
    }

    #[cfg(test)]
    pub fn projection_rebuild_count(&self) -> usize {
        self.tree_rebuild_count
    }

    fn build_node_properties_text(&self) -> String {
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
                if tag_name.eq_ignore_ascii_case("img") {
                    out.push_str("Image Attributes\n");
                    for name in ["src", "alt", "width", "height"] {
                        out.push_str(name);
                        out.push_str(": ");
                        out.push_str(attribute_value(attributes, name).unwrap_or("(missing)"));
                        out.push('\n');
                    }
                    out.push_str("Resource details: available after image loading.\n");
                }
                if tag_name.eq_ignore_ascii_case("li")
                    || tag_name.eq_ignore_ascii_case("pre")
                    || tag_name.eq_ignore_ascii_case("code")
                {
                    if let Some(style) = self
                        .style_context
                        .as_ref()
                        .and_then(|styles| styles.style_for(node_id))
                    {
                        let properties: &[Property] = if tag_name.eq_ignore_ascii_case("li") {
                            &[
                                Property::Display,
                                Property::ListStyleType,
                                Property::ListStylePosition,
                            ]
                        } else {
                            &[Property::WhiteSpace, Property::FontFamily]
                        };
                        out.push_str("Typography\n");
                        for property in properties {
                            out.push_str(property.name());
                            out.push_str(": ");
                            out.push_str(
                                &style
                                    .value(property)
                                    .map(|value| value.display())
                                    .unwrap_or_else(|| String::from("(initial)")),
                            );
                            out.push('\n');
                        }
                    }
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

    fn build_styles_text(&self) -> String {
        let Some(document) = self.document.as_ref() else {
            return self.empty_message.clone();
        };
        let Some(node_id) = self.selected_node else {
            return String::from("Select a DOM node to inspect inline style data.");
        };
        let Some(node) = document.get(node_id) else {
            return String::from("Select a DOM node to inspect inline style data.");
        };
        let Some(style_context) = self.style_context.as_ref() else {
            return String::from("Computed styles are not available for this document.");
        };

        let is_element = matches!(node, Node::Element { .. });
        if !is_element {
            let mut out = String::from("This node does not directly match CSS selectors.\n");
            if let Some(style) = style_context.nearest_element_style(document, node_id) {
                out.push_str("Inherited text style from nearest element parent\n\n");
                for property in &style.properties {
                    if property.property.is_inherited_for_inspector() {
                        out.push_str(property.property.name());
                        out.push_str(": ");
                        out.push_str(&property.value.display());
                        out.push('\n');
                    }
                }
            }
            return out;
        }

        let Node::Element {
            tag_name,
            attributes,
            ..
        } = node
        else {
            unreachable!()
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
        let Some(style) = style_context.style_for(node_id) else {
            out.push_str("Computed styles are not available for this element.\n");
            return out;
        };
        out.push_str("\nComputed Styles\n---------------\n");
        for property in &style.properties {
            out.push_str(property.property.name());
            out.push_str(": ");
            out.push_str(&property.value.display());
            if property
                .matched
                .as_ref()
                .is_some_and(|matched| matched.inherited)
            {
                out.push_str(" (inherited)");
            }
            out.push('\n');
        }
        out.push_str("\nMatched Rules\n-------------\n");
        let mut any = false;
        for property in &style.properties {
            let Some(matched) = property.matched.as_ref() else {
                continue;
            };
            if matched.inherited {
                continue;
            }
            any = true;
            out.push_str(&matched.selector);
            out.push_str(" [");
            out.push_str(&matched.source);
            out.push_str("]\n  ");
            out.push_str(matched.property.name());
            out.push_str(": ");
            out.push_str(&matched.value.display());
            out.push_str(";\n");
        }
        if !any {
            out.push_str("No matched declarations; initial values are shown above.\n");
        }
        out
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
        let labels: Vec<String> = state
            .tree_rows()
            .iter()
            .map(|row| row.label.clone())
            .collect();
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
        let labels: Vec<String> = state
            .tree_rows()
            .iter()
            .map(|row| row.label.clone())
            .collect();
        assert!(!labels.iter().any(|label| label == "<section>"));
    }

    #[test]
    fn repeated_idle_projection_access_does_not_rebuild_or_append() {
        let mut state = build_state(
            "<html><body><main><h1>Example</h1><p>Small document.</p></main></body></html>",
        );
        let first_rows = state.tree_rows().len();
        assert_eq!(state.projection_rebuild_count(), 1);
        for _ in 0..1_000 {
            assert_eq!(state.tree_rows().len(), first_rows);
        }
        assert_eq!(state.projection_rebuild_count(), 1);
    }

    #[test]
    fn projection_rebuild_replaces_previous_rows() {
        let mut state = build_state("<html><body><main></main></body></html>");
        let initial_rows = state.tree_rows().len();
        let document = state.document().unwrap();
        let body_id = document.find_first_element("body").unwrap();
        state.toggle_node(body_id);
        let expanded_rows = state.tree_rows().len();
        assert!(expanded_rows >= initial_rows);

        state.toggle_node(body_id);
        assert_eq!(state.tree_rows().len(), initial_rows);
        assert_eq!(state.projection_rebuild_count(), 3);
    }

    #[test]
    fn navigation_replacement_drops_old_document_projection() {
        let mut state =
            build_state("<html><body><section><p>old document</p></section></body></html>");
        let document = state.document().unwrap();
        let body_id = document.find_first_element("body").unwrap();
        state.toggle_node(body_id);
        assert!(state.tree_rows().iter().any(|row| row.label == "<section>"));

        state.set_document(parse_html("<html><body><article>new</article></body></html>").unwrap());
        let labels: Vec<_> = state
            .tree_rows()
            .iter()
            .map(|row| row.label.as_str())
            .collect();
        assert!(!labels.contains(&"<section>"));
        assert!(state
            .document()
            .unwrap()
            .find_first_element("article")
            .is_some());
    }

    #[test]
    fn example_sized_document_projection_stays_small() {
        let mut state = build_state(
            "<!doctype html><html><head><title>Example Domain</title></head><body><div><h1>Example Domain</h1><p>This domain is for examples.</p></div></body></html>",
        );
        let stats = state.document().unwrap().stats();
        assert!(stats.node_count < 64);
        assert!(state.tree_rows().len() < 64);
        assert!(state.tree_rows().len() <= MAX_INSPECTOR_PROJECTION_ROWS);
    }

    #[test]
    fn styles_panel_shows_computed_values_and_matched_rules() {
        let mut state = build_state(
            "<html><head><style>body { color: #222; } .notice { padding: 8px; }</style></head><body><p class='notice'>Hello</p></body></html>",
        );
        let document = state.document().unwrap();
        let notice = document.find_first_element("p").unwrap();
        state.select_node(notice);
        let text = state.styles_text();
        assert!(text.contains("Computed Styles"));
        assert!(text.contains("color: #222222"));
        assert!(text.contains("padding-left: 8px"));
        assert!(text.contains("Matched Rules"));
        assert!(text.contains(".notice"));
    }
}
