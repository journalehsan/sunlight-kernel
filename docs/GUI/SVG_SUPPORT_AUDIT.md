# SVG icon support and migration

Verified 2026-10-18 against the working tree.

The desktop shell, Files, file dialogs, and Control Panel now use the original
SVG theme assets. They build for `x86_64-sunlightos` with the native Rust std
overlay introduced for Hayro. The earlier failure on `x86_64-unknown-none`
does not apply to this native std target.

## Assets

- The downloaded theme is now `docs/icons/SunlightOS`, with 8,835 SVG paths.
- Removed 8,830 obsolete TGA theme files. All had matching SVG source paths.
- Migrated all 66 distinct theme icons referenced by Rust consumers and RAMFS.
- Theme lookup and installed theme aliases now use `.svg`.
- Voice Recorder's theme-derived bundle icon and manifest now use SVG.
- Upstream artwork, attribution, and licensing were retained; the installed
  theme name is SunlightOS.

Font-generated symbolic glyphs, cursor rasters, wallpapers, and custom bundle
TGAs without corresponding theme SVG sources remain on their existing paths.
These are separate from the migrated downloaded theme.

## Shared loader

`sunlight-ui/image/icon.rs` provides a `Copy` embedded `IconImage` handle.
SVG parsing uses resvg/usvg 0.45.1 with default features disabled. Linked
resource loading and system-font discovery are not enabled. Icons render at
the destination size, preserve aspect ratio, and produce straight ARGB pixels
for the existing canvas alpha compositor.

Parsed trees have an LRU cache of at most 128 entries. Raster caching is
limited to 256 entries and 4 MiB of pixel data. Source size is limited to
2 MiB, intrinsic dimensions to 4096, and raster dimensions to 512. An in-flight
raster owns an `Arc`, so eviction does not invalidate a drawing operation.
File-loaded icon overrides decode once into owned raster images.

The old `TgaImage` re-export and canvas methods remain compatibility aliases
for existing clients. The original zero-allocation TGA reader is still
available under `image::tga`. Freestanding programs that do not enable `svg`
retain their no_std build path.

## Native startup and builds

The four GUI consumers select the `native-svg` feature and the existing
native std overlay. Their normal Rust `main` functions use the original
kernel argv/envp through `sunlight_libc::std_runtime::startup_args`, preserving
existing argument handling and launch tracing. The dialog command also
supports this native startup when built with its GUI package.

```sh
# Build the four GUI consumers and the dialog command.
bash tools/build_svg_apps.sh

# Build one consumer, optionally with additional features.
bash tools/build_svg_apps.sh -p sunlight-files
```

The kernel builder builds these applications in an independent scratch target
directory and embeds the native std artifacts. Normal build/run/write/test
scripts use the same wrapper. Native SVG applications retain ELFOSABI_NONE
and have no Helios personality stamp.

## Validation

```sh
cargo test --offline -p sunlight-ui --features svg --target x86_64-unknown-linux-gnu --lib --test theme_svg
bash tools/test_svg_icons.sh
```

Results:

- 272 shared UI unit tests and four SVG integration tests passed. All 66 used
  theme icons render nonempty images at 16, 32, 48, and 64 px. Checks cover
  transparency, canvas alpha composition, aspect ratio, file-loaded SVGs,
  invalid input, and cache eviction while a raster remains in use.
- All four native GUI consumers and the dialog command built successfully.
- QEMU desktop gate passed: desktop, Files, Control Panel, and the Open
  Document file dialog all displayed through the real UI path. The serial
  log confirms SVG rasterization in four distinct consumer processes.
  Screenshots were captured and the Files screenshot was visually inspected.
- Representative freestanding consumers (display, Writer, Calendar, Devices)
  type-checked individually. They must be checked separately because their
  existing allocator features conflict when unified in one Cargo invocation.
- Files: 29 tests passed, one ignored. Control Panel: 15 tests passed.
  Dialog host library: 14 tests passed.
- Shell: 61 tests passed; the existing
  `sidebar::tests::telemetry_missing_before_first_sample_is_unavailable` test
  failed. The same failure was reproduced from the committed sidebar source
  in an isolated host crate with SVG disabled. Sidebar code was not modified.

Local artifacts are ignored build output:

- `target/svg-icons-proof/serial.log`: desktop gate serial evidence.
- `target/svg-icons-proof/{desktop,files,control-panel,file-dialog}.ppm`:
  screenshots captured directly through QEMU.
- `target/svg-support-audit/ui-svg-tests.log`: shared UI and SVG tests.
- `target/svg-support-audit/consumer-tests.log`: consumer unit tests.
- `target/svg-support-audit/dialog-tests.log`: dialog library tests.
- `target/svg-support-audit/freestanding-compat.log`: compatibility checks.
- `target/svg-support-audit/baseline-sidebar.log`: pre-existing failure.

Before migration, a separate native std probe also rendered six SVG icons at
four sizes in QEMU; all 24 pixel hashes matched the Linux host renderer, and a
known half-transparent red square passed exact pixel checks. That evidence is
in `target/svg-support-audit/native-guest.log`. The desktop gate above verifies
the final shared loader and its actual consumers.
