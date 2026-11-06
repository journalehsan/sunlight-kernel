//! Web SVG uses the shared, bounded static document-image renderer.

use alloc::{format, string::String};
use sunlight_ui::widgets::RasterImage;

pub fn decode(bytes: &[u8]) -> Result<RasterImage, String> {
    let image =
        sunlight_ui::image::decode_document_svg(bytes).map_err(|error| format!("SVG {error:?}"))?;
    Ok(RasterImage {
        width: image.width,
        height: image.height,
        pixels: image.pixels,
    })
}
