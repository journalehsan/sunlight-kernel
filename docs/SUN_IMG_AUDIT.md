# sun-img audit — November 8, 2026

**Status:** source and host-test audit complete; no runtime or visual verification
**Scope:** `sun-img`, `sun-imgc`, their tests, documentation, and the primary
consumer paths in `sunlight-ui`, `sunlight-files`, `sunlight-wallpaper`,
`sunlight-light-lens`, and `rappid-rabbit`.

This is a focused engineering audit. It does not claim that all image-format
defects, dependency risks, or runtime memory limits are resolved.

---

## Overview

`sun-img` is a MIT-licensed Rust 2021 crate supporting `no_std + alloc` when the
`std` feature is disabled. It provides format detection, decoding, and canonical
conversion for SunlightOS images.

| Path | Role |
|------|------|
| `sun-img/src/lib.rs` | Public facade, format detection, TGA decode/encode, conversion options |
| `sun-img/src/simg_v2.rs` | SIMG v2 container: header, raw/LZ4/Sub+LZ4, CRC, ARGB decode |
| `sun-img/src/compressed.rs` | PNG and JPEG adapters over `zune-png` / `zune-jpeg` |
| `sun-img/src/crc32.rs` | Optional IEEE CRC-32 over canonical pixels |
| `sun-imgc/src/main.rs` | Host CLI: inspect, convert, to-simg, from-simg, bench-corpus |

Supported input:

- TGA type-2, 24/32 bpp, top-left or bottom-left origin.
- SIMG v2: raw, LZ4, and Sub+LZ4, with optional CRC-32.
- PNG: palette, alpha, packed grayscale, 16-bit, Adam7.
- JPEG: baseline, progressive, grayscale, and CMYK.
- BMP: detected only; decoding is unsupported.

Primary consumers:

- `sunlight-ui` uses `decode_argb_u32` for SIMG v2, direct JPEG ARGB decode, and
  the shared PNG/JPEG decoder.
- `sunlight-files` uses the shared decoder for previews and thumbnails.
- `sunlight-wallpaper` validates SIMG v2 headers and legacy TGA.
- `sunlight-light-lens` uses `inspect_dimensions` plus shared decode for editing
  and exports TGA/SIMG v2 through the streaming header helpers.
- `rappid-rabbit` uses the shared JPEG/PNG pipeline with its own browser limits.

---

## Verification performed

| Check | Result |
|-------|--------|
| `cargo test -p sun-img -p sun-imgc --target x86_64-unknown-linux-gnu` | 35 passed: 23 lib, 8 compressed, 4 CLI |
| `cargo test -p sunlight-ui --features std --lib --target x86_64-unknown-linux-gnu` | 272 passed |
| `cargo clippy -p sun-img -p sun-imgc --all-targets -- -D warnings` | clean |
| `cargo fmt --check -p sun-img -p sun-imgc` | clean |
| `cargo check -p sun-img --no-default-features` | passes |
| `cargo audit` | 13 workspace vulnerabilities; none in the `sun-img` graph |

The `sun-img` dependency graph resolved to `lz4_flex` 0.11.6, `zune-core`
0.4.12, `zune-jpeg` 0.4.21, and `zune-png` 0.4.10. The workspace advisories
affect `crypto-bigint`, `rsa`, `rustls`, `rustls-webpki`, and `webpki-roots`,
not these image crates.

---

## Findings

### Medium

1. **TGA decode bypasses the documented size limits.**
   `decode_image` routes TGA to `decode_tga` (`sun-img/src/lib.rs:286`), which
   allocates directly at `sun-img/src/lib.rs:323` without the
   `MAX_DIMENSION`/`MAX_DECODED_BYTES` checks used by `inspect_dimensions`
   (`sun-img/src/lib.rs:167`). Confirmed empirically: a 9000×1 TGA is accepted
   by `sun-imgc convert` even though the documented limit is 8192 per side.

2. **TGA descriptor bits are ignored.**
   `inspect_tga` only reads the top-left bit (`sun-img/src/lib.rs:267-276`).
   Bit 4 (right-to-left) is ignored, and bits 0–3 (alpha size) are not honored
   when decoding (`sun-img/src/lib.rs:324-340`). Confirmed empirically: an RTL
   TGA decodes in file order, and a 32 bpp TGA with descriptor `0x20` and zero
   alpha bytes round-trips as fully transparent.

3. **`tools/version_manager.py` adds one blank line to every `Cargo.toml` per
   version bump.**
   The regex at `tools/version_manager.py:876` lets `\s*` capture the trailing
   newline, then `tools/version_manager.py:882-883` appends another. At audit
   time `sun-img/Cargo.toml` contained 247 blank lines and
   `sun-imgc/Cargo.toml` contained 417.

4. **`bench-corpus` summary totals include files that failed to decode.**
   `total_current` is incremented before the skip at
   `sun-imgc/src/main.rs:261-267`, while `total_raw` and `total_v2` are not, so
   `files` and `saved_vs_current_pct` are misleading
   (`sun-imgc/src/main.rs:330-342`).

5. **`sun-imgc inspect` fully decodes PNG/JPEG just to print dimensions.**
   `sun-imgc/src/main.rs:158-163` calls `decode_image` instead of
   `inspect_dimensions` (`sun-img/src/lib.rs:139`). Large images can be slower
   or fail where header inspection would succeed.

### Low

6. **Encode is not OOM-safe and peaks at roughly 5–6× image memory.**
   `rgba_to_bgra` (`sun-img/src/simg_v2.rs:310`), `lz4_compress` (`:404`),
   `build_file` (`:434`), and `encode` (`:462`) use infallible allocations,
   while decode paths use `try_reserve`.

7. **`collect_images` follows symlinks and can recurse forever on a cycle.**
   See `sun-imgc/src/main.rs:362-380`.

8. **`convert_with_options` can silently discard alpha.**
   `ConvertOptions::default()` sets `force_alpha = true`
   (`sun-img/src/lib.rs:60-68`), so selecting `OutputFormat::SimgV2` through
   that API forces opaque alpha (`sun-img/src/lib.rs:234-245`).
   `convert_to_simg_v2` is the alpha-preserving path.

9. **Documentation drift.**
   `docs/color-image-conversion.md:14-23` still says PNG/JPEG/BMP decoding is
   unsupported. `docs/apps/light-lens.md:272,294` says 30 `sun-img` tests
   instead of the current 35.

10. **Minor CLI and API nits.**
    Unused `ImageError::Io` (`sun-img/src/lib.rs:80`), dead
    `_keep_convert_to_simg_v2_link` (`sun-imgc/src/main.rs:424`), fixed `.tmp`
    name races (`sun-imgc/src/main.rs:401`), no `--help`/`--version`, loose TGA
    detection heuristic (`sun-img/src/lib.rs:117`), bitwise-only CRC
    (`sun-img/src/crc32.rs:7`), and a duplicate JPEG header scan
    (`sun-img/src/compressed.rs:130`).

---

## What is solid

- SIMG v2 parsing is strict: magic, version, header size, flags, enums,
  dimensions, payload bounds, and optional CRC are all validated before
  allocation.
- Malformed SIMG v2 never falls back to TGA, preserving the documented
  compatibility boundary.
- The encoder deterministically selects the smallest of raw, LZ4, and Sub+LZ4,
  and emits a CRC over canonical pixels.
- `decode_argb_u32` is a good renderer-oriented path: one allocation, in-place
  LZ4/Sub reversal, and no RGBA copy.
- `docs/SIMG_V2.md` is a clear, accurate on-disk specification.
- Tests cover alpha, palettes, packed grayscale, 16-bit samples, Adam7, JPEG
  variants, and malformed inputs.

---

## Limitations

- All evidence is host-side; no QEMU boot, native UI, or visual output was
  exercised.
- No fuzzing targets and no large real-world corpus benchmark were run.
- The unsafe byte-view in `decode_argb_u32` was only exercised on little-endian
  x86_64; big-endian behavior is untested.
- `cargo audit` is workspace-wide; it shows the `sun-img` graph is clean, but it
  is not a license or patent review.
- The SIMG v2 patent-status notice remains unresolved by design.

---

## Suggested next steps

1. Enforce `checked_layout`/size limits in `decode_tga`, and either implement or
   reject TGA right-to-left and alpha-size descriptor bits.
2. Fix the `version_manager.py` regex and strip the accumulated blank lines from
   the manifests.
3. Make `inspect` use `inspect_dimensions`, fix `bench-corpus` totals, and skip
   symlinks during corpus walks.
4. Add fuzz targets for TGA, SIMG v2, PNG, and JPEG, plus a corpus benchmark,
   before expanding format support.
