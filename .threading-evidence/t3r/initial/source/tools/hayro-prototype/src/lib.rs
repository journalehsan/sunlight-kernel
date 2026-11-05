//! Host-only feasibility experiment; not a SunlightOS native backend.
//! Engine allocations are infallible: use a disposable process for untrusted PDFs.
use hayro::{hayro_syntax::Pdf, vello_cpu::Pixmap, PixmapSettings, RenderCache, RenderSettings};
use std::panic::{catch_unwind, AssertUnwindSafe};

pub const MAX_SOURCE_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_PIXELS: u64 = 16_000_000;
pub type Result<T> = std::result::Result<T, String>;

#[derive(Debug, Clone, Copy)]
pub struct PageInfo {
    /// Dimensions after the PDF's intrinsic rotation and crop, in points.
    pub width_pt: f32,
    pub height_pt: f32,
}

/// Separate from file access and UI. Drop releases the PDF and its parser caches.
/// Rendering returns owned premultiplied RGBA8 storage without a bitmap clone.
pub trait Document {
    fn page_count(&self) -> usize;
    fn page_info(&self, page: usize) -> Result<PageInfo>;
    fn render_page(&self, page: usize, scale: f32) -> Result<PageBitmap>;
}

/// Owns the renderer buffer. Accessors expose top-down RGBA8 without copying.
pub struct PageBitmap(Pixmap);

impl PageBitmap {
    pub fn width(&self) -> u16 {
        self.0.width()
    }
    pub fn height(&self) -> u16 {
        self.0.height()
    }
    pub fn stride(&self) -> usize {
        self.width() as usize * 4
    }
    pub fn data_as_u8_slice(&self) -> &[u8] {
        self.0.data_as_u8_slice()
    }
}

pub struct HayroDocument(Pdf);

fn guarded<T>(operation: impl FnOnce() -> Result<T>) -> Result<T> {
    catch_unwind(AssertUnwindSafe(operation))
        .map_err(|_| "Hayro panicked; discard this document".to_owned())?
}

impl HayroDocument {
    /// Takes ownership of a caller-provided snapshot; no filesystem or source copy.
    pub fn load(bytes: Vec<u8>) -> Result<Self> {
        if bytes.len() > MAX_SOURCE_BYTES {
            return Err("document exceeds 32 MiB".into());
        }
        guarded(|| {
            let pdf = Pdf::new(bytes).map_err(|e| format!("invalid PDF: {e:?}"))?;
            if pdf.pages().is_empty() {
                return Err("document has no pages".into());
            }
            Ok(Self(pdf))
        })
    }
}

impl Document for HayroDocument {
    fn page_count(&self) -> usize {
        self.0.pages().len()
    }

    fn page_info(&self, page: usize) -> Result<PageInfo> {
        guarded(|| {
            let page = self.0.pages().get(page).ok_or("page out of range")?;
            let (width_pt, height_pt) = page.render_dimensions();
            if !width_pt.is_finite()
                || !height_pt.is_finite()
                || width_pt <= 0.0
                || height_pt <= 0.0
            {
                return Err("invalid page dimensions".into());
            }
            Ok(PageInfo {
                width_pt,
                height_pt,
            })
        })
    }

    fn render_page(&self, page: usize, scale: f32) -> Result<PageBitmap> {
        if !scale.is_finite() || scale <= 0.0 || scale > 4.0 {
            return Err("scale must be finite and in (0, 4]".into());
        }
        let info = self.page_info(page)?;
        // Hayro's convenience renderer truncates to u16 rather than rounding up.
        let width = info.width_pt * scale;
        let height = info.height_pt * scale;
        if width < 1.0
            || height < 1.0
            || width > u16::MAX as f32
            || height > u16::MAX as f32
            || width as u64 * height as u64 > MAX_PIXELS
        {
            return Err("page exceeds pixel/dimension budget".into());
        }
        guarded(|| {
            // Cache is render-local to avoid self-referential lifetimes and retained growth.
            let cache = RenderCache::new();
            Ok(PageBitmap(hayro::render(
                &self.0.pages()[page],
                &cache,
                &Default::default(),
                &RenderSettings::default(),
                &PixmapSettings {
                    x_scale: scale,
                    y_scale: scale,
                    bg_color: hayro::vello_cpu::color::palette::css::WHITE,
                },
            )))
        })
    }
}

/// The existing retained canvas requires Vec<u32>; perform one checked conversion
/// at that boundary. White background makes the RGBA result opaque.
pub fn into_raster(
    bitmap: PageBitmap,
) -> Result<sunlight_ui::widgets::document_canvas::RasterImage> {
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(bitmap.width() as usize * bitmap.height() as usize)
        .map_err(|_| "cannot allocate UI raster")?;
    for rgba in bitmap.data_as_u8_slice().chunks_exact(4) {
        if rgba[3] != 255 {
            return Err("expected opaque white-composited page".into());
        }
        pixels.push(u32::from_be_bytes([rgba[3], rgba[0], rgba[1], rgba[2]]));
    }
    Ok(sunlight_ui::widgets::document_canvas::RasterImage {
        width: bitmap.width() as u32,
        height: bitmap.height() as u32,
        pixels,
    })
}
