use fontdue::{Font, FontSettings};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let workspace_root = manifest_dir.parent().unwrap();
    let font_path = workspace_root.join("assets/fonts/Material-Icons/MaterialIcons-Regular.ttf");
    println!("cargo:rerun-if-changed={}", font_path.display());
    println!("cargo:rerun-if-changed=build.rs");

    let bytes = fs::read(&font_path).expect("sunlight-devices: read Material Icons font");
    let font = Font::from_bytes(bytes.as_slice(), FontSettings::default())
        .expect("sunlight-devices: parse Material Icons font");
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());

    for (name, codepoint, size) in [
        ("app", 0xe1b1, 32.0),
        ("refresh", 0xe5d5, 18.0),
        ("display", 0xe30c, 18.0),
        ("network", 0xe894, 18.0),
        ("storage", 0xe1db, 18.0),
        ("audio", 0xe050, 18.0),
        ("keyboard", 0xe312, 18.0),
        ("mouse", 0xe323, 18.0),
        ("usb", 0xe1e0, 18.0),
        ("system", 0xe322, 18.0),
        ("bridge", 0xe335, 18.0),
        ("other", 0xe8fd, 18.0),
        ("warning", 0xe002, 18.0),
        ("section", 0xe8c4, 18.0),
    ] {
        emit_icon(
            &font,
            codepoint,
            size,
            32,
            &out.join(format!("icon_{name}.tga")),
        );
    }
}

fn emit_icon(font: &Font, codepoint: u32, px: f32, canvas: u32, path: &Path) {
    let character = char::from_u32(codepoint).expect("valid Material icon codepoint");
    let (metrics, pixels) = font.rasterize(character, px);
    let glyph_width = metrics.width as u32;
    let glyph_height = metrics.height as u32;
    let offset_x = canvas.saturating_sub(glyph_width) / 2;
    let offset_y = canvas.saturating_sub(glyph_height) / 2;
    let mut tga = vec![0u8; 18 + (canvas * canvas * 4) as usize];
    tga[2] = 2;
    tga[12] = (canvas & 0xff) as u8;
    tga[13] = (canvas >> 8) as u8;
    tga[14] = (canvas & 0xff) as u8;
    tga[15] = (canvas >> 8) as u8;
    tga[16] = 32;
    tga[17] = 0x20;

    for y in 0..glyph_height {
        for x in 0..glyph_width {
            let alpha = pixels[(y * glyph_width + x) as usize];
            let destination = 18 + (((offset_y + y) * canvas + offset_x + x) * 4) as usize;
            tga[destination] = 0xff;
            tga[destination + 1] = 0xff;
            tga[destination + 2] = 0xff;
            tga[destination + 3] = alpha;
        }
    }
    fs::write(path, tga).expect("sunlight-devices: write generated icon");
}
