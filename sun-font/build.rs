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
const GLYPH_LAST: u32 = 0x7E; // '~'
const GLYPH_COUNT: usize = (GLYPH_LAST - GLYPH_FIRST + 1) as usize; // 95
const HEADER_SIZE: usize = 8;
const OFFSET_TABLE_SIZE: usize = GLYPH_COUNT * 4; // 380
const GLYPH_DATA_START: usize = HEADER_SIZE + OFFSET_TABLE_SIZE; // 388

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let workspace_root = manifest_dir.parent().unwrap().to_owned();
    let inter_dir = workspace_root.join("docs/fonts/Inter/static");
    let fira_dir = workspace_root.join("docs/fonts/FiraCode/ttf");

    let inter_regular = inter_dir.join("Inter_18pt-Regular.ttf");
    let inter_medium = inter_dir.join("Inter_18pt-Medium.ttf");
    let inter_semibold = inter_dir.join("Inter_18pt-SemiBold.ttf");
    let fira_regular = fira_dir.join("FiraCode-Regular.ttf");
    let fira_medium = fira_dir.join("FiraCode-Medium.ttf");
    // A static upright Latin serif face. Keep the host lookup explicit so a
    // missing developer dependency fails loudly rather than silently dropping
    // the runtime family.
    let noto_serif = PathBuf::from("/usr/share/fonts/noto/NotoSerif-Regular.ttf");
    if !noto_serif.is_file() {
        panic!(
            "sun-font: Sun Serif source missing; searched /usr/share/fonts/noto/NotoSerif-Regular.ttf"
        );
    }
    println!(
        "cargo:warning=sun-font: Sun Serif source={}",
        noto_serif.display()
    );

    let material_dir = workspace_root.join("assets/fonts/Material-Icons");
    let material_regular = material_dir.join("MaterialIcons-Regular.ttf");

    for p in [
        &inter_regular,
        &inter_medium,
        &inter_semibold,
        &fira_regular,
        &fira_medium,
        &noto_serif,
        &material_regular,
    ] {
        println!("cargo:rerun-if-changed={}", p.display());
    }
    println!("cargo:rerun-if-changed=build.rs");

    let load = |p: &PathBuf| {
        fs::read(p).unwrap_or_else(|e| panic!("sun-font: cannot read {}: {}", p.display(), e))
    };
    let parse = |bytes: Vec<u8>, name: &str| {
        Font::from_bytes(bytes.as_slice(), FontSettings::default())
            .unwrap_or_else(|_| panic!("sun-font: failed to parse {}", name))
    };

    let f_regular = parse(load(&inter_regular), "Inter Regular");
    let f_medium = parse(load(&inter_medium), "Inter Medium");
    let f_semibold = parse(load(&inter_semibold), "Inter SemiBold");
    let f_fira_reg = parse(load(&fira_regular), "FiraCode Regular");
    let f_fira_med = parse(load(&fira_medium), "FiraCode Medium");
    let f_serif = parse(load(&noto_serif), "Noto Serif Regular");
    let f_material = parse(load(&material_regular), "Material Icons");

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    // UI proportional roles (Inter)
    generate(&f_regular, 11.0, &out_dir.join("sunlight_ui_11.mtf"));
    generate(&f_regular, 13.0, &out_dir.join("sunlight_ui_13.mtf"));
    generate(&f_medium, 13.0, &out_dir.join("sunlight_ui_medium_13.mtf"));
    generate(
        &f_semibold,
        13.0,
        &out_dir.join("sunlight_ui_semibold_13.mtf"),
    );
    generate(&f_regular, 16.0, &out_dir.join("sunlight_ui_16.mtf"));
    generate(&f_medium, 18.0, &out_dir.join("sunlight_ui_title_18.mtf"));

    // Monospace roles (Fira Code)
    generate(
        &f_fira_reg,
        14.0,
        &out_dir.join("sunlight_mono_regular_14.mtf"),
    );
    generate(
        &f_fira_med,
        14.0,
        &out_dir.join("sunlight_mono_medium_14.mtf"),
    );
    generate(
        &f_serif,
        16.0,
        &out_dir.join("sunlight_serif_regular_16.mtf"),
    );

    // Material Icons font converted to MiniType (MTF1) format.
    // Uses printable ASCII range for compatibility with current MTF loader.
    // For full icon sets use dynamic loader + richer ranges in future.
    generate(&f_material, 16.0, &out_dir.join("material_icons_16.mtf"));
    generate(&f_material, 24.0, &out_dir.join("material_icons_24.mtf"));
}

fn generate(font: &Font, px: f32, out_path: &PathBuf) {
    // Pull line metrics (may be None for some fonts; use safe defaults).
    let lm = font
        .horizontal_line_metrics(px)
        .unwrap_or_else(|| fontdue::LineMetrics {
            ascent: px * 0.8,
            descent: -(px * 0.2),
            line_gap: 0.0,
            new_line_size: px,
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
        let left = m.xmin.max(-128).min(127) as i8;
        // top = pixels above baseline to the TOP of the bitmap.
        let top_raw = m.ymin + m.height as i32;
        let top = top_raw.max(-128).min(127) as i8;

        let advance = (m.advance_width.ceil() as u32).min(255) as u8;
        let width = m.width.min(255) as u8;
        let height = m.height.min(255) as u8;

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
    out.write_all(&[line_h, ascent_px, GLYPH_COUNT as u8, 0])
        .unwrap();

    // Offset table.
    for off in &offsets {
        out.write_all(&off.to_le_bytes()).unwrap();
    }

    // Glyph data.
    for (advance, left, top, width, height, pixels) in &glyphs {
        out.write_all(&[*advance, *left as u8, *top as u8, *width, *height])
            .unwrap();
        out.write_all(pixels).unwrap();
    }
}
