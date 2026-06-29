# SunlightOS MiniType Font System

SunlightOS uses a lightweight build-time font pipeline (`sun-font`) to render
antialiased text via pre-rasterised Inter glyphs.  No TTF parser or FreeType
runs at boot — the pixel data is baked into the binary at compile time.

## Why MiniType?

| Choice | Reason |
|--------|--------|
| Build-time rasterisation | Zero startup cost; no heap fragmentation from font loading |
| Inter typeface | Open-source (OFL), designed for screen legibility at small sizes |
| No FreeType/HarfBuzz/Skia | Keeps the userland binary small; SunlightOS has no dynamic linking |
| Custom `.mtf` format | Trivial to parse in `no_std`; 8-byte header + alpha bitmap per glyph |

## Font Roles

| Variant | Source | px | Use |
|---------|--------|----|-----|
| `UiSmall` | Inter Regular | 11 | Status bar, captions, hints |
| `UiRegular` | Inter Regular | 13 | File names, toolbar labels, general UI |
| `UiMedium` | Inter Medium | 13 | Selected items, folder names, emphasis |
| `UiLarge` | Inter Regular | 16 | Window titles, large headings |
| `MonoRegular` | Inter Regular | 13 | Paths, technical metadata (JetBrains Mono drop-in later) |

## Font Asset Paths

**Source TTFs** (open-source, OFL licensed):
```
docs/fonts/Inter/static/Inter_18pt-Regular.ttf
docs/fonts/Inter/static/Inter_18pt-Medium.ttf
```

**Generated MTF files** (baked into the binary at build time, not shipped separately):
```
$OUT_DIR/sunlight_ui_11.mtf
$OUT_DIR/sunlight_ui_13.mtf
$OUT_DIR/sunlight_ui_medium_13.mtf
$OUT_DIR/sunlight_ui_16.mtf
$OUT_DIR/sunlight_mono_13.mtf
```

For future OS-image installation (dynamic loading path, not yet used):
```
/usr/share/sunlightos/fonts/minitype/
```

## Regenerating .mtf Files

The `sun-font` build script (`sun-font/build.rs`) regenerates the `.mtf` files
automatically whenever a source TTF changes.  To regenerate manually:

```bash
# Force a clean rebuild of the font bitmaps
cargo clean -p sun-font
cargo build -p sun-font
```

To add **JetBrains Mono** as the proper mono font:
1. Place `JetBrainsMono-Regular.ttf` in `docs/fonts/JetBrainsMono/`
2. Update `sun-font/build.rs` to reference it for `sunlight_mono_13.mtf`
3. Rebuild

## MTF Binary Format

```
[0..4]   magic       : "MTF1"
[4]      line_height : u8   (px, total vertical advance)
[5]      ascent      : u8   (px above baseline)
[6]      glyph_count : u8   (always 95 — ASCII 0x20 ' ' to 0x7E '~')
[7]      reserved    : u8
[8..388] offset_table: 95 × u32 LE  (absolute byte offsets to GlyphHeaders)
[388..]  glyph_data  : variable

GlyphHeader (5 bytes):
  advance : u8
  left    : u8  (reinterpret as i8, left bearing)
  top     : u8  (reinterpret as i8, pixels above baseline to bitmap top)
  width   : u8
  height  : u8

Pixels (width × height bytes):
  Alpha mask, 0 = transparent, 255 = fully opaque
```

## API Quick Reference

```rust
use sun_font::{FontRole, TextStyle, draw_text, measure_text, line_height};

// Draw text — y is the top of the em box
draw_text(&mut canvas, "Hello", x, y, &TextStyle::new(FontRole::UiRegular, color));

// Centred in a rect
draw_text_centered(&mut canvas, rect, "OK", &TextStyle::new(FontRole::UiSmall, color));

// Right-aligned with padding
draw_text_right(&mut canvas, rect, "1.2 MB", &TextStyle::new(FontRole::UiRegular, color), 8);

// Vertically centred in a row of given height
draw_text_vcenter(&mut canvas, "filename.txt", x, row_y, ROW_H,
                  &TextStyle::new(FontRole::UiRegular, theme.text));

// Metrics
let lh: u32 = line_height(FontRole::UiRegular);
let sz: Size = measure_text("path/to/file", FontRole::MonoRegular);
```

## Limitations (v0)

- Latin / printable ASCII only (0x20 – 0x7E).
- Characters outside that range render as `?`.
- No complex text shaping (BiDi, ligatures, kerning pairs).
- No Persian / Arabic / CJK.
- Font data embedded in the binary — not hot-swappable at runtime.

## Future

| Phase | Plan |
|-------|------|
| v0 (now) | MiniType Inter for all UI text |
| v1 | JetBrains Mono for the `MonoRegular` role |
| v2 | Extended Latin (diacritics) via glyph range expansion |
| v3 | Full TTF/OpenType renderer (fontdue or custom) for rich desktop/international text |
| v4 | Per-locale font substitution |

## Sample Rendering

![MiniType font sample](minitype-samples.png)
