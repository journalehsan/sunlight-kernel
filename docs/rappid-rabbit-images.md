# Rabbit shared web images

## Diagnosis

The portrait fetched from `https://stallman.org/Richard_Stallman_at_LibrePlanet_2019.jpg`
is a 2000×2588 progressive JPEG, about 1.45 MiB compressed. Before this change,
Rabbit's decoder returned `image pixel buffer exceeds limit`: its 20,704,000
decoded bytes exceed the 8 MiB browser limit. The 12 MiB cache would also discard
the decoded surface. The misleading `decode_tga` helper already dispatched JPEG
to the shared decoder; the renderer was not the missing JPEG implementation.
The earlier screenshot alone did not prove that any particular fetch finished.

## Shared interfaces and ownership

`sun-img` supplies portable zune JPEG/PNG decoding with std and platform SIMD
disabled. Native support includes baseline/progressive JPEG, grayscale and
CMYK JPEG, static PNG including palette, transparency, 16-bit and Adam7, TGA
type 2 and SIMG v2. BMP recognition is not a BMP decoder. GIF, WebP and APNG
are unsupported. Rabbit now uses the shared decoder for PNG as well as JPEG
and TGA. JPEG can produce ARGB directly, releasing progressive coefficient
storage before conversion and avoiding an intermediate full-image RGBA copy.

Discovery resolves image URLs relative to the final page URL. A single bounded
blocking worker fetches and decodes images, then builds the two-pass layout and
`DocumentScene`. `ImageCache` and scene objects share `Arc<RasterImage>` with
straight ARGB pixels. An owned `NavigationOutput` crosses the one-shot channel;
the UI accepts it only for its current document generation. Navigation drops
the previous DOM/cache; a dropped receiver drops an undeliverable result.
This change preserves the existing one-navigation-at-a-time policy: initial
publication waits for subresource processing. It does not add concurrent image
jobs or blocking image work to UI callbacks.

`DocumentRenderState::complete_images` rejects another document's generation,
ignores identical completions, and otherwise reflows once and diffs stable
scene IDs. Intrinsic dimensions feed both layout passes; HTML/CSS dimensions
and auto-axis aspect ratio continue to control the layout box. Pixel storage
is shared across nodes, scene diffs, reflows and UI transfer.

The Document Viewer uses the same `RasterImage`, retained `DocumentScene` and
`DocumentCanvas` (the shared document widget) path. Its Hayro worker renders PDF
pages into bitmaps, which the viewer converts to ARGB. Hayro is not used to
parse SVG image input.

## Static SVG policy

`sunlight-ui` already provided usvg/resvg SVG-input rendering for native icons.
The new `image::decode_document_svg` uses those same dependencies and ARGB
representation with an untrusted-document policy. Rabbit's default native
build enables it through `sunlight-ui/svg`; its separate optional renderer and
fixed 256×256 distortion are removed. SVGs rasterize once at natural dimensions,
including viewBox scaling and transparency. Document painting then scales to
the normal HTML/CSS box. Large CSS upscales can therefore appear soft.

Supported static shapes, paths, transforms, gradients and clipping use resvg.
Text/fonts, nested image resources (including data URLs), use expansion,
markers, filters, patterns, masks, scripts, animation and foreignObject are
rejected. DTDs/entities and SVGZ are rejected. Both image resolvers are disabled;
no SVG can read a local file or initiate a network fetch. No system fonts are
loaded. CSS background images remain an explicitly unsupported paint feature
of the current two-pass renderer; discovery alone does not paint them.

## Budgets and diagnostics

- Response collection stops at 2 MiB, counting chunk framing. Content-Length
  is checked before allocating; chunked completion does not wait for EOF.
- Raster dimensions are checked before decode: at most 4096 per axis and
  24 MiB of decoded pixels. The cache holds at most 32 entries / 28 MiB.
- The native worker checks available process heap against a conservative
  12 bytes/pixel plus 4 MiB working-set allowance before raster decoding.
  The application still has its existing 128 MiB process heap.
- SVG source is at most 64 KiB, XML at most 2048 nodes and depth 32, output at
  most 1024 per axis / 262144 pixels. Converted nodes and isolation depth are
  bounded, as are offscreen group extents. Resource failures remain placeholders.

Image console messages also reach native serial output, with controls stripped
and length bounded. Distinct response/HTTP, fetch, unsupported-format, decode,
size-validation and cache-attachment failures include URL, document generation,
format and dimensions where known. Scene logs identify attached images versus
placeholders. The shared canvas reports first visible image paint with actual
written-pixel counts; decoded or attached alone is not treated as proof of paint.

## Deterministic verification

`rappid-rabbit/tests/web_images.rs` tests late JPEG placeholder-to-pixel paint,
shared Arc ownership, stale and repeated completions, intrinsic/HTML/CSS sizing,
SVG viewBox/alpha paint, malformed input and resource rejection. A synthetic
2000×2588 progressive JPEG catches both portrait-budget regressions. It was
generated with Pillow from solid RGB (170,70,30), quality 60, progressive mode;
tests only use the committed bytes. Navigation tests reject and release stale
image pixels after loading the same URL again. Shared tests cover direct ARGB
conversion equivalence and safe SVG input; fetch tests cover length, chunking,
truncation and response limits.

Validation commands:

```sh
cargo test -p rappid-rabbit --target x86_64-unknown-linux-gnu
cargo test -p sun-img -p sunlight-ui --features sunlight-ui/svg --lib --tests --target x86_64-unknown-linux-gnu
cargo test -p sunlight-fetch --lib --target x86_64-unknown-linux-gnu
bash tools/build_rappid_rabbit.sh
cargo build --package sunlight-kernel
```

The full shared test invocation (including doctests) additionally exposes an
existing failing doctest at `sunlight-ui/horizon.rs:5`: its indented control
diagram is interpreted as Rust. Unit/integration tests pass independently.

## Observed guest results

The rebuilt ISO was booted in QEMU TCG with 2 vCPUs and 3 GiB RAM, using the
normal root login and desktop. Artifacts are in
`target/rabbit-image-evidence/` (generated, not committed).

- `http://stallman.org/`: HTTP 200; the 1,523,651-byte portrait decoded at
  2000×2588, attached at 320×400 and visibly painted. The first clipped paint
  wrote 96,000 pixels. `stallman.png` shows the portrait with console;
  `stallman-full.png` shows the full portrait after hiding the panel.
  Reloading after Arch and example.com produced generation 4 and painted all
  128,000 pixels of the 320×400 portrait (`stallman-reload.png`).
- `https://archlinux.org/`: HTTP 200; `rss.svg` decoded at 8×8, attached twice,
  and the visible RSS icon painted 64 pixels (`arch.png`). The 600×126
  `archlogo.svg` also decoded, but its CSS background use remains unpainted.
- `http://example.com/`: HTTP 200, 577-byte live page; the centered paragraph
  and wrapping render (`example.png`). The current page has no visible heading.
  A stray `s.js>` text artifact from the existing HTML parser remains visible;
  a direct Golden Fish host probe also produces it as a body text node
  (`example-parser-probe.log`). This image change does not modify that parser
  or text layout.

Stallman's `no-facebook.svg` was rejected under the restricted SVG policy;
the page's other fetched JPEGs and PNGs decoded. Dense-page initial layout and
viewport reflow are slow under TCG; the existing UI-side viewport reflow remains
synchronous. Fetch/decode stayed on the worker, and no OOM or panic was observed.

`guest.serial.log` contains fetch/decode/scene/paint evidence;
`hashes.txt` identifies the tested ISO, embedded kernel and native Rabbit.
Host verification passed 179 Rabbit tests, 309 shared image/UI tests,
31 fetch tests and 11 Document Viewer library tests. The native wrapper,
kernel embedding build and formatting checks passed. No dependencies were
added; Rabbit's direct miniz/resvg dependencies were removed in favor of the
shared pipeline.
