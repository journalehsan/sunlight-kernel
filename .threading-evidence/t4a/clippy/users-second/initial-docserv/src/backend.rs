//! Format-independent document rendering contracts.
use std::path::Path;

/// A top-down bitmap with BGRA8 pixels.
pub struct PageBitmap {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Bytes per scanline.
    pub stride: u32,
    /// Pixel storage, exactly `stride * height` bytes.
    pub data: Vec<u8>,
}

/// Unscaled page dimensions in typographic points (1/72 inch).
pub struct PageInfo {
    /// Page width in points.
    pub width_pt: f32,
    /// Page height in points.
    pub height_pt: f32,
}

/// Opens documents of a supported format.
pub trait DocumentBackend: Send + Sync {
    /// Open a file, reporting format and filesystem errors.
    fn open(&self, path: &Path) -> anyhow::Result<Box<dyn DocumentHandle>>;
    /// Supported lowercase extensions without a leading dot.
    fn supported_extensions(&self) -> &[&str];
}

/// An open document. All page indices are zero-based.
pub trait DocumentHandle: Send + Sync {
    /// Total number of pages, determined when opening the document.
    fn page_count(&self) -> u32;
    /// Unscaled dimensions of a page.
    fn page_info(&self, page: u32) -> anyhow::Result<PageInfo>;
    /// Rasterize a page at a positive, finite pixels-per-point scale.
    fn render_page(&self, page: u32, scale: f32) -> anyhow::Result<PageBitmap>;
    /// Extract a page's text in reading order.
    fn extract_text(&self, page: u32) -> anyhow::Result<String>;
    /// Retained source bytes for this document, if the backend can report it.
    ///
    /// Defaults to 0 for backends that keep no owned source snapshot.
    fn source_bytes(&self) -> u64 {
        0
    }
}
