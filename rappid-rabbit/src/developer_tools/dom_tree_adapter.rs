#![cfg(feature = "dom")]

use alloc::{
    string::{String, ToString},
    vec::Vec,
};

use golden_fish::{Attribute, Document, Node, NodeId};
use sunlight_ui::{
    font::UiSymbol,
    widgets::{TreeItem, TreeModel},
};

const ID_PREVIEW_CHARS: usize = 24;
const CLASS_PREVIEW_CHARS: usize = 28;
const TEXT_PREVIEW_CHARS: usize = 40;
const COMMENT_PREVIEW_CHARS: usize = 40;

pub(crate) struct DomTreeAdapter<'a> {
    document: &'a Document,
    roots: [NodeId; 1],
}

impl<'a> DomTreeAdapter<'a> {
    pub(crate) fn new(document: &'a Document) -> Self {
        Self {
            document,
            roots: [document.root()],
        }
    }
}

impl TreeModel for DomTreeAdapter<'_> {
    type Id = NodeId;

    fn roots(&self) -> &[Self::Id] {
        &self.roots
    }

    fn parent(&self, id: Self::Id) -> Option<Self::Id> {
        self.document.parent(id)
    }

    fn children(&self, id: Self::Id) -> &[Self::Id] {
        self.document.children(id)
    }

    fn item(&self, id: Self::Id) -> TreeItem {
        let mut item = TreeItem::new(tree_label(self.document, id));
        if let Some(icon) = node_icon(self.document, id) {
            item = item.with_icon(icon);
        }
        item.with_secondary_text(node_kind_label(self.document, id))
    }
}

pub(crate) fn tree_label(document: &Document, node_id: NodeId) -> String {
    let Some(node) = document.get(node_id) else {
        return String::from("(missing node)");
    };

    match node {
        Node::Document { .. } => String::from("#document"),
        Node::Element {
            tag_name,
            attributes,
            ..
        } => format_element_label(tag_name, attributes),
        Node::Text { content } => format_text_label(content),
        Node::Comment { content } => format_comment_label(content),
    }
}

pub(crate) fn node_kind_label(document: &Document, node_id: NodeId) -> &'static str {
    match document.get(node_id) {
        Some(Node::Document { .. }) => "document",
        Some(Node::Element { .. }) => "element",
        Some(Node::Text { content }) if content.trim().is_empty() => "whitespace",
        Some(Node::Text { .. }) => "text",
        Some(Node::Comment { .. }) => "comment",
        None => "missing",
    }
}

pub(crate) fn default_expanded_nodes(document: &Document) -> Vec<NodeId> {
    let mut expanded = Vec::new();
    let root_id = document.root();
    expanded.push(root_id);

    if let Some(html_id) = first_html_child(document) {
        if !document.children(html_id).is_empty() {
            expanded.push(html_id);
        }
    }

    expanded
}

pub(crate) fn default_selected_node(document: &Document) -> Option<NodeId> {
    first_html_child(document).or(Some(document.root()))
}

fn first_html_child(document: &Document) -> Option<NodeId> {
    document
        .children(document.root())
        .iter()
        .copied()
        .find(|&child_id| {
            document
                .tag_name(child_id)
                .is_some_and(|tag_name| tag_name.eq_ignore_ascii_case("html"))
        })
}

fn node_icon(document: &Document, node_id: NodeId) -> Option<UiSymbol> {
    match document.get(node_id) {
        Some(Node::Document { .. }) => Some(UiSymbol::Documents),
        Some(Node::Element { .. }) if !document.children(node_id).is_empty() => {
            Some(UiSymbol::Folder)
        }
        Some(Node::Element { .. } | Node::Text { .. } | Node::Comment { .. }) => {
            Some(UiSymbol::File)
        }
        None => None,
    }
}

fn format_element_label(tag_name: &str, attributes: &[Attribute]) -> String {
    let mut label = String::from("<");
    label.push_str(tag_name);

    if let Some(id_attr) = attribute_value(attributes, "id") {
        label.push_str(" id=\"");
        label.push_str(truncate_utf8(id_attr.trim(), ID_PREVIEW_CHARS).as_str());
        label.push('"');
    }

    if let Some(class_attr) = attribute_value(attributes, "class") {
        let normalized = collapse_whitespace(class_attr);
        if !normalized.is_empty() {
            label.push_str(" class=\"");
            label.push_str(truncate_utf8(normalized.as_str(), CLASS_PREVIEW_CHARS).as_str());
            label.push('"');
        }
    }

    label.push('>');
    label
}

fn format_text_label(content: &str) -> String {
    let preview = normalize_text_preview(content, TEXT_PREVIEW_CHARS);
    if preview.is_empty() {
        String::from("#text (whitespace)")
    } else {
        let mut label = String::from("\"");
        label.push_str(preview.as_str());
        label.push('"');
        label
    }
}

fn format_comment_label(content: &str) -> String {
    let preview = normalize_text_preview(strip_comment_delimiters(content), COMMENT_PREVIEW_CHARS);
    let mut label = String::from("<!-- ");
    label.push_str(preview.as_str());
    label.push_str(" -->");
    label
}

pub(crate) fn normalize_text_preview(value: &str, max_chars: usize) -> String {
    let collapsed = collapse_whitespace(value);
    truncate_utf8(collapsed.as_str(), max_chars)
}

fn collapse_whitespace(value: &str) -> String {
    let mut out = String::new();
    for segment in value.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(segment);
    }
    out
}

fn strip_comment_delimiters(value: &str) -> &str {
    let trimmed = value.trim();
    trimmed
        .strip_prefix("<!--")
        .and_then(|value| value.strip_suffix("-->"))
        .map_or(trimmed, str::trim)
}

pub(crate) fn truncate_utf8(value: &str, max_chars: usize) -> String {
    let char_count = value.chars().count();
    if char_count <= max_chars {
        return value.to_string();
    }
    if max_chars == 0 {
        return String::new();
    }
    if max_chars <= 3 {
        return value.chars().take(max_chars).collect();
    }

    let visible_chars = max_chars - 3;
    let mut out = String::new();
    for ch in value.chars().take(visible_chars) {
        out.push(ch);
    }
    out.push_str("...");
    out
}

fn attribute_value<'a>(attributes: &'a [Attribute], name: &str) -> Option<&'a str> {
    attributes
        .iter()
        .find(|attribute| attribute.name().eq_ignore_ascii_case(name))
        .map(Attribute::value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use golden_fish::parse_html;
    use sunlight_ui::widgets::TreeViewState;

    fn parse(html: &str) -> Document {
        parse_html(html).expect("parse should succeed")
    }

    #[test]
    fn document_label_is_stable() {
        let document = parse("<p>hello</p>");
        assert_eq!(tree_label(&document, document.root()), "#document");
    }

    #[test]
    fn element_label_uses_tag_name() {
        let document = parse("<body></body>");
        let body_id = document.find_first_element("body").unwrap();
        assert_eq!(tree_label(&document, body_id), "<body>");
    }

    #[test]
    fn element_label_includes_id_preview() {
        let document = parse(r#"<div id="main-content"></div>"#);
        let div_id = document.find_first_element("div").unwrap();
        assert_eq!(tree_label(&document, div_id), "<div id=\"main-content\">");
    }

    #[test]
    fn element_label_includes_class_preview() {
        let document = parse(r#"<section class="hero    featured primary"></section>"#);
        let section_id = document.find_first_element("section").unwrap();
        assert_eq!(
            tree_label(&document, section_id),
            "<section class=\"hero featured primary\">"
        );
    }

    #[test]
    fn text_preview_normalizes_whitespace() {
        let document = parse("<p>\n  hello   world \t from  SunlightOS </p>");
        let text_id = document.children(document.find_first_element("p").unwrap())[0];
        assert_eq!(
            tree_label(&document, text_id),
            "\"hello world from SunlightOS\""
        );
    }

    #[test]
    fn truncation_is_utf8_safe() {
        assert_eq!(truncate_utf8("سلام دنیا 🌍", 8), "سلام ...");
    }

    #[test]
    fn comment_preview_is_readable() {
        let document = parse("<!--  hello   comment  -->");
        let comment_id = document.children(document.root())[0];
        assert_eq!(tree_label(&document, comment_id), "<!-- hello comment -->");
    }

    #[test]
    fn child_order_is_preserved() {
        let document = parse("<ul><li>one</li><li>two</li><li>three</li></ul>");
        let ul_id = document.find_first_element("ul").unwrap();
        let children = document.children(ul_id);
        let labels: Vec<String> = children
            .iter()
            .copied()
            .map(|child_id| tree_label(&document, child_id))
            .collect();
        assert_eq!(labels, vec!["<li>", "<li>", "<li>"]);
        let texts: Vec<String> = children
            .iter()
            .copied()
            .map(|child_id| tree_label(&document, document.children(child_id)[0]))
            .collect();
        assert_eq!(texts, vec!["\"one\"", "\"two\"", "\"three\""]);
    }

    #[test]
    fn normal_html_tree_starts_with_document_and_html() {
        let document = parse("<html><head></head><body></body></html>");
        let adapter = DomTreeAdapter::new(&document);
        let mut state = TreeViewState::new();
        for node_id in default_expanded_nodes(&document) {
            state.expand(node_id);
        }
        let rows = state.rebuild_rows(&adapter);
        let labels: Vec<String> = rows.into_iter().map(|row| row.label).collect();
        assert_eq!(labels, vec!["#document", "<html>", "<head>", "<body>"]);
    }

    #[test]
    fn html_fragment_preserves_real_children_without_invented_html() {
        let document = parse("<p>First</p><p>Second</p>");
        let adapter = DomTreeAdapter::new(&document);
        let mut state = TreeViewState::new();
        state.expand(document.root());
        let rows = state.rebuild_rows(&adapter);
        let labels: Vec<String> = rows.into_iter().map(|row| row.label).collect();
        assert_eq!(labels, vec!["#document", "<p>", "<p>"]);
    }

    #[test]
    fn empty_document_still_exposes_document_root() {
        let document = parse("");
        let adapter = DomTreeAdapter::new(&document);
        let mut state = TreeViewState::new();
        state.expand(document.root());
        let rows = state.rebuild_rows(&adapter);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].label, "#document");
    }

    #[test]
    fn default_expansion_policy_expands_document_and_html_only() {
        let document =
            parse("<html><head><title>x</title></head><body><main></main></body></html>");
        let expanded = default_expanded_nodes(&document);
        let html_id = document.find_first_element("html").unwrap();
        assert_eq!(expanded, vec![document.root(), html_id]);
    }
}
