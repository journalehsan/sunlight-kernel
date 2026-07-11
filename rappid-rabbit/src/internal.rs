//! Small, allow-listed loader for browser-owned `sunlight://` documents.
//!
//! This is deliberately separate from HTTP fetching: an internal resource is
//! read from the mounted root RAMFS and never enters the network stack.

use alloc::{format, string::String, vec::Vec};

pub const START_URL: &str = "sunlight://start/";
pub const START_PAGE_URL: &str = "sunlight://startpage/";
const START_HTML_PATH: &str = "/usr/share/rappid-rabbit/start/index.html";
const START_IMAGE_PATH: &str = "/usr/share/rappid-rabbit/start/aslan.png";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InternalResourceKind {
    Document,
    Image,
}

impl InternalResourceKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Document => "Document",
            Self::Image => "Image",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InternalResource {
    pub url: String,
    pub ramfs_path: &'static str,
    pub kind: InternalResourceKind,
}

pub fn resolve(url: &str) -> Option<InternalResource> {
    let normalized = normalize(url)?;
    match normalized.as_str() {
        START_URL | START_PAGE_URL => Some(InternalResource {
            url: normalized,
            ramfs_path: START_HTML_PATH,
            kind: InternalResourceKind::Document,
        }),
        "sunlight://start/aslan.png" | "sunlight://startpage/aslan.png" => Some(InternalResource {
            url: normalized,
            ramfs_path: START_IMAGE_PATH,
            kind: InternalResourceKind::Image,
        }),
        _ => None,
    }
}

pub fn resolve_relative(base: &str, raw: &str) -> Option<InternalResource> {
    if raw.is_empty() || raw.contains('?') || raw.contains('#') {
        return None;
    }
    if raw.starts_with("sunlight://") {
        return resolve(raw);
    }
    if base == START_URL || base == START_PAGE_URL {
        if raw == "aslan.png" {
            return resolve(&format!("{base}aslan.png"));
        }
    }
    None
}

fn normalize(url: &str) -> Option<String> {
    if !url.starts_with("sunlight://") || url.contains('\\') || url.contains("..") {
        return None;
    }
    let path = &url[10..];
    if path.is_empty() || !path.starts_with("start") {
        return None;
    }
    if path == "start" {
        return Some(String::from(START_URL));
    }
    if path == "start/" || path == "start/index.html" {
        return Some(String::from(START_URL));
    }
    if path == "start/aslan.png" {
        return Some(String::from("sunlight://start/aslan.png"));
    }
    if path == "startpage" || path == "startpage/" || path == "startpage/index.html" {
        return Some(String::from(START_PAGE_URL));
    }
    if path == "startpage/aslan.png" {
        return Some(String::from("sunlight://startpage/aslan.png"));
    }
    None
}

/// Read an allow-listed resource through the ordinary user-space file API.
/// The browser process cannot address arbitrary RAMFS paths through this
/// function because callers can only obtain the two paths returned by `resolve`.
pub fn read(resource: &InternalResource) -> Result<Vec<u8>, String> {
    let fd = sunlight_libc::open(resource.ramfs_path.as_bytes())
        .map_err(|_| String::from("RAMFS open failed"))?;
    let size = sunlight_libc::fstat(fd)
        .map_err(|_| String::from("RAMFS stat failed"))?
        .size as usize;
    let mut bytes = Vec::with_capacity(size);
    bytes.resize(size, 0);
    let result = sunlight_libc::read(fd, &mut bytes).map_err(|_| String::from("RAMFS read failed"));
    let _ = sunlight_libc::close(fd);
    result.map(|read| {
        bytes.truncate(read);
        bytes
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlisted_routes_and_relative_image_resolve() {
        assert_eq!(
            resolve(START_URL).unwrap().kind,
            InternalResourceKind::Document
        );
        assert_eq!(
            resolve(START_PAGE_URL).unwrap().kind,
            InternalResourceKind::Document
        );
        assert_eq!(
            resolve(START_PAGE_URL).unwrap().ramfs_path,
            START_HTML_PATH
        );
        assert_eq!(
            resolve_relative(START_URL, "aslan.png").unwrap().kind,
            InternalResourceKind::Image
        );
        assert_eq!(
            resolve_relative(START_PAGE_URL, "aslan.png").unwrap().kind,
            InternalResourceKind::Image
        );
        assert!(resolve("sunlight://start/../../etc/shadow").is_none());
        assert!(resolve("sunlight://other/secret").is_none());
        assert!(resolve("sunlight://startpage/").is_some());
    }
}
