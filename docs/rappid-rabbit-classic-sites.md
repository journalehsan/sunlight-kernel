# Arch Wiki, AUR, package listings and DistroWatch

The existing arena renderer now lays out ordinary and nested tables, rather
than stacking every cell vertically. The same style/layout/retained-canvas
pipeline serves every URL. No site-specific selectors or JavaScript dependency
were introduced.

## Reproduction and screenshots

Generated evidence is in `target/rabbit-classic-evidence/` and stays outside
version control. `capture.json` records successful HTTP 200 captures from
`wiki.archlinux.org`, `aur.archlinux.org`, `archlinux.org/packages/`, and
`distrowatch.com`. The Wiki capture follows its redirect to `/title/Main_page`.
These are real page responses, rather than anti-bot pages. The network JSON
files preserve resource URLs, response headers, bodies and SHA-256 hashes.

`comparison-1056.png` and `comparison-754.png` place the original renderer,
JavaScript-disabled Chromium reference, and updated renderer side by side.
Each content viewport is 642px high. The HTML and external stylesheet bodies
are identical between the original and updated host screenshots. Reference
screenshots and computed-style/geometry dumps are named
`SITE-reference-WIDTH.{png,json}`. Individual Rabbit captures are
`SITE-{before,after}-WIDTH.png`.

The new offline `page_snapshot` example renders through
`DocumentRenderState`, Rabbit's actual installed font faces and the shared
`DocumentCanvas`. It writes a PPM framebuffer, which the capture script
converts losslessly to PNG. It does not recreate a page using Chromium or
substitute a screenshot for a rendered document.

```sh
cargo run --offline -p rappid-rabbit --example page_snapshot \
  --target x86_64-unknown-linux-gnu -- \
  page.html https://example.org/ 1056 642 snapshot.ppm resources.tsv
```

The optional manifest has one tab-separated row per resource: absolute URL,
MIME type, local file path. The example loads linked stylesheets in document
order and resolves imports and CSS URLs against their original URLs. For
geometry/cascade investigation, the existing `layout_dump` example remains
available. Missing assets still use the usual image placeholders. The host
probe prioritizes document-order images within the initial cache budget and
adds available background resources. It does not simulate network latency.

## Causes and fixes

| Case | Cause | Engine behavior added |
| --- | --- | --- |
| Package results | `table`, `tr` and `td` all became blocks | Shared column widths, row heights, wrapping inside description cells, header/body group geometry and row backgrounds |
| AUR updates | Fixed layout and overflow were ignored | Fixed column allocation and retained ancestor clipping; hit testing honors those clips |
| DistroWatch | Nested percentage tables and omitted cell/row ends | Automatic table sizing, nested table layout, scoped optional-end recovery, colspan and rowspan placement |
| Legacy markup | HTML table presentation attributes were ignored | Width/height, align/valign, bgcolor, nowrap, cellpadding, cellspacing and border hints below author CSS |
| ArchWiki stylesheet | Its 172,823-byte Vector stylesheet exceeded the 128 KiB per-file cap | A bounded 256 KiB per-file cap, retaining the existing 512 KiB aggregate cap |
| ArchWiki dropdowns | Invisible absolute controls occupied normal flow | Separate positioned-box arrangement, opacity-zero subtree suppression and inherited visibility |
| ArchWiki article | Two columns contain inline-block definition lists | Balanced consecutive block/inline-block children with `column-count` and `column-gap` |
| ArchWiki toolbar/title | `flex-grow` was ignored | Positive row free space distributed to growing items |
| Media/cascade | Print/dark branches and compound width conditions could apply incorrectly | Screen/print matching, light color scheme, all width bounds, comma alternatives, common `calc(px - px)` bounds and `light-dark()` colors |
| Forms and row striping | Attribute selectors and filtered nth-child were rejected | Attribute selectors, compound `:is`, `:not`, `:where`, correct specificity and `:nth-child(... of ...)` |
| Typography/logo fallback | Font shorthand, unitless leading and first-line indent were ignored | Common font shorthand expansion, inherited line-height multipliers and text-indent |
| Google regression | Auto table preferred sizing ignored percentage side cells | Percentage constraints retain the search column's intrinsic width; legacy `center` aligns the table |
| Broken header images | Failed images used a 160×100 unresolved-image box | A failed image uses a small 16×16 fallback while declared dimensions still take precedence |
| Early images | Fetching more initial images than the cache could retain evicted the header before publication | Initial image requests fit the existing 32-entry cache in document order |
| Dense classic pages | Repeated cells retained duplicate computed-style/provenance vectors | Identical immutable styles share an `Arc`; parent styles are borrowed during cascade computation |

Tables use the existing arena and retain row/group/cell boxes for inspection
and painting. Grid width is bounded at 256 columns. Cells establish independent
float contexts. Fieldsets, `flow-root`, and applicable overflow containers also
enclose their floats, keeping search filters above the results.

Parser recovery is shared with Golden Fish. An ancestor end tag closes omitted
intervening elements; a new table/list item closes its optional predecessor
without escaping a nested table/list scope. Uppercase void tags remain void.

Style sharing is scoped to one computed context. Hash buckets compare complete
styles for equality before reusing storage, including winning and overridden
declarations, so a hash collision cannot change CSS or inspector provenance.
Root-relative values resolve before interning. Viewport recomputation builds a
new context; previous worker snapshots keep their immutable styles. A 2,000-cell
test checks sharing, alternating selector matches, inspector declarations and
independent media reflow. Host framebuffer hashes are unchanged by this storage
change for all four full captured pages (`style-sharing-pixel-check.json`).

## Validation

Deterministic fixtures include reduced captured package-list, DistroWatch, and
Vector/ArchWiki patterns. Tests cover both viewport widths, column alignment,
description wrapping, alternating visible rows, spanning cells, nested tables,
HTML hint precedence, vertical alignment, clipped update names/links,
positioned dropdowns, balanced columns, media conditions, selector specificity,
large bounded stylesheets, typography, and Google's centered search table.

```sh
cargo test --offline -p rappid-rabbit -p golden-fish --target x86_64-unknown-linux-gnu
cargo test --offline -p sun-img -p sunlight-ui --features sunlight-ui/svg --lib --tests --target x86_64-unknown-linux-gnu
cargo test --offline -p sunlight-fetch -p sunlight-docview --lib --target x86_64-unknown-linux-gnu
bash tools/build_rappid_rabbit.sh --offline
cargo build --offline --package sunlight-kernel
git diff --check
```

The evidence directory retains build/test logs, original and updated Google,
Arch-home and Stallman host snapshots, QEMU serial logs and desktop captures.
Google's navigation remains at the upper right and its search form stays
centered. Arch's sidebar/float layout and Stallman's 320×400 portrait remain
visible. These checks exposed and corrected the percentage-table regression
after the first minimized tests passed.

All 615 host tests pass: 263 Rabbit/parser, 309 image/UI and 43 fetch/document
viewer checks. Native Rabbit, kernel embedding, hybrid ISO generation, targeted
formatting and diff checks also pass. Existing unrelated build warnings remain.

The final native run uses `release.iso` in QEMU/KVM, two virtual CPUs and
3072 MiB RAM. It passes all 14 `phase3_0.expected` boot markers, Phase 3.0 OK,
PMM and VMM markers. `SITE-release-guest.png` captures all four target pages
and Google in the actual desktop, with a 1056×642 content viewport. The local
replay server serves the captured HTML/CSS bytes; the visible localhost URL is
intentional. Assets absent from the reference capture can return 404, and
those errors remain in the logs. These are static-response validation runs,
not claims about interactive JavaScript behavior.

| Full captured page | Native result after style sharing |
| --- | --- |
| DistroWatch | 10,517 DOM nodes, 23,675,072 bytes of heap after publication; banner, navigation, advertisements and three-column news layout visible |
| Arch packages | 2,959 DOM nodes, 9,938,992 bytes after publication; aligned, striped package rows and filters above the table |
| ArchWiki | 1,142 DOM nodes, 43,846,152 bytes after publication; logo, toolbar and article columns visible |
| AUR | 710 DOM nodes, 6,189,904 bytes after publication; sidebar, update dates and stats alongside the introduction |
| Google | 106 DOM nodes, centered search form and upper-right navigation |

The earlier unshared native run used about 71 MB for packages and 113 MB for
the Wiki; its DistroWatch attempt did not finish within five minutes. Partial
attempts remain in the evidence directory and are not presented as successful
screenshots. The final DistroWatch diagnostic records 19.47 seconds for styles
and 11.69 seconds for layout. These are single-run diagnostic timings with
native allocation and resource work included, not a general performance
benchmark. `release-measurements.json`, serial logs and `release-hashes.txt`
identify the measured run and the embedded/staged native binaries.

The same guest resizes Google from 1056×642 to 754×642 (accepted viewport
version 19), loads the full DistroWatch capture at that narrow size, and
restores it to 1056×642 (accepted generation 6, version 33).
`google-release-narrow.png`, `distrowatch-release-narrow.png` and
`distrowatch-release-restored.png` preserve the results. DistroWatch's fixed
header/ad content can exceed the narrow viewport; its lower table columns and
menu reflow rather than retaining stale wide geometry. Full horizontal scrolling
is still outside this change.

The validated ISO is also staged at `target/sunlightos.iso`; the prior default
is preserved as `target/rabbit-classic-evidence/previous-default.iso`.
`staged-iso.json` verifies byte identity with `release.iso`. The guest and local
capture servers are stopped after validation.

## Remaining limits

These captures demonstrate structural improvements, not pixel equivalence.
Font sizes still use the installed finite face set; select-multiple/list boxes,
some icon masks and heading counters remain approximate. Dense initial style
construction is slow and uses substantial native heap. Initial image requests are limited to the existing 32-entry cache capacity;
later images remain placeholders until a future incremental loading policy.
The decoded-byte cap can still limit large images.

The table subset does not implement column/colgroup sizing, captions, complete
collapsed-border conflict resolution, header/footer reordering or all malformed
table repairs. Fixed/absolute placement lacks complete static-position,
relative-offset, stacking and scroll-anchoring semantics. Overflow clips reuse
the shared canvas's existing text/image clipping approximations; ellipsis
glyphs are not synthesized. Columns balance whole child boxes without inline
fragmentation. Flex shrink, wrapping and the full automatic minimum-size
algorithm remain incomplete. Unknown media features retain the existing
observable approximation, and full feature-query evaluation is still absent.
