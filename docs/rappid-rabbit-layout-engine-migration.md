# Rappid Rabbit layout engine migration

Status: **Milestone 5 (Default Promotion & Visual Parity) — Completed**
(2026-11-06). `LayoutBackend::New` (Modern, Two-Pass Arena) is the primary
production engine from the first scene. `Legacy` remains a maintained runtime
fallback via **Ctrl+E**. Known rendering differences below remain accepted
limitations of this promotion.

## 1. Live rendering path map

```
HTTP body (text/html)
  -> golden_fish::parse_html            (Document)
  -> collect_embedded_stylesheets       (Vec<Stylesheet>)
  -> discover_resources + fetch_external_stylesheets
  -> order_document_stylesheets         (cascade order)
  -> StyleContext::build_with_viewport  (computed styles arena)
  -> DocumentRenderState::new_with_images
       -> build_layout_tree_for_url_with_images   (render.rs LayoutBuilder)
            -> LayoutTree { nodes, inline_boxes, content_size }
       -> New: new_engine_scene -> layout_document -> into_document_scene_with_images
       -> Legacy fallback: scene_from_layout      (render.rs)
            -> DocumentScene (retained RenderObject list)
  -> DocumentCanvas::with_scene         (paint + hit test)
```

Relayout without reparse/refetch:

* `DocumentRenderState::rebuild_for_viewport` -> `rebuild` (resize / devtools
  panel resize / window resize).
* `DocumentRenderState::rebuild_for_images`   -> `rebuild` after the image
  cache changes (decoded or failed).
* `rebuild` re-runs the legacy `LayoutBuilder` with the current `viewport`,
  `StyleContext`, and `ImageCache` for DOM inspector correlation, then the
  selected backend produces the scene and `diff_scenes` patches the retained
  canvas. Construction selects the backend in the same way.

Interaction and identity:

* `owner_node_id` = DOM node id, so `DocumentScene::objects_for_node` and the
  DOM inspector correlate boxes/text/links/controls back to Golden Fish nodes.
* Object ids are `object_id(owner, role, slot)` with a role byte per kind, so
  identical state yields identical ids across rebuilds and diffs stay stable.
* `hit_test` returns the topmost object whose `bounds` contains the point;
  link/control handling reads `RenderInteraction`.

### Existing behavior to preserve

| Feature | Legacy behavior |
| --- | --- |
| Floats | `layout_block` implements `float:left/right` with an active-float list, shrink-to-fit width, `clear` handling, and a `float_containing_block` record. Emitted as ordinary blocks with `float_side`/`clear` metadata on `LayoutNode`. |
| Tables | `layout_table` collects rows (`collect_table_rows`), computes per-column preferred widths, distributes to a target content width, and lays out each cell as a block with `table_dimensions = (rows, columns)`. Cells inherit the row background when transparent. |
| Forms | `collect_inline`/`record_inline_box_with_control` turn `input`/`button` (and friends) into `InlineItem::Control`, producing an `InlineLayoutBox.control` -> `RenderObjectKind::Control` with `RenderInteraction::Control`. `patch_control` updates value/focus in place. |
| Links | `collect_inline` propagates an `a[href]` `LinkTarget` down the inline tree; each text fragment carries it. `scene_from_layout` groups fragments by (anchor, visual line) into non-painting `RenderObjectKind::Link` hit regions plus `RenderInteraction::Link` on the text objects; URL resolution uses `base_url` via `resolve_url`. |
| Decoded images | `image_dimensions` combines specified width/height with intrinsic size (aspect-preserving). `ImageFragment.decoded` carries `Arc<RasterImage>`; the scene emits `RenderObjectKind::Image` with `intrinsic_width/height` and the decoded pixels, else `ImagePlaceholder`. Backgrounds from `background-image: url(...)` are also tracked. |
| Paint order / styling | `ResolvedPaintStyle` carries color, background, borders (per side), corner radii, box/text shadow, gradients, opacity; `scene_from_layout` emits `Box`, `Rectangle`/`RoundedRectangle`, `Border`/`BorderSides`, `BoxShadow`, `Gradient`, `Text`, `Line`, list `Marker`. |

## 2. New pipeline (rappid-rabbit/src/layout.rs)

```
Document + StyleContext + viewport + FontMeasurer + ImageMetrics
  -> layout_document             (build -> measure_all -> arrange -> flatten)
       -> LayoutResult { geometry: LayoutTree<LayoutBox>, render_boxes: Vec<RenderBox> }
  -> into_document_scene         (f32 -> integer retained scene)
```

* `LayoutBox` is a borrowed arena record with `content/padding/border/margin`
  boxes, resolved `LayoutStyle`, `BoxType` (`Block`/`Inline`/`TextRun`/`Image`),
  and intrinsic sizes.
* `RenderBox` is the flattened, paint-ready IR: `node_id`, `bounds`,
  `ResolvedStyle`, `BoxType`.
* `into_document_scene_with_images` emits `Box`, `Rectangle`, `BorderSides`,
  `Text`, decoded `Image`/`ImagePlaceholder`, per-line `Link` regions, and
  `Control` objects. `into_document_scene` uses the same adapter without image
  resources or URL resolution. Shadows, gradients, and list markers remain
  unsupported.

Coverage today: normal block flow, anonymous inline formatting contexts,
common adjoining-margin collapse, images, and single-line flex row/column.
Single-line form controls (`input` text/search/password/submit/button, and
`select`) are laid out as intrinsic-sized replaced widgets and emitted as
`RenderObjectKind::Control`; hidden inputs and `[hidden]`/`display:none`
subtrees are pruned during tree construction.  Floats, tables, `textarea`,
multi-line flex wrapping, and paint effects fall back to normal flow.

## 3. Where `layout_document` can be invoked

`layout_document` needs only `(&Document, &StyleContext, Size, &dyn FontMeasurer,
&dyn ImageMetrics)`, all of which `DocumentRenderState` already owns or can
derive (`RABBIT_FONTS` is a `TextMeasurer`; the cache adapter implements
`ImageMetrics`). `new_engine_scene` provides this adapter for both
`DocumentRenderState::new_with_images` and `DocumentRenderState::rebuild`.
The legacy layout tree is retained for inspector correlation.

## 4. Runtime fallback switch

`DocumentRenderState` carries a `LayoutBackend` (`New` default, `Legacy` fallback).

* `DocumentRenderState::set_backend(LayoutBackend, &dyn TextMeasurer) -> bool`
  switches the active engine and rebuilds from the retained DOM/styles/images.
  It is a pure relayout: the DOM, computed styles, and image cache are not
  reparsed or refetched.
* `DocumentRenderState::backend` and `DocumentRenderState::diagnostics` expose
  the active engine and its per-scene object/feature counts.
* In the live browser, **Ctrl+E** calls `RabbitApp::toggle_layout_backend`,
  which flips the backend, clamps scroll to the new scene, repairs control
  patches, and logs `Switched layout engine to Legacy/Modern: <summary>`
  plus a fresh `DocumentCanvas` scene line to the developer console.
* Startup logs `[info][browser] Layout engine: Modern (Two-Pass Arena)
  [Ctrl+E to toggle Legacy]`. No compile feature changes the default:
  construction and navigation start with `LayoutBackend::New`.

To select the fallback in tests or tooling, call
`set_backend(LayoutBackend::Legacy, measurer)` after construction.

Unsupported new-engine features are **reported**, never silently replaced:
`unsupported_features_new_engine` scans the DOM/computed styles and lists the
paint-affecting features the new engine currently flattens to plain block flow
(`float`, `table`, `background-image`, `box-shadow`,
`text-shadow`, and `form-control` for the widgets it still approximates, i.e.
`textarea` and unsupported `input` types).  The list appears in the
`unsupported=[...]` field of the console diagnostic and in
`LayoutDiagnostics::unsupported`, so a developer switching to the new path sees
exactly what changed instead of a silent placeholder.

## 5. Increment sequence

1. Switch + adapter (`into_document_scene_with_images` producing decoded images
   and per-line link hit regions) — **done**.
2. Comparison harness: same DOM/styles/viewport/fonts/images -> compare box
   geometry, paint order, text wrapping, link hit regions — **done** for the
   text/anchor/image cases; broadened as more fixtures are added.
3. One feature group at a time; images (decoded) first because resize/relayout
   must use updated viewport and resource dimensions — **done** for decoded
   images.
4. Form controls and visibility pruning — **done** for single-line inputs,
   buttons, and collapsed `select`s: intrinsic sizing from `size`/label/active
   option, `[hidden]`/`display:none`/`input[type=hidden]` pruned before any
   `LayoutBox` is created, and UA border/background defaults applied only where
   author CSS is silent.
   * Every single-line text-like input type (`text`, `search`, `password`,
     `email`, `tel`, `url`, `number`, `date`, `datetime-local`, `month`, `week`,
     `time`) is modelled as a size-column field.  `checkbox`, `radio`, `file`,
     `range`, and `textarea` still fall back to block flow and are reported.
   * Explicit author `width`/`height` on a control honours `box-sizing`:
     `border-box` keeps the declared outer size, `content-box` adds used
     padding and border.  This is what keeps a `border-box` control aligned
     with the surrounding form fields.
   * Tests: `form_control_explicit_width_honours_box_sizing`,
     `text_like_input_types_are_modelled_and_sized`,
     `unsupported_input_types_do_not_become_widgets`,
     `new_engine_hidden_inputs_never_reach_the_scene`,
     `new_engine_sizes_text_like_inputs_without_flagging_them`.

5. Block-container fidelity (`form`/`center`/`fieldset`/…) — **done**.  Two
   compounding bugs made the new engine drop whole subtrees on real pages
   (notably Google's hero form and buttons):
   * `user_agent_stylesheet` listed only a subset of HTML block-level tags;
     `form`, `center`, `fieldset`, `legend`, `figure`, `figcaption`, `hgroup`,
     `menu`, `details`, `summary`, and `dialog` fell back to the CSS initial
     `display: inline`, so their block children were orphaned by inline flow
     and produced 0x0 boxes.  They are now block-level.
   * `LayoutBox::is_block_container` accepted only `display: block | flex`;
     any keyword the engine does not model (`-webkit-box`, `grid`,
     `contents`, …) became an unarranged `BoxType::Block` whose element *and*
     subtree were silently dropped (e.g. Google's `.gb_4` "Sign in").  A
     `BoxType::Block` is now arranged unless its display is explicitly
     inline-level.
   * Tests: `form_and_center_are_block_containers`,
     `unmodelled_display_values_keep_their_subtree`,
     `centered_submit_button_keeps_its_intrinsic_box`.

6. Alignment and box-model refinement — **done** (2026-11-06).
   * Block horizontal auto margins absorb free space after width, min/max,
     padding, border, and `box-sizing` resolve. Both auto margins split positive
     free space; one auto margin absorbs it after the fixed margin. Auto-width
     blocks still fill available space unless constrained; intrinsic-width
     block images retain their natural size. Overflow does not create negative
     auto margins.
   * Child placement translates from the freshly arranged border edge. Block
     top margins are applied once rather than twice, and margin boxes now
     include the resolved margins.
   * Inline measurement and arrangement share whitespace and wrapping rules.
     Anchor boundaries no longer invent spaces; whitespace-only text nodes
     preserve separators, and non-breaking spaces remain within words.
     Inherited `text-align` offsets every completed visual line, including
     atomic inline-blocks, controls, and images. `white-space: nowrap` prevents
     soft wrapping, and `<br>` ends the visual line.
   * Inline-blocks establish their own formatting context and preserve their
     declared or intrinsic border box, including block children. The UA
     stylesheet gives `<center>` inherited center alignment. Controls retain
     zero UA margins, 5px/7px padding, and 1px borders; anchors retain zero
     default padding/margins and use actual DOM whitespace for separation.
   * Text and control captions share an approximate ascent/descent baseline;
     padding, borders, margins, and taller controls contribute to line height.
     Images and other atomic boxes use their bottom edge as the baseline.
     Default line height now includes the measured face height.
   * Images resolve CSS width/height before HTML dimension hints, including an
     explicit CSS `auto` override. A single dimension preserves the decoded
     `ImageMetrics` aspect ratio; two explicit dimensions are honored. Width
     constraints and percentage widths participate in measurement and layout,
     and following blocks clear the complete image line. The obsolete
     `image-sizing` unsupported diagnostic was removed.
   * Link hit regions are built from the final aligned text rectangles, with
     one union per anchor/visual line and references to the exact text objects.
     Integer conversion rounds bounds outward, including negative coordinates.
   * The HTML background, or body background when HTML is transparent,
     propagates to the document canvas. The canvas covers at least the
     viewport and grows with content; local element backgrounds retain their
     box boundaries. This intentionally improves on the legacy short-body
     background boundary.

## 6. Verification

Default promotion validation:

* `RUSTFLAGS='' cargo test -p rappid-rabbit --features dom --lib --target
  x86_64-unknown-linux-gnu` — 180 passed (including initial Modern scene and
  fallback round-trip coverage).
* `cargo build -p rappid-rabbit --features dom --release` — passed for the
  configured freestanding target. Existing warnings remain in dependencies
  and pre-existing CSS/legacy-renderer code.
* `cargo build --package sunlight-kernel --features native_document_viewer`
  — passed, including a forced relink with the updated browser built using
  the userspace flags from `tools/build.sh`.
* `rustfmt --check --edition 2021 --config skip_children=true rappid-rabbit/src/layout.rs
  rappid-rabbit/src/render.rs rappid-rabbit/src/main.rs` — passed.
* Dual-engine regressions `margin_auto_centers_block_in_container`,
  `inline_links_preserve_horizontal_flow_and_hitbox`, and
  `text_align_right_aligns_navigation_items` compare final coordinates and
  link bounds from the same DOM/styles/fonts/viewport. Link tests cover
  adjacent anchors, actual spaces, nested styles, non-breaking spaces, and
  wrapping, and verify each hit region equals its visual text union.
* Layout regressions cover one/two auto margins, padding/borders, border-box,
  max-width, overflow, inherited alignment on wrapped lines, nowrap,
  intrinsic inline-blocks, control caption baselines and spacing, and image
  CSS/HTML sizing and aspect ratios. A Google-style header/logo/search-form
  fixture checks centering and non-overlap at 320px and 1024px viewports.
* Live visual verification gates — **met**, as reported in the promotion
  handoff from SunlightOS / QEMU: Google has correct centering, standard search
  input and action buttons, Sign In button, clean links, and no hidden-input
  leaks. Arch Linux (`archlinux.org`) has its dark navigation bar, article
  boxes, and typography rendering cleanly. These live checks were completed
  before this promotion; this change does not claim a new QEMU visual run.
* Legacy-specific tests select `Legacy` explicitly, preserving its geometry,
  paint effects, text fragmentation, and metadata checks. Dual-engine fixtures
  continue to compare distinct backends; general tests exercise the new default.

Earlier bring-up verification (before this alignment pass): the kernel build
with `native_document_viewer` linked, and a headless QEMU boot reached
`[LOGIN-TUI] first frame complete` / `Login screen ready`. The earlier
freestanding-parser Google repro painted the body and submit controls and
pruned hidden inputs. These historical checks do not establish visual parity
for the current changes.

## 7. Known remaining rendering differences (new engine)

* Floats and tables are not yet interpreted; they fall back to normal block
  flow and are reported via `unsupported`.  `textarea` and unsupported `input`
  types are likewise approximated and reported as `form-control`.
* Font metrics expose line height but no actual face ascent/descent. Baselines
  therefore use an approximation; `vertical-align`, sub/sup offsets, and the
  last-line baseline of inline-blocks still need full handling. Floating-point
  line heights can round outward by a pixel compared with legacy integer
  geometry. Tall-face defaults now agree, pinned by
  `default_line_height_honours_measured_face_in_both_engines`.
* Complete inline decoration geometry and preformatted whitespace, complex
  line breaking, flex sizing/wrapping, and full margin-collapse semantics
  still need broader parity coverage.
* Background images, box/text shadows, gradients, corner radii, and list
  markers are not emitted by the new scene builder.
* Image `object-fit`/`object-position` and complete replaced-element paint
  decoration are not implemented.

## 8. Milestone 5 — Default Promotion & Visual Parity: Completed

Both `LayoutBackend::default()` and `DocumentRenderState::new_with_images`
select `New`, including initial scene creation and unsupported-feature
reporting. Google and Arch Linux have met the live visual gates described
above. Ctrl+E continues to switch between Modern and Legacy and logs the active
backend at info severity with diagnostic counts.

Maintenance contract:

* `New` is the primary engine for new documents, navigation, resize, and image
  relayout. New layout development and regression coverage target this path.
* `Legacy` remains available as a maintained diagnostic and rendering fallback.
  Ctrl+E relayout retains the DOM, styles, images, and browser form state and
  clamps scrolling to the selected scene. Navigation creates a new document
  with the Modern default; the toggle applies to the current document.
* Keep explicit Legacy and dual-engine tests passing. Preserve unsupported
  feature diagnostics for the accepted limitations in section 7 so developers
  can identify when the fallback is useful. Removing Legacy requires a future,
  separate migration decision.
