//! Feature-only native compatibility experiment, not a PDF rendering backend.
//! Parser and rasterizer internals allocate infallibly. Limits here cover only
//! input and output; they cannot bound an arbitrary PDF's decoded working set.
#![no_std]

extern crate alloc;

use alloc::vec::Vec;
use hayro_syntax::Pdf;

pub const MAX_SOURCE_BYTES: usize = 256 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    SourceTooLarge,
    Allocation,
    InvalidPdf,
}

/// Bounded, fallible snapshot copy. Subsequent parser allocations are infallible.
pub fn open(bytes: &[u8]) -> Result<Pdf, Error> {
    if bytes.len() > MAX_SOURCE_BYTES {
        return Err(Error::SourceTooLarge);
    }
    let mut snapshot = Vec::new();
    snapshot
        .try_reserve_exact(bytes.len())
        .map_err(|_| Error::Allocation)?;
    snapshot.extend_from_slice(bytes);
    Pdf::new(snapshot).map_err(|_| Error::InvalidPdf)
}

/// Rasterize an original small vector fixture and convert to the existing
/// document canvas's storage. This does not interpret or render a PDF.
#[cfg(feature = "raster")]
pub fn vector_fixture() -> Result<sunlight_ui::widgets::document_canvas::RasterImage, Error> {
    use vello_cpu::{color::palette::css, kurbo::Rect, Pixmap, RenderContext, Resources};

    const WIDTH: u16 = 64;
    const HEIGHT: u16 = 48;
    let mut context = RenderContext::new(WIDTH, HEIGHT);
    context.set_paint(css::WHITE);
    context.fill_rect(&Rect::new(0.0, 0.0, WIDTH.into(), HEIGHT.into()));
    context.set_paint(css::RED);
    context.fill_rect(&Rect::new(8.0, 8.0, 32.0, 24.0));
    context.flush();
    let mut pixmap = Pixmap::new(WIDTH, HEIGHT);
    context.render(&mut pixmap, &mut Resources::new());
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(usize::from(WIDTH) * usize::from(HEIGHT))
        .map_err(|_| Error::Allocation)?;
    for rgba in pixmap.data_as_u8_slice().chunks_exact(4) {
        pixels.push(u32::from_be_bytes([rgba[3], rgba[0], rgba[1], rgba[2]]));
    }
    Ok(sunlight_ui::widgets::document_canvas::RasterImage {
        width: WIDTH.into(),
        height: HEIGHT.into(),
        pixels,
    })
}
