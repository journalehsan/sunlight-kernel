use fontdue::{Font, FontSettings};
use std::env;
use std::fs;
use std::path::PathBuf;

fn emit_icon_tga(font: &Font, cp: u32, px: f32, canvas: u32, out_path: &PathBuf) {
    let Some(ch) = char::from_u32(cp) else {
        emit_placeholder_tga(canvas, out_path);
        return;
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
            img[didx] = 0xFF;
            img[didx + 1] = 0xFF;
            img[didx + 2] = 0xFF;
            img[didx + 3] = a;
        }
    }
    tga.extend_from_slice(&img);
    fs::write(out_path, &tga).ok();
}

fn emit_placeholder_tga(canvas: u32, out_path: &PathBuf) {
    let mut tga: Vec<u8> = vec![0u8; 18];
    tga[2] = 2;
    tga[12] = (canvas & 0xff) as u8;
    tga[13] = (canvas >> 8) as u8;
    tga[14] = (canvas & 0xff) as u8;
    tga[15] = (canvas >> 8) as u8;
    tga[16] = 32;
    tga[17] = 0x20;
    tga.extend_from_slice(&vec![0u8; (canvas * canvas * 4) as usize]);
    fs::write(out_path, &tga).ok();
}

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let workspace_root = manifest_dir.parent().unwrap().parent().unwrap().to_owned();
    let symbols_path =
        workspace_root.join("assets/fonts/material-symbols/MaterialSymbolsOutlined.ttf");

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={}", symbols_path.display());

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let out = out_dir.join("icon_close.tga");
    let font = fs::read(&symbols_path)
        .ok()
        .and_then(|bytes| Font::from_bytes(bytes, FontSettings::default()).ok());
    if let Some(font) = font {
        emit_icon_tga(&font, 0xe5cd, 18.0, 20, &out);
    } else {
        emit_placeholder_tga(20, &out);
    }
}
