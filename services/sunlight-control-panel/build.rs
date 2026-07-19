use fontdue::{Font, FontSettings};
use image::{DynamicImage, GrayImage, ImageReader};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

const ICONS: &[(&str, &str, u32)] = &[(
    "assets/icons/preferences-symbolic.png",
    "preferences-symbolic.raw",
    16,
)];

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rustc-env=SUN_MONO_IMG_CMD=sun-mono-img -o output.raw input.png");
    for (input, output, width) in ICONS {
        println!("cargo:rerun-if-changed={input}");
        let out_path = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR missing"))
            .join("icons")
            .join(output);
        convert_png_to_raw(Path::new(input), &out_path, 128, false, Some(*width)).unwrap_or_else(
            |err| panic!("sunlight-control-panel build: failed to convert {input}: {err}"),
        );
    }

    let workspace_root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("manifest dir"))
        .parent()
        .and_then(|path| path.parent())
        .map(Path::to_path_buf)
        .expect("workspace root");
    let symbols_path =
        workspace_root.join("assets/fonts/material-symbols/MaterialSymbolsOutlined.ttf");
    println!("cargo:rerun-if-changed={}", symbols_path.display());
    let font = fs::read(&symbols_path)
        .ok()
        .and_then(|bytes| Font::from_bytes(bytes, FontSettings::default()).ok());
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR missing"));
    for (name, cp) in [
        ("do_not_disturb_on", 0xe644u32),
        ("do_not_disturb_off", 0xe643u32),
        ("notifications", 0xe7f4u32),
    ] {
        let out = out_dir.join("icons").join(format!("{name}.tga"));
        if let Some(font) = font.as_ref() {
            emit_icon_tga(font, cp, 18.0, 20, &out);
        } else {
            emit_placeholder_tga(20, &out);
        }
    }

    // SunlightOS logo for About SunlightOS (scaled TGA, type-2 BGRA top-left origin).
    let logo_src = workspace_root.join("docs/images/SunlightOS-Logo.png");
    println!("cargo:rerun-if-changed={}", logo_src.display());
    let logo_out = out_dir.join("icons").join("sunlightos-logo.tga");
    convert_png_to_tga(&logo_src, &logo_out, 128).unwrap_or_else(|err| {
        panic!(
            "sunlight-control-panel build: failed to convert logo {}: {err}",
            logo_src.display()
        )
    });
}

fn emit_icon_tga(font: &Font, cp: u32, px: f32, canvas: u32, out_path: &Path) {
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
    if let Some(parent) = out_path.parent() {
        fs::create_dir_all(parent).ok();
    }
    fs::write(out_path, tga).ok();
}

fn emit_placeholder_tga(canvas: u32, out_path: &Path) {
    let mut tga: Vec<u8> = vec![0u8; 18];
    tga[2] = 2;
    tga[12] = (canvas & 0xff) as u8;
    tga[13] = (canvas >> 8) as u8;
    tga[14] = (canvas & 0xff) as u8;
    tga[15] = (canvas >> 8) as u8;
    tga[16] = 32;
    tga[17] = 0x20;
    tga.extend_from_slice(&vec![0u8; (canvas * canvas * 4) as usize]);
    if let Some(parent) = out_path.parent() {
        fs::create_dir_all(parent).ok();
    }
    fs::write(out_path, tga).ok();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        let mut path = env::temp_dir();
        path.push(format!(
            "sunlight-control-panel-{name}-{}",
            std::process::id()
        ));
        path
    }

    #[test]
    fn deterministic_bytes() {
        let input = PathBuf::from("assets/icons/preferences-symbolic.png");
        let out_a = temp_path("a.raw");
        let out_b = temp_path("b.raw");
        convert_png_to_raw(&input, &out_a, 128, false, Some(16)).unwrap();
        convert_png_to_raw(&input, &out_b, 128, false, Some(16)).unwrap();
        let a = fs::read(&out_a).unwrap();
        let b = fs::read(&out_b).unwrap();
        assert_eq!(a, b);
        let _ = fs::remove_file(out_a);
        let _ = fs::remove_file(out_b);
    }

    #[test]
    fn invalid_png_fails_cleanly() {
        let bad = temp_path("bad.png");
        let out = temp_path("bad.raw");
        fs::write(&bad, b"not-a-png").unwrap();
        let err = convert_png_to_raw(&bad, &out, 128, false, None).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("PNG") || msg.contains("png") || msg.contains("format"));
        let _ = fs::remove_file(bad);
        let _ = fs::remove_file(out);
    }
}

/// Resize a PNG and emit an uncompressed 32-bit TGA (BGRA, top-left origin).
fn convert_png_to_tga(input: &Path, output: &Path, target_w: u32) -> Result<(), ConvertError> {
    let image = load_png(input)?;
    let rgba = image.to_rgba8();
    let src_w = rgba.width().max(1);
    let src_h = rgba.height().max(1);
    let target_h = ((src_h as u64 * target_w as u64) / src_w as u64).max(1) as u32;
    let resized = image::imageops::resize(
        &rgba,
        target_w,
        target_h,
        image::imageops::FilterType::Triangle,
    );

    let mut tga: Vec<u8> = vec![0u8; 18];
    tga[2] = 2; // uncompressed true-color
    tga[12] = (target_w & 0xff) as u8;
    tga[13] = (target_w >> 8) as u8;
    tga[14] = (target_h & 0xff) as u8;
    tga[15] = (target_h >> 8) as u8;
    tga[16] = 32;
    tga[17] = 0x20; // top-left origin
    tga.reserve((target_w * target_h * 4) as usize);
    for pixel in resized.pixels() {
        let [r, g, b, a] = pixel.0;
        tga.push(b);
        tga.push(g);
        tga.push(r);
        tga.push(a);
    }
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent).map_err(ConvertError::Io)?;
    }
    fs::write(output, tga).map_err(ConvertError::Io)
}

fn convert_png_to_raw(
    input: &Path,
    output: &Path,
    threshold: u8,
    invert: bool,
    expected_width: Option<u32>,
) -> Result<(), ConvertError> {
    let image = load_png(input)?;
    let luma = image.to_luma8();
    if let Some(width) = expected_width {
        if luma.width() != width {
            return Err(ConvertError::WidthMismatch {
                expected: width,
                actual: luma.width(),
            });
        }
    }
    let raw = encode_binary_rows(&luma, threshold, invert);
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent).map_err(ConvertError::Io)?;
    }
    fs::write(output, raw).map_err(ConvertError::Io)
}

fn load_png(path: &Path) -> Result<DynamicImage, ConvertError> {
    let reader = ImageReader::open(path).map_err(ConvertError::Io)?;
    let reader = reader.with_guessed_format().map_err(ConvertError::Io)?;
    match reader.format() {
        Some(image::ImageFormat::Png) => {}
        Some(other) => return Err(ConvertError::UnsupportedFormat(other)),
        None => return Err(ConvertError::UnknownFormat),
    }
    reader.decode().map_err(ConvertError::Decode)
}

fn encode_binary_rows(image: &GrayImage, threshold: u8, invert: bool) -> Vec<u8> {
    let width = image.width() as usize;
    let height = image.height() as usize;
    let bytes_per_row = width.div_ceil(8);
    let mut out = vec![0u8; bytes_per_row * height];
    for y in 0..height {
        for x in 0..width {
            let sample = image.get_pixel(x as u32, y as u32)[0];
            let on = if invert {
                sample < threshold
            } else {
                sample >= threshold
            };
            if on {
                let index = y * bytes_per_row + x / 8;
                out[index] |= 1 << (7 - (x % 8));
            }
        }
    }
    out
}

#[derive(Debug)]
enum ConvertError {
    Io(std::io::Error),
    Decode(image::ImageError),
    UnsupportedFormat(image::ImageFormat),
    UnknownFormat,
    WidthMismatch { expected: u32, actual: u32 },
}

impl core::fmt::Display for ConvertError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Io(err) => write!(f, "{err}"),
            Self::Decode(err) => write!(f, "{err}"),
            Self::UnsupportedFormat(format) => {
                write!(f, "expected PNG input, got {:?}", format)
            }
            Self::UnknownFormat => write!(f, "could not determine input format"),
            Self::WidthMismatch { expected, actual } => {
                write!(f, "expected width {expected}, got {actual}")
            }
        }
    }
}
