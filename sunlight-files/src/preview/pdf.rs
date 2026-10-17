//! Native PDF/PostScript thumbnails. Poll this future on the thumbnail worker;
//! the capability IPC client performs synchronous RPCs.
use alloc::vec::Vec;
use sunlight_docserv::{client::NativeClient, thumbnail::generate_with_client};

/// Native paths are UTF-8 strings; this module does not require std::path.
pub type Path = str;
/// Extensions registered with this provider.
pub const EXTENSIONS: [&str; 3] = ["pdf", "ps", "eps"];

/// Render page zero into an opaque square BGRA8 thumbnail on a worker task.
pub async fn generate_thumbnail(path: &Path, size: u32) -> anyhow::Result<Vec<u8>> {
    generate_with_client(&mut NativeClient::connect()?, path, size)
}

/// Whether the source filename belongs to this provider.
pub fn supports(path: &[u8]) -> bool {
    let Some(dot) = path.iter().rposition(|&b| b == b'.') else {
        return false;
    };
    let extension = &path[dot + 1..];
    EXTENSIONS
        .iter()
        .any(|ext| extension.eq_ignore_ascii_case(ext.as_bytes()))
}

pub(crate) fn worker_preview(path: &[u8], size: u32) -> Option<sunlight_ui::image::RgbaImage> {
    use core::{
        future::Future,
        task::{Context, Poll, Waker},
    };
    let path = core::str::from_utf8(path).ok()?;
    let mut future = core::pin::pin!(generate_thumbnail(path, size));
    let mut context = Context::from_waker(Waker::noop());
    let Poll::Ready(result) = future.as_mut().poll(&mut context) else {
        return None;
    };
    let data = result.ok()?;
    let pixels = data
        .chunks_exact(4)
        .map(|p| u32::from_le_bytes([p[0], p[1], p[2], p[3]]))
        .collect();
    Some(sunlight_ui::image::RgbaImage {
        width: size,
        height: size,
        pixels,
    })
}
