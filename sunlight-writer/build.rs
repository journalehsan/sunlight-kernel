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
            "sunlight-writer/build: cannot read Material Icons TTF: {}",
            e
        );
    });
    let material_font = Font::from_bytes(material_bytes.as_slice(), FontSettings::default())
        .expect("sunlight-writer/build: failed to parse Material Icons TTF");

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let icons = [
        ("icon_menu.tga", 0xe5d2),
        ("icon_new.tga", 0xe89c),
        ("icon_open.tga", 0xeaf3),
        ("icon_save.tga", 0xe161),
        ("icon_print.tga", 0xe8ad),
        ("icon_share.tga", 0xe80d),
        ("icon_doc.tga", 0xe873),
        ("icon_bold.tga", 0xe238),
        ("icon_italic.tga", 0xe23f),
        ("icon_underline.tga", 0xe249),
        ("icon_align_left.tga", 0xe236),
        ("icon_align_center.tga", 0xe234),
        ("icon_align_right.tga", 0xe237),
        ("icon_align_justify.tga", 0xe235),
        ("icon_bullets.tga", 0xe241),
        ("icon_numbering.tga", 0xe242),
        ("icon_picture.tga", 0xe3f4),
        ("icon_link.tga", 0xe157),
    ];

    for (name, codepoint) in icons {
        emit_icon_tga(&material_font, codepoint, 18.0, 24, &out_dir.join(name));
    }
}

fn emit_icon_tga(font: &Font, cp: u32, px: f32, canvas: u32, out_path: &PathBuf) {
    let ch = char::from_u32(cp).expect("sunlight-writer/build: invalid icon codepoint");
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
            img[didx] = 0xFF;
            img[didx + 1] = 0xFF;
            img[didx + 2] = 0xFF;
            img[didx + 3] = a;
        }
    }

    tga.extend_from_slice(&img);
    fs::write(out_path, &tga).unwrap_or_else(|e| {
        panic!(
            "sunlight-writer/build: failed to write {}: {}",
            out_path.display(),
            e
        );
    });
}
