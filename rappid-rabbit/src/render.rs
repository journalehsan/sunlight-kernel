//! Browser-side retained layout and scene pipeline.
//!
//! This module deliberately stops before full CSS layout.  It handles normal
//! block flow plus a small inline formatter; unsupported display values fall
//! back to inline, while positioning, floats, tables, flex, and grid are not
//! interpreted here.  The output is a generic `sunlight_ui::DocumentScene` so
//! the shared canvas stays unaware of HTML and CSS.

use alloc::{string::String, vec, vec::Vec};

use golden_fish::{Attribute, Document, Node, NodeId};
use sunlight_http::ParsedUrl;
use sunlight_ui::{
    widgets::{
        diff_scenes, DocumentNodeId, DocumentScene, PaintOrder, RenderInteraction, RenderObject,
        RenderObjectId, RenderObjectKind, ScenePatch,
    },
    Color, Rect, Size, VecText,
};

use crate::{
    css::{Color as CssColor, ComputedStyle, Property, PropertyValue, StyleContext},
    resources::discovery::resolve_url,
};

pub const MAX_LAYOUT_DEPTH: usize = 64;
pub const MAX_RENDER_OBJECTS: usize = 8_192;
pub const MAX_TEXT_FRAGMENTS: usize = 4_096;
pub const MAX_LINES: usize = 4_096;
pub const MAX_DOCUMENT_DIMENSION: u32 = 16_384;
/// Bump this when a future in-memory cache must invalidate retained scenes.
pub const DOCUMENT_SCENE_FORMAT_VERSION: u32 = 1;

/// Browser name for the generic canvas document-local node identity.
pub type DomNodeId = DocumentNodeId;

/// Cache identity reserved for the next phase. No cache stores widget
/// references or borrowed DOM data; a future bounded cache can own a scene or
/// a reparsable document snapshot behind this key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentCacheKey {
    pub final_url: String,
    pub viewport: Size,
    pub format_version: u32,
}

pub trait TextMeasurer {
    fn measure_width(&self, text: &str) -> u32;
    fn line_height(&self) -> u32;
}

impl<T: VecText + ?Sized> TextMeasurer for T {
    fn measure_width(&self, text: &str) -> u32 {
        self.measure_w(text)
    }

    fn line_height(&self) -> u32 {
        VecText::line_height(self)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayType {
    Block,
    Inline,
    InlineBlock,
    None,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedPaintStyle {
    pub color: Color,
    pub background: Color,
    pub font_size: u32,
    pub line_height: u32,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub text_align: String,
    pub border_color: Color,
    pub border_width: u32,
    pub border_visible: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextFragment {
    pub owner_node_id: DomNodeId,
    pub bounds: Rect,
    pub text: String,
    pub paint: ResolvedPaintStyle,
    pub link: Option<LinkTarget>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkTarget {
    pub anchor_node_id: DomNodeId,
    pub href: String,
    pub resolved_url: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageFragment {
    pub owner_node_id: DomNodeId,
    pub bounds: Rect,
    pub src: String,
    pub alt: String,
}

/// Geometry retained for an inline element. Inline elements do not establish
/// block formatting contexts, but they still need an owned retained object so
/// DOM-to-scene lookup never has to infer ownership from descendant text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InlineLayoutBox {
    pub owner_node_id: DomNodeId,
    pub bounds: Rect,
    pub paint: ResolvedPaintStyle,
    pub paint_order: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayoutNode {
    pub owner_node_id: DomNodeId,
    pub display: DisplayType,
    pub content_box: Rect,
    pub padding_box: Rect,
    pub border_box: Rect,
    pub margin_box: Rect,
    pub children: Vec<usize>,
    pub text_fragments: Vec<TextFragment>,
    pub image_fragments: Vec<ImageFragment>,
    pub visible: bool,
    pub paint: ResolvedPaintStyle,
    pub paint_order: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LayoutTree {
    pub nodes: Vec<LayoutNode>,
    pub root_nodes: Vec<usize>,
    /// Inline elements get a geometry-bearing layout record without becoming
    /// block formatting contexts of their own.
    pub inline_boxes: Vec<InlineLayoutBox>,
    pub content_size: Size,
}

#[derive(Clone, Debug)]
pub struct DocumentRenderState {
    pub document_id: u64,
    pub final_url: String,
    pub dom: Document,
    pub styles: StyleContext,
    pub layout_tree: LayoutTree,
    pub current_scene: DocumentScene,
    pub previous_scene: Option<DocumentScene>,
    pub last_patch: ScenePatch,
    pub scene_generation: u64,
    pub viewport: Size,
}

impl DocumentRenderState {
    pub fn new(
        document_id: u64,
        final_url: String,
        dom: Document,
        styles: StyleContext,
        viewport: Size,
        measurer: &dyn TextMeasurer,
    ) -> Self {
        let layout_tree =
            build_layout_tree_for_url(&dom, &styles, Some(final_url.as_str()), viewport, measurer);
        let current_scene = scene_from_layout(document_id, viewport, &layout_tree);
        Self {
            document_id,
            final_url,
            dom,
            styles,
            layout_tree,
            current_scene,
            previous_scene: None,
            last_patch: ScenePatch::default(),
            scene_generation: 1,
            viewport,
        }
    }

    /// Reflows without reparsing or refetching.  Object IDs derive from DOM
    /// node ID plus a local role/fragment slot, so unchanged state compares
    /// cleanly even though a fresh layout tree was produced.
    pub fn rebuild_for_viewport(&mut self, viewport: Size, measurer: &dyn TextMeasurer) -> bool {
        if viewport == self.viewport {
            return false;
        }
        self.rebuild(viewport, measurer);
        true
    }

    pub fn rebuild(&mut self, viewport: Size, measurer: &dyn TextMeasurer) {
        let layout_tree = build_layout_tree_for_url(
            &self.dom,
            &self.styles,
            Some(self.final_url.as_str()),
            viewport,
            measurer,
        );
        let scene = scene_from_layout(self.document_id, viewport, &layout_tree);
        self.last_patch = diff_scenes(&self.current_scene, &scene);
        self.previous_scene = Some(self.current_scene.clone());
        self.current_scene = scene;
        self.layout_tree = layout_tree;
        self.viewport = viewport;
        self.scene_generation = self.scene_generation.saturating_add(1);
    }

    /// Returns the stable key that a future bounded in-memory cache will use.
    /// This vertical slice intentionally keeps caching deferred until scene
    /// reuse policy can be introduced without retaining stale browser state.
    pub fn cache_key(&self) -> DocumentCacheKey {
        DocumentCacheKey {
            final_url: self.final_url.clone(),
            viewport: self.viewport,
            format_version: DOCUMENT_SCENE_FORMAT_VERSION,
        }
    }
}

#[derive(Clone)]
struct FlowStyle {
    node_id: DomNodeId,
    display: DisplayType,
    paint: ResolvedPaintStyle,
    width: Option<u32>,
    height: Option<u32>,
    margin: [u32; 4], // top, right, bottom, left
    padding: [u32; 4],
}

#[derive(Clone)]
enum InlineItem {
    Text {
        owner: DomNodeId,
        inline_owners: Vec<InlineOwner>,
        text: String,
        paint: ResolvedPaintStyle,
        link: Option<LinkTarget>,
    },
    Break {
        owner: InlineOwner,
    },
    Image {
        image: ImageFragment,
        inline_owners: Vec<InlineOwner>,
    },
}

#[derive(Clone)]
struct InlineOwner {
    node_id: DomNodeId,
    paint: ResolvedPaintStyle,
    paint_order: u32,
}

struct LayoutBuilder<'a> {
    document: &'a Document,
    styles: &'a StyleContext,
    base_url: Option<ParsedUrl>,
    measurer: &'a dyn TextMeasurer,
    tree: LayoutTree,
    line_count: usize,
    text_fragment_count: usize,
}

pub fn build_layout_tree(
    document: &Document,
    styles: &StyleContext,
    viewport: Size,
    measurer: &dyn TextMeasurer,
) -> LayoutTree {
    build_layout_tree_for_url(document, styles, None, viewport, measurer)
}

fn build_layout_tree_for_url(
    document: &Document,
    styles: &StyleContext,
    final_url: Option<&str>,
    viewport: Size,
    measurer: &dyn TextMeasurer,
) -> LayoutTree {
    let viewport = Size::new(
        viewport.w.clamp(1, MAX_DOCUMENT_DIMENSION),
        viewport.h.clamp(1, MAX_DOCUMENT_DIMENSION),
    );
    let mut builder = LayoutBuilder {
        document,
        styles,
        base_url: final_url.and_then(|url| ParsedUrl::parse(url).ok()),
        measurer,
        tree: LayoutTree::default(),
        line_count: 0,
        text_fragment_count: 0,
    };
    let mut cursor_y = 0i32;
    for &child in document.children(document.root()) {
        if let Some(index) = builder.layout_block(child, 0, viewport.w, &mut cursor_y, 0) {
            builder.tree.root_nodes.push(index);
        }
    }
    builder.tree.content_size = Size::new(viewport.w, cursor_y.max(0) as u32);
    builder.tree
}

impl<'a> LayoutBuilder<'a> {
    fn layout_block(
        &mut self,
        node_id: NodeId,
        available_x: i32,
        available_w: u32,
        cursor_y: &mut i32,
        depth: usize,
    ) -> Option<usize> {
        if depth > MAX_LAYOUT_DEPTH || self.tree.nodes.len() >= MAX_RENDER_OBJECTS {
            return None;
        }
        let Node::Element { children, .. } = self.document.get(node_id)? else {
            return None;
        };
        let style = self.flow_style(node_id);
        if style.display == DisplayType::None {
            return None;
        }
        if matches!(
            style.display,
            DisplayType::Inline | DisplayType::InlineBlock
        ) {
            // An inline at this level is handled by its nearest block parent.
            return None;
        }

        let outer_w = available_w.saturating_sub(style.margin[1].saturating_add(style.margin[3]));
        let horizontal_insets = style.padding[1]
            .saturating_add(style.padding[3])
            .saturating_add(style.paint.border_width.saturating_mul(2));
        let content_w = style
            .width
            .unwrap_or_else(|| outer_w.saturating_sub(horizontal_insets))
            .min(MAX_DOCUMENT_DIMENSION);
        let border_x = available_x.saturating_add(style.margin[3] as i32);
        let content_x = border_x
            .saturating_add(style.paint.border_width as i32)
            .saturating_add(style.padding[3] as i32);
        let border_y = cursor_y.saturating_add(style.margin[0] as i32);
        let content_y = border_y
            .saturating_add(style.paint.border_width as i32)
            .saturating_add(style.padding[0] as i32);
        let index = self.tree.nodes.len();
        self.tree.nodes.push(LayoutNode {
            owner_node_id: style.node_id,
            display: style.display,
            content_box: Rect::new(content_x, content_y, content_w, 0),
            padding_box: Rect::new(content_x, content_y, content_w, 0),
            border_box: Rect::new(border_x, border_y, 0, 0),
            margin_box: Rect::new(available_x, *cursor_y, available_w, 0),
            children: Vec::new(),
            text_fragments: Vec::new(),
            image_fragments: Vec::new(),
            visible: true,
            paint: style.paint.clone(),
            paint_order: node_id.min(u32::MAX as usize) as u32,
        });

        let mut inner_y = content_y;
        let mut inline_nodes = Vec::new();
        for &child in children {
            if self.is_block(child) {
                self.flush_inline(index, &inline_nodes, content_x, content_w, &mut inner_y);
                inline_nodes.clear();
                if let Some(child_index) =
                    self.layout_block(child, content_x, content_w, &mut inner_y, depth + 1)
                {
                    self.tree.nodes[index].children.push(child_index);
                }
            } else {
                inline_nodes.push(child);
            }
        }
        self.flush_inline(index, &inline_nodes, content_x, content_w, &mut inner_y);

        let used_content_h = inner_y.saturating_sub(content_y).max(0) as u32;
        let content_h = style
            .height
            .unwrap_or(0)
            .max(used_content_h)
            .min(MAX_DOCUMENT_DIMENSION);
        let padding_w = content_w
            .saturating_add(style.padding[1])
            .saturating_add(style.padding[3]);
        let padding_h = content_h
            .saturating_add(style.padding[0])
            .saturating_add(style.padding[2]);
        let border_w = padding_w.saturating_add(style.paint.border_width.saturating_mul(2));
        let border_h = padding_h.saturating_add(style.paint.border_width.saturating_mul(2));
        let margin_w = border_w
            .saturating_add(style.margin[1])
            .saturating_add(style.margin[3]);
        let margin_h = border_h
            .saturating_add(style.margin[0])
            .saturating_add(style.margin[2]);
        let node = &mut self.tree.nodes[index];
        node.content_box.h = content_h;
        node.padding_box = Rect::new(
            border_x.saturating_add(style.paint.border_width as i32),
            border_y.saturating_add(style.paint.border_width as i32),
            padding_w,
            padding_h,
        );
        node.border_box = Rect::new(border_x, border_y, border_w, border_h);
        node.margin_box = Rect::new(available_x, *cursor_y, margin_w.min(available_w), margin_h);
        *cursor_y = cursor_y.saturating_add(margin_h as i32);
        Some(index)
    }

    fn flush_inline(
        &mut self,
        parent_index: usize,
        nodes: &[NodeId],
        x: i32,
        w: u32,
        cursor_y: &mut i32,
    ) {
        if nodes.is_empty() || self.line_count >= MAX_LINES {
            return;
        }
        let mut items = Vec::new();
        for &node_id in nodes {
            let parent_style =
                self.flow_style(self.tree.nodes[parent_index].owner_node_id.0 as usize);
            self.collect_inline(node_id, Some(parent_style), None, Vec::new(), &mut items);
        }
        if items.is_empty() {
            return;
        }
        let align = self.tree.nodes[parent_index].paint.text_align.clone();
        let mut line_start = self.tree.nodes[parent_index].text_fragments.len();
        let mut line_x = x;
        let mut line_h = self.measurer.line_height().max(1);
        let mut had_content = false;
        for item in items {
            if self.text_fragment_count >= MAX_TEXT_FRAGMENTS || self.line_count >= MAX_LINES {
                break;
            }
            match item {
                InlineItem::Break { owner } => {
                    self.record_inline_box(owner, Rect::new(line_x, *cursor_y, 0, line_h.max(1)));
                    self.finish_line(parent_index, line_start, line_x, x, w, &align);
                    *cursor_y = cursor_y.saturating_add(line_h as i32);
                    self.line_count = self.line_count.saturating_add(1);
                    line_start = self.tree.nodes[parent_index].text_fragments.len();
                    line_x = x;
                    line_h = self.measurer.line_height().max(1);
                    had_content = false;
                }
                InlineItem::Image {
                    mut image,
                    inline_owners,
                } => {
                    let image_w = image.bounds.w.min(w.max(1));
                    if had_content
                        && line_x.saturating_add(image_w as i32) > x.saturating_add(w as i32)
                    {
                        self.finish_line(parent_index, line_start, line_x, x, w, &align);
                        *cursor_y = cursor_y.saturating_add(line_h as i32);
                        self.line_count = self.line_count.saturating_add(1);
                        line_start = self.tree.nodes[parent_index].text_fragments.len();
                        line_x = x;
                        line_h = self.measurer.line_height().max(1);
                    }
                    image.bounds.x = line_x;
                    image.bounds.y = *cursor_y;
                    image.bounds.w = image_w;
                    line_x = line_x.saturating_add(image_w as i32);
                    line_h = line_h.max(image.bounds.h);
                    for owner in inline_owners {
                        self.record_inline_box(owner, image.bounds);
                    }
                    self.tree.nodes[parent_index].image_fragments.push(image);
                    had_content = true;
                }
                InlineItem::Text {
                    owner,
                    inline_owners,
                    text,
                    paint,
                    link,
                } => {
                    for word in text.split_ascii_whitespace() {
                        let word_w = scaled_measure(self.measurer, word, paint.font_size);
                        let space_w = if had_content {
                            scaled_measure(self.measurer, " ", paint.font_size)
                        } else {
                            0
                        };
                        if had_content
                            && line_x
                                .saturating_add(space_w as i32)
                                .saturating_add(word_w as i32)
                                > x.saturating_add(w as i32)
                        {
                            self.finish_line(parent_index, line_start, line_x, x, w, &align);
                            *cursor_y = cursor_y.saturating_add(line_h as i32);
                            self.line_count = self.line_count.saturating_add(1);
                            line_start = self.tree.nodes[parent_index].text_fragments.len();
                            line_x = x;
                            line_h = self.measurer.line_height().max(1);
                            had_content = false;
                        }
                        if had_content {
                            line_x = line_x.saturating_add(space_w as i32);
                        }
                        let fragment_h = paint.line_height.max(1);
                        let bounds = Rect::new(line_x, *cursor_y, word_w, fragment_h);
                        self.tree.nodes[parent_index]
                            .text_fragments
                            .push(TextFragment {
                                owner_node_id: owner,
                                bounds,
                                text: String::from(word),
                                paint: paint.clone(),
                                link: link.clone(),
                            });
                        for inline_owner in &inline_owners {
                            self.record_inline_box(inline_owner.clone(), bounds);
                        }
                        self.text_fragment_count = self.text_fragment_count.saturating_add(1);
                        line_x = line_x.saturating_add(word_w as i32);
                        line_h = line_h.max(fragment_h);
                        had_content = true;
                    }
                }
            }
        }
        if had_content {
            self.finish_line(parent_index, line_start, line_x, x, w, &align);
            *cursor_y = cursor_y.saturating_add(line_h as i32);
            self.line_count = self.line_count.saturating_add(1);
        }
    }

    fn finish_line(
        &mut self,
        parent: usize,
        start: usize,
        line_x: i32,
        x: i32,
        w: u32,
        align: &str,
    ) {
        let used = line_x.saturating_sub(x).max(0) as u32;
        let offset = match align {
            "center" => w.saturating_sub(used) / 2,
            "right" | "end" => w.saturating_sub(used),
            _ => 0,
        };
        if offset > 0 {
            for fragment in &mut self.tree.nodes[parent].text_fragments[start..] {
                fragment.bounds.x = fragment.bounds.x.saturating_add(offset as i32);
            }
        }
    }

    fn collect_inline(
        &mut self,
        node_id: NodeId,
        inherited: Option<FlowStyle>,
        link: Option<LinkTarget>,
        inline_owners: Vec<InlineOwner>,
        output: &mut Vec<InlineItem>,
    ) {
        let Some(node) = self.document.get(node_id) else {
            return;
        };
        match node {
            Node::Text { content } => {
                if let Some(style) = inherited {
                    if !content.trim().is_empty() {
                        output.push(InlineItem::Text {
                            owner: style.node_id,
                            inline_owners,
                            text: content.clone(),
                            paint: style.paint,
                            link,
                        });
                    }
                }
            }
            Node::Element {
                tag_name,
                attributes,
                children,
            } => {
                let style = self.flow_style(node_id);
                if style.display == DisplayType::None {
                    return;
                }
                let owner = InlineOwner {
                    node_id: style.node_id,
                    paint: style.paint.clone(),
                    paint_order: node_id.min(u32::MAX as usize) as u32,
                };
                if tag_name.eq_ignore_ascii_case("br") {
                    output.push(InlineItem::Break { owner });
                    return;
                }
                if tag_name.eq_ignore_ascii_case("img") {
                    let width = style.width.unwrap_or(160).clamp(1, MAX_DOCUMENT_DIMENSION);
                    let height = style.height.unwrap_or(100).clamp(1, MAX_DOCUMENT_DIMENSION);
                    let mut image_owners = inline_owners;
                    image_owners.push(owner);
                    output.push(InlineItem::Image {
                        image: ImageFragment {
                            owner_node_id: style.node_id,
                            bounds: Rect::new(0, 0, width, height),
                            src: attr(attributes, "src").unwrap_or("").into(),
                            alt: attr(attributes, "alt").unwrap_or("Image").into(),
                        },
                        inline_owners: image_owners,
                    });
                    return;
                }
                let next_link = if tag_name.eq_ignore_ascii_case("a") {
                    let href = attr(attributes, "href").unwrap_or("");
                    Some(LinkTarget {
                        anchor_node_id: style.node_id,
                        href: href.into(),
                        resolved_url: self
                            .base_url
                            .as_ref()
                            .and_then(|base| resolve_url(base, href).ok()),
                    })
                } else {
                    link
                };
                let mut child_owners = inline_owners;
                child_owners.push(owner);
                for &child in children {
                    self.collect_inline(
                        child,
                        Some(style.clone()),
                        next_link.clone(),
                        child_owners.clone(),
                        output,
                    );
                }
            }
            Node::Document { children } => {
                for &child in children {
                    self.collect_inline(
                        child,
                        inherited.clone(),
                        link.clone(),
                        inline_owners.clone(),
                        output,
                    );
                }
            }
            Node::Comment { .. } => {}
        }
    }

    fn is_block(&self, node_id: NodeId) -> bool {
        matches!(self.document.get(node_id), Some(Node::Element { .. }))
            && self.flow_style(node_id).display == DisplayType::Block
    }

    fn record_inline_box(&mut self, owner: InlineOwner, bounds: Rect) {
        if let Some(existing) = self
            .tree
            .inline_boxes
            .iter_mut()
            .find(|existing| existing.owner_node_id == owner.node_id)
        {
            existing.bounds = union(existing.bounds, bounds);
            return;
        }
        self.tree.inline_boxes.push(InlineLayoutBox {
            owner_node_id: owner.node_id,
            bounds,
            paint: owner.paint,
            paint_order: owner.paint_order,
        });
    }

    fn flow_style(&self, node_id: NodeId) -> FlowStyle {
        let style = self.styles.nearest_element_style(self.document, node_id);
        let paint = resolved_paint(style, self.measurer);
        FlowStyle {
            node_id: DocumentNodeId(node_id as u64),
            display: display(style),
            width: value_px(style, Property::Width),
            height: value_px(style, Property::Height),
            margin: sides(
                style,
                Property::MarginTop,
                Property::MarginRight,
                Property::MarginBottom,
                Property::MarginLeft,
            ),
            padding: sides(
                style,
                Property::PaddingTop,
                Property::PaddingRight,
                Property::PaddingBottom,
                Property::PaddingLeft,
            ),
            paint,
        }
    }
}

fn display(style: Option<&ComputedStyle>) -> DisplayType {
    match style.and_then(|style| style.value(&Property::Display)) {
        Some(PropertyValue::Keyword(value)) if value.eq_ignore_ascii_case("block") => {
            DisplayType::Block
        }
        Some(PropertyValue::Keyword(value)) if value.eq_ignore_ascii_case("inline-block") => {
            DisplayType::InlineBlock
        }
        Some(PropertyValue::Keyword(value)) if value.eq_ignore_ascii_case("none") => {
            DisplayType::None
        }
        // Unknown display values safely use inline behavior in this vertical slice.
        _ => DisplayType::Inline,
    }
}

fn sides(
    style: Option<&ComputedStyle>,
    top: Property,
    right: Property,
    bottom: Property,
    left: Property,
) -> [u32; 4] {
    [
        value_px(style, top).unwrap_or(0),
        value_px(style, right).unwrap_or(0),
        value_px(style, bottom).unwrap_or(0),
        value_px(style, left).unwrap_or(0),
    ]
}

fn value_px(style: Option<&ComputedStyle>, property: Property) -> Option<u32> {
    match style.and_then(|style| style.value(&property)) {
        Some(PropertyValue::LengthPx(value)) if *value > 0 => {
            Some((*value as u32).min(MAX_DOCUMENT_DIMENSION))
        }
        Some(PropertyValue::LengthPx(0)) => Some(0),
        _ => None,
    }
}

fn css_color(value: Option<&PropertyValue>, fallback: Color) -> Color {
    match value {
        Some(PropertyValue::Color(CssColor::Rgb(r, g, b))) => Color::rgb(*r, *g, *b),
        Some(PropertyValue::Color(CssColor::Transparent)) => Color::TRANSPARENT,
        _ => fallback,
    }
}

fn keyword(style: Option<&ComputedStyle>, property: Property) -> String {
    match style.and_then(|style| style.value(&property)) {
        Some(PropertyValue::Keyword(value)) | Some(PropertyValue::Raw(value)) => {
            value.to_ascii_lowercase()
        }
        Some(PropertyValue::Normal) => String::from("normal"),
        _ => String::new(),
    }
}

fn resolved_paint(
    style: Option<&ComputedStyle>,
    measurer: &dyn TextMeasurer,
) -> ResolvedPaintStyle {
    let font_size = value_px(style, Property::FontSize).unwrap_or(16).max(1);
    let default_line_height = (font_size.saturating_mul(6) / 5).max(measurer.line_height());
    let line_height = value_px(style, Property::LineHeight)
        .unwrap_or(default_line_height)
        .max(1);
    let border_width = value_px(style, Property::BorderWidth).unwrap_or(0);
    let border_style = keyword(style, Property::BorderStyle);
    ResolvedPaintStyle {
        color: css_color(
            style.and_then(|style| style.value(&Property::Color)),
            Color::rgb(0, 0, 0),
        ),
        background: css_color(
            style.and_then(|style| style.value(&Property::BackgroundColor)),
            Color::TRANSPARENT,
        ),
        font_size,
        line_height,
        bold: matches!(
            keyword(style, Property::FontWeight).as_str(),
            "bold" | "bolder"
        ) || value_px(style, Property::FontWeight).is_some_and(|weight| weight >= 600),
        italic: matches!(
            keyword(style, Property::FontStyle).as_str(),
            "italic" | "oblique"
        ),
        underline: keyword(style, Property::TextDecoration).contains("underline"),
        text_align: keyword(style, Property::TextAlign),
        border_color: css_color(
            style.and_then(|style| style.value(&Property::BorderColor)),
            Color::rgb(0, 0, 0),
        ),
        border_width,
        border_visible: border_width > 0 && border_style != "none",
    }
}

fn scaled_measure(measurer: &dyn TextMeasurer, text: &str, font_size: u32) -> u32 {
    let measured = measurer.measure_width(text).max(1) as u64;
    ((measured.saturating_mul(font_size.max(1) as u64) / 16)
        .max(1)
        .min(MAX_DOCUMENT_DIMENSION as u64)) as u32
}

fn attr<'a>(attributes: &'a [Attribute], name: &str) -> Option<&'a str> {
    attributes
        .iter()
        .find(|attribute| attribute.name().eq_ignore_ascii_case(name))
        .map(Attribute::value)
}

fn object_id(owner: DomNodeId, role: u8, slot: usize) -> RenderObjectId {
    RenderObjectId((owner.0 << 32) | ((role as u64) << 24) | (slot.min(0x00ff_ffff) as u64))
}

pub fn scene_from_layout(document_id: u64, viewport: Size, layout: &LayoutTree) -> DocumentScene {
    let mut scene = DocumentScene::new(document_id, viewport, layout.content_size);
    for node in &layout.nodes {
        push(
            &mut scene,
            RenderObject {
                id: object_id(node.owner_node_id, 1, 0),
                owner_node_id: node.owner_node_id,
                kind: RenderObjectKind::Box,
                bounds: node.border_box,
                clip_bounds: None,
                paint_order: PaintOrder {
                    phase: 0,
                    document_order: node.paint_order,
                    ..PaintOrder::default()
                },
                interaction: None,
            },
        );
        if node.paint.background != Color::TRANSPARENT {
            push(
                &mut scene,
                RenderObject {
                    id: object_id(node.owner_node_id, 2, 0),
                    owner_node_id: node.owner_node_id,
                    kind: RenderObjectKind::Rectangle {
                        fill: node.paint.background,
                    },
                    bounds: node.border_box,
                    clip_bounds: None,
                    paint_order: PaintOrder {
                        phase: 1,
                        document_order: node.paint_order,
                        ..PaintOrder::default()
                    },
                    interaction: None,
                },
            );
        }
        if node.paint.border_visible {
            push(
                &mut scene,
                RenderObject {
                    id: object_id(node.owner_node_id, 3, 0),
                    owner_node_id: node.owner_node_id,
                    kind: RenderObjectKind::Border {
                        color: node.paint.border_color,
                        width: node.paint.border_width,
                    },
                    bounds: node.border_box,
                    clip_bounds: None,
                    paint_order: PaintOrder {
                        phase: 2,
                        document_order: node.paint_order,
                        ..PaintOrder::default()
                    },
                    interaction: None,
                },
            );
        }
        for (slot, fragment) in node.text_fragments.iter().enumerate() {
            let interaction = fragment.link.as_ref().map(|link| RenderInteraction::Link {
                owner_node_id: link.anchor_node_id,
                href: link.href.clone(),
                resolved_url: link.resolved_url.clone(),
            });
            push(
                &mut scene,
                RenderObject {
                    id: object_id(fragment.owner_node_id, 10, slot),
                    owner_node_id: fragment.owner_node_id,
                    kind: RenderObjectKind::Text {
                        text: fragment.text.clone(),
                        color: fragment.paint.color,
                        font_size: fragment.paint.font_size,
                        bold: fragment.paint.bold,
                        italic: fragment.paint.italic,
                        underline: fragment.paint.underline,
                    },
                    bounds: fragment.bounds,
                    clip_bounds: None,
                    paint_order: PaintOrder {
                        phase: 3,
                        document_order: node.paint_order.saturating_add(slot as u32),
                        ..PaintOrder::default()
                    },
                    interaction,
                },
            );
        }
        for (slot, image) in node.image_fragments.iter().enumerate() {
            push(
                &mut scene,
                RenderObject {
                    id: object_id(image.owner_node_id, 20, slot),
                    owner_node_id: image.owner_node_id,
                    kind: RenderObjectKind::ImagePlaceholder {
                        src: image.src.clone(),
                        alt: image.alt.clone(),
                    },
                    bounds: image.bounds,
                    clip_bounds: None,
                    paint_order: PaintOrder {
                        phase: 3,
                        document_order: node.paint_order.saturating_add(slot as u32),
                        ..PaintOrder::default()
                    },
                    interaction: None,
                },
            );
        }
    }
    for inline_box in &layout.inline_boxes {
        push(
            &mut scene,
            RenderObject {
                id: object_id(inline_box.owner_node_id, 1, 1),
                owner_node_id: inline_box.owner_node_id,
                kind: RenderObjectKind::Box,
                bounds: inline_box.bounds,
                clip_bounds: None,
                paint_order: PaintOrder {
                    phase: 0,
                    document_order: inline_box.paint_order,
                    ..PaintOrder::default()
                },
                interaction: None,
            },
        );
        if inline_box.paint.background != Color::TRANSPARENT {
            push(
                &mut scene,
                RenderObject {
                    id: object_id(inline_box.owner_node_id, 2, 1),
                    owner_node_id: inline_box.owner_node_id,
                    kind: RenderObjectKind::Rectangle {
                        fill: inline_box.paint.background,
                    },
                    bounds: inline_box.bounds,
                    clip_bounds: None,
                    paint_order: PaintOrder {
                        phase: 1,
                        document_order: inline_box.paint_order,
                        ..PaintOrder::default()
                    },
                    interaction: None,
                },
            );
        }
        if inline_box.paint.border_visible {
            push(
                &mut scene,
                RenderObject {
                    id: object_id(inline_box.owner_node_id, 3, 1),
                    owner_node_id: inline_box.owner_node_id,
                    kind: RenderObjectKind::Border {
                        color: inline_box.paint.border_color,
                        width: inline_box.paint.border_width,
                    },
                    bounds: inline_box.bounds,
                    clip_bounds: None,
                    paint_order: PaintOrder {
                        phase: 2,
                        document_order: inline_box.paint_order,
                        ..PaintOrder::default()
                    },
                    interaction: None,
                },
            );
        }
    }
    let mut text_owner_boxes: Vec<(DomNodeId, Rect)> = Vec::new();
    for object in &scene.objects {
        if !matches!(object.kind, RenderObjectKind::Text { .. }) {
            continue;
        }
        if let Some((_, bounds)) = text_owner_boxes
            .iter_mut()
            .find(|(owner, _)| *owner == object.owner_node_id)
        {
            *bounds = union(*bounds, object.bounds);
        } else {
            text_owner_boxes.push((object.owner_node_id, object.bounds));
        }
    }
    for (owner, bounds) in text_owner_boxes {
        if !scene.objects_for_node(owner).iter().any(|id| {
            scene
                .object(*id)
                .is_some_and(|object| matches!(object.kind, RenderObjectKind::Box))
        }) {
            push(
                &mut scene,
                RenderObject {
                    id: object_id(owner, 1, 1),
                    owner_node_id: owner,
                    kind: RenderObjectKind::Box,
                    bounds,
                    clip_bounds: None,
                    paint_order: PaintOrder {
                        phase: 0,
                        document_order: owner.0.min(u32::MAX as u64) as u32,
                        ..PaintOrder::default()
                    },
                    interaction: None,
                },
            );
        }
    }
    // Group link text fragments by anchor.  A single anchor may wrap across
    // lines, so it gets one retained hit object per unioned visible bounds.
    let mut links: Vec<(LinkTarget, Rect, Vec<RenderObjectId>)> = Vec::new();
    for object in &scene.objects {
        let Some(RenderInteraction::Link {
            owner_node_id,
            href,
            resolved_url,
        }) = &object.interaction
        else {
            continue;
        };
        let anchor = *owner_node_id;
        if let Some((_, bounds, ids)) = links
            .iter_mut()
            .find(|(target, _, _)| target.anchor_node_id == anchor && target.href == *href)
        {
            *bounds = union(*bounds, object.bounds);
            ids.push(object.id);
        } else {
            links.push((
                LinkTarget {
                    anchor_node_id: anchor,
                    href: href.clone(),
                    resolved_url: resolved_url.clone(),
                },
                object.bounds,
                vec![object.id],
            ));
        }
    }
    for (slot, (link, bounds, text_object_ids)) in links.into_iter().enumerate() {
        push(
            &mut scene,
            RenderObject {
                id: object_id(link.anchor_node_id, 30, slot),
                owner_node_id: link.anchor_node_id,
                kind: RenderObjectKind::Link {
                    href: link.href.clone(),
                    resolved_url: link.resolved_url.clone(),
                    text_object_ids,
                },
                bounds,
                clip_bounds: None,
                paint_order: PaintOrder {
                    phase: 4,
                    document_order: link.anchor_node_id.0.min(u32::MAX as u64) as u32,
                    ..PaintOrder::default()
                },
                interaction: Some(RenderInteraction::Link {
                    owner_node_id: link.anchor_node_id,
                    href: link.href,
                    resolved_url: link.resolved_url,
                }),
            },
        );
    }
    scene.finalize();
    scene
}

fn push(scene: &mut DocumentScene, object: RenderObject) {
    if scene.objects.len() < MAX_RENDER_OBJECTS {
        let _ = scene.push(object);
    }
}

fn union(left: Rect, right: Rect) -> Rect {
    let x = left.x.min(right.x);
    let y = left.y.min(right.y);
    Rect::new(
        x,
        y,
        (left.right().max(right.right()).saturating_sub(x)).max(0) as u32,
        (left.bottom().max(right.bottom()).saturating_sub(y)).max(0) as u32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css::{
        collect_embedded_stylesheets, parse_stylesheet, StyleContext, StylesheetSource,
    };
    use golden_fish::parse_html;

    struct TestMeasure;
    impl TextMeasurer for TestMeasure {
        fn measure_width(&self, text: &str) -> u32 {
            text.chars().count() as u32 * 8
        }
        fn line_height(&self) -> u32 {
            16
        }
    }

    fn render(html: &str) -> DocumentRenderState {
        let dom = parse_html(html).unwrap();
        let embedded = collect_embedded_stylesheets(&dom);
        let styles = StyleContext::build(&dom, &embedded);
        DocumentRenderState::new(
            1,
            String::from("http://example.test/"),
            dom,
            styles,
            Size::new(320, 240),
            &TestMeasure,
        )
    }

    #[test]
    fn blocks_stack_with_padding_margins_and_border() {
        let state = render("<style>body{padding:10px} p{display:block;margin:4px;border:1px solid red}</style><body><p>one</p><p>two</p></body>");
        let paragraphs: Vec<_> = state
            .layout_tree
            .nodes
            .iter()
            .filter(|node| {
                node.border_box.h > 0
                    && node
                        .text_fragments
                        .iter()
                        .any(|f| f.text == "one" || f.text == "two")
            })
            .collect();
        assert_eq!(paragraphs.len(), 2);
        assert!(paragraphs[1].border_box.y > paragraphs[0].border_box.bottom());
        assert_eq!(paragraphs[0].paint.border_width, 1);
    }

    #[test]
    fn block_boxes_honor_explicit_and_auto_width_and_minimum_height() {
        let state = render(
            "<style>body{padding:5px} #fixed{display:block;width:80px;height:40px;padding:3px;border:2px solid black} #auto{display:block}</style><body><p id=fixed>x</p><p id=auto>y</p></body>",
        );
        let fixed = state
            .layout_tree
            .nodes
            .iter()
            .find(|node| {
                node.text_fragments
                    .iter()
                    .any(|fragment| fragment.text == "x")
            })
            .unwrap();
        let automatic = state
            .layout_tree
            .nodes
            .iter()
            .find(|node| {
                node.text_fragments
                    .iter()
                    .any(|fragment| fragment.text == "y")
            })
            .unwrap();
        assert_eq!(fixed.content_box.w, 80);
        assert!(fixed.content_box.h >= 40);
        assert_eq!(fixed.border_box.w, 90);
        assert!(automatic.content_box.w > fixed.content_box.w);
        assert_eq!(
            state.current_scene.content_size.h,
            state.layout_tree.content_size.h
        );
    }

    #[test]
    fn none_is_excluded_and_text_wraps_and_breaks() {
        let state = render("<style>p{display:block;width:40px} .gone{display:none}</style><p>one two<br>three</p><p class=gone>hidden</p>");
        let words: Vec<_> = state
            .current_scene
            .objects
            .iter()
            .filter_map(|object| match &object.kind {
                RenderObjectKind::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert!(words.contains(&"one") && words.contains(&"two") && words.contains(&"three"));
        assert!(!words.contains(&"hidden"));
        let one = state.current_scene.objects.iter().find(|object| matches!(&object.kind, RenderObjectKind::Text { text, .. } if text == "one")).unwrap();
        let three = state.current_scene.objects.iter().find(|object| matches!(&object.kind, RenderObjectKind::Text { text, .. } if text == "three")).unwrap();
        assert!(three.bounds.y > one.bounds.y);
    }

    #[test]
    fn scene_has_stable_ownership_background_border_and_link_metadata() {
        let state = render("<style>p{display:block;background-color:#eee;border:1px solid #222} a{color:blue;font-weight:bold}</style><p><a href='/download'>Download</a></p>");
        assert!(state
            .current_scene
            .objects
            .iter()
            .any(|object| matches!(object.kind, RenderObjectKind::Rectangle { .. })));
        assert!(state
            .current_scene
            .objects
            .iter()
            .any(|object| matches!(object.kind, RenderObjectKind::Border { .. })));
        assert!(state
            .current_scene
            .objects
            .iter()
            .all(|object| object.id.0 != 0 || object.owner_node_id.0 == 0));
        let link = state
            .current_scene
            .objects
            .iter()
            .find(|object| matches!(object.kind, RenderObjectKind::Link { .. }))
            .unwrap();
        assert!(!state
            .current_scene
            .objects_for_node(link.owner_node_id)
            .is_empty());
    }

    #[test]
    fn every_visible_inline_element_has_owned_scene_objects() {
        let state = render(
            "<p><span><strong>bold</strong></span> <a href='/go'>link</a><br><em>after</em></p>",
        );
        for tag in ["p", "span", "strong", "a", "br", "em"] {
            let node = state.dom.find_first_element(tag).unwrap();
            assert!(
                !state
                    .current_scene
                    .objects_for_node(DocumentNodeId(node as u64))
                    .is_empty(),
                "{tag} has no retained objects"
            );
        }
        let bold = state
            .current_scene
            .objects
            .iter()
            .find(|object| matches!(&object.kind, RenderObjectKind::Text { text, bold: true, .. } if text == "bold"))
            .unwrap();
        assert!(bold.owner_node_id.0 > 0);
        assert!(state.current_scene.objects.iter().any(|object| {
            matches!(&object.kind, RenderObjectKind::Link { href, text_object_ids, .. } if href == "/go" && !text_object_ids.is_empty())
        }));
    }

    #[test]
    fn unchanged_scene_has_no_patch_and_geometry_change_is_dirty_update() {
        let mut state = render("<p>hello world</p>");
        assert_eq!(
            state.cache_key().format_version,
            DOCUMENT_SCENE_FORMAT_VERSION
        );
        assert_eq!(state.cache_key().viewport, Size::new(320, 240));
        state.rebuild(Size::new(320, 240), &TestMeasure);
        assert!(state.last_patch.is_empty());
        state.rebuild(Size::new(120, 240), &TestMeasure);
        assert!(state.last_patch.operations.iter().any(|operation| matches!(
            operation,
            sunlight_ui::widgets::ScenePatchOperation::Update { .. }
        )));
        assert!(!state.last_patch.dirty_regions.is_empty());
    }

    #[test]
    fn text_change_updates_only_its_stable_render_object() {
        let mut state = render("<p>before</p><p>unchanged</p>");
        let before_node = state
            .dom
            .find_first_element("p")
            .and_then(|paragraph| state.dom.children(paragraph).first().copied())
            .unwrap();
        let unchanged_id = state
            .current_scene
            .objects
            .iter()
            .find(|object| matches!(&object.kind, RenderObjectKind::Text { text, .. } if text == "unchanged"))
            .map(|object| object.id)
            .unwrap();
        if let Some(Node::Text { content }) = state.dom.get_mut(before_node) {
            *content = String::from("after");
        } else {
            panic!("expected paragraph text node");
        }
        state.rebuild(Size::new(320, 240), &TestMeasure);
        assert!(state.last_patch.operations.iter().any(|operation| matches!(
            operation,
            sunlight_ui::widgets::ScenePatchOperation::Update { object, .. }
                if matches!(&object.kind, RenderObjectKind::Text { text, .. } if text == "after")
        )));
        assert!(!state.last_patch.operations.iter().any(|operation| matches!(
            operation,
            sunlight_ui::widgets::ScenePatchOperation::Update { object, .. } if object.id == unchanged_id
        )));
    }

    #[test]
    fn style_change_updates_only_affected_nodes() {
        let mut state = render(
            "<style>.notice { color: red; background-color: #eeeeee; }</style><p class=notice>notice</p><p>unchanged</p>",
        );
        let unchanged_id = state
            .current_scene
            .objects
            .iter()
            .find(|object| matches!(&object.kind, RenderObjectKind::Text { text, .. } if text == "unchanged"))
            .map(|object| object.id)
            .unwrap();
        state.styles = StyleContext::build(
            &state.dom,
            &[parse_stylesheet(
                ".notice { color: blue; background-color: #dddddd; }",
                StylesheetSource::Embedded,
            )],
        );
        state.rebuild(Size::new(320, 240), &TestMeasure);
        assert!(state.last_patch.operations.iter().any(|operation| matches!(
            operation,
            sunlight_ui::widgets::ScenePatchOperation::Update { object, .. }
                if matches!(&object.kind, RenderObjectKind::Text { text, color, .. } if text == "notice" && *color == Color::rgb(0, 0, 255))
        )));
        assert!(!state.last_patch.operations.iter().any(|operation| matches!(
            operation,
            sunlight_ui::widgets::ScenePatchOperation::Update { object, .. } if object.id == unchanged_id
        )));
    }

    #[test]
    fn acceptance_fixture_emits_retained_web_scene() {
        let state = render(include_str!("../tests/fixtures/document-canvas.html"));
        assert!(state
            .current_scene
            .objects
            .iter()
            .any(|object| matches!(object.kind, RenderObjectKind::Rectangle { .. })));
        assert!(state.current_scene.objects.iter().any(|object| {
            matches!(&object.kind, RenderObjectKind::Text { text, underline: true, bold: true, .. } if text == "Download")
        }));
        assert!(state.current_scene.objects.iter().any(|object| {
            matches!(&object.kind, RenderObjectKind::Link { href, .. } if href == "/download")
        }));
        assert!(state.current_scene.content_size.h > 0);
    }

    #[test]
    fn image_placeholder_keeps_owner_and_source_metadata() {
        let state = render("<p><img src='/logo.simg' alt='Sunlight logo'></p>");
        let image_node = state.dom.find_first_element("img").unwrap();
        let image = state
            .current_scene
            .objects
            .iter()
            .find(|object| matches!(object.kind, RenderObjectKind::ImagePlaceholder { .. }))
            .unwrap();
        assert_eq!(image.owner_node_id, DocumentNodeId(image_node as u64));
        assert!(matches!(
            &image.kind,
            RenderObjectKind::ImagePlaceholder { src, alt }
                if src == "/logo.simg" && alt == "Sunlight logo"
        ));
        assert!(state
            .current_scene
            .objects_for_node(DocumentNodeId(image_node as u64))
            .contains(&image.id));
    }

    #[test]
    fn retained_scene_is_paint_ordered_and_reflows_without_navigation() {
        let mut state = render("<p style='background-color:#eee;border:1px solid black'>wide words wrap after a narrower viewport</p>");
        assert!(state
            .current_scene
            .objects
            .windows(2)
            .all(|pair| pair[0].paint_order <= pair[1].paint_order));

        let document_id = state.document_id;
        let final_url = state.final_url.clone();
        assert!(state.rebuild_for_viewport(Size::new(120, 240), &TestMeasure));
        assert_eq!(state.document_id, document_id);
        assert_eq!(state.final_url, final_url);
        assert_eq!(state.viewport, Size::new(120, 240));
        assert_eq!(state.scene_generation, 2);
    }
}
