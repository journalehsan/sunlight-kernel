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

use crate::{self as libc, MAX_PATH};

use super::sun_exec::{self, LaunchError, LaunchResult};

const MAX_EXT: usize = 16;
const MAX_DESKTOP_FILE: usize = 4096;
const MAX_EXEC_LINE: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenError {
    MissingPath,
    PathTooLong,
    NoAssociation,
    InvalidDesktopEntry,
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
        b"text/plain" | b"text/markdown" | b"text/toml" | b"text/rust" | b"application/json" => {
            Some(b"/bin/sunlight-edit")
        }
        b"image/x-sunlight-simg" | b"image/x-tga" => Some(b"/bin/light-lens"),
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

    let mut ext = [0u8; MAX_EXT];
    if let Some(ext_len) = extension_bytes(path, &mut ext) {
        if &ext[..ext_len] == b"desktop" {
            return launch_desktop_entry(trace, source, path);
        }
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

/// Launch the application described by a freedesktop `.desktop` entry (v1: `Exec=` only).
pub fn launch_desktop_entry(
    trace: LaunchTrace,
    source: LaunchSource,
    path: &[u8],
) -> Result<LaunchResult, OpenError> {
    let mut buf = [0u8; MAX_DESKTOP_FILE];
    let len = read_file_bytes(path, &mut buf)?;
    let exec_value = find_exec_value(&buf[..len]).ok_or(OpenError::InvalidDesktopEntry)?;

    let mut normalized = [0u8; MAX_EXEC_LINE];
    let norm_len =
        normalize_exec_line(exec_value, &mut normalized).ok_or(OpenError::InvalidDesktopEntry)?;

    let mut words = [&[][..]; libc::MAX_ARGS - 1];
    let word_count = sun_exec::split_words(&normalized[..norm_len], &mut words)
        .map_err(OpenError::LaunchFailed)?;
    if word_count == 0 {
        return Err(OpenError::InvalidDesktopEntry);
    }

    sun_exec::launch_from_words(trace, source, &words[..word_count], true)
        .map_err(OpenError::LaunchFailed)
}

fn read_file_bytes(path: &[u8], buf: &mut [u8]) -> Result<usize, OpenError> {
    let fd = libc::open(path).map_err(|_| OpenError::InvalidDesktopEntry)?;
    let mut total = 0usize;
    while total < buf.len() {
        let n = libc::read(fd, &mut buf[total..]).map_err(|_| OpenError::InvalidDesktopEntry)?;
        if n == 0 {
            break;
        }
        total += n;
    }
    let _ = libc::close(fd);
    if total == 0 {
        return Err(OpenError::InvalidDesktopEntry);
    }
    Ok(total)
}

fn find_exec_value(data: &[u8]) -> Option<&[u8]> {
    let mut start = 0usize;
    while start < data.len() {
        let line_end = data[start..]
            .iter()
            .position(|&b| b == b'\n')
            .map(|idx| start + idx)
            .unwrap_or(data.len());
        let mut line = &data[start..line_end];
        if line.ends_with(&[b'\r']) {
            line = &line[..line.len() - 1];
        }
        if let Some(rest) = line.strip_prefix(b"Exec=") {
            return Some(rest);
        }
        if let Some(rest) = line.strip_prefix(b"exec=") {
            return Some(rest);
        }
        if line_end >= data.len() {
            break;
        }
        start = line_end + 1;
    }
    None
}

fn normalize_exec_line(value: &[u8], out: &mut [u8]) -> Option<usize> {
    let mut out_len = 0usize;
    let mut idx = 0usize;
    while idx < value.len() {
        if value[idx] == b'%' && idx + 1 < value.len() {
            if value[idx + 1] == b'%' {
                if out_len < out.len() {
                    out[out_len] = b'%';
                    out_len += 1;
                }
            }
            idx += 2;
            continue;
        }
        if value[idx].is_ascii_whitespace() {
            if out_len > 0 && out_len < out.len() && out[out_len - 1] != b' ' {
                out[out_len] = b' ';
                out_len += 1;
            }
            while idx < value.len() && value[idx].is_ascii_whitespace() {
                idx += 1;
            }
            continue;
        }
        if out_len < out.len() {
            out[out_len] = value[idx];
            out_len += 1;
        }
        idx += 1;
    }
    while out_len > 0 && out[out_len - 1] == b' ' {
        out_len -= 1;
    }
    if out_len == 0 {
        None
    } else {
        Some(out_len)
    }
}

fn mime_from_extension(ext: &[u8]) -> &'static [u8] {
    match ext {
        b"txt" | b"log" | b"conf" => b"text/plain",
        b"md" => b"text/markdown",
        b"toml" => b"text/toml",
        b"rs" => b"text/rust",
        b"json" => b"application/json",
        b"simg" => b"image/x-sunlight-simg",
        b"tga" => b"image/x-tga",
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
