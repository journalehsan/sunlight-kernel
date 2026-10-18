//! Embedded SVG/TGA icons. SVG trees and size-specific rasters have bounded,
//! process-local caches; handles borrow the embedded source and remain `Copy`.

use super::tga::{TgaError, TgaImage};

#[derive(Debug, Clone, Copy)]
pub enum IconError {
    Tga(TgaError),
    SvgUnavailable,
    InvalidSvg,
    LimitExceeded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    Tga(TgaImage),
    #[cfg_attr(not(feature = "svg"), allow(dead_code))]
    Svg(&'static [u8]),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IconImage {
    pub width: u32,
    pub height: u32,
    source: Source,
}

impl IconImage {
    pub fn parse(bytes: &'static [u8]) -> Result<Self, IconError> {
        // Embedded XML may have a declaration, whitespace, or a UTF-8 BOM.
        let xml = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
        if xml.iter().find(|b| !b.is_ascii_whitespace()) == Some(&b'<') {
            #[cfg(feature = "svg")]
            {
                let tree = svg::tree(bytes)?;
                let size = tree.size().to_int_size();
                return Ok(Self {
                    width: size.width(),
                    height: size.height(),
                    source: Source::Svg(bytes),
                });
            }
            #[cfg(not(feature = "svg"))]
            return Err(IconError::SvgUnavailable);
        }
        let image = TgaImage::parse(bytes).map_err(IconError::Tga)?;
        Ok(Self {
            width: image.width,
            height: image.height,
            source: Source::Tga(image),
        })
    }

    /// Prepare once before sampling pixels. SVG rasterization uses the actual
    /// destination size; the returned handle keeps evicted rasters alive.
    pub fn rasterize(&self, _width: u32, _height: u32) -> Result<IconRaster, IconError> {
        match self.source {
            Source::Tga(image) => Ok(IconRaster {
                width: image.width,
                height: image.height,
                source: RasterSource::Tga(image),
            }),
            Source::Svg(_bytes) => {
                #[cfg(feature = "svg")]
                {
                    let image = svg::raster(_bytes, _width, _height)?;
                    Ok(IconRaster {
                        width: image.width,
                        height: image.height,
                        source: RasterSource::Svg(image),
                    })
                }
                #[cfg(not(feature = "svg"))]
                Err(IconError::SvgUnavailable)
            }
        }
    }

    /// Legacy pixel access. Drawing code should prepare a raster once instead
    /// of looking up the SVG cache for every pixel.
    pub fn pixel_argb(&self, x: u32, y: u32) -> u32 {
        match self.source {
            Source::Tga(image) => image.pixel_argb(x, y),
            Source::Svg(_) => self
                .rasterize(self.width, self.height)
                .map_or(0, |image| image.pixel_argb(x, y)),
        }
    }

    pub fn pixel_xrgb(&self, x: u32, y: u32) -> u32 {
        self.pixel_argb(x, y) & 0x00ff_ffff
    }

    pub fn has_alpha(&self) -> bool {
        match self.source {
            Source::Tga(image) => image.has_alpha(),
            Source::Svg(_) => true,
        }
    }
}

pub struct IconRaster {
    pub width: u32,
    pub height: u32,
    source: RasterSource,
}

enum RasterSource {
    Tga(TgaImage),
    #[cfg(feature = "svg")]
    Svg(alloc::sync::Arc<super::RgbaImage>),
}

impl IconRaster {
    pub fn pixel_argb(&self, x: u32, y: u32) -> u32 {
        match &self.source {
            RasterSource::Tga(image) => image.pixel_argb(x, y),
            #[cfg(feature = "svg")]
            RasterSource::Svg(image) => image.pixel(x, y),
        }
    }
}

/// Decode an SVG loaded from a file without retaining its source or tree.
/// Callers own the returned raster and choose when it is retired.
#[cfg(feature = "svg")]
pub fn decode_svg(bytes: &[u8], width: u32, height: u32) -> Result<super::RgbaImage, IconError> {
    svg::render(&svg::parse(bytes)?, width, height)
}

#[cfg(feature = "svg")]
mod svg {
    use super::IconError;
    use crate::image::RgbaImage;
    use alloc::{boxed::Box, sync::Arc, vec::Vec};
    use std::sync::Mutex;

    const MAX_SOURCE: usize = 2 * 1024 * 1024;
    const MAX_DIMENSION: u32 = 4096;
    const MAX_RASTER_DIMENSION: u32 = 512;
    const MAX_TREES: usize = 128;
    const MAX_RASTERS: usize = 256;
    const MAX_CACHED_BYTES: usize = 4 * 1024 * 1024;

    type SourceKey = (usize, usize);
    type RasterKey = (SourceKey, u32, u32);
    static TREES: Mutex<Vec<(SourceKey, Arc<resvg::usvg::Tree>)>> = Mutex::new(Vec::new());
    static RASTERS: Mutex<Vec<(RasterKey, Arc<RgbaImage>)>> = Mutex::new(Vec::new());

    fn key(bytes: &'static [u8]) -> SourceKey {
        (bytes.as_ptr() as usize, bytes.len())
    }

    pub(super) fn parse(bytes: &[u8]) -> Result<resvg::usvg::Tree, IconError> {
        if bytes.len() > MAX_SOURCE {
            return Err(IconError::LimitExceeded);
        }
        // Embedded icons are self-contained: do not load linked files or
        // discover host fonts while parsing an application asset.
        let mut options = resvg::usvg::Options::default();
        options.image_href_resolver.resolve_string = Box::new(|_, _| None);
        let tree =
            resvg::usvg::Tree::from_data(bytes, &options).map_err(|_| IconError::InvalidSvg)?;
        let size = tree.size();
        if size.width() > MAX_DIMENSION as f32 || size.height() > MAX_DIMENSION as f32 {
            return Err(IconError::LimitExceeded);
        }
        Ok(tree)
    }

    pub(super) fn tree(bytes: &'static [u8]) -> Result<Arc<resvg::usvg::Tree>, IconError> {
        if bytes.len() > MAX_SOURCE {
            return Err(IconError::LimitExceeded);
        }
        let mut cache = TREES.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(index) = cache.iter().position(|(k, _)| *k == key(bytes)) {
            let entry = cache.remove(index);
            let tree = entry.1.clone();
            cache.push(entry);
            return Ok(tree);
        }
        let tree = parse(bytes)?;
        let tree = Arc::new(tree);
        if cache.len() == MAX_TREES {
            cache.remove(0);
        }
        cache.push((key(bytes), tree.clone()));
        Ok(tree)
    }

    pub(super) fn raster(
        bytes: &'static [u8],
        width: u32,
        height: u32,
    ) -> Result<Arc<RgbaImage>, IconError> {
        if width == 0 || height == 0 {
            return Err(IconError::LimitExceeded);
        }
        let width = width.min(MAX_RASTER_DIMENSION);
        let height = height.min(MAX_RASTER_DIMENSION);
        let key = (key(bytes), width, height);
        {
            let mut cache = RASTERS.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(index) = cache.iter().position(|(k, _)| *k == key) {
                let entry = cache.remove(index);
                let image = entry.1.clone();
                cache.push(entry);
                return Ok(image);
            }
        }
        let tree = tree(bytes)?;
        let image = Arc::new(render(&tree, width, height)?);
        #[cfg(all(feature = "app", target_os = "sunlightos"))]
        {
            use core::sync::atomic::{AtomicBool, Ordering};
            static REPORTED: AtomicBool = AtomicBool::new(false);
            if !REPORTED.swap(true, Ordering::Relaxed) {
                sunlight_ipc::debug_log(&alloc::format!(
                    "[ICON-SVG] pid={} runtime raster ready\n",
                    sunlight_ipc::getpid()
                ));
            }
        }
        let mut cache = RASTERS.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((_, existing)) = cache.iter().find(|(k, _)| *k == key) {
            return Ok(existing.clone());
        }
        let mut used: usize = cache.iter().map(|(_, image)| image.pixels.len() * 4).sum();
        while !cache.is_empty()
            && (cache.len() >= MAX_RASTERS || used + image.pixels.len() * 4 > MAX_CACHED_BYTES)
        {
            used -= cache.remove(0).1.pixels.len() * 4;
        }
        cache.push((key, image.clone()));
        Ok(image)
    }
    pub(super) fn render(
        tree: &resvg::usvg::Tree,
        width: u32,
        height: u32,
    ) -> Result<RgbaImage, IconError> {
        if width == 0 || height == 0 {
            return Err(IconError::LimitExceeded);
        }
        let width = width.min(MAX_RASTER_DIMENSION);
        let height = height.min(MAX_RASTER_DIMENSION);
        let mut pixmap =
            resvg::tiny_skia::Pixmap::new(width, height).ok_or(IconError::LimitExceeded)?;
        let scale = (width as f32 / tree.size().width()).min(height as f32 / tree.size().height());
        let tx = (width as f32 - tree.size().width() * scale) * 0.5;
        let ty = (height as f32 - tree.size().height() * scale) * 0.5;
        resvg::render(
            &tree,
            resvg::tiny_skia::Transform::from_row(scale, 0.0, 0.0, scale, tx, ty),
            &mut pixmap.as_mut(),
        );
        let pixels = pixmap
            .data()
            .chunks_exact(4)
            .map(|rgba| {
                let a = u32::from(rgba[3]);
                let straight = |c: u8| {
                    if a == 0 {
                        0
                    } else {
                        ((u32::from(c) * 255 + a / 2) / a).min(255)
                    }
                };
                (a << 24) | (straight(rgba[0]) << 16) | (straight(rgba[1]) << 8) | straight(rgba[2])
            })
            .collect();
        Ok(RgbaImage {
            width,
            height,
            pixels,
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn cache_eviction_keeps_in_flight_rasters_valid() {
            const RED: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><rect width="16" height="16" fill="#ff0000"/></svg>"##;
            let held = raster(RED, 64, 64).unwrap();
            assert!(Arc::ptr_eq(&held, &raster(RED, 64, 64).unwrap()));
            for size in 500..512 {
                raster(RED, size, size).unwrap();
            }
            let cache = RASTERS.lock().unwrap();
            assert!(cache.len() <= MAX_RASTERS);
            assert!(
                cache.iter().map(|(_, i)| i.pixels.len() * 4).sum::<usize>() <= MAX_CACHED_BYTES
            );
            assert_eq!(held.pixel(32, 32), 0xffff0000);
        }
    }
}
