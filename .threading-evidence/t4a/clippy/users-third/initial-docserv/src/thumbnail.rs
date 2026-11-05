//! Shared alloc-only document thumbnail lifecycle and square BGRA8 output.
use crate::{
    client::DocClient,
    ipc::{DocRequest, DocResponse, DocumentPath},
};
use alloc::vec::Vec;
use anyhow::{bail, ensure, Context, Result};

/// Render page zero at `size / 792` and fit it into a white `size × size`
/// BGRA8 thumbnail. Always attempt Close, including after a rendering error.
pub fn generate_with_client(
    client: &mut impl DocClient,
    path: impl Into<DocumentPath>,
    size: u32,
) -> Result<Vec<u8>> {
    ensure!(
        (1..=512).contains(&size),
        "thumbnail edge must be in 1..=512"
    );
    let handle = match client.request(DocRequest::Open { path: path.into() })? {
        DocResponse::Opened { handle } => handle,
        DocResponse::Err(error) => bail!("{error}"),
        _ => bail!("unexpected thumbnail Open response"),
    };
    let rendered = client.request(DocRequest::RenderPage {
        handle,
        page: 0,
        scale: size as f32 / 792.0,
    });
    let closed = client.request(DocRequest::Close { handle });
    let response = rendered?;
    match closed? {
        DocResponse::Ok => {}
        DocResponse::Err(error) => bail!("thumbnail close: {error}"),
        _ => bail!("unexpected thumbnail Close response"),
    }
    let (width, height, stride, data) = match response {
        DocResponse::RenderedPage {
            width,
            height,
            stride,
            data,
        } => (width, height, stride, data),
        DocResponse::Err(error) => bail!("{error}"),
        _ => bail!("unexpected thumbnail RenderPage response"),
    };
    ensure!(
        width > 0
            && height > 0
            && stride >= width.checked_mul(4).context("thumbnail stride overflow")?,
        "invalid thumbnail bitmap"
    );
    ensure!(
        data.len()
            == (stride as usize)
                .checked_mul(height as usize)
                .context("thumbnail size overflow")?,
        "invalid thumbnail storage"
    );
    let mut output = alloc::vec![255; size as usize * size as usize * 4];
    let longest = width.max(height);
    let fitted_width = (u64::from(width) * u64::from(size) / u64::from(longest)).max(1) as u32;
    let fitted_height = (u64::from(height) * u64::from(size) / u64::from(longest)).max(1) as u32;
    let left = (size - fitted_width) / 2;
    let top = (size - fitted_height) / 2;
    for y in 0..fitted_height {
        for x in 0..fitted_width {
            let sx = u64::from(x) * u64::from(width) / u64::from(fitted_width);
            let sy = u64::from(y) * u64::from(height) / u64::from(fitted_height);
            let source = sy as usize * stride as usize + sx as usize * 4;
            let destination = ((y + top) as usize * size as usize + (x + left) as usize) * 4;
            output[destination..destination + 4].copy_from_slice(&data[source..source + 4]);
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::collections::VecDeque;
    struct Mock {
        requests: Vec<DocRequest>,
        replies: VecDeque<DocResponse>,
    }
    impl DocClient for Mock {
        fn request(&mut self, request: DocRequest) -> Result<DocResponse> {
            self.requests.push(request);
            self.replies.pop_front().context("mock exhausted")
        }
    }
    #[test]
    fn square_thumbnail_preserves_bgra_and_closes() -> Result<()> {
        let mut mock = Mock {
            requests: Vec::new(),
            replies: [
                DocResponse::Opened { handle: 9 },
                DocResponse::RenderedPage {
                    width: 1,
                    height: 2,
                    stride: 4,
                    data: alloc::vec![204, 76, 25, 255, 204, 76, 25, 255],
                },
                DocResponse::Ok,
            ]
            .into(),
        };
        let data = generate_with_client(&mut mock, "sample.pdf", 4)?;
        assert_eq!(data.len(), 4 * 4 * 4);
        assert_eq!(&data[..4], &[255, 255, 255, 255]);
        assert_eq!(&data[4..8], &[204, 76, 25, 255]);
        assert!(matches!(
            mock.requests.last(),
            Some(DocRequest::Close { handle: 9 })
        ));
        Ok(())
    }
    #[test]
    fn render_failure_still_closes() {
        let mut mock = Mock {
            requests: Vec::new(),
            replies: [
                DocResponse::Opened { handle: 3 },
                DocResponse::Err("bad document".into()),
                DocResponse::Ok,
            ]
            .into(),
        };
        assert!(generate_with_client(&mut mock, "bad.pdf", 128).is_err());
        assert!(matches!(
            mock.requests.last(),
            Some(DocRequest::Close { handle: 3 })
        ));
    }
}
