//! SunlightOS Open Resolver (v1).
//!
//! Resolves a file path to a default application using extension-based MIME
//! typing and a static association table, then launches via [`sun_exec`].
//!
//! Future direction:
//! - Extension detection will become content-based MIME sniffing.
//! - Associations will come from app manifests / desktop metadata.
//! - Per-user and per-type overrides will be supported.
//! - An "Open With" menu can layer on top of this resolver.
//! - Image, music, PDF, and browser apps can register handlers later.

use sunlight_ipc::launch_trace::{LaunchSource, LaunchTrace};

use crate::MAX_PATH;

use super::sun_exec::{self, LaunchError, LaunchResult};

const MAX_EXT: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenError {
    MissingPath,
    PathTooLong,
    NoAssociation,
    LaunchFailed(LaunchError),
}

/// Map a file path to a MIME type using its extension (lowercased).
pub fn mime_from_path(path: &[u8]) -> &'static [u8] {
    let mut ext = [0u8; MAX_EXT];
    let Some(ext_len) = extension_bytes(path, &mut ext) else {
        return b"application/octet-stream";
    };
    mime_from_extension(&ext[..ext_len])
}

/// Look up the default application path for a MIME type.
pub fn app_for_mime(mime: &[u8]) -> Option<&'static [u8]> {
    match mime {
        b"text/plain"
        | b"text/markdown"
        | b"text/toml"
        | b"text/rust"
        | b"application/json" => Some(b"/bin/sunlight-edit"),
        _ => None,
    }
}

/// Resolve and launch the default application for `path`.
pub fn open_path(
    trace: LaunchTrace,
    source: LaunchSource,
    path: &[u8],
) -> Result<LaunchResult, OpenError> {
    if path.is_empty() {
        return Err(OpenError::MissingPath);
    }
    if path.len() >= MAX_PATH || path.contains(&0) {
        return Err(OpenError::PathTooLong);
    }

    let mime = mime_from_path(path);
    let app = app_for_mime(mime).ok_or(OpenError::NoAssociation)?;

    sun_exec::launch(sun_exec::LaunchRequest {
        trace,
        source,
        command: app,
        args: &[path],
        require_display: true,
    })
    .map_err(OpenError::LaunchFailed)
}

fn mime_from_extension(ext: &[u8]) -> &'static [u8] {
    match ext {
        b"txt" | b"log" | b"conf" => b"text/plain",
        b"md" => b"text/markdown",
        b"toml" => b"text/toml",
        b"rs" => b"text/rust",
        b"json" => b"application/json",
        _ => b"application/octet-stream",
    }
}

/// Extract the final path component's extension into `out` (lowercased, no dot).
fn extension_bytes(path: &[u8], out: &mut [u8; MAX_EXT]) -> Option<usize> {
    let base_start = path
        .iter()
        .rposition(|&b| b == b'/')
        .map(|idx| idx + 1)
        .unwrap_or(0);
    let base = &path[base_start..];
    if base.is_empty() {
        return None;
    }

    let dot = base.iter().rposition(|&b| b == b'.')?;
    if dot == 0 || dot + 1 >= base.len() {
        return None;
    }

    let ext = &base[dot + 1..];
    if ext.is_empty() || ext.len() > out.len() {
        return None;
    }

    for (i, &b) in ext.iter().enumerate() {
        out[i] = b.to_ascii_lowercase();
    }
    Some(ext.len())
}
