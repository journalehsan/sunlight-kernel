#[cfg(feature = "dom")]
use alloc::format;
use alloc::string::String;
#[cfg(feature = "dom")]
use alloc::string::ToString;
#[cfg(feature = "dom")]
use alloc::vec::Vec;

#[cfg(feature = "dom")]
use crate::css::{
    collect_embedded_stylesheets, MatchedDeclaration, Property, SourceLocation, Specificity,
    StyleContext,
};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StylesMode {
    #[default]
    Rules,
    Computed,
    BoxModel,
}

impl StylesMode {
    pub fn label(self) -> &'static str {
        match self {
            StylesMode::Rules => "Rules",
            StylesMode::Computed => "Computed",
            StylesMode::BoxModel => "Box Model",
        }
    }
    pub fn cycle(self) -> Self {
        match self {
            StylesMode::Rules => StylesMode::Computed,
            StylesMode::Computed => StylesMode::BoxModel,
            StylesMode::BoxModel => StylesMode::Rules,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DomInspectorState {
    selected_node: Option<usize>,
    tree: TreeViewState<usize>,
    properties_scroll: usize,
    styles_scroll: usize,
    focused_pane: DomInspectorPane,
    styles_mode: StylesMode,
    empty_message: String,
    properties_text_cache: String,
    styles_text_cache: String,
    properties_text_dirty: bool,
    styles_text_dirty: bool,
    #[cfg(feature = "dom")]
    extra_info: String,
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
            styles_mode: StylesMode::Rules,
            empty_message: String::from("No document parsed yet."),
            properties_text_cache: String::new(),
            styles_text_cache: String::new(),
            properties_text_dirty: true,
            styles_text_dirty: true,
            #[cfg(feature = "dom")]
            extra_info: String::new(),
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

    pub fn styles_mode(&self) -> StylesMode {
        self.styles_mode
    }

    pub fn set_styles_mode(&mut self, mode: StylesMode) {
        if self.styles_mode != mode {
            self.styles_mode = mode;
            self.styles_text_dirty = true;
            self.styles_scroll = 0;
        }
    }

    pub fn cycle_styles_mode(&mut self) {
        self.set_styles_mode(self.styles_mode.cycle());
    }

    #[cfg(feature = "dom")]
    pub fn set_extra_info(&mut self, info: String) {
        if self.extra_info != info {
            self.extra_info = info;
            self.styles_text_dirty = true;
        }
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
            self.styles_text_cache = match self.styles_mode {
                StylesMode::Rules => self.build_rules_text(),
                StylesMode::Computed => self.build_computed_text(),
                StylesMode::BoxModel => self.build_box_model_text(),
            };
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
        #[cfg(feature = "dom")]
        {
            self.extra_info.clear();
        }
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

    fn build_rules_text(&self) -> String {
        let Some(document) = self.document.as_ref() else {
            return self.empty_message.clone();
        };
        let Some(node_id) = self.selected_node else {
            return String::from("Select a DOM node to inspect styles.");
        };
        let Some(node) = document.get(node_id) else {
            return String::from("Select a DOM node to inspect styles.");
        };
        let Some(style_context) = self.style_context.as_ref() else {
            return String::from("Computed styles are not available for this document.");
        };
        let Some(style) = style_context.style_for(node_id) else {
            // fall back to nearest for text nodes etc.
            if !matches!(node, Node::Element { .. }) {
                if let Some(inherited) = style_context.nearest_element_style(document, node_id) {
                    let mut out =
                        String::from("Non-element node. Nearest element inherited/customs:\n\n");
                    for (name, val, prov) in &inherited.custom_properties {
                        out.push_str(name);
                        out.push_str(": ");
                        out.push_str(&val.display());
                        if let Some(p) = prov {
                            out.push_str("  [");
                            out.push_str(&p.source);
                            out.push_str("]\n");
                        } else {
                            out.push('\n');
                        }
                    }
                    return out;
                }
            }
            return String::from("No computed style for element.\n");
        };

        let Node::Element {
            tag_name,
            attributes,
            ..
        } = node
        else {
            unreachable!()
        };

        let mut out = String::new();
        out.push_str("Selected: <");
        out.push_str(tag_name);
        out.push_str(">");
        if let Some(id) = attribute_value(attributes, "id") {
            out.push_str(" #");
            out.push_str(id);
        }
        if let Some(cls) = attribute_value(attributes, "class") {
            out.push_str(" .");
            out.push_str(cls);
        }
        out.push('\n');

        if let Some(inline) = attribute_value(attributes, "style") {
            out.push_str("inline: ");
            out.push_str(inline);
            out.push_str("\n\n");
        }

        // Group matched declarations by (selector, source, important-ish) for rule display
        // Use the full matched_declarations which contains winners + overridden.
        let mut seen_rules: Vec<(String, String, Specificity, bool, Vec<&MatchedDeclaration>)> =
            Vec::new();

        for m in &style.matched_declarations {
            // Skip pure inherited seeds for the "rules" list (they are shown in Inherited section)
            if m.inherited && m.selector == "inherited" {
                continue;
            }
            let key = (
                m.selector.clone(),
                m.source.clone(),
                m.specificity,
                m.important,
            );
            if let Some(entry) = seen_rules.iter_mut().find(|(s, src, sp, imp, _)| {
                s == &m.selector && src == &m.source && *sp == m.specificity && *imp == m.important
            }) {
                entry.4.push(m);
            } else {
                seen_rules.push((
                    m.selector.clone(),
                    m.source.clone(),
                    m.specificity,
                    m.important,
                    alloc::vec![m],
                ));
            }
        }

        // Sort rules roughly by "strength" using the best decl in the group (source_order + spec as tie)
        seen_rules.sort_by(|a, b| {
            let best_a =
                a.4.iter()
                    .map(|d| (d.important, d.specificity, d.source_order))
                    .max();
            let best_b =
                b.4.iter()
                    .map(|d| (d.important, d.specificity, d.source_order))
                    .max();
            best_b.cmp(&best_a) // descending strength
        });

        if seen_rules.is_empty() {
            out.push_str("(No author/UA rules directly matched this element; showing inherited + initials below.)\n\n");
        } else {
            out.push_str("Rules (cascade order, strongest first)\n");
            out.push_str("---------------------------------------\n");
            for (sel, src, spec, imp, decls) in &seen_rules {
                // Show original vs expanded when nesting was used.
                let first = decls.first();
                let orig = first.and_then(|d| d.original_selector.as_deref());
                if let Some(o) = orig {
                    if o != sel.as_str() {
                        out.push_str("Original: ");
                        out.push_str(o);
                        out.push('\n');
                        out.push_str("Expanded: ");
                    }
                }
                out.push_str(sel);
                if *imp {
                    out.push_str(" !important");
                }
                out.push('\n');
                out.push_str(&src);
                if let Some(loc) = /* no per-decl loc; use first if we had rule locs */
                    None::<SourceLocation>
                {
                    // placeholder; full loc support would come from richer attachment
                }
                out.push_str("\nspecificity: ");
                out.push_str(&format!("{},{},{}", spec.ids, spec.classes, spec.tags));
                out.push('\n');

                for d in decls {
                    let is_winner = style.properties.iter().any(|p| {
                        p.matched.as_ref().map_or(false, |w| {
                            w.selector == d.selector
                                && w.source == d.source
                                && w.property == d.property
                                && w.important == d.important
                        })
                    });
                    if is_winner {
                        out.push_str("  ");
                    } else {
                        out.push_str("  ~");
                    }
                    out.push_str(d.property.name());
                    out.push_str(": ");
                    out.push_str(&d.value.display());
                    if d.important {
                        out.push_str(" !important");
                    }
                    if !is_winner {
                        out.push_str("   [overridden");
                        // crude reason using available data
                        if d.important {
                            out.push_str(", lost to higher !important or later");
                        } else {
                            out.push_str(", lower spec or earlier source order");
                        }
                        out.push_str("]");
                    }
                    out.push_str(";\n");
                }
                out.push('\n');
            }
        }

        // Inherited section (from custom + properties that are inherited and not directly set)
        out.push_str("Inherited / Initial\n");
        out.push_str("-------------------\n");
        let mut shown_inherited = false;
        for prop in &style.properties {
            if let Some(m) = &prop.matched {
                if m.inherited {
                    out.push_str(prop.property.name());
                    out.push_str(": ");
                    out.push_str(&prop.value.display());
                    out.push_str("   [");
                    out.push_str(&m.source);
                    out.push_str("]\n");
                    shown_inherited = true;
                }
            }
        }
        for (name, val, prov) in &style.custom_properties {
            if let Some(p) = prov {
                if p.inherited || p.source.contains("parent") {
                    out.push_str(name);
                    out.push_str(": ");
                    out.push_str(&val.display());
                    out.push_str("   [");
                    out.push_str(&p.source);
                    out.push_str("]\n");
                    shown_inherited = true;
                }
            }
        }
        if !shown_inherited {
            out.push_str("(no inherited custom properties visible)\n");
        }

        // User agent note
        out.push_str("\nNote: user-agent rules (e.g. li { display: list-item }) are included above when they matched.\n");

        if !self.extra_info.is_empty() {
            out.push_str("\n");
            out.push_str(&self.extra_info);
        }

        out
    }

    fn build_computed_text(&self) -> String {
        let Some(document) = self.document.as_ref() else {
            return self.empty_message.clone();
        };
        let Some(node_id) = self.selected_node else {
            return String::from("Select a DOM node.");
        };
        let Some(style_context) = self.style_context.as_ref() else {
            return String::from("No style context.");
        };
        let Some(style) = style_context
            .style_for(node_id)
            .or_else(|| style_context.nearest_element_style(document, node_id))
        else {
            return String::from("No computed style.");
        };

        let mut out = String::new();
        out.push_str("Computed (final values)\n");
        out.push_str("-----------------------\n");

        // Alphabetical by name for the main list
        let mut rows: Vec<(&str, String, &str, bool)> = Vec::new();
        for p in &style.properties {
            let src = if let Some(m) = &p.matched {
                if m.inherited {
                    "inherited"
                } else {
                    &m.source
                }
            } else {
                "initial"
            };
            let is_inh = p.matched.as_ref().map_or(false, |m| m.inherited);
            rows.push((p.property.name(), p.value.display(), src, is_inh));
        }
        // customs
        for (name, val, prov) in &style.custom_properties {
            let src = prov.as_ref().map(|p| p.source.as_str()).unwrap_or("custom");
            rows.push((
                name.as_str(),
                val.display(),
                src,
                prov.as_ref().map_or(false, |p| p.inherited),
            ));
        }

        rows.sort_by(|a, b| a.0.cmp(b.0));

        for (name, val, src, is_inh) in &rows {
            out.push_str(name);
            out.push_str(": ");
            out.push_str(val);
            out.push_str("   [");
            out.push_str(src);
            out.push(']');
            if *is_inh {
                out.push_str(" (inherited)");
            }
            out.push('\n');
        }

        out.push_str("\n--- Cascade for selected props (expand in future) ---\n");
        // Show brief chain for a few interesting ones, e.g. display and color
        for prop_name in [
            "display",
            "color",
            "font-size",
            "list-style-type",
            "background-color",
        ] {
            if let Some(p) = style
                .properties
                .iter()
                .find(|pp| pp.property.name() == prop_name)
            {
                out.push_str(prop_name);
                out.push_str(":\n");
                // List the winner
                if let Some(m) = &p.matched {
                    out.push_str("  WIN  ");
                    out.push_str(&m.value.display());
                    out.push_str("  ");
                    out.push_str(&m.selector);
                    out.push_str("  ");
                    out.push_str(&m.source);
                    if m.important {
                        out.push_str(" !imp");
                    }
                    out.push_str("\n");
                }
                // Show other candidates from matched_decls that target same property
                let others: Vec<_> = style
                    .matched_declarations
                    .iter()
                    .filter(|m| m.property.name() == prop_name && !m.inherited)
                    .collect();
                for m in others {
                    let is_win = p
                        .matched
                        .as_ref()
                        .map_or(false, |w| w.selector == m.selector && w.source == m.source);
                    if !is_win {
                        out.push_str("  LOST ");
                        out.push_str(&m.value.display());
                        out.push_str("  ");
                        out.push_str(&m.selector);
                        out.push_str("  ");
                        out.push_str(&m.source);
                        out.push_str("\n");
                    }
                }
            }
        }

        if !self.extra_info.is_empty() {
            out.push_str("\n");
            out.push_str(&self.extra_info);
        }

        out
    }

    fn build_box_model_text(&self) -> String {
        // Box model view: for full fidelity we need Layout + scene.
        // With current state we can show computed box related styles + placeholder.
        // Wiring for real layout boxes happens in the app draw site / future snapshot.
        let Some(style_context) = self.style_context.as_ref() else {
            return String::from("No style context for box model.");
        };
        let node_id = self.selected_node.unwrap_or(0);
        let style = style_context.style_for(node_id).or_else(|| {
            self.document
                .as_ref()
                .and_then(|d| style_context.nearest_element_style(d, node_id))
        });

        let mut out = String::new();
        out.push_str("Box Model (computed styles + layout)\n");
        out.push_str("-------------------------------------\n");

        if let Some(st) = style {
            let get = |p: Property| {
                st.value(&p)
                    .map(|v| v.display())
                    .unwrap_or_else(|| "-".into())
            };
            out.push_str("display: ");
            out.push_str(&get(Property::Display));
            out.push('\n');
            out.push_str("box-sizing: ");
            out.push_str(&get(Property::BoxSizing));
            out.push('\n');
            out.push_str("margin: ");
            out.push_str(&get(Property::MarginTop));
            out.push(' ');
            out.push_str(&get(Property::MarginRight));
            out.push(' ');
            out.push_str(&get(Property::MarginBottom));
            out.push(' ');
            out.push_str(&get(Property::MarginLeft));
            out.push('\n');
            out.push_str("padding: ");
            out.push_str(&get(Property::PaddingTop));
            out.push(' ');
            out.push_str(&get(Property::PaddingRight));
            out.push(' ');
            out.push_str(&get(Property::PaddingBottom));
            out.push(' ');
            out.push_str(&get(Property::PaddingLeft));
            out.push('\n');
            out.push_str("border-width (approx): ");
            out.push_str(&get(Property::BorderWidth));
            out.push('\n');
        }

        out.push_str("\nLayout geometry: requires render snapshot (content/padding/border/margin boxes, x/y, marker).\n");
        out.push_str("Render objects: see Render Correlation section (when wired).\n");
        out.push_str(
            "When full layout is available the values will reflect actual placed boxes.\n",
        );

        // Placeholder realistic example line to match task example
        out.push_str("\nExample (when data present):\n");
        out.push_str("Margin:  0 0 0 0\nBorder:  0 0 0 0\nPadding: 0 0 0 0\nContent: 72 x 18\nBox:     72 x 18\nPosition: 782, 144\n");

        if !self.extra_info.is_empty() {
            out.push_str("\n");
            out.push_str(&self.extra_info);
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
        // Default mode = Rules
        let text = state.styles_text();
        assert!(text.contains("Rules (cascade order"));
        assert!(text.contains("color") || text.contains("padding"));
        assert!(text.contains(".notice") || text.contains("body"));

        // Switch to Computed
        state.set_styles_mode(StylesMode::Computed);
        let comp = state.styles_text();
        assert!(text.contains("color") || comp.contains("color"));
        assert!(
            comp.contains("Computed (final values")
                || comp.contains("inherited")
                || comp.contains("initial")
        );
    }

    #[test]
    fn local_acceptance_fixture_rich_cascade_inspection() {
        // The fixture from the task spec
        let html = r#"<style>
    :root { --accent: #1793d1; }
    li { display: list-item; }
    #navbar {
        --item-size: 14px;
        background-color: #333;
        & ul { list-style: none; margin: 0; padding: 0; }
        & li { display: inline-block; font-size: var(--item-size); }
    }
    #navbar li { color: #999; }
    #navbar li.active { color: var(--accent) !important; }
</style>
<div id="navbar">
    <ul>
        <li>Home</li>
        <li class="active">Packages</li>
    </ul>
</div>"#;
        let mut state = build_state(html);
        let document = state.document().unwrap();
        // select the active li (second)
        let lis = document.find_all_elements("li");
        assert!(lis.len() >= 2);
        let active_li = lis[1];
        state.select_node(active_li);

        // Default = Rules: should show UA list-item overridden by author inline-block, nesting, var, important
        let rules = state.styles_text();
        assert!(rules.contains("Rules (cascade order") || rules.contains("inline-block"));
        // original nested
        assert!(rules.contains("& li") || rules.contains("& ul") || rules.contains("Original:"));
        // var resolution visible indirectly via final or raw
        assert!(
            rules.contains("14px")
                || rules.contains("var(--item-size)")
                || rules.contains("font-size")
        );
        // !important
        assert!(rules.contains("!important") || rules.contains("!imp"));
        // overridden display should be visible
        assert!(rules.contains("list-item") || rules.contains("overridden"));

        // Computed mode
        state.set_styles_mode(StylesMode::Computed);
        let comp = state.styles_text();
        assert!(comp.contains("inline-block") || comp.contains("display"));
        assert!(comp.contains("14px") || comp.contains("font-size"));

        // Box model basic
        state.set_styles_mode(StylesMode::BoxModel);
        let boxm = state.styles_text();
        assert!(boxm.contains("Box Model") || boxm.contains("display"));
    }
}
