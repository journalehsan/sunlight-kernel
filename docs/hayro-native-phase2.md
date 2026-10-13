# Hayro native compilation: Phase 2

> Current policy: Hayro is the preferred host-only PDF backend. MuPDF is disabled
> by default and retained for explicit rollback. Permanent removal requires the
> full Hayro renderer to build and render a PDF natively on SunlightOS. See
> [current backend status](document-viewer.md#backend-deprecation-status).

## Result (2026-10-12)

**The parser and PDF image decoders compile without forks for SunlightOS;
the complete Hayro PDF renderer does not.** A parser smoke executable links
using the existing userspace linker script and bounded native allocator. It
has not been installed or executed in SunlightOS. No native PDF rendering or
in-system document canvas display is claimed.

The new, isolated experiment is [tools/hayro-native-probe](../tools/hayro-native-probe/).
The [complete compatibility matrix](hayro-phase2-matrix.md) inventories **84
packages: 70 target participants, 14 host-only, with two of the target
participants also built for the host**. These counts include the Phase 1 driver
and existing canvas dependencies; they are not 70 Hayro runtime libraries.
Every mandatory `std` crate in that selected graph is marked H or F.
H means unconditional `std`; F means the selected features require it.
A packages can still lead to F dependencies: for example, `euclid` forwards
`std` to `num-traits`, and `miniz_oxide` selects `simd-adler32`.

The user's later instruction permits removing MuPDF if it causes issues.
Retention is therefore provisional. This investigation demonstrates neither a
MuPDF runtime fault nor a working Hayro replacement. MuPDF and Ghostscript
integration, the document service/viewer, kernel, root manifests/lockfile, and
Phase 1 prototype were left untouched. No general `std` layer or upstream fork
was introduced.

## Audit method and upstream features

Audited published sources from the local Cargo registry against resolved
manifests, normal/build dependency edges, and compiler diagnostics. The original
Phase 1 lockfile is stale against current local package versions: its `--locked`
metadata command refuses resolution. A copy in
`/tmp/sunlight-hayro-phase2-host` was resolved offline, preserving the original.
Only the temporary copy's sample fixture path was adjusted for its new location.
The source audit and reproduced host renderer use that resolved copy. The new
probe has its own pinned dependencies, workspace and lockfile.

`cargo metadata` alone overstates this graph: inactive dependency dev features
include `glifo`, a second font stack, and `foldhash/std`. The inventory script
intersects metadata with `cargo tree -e normal,build --format '{p}|{f}'` and
propagates host/target roles through non-dev edges. The matrix excludes those
inactive packages/features. Source anchors are relative to each crate's registry
root and may show cfg or example code; reviewed classifications, rather than raw
text matches, determine whether an API is mandatory.

The current upstream [Hayro manifest](https://raw.githubusercontent.com/LaurenzV/hayro/main/hayro/Cargo.toml)
still offers embedded fonts/CMaps, logging, and scaler SIMD, with no `no_std`
switch. The [interpreter manifest](https://raw.githubusercontent.com/LaurenzV/hayro/main/hayro-interpret/Cargo.toml)
still forces syntax `std`, `images`, `unsafe`, and color management. These
observations agree with the published 0.8.0 source. The experiment does not
substitute Git main for the pinned releases.

| Boundary | Published feature path | Experiment / implication |
| --- | --- | --- |
| `hayro-syntax` 0.8.0 | `default-features = false` | Native release rlib succeeds with only `smallvec` as a dependency |
| Syntax image codecs | Add `images`, leave `std` and `unsafe` off | Native release rlibs succeed for CCITT, JBIG2, JPEG2000, JPEG; this does not establish native codec correctness |
| Syntax Flate decoding | Leave `unsafe` off | Uses built-in alloc-only decoder; sample content streams decode on host without `flate2` |
| Syntax cache/ownership | Disable `std` | Upstream `sync.rs` selects `BTreeMap/BTreeSet`, `Rc`, `RefCell`, `OnceCell`; single-thread ownership, not `Send/Sync` parity |
| `vello_cpu` 0.3.0 | Defaults off, `libm`, `u8_pipeline` | Native check succeeds; release code generation fails with existing target flags |
| Vello PNG / multithreading / text | All omitted | Removes PNG I/O and worker dependencies; Hayro cannot request this configuration through its public features |
| Geometry/font stack | Defaults off, `libm` where supported | Upstream alternatives exist for kurbo/color/peniko/skrifa; normal Hayro graph selects `std` |
| `hayro-cmap` | Defaults off | Parser is alloc-only; embedded CMaps force `std`, `LazyLock` and Brotli's std adapter |
| `embed-fonts` | Embedded substitute standard fonts | Avoids OS font discovery; disabling it is not a std port and requires a font-loading policy |
| Hayro `simd` | Disable defaults, omit `simd` | Removes optional pic-scale acceleration, but neither scalar pic-scale nor moxcms/pxfm becomes alloc-only; other dispatch remains |

Upstream [Vello CPU's manifest](https://raw.githubusercontent.com/linebender/vello/main/vello_cpu/Cargo.toml)
also exposes the `libm` path and explicitly makes PNG and multithreading imply
`std`. Feature addition cannot subtract features forced by Hayro's manifests.
Consequently, adding direct `libm` dependencies alongside Hayro would leave
the `std` requirements active.

## Blockers and existing userspace capabilities

| Category | Concrete blockers / scope | Existing SunlightOS capability and remaining gap |
| --- | --- | --- |
| Filesystem | Phase 1 CLI and production backend use `File`, `Path`, metadata and output writers. Core Hayro accepts owned bytes; filesystem access is not intrinsically required for memory rendering. ICC/PNG/Flate use `std::io` for memory parsing too. | `sunlight-libc` open/read/close and alloc-only wire paths exist. A bounded VFS snapshot adapter is small future work, but it does not supply Rust I/O traits. No filesystem shim needed for this probe. |
| Synchronization | Interpreter `cache.rs`: `Arc`, `Mutex`; ICC/colors/functions: `OnceLock`, `LazyLock`. Embedded CMaps also use `LazyLock`. | Core atomics, alloc Arc/Rc and existing internal spinlocks exist. No native public equivalent of all Rust blocking locks was established. Parser already uses upstream single-thread alternatives. Interpreter changes need upstream design, not a replacement module named std. |
| Threading | No Rayon, thread-local or worker dependency selected in Hayro's actual graph. The production viewer separately uses std threads/channels. | Native thread spawn/TLS exists, but `sunlight-libc/src/thread.rs` documents absent join and deferred stack/TLS reclamation. A single-thread renderer need not add workers. No kernel threading change demonstrated necessary. |
| Allocation | Infallible parser containers, decoded streams, glyph caches, color tables and Vello Pixmap. Pixel/source limits do not bound total engine working set. `alloc-stdlib` and zlib-rs's selected Rust allocator configuration require std. | Reclaiming allocator, telemetry, 256 KiB static or 1/8/16 MiB dynamic presets exist. Null allocation plus `try_reserve` can report failure at our snapshot boundary. Internal infallible allocations terminate; a native fallible parser/render contract is absent. |
| Font handling | Interpreter font cache uses shared ownership/cells; default skrifa/read-fonts/font-types use std/math features. CJK embedded CMaps add std/Brotli. | Embedded fonts avoid fontconfig/platform lookup for the sample. Existing sun-font/canvas does not replace PDF font parsing, hinting, encodings, or outlines. Defaults-off/libm font support needs a deliberate upstream feature path. |
| Rasterization | Hayro forces Vello std/png, image/png, scalar pic-scale, moxcms/pxfm. `image`, `png`, `fdeflate`, `flate2`, moxcms, pic-scale and pxfm lack a working alloc-only switch in these releases. | Existing ARGB8888 `RasterImage`/DocumentCanvas is no_std. Small alloc-only Vello fixture works on host; native codegen blocker remains. A custom PDF Device implementation would be substantial new rendering maintenance and was not attempted. |
| Platform APIs | SIMD runtime detection, x86 OnceLock dispatch in pxfm, color-management time/I/O APIs, unwind guard in Phase 1, and missing target std. | Native target has core/alloc, syscalls, CRT and atomics. CPU initialization enables OSFXSR, but source search found no FXSAVE/FXRSTOR or XSAVE/XRSTOR save/restore implementation; SIMD state isolation remains unvalidated. Existing flags and an unsafe ABI override must not certify SIMD execution. |
| Build-time dependencies | Rust proc macros, PHF table generators, autocfg probes, build.rs scripts. | They execute on the Linux build host and may use std. No Hayro-side cc/cmake/bindgen/system zlib/fontconfig/GPU library in this graph. These do not need a native std port. |

Important source locations beyond matrix lib.rs anchors:
`hayro-interpret/src/cache.rs:5-7`, `color/icc.rs:7`, `color/device_cmyk.rs:3`,
`hayro-syntax/src/sync.rs`, `filter/lzw_flate.rs:16`,
`hayro-cmap/src/bcmap/embedded.rs:2`, `moxcms/src/profile.rs:41,690`,
`moxcms/src/dat.rs:32`, `pxfm/src/acosf.rs:195-199`,
`fdeflate/src/compress.rs:2`, `flate2/src/lib.rs:131`,
and `vello_common/src/encode.rs:32` (cfg std; alloc-only alternative exists).
These paths are relative to the corresponding published crate.

Host-only: autocfg, bytemuck_derive, fastrand, phf_generator, phf_macros,
proc-macro2, quote, syn 2 and 3, synstructure, thiserror-impl, unicode-ident,
yoke-derive, zerofrom-derive. `phf_shared` and `siphasher` are compiled for both
domains. `siphasher/std` is an empty feature and its source remains no_std, so
the feature name alone is not a direct blocker. Build scripts for crc32fast,
num-traits, proc-macro2, quote and thiserror run on the host; their libraries can
still be target participants with independent requirements.

## Exact compiler and linker findings

Toolchain: `rustc 1.98.0-nightly (8954863c8 2026-06-05)`, LLVM 22.1.6.
Target: existing `x86_64-unknown-none`. Native library commands use static
relocation, no red zone, `target-cpu=x86-64-v2`; executable commands additionally
use the existing `services/user-space.ld` with an absolute path. Cargo runs the
isolated crate linker from that crate's directory, so a root-relative linker
path initially failed with `cannot find linker script services/user-space.ld`.
Using the absolute path resolved this invocation issue without changing the script.

| Attempt | Actual result | Evidence under `target/hayro-phase2/` |
| --- | --- | --- |
| Parser only, native release build | Success, rlib | `hayro-phase2-parser-native.log` |
| Parser + images, native release build | Success, rlibs | `hayro-phase2-images-native.log` |
| Parser + images + native CRT/allocator, executable link | Success, System V x86_64 EXEC ELF, entry 0x400270; no Linux std/Helios stamp | `hayro-phase2-parser-link.log`; ELF under `/tmp/hayro-phase2-parser-elf/` |
| Alloc-only Vello + existing UI, native release check | Success, metadata only | `hayro-phase2-raster-check.log` |
| Same Vello native release build | Failure in `vello_common` and `vello_cpu`: `rustc-LLVM ERROR: Do not know how to split the result of this operator!` | `hayro-phase2-raster-native.log` |
| Diagnostic Vello build with `target-feature=-soft-float,+sse,+sse2` | Codegen succeeds, with target ABI warnings; diagnostic rlibs, not a supported build or executable | `hayro-phase2-raster-sse.log` |
| Full Hayro graph native build / keep-going check | E0463 missing std and cascading prelude/type/macro failures | `hayro-phase2-full-native.log`, `hayro-phase2-full-keepgoing.log` |

The keep-going check directly failed these **14 earliest compilation units**:
`alloc-stdlib`, `byteorder-lite`, `color`, `crc32fast`, `fearless_simd`,
`font-types`, `linebender_resource_handle`, `num-traits`, `polycool`, `pxfm`,
`rustc-hash`, `simd-adler32`, `zlib-rs`, and `zune-core`. E0463 includes explicit
`extern crate std` in some no_std-marked crates. Hundreds of missing prelude
symbols are consequences, not hundreds of separate primitive gaps. Cargo cannot
compile their parents, so this list is not the complete std blocker inventory;
the matrix includes the remaining mandatory parent crates.

The SSE diagnostic warns both:
`target feature soft-float cannot be disabled with -Ctarget-feature` and
`target feature soft-float must be enabled to ensure that the ABI of the current
target can be implemented correctly`. Existing `target-cpu=x86-64-v2` alone
does not enable SSE in `rustc --print cfg` for this target. This isolates a
second problem beyond std but does not establish the LLVM root cause or a safe
fix. Rebuilding core/alloc with a different target ABI or changing target/kernel
CPU handling exceeds the experiment; no such change was made.

Full rendering never reached linking. There is therefore **no measured full
renderer linker result** and no successful native PDF compile on which to base
an in-system render attempt. Parser ELF success does not satisfy that condition.

## Measured results and allocation behavior

Host measurements on Linux x86_64, release builds, the original four-page
115,543-byte sample, single diagnostic runs. They are not native benchmarks.

| Experiment | Result |
| --- | --- |
| New alloc-only parser/image/raster probe | Four tests pass: sample pages/dimensions/content, invalid/oversized input, raster pixel layout, lifecycle/failed snapshot allocation |
| Alloc-only repeated lifecycle | 21 complete sample open/decode-all-four/drop + 64x48 vector raster/drop cycles; 19,992 decoded operations; 5.084 ms total |
| Requested live heap, new probe | Baseline/final both 21,531 B; peak 168,557 B; incremental peak 147,026 B |
| Controlled allocation failure | Fail the next host allocator request; bounded snapshot `try_reserve_exact` returns `Error::Allocation`; no panic |
| Phase 1 renderer regression in temporary copy | All three original rendering tests pass |
| Host full renderer open | 0.084 ms, four pages |
| 0.75x full PDF raster | 446x631; 1,125,704 B; 2.165–4.469 ms/page |
| 1.0x full PDF raster | 595x841; 2,001,580 B; 2.373–5.152 ms/page |
| 1.5x full PDF raster | 892x1262; 4,502,816 B; 3.311–7.270 ms/page |
| 2.0x full PDF raster | 1190x1683; 8,011,080 B; 4.786–6.710 ms/page |
| Full renderer requested heap | Peak 10,360,517 B; every one of 21 additional open/render-all-four/close cycles returns to 1,864 B |
| Full renderer process peak RSS | 19,484 KiB, sole child of fresh Python measurement process |

Different argv paths affect the driver baseline; neither baseline is engine
retained memory alone. Requested bytes exclude allocator headers/fragmentation,
stacks and mappings. Timings exclude full-renderer canvas conversion/output;
the new probe timing includes parsing, decoding and the tiny vector fixture.
These are distinct workloads and cannot be compared as renderer speeds.
No growing live allocation was seen in these sample lifecycles; this is not a
general leak/conformance/safety proof. Logs and raster outputs are generated
artifacts under `target/hayro-phase2/`.

The probe bounds source snapshots to 256 KiB and uses a fixed 64x48 raster
(12,288 B RGBA plus 12,288 B ARGB at conversion). Its native executable selects
the existing 1 MiB dynamic allocator. The fixed trusted fixture keeps output
size bounded; arbitrary PDFs can exceed the heap through decoded streams,
objects, images, fonts or recursion. The source limit is **not a total memory
budget or a safe untrusted-PDF limit**. Renderer/parser internals use infallible
allocations. Native panic behavior terminates the probe with status 2; aborts
or faults can also terminate it, and OOM is not a recoverable document error.
Only snapshot reservation failure is returned as an ordinary error. No heap
enlargement or catch_unwind emulation was added.

## Smallest maintainable recommendation and stop boundary

The follow-up [native std gap analysis](hayro-std-gap-analysis.md) maps the
alternative Rust platform-port route to existing native facilities, concrete
threading/synchronization/SIMD gaps and staged acceptance gates. It is an audit
and plan, not an implemented std port or a native rendering result.

Use the unmodified alloc-only syntax parser as the native feasibility foothold,
with image features enabled only when needed. It is independently testable and
does not require a new kernel primitive. Do not present it as a viewer backend.
Preserve the production renderer provisionally while separately collecting
evidence for any MuPDF fault; removing it remains allowed once that decision
has concrete evidence and an acceptable replacement/fallback.

There is **no complete native rendering integration within minimal changes** in
the audited releases. The next viable route is upstream-owned alloc-only
interpreter/render features: stop forcing syntax std/unsafe, select Vello libm
without PNG, configure the font/math stack, and address color management and
scaling. Simply changing imports in Hayro is insufficient: moxcms, pic-scale,
pxfm, image/PNG and embedded CMap decompression have independent requirements.
Omitting ICC, scaling, fonts or broad PDF operations to draw sample vectors
would be a new limited PDF renderer with significant maintenance, so no custom
Device or broad set of forks was started.

Before any native render claim, require a supported target/SIMD configuration,
bounded/fallible decoded/raster working-set behavior or validated process failure
containment, a full native executable link, and a real SunlightOS test displaying
a PDF raster through the existing canvas. That gate should measure VFS snapshot
loading, display pixels, repeated release/heap recovery, and shared-memory
ownership. This phase stops at the concrete compilation/ABI blockers.

## Reproduction

Run these commands from the repository root. The probe is isolated; do not run
the version-changing general build scripts for this experiment.

```sh
cargo test --manifest-path tools/hayro-native-probe/Cargo.toml \
  --target x86_64-unknown-linux-gnu --release --features raster,images \
  --locked --offline -- --nocapture
cargo clippy --manifest-path tools/hayro-native-probe/Cargo.toml \
  --target x86_64-unknown-linux-gnu --features raster,images \
  --all-targets --locked --offline -- -D warnings
RUSTFLAGS='-C relocation-model=static -C no-redzone -C target-cpu=x86-64-v2' \
  cargo build --manifest-path tools/hayro-native-probe/Cargo.toml \
  --target x86_64-unknown-none --release --features images --locked --offline
RUSTFLAGS="-C link-arg=-T$PWD/services/user-space.ld -C relocation-model=static -C no-redzone -C target-cpu=x86-64-v2" \
  cargo build --manifest-path tools/hayro-native-probe/Cargo.toml \
  --target x86_64-unknown-none --release --features native-entry,images \
  --bin hayro-parser-native --locked --offline
# Type checks, but replacing check with build reproduces the LLVM failure:
RUSTFLAGS='-C relocation-model=static -C no-redzone -C target-cpu=x86-64-v2' \
  cargo check --manifest-path tools/hayro-native-probe/Cargo.toml \
  --target x86_64-unknown-none --release --features raster --locked --offline
# Expected failure: mandatory std dependencies.
RUSTFLAGS='-C relocation-model=static -C no-redzone -C target-cpu=x86-64-v2' \
  cargo check --manifest-path tools/hayro-native-probe/Cargo.toml \
  --target x86_64-unknown-none --release --features full-render \
  --locked --offline --keep-going
```

To regenerate the matrix with the preserved temporary Phase 1 copy:

```sh
cargo metadata --manifest-path /tmp/sunlight-hayro-phase2-host/Cargo.toml \
  --format-version 1 --locked --offline --filter-platform x86_64-unknown-none \
  > /tmp/hayro-phase2-metadata.json
cargo tree --manifest-path /tmp/sunlight-hayro-phase2-host/Cargo.toml \
  --target x86_64-unknown-none --locked --offline -e normal,build \
  --prefix none --format '{p}|{f}' > /tmp/hayro-phase2-tree-features.txt
python3 tools/hayro-native-probe/audit.py /tmp/hayro-phase2-metadata.json \
  /tmp/hayro-phase2-tree-features.txt docs/hayro-phase2-matrix.md
```
