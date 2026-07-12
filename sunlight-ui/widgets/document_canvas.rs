use alloc::{collections::BTreeMap, string::String, sync::Arc, vec, vec::Vec};

use crate::font::VecText;
use crate::geom::{Point, Rect, Size};
use crate::paint::Canvas;
use crate::theme::{Color, Theme};

/// Temporary per-item editing state — never stored in persistent document data.
///
/// The canvas and its callers share this transient state.  The persistent text
/// content lives in an owner-managed buffer; the edit state only holds the
/// caret / selection metadata used for rendering and hit testing.
///
/// ## Caret indexing strategy
///
/// The caret is indexed by **byte offset** into the UTF-8 text.  Every caret
/// movement, insertion, and deletion boundary is validated via
/// `str::char_indices()` so the caret never lands in the middle of a multi-byte
/// code point.  The font measurement functions (`measure_text_width` and the
/// cluster-aware helpers below) accept arbitrary byte slices and always produce
/// a valid width, so hit-testing and pixel-to-caret mapping are safe regardless
/// of non-ASCII or emoji content.
///
/// ## Grapheme-cluster awareness
///
/// Font rendering and measurement operate at the Unicode scalar-value
/// level (individual `char`s).  Full grapheme-cluster awareness (e.g. moving
/// the caret across a ZWJ emoji sequence as a single unit) is not yet
/// implemented but the byte-offset infrastructure is structured so that it
/// can be added later without changing the data model.
#[derive(Clone, Debug, Default)]
pub struct TextEditState {
    pub active_item_index: Option<usize>,
    pub caret_byte: usize,
    pub selection_anchor_byte: Option<usize>,
    pub preferred_caret_x: Option<u32>,
}

impl TextEditState {
    pub fn is_editing(&self) -> bool {
        self.active_item_index.is_some()
    }

    pub fn clear(&mut self) {
        self.active_item_index = None;
        self.caret_byte = 0;
        self.selection_anchor_byte = None;
        self.preferred_caret_x = None;
    }
}

/// Pixel X-offset (from the start of the text) of the caret at `byte_offset`.
///
/// Uses the same `measure_text_width` helper employed by rendering, so the
/// caret always sits at a position consistent with the visible glyphs.
pub fn caret_x_at_byte(font: Option<&dyn VecText>, text: &str, byte_offset: usize) -> u32 {
    let clamped = byte_offset.min(text.len());
    if clamped == 0 {
        return 0;
    }
    let prefix = &text[..clamped];
    measure_text_width(font, prefix)
}

/// Best byte offset for a given horizontal pixel offset inside rendered text.
///
/// Walks logical characters (`char_indices`) so the returned offset is always
/// a valid UTF-8 code-point boundary.  When `target_x` falls past the halfway
/// point of a glyph, the offset immediately after that glyph is returned
/// (closest-nearest behaviour).
pub fn byte_offset_at_x(font: Option<&dyn VecText>, text: &str, target_x: i32) -> usize {
    if target_x <= 0 || text.is_empty() {
        return 0;
    }
    let mut prev_offset = 0usize;
    let mut prev_w = 0u32;
    for (idx, ch) in text.char_indices() {
        let next = idx + ch.len_utf8();
        let w = measure_text_width(font, &text[..next]);
        let target = target_x.max(0) as u32;
        if target <= w {
            let mid = (prev_w + w) / 2;
            return if target <= mid { idx } else { next };
        }
        prev_offset = next;
        prev_w = w;
    }
    prev_offset.max(text.len())
}

/// One visual line produced by [`layout_text_lines`].
///
/// Byte range `[byte_start, byte_end)` references the full text buffer.
/// When `ends_with_newline` is true, `byte_end` includes the `'\n'` that
/// caused the break; the renderer should skip that trailing byte.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextLineLayout {
    pub byte_start: usize,
    pub byte_end: usize,
    pub y_offset: i32,
    pub pixel_width: u32,
    pub ends_with_newline: bool,
}

/// Lay out `text` into visual lines given a font, maximum pixel width, and
/// per-line height.
///
/// # Breaking rules
///
/// - Explicit `'\n'` always causes a line break.  The newline byte is included
///   in `byte_end` of the line that ends with it.
/// - When the rendered width of the current line exceeds `max_width`, a *soft
///   wrap* is inserted at the last space character before the overflow.  If no
///   space exists, the line breaks at the last character that fits (simple
///   character-level fallback).
/// - Every `byte_start` and `byte_end` is a valid UTF-8 boundary.
///
/// Callers use the returned layout for rendering, caret placement, hit testing,
/// and vertical navigation — no separate line-breaking pass is needed.
pub fn layout_text_lines(
    font: Option<&dyn VecText>,
    text: &str,
    max_width: u32,
    line_height: u32,
) -> Vec<TextLineLayout> {
    let mut lines: Vec<TextLineLayout> = Vec::new();
    let mut byte_start = 0usize;
    let mut y_offset = 0i32;

    while byte_start < text.len() {
        let mut line_end = byte_start;
        let mut last_space_end: Option<usize> = None;
        let mut ends_with_newline = false;

        for (idx, ch) in text[byte_start..].char_indices() {
            let abs_idx = byte_start + idx;
            let next = abs_idx + ch.len_utf8();

            if ch == '\n' {
                line_end = next;
                ends_with_newline = true;
                break;
            }

            let w = measure_text_width(font, &text[byte_start..next]);
            if w > max_width {
                if let Some(space) = last_space_end {
                    line_end = space;
                } else if line_end == byte_start {
                    line_end = next;
                }
                ends_with_newline = false;
                break;
            }

            if ch == ' ' {
                last_space_end = Some(next);
            }
            line_end = next;
        }

        if line_end == byte_start && byte_start < text.len() {
            let first = text[byte_start..].chars().next().unwrap();
            line_end = byte_start + first.len_utf8();
        }

        let px_width = measure_text_width(font, &text[byte_start..line_end]);
        lines.push(TextLineLayout {
            byte_start,
            byte_end: line_end,
            y_offset,
            pixel_width: px_width,
            ends_with_newline,
        });

        byte_start = line_end;
        y_offset = y_offset.wrapping_add(line_height as i32);
    }

    lines
}

/// Index into `lines` that contains `byte_offset`.  If the offset falls on a
/// boundary between two lines (e.g. just after a `'\n'`), the *next* line is
/// returned so that typing starts on the fresh visual row.
pub fn find_line_index(lines: &[TextLineLayout], byte_offset: usize) -> Option<usize> {
    if lines.is_empty() {
        return None;
    }
    for (idx, line) in lines.iter().enumerate() {
        if byte_offset >= line.byte_start && byte_offset < line.byte_end {
            return Some(idx);
        }
        if byte_offset == line.byte_end && line.ends_with_newline && idx + 1 < lines.len() {
            return Some(idx + 1);
        }
    }
    Some(lines.len().saturating_sub(1))
}

/// Byte offset of the *start* of the visual line (Home key position).
pub fn line_home_byte(lines: &[TextLineLayout], line_index: usize) -> usize {
    lines.get(line_index).map_or(0, |l| l.byte_start)
}

/// Byte offset of the *end* of the visual line (End key position).
/// Skips a trailing `'\n'` so the caret sits just before the newline rather
/// than after it.
pub fn line_end_byte(lines: &[TextLineLayout], line_index: usize) -> usize {
    lines.get(line_index).map_or(0, |l| {
        if l.ends_with_newline && l.byte_end > l.byte_start {
            l.byte_end.saturating_sub(1)
        } else {
            l.byte_end
        }
    })
}

/// Caret pixel x-offset *relative to the start of a single visual line*.
/// `caret_byte` must lie within `[line.byte_start, line.byte_end]`.
pub fn caret_x_on_line(
    font: Option<&dyn VecText>,
    text: &str,
    line: &TextLineLayout,
    caret_byte: usize,
) -> u32 {
    let visible_text = line_visible_text(text, line);
    let clamped = caret_byte.min(line.byte_end).max(line.byte_start);
    let local = (clamped - line.byte_start).min(visible_text.len());
    caret_x_at_byte(font, visible_text, local)
}

/// Byte offset within `line` nearest to a horizontal pixel offset `target_x`
/// (measured from the start of the line).  Always returns a valid UTF-8
/// boundary relative to the line's `byte_start`.
pub fn byte_at_x_on_line(
    font: Option<&dyn VecText>,
    text: &str,
    line: &TextLineLayout,
    target_x: i32,
) -> usize {
    let visible_text = line_visible_text(text, line);
    let local = byte_offset_at_x(font, visible_text, target_x);
    line.byte_start + local
}

fn line_visible_text<'a>(text: &'a str, line: &TextLineLayout) -> &'a str {
    let slice = &text[line.byte_start..line.byte_end];
    if line.ends_with_newline && slice.ends_with('\n') {
        &slice[..slice.len() - 1]
    } else {
        slice
    }
}

/// Find which visual line a document-local (y, x_hint) point lands on, and the
/// nearest byte offset within that line for horizontal positioning.
/// `item_y` is the document-coordinate Y of the text item top.
pub fn click_to_line_and_byte(
    font: Option<&dyn VecText>,
    text: &str,
    lines: &[TextLineLayout],
    item_y: i32,
    click_y: i32,
    click_x: i32,
    item_x: i32,
) -> Option<(usize, usize)> {
    if lines.is_empty() {
        return None;
    }
    let rel_y = click_y - item_y;
    let mut best_line = 0usize;
    let mut best_dist = i32::MAX;
    for (idx, line) in lines.iter().enumerate() {
        let dist = (rel_y - line.y_offset).abs();
        if dist < best_dist {
            best_dist = dist;
            best_line = idx;
        }
    }
    let line = &lines[best_line];
    let local_x = click_x - item_x;
    let byte = byte_at_x_on_line(font, text, line, local_x);
    Some((best_line, byte))
}

const WRITER_WORKSPACE_GAP: i32 = 16;
const DOCUMENT_MARGIN: i32 = 28;

/// Generic, document-local identity for the node that owns a render object.
///
/// The canvas intentionally does not know whether this identity originated in
/// HTML, a Writer document, mail, or a help page.  Producers translate their
/// own stable node identifiers to this opaque value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DocumentNodeId(pub u64);

/// Stable identity for one retained object in a [`DocumentScene`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RenderObjectId(pub u64);

/// Generic resolved family identity carried by retained text.  Producers map
/// their own CSS or document vocabulary here; Canvas only selects a supplied
/// shared face and never interprets HTML.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DocumentFontFamily {
    #[default]
    SansSerif,
    Serif,
    Monospace,
}

impl DocumentFontFamily {
    pub fn resolve_css_list(value: &str) -> Self {
        let mut fallback = Self::SansSerif;
        for raw in value.split(',') {
            let name = raw
                .trim()
                .trim_matches('\"')
                .trim_matches('\'')
                .to_ascii_lowercase();
            match name.as_str() {
                // The native family names and common web aliases share the
                // same cached face.  Unknown names are deliberately ignored
                // until a later generic fallback appears in the list.
                "sun serif" | "serif" | "times" | "times new roman" | "georgia" | "noto serif" => {
                    return Self::Serif
                }
                "sun font" | "sans-serif" | "arial" | "helvetica" | "verdana" | "inter" => {
                    fallback = Self::SansSerif
                }
                "sun mono" | "monospace" | "fira code" | "courier" | "courier new" | "consolas" => {
                    return Self::Monospace
                }
                _ => {}
            }
        }
        fallback
    }
}

/// Generic decoded raster data shared by retained document producers.
///
/// Pixels are packed ARGB8888 and top-down. The scene stores this behind an
/// `Arc` so reflows and duplicated HTML image elements do not copy pixels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RasterImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u32>,
}

/// A small deterministic paint ordering key.  It is deliberately more
/// expressive than just z-index so later stacking-context work has a stable
/// place to grow without changing the scene format.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct PaintOrder {
    pub stacking_context: u32,
    pub z_index: i32,
    pub phase: u8,
    pub document_order: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CornerRadii {
    pub top_left: u32,
    pub top_right: u32,
    pub bottom_right: u32,
    pub bottom_left: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RenderObjectKind {
    /// Structural retained box.  It carries geometry and ownership even when
    /// the element has no visible background or border in this first pass.
    Box,
    Text {
        text: String,
        color: Color,
        font_size: u32,
        bold: bool,
        italic: bool,
        underline: bool,
        line_through: bool,
        monospace: bool,
        font_family: DocumentFontFamily,
    },
    Rectangle {
        fill: Color,
    },
    RoundedRectangle {
        fill: Color,
        radii: CornerRadii,
    },
    Border {
        color: Color,
        width: u32,
    },
    BorderSides {
        colors: [Color; 4],
        widths: [u32; 4],
        radii: CornerRadii,
    },
    BoxShadow {
        box_bounds: Rect,
        radii: CornerRadii,
        offset_x: i32,
        offset_y: i32,
        blur: u32,
        spread: i32,
        color: Color,
        inset: bool,
    },
    Line {
        color: Color,
        width: u32,
    },
    /// A non-painting hit region.  Text fragments for this anchor remain
    /// individual objects so they can be patched independently.
    Link {
        href: String,
        resolved_url: Option<String>,
        text_object_ids: Vec<RenderObjectId>,
    },
    ImagePlaceholder {
        src: String,
        alt: String,
    },
    Image {
        image: Arc<RasterImage>,
        source_url: String,
        intrinsic_width: u32,
        intrinsic_height: u32,
        alt: String,
    },
    /// Generic browser/document control. The producer owns its semantics;
    /// Canvas only paints the supplied retained visual and caret data.
    Control {
        label: String,
        placeholder: String,
        value: String,
        color: Color,
        background: Color,
        border_color: Color,
        border_width: u32,
        focused: bool,
        disabled: bool,
        editable: bool,
        kind: u8,
        caret_offset: Option<u32>,
        font_size: u32,
        font_family: DocumentFontFamily,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RenderInteraction {
    Link {
        owner_node_id: DocumentNodeId,
        href: String,
        resolved_url: Option<String>,
    },
    Control {
        owner_node_id: DocumentNodeId,
    },
}

/// One retained, generic visual or interactive object.  Bounds are always in
/// document coordinates; viewport scrolling is a canvas concern.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderObject {
    pub id: RenderObjectId,
    pub owner_node_id: DocumentNodeId,
    pub kind: RenderObjectKind,
    pub bounds: Rect,
    pub clip_bounds: Option<Rect>,
    pub paint_order: PaintOrder,
    pub interaction: Option<RenderInteraction>,
}

/// Retained scene shared by read-only document surfaces.  The two indexes are
/// built once when a producer finalizes the scene, avoiding full scans during
/// later object or node lookups.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentScene {
    pub document_id: u64,
    pub viewport_size: Size,
    pub content_size: Size,
    pub objects: Vec<RenderObject>,
    node_to_object_map: BTreeMap<DocumentNodeId, Vec<RenderObjectId>>,
    object_index: BTreeMap<RenderObjectId, usize>,
}

impl DocumentScene {
    pub fn new(document_id: u64, viewport_size: Size, content_size: Size) -> Self {
        Self {
            document_id,
            viewport_size,
            content_size,
            objects: Vec::new(),
            node_to_object_map: BTreeMap::new(),
            object_index: BTreeMap::new(),
        }
    }

    pub fn push(&mut self, object: RenderObject) -> bool {
        if self.object_index.contains_key(&object.id) {
            return false;
        }
        let index = self.objects.len();
        self.node_to_object_map
            .entry(object.owner_node_id)
            .or_default()
            .push(object.id);
        self.object_index.insert(object.id, index);
        self.objects.push(object);
        true
    }

    /// Sorts objects into deterministic paint order and rebuilds the object
    /// index.  Call after producing a scene and before comparing or painting.
    pub fn finalize(&mut self) {
        self.objects.sort_by_key(|object| object.paint_order);
        self.rebuild_indexes();
    }

    pub fn object(&self, id: RenderObjectId) -> Option<&RenderObject> {
        self.object_index
            .get(&id)
            .and_then(|index| self.objects.get(*index))
    }

    pub fn objects_for_node(&self, node_id: DocumentNodeId) -> &[RenderObjectId] {
        self.node_to_object_map
            .get(&node_id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn max_scroll_y(&self, viewport_h: u32) -> u32 {
        self.content_size.h.saturating_sub(viewport_h)
    }

    /// Returns the topmost retained object containing a document-coordinate
    /// point. Consumers can inspect [`RenderObject::interaction`] to dispatch
    /// links without teaching the canvas about URLs or navigation.
    pub fn hit_test(&self, point: Point) -> Option<&RenderObject> {
        self.objects
            .iter()
            .rev()
            .find(|object| object.bounds.contains(point))
    }

    /// Applies an incremental object patch to this retained scene. The
    /// document producer remains responsible for viewport/content metadata;
    /// a `ReplaceScene` deliberately returns `false` so its caller can swap
    /// in the new complete scene instead.
    pub fn apply_patch(&mut self, patch: &ScenePatch) -> bool {
        if patch
            .operations
            .iter()
            .any(|operation| matches!(operation, ScenePatchOperation::ReplaceScene))
        {
            return false;
        }

        for operation in &patch.operations {
            match operation {
                ScenePatchOperation::Insert { object, .. } => {
                    if self.object(object.id).is_some() {
                        return false;
                    }
                    self.objects.push(object.clone());
                }
                ScenePatchOperation::Update { object, .. } => {
                    let Some(index) = self.object_index.get(&object.id).copied() else {
                        return false;
                    };
                    self.objects[index] = object.clone();
                }
                ScenePatchOperation::Remove { id, .. } => {
                    let Some(index) = self.object_index.get(id).copied() else {
                        return false;
                    };
                    self.objects.remove(index);
                    self.rebuild_indexes();
                }
                ScenePatchOperation::Reorder {
                    id, paint_order, ..
                } => {
                    let Some(index) = self.object_index.get(id).copied() else {
                        return false;
                    };
                    self.objects[index].paint_order = *paint_order;
                }
                ScenePatchOperation::ReplaceScene => return false,
            }
        }
        self.finalize();
        true
    }

    fn rebuild_indexes(&mut self) {
        self.node_to_object_map.clear();
        self.object_index.clear();
        for (index, object) in self.objects.iter().enumerate() {
            self.node_to_object_map
                .entry(object.owner_node_id)
                .or_default()
                .push(object.id);
            self.object_index.insert(object.id, index);
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScenePatchOperation {
    Insert {
        object: RenderObject,
        dirty: Rect,
    },
    Update {
        object: RenderObject,
        dirty: Rect,
    },
    Remove {
        id: RenderObjectId,
        dirty: Rect,
    },
    Reorder {
        id: RenderObjectId,
        paint_order: PaintOrder,
        dirty: Rect,
    },
    ReplaceScene,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScenePatch {
    pub operations: Vec<ScenePatchOperation>,
    pub dirty_regions: Vec<Rect>,
}

impl ScenePatch {
    pub fn is_empty(&self) -> bool {
        self.operations.is_empty()
    }

    /// Adds a dirty rectangle to a bounded, coalesced region list. This keeps
    /// diagnostics useful now and gives a future partial compositor a small
    /// repaint set instead of one allocation per object change.
    fn add_dirty(&mut self, rect: Rect) {
        const MAX_DIRTY_REGIONS: usize = 64;
        if rect.w == 0 || rect.h == 0 {
            return;
        }
        if let Some(existing) = self
            .dirty_regions
            .iter_mut()
            .find(|existing| rects_touch_or_overlap(**existing, rect))
        {
            *existing = union_rect(*existing, rect);
            return;
        }
        if self.dirty_regions.len() < MAX_DIRTY_REGIONS {
            self.dirty_regions.push(rect);
        } else if let Some(first) = self.dirty_regions.first_mut() {
            *first = union_rect(*first, rect);
        }
    }
}

fn rects_touch_or_overlap(left: Rect, right: Rect) -> bool {
    left.x <= right.right()
        && right.x <= left.right()
        && left.y <= right.bottom()
        && right.y <= left.bottom()
}

fn union_rect(left: Rect, right: Rect) -> Rect {
    let x = left.x.min(right.x);
    let y = left.y.min(right.y);
    let right_edge = left.right().max(right.right());
    let bottom_edge = left.bottom().max(right.bottom());
    Rect::new(
        x,
        y,
        (right_edge.saturating_sub(x)).max(0) as u32,
        (bottom_edge.saturating_sub(y)).max(0) as u32,
    )
}

/// Compares retained scenes by stable render-object identity.  The canvas may
/// currently repaint as a whole, but dirty regions are retained for diagnostics
/// and a future partial compositor.
pub fn diff_scenes(previous: &DocumentScene, next: &DocumentScene) -> ScenePatch {
    if previous.document_id != next.document_id {
        return ScenePatch {
            operations: vec![ScenePatchOperation::ReplaceScene],
            dirty_regions: vec![Rect::new(
                0,
                0,
                previous.content_size.w.max(next.content_size.w),
                previous.content_size.h.max(next.content_size.h),
            )],
        };
    }

    let mut patch = ScenePatch::default();
    for old in &previous.objects {
        if next.object(old.id).is_none() {
            patch.add_dirty(old.bounds);
            patch.operations.push(ScenePatchOperation::Remove {
                id: old.id,
                dirty: old.bounds,
            });
        }
    }
    for new in &next.objects {
        match previous.object(new.id) {
            None => {
                patch.add_dirty(new.bounds);
                patch.operations.push(ScenePatchOperation::Insert {
                    object: new.clone(),
                    dirty: new.bounds,
                });
            }
            Some(old) if old == new => {}
            Some(old) => {
                let dirty = union_rect(old.bounds, new.bounds);
                patch.add_dirty(dirty);
                if old.id == new.id
                    && old.owner_node_id == new.owner_node_id
                    && old.kind == new.kind
                    && old.bounds == new.bounds
                    && old.clip_bounds == new.clip_bounds
                    && old.interaction == new.interaction
                    && old.paint_order != new.paint_order
                {
                    patch.operations.push(ScenePatchOperation::Reorder {
                        id: new.id,
                        paint_order: new.paint_order,
                        dirty,
                    });
                } else {
                    patch.operations.push(ScenePatchOperation::Update {
                        object: new.clone(),
                        dirty,
                    });
                }
            }
        }
    }
    patch
}

fn draw_text(
    canvas: &mut Canvas,
    font: Option<&dyn VecText>,
    x: i32,
    y: i32,
    text: &str,
    color: Color,
) {
    if let Some(font) = font {
        font.draw(canvas, text, x, y, color);
    } else {
        canvas.draw_text(x, y, text, color);
    }
}

fn draw_text_vcenter(
    canvas: &mut Canvas,
    font: Option<&dyn VecText>,
    x: i32,
    y: i32,
    h: u32,
    text: &str,
    color: Color,
) {
    if let Some(font) = font {
        font.draw_vcenter(canvas, text, x, y, h, color);
    } else {
        let ty = y + (h as i32 - crate::paint::font::GLYPH_H as i32) / 2;
        canvas.draw_text(x, ty, text, color);
    }
}

fn measure_text_width(font: Option<&dyn VecText>, text: &str) -> u32 {
    font.map(|font| font.measure_w(text))
        .unwrap_or_else(|| Canvas::measure_text(text))
}

fn clip_text_to_width<'a>(font: Option<&dyn VecText>, text: &'a str, max_w: u32) -> &'a str {
    if max_w == 0 {
        return "";
    }
    if measure_text_width(font, text) <= max_w {
        return text;
    }

    let mut end = 0;
    for (idx, ch) in text.char_indices() {
        let next = idx + ch.len_utf8();
        if measure_text_width(font, &text[..next]) > max_w {
            break;
        }
        end = next;
    }
    &text[..end]
}

/// Approximate bounding box for immediate-mode items used during
/// semantic hit-testing.  Text boxes are computed from the item's font
/// and measured width; shape items use their explicit geometry.
fn item_bounds(item: DocumentCanvasItem) -> Rect {
    match item {
        DocumentCanvasItem::Text { x, y, text, style } => {
            let w = measure_text_width(style.font, text);
            let h = style
                .font
                .map(|f| f.line_height())
                .unwrap_or(crate::paint::font::GLYPH_H);
            Rect::new(x, y, w.max(1), h.max(1))
        }
        DocumentCanvasItem::LinkText { x, y, text, style, .. } => {
            let w = measure_text_width(style.font, text);
            let h = style
                .font
                .map(|f| f.line_height())
                .unwrap_or(crate::paint::font::GLYPH_H);
            Rect::new(x, y, w.max(1), h.max(1))
        }
        DocumentCanvasItem::Rect { x, y, w, h, .. } => Rect::new(x, y, w, h),
        DocumentCanvasItem::ImagePlaceholder { x, y, w, h, .. } => Rect::new(x, y, w, h),
        DocumentCanvasItem::Line { x1, y1, x2, y2, .. } => {
            let lx = x1.min(x2);
            let ly = y1.min(y2);
            let w = (x2 - x1).unsigned_abs().max(1);
            let h = (y2 - y1).unsigned_abs().max(1);
            Rect::new(lx, ly, w, h)
        }
    }
}

fn draw_line_clipped(
    canvas: &mut Canvas,
    bounds: Rect,
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
    style: DocumentStrokeStyle,
) {
    let mut x = x1;
    let mut y = y1;
    let dx = (x2 - x1).abs();
    let sx = if x1 <= x2 { 1 } else { -1 };
    let dy = -(y2 - y1).abs();
    let sy = if y1 <= y2 { 1 } else { -1 };
    let mut err = dx + dy;

    loop {
        let thickness = style.thickness.max(1) as i32;
        let half = thickness / 2;
        for py in 0..thickness {
            for px in 0..thickness {
                let tx = x + px - half;
                let ty = y + py - half;
                if bounds.contains(Point::new(tx, ty)) {
                    canvas.put_pixel(tx, ty, style.color);
                }
            }
        }

        if x == x2 && y == y2 {
            break;
        }
        let e2 = err * 2;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DocumentCanvasMode {
    Editable,
    ReadOnly,
}

impl Default for DocumentCanvasMode {
    fn default() -> Self {
        Self::Editable
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DocumentCanvasPresentation {
    Browser,
    Writer,
    Preview,
}

impl Default for DocumentCanvasPresentation {
    fn default() -> Self {
        Self::Preview
    }
}

#[derive(Clone, Copy)]
pub struct DocumentTextStyle<'a> {
    pub font: Option<&'a dyn VecText>,
    pub color: Color,
}

impl<'a> DocumentTextStyle<'a> {
    pub const fn new(font: Option<&'a dyn VecText>, color: Color) -> Self {
        Self { font, color }
    }
}

impl<'a> Default for DocumentTextStyle<'a> {
    fn default() -> Self {
        Self {
            font: None,
            color: Color::rgb(0x24, 0x24, 0x28),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DocumentStrokeStyle {
    pub color: Color,
    pub thickness: u32,
}

impl DocumentStrokeStyle {
    pub const fn new(color: Color, thickness: u32) -> Self {
        Self { color, thickness }
    }
}

impl Default for DocumentStrokeStyle {
    fn default() -> Self {
        Self {
            color: Color::rgb(0xD7, 0xD3, 0xCD),
            thickness: 1,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DocumentRectStyle {
    pub fill: Color,
    pub border: Option<DocumentStrokeStyle>,
}

impl DocumentRectStyle {
    pub const fn new(fill: Color, border: Option<DocumentStrokeStyle>) -> Self {
        Self { fill, border }
    }
}

impl Default for DocumentRectStyle {
    fn default() -> Self {
        Self {
            fill: Color::rgb(0xF9, 0xF7, 0xF2),
            border: Some(DocumentStrokeStyle::default()),
        }
    }
}

#[derive(Clone, Copy)]
pub enum DocumentCanvasItem<'a> {
    Text {
        x: i32,
        y: i32,
        text: &'a str,
        style: DocumentTextStyle<'a>,
    },
    Rect {
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        style: DocumentRectStyle,
    },
    Line {
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        style: DocumentStrokeStyle,
    },
    LinkText {
        x: i32,
        y: i32,
        text: &'a str,
        url: &'a str,
        style: DocumentTextStyle<'a>,
    },
    ImagePlaceholder {
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        label: &'a str,
    },
}

#[derive(Clone, Copy)]
pub struct DocumentCanvas<'a> {
    pub rect: Rect,
    pub mode: DocumentCanvasMode,
    pub empty_label: &'a str,
    pub presentation: DocumentCanvasPresentation,
    pub items: &'a [DocumentCanvasItem<'a>],
    /// Optional retained scene for generic read-only document rendering.
    /// Existing Writer callers can continue supplying `items` unchanged.
    pub scene: Option<&'a DocumentScene>,
    pub scroll_y: u32,
    pub body_font: Option<&'a dyn VecText>,
    pub small_font: Option<&'a dyn VecText>,
    pub scene_heading_font: Option<&'a dyn VecText>,
    pub scene_serif_font: Option<&'a dyn VecText>,
    pub scene_mono_font: Option<&'a dyn VecText>,
    pub edit_state: Option<&'a TextEditState>,
}

impl<'a> DocumentCanvas<'a> {
    pub fn new(rect: Rect, items: &'a [DocumentCanvasItem<'a>]) -> Self {
        Self {
            rect,
            mode: DocumentCanvasMode::Editable,
            empty_label: "Document Canvas Ready",
            presentation: DocumentCanvasPresentation::Preview,
            items,
            scene: None,
            scroll_y: 0,
            body_font: None,
            small_font: None,
            scene_heading_font: None,
            scene_serif_font: None,
            scene_mono_font: None,
            edit_state: None,
        }
    }

    pub fn with_edit_state(mut self, state: &'a TextEditState) -> Self {
        self.edit_state = Some(state);
        self
    }

    pub fn with_mode(mut self, mode: DocumentCanvasMode) -> Self {
        self.mode = mode;
        self
    }

    pub fn with_empty_label(mut self, empty_label: &'a str) -> Self {
        self.empty_label = empty_label;
        self
    }

    pub fn with_presentation(mut self, presentation: DocumentCanvasPresentation) -> Self {
        self.presentation = presentation;
        self.scroll_y = self.scene.map_or(self.scroll_y, |scene| {
            self.scroll_y.min(scene.max_scroll_y(self.content_rect().h))
        });
        self
    }

    pub fn with_scene(mut self, scene: &'a DocumentScene) -> Self {
        self.scene = Some(scene);
        self.mode = DocumentCanvasMode::ReadOnly;
        self.scroll_y = self.scroll_y.min(scene.max_scroll_y(self.content_rect().h));
        self
    }

    pub fn with_scroll_y(mut self, scroll_y: u32) -> Self {
        self.scroll_y = self.scene.map_or(scroll_y, |scene| {
            scroll_y.min(scene.max_scroll_y(self.content_rect().h))
        });
        self
    }
}

/// Semantically classified hit-test result for a document canvas.
///
/// Callers map this to the appropriate system cursor without needing
/// cursor-specific knowledge:
///
/// - [`CanvasHitTarget::None`] -> default pointer
/// - [`CanvasHitTarget::Text`] -> text I-beam
/// - [`CanvasHitTarget::Link`] -> hand / pointing hand
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CanvasHitTarget {
    None,
    Text,
    Link,
}

impl<'a> DocumentCanvas<'a> {

    /// Returns the retained object at a window-local point.  The widget owns
    /// the conversion from its clipped, scrolled surface into document
    /// coordinates; the caller remains responsible for interpreting any
    /// generic interaction metadata.
    pub fn hit_test(&self, point: Point) -> Option<&RenderObject> {
        let content = self.content_rect();
        if !content.contains(point) {
            return None;
        }
        let document_point = Point::new(
            point.x.saturating_sub(content.x),
            point
                .y
                .saturating_sub(content.y)
                .saturating_add(self.scroll_y as i32),
        );
        self.scene.and_then(|scene| scene.hit_test(document_point))
    }

    /// Returns the semantic hit-target under a window-local point.
    ///
    /// Supports both retained-scene (browser) and immediate-mode (Writer)
    /// canvases.  Link targets always take priority over text targets.
    pub fn hit_target(&self, point: Point) -> CanvasHitTarget {
        let content = self.content_rect();
        if !content.contains(point) {
            return CanvasHitTarget::None;
        }

        if let Some(scene) = self.scene {
            let document_point = Point::new(
                point.x.saturating_sub(content.x),
                point
                    .y
                    .saturating_sub(content.y)
                    .saturating_add(self.scroll_y as i32),
            );
            if let Some(obj) = scene.hit_test(document_point) {
                if matches!(
                    obj.interaction,
                    Some(RenderInteraction::Link { .. })
                ) {
                    return CanvasHitTarget::Link;
                }
                if matches!(obj.kind, RenderObjectKind::Text { .. }) {
                    return CanvasHitTarget::Text;
                }
            }
            return CanvasHitTarget::None;
        }

        let rel_x = point.x - content.x;
        let rel_y = point.y - content.y;
        let target_point = Point::new(rel_x, rel_y);

        for item in self.items.iter().rev() {
            let bounds = item_bounds(*item);
            if bounds.contains(target_point) {
                return match *item {
                    DocumentCanvasItem::LinkText { .. } => CanvasHitTarget::Link,
                    DocumentCanvasItem::Text { .. } => CanvasHitTarget::Text,
                    _ => continue,
                };
            }
        }
        CanvasHitTarget::None
    }

    pub fn with_fonts(
        mut self,
        _title_font: Option<&'a dyn VecText>,
        _subtitle_font: Option<&'a dyn VecText>,
        body_font: Option<&'a dyn VecText>,
        small_font: Option<&'a dyn VecText>,
    ) -> Self {
        self.body_font = body_font;
        self.small_font = small_font;
        self.scene_heading_font = _title_font;
        self
    }

    pub fn with_scene_font_families(
        mut self,
        serif_font: Option<&'a dyn VecText>,
        mono_font: Option<&'a dyn VecText>,
    ) -> Self {
        self.scene_serif_font = serif_font;
        self.scene_mono_font = mono_font;
        self
    }

    pub fn page_rect(&self) -> Rect {
        if self.presentation == DocumentCanvasPresentation::Browser {
            return self.rect;
        }
        let workspace = self.rect.inset(WRITER_WORKSPACE_GAP);
        let desired_w = 860u32.min(workspace.w);
        let x = workspace.x + ((workspace.w as i32 - desired_w as i32) / 2);
        Rect::new(x, workspace.y, desired_w, workspace.h)
    }

    pub fn content_rect(&self) -> Rect {
        if self.presentation == DocumentCanvasPresentation::Browser {
            self.rect
        } else {
            self.page_rect().inset(DOCUMENT_MARGIN)
        }
    }

    pub fn viewport_size(&self) -> Size {
        self.content_rect().size()
    }

    pub fn draw(&self, canvas: &mut Canvas, theme: &Theme) {
        if self.presentation != DocumentCanvasPresentation::Browser {
            let page = self.page_rect();
            canvas.fill_rect(page, Color::rgb(0xFB, 0xFA, 0xF7));
        }
        let content = self.content_rect();
        if let Some(scene) = self.scene {
            self.draw_scene(canvas, content, scene);
        } else if self.items.is_empty() {
            self.draw_empty_label(canvas, content, theme);
        } else {
            self.draw_items(canvas, content);
        }
    }

    fn draw_empty_label(&self, canvas: &mut Canvas, content: Rect, theme: &Theme) {
        let badge_w = measure_text_width(self.body_font, self.empty_label) + 28;
        let badge = Rect::new(
            content.x + ((content.w as i32 - badge_w as i32) / 2),
            content.y + (content.h as i32 / 2) - 18,
            badge_w,
            36,
        );
        canvas.fill_rounded_rect(badge, 18, theme.panel);
        canvas.stroke_rounded_rect(badge, 18, 1, theme.accent.darken(80));
        draw_text_vcenter(
            canvas,
            self.body_font,
            badge.x + 14,
            badge.y,
            badge.h,
            self.empty_label,
            theme.text,
        );
    }

    fn draw_items(&self, canvas: &mut Canvas, content: Rect) {
        let active_idx = self.edit_state.and_then(|es| es.active_item_index);
        for (idx, item) in self.items.iter().enumerate() {
            let is_active = active_idx == Some(idx);
            let caret_byte = if is_active {
                self.edit_state.map_or(0, |es| es.caret_byte)
            } else {
                0
            };
            match *item {
                DocumentCanvasItem::Text { x, y, text, style } => {
                    let px = content.x + x;
                    let py = content.y + y;
                    if py >= content.bottom() {
                        continue;
                    }
                    let max_w = (content.right() - px).max(0) as u32;
                    if is_active {
                        let line_h = style
                            .font
                            .map(|f| f.line_height())
                            .unwrap_or(crate::paint::font::GLYPH_H);
                        let lines = layout_text_lines(style.font, text, max_w, line_h);
                        for line in &lines {
                            let ly = py + line.y_offset;
                            if ly + line_h as i32 <= content.y || ly >= content.bottom() {
                                continue;
                            }
                            let mut line_text = &text[line.byte_start..line.byte_end];
                            if line.ends_with_newline && line_text.ends_with('\n') {
                                line_text = &line_text[..line_text.len() - 1];
                            }
                            let visible = clip_text_to_width(
                                style.font,
                                line_text,
                                max_w,
                            );
                            if !visible.is_empty() {
                                draw_text(canvas, style.font, px, ly, visible, style.color);
                            }
                        }
                        let caret_line =
                            find_line_index(&lines, caret_byte).unwrap_or(0usize);
                        if let Some(line) = lines.get(caret_line) {
                            let caret_ox =
                                caret_x_on_line(style.font, text, line, caret_byte);
                            let caret_px = px + caret_ox as i32;
                            let caret_py = py + line.y_offset;
                            if let Some(caret_rect) = Rect::new(
                                caret_px,
                                caret_py,
                                1,
                                line_h,
                            )
                            .intersect(content)
                            {
                                canvas.fill_rect(caret_rect, Color::rgb(0x25, 0x63, 0xEB));
                            }
                        }
                    } else {
                        if py < content.y {
                            continue;
                        }
                        let visible = clip_text_to_width(style.font, text, max_w);
                        if !visible.is_empty() {
                            draw_text(canvas, style.font, px, py, visible, style.color);
                        }
                    }
                }
                DocumentCanvasItem::Rect { x, y, w, h, style } => {
                    let rect = Rect::new(content.x + x, content.y + y, w, h);
                    if let Some(clipped) = rect.intersect(content) {
                        canvas.fill_rect(clipped, style.fill);
                        if let Some(border) = style.border {
                            canvas.draw_rect(clipped, border.color);
                        }
                    }
                }
                DocumentCanvasItem::Line {
                    x1,
                    y1,
                    x2,
                    y2,
                    style,
                } => {
                    draw_line_clipped(
                        canvas,
                        content,
                        content.x + x1,
                        content.y + y1,
                        content.x + x2,
                        content.y + y2,
                        style,
                    );
                }
                DocumentCanvasItem::LinkText {
                    x,
                    y,
                    text,
                    url,
                    style,
                } => {
                    let px = content.x + x;
                    let py = content.y + y;
                    if py < content.y || py >= content.bottom() {
                        continue;
                    }
                    let max_w = (content.right() - px).max(0) as u32;
                    let visible = clip_text_to_width(style.font, text, max_w);
                    if visible.is_empty() {
                        continue;
                    }
                    let color = if url.is_empty() {
                        style.color
                    } else {
                        style.color
                    };
                    draw_text(canvas, style.font, px, py, visible, color);
                    let underline_w = measure_text_width(style.font, visible);
                    let underline_y = py + crate::paint::font::GLYPH_H as i32 + 2;
                    if underline_y < content.bottom() {
                        canvas.hbar(px, underline_y, underline_w, 1, color.lighten(18));
                    }
                }
                DocumentCanvasItem::ImagePlaceholder { x, y, w, h, label } => {
                    let rect = Rect::new(content.x + x, content.y + y, w, h);
                    if let Some(clipped) = rect.intersect(content) {
                        canvas.fill_rect(clipped, Color::rgb(0xF6, 0xF3, 0xEE));
                        canvas.draw_rect(clipped, Color::rgb(0xD3, 0xCD, 0xC4));
                        draw_line_clipped(
                            canvas,
                            content,
                            rect.x + 8,
                            rect.y + 8,
                            rect.right() - 9,
                            rect.bottom() - 9,
                            DocumentStrokeStyle::new(Color::rgb(0xD8, 0xD1, 0xC8), 1),
                        );
                        draw_line_clipped(
                            canvas,
                            content,
                            rect.right() - 9,
                            rect.y + 8,
                            rect.x + 8,
                            rect.bottom() - 9,
                            DocumentStrokeStyle::new(Color::rgb(0xD8, 0xD1, 0xC8), 1),
                        );
                        let label_rect = Rect::new(
                            rect.x + 10,
                            rect.y + (rect.h as i32 / 2) - 10,
                            rect.w.saturating_sub(20),
                            20,
                        );
                        let visible =
                            clip_text_to_width(self.small_font, label, label_rect.w.max(1));
                        draw_text_vcenter(
                            canvas,
                            self.small_font,
                            label_rect.x,
                            label_rect.y,
                            label_rect.h,
                            visible,
                            Color::rgb(0x8B, 0x86, 0x80),
                        );
                    }
                }
            }
        }
    }

    fn draw_scene(&self, canvas: &mut Canvas, content: Rect, scene: &DocumentScene) {
        for object in &scene.objects {
            let bounds = object
                .bounds
                .translate(content.x, content.y - self.scroll_y as i32);
            let object_clip = object
                .clip_bounds
                .map(|clip| clip.translate(content.x, content.y - self.scroll_y as i32));
            let paint_bounds = match object_clip {
                Some(clip) => bounds.intersect(clip),
                None => Some(bounds),
            };
            let Some(clipped) = paint_bounds.and_then(|bounds| bounds.intersect(content)) else {
                continue;
            };
            match &object.kind {
                RenderObjectKind::Box => {}
                RenderObjectKind::Rectangle { fill } => canvas.fill_rect(clipped, *fill),
                RenderObjectKind::RoundedRectangle { fill, radii } => {
                    fill_corner_rounded(canvas, bounds, clipped, *radii, *fill, false);
                }
                RenderObjectKind::Border { color, width } => {
                    let width = (*width).max(1).min(bounds.w.min(bounds.h).max(1));
                    for offset in 0..width {
                        let inset = Rect::new(
                            bounds.x + offset as i32,
                            bounds.y + offset as i32,
                            bounds.w.saturating_sub(offset * 2),
                            bounds.h.saturating_sub(offset * 2),
                        );
                        if let Some(visible) = inset.intersect(content) {
                            canvas.draw_rect(visible, *color);
                        }
                    }
                }
                RenderObjectKind::BorderSides {
                    colors,
                    widths,
                    radii,
                } => {
                    paint_corner_border(canvas, bounds, clipped, *radii, *colors, *widths);
                }
                RenderObjectKind::BoxShadow {
                    box_bounds,
                    radii,
                    offset_x,
                    offset_y,
                    blur,
                    spread,
                    color,
                    inset,
                } => {
                    let base = box_bounds.translate(content.x, content.y - self.scroll_y as i32);
                    paint_box_shadow(
                        canvas, content, base, *radii, *offset_x, *offset_y, *blur, *spread,
                        *color, *inset,
                    );
                }
                RenderObjectKind::Line { color, width } => draw_line_clipped(
                    canvas,
                    content,
                    bounds.x,
                    bounds.y,
                    bounds.right().saturating_sub(1),
                    bounds.bottom().saturating_sub(1),
                    DocumentStrokeStyle::new(*color, *width),
                ),
                RenderObjectKind::Text {
                    text,
                    color,
                    font_size,
                    font_family,
                    bold,
                    italic,
                    underline,
                    line_through,
                    ..
                } => {
                    let font = match font_family {
                        DocumentFontFamily::Serif => self.scene_serif_font.or(self.body_font),
                        DocumentFontFamily::Monospace => self.scene_mono_font.or(self.body_font),
                        DocumentFontFamily::SansSerif if *font_size >= 24 => {
                            self.scene_heading_font.or(self.body_font)
                        }
                        DocumentFontFamily::SansSerif => self.body_font,
                    };
                    let visible = clip_text_to_width(font, text, clipped.w);
                    if !visible.is_empty() {
                        // The shared vector font currently has one face.  Keep the
                        // approximation here generic: a second one-pixel pass gives
                        // semantic bold useful weight, while a small x shift gives
                        // italic text a distinct, non-invasive fallback.
                        let text_x = bounds.x.saturating_add(*italic as i32);
                        draw_text(canvas, font, text_x, bounds.y, visible, *color);
                        if *bold {
                            draw_text(
                                canvas,
                                font,
                                text_x.saturating_add(1),
                                bounds.y,
                                visible,
                                *color,
                            );
                        }
                        if *underline {
                            let underline_y = bounds.y + bounds.h.saturating_sub(2) as i32;
                            if underline_y >= content.y && underline_y < content.bottom() {
                                canvas.hbar(
                                    bounds.x,
                                    underline_y,
                                    measure_text_width(font, visible),
                                    1,
                                    *color,
                                );
                            }
                        }
                        if *line_through {
                            let line_y = bounds.y + (bounds.h / 2) as i32;
                            if line_y >= content.y && line_y < content.bottom() {
                                canvas.hbar(
                                    bounds.x,
                                    line_y,
                                    measure_text_width(font, visible),
                                    1,
                                    *color,
                                );
                            }
                        }
                    }
                }
                RenderObjectKind::ImagePlaceholder { alt, .. } => {
                    canvas.fill_rect(clipped, Color::rgb(0xF6, 0xF3, 0xEE));
                    canvas.draw_rect(clipped, Color::rgb(0xD3, 0xCD, 0xC4));
                    let label = if alt.is_empty() {
                        "Image"
                    } else {
                        alt.as_str()
                    };
                    draw_text_vcenter(
                        canvas,
                        self.small_font,
                        clipped.x + 6,
                        clipped.y,
                        clipped.h,
                        clip_text_to_width(self.small_font, label, clipped.w.saturating_sub(12)),
                        Color::rgb(0x8B, 0x86, 0x80),
                    );
                }
                RenderObjectKind::Image { image, .. } => {
                    draw_raster_image(canvas, clipped, bounds, image);
                }
                RenderObjectKind::Control {
                    label,
                    placeholder,
                    value,
                    color,
                    background,
                    border_color,
                    border_width,
                    focused,
                    disabled,
                    editable: _,
                    caret_offset,
                    font_family,
                    kind,
                    ..
                } => {
                    canvas.fill_rect(clipped, *background);
                    let width = (*border_width).max(1).min(bounds.w.min(bounds.h).max(1));
                    for offset in 0..width {
                        let inset = Rect::new(
                            bounds.x + offset as i32,
                            bounds.y + offset as i32,
                            bounds.w.saturating_sub(offset * 2),
                            bounds.h.saturating_sub(offset * 2),
                        );
                        if let Some(visible) = inset.intersect(content) {
                            canvas.draw_rect(
                                visible,
                                if *focused {
                                    Color::rgb(0x3B, 0x82, 0xF6)
                                } else {
                                    *border_color
                                },
                            );
                        }
                    }
                    let text = if value.is_empty() { placeholder } else { value };
                    let text_color = if value.is_empty() {
                        Color::rgb(0x77, 0x77, 0x77)
                    } else if *disabled {
                        Color::rgb(0x77, 0x77, 0x77)
                    } else {
                        *color
                    };
                    let font = match font_family {
                        DocumentFontFamily::Serif => self.scene_serif_font.or(self.body_font),
                        DocumentFontFamily::Monospace => self.scene_mono_font.or(self.body_font),
                        DocumentFontFamily::SansSerif => self.body_font,
                    };
                    let text_rect = Rect::new(
                        bounds.x + width as i32 + 7,
                        bounds.y,
                        bounds.w.saturating_sub(width * 2 + 14),
                        bounds.h,
                    );
                    let visible = clip_text_to_width(font, text, text_rect.w);
                    if !visible.is_empty() {
                        draw_text_vcenter(
                            canvas,
                            font,
                            text_rect.x,
                            text_rect.y,
                            text_rect.h,
                            visible,
                            text_color,
                        );
                    }
                    if *focused {
                        if let Some(offset) = caret_offset {
                            let caret_x = text_rect.x + (*offset as i32).min(text_rect.w as i32);
                            if let Some(caret) = Rect::new(
                                caret_x,
                                text_rect.y + 3,
                                1,
                                text_rect.h.saturating_sub(6),
                            )
                            .intersect(content)
                            {
                                canvas.fill_rect(caret, Color::rgb(0x25, 0x63, 0xEB));
                            }
                        }
                    }
                    if !label.is_empty() {
                        let label = clip_text_to_width(font, label, text_rect.w);
                        let label_w = measure_text_width(font, label) as i32;
                        let label_x = if *kind == 1 {
                            bounds.x + (bounds.w as i32 - label_w).max(0) / 2
                        } else {
                            text_rect.x
                        };
                        draw_text_vcenter(
                            canvas,
                            font,
                            label_x,
                            text_rect.y,
                            text_rect.h,
                            label,
                            text_color,
                        );
                    }
                }
                // A link has interaction metadata but no extra paint.  Its
                // text fragments contain the resolved visual decoration.
                RenderObjectKind::Link { .. } => {}
            }
        }
    }
}

fn corner_contains(rect: Rect, mut radii: CornerRadii, x: i32, y: i32) -> bool {
    if !rect.contains(Point::new(x, y)) {
        return false;
    }
    let limit = rect.w.min(rect.h) / 2;
    radii.top_left = radii.top_left.min(limit);
    radii.top_right = radii.top_right.min(limit);
    radii.bottom_right = radii.bottom_right.min(limit);
    radii.bottom_left = radii.bottom_left.min(limit);
    let tests = [
        (
            rect.x + radii.top_left as i32,
            rect.y + radii.top_left as i32,
            radii.top_left,
            x < rect.x + radii.top_left as i32 && y < rect.y + radii.top_left as i32,
        ),
        (
            rect.right() - radii.top_right as i32 - 1,
            rect.y + radii.top_right as i32,
            radii.top_right,
            x >= rect.right() - radii.top_right as i32 && y < rect.y + radii.top_right as i32,
        ),
        (
            rect.right() - radii.bottom_right as i32 - 1,
            rect.bottom() - radii.bottom_right as i32 - 1,
            radii.bottom_right,
            x >= rect.right() - radii.bottom_right as i32
                && y >= rect.bottom() - radii.bottom_right as i32,
        ),
        (
            rect.x + radii.bottom_left as i32,
            rect.bottom() - radii.bottom_left as i32 - 1,
            radii.bottom_left,
            x < rect.x + radii.bottom_left as i32 && y >= rect.bottom() - radii.bottom_left as i32,
        ),
    ];
    for (cx, cy, radius, applies) in tests {
        if applies && radius > 0 {
            let dx = x - cx;
            let dy = y - cy;
            return dx * dx + dy * dy <= (radius as i32 * radius as i32);
        }
    }
    true
}

fn fill_corner_rounded(
    canvas: &mut Canvas,
    rect: Rect,
    clip: Rect,
    radii: CornerRadii,
    color: Color,
    blend: bool,
) {
    let Some(area) = rect.intersect(clip) else {
        return;
    };
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            if corner_contains(rect, radii, x, y) {
                if blend {
                    canvas.blend_pixel(x, y, color);
                } else {
                    canvas.put_pixel(x, y, color);
                }
            }
        }
    }
}

fn paint_corner_border(
    canvas: &mut Canvas,
    rect: Rect,
    clip: Rect,
    radii: CornerRadii,
    colors: [Color; 4],
    widths: [u32; 4],
) {
    let Some(area) = rect.intersect(clip) else {
        return;
    };
    let inner = Rect::new(
        rect.x + widths[3] as i32,
        rect.y + widths[0] as i32,
        rect.w.saturating_sub(widths[1] + widths[3]),
        rect.h.saturating_sub(widths[0] + widths[2]),
    );
    let inner_radii = CornerRadii {
        top_left: radii.top_left.saturating_sub(widths[0].max(widths[3])),
        top_right: radii.top_right.saturating_sub(widths[0].max(widths[1])),
        bottom_right: radii.bottom_right.saturating_sub(widths[2].max(widths[1])),
        bottom_left: radii.bottom_left.saturating_sub(widths[2].max(widths[3])),
    };
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            if !corner_contains(rect, radii, x, y) || corner_contains(inner, inner_radii, x, y) {
                continue;
            }
            let distances = [
                (y - rect.y).max(0) as u32,
                (rect.right() - 1 - x).max(0) as u32,
                (rect.bottom() - 1 - y).max(0) as u32,
                (x - rect.x).max(0) as u32,
            ];
            let side = distances
                .iter()
                .enumerate()
                .min_by_key(|(_, d)| *d)
                .map(|(i, _)| i)
                .unwrap_or(0);
            canvas.put_pixel(x, y, colors[side]);
        }
    }
}

fn paint_box_shadow(
    canvas: &mut Canvas,
    clip: Rect,
    base: Rect,
    radii: CornerRadii,
    offset_x: i32,
    offset_y: i32,
    blur: u32,
    spread: i32,
    color: Color,
    inset: bool,
) {
    const MAX_SHADOW_PIXEL_AREA: u64 = 2_000_000;
    let blur = blur.min(32);
    let extent = blur.saturating_add(spread.max(0) as u32);
    if base.w.saturating_add(extent * 2) as u64 * base.h.saturating_add(extent * 2) as u64
        > MAX_SHADOW_PIXEL_AREA
    {
        return;
    }
    if inset {
        let Some(area) = base.intersect(clip) else {
            return;
        };
        let extent = blur.max(1) as i32;
        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                if !corner_contains(base, radii, x, y) {
                    continue;
                }
                let d = (x - base.x + offset_x)
                    .min(base.right() - 1 - x - offset_x)
                    .min(y - base.y + offset_y)
                    .min(base.bottom() - 1 - y - offset_y)
                    .max(0);
                if d < extent {
                    let alpha =
                        (color.a() as u32 * (extent - d) as u32 / extent as u32).min(255) as u8;
                    canvas.blend_pixel(x, y, Color::rgba(color.r(), color.g(), color.b(), alpha));
                }
            }
        }
        return;
    }
    let spread_rect = Rect::new(
        base.x + offset_x - spread,
        base.y + offset_y - spread,
        base.w.saturating_add((spread.max(0) as u32) * 2),
        base.h.saturating_add((spread.max(0) as u32) * 2),
    );
    for ring in (0..=blur).rev() {
        let alpha = if blur == 0 {
            color.a()
        } else {
            (color.a() as u32 / (blur + 2)).max(1) as u8
        };
        let rect = Rect::new(
            spread_rect.x - ring as i32,
            spread_rect.y - ring as i32,
            spread_rect.w.saturating_add(ring * 2),
            spread_rect.h.saturating_add(ring * 2),
        );
        let expanded = CornerRadii {
            top_left: radii.top_left + ring,
            top_right: radii.top_right + ring,
            bottom_right: radii.bottom_right + ring,
            bottom_left: radii.bottom_left + ring,
        };
        fill_corner_rounded(
            canvas,
            rect,
            clip,
            expanded,
            Color::rgba(color.r(), color.g(), color.b(), alpha),
            true,
        );
    }
}

fn draw_raster_image(canvas: &mut Canvas, clipped: Rect, destination: Rect, image: &RasterImage) {
    if image.width == 0
        || image.height == 0
        || image.pixels.len() != (image.width as usize).saturating_mul(image.height as usize)
    {
        return;
    }
    for destination_y in clipped.y..clipped.bottom() {
        let relative_y = destination_y.saturating_sub(destination.y) as u32;
        let source_y = relative_y.saturating_mul(image.height) / destination.h.max(1);
        for destination_x in clipped.x..clipped.right() {
            let relative_x = destination_x.saturating_sub(destination.x) as u32;
            let source_x = relative_x.saturating_mul(image.width) / destination.w.max(1);
            let source_index = source_y as usize * image.width as usize + source_x as usize;
            let source = image.pixels[source_index];
            let alpha = (source >> 24) as u8;
            if alpha == 0 {
                continue;
            }
            let destination_index =
                destination_y as usize * canvas.stride as usize + destination_x as usize;
            if destination_index >= canvas.pixels.len() {
                continue;
            }
            let rgb = source & 0x00ff_ffff;
            if alpha == 255 {
                canvas.pixels[destination_index] = rgb;
                continue;
            }
            let existing = canvas.pixels[destination_index];
            let source_red = (rgb >> 16) & 0xff;
            let source_green = (rgb >> 8) & 0xff;
            let source_blue = rgb & 0xff;
            let destination_red = (existing >> 16) & 0xff;
            let destination_green = (existing >> 8) & 0xff;
            let destination_blue = existing & 0xff;
            let alpha = alpha as u32;
            let inverse_alpha = 255 - alpha;
            canvas.pixels[destination_index] =
                (((source_red * alpha + destination_red * inverse_alpha) >> 8) << 16)
                    | (((source_green * alpha + destination_green * inverse_alpha) >> 8) << 8)
                    | ((source_blue * alpha + destination_blue * inverse_alpha) >> 8);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        byte_at_x_on_line, byte_offset_at_x, caret_x_at_byte, caret_x_on_line,
        click_to_line_and_byte, diff_scenes, find_line_index, layout_text_lines, line_end_byte,
        line_home_byte, DocumentCanvas, DocumentCanvasItem, DocumentCanvasMode,
        DocumentCanvasPresentation, DocumentFontFamily, DocumentNodeId, DocumentScene,
        DocumentStrokeStyle, DocumentTextStyle, PaintOrder, RasterImage, RenderObject,
        RenderObjectId, RenderObjectKind, ScenePatchOperation, TextEditState, TextLineLayout,
    };
    use crate::{Canvas, Color, Point, Rect, Size, Theme};
    #[cfg(feature = "app")]
    use crate::CursorShape;
    use alloc::sync::Arc;

    fn scene_with_text(text: &str, x: i32) -> DocumentScene {
        let mut scene = DocumentScene::new(7, Size::new(200, 100), Size::new(200, 100));
        assert!(scene.push(RenderObject {
            id: RenderObjectId(1),
            owner_node_id: DocumentNodeId(3),
            kind: RenderObjectKind::Text {
                text: text.into(),
                color: Color::rgb(1, 2, 3),
                font_size: 16,
                bold: false,
                italic: false,
                underline: false,
                line_through: false,
                monospace: false,
                font_family: DocumentFontFamily::SansSerif,
            },
            bounds: Rect::new(x, 4, 32, 16),
            clip_bounds: None,
            paint_order: PaintOrder::default(),
            interaction: None,
        }));
        scene.finalize();
        scene
    }

    #[test]
    fn css_generic_font_lists_resolve_without_html_knowledge() {
        assert_eq!(
            DocumentFontFamily::resolve_css_list("serif"),
            DocumentFontFamily::Serif
        );
        assert_eq!(
            DocumentFontFamily::resolve_css_list("Georgia, 'Times New Roman', serif"),
            DocumentFontFamily::Serif
        );
        assert_eq!(
            DocumentFontFamily::resolve_css_list("Times New Roman"),
            DocumentFontFamily::Serif
        );
        assert_eq!(
            DocumentFontFamily::resolve_css_list("Arial, sans-serif"),
            DocumentFontFamily::SansSerif
        );
        assert_eq!(
            DocumentFontFamily::resolve_css_list("Helvetica"),
            DocumentFontFamily::SansSerif
        );
        assert_eq!(
            DocumentFontFamily::resolve_css_list("monospace"),
            DocumentFontFamily::Monospace
        );
        assert_eq!(
            DocumentFontFamily::resolve_css_list("Fira Code"),
            DocumentFontFamily::Monospace
        );
    }

    #[test]
    fn mode_defaults_to_editable() {
        assert_eq!(DocumentCanvasMode::default(), DocumentCanvasMode::Editable);
    }

    #[test]
    fn content_rect_stays_within_page() {
        let widget = DocumentCanvas::new(Rect::new(0, 0, 1240, 680), &[])
            .with_presentation(DocumentCanvasPresentation::Writer);
        assert!(widget
            .page_rect()
            .intersect(widget.content_rect())
            .is_some());
        assert!(widget.content_rect().right() <= widget.page_rect().right());
        assert!(widget.content_rect().bottom() <= widget.page_rect().bottom());
    }

    #[test]
    fn browser_presentation_uses_the_entire_canvas_as_document_viewport() {
        let rect = Rect::new(12, 24, 800, 500);
        let widget =
            DocumentCanvas::new(rect, &[]).with_presentation(DocumentCanvasPresentation::Browser);
        assert_eq!(widget.content_rect(), rect);
        assert_eq!(widget.page_rect(), rect);
        assert_eq!(widget.viewport_size(), rect.size());
    }

    #[test]
    fn writer_presentation_keeps_only_page_workspace_and_document_margin() {
        let rect = Rect::new(0, 100, 1_200, 600);
        let widget =
            DocumentCanvas::new(rect, &[]).with_presentation(DocumentCanvasPresentation::Writer);
        assert_eq!(widget.content_rect().y, rect.y + 16 + 28);
        assert_eq!(widget.content_rect().h, rect.h - 32 - 56);
        assert!(widget.content_rect().x > rect.x);
        assert!(widget.content_rect().right() < rect.right());
    }

    #[test]
    fn empty_document_draw_does_not_panic() {
        let mut pixels = [0u32; 320 * 240];
        let mut canvas = Canvas::new(&mut pixels, 320, 320, 240);
        let widget = DocumentCanvas::new(Rect::new(0, 0, 320, 240), &[])
            .with_mode(DocumentCanvasMode::ReadOnly);
        widget.draw(&mut canvas, &Theme::sunlight_dark());
    }

    #[test]
    fn retained_scene_selects_read_only_mode_and_hit_tests_in_document_coordinates() {
        let mut scene = scene_with_text("link", 8);
        scene.content_size = Size::new(200, 1_000);
        let widget = DocumentCanvas::new(Rect::new(0, 0, 640, 480), &[])
            .with_scene(&scene)
            .with_scroll_y(4);
        assert_eq!(widget.mode, DocumentCanvasMode::ReadOnly);

        let content = widget.content_rect();
        let point = Point::new(content.x + 10, content.y);
        assert_eq!(
            widget.hit_test(point).map(|object| object.id),
            Some(RenderObjectId(1))
        );
    }

    #[test]
    fn browser_hit_testing_starts_at_viewport_origin_and_scrolls_in_document_space() {
        let mut scene = scene_with_text("link", 8);
        scene.content_size = Size::new(200, 1_000);
        let rect = Rect::new(40, 80, 640, 480);
        let widget = DocumentCanvas::new(rect, &[])
            .with_presentation(DocumentCanvasPresentation::Browser)
            .with_scene(&scene)
            .with_scroll_y(4);
        assert_eq!(
            widget.hit_test(Point::new(rect.x + 8, rect.y)),
            scene.objects.first()
        );
        assert_eq!(widget.hit_test(Point::new(rect.x + 8, rect.y - 1)), None);
    }

    #[test]
    fn browser_scene_draws_decoded_raster_image() {
        let mut scene = DocumentScene::new(7, Size::new(2, 2), Size::new(2, 2));
        assert!(scene.push(RenderObject {
            id: RenderObjectId(2),
            owner_node_id: DocumentNodeId(1),
            kind: RenderObjectKind::Image {
                image: Arc::new(RasterImage {
                    width: 1,
                    height: 1,
                    pixels: vec![0xFFFF0000],
                }),
                source_url: String::from("https://example.com/red.png"),
                intrinsic_width: 1,
                intrinsic_height: 1,
                alt: String::new(),
            },
            bounds: Rect::new(0, 0, 2, 2),
            clip_bounds: None,
            paint_order: PaintOrder::default(),
            interaction: None,
        }));
        scene.finalize();
        let widget = DocumentCanvas::new(Rect::new(0, 0, 2, 2), &[])
            .with_scene(&scene)
            .with_presentation(DocumentCanvasPresentation::Browser);
        let mut pixels = [0u32; 4];
        let mut canvas = Canvas::new(&mut pixels, 2, 2, 2);
        widget.draw(&mut canvas, &Theme::sunlight_dark());
        assert_eq!(pixels[0], 0x00FF0000);
        assert_eq!(pixels[3], 0x00FF0000);
    }

    #[test]
    fn sample_items_can_be_constructed() {
        let items = [
            DocumentCanvasItem::Text {
                x: 0,
                y: 0,
                text: "Sample",
                style: DocumentTextStyle::default(),
            },
            DocumentCanvasItem::Line {
                x1: 0,
                y1: 12,
                x2: 60,
                y2: 12,
                style: DocumentStrokeStyle::new(Color::rgb(1, 2, 3), 1),
            },
        ];
        let widget = DocumentCanvas::new(Rect::new(0, 0, 640, 480), &items);
        assert_eq!(widget.items.len(), 2);
    }

    #[test]
    fn retained_scene_indexes_and_diff_are_stable() {
        let old = scene_with_text("one", 4);
        assert_eq!(
            old.objects_for_node(DocumentNodeId(3)),
            &[RenderObjectId(1)]
        );
        assert_eq!(
            old.object(RenderObjectId(1)).unwrap().owner_node_id,
            DocumentNodeId(3)
        );
        assert!(diff_scenes(&old, &old).is_empty());

        let text_changed = scene_with_text("two", 4);
        let patch = diff_scenes(&old, &text_changed);
        assert!(matches!(
            patch.operations.as_slice(),
            [ScenePatchOperation::Update { .. }]
        ));

        let moved = scene_with_text("one", 24);
        let patch = diff_scenes(&old, &moved);
        assert!(matches!(
            patch.operations.as_slice(),
            [ScenePatchOperation::Update { .. }]
        ));
        assert_eq!(patch.dirty_regions[0], Rect::new(4, 4, 52, 16));
    }

    #[test]
    fn retained_scene_applies_insert_remove_update_and_reorder() {
        let mut current = scene_with_text("one", 4);

        let inserted = scene_with_text("one", 4);
        let mut next = inserted.clone();
        assert!(next.push(RenderObject {
            id: RenderObjectId(2),
            owner_node_id: DocumentNodeId(4),
            kind: RenderObjectKind::Rectangle {
                fill: Color::rgb(4, 5, 6),
            },
            bounds: Rect::new(40, 4, 20, 16),
            clip_bounds: None,
            paint_order: PaintOrder {
                phase: 1,
                ..PaintOrder::default()
            },
            interaction: None,
        }));
        next.finalize();
        let patch = diff_scenes(&current, &next);
        assert!(patch
            .operations
            .iter()
            .any(|operation| matches!(operation, ScenePatchOperation::Insert { .. })));
        assert!(current.apply_patch(&patch));
        assert_eq!(current.objects, next.objects);
        assert_eq!(
            current.objects_for_node(DocumentNodeId(4)),
            &[RenderObjectId(2)]
        );

        let mut reordered = current.clone();
        reordered.objects[0].paint_order.phase = 9;
        reordered.finalize();
        let patch = diff_scenes(&current, &reordered);
        assert!(matches!(
            patch.operations.as_slice(),
            [ScenePatchOperation::Reorder { .. }]
        ));
        assert!(current.apply_patch(&patch));
        assert_eq!(current.objects, reordered.objects);

        let empty = DocumentScene::new(7, Size::new(200, 100), Size::new(200, 100));
        let patch = diff_scenes(&current, &empty);
        assert!(patch
            .operations
            .iter()
            .all(|operation| matches!(operation, ScenePatchOperation::Remove { .. })));
        assert!(current.apply_patch(&patch));
        assert!(current.objects.is_empty());
    }

    #[test]
    fn replacement_patch_requires_the_producer_to_swap_the_scene() {
        let mut old = scene_with_text("old", 4);
        let next = DocumentScene::new(8, Size::new(200, 100), Size::new(200, 100));
        let patch = diff_scenes(&old, &next);
        assert!(matches!(
            patch.operations.as_slice(),
            [ScenePatchOperation::ReplaceScene]
        ));
        assert!(!old.apply_patch(&patch));
        assert_eq!(old.objects[0].id, RenderObjectId(1));
    }

    // ── Editing / caret tests ──────────────────────────────────────────────

    #[test]
    fn text_edit_state_defaults_to_inactive() {
        let state = TextEditState::default();
        assert!(!state.is_editing());
        assert_eq!(state.active_item_index, None);
        assert_eq!(state.caret_byte, 0);
        assert_eq!(state.selection_anchor_byte, None);
    }

    #[test]
    fn text_edit_state_clear_resets_all_fields() {
        let mut state = TextEditState {
            active_item_index: Some(2),
            caret_byte: 5,
            selection_anchor_byte: Some(3),
            preferred_caret_x: Some(42),
        };
        state.clear();
        assert!(!state.is_editing());
        assert_eq!(state.caret_byte, 0);
        assert_eq!(state.selection_anchor_byte, None);
        assert_eq!(state.preferred_caret_x, None);
    }

    #[test]
    fn caret_x_at_byte_empty_text() {
        assert_eq!(caret_x_at_byte(None, "", 0), 0);
        assert_eq!(caret_x_at_byte(None, "", 5), 0);
    }

    #[test]
    fn caret_x_at_byte_ascii() {
        // Built-in font: each char = GLYPH_W(5) + 1 = 6 px
        assert_eq!(caret_x_at_byte(None, "abc", 0), 0);
        assert_eq!(caret_x_at_byte(None, "abc", 1), 6);
        assert_eq!(caret_x_at_byte(None, "abc", 2), 12);
        assert_eq!(caret_x_at_byte(None, "abc", 3), 18);
        assert_eq!(caret_x_at_byte(None, "abc", 99), 18); // clamped
    }

    #[test]
    fn byte_offset_at_x_empty_text() {
        assert_eq!(byte_offset_at_x(None, "", 0), 0);
        assert_eq!(byte_offset_at_x(None, "", 10), 0);
    }

    #[test]
    fn byte_offset_at_x_ascii_fixed_width() {
        // Built-in font: each char = 6 px. Midpoints: 3, 9, 15, 21, 27
        assert_eq!(byte_offset_at_x(None, "hello", 0), 0);
        assert_eq!(byte_offset_at_x(None, "hello", 1), 0);
        assert_eq!(byte_offset_at_x(None, "hello", 3), 0); // exactly at mid of h
        assert_eq!(byte_offset_at_x(None, "hello", 4), 1); // past mid → after h
        assert_eq!(byte_offset_at_x(None, "hello", 6), 1); // at boundary
        assert_eq!(byte_offset_at_x(None, "hello", 9), 1); // exactly at mid of e
        assert_eq!(byte_offset_at_x(None, "hello", 10), 2); // past mid → after e
        assert_eq!(byte_offset_at_x(None, "hello", 30), 5); // end
        assert_eq!(byte_offset_at_x(None, "hello", 999), 5); // past end
    }

    #[test]
    fn caret_roundtrip_ascii() {
        let text = "Hello World";
        for byte_offset in 0..=text.len() {
            let x = caret_x_at_byte(None, text, byte_offset) as i32;
            let roundtrip = byte_offset_at_x(None, text, x);
            // Should at minimum not go backwards
            assert!(roundtrip <= byte_offset);
            // And the x at this roundtrip should be close
            let rx = caret_x_at_byte(None, text, roundtrip) as i32;
            assert!(rx <= x + 1);
        }
    }

    #[test]
    fn byte_offset_at_x_always_returns_code_point_boundary() {
        // Even with negative or huge x, we get a valid boundary
        for text in ["", "a", "abc", "Hello", ""] {
            for x in [-5, 0, 1, 10, 100, 1000] {
                let offset = byte_offset_at_x(None, text, x);
                assert!(text.is_char_boundary(offset));
            }
        }
    }

    #[test]
    fn draw_with_edit_state_renders_caret_for_active_text_item() {
        let items = [
            DocumentCanvasItem::Text {
                x: 0,
                y: 0,
                text: "AB",
                style: DocumentTextStyle::default(),
            },
            DocumentCanvasItem::Text {
                x: 0,
                y: 20,
                text: "CD",
                style: DocumentTextStyle::default(),
            },
        ];
        let edit_state = TextEditState {
            active_item_index: Some(1),
            caret_byte: 1,
            selection_anchor_byte: None,
            preferred_caret_x: None,
        };
        let widget = DocumentCanvas::new(Rect::new(0, 0, 320, 240), &items)
            .with_edit_state(&edit_state);
        let mut pixels = [0u32; 320 * 240];
        let mut canvas = Canvas::new(&mut pixels, 320, 320, 240);
        widget.draw(&mut canvas, &Theme::sunlight_dark());
        // Caret is drawn at (content.x + item.x + caret_ox, content.y + item.y)
        // item 1: x=0, y=20, text="CD", caret_byte=1 → caret_ox = 6 (after 'C')
        // content_rect: inset(16)_inset(28) = (44,44,232,152)
        // caret pixel: x=44+0+6=50, y=44+20=64..70
        let caret_x = widget.content_rect().x + 0 + 6;
        let caret_y = widget.content_rect().y + 20 + 2;
        let stride = 320usize;
        let idx = caret_y as usize * stride + caret_x as usize;
        assert_eq!(pixels[idx], Color::rgb(0x25, 0x63, 0xEB).0);
    }

    #[test]
    fn draw_without_edit_state_does_not_render_caret() {
        let items = [DocumentCanvasItem::Text {
            x: 0,
            y: 0,
            text: "AB",
            style: DocumentTextStyle::default(),
        }];
        let widget = DocumentCanvas::new(Rect::new(0, 0, 320, 240), &items);
        let mut pixels = [0u32; 320 * 240];
        let mut canvas = Canvas::new(&mut pixels, 320, 320, 240);
        widget.draw(&mut canvas, &Theme::sunlight_dark());
        // No blue (caret) pixel should be present — the background
        // is filled, text is drawn, but no 0x2563EB pixel
        assert!(!pixels
            .iter()
            .any(|&pixel| pixel == Color::rgb(0x25, 0x63, 0xEB).0));
    }

    // ── Multiline layout tests ─────────────────────────────────────────────

    #[test]
    fn layout_empty_text_is_single_empty_line() {
        let lines = layout_text_lines(None, "", 60, 7);
        assert!(lines.is_empty());
    }

    #[test]
    fn layout_explicit_newlines() {
        // Built-in font: 6px per char. "ab\ncd" -> 4 chars across 2 lines
        let lines = layout_text_lines(None, "ab\ncd", 600, 7);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].byte_start, 0);
        assert_eq!(lines[0].byte_end, 3); // "ab\n"
        assert!(lines[0].ends_with_newline);
        assert_eq!(lines[1].byte_start, 3);
        assert_eq!(lines[1].byte_end, 5); // "cd"
        assert!(!lines[1].ends_with_newline);
    }

    #[test]
    fn layout_newline_at_end_creates_trailing_line() {
        // "ab\n" is 3 bytes.  The '\n' is included in the one visual line.
        let lines = layout_text_lines(None, "ab\n", 600, 7);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].byte_end, 3); // "ab\n"
        assert!(lines[0].ends_with_newline);
        // The caret can sit at byte 3 (past '\n'), which maps to line 0.
        assert_eq!(find_line_index(&lines, 3), Some(0));
    }

    #[test]
    fn layout_soft_wrap_at_space() {
        // Built-in font: 6px per char. "hello world" = 11 chars = 66px
        // max_width = 36px, so "hello " at most fits, "world" wraps
        let lines = layout_text_lines(None, "hello world", 36, 7);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].byte_start, 0);
        assert_eq!(lines[1].byte_start, 6); // after "hello "
        assert!(!lines[0].ends_with_newline);
    }

    #[test]
    fn layout_no_space_fallback_wrap() {
        // "abcdef" = 6 chars = 36px. max_width=20 forces mid-word break
        let lines = layout_text_lines(None, "abcdef", 20, 7);
        assert!(lines.len() >= 2);
        assert_eq!(lines[0].byte_start, 0);
        assert!(!lines[0].ends_with_newline);
        // All lines combined cover the full text
        let last = lines.last().unwrap();
        assert_eq!(last.byte_end, "abcdef".len());
    }

    #[test]
    fn find_line_for_byte() {
        let lines = layout_text_lines(None, "ab\ncd", 600, 7);
        assert_eq!(find_line_index(&lines, 0), Some(0)); // 'a'
        assert_eq!(find_line_index(&lines, 2), Some(0)); // '\n'
        assert_eq!(find_line_index(&lines, 3), Some(1)); // 'c'
        assert_eq!(find_line_index(&lines, 5), Some(1)); // end
        assert_eq!(find_line_index(&lines, 999), Some(1)); // past end
    }

    #[test]
    fn line_home_and_end_bytes() {
        let lines = layout_text_lines(None, "one\ntwo\nthree", 600, 7);
        // Line 0: "one\n" bytes 0..4. Home=0, End=3 (skip '\n')
        assert_eq!(line_home_byte(&lines, 0), 0);
        assert_eq!(line_end_byte(&lines, 0), 3);
        // Line 1: "two\n" bytes 4..8. Home=4, End=7
        assert_eq!(line_home_byte(&lines, 1), 4);
        assert_eq!(line_end_byte(&lines, 1), 7);
        // Line 2: "three" bytes 8..13. Home=8, End=13 (no '\n' to skip)
        assert_eq!(line_home_byte(&lines, 2), 8);
        assert_eq!(line_end_byte(&lines, 2), 13);
    }

    #[test]
    fn caret_x_on_line_correct() {
        let lines = layout_text_lines(None, "abc", 600, 7);
        let line = &lines[0];
        // "abc": each char 6px
        assert_eq!(caret_x_on_line(None, "abc", line, 0), 0);
        assert_eq!(caret_x_on_line(None, "abc", line, 1), 6);
        assert_eq!(caret_x_on_line(None, "abc", line, 3), 18);
    }

    #[test]
    fn byte_at_x_on_line_snaps_to_boundaries() {
        let lines = layout_text_lines(None, "xyz", 600, 7);
        let line = &lines[0];
        assert_eq!(byte_at_x_on_line(None, "xyz", line, 0), 0);
        assert_eq!(byte_at_x_on_line(None, "xyz", line, 4), 1); // past mid of x
        assert_eq!(byte_at_x_on_line(None, "xyz", line, 10), 2); // past mid of y
    }

    #[test]
    fn click_to_line_and_byte_multiline() {
        let lines = layout_text_lines(None, "ab\ncd", 600, 7);
        // Line 0 at y=0, line 1 at y=7. item_x=10, item_y=100
        // Click at (x=16, y=102) — on line 0, x offset 6 → byte 1 ('b')
        let result = click_to_line_and_byte(None, "ab\ncd", &lines, 100, 102, 16, 10);
        assert_eq!(result, Some((0, 1)));
        // Click at (x=10, y=108) — on line 1, x offset 0 → byte 3 (start of "cd")
        let result2 = click_to_line_and_byte(None, "ab\ncd", &lines, 100, 108, 10, 10);
        assert_eq!(result2, Some((1, 3)));
    }

    #[test]
    fn draw_multiline_active_text_renders_lines_and_caret() {
        let items = [DocumentCanvasItem::Text {
            x: 0,
            y: 0,
            text: "AB\nCD",
            style: DocumentTextStyle::default(),
        }];
        let edit_state = TextEditState {
            active_item_index: Some(0),
            caret_byte: 4, // after '\n' → second line, byte offset 4 ("CD"[0+3] → byte 0 + ('\n' at 3) = 4
            selection_anchor_byte: None,
            preferred_caret_x: None,
        };
        let widget = DocumentCanvas::new(Rect::new(0, 0, 320, 240), &items)
            .with_edit_state(&edit_state);
        let mut pixels = [0u32; 320 * 240];
        let mut canvas = Canvas::new(&mut pixels, 320, 320, 240);
        widget.draw(&mut canvas, &Theme::sunlight_dark());
        // Caret on line 1: line_h=7, y_offset=7, content.y=44
        // caret_byte=4, second line text="CD", caret_ox=caret_x_at_byte(None,"CD",1)=6
        // caret_px=44+6=50, caret_py=44+7=51
        let content = widget.content_rect();
        let caret_y = content.y + 7 + 1; // second line middle
        let stride = 320usize;
        let idx = caret_y as usize * stride + (content.x + 6) as usize;
        assert_eq!(pixels[idx], Color::rgb(0x25, 0x63, 0xEB).0);
    }

    #[test]
    fn layout_wrap_preserves_emoji_unicode_boundaries() {
        // Mixed ASCII and emoji — must not panic and must split at valid boundaries
        let text = "SunlightOS ☀️  Rabbit 🐇  Penguin 🐧  Rust 🦀";
        let lines = layout_text_lines(None, text, 80, 7);
        assert!(!lines.is_empty());
        for line in &lines {
            assert!(text.is_char_boundary(line.byte_start));
            assert!(text.is_char_boundary(line.byte_end));
        }
    }

    // ------------------------------------------------------------------
    // hit_target / canvas-hit-target tests
    // ------------------------------------------------------------------

    #[test]
    fn hit_target_returns_none_for_empty_canvas() {
        let items: &[DocumentCanvasItem] = &[];
        let canvas =
            DocumentCanvas::new(Rect::new(0, 0, 400, 300), items)
                .with_mode(DocumentCanvasMode::Editable)
                .with_presentation(DocumentCanvasPresentation::Writer);
        let content = canvas.content_rect();
        let pt = Point::new(content.x + 10, content.y + 10);
        assert_eq!(canvas.hit_target(pt), super::CanvasHitTarget::None);
    }

    #[test]
    fn hit_target_returns_text_for_text_item() {
        let items = &[DocumentCanvasItem::Text {
            x: 10,
            y: 10,
            text: "Hello",
            style: DocumentTextStyle::default(),
        }];
        let canvas =
            DocumentCanvas::new(Rect::new(0, 0, 400, 300), items)
                .with_mode(DocumentCanvasMode::Editable)
                .with_presentation(DocumentCanvasPresentation::Writer);
        let content = canvas.content_rect();
        let pt = Point::new(content.x + 15, content.y + 13);
        assert_eq!(canvas.hit_target(pt), super::CanvasHitTarget::Text);
    }

    #[test]
    fn hit_target_returns_link_for_link_text_item() {
        let items = &[DocumentCanvasItem::LinkText {
            x: 10,
            y: 10,
            text: "Click here",
            url: "https://example.com",
            style: DocumentTextStyle::default(),
        }];
        let canvas =
            DocumentCanvas::new(Rect::new(0, 0, 400, 300), items)
                .with_mode(DocumentCanvasMode::Editable)
                .with_presentation(DocumentCanvasPresentation::Writer);
        let content = canvas.content_rect();
        let pt = Point::new(content.x + 15, content.y + 13);
        assert_eq!(canvas.hit_target(pt), super::CanvasHitTarget::Link);
    }

    #[test]
    fn hit_target_returns_none_for_non_interactive_rect() {
        let items = &[DocumentCanvasItem::Rect {
            x: 10,
            y: 10,
            w: 100,
            h: 50,
            style: Default::default(),
        }];
        let canvas =
            DocumentCanvas::new(Rect::new(0, 0, 400, 300), items)
                .with_mode(DocumentCanvasMode::Editable)
                .with_presentation(DocumentCanvasPresentation::Writer);
        let content = canvas.content_rect();
        let pt = Point::new(content.x + 20, content.y + 20);
        assert_eq!(canvas.hit_target(pt), super::CanvasHitTarget::None);
    }

    #[test]
    fn hit_target_returns_none_outside_content_rect() {
        let items = &[DocumentCanvasItem::Text {
            x: 10,
            y: 10,
            text: "Text",
            style: DocumentTextStyle::default(),
        }];
        let canvas =
            DocumentCanvas::new(Rect::new(0, 0, 400, 300), items)
                .with_mode(DocumentCanvasMode::Editable)
                .with_presentation(DocumentCanvasPresentation::Writer);
        let pt = Point::new(-50, -50);
        assert_eq!(canvas.hit_target(pt), super::CanvasHitTarget::None);
    }

    #[test]
    fn hit_target_unicode_text_is_safe() {
        let items = &[DocumentCanvasItem::Text {
            x: 10,
            y: 10,
            text: "SunlightOS ☀️  🐇  🐧  🦀",
            style: DocumentTextStyle::default(),
        }];
        let canvas =
            DocumentCanvas::new(Rect::new(0, 0, 400, 300), items)
                .with_mode(DocumentCanvasMode::Editable)
                .with_presentation(DocumentCanvasPresentation::Writer);
        let content = canvas.content_rect();
        let pt = Point::new(content.x + 15, content.y + 13);
        assert_eq!(canvas.hit_target(pt), super::CanvasHitTarget::Text);
    }

    #[test]
    fn hit_target_is_deterministic() {
        let items = &[DocumentCanvasItem::Text {
            x: 10,
            y: 10,
            text: "Stable",
            style: DocumentTextStyle::default(),
        }];
        let canvas =
            DocumentCanvas::new(Rect::new(0, 0, 400, 300), items)
                .with_mode(DocumentCanvasMode::Editable)
                .with_presentation(DocumentCanvasPresentation::Writer);
        let content = canvas.content_rect();
        let pt = Point::new(content.x + 15, content.y + 13);
        let a = canvas.hit_target(pt);
        let b = canvas.hit_target(pt);
        let c = canvas.hit_target(pt);
        assert_eq!(a, b);
        assert_eq!(b, c);
        assert_eq!(a, super::CanvasHitTarget::Text);
    }

    #[test]
    fn hit_target_scene_text_returns_text() {
        let scene = scene_with_text("Hello", 10);
        let canvas =
            DocumentCanvas::new(Rect::new(0, 0, 400, 300), &[])
                .with_scene(&scene)
                .with_presentation(DocumentCanvasPresentation::Browser);
        let content = canvas.content_rect();
        let pt = Point::new(content.x + 15, content.y + 8);
        assert_eq!(canvas.hit_target(pt), super::CanvasHitTarget::Text);
    }

    #[test]
    fn hit_target_scene_link_takes_priority_over_text() {
        use super::RenderInteraction;
        let mut scene = DocumentScene::new(7, Size::new(200, 100), Size::new(200, 100));
        // Text box underneath
        assert!(scene.push(RenderObject {
            id: RenderObjectId(1),
            owner_node_id: DocumentNodeId(3),
            kind: RenderObjectKind::Text {
                text: "Linked text".into(),
                color: Color::rgb(1, 2, 3),
                font_size: 16,
                bold: false,
                italic: false,
                underline: false,
                line_through: false,
                monospace: false,
                font_family: DocumentFontFamily::SansSerif,
            },
            bounds: Rect::new(10, 4, 80, 20),
            clip_bounds: None,
            paint_order: PaintOrder::default(),
            interaction: None,
        }));
        // Link hit region on top — same position, so hit_test hits this first
        assert!(scene.push(RenderObject {
            id: RenderObjectId(2),
            owner_node_id: DocumentNodeId(3),
            kind: RenderObjectKind::Link {
                href: "https://example.com".into(),
                resolved_url: Some("https://example.com".into()),
                text_object_ids: vec![RenderObjectId(1)],
            },
            bounds: Rect::new(10, 4, 80, 20),
            clip_bounds: None,
            paint_order: PaintOrder::default(),
            interaction: Some(RenderInteraction::Link {
                owner_node_id: DocumentNodeId(3),
                href: "https://example.com".into(),
                resolved_url: Some("https://example.com".into()),
            }),
        }));
        scene.finalize();
        let canvas =
            DocumentCanvas::new(Rect::new(0, 0, 400, 300), &[])
                .with_scene(&scene)
                .with_presentation(DocumentCanvasPresentation::Browser);
        let content = canvas.content_rect();
        let pt = Point::new(content.x + 15, content.y + 8);
        // Link should win — it's drawn on top (pushed after the text object)
        assert_eq!(canvas.hit_target(pt), super::CanvasHitTarget::Link);
    }

    #[test]
    #[cfg(feature = "app")]
    fn cursor_shape_discriminants_match_display_server() {
        // These must stay in sync with services/sunlight-display/src/main.rs
        // CursorShape enum.  If the server reorders, these tests break.
        assert_eq!(CursorShape::Pointer as u8, 0);
        assert_eq!(CursorShape::Hand as u8, 1);
        assert_eq!(CursorShape::Move as u8, 6);
        assert_eq!(CursorShape::Wait as u8, 7);
        assert_eq!(CursorShape::Text as u8, 9);
        // Existing shapes unchanged
        assert_eq!(CursorShape::ResizeHorizontal as u8, 2);
        assert_eq!(CursorShape::ResizeVertical as u8, 3);
        assert_eq!(CursorShape::ResizeNwse as u8, 4);
        assert_eq!(CursorShape::ResizeNesw as u8, 5);
        assert_eq!(CursorShape::Help as u8, 8);
    }
}
