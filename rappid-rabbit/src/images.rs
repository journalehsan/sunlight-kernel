//! Browser-owned raster image discovery, bounded caching, and decoding.
//!
//! The browser keeps decoded pixels in `sunlight_ui`'s generic raster image
//! container so the retained scene and shared canvas remain HTML-independent.

use alloc::{string::String, sync::Arc, vec, vec::Vec};

use miniz_oxide::inflate::decompress_to_vec_zlib_with_limit;
use sunlight_ui::widgets::RasterImage;

pub const MAX_IMAGE_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_IMAGE_DECODED_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_IMAGE_DIMENSION: u32 = 4_096;
pub const MAX_IMAGE_CACHE_ENTRIES: usize = 32;
pub const MAX_IMAGE_CACHE_BYTES: usize = 12 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageFormat {
    Png,
    Jpeg,
    Tga,
    Svg,
    Unknown,
}

impl ImageFormat {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpeg",
            Self::Tga => "tga",
            Self::Svg => "svg",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedImage {
    pub image: Arc<RasterImage>,
    pub format: ImageFormat,
    pub byte_size: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImageResourceState {
    Loading,
    Decoded(DecodedImage),
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageResource {
    pub resolved_url: String,
    pub state: ImageResourceState,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImageCache {
    entries: Vec<ImageResource>,
    decoded_bytes: usize,
}

impl ImageCache {
    pub fn get(&self, url: &str) -> Option<&ImageResource> {
        self.entries.iter().find(|entry| entry.resolved_url == url)
    }

    pub fn decoded(&self, url: &str) -> Option<&DecodedImage> {
        match self.get(url).map(|entry| &entry.state) {
            Some(ImageResourceState::Decoded(image)) => Some(image),
            _ => None,
        }
    }

    pub fn failed(&self, url: &str) -> Option<&str> {
        match self.get(url).map(|entry| &entry.state) {
            Some(ImageResourceState::Failed(error)) => Some(error.as_str()),
            _ => None,
        }
    }

    pub fn insert_decoded(&mut self, url: String, decoded: DecodedImage) {
        self.remove_existing(&url);
        let decoded_len = decoded
            .image
            .pixels
            .len()
            .saturating_mul(core::mem::size_of::<u32>());
        while !self.entries.is_empty()
            && (self.entries.len() >= MAX_IMAGE_CACHE_ENTRIES
                || self.decoded_bytes.saturating_add(decoded_len) > MAX_IMAGE_CACHE_BYTES)
        {
            self.remove_at(0);
        }
        if decoded_len > MAX_IMAGE_CACHE_BYTES || self.entries.len() >= MAX_IMAGE_CACHE_ENTRIES {
            return;
        }
        self.decoded_bytes = self.decoded_bytes.saturating_add(decoded_len);
        self.entries.push(ImageResource {
            resolved_url: url,
            state: ImageResourceState::Decoded(decoded),
        });
    }

    pub fn insert_failed(&mut self, url: String, error: impl Into<String>) {
        self.remove_existing(&url);
        if self.entries.len() >= MAX_IMAGE_CACHE_ENTRIES {
            self.remove_at(0);
        }
        self.entries.push(ImageResource {
            resolved_url: url,
            state: ImageResourceState::Failed(error.into()),
        });
    }

    fn remove_existing(&mut self, url: &str) {
        if let Some(index) = self
            .entries
            .iter()
            .position(|entry| entry.resolved_url == url)
        {
            self.remove_at(index);
        }
    }

    fn remove_at(&mut self, index: usize) {
        let entry = self.entries.remove(index);
        if let ImageResourceState::Decoded(decoded) = entry.state {
            self.decoded_bytes = self.decoded_bytes.saturating_sub(
                decoded
                    .image
                    .pixels
                    .len()
                    .saturating_mul(core::mem::size_of::<u32>()),
            );
        }
    }
}

pub fn detect_format(bytes: &[u8], content_type: Option<&str>, url: &str) -> ImageFormat {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return ImageFormat::Png;
    }
    if bytes.len() >= 3 && bytes[..3] == [0xFF, 0xD8, 0xFF] {
        return ImageFormat::Jpeg;
    }
    if bytes.len() >= 18 && bytes[2] == 2 && matches!(bytes[16], 24 | 32) {
        return ImageFormat::Tga;
    }
    let content_type = content_type.unwrap_or_default().to_ascii_lowercase();
    if content_type.contains("image/svg") || looks_like_svg(bytes) {
        return ImageFormat::Svg;
    }
    if content_type.contains("png") {
        return ImageFormat::Png;
    }
    if content_type.contains("jpeg") || content_type.contains("jpg") {
        return ImageFormat::Jpeg;
    }
    let path = url.split('?').next().unwrap_or(url).to_ascii_lowercase();
    if path.ends_with(".svg") {
        ImageFormat::Svg
    } else if path.ends_with(".png") {
        ImageFormat::Png
    } else if path.ends_with(".jpg") || path.ends_with(".jpeg") {
        ImageFormat::Jpeg
    } else if path.ends_with(".tga") || path.ends_with(".simg") {
        ImageFormat::Tga
    } else {
        ImageFormat::Unknown
    }
}

fn looks_like_svg(bytes: &[u8]) -> bool {
    let sample = core::str::from_utf8(&bytes[..bytes.len().min(512)]).unwrap_or_default();
    let sample = sample.trim_start_matches('\u{feff}').trim_start();
    sample.starts_with("<svg") || (sample.starts_with("<?xml") && sample.contains("<svg"))
}

pub fn decode_image(
    bytes: &[u8],
    content_type: Option<&str>,
    url: &str,
) -> Result<DecodedImage, String> {
    if bytes.len() > MAX_IMAGE_RESPONSE_BYTES {
        return Err(String::from("response exceeds image byte limit"));
    }
    let format = detect_format(bytes, content_type, url);
    let image = match format {
        ImageFormat::Png => decode_png(bytes)?,
        ImageFormat::Tga => decode_tga(bytes)?,
        ImageFormat::Jpeg => return Err(String::from("JPEG decoding is not implemented")),
        ImageFormat::Svg => {
            #[cfg(feature = "svg")]
            {
                return Ok(DecodedImage {
                    image: Arc::new(crate::svg::rasterize(
                        &crate::svg::parse(bytes)?,
                        crate::svg::SvgRasterKey {
                            target_width: 256,
                            target_height: 256,
                            scale_factor: 1,
                        },
                    )?),
                    format,
                    byte_size: bytes.len(),
                });
            }
            #[cfg(not(feature = "svg"))]
            {
                return Err(String::from("SVG renderer is unavailable for this target"));
            }
        }
        ImageFormat::Unknown => return Err(String::from("unsupported image format")),
    };
    Ok(DecodedImage {
        image: Arc::new(image),
        format,
        byte_size: bytes.len(),
    })
}

fn decode_tga(bytes: &[u8]) -> Result<RasterImage, String> {
    let image = sunlight_ui::image::decode_simg(bytes).map_err(|error| format_tga_error(error))?;
    validate_pixels(image.width, image.height, image.pixels.len())?;
    Ok(RasterImage {
        width: image.width,
        height: image.height,
        pixels: image.pixels,
    })
}

fn format_tga_error(error: sunlight_ui::image::DecodeError) -> String {
    match error {
        sunlight_ui::image::DecodeError::TooShort => String::from("truncated TGA header"),
        sunlight_ui::image::DecodeError::UnsupportedType(_) => String::from("unsupported TGA type"),
        sunlight_ui::image::DecodeError::UnsupportedDepth(_) => {
            String::from("unsupported TGA depth")
        }
        sunlight_ui::image::DecodeError::Truncated => String::from("truncated TGA pixels"),
    }
}

fn decode_png(bytes: &[u8]) -> Result<RasterImage, String> {
    if bytes.len() < 33 || !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(String::from("invalid PNG signature"));
    }
    let mut offset = 8usize;
    let mut width = 0u32;
    let mut height = 0u32;
    let mut bit_depth = 0u8;
    let mut color_type = 0u8;
    let mut interlace = 0u8;
    let mut palette = Vec::new();
    let mut palette_alpha = Vec::new();
    let mut idat = Vec::new();
    let mut saw_ihdr = false;
    while offset.checked_add(12).is_some_and(|end| end <= bytes.len()) {
        let length = be_u32(&bytes[offset..offset + 4]) as usize;
        let data_start = offset
            .checked_add(8)
            .ok_or_else(|| String::from("PNG chunk overflow"))?;
        let data_end = data_start
            .checked_add(length)
            .ok_or_else(|| String::from("PNG chunk overflow"))?;
        let next = data_end
            .checked_add(4)
            .ok_or_else(|| String::from("PNG chunk overflow"))?;
        if next > bytes.len() {
            return Err(String::from("truncated PNG chunk"));
        }
        let kind = &bytes[offset + 4..offset + 8];
        let data = &bytes[data_start..data_end];
        if kind == b"IHDR" {
            if saw_ihdr || data.len() != 13 {
                return Err(String::from("invalid PNG IHDR"));
            }
            width = be_u32(&data[0..4]);
            height = be_u32(&data[4..8]);
            bit_depth = data[8];
            color_type = data[9];
            if data[10] != 0 || data[11] != 0 {
                return Err(String::from("unsupported PNG compression or filter method"));
            }
            interlace = data[12];
            saw_ihdr = true;
        } else if kind == b"IDAT" {
            if !saw_ihdr {
                return Err(String::from("PNG data before IHDR"));
            }
            if idat.len().saturating_add(data.len()) > MAX_IMAGE_RESPONSE_BYTES {
                return Err(String::from("PNG compressed data exceeds limit"));
            }
            idat.extend_from_slice(data);
        } else if kind == b"PLTE" {
            if !saw_ihdr || !idat.is_empty() || data.is_empty() || data.len() % 3 != 0 {
                return Err(String::from("invalid PNG palette"));
            }
            if data.len() / 3 > 256 {
                return Err(String::from("PNG palette exceeds 256 entries"));
            }
            palette.clear();
            palette.extend_from_slice(data);
        } else if kind == b"tRNS" {
            if !saw_ihdr || !idat.is_empty() {
                return Err(String::from("invalid PNG transparency chunk"));
            }
            if color_type != 3 {
                return Err(String::from(
                    "PNG transparency is only supported for indexed images",
                ));
            }
            palette_alpha.clear();
            palette_alpha.extend_from_slice(data);
        } else if kind == b"IEND" {
            break;
        }
        offset = next;
    }
    if !saw_ihdr || idat.is_empty() {
        return Err(String::from("PNG is missing required data"));
    }
    if width == 0 || height == 0 || width > MAX_IMAGE_DIMENSION || height > MAX_IMAGE_DIMENSION {
        return Err(String::from("PNG dimensions exceed limit"));
    }
    if bit_depth != 8 || interlace != 0 || !matches!(color_type, 0 | 2 | 3 | 4 | 6) {
        return Err(String::from("unsupported PNG color format"));
    }
    if color_type == 3 && (palette.is_empty() || palette.len() % 3 != 0) {
        return Err(String::from("indexed PNG is missing a valid palette"));
    }
    if palette_alpha.len() > palette.len() / 3 {
        return Err(String::from("PNG transparency exceeds palette size"));
    }
    let channels = match color_type {
        0 => 1usize,
        2 => 3,
        3 => 1,
        4 => 2,
        6 => 4,
        _ => return Err(String::from("unsupported PNG color format")),
    };
    let row_bytes = (width as usize)
        .checked_mul(channels)
        .ok_or_else(|| String::from("PNG row overflow"))?;
    let inflated_len = (row_bytes
        .checked_add(1)
        .ok_or_else(|| String::from("PNG row overflow"))?)
    .checked_mul(height as usize)
    .ok_or_else(|| String::from("PNG output overflow"))?;
    if inflated_len > MAX_IMAGE_DECODED_BYTES {
        return Err(String::from("PNG decoded data exceeds limit"));
    }
    let inflated = decompress_to_vec_zlib_with_limit(&idat, inflated_len)
        .map_err(|_| String::from("PNG decompression failed"))?;
    if inflated.len() != inflated_len {
        return Err(String::from("truncated PNG pixels"));
    }
    let pixel_count = (width as usize)
        .checked_mul(height as usize)
        .ok_or_else(|| String::from("PNG pixel overflow"))?;
    let pixel_bytes = pixel_count
        .checked_mul(core::mem::size_of::<u32>())
        .ok_or_else(|| String::from("PNG pixel overflow"))?;
    if pixel_bytes > MAX_IMAGE_DECODED_BYTES {
        return Err(String::from("PNG pixel buffer exceeds limit"));
    }
    let mut pixels = vec![0u32; pixel_count];
    let mut previous = vec![0u8; row_bytes];
    let mut current = vec![0u8; row_bytes];
    for y in 0..height as usize {
        let row_start = y * (row_bytes + 1);
        let filter = inflated[row_start];
        current.copy_from_slice(&inflated[row_start + 1..row_start + 1 + row_bytes]);
        unfilter_png_row(&mut current, &previous, channels, filter)?;
        for x in 0..width as usize {
            let source = &current[x * channels..x * channels + channels];
            pixels[y * width as usize + x] = match color_type {
                0 => {
                    let gray = source[0] as u32;
                    0xFF00_0000 | (gray << 16) | (gray << 8) | gray
                }
                2 => {
                    0xFF00_0000
                        | ((source[0] as u32) << 16)
                        | ((source[1] as u32) << 8)
                        | source[2] as u32
                }
                3 => {
                    let index = source[0] as usize;
                    let palette_offset = index.saturating_mul(3);
                    if palette_offset + 2 >= palette.len() {
                        return Err(String::from("PNG palette index is out of range"));
                    }
                    let alpha = palette_alpha.get(index).copied().unwrap_or(0xFF) as u32;
                    (alpha << 24)
                        | ((palette[palette_offset] as u32) << 16)
                        | ((palette[palette_offset + 1] as u32) << 8)
                        | palette[palette_offset + 2] as u32
                }
                4 => {
                    let gray = source[0] as u32;
                    ((source[1] as u32) << 24) | (gray << 16) | (gray << 8) | gray
                }
                6 => {
                    ((source[3] as u32) << 24)
                        | ((source[0] as u32) << 16)
                        | ((source[1] as u32) << 8)
                        | source[2] as u32
                }
                _ => 0,
            };
        }
        core::mem::swap(&mut current, &mut previous);
    }
    Ok(RasterImage {
        width,
        height,
        pixels,
    })
}

fn unfilter_png_row(
    current: &mut [u8],
    previous: &[u8],
    bytes_per_pixel: usize,
    filter: u8,
) -> Result<(), String> {
    match filter {
        0 => {}
        1 => {
            for index in bytes_per_pixel..current.len() {
                current[index] = current[index].wrapping_add(current[index - bytes_per_pixel]);
            }
        }
        2 => {
            for (index, byte) in current.iter_mut().enumerate() {
                *byte = byte.wrapping_add(previous[index]);
            }
        }
        3 => {
            for index in 0..current.len() {
                let left = if index >= bytes_per_pixel {
                    current[index - bytes_per_pixel]
                } else {
                    0
                };
                let up = previous[index];
                current[index] = current[index].wrapping_add(((left as u16 + up as u16) / 2) as u8);
            }
        }
        4 => {
            for index in 0..current.len() {
                let left = if index >= bytes_per_pixel {
                    current[index - bytes_per_pixel]
                } else {
                    0
                };
                let up = previous[index];
                let upper_left = if index >= bytes_per_pixel {
                    previous[index - bytes_per_pixel]
                } else {
                    0
                };
                current[index] = current[index].wrapping_add(paeth(left, up, upper_left));
            }
        }
        _ => return Err(String::from("unsupported PNG row filter")),
    }
    Ok(())
}

fn paeth(left: u8, up: u8, upper_left: u8) -> u8 {
    let prediction = left as i32 + up as i32 - upper_left as i32;
    let left_distance = (prediction - left as i32).unsigned_abs();
    let up_distance = (prediction - up as i32).unsigned_abs();
    let upper_left_distance = (prediction - upper_left as i32).unsigned_abs();
    if left_distance <= up_distance && left_distance <= upper_left_distance {
        left
    } else if up_distance <= upper_left_distance {
        up
    } else {
        upper_left
    }
}

fn validate_pixels(width: u32, height: u32, pixel_count: usize) -> Result<(), String> {
    if width == 0 || height == 0 || width > MAX_IMAGE_DIMENSION || height > MAX_IMAGE_DIMENSION {
        return Err(String::from("image dimensions exceed limit"));
    }
    let expected = (width as usize)
        .checked_mul(height as usize)
        .ok_or_else(|| String::from("image pixel overflow"))?;
    if expected != pixel_count || expected.saturating_mul(4) > MAX_IMAGE_DECODED_BYTES {
        return Err(String::from("image pixel buffer exceeds limit"));
    }
    Ok(())
}

fn be_u32(bytes: &[u8]) -> u32 {
    u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

#[cfg(test)]
mod tests {
    use super::*;
    use miniz_oxide::deflate::compress_to_vec_zlib;

    const TINY_PNG: &[u8] = &[
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6,
        0, 0, 0, 31, 21, 196, 137, 0, 0, 0, 13, 73, 68, 65, 84, 120, 156, 99, 248, 207, 192, 240,
        31, 0, 5, 0, 1, 255, 137, 153, 61, 29, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
    ];

    #[test]
    fn decodes_tiny_png_to_argb() {
        let decoded = decode_image(TINY_PNG, Some("image/png"), "https://x/rabbit.png").unwrap();
        assert_eq!(decoded.format, ImageFormat::Png);
        assert_eq!(decoded.image.width, 1);
        assert_eq!(decoded.image.height, 1);
        assert_eq!(decoded.image.pixels.len(), 1);
    }

    #[test]
    fn malformed_png_is_rejected_without_panicking() {
        assert!(decode_image(b"\x89PNG\r\n\x1a\n", None, "x.png").is_err());
        let mut truncated = TINY_PNG.to_vec();
        truncated.truncate(40);
        assert!(decode_image(&truncated, None, "x.png").is_err());
    }

    #[test]
    fn magic_beats_content_type_and_url_extension() {
        assert_eq!(
            detect_format(TINY_PNG, Some("image/jpeg"), "https://x/not-a-jpeg.jpg"),
            ImageFormat::Png
        );
    }

    #[test]
    fn detects_svg_by_extension_content_type_and_xml_magic() {
        assert_eq!(
            detect_format(b"<svg viewBox='0 0 1 1'/>", None, "x.svg"),
            ImageFormat::Svg
        );
        assert_eq!(
            detect_format(b"<svg viewBox='0 0 1 1'/>", Some("image/svg+xml"), "x.bin"),
            ImageFormat::Svg
        );
        assert_eq!(
            detect_format(b"<?xml version='1.0'?><svg/>", None, "x.bin"),
            ImageFormat::Svg
        );
    }

    #[test]
    fn cache_deduplicates_and_remembers_failures() {
        let decoded = decode_image(TINY_PNG, None, "https://x/rabbit.png").unwrap();
        let mut cache = ImageCache::default();
        cache.insert_decoded(String::from("https://x/rabbit.png"), decoded);
        assert!(cache.decoded("https://x/rabbit.png").is_some());
        cache.insert_failed(String::from("https://x/missing.png"), "HTTP 404");
        assert_eq!(cache.failed("https://x/missing.png"), Some("HTTP 404"));
    }

    #[test]
    fn decodes_indexed_png_with_palette_and_transparency() {
        let png = png_with_chunks(
            2,
            1,
            3,
            &[b"PLTE", &[0xFF, 0, 0, 0, 0x80, 0xFF]],
            &[b"tRNS", &[0xFF, 0x80]],
            &[b"IDAT", &compress_to_vec_zlib(&[0, 0, 1], 6)],
        );
        let decoded = decode_image(&png, Some("image/png"), "https://x/indexed.png").unwrap();
        assert_eq!(decoded.image.width, 2);
        assert_eq!(decoded.image.height, 1);
        assert_eq!(decoded.image.pixels, vec![0xFFFF_0000, 0x8000_80FF]);
    }

    #[test]
    fn indexed_png_without_palette_reports_its_specific_error() {
        let png = png_with_chunks(
            1,
            1,
            3,
            &[],
            &[],
            &[b"IDAT", &compress_to_vec_zlib(&[0, 0], 6)],
        );
        assert_eq!(
            decode_image(&png, Some("image/png"), "https://x/indexed.png"),
            Err(String::from("indexed PNG is missing a valid palette"))
        );
    }

    fn png_with_chunks(
        width: u32,
        height: u32,
        color_type: u8,
        first: &[&[u8]],
        second: &[&[u8]],
        idat: &[&[u8]],
    ) -> Vec<u8> {
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&width.to_be_bytes());
        ihdr.extend_from_slice(&height.to_be_bytes());
        ihdr.extend_from_slice(&[8, color_type, 0, 0, 0]);
        push_png_chunk(&mut png, b"IHDR", &ihdr);
        for chunk in [first, second, idat] {
            if let [kind, data] = chunk {
                push_png_chunk(&mut png, kind, data);
            }
        }
        push_png_chunk(&mut png, b"IEND", &[]);
        png
    }

    fn push_png_chunk(png: &mut Vec<u8>, kind: &[u8], data: &[u8]) {
        png.extend_from_slice(&(data.len() as u32).to_be_bytes());
        png.extend_from_slice(kind);
        png.extend_from_slice(data);
        png.extend_from_slice(&0u32.to_be_bytes());
    }
}
