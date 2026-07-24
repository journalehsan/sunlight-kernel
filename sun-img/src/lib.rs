//! SunlightOS image conversion foundation (TGA + SIMG v2).
//!
//! Host tooling uses the default `std` feature. Freestanding UI crates depend
//! with `default-features = false` (`no_std` + `alloc`).

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::fmt;

pub mod crc32;
pub mod simg_v2;

pub use simg_v2::{
    decode as decode_simg_v2, decode_argb_u32 as decode_simg_v2_argb_u32,
    decode_bgra as decode_simg_v2_bgra, encode as encode_simg_v2,
    encode_with_method as encode_simg_v2_with, is_simg_v2, method_name as simg_v2_method_name,
    parse_header as parse_simg_v2_header, Compression as SimgV2Compression,
    EncodeReport as SimgV2EncodeReport, Filter as SimgV2Filter, SimgV2Header,
    ENCODER_VERSION as SIMG_V2_ENCODER_VERSION, HEADER_SIZE as SIMG_V2_HEADER_SIZE,
    MAGIC as SIMG_V2_MAGIC, MAX_DECODED_BYTES, MAX_DIMENSION, VERSION as SIMG_V2_VERSION,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageRgba8 {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageFormat {
    Tga,
    SimgV2,
    Bmp,
    Png,
    Jpeg,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputFormat {
    TgaRgba32,
    SimgV2,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConvertOptions {
    pub output_format: OutputFormat,
    pub force_alpha: bool,
    pub flip_vertical: bool,
}

impl Default for ConvertOptions {
    fn default() -> Self {
        Self {
            output_format: OutputFormat::TgaRgba32,
            force_alpha: true,
            flip_vertical: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageError {
    UnsupportedFormat,
    InvalidHeader,
    InvalidDimensions,
    TruncatedInput,
    UnsupportedBitDepth,
    UnsupportedCompression,
    DecodeFailed(String),
    EncodeFailed(String),
    Io(String),
}

impl fmt::Display for ImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedFormat => write!(f, "unsupported image format"),
            Self::InvalidHeader => write!(f, "invalid image header"),
            Self::InvalidDimensions => write!(f, "invalid image dimensions"),
            Self::TruncatedInput => write!(f, "truncated image input"),
            Self::UnsupportedBitDepth => write!(f, "unsupported bit depth"),
            Self::UnsupportedCompression => write!(f, "unsupported compression"),
            Self::DecodeFailed(msg) => write!(f, "decode failed: {msg}"),
            Self::EncodeFailed(msg) => write!(f, "encode failed: {msg}"),
            Self::Io(msg) => write!(f, "io error: {msg}"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for ImageError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TgaOrigin {
    TopLeft,
    BottomLeft,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TgaInfo {
    pub width: u32,
    pub height: u32,
    pub bit_depth: u8,
    pub origin: TgaOrigin,
    pub supported: bool,
}

pub fn detect_format(bytes: &[u8]) -> ImageFormat {
    if simg_v2::is_simg_v2(bytes) {
        return ImageFormat::SimgV2;
    }
    if is_png(bytes) {
        return ImageFormat::Png;
    }
    if is_jpeg(bytes) {
        return ImageFormat::Jpeg;
    }
    if is_bmp(bytes) {
        return ImageFormat::Bmp;
    }
    if inspect_tga(bytes).is_ok() {
        return ImageFormat::Tga;
    }
    ImageFormat::Unknown
}

/// Decode TGA or SIMG v2 into host RGBA8 (straight alpha).
pub fn decode_image(bytes: &[u8]) -> Result<ImageRgba8, ImageError> {
    if simg_v2::is_simg_v2(bytes) {
        // Strict v2 path — never fall back to TGA after magic match.
        return simg_v2::decode(bytes);
    }
    if bytes.len() < 18 {
        return Err(ImageError::TruncatedInput);
    }
    match detect_format(bytes) {
        ImageFormat::Tga => decode_tga(bytes),
        ImageFormat::SimgV2 => simg_v2::decode(bytes),
        ImageFormat::Bmp | ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::Unknown => {
            Err(ImageError::UnsupportedFormat)
        }
    }
}

pub fn encode_tga_rgba32(image: &ImageRgba8) -> Result<Vec<u8>, ImageError> {
    validate_rgba_image(image)?;
    let expected_len = rgba_len(image.width, image.height)?;
    let mut out = Vec::with_capacity(18 + expected_len);
    out.extend_from_slice(&[
        0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ]);
    out.extend_from_slice(
        &(u16::try_from(image.width).map_err(|_| ImageError::InvalidDimensions)?).to_le_bytes(),
    );
    out.extend_from_slice(
        &(u16::try_from(image.height).map_err(|_| ImageError::InvalidDimensions)?).to_le_bytes(),
    );
    out.push(32);
    out.push(0x28);

    for chunk in image.pixels.chunks_exact(4) {
        let r = chunk[0];
        let g = chunk[1];
        let b = chunk[2];
        let a = chunk[3];
        out.push(b);
        out.push(g);
        out.push(r);
        out.push(a);
    }

    Ok(out)
}

pub fn convert_to_tga_rgba32(input: &[u8]) -> Result<Vec<u8>, ImageError> {
    convert_with_options(input, &ConvertOptions::default())
}

pub fn convert_with_options(input: &[u8], options: &ConvertOptions) -> Result<Vec<u8>, ImageError> {
    let mut image = decode_image(input)?;
    if options.force_alpha {
        force_alpha_opaque(&mut image)?;
    }
    if options.flip_vertical {
        flip_vertical(&mut image)?;
    }
    match options.output_format {
        OutputFormat::TgaRgba32 => encode_tga_rgba32(&image),
        OutputFormat::SimgV2 => Ok(simg_v2::encode(&image)?.bytes),
    }
}

/// Encode input image bytes (TGA/SIMG) to SIMG v2 without force-opaque alpha.
pub fn convert_to_simg_v2(input: &[u8]) -> Result<simg_v2::EncodeReport, ImageError> {
    let image = decode_image(input)?;
    simg_v2::encode(&image)
}

pub fn inspect_tga(bytes: &[u8]) -> Result<TgaInfo, ImageError> {
    if bytes.len() < 18 {
        return Err(ImageError::TruncatedInput);
    }
    // Do not accept SIMG v2 as TGA.
    if simg_v2::is_simg_v2(bytes) {
        return Err(ImageError::UnsupportedFormat);
    }
    let color_map_type = bytes[1];
    let image_type = bytes[2];
    let width = u16::from_le_bytes([bytes[12], bytes[13]]) as u32;
    let height = u16::from_le_bytes([bytes[14], bytes[15]]) as u32;
    let bit_depth = bytes[16];
    let descriptor = bytes[17];
    let origin = if descriptor & 0x20 != 0 {
        TgaOrigin::TopLeft
    } else {
        TgaOrigin::BottomLeft
    };
    if width == 0 || height == 0 {
        return Err(ImageError::InvalidDimensions);
    }
    let supported = color_map_type == 0 && image_type == 2 && matches!(bit_depth, 24 | 32);
    Ok(TgaInfo {
        width,
        height,
        bit_depth,
        origin,
        supported,
    })
}

fn decode_tga(bytes: &[u8]) -> Result<ImageRgba8, ImageError> {
    let info = inspect_tga(bytes)?;
    if bytes[1] != 0 {
        return Err(ImageError::UnsupportedFormat);
    }
    match bytes[2] {
        2 => {}
        9..=11 => return Err(ImageError::UnsupportedCompression),
        _ => return Err(ImageError::UnsupportedFormat),
    }
    if !matches!(info.bit_depth, 24 | 32) {
        return Err(ImageError::UnsupportedBitDepth);
    }

    let id_len = bytes[0] as usize;
    let color_map_len = u16::from_le_bytes([bytes[5], bytes[6]]) as usize;
    let color_map_entry_bits = bytes[7] as usize;
    let color_map_bytes = if bytes[1] != 0 {
        color_map_len * color_map_entry_bits.div_ceil(8)
    } else {
        0
    };
    let data_offset = 18usize
        .checked_add(id_len)
        .and_then(|v| v.checked_add(color_map_bytes))
        .ok_or(ImageError::InvalidHeader)?;

    let pixel_size = (info.bit_depth / 8) as usize;
    let pixel_count = pixel_count(info.width, info.height)?;
    let bytes_needed = pixel_count
        .checked_mul(pixel_size)
        .and_then(|n| data_offset.checked_add(n))
        .ok_or(ImageError::InvalidDimensions)?;
    if bytes.len() < bytes_needed {
        return Err(ImageError::TruncatedInput);
    }

    let mut pixels = vec![0u8; rgba_len(info.width, info.height)?];
    for y in 0..info.height {
        let src_row = match info.origin {
            TgaOrigin::TopLeft => y,
            TgaOrigin::BottomLeft => info.height - 1 - y,
        };
        let row_base = data_offset + src_row as usize * info.width as usize * pixel_size;
        for x in 0..info.width {
            let src = row_base + x as usize * pixel_size;
            let dst = ((y * info.width + x) * 4) as usize;
            let b = bytes[src];
            let g = bytes[src + 1];
            let r = bytes[src + 2];
            let a = if pixel_size == 4 {
                bytes[src + 3]
            } else {
                0xFF
            };
            pixels[dst] = r;
            pixels[dst + 1] = g;
            pixels[dst + 2] = b;
            pixels[dst + 3] = a;
        }
    }

    Ok(ImageRgba8 {
        width: info.width,
        height: info.height,
        pixels,
    })
}

fn validate_rgba_image(image: &ImageRgba8) -> Result<(), ImageError> {
    if image.width == 0 || image.height == 0 {
        return Err(ImageError::InvalidDimensions);
    }
    let expected = rgba_len(image.width, image.height)?;
    if image.pixels.len() != expected {
        return Err(ImageError::InvalidDimensions);
    }
    Ok(())
}

fn force_alpha_opaque(image: &mut ImageRgba8) -> Result<(), ImageError> {
    validate_rgba_image(image)?;
    for px in image.pixels.chunks_exact_mut(4) {
        px[3] = 0xFF;
    }
    Ok(())
}

fn flip_vertical(image: &mut ImageRgba8) -> Result<(), ImageError> {
    validate_rgba_image(image)?;
    let row_len = image.width as usize * 4;
    for y in 0..(image.height as usize / 2) {
        let top = y * row_len;
        let bottom = (image.height as usize - 1 - y) * row_len;
        for i in 0..row_len {
            image.pixels.swap(top + i, bottom + i);
        }
    }
    Ok(())
}

fn pixel_count(width: u32, height: u32) -> Result<usize, ImageError> {
    if width == 0 || height == 0 {
        return Err(ImageError::InvalidDimensions);
    }
    width
        .checked_mul(height)
        .and_then(|n| usize::try_from(n).ok())
        .ok_or(ImageError::InvalidDimensions)
}

pub(crate) fn rgba_len(width: u32, height: u32) -> Result<usize, ImageError> {
    pixel_count(width, height)?
        .checked_mul(4)
        .ok_or(ImageError::InvalidDimensions)
}

fn is_png(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A])
}

fn is_jpeg(bytes: &[u8]) -> bool {
    bytes.len() >= 3 && bytes[0] == 0xFF && bytes[1] == 0xD8 && bytes[2] == 0xFF
}

fn is_bmp(bytes: &[u8]) -> bool {
    bytes.starts_with(b"BM")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny_tga24_top_left() -> Vec<u8> {
        let mut out = vec![
            0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes());
        out.push(24);
        out.push(0x20);
        let pixels = [
            [0x00, 0x00, 0xFF],
            [0x00, 0xFF, 0x00],
            [0xFF, 0x00, 0x00],
            [0xFF, 0xFF, 0xFF],
        ];
        for px in pixels {
            out.extend_from_slice(&px);
        }
        out
    }

    fn tiny_tga32_top_left() -> Vec<u8> {
        let mut out = vec![
            0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.push(32);
        out.push(0x28);
        out.extend_from_slice(&[0x00, 0x00, 0xFF, 0x80]);
        out.extend_from_slice(&[0x00, 0xFF, 0x00, 0x40]);
        out
    }

    #[test]
    fn detects_formats() {
        assert_eq!(detect_format(&tiny_tga24_top_left()), ImageFormat::Tga);
        assert_eq!(
            detect_format(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]),
            ImageFormat::Png
        );
        assert_eq!(detect_format(&[b'B', b'M', 0, 0]), ImageFormat::Bmp);
        assert_eq!(detect_format(&[0xFF, 0xD8, 0xFF, 0xE0]), ImageFormat::Jpeg);
        let v2 = encode_simg_v2(&ImageRgba8 {
            width: 1,
            height: 1,
            pixels: vec![1, 2, 3, 4],
        })
        .unwrap();
        assert_eq!(detect_format(&v2.bytes), ImageFormat::SimgV2);
    }

    #[test]
    fn rejects_empty_and_truncated() {
        assert_eq!(inspect_tga(&[]), Err(ImageError::TruncatedInput));
        let mut tiny = tiny_tga24_top_left();
        tiny.truncate(10);
        assert_eq!(decode_image(&tiny), Err(ImageError::TruncatedInput));
    }

    #[test]
    fn decodes_tiny_24bit_tga() {
        let image = decode_image(&tiny_tga24_top_left()).unwrap();
        assert_eq!(image.width, 2);
        assert_eq!(image.height, 2);
        assert_eq!(&image.pixels[0..4], &[0xFF, 0x00, 0x00, 0xFF]);
        assert_eq!(&image.pixels[4..8], &[0x00, 0xFF, 0x00, 0xFF]);
        assert_eq!(&image.pixels[8..12], &[0x00, 0x00, 0xFF, 0xFF]);
    }

    #[test]
    fn decodes_tiny_32bit_tga_with_alpha() {
        let image = decode_image(&tiny_tga32_top_left()).unwrap();
        assert_eq!(image.width, 2);
        assert_eq!(image.height, 1);
        assert_eq!(&image.pixels[0..4], &[0xFF, 0x00, 0x00, 0x80]);
        assert_eq!(&image.pixels[4..8], &[0x00, 0xFF, 0x00, 0x40]);
    }

    #[test]
    fn handles_bottom_left_origin() {
        let mut bytes = tiny_tga24_top_left();
        bytes[17] = 0x00;
        let image = decode_image(&bytes).unwrap();
        assert_eq!(&image.pixels[0..4], &[0x00, 0x00, 0xFF, 0xFF]);
        assert_eq!(&image.pixels[8..12], &[0xFF, 0x00, 0x00, 0xFF]);
    }

    #[test]
    fn encodes_valid_tga_2x2() {
        let image = ImageRgba8 {
            width: 2,
            height: 2,
            pixels: vec![
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 128,
            ],
        };
        let bytes = encode_tga_rgba32(&image).unwrap();
        assert_eq!(bytes[2], 2);
        assert_eq!(bytes[16], 32);
        assert_eq!(bytes[17], 0x28);
        assert_eq!(bytes.len(), 18 + 16);
    }

    #[test]
    fn round_trips_rgba_2x2() {
        let image = ImageRgba8 {
            width: 2,
            height: 2,
            pixels: vec![
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 128,
            ],
        };
        let bytes = encode_tga_rgba32(&image).unwrap();
        let decoded = decode_image(&bytes).unwrap();
        assert_eq!(decoded, image);
    }

    #[test]
    fn rejects_rle_cleanly() {
        let mut bytes = tiny_tga24_top_left();
        bytes[2] = 10;
        assert_eq!(
            decode_image(&bytes),
            Err(ImageError::UnsupportedCompression)
        );
    }

    #[test]
    fn legacy_tga_matches_simg_v2_canonical_pixels() {
        let tga = tiny_tga32_top_left();
        let legacy = decode_image(&tga).unwrap();
        let v2 = encode_simg_v2(&legacy).unwrap();
        let from_v2 = decode_image(&v2.bytes).unwrap();
        assert_eq!(legacy, from_v2);
    }

    #[test]
    fn malformed_v2_does_not_fallback_to_tga() {
        let mut bytes = encode_simg_v2(&ImageRgba8 {
            width: 1,
            height: 1,
            pixels: vec![1, 2, 3, 4],
        })
        .unwrap()
        .bytes;
        // Corrupt version after magic.
        bytes[4] = 9;
        assert!(matches!(
            decode_image(&bytes),
            Err(ImageError::UnsupportedFormat)
        ));
    }
}
