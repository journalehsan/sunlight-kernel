//! Viewer navigation, fit-aware zoom, and a memory-bounded render cache.
use crate::zoom::{sanitize_scale, zoom_percent, ZoomMode};
use alloc::{vec, vec::Vec};
use anyhow::{bail, ensure, Context, Result};
use sunlight_docserv::{
    client::{DocClient, NativeClient},
    ipc::{DocRequest, DocResponse, DocumentPath},
};

/// Maximum rendered pages retained at once.
pub const MAX_CACHED_PAGES: usize = 5;
/// Maximum bytes retained by cached page bitmaps (strict memory bound).
pub const CACHE_BUDGET_BYTES: usize = 40 * 1024 * 1024;

/// Dimensions and pixels borrowed from the render cache.
pub struct BitmapRef<'a> {
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub data: &'a [u8],
}

/// Owns one service handle and its scale-specific page cache.
pub struct DocViewer<C: DocClient = NativeClient> {
    handle: u64,
    current_page: u32,
    total_pages: u32,
    zoom: f32,
    mode: ZoomMode,
    viewport: (u32, u32),
    page_pt: Vec<Option<(f32, f32)>>,
    cache: Vec<Option<Vec<u8>>>,
    dimensions: Vec<Option<(u32, u32, u32)>>,
    client: C,
    closed: bool,
}

impl DocViewer<NativeClient> {
    /// Open a document through the native service.
    pub fn open(path: impl Into<DocumentPath>) -> Result<Self> {
        Self::open_with_client(path, NativeClient::connect()?)
    }
}

impl<C: DocClient> DocViewer<C> {
    /// Open with an injectable RPC transport.
    pub fn open_with_client(path: impl Into<DocumentPath>, mut client: C) -> Result<Self> {
        let handle = match client.request(DocRequest::Open { path: path.into() })? {
            DocResponse::Opened { handle } if handle != 0 => handle,
            DocResponse::Err(error) => bail!("{error}"),
            _ => bail!("unexpected Open response"),
        };
        let count = (|| -> Result<u32> {
            match client.request(DocRequest::PageCount { handle })? {
                DocResponse::PageCount { count } => {
                    ensure!(
                        count > 0 && count <= 100_000,
                        "invalid or excessive page count"
                    );
                    Ok(count)
                }
                DocResponse::Err(error) => bail!("{error}"),
                _ => bail!("unexpected PageCount response"),
            }
        })();
        let total_pages = match count {
            Ok(count) => count,
            Err(error) => {
                let _ = client.request(DocRequest::Close { handle });
                return Err(error);
            }
        };
        Ok(Self {
            handle,
            current_page: 0,
            total_pages,
            zoom: 1.5,
            mode: ZoomMode::Fixed(1.5),
            viewport: (800, 640),
            page_pt: vec![None; total_pages as usize],
            cache: vec![None; total_pages as usize],
            dimensions: vec![None; total_pages as usize],
            client,
            closed: false,
        })
    }

    /// Unscaled point dimensions of a page, queried once and cached.
    pub fn page_dimensions(&mut self, page: u32) -> Result<(f32, f32)> {
        ensure!(page < self.total_pages, "page index is out of range");
        if let Some(dimensions) = self.page_pt[page as usize] {
            return Ok(dimensions);
        }
        let dimensions = match self.client.request(DocRequest::PageInfo {
            handle: self.handle,
            page,
        })? {
            DocResponse::PageDimensions {
                width_pt,
                height_pt,
            } => {
                ensure!(
                    width_pt.is_finite()
                        && height_pt.is_finite()
                        && width_pt > 0.0
                        && height_pt > 0.0,
                    "invalid page dimensions"
                );
                (width_pt, height_pt)
            }
            DocResponse::Err(error) => bail!("{error}"),
            _ => bail!("unexpected PageInfo response"),
        };
        self.page_pt[page as usize] = Some(dimensions);
        Ok(dimensions)
    }

    /// Record the drawable viewport (pixels) used to resolve fit modes.
    pub fn set_viewport(&mut self, width: u32, height: u32) {
        self.viewport = (width.max(1), height.max(1));
    }

    /// Render a page once at the current zoom.
    pub fn ensure_rendered(&mut self, page: u32) -> Result<()> {
        ensure!(!self.closed, "document is closed");
        ensure!(page < self.total_pages, "page index is out of range");
        if self.cache[page as usize].is_some() {
            return Ok(());
        }
        match self.client.request(DocRequest::RenderPage {
            handle: self.handle,
            page,
            scale: self.zoom,
        })? {
            DocResponse::RenderedPage {
                width,
                height,
                stride,
                data,
            } => {
                let minimum_stride = width.checked_mul(4).context("bitmap stride overflow")?;
                let expected = (stride as usize)
                    .checked_mul(height as usize)
                    .context("bitmap size overflow")?;
                ensure!(
                    width > 0 && height > 0 && stride >= minimum_stride && data.len() == expected,
                    "invalid BGRA8 bitmap"
                );
                self.dimensions[page as usize] = Some((width, height, stride));
                self.cache[page as usize] = Some(data);
                self.prune_cache(page);
                Ok(())
            }
            DocResponse::Err(error) => bail!("{error}"),
            _ => bail!("unexpected RenderPage response"),
        }
    }

    /// Keep the current page and up to two pages on either side, subject to the
    /// page-count and byte budgets. Farthest pages are evicted first.
    pub fn prefetch(&mut self, around: u32) -> Result<()> {
        ensure!(around < self.total_pages, "page index is out of range");
        let start = around.saturating_sub(2);
        let end = around.saturating_add(2).min(self.total_pages - 1);
        for (index, slot) in self.cache.iter_mut().enumerate() {
            if index < start as usize || index > end as usize {
                *slot = None;
                self.dimensions[index] = None;
            }
        }
        self.ensure_rendered(around)?;
        for page in start..=end {
            if self.cached_bytes() >= CACHE_BUDGET_BYTES {
                break;
            }
            self.ensure_rendered(page)?;
        }
        self.prune_cache(around);
        Ok(())
    }

    /// Evict cached pages outside `keep` until the budgets are satisfied.
    fn prune_cache(&mut self, keep: u32) {
        while self.cached_pages() > MAX_CACHED_PAGES || self.cached_bytes() > CACHE_BUDGET_BYTES {
            let victim = (0..self.total_pages as usize)
                .filter(|&index| index != keep as usize && self.cache[index].is_some())
                .max_by_key(|&index| index.abs_diff(keep as usize));
            let Some(victim) = victim else { break };
            self.cache[victim] = None;
            self.dimensions[victim] = None;
        }
    }

    /// Advance to the next page, stopping at the last page.
    pub fn next_page(&mut self) -> Result<()> {
        self.go_to((self.current_page + 1).min(self.total_pages - 1))
    }
    /// Move to the previous page, stopping at the first page.
    pub fn prev_page(&mut self) -> Result<()> {
        self.go_to(self.current_page.saturating_sub(1))
    }
    /// Navigate to a zero-based page and prefetch its neighbors.
    pub fn go_to(&mut self, page: u32) -> Result<()> {
        self.prefetch(page)?;
        self.current_page = page;
        Ok(())
    }
    /// Change the active zoom mode, choosing a fresh scale for `page`.
    pub fn set_mode(&mut self, page: u32, mode: ZoomMode) -> Result<()> {
        self.mode = mode;
        self.set_view(page, mode)
    }
    /// Apply navigation and zoom together, rendering the cache only once.
    pub fn set_view(&mut self, page: u32, mode: ZoomMode) -> Result<()> {
        ensure!(page < self.total_pages, "page index is out of range");
        let dimensions = self.page_dimensions(page)?;
        let scale = sanitize_scale(dimensions, mode.resolve(dimensions, self.viewport));
        self.mode = mode;
        if (self.zoom - scale).abs() > 1e-4 {
            self.zoom = scale;
            self.invalidate();
        } else {
            self.zoom = scale;
        }
        self.go_to(page)
    }
    /// Release rendered pages, for resize and scale changes.
    pub fn invalidate(&mut self) {
        for slot in &mut self.cache {
            *slot = None;
        }
        for slot in &mut self.dimensions {
            *slot = None;
        }
    }
    /// Current zero-based page index.
    pub fn current_page(&self) -> u32 {
        self.current_page
    }
    /// Total number of pages.
    pub fn total_pages(&self) -> u32 {
        self.total_pages
    }
    /// Current resolved pixels-per-point scale.
    pub fn zoom(&self) -> f32 {
        self.zoom
    }
    /// Current resolved zoom percentage.
    pub fn zoom_percent(&self) -> u32 {
        zoom_percent(self.zoom)
    }
    /// Active zoom mode.
    pub fn mode(&self) -> ZoomMode {
        self.mode
    }
    /// Number of resident page bitmaps.
    pub fn cached_pages(&self) -> usize {
        self.cache.iter().filter(|p| p.is_some()).count()
    }
    /// Bytes retained by page bitmaps.
    pub fn cached_bytes(&self) -> usize {
        self.cache.iter().flatten().map(Vec::len).sum()
    }
    /// Read a cached bitmap without copying it.
    pub fn bitmap(&self, page: u32) -> Option<BitmapRef<'_>> {
        let &(width, height, stride) = self.dimensions.get(page as usize)?.as_ref()?;
        Some(BitmapRef {
            width,
            height,
            stride,
            data: self.cache.get(page as usize)?.as_deref()?,
        })
    }
    /// Close once; retain the handle for retry if the service rejects close.
    pub fn close(&mut self) -> Result<()> {
        if self.closed {
            return Ok(());
        }
        match self.client.request(DocRequest::Close {
            handle: self.handle,
        })? {
            DocResponse::Ok => {
                self.closed = true;
                self.invalidate();
                Ok(())
            }
            DocResponse::Err(error) => bail!("{error}"),
            _ => bail!("unexpected Close response"),
        }
    }
}

impl<C: DocClient> Drop for DocViewer<C> {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zoom::ZoomPreset;
    use alloc::{rc::Rc, vec};
    use core::cell::RefCell;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Rec {
        Open,
        PageCount,
        PageInfo,
        Render,
        Close,
    }

    /// Request-driven mock: replies are derived from each request variant.
    struct MockClient {
        count: u32,
        page_pt: (f32, f32),
        requests: Rc<RefCell<alloc::vec::Vec<Rec>>>,
        renders: Rc<RefCell<usize>>,
    }
    impl MockClient {
        fn new(count: u32, page_pt: (f32, f32)) -> Self {
            Self {
                count,
                page_pt,
                requests: Rc::new(RefCell::new(alloc::vec::Vec::new())),
                renders: Rc::new(RefCell::new(0)),
            }
        }
    }
    impl DocClient for MockClient {
        fn request(&mut self, request: DocRequest) -> Result<DocResponse> {
            let (record, response) = match request {
                DocRequest::Open { .. } => (Rec::Open, DocResponse::Opened { handle: 7 }),
                DocRequest::PageCount { .. } => {
                    (Rec::PageCount, DocResponse::PageCount { count: self.count })
                }
                DocRequest::PageInfo { page, .. } => {
                    ensure!(page < self.count, "page out of range");
                    (
                        Rec::PageInfo,
                        DocResponse::PageDimensions {
                            width_pt: self.page_pt.0,
                            height_pt: self.page_pt.1,
                        },
                    )
                }
                DocRequest::RenderPage { page, .. } => {
                    ensure!(page < self.count, "page out of range");
                    *self.renders.borrow_mut() += 1;
                    (
                        Rec::Render,
                        DocResponse::RenderedPage {
                            width: 1,
                            height: 1,
                            stride: 4,
                            data: vec![1, 2, 3, 255],
                        },
                    )
                }
                DocRequest::Close { .. } => (Rec::Close, DocResponse::Ok),
                _ => (Rec::Open, DocResponse::Err("unsupported".into())),
            };
            self.requests.borrow_mut().push(record);
            Ok(response)
        }
    }

    const A4: (f32, f32) = (595.276, 841.890);

    #[test]
    fn opens_renders_zero_and_closes_without_service() -> Result<()> {
        let client = MockClient::new(1, A4);
        let requests = client.requests.clone();
        let mut viewer = DocViewer::open_with_client("sample.pdf", client)?;
        viewer.set_viewport(800, 600);
        viewer.set_view(0, ZoomPreset::ActualSize.mode())?;
        assert_eq!(viewer.zoom(), 1.0);
        viewer.ensure_rendered(0)?;
        viewer.ensure_rendered(0)?;
        assert_eq!(
            viewer.bitmap(0).context("missing bitmap")?.data,
            &[1, 2, 3, 255]
        );
        viewer.close()?;
        viewer.close()?;
        drop(viewer);
        assert_eq!(requests.borrow().last(), Some(&Rec::Close));
        Ok(())
    }

    #[test]
    fn fit_page_resolves_from_page_points_and_viewport() -> Result<()> {
        let client = MockClient::new(1, (600.0, 800.0));
        let mut viewer = DocViewer::open_with_client("sample.pdf", client)?;
        viewer.set_viewport(600, 400);
        viewer.set_view(0, ZoomMode::FitPage)?;
        // Fit page is limited by height: 400 / 800 = 0.5.
        assert!((viewer.zoom() - 0.5).abs() < 1e-4);
        assert_eq!(viewer.zoom_percent(), 50);
        // A taller viewport raises the fit scale.
        viewer.set_viewport(1200, 800);
        viewer.set_view(0, ZoomMode::FitPage)?;
        assert!((viewer.zoom() - 1.0).abs() < 1e-4);
        Ok(())
    }

    #[test]
    fn fit_width_uses_viewport_width() -> Result<()> {
        let client = MockClient::new(1, (600.0, 800.0));
        let mut viewer = DocViewer::open_with_client("sample.pdf", client)?;
        viewer.set_viewport(900, 200);
        viewer.set_view(0, ZoomMode::FitWidth)?;
        assert!((viewer.zoom() - 1.5).abs() < 1e-4);
        Ok(())
    }

    #[test]
    fn page_dimensions_are_queried_once() -> Result<()> {
        let client = MockClient::new(1, A4);
        let requests = client.requests.clone();
        let mut viewer = DocViewer::open_with_client("sample.pdf", client)?;
        viewer.set_viewport(800, 600);
        viewer.set_view(0, ZoomMode::FitPage)?;
        viewer.set_view(0, ZoomMode::FitWidth)?;
        let page_info_requests = requests
            .borrow()
            .iter()
            .filter(|r| **r == Rec::PageInfo)
            .count();
        assert_eq!(page_info_requests, 1);
        Ok(())
    }

    #[test]
    fn cache_is_bounded_by_page_count() -> Result<()> {
        let client = MockClient::new(20, (600.0, 800.0));
        let mut viewer = DocViewer::open_with_client("sample.pdf", client)?;
        viewer.set_viewport(800, 600);
        viewer.set_view(10, ZoomPreset::ActualSize.mode())?;
        assert!(viewer.cached_pages() <= MAX_CACHED_PAGES);
        assert!(viewer.cached_bytes() <= CACHE_BUDGET_BYTES);
        Ok(())
    }

    #[test]
    fn unchanged_page_does_not_rerender() -> Result<()> {
        let client = MockClient::new(6, A4);
        let renders = client.renders.clone();
        let mut viewer = DocViewer::open_with_client("sample.pdf", client)?;
        viewer.set_viewport(800, 600);
        viewer.set_view(0, ZoomPreset::ActualSize.mode())?;
        let first = *renders.borrow();
        assert!(first >= 1);
        // Same page and same scale must reuse the cache.
        viewer.set_view(0, ZoomPreset::ActualSize.mode())?;
        assert_eq!(*renders.borrow(), first);
        Ok(())
    }
}
