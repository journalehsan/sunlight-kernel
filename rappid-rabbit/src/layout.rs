//! Index-based CSS style-to-geometry engine.
//!
//! The DOM, computed styles, and layout boxes are all arenas.  A [`LayoutBox`]
//! borrows DOM text and URL attributes instead of cloning them, while the final
//! [`RenderBox`] carries every resolved paint property needed by
//! `DocumentWidget`.
//!
//! The implementation covers normal block flow, anonymous inline formatting
//! contexts, common adjoining-margin collapse, images, single-line flex
//! row/column containers, and intrinsic-sized single-line form controls.
//! Non-rendered nodes (`display: none`, `[hidden]`, `input[type=hidden]`) are
//! pruned during tree construction, so they never occupy a `LayoutBox` or
//! shift adjacent flow.  Unsupported CSS falls back to normal flow.  This is
//! intentional: returning deterministic geometry is safer than dropping a
//! subtree or silently switching to a nested retained object graph.

use alloc::{borrow::Cow, collections::BTreeMap, format, string::String, sync::Arc, vec, vec::Vec};

use golden_fish::{Attribute, Document, Node, NodeId};
use sunlight_ui::{
    widgets::{
        CornerRadii, DocumentFontFamily, DocumentNodeId, DocumentScene, PaintOrder, RasterImage,
        RenderInteraction, RenderObject, RenderObjectId, RenderObjectKind,
    },
    Color, Rect as IntRect, Size,
};

use crate::css::{ComputedStyle, Property, PropertyValue, StyleContext};

const EPSILON: f32 = 0.01;
const MAX_LAYOUT_BOXES: usize = 32_768;
const MAX_INLINE_TOKENS: usize = 65_536;
/// Default `size` for a single-line text field when the attribute is absent.
/// Mirrors the HTML default of 20 average characters.
const DEFAULT_TEXT_COLUMNS: u32 = 20;
/// Horizontal room reserved for a `select`'s disclosure arrow, beyond its
/// padding and border.
const SELECT_ARROW_WIDTH: f32 = 18.0;
/// Extra room around a button caption so its border does not touch the text.
const BUTTON_INNER_PADDING: f32 = 6.0;

/// A final device-independent rectangle.  It is deliberately local to the
/// browser layout IR; the shared canvas converts it once when it builds its
/// retained, integer-coordinate scene.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn right(self) -> f32 {
        self.x + self.width
    }

    pub fn bottom(self) -> f32 {
        self.y + self.height
    }

    fn union(self, other: Self) -> Self {
        if self.width == 0.0 && self.height == 0.0 {
            return other;
        }
        if other.width == 0.0 && other.height == 0.0 {
            return self;
        }
        let x = min(self.x, other.x);
        let y = min(self.y, other.y);
        let right = max(self.right(), other.right());
        let bottom = max(self.bottom(), other.bottom());
        Self::new(x, y, max(0.0, right - x), max(0.0, bottom - y))
    }
}

// `font-family` is retained as a Cow so a future computed-style arena can cache
// normalized families without changing this interface.
//
/// Per-side values in CSS order.  Layout converts these to f32 before exposing
/// a [`RenderBox`]; keeping the order explicit avoids repeated named fields in
/// hot loops.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Edges<T> {
    pub top: T,
    pub right: T,
    pub bottom: T,
    pub left: T,
}

impl<T> Edges<T> {
    pub const fn new(top: T, right: T, bottom: T, left: T) -> Self {
        Self {
            top,
            right,
            bottom,
            left,
        }
    }
}

impl Edges<f32> {
    pub fn horizontal(self) -> f32 {
        self.left + self.right
    }

    pub fn vertical(self) -> f32 {
        self.top + self.bottom
    }
}

/// CSS box sizing.  `Width` in an internal style remains the declared value;
/// the used content width is calculated only by the arrange pass.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BoxSizing {
    #[default]
    ContentBox,
    BorderBox,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Display {
    Block,
    Flex,
    Inline,
    InlineBlock,
    None,
    #[default]
    Other,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FlexDirection {
    #[default]
    Row,
    Column,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MainAxisAlignment {
    #[default]
    Start,
    End,
    Center,
    SpaceBetween,
    SpaceAround,
    SpaceEvenly,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CrossAxisAlignment {
    #[default]
    Stretch,
    Start,
    End,
    Center,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WhiteSpace {
    #[default]
    Normal,
    Nowrap,
    Pre,
    PreWrap,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextAlign {
    #[default]
    Start,
    End,
    Center,
    Justify,
}

/// A CSS dimension retained until the containing block is known.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Dimension {
    #[default]
    Auto,
    Px(f32),
    Percent(f32),
    Vw(f32),
    Vh(f32),
}

impl Dimension {
    fn resolve(self, containing: f32, fallback: f32) -> f32 {
        match self {
            Self::Auto => fallback,
            Self::Px(value) => value,
            Self::Percent(value) => containing * value,
            Self::Vw(_) | Self::Vh(_) => value_placeholder(self),
        }
    }

    fn fixed(self) -> Option<f32> {
        match self {
            Self::Px(value) => Some(value),
            Self::Auto | Self::Percent(_) | Self::Vw(_) | Self::Vh(_) => None,
        }
    }
}

fn value_placeholder(dimension: Dimension) -> f32 {
    match dimension {
        Dimension::Px(value) | Dimension::Vw(value) | Dimension::Vh(value) => value,
        Dimension::Percent(_) | Dimension::Auto => 0.0,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LayoutStyle<'a> {
    pub display: Display,
    pub color: Color,
    pub background_color: Color,
    pub font_family: Cow<'a, str>,
    pub font_size: f32,
    pub line_height: f32,
    pub font_weight: u16,
    pub italic: bool,
    pub underline: bool,
    pub line_through: bool,
    pub white_space: WhiteSpace,
    pub text_align: TextAlign,
    pub box_sizing: BoxSizing,
    pub width: Dimension,
    pub height: Dimension,
    pub min_width: Dimension,
    pub max_width: Option<Dimension>,
    pub min_height: Dimension,
    pub max_height: Option<Dimension>,
    pub margin: Edges<Dimension>,
    pub padding: Edges<Dimension>,
    pub border: Edges<f32>,
    pub border_color: Edges<Color>,
    pub flex_direction: FlexDirection,
    pub justify_content: MainAxisAlignment,
    pub align_items: CrossAxisAlignment,
    pub row_gap: f32,
    pub column_gap: f32,
}

/// Fully resolved values attached to the flattened, paint-ready IR.  Percent
/// and relative units have already been reduced to f32 DIPs.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedStyle<'a> {
    pub display: Display,
    pub color: Color,
    pub background_color: Color,
    pub font_family: Cow<'a, str>,
    pub font_size: f32,
    pub line_height: f32,
    pub font_weight: u16,
    pub italic: bool,
    pub underline: bool,
    pub line_through: bool,
    pub white_space: WhiteSpace,
    pub text_align: TextAlign,
    pub box_sizing: BoxSizing,
    pub width: Option<f32>,
    pub height: Option<f32>,
    pub margin: Edges<f32>,
    pub padding: Edges<f32>,
    pub border: Edges<f32>,
    pub border_color: Edges<Color>,
    pub flex_direction: FlexDirection,
    pub justify_content: MainAxisAlignment,
    pub align_items: CrossAxisAlignment,
    pub row_gap: f32,
    pub column_gap: f32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageResource<'a> {
    pub src: &'a str,
    pub alt: &'a str,
}

/// An anchor inherited through the inline flow.  The URL is kept raw and
/// resolved once by the scene builder against the document base URL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkStyle<'a> {
    pub anchor_node_id: NodeId,
    pub href: &'a str,
}

/// Minimal paint descriptor consumed by `DocumentWidget`.
///
/// Text is a borrowed slice of the DOM buffer and URLs borrow their source
/// attributes.  Style data is copied from the resolved layout arena so the
/// painter never has to query `StyleContext`.
#[derive(Clone, Debug, PartialEq)]
pub struct RenderBox<'a> {
    pub node_id: NodeId,
    pub bounds: Rect,
    pub style: ResolvedStyle<'a>,
    pub node_type: BoxType<'a>,
    /// Anchor inherited from an enclosing `a[href]`, if any.
    pub link: Option<LinkStyle<'a>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BoxType<'a> {
    Block,
    Inline,
    TextRun(&'a str),
    Image(ImageResource<'a>),
    /// A replaced form widget: `input`, `button`, or `select`.  It behaves
    /// like an inline-level replaced element (no layout children) and carries
    /// borrowed DOM data so the scene builder can emit a real control.
    Form(FormWidget<'a>),
}

/// The interactive form widget rendered by a [`BoxType::Form`] box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormWidgetKind {
    Text,
    Search,
    Password,
    Submit,
    Button,
    Select,
}

impl FormWidgetKind {
    /// Retained-scene control kind byte: `1` is a labelled button, `0` is a
    /// single-line field/select in the retained scene.
    pub fn control_kind(self) -> u8 {
        match self {
            FormWidgetKind::Submit | FormWidgetKind::Button => 1,
            _ => 0,
        }
    }

    pub fn is_button(self) -> bool {
        matches!(self, FormWidgetKind::Submit | FormWidgetKind::Button)
    }
}

/// Borrowed form-widget data captured during tree construction.
///
/// Every field borrows the DOM buffer, so a widget never allocates; the
/// retained scene copies the fields it needs when it builds the control.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormWidget<'a> {
    pub kind: FormWidgetKind,
    /// Button caption, or the active option's text for a `select`.
    pub label: &'a str,
    pub placeholder: &'a str,
    pub value: &'a str,
    pub disabled: bool,
    pub editable: bool,
    /// `size` attribute (columns) for single-line text inputs.
    pub size: Option<u32>,
}

/// Calls into the native font stack.  Implementations should query actual
/// shaped advances, not estimate from character counts.  The trait keeps the
/// layout crate independent of any particular font engine.
pub trait FontMeasurer {
    fn measure_width(&self, family: &str, font_size: f32, text: &str) -> f32;

    fn line_height(&self, _family: &str, font_size: f32) -> f32 {
        max(font_size * 1.2, 1.0)
    }
}

#[derive(Default)]
pub struct NoFontMetrics;

impl FontMeasurer for NoFontMetrics {
    fn measure_width(&self, _family: &str, font_size: f32, text: &str) -> f32 {
        text.chars().count() as f32 * font_size * 0.55
    }
}

/// Resolves an image URL to its decoded intrinsic dimensions.
pub trait ImageMetrics {
    fn intrinsic_size(&self, src: &str) -> Option<RectSize>;
}

#[derive(Default)]
pub struct NoImageMetrics;

impl ImageMetrics for NoImageMetrics {
    fn intrinsic_size(&self, _src: &str) -> Option<RectSize> {
        None
    }
}

/// Browser-owned resources the retained scene needs but layout itself does not.
///
/// Layout works on geometry and intrinsic sizes only.  Converting the flattened
/// IR into a paint-ready scene additionally needs decoded pixels and resolved
/// URLs, which live in the browser's image cache and URL resolver.  Keeping
/// them behind a trait lets the layout crate stay independent of
/// `sunlight_http`/`ImageCache` while `render.rs` supplies the browser
/// implementation.
pub trait SceneImageSource {
    /// Resolve a raw `src`/`href` against the document base URL.
    fn resolve(&self, raw: &str) -> Option<String>;

    /// Decoded pixels for a raw `src`, looked up by its resolved URL.
    fn decoded(&self, src: &str) -> Option<Arc<RasterImage>>;
}

/// Source used when no browser resources are available (pure layout tests).
#[derive(Default)]
pub struct NoSceneImages;

impl SceneImageSource for NoSceneImages {
    fn resolve(&self, _raw: &str) -> Option<String> {
        None
    }

    fn decoded(&self, _src: &str) -> Option<Arc<RasterImage>> {
        None
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RectSize {
    pub width: f32,
    pub height: f32,
}

impl RectSize {
    pub const fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }
}

/// Style resolution facade over Golden Fish's index-addressed DOM.
///
/// Golden Fish already owns a compact node arena and `StyleContext` owns a
/// parallel computed-style arena.  Keeping both borrowed here avoids a second
/// tree while still giving layout a typed style-tree interface.
pub struct StyleTree<'a> {
    document: &'a Document,
    context: &'a StyleContext,
}

impl<'a> StyleTree<'a> {
    pub fn new(document: &'a Document, context: &'a StyleContext) -> Self {
        Self { document, context }
    }

    pub fn document(&self) -> &'a Document {
        self.document
    }

    pub fn computed(&self, node_id: NodeId) -> Option<&'a ComputedStyle> {
        self.context.nearest_element_style(self.document, node_id)
    }

    fn resolve(
        &self,
        node_id: NodeId,
        viewport_width: f32,
        viewport_height: f32,
    ) -> LayoutStyle<'a> {
        let computed = self.computed(node_id);
        let font_size = css_length(
            computed,
            Property::FontSize,
            viewport_width,
            viewport_height,
            16.0,
        )
        .unwrap_or(16.0)
        .max(1.0);
        let family = computed
            .and_then(|style| match style.value(&Property::FontFamily) {
                Some(PropertyValue::Keyword(value)) | Some(PropertyValue::Raw(value)) => {
                    Some(Cow::Borrowed(value.as_str()))
                }
                _ => None,
            })
            .unwrap_or(Cow::Borrowed("sans-serif"));
        let measured_line_height = self
            .document
            .get(node_id)
            .and_then(|_| {
                css_length(
                    computed,
                    Property::LineHeight,
                    viewport_width,
                    viewport_height,
                    font_size * 1.2,
                )
            })
            .unwrap_or(font_size * 1.2);
        LayoutStyle {
            display: display(computed),
            color: css_color(computed, Property::Color, Color::rgb(0, 0, 0)),
            background_color: css_color(computed, Property::BackgroundColor, Color::TRANSPARENT),
            font_family: family,
            font_size,
            line_height: measured_line_height.max(1.0),
            font_weight: font_weight(computed),
            italic: matches!(
                keyword(computed, Property::FontStyle).as_str(),
                "italic" | "oblique"
            ),
            underline: keyword(computed, Property::TextDecoration).contains("underline"),
            line_through: keyword(computed, Property::TextDecoration).contains("line-through"),
            white_space: white_space(computed),
            text_align: text_align(computed),
            box_sizing: if keyword(computed, Property::BoxSizing) == "border-box" {
                BoxSizing::BorderBox
            } else {
                BoxSizing::ContentBox
            },
            width: dimension(computed, Property::Width),
            height: dimension(computed, Property::Height),
            min_width: dimension(computed, Property::MinWidth),
            max_width: optional_dimension(computed, Property::MaxWidth),
            min_height: dimension(computed, Property::MinHeight),
            max_height: None,
            margin: Edges::new(
                dimension(computed, Property::MarginTop),
                dimension(computed, Property::MarginRight),
                dimension(computed, Property::MarginBottom),
                dimension(computed, Property::MarginLeft),
            ),
            padding: Edges::new(
                dimension(computed, Property::PaddingTop),
                dimension(computed, Property::PaddingRight),
                dimension(computed, Property::PaddingBottom),
                dimension(computed, Property::PaddingLeft),
            ),
            border: Edges::new(
                css_px(computed, Property::BorderTopWidth).unwrap_or(0.0),
                css_px(computed, Property::BorderRightWidth).unwrap_or(0.0),
                css_px(computed, Property::BorderBottomWidth).unwrap_or(0.0),
                css_px(computed, Property::BorderLeftWidth).unwrap_or(0.0),
            ),
            border_color: Edges::new(
                css_color(
                    computed,
                    Property::BorderTopColor,
                    css_color(computed, Property::Color, Color::rgb(0, 0, 0)),
                ),
                css_color(
                    computed,
                    Property::BorderRightColor,
                    css_color(computed, Property::Color, Color::rgb(0, 0, 0)),
                ),
                css_color(
                    computed,
                    Property::BorderBottomColor,
                    css_color(computed, Property::Color, Color::rgb(0, 0, 0)),
                ),
                css_color(
                    computed,
                    Property::BorderLeftColor,
                    css_color(computed, Property::Color, Color::rgb(0, 0, 0)),
                ),
            ),
            flex_direction: if keyword(computed, Property::FlexDirection) == "column" {
                FlexDirection::Column
            } else {
                FlexDirection::Row
            },
            justify_content: main_axis_alignment(computed),
            align_items: cross_axis_alignment(computed),
            row_gap: css_length(
                computed,
                Property::RowGap,
                viewport_width,
                viewport_height,
                0.0,
            )
            .or_else(|| {
                css_length(
                    computed,
                    Property::Gap,
                    viewport_width,
                    viewport_height,
                    0.0,
                )
            })
            .unwrap_or(0.0),
            column_gap: css_length(
                computed,
                Property::ColumnGap,
                viewport_width,
                viewport_height,
                0.0,
            )
            .or_else(|| {
                css_length(
                    computed,
                    Property::Gap,
                    viewport_width,
                    viewport_height,
                    0.0,
                )
            })
            .unwrap_or(0.0),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct LayoutId(pub u32);

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct IntrinsicSize {
    pub min_content_width: f32,
    pub max_content_width: f32,
    pub natural_height: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextFragment<'a> {
    pub text: &'a str,
    pub bounds: Rect,
}

/// One arena record.  Parent/child links are integers; children are contiguous
/// in DOM order even though the two layout passes may visit them differently.
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutBox<'a> {
    pub id: LayoutId,
    pub dom_node: NodeId,
    pub parent: Option<LayoutId>,
    pub children: Vec<LayoutId>,
    pub style: LayoutStyle<'a>,
    pub kind: BoxType<'a>,
    /// Anchor inherited from the nearest enclosing `a[href]`.
    pub link: Option<LinkStyle<'a>>,
    pub intrinsic: IntrinsicSize,
    pub content_box: Rect,
    pub padding_box: Rect,
    pub border_box: Rect,
    pub margin_box: Rect,
    pub used_margin: Edges<f32>,
    pub used_padding: Edges<f32>,
    pub fragments: Vec<TextFragment<'a>>,
    pub image_size: Option<RectSize>,
}

impl<'a> LayoutBox<'a> {
    fn contributes_children_to_inline_flow(&self) -> bool {
        matches!(
            self.kind,
            BoxType::Inline | BoxType::TextRun(_) | BoxType::Image(_) | BoxType::Form(_)
        ) && self.style.display != Display::None
    }

    fn is_block_container(&self) -> bool {
        // A `BoxType::Block` box must always be arranged by its containing
        // block.  Only explicit inline-level displays hand the box to the
        // inline formatter instead.  `Display::Other` covers keywords this
        // engine does not model (`-webkit-box`, `grid`, `contents`, ...);
        // treating them as non-block would leave the box unarranged and drop
        // the element and its whole subtree.
        !matches!(self.kind, BoxType::TextRun(_))
            && !matches!(self.style.display, Display::Inline | Display::InlineBlock)
    }
}

/// Borrowed services and viewport used by both passes.
pub struct GeometryContext<'a> {
    pub styles: StyleTree<'a>,
    pub viewport_width: f32,
    pub viewport_height: f32,
    pub root_font_size: f32,
    pub fonts: &'a dyn FontMeasurer,
    pub images: &'a dyn ImageMetrics,
}

impl<'a> GeometryContext<'a> {
    pub fn new(
        document: &'a Document,
        styles: &'a StyleContext,
        viewport: Size,
        fonts: &'a dyn FontMeasurer,
        images: &'a dyn ImageMetrics,
    ) -> Self {
        Self {
            styles: StyleTree::new(document, styles),
            viewport_width: viewport.w.max(1) as f32,
            viewport_height: viewport.h.max(1) as f32,
            root_font_size: 16.0,
            fonts,
            images,
        }
    }

    fn dimension(&self, value: Dimension, _font_size: f32, containing: f32) -> f32 {
        match value {
            Dimension::Vw(percent) => self.viewport_width * percent / 100.0,
            Dimension::Vh(percent) => self.viewport_height * percent / 100.0,
            _ => value.resolve(containing, 0.0),
        }
        .max(0.0)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LayoutTree<'a> {
    pub boxes: Vec<LayoutBox<'a>>,
    pub root: Option<LayoutId>,
    pub content_size: RectSize,
    /// HTML/body background propagated to the document canvas.
    pub canvas_background: Option<(NodeId, Color)>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LayoutResult<'a> {
    /// Generated content owned by list items, separate from DOM text.
    pub list_markers: Vec<RenderMarker<'a>>,
    pub geometry: LayoutTree<'a>,
    pub render_boxes: Vec<RenderBox<'a>>,
}

/// A generated list label positioned beside its item's first line.
#[derive(Clone, Debug, PartialEq)]
pub struct RenderMarker<'a> {
    pub node_id: NodeId,
    pub bounds: Rect,
    pub label: String,
    pub style: ResolvedStyle<'a>,
}

/// Converts the f32 browser IR into the shared `DocumentWidget` retained scene.
///
/// This is the only integer conversion boundary.  Text and URL references are
/// copied into the retained render objects here; layout itself remains
/// borrowed and allocation-light.
pub fn into_document_scene<'a>(
    document_id: u64,
    viewport: Size,
    result: &'a LayoutResult<'a>,
) -> DocumentScene {
    into_document_scene_with_images(document_id, viewport, result, &NoSceneImages)
}

/// Converts the f32 browser IR into the shared retained scene, preserving
/// decoded images and link hit regions.
///
/// This is the browser-facing variant of [`into_document_scene`].  It differs
/// in two ways that the pure-layout helper cannot express: images carry their
/// decoded pixels, resolved URL, and intrinsic dimensions, and each rendered
/// anchor loses its per-fragment link metadata in favour of one non-painting
/// `Link` hit region per visual line.
pub fn into_document_scene_with_images<'a, S: SceneImageSource + ?Sized>(
    document_id: u64,
    viewport: Size,
    result: &'a LayoutResult<'a>,
    images: &S,
) -> DocumentScene {
    let mut scene = DocumentScene::new(
        document_id,
        viewport,
        Size::new(
            ceil_u32(result.geometry.content_size.width),
            ceil_u32(result.geometry.content_size.height),
        ),
    );
    if let Some((node_id, fill)) = result.geometry.canvas_background {
        scene.push(RenderObject {
            id: RenderObjectId(u64::MAX),
            owner_node_id: DocumentNodeId(node_id as u64),
            kind: RenderObjectKind::Rectangle { fill },
            bounds: IntRect::new(
                0,
                0,
                viewport.w.max(scene.content_size.w),
                viewport.h.max(scene.content_size.h),
            ),
            clip_bounds: None,
            paint_order: PaintOrder {
                phase: 1,
                ..PaintOrder::default()
            },
            interaction: None,
        });
    }
    // Accumulated link hit regions keyed by (anchor node, href, line y, height)
    // so a wrapped anchor yields one tight rectangle per visual line.
    let mut links: Vec<LinkRegion> = Vec::new();
    for (order, item) in result.render_boxes.iter().enumerate() {
        let object_slot = (order as u64).saturating_mul(3);
        let bounds = int_rect(item.bounds);
        let family = DocumentFontFamily::resolve_css_list(&item.style.font_family);
        let document_order = order.min(u32::MAX as usize) as u32;
        let (kind, phase) = match &item.node_type {
            BoxType::Block | BoxType::Inline => (RenderObjectKind::Box, 0),
            BoxType::Image(image) => (image_object_kind(image, images), 3),
            BoxType::Form(widget) => (form_object_kind(widget, &item.style, family), 5),
            BoxType::TextRun(text) => (
                RenderObjectKind::Text {
                    text: String::from(*text),
                    color: item.style.color,
                    font_size: ceil_u32(item.style.font_size),
                    bold: item.style.font_weight >= 600,
                    italic: item.style.italic,
                    underline: item.style.underline,
                    line_through: item.style.line_through,
                    monospace: family == DocumentFontFamily::Monospace,
                    font_family: family,
                },
                4,
            ),
        };
        let object_id = RenderObjectId(object_slot.saturating_add(1));
        let interaction = match &item.node_type {
            BoxType::TextRun(_) => item.link.as_ref().map(|link| {
                let resolved_url = images.resolve(link.href);
                push_link_region(
                    &mut links,
                    link,
                    resolved_url.as_deref(),
                    bounds,
                    document_order,
                    object_id,
                );
                RenderInteraction::Link {
                    owner_node_id: DocumentNodeId(link.anchor_node_id as u64),
                    href: String::from(link.href),
                    resolved_url,
                }
            }),
            BoxType::Form(_) => Some(RenderInteraction::Control {
                owner_node_id: DocumentNodeId(item.node_id as u64),
            }),
            _ => None,
        };
        scene.push(RenderObject {
            id: object_id,
            owner_node_id: DocumentNodeId(item.node_id as u64),
            kind,
            bounds,
            clip_bounds: None,
            paint_order: PaintOrder {
                phase,
                document_order,
                ..PaintOrder::default()
            },
            interaction,
        });
        if matches!(item.node_type, BoxType::Block | BoxType::Inline)
            && item.style.background_color != Color::TRANSPARENT
            && !result
                .geometry
                .canvas_background
                .is_some_and(|(node_id, _)| node_id == item.node_id)
        {
            scene.push(RenderObject {
                id: RenderObjectId(object_slot.saturating_add(2)),
                owner_node_id: DocumentNodeId(item.node_id as u64),
                kind: RenderObjectKind::Rectangle {
                    fill: item.style.background_color,
                },
                bounds,
                clip_bounds: None,
                paint_order: PaintOrder {
                    phase: 1,
                    document_order,
                    ..PaintOrder::default()
                },
                interaction: None,
            });
        }
        if matches!(item.node_type, BoxType::Block | BoxType::Inline)
            && [
                item.style.border.top,
                item.style.border.right,
                item.style.border.bottom,
                item.style.border.left,
            ]
            .iter()
            .any(|width| *width > 0.0)
        {
            scene.push(RenderObject {
                id: RenderObjectId(object_slot.saturating_add(3)),
                owner_node_id: DocumentNodeId(item.node_id as u64),
                kind: RenderObjectKind::BorderSides {
                    colors: [
                        item.style.border_color.top,
                        item.style.border_color.right,
                        item.style.border_color.bottom,
                        item.style.border_color.left,
                    ],
                    widths: [
                        ceil_u32(item.style.border.top),
                        ceil_u32(item.style.border.right),
                        ceil_u32(item.style.border.bottom),
                        ceil_u32(item.style.border.left),
                    ],
                    radii: CornerRadii::default(),
                },
                bounds,
                clip_bounds: None,
                paint_order: PaintOrder {
                    phase: 2,
                    document_order,
                    ..PaintOrder::default()
                },
                interaction: None,
            });
        }
    }
    for marker in &result.list_markers {
        let family = DocumentFontFamily::resolve_css_list(&marker.style.font_family);
        scene.push(RenderObject {
            id: RenderObjectId(((marker.node_id as u64) << 32) | (31u64 << 24)),
            owner_node_id: DocumentNodeId(marker.node_id as u64),
            kind: RenderObjectKind::Text {
                text: marker.label.clone(),
                color: marker.style.color,
                font_size: ceil_u32(marker.style.font_size),
                bold: marker.style.font_weight >= 600,
                italic: marker.style.italic,
                underline: false,
                line_through: false,
                monospace: family == DocumentFontFamily::Monospace,
                font_family: family,
            },
            bounds: int_rect(marker.bounds),
            clip_bounds: None,
            paint_order: PaintOrder {
                phase: 4,
                ..PaintOrder::default()
            },
            interaction: None,
        });
    }
    for (slot, region) in links.into_iter().enumerate() {
        scene.push(RenderObject {
            id: RenderObjectId(
                ((region.anchor_node_id as u64) << 32)
                    | (30u64 << 24)
                    | (slot.min(0x00ff_ffff) as u64),
            ),
            owner_node_id: DocumentNodeId(region.anchor_node_id as u64),
            kind: RenderObjectKind::Link {
                href: String::from(region.href),
                resolved_url: region
                    .resolved_url
                    .as_ref()
                    .map(|url| String::from(url.as_str())),
                text_object_ids: region.text_object_ids,
            },
            bounds: region.bounds,
            clip_bounds: None,
            paint_order: PaintOrder {
                phase: 4,
                document_order: region.document_order,
                ..PaintOrder::default()
            },
            interaction: Some(RenderInteraction::Link {
                owner_node_id: DocumentNodeId(region.anchor_node_id as u64),
                href: String::from(region.href),
                resolved_url: region
                    .resolved_url
                    .as_ref()
                    .map(|url| String::from(url.as_str())),
            }),
        });
    }
    scene.finalize();
    scene
}

/// One accumulated `a[href]` hit region grouped by anchor and visual line.
struct LinkRegion<'a> {
    anchor_node_id: NodeId,
    href: &'a str,
    resolved_url: Option<String>,
    bounds: IntRect,
    document_order: u32,
    text_object_ids: Vec<RenderObjectId>,
}

fn push_link_region<'a>(
    links: &mut Vec<LinkRegion<'a>>,
    link: &'a LinkStyle<'a>,
    resolved_url: Option<&str>,
    bounds: IntRect,
    document_order: u32,
    object_id: RenderObjectId,
) {
    if let Some(existing) = links.iter_mut().find(|region| {
        region.anchor_node_id == link.anchor_node_id
            && region.href == link.href
            && region.bounds.y == bounds.y
            && region.bounds.h == bounds.h
    }) {
        existing.bounds = union_rect(existing.bounds, bounds);
        existing.text_object_ids.push(object_id);
        return;
    }
    links.push(LinkRegion {
        anchor_node_id: link.anchor_node_id,
        href: link.href,
        resolved_url: resolved_url.map(String::from),
        bounds,
        document_order,
        text_object_ids: vec![object_id],
    });
}

/// Resolves one image `RenderBox` to decoded pixels or a placeholder.
fn image_object_kind<S: SceneImageSource + ?Sized>(
    image: &ImageResource<'_>,
    images: &S,
) -> RenderObjectKind {
    let resolved_url = images.resolve(image.src);
    match images.decoded(image.src) {
        Some(decoded) => {
            let (intrinsic_width, intrinsic_height) = (decoded.width, decoded.height);
            RenderObjectKind::Image {
                image: decoded,
                source_url: resolved_url.unwrap_or_else(|| String::from(image.src)),
                intrinsic_width,
                intrinsic_height,
                alt: String::from(image.alt),
            }
        }
        None => RenderObjectKind::ImagePlaceholder {
            src: resolved_url.unwrap_or_else(|| String::from(image.src)),
            alt: String::from(image.alt),
        },
    }
}

/// User-agent defaults for a form widget when author CSS leaves them unset.
///
/// A control always needs a visible box, so missing borders fall back to a
/// 1px grey outline and a transparent background becomes the platform field
/// fill.
const UA_CONTROL_BORDER: Color = Color::rgb(0x88, 0x88, 0x88);
const UA_CONTROL_BACKGROUND: Color = Color::rgb(0xFF, 0xFF, 0xFF);
const UA_CONTROL_DISABLED_BACKGROUND: Color = Color::rgb(0xDD, 0xDD, 0xDD);

/// Builds a retained `Control` object from a laid-out form widget, applying
/// user-agent border/background defaults only where author CSS is silent.
fn form_object_kind(
    widget: &FormWidget<'_>,
    style: &ResolvedStyle<'_>,
    family: DocumentFontFamily,
) -> RenderObjectKind {
    let border_width = [
        style.border.top,
        style.border.right,
        style.border.bottom,
        style.border.left,
    ]
    .into_iter()
    .fold(0.0_f32, max);
    let border_width = if border_width > 0.0 {
        ceil_u32(border_width)
    } else {
        1
    };
    let border_color = if style.border_color.top.a() > 0 {
        style.border_color.top
    } else {
        UA_CONTROL_BORDER
    };
    let background = match style.background_color {
        Color::TRANSPARENT if widget.disabled => UA_CONTROL_DISABLED_BACKGROUND,
        Color::TRANSPARENT => UA_CONTROL_BACKGROUND,
        color => color,
    };
    RenderObjectKind::Control {
        label: String::from(widget.label),
        placeholder: String::from(widget.placeholder),
        value: String::from(widget.value),
        color: style.color,
        background,
        border_color,
        border_width,
        focused: false,
        disabled: widget.disabled,
        editable: widget.editable,
        kind: widget.kind.control_kind(),
        caret_offset: None,
        font_size: ceil_u32(style.font_size),
        font_family: family,
    }
}

fn union_rect(left: IntRect, right: IntRect) -> IntRect {
    let x = left.x.min(right.x);
    let y = left.y.min(right.y);
    let width = left.right().max(right.right()).saturating_sub(x) as u32;
    let height = left.bottom().max(right.bottom()).saturating_sub(y) as u32;
    IntRect::new(x, y, width, height)
}

/// Runs style-tree ingestion, intrinsic measurement, and coordinate layout.
///
/// The returned render boxes are flattened in paint order: background boxes
/// first, then images, then text fragments.  The tree remains available for
/// inspector and hit-testing code.
pub fn layout_document<'a>(
    document: &'a Document,
    styles: &'a StyleContext,
    viewport: Size,
    fonts: &'a dyn FontMeasurer,
    images: &'a dyn ImageMetrics,
) -> LayoutResult<'a> {
    let mut engine = LayoutEngine {
        context: GeometryContext::new(document, styles, viewport, fonts, images),
        tree: LayoutTree::default(),
    };
    engine.build(document.root(), None, None);
    engine.measure_all();
    if let Some(root) = engine.tree.root {
        engine.arrange(
            root,
            engine.context.viewport_width,
            engine.context.viewport_height,
        );
        engine.finish_parent_boxes(root);
        engine.tree.content_size = engine.tree.boxes.iter().fold(
            RectSize::new(
                engine.context.viewport_width,
                engine.context.viewport_height,
            ),
            |extent, item| {
                RectSize::new(
                    max(extent.width, item.margin_box.right()),
                    max(extent.height, item.margin_box.bottom()),
                )
            },
        );
    }
    for tag in ["html", "body"] {
        if let Some(item) = engine.tree.boxes.iter().find(|item| {
            matches!(document.get(item.dom_node),
            Some(Node::Element { tag_name, .. }) if tag_name.eq_ignore_ascii_case(tag))
        }) {
            if item.style.background_color != Color::TRANSPARENT {
                engine.tree.canvas_background = Some((item.dom_node, item.style.background_color));
                break;
            }
        }
    }
    let mut render_boxes = Vec::new();
    if let Some(root) = engine.tree.root {
        engine.flatten(root, &mut render_boxes);
    }
    let list_markers = engine.list_markers();
    LayoutResult {
        list_markers,
        geometry: engine.tree,
        render_boxes,
    }
}

struct LayoutEngine<'a> {
    context: GeometryContext<'a>,
    tree: LayoutTree<'a>,
}

impl<'a> LayoutEngine<'a> {
    fn build(
        &mut self,
        node_id: NodeId,
        parent: Option<LayoutId>,
        inherited_link: Option<LinkStyle<'a>>,
    ) -> Option<LayoutId> {
        if self.tree.boxes.len() >= MAX_LAYOUT_BOXES {
            return None;
        }
        let document = self.context.styles.document();
        let node = document.get(node_id)?;
        let mut style = self.context.styles.resolve(
            node_id,
            self.context.viewport_width,
            self.context.viewport_height,
        );
        if matches!(
            self.context
                .styles
                .computed(node_id)
                .and_then(|style| style.value(&Property::LineHeight)),
            None | Some(PropertyValue::Normal) | Some(PropertyValue::Auto)
        ) {
            style.line_height = max(
                style.line_height,
                self.context
                    .fonts
                    .line_height(&style.font_family, style.font_size),
            );
        }
        if matches!(node, Node::Document { .. }) {
            style.display = Display::Block;
        }
        // Visibility pruning happens before a `LayoutBox` is created, so a
        // hidden node occupies no arena slot and no space in the layout tree.
        // The HTML `hidden` attribute and `display: none` both remove the node;
        // `input[type=hidden]` is a non-rendered control by definition.
        if is_pruned(node, &style) {
            return None;
        }
        let link = match node {
            Node::Element {
                tag_name,
                attributes,
                ..
            } if tag_name.eq_ignore_ascii_case("a") => match attribute_value(attributes, "href") {
                Some(href) if !href.is_empty() => Some(LinkStyle {
                    anchor_node_id: node_id,
                    href,
                }),
                _ => inherited_link.clone(),
            },
            _ => inherited_link.clone(),
        };
        let kind = match node {
            Node::Document { .. } => BoxType::Block,
            Node::Element {
                tag_name,
                attributes,
                ..
            } if tag_name.eq_ignore_ascii_case("img") => {
                let src = attribute_value(attributes, "src").unwrap_or("");
                // HTML dimensions are presentational hints, below author CSS.
                // An explicit CSS `auto` also overrides the HTML attribute.
                let computed = self.context.styles.computed(node_id);
                for (property, name, dimension) in [
                    (Property::Width, "width", &mut style.width),
                    (Property::Height, "height", &mut style.height),
                ] {
                    let specified = computed
                        .and_then(|style| {
                            style
                                .properties
                                .iter()
                                .find(|entry| entry.property == property)
                        })
                        .is_some_and(|property| property.matched.is_some());
                    if !specified {
                        if let Some(value) = attribute_value(attributes, name)
                            .and_then(|value| value.trim().parse::<u32>().ok())
                        {
                            *dimension = Dimension::Px(value as f32);
                        }
                    }
                }
                BoxType::Image(ImageResource {
                    src,
                    alt: attribute_value(attributes, "alt").unwrap_or(""),
                })
            }
            Node::Element {
                tag_name,
                attributes,
                children,
                ..
            } if is_form_widget_tag(tag_name) => {
                match form_widget(document, tag_name, attributes, children) {
                    Some(widget) => BoxType::Form(widget),
                    None => match style.display {
                        Display::Inline | Display::InlineBlock => BoxType::Inline,
                        _ => BoxType::Block,
                    },
                }
            }
            Node::Element { .. } => match style.display {
                Display::Inline | Display::InlineBlock => BoxType::Inline,
                _ => BoxType::Block,
            },
            Node::Text { content } => BoxType::TextRun(content),
            Node::Comment { .. } => return None,
        };
        let id = LayoutId(self.tree.boxes.len() as u32);
        let image_size = match kind {
            BoxType::Image(ref image) => self.context.images.intrinsic_size(image.src),
            _ => None,
        };
        self.tree.boxes.push(LayoutBox {
            id,
            dom_node: node_id,
            parent,
            children: Vec::new(),
            style,
            kind: kind.clone(),
            link: link.clone(),
            intrinsic: IntrinsicSize::default(),
            content_box: Rect::default(),
            padding_box: Rect::default(),
            border_box: Rect::default(),
            margin_box: Rect::default(),
            used_margin: Edges::default(),
            used_padding: Edges::default(),
            fragments: Vec::new(),
            image_size,
        });
        if parent.is_none() {
            self.tree.root = Some(id);
        }
        // Replaced widgets have no layout children: a `select`'s options are
        // painted by the control, not laid out as blocks.
        if matches!(kind, BoxType::Form(_)) {
            return Some(id);
        }
        for &child in document.children(node_id) {
            if let Some(child_id) = self.build(child, Some(id), link.clone()) {
                self.tree.boxes[id.0 as usize].children.push(child_id);
            }
        }
        Some(id)
    }

    /// Pass 1: measure every subtree bottom-up.  Inline blocks are measured
    /// twice: once at min-content width and once without wrapping, which
    /// provides both intrinsic bounds without retaining per-width caches.
    fn measure_all(&mut self) {
        let Some(root) = self.tree.root else {
            return;
        };
        let mut stack = vec![(root, false)];
        while let Some((id, visited)) = stack.pop() {
            if !visited {
                stack.push((id, true));
                for &child in self.tree.boxes[id.0 as usize].children.iter().rev() {
                    stack.push((child, false));
                }
                continue;
            }
            self.measure_box(id);
        }
    }

    fn measure_box(&mut self, id: LayoutId) {
        let index = id.0 as usize;
        let kind = self.tree.boxes[index].kind.clone();
        let style = self.tree.boxes[index].style.clone();
        let intrinsic = match kind {
            BoxType::TextRun(text) => {
                let (min, max_width) = text_intrinsic(&self.context, &style, text);
                IntrinsicSize {
                    min_content_width: min,
                    max_content_width: max_width,
                    natural_height: style.line_height,
                }
            }
            BoxType::Image(_) => {
                let size = self.tree.boxes[index]
                    .image_size
                    .unwrap_or(RectSize::new(160.0, 100.0));
                IntrinsicSize {
                    min_content_width: size.width,
                    max_content_width: size.width,
                    natural_height: size.height,
                }
            }
            BoxType::Form(ref widget) => {
                let size = form_border_size(&self.context, &style, widget);
                IntrinsicSize {
                    min_content_width: size.width,
                    max_content_width: size.width,
                    natural_height: size.height,
                }
            }
            BoxType::Inline if style.display == Display::InlineBlock => self.measure_block(id),
            BoxType::Inline => self.measure_inline_container(id),
            BoxType::Block => match style.display {
                Display::Flex => self.measure_flex(id),
                _ => self.measure_block(id),
            },
        };
        self.tree.boxes[index].intrinsic = intrinsic;
    }

    fn measure_block(&self, id: LayoutId) -> IntrinsicSize {
        let layout_box = &self.tree.boxes[id.0 as usize];
        let mut min_width: f32 = 0.0;
        let mut max_width: f32 = 0.0;
        let mut height: f32 = 0.0;
        let mut inline_children = Vec::new();
        let flush_inline = |items: &mut Vec<LayoutId>,
                            min_width: &mut f32,
                            max_width: &mut f32,
                            height: &mut f32| {
            if items.is_empty() {
                return;
            }
            let (min, max_width_for_run, run_height) =
                measure_inline(&self.context, &self.tree.boxes, items, None);
            *min_width = max(*min_width, min);
            *max_width = max(*max_width, max_width_for_run);
            *height += run_height;
            items.clear();
        };
        for &child in &layout_box.children {
            let child_box = &self.tree.boxes[child.0 as usize];
            if child_box.is_block_container() {
                flush_inline(
                    &mut inline_children,
                    &mut min_width,
                    &mut max_width,
                    &mut height,
                );
                let padding = resolve_edges(
                    &self.context,
                    child_box.style.padding,
                    &child_box.style,
                    self.context.viewport_width,
                );
                let margins = resolve_edges(
                    &self.context,
                    child_box.style.margin,
                    &child_box.style,
                    self.context.viewport_width,
                );
                let outer = padding.horizontal() + child_box.style.border.horizontal();
                let child_width = |natural| {
                    let content = if matches!(child_box.kind, BoxType::Image(_) | BoxType::Form(_))
                    {
                        replaced_content_size(
                            &self.context,
                            child_box,
                            self.context.viewport_width,
                            self.context.viewport_height,
                        )
                        .width
                    } else {
                        natural
                    };
                    constrain_border_width(
                        &self.context,
                        &child_box.style,
                        self.context.viewport_width,
                        used_border_dimension(
                            &self.context,
                            child_box.style.width,
                            &child_box.style,
                            self.context.viewport_width,
                            content,
                            outer,
                        ),
                        outer,
                    ) + margins.horizontal()
                };
                min_width = max(
                    min_width,
                    child_width(child_box.intrinsic.min_content_width),
                );
                max_width = max(
                    max_width,
                    child_width(child_box.intrinsic.max_content_width),
                );
                let margins = self.context.dimension(
                    child_box.style.margin.top,
                    child_box.style.font_size,
                    self.context.viewport_width,
                ) + self.context.dimension(
                    child_box.style.margin.bottom,
                    child_box.style.font_size,
                    self.context.viewport_width,
                );
                height += child_box.intrinsic.natural_height + max(0.0, margins);
            } else if child_box.contributes_children_to_inline_flow() {
                inline_children.push(child);
            }
        }
        flush_inline(
            &mut inline_children,
            &mut min_width,
            &mut max_width,
            &mut height,
        );
        IntrinsicSize {
            min_content_width: max(min_width, layout_box.intrinsic.min_content_width),
            max_content_width: max(max_width, layout_box.intrinsic.max_content_width),
            natural_height: max(height, layout_box.intrinsic.natural_height),
        }
    }

    fn measure_inline_container(&self, id: LayoutId) -> IntrinsicSize {
        let layout_box = &self.tree.boxes[id.0 as usize];
        let items: Vec<LayoutId> = layout_box.children.clone();
        let (min, max_width, height) =
            measure_inline(&self.context, &self.tree.boxes, &items, None);
        IntrinsicSize {
            min_content_width: min,
            max_content_width: max_width,
            natural_height: height,
        }
    }

    fn measure_flex(&self, id: LayoutId) -> IntrinsicSize {
        let layout_box = &self.tree.boxes[id.0 as usize];
        let mut children = layout_box.children.iter().copied();
        if layout_box.style.flex_direction == FlexDirection::Row {
            let mut min = 0.0;
            let mut max_width = 0.0;
            let mut height: f32 = 0.0;
            let mut seen = false;
            for child in children.by_ref() {
                let item = &self.tree.boxes[child.0 as usize];
                min += item.intrinsic.min_content_width;
                max_width += item.intrinsic.max_content_width;
                height = max(height, item.intrinsic.natural_height);
                if seen {
                    min += layout_box.style.column_gap;
                    max_width += layout_box.style.column_gap;
                }
                seen = true;
            }
            IntrinsicSize {
                min_content_width: min,
                max_content_width: max_width,
                natural_height: height,
            }
        } else {
            let mut width: f32 = 0.0;
            let mut height = 0.0;
            let mut seen = false;
            for child in children.by_ref() {
                let item = &self.tree.boxes[child.0 as usize];
                width = max(width, item.intrinsic.max_content_width);
                height += item.intrinsic.natural_height;
                if seen {
                    height += layout_box.style.row_gap;
                }
                seen = true;
            }
            IntrinsicSize {
                min_content_width: width,
                max_content_width: width,
                natural_height: height,
            }
        }
    }

    /// Pass 2: arrange a border box at `(x, y)`.  Block widths are constrained
    /// against the containing block; child coordinates are assigned only after
    /// the parent's padding edge is known.
    fn arrange(&mut self, id: LayoutId, available_width: f32, available_height: f32) -> f32 {
        let index = id.0 as usize;
        let style = self.tree.boxes[index].style.clone();
        if style.display == Display::None {
            return 0.0;
        }
        let padding = Edges::new(
            self.context
                .dimension(style.padding.top, style.font_size, available_width),
            self.context
                .dimension(style.padding.right, style.font_size, available_width),
            self.context
                .dimension(style.padding.bottom, style.font_size, available_width),
            self.context
                .dimension(style.padding.left, style.font_size, available_width),
        );
        let border = style.border;
        let mut margins = Edges::new(
            self.context
                .dimension(style.margin.top, style.font_size, available_width),
            self.context
                .dimension(style.margin.right, style.font_size, available_width),
            self.context
                .dimension(style.margin.bottom, style.font_size, available_width),
            self.context
                .dimension(style.margin.left, style.font_size, available_width),
        );
        self.tree.boxes[index].used_padding = padding;
        let outer_horizontal = padding.horizontal() + border.left + border.right;
        let specified_width = match style.width {
            Dimension::Auto => None,
            value => Some(
                self.context
                    .dimension(value, style.font_size, available_width),
            ),
        };
        let mut border_width = match (specified_width, style.box_sizing) {
            (Some(width), BoxSizing::BorderBox) => max(width, outer_horizontal),
            (Some(width), BoxSizing::ContentBox) => width + outer_horizontal,
            (None, _)
                if matches!(
                    self.tree.boxes[index].kind,
                    BoxType::Image(_) | BoxType::Form(_)
                ) =>
            {
                replaced_content_size(
                    &self.context,
                    &self.tree.boxes[index],
                    available_width,
                    available_height,
                )
                .width
                    + outer_horizontal
            }
            (None, _) if style.display == Display::InlineBlock => {
                let intrinsic = self.tree.boxes[index].intrinsic;
                max(
                    intrinsic.min_content_width,
                    min(
                        intrinsic.max_content_width,
                        max(
                            0.0,
                            available_width - margins.horizontal() - outer_horizontal,
                        ),
                    ),
                ) + outer_horizontal
            }
            (None, _) => max(outer_horizontal, available_width - margins.horizontal()),
        };
        border_width = constrain_border_width(
            &self.context,
            &style,
            available_width,
            border_width,
            outer_horizontal,
        );
        let content_width = max(0.0, border_width - outer_horizontal);
        // Auto margins absorb positive free space only in normal block flow.
        // Inline-level auto margins remain zero; text-align positions the line.
        if self.tree.boxes[index].is_block_container() {
            let free = max(0.0, available_width - border_width - margins.horizontal());
            match (
                style.margin.left == Dimension::Auto,
                style.margin.right == Dimension::Auto,
            ) {
                (true, true) => {
                    margins.left = free * 0.5;
                    margins.right = free * 0.5;
                }
                (true, false) => margins.left = free,
                (false, true) => margins.right = free,
                _ => {}
            }
        }
        self.tree.boxes[index].used_margin = margins;
        let x = margins.left;
        let y = margins.top;
        self.tree.boxes[index].content_box = Rect::new(
            x + border.left + padding.left,
            y + border.top + padding.top,
            content_width,
            0.0,
        );
        self.tree.boxes[index].padding_box = Rect::new(
            x + border.left,
            y + border.top,
            content_width + padding.horizontal(),
            0.0,
        );
        self.tree.boxes[index].border_box = Rect::new(x, y, border_width, 0.0);

        let content_height = match &self.tree.boxes[index].kind {
            BoxType::Image(_) | BoxType::Form(_) => {
                let mut size = replaced_content_size(
                    &self.context,
                    &self.tree.boxes[index],
                    available_width,
                    available_height,
                );
                if matches!(self.tree.boxes[index].kind, BoxType::Image(_))
                    && style.height == Dimension::Auto
                    && size.width > EPSILON
                {
                    size.height *= content_width / size.width;
                }
                size.height
            }
            _ => match self.tree.boxes[index].style.display {
                Display::Flex => {
                    self.arrange_flex_children(id, content_width, available_height);
                    self.tree.boxes[index]
                        .children
                        .iter()
                        .map(|child| {
                            let child_box = &self.tree.boxes[child.0 as usize];
                            child_box.margin_box.bottom() - margin_top(child_box)
                        })
                        .fold(0.0_f32, max)
                }
                _ => self.arrange_block_children(id, content_width, available_height),
            },
        };
        let specified_height = match style.height {
            Dimension::Auto => None,
            value => Some(
                self.context
                    .dimension(value, style.font_size, available_height),
            ),
        };
        let mut border_height = specified_height
            .map(|height| {
                if style.box_sizing == BoxSizing::BorderBox {
                    height
                } else {
                    height + padding.vertical() + border.top + border.bottom
                }
            })
            .unwrap_or(content_height + padding.vertical() + border.top + border.bottom);
        border_height = max(border_height, padding.vertical() + border.vertical());
        let min_height = match style.min_height {
            Dimension::Auto => 0.0,
            value => {
                let minimum = self
                    .context
                    .dimension(value, style.font_size, available_height);
                if style.box_sizing == BoxSizing::BorderBox {
                    minimum
                } else {
                    minimum + padding.vertical() + border.top + border.bottom
                }
            }
        };
        border_height = max(border_height, min_height);
        let content_height = max(
            0.0,
            border_height - padding.vertical() - border.top - border.bottom,
        );
        self.set_box_heights(index, content_height, padding, border);
        border_height + margins.top + margins.bottom
    }

    fn set_box_heights(
        &mut self,
        index: usize,
        content_height: f32,
        padding: Edges<f32>,
        border: Edges<f32>,
    ) {
        let mut content = self.tree.boxes[index].content_box;
        content.height = content_height;
        let mut padding_box = self.tree.boxes[index].padding_box;
        padding_box.height = content_height + padding.vertical();
        let mut border_box = self.tree.boxes[index].border_box;
        border_box.height = padding_box.height + border.top + border.bottom;
        let margins = self.tree.boxes[index].used_margin;
        let margin_box = Rect::new(
            border_box.x - margins.left,
            border_box.y - margins.top,
            border_box.width + margins.horizontal(),
            border_box.height + margins.vertical(),
        );
        self.tree.boxes[index].content_box = content;
        self.tree.boxes[index].padding_box = padding_box;
        self.tree.boxes[index].border_box = border_box;
        self.tree.boxes[index].margin_box = margin_box;
    }

    fn arrange_block_children(&mut self, parent: LayoutId, width: f32, height: f32) -> f32 {
        let children = self.tree.boxes[parent.0 as usize].children.clone();
        let origin_x = self.tree.boxes[parent.0 as usize].content_box.x;
        let origin_y = self.tree.boxes[parent.0 as usize].content_box.y;
        let mut inline_run = Vec::new();
        let mut cursor = 0.0_f32;
        let mut pending_margin = 0.0_f32;
        let flush_inline = |engine: &mut Self, run: &mut Vec<LayoutId>, cursor: &mut f32| {
            if run.is_empty() {
                return;
            }
            let run_rect =
                engine.arrange_inline_run(parent, run, origin_x, origin_y + *cursor, width);
            *cursor += run_rect.height;
            run.clear();
        };
        for child in children {
            let child_index = child.0 as usize;
            if self.tree.boxes[child_index].is_block_container() {
                flush_inline(self, &mut inline_run, &mut cursor);
                let child_style = self.tree.boxes[child_index].style.clone();
                let child_margin_top =
                    self.context
                        .dimension(child_style.margin.top, child_style.font_size, width);
                let collapsed = if cursor == 0.0 && pending_margin == 0.0 {
                    child_margin_top
                } else if cursor == 0.0 {
                    pending_margin + child_margin_top
                } else if pending_margin == 0.0 {
                    child_margin_top
                } else if child_margin_top == 0.0 {
                    pending_margin
                } else {
                    collapse_margins(pending_margin, child_margin_top)
                };
                cursor += collapsed;
                let child_y = origin_y + cursor;
                self.arrange_child(child, origin_x, child_y, width, height);
                let child_margin_bottom =
                    self.context
                        .dimension(child_style.margin.bottom, child_style.font_size, width);
                cursor += self.tree.boxes[child_index].border_box.height;
                pending_margin = child_margin_bottom;
            } else {
                inline_run.push(child);
            }
        }
        flush_inline(self, &mut inline_run, &mut cursor);
        cursor + collapse_margins(0.0, pending_margin)
    }

    fn arrange_child(&mut self, child: LayoutId, parent_x: f32, y: f32, width: f32, height: f32) {
        self.arrange(child, width, height);
        let local_y = self.tree.boxes[child.0 as usize].border_box.y;
        translate_subtree(&mut self.tree.boxes, child, parent_x, y - local_y);
    }

    fn arrange_flex_children(&mut self, parent: LayoutId, width: f32, height: f32) {
        let parent_index = parent.0 as usize;
        let parent_style = self.tree.boxes[parent_index].style.clone();
        let origin_x = self.tree.boxes[parent_index].content_box.x;
        let origin_y = self.tree.boxes[parent_index].content_box.y;
        let children: Vec<_> = self.tree.boxes[parent_index]
            .children
            .iter()
            .copied()
            .collect();
        if parent_style.flex_direction == FlexDirection::Row {
            let gap = parent_style.column_gap;
            let fixed: f32 = children.iter().fold(0.0, |sum, child| {
                sum + self.tree.boxes[child.0 as usize]
                    .style
                    .width
                    .fixed()
                    .unwrap_or(
                        self.tree.boxes[child.0 as usize]
                            .intrinsic
                            .max_content_width,
                    )
            });
            let total_gap = gap * children.len().saturating_sub(1) as f32;
            let free = max(0.0, width - fixed - total_gap);
            let (offset, distributed_gap) =
                distribute_main_axis(parent_style.justify_content, free, children.len());
            let mut x = offset;
            for (index, child) in children.iter().copied().enumerate() {
                let child_width = self.tree.boxes[child.0 as usize]
                    .style
                    .width
                    .fixed()
                    .unwrap_or(
                        self.tree.boxes[child.0 as usize]
                            .intrinsic
                            .max_content_width,
                    );
                self.arrange_child(child, origin_x + x, origin_y, child_width, height);
                self.align_flex_cross_axis(child, origin_y, height, parent_style.align_items, true);
                x += child_width + gap + distributed_gap;
                if index + 1 == children.len() {
                    x -= distributed_gap;
                }
            }
        } else {
            let gap = parent_style.row_gap;
            let fixed: f32 = children.iter().fold(0.0, |sum, child| {
                sum + self.tree.boxes[child.0 as usize]
                    .style
                    .height
                    .fixed()
                    .unwrap_or(self.tree.boxes[child.0 as usize].intrinsic.natural_height)
            });
            let total_gap = gap * children.len().saturating_sub(1) as f32;
            let free = max(0.0, height - fixed - total_gap);
            let (offset, distributed_gap) =
                distribute_main_axis(parent_style.justify_content, free, children.len());
            let mut y = offset;
            for (index, child) in children.iter().copied().enumerate() {
                let child_height = self.tree.boxes[child.0 as usize]
                    .style
                    .height
                    .fixed()
                    .unwrap_or(self.tree.boxes[child.0 as usize].intrinsic.natural_height);
                self.arrange_child(child, origin_x, origin_y + y, width, child_height);
                self.align_flex_cross_axis(child, origin_x, width, parent_style.align_items, false);
                y += child_height + gap + distributed_gap;
                if index + 1 == children.len() {
                    y -= distributed_gap;
                }
            }
        }
    }

    fn align_flex_cross_axis(
        &mut self,
        child: LayoutId,
        _origin: f32,
        extent: f32,
        alignment: CrossAxisAlignment,
        horizontal: bool,
    ) {
        let box_ = &self.tree.boxes[child.0 as usize];
        let child_extent = if horizontal {
            box_.margin_box.height
        } else {
            box_.margin_box.width
        };
        let delta = match alignment {
            CrossAxisAlignment::Start | CrossAxisAlignment::Stretch => 0.0,
            CrossAxisAlignment::End => extent - child_extent,
            CrossAxisAlignment::Center => (extent - child_extent) * 0.5,
        };
        if horizontal {
            translate_subtree(&mut self.tree.boxes, child, 0.0, delta);
        } else {
            translate_subtree(&mut self.tree.boxes, child, delta, 0.0);
        }
    }

    fn finish_parent_boxes(&mut self, root: LayoutId) {
        let mut stack = vec![root];
        let mut order = Vec::new();
        while let Some(id) = stack.pop() {
            order.push(id);
            stack.extend(self.tree.boxes[id.0 as usize].children.iter().copied());
        }
        for parent in order.into_iter().rev() {
            let children = self.tree.boxes[parent.0 as usize].children.clone();
            let parent_index = parent.0 as usize;
            if self.tree.boxes[parent_index].kind == BoxType::Inline
                && self.tree.boxes[parent_index].style.display != Display::InlineBlock
            {
                let mut union = Rect::default();
                for child in children {
                    let child_box = &self.tree.boxes[child.0 as usize];
                    union = union.union(child_box.margin_box);
                    for fragment in &child_box.fragments {
                        union = union.union(fragment.bounds);
                    }
                }
                if union.width > 0.0 || union.height > 0.0 {
                    self.tree.boxes[parent_index].margin_box = union;
                    self.tree.boxes[parent_index].border_box = union;
                }
            }
        }
    }

    fn list_markers(&self) -> Vec<RenderMarker<'a>> {
        let mut markers = Vec::new();
        let mut ordinals = BTreeMap::new();
        let document = self.context.styles.document();
        for item in &self.tree.boxes {
            let computed = self.context.styles.computed(item.dom_node);
            if !matches!(document.get(item.dom_node), Some(Node::Element { .. }))
                || keyword(computed, Property::Display) != "list-item"
            {
                continue;
            }
            let parent = item
                .parent
                .map(|id| self.tree.boxes[id.0 as usize].dom_node);
            let start = parent
                .and_then(|id| match document.get(id) {
                    Some(Node::Element { attributes, .. }) => attribute_value(attributes, "start")
                        .and_then(|value| value.parse::<i64>().ok()),
                    _ => None,
                })
                .unwrap_or(1);
            let ordinal = ordinals.entry(parent).or_insert(start);
            if let Some(Node::Element { attributes, .. }) = document.get(item.dom_node) {
                if let Some(value) =
                    attribute_value(attributes, "value").and_then(|value| value.parse::<i64>().ok())
                {
                    *ordinal = value;
                }
            }
            let style_type = keyword(computed, Property::ListStyleType);
            let label = match style_type.as_str() {
                "none" => String::new(),
                "disc" => String::from("•"),
                "circle" => String::from("◦"),
                "square" => String::from("▪"),
                _ => format!("{}.", ordinal),
            };
            *ordinal = ordinal.saturating_add(1);
            if label.is_empty() {
                continue;
            }
            let width = self.context.fonts.measure_width(
                &item.style.font_family,
                item.style.font_size,
                &label,
            );
            // Use the first line's vertical position, including a nested inline
            // span or anchor. Nested blocks/lists establish their own lines.
            let first_line = item
                .children
                .iter()
                .find_map(|id| {
                    let child = &self.tree.boxes[id.0 as usize];
                    child
                        .fragments
                        .first()
                        .map(|fragment| fragment.bounds.y)
                        .or_else(|| {
                            (child.contributes_children_to_inline_flow()
                                && child.border_box.height > 0.0)
                                .then_some(child.border_box.y)
                        })
                })
                .unwrap_or(item.content_box.y);
            markers.push(RenderMarker {
                node_id: item.dom_node,
                bounds: Rect::new(
                    item.content_box.x - width - item.style.font_size * 0.5,
                    first_line,
                    width,
                    item.style.line_height,
                ),
                label,
                style: resolve_output_style(item),
            });
        }
        markers
    }

    fn flatten(&self, id: LayoutId, output: &mut Vec<RenderBox<'a>>) {
        let layout_box = &self.tree.boxes[id.0 as usize];
        let resolved = resolve_output_style(layout_box);
        let link = layout_box.link.clone();
        match &layout_box.kind {
            BoxType::Block => {
                output.push(RenderBox {
                    node_id: layout_box.dom_node,
                    bounds: layout_box.border_box,
                    style: resolved.clone(),
                    node_type: BoxType::Block,
                    link: link.clone(),
                });
            }
            BoxType::Inline => {
                if layout_box.border_box.width > 0.0 || layout_box.border_box.height > 0.0 {
                    output.push(RenderBox {
                        node_id: layout_box.dom_node,
                        bounds: layout_box.border_box,
                        style: resolved.clone(),
                        node_type: BoxType::Inline,
                        link: link.clone(),
                    });
                }
            }
            BoxType::Image(image) => {
                output.push(RenderBox {
                    node_id: layout_box.dom_node,
                    bounds: layout_box.content_box,
                    style: resolved.clone(),
                    node_type: BoxType::Image(image.clone()),
                    link: link.clone(),
                });
            }
            BoxType::Form(widget) => {
                output.push(RenderBox {
                    node_id: layout_box.dom_node,
                    bounds: layout_box.border_box,
                    style: resolved.clone(),
                    node_type: BoxType::Form(widget.clone()),
                    link: link.clone(),
                });
            }
            BoxType::TextRun(_) => {
                for fragment in &layout_box.fragments {
                    if !fragment.text.is_empty() {
                        output.push(RenderBox {
                            node_id: layout_box.dom_node,
                            bounds: fragment.bounds,
                            style: resolved.clone(),
                            node_type: BoxType::TextRun(fragment.text),
                            link: link.clone(),
                        });
                    }
                }
            }
        }
        for &child in &layout_box.children {
            self.flatten(child, output);
        }
    }
}

#[derive(Clone, Debug)]
enum InlineToken<'a> {
    Text {
        layout_id: LayoutId,
        text: &'a str,
        style: LayoutStyle<'a>,
    },
    Atomic {
        layout_id: LayoutId,
        style: LayoutStyle<'a>,
        size: RectSize,
        baseline: f32,
    },
    Break,
}

fn collect_inline_tokens<'a>(
    context: &GeometryContext<'_>,
    boxes: &[LayoutBox<'a>],
    ids: &[LayoutId],
    available: f32,
    arranged: bool,
) -> Vec<InlineToken<'a>> {
    let mut tokens = Vec::new();
    let mut stack: Vec<LayoutId> = ids.iter().rev().copied().collect();
    while let Some(id) = stack.pop() {
        if tokens.len() >= MAX_INLINE_TOKENS {
            break;
        }
        let item = &boxes[id.0 as usize];
        if matches!(context.styles.document().get(item.dom_node),
            Some(Node::Element { tag_name, .. }) if tag_name.eq_ignore_ascii_case("br"))
        {
            tokens.push(InlineToken::Break);
            continue;
        }
        match &item.kind {
            BoxType::TextRun(text) => tokens.push(InlineToken::Text {
                layout_id: id,
                text,
                style: item.style.clone(),
            }),
            BoxType::Image(_) | BoxType::Form(_) | BoxType::Inline
                if !matches!(item.kind, BoxType::Inline)
                    || item.style.display == Display::InlineBlock =>
            {
                let padding = resolve_edges(context, item.style.padding, &item.style, available);
                let margins = resolve_edges(context, item.style.margin, &item.style, available);
                let outer_x = padding.horizontal() + item.style.border.horizontal();
                let outer_y = padding.vertical() + item.style.border.vertical();
                let content = match item.kind {
                    BoxType::Image(_) | BoxType::Form(_) => {
                        replaced_content_size(context, item, available, context.viewport_height)
                    }
                    _ => RectSize::new(
                        item.intrinsic.max_content_width,
                        item.intrinsic.natural_height,
                    ),
                };
                let mut border_size = if arranged {
                    RectSize::new(item.border_box.width, item.border_box.height)
                } else {
                    RectSize::new(
                        used_border_dimension(
                            context,
                            item.style.width,
                            &item.style,
                            available,
                            content.width,
                            outer_x,
                        ),
                        used_border_dimension(
                            context,
                            item.style.height,
                            &item.style,
                            context.viewport_height,
                            content.height,
                            outer_y,
                        ),
                    )
                };
                if !arranged {
                    border_size.width = constrain_border_width(
                        context,
                        &item.style,
                        available,
                        border_size.width,
                        outer_x,
                    );
                    if matches!(item.kind, BoxType::Image(_))
                        && item.style.height == Dimension::Auto
                        && content.width > EPSILON
                    {
                        border_size.height = max(0.0, border_size.width - outer_x) * content.height
                            / content.width
                            + outer_y;
                    }
                }
                let baseline = if matches!(item.kind, BoxType::Form(_)) {
                    margins.top
                        + item.style.border.top
                        + padding.top
                        + max(
                            0.0,
                            (border_size.height - outer_y - item.style.line_height) * 0.5,
                        )
                        + text_baseline(&item.style)
                } else {
                    margins.top + border_size.height
                };
                tokens.push(InlineToken::Atomic {
                    layout_id: id,
                    style: item.style.clone(),
                    size: RectSize::new(
                        border_size.width + margins.horizontal(),
                        border_size.height + margins.vertical(),
                    ),
                    baseline,
                });
            }
            BoxType::Inline => stack.extend(item.children.iter().rev().copied()),
            _ => {}
        }
    }
    tokens
}

fn resolve_edges(
    context: &GeometryContext<'_>,
    edges: Edges<Dimension>,
    style: &LayoutStyle<'_>,
    width: f32,
) -> Edges<f32> {
    Edges::new(
        context.dimension(edges.top, style.font_size, width),
        context.dimension(edges.right, style.font_size, width),
        context.dimension(edges.bottom, style.font_size, width),
        context.dimension(edges.left, style.font_size, width),
    )
}

fn used_border_dimension(
    context: &GeometryContext<'_>,
    dimension: Dimension,
    style: &LayoutStyle<'_>,
    available: f32,
    natural: f32,
    outer: f32,
) -> f32 {
    if dimension == Dimension::Auto {
        return natural + outer;
    }
    let declared = context.dimension(dimension, style.font_size, available);
    match style.box_sizing {
        BoxSizing::BorderBox => max(declared, outer),
        BoxSizing::ContentBox => declared + outer,
    }
}

/// Shared by intrinsic measurement and arrangement so clamped images do not
/// leave an oversized intrinsic wrapper when the actual image is smaller.
fn constrain_border_width(
    context: &GeometryContext<'_>,
    style: &LayoutStyle<'_>,
    available: f32,
    width: f32,
    outer: f32,
) -> f32 {
    let minimum = if style.min_width == Dimension::Auto {
        outer
    } else {
        used_border_dimension(context, style.min_width, style, available, 0.0, outer)
    };
    let maximum = style
        .max_width
        .map(|value| used_border_dimension(context, value, style, available, 0.0, outer));
    max(
        minimum,
        maximum.map_or(width, |maximum| min(width, maximum)),
    )
}

/// Replaced content dimensions before min/max constraints. A single explicit
/// image dimension scales the other through the decoded intrinsic ratio.
fn replaced_content_size(
    context: &GeometryContext<'_>,
    item: &LayoutBox<'_>,
    width: f32,
    height: f32,
) -> RectSize {
    let style = &item.style;
    let padding = resolve_edges(context, style.padding, style, width);
    let outer_x = padding.horizontal() + style.border.horizontal();
    let outer_y = padding.vertical() + style.border.vertical();
    let natural = match &item.kind {
        BoxType::Form(widget) => form_content_size(context, style, widget),
        _ => item
            .image_size
            .filter(|size| size.width > 0.0 && size.height > 0.0)
            .unwrap_or(RectSize::new(160.0, 100.0)),
    };
    let mut result = RectSize::new(
        max(
            0.0,
            used_border_dimension(context, style.width, style, width, natural.width, outer_x)
                - outer_x,
        ),
        max(
            0.0,
            used_border_dimension(
                context,
                style.height,
                style,
                height,
                natural.height,
                outer_y,
            ) - outer_y,
        ),
    );
    if matches!(item.kind, BoxType::Image(_)) {
        if style.height == Dimension::Auto {
            result.height = result.width * natural.height / natural.width;
        } else if style.width == Dimension::Auto {
            result.width = result.height * natural.width / natural.height;
        }
    }
    result
}

/// Approximate ascent until the font interface exposes face ascent/descent.
fn text_baseline(style: &LayoutStyle<'_>) -> f32 {
    min(
        style.line_height,
        max(0.0, (style.line_height - style.font_size) * 0.5) + style.font_size * 0.8,
    )
}

fn text_intrinsic(
    context: &GeometryContext<'_>,
    style: &LayoutStyle<'_>,
    text: &str,
) -> (f32, f32) {
    if matches!(style.white_space, WhiteSpace::Pre | WhiteSpace::PreWrap) {
        let (lines, longest) = inline_lines(
            context,
            vec![InlineToken::Text {
                layout_id: LayoutId(0),
                text,
                style: style.clone(),
            }],
            None,
            false,
        );
        return (longest, lines.iter().map(|line| line.width).fold(0.0, max));
    }
    let full = text_advance(context, style, collapse_text(text));
    let mut longest = 0.0_f32;
    for word in text.split_ascii_whitespace() {
        longest = max(longest, text_advance(context, style, word));
    }
    (longest, full)
}

/// A single-line form widget's natural content size in device-independent
/// pixels.  Every branch uses the shared [`FontMeasurer`] so the default
/// sizing tracks the active face, and none of them allocate.
fn form_content_size(
    context: &GeometryContext<'_>,
    style: &LayoutStyle<'_>,
    widget: &FormWidget<'_>,
) -> RectSize {
    match widget.kind {
        FormWidgetKind::Submit | FormWidgetKind::Button => {
            let caption = if widget.label.is_empty() {
                widget.value
            } else {
                widget.label
            };
            let width = text_advance(context, style, caption) + BUTTON_INNER_PADDING;
            // Buttons are at least a comfortable tap target even with a short
            // or empty caption.
            RectSize::new(width.max(24.0), style.line_height)
        }
        FormWidgetKind::Select => {
            // A collapsed dropdown shows only the active option; sizing to it
            // keeps the control as narrow as its selected content.
            let width = text_advance(context, style, widget.label) + SELECT_ARROW_WIDTH;
            RectSize::new(width.max(24.0), style.line_height)
        }
        _ => {
            // Text-like inputs reserve `size` columns of the digit advance,
            // which is the standard "approx 20ch" fallback when unspecified.
            let columns = widget.size.unwrap_or(DEFAULT_TEXT_COLUMNS).clamp(1, 512);
            let digit = text_advance(context, style, "0").max(style.font_size * 0.5);
            RectSize::new(digit * columns as f32, style.line_height)
        }
    }
}

/// Content size plus padding and border, i.e. the widget's intrinsic border
/// box.  Used both for intrinsic measurement and for the replaced-inline token
/// that places the widget on a line.
fn form_border_size(
    context: &GeometryContext<'_>,
    style: &LayoutStyle<'_>,
    widget: &FormWidget<'_>,
) -> RectSize {
    let padding = Edges::new(
        context.dimension(style.padding.top, style.font_size, context.viewport_width),
        context.dimension(style.padding.right, style.font_size, context.viewport_width),
        context.dimension(
            style.padding.bottom,
            style.font_size,
            context.viewport_width,
        ),
        context.dimension(style.padding.left, style.font_size, context.viewport_width),
    );
    let content = form_content_size(context, style, widget);
    RectSize::new(
        content.width + padding.horizontal() + style.border.left + style.border.right,
        content.height + padding.vertical() + style.border.top + style.border.bottom,
    )
}

fn is_form_widget_tag(tag_name: &str) -> bool {
    tag_name.eq_ignore_ascii_case("input")
        || tag_name.eq_ignore_ascii_case("button")
        || tag_name.eq_ignore_ascii_case("select")
}

/// Classifies a form element and borrows the data needed to size and paint it.
///
/// Returns `None` for an `input` type the engine does not model as a widget
/// (including `hidden`, which is pruned before this point); the caller then
/// falls back to a plain inline/block box.
fn form_widget<'a>(
    document: &'a Document,
    tag_name: &str,
    attributes: &'a [Attribute],
    children: &[NodeId],
) -> Option<FormWidget<'a>> {
    let disabled = attributes
        .iter()
        .any(|attribute| attribute.name().eq_ignore_ascii_case("disabled"));
    let readonly = attributes
        .iter()
        .any(|attribute| attribute.name().eq_ignore_ascii_case("readonly"));
    if tag_name.eq_ignore_ascii_case("input") {
        let input_type = attribute_value(attributes, "type").unwrap_or("text");
        let kind = match input_type.to_ascii_lowercase().as_str() {
            "submit" => FormWidgetKind::Submit,
            "button" | "reset" => FormWidgetKind::Button,
            "search" => FormWidgetKind::Search,
            "password" => FormWidgetKind::Password,
            // Every single-line text-like input shares the same "size columns"
            // metric.  `email`, `tel`, etc. are not yet distinguished visually,
            // but they must not fall back to plain block flow.
            "text" | "email" | "tel" | "url" | "number" | "date" | "datetime-local" | "month"
            | "week" | "time" => FormWidgetKind::Text,
            // `hidden` is pruned before this point; checkboxes, radios, files,
            // ranges, and other non-text types are not modelled yet and fall
            // back to a plain box, reported via `form-control`.
            _ => return None,
        };
        let value = attribute_value(attributes, "value").unwrap_or("");
        return Some(FormWidget {
            kind,
            label: if kind.is_button() { value } else { "" },
            placeholder: attribute_value(attributes, "placeholder").unwrap_or(""),
            value: if kind.is_button() { "" } else { value },
            disabled,
            editable: !kind.is_button() && !readonly && !disabled,
            size: attribute_value(attributes, "size").and_then(|raw| raw.parse::<u32>().ok()),
        });
    }
    if tag_name.eq_ignore_ascii_case("button") {
        let caption = element_text(document, children)
            .filter(|text| !text.is_empty())
            .or_else(|| attribute_value(attributes, "value"))
            .unwrap_or("");
        return Some(FormWidget {
            kind: FormWidgetKind::Button,
            label: caption,
            placeholder: "",
            value: "",
            disabled,
            editable: false,
            size: None,
        });
    }
    // `<select>`: collapse to the selected (or first) option's caption.
    let selected = active_option_text(document, children);
    Some(FormWidget {
        kind: FormWidgetKind::Select,
        label: selected,
        placeholder: "",
        value: selected,
        disabled,
        editable: false,
        size: None,
    })
}

/// Depth-first search for the first non-empty text run in a subtree.
fn element_text<'a>(document: &'a Document, children: &[NodeId]) -> Option<&'a str> {
    let mut stack: Vec<NodeId> = children.iter().rev().copied().collect();
    while let Some(node_id) = stack.pop() {
        match document.get(node_id) {
            Some(Node::Text { content }) => {
                let trimmed = collapse_text(content);
                if !trimmed.is_empty() {
                    return Some(trimmed);
                }
            }
            Some(Node::Element { children, .. }) => {
                stack.extend(children.iter().rev().copied());
            }
            _ => {}
        }
    }
    None
}

/// Text of the option a collapsed `select` displays: the `selected` option if
/// one exists, otherwise the first option.
fn active_option_text<'a>(document: &'a Document, children: &[NodeId]) -> &'a str {
    let mut first: Option<&'a str> = None;
    let mut stack: Vec<NodeId> = children.iter().rev().copied().collect();
    while let Some(node_id) = stack.pop() {
        let Some(Node::Element {
            tag_name,
            attributes,
            children,
        }) = document.get(node_id)
        else {
            continue;
        };
        if tag_name.eq_ignore_ascii_case("option") {
            let text = element_text(document, children).unwrap_or("");
            if attributes
                .iter()
                .any(|attribute| attribute.name().eq_ignore_ascii_case("selected"))
            {
                return text;
            }
            if first.is_none() {
                first = Some(text);
            }
            continue;
        }
        // `optgroup` and friends nest their options.
        stack.extend(children.iter().rev().copied());
    }
    first.unwrap_or("")
}

/// A node is pruned from the layout tree when it cannot render: an explicit
/// `display: none`, the boolean `hidden` attribute, or `<input type="hidden">`.
fn is_pruned(node: &Node, style: &LayoutStyle<'_>) -> bool {
    if style.display == Display::None {
        return true;
    }
    let Node::Element {
        tag_name,
        attributes,
        ..
    } = node
    else {
        return false;
    };
    if attributes
        .iter()
        .any(|attribute| attribute.name().eq_ignore_ascii_case("hidden"))
    {
        return true;
    }
    tag_name.eq_ignore_ascii_case("input")
        && attribute_value(attributes, "type")
            .is_some_and(|value| value.eq_ignore_ascii_case("hidden"))
}

struct InlinePlacement<'a> {
    owner: LayoutId,
    text: Option<&'a str>,
    bounds: Rect,
    baseline: f32,
}

#[derive(Default)]
struct InlineLine<'a> {
    items: Vec<InlinePlacement<'a>>,
    width: f32,
    ascent: f32,
    descent: f32,
}

impl<'a> InlineLine<'a> {
    fn push(
        &mut self,
        owner: LayoutId,
        text: Option<&'a str>,
        size: RectSize,
        baseline: f32,
        separator: f32,
    ) {
        self.width += separator;
        self.items.push(InlinePlacement {
            owner,
            text,
            bounds: Rect::new(self.width, 0.0, size.width, size.height),
            baseline,
        });
        self.width += size.width;
        self.ascent = max(self.ascent, baseline);
        self.descent = max(self.descent, size.height - baseline);
    }

    fn height(&self) -> f32 {
        self.ascent + self.descent
    }
}

/// Measurement and arrangement share whitespace, wrapping and line metrics.
/// Only whitespace actually present in the DOM introduces a separator.
fn inline_lines<'a>(
    context: &GeometryContext<'_>,
    tokens: Vec<InlineToken<'a>>,
    available: Option<f32>,
    nowrap: bool,
) -> (Vec<InlineLine<'a>>, f32) {
    let mut lines = Vec::new();
    let mut line = InlineLine::default();
    let mut pending_space = false;
    let mut longest = 0.0_f32;
    for token in tokens {
        match token {
            InlineToken::Text {
                layout_id,
                text,
                style,
            } => {
                if matches!(style.white_space, WhiteSpace::Pre | WhiteSpace::PreWrap) {
                    pending_space = false;
                    for segment in text.split_inclusive('\n') {
                        let newline = segment.ends_with('\n');
                        let content = if newline {
                            segment
                                .strip_suffix('\n')
                                .unwrap()
                                .strip_suffix('\r')
                                .unwrap_or(&segment[..segment.len() - 1])
                        } else {
                            segment
                        };
                        for chunk in content.split_inclusive([' ', '\t']) {
                            let tab = chunk.ends_with('\t');
                            let chunk = chunk.strip_suffix('\t').unwrap_or(chunk);
                            let width = text_advance(context, &style, chunk);
                            let wrap = style.white_space == WhiteSpace::PreWrap
                                && !nowrap
                                && !line.items.is_empty()
                                && available
                                    .is_some_and(|limit| line.width + width > limit + EPSILON);
                            if wrap {
                                lines.push(core::mem::take(&mut line));
                            }
                            if !chunk.is_empty() {
                                line.push(
                                    layout_id,
                                    Some(chunk),
                                    RectSize::new(width, style.line_height),
                                    text_baseline(&style),
                                    0.0,
                                );
                            }
                            if tab {
                                let tab_width = max(1.0, text_advance(context, &style, " ") * 8.0);
                                line.width =
                                    ((line.width / tab_width) as u32 + 1) as f32 * tab_width;
                            }
                            longest = max(
                                longest,
                                if style.white_space == WhiteSpace::Pre {
                                    line.width
                                } else {
                                    width
                                },
                            );
                        }
                        // Empty preserved lines still have the current font's height.
                        line.ascent = max(line.ascent, text_baseline(&style));
                        line.descent = max(line.descent, style.line_height - text_baseline(&style));
                        if newline {
                            lines.push(core::mem::take(&mut line));
                        }
                    }
                    continue;
                }
                if text.as_bytes().first().is_some_and(u8::is_ascii_whitespace) {
                    pending_space = true;
                }
                for (index, word) in text.split_ascii_whitespace().enumerate() {
                    let width = text_advance(context, &style, word);
                    longest = max(longest, width);
                    let separator = if !line.items.is_empty() && (pending_space || index > 0) {
                        text_advance(context, &style, " ")
                    } else {
                        0.0
                    };
                    let wrap = !nowrap
                        && style.white_space != WhiteSpace::Nowrap
                        && available
                            .is_some_and(|limit| line.width + separator + width > limit + EPSILON)
                        && !line.items.is_empty();
                    if wrap {
                        lines.push(core::mem::take(&mut line));
                    }
                    line.push(
                        layout_id,
                        Some(word),
                        RectSize::new(width, style.line_height),
                        text_baseline(&style),
                        if wrap { 0.0 } else { separator },
                    );
                    pending_space = false;
                }
                if text.as_bytes().last().is_some_and(u8::is_ascii_whitespace) {
                    pending_space = true;
                }
            }
            InlineToken::Atomic {
                layout_id,
                size,
                style,
                baseline,
            } => {
                longest = max(longest, size.width);
                let separator = if pending_space && !line.items.is_empty() {
                    text_advance(context, &style, " ")
                } else {
                    0.0
                };
                let wrap = !nowrap
                    && available
                        .is_some_and(|limit| line.width + separator + size.width > limit + EPSILON)
                    && !line.items.is_empty();
                if wrap {
                    lines.push(core::mem::take(&mut line));
                }
                line.push(
                    layout_id,
                    None,
                    size,
                    baseline,
                    if wrap { 0.0 } else { separator },
                );
                pending_space = false;
            }
            InlineToken::Break => {
                lines.push(core::mem::take(&mut line));
                pending_space = false;
            }
        }
    }
    if !line.items.is_empty() || line.height() > 0.0 {
        lines.push(line);
    }
    (lines, longest)
}

fn measure_inline<'a>(
    context: &GeometryContext<'a>,
    boxes: &[LayoutBox<'a>],
    ids: &[LayoutId],
    available: Option<f32>,
) -> (f32, f32, f32) {
    let tokens = collect_inline_tokens(
        context,
        boxes,
        ids,
        available.unwrap_or(context.viewport_width),
        false,
    );
    let (lines, longest) = inline_lines(context, tokens, available, false);
    (
        longest,
        lines.iter().map(|line| line.width).fold(0.0, max),
        lines.iter().map(InlineLine::height).sum(),
    )
}

impl<'a> LayoutEngine<'a> {
    fn arrange_inline_run(
        &mut self,
        parent: LayoutId,
        ids: &[LayoutId],
        origin_x: f32,
        origin_y: f32,
        available: f32,
    ) -> Rect {
        // Atomic inline boxes establish their own formatting context before
        // being positioned in the containing visual line.
        let tokens = collect_inline_tokens(&self.context, &self.tree.boxes, ids, available, false);
        for token in tokens {
            match token {
                InlineToken::Atomic { layout_id, .. } => {
                    self.arrange(layout_id, available, self.context.viewport_height);
                }
                InlineToken::Text { layout_id, .. } => {
                    self.tree.boxes[layout_id.0 as usize].fragments.clear()
                }
                InlineToken::Break => {}
            }
        }
        let style = self.tree.boxes[parent.0 as usize].style.clone();
        let tokens = collect_inline_tokens(&self.context, &self.tree.boxes, ids, available, true);
        let (lines, _) = inline_lines(
            &self.context,
            tokens,
            Some(available),
            style.white_space == WhiteSpace::Nowrap,
        );
        let mut y = 0.0;
        let mut widest = 0.0_f32;
        let mut owners: Vec<LayoutId> = Vec::new();
        for mut line in lines {
            if !line.items.is_empty() {
                line.ascent = max(line.ascent, text_baseline(&style));
                line.descent = max(line.descent, style.line_height - text_baseline(&style));
            } else {
                line.ascent = style.line_height;
                line.descent = 0.0;
            }
            let offset = match style.text_align {
                TextAlign::Center => max(0.0, available - line.width) * 0.5,
                TextAlign::End => max(0.0, available - line.width),
                _ => 0.0,
            };
            widest = max(widest, line.width);
            let height = line.height();
            for item in line.items {
                let rect = absolute(
                    item.bounds,
                    origin_x + offset,
                    origin_y + y + line.ascent - item.baseline,
                );
                let index = item.owner.0 as usize;
                if let Some(text) = item.text {
                    self.tree.boxes[index]
                        .fragments
                        .push(TextFragment { text, bounds: rect });
                    if !owners.contains(&item.owner) {
                        owners.push(item.owner);
                    }
                } else {
                    let local = self.tree.boxes[index].margin_box;
                    translate_subtree(
                        &mut self.tree.boxes,
                        item.owner,
                        rect.x - local.x,
                        rect.y - local.y,
                    );
                }
            }
            y += height;
        }
        for owner in owners {
            let item = &mut self.tree.boxes[owner.0 as usize];
            let union = item
                .fragments
                .iter()
                .fold(Rect::default(), |bounds, fragment| {
                    bounds.union(fragment.bounds)
                });
            item.content_box = union;
            item.padding_box = union;
            item.border_box = union;
            item.margin_box = union;
        }
        Rect::new(origin_x, origin_y, widest, y)
    }
}

fn absolute(rect: Rect, x: f32, y: f32) -> Rect {
    Rect::new(rect.x + x, rect.y + y, rect.width, rect.height)
}

fn translate_subtree(boxes: &mut [LayoutBox<'_>], root: LayoutId, dx: f32, dy: f32) {
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        let index = id.0 as usize;
        translate_rect(&mut boxes[index].content_box, dx, dy);
        translate_rect(&mut boxes[index].padding_box, dx, dy);
        translate_rect(&mut boxes[index].border_box, dx, dy);
        translate_rect(&mut boxes[index].margin_box, dx, dy);
        for fragment in &mut boxes[index].fragments {
            translate_rect(&mut fragment.bounds, dx, dy);
        }
        stack.extend(boxes[index].children.iter().copied());
    }
}

fn translate_rect(rect: &mut Rect, dx: f32, dy: f32) {
    rect.x += dx;
    rect.y += dy;
}

fn distribute_main_axis(alignment: MainAxisAlignment, free: f32, count: usize) -> (f32, f32) {
    match alignment {
        MainAxisAlignment::Start => (0.0, 0.0),
        MainAxisAlignment::End => (free, 0.0),
        MainAxisAlignment::Center => (free * 0.5, 0.0),
        MainAxisAlignment::SpaceBetween if count > 1 => (0.0, free / (count - 1) as f32),
        MainAxisAlignment::SpaceAround if count > 0 => {
            let gap = free / count as f32;
            (gap * 0.5, gap)
        }
        MainAxisAlignment::SpaceEvenly if count > 0 && count < usize::MAX => {
            let gap = free / (count + 1) as f32;
            (gap, gap)
        }
        _ => (free * 0.5, 0.0),
    }
}

fn collapse_margins(first: f32, second: f32) -> f32 {
    let positive = max(0.0, first).max(max(0.0, second));
    let negative = min(0.0, first).min(min(0.0, second));
    positive + negative
}

fn margin_top(layout_box: &LayoutBox<'_>) -> f32 {
    layout_box.content_box.y
        - layout_box.border_box.y
        - layout_box.style.padding.top.resolve(0.0, 0.0)
}

fn resolve_output_style<'src>(layout_box: &LayoutBox<'src>) -> ResolvedStyle<'src> {
    let style = &layout_box.style;
    let width = layout_box.content_box.width;
    ResolvedStyle {
        display: style.display,
        color: style.color,
        background_color: style.background_color,
        font_family: style.font_family.clone(),
        font_size: style.font_size,
        line_height: style.line_height,
        font_weight: style.font_weight,
        italic: style.italic,
        underline: style.underline,
        line_through: style.line_through,
        white_space: style.white_space,
        text_align: style.text_align,
        box_sizing: style.box_sizing,
        width: match style.width {
            Dimension::Auto => None,
            _ => Some(width),
        },
        height: match style.height {
            Dimension::Auto => None,
            _ => Some(layout_box.content_box.height),
        },
        margin: layout_box.used_margin,
        padding: layout_box.used_padding,
        border: style.border,
        border_color: style.border_color,
        flex_direction: style.flex_direction,
        justify_content: style.justify_content,
        align_items: style.align_items,
        row_gap: style.row_gap,
        column_gap: style.column_gap,
    }
}

fn attribute_value<'a>(attributes: &'a [Attribute], name: &str) -> Option<&'a str> {
    attributes
        .iter()
        .find(|attribute| attribute.name().eq_ignore_ascii_case(name))
        .map(Attribute::value)
}

fn css_value<'a>(
    style: Option<&'a ComputedStyle>,
    property: Property,
) -> Option<&'a PropertyValue> {
    style?.value(&property)
}

fn css_px(style: Option<&ComputedStyle>, property: Property) -> Option<f32> {
    match css_value(style, property)? {
        PropertyValue::LengthPx(value) => Some(*value as f32),
        PropertyValue::Percentage(value) => Some(*value as f32 / 100.0),
        _ => None,
    }
}

fn css_length(
    style: Option<&ComputedStyle>,
    property: Property,
    viewport_width: f32,
    viewport_height: f32,
    fallback: f32,
) -> Option<f32> {
    match css_value(style, property)? {
        PropertyValue::LengthPx(value) => Some(*value as f32),
        PropertyValue::LengthEm(value) => Some(fallback * *value as f32 / 1000.0),
        PropertyValue::LengthVw(value) => Some(viewport_width * *value as f32 / 100_000.0),
        PropertyValue::LengthVh(value) => Some(viewport_height * *value as f32 / 100_000.0),
        PropertyValue::Percentage(value) => Some(fallback * *value as f32 / 10_000.0),
        _ => None,
    }
}

fn int_rect(rect: Rect) -> IntRect {
    let x = floor_i32(rect.x);
    let y = floor_i32(rect.y);
    IntRect::new(
        x,
        y,
        ceil_u32(rect.width + rect.x - x as f32),
        ceil_u32(rect.height + rect.y - y as f32),
    )
}

fn floor_i32(value: f32) -> i32 {
    let value = value.clamp(i32::MIN as f32, i32::MAX as f32);
    if value < 0.0 && value as i32 as f32 != value {
        value as i32 - 1
    } else {
        value as i32
    }
}

fn ceil_u32(value: f32) -> u32 {
    let clamped = value.max(0.0).min(u32::MAX as f32);
    let truncated = clamped as u32;
    if truncated as f32 + EPSILON < clamped {
        truncated + 1
    } else {
        truncated
    }
}

fn dimension(style: Option<&ComputedStyle>, property: Property) -> Dimension {
    match css_value(style, property) {
        Some(PropertyValue::LengthPx(value)) => Dimension::Px(*value as f32),
        Some(PropertyValue::LengthEm(value)) => Dimension::Px(*value as f32 / 1000.0 * 16.0),
        Some(PropertyValue::LengthRem(value)) => Dimension::Px(*value as f32 / 1000.0 * 16.0),
        Some(PropertyValue::LengthVw(value)) => Dimension::Vw(*value as f32 / 1000.0),
        Some(PropertyValue::LengthVh(value)) => Dimension::Vh(*value as f32 / 1000.0),
        Some(PropertyValue::Percentage(value)) => Dimension::Percent(*value as f32 / 10_000.0),
        _ => Dimension::Auto,
    }
}

fn optional_dimension(style: Option<&ComputedStyle>, property: Property) -> Option<Dimension> {
    match dimension(style, property) {
        Dimension::Auto => None,
        value => Some(value),
    }
}

fn display(style: Option<&ComputedStyle>) -> Display {
    match keyword(style, Property::Display).as_str() {
        "block" | "list-item" | "table" | "table-row" | "table-cell" => Display::Block,
        "flex" => Display::Flex,
        "inline" => Display::Inline,
        "inline-block" | "inline-flex" => Display::InlineBlock,
        "none" => Display::None,
        _ => Display::Other,
    }
}

fn keyword(style: Option<&ComputedStyle>, property: Property) -> String {
    match css_value(style, property) {
        Some(PropertyValue::Keyword(value)) | Some(PropertyValue::Raw(value)) => {
            value.to_ascii_lowercase()
        }
        Some(PropertyValue::Normal) => String::from("normal"),
        _ => String::new(),
    }
}

fn css_color(style: Option<&ComputedStyle>, property: Property, fallback: Color) -> Color {
    match css_value(style, property) {
        Some(PropertyValue::Color(crate::css::Color::Rgb(red, green, blue))) => {
            Color::rgb(*red, *green, *blue)
        }
        Some(PropertyValue::Color(crate::css::Color::Rgba(red, green, blue, alpha))) => {
            Color::rgba(*red, *green, *blue, *alpha)
        }
        Some(PropertyValue::Color(crate::css::Color::Transparent)) => Color::TRANSPARENT,
        _ => fallback,
    }
}

fn font_weight(style: Option<&ComputedStyle>) -> u16 {
    match css_value(style, Property::FontWeight) {
        Some(PropertyValue::LengthPx(value)) => (*value).clamp(1, u16::MAX as i32) as u16,
        Some(PropertyValue::Keyword(value)) if value.eq_ignore_ascii_case("bold") => 700,
        Some(PropertyValue::Keyword(value)) if value.eq_ignore_ascii_case("normal") => 400,
        _ => 400,
    }
}

fn white_space(style: Option<&ComputedStyle>) -> WhiteSpace {
    match keyword(style, Property::WhiteSpace).as_str() {
        "nowrap" => WhiteSpace::Nowrap,
        "pre" => WhiteSpace::Pre,
        "pre-wrap" => WhiteSpace::PreWrap,
        _ => WhiteSpace::Normal,
    }
}

fn text_align(style: Option<&ComputedStyle>) -> TextAlign {
    match keyword(style, Property::TextAlign).as_str() {
        "end" | "right" => TextAlign::End,
        "center" => TextAlign::Center,
        "justify" => TextAlign::Justify,
        _ => TextAlign::Start,
    }
}

fn main_axis_alignment(style: Option<&ComputedStyle>) -> MainAxisAlignment {
    match keyword(style, Property::JustifyContent).as_str() {
        "end" | "flex-end" | "right" => MainAxisAlignment::End,
        "center" => MainAxisAlignment::Center,
        "space-between" => MainAxisAlignment::SpaceBetween,
        "space-around" => MainAxisAlignment::SpaceAround,
        "space-evenly" => MainAxisAlignment::SpaceEvenly,
        _ => MainAxisAlignment::Start,
    }
}

fn cross_axis_alignment(style: Option<&ComputedStyle>) -> CrossAxisAlignment {
    match keyword(style, Property::AlignItems).as_str() {
        "start" | "flex-start" => CrossAxisAlignment::Start,
        "end" | "flex-end" => CrossAxisAlignment::End,
        "center" => CrossAxisAlignment::Center,
        _ => CrossAxisAlignment::Stretch,
    }
}

fn text_advance(context: &GeometryContext<'_>, style: &LayoutStyle<'_>, text: &str) -> f32 {
    context
        .fonts
        .measure_width(&style.font_family, style.font_size, text)
        .max(0.0)
}

fn collapse_text(text: &str) -> &str {
    text.trim()
}

fn min(first: f32, second: f32) -> f32 {
    if first < second {
        first
    } else {
        second
    }
}

fn max(first: f32, second: f32) -> f32 {
    if first > second {
        first
    } else {
        second
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css::collect_embedded_stylesheets;
    use golden_fish::parse_html;

    struct FixedFont;

    impl FontMeasurer for FixedFont {
        fn measure_width(&self, _family: &str, font_size: f32, text: &str) -> f32 {
            text.chars().count() as f32 * font_size * 0.5
        }
    }

    fn layout(html: &str, width: u32) -> LayoutResult<'_> {
        let document = Box::leak(Box::new(parse_html(html).unwrap()));
        let context = Box::leak(Box::new(StyleContext::build(
            document,
            &collect_embedded_stylesheets(document),
        )));
        let fonts = Box::leak(Box::new(FixedFont));
        let images = Box::leak(Box::new(NoImageMetrics));
        layout_document(document, context, Size::new(width, 600), fonts, images)
    }

    #[test]
    fn border_box_and_margin_collapse_produce_expected_geometry() {
        let result = layout(
            "<style>
                body { margin: 0; }
                .group { width: 200px; }
                .first { height: 20px; margin-bottom: 12px; }
                .second { height: 30px; margin-top: 20px; box-sizing: border-box;
                          width: 120px; padding: 5px; border: 2px solid black; }
             </style><div class=group><div class=first></div><div class=second></div></div>",
            320,
        );
        let blocks: Vec<_> = result
            .render_boxes
            .iter()
            .filter(|item| item.node_type == BoxType::Block)
            .collect();
        let first = blocks
            .iter()
            .find(|item| item.style.margin.bottom == 12.0)
            .expect("first block");
        let second = blocks
            .iter()
            .find(|item| item.style.border.top == 2.0)
            .expect("second block");
        assert_eq!(first.bounds.y + first.bounds.height + 20.0, second.bounds.y);
        assert_eq!(second.bounds.width, 120.0);
    }

    #[test]
    fn text_wraps_into_borrowed_line_fragments() {
        let result = layout(
            "<style>body { margin: 0; } p { display: block; width: 50px; font-size: 10px; line-height: 12px; }</style><p>one two three</p>",
            200,
        );
        let runs: Vec<_> = result
            .render_boxes
            .iter()
            .filter_map(|item| match &item.node_type {
                BoxType::TextRun(text) => Some((*text, item.bounds)),
                _ => None,
            })
            .collect();
        assert_eq!(runs.len(), 3);
        assert!(runs[0].1.y < runs[2].1.y);
        assert_eq!(runs[2].0, "three");
    }

    #[test]
    fn flex_row_applies_justification_and_centering() {
        let result = layout(
            "<style>
              body { margin: 0; }
              .row { display: flex; width: 300px; height: 40px;
                     justify-content: space-between; align-items: center; }
              .item { width: 20px; height: 10px; }
             </style><div class=row><div class=item></div><div class=item></div></div>",
            320,
        );
        let items: Vec<_> = result
            .render_boxes
            .iter()
            .filter(|item| item.bounds.width == 20.0 && item.bounds.height == 10.0)
            .collect();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].bounds.x, 0.0);
        assert_eq!(items[1].bounds.x, 280.0);
    }

    #[test]
    fn viewport_units_are_parsed() {
        let document =
            parse_html("<style>p { width: 50vw; height: 25vh; }</style><p></p>").expect("DOM");
        let styles = StyleContext::build(&document, &collect_embedded_stylesheets(&document));
        let paragraph = document.find_first_element("p").expect("p");
        assert!(matches!(
            styles.style_for(paragraph).unwrap().value(&Property::Width),
            Some(PropertyValue::LengthVw(50_000))
        ));
        assert!(matches!(
            styles
                .style_for(paragraph)
                .unwrap()
                .value(&Property::Height),
            Some(PropertyValue::LengthVh(25_000))
        ));
    }

    #[test]
    fn render_boxes_convert_to_a_finalized_document_scene() {
        let result = layout(
            "<style>body { margin: 0; } p { width: 50px; }</style><p>one two</p>",
            200,
        );
        let scene = into_document_scene(7, Size::new(200, 600), &result);
        let text_count = scene
            .objects
            .iter()
            .filter(|object| matches!(object.kind, RenderObjectKind::Text { .. }))
            .count();
        assert_eq!(text_count, 2);
        assert!(scene
            .objects
            .windows(2)
            .all(|pair| pair[0].paint_order <= pair[1].paint_order));
    }

    fn count_form_boxes(result: &LayoutResult<'_>) -> usize {
        result
            .geometry
            .boxes
            .iter()
            .filter(|layout_box| matches!(layout_box.kind, BoxType::Form(_)))
            .count()
    }

    #[test]
    fn hidden_input_creates_zero_layout_boxes() {
        let result = layout(
            "<style>body { margin: 0; }</style><form><input type=hidden name=csrf value=abc></form>",
            320,
        );
        assert_eq!(count_form_boxes(&result), 0);
        assert!(!result
            .render_boxes
            .iter()
            .any(|item| matches!(item.node_type, BoxType::Form(_))));
        let scene = into_document_scene(1, Size::new(320, 600), &result);
        assert!(!scene
            .objects
            .iter()
            .any(|object| matches!(object.kind, RenderObjectKind::Control { .. })));
    }

    #[test]
    fn hidden_input_does_not_shift_adjacent_flow_boxes() {
        let with_hidden = layout(
            "<style>body { margin: 0; } div { display: block; height: 20px; }</style><div id=before></div><input type=hidden name=x><div id=after></div>",
            320,
        );
        let without_hidden = layout(
            "<style>body { margin: 0; } div { display: block; height: 20px; }</style><div id=before></div><div id=after></div>",
            320,
        );
        // The last block in paint order is `#after`; the hidden input must not
        // insert a box or push it down.
        let last_block = |result: &LayoutResult<'_>| {
            result
                .render_boxes
                .iter()
                .filter(|item| item.node_type == BoxType::Block)
                .next_back()
                .expect("a block box")
                .bounds
        };
        assert_eq!(last_block(&with_hidden), last_block(&without_hidden));
    }

    #[test]
    fn hidden_attribute_and_display_none_prune_subtrees() {
        let result = layout(
            "<style>body { margin: 0; }</style><div hidden><p>gone</p></div><span style='display:none'>also</span><p>kept</p>",
            320,
        );
        assert!(!result.render_boxes.iter().any(|item| {
            matches!(&item.node_type, BoxType::TextRun(text) if *text == "gone" || *text == "also")
        }));
        assert!(result
            .render_boxes
            .iter()
            .any(|item| { matches!(&item.node_type, BoxType::TextRun(text) if *text == "kept") }));
    }

    #[test]
    fn text_input_uses_size_attribute_or_default_columns() {
        let width = |result: &LayoutResult<'_>| {
            result
                .geometry
                .boxes
                .iter()
                .find(|layout_box| matches!(layout_box.kind, BoxType::Form(_)))
                .expect("text input box")
                .intrinsic
                .max_content_width
        };
        let default = layout(
            "<style>body{margin:0} input{font-size:16px}</style><input type=text>",
            400,
        );
        let sized = layout(
            "<style>body{margin:0} input{font-size:16px}</style><input type=text size=40>",
            400,
        );
        let default_width = width(&default);
        assert!(default_width > 0.0);
        // `size=40` reserves twice the columns of the 20-column default.
        assert!(
            width(&sized) > default_width * 1.5,
            "size=40 should exceed the default width"
        );
    }

    #[test]
    fn submit_button_width_tracks_its_label() {
        let width = |result: &LayoutResult<'_>| {
            result
                .geometry
                .boxes
                .iter()
                .find(|layout_box| matches!(layout_box.kind, BoxType::Form(_)))
                .expect("button box")
                .intrinsic
                .max_content_width
        };
        let short = layout(
            "<style>body{margin:0} input{font-size:16px}</style><input type=submit value=Go>",
            400,
        );
        let long = layout(
            "<style>body{margin:0} input{font-size:16px}</style><input type=submit value='Submit the form now'>",
            400,
        );
        assert!(width(&long) > width(&short));
    }

    #[test]
    fn select_collapses_to_active_option_width() {
        // Returns the width of the first form box, avoiding a closure that
        // would hand out a borrowed `LayoutBox`.
        let select_width = |result: &LayoutResult<'_>| -> f32 {
            result
                .geometry
                .boxes
                .iter()
                .find(|layout_box| matches!(layout_box.kind, BoxType::Form(_)))
                .expect("select box")
                .intrinsic
                .max_content_width
        };
        let result = layout(
            "<style>body{margin:0} select{font-size:16px}</style><select><option>short</option><option>a much longer option label</option></select>",
            600,
        );
        let select = result
            .geometry
            .boxes
            .iter()
            .find(|layout_box| matches!(layout_box.kind, BoxType::Form(_)))
            .expect("select box");
        let widget = match &select.kind {
            BoxType::Form(widget) => widget,
            _ => unreachable!(),
        };
        // Collapsed to the first option because none is `selected`.
        assert_eq!(widget.label, "short");
        let collapsed_width = select.intrinsic.max_content_width;
        assert!(collapsed_width > 0.0);
        // A collapsed select is narrower than one showing the wide option.
        let wide = layout(
            "<style>body{margin:0} select{font-size:16px}</style><select><option>a much longer option label</option></select>",
            600,
        );
        assert!(collapsed_width < select_width(&wide));
    }

    #[test]
    fn form_widget_emits_a_control_with_user_agent_defaults() {
        let result = layout(
            "<style>body{margin:0} input{font-size:16px;border:none;background-color:transparent}</style><input type=text value=hello>",
            400,
        );
        let scene = into_document_scene(1, Size::new(400, 600), &result);
        let control = scene
            .objects
            .iter()
            .find_map(|object| match &object.kind {
                RenderObjectKind::Control {
                    value,
                    border_width,
                    background,
                    ..
                } => Some((value.clone(), *border_width, *background)),
                _ => None,
            })
            .expect("retained control object");
        assert_eq!(control.0, "hello");
        // No author border: the widget still draws an outline.
        assert!(control.1 >= 1);
        // Transparent author background: the default field fill is used.
        assert_ne!(control.2, Color::TRANSPARENT);
        assert!(scene
            .objects
            .iter()
            .any(|object| matches!(&object.interaction, Some(RenderInteraction::Control { .. }))));
    }

    /// Border-box width of the first form widget in the flattened IR, which is
    /// what the retained scene paints for the control.
    fn form_border_width(result: &LayoutResult<'_>) -> f32 {
        result
            .render_boxes
            .iter()
            .find(|item| matches!(item.node_type, BoxType::Form(_)))
            .expect("a form widget box")
            .bounds
            .width
    }

    #[test]
    fn form_control_explicit_width_honours_box_sizing() {
        // `content-box` adds used padding + border to the declared content
        // width; `border-box` treats the declared width as the outer edge.
        let content_box = layout(
            "<style>body{margin:0} input{width:100px;box-sizing:content-box;padding:5px;border:2px solid black}</style><input type=text>",
            400,
        );
        let border_box = layout(
            "<style>body{margin:0} input{width:100px;box-sizing:border-box;padding:5px;border:2px solid black}</style><input type=text>",
            400,
        );
        // 100 content + 2*5 padding + 2*2 border = 114.
        assert_eq!(form_border_width(&content_box), 114.0);
        // Declared outer width is authoritative under `border-box`.
        assert_eq!(form_border_width(&border_box), 100.0);
    }

    #[test]
    fn text_like_input_types_are_modelled_and_sized() {
        // Every single-line text-like type must produce a real widget with a
        // non-zero intrinsic box rather than falling back to plain block flow.
        for input_type in [
            "email",
            "tel",
            "url",
            "number",
            "date",
            "datetime-local",
            "month",
            "week",
            "time",
        ] {
            let html = format!(
                "<style>body{{margin:0}} input{{font-size:16px}}</style><input type={input_type}>"
            );
            let result = layout(&html, 400);
            let widget_box = result
                .geometry
                .boxes
                .iter()
                .find(|layout_box| matches!(layout_box.kind, BoxType::Form(_)))
                .unwrap_or_else(|| panic!("`{input_type}` should be a form widget"));
            assert!(
                widget_box.intrinsic.max_content_width > 0.0,
                "`{input_type}` needs a non-zero intrinsic width"
            );
            assert!(widget_box.intrinsic.natural_height > 0.0);
            assert!(
                form_border_width(&result) > 0.0,
                "`{input_type}` needs a non-zero painted box"
            );
        }
    }

    #[test]
    fn unsupported_input_types_do_not_become_widgets() {
        // Checkboxes and files still fall back to plain boxes; only the
        // single-line text-like family is modelled.
        for input_type in ["checkbox", "radio", "file", "range"] {
            let html = format!("<style>body{{margin:0}}</style><input type={input_type}>");
            let result = layout(&html, 320);
            assert_eq!(
                count_form_boxes(&result),
                0,
                "`{input_type}` must not be reported as a modelled widget"
            );
        }
    }

    /// Text of every rendered `TextRun` in paint order.
    fn rendered_words(result: &LayoutResult<'_>) -> Vec<String> {
        result
            .render_boxes
            .iter()
            .filter_map(|item| match &item.node_type {
                BoxType::TextRun(text) => Some((*text).to_string()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn form_and_center_are_block_containers() {
        // `form` and `center` are block-level in the HTML UA stylesheet.  If
        // they fall back to the CSS `display: inline` initial value, their
        // block children are orphaned by inline flow and disappear.
        for (tag, text) in [("form", "inform"), ("center", "centered")] {
            let html = format!("<style>body{{margin:0}}</style><{tag}><div>{text}</div></{tag}>");
            let result = layout(&html, 320);
            assert!(
                rendered_words(&result).contains(&text.to_string()),
                "`<{tag}>` must keep its block child visible"
            );
        }
    }

    #[test]
    fn unmodelled_display_values_keep_their_subtree() {
        // `display: -webkit-box` is not one of the keywords this engine
        // models.  The box must still be arranged as a block container so its
        // subtree survives, rather than being silently dropped.
        let result = layout(
            "<style>body{margin:0}</style><p><span style='display:-webkit-box'>WEBKIT</span></p>",
            320,
        );
        assert!(rendered_words(&result).contains(&String::from("WEBKIT")));
        // Every block box carries real geometry; an unmodelled display must
        // not leave a zero-area, unarranged record.
        assert!(result.geometry.boxes.iter().all(|layout_box| {
            layout_box.kind != BoxType::Block || layout_box.border_box.width > 0.0
        }));
    }

    #[test]
    fn centered_submit_button_keeps_its_intrinsic_box() {
        // Mirrors Google's hero form: a `center` block wrapping submit inputs.
        // The buttons must keep both intrinsic sizing and a non-zero box.
        let result = layout(
            "<style>body{margin:0} input{font-size:16px}</style><center><input type=submit value='Google Search'><input type=submit value='I am Feeling Lucky'></center>",
            1056,
        );
        let buttons: Vec<_> = result
            .render_boxes
            .iter()
            .filter(|item| matches!(item.node_type, BoxType::Form(_)))
            .collect();
        assert_eq!(buttons.len(), 2);
        let total = buttons
            .iter()
            .map(|button| button.bounds.width)
            .sum::<f32>();
        assert!((buttons[0].bounds.x - (1056.0 - total) * 0.5).abs() < EPSILON);
        assert!((buttons[1].bounds.x - buttons[0].bounds.right()).abs() < EPSILON);
        assert_eq!(buttons[0].bounds.y, buttons[1].bounds.y);
        for button in buttons {
            assert!(button.bounds.width > 0.0, "button width must be non-zero");
            assert!(button.bounds.height > 0.0, "button height must be non-zero");
        }
    }
    fn block_with_width<'a, 'b>(result: &'b LayoutResult<'a>, width: f32) -> &'b LayoutBox<'a> {
        result
            .geometry
            .boxes
            .iter()
            .find(|item| item.kind == BoxType::Block && item.style.width == Dimension::Px(width))
            .unwrap()
    }

    #[test]
    fn auto_margins_use_containing_content_width_and_border_box() {
        let result = layout("<style>body{margin:0} main{width:400px;padding:10px;border:2px solid} div{width:100px;padding:10px;border:2px solid;margin:0 auto}</style><main><div>logo</div></main>", 600);
        let child = block_with_width(&result, 100.0);
        assert_eq!(child.border_box, Rect::new(150.0, 12.0, 124.0, 43.2));
        assert_eq!(child.used_margin.left, 138.0);
        assert_eq!(child.used_margin.right, 138.0);
        let text = result
            .render_boxes
            .iter()
            .find(|item| matches!(item.node_type, BoxType::TextRun("logo")))
            .unwrap();
        assert_eq!(text.bounds.x, child.content_box.x);
    }

    #[test]
    fn single_auto_margin_max_width_and_overflow_are_resolved() {
        for (rule, expected_width, left, right) in [
            (
                "width:100px;margin-left:auto;margin-right:20px",
                100.0,
                180.0,
                20.0,
            ),
            (
                "width:100px;margin-left:20px;margin-right:auto",
                100.0,
                20.0,
                180.0,
            ),
            ("max-width:200px;margin:0 auto", 200.0, 50.0, 50.0),
            ("width:400px;margin:0 auto", 400.0, 0.0, 0.0),
            (
                "width:100px;box-sizing:border-box;padding:10px;border:2px solid;margin:0 auto",
                100.0,
                100.0,
                100.0,
            ),
        ] {
            let html =
                alloc::format!("<style>body{{margin:0}} div{{{rule}}}</style><div>one</div>");
            let result = layout(&html, 300);
            let child = result
                .geometry
                .boxes
                .iter()
                .find(|item| {
                    item.parent.is_some()
                        && item.used_margin.left == left
                        && item.border_box.width == expected_width
                        && item.content_box.height > 0.0
                })
                .unwrap();
            assert_eq!(child.border_box.x, left, "{rule}");
            assert_eq!(child.used_margin.right, right, "{rule}");
        }
    }

    #[test]
    fn centered_inline_block_keeps_its_box_and_arranges_block_children() {
        let result = layout("<style>body{margin:0;text-align:center} span{display:inline-block;width:100px;padding:5px;border:1px solid} div{height:20px}</style><body><span><div>logo</div></span></body>", 300);
        let item = result
            .geometry
            .boxes
            .iter()
            .find(|item| item.style.display == Display::InlineBlock)
            .unwrap();
        assert_eq!(item.border_box.x, 94.0);
        assert_eq!(item.border_box.width, 112.0);
        assert_eq!(item.border_box.height, 32.0);
        let text = result
            .render_boxes
            .iter()
            .find(|item| matches!(item.node_type, BoxType::TextRun("logo")))
            .unwrap();
        assert_eq!(text.bounds.x, 134.0);
    }

    #[test]
    fn intrinsic_inline_block_width_includes_explicit_block_child() {
        let result = layout("<style>center{width:300px} span{display:inline-block;padding:5px} div{width:100px;height:20px}</style><center><span><div>logo</div></span></center>", 300);
        let item = result
            .geometry
            .boxes
            .iter()
            .find(|item| item.style.display == Display::InlineBlock)
            .unwrap();
        assert_eq!(item.border_box.x, 95.0);
        assert_eq!(item.border_box.width, 110.0);
        assert_eq!(item.border_box.height, 30.0);
    }

    #[test]
    fn controls_share_caption_baseline_and_reserve_padding_and_margins() {
        let result = layout("<style>body{margin:0;font-size:16px;line-height:20px} input,button{font-size:16px;line-height:20px} button{padding:9px 10px;margin-left:7px}</style>label <input size=4> <button>Go</button> tail", 400);
        let field = result
            .geometry
            .boxes
            .iter()
            .find(|item| matches!(&item.kind, BoxType::Form(widget) if !widget.kind.is_button()))
            .unwrap();
        let button = result
            .geometry
            .boxes
            .iter()
            .find(|item| matches!(&item.kind, BoxType::Form(widget) if widget.kind.is_button()))
            .unwrap();
        let caption_baseline =
            |item: &LayoutBox<'_>| item.content_box.y + text_baseline(&item.style);
        assert!((caption_baseline(field) - caption_baseline(button)).abs() < EPSILON);
        assert_eq!(field.border_box.width, 48.0);
        assert_eq!(field.border_box.height, 32.0);
        assert_eq!(button.border_box.x - field.border_box.right(), 15.0);
        let label = result
            .render_boxes
            .iter()
            .find(|item| matches!(item.node_type, BoxType::TextRun("label")))
            .unwrap();
        assert!(
            (label.bounds.y + text_baseline(&label_style(&result, label.node_id))
                - caption_baseline(field))
            .abs()
                < EPSILON
        );
        assert_eq!(field.content_box.x - field.border_box.x, 8.0);
    }

    fn label_style<'a>(result: &'a LayoutResult<'a>, node: NodeId) -> LayoutStyle<'a> {
        result
            .geometry
            .boxes
            .iter()
            .find(|item| item.dom_node == node)
            .unwrap()
            .style
            .clone()
    }

    struct LogoMetrics;
    impl ImageMetrics for LogoMetrics {
        fn intrinsic_size(&self, _src: &str) -> Option<RectSize> {
            Some(RectSize::new(272.0, 92.0))
        }
    }

    #[test]
    fn image_dimensions_preserve_ratio_and_following_block_flow() {
        for (attributes, expected) in [
            ("width=136", RectSize::new(136.0, 46.0)),
            ("height=46", RectSize::new(136.0, 46.0)),
            ("width=136 height=30", RectSize::new(136.0, 30.0)),
            ("width=200 style='width:136px'", RectSize::new(136.0, 46.0)),
            ("width=136 style='width:auto'", RectSize::new(272.0, 92.0)),
            (
                "style='width:50%'",
                RectSize::new(150.0, 150.0 * 92.0 / 272.0),
            ),
            ("style='max-width:136px'", RectSize::new(136.0, 46.0)),
            (
                "style='width:152px;padding:7px;border:1px solid;box-sizing:border-box'",
                RectSize::new(136.0, 46.0),
            ),
            (
                "width=136 style='display:block;margin:0 auto'",
                RectSize::new(136.0, 46.0),
            ),
        ] {
            let html = alloc::format!("<style>body{{margin:0}}</style><img src=logo {attributes}><div style='height:10px'>after</div>");
            let document = parse_html(&html).unwrap();
            let styles = StyleContext::build(&document, &collect_embedded_stylesheets(&document));
            let result = layout_document(
                &document,
                &styles,
                Size::new(300, 200),
                &FixedFont,
                &LogoMetrics,
            );
            let image = result
                .render_boxes
                .iter()
                .find(|item| matches!(item.node_type, BoxType::Image(_)))
                .unwrap();
            assert!(
                (image.bounds.width - expected.width).abs() < EPSILON,
                "{attributes}: {:?}",
                image.bounds
            );
            assert!(
                (image.bounds.height - expected.height).abs() < EPSILON,
                "{attributes}: {:?}",
                image.bounds
            );
            let after = result
                .render_boxes
                .iter()
                .find(|item| matches!(item.node_type, BoxType::TextRun("after")))
                .unwrap();
            assert!(
                after.bounds.y + EPSILON >= image.bounds.bottom(),
                "{attributes}"
            );
            if attributes.contains("display:block") {
                assert_eq!(image.bounds.x, 82.0);
            }
        }
    }

    #[test]
    fn inherited_alignment_applies_to_each_wrapped_line_and_nowrap() {
        let result = layout("<style>body{margin:0;text-align:right;font-size:10px;line-height:12px} div{width:50px}</style><body><div>one two three</div></body>", 200);
        let runs: Vec<_> = result
            .render_boxes
            .iter()
            .filter(|item| matches!(item.node_type, BoxType::TextRun(_)))
            .collect();
        assert_eq!(runs[1].bounds.right(), 50.0);
        assert_eq!(runs[2].bounds.right(), 50.0);
        let nowrap = layout("<style>body{margin:0} nav{width:20px;white-space:nowrap}</style><body><nav><a>Gmail</a> <a>Images</a></nav></body>", 200);
        let runs: Vec<_> = nowrap
            .render_boxes
            .iter()
            .filter(|item| matches!(item.node_type, BoxType::TextRun(_)))
            .collect();
        assert_eq!(runs[0].bounds.y, runs[1].bounds.y);
        assert!(nowrap.geometry.content_size.width >= runs[1].bounds.right());
    }

    #[test]
    fn negative_fractional_bounds_round_outwards() {
        assert_eq!(
            int_rect(Rect::new(-2.5, -1.5, 10.0, 8.0)),
            IntRect::new(-3, -2, 11, 9)
        );
    }
    #[test]
    fn google_style_hero_reflows_without_squashing_or_overlapping() {
        let document = parse_html("<html><head><style>body{margin:0;font-size:16px;line-height:20px} header{text-align:right} .hero{width:584px;max-width:90%;margin:0 auto;text-align:center} img{display:block;width:136px;margin:20px auto} input{width:100%;height:46px;box-sizing:border-box} .buttons{margin-top:18px}</style></head><body><header><a href='/mail'>Gmail</a> <a href='/images'>Images</a></header><img src=logo width=272 height=92 style='height:auto'><form class=hero><input type=search><div class=buttons><button>Google Search</button> <button>Feeling Lucky</button></div></form></body></html>").unwrap();
        let styles = StyleContext::build(&document, &collect_embedded_stylesheets(&document));
        for width in [320, 1024] {
            let result = layout_document(
                &document,
                &styles,
                Size::new(width, 600),
                &FixedFont,
                &LogoMetrics,
            );
            let image = result
                .render_boxes
                .iter()
                .find(|item| matches!(item.node_type, BoxType::Image(_)))
                .unwrap();
            assert_eq!(image.bounds.width, 136.0);
            assert_eq!(image.bounds.height, 46.0);
            assert_eq!(image.bounds.x, (width as f32 - 136.0) * 0.5);
            let field = result.render_boxes.iter().find(|item| matches!(&item.node_type, BoxType::Form(widget) if !widget.kind.is_button())).unwrap();
            let expected_width = min(584.0, width as f32 * 0.9);
            assert!((field.bounds.width - expected_width).abs() < EPSILON);
            assert!((field.bounds.x - (width as f32 - expected_width) * 0.5).abs() < EPSILON);
            assert!(field.bounds.y >= image.bounds.bottom());
            let buttons: Vec<_> = result.render_boxes.iter().filter(|item| matches!(&item.node_type, BoxType::Form(widget) if widget.kind.is_button())).collect();
            assert_eq!(buttons.len(), 2);
            assert_eq!(buttons[0].bounds.y, buttons[1].bounds.y);
            assert!(buttons[0].bounds.y >= field.bounds.bottom() + 18.0);
            let row_center = (buttons[0].bounds.x + buttons[1].bounds.right()) * 0.5;
            assert!((row_center - width as f32 * 0.5).abs() < EPSILON);
        }
    }

    #[test]
    fn max_width_image_constrains_intrinsic_inline_block_wrapper() {
        let document = parse_html("<style>span{display:inline-block} img{max-width:136px}</style><center><span><img src=logo></span></center>").unwrap();
        let styles = StyleContext::build(&document, &collect_embedded_stylesheets(&document));
        let result = layout_document(
            &document,
            &styles,
            Size::new(300, 200),
            &FixedFont,
            &LogoMetrics,
        );
        let wrapper = result
            .geometry
            .boxes
            .iter()
            .find(|item| item.kind == BoxType::Inline && item.style.display == Display::InlineBlock)
            .unwrap();
        assert_eq!(wrapper.border_box.width, 136.0);
        assert_eq!(wrapper.border_box.x, 82.0);
    }
    #[test]
    fn lists_emit_owned_nested_markers_and_honor_visible_ordinals() {
        let result = layout(
            include_str!("../tests/fixtures/text-centric-page.html"),
            320,
        );
        let labels: Vec<_> = result
            .list_markers
            .iter()
            .map(|marker| marker.label.as_str())
            .collect();
        assert_eq!(labels, ["•", "•", "•", "3.", "4.", "9.", "10."]);
        assert!(result.list_markers[2].bounds.x > result.list_markers[1].bounds.x);
        for marker in &result.list_markers {
            let item = result
                .geometry
                .boxes
                .iter()
                .find(|item| item.dom_node == marker.node_id)
                .unwrap();
            assert!(marker.bounds.right() < item.content_box.x);
            assert!(marker.bounds.y >= item.content_box.y);
        }
        let scene = into_document_scene(1, Size::new(320, 600), &result);
        for marker in &result.list_markers {
            assert!(scene.objects_for_node(DocumentNodeId(marker.node_id as u64)).iter().any(|id| {
                matches!(&scene.object(*id).unwrap().kind, RenderObjectKind::Text { text, .. } if text == &marker.label)
            }));
        }
        let suppressed = layout("<ul style='list-style:none'><li>navigation</li></ul>", 320);
        assert!(suppressed.list_markers.is_empty());
    }

    #[test]
    fn preformatted_text_preserves_spaces_blank_lines_tabs_and_overflow() {
        let result = layout("<pre>first  line\n    indented\n\nlast\n\tTabbed</pre>", 80);
        let runs: Vec<_> = result
            .render_boxes
            .iter()
            .filter_map(|item| match item.node_type {
                BoxType::TextRun(text) => Some((text, item.bounds)),
                _ => None,
            })
            .collect();
        let first_y = runs[0].1.y;
        let first_line: String = runs
            .iter()
            .filter(|(_, bounds)| bounds.y == first_y)
            .map(|(text, _)| *text)
            .collect();
        assert_eq!(first_line, "first  line");
        let indented = runs.iter().find(|(text, _)| *text == "indented").unwrap().1;
        let last = runs.iter().find(|(text, _)| *text == "last").unwrap().1;
        assert_eq!(indented.x, runs[0].1.x + 32.0);
        assert!(
            (last.y - indented.bottom() - 19.2).abs() < 0.1,
            "blank line must retain height: indented={indented:?} last={last:?} runs={runs:?}"
        );
        let tabbed = runs.iter().find(|(text, _)| *text == "Tabbed").unwrap().1;
        assert_eq!(tabbed.x, runs[0].1.x + 64.0);
        assert!(result.geometry.content_size.width > 80.0);
        assert!(runs.iter().all(|(text, _)| !text.contains('\t')));
    }
}
