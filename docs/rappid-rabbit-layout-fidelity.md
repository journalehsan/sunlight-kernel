# Google and Arch layout investigation

The modern arena renderer remains the only renderer. This change adds no
JavaScript engine, site selectors, URL dispatch, alternative renderer, image
path, network policy, or worker/UI ownership changes.

## Reproduction and sources

Evidence is generated under `target/rabbit-layout-evidence/`, outside version
control. Baseline screenshots are `google-before.png` and `arch-before.png`.
The desktop framebuffer is 1280×800 and Rabbit's content viewport is
1056×642, with content origin (197,148). The first Google baseline used TCG;
the Arch baseline used KVM. Both used the original ISO, 2 vCPUs, 3072 MiB RAM,
and identical window/content dimensions. The original ISO is preserved as
`baseline.iso` with its SHA-256 in `baseline-hashes.txt`.

The host captures include response headers, timestamps and hashes in
`capture.json`, complete HTML, the Arch stylesheets including imported `generic.css`, extracted Google CSS,
Arch's `homepage.js`, network resource bodies and a Chromium reference script.
Chromium runs with JavaScript disabled and Rabbit's user agent
`SunlightOS-fetch/0.1`; its reference screenshots and DOM/computed-style trees
are saved at 1056×642 and 756×642. These are references for the captured static
response, not a different browser-user-agent version of Google.

`layout_dump` dumps the modern layout arena, DOM relationships and all computed
properties with winning declarations, selectors and source locations. Its
standalone font metric is deliberately deterministic (half the CSS font size
per character); its geometry is useful for tracing layout, not for claiming
pixel agreement with either browser's installed fonts. Missing image resources
in this host-only probe use the engine's usual placeholder metrics. Guest
capture geometry includes the actual Rabbit fonts and decoded resources.

```sh
cargo run -p rappid-rabbit --example layout_dump \
  --target x86_64-unknown-linux-gnu -- page.html 1056 642 [sheet.css ...]
```

Rabbit's optional `--trace-document` switch records exact response bytes as
bounded hexadecimal serial records, external CSS, and the DOM/layout tree with
relevant computed properties. It is off by default and executes on the existing
navigation worker. Capture records use `[RABBIT][CAPTURE]`; geometry records use
`[RABBIT][TREE]`. Text bodies are not silently reconstructed from screenshots.
This diagnostic output can contain private page content; use it only for pages
you intend to capture.

## Verified causes

| Element | Source CSS and reference behavior | Baseline behavior / cause |
| --- | --- | --- |
| Google `#gb > .gb_de.gb_4d` | `float:right; display:flex; height:48px; box-sizing:border-box; align-items:center; justify-content:flex-end`, with 32px left and 4px right padding | Float ignored. More seriously, flex alignment received the viewport's height rather than the declared container content height. The links and Sign in appeared beside the search buttons, hundreds of pixels below the header. |
| Google `.gb_Od` | A direct flex item; its `float:right` has no effect on flex-item placement. Its children are an inline apps control and an inline-block Sign in anchor. | Item measurement omitted item padding from flex intrinsic contributions. Once the outer float shrank to fit, this omission made the row too narrow and wrapped the Sign in anchor onto another line. |
| Google `a.gb_6a.gb_0d` padding cascade | Equal-specificity later `.gb_Ta a.gb_0d` sets side padding 12px, overriding the earlier 24px. | Per-rule declaration indices were added to a rule counter that advanced by only one. An earlier long rule could outrank a later short rule. The used control width was therefore also wrong. |
| Google `a.gb_6a.gb_0d` | `min-height:40px; box-sizing:border-box; padding:10px 12px; margin:12px 16px 12px 10px; line-height:18px`. Reference top=10px, height=40px, center y=30px. | The anchor is static HTML. The same navigation row renders correctly in Chromium without JS. Its margins do not imply placing it below the navigation. |
| Arch `#content-left-wrapper` | `float:left; width:100%` | Treated as an ordinary block. |
| Arch `#content-left` | Right margin 340px reserves the sidebar plus a 40px gutter. | The reserved space existed; the sidebar did not occupy it. |
| Arch `#content-right` | `float:left; width:300px; margin-left:-300px`. It follows the entire main column in source but sits beside it visually. | Sidebar markup was present, below the viewport. The engine ignored float and clamped the computed -300px margin to a used value of zero. At 1056px, the host probe placed its top near 2013px, rather than at the content top. |
| Arch `#news h3` / `#news h3 a` | Heading floats left and shrinks to its text. Its block anchor supplies `background:#1794d1; color:white; font-size:15px; padding:2px 10px`. | Background already painted. Without float shrink-to-fit, the heading and anchor expanded across the main column (about 663px in the probe), instead of the reference label's roughly 108px. No background decoder change was required. |
| Arch `#news .rss-icon` | `float:right; margin-top:1em`; child SVG has intrinsic size 8×8. | Inline normal flow placed it below the label at the left. Decode succeeded; its containing anchor's layout was wrong. |
| Arch `#news h4` | `clear:both` separates the first article from the floated heading/RSS row. | Clear was ignored. |
| Arch `.timestamp` | Right float with a negative top margin. | Negative margins were clamped and floating text needed to remain an inline text run inside the float. |

The current Arch structure uses floats, not grid, flex columns, positioning or
JS-created sidebar content. Its stylesheet's Last-Modified is 2026-01-25; the
navbar stylesheet is byte-identical to the prior saved capture. The news and
sidebar hierarchy also remains present in the prior HTML. Live package rows and
Google response tokens vary; old screenshots alone were not used as a layout
specification.

Arch's script creates package-search suggestions after input and positions that
suggestion list. It does not create the sidebar, news heading or RSS link.
Google's static response also contains its Sign in control. None of the target
layout corrections requires JavaScript. This does not claim that either site's
interactive application features work without JavaScript.

The layout rules are based on [CSS 2 floats and clearing](https://www.w3.org/TR/CSS2/visuren.html#floats)
and [Flexbox cross-size determination](https://www.w3.org/TR/css-flexbox-1/#cross-sizing).
The implemented subset and remaining limits are stated below.

## Implemented behavior and limits

- Preserve declaration source order across whole rules and stylesheet boundaries,
  including custom properties and inactive media ranges.
- Retain signed horizontal and vertical margins, while keeping padding and
  dimensions nonnegative.
- Blockify floats and direct inline flex items. Ignore float on flex items and
  whitespace-only anonymous flex items; text does not inherit float/clear.
- Place sibling left/right floats outside normal block flow, shrink auto-width
  floats to fit, wrap floats that cannot fit, honor side-specific/both clearing,
  and enclose descendant floats when their container is itself floated.
- Reduce local inline line widths beside floats, move an unbreakable item below
  a float if necessary, and recover full line width below the exclusion.
- Include item border, padding and margins in flex intrinsic/used space;
  account for gaps and main-axis justification; align against the definite content
  cross size or the measured line for an auto-height row. Auto-height flex
  calculations are relative to the container origin, rather than absolute y.

Exclusions and clearance propagate through intervening ordinary blocks sharing
one CSS block formatting context, with coordinate conversion at each content
edge. Descendant floats also affect subsequent siblings. Floats, inline blocks,
flex containers/items and the document root establish independent contexts and
enclose their own floats. This fixes the narrow navbar overlap found in guest
validation without shrinking ordinary block backgrounds or padding boxes.
Late floats following partially constructed inline lines, full parent/child
margin collapse, overflow-created contexts and avoidance between separate BFCs
remain incomplete. Flex auto margins, grow,
shrink/basis, wrapping, baseline/align-self/stretch and grid are not implemented
by this change. Existing absolute/fixed positioning, inline SVG, negative
text-indent (Arch logo fallback text), border triangles (the small news arrow),
font-shorthand/typography, unitless line-height, fractional font-size rounding,
and full table layout limitations remain. For example, the news link's
`line-height:1.4` still falls back to 1.2 in layout, giving a 22px label height
instead of Chromium's 25px. Its text width also differs with the installed
font metrics. These are
renderer/CSS limits; they are not evidence of a JS dependency.

## Deterministic regression coverage

`tests/fixtures/google-navigation-layout.html` retains the floated, padded 48px
flex navigation and inline control structure. Its test checks row/link/control
alignment at 1056px and 756px with three different viewport heights, proving
that alignment does not depend on viewport height.

`tests/fixtures/arch-home-layout.html` retains the full-width left float,
negative-margin sidebar, block blue anchor, RSS image and clearing article
heading/footer. Tests assert geometry, signed margin, background color and
column gutter at both widths. Additional cases cover padding/borders/margins/
gaps, auto-height flex at a nonzero origin, float wrapping/side-specific clearing,
inline width recovery, floated text with a negative vertical margin, avoiding
double-counted control border/padding in flex rows, inherited exclusions through
nested padded blocks, descendant clearing and inline-block context boundaries.

## Controlled capture comparison

`guest-captures.json` records validated byte counts and SHA-256 hashes of the
actual guest HTML and CSS. The final reference run uses those exact bodies,
including the imported Arch stylesheet. `google-guest-dom.txt` and
`arch-guest-dom.txt` additionally replay the exact bodies through Golden Fish to
record the complete DOM, including nodes omitted from layout. Native tree
records describe the retained geometry. The computed-property capture records
cascade values; float/flex display blockification is applied in the layout arena.
Google's optional external stylesheet
was empty in both the guest and host request. Its long diagnostic URL label is
resolved unambiguously from the full link in the captured HTML, with that
provenance retained in the manifest.

The original ISO additionally replays these exact documents through a local
fixture server. `google-captured-before.png` and `arch-captured-before.png`
therefore use the same HTML/CSS bytes as the final renderer's
`google-captured-after.png` and `arch-captured-after.png`.
`controlled-comparison.json` verifies the document hashes match. The images are
replayed from the preserved reference resources. `replay-network.jsonl` records
all successful fixture responses and hashes. These controlled captures supplement
the original live baselines; the local fixture URL is visible in their address
bars. The invalid first Arch replay with missing image mappings is retained
separately and is not the comparison screenshot.

`prior-source-comparison.json` confirms that the target ancestor structure in
both prior HTML captures matches the current source, despite changing response
tokens/package rows. The archived navbar CSS is byte-identical.

## Tests and builds

All 586 host checks pass: 204 Rabbit, 31 Golden Fish/parser, 309 shared image/UI,
31 fetch and 11 Document Viewer tests. This includes the existing navigation/
reflow worker, stale-generation/viewport/image-revision, forms, image sizing,
CSS backgrounds, SVG and retained-canvas tests. Commands used:

```sh
cargo test --offline -p rappid-rabbit -p golden-fish --target x86_64-unknown-linux-gnu
cargo test --offline -p sun-img -p sunlight-ui --features sunlight-ui/svg --lib --tests --target x86_64-unknown-linux-gnu
cargo test --offline -p sunlight-fetch -p sunlight-docview --lib --target x86_64-unknown-linux-gnu
bash tools/build_rappid_rabbit.sh --offline
cargo build --package sunlight-kernel
rustfmt --edition 2021 --check rappid-rabbit/src/{css,layout,main,render}.rs rappid-rabbit/examples/layout_dump.rs rappid-rabbit/tests/layout_fidelity.rs
git diff --check
```

Host tests used an isolated Cargo target/cache directory to avoid contention
with the IDE's workspace checks. No workspace/toolchain configuration changed.
The native build, actual kernel embedding build and hybrid ISO creation pass.
`final-hashes.txt` identifies the ISO, kernel and byte-identical staged/embedded
Rabbit binaries. The validated image is also staged at the standard
`target/sunlightos.iso` path with the same hash. Existing warnings in other code are retained in the logs;
these checks do not claim a warning-free workspace.

The full-page dump also exposed the cascade-order failure after the initial
minimized fixtures passed. A dedicated CSS test now covers later short rules,
custom properties and media changes, and the Google fixture retains the two
competing padding rules. Passing fixtures were not treated as visual proof.


## Guest validation

The final ISO boots with all markers from `tools/tests/phase3_0.expected`, plus
`[SunlightOS] Phase 3.0 OK`, `[PMM]`, `[VMM]` and the shell-ready marker. The
check runs against the actual visual-validation boot (`boot-gate.json`), rather
than rebuilding a different feature-specific test image. No panic, OOM,
page-fault or general-protection-fault markers were found.

At the same 1056×642 content viewport (1280×800 framebuffer):

| Page | Before | After | Verified result |
| --- | --- | --- | --- |
| Google | `google-before.png` (live), `google-captured-before.png` (exact replay) | `google-after.png` | Sign in returns to the upper-right navigation. Guest border box (945,10.4), 85×40; JS-disabled Chromium (945,10), 85×40. The deterministic original-engine probe on the exact response gives (933,307.4), 97×40. |
| Arch | `arch-before.png` (live), `arch-captured-before.png` (exact replay, generation 3) | `arch-after.png` | Sidebar starts beside content at y=96 with width 300 and a used -300px margin; 40px gutter. Blue label is 101×22 rather than full column width. The RSS image moves from rounded scene bounds (26,394), 9×9 to (681,334), 9×9; CSS/intrinsic image size remains 8×8. |
| example.com | Prior validated reference in the existing visual-evidence directory | `example-after.png` | Real 577-byte response, paragraph, no script URL token or invented heading. HTTP 200. |
| stallman.org | Prior `target/rabbit-visual-evidence/stallman-final.png` | `stallman-after.png` | 2000×2588 JPEG retained at 320×400; 128,000 portrait pixels painted. Caption and link flow remain visible. HTTP 200. |

`final.serial.log` includes exact HTML/CSS capture records, actual DOM/layout
geometry, resource status/decode/paint stages, navigation generations and worker
reflows. The first capture attempt's diagnostic truncation is retained in
`pre-complete-capture.serial.log`; it is not used as byte-exact evidence.
The completed capture uses 64-byte data packets and 200-byte UTF-8-safe tree
chunks, below the existing 256-byte kernel debug-message limit.

example.com HTTPS failed in the unchanged TLS daemon with
`InvalidCertificate(UnknownIssuer)` / code 50; its HTTP retry succeeded.
Stallman's existing restricted-SVG rejection of `no-facebook.svg` remains.
Neither condition was worked around by changing networking or image limits.

The important layout differences are corrected, while typography/line-height,
logo fallback text, the news triangle, full tables and broader float/flex
formatting remain incomplete as described above. No remaining difference
identified here requires JavaScript; search suggestions and other interactive
site features are separate JS-dependent functionality.

The final guest also replays Arch's preserved response after Stallman
(generation 6), then resizes the window from the baseline viewport to **754×642**. `arch-after-narrow.png` shows
the main text rewrapping, the sidebar remaining beside it, and the RSS staying
at the main column's right edge. The latest run also checks the navbar's line
boxes after fixing inherited float exclusions. The controlled before/after
captures use preserved response bodies under `controlled/`, avoiding changes to
live package rows between navigations. The accepted worker result uses the final viewport version; intermediate
superseded viewport jobs do not replace it. Reflow timings remain diagnostic,
not a performance benchmark. The original wide reference remains tied to the first Arch response.

The final 754×642 controlled reference is
`arch-controlled-reference-754.png`; the earlier image
`arch-before-float-propagation.png` records the intermediate navbar overlap that
prompted the shared-context fix. The final accepted reflow is generation 6,
version 20 (880ms worker time in this run), with the correct current geometry.

Generated evidence and partial attempts stay under `target/`; only the renderer,
diagnostic helper, deterministic fixtures/tests and this report belong in Git.

The guest and temporary localhost replay server were stopped after capture.
`final-guest.png` preserves the last frame. The validated ISO remains available
both as `target/sunlightos.iso` and in the evidence directory.
