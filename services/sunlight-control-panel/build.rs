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
