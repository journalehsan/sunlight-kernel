use alloc::{collections::BTreeMap, string::String, sync::Arc, vec, vec::Vec};

use crate::font::VecText;
use crate::geom::{Point, Rect, Size};
use crate::paint::Canvas;
use crate::theme::{Color, Theme};

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
        }
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
        for item in self.items {
            match *item {
                DocumentCanvasItem::Text { x, y, text, style } => {
                    let px = content.x + x;
                    let py = content.y + y;
                    if py < content.y || py >= content.bottom() {
                        continue;
                    }
                    let max_w = (content.right() - px).max(0) as u32;
                    let visible = clip_text_to_width(style.font, text, max_w);
                    if !visible.is_empty() {
                        draw_text(canvas, style.font, px, py, visible, style.color);
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
        diff_scenes, DocumentCanvas, DocumentCanvasItem, DocumentCanvasMode,
        DocumentCanvasPresentation, DocumentFontFamily, DocumentNodeId, DocumentScene,
        DocumentStrokeStyle, DocumentTextStyle, PaintOrder, RasterImage, RenderObject,
        RenderObjectId, RenderObjectKind, ScenePatchOperation,
    };
    use crate::{Canvas, Color, Point, Rect, Size, Theme};
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
}
