//! Build script for sunlight-edit: rasterize Material Icons glyphs into small
//! TGA (white + alpha) assets so we can draw them without shipping large
//! checked-in TGAs from docs/icons. This reduces binary size / RAM and gives
//! consistent Material look via the same icon font used elsewhere.

use fontdue::{Font, FontSettings};
use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let workspace_root = manifest_dir.parent().unwrap().to_owned();
    let material_path =
        workspace_root.join("assets/fonts/Material-Icons/MaterialIcons-Regular.ttf");

    println!("cargo:rerun-if-changed={}", material_path.display());
    println!("cargo:rerun-if-changed=build.rs");

    let material_bytes = fs::read(&material_path).unwrap_or_else(|e| {
        panic!("sunlight-edit/build: cannot read Material Icons TTF: {}", e);
    });
    let material_font = Font::from_bytes(material_bytes.as_slice(), FontSettings::default())
        .expect("sunlight-edit/build: failed to parse Material Icons TTF");

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    // Toolbar / action icons at ~16-18px rendered into 24px square canvases.
    // Using slightly larger source raster then scaling gives better quality.
    emit_icon_tga(
        &material_font,
        0xe89c,
        18.0,
        24,
        &out_dir.join("icon_new.tga"),
    ); // note_add
    emit_icon_tga(
        &material_font,
        0xeaf3,
        18.0,
        24,
        &out_dir.join("icon_open.tga"),
    ); // file_open
    emit_icon_tga(
        &material_font,
        0xe161,
        18.0,
        24,
        &out_dir.join("icon_save.tga"),
    ); // save
    emit_icon_tga(
        &material_font,
        0xe171,
        18.0,
        24,
        &out_dir.join("icon_save_as.tga"),
    ); // save_as
    emit_icon_tga(
        &material_font,
        0xe8b6,
        18.0,
        24,
        &out_dir.join("icon_find.tga"),
    ); // search
    emit_icon_tga(
        &material_font,
        0xe881,
        18.0,
        24,
        &out_dir.join("icon_replace.tga"),
    ); // find_replace
    emit_icon_tga(
        &material_font,
        0xe14e,
        18.0,
        24,
        &out_dir.join("icon_cut.tga"),
    ); // content_cut
    emit_icon_tga(
        &material_font,
        0xe14d,
        18.0,
        24,
        &out_dir.join("icon_copy.tga"),
    ); // content_copy
    emit_icon_tga(
        &material_font,
        0xe14f,
        18.0,
        24,
        &out_dir.join("icon_paste.tga"),
    ); // content_paste
    emit_icon_tga(
        &material_font,
        0xe162,
        18.0,
        24,
        &out_dir.join("icon_select_all.tga"),
    ); // select_all
    emit_icon_tga(
        &material_font,
        0xe5c8,
        18.0,
        24,
        &out_dir.join("icon_next.tga"),
    ); // arrow_forward (find next)
    emit_icon_tga(
        &material_font,
        0xe5c4,
        18.0,
        24,
        &out_dir.join("icon_prev.tga"),
    ); // arrow_back   (find prev)

    // Hamburger / menu icon for the toolbar "Menu" button (replaces text label).
    emit_icon_tga(
        &material_font,
        0xe5d2,
        18.0,
        24,
        &out_dir.join("icon_menu.tga"),
    ); // menu (hamburger)
}

/// Rasterize one Material Icon codepoint into a square TGA (type 2, 32bpp top-down)
/// with white RGB and coverage in alpha. The glyph is centered.
fn emit_icon_tga(font: &Font, cp: u32, px: f32, canvas: u32, out_path: &PathBuf) {
    let ch = char::from_u32(cp).expect("sunlight-edit/build: invalid icon codepoint");
    let (metrics, pixels) = font.rasterize(ch, px);

    let gw = metrics.width as u32;
    let gh = metrics.height as u32;

    let dst_x = canvas.saturating_sub(gw) / 2;
    let dst_y = canvas.saturating_sub(gh) / 2;

    // TGA header (18 bytes), type 2, 32 bpp, top-down.
    let mut tga: Vec<u8> = vec![0u8; 18];
    tga[2] = 2;
    tga[12] = (canvas & 0xff) as u8;
    tga[13] = (canvas >> 8) as u8;
    tga[14] = (canvas & 0xff) as u8;
    tga[15] = (canvas >> 8) as u8;
    tga[16] = 32;
    tga[17] = 0x20; // top-down

    let mut img = vec![0u8; (canvas * canvas * 4) as usize];
    for gy in 0..gh {
        for gx in 0..gw {
            let a = pixels[(gy * gw + gx) as usize];
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

    fs::write(out_path, &tga).unwrap_or_else(|e| {
        panic!(
            "sunlight-edit/build: failed to write {}: {}",
            out_path.display(),
            e
        );
    });

    println!("Generated icon U+{:04X} -> {}", cp, out_path.display());
}
