use super::icon_theme::name as icon_name;

pub const MAX_MIME_ICON_NAME: usize = 64;
pub const DIRECTORY_MIMETYPE_ICON: &str = "inode-directory";
pub const UNKNOWN_ICON: &str = "unknown";

#[derive(Clone, Copy)]
pub struct MimeIconLookup<'a> {
    pub exact: Option<&'a str>,
    pub family: Option<&'static str>,
    pub generic: &'static str,
}

pub fn resolve_file_icon<'a>(
    mime: &[u8],
    out: &'a mut [u8; MAX_MIME_ICON_NAME],
) -> MimeIconLookup<'a> {
    let exact = mime_to_freedesktop_name(mime, out);
    let family = family_fallback_name(mime);
    let generic = generic_fallback_name(mime);
    MimeIconLookup {
        exact,
        family,
        generic,
    }
}

pub fn mime_to_freedesktop_name<'a>(
    mime: &[u8],
    out: &'a mut [u8; MAX_MIME_ICON_NAME],
) -> Option<&'a str> {
    if mime.is_empty() || mime.len() > out.len() {
        return None;
    }
    let mut slash_found = false;
    for (idx, &byte) in mime.iter().enumerate() {
        out[idx] = if byte == b'/' {
            slash_found = true;
            b'-'
        } else {
            byte
        };
    }
    if !slash_found {
        return None;
    }
    core::str::from_utf8(&out[..mime.len()]).ok()
}

pub fn family_fallback_name(mime: &[u8]) -> Option<&'static str> {
    if has_prefix(mime, b"text/") {
        Some(icon_name::TEXT_GENERIC)
    } else if has_prefix(mime, b"image/") {
        Some(icon_name::IMAGE_GENERIC)
    } else if has_prefix(mime, b"audio/") {
        Some(icon_name::AUDIO_GENERIC)
    } else if has_prefix(mime, b"video/") {
        Some(icon_name::VIDEO_GENERIC)
    } else if has_prefix(mime, b"application/") {
        Some("application-octet-stream")
    } else {
        None
    }
}

pub fn generic_fallback_name(mime: &[u8]) -> &'static str {
    if has_prefix(mime, b"image/") {
        icon_name::IMAGE_GENERIC
    } else if has_prefix(mime, b"text/") {
        icon_name::TEXT_GENERIC
    } else if has_prefix(mime, b"audio/") {
        icon_name::AUDIO_GENERIC
    } else if has_prefix(mime, b"video/") {
        icon_name::VIDEO_GENERIC
    } else {
        "application-octet-stream"
    }
}

pub fn is_image_mime(mime: &[u8]) -> bool {
    has_prefix(mime, b"image/")
}

pub fn is_text_like_mime(mime: &[u8]) -> bool {
    has_prefix(mime, b"text/")
        || mime == b"application/json"
        || mime == b"application/toml"
        || mime == b"application/xml"
}

fn has_prefix(bytes: &[u8], prefix: &[u8]) -> bool {
    bytes.len() >= prefix.len() && &bytes[..prefix.len()] == prefix
}
