//! Build script for sunlight-vortex-shell.
//!
//! Rasterises selected glyphs from the bundled Material Symbols font
//! into small centered TGA icons (white+alpha) for use in top panel
//! and dock controls. This lets us use real font glyphs without
//! shipping a TTF parser or doing runtime rasterisation.
//!
//! Generated files are placed in OUT_DIR and included via include_bytes!.

use fontdue::{Font, FontSettings};
use std::env;
use std::fs;
use std::path::PathBuf;

/// Emit a 24x24 TGA (32bpp top-down BGRA white+alpha) for a glyph codepoint.
fn emit_icon_tga(font: &Font, cp: u32, px: f32, canvas: u32, out_path: &PathBuf) {
    let ch = match char::from_u32(cp) {
        Some(c) => c,
        None => {
            // Write a 1x1 transparent placeholder on bad cp
            let mut tga: Vec<u8> = vec![0u8; 18];
            tga[2] = 2;
            tga[12] = 1;
            tga[13] = 0;
            tga[14] = 1;
            tga[15] = 0;
            tga[16] = 32;
            tga[17] = 0x20;
            tga.extend_from_slice(&[0u8; 4]);
            fs::write(out_path, &tga).ok();
            return;
        }
    };
    let (metrics, pixels) = font.rasterize(ch, px);

    let gw = metrics.width as u32;
    let gh = metrics.height as u32;

    let dst_x = (canvas.saturating_sub(gw)) / 2;
    let dst_y = (canvas.saturating_sub(gh)) / 2;

    let mut tga: Vec<u8> = vec![0u8; 18];
    tga[2] = 2;
    tga[12] = (canvas & 0xff) as u8;
    tga[13] = (canvas >> 8) as u8;
    tga[14] = (canvas & 0xff) as u8;
    tga[15] = (canvas >> 8) as u8;
    tga[16] = 32;
    tga[17] = 0x20;

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
            img[didx + 0] = 0xFF;
            img[didx + 1] = 0xFF;
            img[didx + 2] = 0xFF;
            img[didx + 3] = a;
        }
    }
    tga.extend_from_slice(&img);
    fs::write(out_path, &tga).ok();
}

/// Emit a minimal valid  transparent square TGA so include_bytes always succeeds.
fn emit_placeholder_tga(canvas: u32, out_path: &PathBuf) {
    let mut tga: Vec<u8> = vec![0u8; 18];
    tga[2] = 2;
    tga[12] = (canvas & 0xff) as u8;
    tga[13] = (canvas >> 8) as u8;
    tga[14] = (canvas & 0xff) as u8;
    tga[15] = (canvas >> 8) as u8;
    tga[16] = 32;
    tga[17] = 0x20;
    // All transparent pixels (A=0)
    let img = vec![0u8; (canvas * canvas * 4) as usize];
    tga.extend_from_slice(&img);
    fs::write(out_path, &tga).ok();
}

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let workspace_root = manifest_dir.parent().unwrap().parent().unwrap().to_owned();
    let symbols_path =
        workspace_root.join("assets/fonts/material-symbols/MaterialSymbolsOutlined.ttf");

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={}", symbols_path.display());

    let font = if let Ok(bytes) = fs::read(&symbols_path) {
        Font::from_bytes(bytes.as_slice(), FontSettings::default()).ok()
    } else {
        None
    };

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    // Chosen codepoints for Material Symbols Outlined (approximate; adjust if a glyph is blank).
    // Many overlap with legacy Material Icons range.
    // 24px raster target, 20px canvas for panel (small controls).
    const CANVAS: u32 = 20;
    const PX: f32 = 18.0;

    // Map names -> cps (these are common values used by the font family)
    let icons: &[(&str, u32)] = &[
        ("home", 0xe88a),
        ("search", 0xe8b6),
        ("terminal", 0xe8d6),
        ("folder", 0xe2c7),
        ("calendar_month", 0xebcc),
        ("notifications", 0xe7f4),
        ("logout", 0xe9ba),
        ("lan", 0xe875),
        ("menu", 0xe5d2),
        ("settings", 0xe8b8),
        ("edit", 0xe3c9),
        ("calculate", 0xeaf0),
        ("public", 0xe80b), // web / public
        ("code", 0xe86f),
        ("article", 0xe8e2), // for office-ish
        ("sunny", 0xe430),   // sun / light for logo
        ("close", 0xe5cd),
        ("do_not_disturb_on", 0xe644),
        ("do_not_disturb_off", 0xe643),
    ];

    for (name, cp) in icons {
        let out = out_dir.join(format!("icon_{}.tga", name));
        if let Some(ref f) = font {
            emit_icon_tga(f, *cp, PX, CANVAS, &out);
        } else {
            emit_placeholder_tga(CANVAS, &out);
        }
    }

    // Also emit a couple larger for dock/start if wanted (24px canvas)
    let big: &[(&str, u32)] = &[
        ("start_menu", 0xe5d2), // menu
    ];
    for (name, cp) in big {
        let out = out_dir.join(format!("icon_{}.tga", name));
        if let Some(ref f) = font {
            emit_icon_tga(f, *cp, 20.0, 24, &out);
        } else {
            emit_placeholder_tga(24, &out);
        }
    }

    println!("vortex-shell: material-symbols icons rastered (if font present)");
}
