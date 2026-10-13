# PDF and PostScript viewer

## Backend deprecation status

Hayro 0.8.0 is the preferred PDF renderer. `hayro-host` is the default Linux GNU
verification feature; `hayro-native` renders using SunlightOS native Rust std.
`bash tools/build_docserv.sh` builds the native Hayro service and native std
UI/client. Both use native ELFs. The existing Helios viewer remains available
with `bash tools/build_docview.sh` and can call the same native renderer.
The [native std implementation and gates](native-std-foundation.md) describe
the platform subset, lifecycle and validation boundaries.

Native service/std gates pass on two and four CPUs. The native std viewer passes
six-page navigation, zoom, resize, cache recovery, Files thumbnails and close
against this service; its production UEFI Run/Open Document dialog gate also
passes on twelve CPUs. The default 14 host service/viewer tests pass.

The existing PDF Reader UI, Document Canvas, IPC schema and BGRA8 bitmap contract
are unchanged. Hayro supports PDF page count and rasterization; text extraction
returns an explicit unsupported error. XPS/CBZ/EPUB and PS/EPS conversion are
available only with the legacy feature. Selecting both backends prefers Hayro
for PDFs; errors do not silently fall back to MuPDF.

MuPDF and Ghostscript source are retained for reference and rollback, disabled
by default. Their C/C++ toolchain, libclang and bindgen compatibility patch are
only needed for an explicit legacy build. **MuPDF remains available as an explicit rollback during native std rollout.** Unrelated Helios functionality remains unchanged.

```sh
# Preferred host backend and existing PDF Reader UI tests
cargo test -p sunlight-docserv -p sunlight-docview --target x86_64-unknown-linux-gnu
cargo clippy -p sunlight-docserv -p sunlight-docview --target x86_64-unknown-linux-gnu -- -D warnings
# Native alloc-only client/viewer libraries (no rendering engine)
cargo check -p sunlight-docserv -p sunlight-docview --no-default-features --lib
# Legacy host rollback, including text extraction and Ghostscript tests
cargo --config tools/docserv-mupdf-legacy.toml test -p sunlight-docserv \
  --no-default-features --features mupdf-legacy --target x86_64-unknown-linux-gnu
# Legacy guest rollback; musl toolchain still required
DOCSERV_BACKEND=mupdf-legacy bash tools/build_docserv.sh
```

Historical validation for the deprecation preparation: the 13 host service/viewer tests,
five legacy MuPDF/Ghostscript tests, five combined-backend preference tests,
three host Hayro prototype tests, and three parser-probe host tests pass. Native
alloc-only docserv/viewer libraries check successfully, the default kernel
builds, and the Phase 3.0 and Helios Note regression boot gates pass. The
host-only guards reject musl builds and use of `hayro-host` for native startup.
Docserv's focused clippy check passes; the broader strict clippy command remains
blocked by existing diagnostics in `sunlight-libc` and `sunlight-audio`.
Evidence logs are local build artifacts under `target/pdf-deprecation/`.

## Default viewer launch

The default kernel packages the existing `sunlight-docview` UI/client executable
independently of the optional rendering service. `tools/build_docview.sh` builds
only that executable with Rust's bundled musl tools. Its existing std-based UI
uses Helios for execution; no Hayro or MuPDF rendering engine is linked into it.
The PDF Reader UI and shared Document Canvas are unchanged.

Previously the desktop advertised the viewer but the default kernel excluded
its embedded ELF mapping. Launch then read the RAMFS script placeholder and
failed with `UnsupportedPersonality`. The viewer now has an unconditional ELF
mapping; the build validates its Helios stamp before embedding it. MuPDF and
Ghostscript remain opt-in. Selecting a document requires an available `docserv`.
The default build now embeds both the native std viewer and Hayro service, so a
plain `bash tools/build.sh` or `./tools/runs.sh --build` renders documents with
no MuPDF compiler dependency. An image built without the service leaves the
viewer fully functional except that every open ends in
`[DOCVIEW] render failed: docserv is unavailable`; set
`SUNLIGHT_DOCUMENT_VIEWER=0` only to build that smaller image deliberately.
The `native_document_viewer` kernel feature selects that viewer ELF; the
unconditional default mapping continues to use the existing Helios executable.
Both viewer workers use the same IPC protocol. Backend failure now stops the rendering placeholder
and logs the error to serial.

```sh
bash tools/build_docview.sh
bash tools/test_docview_launch.sh
```

The launch gate boots an isolated UEFI VM, authenticates, launches the actual
viewer through the desktop Run dialog and shared `sun-exec` path, verifies window creation and the Open Document request,
and captures `target/docview-launch-fix/viewer-open-dialog.ppm`. It does not use
the user's VM disk or enable a PDF renderer.

Verified: UEFI/OVMF with 12 vCPUs and 4 GiB RAM successfully spawns the viewer,
creates its window, and displays the Open Document dialog with no renderer
registered. The 13 host service/viewer tests, native alloc-only library
checks, and normal Phase 3.0 boot gate also pass. The normal
`target/sunlightos.iso` has been rebuilt without test login injection.

The remaining sections describe the retained **legacy** runtime and its gates.
All guest build commands below require `DOCSERV_BACKEND=mupdf-legacy` in the
environment. `SUNLIGHT_DOCUMENT_VIEWER=1` selects native Hayro unless
`DOCSERV_BACKEND=mupdf-legacy` is explicitly supplied.


`sunlight-docserv` registers the native `docserv` endpoint and owns document
handles. `sunlight-docview` uses the existing retained document canvas and its
ARGB8888 `RasterImage` API. The wire bitmap format is top-down BGRA8. Navigation
retains at most the current page and two pages on either side; zoom and resize
replace the old-scale cache. Rendering, prefetch, and Close run on a worker,
while the UI coalesces rapid navigation into the latest requested page.

The legacy MuPDF 0.4 backend is a std/C userspace engine; it cannot be built for the workspace's
default bare-metal target. The renderer and viewer runtime use static musl
ELFs through Helios. The protocol, client, thumbnail helper, and viewer state
support `no_std + alloc`. Native apps such as Files remain freestanding. A
tagged native syscall namespace lets the musl processes use capability IPC,
shared memory, logging, and native spawn without Linux syscall collisions.
The kernel verifies shared mapping capacities before either side copies data.

MuPDF supports PDF, XPS, CBZ, and EPUB here. PS/EPS use Ghostscript conversion;
the native build uses SunlightOS spawn instead of Linux fork. Conversion has a
30-second timeout and removes temporary PDFs after snapshotting their contents.
Source snapshots are limited to 32 MiB, individual renders to 16 megapixels,
and outstanding serialized replies to a combined 128 MiB.
The native service unit uses the existing root service identity. Handle access
is scoped to the authenticated sender badge. The file manager dispatches PDF,
PS, and EPS thumbnails exclusively on its existing worker; terminated client
handles and abandoned response buffers are reaped on the next incoming RPC.
The fixed-size
128-pixel cache uses path and source-content hashes because native stat does
not currently expose mtime. The daemon also supports these formats and uses a
reclaiming allocator for repeated requests.

## Build

Host verification explicitly selects the host target:

```sh
cargo test -p sunlight-docserv -p sunlight-docview --target x86_64-unknown-linux-gnu
cargo check -p sunlight-docserv -p sunlight-docview --no-default-features --lib
cargo clippy -p sunlight-docserv -p sunlight-docview --target x86_64-unknown-linux-gnu -- -D warnings
cargo build --release -p sunlight-docserv -p sunlight-docview --target x86_64-unknown-linux-gnu
```

The host PostScript test requires `gs`. MuPDF builds its bundled C libraries
and requires a C compiler and libclang. `vendor/bindgen-compat` preserves MuPDF
0.4's bindgen API while using bindgen 0.72.1 to support Clang 22. MuPDF remains
at the requested `0.4` dependency range, resolved to 0.4.4 in Cargo.lock.

For native binaries, supply a musl C/C++ compiler toolchain:

```sh
export DOCSERV_CC=/path/to/x86_64-linux-musl-gcc
export DOCSERV_CXX=/path/to/x86_64-linux-musl-g++
export DOCSERV_AR=/path/to/x86_64-linux-musl-ar
bash tools/build_docserv.sh
```

Build the static Ghostscript executable from the extracted upstream source:

```sh
export DOCSERV_GS_SOURCE=/path/to/ghostscript-10.08.0
bash tools/build_docserv_ghostscript.sh
```

The tested [Ghostscript 10.08.0 release](https://ghostscript.com/releases/index.html)
source archive is available from the
[upstream release](https://github.com/ArtifexSoftware/ghostpdl-downloads/releases/tag/gs10080).
Its SHA-256 is
`caf199e3f233f1290b27d0972d636f66c303355f2353309b7bfddf1edda06b3d`.
MuPDF and Ghostscript carry AGPL licenses. Downloads, compiler toolchains,
converted files, ELFs, and test images belong outside source control.

By default `bash tools/build.sh` and `./tools/runs.sh --build` build and embed
the native Hayro service and viewer, which need no musl C/C++ toolchain. To omit
them for a smaller image, set `SUNLIGHT_DOCUMENT_VIEWER=0`; an image built
without `docserv` launches the viewer but every open reports `docserv is
unavailable`. For manual builds, enable kernel `native_document_viewer` and
supervisor `document-service` explicitly, and build the native service first
with `bash tools/build_docserv.sh`; the kernel checks the artifacts and tracks
them as build inputs. PostScript support is optional: supply `DOCSERV_GS_SOURCE`
or an already built `target/docserv-gs` to include it (requires
`DOCSERV_BACKEND=mupdf-legacy`). For a minimal build without documents, set
`SUNLIGHT_DOCUMENT_VIEWER=0`, or build both the kernel and `sunlightd` with
`--no-default-features`.
The opener registers document extensions and the `docview`/`document-viewer`
aliases. Launch a file through `sun-open` or run `sunlight-docview path.pdf`
inside an active desktop session. Document Viewer is also listed under
Productivity in the Start Menu, and both shell searches accept `pdf` and
`docview`. Launching it without a document opens the shared Open Document
dialog; cancelling closes the viewer. Files uses the central PDF association
and displays Document Viewer in the Properties dialog's Opens with field.

## Verification

The CC0 fixtures are original six-page PDF and one-page PostScript documents.
Host integration tests run a background registry over serialized requests,
verify rendering/text, owner isolation, invalid pages/scales, thumbnail
cleanup, and an empty registry after Close. The UI test uses the real PDF,
visits five pages forward and back, checks cache byte stability, zooms,
resizes, and paints the existing document widget. Its inspection artifact is
`target/docview-widget.ppm`.

For the native IPC boot gate, build `sunlight-docserv` with its `native-smoke`
feature (`DOCSERV_FEATURES=native-smoke bash tools/build_docserv.sh`) and run
`bash tools/test_docserv.sh`. The gate supervises the binary, verifies native
open/count/render/text/close RPCs, and writes `target/docserv-serial.log`.
Afterward rebuild the normal binaries and supervisor without gate features
before producing a regular boot image.

For the full native window and alloc-only Files worker gate, build with
`DOCSERV_FEATURES=sunlight-docview/native-ui-smoke bash tools/build_docserv.sh`
and run `DOCSERV_TEST_UI=1 bash tools/test_docserv.sh`. This test logs into the
desktop, captures `target/docview-native.ppm`, confirms page/cache/zoom
behavior, closes the document, and verifies the real Files thumbnail worker.
The QMP control socket requires an execution environment that permits local
Unix sockets. For native Ghostscript verification, use
`DOCSERV_FEATURES=native-postscript-smoke` when building and
`DOCSERV_TEST_POSTSCRIPT=1` when running the boot gate.

Keys: Right/PageDown and Left/PageUp navigate; `+`/`-` change zoom in 0.25 steps
between 0.5 and 4; Ctrl+G accepts a one-based page number; the wheel scrolls the
page vertically. The title shows the filename and current page count.
