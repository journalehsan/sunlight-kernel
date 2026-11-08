//! Browser-owned raster image discovery, bounded caching, and decoding.
//!
//! The browser keeps decoded pixels in `sunlight_ui`'s generic raster image
//! container so the retained scene and shared canvas remain HTML-independent.

use alloc::{string::String, sync::Arc, vec::Vec};

use sunlight_ui::widgets::RasterImage;

pub const MAX_IMAGE_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
// A 2000x2588 portrait needs 19.7 MiB. JPEG conversion releases decoder
// scratch before building ARGB; the single worker bounds concurrent decodes.
pub const MAX_IMAGE_DECODED_BYTES: usize = 24 * 1024 * 1024;
pub const MAX_IMAGE_DIMENSION: u32 = 4_096;
pub const MAX_IMAGE_CACHE_ENTRIES: usize = 32;
pub const MAX_IMAGE_CACHE_BYTES: usize = 28 * 1024 * 1024;

/// Keep the first document images resident until the initial scene exists.
/// Fetching more entries than this bounded cache can hold only evicts the
/// header/logo for later images before anything can be painted.
pub fn initial_image_candidates(
    candidates: &[crate::resources::discovery::ResourceCandidate],
) -> impl Iterator<Item = &crate::resources::discovery::ResourceCandidate> {
    candidates
        .iter()
        .filter(|c| c.resource_type == crate::resources::request::ResourceType::Image)
        .take(MAX_IMAGE_CACHE_ENTRIES)
}

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
    /// Pixel-budget eviction keeps geometry and terminal resource readiness.
    Evicted {
        width: u32,
        height: u32,
    },
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

    pub fn intrinsic_dimensions(&self, url: &str) -> Option<(u32, u32)> {
        match &self.get(url)?.state {
            ImageResourceState::Decoded(decoded) => {
                Some((decoded.image.width, decoded.image.height))
            }
            ImageResourceState::Evicted { width, height } => Some((*width, *height)),
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
        while self.entries.len() >= MAX_IMAGE_CACHE_ENTRIES {
            self.remove_at(0);
        }
        while self.decoded_bytes.saturating_add(decoded_len) > MAX_IMAGE_CACHE_BYTES {
            let Some(index) = self
                .entries
                .iter()
                .position(|entry| matches!(entry.state, ImageResourceState::Decoded(_)))
            else {
                break;
            };
            let ImageResourceState::Decoded(old) = &self.entries[index].state else {
                unreachable!()
            };
            let dimensions = (old.image.width, old.image.height);
            self.decoded_bytes = self
                .decoded_bytes
                .saturating_sub(old.image.pixels.len().saturating_mul(4));
            self.entries[index].state = ImageResourceState::Evicted {
                width: dimensions.0,
                height: dimensions.1,
            };
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
        ImageFormat::Png => {
            let (width, height) = sunlight_ui::image::inspect_image_dimensions(bytes)
                .map_err(|_| String::from("invalid PNG header"))?;
            validate_pixels(width, height, width as usize * height as usize)?;
            decode_raster(bytes)?
        }
        ImageFormat::Tga => {
            let (width, height) = sunlight_ui::image::inspect_image_dimensions(bytes)
                .map_err(|_| String::from("invalid TGA header"))?;
            validate_pixels(width, height, width as usize * height as usize)?;
            decode_raster(bytes)?
        }
        ImageFormat::Jpeg => {
            let (width, height) = sunlight_ui::image::inspect_image_dimensions(bytes)
                .map_err(|_| String::from("invalid JPEG header"))?;
            validate_pixels(width, height, width as usize * height as usize)?;
            decode_raster(bytes)?
        }
        ImageFormat::Svg => {
            #[cfg(feature = "svg")]
            {
                return Ok(DecodedImage {
                    image: Arc::new(crate::svg::decode(bytes)?),
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

fn decode_raster(bytes: &[u8]) -> Result<RasterImage, String> {
    let image = sunlight_ui::image::decode_image(bytes).map_err(|error| format_tga_error(error))?;
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
        sunlight_ui::image::DecodeError::CompressedImageInvalid => {
            String::from("invalid PNG/JPEG image")
        }
        sunlight_ui::image::DecodeError::SimgV2Invalid => String::from("invalid SIMG v2 image"),
        sunlight_ui::image::DecodeError::SimgV2Unsupported => {
            String::from("unsupported SIMG v2 image")
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn a_long_page_keeps_its_banner_in_the_initial_image_batch() {
        use crate::resources::{
            discovery::{DiscoveryClassification, ResourceCandidate},
            request::ResourceType,
        };
        let mut candidates = Vec::new();
        for index in 0..64 {
            candidates.push(ResourceCandidate {
                raw_url: alloc::format!("/image-{index}.png"),
                resolved_url: alloc::format!("https://example.org/image-{index}.png"),
                resource_type: ResourceType::Image,
                classification: DiscoveryClassification::EmbeddedResource,
                enqueue_for_fetch: true,
            });
            candidates.push(ResourceCandidate {
                raw_url: alloc::format!("/page-{index}"),
                resolved_url: alloc::format!("https://example.org/page-{index}"),
                resource_type: ResourceType::Navigation,
                classification: DiscoveryClassification::OrdinaryNavigationLink,
                enqueue_for_fetch: false,
            });
        }
        let mut cache = ImageCache::default();
        for candidate in initial_image_candidates(&candidates) {
            cache.insert_decoded(
                candidate.resolved_url.clone(),
                decode_image(TINY_PNG, Some("image/png"), &candidate.resolved_url).unwrap(),
            );
        }
        assert!(cache.decoded("https://example.org/image-0.png").is_some());
        assert!(cache.decoded("https://example.org/image-31.png").is_some());
        assert!(cache.get("https://example.org/image-63.png").is_none());
        assert!(cache.entries.len() <= MAX_IMAGE_CACHE_ENTRIES);
    }

    #[test]
    fn jpeg_uses_shared_decoder_and_browser_limits() {
        let bytes = include_bytes!("../../sun-img/tests/fixtures/gray.jpg");
        let decoded = decode_image(bytes, Some("image/jpeg"), "test.jpg").unwrap();
        assert_eq!(decoded.format, ImageFormat::Jpeg);
        assert_eq!((decoded.image.width, decoded.image.height), (8, 8));
        assert!(decoded.image.pixels.iter().all(|p| *p == 0xff7b7b7b));
        let mut oversized = bytes.to_vec();
        let sof = oversized
            .windows(2)
            .position(|p| p == [0xff, 0xc0])
            .unwrap();
        oversized[sof + 5..sof + 7].copy_from_slice(&4096u16.to_be_bytes());
        oversized[sof + 7..sof + 9].copy_from_slice(&4096u16.to_be_bytes());
        assert!(decode_image(&oversized, None, "test.jpg").is_err());
    }

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
    fn shared_png_palette_transparency_and_adam7_reach_rabbit() {
        for bytes in [
            include_bytes!("../../sun-img/tests/fixtures/palette.png").as_slice(),
            include_bytes!("../../sun-img/tests/fixtures/adam7.png").as_slice(),
        ] {
            let decoded = decode_image(bytes, Some("image/png"), "https://x/image.png").unwrap();
            assert_eq!((decoded.image.width, decoded.image.height), (2, 2));
            assert_eq!(
                decoded.image.pixels,
                vec![0xffff0000, 0x8000ff00, 0x000000ff, 0x400c2238]
            );
        }
    }
}
