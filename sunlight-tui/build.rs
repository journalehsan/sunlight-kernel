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

    let material_path =
        workspace_root.join("assets/fonts/Material-Icons/MaterialIcons-Regular.ttf");

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={}", inter_path.display());
    println!("cargo:rerun-if-changed={}", fira_regular.display());
    println!("cargo:rerun-if-changed={}", fira_semibold.display());
    println!("cargo:rerun-if-changed={}", material_path.display());

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

    let material_bytes = fs::read(&material_path).unwrap_or_else(|e| {
        panic!(
            "sunlight-tui build: cannot read {}: {}",
            material_path.display(),
            e
        )
    });
    let material_font = Font::from_bytes(material_bytes.as_slice(), FontSettings::default())
        .expect("sunlight-tui build: failed to parse Material Icons");

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

    // Material Icons (replacing the old TGA action icons for login buttons/avatars)
    // Chosen glyphs that render nicely at ~28px inside the 32px slots.
    emit_icon_tga(
        &material_font,
        0xe853,
        28.0,
        32,
        &out_dir.join("icon_users.tga"),
    ); // account_circle
    emit_icon_tga(
        &material_font,
        0xe053,
        28.0,
        32,
        &out_dir.join("icon_reboot.tga"),
    ); // restart_alt
    emit_icon_tga(
        &material_font,
        0xe8ac,
        28.0,
        32,
        &out_dir.join("icon_shutdown.tga"),
    ); // power_settings_new
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

/// Rasterize a Material Icons glyph and emit a minimal 32-bpp top-down TGA
/// (with white RGB + alpha from coverage) centered in a `canvas` x `canvas` image.
/// This lets the existing TGA loader + tinted drawer work for action icons.
fn emit_icon_tga(font: &Font, cp: u32, px: f32, canvas: u32, out_path: &PathBuf) {
    let ch = char::from_u32(cp).expect("invalid codepoint for icon");
    let (metrics, pixels) = font.rasterize(ch, px);

    let gw = metrics.width as u32;
    let gh = metrics.height as u32;

    // Center the glyph inside the canvas.
    let dst_x = (canvas.saturating_sub(gw)) / 2;
    let dst_y = (canvas.saturating_sub(gh)) / 2;

    // TGA header for type 2 (uncompressed truecolor), 32bpp, top-down.
    // 18 bytes header.
    let mut tga: Vec<u8> = vec![0u8; 18];
    tga[2] = 2; // image type = uncompressed true-color
                // width/height little endian at 12/14
    tga[12] = (canvas & 0xff) as u8;
    tga[13] = (canvas >> 8) as u8;
    tga[14] = (canvas & 0xff) as u8;
    tga[15] = (canvas >> 8) as u8;
    tga[16] = 32; // bpp
    tga[17] = 0x20; // top-down, no alpha attr bits needed for our parser

    // Image data: top-down BGRA (B,G,R,A)
    let mut img = vec![0u8; (canvas * canvas * 4) as usize];
    for gy in 0..gh {
        for gx in 0..gw {
            let src_idx = (gy * gw + gx) as usize;
            let a = pixels[src_idx];
            if a == 0 {
                continue;
            }
            let dx = dst_x + gx;
            let dy = dst_y + gy;
            if dx >= canvas || dy >= canvas {
                continue;
            }
            let didx = ((dy * canvas + dx) * 4) as usize;
            // White shape + coverage in alpha. Tinted drawer will use alpha.
            img[didx + 0] = 0xFF; // B
            img[didx + 1] = 0xFF; // G
            img[didx + 2] = 0xFF; // R
            img[didx + 3] = a; // A
        }
    }

    tga.extend_from_slice(&img);

    fs::write(out_path, &tga).unwrap_or_else(|e| {
        panic!(
            "sunlight-tui build: cannot write icon tga {}: {}",
            out_path.display(),
            e
        )
    });

    println!(
        "Generated icon U+{:04X} -> {} ({}x{} canvas)",
        cp,
        out_path.display(),
        canvas,
        canvas
    );
}
