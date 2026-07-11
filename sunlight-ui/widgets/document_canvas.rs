use alloc::{collections::BTreeMap, string::String, vec, vec::Vec};

use crate::font::VecText;
use crate::geom::{Point, Rect, Size};
use crate::paint::Canvas;
use crate::theme::{Color, Theme};

const HOST_RADIUS: u32 = 20;
const PAGE_RADIUS: u32 = 10;
const SURFACE_RADIUS: u32 = 12;
const PAGE_HEADER_H: u32 = 62;
const PAGE_FOOTER_H: u32 = 42;
const PAGE_INSET: i32 = 24;
const SURFACE_INSET_X: i32 = 24;
const SURFACE_TOP_GAP: i32 = 18;
const CONTENT_INSET: i32 = 28;

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
    },
    Rectangle {
        fill: Color,
    },
    Border {
        color: Color,
        width: u32,
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
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RenderInteraction {
    Link {
        owner_node_id: DocumentNodeId,
        href: String,
        resolved_url: Option<String>,
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

fn fill_vertical_gradient(canvas: &mut Canvas, rect: Rect, top: Color, bottom: Color) {
    let h = rect.h.max(1);
    for row in 0..h {
        let mix = row * 255 / h;
        let r = ((top.r() as u32 * (255 - mix) + bottom.r() as u32 * mix) / 255) as u8;
        let g = ((top.g() as u32 * (255 - mix) + bottom.g() as u32 * mix) / 255) as u8;
        let b = ((top.b() as u32 * (255 - mix) + bottom.b() as u32 * mix) / 255) as u8;
        canvas.fill_rect(
            Rect::new(rect.x, rect.y + row as i32, rect.w, 1),
            Color::rgb(r, g, b),
        );
    }
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
    pub title: &'a str,
    pub subtitle: &'a str,
    pub empty_label: &'a str,
    pub footer_note: &'a str,
    pub show_guides: bool,
    pub items: &'a [DocumentCanvasItem<'a>],
    /// Optional retained scene for generic read-only document rendering.
    /// Existing Writer callers can continue supplying `items` unchanged.
    pub scene: Option<&'a DocumentScene>,
    pub scroll_y: u32,
    pub title_font: Option<&'a dyn VecText>,
    pub subtitle_font: Option<&'a dyn VecText>,
    pub body_font: Option<&'a dyn VecText>,
    pub small_font: Option<&'a dyn VecText>,
    pub scene_heading_font: Option<&'a dyn VecText>,
}

impl<'a> DocumentCanvas<'a> {
    pub fn new(rect: Rect, items: &'a [DocumentCanvasItem<'a>]) -> Self {
        Self {
            rect,
            mode: DocumentCanvasMode::Editable,
            title: "Document Canvas",
            subtitle: "Reusable fixed-coordinate page surface",
            empty_label: "Document Canvas Ready",
            footer_note: "Fixed-coordinate page surface",
            show_guides: true,
            items,
            scene: None,
            scroll_y: 0,
            title_font: None,
            subtitle_font: None,
            body_font: None,
            small_font: None,
            scene_heading_font: None,
        }
    }

    pub fn with_mode(mut self, mode: DocumentCanvasMode) -> Self {
        self.mode = mode;
        self
    }

    pub fn with_titles(mut self, title: &'a str, subtitle: &'a str) -> Self {
        self.title = title;
        self.subtitle = subtitle;
        self
    }

    pub fn with_empty_label(mut self, empty_label: &'a str) -> Self {
        self.empty_label = empty_label;
        self
    }

    pub fn with_footer_note(mut self, footer_note: &'a str) -> Self {
        self.footer_note = footer_note;
        self
    }

    pub fn with_guides(mut self, show_guides: bool) -> Self {
        self.show_guides = show_guides;
        self
    }

    pub fn with_scene(mut self, scene: &'a DocumentScene) -> Self {
        self.scene = Some(scene);
        self.mode = DocumentCanvasMode::ReadOnly;
        self.show_guides = false;
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
        title_font: Option<&'a dyn VecText>,
        subtitle_font: Option<&'a dyn VecText>,
        body_font: Option<&'a dyn VecText>,
        small_font: Option<&'a dyn VecText>,
    ) -> Self {
        self.title_font = title_font;
        self.subtitle_font = subtitle_font;
        self.body_font = body_font;
        self.small_font = small_font;
        self.scene_heading_font = title_font;
        self
    }

    pub fn host_rect(&self) -> Rect {
        self.rect.inset(18)
    }

    pub fn page_rect(&self) -> Rect {
        let host = self.host_rect();
        let desired_w = 860u32.min(host.w.saturating_sub(96)).max(620);
        let desired_h = host.h.saturating_sub(56).max(420);
        let x = host.x + ((host.w as i32 - desired_w as i32) / 2);
        let y = host.y + 26;
        Rect::new(x, y, desired_w, desired_h)
    }

    pub fn document_rect(&self) -> Rect {
        let page = self.page_rect();
        let top = page.y + PAGE_HEADER_H as i32 + SURFACE_TOP_GAP;
        let bottom = page.bottom() - PAGE_FOOTER_H as i32;
        Rect::new(
            page.x + SURFACE_INSET_X,
            top,
            page.w.saturating_sub((SURFACE_INSET_X * 2) as u32),
            (bottom - top).max(0) as u32,
        )
    }

    pub fn content_rect(&self) -> Rect {
        self.document_rect().inset(CONTENT_INSET)
    }

    pub fn viewport_size(&self) -> Size {
        self.content_rect().size()
    }

    pub fn draw(&self, canvas: &mut Canvas, theme: &Theme) {
        fill_vertical_gradient(canvas, self.rect, theme.bg.lighten(14), theme.bg.darken(8));

        let host = self.host_rect();
        canvas.fill_rounded_rect(host, HOST_RADIUS, theme.panel.darken(8));
        canvas.stroke_rounded_rect(host, HOST_RADIUS, 1, theme.border);

        let page = self.page_rect();
        let shadow = page.translate(10, 12);
        canvas.fill_rounded_rect(shadow, PAGE_RADIUS, Color::rgb(0x16, 0x16, 0x18));
        canvas.fill_rounded_rect(page, PAGE_RADIUS, Color::rgb(0xFB, 0xFA, 0xF7));
        canvas.stroke_rounded_rect(page, PAGE_RADIUS, 1, Color::rgb(0xD7, 0xD3, 0xCD));

        let page_top = Rect::new(page.x, page.y, page.w, PAGE_HEADER_H);
        fill_vertical_gradient(
            canvas,
            page_top,
            Color::rgb(0xFF, 0xFF, 0xFF),
            Color::rgb(0xF4, 0xF1, 0xED),
        );
        canvas.hbar(
            page.x,
            page.y + PAGE_HEADER_H as i32 - 1,
            page.w,
            1,
            Color::rgb(0xE7, 0xE1, 0xD8),
        );

        draw_text(
            canvas,
            self.title_font,
            page.x + PAGE_INSET,
            page.y + 22,
            self.title,
            Color::rgb(0x24, 0x24, 0x28),
        );
        draw_text(
            canvas,
            self.subtitle_font,
            page.x + PAGE_INSET,
            page.y + 40,
            self.subtitle,
            Color::rgb(0x72, 0x72, 0x7C),
        );

        let document = self.document_rect();
        canvas.fill_rounded_rect(document, SURFACE_RADIUS, Color::rgb(0xFF, 0xFF, 0xFF));
        canvas.stroke_rounded_rect(document, SURFACE_RADIUS, 1, Color::rgb(0xE0, 0xDB, 0xD4));
        canvas.hbar(
            document.x + 1,
            document.y + 1,
            document.w.saturating_sub(2),
            4,
            theme.accent.lighten(34),
        );

        let content = self.content_rect();
        if self.show_guides {
            self.draw_guides(canvas, content);
        }

        if let Some(scene) = self.scene {
            self.draw_scene(canvas, content, scene);
        } else if self.items.is_empty() {
            self.draw_empty_label(canvas, content, theme);
        } else {
            self.draw_items(canvas, content);
        }

        draw_text(
            canvas,
            self.small_font,
            page.x + PAGE_INSET,
            page.bottom() - 30,
            self.footer_note,
            Color::rgb(0x86, 0x82, 0x7B),
        );
    }

    fn draw_guides(&self, canvas: &mut Canvas, content: Rect) {
        let guide_color = Color::rgb(0xEE, 0xE8, 0xE0);
        let margin_color = Color::rgb(0xF2, 0xEA, 0xE0);
        let guide_margin_x = content.x + 54;
        canvas.fill_rect(
            Rect::new(guide_margin_x, content.y, 1, content.h),
            margin_color,
        );

        let mut y = content.y + 26;
        while y < content.bottom() - 14 {
            canvas.fill_rect(
                Rect::new(content.x, y, content.w, 1),
                if (y - content.y) % 56 == 0 {
                    guide_color.darken(8)
                } else {
                    guide_color
                },
            );
            y += 28;
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
                    underline,
                    ..
                } => {
                    let font = if *font_size >= 24 {
                        self.scene_heading_font.or(self.body_font)
                    } else {
                        self.body_font
                    };
                    let visible = clip_text_to_width(font, text, clipped.w);
                    if !visible.is_empty() {
                        draw_text(canvas, font, bounds.x, bounds.y, visible, *color);
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
                // A link has interaction metadata but no extra paint.  Its
                // text fragments contain the resolved visual decoration.
                RenderObjectKind::Link { .. } => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        diff_scenes, DocumentCanvas, DocumentCanvasItem, DocumentCanvasMode, DocumentNodeId,
        DocumentScene, DocumentStrokeStyle, DocumentTextStyle, PaintOrder, RenderObject,
        RenderObjectId, RenderObjectKind, ScenePatchOperation,
    };
    use crate::{Canvas, Color, Point, Rect, Size, Theme};

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
    fn mode_defaults_to_editable() {
        assert_eq!(DocumentCanvasMode::default(), DocumentCanvasMode::Editable);
    }

    #[test]
    fn content_rect_stays_within_page() {
        let widget = DocumentCanvas::new(Rect::new(0, 0, 1240, 680), &[]);
        assert!(widget
            .page_rect()
            .intersect(widget.content_rect())
            .is_some());
        assert!(widget.content_rect().right() <= widget.page_rect().right());
        assert!(widget.content_rect().bottom() <= widget.page_rect().bottom());
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
