//! Preferred PDF renderer, currently supported only on the Linux GNU host.
//! Native SunlightOS rendering remains blocked by Hayro's std dependencies.
use std::{
    io::Read,
    panic::{catch_unwind, AssertUnwindSafe},
    path::Path,
};

use anyhow::{ensure, Context, Result};
use hayro::{hayro_syntax::Pdf, PixmapSettings, RenderCache, RenderSettings};

use crate::backend::{DocumentBackend, DocumentHandle, PageBitmap, PageInfo};

const MAX_SOURCE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_PIXELS: u64 = 16_000_000;

/// PDF-only engine. Other formats remain available in the legacy backend.
pub struct HayroBackend;
struct HayroHandle(Pdf);

fn guarded<T>(operation: impl FnOnce() -> Result<T>) -> Result<T> {
    catch_unwind(AssertUnwindSafe(operation))
        .map_err(|_| anyhow::anyhow!("Hayro panicked while processing the PDF"))?
}

impl DocumentBackend for HayroBackend {
    fn open(&self, path: &Path) -> Result<Box<dyn DocumentHandle>> {
        // Bound the read itself, including files that grow after metadata checks.
        let file = std::fs::File::open(path).context("open PDF snapshot")?;
        let mut bytes = Vec::new();
        file.take(MAX_SOURCE_BYTES + 1)
            .read_to_end(&mut bytes)
            .context("read PDF snapshot")?;
        ensure!(
            bytes.len() as u64 <= MAX_SOURCE_BYTES,
            "document exceeds the 32 MiB source limit"
        );
        guarded(|| {
            let pdf = Pdf::new(bytes).map_err(|error| anyhow::anyhow!("invalid PDF: {error:?}"))?;
            ensure!(!pdf.pages().is_empty(), "document contains no pages");
            u32::try_from(pdf.pages().len()).context("too many PDF pages")?;
            Ok(Box::new(HayroHandle(pdf)) as Box<dyn DocumentHandle>)
        })
    }

    fn supported_extensions(&self) -> &[&str] {
        &["pdf"]
    }
}

impl DocumentHandle for HayroHandle {
    fn page_count(&self) -> u32 {
        self.0.pages().len() as u32
    }

    fn page_info(&self, page: u32) -> Result<PageInfo> {
        guarded(|| {
            let page = self
                .0
                .pages()
                .get(page as usize)
                .context("page out of range")?;
            let (width_pt, height_pt) = page.render_dimensions();
            ensure!(
                width_pt.is_finite() && height_pt.is_finite() && width_pt > 0.0 && height_pt > 0.0,
                "invalid PDF page dimensions"
            );
            Ok(PageInfo {
                width_pt,
                height_pt,
            })
        })
    }

    fn render_page(&self, page: u32, scale: f32) -> Result<PageBitmap> {
        ensure!(
            scale.is_finite() && scale > 0.0 && scale <= 4.0,
            "scale must be finite and in (0, 4]"
        );
        let info = self.page_info(page)?;
        // Hayro's convenience renderer truncates scaled dimensions to u16.
        let width = info.width_pt * scale;
        let height = info.height_pt * scale;
        ensure!(
            width >= 1.0
                && height >= 1.0
                && width <= u16::MAX as f32
                && height <= u16::MAX as f32
                && width as u64 * height as u64 <= MAX_PIXELS,
            "page exceeds the pixel/dimension budget"
        );
        guarded(|| {
            let pixmap = hayro::render(
                &self.0.pages()[page as usize],
                &RenderCache::new(),
                &Default::default(),
                &RenderSettings::default(),
                &PixmapSettings {
                    x_scale: scale,
                    y_scale: scale,
                    bg_color: hayro::vello_cpu::color::palette::css::WHITE,
                },
            );
            let width = u32::from(pixmap.width());
            let height = u32::from(pixmap.height());
            let stride = width.checked_mul(4).context("bitmap stride overflow")?;
            let mut data = pixmap.data_as_u8_slice().to_vec();
            // Keep the existing top-down BGRA8 wire format and opaque canvas input.
            for rgba in data.chunks_exact_mut(4) {
                ensure!(rgba[3] == 255, "expected an opaque PDF page");
                rgba.swap(0, 2);
            }
            Ok(PageBitmap {
                width,
                height,
                stride,
                data,
            })
        })
    }

    fn extract_text(&self, page: u32) -> Result<String> {
        self.page_info(page)?;
        anyhow::bail!("Hayro text extraction is not implemented")
    }
}
