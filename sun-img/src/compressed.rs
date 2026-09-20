//! Compressed interchange formats, decoded without std or platform SIMD.
use alloc::{format, vec, vec::Vec};
use zune_core::{colorspace::ColorSpace, options::DecoderOptions};

use crate::{ImageError, ImageRgba8, MAX_DECODED_BYTES, MAX_DIMENSION};

fn options() -> DecoderOptions {
    DecoderOptions::default()
        .set_max_width(MAX_DIMENSION as usize)
        .set_max_height(MAX_DIMENSION as usize)
        .set_strict_mode(true)
        .set_use_unsafe(false)
}

fn checked_len(width: u32, height: u32) -> Result<usize, ImageError> {
    if width > MAX_DIMENSION || height > MAX_DIMENSION {
        return Err(ImageError::InvalidDimensions);
    }
    let len = crate::rgba_len(width, height)?;
    if len > MAX_DECODED_BYTES as usize {
        return Err(ImageError::InvalidDimensions);
    }
    Ok(len)
}

fn check_input(bytes: &[u8]) -> Result<(), ImageError> {
    if bytes.len() > MAX_DECODED_BYTES as usize {
        return Err(ImageError::DecodeFailed(format!(
            "encoded image exceeds 64 MiB"
        )));
    }
    Ok(())
}

fn decode_error(error: impl core::fmt::Debug) -> ImageError {
    ImageError::DecodeFailed(format!("{error:?}"))
}

/// Decode a static PNG (including palette, transparency, 16-bit and Adam7).
/// 16-bit channels are reduced to their high eight bits. APNG is rejected.
pub fn decode_png(bytes: &[u8]) -> Result<ImageRgba8, ImageError> {
    check_input(bytes)?;
    if bytes.len() < 33 {
        return Err(ImageError::TruncatedInput);
    }
    if !crate::is_png(bytes) || bytes[8..16] != *b"\0\0\0\rIHDR" {
        return Err(ImageError::InvalidHeader);
    }
    // Bound output before the decoder gathers IDAT/metadata chunks.
    let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
    let len = checked_len(width, height)?;
    // Expand native channels ourselves; automatic alpha insertion mishandles
    // packed grayscale in the 0.4 PNG decoder.
    let opts = options()
        .png_set_confirm_crc(true)
        .png_set_strip_to_8bit(true);
    let mut decoder = zune_png::PngDecoder::new_with_options(bytes, opts);
    decoder.decode_headers().map_err(decode_error)?;
    if decoder.is_animated() {
        return Err(ImageError::UnsupportedFormat);
    }
    let colorspace = decoder.get_colorspace().ok_or(ImageError::InvalidHeader)?;
    let raw = decoder.decode_raw().map_err(decode_error)?;
    let pixels = into_rgba(raw, colorspace, len)?;
    Ok(ImageRgba8 {
        width,
        height,
        pixels,
    })
}

/// Decode baseline or progressive JPEG into opaque RGBA8.
pub fn decode_jpeg(bytes: &[u8]) -> Result<ImageRgba8, ImageError> {
    check_input(bytes)?;
    let (width, height) = jpeg_dimensions(bytes)?;
    checked_len(width, height)?;
    // RGB avoids the 0.4 decoder's CMYK-to-RGBA stride issue. Grayscale
    // streams select Luma internally; both are expanded below.
    let mut decoder = zune_jpeg::JpegDecoder::new_with_options(
        bytes,
        options()
            .jpeg_set_out_colorspace(ColorSpace::RGB)
            .jpeg_set_max_scans(100),
    );
    decoder.decode_headers().map_err(decode_error)?;
    let info = decoder.info().ok_or(ImageError::InvalidHeader)?;
    let width = u32::from(info.width);
    let height = u32::from(info.height);
    let len = checked_len(width, height)?;
    let colorspace = decoder
        .get_output_colorspace()
        .ok_or(ImageError::InvalidHeader)?;
    let raw_len = decoder
        .output_buffer_size()
        .ok_or(ImageError::InvalidDimensions)?;
    if raw_len > len {
        return Err(ImageError::InvalidDimensions);
    }
    let mut raw = vec![0; raw_len];
    decoder.decode_into(&mut raw).map_err(decode_error)?;
    let pixels = into_rgba(raw, colorspace, len)?;
    Ok(ImageRgba8 {
        width,
        height,
        pixels,
    })
}

/// Read JPEG dimensions without allocating or decoding entropy data. A partial
/// file is sufficient if it includes the complete SOF segment.
pub(crate) fn jpeg_dimensions(bytes: &[u8]) -> Result<(u32, u32), ImageError> {
    if !crate::is_jpeg(bytes) {
        return Err(ImageError::InvalidHeader);
    }
    let mut offset = 2usize;
    loop {
        if bytes.get(offset) != Some(&0xff) {
            return Err(ImageError::InvalidHeader);
        }
        while bytes.get(offset) == Some(&0xff) {
            offset += 1;
        }
        let marker = *bytes.get(offset).ok_or(ImageError::TruncatedInput)?;
        offset += 1;
        if marker == 0xda || marker == 0xd9 {
            return Err(ImageError::InvalidHeader);
        }
        if marker == 0x01 || (0xd0..=0xd7).contains(&marker) {
            continue;
        }
        let header = bytes
            .get(offset..offset + 2)
            .ok_or(ImageError::TruncatedInput)?;
        let len = u16::from_be_bytes([header[0], header[1]]) as usize;
        if len < 2 {
            return Err(ImageError::InvalidHeader);
        }
        let segment = bytes
            .get(offset..offset + len)
            .ok_or(ImageError::TruncatedInput)?;
        if matches!(marker, 0xc0..=0xc2) {
            if segment.len() < 8 {
                return Err(ImageError::InvalidHeader);
            }
            let height = u16::from_be_bytes([segment[3], segment[4]]) as u32;
            let width = u16::from_be_bytes([segment[5], segment[6]]) as u32;
            checked_len(width, height)?;
            return Ok((width, height));
        }
        offset += len;
    }
}

fn into_rgba(raw: Vec<u8>, colorspace: ColorSpace, len: usize) -> Result<Vec<u8>, ImageError> {
    if colorspace == ColorSpace::RGBA && raw.len() == len {
        return Ok(raw);
    }
    let components = match colorspace {
        ColorSpace::Luma => 1,
        ColorSpace::LumaA => 2,
        ColorSpace::RGB => 3,
        _ => return Err(ImageError::UnsupportedBitDepth),
    };
    if raw.len() != len / 4 * components {
        return Err(ImageError::InvalidDimensions);
    }
    let mut rgba = Vec::with_capacity(len);
    for p in raw.chunks_exact(components) {
        match colorspace {
            ColorSpace::Luma => rgba.extend_from_slice(&[p[0], p[0], p[0], 255]),
            ColorSpace::LumaA => rgba.extend_from_slice(&[p[0], p[0], p[0], p[1]]),
            ColorSpace::RGB => rgba.extend_from_slice(&[p[0], p[1], p[2], 255]),
            _ => unreachable!(),
        }
    }
    Ok(rgba)
}
