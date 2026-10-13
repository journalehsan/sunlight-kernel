# Hayro 0.8.0 PDF backend investigation

> Current policy: Hayro is the preferred host-only PDF backend. MuPDF is disabled
> by default and retained for explicit rollback. Permanent removal requires the
> full Hayro renderer to build and render a PDF natively on SunlightOS. See
> [current backend status](document-viewer.md#backend-deprecation-status).

## Decision (2026-10-12)

**Host rendering works; a native SunlightOS replacement is blocked.** Hayro
0.8.0 is pure Rust, but its full rendering stack requires `std`. SunlightOS's
native userspace target is `x86_64-unknown-none`, with `no_std + alloc` libraries,
not a SunlightOS port of Rust `std`. Turning off Hayro's default features does
not remove its unconditional `std` dependencies. No compatibility shims, Linux
dependencies, C runtime, production backend selection, or UI rewrites were
added. The existing MuPDF implementation is intact.

The isolated experiment is in `tools/hayro-prototype/`. It has its own workspace
and lockfile, pins Hayro to `=0.8.0`, and uses the existing `sunlight-ui` document
canvas on the host. It is **not a native executable or a production-safe
untrusted-document service**. In particular, internal engine OOM cannot be
converted into an ordinary request error.

## Existing integration and failure evidence

The relevant components are:

| Component | Responsibility |
| --- | --- |
| `sunlight-docserv/src/backend.rs` | Format-neutral `DocumentBackend` / `DocumentHandle`, count, point dimensions, BGRA8 render, text; Drop releases handles |
| `backends/mupdf_backend.rs` | MuPDF 0.4.4; immutable source snapshot, page validation, RGB-to-BGRA conversion |
| `backends/ghostscript_backend.rs` | PS/EPS conversion to a temporary PDF followed by MuPDF; native spawn, timeout and cleanup |
| `service.rs`, `client.rs`, `ipc.rs` | Owner-scoped document registry, serialized requests/replies and capability shared mappings |
| `sunlight-docview/src/viewer.rs` | Navigation and up to five cached rendered pages |
| `sunlight-docview/src/ui.rs` | Worker thread, coalesced render intent, BGRA-to-ARGB conversion and retained image scene |
| `sunlight-ui/widgets/document_canvas.rs` | Existing `RasterImage { pixels: Vec<u32> }`, ARGB8888 canvas painting |
| `tools/build_docserv.sh` | Static musl renderer **and viewer**, external C/C++/ar toolchain, Helios ELF stamp |
| `tools/test_docserv.sh`, `docserv_boot_gate.py` | Optional QEMU IPC/window/thumbnail gates |

The Helios bridge is not just a PDF rendering adapter. Both binaries link Rust's
Linux musl `std`, and `tools/stamp_helios_elf.sh` sets Linux OSABI and the `HLNX01`
marker. Native capability IPC is available to these processes through the
tagged syscall namespace. Replacing MuPDF alone would not remove the viewer's
`std::thread` / channels / filesystem dependency or the musl build target.
Kernel embedding uses `document_viewer` / `document_postscript`, and supervisor
packaging uses `document-service`. Normal native services instead use
`services/user-space.ld`, static relocation, no red zone and x86-64-v2.

### Reproduced failure

Running the documented host command on the current working tree:

```sh
cargo test -p sunlight-docserv -p sunlight-docview --target x86_64-unknown-linux-gnu --locked
```

fails before tests run, with **63 MuPDF compilation errors**. Representative
errors are `E0609: no field w on type mupdf_sys::fz_image` (only `_address` is
available) and missing `fz_stext_block` / `fz_stext_line` fields.

Cargo also reports that the `vendor/bindgen-compat` patch is unused. The facade
currently declares `version = "0.2.4671-alpha.2"`, but the MuPDF build requires
bindgen 0.69. The root manifest documents that this facade exists to use bindgen
0.72.1 behind the older API for Clang 22. The current version mismatch prevents
that facade from being selected. `tools/version_manager.py` enumerates manifests
recursively, excluding target/hidden directories but not vendor/, consistent
with the facade having been changed by repository-wide versioning. This is a
concrete build failure; the exact historical edit that changed the version was
not independently established. No unrelated versioning repair was made here.

An existing, previously built host `target/x86_64-unknown-linux-gnu/release/
sunlight-docserv --check assets/documents/SunlightOS_PDF_Reader_Test.pdf`
successfully opens and closes the supplied sample (`[DOCSERV] opened 1`). That
command tests opening, not rendering, and the binary predates this rebuild.

### Logs and remaining runtime uncertainty

`target/docserv-serial.log` contains `[DOCSERV] ready`, but no successful
`native IPC open/render/text/close OK` or native viewer gate marker. The examined
`/tmp/docview-current-serial.log` has no DOCSERV/DOCVIEW marker. Earlier musl build
logs end in successful builds. These artifacts do not establish a MuPDF crash
or explain the user's unresolved rendering bugs. No current MuPDF native gate
was claimed: the host rebuild is blocked, and the existing artifacts/ISO are
not evidence for a freshly tested native renderer.

### Code-level failure/pressure points (not proven crashes)

* Each MuPDF page operation reconstructs a document from the snapshot to avoid
  unsafe `Send`/`Sync` declarations on raw pointers. Count, dimensions, render
  and text can repeatedly parse the same document.
* Source size is checked with metadata before `std::fs::read`; a growing file
  can exceed the advertised snapshot budget. The prototype uses a bounded read.
* A render allocates an RGB MuPDF pixmap and a BGRA buffer. Reply serialization,
  shared-memory transfer, client snapshot/deserialization, cache and ARGB UI
  conversion add more large buffers. Five cached pages compound this pressure.
* Source/16-megapixel/128-MiB serialized-reply limits constrain selected stages,
  not the full engine working set. `vec!`, collections, serialization and the
  UI conversion use infallible allocation. OOM is not an ordinary service error.
* Helios must supply Linux `std` services as well as native IPC; successful native
  IPC alone does not prove the Linux runtime calls used by rendering and the
  viewer worker are correct. These are investigation targets, not identified
  syscall failures in the inspected logs.

## Dependency and portability audit

Sources audited were the downloaded, published 0.8.0 manifests/source and the
prototype's resolved lockfile/`cargo tree`, rather than assuming current GitHub
main matches the release. Upstream references:
[Hayro 0.8.0](https://docs.rs/hayro/0.8.0/hayro/),
[published manifest](https://docs.rs/crate/hayro/0.8.0/source/Cargo.toml),
[interpreter manifest](https://docs.rs/crate/hayro-interpret/0.8.0/source/Cargo.toml),
[syntax manifest](https://docs.rs/crate/hayro-syntax/0.8.0/source/Cargo.toml).

Hayro declares edition 2024, Rust >=1.92, and MIT/Apache-2.0. The installed
`rustc 1.98.0-nightly (8954863c8 2026-06-05)` satisfies the version requirement.
The wrapper uses repository Rust 2021 style.

| Dependency group (resolved versions) | Portability finding |
| --- | --- |
| `hayro` 0.8.0 | Unconditional `std::cell`, `std::rc`, `std::ops`; no `no_std` switch |
| `hayro-interpret` 0.8.0 | Unconditional `std` caches (`Arc`, `Mutex`); explicitly enables syntax `std`, `images`, `unsafe` |
| `hayro-syntax` 0.8.0 | Has a real `no_std` mode, but interpreter forces `std`; selected mode uses `Mutex`, `RwLock`, `OnceLock` and shared ownership |
| `vello_cpu` / `vello_common` 0.3.0 | Hayro forces `std`, `png`, `u8_pipeline`; upstream libm configuration cannot be selected through Hayro's public features; no Rayon/multithreading feature in this graph |
| `fearless_simd` 1.1.0 and 0.7.0 | Separate versions in Hayro and Vello; runtime SIMD dispatch remains even with Hayro `simd` off |
| `pic-scale` 0.7.12, `moxcms` 0.8.1, `pxfm` 0.1.30 | Pure Rust image scaling/color management; runtime/platform and allocation work still needs a native port audit |
| `hayro-cmap` 0.1.0, `hayro-postscript` 0.1.0, `brotli` 8.0.4 / decompressor 5.0.3 | Embedded CMaps enable `std` and Brotli `std`; PostScript here parses PDF font/CMap data, not a replacement for Ghostscript PS document support |
| `hayro-ccitt` 0.4.0, `hayro-jbig2` 0.3.1, `hayro-jpeg2000` 0.4.1, `zune-jpeg` 0.5.15 / core 0.5.3 | Rust PDF image codecs; syntax's selected features also enable codec SIMD/runtime dispatch |
| `flate2` 1.1.10, `zlib-rs` 0.6.8, miniz/adler/crc | Rust compression backends; no system zlib/C dependency, but selected configurations require `std` |
| `image` 0.25.10, `png` 0.18.1 | PNG support is selected unconditionally upstream even though the prototype writes PPM; introduces `std` I/O/compression features |
| `skrifa` 0.46.2, `read-fonts` 0.43.3, `font-types` 0.12.6 | Rust font parsing/outlines; embedded standard fonts permit this English sample without OS font lookup |
| `kurbo` 0.13.1, `polycool`, `peniko`, `color`, `guillotiere`, `euclid` | Rust geometry/color/atlas code; features/math must be revisited for a native build |
| hashbrown/foldhash/rustc-hash, smallvec/arrayvec, bytemuck, yoke/zerofrom, once_cell, bitflags, siphasher, memchr, resource handles, thiserror | Rust storage, caching and utilities; some are `no_std` capable, but feature unification currently enables `std` in several |
| proc-macro2/quote/syn, derive macros, phf generator/fastrand, autocfg | Host build-time Rust tools, not SunlightOS runtime APIs |

The resolved rendering graph has no `cc`, `cmake`, bindgen, MuPDF, `libc`,
fontconfig, GPU framework or Linux syscall crate. Thus no new C/C++ compiler
or library is required for the host experiment. This does **not** imply a
freestanding executable: a Linux host build still links the host Rust `std`.
The existing canvas pulls its existing Rust image dependencies through
`sun-img`; these are not a newly introduced UI stack.

The prototype disables Hayro's optional scaling `simd` feature and enables
embedded fonts/CMaps. This does **not** disable all SIMD. SunlightOS CPU setup
currently enables x87/SSE and comments in its build recipe exclude AVX until
XSAVE/YMM switching exists. Both fearless_simd versions, JPEG decoders and
color management require explicit dispatch review before any native execution.
Host timings below use host runtime feature detection and are not x86-64-v2-only
SunlightOS predictions.

## Native feasibility and exact blockers

```sh
cargo check --manifest-path tools/hayro-prototype/Cargo.toml \
  --target x86_64-unknown-none --lib --locked --offline
```

fails with `E0463: can't find crate for std`, starting in `simd_adler32`,
`zlib_rs`, `zune_core` and `linebender_resource_handle` (full output:
`target/hayro-native-check.log`). Hayro/interpreter and their forced renderer
features require `std` independently of these first errors. Adding `#![no_std]`
to the wrapper or installing a target does not resolve this graph.

Native facilities already available are `sunlight-libc` VFS open/read/close,
its reclaiming global allocator and CRT, capability IPC/shared memory, threads,
and the `no_std` retained canvas. They do not constitute implementations of
`std::sync`, Rust's platform runtime, PNG/compression I/O APIs, or a supported
native Rust `std` target. The allocator's native dynamic heap presets are 1,
8 and 16 MiB; even this sample's host peak is significant within that budget.
Changing it without auditing engine allocations would not make OOM recoverable.

The existing backend trait is itself gated behind docserv's `std` feature and
uses `std::path::Path`. Its native port would need an alloc-only file/snapshot
boundary as well. The wire protocol already has an alloc-only UTF-8 path form,
so that part can be reused without replacing the IPC protocol.

`-Zbuild-std` is not a completed SunlightOS platform port. Switching to a Linux
musl target would keep Helios. Either a coordinated upstream `no_std + alloc`
port of the interpreter/rasterizer dependency graph, or a deliberate native
Rust `std` platform project, is required. Both exceed this prototype's scope.
No broad compatibility shims were attempted.

## Prototype behavior and safety boundary

The engine interface accepts owned bytes, queries page count and rotated/cropped
point dimensions, and returns an owned `PageBitmap` with width/height/stride and
borrowed RGBA8 access. Drop closes the document and releases parser caches.
The file driver bounds snapshots to 32 MiB. Scale, finite dimensions, u16
renderer bounds and a 16-megapixel output limit are checked before rendering.
RenderCache is local to each render to avoid self-referential ownership and
unbounded retained outline caches. Reusing that cache is a future optimization.

Loading takes the source Vec by ownership; Hayro shares it internally without
copying the PDF. Returning the pixmap moves its ownership without a raster copy.
At the canvas boundary, a checked allocation converts RGBA to the existing
ARGB `Vec<u32>` once. The source bitmap is dropped immediately afterward.
The scene holds an `Arc<RasterImage>`; scene use does not clone pixels.
Avoiding that final conversion would require a buffer-format/ownership change
to upstream Vello or the existing UI, which was intentionally left out.

Common parse errors and invalid requests return errors. On the host, unwind
panics are caught and the caller is told to discard the document; the CLI exits
on that error. This does not catch OOM aborts, stack overflow, abort-mode panics,
or enforce decompressed-stream/image/font working-set limits. Upstream Pixmap
uses infallible `vec!` allocation and Hayro's render API does not return a
fallible render result. The wrapper's checked pixel/source limits and
`try_reserve_exact` at its own UI boundary do not repair that contract. A
separate disposable process is the appropriate boundary for untrusted host
experiments; the current command has no hard CPU/RSS limiter.

This prototype does not implement text extraction, links/bookmarks, passwords,
other document formats, or the production service trait's text operation.
It paints through the actual canvas on the host; it does not create a native
compositor window or claim a QEMU validation.

## Validation and measured results

Input: `assets/documents/SunlightOS_PDF_Reader_Test.pdf`, 115,543 bytes,
SHA-256 `9f0e3ad175e847529057fe01bbdf141068ac74b6b42817dd3a5a08e00186afd2`.
Poppler `pdfinfo` reports four A4 pages, 595.276 × 841.890 points, rotation 0,
unencrypted PDF 1.4. `pdftotext` confirms the English typography/table/graphics/
navigation test content, including accented Latin characters.

Three host integration tests pass: sample count/dimensions/storage and repeated
rendering; invalid scales/pages/malformed documents; and an original generated
90° fixture checking both swapped dimensions and the position of a red vector
rectangle. The sample test also checks substantial red-curve, yellow-vector
and cyan-image pixel populations on page 3. All four pages were rendered at
75%, 100%, 150% and 200%; full renders and canvas outputs were inspected.
Text, bold/size variation, accented characters, table rows/grid/alignment,
circle/rounded rectangle/Bezier curve and RGB image are visible and consistent
with the reference. The rasterized link labels appear; link activation is not
part of raster validation.

At 100%, compared with Poppler `pdftoppm -r 72 -png`, mean absolute channel
differences are 0.935, 0.970, 0.845 and 0.722 on a 0–255 scale for pages 1–4.
The reference is cropped from 596×842 to Hayro's 595×841 for comparison;
Hayro truncates scaled dimensions in its convenience API, while the existing
MuPDF backend rounds up. These averages and visual inspection are useful smoke
evidence, not pixel-exact equivalence or a general PDF conformance test.
Canvas artifacts include viewport padding/clipping, as expected for a viewport
the same size as its page; the full-page raster artifacts retain the footer.

Measured on a Linux x86_64 Intel i7-1260P, release build, single rendering thread,
local cache rebuilt for each invocation:

| Scale | Output dimensions | RGBA bytes/page | Observed render time range |
| --- | --- | --- | --- |
| 0.75 | 446 × 631 | 1,125,704 | 1.17–2.51 ms |
| 1.0 | 595 × 841 | 2,001,580 | 1.32–2.86 ms |
| 1.5 | 892 × 1262 | 4,502,816 | 1.79–4.06 ms |
| 2.0 | 1190 × 1683 | 8,011,080 | 2.60–3.50 ms |

Open took 0.057 ms. Peak requested live heap was 10,360,492 bytes (9.88 MiB),
measured by a counting wrapper over Rust's System allocator. This includes the
driver/UI allocations and excludes allocator overhead, stacks and mappings.
Linux process peak RSS was 19,604 KiB, measured using Python
`resource.getrusage(RUSAGE_CHILDREN)` in a fresh measurement process. Timings
exclude canvas conversion/painting and output writing; they are one diagnostic
run, not a statistically controlled benchmark.

After initial four-page rendering at four scales, **21 additional complete
open/render-all-four/close cycles** returned to exactly **1,839 live requested
bytes** after every close. The CLI asserts this stable baseline. No crash or
growing tracked allocation was observed. This is evidence against leaks in
these exercised lifecycles, not proof of leak freedom; no Valgrind or sanitizer
run was available. Large/hostile documents, other image codecs, encryption,
complex transparency, CJK and native runtime behavior remain unvalidated.

Generated inspection artifacts remain under `target/hayro-output/` (four
`page-*.ppm`, four `canvas-*.ppm`; locally generated contact/canvas PNGs),
with `target/hayro-metrics.log`, `target/hayro-time.log`, and
`target/hayro-native-check.log`. They are not committed source assets.

## Reproduction

```sh
cargo fetch --manifest-path tools/hayro-prototype/Cargo.toml --target x86_64-unknown-linux-gnu
cargo test --manifest-path tools/hayro-prototype/Cargo.toml --target x86_64-unknown-linux-gnu --release --locked
cargo clippy --manifest-path tools/hayro-prototype/Cargo.toml --target x86_64-unknown-linux-gnu --all-targets --locked -- -D warnings
cargo run --manifest-path tools/hayro-prototype/Cargo.toml --target x86_64-unknown-linux-gnu --release --locked -- assets/documents/SunlightOS_PDF_Reader_Test.pdf target/hayro-output
pdftoppm -r 72 -png assets/documents/SunlightOS_PDF_Reader_Test.pdf target/hayro-reference
```

The prototype prints open/render times, output sizes, peak requested heap and
post-close live bytes. Peak RSS can be measured by running the built executable
as the sole child of a fresh Python process and reading
`resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss` (KiB on Linux), or with
GNU time if installed. Never use `tools/build.sh` just to test this experiment:
it also changes repository versions and rebuilds unrelated services.

## Recommended next steps

1. Retain MuPDF for production. Separately restore the bindgen facade's compatible
   version and exclude vendored compatibility manifests from version rewriting;
   then rerun host and native gates on the supplied four-page sample and collect
   an actual runtime failure trace.
2. Discuss a targeted upstream Hayro `no_std + alloc` port: interpreter caches,
   rasterizer features, optional PNG, embedded CMap decompression, math and SIMD
   dispatch. The parser already provides part of this path; rendering does not.
   Do not disguise a Linux target as native execution.
3. Require fallible/budgeted decompression and raster allocation, CPU cancellation
   and document-process containment before enabling untrusted PDFs on a bounded
   native heap. Output pixel limits alone are insufficient.
4. Once that port builds, introduce an opt-in adapter for the existing backend
   contract and preserve MuPDF for other formats/text until parity is tested.
   Test native VFS snapshots, repeated render/close, shared-memory ownership,
   canvas display and CPU context/SIMD correctness under QEMU.
5. Expand the PDF corpus and benchmark native peak memory/latency before changing
   the default. Review cache reuse and render-to-caller storage to reduce peak
   copies, while preserving the current viewer and scene infrastructure.
