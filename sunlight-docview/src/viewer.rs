//! Viewer navigation and a five-page sliding render cache.
use alloc::{vec, vec::Vec};
use anyhow::{bail, ensure, Context, Result};
use sunlight_docserv::{
    client::{DocClient, NativeClient},
    ipc::{DocRequest, DocResponse, DocumentPath},
};

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
            cache: vec![None; total_pages as usize],
            dimensions: vec![None; total_pages as usize],
            client,
            closed: false,
        })
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
                Ok(())
            }
            DocResponse::Err(error) => bail!("{error}"),
            _ => bail!("unexpected RenderPage response"),
        }
    }

    /// Keep only the current page and up to two pages on either side.
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
            self.ensure_rendered(page)?;
        }
        Ok(())
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
    /// Change zoom, discard old-scale bitmaps, and render the current window.
    pub fn set_zoom(&mut self, zoom: f32) -> Result<()> {
        self.set_view(self.current_page, zoom)
    }
    /// Apply navigation and scale together, rendering the cache only once.
    pub fn set_view(&mut self, page: u32, zoom: f32) -> Result<()> {
        ensure!(zoom.is_finite(), "zoom must be finite");
        ensure!(page < self.total_pages, "page index is out of range");
        let zoom = zoom.clamp(0.5, 4.0);
        if self.zoom != zoom {
            self.zoom = zoom;
            self.invalidate();
        }
        self.go_to(page)
    }
    /// Release rendered pages, for resize and scale changes.
    pub fn invalidate(&mut self) {
        self.cache.fill(None);
        self.dimensions.fill(None);
    }
    /// Current zero-based page index.
    pub fn current_page(&self) -> u32 {
        self.current_page
    }
    /// Total number of pages.
    pub fn total_pages(&self) -> u32 {
        self.total_pages
    }
    /// Current pixels-per-point scale.
    pub fn zoom(&self) -> f32 {
        self.zoom
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
    use alloc::{collections::VecDeque, rc::Rc};
    use core::cell::RefCell;

    struct MockClient {
        requests: Rc<RefCell<Vec<DocRequest>>>,
        replies: VecDeque<DocResponse>,
    }
    impl DocClient for MockClient {
        fn request(&mut self, request: DocRequest) -> Result<DocResponse> {
            self.requests.borrow_mut().push(request);
            self.replies.pop_front().context("mock replies exhausted")
        }
    }

    #[test]
    fn opens_renders_zero_and_closes_without_service() -> Result<()> {
        let requests = Rc::new(RefCell::new(Vec::new()));
        let client = MockClient {
            requests: requests.clone(),
            replies: [
                DocResponse::Opened { handle: 42 },
                DocResponse::PageCount { count: 1 },
                DocResponse::RenderedPage {
                    width: 1,
                    height: 1,
                    stride: 4,
                    data: vec![1, 2, 3, 255],
                },
                DocResponse::Ok,
            ]
            .into(),
        };
        let mut viewer = DocViewer::open_with_client("sample.pdf", client)?;
        assert_eq!(viewer.zoom(), 1.5);
        viewer.ensure_rendered(0)?;
        viewer.ensure_rendered(0)?;
        assert_eq!(
            viewer.bitmap(0).context("missing bitmap")?.data,
            &[1, 2, 3, 255]
        );
        viewer.close()?;
        viewer.close()?;
        drop(viewer);
        assert_eq!(requests.borrow().len(), 4);
        assert!(matches!(
            requests.borrow().last(),
            Some(DocRequest::Close { handle: 42 })
        ));
        Ok(())
    }
}
