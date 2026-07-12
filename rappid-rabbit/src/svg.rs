//! Isolated static-SVG adapter for bring-up builds.
//!
//! The adapter never supplies a resource resolver or system-font database.
//! `usvg` therefore parses only the bytes handed to it; external references,
//! scripts, animation, and foreignObject are not made available to the tree.

use alloc::{format, string::String, vec::Vec};

use sunlight_ui::widgets::RasterImage;

pub const MAX_SVG_SOURCE_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_SVG_RASTER_PIXELS: usize = 8 * 1024 * 1024;
pub const MAX_SVG_DIMENSION: u32 = 4096;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SvgDocument {
    pub intrinsic_width: u32,
    pub intrinsic_height: u32,
    pub view_box: String,
    source: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SvgRasterKey {
    pub target_width: u32,
    pub target_height: u32,
    pub scale_factor: u32,
}

pub fn parse(bytes: &[u8]) -> Result<SvgDocument, String> {
    if bytes.len() > MAX_SVG_SOURCE_BYTES {
        return Err(String::from("SVG source exceeds limit"));
    }
    let options = resvg::usvg::Options::default();
    let tree =
        resvg::usvg::Tree::from_data(bytes, &options).map_err(|_| String::from("invalid SVG"))?;
    let size = tree.size().to_int_size();
    let width = size.width().min(MAX_SVG_DIMENSION);
    let height = size.height().min(MAX_SVG_DIMENSION);
    if width == 0 || height == 0 {
        return Err(String::from("SVG has zero dimensions"));
    }
    Ok(SvgDocument {
        intrinsic_width: width,
        intrinsic_height: height,
        view_box: format!("{}x{}", tree.size().width(), tree.size().height()),
        source: bytes.to_vec(),
    })
}

pub fn rasterize(document: &SvgDocument, key: SvgRasterKey) -> Result<RasterImage, String> {
    let width = key.target_width.min(MAX_SVG_DIMENSION).max(1);
    let height = key.target_height.min(MAX_SVG_DIMENSION).max(1);
    let pixels = (width as usize)
        .checked_mul(height as usize)
        .ok_or_else(|| String::from("SVG pixel overflow"))?;
    if pixels > MAX_SVG_RASTER_PIXELS {
        return Err(String::from("SVG raster exceeds limit"));
    }
    let tree = resvg::usvg::Tree::from_data(&document.source, &resvg::usvg::Options::default())
        .map_err(|_| String::from("invalid SVG"))?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)
        .ok_or_else(|| String::from("SVG pixmap allocation failed"))?;
    let sx = width as f32 / document.intrinsic_width.max(1) as f32;
    let sy = height as f32 / document.intrinsic_height.max(1) as f32;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(sx, sy),
        &mut pixmap.as_mut(),
    );
    let mut argb = Vec::with_capacity(pixels);
    for rgba in pixmap.data().chunks_exact(4) {
        let a = rgba[3] as u32;
        // tiny-skia is premultiplied RGBA; unpremultiply before storing in the
        // shared ARGB8888 canvas format, avoiding double darkening.
        let unpm = |channel: u8| {
            if a == 0 {
                0
            } else {
                ((channel as u32 * 255 + a / 2) / a).min(255)
            }
        };
        argb.push((a << 24) | (unpm(rgba[0]) << 16) | (unpm(rgba[1]) << 8) | unpm(rgba[2]));
    }
    Ok(RasterImage {
        width,
        height,
        pixels: argb,
    })
}
