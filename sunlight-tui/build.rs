//! Build script: rasterize glyphs for framebuffer TTY screen.
//!
//! Generates MTF font atlases:
//! - login_regular_15.mtf  (15px Inter Regular — TTY UI chrome)
//! - login_title_24.mtf    (24px Inter Regular — TTY titles)
//! - tty_mono_regular_14.mtf (14px Fira Code Regular — shell/terminal grid)
//! - tty_mono_bold_14.mtf    (14px Fira Code SemiBold — bold shell text)
//!
//! MTF binary layout matches sun-font format:
//! [0..4]   magic  "MTF1"
//! [4]      line_height : u8  (px)
//! [5]      ascent      : u8  (px above baseline)
//! [6]      glyph_count : u8  (always 95, covers 0x20 ' ' … 0x7E '~')
//! [7]      reserved    : u8
//! [8..388] offset_table: 95 × u32 LE absolute file offsets to each GlyphHeader
//! [388..]  glyph_data
//!   GlyphHeader (5 bytes):
//!     advance : u8
//!     left    : u8  (interpret as i8)
//!     top     : u8  (interpret as i8)
//!     width   : u8
//!     height  : u8
//!   Pixels   : [u8; width * height]  (alpha mask, 0 = transparent)

use fontdue::{Font, FontSettings};
use std::env;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

const GLYPH_FIRST: u32 = 0x20;
const GLYPH_LAST: u32 = 0x7E;
const GLYPH_COUNT: usize = (GLYPH_LAST - GLYPH_FIRST + 1) as usize;
const HEADER_SIZE: usize = 8;
const OFFSET_TABLE_SIZE: usize = GLYPH_COUNT * 4;
const GLYPH_DATA_START: usize = HEADER_SIZE + OFFSET_TABLE_SIZE;

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let workspace_root = manifest_dir.parent().unwrap();
    let inter_path = workspace_root.join("docs/fonts/Inter/static/Inter_18pt-Regular.ttf");
    let fira_dir = workspace_root.join("docs/fonts/FiraCode/ttf");
    let fira_regular = fira_dir.join("FiraCode-Regular.ttf");
    let fira_semibold = fira_dir.join("FiraCode-SemiBold.ttf");

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={}", inter_path.display());
    println!("cargo:rerun-if-changed={}", fira_regular.display());
    println!("cargo:rerun-if-changed={}", fira_semibold.display());

    let inter_bytes = fs::read(&inter_path).unwrap_or_else(|e| {
        panic!(
            "sunlight-tui build: cannot read {}: {}",
            inter_path.display(),
            e
        )
    });
    let inter = Font::from_bytes(inter_bytes.as_slice(), FontSettings::default())
        .expect("sunlight-tui build: failed to parse Inter Regular");

    let fira_regular_bytes = fs::read(&fira_regular).unwrap_or_else(|e| {
        panic!(
            "sunlight-tui build: cannot read {}: {}",
            fira_regular.display(),
            e
        )
    });
    let fira_regular_font =
        Font::from_bytes(fira_regular_bytes.as_slice(), FontSettings::default())
            .expect("sunlight-tui build: failed to parse Fira Code Regular");

    let fira_semibold_bytes = fs::read(&fira_semibold).unwrap_or_else(|e| {
        panic!(
            "sunlight-tui build: cannot read {}: {}",
            fira_semibold.display(),
            e
        )
    });
    let fira_semibold_font =
        Font::from_bytes(fira_semibold_bytes.as_slice(), FontSettings::default())
            .expect("sunlight-tui build: failed to parse Fira Code SemiBold");

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    // UI chrome fonts (Inter)
    generate(&inter, 15.0, &out_dir.join("login_regular_15.mtf"));
    generate(&inter, 24.0, &out_dir.join("login_title_24.mtf"));

    // Shell/terminal monospace fonts (Fira Code)
    generate(
        &fira_regular_font,
        14.0,
        &out_dir.join("tty_mono_regular_14.mtf"),
    );
    generate(
        &fira_semibold_font,
        14.0,
        &out_dir.join("tty_mono_bold_14.mtf"),
    );
}

fn generate(font: &Font, px: f32, out_path: &PathBuf) {
    let lm = font
        .horizontal_line_metrics(px)
        .unwrap_or_else(|| fontdue::LineMetrics {
            ascent: px * 0.8,
            descent: -(px * 0.2),
            line_gap: 0.0,
            new_line_size: px,
        });

    let ascent_px = lm.ascent.ceil() as u8;
    let line_h = (lm.ascent - lm.descent + lm.line_gap).ceil() as u8;

    let mut glyphs: Vec<(u8, i8, i8, u8, u8, Vec<u8>)> = Vec::with_capacity(GLYPH_COUNT);
    for code in GLYPH_FIRST..=GLYPH_LAST {
        let ch = char::from_u32(code).unwrap();
        let (m, pixels) = font.rasterize(ch, px);

        let left = m.xmin.max(-128).min(127) as i8;
        let top_raw = m.ymin + m.height as i32;
        let top = top_raw.max(-128).min(127) as i8;
        let advance = (m.advance_width.ceil() as u32).min(255) as u8;
        let width = m.width.min(255) as u8;
        let height = m.height.min(255) as u8;

        glyphs.push((advance, left, top, width, height, pixels));
    }

    let mut offsets = [0u32; GLYPH_COUNT];
    let mut pos = GLYPH_DATA_START;
    for (i, (_, _, _, w, h, _)) in glyphs.iter().enumerate() {
        offsets[i] = pos as u32;
        pos += 5 + (*w as usize) * (*h as usize);
    }

    let mut out = fs::File::create(out_path).unwrap_or_else(|e| {
        panic!(
            "sunlight-tui build: cannot create {}: {}",
            out_path.display(),
            e
        )
    });

    out.write_all(b"MTF1").unwrap();
    out.write_all(&[line_h, ascent_px, GLYPH_COUNT as u8, 0])
        .unwrap();

    for off in &offsets {
        out.write_all(&off.to_le_bytes()).unwrap();
    }

    for (advance, left, top, width, height, pixels) in &glyphs {
        out.write_all(&[*advance, *left as u8, *top as u8, *width, *height])
            .unwrap();
        out.write_all(pixels).unwrap();
    }

    println!("Generated {} ({} bytes)", out_path.display(), pos);
}
