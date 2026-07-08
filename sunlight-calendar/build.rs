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
        panic!(
            "sunlight-calendar/build: cannot read Material Icons TTF: {}",
            e
        );
    });
    let material_font = Font::from_bytes(material_bytes.as_slice(), FontSettings::default())
        .expect("sunlight-calendar/build: failed to parse Material Icons TTF");

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    // Navigation icons
    emit_icon_tga(
        &material_font,
        0xe5cb,
        18.0,
        24,
        &out_dir.join("icon_prev.tga"),
    ); // chevron_left
    emit_icon_tga(
        &material_font,
        0xe5cc,
        18.0,
        24,
        &out_dir.join("icon_next.tga"),
    ); // chevron_right
    emit_icon_tga(
        &material_font,
        0xe8df,
        18.0,
        24,
        &out_dir.join("icon_today.tga"),
    ); // calendar_today

    // Action icons
    emit_icon_tga(
        &material_font,
        0xe145,
        18.0,
        24,
        &out_dir.join("icon_add.tga"),
    ); // add
    emit_icon_tga(
        &material_font,
        0xe5d2,
        18.0,
        24,
        &out_dir.join("icon_menu.tga"),
    ); // menu (hamburger)
    emit_icon_tga(
        &material_font,
        0xe878,
        18.0,
        24,
        &out_dir.join("icon_event.tga"),
    ); // event
    emit_icon_tga(
        &material_font,
        0xe872,
        18.0,
        24,
        &out_dir.join("icon_delete.tga"),
    ); // delete
    emit_icon_tga(
        &material_font,
        0xe150,
        18.0,
        24,
        &out_dir.join("icon_edit.tga"),
    ); // edit
    emit_icon_tga(
        &material_font,
        0xe5d5,
        18.0,
        24,
        &out_dir.join("icon_close.tga"),
    ); // close
    emit_icon_tga(
        &material_font,
        0xe5c8,
        18.0,
        24,
        &out_dir.join("icon_forward.tga"),
    ); // arrow_forward
    emit_icon_tga(
        &material_font,
        0xe5c4,
        18.0,
        24,
        &out_dir.join("icon_back.tga"),
    ); // arrow_back
}

fn emit_icon_tga(font: &Font, cp: u32, px: f32, canvas: u32, out_path: &PathBuf) {
    let ch = char::from_u32(cp).expect("sunlight-calendar/build: invalid icon codepoint");
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
            "sunlight-calendar/build: failed to write {}: {}",
            out_path.display(),
            e
        );
    });

    println!("Generated icon U+{:04X} -> {}", cp, out_path.display());
}
