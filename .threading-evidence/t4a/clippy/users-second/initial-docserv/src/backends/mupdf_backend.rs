//! MuPDF rendering, with immutable source snapshots safe to share across threads.
use std::{path::Path, sync::Arc};

use anyhow::{ensure, Context, Result};
use mupdf::{Colorspace, Document, Matrix, Page};

use crate::backend::{DocumentBackend, DocumentHandle, PageBitmap, PageInfo};

/// Primary engine for PDF, XPS, CBZ, and EPUB documents.
pub struct MuPdfBackend;

struct MuPdfHandle {
    bytes: Arc<Vec<u8>>,
    extension: String,
    count: u32,
}

impl MuPdfHandle {
    fn with_page<T>(&self, index: u32, operation: impl FnOnce(Page) -> Result<T>) -> Result<T> {
        ensure!(
            index < self.count,
            "page {index} is out of range ({} pages)",
            self.count
        );
        // MuPDF 0.4's raw document pointers are neither Send nor Sync. Reopen
        // the immutable snapshot in the calling thread's MuPDF context rather
        // than declaring an unsafe Send implementation for those pointers.
        let document = Document::from_bytes(&self.bytes, &self.extension)
            .context("reopen document snapshot")?;
        let page = document
            .load_page(i32::try_from(index)?)
            .context("load page")?;
        operation(page).with_context(|| format!("process page {}", index + 1))
    }
}

impl DocumentBackend for MuPdfBackend {
    fn open(&self, path: &Path) -> Result<Box<dyn DocumentHandle>> {
        ensure!(
            std::fs::metadata(path).context("stat document")?.len() <= 32 * 1024 * 1024,
            "document exceeds the 32 MiB source limit"
        );
        let filename = path.to_str().context("MuPDF requires a UTF-8 path")?;
        let document =
            Document::open(filename).with_context(|| format!("open {}", path.display()))?;
        ensure!(
            !document.needs_password().context("check encryption")?,
            "document requires a password"
        );
        let bytes = std::fs::read(path).context("snapshot document")?;
        let extension = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("pdf")
            .to_ascii_lowercase();
        // Count the immutable bytes that future operations will use, so an
        // edit between opening the file and snapshotting cannot stale the count.
        let snapshot =
            Document::from_bytes(&bytes, &extension).context("validate document snapshot")?;
        let count = u32::try_from(snapshot.page_count().context("count pages")?)?;
        ensure!(count > 0, "document contains no pages");
        Ok(Box::new(MuPdfHandle {
            bytes: Arc::new(bytes),
            extension,
            count,
        }))
    }

    fn supported_extensions(&self) -> &[&str] {
        // PS/EPS are handled by GhostscriptBackend, not falsely advertised here.
        &["pdf", "xps", "cbz", "epub"]
    }
}

impl DocumentHandle for MuPdfHandle {
    fn page_count(&self) -> u32 {
        self.count
    }

    fn page_info(&self, page: u32) -> Result<PageInfo> {
        self.with_page(page, |page| {
            let bounds = page.bounds().context("read page bounds")?;
            Ok(PageInfo {
                width_pt: bounds.x1 - bounds.x0,
                height_pt: bounds.y1 - bounds.y0,
            })
        })
    }

    fn render_page(&self, page: u32, scale: f32) -> Result<PageBitmap> {
        ensure!(
            scale.is_finite() && scale > 0.0 && scale <= 4.0,
            "scale must be finite and in (0, 4]"
        );
        self.with_page(page, |page| {
            let bounds = page.bounds().context("read page bounds")?;
            let width = ((bounds.x1 - bounds.x0) * scale).ceil();
            let height = ((bounds.y1 - bounds.y0) * scale).ceil();
            ensure!(
                width > 0.0 && height > 0.0 && width * height <= 16_000_000.0,
                "page exceeds the 16 megapixel rendering limit"
            );
            // Opaque RGB avoids premultiplied-alpha ambiguity on white pages.
            let pixmap = page
                .to_pixmap(
                    &Matrix::new_scale(scale, scale),
                    &Colorspace::device_rgb(),
                    0.0,
                    true,
                )
                .context("rasterize page")?;
            ensure!(pixmap.n() == 3, "unexpected RGB pixmap component count");
            let width = pixmap.width();
            let height = pixmap.height();
            let source_stride = usize::try_from(pixmap.stride())?;
            let stride = width.checked_mul(4).context("bitmap stride overflow")?;
            let len = (stride as usize)
                .checked_mul(height as usize)
                .context("bitmap size overflow")?;
            let mut data = vec![0; len];
            for (src, dst) in pixmap
                .samples()
                .chunks_exact(source_stride)
                .zip(data.chunks_exact_mut(stride as usize))
            {
                for (rgb, bgra) in src[..width as usize * 3]
                    .chunks_exact(3)
                    .zip(dst.chunks_exact_mut(4))
                {
                    bgra.copy_from_slice(&[rgb[2], rgb[1], rgb[0], 255]);
                }
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
        self.with_page(page, |page| page.to_text().context("extract text"))
    }
}
