# Golden Fish progressive document presentation

## Architecture and compatibility

Previously, Rabbit's navigation worker collected the main response, parsed the
entire Golden Fish DOM, loaded/applied stylesheets, fetched/decoded the first
32 discovered images, and built the two-pass layout and retained scene. Only
then did its one-shot result replace the loading label in DocumentCanvas.
Images used resolved URLs in ImageCache; stable scene object IDs belonged to
individual DOM nodes. Raster pixels were shared through Arc<RasterImage>.

Navigation now publishes the complete parsed/styled DOM and initial scene before
image fetching. Text, backgrounds/colors, borders, hyperlinks and form controls
use the existing renderer. Images remain layout objects and initially paint
placeholders. HTML/CSS pixel dimensions reserve both geometry and intrinsic
wrapper contributions; unknown intrinsic sizes keep the existing 160×100
fallback until dimensions arrive. Single-axis sizing uses the intrinsic aspect
ratio once available. Existing failed unsized icons still become 16×16.

The UI owns widgets, forms, scroll, presentation admission and resource commits.
The original single blocking worker still owns navigation parsing, CSS and
layout/scene construction. Image preparation uses a separate single blocking
worker built with the existing Tokio infrastructure. One shared atomic lane
guard admits at most one image job across navigation generations, including
uncancelable old IO. New navigation/layout can run while old image IO finishes;
new image work waits for that lane without accumulating canceled closures.
There is no parallel DOM execution or second rendering engine/widget.

Every image job returns owned cache state and diagnostic records, never DOM
references or callbacks. Navigation drops its image/reflow receivers. Results
must match the navigation generation; scene updates also match viewport version,
image revision and DOM revision. A canceled image response is discarded before
decode when possible. UI commits reapply current form state, preserving edits,
selection and focus made during worker execution. Image updates retain scroll
except the existing clamp when geometry makes a position invalid.

**Normal Rabbit navigation does not execute SunJS.** `js-dom-fixture` is the
existing explicit host execution bridge. This change leaves that boundary and
all script/listener behavior intact. It creates no DOMContentLoaded, load,
image-load or error events. Resource completion continues for every admitted
image, including distant and hidden images, regardless of paint admission or
whether layout/presentation is being polled. A test deliberately completes all
resources without running layout updates or presenting any images. Parse-ready,
resource-ready and presentation-ready are separate concepts. If normal browser
script execution is introduced later, its load contract must use resource
completion, never the presentation filter.

## Incremental presentation and layout

ImagePresentation admits up to four new visible/nearby objects per UI tick.
Visible objects precede those within one viewport above or below; distant
objects remain deferred. Resource requests use the same priorities, including
CSS backgrounds that have geometry but no decoded paint object yet. An active
request is not preempted. Scrolling either way, resize and scene revisions
reconsider the policy. Replaced pixels requeue admission; removed objects and
old generations release their entries. Stable idle views skip rescanning.

DocumentCanvas accepts an optional image-ID filter. Its default remains eager
for Document Viewer and other consumers. The filter changes only raster
painting; it does not affect hit regions, layout, resource readiness or scripts.
The canvas already clips objects to the viewport and ancestor overflow bounds.
Nearby admission alone does not request a visible repaint.

Resource revisions coalesce into worker scene updates. Fixed pixel dimensions,
unchanged intrinsic dimensions and CSS backgrounds use cached geometry and
retained object patches, without layout. Other changed intrinsic metrics and
viewport changes conservatively use the canonical layout engine. Background
capture and repaint share the same sizing, tiling, ordering and clipping code.
Pixel-budget eviction retains bounded intrinsic metadata so an evicted image
does not revert to unknown geometry. Repeat completions are no-ops, failed
resources are terminal, and attempts are bounded to 32 per navigation, including
fixture DOM mutations. Changed DOM revisions rediscover image URLs without
reexecuting scripts or stylesheets.

ScenePatch retains bounded dirty regions. Offscreen-only patches do not request
a document repaint unless content extent changes; visible patches use the
existing whole-surface repaint path. DocumentCanvas/window composition does not
support region-only repaint, so text and existing visible images are repainted
on those frames. There is no replacement compositor in this change.

## Measurements

`[RABBIT][PRESENT]` records navigation-start elapsed microseconds at first actual
document draw, first clipped raster pixel write, completion of the current
visible image set, and terminal completion of admitted image resources. Visible
completion resets when the scene/view changes. Transparent-only images may
complete without a first pixel write. Counters report Render draws, request-gate
cache hits/misses and canonical layout runs/passes. A shared counter includes
superseded worker layout runs; one canonical run is two layout passes.

The offline release benchmark runs real Golden Fish parsing, CSS, layout,
decoding, retained patches and framebuffer painting at 640×480. Each image has
a controlled 30 ms preparation delay. These are host renderer/navigation-fixture
timings, excluding main-document HTTP, external CSS, native IPC, UI tick delay
and guest display presentation. They do not establish native browsing latency
or network throughput. Five sequential samples per fixture use the committed
8×8 JPEG and 2000×2588 progressive JPEG, without external network access.

The original implementation was measured before editing (HEAD `79d8b7b19`).
Medians in milliseconds, before → progressive:

| Fixture | Initial document | First visible image | Visible images complete | All admitted resources/page ready |
| --- | ---: | ---: | ---: | ---: |
| Text only | 0.503 → 0.793 | none | 0.503 → 0.793 | 0.503 → 0.793 |
| Large JPEG | 60.622 → 0.309 | 60.622 → 50.680 | 60.622 → 50.680 | 60.622 → 50.680 |
| ArchWiki style, two visible/one distant | 92.331 → 2.267 | 92.331 → 32.657 | 92.331 → 63.252 | 92.331 → 93.438 |

All fixtures used one canonical layout run/two passes. Repaints changed from
1 to 1/2/3 for text/large/ArchWiki respectively. Request cache misses remained
0/1/3, with zero hits; repeated DOM image URLs share decoded pixels.

A same-build `--legacy` control reproduces the former fetch-all-before-publish
sequence. Its initial/page-ready medians were 0.949/52.013/92.225 ms. Total
readiness varies by fixture: the large JPEG is slightly faster than that
control, while ArchWiki is slightly slower. The measured benefit is earlier
usable content and incremental multi-image presentation; these measurements do **not** demonstrate a consistent
total-page-ready speedup. Host scheduling/decoder variance affects these small
samples. Raw original/control/after logs are `target/progressive-before.log`,
`target/progressive-legacy.log` and `target/progressive-after.log`.

Reproduce the control and staged measurements after building, with no other
builds running. Leave RABBIT_EVIDENCE_DIR unset for timing samples:

```sh
cargo build -p rappid-rabbit --example progressive_presentation --release --target x86_64-unknown-linux-gnu
target/x86_64-unknown-linux-gnu/release/examples/progressive_presentation --legacy
target/x86_64-unknown-linux-gnu/release/examples/progressive_presentation
RABBIT_EVIDENCE_DIR=target/progressive-evidence target/x86_64-unknown-linux-gnu/release/examples/progressive_presentation
```

The evidence option exports initial/final PPM framebuffers in a separate run.
Final control/progressive framebuffers matched byte-for-byte for all three
fixtures. Tests additionally compare late-background pixels and fixed-image
scenes against full canonical reflow, including inline-block, flex, table and
float contexts. Existing captured Arch navbar/home/wiki regressions also pass.

## Regression evidence

Commands executed:

```sh
cargo test -p rappid-rabbit --features js-dom-fixture --target x86_64-unknown-linux-gnu --lib --bins --tests
cargo test -p golden-fish -p sun-img -p sunlight-ui -p sunlight-fetch --features sunlight-ui/svg --lib --tests --target x86_64-unknown-linux-gnu
cargo test -p sunlight-docview --lib --target x86_64-unknown-linux-gnu
bash tools/build_rappid_rabbit.sh
bash tools/test.sh phase3.0
```

Browser tests: 266 passed. Shared tests: 378 passed. Document Viewer: 11 passed.
Native Rabbit release build passed. QEMU Phase 3.0 boot gate passed, ending in
`[SunlightOS] Foundation Complete` and `Phase 3.0 gate PASSED`. The boot gate is
not a native browser interaction/visual test. Logs are under
`target/progressive-{tests,shared-tests,docview-tests,native-build,qemu}.log`.
Existing unrelated compiler warnings remain; no dependencies were added.

The new browser tests cover document-first publication with a withheld image
channel; text-only/large/multiple images; both-direction scrolling and nearby
priority; batched completions and bounded admission; resize during image work;
scaled scene geometry at the policy's zoom boundary; generation cancellation;
independent resource readiness; worker failure/decode failure; changed image
src and hidden-to-visible images through SunJS; listener ordering/no paint load
events; preserved form value, selection, focus and scroll; stable idle ticks;
cache-budget geometry; and 32 repeated navigation/drop cycles with weak raster
references proving release. The lane test prevents queued work during 100 polls
and demonstrates the navigation worker remains available.

For a local HTTP fixture, with delayed image responses and deliberate failure:

```sh
python3 rappid-rabbit/tests/fixtures/progressive/serve.py --image-delay-ms 150
# Host: http://127.0.0.1:8765/archwiki.html
# QEMU user networking: http://10.0.2.2:8765/archwiki.html
# Also: /text.html, /large.html, /failure.html
```

The fixture server's HTML, JPEG and failure routes were exercised locally.
SunJS currently has no createElement/appendChild support: tests change src,
reveal an existing image into the layout and remove/update supported DOM content.
They do not add a new DOM API solely for this work. Rabbit also has no user zoom
control; scaled-coordinate policy tests cover its future layout boundary, not
a native zoom UI. Native browser scrolling/visual interaction and repeated
native window/heap measurements remain unverified in this run; reference-release
tests do not prove the absence of every possible native process leak.

## Limits and changed files

Resource response/decode limits, 32 cache entries, 28 MiB decoded cache, 4096
background tiles and the 128 MiB process heap remain bounded. Cache pressure can
leave pixel-evicted images as dimension-preserving placeholders until a later
cache replacement/reload; this work does not add a general resource refetcher.
Unknown dimensions can still reflow. Complex intrinsic/percentage sizing uses
conservative full layout; visible frames still repaint the full surface.
A slow in-flight old image request can delay the next image request until its
existing transport timeout, while document navigation/input/layout proceed.
Two bounded workers can overlap image preparation with navigation allocations;
the existing native working-set check is conservative, not an arbitrary-page
OOM guarantee. Huge/tiled visible image sets need multiple admission ticks.

Implementation files:

- `rappid-rabbit/src/main.rs`: Stage A handoff, resource lane, UI scheduling,
  cancellation, diagnostics/metrics and seven runtime regression tests.
- `rappid-rabbit/src/presentation.rs`, `rappid-rabbit/src/lib.rs`: viewport
  admission policy and module export.
- `rappid-rabbit/src/render.rs`, `rappid-rabbit/src/layout.rs`,
  `rappid-rabbit/src/backgrounds.rs`: resource staging, geometry-preserving
  patches, intrinsic dependencies and shared background repaint geometry.
- `rappid-rabbit/src/images.rs`: bounded intrinsic metadata after pixel eviction.
- `rappid-rabbit/src/developer_tools/network.rs`: merge resource diagnostics
  without replacing pane focus, selection or scroll.
- `sunlight-ui/widgets/document_canvas.rs`: optional image filter and actual
  raster-write counter; existing consumers retain eager behavior.
- `rappid-rabbit/tests/progressive_images.rs`: eight rendering/JS/cache tests.
- `rappid-rabbit/tests/fixtures/progressive/{text,large,archwiki,failure}.html`
  and `serve.py`: offline fixtures and delayed-resource server.
- `rappid-rabbit/examples/progressive_presentation.rs`: repeatable benchmark and
  framebuffer export; `rappid-rabbit/Cargo.toml` gates it on the existing dom
  feature.
- `docs/rappid-rabbit-progressive-presentation.md`,
  `docs/rappid-rabbit-images.md`, `docs/rappid-rabbit-native-runtime.md`: report
  and updated architecture references.

No commits or pushes were made.
