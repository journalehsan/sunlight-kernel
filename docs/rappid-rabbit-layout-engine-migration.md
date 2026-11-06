# Rappid Rabbit layout engine migration

Status: **Legacy engine decommissioned; modern engine permanently adopted**
(2026-11-06).

The two-pass arena engine in `rappid-rabbit/src/layout.rs` is the sole document
renderer. The legacy builder, layout types, scene converter, backend enum,
runtime dispatch, and comparison tests have been removed. Ctrl+E no longer
switches renderers. Startup identifies the modern engine; scene diagnostics
report object counts and unsupported features without backend-switch messages.

## Canonical rendering pipeline

```text
HTML -> Golden Fish Document -> StyleContext computed-style arena
  -> DocumentRenderState::new_with_images / rebuild
  -> render_document (browser font and image-cache adapters)
  -> layout_document (build -> measure -> arrange -> flatten)
  -> into_document_scene_with_images
  -> DocumentScene -> DocumentCanvas (paint and hit test)
```

All construction, navigation, viewport changes, and image completions use this
same pipeline. Layout borrows DOM text and style data; the scene owns its text,
resolved URLs, and shared decoded image pixels. No alternate renderer or
fallback layout pass runs behind the scene.

`DocumentRenderState::layout_geometry` retains only node IDs and the modern
arena's content, padding, border, and margin rectangles for the DOM inspector.
The inspector therefore describes the geometry that actually produced the
scene. It no longer reports floats/tables/shadows calculated by another engine.

`rebuild_for_viewport` avoids work when dimensions are unchanged.
`rebuild_for_images` reuses the retained DOM and styles with refreshed image
resources. Rebuilds update inspector geometry, diagnostics, scene generation,
and `diff_scenes`; form value/focus patches and browser scrolling keep their
existing retained-scene behavior. Identical inputs produce identical scene IDs
and an empty patch. Object ownership continues to use Golden Fish node IDs.

## Text and minimalist documents

The user-agent stylesheet supplies defaults through the ordinary CSS cascade,
so author styles override them:

- Headings h1–h6 have bold, descending text sizes and vertical margins.
- Paragraphs and preformatted blocks have one-em vertical margins; adjoining
  block margins use the engine's existing collapse rules.
- Blockquotes are indented, lists reserve a 40px marker gutter, and horizontal
  rules paint a one-pixel gray top border with vertical spacing.
- Anchors are blue and underlined, with one tight hit region per visual line.
- Code and preformatted text use monospace. Preformatted text preserves spaces,
  explicit newlines, blank lines, and eight-space tab stops, and can overflow
  horizontally. `pre-wrap` preserves whitespace while allowing wrapping.
- List items generate owned markers outside the text column, including nested
  bullets and ordered labels with `ol[start]` and `li[value]`. Hidden items are
  pruned before numbering; `list-style:none` suppresses markers.

The deterministic `minimalist-page.html` and `text-centric-page.html` fixtures
exercise the patterns used by pages such as example.com and stallman.org. They
are representative local documents, not snapshots or live-site screenshots.
Tests cover centered columns at wide and narrow widths, all six headings,
paragraph gaps, blockquote/list indentation, link styles and hit regions,
monospace text, horizontal rules, nested list ownership, visible ordinals,
preformatted whitespace/overflow, and stable scene/inspector rebuilds.

## Remaining layout limits

Normal block and inline flow, common adjoining-margin collapse, intrinsic image
sizing, single-line flex rows/columns, and single-line form controls are
supported. Floats, full table layout, textarea and other unsupported controls,
and multiline flex wrapping are approximated by normal flow. Single-URL CSS
background images now use the shared image painter; the supported subset is
listed in `rappid-rabbit-images.md`. Gradients, box shadows and text shadows
remain unsupported by the painter.
Diagnostics continue to report float/table/control and unsupported background/shadow
features. List markers currently use outside placement and decimal or basic
bullet labels; inside placement and alphabetic/Roman numbering are not modeled.
There is no runtime route back to the retired renderer.

## Validation

The workspace defaults to the freestanding kernel target. Run Rabbit tests on
the host, then compile the native release and kernel embedding:

```sh
CARGO_BUILD_TARGET=x86_64-unknown-linux-gnu cargo test -p rappid-rabbit
bash tools/build_rappid_rabbit.sh
cargo build --package sunlight-kernel
rustfmt --edition 2021 --config skip_children=true --check \
  rappid-rabbit/src/render.rs rappid-rabbit/src/layout.rs \
  rappid-rabbit/src/css.rs rappid-rabbit/src/main.rs
```

The native release uses SunlightOS std and its reclaiming 128 MiB allocator.
The kernel build invokes the native Rabbit wrapper in its embedded-build target
directory and stages the resulting ELF for `include_bytes!`. Host fixtures
validate layout and interaction metadata, not live DNS/TLS or guest pixels.
