//! Build script for sunlight-light-lens: emit Material Icons based app icon and
//! missing-image placeholder as TGA. Replaces large checked-in TGAs with
//! glyphs from the Material Icons font (via minitype-style build rasterization)
//! for RAM / consistency wins. Also used for monochrome action icon sources.

use fontdue::{Font, FontSettings};
use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let workspace_root = manifest_dir.parent().unwrap().to_owned();
    let material_path = workspace_root.join("assets/fonts/Material-Icons/MaterialIcons-Regular.ttf");

    println!("cargo:rerun-if-changed={}", material_path.display());
    println!("cargo:rerun-if-changed=build.rs");

    let material_bytes = fs::read(&material_path).unwrap_or_else(|e| {
        panic!("sunlight-light-lens/build: cannot read Material Icons: {}", e);
    });
    let font = Font::from_bytes(material_bytes.as_slice(), FontSettings::default())
        .expect("sunlight-light-lens/build: parse Material Icons failed");

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    // App icon (titlebar / empty state). 32px canvas good for 22-72 usage sites.
    emit_icon_tga(&font, 0xe410, 26.0, 32, &out_dir.join("icon_app.tga")); // photo

    // Missing / error image placeholder.
    emit_icon_tga(&font, 0xe3ad, 26.0, 48, &out_dir.join("icon_missing.tga")); // broken_image (larger canvas for 64px use)
}

fn emit_icon_tga(font: &Font, cp: u32, px: f32, canvas: u32, out_path: &PathBuf) {
    let ch = char::from_u32(cp).expect("invalid cp");
    let (metrics, pixels) = font.rasterize(ch, px);

    let gw = metrics.width as u32;
    let gh = metrics.height as u32;

    let dst_x = canvas.saturating_sub(gw) / 2;
    let dst_y = canvas.saturating_sub(gh) / 2;

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
            let a = pixels[(gy * gw + gx) as usize];
            if a == 0 { continue; }
            let dx = dst_x + gx;
            let dy = dst_y + gy;
            if dx >= canvas || dy >= canvas { continue; }
            let didx = ((dy * canvas + dx) * 4) as usize;
            img[didx + 0] = 0xFF;
            img[didx + 1] = 0xFF;
            img[didx + 2] = 0xFF;
            img[didx + 3] = a;
        }
    }
    tga.extend_from_slice(&img);

    fs::write(out_path, &tga).unwrap_or_else(|e| panic!("write {}: {}", out_path.display(), e));
    println!("light-lens icon U+{:04X} -> {}", cp, out_path.display());
}
