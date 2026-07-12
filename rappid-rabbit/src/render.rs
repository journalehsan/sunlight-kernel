//! Browser-side retained layout and scene pipeline.
//!
//! This module deliberately stops before full CSS layout.  It handles normal
//! block flow plus a small inline formatter; unsupported display values fall
//! back to inline, while positioning, floats, tables, flex, and grid are not
//! interpreted here.  The output is a generic `sunlight_ui::DocumentScene` so
//! the shared canvas stays unaware of HTML and CSS.

use alloc::{format, string::String, vec, vec::Vec};

use golden_fish::{Attribute, Document, Node, NodeId};
use sunlight_http::ParsedUrl;
use sunlight_ui::{
    widgets::{
        diff_scenes, CornerRadii, DocumentFontFamily, DocumentNodeId, DocumentScene, PaintOrder,
        RenderInteraction, RenderObject, RenderObjectId, RenderObjectKind, ScenePatch,
    },
    Color, Rect, Size, VecText,
};

use crate::{
    css::{Color as CssColor, ComputedStyle, Property, PropertyValue, StyleContext},
    form::FormControlState,
    images::ImageCache,
    resources::discovery::resolve_url,
};

pub const MAX_LAYOUT_DEPTH: usize = 64;
pub const MAX_RENDER_OBJECTS: usize = 8_192;
pub const MAX_TEXT_FRAGMENTS: usize = 4_096;
pub const MAX_LINES: usize = 4_096;
pub const MAX_LIST_MARKERS: usize = 2_048;
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
    fn measure_width_for(&self, _family: DocumentFontFamily, text: &str) -> u32 {
        self.measure_width(text)
    }
    fn line_height_for(&self, _family: DocumentFontFamily) -> u32 {
        self.line_height()
    }
    /// Returns the advance of the face that will actually paint this CSS text
    /// size.  The default preserves the old proportional approximation for
    /// simple measurers; applications with a finite font-face set should
    /// override it so layout and painting share exact metrics.
    fn measure_width_for_size(
        &self,
        family: DocumentFontFamily,
        font_size: u32,
        text: &str,
    ) -> u32 {
        let measured = self.measure_width_for(family, text).max(1) as u64;
        ((measured.saturating_mul(font_size.max(1) as u64) / 16)
            .max(1)
            .min(MAX_DOCUMENT_DIMENSION as u64)) as u32
    }
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
    ListItem,
    Flex,
    InlineFlex,
    Inline,
    InlineBlock,
    Table,
    TableHeaderGroup,
    TableRowGroup,
    TableFooterGroup,
    TableRow,
    TableCell,
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
    pub line_through: bool,
    pub monospace: bool,
    pub font_family: DocumentFontFamily,
    pub white_space: String,
    pub baseline_offset: i32,
    pub text_align: String,
    pub border_color: Color,
    pub border_width: u32,
    pub border_visible: bool,
    pub border_colors: [Color; 4],
    pub border_widths: [u32; 4],
    pub corner_radii: CornerRadii,
    pub box_shadow: Option<ResolvedBoxShadow>,
    pub background_image: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolvedBoxShadow {
    pub offset_x: i32,
    pub offset_y: i32,
    pub blur: u32,
    pub spread: i32,
    pub color: Color,
    pub inset: bool,
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
    pub resolved_url: Option<String>,
    pub alt: String,
    pub intrinsic_size: Option<Size>,
    pub decoded: Option<alloc::sync::Arc<sunlight_ui::widgets::RasterImage>>,
    pub link: Option<LinkTarget>,
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
    pub control: Option<ControlLayout>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControlLayout {
    pub label: String,
    pub placeholder: String,
    pub value: String,
    pub disabled: bool,
    pub editable: bool,
    pub kind: u8,
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
    pub marker: Option<ListMarker>,
    pub visible: bool,
    pub paint: ResolvedPaintStyle,
    pub paint_order: u32,
    pub float_side: String,
    pub clear: String,
    pub float_containing_block: Option<DomNodeId>,
    pub table_dimensions: Option<(usize, usize)>,
}

/// A generated list marker, deliberately separate from DOM text.  Its owner
/// is the `li`, so retained scene lookup and inspector selection stay stable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListMarker {
    pub owner_node_id: DomNodeId,
    pub bounds: Rect,
    pub label: Option<String>,
    pub shape: MarkerShape,
    pub ordinal: u32,
    pub style_type: String,
    pub paint_order: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkerShape {
    Text,
    Disc,
    Circle,
    Square,
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
    images: ImageCache,
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
        Self::new_with_images(
            document_id,
            final_url,
            dom,
            styles,
            viewport,
            measurer,
            ImageCache::default(),
        )
    }

    pub fn new_with_images(
        document_id: u64,
        final_url: String,
        dom: Document,
        styles: StyleContext,
        viewport: Size,
        measurer: &dyn TextMeasurer,
        images: ImageCache,
    ) -> Self {
        let layout_tree = build_layout_tree_for_url_with_images(
            &dom,
            &styles,
            Some(final_url.as_str()),
            viewport,
            measurer,
            &images,
        );
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
            images,
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
        let layout_tree = build_layout_tree_for_url_with_images(
            &self.dom,
            &self.styles,
            Some(self.final_url.as_str()),
            viewport,
            measurer,
            &self.images,
        );
        let scene = scene_from_layout(self.document_id, viewport, &layout_tree);
        self.last_patch = diff_scenes(&self.current_scene, &scene);
        self.previous_scene = Some(self.current_scene.clone());
        self.current_scene = scene;
        self.layout_tree = layout_tree;
        self.viewport = viewport;
        self.scene_generation = self.scene_generation.saturating_add(1);
    }

    pub fn rebuild_for_images(&mut self, measurer: &dyn TextMeasurer, images: ImageCache) {
        self.images = images;
        self.rebuild(self.viewport, measurer);
    }

    /// Patch one retained control after browser-owned form state changes.
    pub fn patch_control(
        &mut self,
        control_id: DomNodeId,
        state: &FormControlState,
        focused: bool,
        measurer: &dyn TextMeasurer,
    ) -> bool {
        let previous = self.current_scene.clone();
        let Some(index) = self.current_scene.objects.iter().position(|object| {
            object.owner_node_id == control_id
                && matches!(object.kind, RenderObjectKind::Control { .. })
        }) else {
            return false;
        };
        let object = &mut self.current_scene.objects[index];
        let RenderObjectKind::Control {
            value,
            focused: visual_focus,
            caret_offset,
            ..
        } = &mut object.kind
        else {
            return false;
        };
        *visual_focus = focused;
        match state {
            FormControlState::Text { state, .. } => {
                *value = state.current_value.clone();
                *caret_offset = Some(measurer.measure_width(
                    &state.current_value[..state.cursor_position.min(state.current_value.len())],
                ));
            }
            FormControlState::Button { .. } => *caret_offset = None,
        }
        self.current_scene.finalize();
        self.last_patch = diff_scenes(&previous, &self.current_scene);
        self.previous_scene = Some(previous);
        true
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
    width_percent: Option<u32>,
    min_width: u32,
    max_width: Option<u32>,
    height: Option<u32>,
    min_height: u32,
    margin: [u32; 4], // top, right, bottom, left
    margin_auto: [bool; 4],
    padding: [u32; 4],
    flex_direction: String,
    flex_wrap: String,
    justify_content: String,
    align_items: String,
    align_content: String,
    row_gap: u32,
    column_gap: u32,
    float: String,
    clear: String,
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
    Control {
        owner: InlineOwner,
        control: ControlLayout,
        width: u32,
        height: u32,
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
    images: &'a ImageCache,
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
    build_layout_tree_for_url_with_images(
        document,
        styles,
        None,
        viewport,
        measurer,
        &ImageCache::default(),
    )
}

fn build_layout_tree_for_url_with_images(
    document: &Document,
    styles: &StyleContext,
    final_url: Option<&str>,
    viewport: Size,
    measurer: &dyn TextMeasurer,
    images: &ImageCache,
) -> LayoutTree {
    let viewport = Size::new(
        viewport.w.clamp(1, MAX_DOCUMENT_DIMENSION),
        viewport.h.clamp(1, MAX_DOCUMENT_DIMENSION),
    );
    let mut builder = LayoutBuilder {
        document,
        styles,
        base_url: final_url.and_then(|url| ParsedUrl::parse(url).ok()),
        images,
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
    fn layout_table(
        &mut self,
        node_id: NodeId,
        available_x: i32,
        available_w: u32,
        cursor_y: &mut i32,
        depth: usize,
        mut style: FlowStyle,
    ) -> Option<usize> {
        const MAX_TABLE_ROWS: usize = 256;
        const MAX_TABLE_COLUMNS: usize = 32;
        const MAX_TABLE_CELLS: usize = 4096;
        let mut rows = Vec::new();
        self.collect_table_rows(node_id, &mut rows, depth, MAX_TABLE_ROWS);
        let row_cells = rows
            .iter()
            .map(|row| {
                self.document
                    .children(*row)
                    .iter()
                    .copied()
                    .filter(|cell| {
                        self.document
                            .get(*cell)
                            .and_then(Node::tag_name)
                            .is_some_and(|tag| {
                                tag.eq_ignore_ascii_case("td") || tag.eq_ignore_ascii_case("th")
                            })
                    })
                    .map(|cell| {
                        let span = match self.document.get(cell) {
                            Some(Node::Element { attributes, .. }) => attr(attributes, "colspan")
                                .and_then(|value| value.parse::<usize>().ok())
                                .unwrap_or(1)
                                .clamp(1, MAX_TABLE_COLUMNS),
                            _ => 1,
                        };
                        (cell, span)
                    })
                    .take(MAX_TABLE_COLUMNS)
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let columns = row_cells
            .iter()
            .map(|cells| cells.iter().map(|(_, span)| *span).sum::<usize>())
            .max()
            .unwrap_or(0)
            .min(MAX_TABLE_COLUMNS);
        if columns == 0 {
            return None;
        }
        if let Some(percent) = style.width_percent {
            style.width = Some(((available_w as u64 * percent as u64) / 10_000) as u32);
        }
        let mut preferred = vec![24u32; columns];
        for cells in &row_cells {
            let mut column = 0usize;
            for (cell, span) in cells {
                let span = (*span).min(columns.saturating_sub(column)).max(1);
                let cell_style = self.flow_style(*cell);
                let each = self
                    .inline_preferred_width(*cell)
                    .saturating_add(
                        cell_style.padding[1]
                            + cell_style.padding[3]
                            + cell_style.paint.border_width * 2,
                    )
                    .min(320)
                    .div_ceil(span as u32);
                for width in &mut preferred[column..column + span] {
                    *width = (*width).max(each);
                }
                column += span;
            }
        }
        let preferred_total = preferred.iter().copied().sum::<u32>().max(1);
        let shrink_to_fit = matches!(style.float.as_str(), "left" | "right");
        let content_w = style
            .width
            .unwrap_or_else(|| {
                if shrink_to_fit {
                    preferred_total.min(available_w)
                } else {
                    available_w
                }
            })
            .max(style.min_width)
            .min(style.max_width.unwrap_or(MAX_DOCUMENT_DIMENSION))
            .min(available_w.max(1));
        let mut widths = vec![0u32; columns];
        if preferred_total <= content_w {
            let extra = content_w - preferred_total;
            for (index, width) in widths.iter_mut().enumerate() {
                *width = preferred[index].saturating_add(extra / columns as u32);
            }
        } else {
            for (index, width) in widths.iter_mut().enumerate() {
                *width =
                    ((preferred[index] as u64 * content_w as u64) / preferred_total as u64) as u32;
                *width = (*width).max(16);
            }
        }
        let assigned = widths.iter().copied().sum::<u32>();
        if assigned != content_w {
            let last = widths.last_mut().unwrap();
            if assigned < content_w {
                *last = last.saturating_add(content_w - assigned);
            } else {
                *last = last.saturating_sub(assigned - content_w).max(1);
            }
        }
        let border_x = available_x.saturating_add(style.margin[3] as i32);
        let border_y = cursor_y.saturating_add(style.margin[0] as i32);
        let content_x = border_x
            .saturating_add(style.paint.border_width as i32)
            .saturating_add(style.padding[3] as i32);
        let content_y = border_y
            .saturating_add(style.paint.border_width as i32)
            .saturating_add(style.padding[0] as i32);
        let index = self.tree.nodes.len();
        self.tree.nodes.push(LayoutNode {
            owner_node_id: style.node_id,
            display: DisplayType::Table,
            content_box: Rect::new(content_x, content_y, content_w, 0),
            padding_box: Rect::new(border_x, border_y, 0, 0),
            border_box: Rect::new(border_x, border_y, 0, 0),
            margin_box: Rect::new(available_x, *cursor_y, 0, 0),
            children: Vec::new(),
            text_fragments: Vec::new(),
            image_fragments: Vec::new(),
            marker: None,
            visible: true,
            paint: style.paint.clone(),
            paint_order: node_id.min(u32::MAX as usize) as u32,
            float_side: style.float.clone(),
            clear: style.clear.clone(),
            float_containing_block: None,
            table_dimensions: Some((row_cells.len(), columns)),
        });
        let mut row_y = content_y;
        let mut cell_count = 0usize;
        for cells in row_cells {
            if cell_count >= MAX_TABLE_CELLS {
                break;
            }
            let mut x = content_x;
            let mut row_h = 0u32;
            let mut column = 0usize;
            for (cell, span) in cells {
                if cell_count >= MAX_TABLE_CELLS {
                    break;
                }
                let span = span.min(columns.saturating_sub(column)).max(1);
                let cell_width = widths[column..column + span].iter().copied().sum::<u32>();
                let mut cell_cursor = row_y;
                if let Some(cell_index) =
                    self.layout_block(cell, x, cell_width, &mut cell_cursor, depth + 1)
                {
                    row_h = row_h.max(cell_cursor.saturating_sub(row_y).max(0) as u32);
                    self.tree.nodes[index].children.push(cell_index);
                }
                x = x.saturating_add(cell_width as i32);
                column += span;
                cell_count += 1;
            }
            row_h = row_h.max(1);
            row_y = row_y.saturating_add(row_h as i32);
        }
        let content_h = row_y.saturating_sub(content_y).max(0) as u32;
        let padding_w = content_w.saturating_add(style.padding[1] + style.padding[3]);
        let padding_h = content_h.saturating_add(style.padding[0] + style.padding[2]);
        let border_w = padding_w.saturating_add(style.paint.border_width * 2);
        let border_h = padding_h.saturating_add(style.paint.border_width * 2);
        let margin_h = border_h.saturating_add(style.margin[0] + style.margin[2]);
        let node = &mut self.tree.nodes[index];
        node.content_box.h = content_h;
        node.padding_box = Rect::new(
            border_x + style.paint.border_width as i32,
            border_y + style.paint.border_width as i32,
            padding_w,
            padding_h,
        );
        node.border_box = Rect::new(border_x, border_y, border_w, border_h);
        node.margin_box = Rect::new(
            available_x,
            *cursor_y,
            border_w.saturating_add(style.margin[1] + style.margin[3]),
            margin_h,
        );
        *cursor_y = cursor_y.saturating_add(margin_h as i32);
        Some(index)
    }

    fn collect_table_rows(
        &self,
        node_id: NodeId,
        rows: &mut Vec<NodeId>,
        depth: usize,
        limit: usize,
    ) {
        if rows.len() >= limit || depth > MAX_LAYOUT_DEPTH {
            return;
        }
        for &child in self.document.children(node_id) {
            let tag = self
                .document
                .get(child)
                .and_then(Node::tag_name)
                .unwrap_or("");
            if tag.eq_ignore_ascii_case("tr") {
                rows.push(child);
            } else if matches!(
                tag.to_ascii_lowercase().as_str(),
                "thead" | "tbody" | "tfoot"
            ) {
                self.collect_table_rows(child, rows, depth + 1, limit);
            }
            if rows.len() >= limit {
                break;
            }
        }
    }

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
        let mut style = self.flow_style(node_id);
        if style.display == DisplayType::None {
            return None;
        }
        if matches!(
            style.display,
            DisplayType::Inline | DisplayType::InlineBlock
        ) && style.float == "none"
        {
            // An inline at this level is handled by its nearest block parent.
            return None;
        }
        if style.display == DisplayType::Table {
            return self.layout_table(node_id, available_x, available_w, cursor_y, depth, style);
        }
        if style.width.is_none() && matches!(style.float.as_str(), "left" | "right") {
            style.width = Some(self.inline_preferred_width(node_id).min(available_w.max(1)));
        }
        if self
            .document
            .get(node_id)
            .and_then(Node::tag_name)
            .is_some_and(|tag_name| tag_name.eq_ignore_ascii_case("img"))
        {
            return self.layout_block_image(node_id, available_x, available_w, cursor_y, style);
        }

        if let Some(percent) = style.width_percent {
            style.width = Some(
                ((available_w as u64 * percent as u64) / 10_000).min(MAX_DOCUMENT_DIMENSION as u64)
                    as u32,
            );
        }
        let horizontal_insets = style.padding[1]
            .saturating_add(style.padding[3])
            .saturating_add(style.paint.border_width.saturating_mul(2));
        if style.width.is_some() && (style.margin_auto[1] || style.margin_auto[3]) {
            let occupied = style
                .width
                .unwrap_or(0)
                .saturating_add(horizontal_insets)
                .saturating_add(if style.margin_auto[1] {
                    0
                } else {
                    style.margin[1]
                })
                .saturating_add(if style.margin_auto[3] {
                    0
                } else {
                    style.margin[3]
                });
            let free = available_w.saturating_sub(occupied);
            match (style.margin_auto[3], style.margin_auto[1]) {
                (true, true) => {
                    style.margin[3] = free / 2;
                    style.margin[1] = free - style.margin[3];
                }
                (true, false) => style.margin[3] = free,
                (false, true) => style.margin[1] = free,
                _ => {}
            }
        }
        let outer_w = available_w.saturating_sub(style.margin[1].saturating_add(style.margin[3]));
        let content_w = style
            .width
            .unwrap_or_else(|| outer_w.saturating_sub(horizontal_insets))
            .max(style.min_width)
            .min(style.max_width.unwrap_or(MAX_DOCUMENT_DIMENSION))
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
            marker: None,
            visible: true,
            paint: style.paint.clone(),
            paint_order: node_id.min(u32::MAX as usize) as u32,
            float_side: style.float.clone(),
            clear: style.clear.clone(),
            float_containing_block: None,
            table_dimensions: None,
        });

        let used_content_h = if matches!(style.display, DisplayType::Flex | DisplayType::InlineFlex)
        {
            self.layout_flex_children(
                index, children, content_x, content_y, content_w, &style, depth,
            )
        } else {
            // A marker column exists only when computed list style emits a marker.
            let is_list_item = style.display == DisplayType::ListItem;
            let marker_w = if is_list_item {
                self.list_marker(node_id, content_x, content_y, 24, &style)
                    .map(|marker| {
                        self.tree.nodes[index].marker = Some(marker);
                        24
                    })
                    .unwrap_or(0)
            } else {
                0
            };
            let child_x = content_x.saturating_add(marker_w as i32);
            let child_w = content_w.saturating_sub(marker_w);
            let mut inner_y = content_y;
            let mut inline_nodes = Vec::new();
            let mut active_floats: Vec<(String, Rect)> = Vec::new();
            let self_is_floated_anchor = style.float != "none"
                && self
                    .document
                    .get(node_id)
                    .and_then(Node::tag_name)
                    .is_some_and(|tag| tag.eq_ignore_ascii_case("a"));
            if self_is_floated_anchor {
                self.flush_inline(index, &[node_id], child_x, child_w, &mut inner_y);
            } else {
                for &child in children {
                    if self.is_block(child) {
                        self.flush_inline(index, &inline_nodes, child_x, child_w, &mut inner_y);
                        inline_nodes.clear();
                        let child_style = self.flow_style(child);
                        if child_style.clear != "none" {
                            let clear_left = matches!(child_style.clear.as_str(), "left" | "both");
                            let clear_right =
                                matches!(child_style.clear.as_str(), "right" | "both");
                            let cleared_y = active_floats
                                .iter()
                                .filter(|(side, _)| {
                                    (clear_left && side == "left")
                                        || (clear_right && side == "right")
                                })
                                .map(|(_, bounds)| bounds.bottom())
                                .max()
                                .unwrap_or(inner_y);
                            inner_y = inner_y.max(cleared_y);
                        }
                        if matches!(child_style.float.as_str(), "left" | "right") {
                            const MAX_ACTIVE_FLOATS: usize = 64;
                            let start = self.tree.nodes.len();
                            let inline_start = self.tree.inline_boxes.len();
                            let mut float_y = inner_y;
                            let mut temp_cursor = float_y;
                            if let Some(child_index) = self.layout_block(
                                child,
                                child_x,
                                child_w,
                                &mut temp_cursor,
                                depth + 1,
                            ) {
                                let end = self.tree.nodes.len();
                                let inline_end = self.tree.inline_boxes.len();
                                let item_w = self.tree.nodes[child_index]
                                    .margin_box
                                    .w
                                    .min(child_w)
                                    .max(1);
                                for _ in 0..MAX_ACTIVE_FLOATS {
                                    let left_used = active_floats
                                        .iter()
                                        .filter(|(side, bounds)| {
                                            side == "left"
                                                && bounds.y <= float_y
                                                && bounds.bottom() > float_y
                                        })
                                        .map(|(_, b)| b.w)
                                        .sum::<u32>();
                                    let right_used = active_floats
                                        .iter()
                                        .filter(|(side, bounds)| {
                                            side == "right"
                                                && bounds.y <= float_y
                                                && bounds.bottom() > float_y
                                        })
                                        .map(|(_, b)| b.w)
                                        .sum::<u32>();
                                    if item_w <= child_w.saturating_sub(left_used + right_used) {
                                        break;
                                    }
                                    let Some(next_y) = active_floats
                                        .iter()
                                        .filter(|(_, b)| b.bottom() > float_y)
                                        .map(|(_, b)| b.bottom())
                                        .min()
                                    else {
                                        break;
                                    };
                                    float_y = next_y;
                                }
                                let left_used = active_floats
                                    .iter()
                                    .filter(|(side, bounds)| {
                                        side == "left"
                                            && bounds.y <= float_y
                                            && bounds.bottom() > float_y
                                    })
                                    .map(|(_, b)| b.w)
                                    .sum::<u32>();
                                let right_used = active_floats
                                    .iter()
                                    .filter(|(side, bounds)| {
                                        side == "right"
                                            && bounds.y <= float_y
                                            && bounds.bottom() > float_y
                                    })
                                    .map(|(_, b)| b.w)
                                    .sum::<u32>();
                                let target_x = if child_style.float == "right" {
                                    child_x.saturating_add(
                                        child_w.saturating_sub(right_used).saturating_sub(item_w)
                                            as i32,
                                    )
                                } else {
                                    child_x.saturating_add(left_used as i32)
                                };
                                let current = self.tree.nodes[child_index].margin_box;
                                self.translate_layout_range(
                                    start,
                                    end,
                                    inline_start,
                                    inline_end,
                                    target_x - current.x,
                                    float_y - current.y,
                                );
                                if active_floats.len() < MAX_ACTIVE_FLOATS {
                                    active_floats.push((
                                        child_style.float.clone(),
                                        self.tree.nodes[child_index].margin_box,
                                    ));
                                }
                                self.tree.nodes[child_index].float_containing_block =
                                    Some(self.tree.nodes[index].owner_node_id);
                                self.tree.nodes[index].children.push(child_index);
                            }
                            continue;
                        }
                        let left_used = active_floats
                            .iter()
                            .filter(|(side, bounds)| {
                                side == "left" && bounds.y <= inner_y && bounds.bottom() > inner_y
                            })
                            .map(|(_, bounds)| bounds.w)
                            .sum::<u32>();
                        let right_used = active_floats
                            .iter()
                            .filter(|(side, bounds)| {
                                side == "right" && bounds.y <= inner_y && bounds.bottom() > inner_y
                            })
                            .map(|(_, bounds)| bounds.w)
                            .sum::<u32>();
                        let flow_x = child_x.saturating_add(left_used as i32);
                        let flow_w = child_w.saturating_sub(left_used + right_used).max(1);
                        if let Some(child_index) =
                            self.layout_block(child, flow_x, flow_w, &mut inner_y, depth + 1)
                        {
                            self.tree.nodes[index].children.push(child_index);
                        }
                    } else {
                        inline_nodes.push(child);
                    }
                }
                self.flush_inline(index, &inline_nodes, child_x, child_w, &mut inner_y);
            }
            inner_y.saturating_sub(content_y).max(0) as u32
        };
        let content_h = style
            .height
            .unwrap_or(0)
            .max(used_content_h)
            .max(style.min_height)
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
        if let Some(raw_url) = style.paint.background_image.as_deref() {
            if let Some(resolved_url) = self
                .base_url
                .as_ref()
                .and_then(|base| resolve_url(base, raw_url).ok())
            {
                let decoded = self.images.decoded(&resolved_url);
                let link = self.document.get(node_id).and_then(|node| match node {
                    Node::Element {
                        tag_name,
                        attributes,
                        ..
                    } if tag_name.eq_ignore_ascii_case("a") => {
                        let href = attr(attributes, "href").unwrap_or("");
                        Some(LinkTarget {
                            anchor_node_id: style.node_id,
                            href: href.into(),
                            resolved_url: self
                                .base_url
                                .as_ref()
                                .and_then(|base| resolve_url(base, href).ok()),
                        })
                    }
                    _ => None,
                });
                node.image_fragments.push(ImageFragment {
                    owner_node_id: style.node_id,
                    bounds: Rect::new(content_x, content_y, content_w, content_h),
                    src: raw_url.into(),
                    resolved_url: Some(resolved_url),
                    alt: String::from(""),
                    intrinsic_size: decoded
                        .map(|image| Size::new(image.image.width, image.image.height)),
                    decoded: decoded.map(|image| image.image.clone()),
                    link,
                });
            }
        }
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

    fn layout_block_image(
        &mut self,
        node_id: NodeId,
        available_x: i32,
        available_w: u32,
        cursor_y: &mut i32,
        style: FlowStyle,
    ) -> Option<usize> {
        let Node::Element { attributes, .. } = self.document.get(node_id)? else {
            return None;
        };
        let src = attr(attributes, "src").unwrap_or("");
        let resolved_url = self
            .base_url
            .as_ref()
            .and_then(|base| resolve_url(base, src).ok());
        let decoded = resolved_url
            .as_deref()
            .and_then(|url| self.images.decoded(url));
        let intrinsic_size = decoded.map(|image| Size::new(image.image.width, image.image.height));
        let (content_w, content_h) = image_dimensions(
            style.width.or(parse_dimension(attr(attributes, "width"))),
            style.height.or(parse_dimension(attr(attributes, "height"))),
            intrinsic_size,
        );
        let border_x = available_x.saturating_add(style.margin[3] as i32);
        let border_y = cursor_y.saturating_add(style.margin[0] as i32);
        let content_x = border_x
            .saturating_add(style.paint.border_width as i32)
            .saturating_add(style.padding[3] as i32);
        let content_y = border_y
            .saturating_add(style.paint.border_width as i32)
            .saturating_add(style.padding[0] as i32);
        let padding_w = content_w
            .saturating_add(style.padding[1])
            .saturating_add(style.padding[3]);
        let padding_h = content_h
            .saturating_add(style.padding[0])
            .saturating_add(style.padding[2]);
        let border_w = padding_w.saturating_add(style.paint.border_width.saturating_mul(2));
        let border_h = padding_h.saturating_add(style.paint.border_width.saturating_mul(2));
        let margin_h = border_h
            .saturating_add(style.margin[0])
            .saturating_add(style.margin[2]);
        let index = self.tree.nodes.len();
        self.tree.nodes.push(LayoutNode {
            owner_node_id: style.node_id,
            display: style.display,
            content_box: Rect::new(content_x, content_y, content_w, content_h),
            padding_box: Rect::new(
                border_x.saturating_add(style.paint.border_width as i32),
                border_y.saturating_add(style.paint.border_width as i32),
                padding_w,
                padding_h,
            ),
            border_box: Rect::new(border_x, border_y, border_w, border_h),
            margin_box: Rect::new(available_x, *cursor_y, border_w.min(available_w), margin_h),
            children: Vec::new(),
            text_fragments: Vec::new(),
            image_fragments: vec![ImageFragment {
                owner_node_id: style.node_id,
                bounds: Rect::new(content_x, content_y, content_w, content_h),
                src: src.into(),
                resolved_url,
                alt: attr(attributes, "alt").unwrap_or("Image").into(),
                intrinsic_size,
                decoded: decoded.map(|image| image.image.clone()),
                link: None,
            }],
            marker: None,
            visible: true,
            paint: style.paint,
            paint_order: node_id.min(u32::MAX as usize) as u32,
            float_side: style.float.clone(),
            clear: style.clear.clone(),
            float_containing_block: None,
            table_dimensions: None,
        });
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
        // Whitespace belongs to the DOM text stream, not to inline wrappers.
        // In particular, entering or leaving an anchor must neither create nor
        // remove a collapsed space.  Keep the separator pending until the next
        // visible word so a line break can discard it as CSS normal whitespace
        // requires.
        let mut pending_space = false;
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
                    pending_space = false;
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
                InlineItem::Control {
                    owner,
                    control,
                    width,
                    height,
                } => {
                    let width = width.min(w.max(1));
                    if had_content
                        && line_x.saturating_add(width as i32) > x.saturating_add(w as i32)
                    {
                        self.finish_line(parent_index, line_start, line_x, x, w, &align);
                        *cursor_y = cursor_y.saturating_add(line_h as i32);
                        self.line_count = self.line_count.saturating_add(1);
                        line_start = self.tree.nodes[parent_index].text_fragments.len();
                        line_x = x;
                        line_h = self.measurer.line_height().max(1);
                        had_content = false;
                    }
                    let bounds = Rect::new(line_x, *cursor_y, width, height.max(1));
                    self.record_inline_box_with_control(owner, bounds, control);
                    line_x = line_x.saturating_add(width as i32);
                    line_h = line_h.max(height.max(1));
                    had_content = true;
                }
                InlineItem::Text {
                    owner,
                    inline_owners,
                    text,
                    paint,
                    link,
                } => {
                    if matches!(paint.white_space.as_str(), "pre" | "pre-wrap") {
                        let parts: Vec<&str> = text
                            .split('\n')
                            .take(MAX_LINES.saturating_sub(self.line_count))
                            .collect();
                        for (part_index, part) in parts.iter().enumerate() {
                            let part_w = scaled_measure_for(
                                self.measurer,
                                part,
                                paint.font_size,
                                paint.font_family,
                            );
                            // `pre-wrap` preserves characters but moves a long physical line to
                            // a new visual line instead of overlapping its parent box.
                            if paint.white_space == "pre-wrap"
                                && had_content
                                && line_x.saturating_add(part_w as i32) > x.saturating_add(w as i32)
                            {
                                self.finish_line(parent_index, line_start, line_x, x, w, &align);
                                *cursor_y = cursor_y.saturating_add(line_h as i32);
                                self.line_count = self.line_count.saturating_add(1);
                                line_start = self.tree.nodes[parent_index].text_fragments.len();
                                line_x = x;
                            }
                            if !part.is_empty() {
                                let bounds = Rect::new(
                                    line_x,
                                    cursor_y.saturating_add(paint.baseline_offset),
                                    part_w,
                                    paint.line_height.max(1),
                                );
                                self.tree.nodes[parent_index]
                                    .text_fragments
                                    .push(TextFragment {
                                        owner_node_id: owner,
                                        bounds,
                                        text: String::from(*part),
                                        paint: paint.clone(),
                                        link: link.clone(),
                                    });
                                for inline_owner in &inline_owners {
                                    self.record_inline_box(inline_owner.clone(), bounds);
                                }
                                self.text_fragment_count =
                                    self.text_fragment_count.saturating_add(1);
                                line_x = line_x.saturating_add(part_w as i32);
                                line_h = line_h.max(paint.line_height);
                                had_content = true;
                            }
                            if part_index + 1 < parts.len() {
                                self.finish_line(parent_index, line_start, line_x, x, w, &align);
                                *cursor_y = cursor_y.saturating_add(line_h as i32);
                                self.line_count = self.line_count.saturating_add(1);
                                line_start = self.tree.nodes[parent_index].text_fragments.len();
                                line_x = x;
                                line_h = self.measurer.line_height().max(1);
                                had_content = false;
                            }
                        }
                        continue;
                    }
                    if had_content && text.as_bytes().first().is_some_and(u8::is_ascii_whitespace) {
                        pending_space = true;
                    }
                    let words: Vec<&str> = text.split_ascii_whitespace().collect();
                    for (word_index, word) in words.iter().enumerate() {
                        let word_w = scaled_measure_for(
                            self.measurer,
                            word,
                            paint.font_size,
                            paint.font_family,
                        );
                        let space_w = if had_content && pending_space {
                            scaled_measure_for(
                                self.measurer,
                                " ",
                                paint.font_size,
                                paint.font_family,
                            )
                        } else {
                            0
                        };
                        if paint.white_space != "nowrap"
                            && had_content
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
                            pending_space = false;
                        }
                        if had_content && pending_space {
                            line_x = line_x.saturating_add(space_w as i32);
                        }
                        let fragment_h = paint.line_height.max(1);
                        let bounds = Rect::new(
                            line_x,
                            cursor_y.saturating_add(paint.baseline_offset),
                            word_w,
                            fragment_h,
                        );
                        self.tree.nodes[parent_index]
                            .text_fragments
                            .push(TextFragment {
                                owner_node_id: owner,
                                bounds,
                                text: String::from(*word),
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
                        pending_space = word_index + 1 < words.len()
                            || text.as_bytes().last().is_some_and(u8::is_ascii_whitespace);
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
                    // Keep normal-mode whitespace-only nodes: `flush_inline`
                    // collapses them into a pending separator between words.
                    // Dropping them here makes adjacent inline elements look as
                    // if they were separated (or joined) based on their tag.
                    if !content.is_empty() {
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
                let mut style = self.flow_style(node_id);
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
                if tag_name.eq_ignore_ascii_case("sub") {
                    style.paint.baseline_offset = (style.paint.line_height / 4) as i32;
                } else if tag_name.eq_ignore_ascii_case("sup") {
                    style.paint.baseline_offset = -((style.paint.line_height / 4) as i32);
                }
                if tag_name.eq_ignore_ascii_case("input") || tag_name.eq_ignore_ascii_case("button")
                {
                    let input_type = attr(attributes, "type")
                        .unwrap_or("text")
                        .to_ascii_lowercase();
                    let is_button = tag_name.eq_ignore_ascii_case("button")
                        || matches!(input_type.as_str(), "submit" | "button");
                    let label = if is_button {
                        if tag_name.eq_ignore_ascii_case("button") {
                            let text = collect_text_content(self.document, node_id);
                            if text.is_empty() {
                                String::from(attr(attributes, "value").unwrap_or(""))
                            } else {
                                text
                            }
                        } else {
                            String::from(attr(attributes, "value").unwrap_or(""))
                        }
                    } else {
                        String::new()
                    };
                    let default_width = if is_button {
                        self.measurer.measure_width(&label).saturating_add(28)
                    } else {
                        attr(attributes, "size")
                            .and_then(|v| v.parse::<u32>().ok())
                            .unwrap_or(20)
                            .saturating_mul(8)
                            .saturating_add(18)
                    };
                    let width = style.width.unwrap_or(default_width).max(24);
                    let height = style.height.unwrap_or(
                        style
                            .paint
                            .line_height
                            .saturating_add(style.padding[0] + style.padding[2] + 2)
                            .max(28),
                    );
                    output.push(InlineItem::Control {
                        owner,
                        control: ControlLayout {
                            label,
                            placeholder: attr(attributes, "placeholder").unwrap_or("").into(),
                            value: if is_button {
                                String::new()
                            } else {
                                attr(attributes, "value").unwrap_or("").into()
                            },
                            disabled: attributes
                                .iter()
                                .any(|a| a.name().eq_ignore_ascii_case("disabled")),
                            editable: !is_button
                                && !attributes
                                    .iter()
                                    .any(|a| a.name().eq_ignore_ascii_case("readonly")),
                            kind: if is_button { 1 } else { 0 },
                        },
                        width,
                        height,
                    });
                    return;
                }
                if tag_name.eq_ignore_ascii_case("img") {
                    let src = attr(attributes, "src").unwrap_or("");
                    let resolved_url = self
                        .base_url
                        .as_ref()
                        .and_then(|base| resolve_url(base, src).ok());
                    let decoded = resolved_url
                        .as_deref()
                        .and_then(|url| self.images.decoded(url));
                    let intrinsic_size =
                        decoded.map(|image| Size::new(image.image.width, image.image.height));
                    let html_width = parse_dimension(attr(attributes, "width"));
                    let html_height = parse_dimension(attr(attributes, "height"));
                    let specified_width = style.width.or(html_width);
                    let specified_height = style.height.or(html_height);
                    let (width, height) =
                        image_dimensions(specified_width, specified_height, intrinsic_size);
                    let mut image_owners = inline_owners;
                    image_owners.push(owner);
                    output.push(InlineItem::Image {
                        image: ImageFragment {
                            owner_node_id: style.node_id,
                            bounds: Rect::new(0, 0, width, height),
                            src: src.into(),
                            resolved_url,
                            alt: attr(attributes, "alt").unwrap_or("Image").into(),
                            intrinsic_size,
                            decoded: decoded.map(|image| image.image.clone()),
                            link,
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
        if !matches!(self.document.get(node_id), Some(Node::Element { .. })) {
            return false;
        }
        let style = self.flow_style(node_id);
        matches!(style.float.as_str(), "left" | "right")
            || matches!(
                style.display,
                DisplayType::Block | DisplayType::ListItem | DisplayType::Flex | DisplayType::Table
            )
    }

    /// Deliberately small flex formatter for ordinary website navigation and
    /// toolbars. Items are laid out once using their preferred content width,
    /// then translated as a retained subtree so DOM ownership stays intact.
    fn layout_flex_children(
        &mut self,
        parent: usize,
        children: &[NodeId],
        x: i32,
        y: i32,
        width: u32,
        style: &FlowStyle,
        depth: usize,
    ) -> u32 {
        const MAX_FLEX_ITEMS: usize = 256;
        const MAX_FLEX_LINES: usize = 256;
        let row = !matches!(style.flex_direction.as_str(), "column" | "column-reverse");
        let reverse = matches!(
            style.flex_direction.as_str(),
            "row-reverse" | "column-reverse"
        );
        let mut items: Vec<(usize, u32, u32, usize, usize, usize, usize)> = Vec::new();
        let mut anonymous_children = Vec::new();
        for &child in children.iter().take(MAX_FLEX_ITEMS) {
            if !self.is_block(child) {
                anonymous_children.push(child);
                continue;
            }
            if !matches!(self.document.get(child), Some(Node::Element { .. }))
                || self.flow_style(child).display == DisplayType::None
            {
                continue;
            }
            let preferred = self.preferred_flex_width(child, width).max(1);
            let start = self.tree.nodes.len();
            let inline_start = self.tree.inline_boxes.len();
            let mut child_cursor = 0;
            let Some(index) = self.layout_block(child, 0, preferred, &mut child_cursor, depth + 1)
            else {
                // Direct text/inline children remain handled by the normal
                // inline fallback; they are not allowed to corrupt flex flow.
                continue;
            };
            let end = self.tree.nodes.len();
            let inline_end = self.tree.inline_boxes.len();
            let item_w = self.tree.nodes[index].margin_box.w.max(1);
            let item_h = self.tree.nodes[index].margin_box.h.max(1);
            self.tree.nodes[parent].children.push(index);
            items.push((index, item_w, item_h, start, end, inline_start, inline_end));
        }
        if items.is_empty() {
            let mut fallback_y = y;
            self.flush_inline(parent, &anonymous_children, x, width, &mut fallback_y);
            return fallback_y.saturating_sub(y).max(0) as u32;
        }
        if reverse {
            items.reverse();
        }

        let gap_main = if row { style.column_gap } else { style.row_gap };
        let mut lines: Vec<Vec<usize>> = Vec::new();
        let mut line_widths: Vec<u32> = Vec::new();
        let mut line_cross: Vec<u32> = Vec::new();
        for item_index in 0..items.len() {
            let main = if row {
                items[item_index].1
            } else {
                items[item_index].2
            };
            let current = line_widths.last().copied().unwrap_or(0);
            let count = lines.last().map_or(0, Vec::len) as u32;
            let proposed =
                current
                    .saturating_add(main)
                    .saturating_add(if count > 0 { gap_main } else { 0 });
            let wrap = style.flex_wrap == "wrap"
                && !lines.is_empty()
                && proposed > if row { width } else { MAX_DOCUMENT_DIMENSION };
            if wrap && lines.len() < MAX_FLEX_LINES {
                lines.push(Vec::new());
                line_widths.push(0);
                line_cross.push(0);
            }
            if lines.is_empty() {
                lines.push(Vec::new());
                line_widths.push(0);
                line_cross.push(0);
            }
            let line = lines.len() - 1;
            if !lines[line].is_empty() {
                line_widths[line] = line_widths[line].saturating_add(gap_main);
            }
            lines[line].push(item_index);
            line_widths[line] = line_widths[line].saturating_add(main);
            line_cross[line] = line_cross[line].max(if row {
                items[item_index].2
            } else {
                items[item_index].1
            });
        }

        let mut cross_cursor = 0u32;
        for (line_index, line) in lines.iter().enumerate() {
            let line_main = line_widths[line_index];
            let main_limit = if row {
                width
            } else {
                line_cross.iter().copied().max().unwrap_or(0)
            };
            let free = main_limit.saturating_sub(line_main);
            let (leading, distributed) =
                justify_space(&style.justify_content, free, line.len(), gap_main);
            let mut main_cursor = leading;
            for item_ref in line {
                let (node_index, item_w, item_h, start, end, inline_start, inline_end) =
                    items[*item_ref];
                let item_main = if row { item_w } else { item_h };
                let line_size = line_cross[line_index];
                let item_cross = if row { item_h } else { item_w };
                let align = match style.align_items.as_str() {
                    "center" => line_size.saturating_sub(item_cross) / 2,
                    "flex-end" => line_size.saturating_sub(item_cross),
                    _ => 0,
                };
                let target_x = if row {
                    x.saturating_add(main_cursor as i32)
                } else {
                    x.saturating_add(align as i32)
                };
                let target_y = if row {
                    y.saturating_add(cross_cursor as i32)
                        .saturating_add(align as i32)
                } else {
                    y.saturating_add(main_cursor as i32)
                };
                let current_box = self.tree.nodes[node_index].border_box;
                self.translate_layout_range(
                    start,
                    end,
                    inline_start,
                    inline_end,
                    target_x.saturating_sub(current_box.x),
                    target_y.saturating_sub(current_box.y),
                );
                main_cursor = main_cursor
                    .saturating_add(item_main)
                    .saturating_add(distributed);
            }
            cross_cursor = cross_cursor.saturating_add(line_cross[line_index]);
            if line_index + 1 < lines.len() {
                cross_cursor = cross_cursor.saturating_add(style.row_gap);
            }
        }
        if !anonymous_children.is_empty() {
            let mut fallback_y = y.saturating_add(cross_cursor as i32);
            self.flush_inline(parent, &anonymous_children, x, width, &mut fallback_y);
            cross_cursor = cross_cursor.max(fallback_y.saturating_sub(y).max(0) as u32);
        }
        if row {
            cross_cursor
        } else {
            cross_cursor.max(line_widths.into_iter().max().unwrap_or(0))
        }
    }

    fn preferred_flex_width(&self, node_id: NodeId, available: u32) -> u32 {
        let style = self.flow_style(node_id);
        if let Some(width) = style.width {
            return width.min(available.max(1));
        }
        let measured = self.inline_preferred_width(node_id).max(1);
        measured
            .saturating_add(style.padding[1])
            .saturating_add(style.padding[3])
            .saturating_add(style.paint.border_width.saturating_mul(2))
            .min(available.max(1))
    }

    fn inline_preferred_width(&self, node_id: NodeId) -> u32 {
        match self.document.get(node_id) {
            Some(Node::Text { content }) => content
                .split_ascii_whitespace()
                .map(|word| self.measurer.measure_width(word))
                .max()
                .unwrap_or(0),
            Some(Node::Element { children, .. }) => children
                .iter()
                .map(|child| self.inline_preferred_width(*child))
                .sum::<u32>()
                .min(MAX_DOCUMENT_DIMENSION),
            _ => 0,
        }
    }

    fn translate_layout_range(
        &mut self,
        start: usize,
        end: usize,
        inline_start: usize,
        inline_end: usize,
        dx: i32,
        dy: i32,
    ) {
        for node in self.tree.nodes.get_mut(start..end).into_iter().flatten() {
            node.content_box = node.content_box.translate(dx, dy);
            node.padding_box = node.padding_box.translate(dx, dy);
            node.border_box = node.border_box.translate(dx, dy);
            node.margin_box = node.margin_box.translate(dx, dy);
            for fragment in &mut node.text_fragments {
                fragment.bounds = fragment.bounds.translate(dx, dy);
            }
            for image in &mut node.image_fragments {
                image.bounds = image.bounds.translate(dx, dy);
            }
            if let Some(marker) = &mut node.marker {
                marker.bounds = marker.bounds.translate(dx, dy);
            }
        }
        for inline_box in self
            .tree
            .inline_boxes
            .get_mut(inline_start..inline_end)
            .into_iter()
            .flatten()
        {
            inline_box.bounds = inline_box.bounds.translate(dx, dy);
        }
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
            control: None,
        });
    }

    fn record_inline_box_with_control(
        &mut self,
        owner: InlineOwner,
        bounds: Rect,
        control: ControlLayout,
    ) {
        self.tree.inline_boxes.push(InlineLayoutBox {
            owner_node_id: owner.node_id,
            bounds,
            paint: owner.paint,
            paint_order: owner.paint_order,
            control: Some(control),
        });
    }

    fn flow_style(&self, node_id: NodeId) -> FlowStyle {
        let style = self.styles.nearest_element_style(self.document, node_id);
        let paint = resolved_paint(style, self.measurer);
        FlowStyle {
            node_id: DocumentNodeId(node_id as u64),
            display: display(style),
            width: value_px(style, Property::Width),
            width_percent: value_percent(style, Property::Width),
            min_width: value_px(style, Property::MinWidth).unwrap_or(0),
            max_width: value_px(style, Property::MaxWidth),
            height: value_px(style, Property::Height),
            min_height: value_px(style, Property::MinHeight).unwrap_or(0),
            margin: sides(
                style,
                Property::MarginTop,
                Property::MarginRight,
                Property::MarginBottom,
                Property::MarginLeft,
            ),
            margin_auto: [
                value_auto(style, Property::MarginTop),
                value_auto(style, Property::MarginRight),
                value_auto(style, Property::MarginBottom),
                value_auto(style, Property::MarginLeft),
            ],
            padding: sides(
                style,
                Property::PaddingTop,
                Property::PaddingRight,
                Property::PaddingBottom,
                Property::PaddingLeft,
            ),
            flex_direction: keyword(style, Property::FlexDirection),
            flex_wrap: keyword(style, Property::FlexWrap),
            justify_content: keyword(style, Property::JustifyContent),
            align_items: keyword(style, Property::AlignItems),
            align_content: keyword(style, Property::AlignContent),
            row_gap: value_px(style, Property::RowGap)
                .unwrap_or(0)
                .min(MAX_DOCUMENT_DIMENSION),
            column_gap: value_px(style, Property::ColumnGap)
                .unwrap_or(0)
                .min(MAX_DOCUMENT_DIMENSION),
            float: keyword(style, Property::Float),
            clear: keyword(style, Property::Clear),
            paint,
        }
    }

    fn list_marker(
        &self,
        node_id: NodeId,
        x: i32,
        y: i32,
        width: u32,
        style: &FlowStyle,
    ) -> Option<ListMarker> {
        if self
            .tree
            .nodes
            .iter()
            .filter(|node| node.marker.is_some())
            .count()
            >= MAX_LIST_MARKERS
        {
            return None;
        }
        let parent = self.document.parent(node_id)?;
        let parent_tag = self.document.get(parent)?.tag_name()?;
        let ordered = parent_tag.eq_ignore_ascii_case("ol");
        if !ordered && !parent_tag.eq_ignore_ascii_case("ul") {
            return None;
        }
        let mut ordinal = if ordered {
            self.document
                .get(parent)
                .and_then(|node| match node {
                    Node::Element { attributes, .. } => attr(attributes, "start"),
                    _ => None,
                })
                .and_then(|value| value.parse::<u32>().ok())
                .unwrap_or(1)
        } else {
            0
        };
        for &sibling in self.document.children(parent) {
            if sibling == node_id {
                break;
            }
            if self
                .document
                .get(sibling)
                .and_then(Node::tag_name)
                .is_some_and(|tag| tag.eq_ignore_ascii_case("li"))
                && self.flow_style(sibling).display != DisplayType::None
            {
                ordinal = ordinal.saturating_add(1);
            }
        }
        if ordered {
            if let Some(Node::Element { attributes, .. }) = self.document.get(node_id) {
                ordinal = attr(attributes, "value")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(ordinal);
            }
        }
        let style_type = keyword(self.styles.style_for(node_id), Property::ListStyleType);
        if style_type == "none" {
            return None;
        }
        let (label, shape) = if ordered {
            (Some(format_marker(ordinal, &style_type)), MarkerShape::Text)
        } else {
            match style_type.as_str() {
                "circle" => (None, MarkerShape::Circle),
                "square" => (None, MarkerShape::Square),
                _ => (None, MarkerShape::Disc),
            }
        };
        Some(ListMarker {
            owner_node_id: style.node_id,
            bounds: Rect::new(x, y, width, style.paint.line_height.max(1)),
            label,
            shape,
            ordinal,
            style_type,
            paint_order: node_id.min(u32::MAX as usize) as u32,
        })
    }
}

fn display(style: Option<&ComputedStyle>) -> DisplayType {
    match style.and_then(|style| style.value(&Property::Display)) {
        Some(PropertyValue::Keyword(value)) if value.eq_ignore_ascii_case("block") => {
            DisplayType::Block
        }
        Some(PropertyValue::Keyword(value)) if value.eq_ignore_ascii_case("list-item") => {
            DisplayType::ListItem
        }
        Some(PropertyValue::Keyword(value)) if value.eq_ignore_ascii_case("flex") => {
            DisplayType::Flex
        }
        Some(PropertyValue::Keyword(value)) if value.eq_ignore_ascii_case("inline-flex") => {
            DisplayType::InlineFlex
        }
        Some(PropertyValue::Keyword(value)) if value.eq_ignore_ascii_case("inline-block") => {
            DisplayType::InlineBlock
        }
        Some(PropertyValue::Keyword(value)) if value.eq_ignore_ascii_case("table") => {
            DisplayType::Table
        }
        Some(PropertyValue::Keyword(value)) if value.eq_ignore_ascii_case("table-header-group") => {
            DisplayType::TableHeaderGroup
        }
        Some(PropertyValue::Keyword(value)) if value.eq_ignore_ascii_case("table-row-group") => {
            DisplayType::TableRowGroup
        }
        Some(PropertyValue::Keyword(value)) if value.eq_ignore_ascii_case("table-footer-group") => {
            DisplayType::TableFooterGroup
        }
        Some(PropertyValue::Keyword(value)) if value.eq_ignore_ascii_case("table-row") => {
            DisplayType::TableRow
        }
        Some(PropertyValue::Keyword(value)) if value.eq_ignore_ascii_case("table-cell") => {
            DisplayType::TableCell
        }
        Some(PropertyValue::Keyword(value)) if value.eq_ignore_ascii_case("none") => {
            DisplayType::None
        }
        // Unknown display values safely use inline behavior in this vertical slice.
        _ => DisplayType::Inline,
    }
}

fn justify_space(keyword: &str, free: u32, count: usize, gap: u32) -> (u32, u32) {
    if count == 0 {
        return (0, 0);
    }
    let free = free.min(MAX_DOCUMENT_DIMENSION);
    match keyword {
        "center" => (free / 2, gap),
        "flex-end" => (free, gap),
        "space-between" if count > 1 => (0, free / (count as u32 - 1)),
        "space-around" => {
            let unit = free / count as u32;
            (unit / 2, gap.saturating_add(unit))
        }
        "space-evenly" => {
            let unit = free / (count as u32 + 1);
            (unit, gap.saturating_add(unit))
        }
        _ => (0, gap),
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

fn value_percent(style: Option<&ComputedStyle>, property: Property) -> Option<u32> {
    match style.and_then(|style| style.value(&property)) {
        Some(PropertyValue::Percentage(value)) if *value >= 0 => Some(*value as u32),
        _ => None,
    }
}

fn value_auto(style: Option<&ComputedStyle>, property: Property) -> bool {
    matches!(
        style.and_then(|style| style.value(&property)),
        Some(PropertyValue::Auto)
    )
}

fn attr<'a>(attributes: &'a [Attribute], name: &str) -> Option<&'a str> {
    attributes
        .iter()
        .find(|attribute| attribute.name().eq_ignore_ascii_case(name))
        .map(Attribute::value)
}

fn collect_text_content(document: &Document, node_id: NodeId) -> String {
    let mut result = String::new();
    let mut stack = vec![node_id];
    while let Some(id) = stack.pop() {
        match document.get(id) {
            Some(Node::Text { content }) => result.push_str(content),
            Some(Node::Element { children, .. }) => {
                for child in children.iter().rev() {
                    stack.push(*child);
                }
            }
            _ => {}
        }
    }
    result.trim().into()
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
    let requested_family = keyword(style, Property::FontFamily);
    let font_family = DocumentFontFamily::resolve_css_list(&requested_family);
    let default_line_height =
        (font_size.saturating_mul(6) / 5).max(measurer.line_height_for(font_family));
    let line_height = value_px(style, Property::LineHeight)
        .unwrap_or(default_line_height)
        .max(1);
    let fallback_width = value_px(style, Property::BorderWidth).unwrap_or(0);
    let border_widths = [
        Property::BorderTopWidth,
        Property::BorderRightWidth,
        Property::BorderBottomWidth,
        Property::BorderLeftWidth,
    ]
    .map(|property| value_px(style, property).unwrap_or(fallback_width));
    let border_width = border_widths.iter().copied().max().unwrap_or(0);
    let border_style = keyword(style, Property::BorderStyle);
    let border_styles = [
        Property::BorderTopStyle,
        Property::BorderRightStyle,
        Property::BorderBottomStyle,
        Property::BorderLeftStyle,
    ]
    .map(|property| {
        let side = keyword(style, property);
        if side.is_empty() {
            border_style.clone()
        } else {
            side
        }
    });
    let foreground = css_color(
        style.and_then(|style| style.value(&Property::Color)),
        Color::rgb(0, 0, 0),
    );
    let fallback_border = css_color(
        style.and_then(|style| style.value(&Property::BorderColor)),
        foreground,
    );
    let border_colors = [
        Property::BorderTopColor,
        Property::BorderRightColor,
        Property::BorderBottomColor,
        Property::BorderLeftColor,
    ]
    .map(|property| {
        css_color(
            style.and_then(|style| style.value(&property)),
            fallback_border,
        )
    });
    let corner_radii = CornerRadii {
        top_left: value_px(style, Property::BorderTopLeftRadius).unwrap_or(0),
        top_right: value_px(style, Property::BorderTopRightRadius).unwrap_or(0),
        bottom_right: value_px(style, Property::BorderBottomRightRadius).unwrap_or(0),
        bottom_left: value_px(style, Property::BorderBottomLeftRadius).unwrap_or(0),
    };
    let box_shadow = style
        .and_then(|style| style.value(&Property::BoxShadow))
        .and_then(|value| match value {
            PropertyValue::Keyword(raw) | PropertyValue::Raw(raw) => {
                parse_box_shadow(raw, font_size, foreground)
            }
            _ => None,
        });
    ResolvedPaintStyle {
        color: foreground,
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
        line_through: keyword(style, Property::TextDecoration).contains("line-through"),
        monospace: font_family == DocumentFontFamily::Monospace,
        font_family,
        white_space: {
            let value = keyword(style, Property::WhiteSpace);
            if matches!(value.as_str(), "pre" | "pre-wrap" | "nowrap") {
                value
            } else {
                String::from("normal")
            }
        },
        baseline_offset: 0,
        text_align: keyword(style, Property::TextAlign),
        border_color: fallback_border,
        border_width,
        border_visible: border_widths
            .iter()
            .zip(border_styles.iter())
            .any(|(width, style)| *width > 0 && style != "none"),
        border_colors,
        border_widths,
        corner_radii,
        box_shadow,
        background_image: style
            .and_then(|style| style.value(&Property::BackgroundImage))
            .and_then(|value| {
                let raw = match value {
                    PropertyValue::Keyword(value) | PropertyValue::Raw(value) => value,
                    _ => return None,
                };
                let inner = raw.strip_prefix("url(")?.strip_suffix(')')?.trim();
                let inner = inner.trim_matches(['\"', '\'']);
                (!inner.is_empty() && !inner.eq_ignore_ascii_case("none"))
                    .then(|| String::from(inner))
            }),
    }
}

fn scaled_measure_for(
    measurer: &dyn TextMeasurer,
    text: &str,
    font_size: u32,
    family: DocumentFontFamily,
) -> u32 {
    measurer
        .measure_width_for_size(family, font_size, text)
        .clamp(1, MAX_DOCUMENT_DIMENSION)
}

fn parse_box_shadow(raw: &str, font_size: u32, current_color: Color) -> Option<ResolvedBoxShadow> {
    if raw.contains(',') || raw.eq_ignore_ascii_case("none") {
        return None;
    }
    let mut lengths = Vec::new();
    let mut color = current_color;
    let mut inset = false;
    for token in raw.split_ascii_whitespace() {
        if token.eq_ignore_ascii_case("inset") {
            inset = true;
            continue;
        }
        if token.eq_ignore_ascii_case("currentcolor") {
            color = current_color;
            continue;
        }
        if let Some(parsed) = parse_shadow_color(token) {
            color = parsed;
            continue;
        }
        lengths.push(parse_shadow_length(token, font_size)?);
    }
    if !(2..=4).contains(&lengths.len()) {
        return None;
    }
    let blur = lengths.get(2).copied().unwrap_or(0).max(0).min(32) as u32;
    Some(ResolvedBoxShadow {
        offset_x: lengths[0],
        offset_y: lengths[1],
        blur,
        spread: lengths.get(3).copied().unwrap_or(0).clamp(-32, 32),
        color,
        inset,
    })
}

fn parse_shadow_length(token: &str, font_size: u32) -> Option<i32> {
    if matches!(token, "0" | "+0" | "-0") {
        return Some(0);
    }
    let lower = token.to_ascii_lowercase();
    let (number, scale) = if let Some(number) = lower.strip_suffix("px") {
        (number, 1000i32)
    } else if let Some(number) = lower.strip_suffix("em") {
        (number, font_size.min(4096) as i32 * 1000)
    } else {
        return None;
    };
    let negative = number.starts_with('-');
    let unsigned = number.trim_start_matches(['+', '-']);
    let (whole, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    let whole = if whole.is_empty() {
        0
    } else {
        whole.parse::<i32>().ok()?
    };
    let fraction = fraction
        .as_bytes()
        .iter()
        .take(3)
        .fold((0i32, 1i32), |(value, divisor), digit| {
            ((value * 10) + (*digit - b'0') as i32, divisor * 10)
        });
    let thousandths = whole * 1000
        + if fraction.1 > 1 {
            fraction.0 * 1000 / fraction.1
        } else {
            0
        };
    let value = thousandths.saturating_mul(scale) / 1_000_000;
    Some(if negative { -value } else { value })
}

fn parse_shadow_color(token: &str) -> Option<Color> {
    let lower = token.to_ascii_lowercase();
    let (r, g, b) = match lower.as_str() {
        "black" => (0, 0, 0),
        "white" => (255, 255, 255),
        "red" => (255, 0, 0),
        "yellow" => (255, 255, 0),
        "gray" | "grey" => (128, 128, 128),
        _ => {
            let hex = lower.strip_prefix('#')?;
            if hex.len() == 3 {
                let d = |byte| char::from(byte).to_digit(16).map(|v| (v * 17) as u8);
                (
                    d(hex.as_bytes()[0])?,
                    d(hex.as_bytes()[1])?,
                    d(hex.as_bytes()[2])?,
                )
            } else if hex.len() == 6 {
                (
                    u8::from_str_radix(&hex[0..2], 16).ok()?,
                    u8::from_str_radix(&hex[2..4], 16).ok()?,
                    u8::from_str_radix(&hex[4..6], 16).ok()?,
                )
            } else {
                return None;
            }
        }
    };
    Some(Color::rgb(r, g, b))
}

fn format_marker(ordinal: u32, style_type: &str) -> String {
    let alpha = |mut value: u32, upper: bool| {
        let mut out = String::new();
        value = value.max(1);
        while value > 0 {
            value -= 1;
            let base = if upper { b'A' } else { b'a' };
            out.insert(0, (base + (value % 26) as u8) as char);
            value /= 26;
        }
        out
    };
    let roman = |mut value: u32, upper: bool| {
        if value == 0 || value > 3999 {
            return format!("{value}.");
        }
        let mut out = String::new();
        for (unit, text) in [
            (1000, "M"),
            (900, "CM"),
            (500, "D"),
            (400, "CD"),
            (100, "C"),
            (90, "XC"),
            (50, "L"),
            (40, "XL"),
            (10, "X"),
            (9, "IX"),
            (5, "V"),
            (4, "IV"),
            (1, "I"),
        ] {
            while value >= unit {
                out.push_str(text);
                value -= unit;
            }
        }
        if upper {
            format!("{out}.")
        } else {
            format!("{}.", out.to_ascii_lowercase())
        }
    };
    match style_type {
        "lower-alpha" => format!("{}.", alpha(ordinal, false)),
        "upper-alpha" => format!("{}.", alpha(ordinal, true)),
        "lower-roman" => roman(ordinal, false),
        "upper-roman" => roman(ordinal, true),
        _ => format!("{ordinal}."),
    }
}

fn object_id(owner: DomNodeId, role: u8, slot: usize) -> RenderObjectId {
    RenderObjectId((owner.0 << 32) | ((role as u64) << 24) | (slot.min(0x00ff_ffff) as u64))
}

fn parse_dimension(value: Option<&str>) -> Option<u32> {
    value
        .and_then(|value| value.trim().parse::<u32>().ok())
        .filter(|value| *value > 0)
        .map(|value| value.min(MAX_DOCUMENT_DIMENSION))
}

fn image_dimensions(
    specified_width: Option<u32>,
    specified_height: Option<u32>,
    intrinsic: Option<Size>,
) -> (u32, u32) {
    match (specified_width, specified_height, intrinsic) {
        (Some(width), Some(height), _) => (width, height),
        (Some(width), None, Some(intrinsic)) => (
            width,
            ((width as u64 * intrinsic.h as u64) / intrinsic.w.max(1) as u64)
                .clamp(1, MAX_DOCUMENT_DIMENSION as u64) as u32,
        ),
        (None, Some(height), Some(intrinsic)) => (
            ((height as u64 * intrinsic.w as u64) / intrinsic.h.max(1) as u64)
                .clamp(1, MAX_DOCUMENT_DIMENSION as u64) as u32,
            height,
        ),
        (None, None, Some(intrinsic)) => (intrinsic.w, intrinsic.h),
        (Some(width), None, None) => (width, 100),
        (None, Some(height), None) => (160, height),
        (None, None, None) => (160, 100),
    }
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
        if let Some(shadow) = node.paint.box_shadow.filter(|shadow| !shadow.inset) {
            let extent = shadow
                .blur
                .saturating_add(shadow.spread.max(0) as u32)
                .min(64);
            let bounds = Rect::new(
                node.border_box
                    .x
                    .saturating_add(shadow.offset_x)
                    .saturating_sub(extent as i32),
                node.border_box
                    .y
                    .saturating_add(shadow.offset_y)
                    .saturating_sub(extent as i32),
                node.border_box.w.saturating_add(extent * 2),
                node.border_box.h.saturating_add(extent * 2),
            );
            push(
                &mut scene,
                RenderObject {
                    id: object_id(node.owner_node_id, 4, 0),
                    owner_node_id: node.owner_node_id,
                    kind: RenderObjectKind::BoxShadow {
                        box_bounds: node.border_box,
                        radii: node.paint.corner_radii,
                        offset_x: shadow.offset_x,
                        offset_y: shadow.offset_y,
                        blur: shadow.blur,
                        spread: shadow.spread,
                        color: shadow.color,
                        inset: false,
                    },
                    bounds,
                    clip_bounds: None,
                    paint_order: PaintOrder {
                        phase: 0,
                        document_order: node.paint_order,
                        ..PaintOrder::default()
                    },
                    interaction: None,
                },
            );
        }
        if node.paint.background != Color::TRANSPARENT {
            push(
                &mut scene,
                RenderObject {
                    id: object_id(node.owner_node_id, 2, 0),
                    owner_node_id: node.owner_node_id,
                    kind: if node.paint.corner_radii == CornerRadii::default() {
                        RenderObjectKind::Rectangle {
                            fill: node.paint.background,
                        }
                    } else {
                        RenderObjectKind::RoundedRectangle {
                            fill: node.paint.background,
                            radii: node.paint.corner_radii,
                        }
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
        if let Some(shadow) = node.paint.box_shadow.filter(|shadow| shadow.inset) {
            push(
                &mut scene,
                RenderObject {
                    id: object_id(node.owner_node_id, 5, 0),
                    owner_node_id: node.owner_node_id,
                    kind: RenderObjectKind::BoxShadow {
                        box_bounds: node.border_box,
                        radii: node.paint.corner_radii,
                        offset_x: shadow.offset_x,
                        offset_y: shadow.offset_y,
                        blur: shadow.blur,
                        spread: shadow.spread,
                        color: shadow.color,
                        inset: true,
                    },
                    bounds: node.border_box,
                    clip_bounds: Some(node.border_box),
                    paint_order: PaintOrder {
                        phase: 2,
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
                    kind: if node.paint.corner_radii == CornerRadii::default()
                        && node
                            .paint
                            .border_colors
                            .iter()
                            .all(|color| *color == node.paint.border_colors[0])
                        && node
                            .paint
                            .border_widths
                            .iter()
                            .all(|width| *width == node.paint.border_widths[0])
                    {
                        RenderObjectKind::Border {
                            color: node.paint.border_colors[0],
                            width: node.paint.border_widths[0],
                        }
                    } else {
                        RenderObjectKind::BorderSides {
                            colors: node.paint.border_colors,
                            widths: node.paint.border_widths,
                            radii: node.paint.corner_radii,
                        }
                    },
                    bounds: node.border_box,
                    clip_bounds: None,
                    paint_order: PaintOrder {
                        phase: 3,
                        document_order: node.paint_order,
                        ..PaintOrder::default()
                    },
                    interaction: None,
                },
            );
        }
        if let Some(marker) = &node.marker {
            let marker_id = object_id(marker.owner_node_id, 40, 0);
            let kind = match marker.shape {
                MarkerShape::Text => RenderObjectKind::Text {
                    text: marker.label.clone().unwrap_or_default(),
                    color: node.paint.color,
                    font_size: node.paint.font_size,
                    bold: false,
                    italic: false,
                    underline: false,
                    line_through: false,
                    monospace: false,
                    font_family: DocumentFontFamily::SansSerif,
                },
                MarkerShape::Disc | MarkerShape::Square => RenderObjectKind::Rectangle {
                    fill: node.paint.color,
                },
                MarkerShape::Circle => RenderObjectKind::Border {
                    color: node.paint.color,
                    width: 1,
                },
            };
            let bounds = match marker.shape {
                MarkerShape::Text => marker.bounds,
                _ => Rect::new(
                    marker.bounds.x + 8,
                    marker.bounds.y + (marker.bounds.h / 2) as i32 - 3,
                    6,
                    6,
                ),
            };
            push(
                &mut scene,
                RenderObject {
                    id: marker_id,
                    owner_node_id: marker.owner_node_id,
                    kind,
                    bounds,
                    clip_bounds: None,
                    paint_order: PaintOrder {
                        phase: 4,
                        document_order: marker.paint_order,
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
                        line_through: fragment.paint.line_through,
                        monospace: fragment.paint.monospace,
                        font_family: fragment.paint.font_family,
                    },
                    bounds: fragment.bounds,
                    clip_bounds: None,
                    paint_order: PaintOrder {
                        phase: 4,
                        document_order: node.paint_order.saturating_add(slot as u32),
                        ..PaintOrder::default()
                    },
                    interaction,
                },
            );
        }
        for (slot, image) in node.image_fragments.iter().enumerate() {
            let interaction = image.link.as_ref().map(|link| RenderInteraction::Link {
                owner_node_id: link.anchor_node_id,
                href: link.href.clone(),
                resolved_url: link.resolved_url.clone(),
            });
            push(
                &mut scene,
                RenderObject {
                    id: object_id(image.owner_node_id, 20, slot),
                    owner_node_id: image.owner_node_id,
                    kind: match &image.decoded {
                        Some(decoded) => RenderObjectKind::Image {
                            image: decoded.clone(),
                            source_url: image
                                .resolved_url
                                .clone()
                                .unwrap_or_else(|| image.src.clone()),
                            intrinsic_width: image.intrinsic_size.map_or(0, |size| size.w),
                            intrinsic_height: image.intrinsic_size.map_or(0, |size| size.h),
                            alt: image.alt.clone(),
                        },
                        None => RenderObjectKind::ImagePlaceholder {
                            src: image.src.clone(),
                            alt: image.alt.clone(),
                        },
                    },
                    bounds: image.bounds,
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
    }
    for inline_box in &layout.inline_boxes {
        if let Some(control) = &inline_box.control {
            push(
                &mut scene,
                RenderObject {
                    id: object_id(inline_box.owner_node_id, 50, 0),
                    owner_node_id: inline_box.owner_node_id,
                    kind: RenderObjectKind::Control {
                        label: control.label.clone(),
                        placeholder: control.placeholder.clone(),
                        value: control.value.clone(),
                        color: inline_box.paint.color,
                        background: if control.disabled {
                            Color::rgb(0xDD, 0xDD, 0xDD)
                        } else if inline_box.paint.background == Color::TRANSPARENT {
                            Color::rgb(0xFF, 0xFF, 0xFF)
                        } else {
                            inline_box.paint.background
                        },
                        border_color: inline_box.paint.border_color,
                        border_width: inline_box.paint.border_width.max(1),
                        focused: false,
                        disabled: control.disabled,
                        editable: control.editable,
                        kind: control.kind,
                        caret_offset: None,
                        font_size: inline_box.paint.font_size,
                        font_family: inline_box.paint.font_family,
                    },
                    bounds: inline_box.bounds,
                    clip_bounds: Some(inline_box.bounds),
                    paint_order: PaintOrder {
                        phase: 5,
                        document_order: inline_box.paint_order,
                        ..PaintOrder::default()
                    },
                    interaction: Some(RenderInteraction::Control {
                        owner_node_id: inline_box.owner_node_id,
                    }),
                },
            );
            continue;
        }
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
    // Group link fragments by anchor *and visual line*.  The text formatter is
    // the sole source of inline advances; these retained hit regions only
    // reuse the final fragment bounds.  Keeping wrapped lines separate avoids
    // a clickable rectangle across the whitespace between them.
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
        if let Some((_, bounds, ids)) = links.iter_mut().find(|(target, bounds, _)| {
            target.anchor_node_id == anchor
                && target.href == *href
                && bounds.y == object.bounds.y
                && bounds.h == object.bounds.h
        }) {
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

    fn text_bounds_for(state: &DocumentRenderState, owner: NodeId) -> Vec<Rect> {
        state
            .current_scene
            .objects
            .iter()
            .filter(|object| object.owner_node_id == DocumentNodeId(owner as u64))
            .filter_map(|object| match object.kind {
                RenderObjectKind::Text { .. } => Some(object.bounds),
                _ => None,
            })
            .collect()
    }

    fn extent(bounds: &[Rect]) -> u32 {
        let left = bounds.iter().map(|bounds| bounds.x).min().unwrap();
        let right = bounds.iter().map(|bounds| bounds.right()).max().unwrap();
        right.saturating_sub(left) as u32
    }

    #[test]
    fn anchor_text_uses_the_same_word_positions_and_advance_as_a_span() {
        let state = render(
            "<p><span>Linux mailing list service</span></p><p><a href='/lists'>Linux mailing list service</a></p>",
        );
        let span = state.dom.find_first_element("span").unwrap();
        let anchor = state.dom.find_first_element("a").unwrap();
        let span_bounds = text_bounds_for(&state, span);
        let anchor_bounds = text_bounds_for(&state, anchor);

        assert_eq!(span_bounds.len(), 4);
        assert_eq!(
            span_bounds
                .iter()
                .map(|bounds| bounds.x)
                .collect::<Vec<_>>(),
            anchor_bounds
                .iter()
                .map(|bounds| bounds.x)
                .collect::<Vec<_>>()
        );
        assert_eq!(extent(&span_bounds), extent(&anchor_bounds));
        assert!(state.current_scene.objects.iter().any(|object| {
            matches!(&object.kind, RenderObjectKind::Link { href, text_object_ids, .. }
                if href == "/lists" && text_object_ids.len() == anchor_bounds.len())
        }));
        assert!(
            state
                .current_scene
                .objects
                .iter()
                .filter(|object| {
                    object.owner_node_id == DocumentNodeId(anchor as u64)
                        && matches!(
                            &object.kind,
                            RenderObjectKind::Text {
                                underline: true,
                                ..
                            }
                        )
                })
                .count()
                == anchor_bounds.len()
        );
    }

    #[test]
    fn heading_advances_use_the_metrics_of_its_painted_face() {
        struct FaceMeasure;
        impl TextMeasurer for FaceMeasure {
            fn measure_width(&self, text: &str) -> u32 {
                text.chars().count() as u32 * 8
            }

            fn line_height(&self) -> u32 {
                16
            }

            fn measure_width_for_size(
                &self,
                _family: DocumentFontFamily,
                _font_size: u32,
                text: &str,
            ) -> u32 {
                // Represents the actual face chosen by the canvas, rather
                // than a synthetic scale of the body face.
                text.chars().count() as u32 * 11
            }
        }

        let dom = parse_html("<h3>Other articles</h3>").unwrap();
        let styles = StyleContext::build(&dom, &[]);
        let layout = build_layout_tree(&dom, &styles, Size::new(320, 240), &FaceMeasure);
        let fragments = &layout.nodes[0].text_fragments;
        assert_eq!(fragments.len(), 2);
        assert_eq!(fragments[1].bounds.x, fragments[0].bounds.right() + 11);
        assert_eq!(fragments[0].paint.font_size, 24);
    }

    #[test]
    fn wrapped_anchor_has_one_tight_hit_region_per_visual_line() {
        let state = render(
            "<style>p{display:block;width:104px}</style><p><a href='/lists'>Linux mailing list service</a></p>",
        );
        let anchor = state.dom.find_first_element("a").unwrap();
        let text = text_bounds_for(&state, anchor);
        assert_eq!(text.len(), 4);
        let links: Vec<_> = state
            .current_scene
            .objects
            .iter()
            .filter(|object| matches!(&object.kind, RenderObjectKind::Link { href, .. } if href == "/lists"))
            .collect();
        let lines: Vec<i32> = text.iter().map(|bounds| bounds.y).collect();
        assert_eq!(links.len(), 2);
        for link in links {
            assert!(lines.contains(&link.bounds.y));
            assert!(link.bounds.h > 0);
        }
    }

    #[test]
    fn inline_whitespace_is_from_text_nodes_not_anchor_boundaries() {
        let state = render(
            "<p>plain <a href='/one'>link</a> tail</p><p><a href='/one'>one</a><a href='/two'>two</a></p>",
        );
        let word = |text| {
            state
                .current_scene
                .objects
                .iter()
                .find(|object| matches!(&object.kind, RenderObjectKind::Text { text: value, .. } if value == text))
                .unwrap()
                .bounds
        };
        let plain = word("plain");
        let link = word("link");
        let tail = word("tail");
        assert_eq!(link.x, plain.right() + 8);
        assert_eq!(tail.x, link.right() + 8);

        let one = word("one");
        let two = word("two");
        assert_eq!(two.x, one.right());
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
    fn form_fixture_emits_owned_control_hit_regions() {
        let state = render(include_str!("../tests/fixtures/form-lesson.html"));
        let controls = state
            .current_scene
            .objects
            .iter()
            .filter(|object| matches!(object.kind, RenderObjectKind::Control { .. }))
            .collect::<Vec<_>>();
        assert_eq!(controls.len(), 6);
        assert!(controls
            .iter()
            .all(|object| matches!(object.interaction, Some(RenderInteraction::Control { .. }))));
        assert!(controls.iter().all(|object| object.owner_node_id.0 > 0));
    }

    #[test]
    fn image_lesson_fixture_keeps_duplicate_and_failed_images_as_distinct_nodes() {
        let state = render(include_str!("../tests/fixtures/image-lesson.html"));
        let placeholders = state
            .current_scene
            .objects
            .iter()
            .filter(|object| matches!(object.kind, RenderObjectKind::ImagePlaceholder { .. }))
            .count();
        assert_eq!(placeholders, 3);
        assert!(state.current_scene.objects.iter().any(|object| {
            matches!(
                &object.kind,
                RenderObjectKind::ImagePlaceholder { alt, .. } if alt == "Missing image fallback"
            ) && object.bounds.w == 180
                && object.bounds.h == 80
        }));
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
    fn decoded_image_uses_intrinsic_size_and_retained_image_object() {
        use crate::images::{decode_image, ImageCache};

        const TINY_PNG: &[u8] = &[
            137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1,
            8, 6, 0, 0, 0, 31, 21, 196, 137, 0, 0, 0, 13, 73, 68, 65, 84, 120, 156, 99, 248, 207,
            192, 240, 31, 0, 5, 0, 1, 255, 137, 153, 61, 29, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66,
            96, 130,
        ];
        let dom =
            parse_html("<p><a href='/go'><img src='/rabbit.png' alt='Rabbit'></a></p>").unwrap();
        let styles = StyleContext::build(&dom, &[]);
        let mut images = ImageCache::default();
        images.insert_decoded(
            String::from("https://example.com/rabbit.png"),
            decode_image(
                TINY_PNG,
                Some("image/png"),
                "https://example.com/rabbit.png",
            )
            .unwrap(),
        );
        let state = DocumentRenderState::new_with_images(
            1,
            String::from("https://example.com/index.html"),
            dom.clone(),
            styles,
            Size::new(320, 240),
            &TestMeasure,
            images,
        );
        let image_node = dom.find_first_element("img").unwrap();
        let image = state
            .current_scene
            .objects
            .iter()
            .find(|object| matches!(object.kind, RenderObjectKind::Image { .. }))
            .unwrap();
        assert_eq!(image.owner_node_id, DocumentNodeId(image_node as u64));
        assert_eq!(image.bounds.w, 1);
        assert_eq!(image.bounds.h, 1);
        assert!(matches!(
            &image.kind,
            RenderObjectKind::Image {
                source_url,
                intrinsic_width: 1,
                intrinsic_height: 1,
                alt,
                ..
            } if source_url == "https://example.com/rabbit.png" && alt == "Rabbit"
        ));
        assert!(matches!(
            image.interaction,
            Some(RenderInteraction::Link { .. })
        ));
    }

    #[test]
    fn image_dimension_priority_preserves_aspect_ratio() {
        assert_eq!(
            image_dimensions(Some(120), Some(80), Some(Size::new(20, 10))),
            (120, 80)
        );
        assert_eq!(
            image_dimensions(Some(120), None, Some(Size::new(20, 10))),
            (120, 60)
        );
        assert_eq!(
            image_dimensions(None, Some(80), Some(Size::new(20, 10))),
            (160, 80)
        );
        assert_eq!(
            image_dimensions(None, None, Some(Size::new(20, 10))),
            (20, 10)
        );
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

    #[test]
    fn image_resources_are_shared_by_url_but_each_img_node_has_its_own_object() {
        use crate::images::ImageCache;
        use alloc::sync::Arc;
        use sunlight_ui::widgets::ScenePatchOperation;

        let dom =
            parse_html("<p><img src='/red.png'><img src='/blue.png'><img src='/red.png'></p>")
                .unwrap();
        let styles = StyleContext::build(&dom, &[]);
        let mut only_red = ImageCache::default();
        only_red.insert_decoded(
            String::from("https://example.com/red.png"),
            decoded_solid_image(0xFFFF_0000),
        );
        let mut state = DocumentRenderState::new_with_images(
            1,
            String::from("https://example.com/index.html"),
            dom,
            styles,
            Size::new(320, 240),
            &TestMeasure,
            only_red.clone(),
        );

        let initial_images: Vec<_> = state
            .current_scene
            .objects
            .iter()
            .filter(|object| matches!(object.kind, RenderObjectKind::Image { .. }))
            .collect();
        assert_eq!(initial_images.len(), 2);
        assert_ne!(initial_images[0].id, initial_images[1].id);
        let (first_pixels, second_pixels) = match (&initial_images[0].kind, &initial_images[1].kind)
        {
            (
                RenderObjectKind::Image { image: first, .. },
                RenderObjectKind::Image { image: second, .. },
            ) => (first, second),
            _ => unreachable!(),
        };
        assert!(Arc::ptr_eq(first_pixels, second_pixels));
        assert_eq!(
            state
                .current_scene
                .objects
                .iter()
                .filter(|object| matches!(object.kind, RenderObjectKind::ImagePlaceholder { .. }))
                .count(),
            1
        );

        let mut completed = only_red;
        completed.insert_decoded(
            String::from("https://example.com/blue.png"),
            decoded_solid_image(0xFF00_00FF),
        );
        state.rebuild_for_images(&TestMeasure, completed);
        let images: Vec<_> = state
            .current_scene
            .objects
            .iter()
            .filter(|object| matches!(object.kind, RenderObjectKind::Image { .. }))
            .collect();
        assert_eq!(images.len(), 3);
        assert!(images
            .iter()
            .all(|object| object.bounds.w > 0 && object.bounds.h > 0));
        assert!(state.last_patch.operations.iter().any(|operation| matches!(
            operation,
            ScenePatchOperation::Update { object, .. }
                if matches!(&object.kind, RenderObjectKind::Image { source_url, .. }
                    if source_url == "https://example.com/blue.png")
        )));
    }

    #[test]
    fn image_scene_is_independent_of_resource_completion_order() {
        use crate::images::ImageCache;

        let render_with_order = |urls: &[(&str, u32)]| {
            let dom = parse_html("<p><img src='/a.png'><img src='/b.png'><img src='/c.png'></p>")
                .unwrap();
            let styles = StyleContext::build(&dom, &[]);
            let mut images = ImageCache::default();
            for (url, color) in urls {
                images.insert_decoded(
                    format!("https://example.com/{url}"),
                    decoded_solid_image(*color),
                );
            }
            DocumentRenderState::new_with_images(
                1,
                String::from("https://example.com/index.html"),
                dom,
                styles,
                Size::new(320, 240),
                &TestMeasure,
                images,
            )
        };
        let forward = render_with_order(&[
            ("a.png", 0xFFFF_0000),
            ("b.png", 0xFF00_FF00),
            ("c.png", 0xFF00_00FF),
        ]);
        let reverse = render_with_order(&[
            ("c.png", 0xFF00_00FF),
            ("b.png", 0xFF00_FF00),
            ("a.png", 0xFFFF_0000),
        ]);
        assert_eq!(forward.current_scene, reverse.current_scene);
    }

    #[test]
    fn deterministic_image_matrix_keeps_every_decoded_node_visible() {
        use crate::images::ImageCache;

        let dom = parse_html(include_str!("../tests/fixtures/image-matrix.html")).unwrap();
        let styles = StyleContext::build(&dom, &[]);
        let mut images = ImageCache::default();
        for (name, color) in [
            ("rgb-red.png", 0xFFFF_0000),
            ("rgb-blue.png", 0xFF00_00FF),
            ("rgba.png", 0x8000_FF00),
            ("indexed.png", 0xFFFF_FF00),
            ("gray.png", 0xFF80_8080),
        ] {
            images.insert_decoded(
                format!("https://example.com/{name}"),
                decoded_solid_image(color),
            );
        }
        let state = DocumentRenderState::new_with_images(
            1,
            String::from("https://example.com/index.html"),
            dom,
            styles,
            Size::new(320, 240),
            &TestMeasure,
            images,
        );
        let objects: Vec<_> = state
            .current_scene
            .objects
            .iter()
            .filter(|object| matches!(object.kind, RenderObjectKind::Image { .. }))
            .collect();
        assert_eq!(objects.len(), 6);
        assert_eq!(
            objects
                .iter()
                .map(|object| object.id)
                .collect::<alloc::collections::BTreeSet<_>>()
                .len(),
            6
        );
        assert!(objects
            .iter()
            .all(|object| object.bounds.w > 0 && object.bounds.h > 0));
    }

    #[test]
    fn lists_have_owned_stable_markers_and_visible_ordinals() {
        let mut state = render("<ol start=3><li>one</li><li style='display:none'>hidden</li><li>three</li></ol><ul><li>a</li><li>b</li></ul>");
        let markers: Vec<_> = state
            .layout_tree
            .nodes
            .iter()
            .filter_map(|node| node.marker.as_ref())
            .collect();
        assert_eq!(markers.len(), 4);
        assert_eq!(markers[0].label.as_deref(), Some("3."));
        assert_eq!(markers[1].label.as_deref(), Some("4."));
        let ids: alloc::collections::BTreeSet<_> = state
            .current_scene
            .objects
            .iter()
            .filter(|object| (object.id.0 >> 24) & 0xff == 40)
            .map(|object| object.id)
            .collect();
        assert_eq!(ids.len(), 4);
        let before = ids.clone();
        state.rebuild(Size::new(320, 240), &TestMeasure);
        let after: alloc::collections::BTreeSet<_> = state
            .current_scene
            .objects
            .iter()
            .filter(|object| (object.id.0 >> 24) & 0xff == 40)
            .map(|object| object.id)
            .collect();
        assert_eq!(before, after);
    }

    #[test]
    fn typography_fixture_preserves_pre_and_emits_semantic_scene_objects() {
        let state = render(include_str!("../tests/fixtures/typography-lists.html"));
        assert!(state.current_scene.objects.iter().any(|object| matches!(&object.kind, RenderObjectKind::Text { text, .. } if text.contains("    println"))));
        assert!(state.current_scene.objects.iter().any(|object| matches!(&object.kind, RenderObjectKind::Text { text, bold: true, .. } if text == "Bold")));
        assert!(state.current_scene.objects.iter().any(|object| matches!(&object.kind, RenderObjectKind::Text { text, italic: true, .. } if text == "italic")));
        assert!(state.current_scene.objects.iter().any(|object| matches!(&object.kind, RenderObjectKind::Text { text, monospace: true, .. } if text == "monospace")));
        assert!(state.current_scene.objects.iter().any(|object| matches!(&object.kind, RenderObjectKind::Text { text, line_through: true, .. } if text == "removed")));
        assert!(state.current_scene.objects.iter().any(|object| matches!(&object.kind, RenderObjectKind::Text { text, underline: true, .. } if text == "inserted")));
        assert!(state
            .current_scene
            .objects
            .iter()
            .any(|object| matches!(object.kind, RenderObjectKind::Border { .. })));
    }

    #[test]
    fn flex_navigation_is_horizontal_and_suppresses_only_its_markers() {
        let state = render(include_str!("../tests/fixtures/flex-navigation.html"));
        let nav_markers = state
            .layout_tree
            .nodes
            .iter()
            .filter(|node| node.marker.is_some() && node.border_box.y < 220)
            .count();
        assert_eq!(nav_markers, 0);
        assert!(state
            .layout_tree
            .nodes
            .iter()
            .any(|node| node.marker.is_some()));
        let links: Vec<_> = state
            .current_scene
            .objects
            .iter()
            .filter(|object| matches!(object.kind, RenderObjectKind::Link { .. }))
            .collect();
        assert!(links.len() >= 6);
        let link_text_y: Vec<_> = links.iter().map(|object| object.bounds.y).collect();
        assert!(link_text_y.iter().take(5).any(|y| *y == link_text_y[0]));
        let header = state.dom.find_first_element("header").unwrap();
        let header_node = state
            .layout_tree
            .nodes
            .iter()
            .find(|node| node.owner_node_id == DocumentNodeId(header as u64))
            .unwrap();
        let background = state
            .current_scene
            .objects
            .iter()
            .find(|object| {
                object.owner_node_id == DocumentNodeId(header as u64)
                    && matches!(object.kind, RenderObjectKind::Rectangle { .. })
            })
            .unwrap();
        assert_eq!(background.bounds, header_node.border_box);
    }

    #[test]
    fn css_font_family_fallbacks_are_preserved_in_retained_text() {
        let state = render(include_str!("../tests/fixtures/font-families.html"));
        assert!(state.current_scene.objects.iter().any(|object| matches!(&object.kind,
            RenderObjectKind::Text { text, font_family: DocumentFontFamily::Serif, .. } if text == "Serif:" || text == "Named")));
        assert!(state.current_scene.objects.iter().any(|object| matches!(&object.kind,
            RenderObjectKind::Text { text, font_family: DocumentFontFamily::Monospace, .. } if text.contains("Sun Mono"))));
    }

    fn decoded_solid_image(pixel: u32) -> crate::images::DecodedImage {
        crate::images::DecodedImage {
            image: alloc::sync::Arc::new(sunlight_ui::widgets::RasterImage {
                width: 1,
                height: 1,
                pixels: vec![pixel],
            }),
            format: crate::images::ImageFormat::Png,
            byte_size: 4,
        }
    }

    #[test]
    fn classic_kernel_fixture_centers_floats_clears_and_grids_table() {
        let html = include_str!("../tests/fixtures/kernel-classic-layout.html");
        let dom = parse_html(html).unwrap();
        let styles = StyleContext::build(&dom, &collect_embedded_stylesheets(&dom));
        let state = DocumentRenderState::new(
            9,
            String::from("https://fixture.test/"),
            dom,
            styles,
            Size::new(1000, 700),
            &TestMeasure,
        );
        let by_id = |id: &str| {
            (0..state.dom.node_count())
                .find_map(|node_id| match state.dom.get(node_id) {
                    Some(Node::Element { attributes, .. })
                        if attr(attributes, "id") == Some(id) =>
                    {
                        Some(node_id)
                    }
                    _ => None,
                })
                .unwrap()
        };
        let banner_id = by_id("banner");
        let latest_id = by_id("latest-download");
        let releases_id = by_id("releases");
        let node = |id| {
            state
                .layout_tree
                .nodes
                .iter()
                .find(|node| node.owner_node_id == DocumentNodeId(id as u64))
                .unwrap()
        };
        let banner = node(banner_id);
        assert_eq!(banner.content_box.w, 800);
        assert_eq!(banner.padding_box.w, 832);
        assert!(banner.border_box.x >= 80 && banner.border_box.x <= 84);
        assert_eq!(banner.paint.corner_radii.bottom_left, 8);
        assert_eq!(banner.paint.corner_radii.bottom_right, 8);
        assert_eq!(banner.paint.corner_radii.top_left, 0);
        assert_eq!(banner.paint.border_colors[0], Color::rgb(0xdd, 0xda, 0xdf));
        assert_eq!(banner.paint.border_colors[1], Color::rgb(0xcc, 0xc8, 0xb8));
        assert_eq!(banner.paint.border_colors[2], Color::rgb(0xbb, 0xb5, 0x9f));
        assert!(state
            .current_scene
            .objects
            .iter()
            .any(
                |object| object.owner_node_id == DocumentNodeId(banner_id as u64)
                    && matches!(
                        object.kind,
                        RenderObjectKind::BoxShadow { inset: false, .. }
                    )
            ));
        let latest = node(latest_id);
        let releases = node(releases_id);
        assert!(latest.border_box.x > 700);
        assert!(releases.border_box.y >= latest.margin_box.bottom());
        assert_eq!(releases.content_box.w, 1000);
        assert!(state
            .current_scene
            .objects
            .iter()
            .any(
                |object| object.owner_node_id == DocumentNodeId(latest_id as u64)
                    && matches!(object.kind, RenderObjectKind::BoxShadow { inset: true, .. })
            ));
        let cell_nodes = state
            .layout_tree
            .nodes
            .iter()
            .filter(|node| node.display == DisplayType::TableCell)
            .collect::<Vec<_>>();
        assert_eq!(cell_nodes.len(), 12);
        assert_eq!(cell_nodes[0].border_box.x, cell_nodes[4].border_box.x);
        assert_eq!(cell_nodes[1].border_box.x, cell_nodes[5].border_box.x);
        assert!(state.current_scene.objects.iter().any(|object| matches!(object.interaction, Some(RenderInteraction::Link { owner_node_id, .. }) if owner_node_id == DocumentNodeId(latest_id as u64)) && object.bounds.x >= latest.border_box.x));
    }

    #[test]
    fn import_dependent_kernel_cards_reach_layout_and_scene() {
        let dom = parse_html(include_str!(
            "../tests/fixtures/import-kernel-cards/index.html"
        ))
        .unwrap();
        let sheets = [
            parse_stylesheet(
                include_str!("../tests/fixtures/import-kernel-cards/base.css"),
                StylesheetSource::External(String::from("https://fixture.test/base.css")),
            ),
            parse_stylesheet(
                include_str!("../tests/fixtures/import-kernel-cards/cards/card.css"),
                StylesheetSource::External(String::from("https://fixture.test/cards/card.css")),
            ),
            parse_stylesheet(
                include_str!("../tests/fixtures/import-kernel-cards/main.css"),
                StylesheetSource::External(String::from("https://fixture.test/main.css")),
            ),
        ];
        let styles = StyleContext::build(&dom, &sheets);
        let state = DocumentRenderState::new(
            10,
            String::from("https://fixture.test/index.html"),
            dom,
            styles,
            Size::new(1000, 700),
            &TestMeasure,
        );
        let by_id = |id: &str| {
            (0..state.dom.node_count())
                .find(|node_id| match state.dom.get(*node_id) {
                    Some(Node::Element { attributes, .. }) => attr(attributes, "id") == Some(id),
                    _ => false,
                })
                .unwrap()
        };
        let layout = |id| {
            state
                .layout_tree
                .nodes
                .iter()
                .find(|node| node.owner_node_id == DocumentNodeId(id as u64))
                .unwrap()
        };
        let banner_id = by_id("banner");
        let featured_id = by_id("featured");
        let latest_id = by_id("latest");
        let releases_id = by_id("releases");
        let banner = layout(banner_id);
        let featured = layout(featured_id);
        let latest = layout(latest_id);
        let releases = layout(releases_id);
        assert_eq!(banner.content_box.w, 800);
        assert!(banner.border_box.x >= 80 && banner.border_box.x <= 84);
        assert_eq!(banner.paint.background, Color::rgb(255, 255, 255));
        assert_eq!(featured.paint.background, Color::rgb(255, 255, 255));
        assert_eq!(latest.paint.background, Color::rgb(0xff, 0xd1, 0x33));
        assert_eq!(latest.float_side, "right");
        assert!(latest.border_box.x > featured.content_box.x + featured.content_box.w as i32 / 2);
        assert!(releases.border_box.y >= latest.margin_box.bottom());
        assert!(state.current_scene.objects.iter().any(|object| {
            object.owner_node_id == DocumentNodeId(banner_id as u64)
                && matches!(
                    object.kind,
                    RenderObjectKind::BoxShadow { inset: false, .. }
                )
        }));
        assert!(state.current_scene.objects.iter().any(|object| {
            object.owner_node_id == DocumentNodeId(latest_id as u64)
                && matches!(object.kind, RenderObjectKind::BoxShadow { inset: true, .. })
        }));
    }
}
