//! Build script: rasterise Inter glyphs into .mtf font files embedded in the binary.
//!
//! MTF binary layout
//! -----------------
//! [0..4]   magic  "MTF1"
//! [4]      line_height : u8  (px)
//! [5]      ascent      : u8  (px above baseline)
//! [6]      glyph_count : u8  (always 95, covers 0x20 ' ' … 0x7E '~')
//! [7]      reserved    : u8
//! [8..388] offset_table: 95 × u32 LE absolute file offsets to each GlyphHeader
//! [388..]  glyph_data  (variable length, one entry per glyph)
//!   GlyphHeader (5 bytes):
//!     advance : u8
//!     left    : u8  (interpret as i8 – left bearing)
//!     top     : u8  (interpret as i8 – pixels above baseline to bitmap top)
//!     width   : u8
//!     height  : u8
//!   Pixels   : [u8; width * height]  (alpha mask, 0 = transparent)

use fontdue::{Font, FontSettings};
use std::env;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

const GLYPH_FIRST: u32 = 0x20; // ' '
const GLYPH_LAST: u32 = 0x7E;  // '~'
const GLYPH_COUNT: usize = (GLYPH_LAST - GLYPH_FIRST + 1) as usize; // 95
const HEADER_SIZE: usize = 8;
const OFFSET_TABLE_SIZE: usize = GLYPH_COUNT * 4; // 380
const GLYPH_DATA_START: usize = HEADER_SIZE + OFFSET_TABLE_SIZE; // 388

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let workspace_root = manifest_dir.parent().unwrap().to_owned();
    let fonts_dir = workspace_root.join("docs/fonts/Inter/static");

    let regular = fonts_dir.join("Inter_18pt-Regular.ttf");
    let medium = fonts_dir.join("Inter_18pt-Medium.ttf");

    // Inform Cargo to re-run if the TTF source files change.
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={}", regular.display());
    println!("cargo:rerun-if-changed={}", medium.display());

    let regular_bytes = fs::read(&regular)
        .unwrap_or_else(|e| panic!("sun-font: cannot read {}: {}", regular.display(), e));
    let medium_bytes = fs::read(&medium)
        .unwrap_or_else(|e| panic!("sun-font: cannot read {}: {}", medium.display(), e));

    let regular_font = Font::from_bytes(regular_bytes.as_slice(), FontSettings::default())
        .expect("sun-font: failed to parse Inter Regular");
    let medium_font = Font::from_bytes(medium_bytes.as_slice(), FontSettings::default())
        .expect("sun-font: failed to parse Inter Medium");

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    generate(&regular_font, 11.0, &out_dir.join("sunlight_ui_11.mtf"));
    generate(&regular_font, 13.0, &out_dir.join("sunlight_ui_13.mtf"));
    generate(&medium_font,  13.0, &out_dir.join("sunlight_ui_medium_13.mtf"));
    generate(&regular_font, 16.0, &out_dir.join("sunlight_ui_16.mtf"));
    // Mono role: use Inter Regular at 13px (JetBrains Mono can replace later)
    generate(&regular_font, 13.0, &out_dir.join("sunlight_mono_13.mtf"));
}

fn generate(font: &Font, px: f32, out_path: &PathBuf) {
    // Pull line metrics (may be None for some fonts; use safe defaults).
    let lm = font.horizontal_line_metrics(px).unwrap_or_else(|| {
        fontdue::LineMetrics {
            ascent: px * 0.8,
            descent: -(px * 0.2),
            line_gap: 0.0,
            new_line_size: px,
        }
    });

    let ascent_px = lm.ascent.ceil() as u8;
    // Total line height = ascent + |descent| + line_gap, rounded up.
    let line_h = (lm.ascent - lm.descent + lm.line_gap).ceil() as u8;

    // First pass: rasterise all 95 printable ASCII glyphs, collect (metrics, bitmap).
    let mut glyphs: Vec<(u8, i8, i8, u8, u8, Vec<u8>)> = Vec::with_capacity(GLYPH_COUNT);
    for code in GLYPH_FIRST..=GLYPH_LAST {
        let ch = char::from_u32(code).unwrap();
        let (m, pixels) = font.rasterize(ch, px);

        // Clamp bearings to i8 range.
        let left  = m.xmin.max(-128).min(127) as i8;
        // top = pixels above baseline to the TOP of the bitmap.
        let top_raw = m.ymin + m.height as i32;
        let top = top_raw.max(-128).min(127) as i8;

        let advance = (m.advance_width.ceil() as u32).min(255) as u8;
        let width   = m.width.min(255) as u8;
        let height  = m.height.min(255) as u8;

        glyphs.push((advance, left, top, width, height, pixels));
    }

    // Second pass: compute absolute file offsets for each glyph.
    let mut offsets = [0u32; GLYPH_COUNT];
    let mut pos = GLYPH_DATA_START;
    for (i, (_, _, _, w, h, _)) in glyphs.iter().enumerate() {
        offsets[i] = pos as u32;
        pos += 5 + (*w as usize) * (*h as usize);
    }

    // Write the file.
    let mut out = fs::File::create(out_path)
        .unwrap_or_else(|e| panic!("sun-font: cannot create {}: {}", out_path.display(), e));

    // Header.
    out.write_all(b"MTF1").unwrap();
    out.write_all(&[line_h, ascent_px, GLYPH_COUNT as u8, 0]).unwrap();

    // Offset table.
    for off in &offsets {
        out.write_all(&off.to_le_bytes()).unwrap();
    }

    // Glyph data.
    for (advance, left, top, width, height, pixels) in &glyphs {
        out.write_all(&[*advance, *left as u8, *top as u8, *width, *height]).unwrap();
        out.write_all(pixels).unwrap();
    }
}
