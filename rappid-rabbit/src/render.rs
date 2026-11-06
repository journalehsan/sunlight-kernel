//! Browser-side retained layout and scene pipeline.
//!
//! The two-pass arena engine in `crate::layout` supplies all geometry and
//! retained scene objects. This module connects browser fonts, image resources,
//! inspector geometry, diagnostics, scene diffs, and form state to that engine.

use alloc::{format, string::String, sync::Arc, vec, vec::Vec};

use golden_fish::{Attribute, Document, Node, NodeId};
use sunlight_http::ParsedUrl;
use sunlight_ui::{
    widgets::{
        diff_scenes, DocumentFontFamily, DocumentNodeId, DocumentScene, RenderObjectKind,
        ScenePatch,
    },
    Size, VecText,
};

use crate::{
    css::{Property, StyleContext},
    form::FormControlState,
    images::ImageCache,
    layout::{self, FontMeasurer as LayoutFontMeasurer, ImageMetrics, SceneImageSource},
    resources::discovery::resolve_url,
};

pub const MAX_DOCUMENT_DIMENSION: u32 = 16_384;
/// Bump this when a future in-memory cache must invalidate retained scenes.
pub const DOCUMENT_SCENE_FORMAT_VERSION: u32 = 1;

/// Retained scene counts and unsupported paint features after each rebuild.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LayoutDiagnostics {
    pub object_count: usize,
    pub image_count: usize,
    pub placeholder_count: usize,
    pub link_count: usize,
    pub control_count: usize,
    /// Painted features the layout engine could not represent, if any.
    pub unsupported: Vec<&'static str>,
}

impl LayoutDiagnostics {
    pub fn summary(&self) -> alloc::string::String {
        if self.unsupported.is_empty() {
            format!(
                "layout objects={} images={} placeholders={} links={} controls={}",
                self.object_count,
                self.image_count,
                self.placeholder_count,
                self.link_count,
                self.control_count,
            )
        } else {
            format!(
                "layout objects={} images={} placeholders={} links={} controls={} unsupported=[{}]",
                self.object_count,
                self.image_count,
                self.placeholder_count,
                self.link_count,
                self.control_count,
                self.unsupported.join(","),
            )
        }
    }
}

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

/// Owned inspector geometry copied from the canonical layout arena.
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutGeometry {
    pub node_id: NodeId,
    pub content_box: layout::Rect,
    pub padding_box: layout::Rect,
    pub border_box: layout::Rect,
    pub margin_box: layout::Rect,
}

#[derive(Clone, Debug)]
pub struct DocumentRenderState {
    pub document_id: u64,
    pub final_url: String,
    pub dom: Arc<Document>,
    pub styles: Arc<StyleContext>,
    pub layout_geometry: Vec<LayoutGeometry>,
    pub current_scene: Arc<DocumentScene>,
    pub previous_scene: Option<Arc<DocumentScene>>,
    pub last_patch: ScenePatch,
    pub scene_generation: u64,
    pub viewport: Size,
    pub diagnostics: LayoutDiagnostics,
    images: ImageCache,
    pub image_revision: u64,
    /// Reject worker snapshots created before a controlled DOM mutation.
    pub dom_revision: u64,
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
        let (current_scene, layout_geometry) = render_document(
            document_id,
            &final_url,
            &dom,
            &styles,
            viewport,
            measurer,
            &images,
        );
        let mut diagnostics = diagnose_scene(&current_scene);
        diagnostics.unsupported = unsupported_features(&dom, &styles);
        Self {
            document_id,
            final_url,
            dom: Arc::new(dom),
            styles: Arc::new(styles),
            layout_geometry,
            current_scene: Arc::new(current_scene),
            previous_scene: None,
            last_patch: ScenePatch::default(),
            scene_generation: 1,
            viewport,
            diagnostics,
            images,
            image_revision: 0,
            dom_revision: 0,
        }
    }

    /// Reflows without reparsing or refetching. Deterministic scene IDs let
    /// unchanged state compare cleanly after a fresh layout pass.
    pub fn rebuild_for_viewport(&mut self, viewport: Size, measurer: &dyn TextMeasurer) -> bool {
        if viewport == self.viewport {
            return false;
        }
        self.rebuild(viewport, measurer);
        true
    }

    /// Cheap owned worker input: DOM, styles and decoded pixels are shared;
    /// UI-only inspector geometry and historical scene diffs are not copied.
    pub fn reflow_snapshot(&self) -> Self {
        Self {
            document_id: self.document_id,
            final_url: self.final_url.clone(),
            dom: self.dom.clone(),
            styles: self.styles.clone(),
            layout_geometry: Vec::new(),
            current_scene: self.current_scene.clone(),
            previous_scene: None,
            last_patch: ScenePatch::default(),
            scene_generation: self.scene_generation,
            viewport: self.viewport,
            diagnostics: LayoutDiagnostics::default(),
            images: self.images.clone(),
            image_revision: self.image_revision,
            dom_revision: self.dom_revision,
        }
    }

    /// Fixture host invalidation: refresh styles and run the canonical two-pass
    /// layout/scene paint pipeline, producing dirty regions for DocumentCanvas.
    #[cfg(feature = "js-dom-fixture")]
    pub(crate) fn rebuild_for_fixture_dom(
        &mut self,
        dom: Arc<Document>,
        measurer: &dyn TextMeasurer,
    ) {
        self.dom = dom;
        self.dom_revision = self.dom_revision.wrapping_add(1);
        let sheets = crate::css::collect_embedded_stylesheets(&self.dom);
        self.styles = Arc::new(StyleContext::build_with_viewport(
            &self.dom,
            &sheets,
            self.viewport.w,
        ));
        self.rebuild(self.viewport, measurer);
    }

    pub fn rebuild(&mut self, viewport: Size, measurer: &dyn TextMeasurer) {
        if viewport.w != self.viewport.w && self.styles.viewport_changes_rules(viewport.w) {
            self.styles = Arc::new(self.styles.for_viewport(&self.dom, viewport.w));
        }
        let (scene, layout_geometry) = render_document(
            self.document_id,
            &self.final_url,
            &self.dom,
            &self.styles,
            viewport,
            measurer,
            &self.images,
        );
        self.diagnostics = diagnose_scene(&scene);
        self.diagnostics.unsupported = unsupported_features(&self.dom, &self.styles);
        self.last_patch = diff_scenes(&self.current_scene, &scene);
        self.previous_scene = Some(self.current_scene.clone());
        self.current_scene = Arc::new(scene);
        self.layout_geometry = layout_geometry;
        self.viewport = viewport;
        self.scene_generation = self.scene_generation.saturating_add(1);
    }

    pub fn rebuild_for_images(&mut self, measurer: &dyn TextMeasurer, images: ImageCache) -> bool {
        self.complete_images(self.document_id, measurer, images)
    }

    /// Owned completion for a document generation. No parsing/fetching on the
    /// UI thread; repeat completions need no layout or scene update.
    pub fn complete_images(
        &mut self,
        document_id: u64,
        measurer: &dyn TextMeasurer,
        images: ImageCache,
    ) -> bool {
        if document_id != self.document_id || images == self.images {
            return false;
        }
        self.images = images;
        self.image_revision = self.image_revision.wrapping_add(1);
        self.rebuild(self.viewport, measurer);
        true
    }

    /// Patch one retained control after browser-owned form state changes.
    pub fn patch_control(
        &mut self,
        control_id: DomNodeId,
        state: &FormControlState,
        focused: bool,
        measurer: &dyn TextMeasurer,
    ) -> bool {
        let Some(index) = self.current_scene.objects.iter().position(|object| {
            object.owner_node_id == control_id
                && matches!(object.kind, RenderObjectKind::Control { .. })
        }) else {
            return false;
        };
        let password = self
            .dom
            .get_attribute(control_id.0 as usize, "type")
            .is_some_and(|v| v.eq_ignore_ascii_case("password"));
        let desired_value = match state {
            FormControlState::Text { state, .. } => Some(if password {
                "*".repeat(state.current_value.chars().count())
            } else {
                state.current_value.clone()
            }),
            FormControlState::Button {
                kind: crate::form::FormControlKind::Checkbox { checked },
                ..
            } => Some(if *checked { "[x]" } else { "[ ]" }.into()),
            _ => None,
        };
        let desired_caret = match state {
            FormControlState::Text { state, .. } if focused => {
                let prefix =
                    &state.current_value[..state.cursor_position.min(state.current_value.len())];
                Some(measurer.measure_width(&if password {
                    "*".repeat(prefix.chars().count())
                } else {
                    prefix.into()
                }))
            }
            _ => None,
        };
        if let RenderObjectKind::Control {
            value,
            focused: visual_focus,
            caret_offset,
            ..
        } = &self.current_scene.objects[index].kind
        {
            if *visual_focus == focused
                && *caret_offset == desired_caret
                && desired_value
                    .as_ref()
                    .is_none_or(|desired| value == desired)
            {
                return false;
            }
        }
        // Unchanged controls must not make Arc::make_mut clone the entire
        // retained scene on the UI thread after every worker completion.
        let previous = self.current_scene.clone();
        let object = &mut Arc::make_mut(&mut self.current_scene).objects[index];
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
        *caret_offset = desired_caret;
        if let Some(desired) = desired_value {
            *value = desired.clone();
        }
        Arc::make_mut(&mut self.current_scene).finalize();
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

/// Browser-side `ImageMetrics` view of the live image cache.
struct CacheImageMetrics<'a> {
    cache: &'a ImageCache,
    base_url: Option<ParsedUrl>,
}

impl CacheImageMetrics<'_> {
    fn resolved(&self, src: &str) -> Option<String> {
        self.base_url
            .as_ref()
            .and_then(|base| resolve_url(base, src).ok())
    }
}

impl ImageMetrics for CacheImageMetrics<'_> {
    fn intrinsic_size(&self, src: &str) -> Option<layout::RectSize> {
        let url = self.resolved(src)?;
        self.cache.decoded(&url).map(|decoded| {
            layout::RectSize::new(decoded.image.width as f32, decoded.image.height as f32)
        })
    }
}

/// Browser-side scene resources: URL resolution and decoded pixels.
impl SceneImageSource for CacheImageMetrics<'_> {
    fn resolve(&self, raw: &str) -> Option<String> {
        self.resolved(raw)
    }

    fn decoded(&self, src: &str) -> Option<alloc::sync::Arc<sunlight_ui::widgets::RasterImage>> {
        let url = self.resolved(src)?;
        self.cache
            .decoded(&url)
            .map(|decoded| decoded.image.clone())
    }
}

/// Adapts browser font metrics to layout using the exact painted advances.
/// This mirrors
/// `TextMeasurer::measure_width_for_size` for a resolved CSS family rather than
/// estimating from character counts.
struct MeasurerFonts<'a> {
    measurer: &'a dyn TextMeasurer,
}

impl LayoutFontMeasurer for MeasurerFonts<'_> {
    fn measure_width(&self, family: &str, font_size: f32, text: &str) -> f32 {
        let family = DocumentFontFamily::resolve_css_list(family);
        let font_size = font_size.max(0.0).min(MAX_DOCUMENT_DIMENSION as f32) as u32;
        self.measurer
            .measure_width_for_size(family, font_size, text)
            .max(0) as f32
    }

    fn line_height(&self, family: &str, font_size: f32) -> f32 {
        let family = DocumentFontFamily::resolve_css_list(family);
        let requested = (font_size * 1.2).max(1.0) as u32;
        self.measurer
            .line_height_for(family)
            .max(requested.min(MAX_DOCUMENT_DIMENSION)) as f32
    }
}

/// Runs the index-based engine and converts its result into a retained scene.
fn render_document(
    document_id: u64,
    final_url: &str,
    dom: &Document,
    styles: &StyleContext,
    viewport: Size,
    measurer: &dyn TextMeasurer,
    images: &ImageCache,
) -> (DocumentScene, Vec<LayoutGeometry>) {
    let base_url = ParsedUrl::parse(final_url).ok();
    let fonts = MeasurerFonts { measurer };
    let image_metrics = CacheImageMetrics {
        cache: images,
        base_url: base_url.clone(),
    };
    let result = layout::layout_document(dom, styles, viewport, &fonts, &image_metrics);
    let scene_image_source = CacheImageMetrics {
        cache: images,
        base_url,
    };
    let geometry = result
        .geometry
        .boxes
        .iter()
        .map(|item| LayoutGeometry {
            node_id: item.dom_node,
            content_box: item.content_box,
            padding_box: item.padding_box,
            border_box: item.border_box,
            margin_box: item.margin_box,
        })
        .collect();
    let mut scene = layout::into_document_scene_with_images(
        document_id,
        viewport,
        &result,
        &scene_image_source,
    );
    crate::backgrounds::paint(&mut scene, &result, styles, &scene_image_source);
    (scene, geometry)
}

/// Counts retained object kinds and reports paint features the scene dropped.
fn diagnose_scene(scene: &DocumentScene) -> LayoutDiagnostics {
    let mut diagnostics = LayoutDiagnostics {
        object_count: scene.objects.len(),
        ..LayoutDiagnostics::default()
    };
    for object in &scene.objects {
        match &object.kind {
            RenderObjectKind::Image { .. } => diagnostics.image_count += 1,
            RenderObjectKind::ImagePlaceholder { .. } => diagnostics.placeholder_count += 1,
            RenderObjectKind::Link { .. } => diagnostics.link_count += 1,
            RenderObjectKind::Control { .. } => diagnostics.control_count += 1,
            _ => {}
        }
    }
    diagnostics
}

/// Reports features approximated by normal flow or omitted from painting.
/// Feature names are deduplicated to keep the console diagnostic bounded.
fn unsupported_features(dom: &Document, styles: &StyleContext) -> Vec<&'static str> {
    let mut unsupported = Vec::new();
    let mut stack = vec![dom.root()];
    while let Some(node_id) = stack.pop() {
        let Some(node) = dom.get(node_id) else {
            continue;
        };
        stack.extend(dom.children(node_id).iter().copied());
        let Node::Element {
            tag_name,
            attributes,
            ..
        } = node
        else {
            continue;
        };
        let style = styles.nearest_element_style(dom, node_id);
        if matches!(
            tag_name.to_ascii_lowercase().as_str(),
            "table" | "thead" | "tbody" | "tfoot" | "tr" | "td" | "th"
        ) {
            push_unique(&mut unsupported, "table");
        }
        // The engine lays out and paints the common single-line
        // controls; only the widgets it still approximates are reported.
        if form_control_is_approximated(tag_name, attributes) {
            push_unique(&mut unsupported, "form-control");
        }
        if style
            .and_then(|style| style.value(&Property::BackgroundImage))
            .is_some_and(|v| {
                let raw = v.display();
                raw != "none" && crate::css::css_image_url(&raw).is_none()
            })
        {
            push_unique(&mut unsupported, "background-image");
        }
        if style
            .and_then(|style| style.value(&Property::BoxShadow))
            .is_some()
        {
            push_unique(&mut unsupported, "box-shadow");
        }
        if style
            .and_then(|style| style.value(&Property::TextShadow))
            .is_some()
        {
            push_unique(&mut unsupported, "text-shadow");
        }
    }
    unsupported
}

fn push_unique(list: &mut Vec<&'static str>, value: &'static str) {
    if !list.contains(&value) {
        list.push(value);
    }
}

/// Whether layout still approximates this form control.
///
/// `input` (single-line), `button`, and `select` are now sized and painted as
/// real controls, so they are no longer reported.  `textarea` and multi-line
/// or otherwise unsupported input types remain approximated by block flow.
fn form_control_is_approximated(tag_name: &str, attributes: &[Attribute]) -> bool {
    if tag_name.eq_ignore_ascii_case("textarea") {
        return true;
    }
    if tag_name.eq_ignore_ascii_case("input") {
        let input_type = attr(attributes, "type").unwrap_or("text");
        return !matches!(
            input_type.to_ascii_lowercase().as_str(),
            "text"
                | "search"
                | "password"
                | "email"
                | "tel"
                | "url"
                | "number"
                | "date"
                | "datetime-local"
                | "month"
                | "week"
                | "time"
                | "submit"
                | "button"
                | "reset"
                | "hidden"
        );
    }
    false
}

fn attr<'a>(attributes: &'a [Attribute], name: &str) -> Option<&'a str> {
    attributes
        .iter()
        .find(|attribute| attribute.name().eq_ignore_ascii_case(name))
        .map(|attribute| attribute.value())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css::{
        collect_embedded_stylesheets, parse_stylesheet, StyleContext, StylesheetSource,
    };
    use golden_fish::parse_html;
    use sunlight_ui::{
        widgets::{RenderInteraction, RenderObjectId},
        Color, Rect,
    };

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
    fn unchanged_control_keeps_shared_scene_and_edits_still_patch() {
        let mut state = render("<input value=hello>");
        let mut forms = crate::form::FormState::new();
        forms.build_from_dom(&state.dom);
        let (&id, control) = forms.controls.iter().next().unwrap();
        let control = control.clone();
        let snapshot = state.reflow_snapshot();
        assert!(!state.patch_control(id, &control, false, &TestMeasure));
        assert!(Arc::ptr_eq(&snapshot.current_scene, &state.current_scene));
        assert!(state.patch_control(id, &control, true, &TestMeasure));
        let focused_scene = state.current_scene.clone();
        assert!(!state.patch_control(id, &control, true, &TestMeasure));
        assert!(Arc::ptr_eq(&focused_scene, &state.current_scene));
        let mut edited = control;
        if let crate::form::FormControlState::Text { state, .. } = &mut edited {
            state.current_value = String::from("changed");
        }
        assert!(state.patch_control(id, &edited, true, &TestMeasure));
        assert!(state.current_scene.objects.iter().any(|object| matches!(&object.kind, RenderObjectKind::Control {value, ..} if value == "changed")));
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
        if let Some(Node::Text { content }) = Arc::make_mut(&mut state.dom).get_mut(before_node) {
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
        state.styles = Arc::new(StyleContext::build(
            &state.dom,
            &[parse_stylesheet(
                ".notice { color: blue; background-color: #dddddd; }",
                StylesheetSource::Embedded,
            )],
        ));
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
    fn decoded_entities_and_emoji_remain_inline_owned_text() {
        let state = render(include_str!("../tests/fixtures/html-entities-emoji.html"));
        let text = state
            .current_scene
            .objects
            .iter()
            .filter_map(|object| match &object.kind {
                RenderObjectKind::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(text.iter().any(|value| value.contains("©")));
        assert!(text.iter().any(|value| value.contains("🐇")));
        assert!(text.iter().any(|value| *value == "Kernel\u{a0}Archives"));
        assert!(text.iter().any(|value| value.contains("&nbsp;")));
        assert!(!text.iter().any(|value| value.contains("&#x1F407;")));
        assert!(!text.iter().any(|value| value.contains("var literal")));
        assert!(state.current_scene.objects.iter().any(|object| {
            matches!(&object.kind, RenderObjectKind::Text { text, .. } if text.contains("🐇"))
                && matches!(object.interaction, Some(RenderInteraction::Link { .. }))
        }));
        assert!(state.current_scene.objects.iter().any(|object| {
            matches!(&object.kind, RenderObjectKind::Text { text, color, .. }
                if text.contains("foreground") && *color == Color::rgb(255, 255, 255))
        }));
    }

    fn text_objects(state: &DocumentRenderState) -> Vec<(String, Rect)> {
        state
            .current_scene
            .objects
            .iter()
            .filter_map(|object| match &object.kind {
                RenderObjectKind::Text { text, .. } => Some((text.clone(), object.bounds)),
                _ => None,
            })
            .collect()
    }

    fn link_objects(state: &DocumentRenderState) -> Vec<(String, Rect)> {
        state
            .current_scene
            .objects
            .iter()
            .filter_map(|object| match &object.kind {
                RenderObjectKind::Link { href, .. } => Some((href.clone(), object.bounds)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn margin_auto_centers_block_in_container() {
        let new = render("<style>body{margin:0} main{width:280px;padding:10px;border:2px solid} div{width:100px;padding:5px;border:1px solid;margin:0 auto;font-size:16px;line-height:20px}</style><main><div>search</div></main>");
        let new_text = text_objects(&new);
        assert_eq!(new_text[0].1.x, 102);
        let owner = new.dom.find_first_element("div").unwrap();
        let border = new
            .current_scene
            .objects
            .iter()
            .find(|object| {
                object.owner_node_id == DocumentNodeId(owner as u64)
                    && matches!(object.kind, RenderObjectKind::Box)
            })
            .unwrap();
        assert_eq!(border.bounds.x, 96);
        assert_eq!(border.bounds.w, 112);
    }

    #[test]
    fn inline_links_preserve_horizontal_flow_and_hitbox() {
        for content in [
            "plain <a href='/one'>link</a> tail",
            "<a href='/one'>one</a><a href='/two'>two</a>",
            "<a href='/one'>one</a> <a href='/two'>two</a>",
            "<a href='/one'>one <strong>two</strong> three four</a>",
            "<a href='/one'>Kernel&nbsp;Archives</a>",
        ] {
            let html = alloc::format!("<style>body{{margin:0;font-size:16px;line-height:20px}} p{{width:104px}}</style><body><p>{content}</p></body>");
            let new = render(&html);
            assert!(
                !link_objects(&new).is_empty(),
                "missing links for {content}"
            );
            for link in new
                .current_scene
                .objects
                .iter()
                .filter(|object| matches!(object.kind, RenderObjectKind::Link { .. }))
            {
                let RenderObjectKind::Link {
                    text_object_ids, ..
                } = &link.kind
                else {
                    unreachable!()
                };
                let visual = text_object_ids
                    .iter()
                    .map(|id| {
                        new.current_scene
                            .objects
                            .iter()
                            .find(|object| object.id == *id)
                            .unwrap()
                            .bounds
                    })
                    .reduce(|a, b| {
                        let x = a.x.min(b.x);
                        let y = a.y.min(b.y);
                        Rect::new(
                            x,
                            y,
                            (a.right().max(b.right()) - x) as u32,
                            (a.bottom().max(b.bottom()) - y) as u32,
                        )
                    })
                    .unwrap();
                assert_eq!(link.bounds, visual);
            }
        }
    }

    #[test]
    fn text_align_right_aligns_navigation_items() {
        let new = render("<style>body{margin:0;text-align:right;font-size:16px;line-height:20px} nav{width:120px}</style><body><nav><a href='/mail'>Gmail</a> <a href='/images'>Images</a></nav></body>");
        let texts = text_objects(&new);
        assert_eq!(texts.len(), 2);
        assert_eq!(texts[0].1.x, 24);
        assert_eq!(texts[0].1.y, texts[1].1.y);
        assert_eq!(texts[1].1.right(), 120);
    }

    #[test]
    fn canvas_background_covers_viewport_and_preserves_local_backgrounds() {
        for (html_background, expected) in [
            ("transparent", Color::rgb(255, 255, 255)),
            ("#123456", Color::rgb(0x12, 0x34, 0x56)),
        ] {
            let html = alloc::format!("<html><head><style>html{{background:{html_background}}} body{{margin:8px}} div{{background:red;height:10px}}</style></head><body><div></div></body></html>");
            let new = render(&html);
            let canvas = new
                .current_scene
                .objects
                .iter()
                .find(|object| object.id == RenderObjectId(u64::MAX))
                .unwrap();
            assert!(
                matches!(canvas.kind, RenderObjectKind::Rectangle { fill } if fill == expected)
            );
            assert_eq!(canvas.bounds, Rect::new(0, 0, 320, 240));
            assert!(new.current_scene.objects.iter().any(|object| matches!(object.kind, RenderObjectKind::Rectangle { fill } if fill == Color::rgb(255, 0, 0)) && object.bounds.x == 8 && object.bounds.h == 10));
        }
    }

    #[test]
    fn simple_anchor_emits_one_link_hit_region() {
        let new = render("<p><a href='/go'>single word</a></p>");
        let new_links = link_objects(&new);
        assert_eq!(new_links.len(), 1);
        assert_eq!(new_links[0].0, "/go");
        // The link's interactive text must still resolve back to the anchor.
        let anchor = new.dom.find_first_element("a").unwrap();
        let owned = new
            .current_scene
            .objects_for_node(DocumentNodeId(anchor as u64));
        assert!(!owned.is_empty());
    }

    #[test]
    fn wrapped_anchor_keeps_one_hit_region_per_visual_line() {
        let new = render(
            "<style>p{display:block;width:104px}</style><p><a href='/lists'>Linux mailing list service</a></p>",
        );
        let links = link_objects(&new);
        assert!(links.len() >= 2, "expected a wrapped anchor, got {links:?}");
        let line_ys: Vec<i32> = links.iter().map(|(_, bounds)| bounds.y).collect();
        let unique: alloc::collections::BTreeSet<i32> = line_ys.iter().copied().collect();
        assert_eq!(unique.len(), links.len());
    }

    #[test]
    fn renderer_preserves_decoded_images_and_uses_viewport_dimensions() {
        use crate::images::ImageCache;
        let dom = parse_html("<p><img src='/a.png'></p>").unwrap();
        let styles = StyleContext::build(&dom, &[]);
        let mut images = ImageCache::default();
        images.insert_decoded(
            String::from("http://example.test/a.png"),
            decoded_solid_image(0xFFFF_0000),
        );
        let mut state = DocumentRenderState::new_with_images(
            1,
            String::from("http://example.test/"),
            dom,
            styles,
            Size::new(320, 240),
            &TestMeasure,
            images,
        );
        assert!(state.current_scene.objects.iter().any(
            |object| matches!(&object.kind, RenderObjectKind::Image { source_url, .. }
                if source_url == "http://example.test/a.png")
        ));
        assert_eq!(state.diagnostics.image_count, 1);
        assert_eq!(state.diagnostics.placeholder_count, 0);
        // A viewport change must relayout and reuse the decoded pixels.
        assert!(state.rebuild_for_viewport(Size::new(200, 240), &TestMeasure));
        assert_eq!(state.viewport, Size::new(200, 240));
        assert!(state.diagnostics.image_count >= 1);
    }

    #[test]
    fn renderer_reports_unsupported_tables_and_forms_but_supports_sibling_floats() {
        let new = render(
            "<style>.f{float:left}</style><div class=f>float</div><table><tr><td>cell</td></tr></table><form><textarea>notes</textarea></form>",
        );
        let unsupported = &new.diagnostics.unsupported;
        assert!(!unsupported.contains(&"float"));
        assert!(unsupported.contains(&"table"));
        assert!(unsupported.contains(&"form-control"));
    }

    #[test]
    fn renderer_lays_out_single_line_controls_instead_of_flagging_them() {
        let new = render(
            "<form><input type=text value=x><input type=submit value=go><select><option>alpha</option></select></form>",
        );
        assert!(!new.diagnostics.unsupported.contains(&"form-control"));
        let controls = new
            .current_scene
            .objects
            .iter()
            .filter(|object| matches!(object.kind, RenderObjectKind::Control { .. }))
            .count();
        assert_eq!(controls, 3);
    }

    #[test]
    fn renderer_hidden_inputs_never_reach_the_scene() {
        let new = render(
            "<form><input type=hidden name=csrf value=abc><input type=text value=visible></form>",
        );
        let controls: Vec<_> = new
            .current_scene
            .objects
            .iter()
            .filter(|object| matches!(object.kind, RenderObjectKind::Control { .. }))
            .collect();
        // Only the visible text field is retained; the hidden input is pruned
        // before layout, so it leaves no control object and no geometry.
        assert_eq!(controls.len(), 1);
        assert!(controls
            .iter()
            .all(|object| object.bounds.w > 0 && object.bounds.h > 0));
        assert_eq!(new.diagnostics.control_count, 1);
    }

    #[test]
    fn renderer_sizes_text_like_inputs_without_flagging_them() {
        let new =
            render("<form><input type=email value=a@b.c><input type=search placeholder=q></form>");
        assert!(!new.diagnostics.unsupported.contains(&"form-control"));
        let controls: Vec<_> = new
            .current_scene
            .objects
            .iter()
            .filter(|object| matches!(object.kind, RenderObjectKind::Control { .. }))
            .collect();
        assert_eq!(controls.len(), 2);
        assert!(controls
            .iter()
            .all(|object| object.bounds.w > 0 && object.bounds.h > 0));
    }

    fn geometry_for<'a>(state: &'a DocumentRenderState, tag: &str) -> &'a LayoutGeometry {
        let node = state.dom.find_first_element(tag).unwrap();
        state
            .layout_geometry
            .iter()
            .find(|item| item.node_id == node)
            .unwrap()
    }

    #[test]
    fn plain_semantic_text_has_spacing_indentation_and_default_link_styles() {
        let state = render(include_str!("../tests/fixtures/text-centric-page.html"));
        let headings: Vec<_> = (1..=6)
            .map(|level| {
                let tag = format!("h{level}");
                let geometry = geometry_for(&state, &tag);
                assert_eq!(geometry.content_box.x, 8.0);
                assert!(geometry.margin_box.height > geometry.border_box.height);
                geometry
            })
            .collect();
        assert!(headings
            .windows(2)
            .all(|pair| pair[0].border_box.bottom() < pair[1].border_box.y));
        let sizes: Vec<_> = [
            "Writing",
            "Topics",
            "Reading",
            "Quotation",
            "Source",
            "Endnotes",
        ]
        .iter()
        .map(|heading| {
            state
                .current_scene
                .objects
                .iter()
                .find_map(|object| match &object.kind {
                    RenderObjectKind::Text {
                        text,
                        font_size,
                        bold: true,
                        ..
                    } if text == heading => Some(*font_size),
                    _ => None,
                })
                .unwrap()
        })
        .collect();
        assert!(sizes.windows(2).all(|pair| pair[0] > pair[1]));
        let paragraphs: Vec<_> = state.layout_geometry.iter().filter(|item| {
            matches!(state.dom.get(item.node_id), Some(Node::Element { tag_name, .. }) if tag_name == "p")
        }).collect();
        assert!(paragraphs[1].border_box.y >= paragraphs[0].border_box.bottom() + 16.0);
        assert_eq!(geometry_for(&state, "blockquote").content_box.x, 48.0);
        assert_eq!(geometry_for(&state, "ul").content_box.x, 48.0);
        let anchor = state.dom.find_first_element("a").unwrap();
        let text_node = state.dom.children(anchor)[0];
        assert!(state.current_scene.objects.iter().any(|object| {
            object.owner_node_id == DocumentNodeId(text_node as u64)
                && matches!(object.kind, RenderObjectKind::Text { color, underline: true, .. } if color == Color::rgb(0, 0, 255))
        }));
        assert!(state.current_scene.objects.iter().any(|object| {
            matches!(&object.kind, RenderObjectKind::Text { text, monospace: true, .. } if text == "inline_code")
        }));
        let hr = state.dom.find_first_element("hr").unwrap();
        assert!(
            state.current_scene.objects.iter().any(|object| {
                object.owner_node_id == DocumentNodeId(hr as u64)
                    && matches!(
                        object.kind,
                        RenderObjectKind::BorderSides {
                            widths: [1, 0, 0, 0],
                            ..
                        }
                    )
                    && object.bounds.w > 0
            }),
            "hr objects: {:?}",
            state
                .current_scene
                .objects
                .iter()
                .filter(|object| object.owner_node_id == DocumentNodeId(hr as u64))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn minimalist_column_centers_and_reflows_without_losing_links_or_geometry() {
        let mut state = render(include_str!("../tests/fixtures/minimalist-page.html"));
        state.rebuild_for_viewport(Size::new(900, 600), &TestMeasure);
        let wide = geometry_for(&state, "main");
        assert_eq!(wide.content_box.width, 600.0);
        assert_eq!(wide.border_box.x * 2.0 + wide.border_box.width, 900.0);
        assert_eq!(state.diagnostics.link_count, 1);
        assert!(state.rebuild_for_viewport(Size::new(240, 600), &TestMeasure));
        let narrow = geometry_for(&state, "main");
        assert_eq!(narrow.border_box.x, 0.0);
        assert_eq!(narrow.border_box.width, 240.0);
        assert!(state.current_scene.content_size.h >= 600);
        assert_eq!(state.diagnostics.link_count, 1);
        let snapshot = state.current_scene.clone();
        let geometry = state.layout_geometry.clone();
        state.rebuild(state.viewport, &TestMeasure);
        assert_eq!(state.layout_geometry, geometry);
        assert_eq!(state.current_scene, snapshot);
        assert!(state.last_patch.is_empty());
    }
    #[test]
    fn layout_uses_the_painted_faces_advances_and_measured_line_height() {
        struct FaceMeasure;
        impl TextMeasurer for FaceMeasure {
            fn measure_width(&self, text: &str) -> u32 {
                text.chars().count() as u32 * 8
            }
            fn line_height(&self) -> u32 {
                24
            }
            fn measure_width_for_size(&self, _: DocumentFontFamily, _: u32, text: &str) -> u32 {
                text.chars().count() as u32 * 11
            }
        }
        let dom = parse_html("<h3>Other articles</h3><p>one</p>").unwrap();
        let styles = StyleContext::build(&dom, &[]);
        let state = DocumentRenderState::new(
            1,
            String::from("http://example.test/"),
            dom,
            styles,
            Size::new(320, 240),
            &FaceMeasure,
        );
        let texts = text_objects(&state);
        assert_eq!(texts[0].0, "Other");
        assert_eq!(texts[1].0, "articles");
        assert_eq!(texts[1].1.x, texts[0].1.right() + 11);
        assert_eq!(texts[2].0, "one");
        assert_eq!(geometry_for(&state, "p").content_box.height, 24.0);
        // Fractional heading/margin positions round outward at the scene boundary.
        assert!((24..=25).contains(&texts[2].1.h));
        assert_eq!(state.scene_generation, 1);
        assert!(state.previous_scene.is_none());
    }
}
