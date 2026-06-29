---
name: minitype-font-system
description: sun-font crate — build-time rasterised Inter bitmap fonts for SunlightOS UI; File Manager migrated as first consumer
metadata:
  type: project
---

SunlightOS now has a `sun-font` crate (workspace root) providing antialiased text via Inter glyphs baked at build time.

**Why:** The old 5×7 pixel bitmap font was too retro; needed a modern, lightweight alternative without FreeType/HarfBuzz at runtime.

**What was built:**
- `sun-font/build.rs`: uses `fontdue` (build-dep only) to rasterise Inter TTF into `.mtf` files stored in `$OUT_DIR`
- `sun-font/src/lib.rs`: `no_std` runtime — `FontRole`, `TextStyle`, `draw_text`, `measure_text`, `draw_text_centered`, `draw_text_right`, `draw_text_vcenter`, `line_height`, `ascent`
- `sun-font/src/bin/demo.rs`: `minitype-demo` binary (requires `--features std`) renders a PPM sample sheet
- `sunlight-files/src/main.rs`: all 28 `canvas.draw_text` calls replaced with `sf_draw`/`sf_vcenter`/`sf_right`/`sf_centered` using appropriate `FontRole`
- `docs/MINITYPE_FONTS.md`: full documentation
- `assets/fonts/minitype/generate.sh`: manual regeneration script

**MTF format:** 8-byte header + 95×4 offset table + per-glyph (5-byte header + alpha bitmap). Magic = "MTF1". `y` in `draw_text` = em-box top; baseline = y + ascent.

**Font sources:** `docs/fonts/Inter/static/Inter_18pt-Regular.ttf` and `Inter_18pt-Medium.ttf` (OFL licensed, already in repo).

**Font roles:**
- UiSmall = Inter 11px, UiRegular = Inter 13px, UiMedium = Inter Medium 13px, UiLarge = Inter 16px, MonoRegular = Inter 13px (JetBrains Mono drop-in later)

**Build:** `cargo build -p sun-font` or `cargo build -p sunlight-files` — both compile clean for `x86_64-unknown-none`.
**Tests:** `cargo test -p sun-font --target x86_64-unknown-linux-gnu` — 6/6 pass.

**How to apply:** When adding text to any UI widget, import `sun_font::{FontRole, TextStyle, draw_text}` and avoid the old `canvas.draw_text()` for new code.
