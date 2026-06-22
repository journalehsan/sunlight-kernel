//! Static File Handler
//!
//! Phase 1 - The Secure File Streamer:
//! Serve static files via VFS capability with path traversal protection.
//!
//! Security Model:
//! - `sanitize_path`: Rejects `..` and `.` components → prevents directory escape
//! - VFS capability token: Kernel-enforced access scope limits to document root
//! - 4 KB chunked streaming: Zero-copy to heap, fixed stack buffer
//! - 403 for traversal, 404 for missing (no info leak)
//!
//! VFS Protocol (register IPC, 32-byte path limit):
//! - OPEN: path in words[0..3], returns file handle
//! - READ: handle + offset + count, returns inline (≤16B) or SHM data
//! - CLOSE: handle, releases kernel file descriptor

use crate::net::TcpStream;
use core::fmt::Write;
use heapless::String;
use sunlight_ipc::{ipc_call, CapabilityToken, IpcMsg, VfsMsg};

/// Maximum VFS path length via register IPC (4 words × 8 bytes)
const VFS_PATH_MAX: usize = 32;

/// Document root for static file serving
const DOCROOT: &str = "/var/lib/sunlight/www/";

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Stream a static file from VFS directly to the TCP connection.
///
/// 1. Sanitizes the request path (blocks `..` traversal)
/// 2. Builds the full VFS path under `/var/lib/sunlight/www/`
/// 3. Opens the file via VFS IPC capability
/// 4. Sends HTTP 200 OK with the correct Content-Type
/// 5. Reads in 4 KB chunks and writes each to the socket
/// 6. On error, sends appropriate HTTP error (403/404/414/500)
pub fn serve_static_file(
    stream: &mut TcpStream,
    req_path: &str,
    vfs_cap: CapabilityToken,
    shm_pool: &crate::ShmPagePool,
) {
    // 1. Block path traversal attacks
    let sanitized = match sanitize_path(req_path) {
        Ok(p) => p,
        Err(SanitizePathError::Traversal) => {
            send_error(stream, 403, "Forbidden", "Path traversal detected.", shm_pool);
            return;
        }
        Err(SanitizePathError::Empty) => {
            send_error(stream, 400, "Bad Request", "Invalid path.", shm_pool);
            return;
        }
    };

    // 2. Build the full VFS path (docroot + sanitized relative path)
    let full_path = match build_vfs_path(sanitized.as_str()) {
        Some(p) => p,
        None => {
            send_error(stream, 414, "URI Too Long", "Request path too long.", shm_pool);
            return;
        }
    };

    // 3. Open the file via VFS IPC using our capability token
    let handle = match vfs_open(vfs_cap, full_path.as_str()) {
        Some(h) => h,
        None => {
            send_error(stream, 404, "Not Found", "The requested file does not exist.", shm_pool);
            return;
        }
    };

    // 4. Guess MIME type and send HTTP 200 OK headers
    let mime_type = mime_type_for_path(&sanitized);
    send_headers(stream, 200, "OK", mime_type, shm_pool);

    // 5. Zero-allocation streaming loop (4 KB chunks)
    let mut buffer = [0u8; 4096];
    let mut offset = 0usize;
    loop {
        match vfs_read_chunk(vfs_cap, handle, offset, &mut buffer) {
            Some(0) => break,
            Some(n) => {
                if stream.write_all(&buffer[..n], shm_pool).is_err() {
                    break;
                }
                offset += n;
            }
            None => break,
        }
    }

    // 6. Cleanup
    vfs_close(vfs_cap, handle);
}

/// Result of path sanitization
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SanitizePathError {
    /// Path attempts to escape the document root (contains `..` or `.`)
    Traversal,
    /// Path is empty or does not start with `/`
    Empty,
}

/// Sanitize a request path to prevent directory traversal.
///
/// Returns a relative path under the document root (e.g., `"index.html"`).
pub fn sanitize_path(request_path: &str) -> Result<String<512>, SanitizePathError> {
    if !request_path.starts_with('/') {
        return Err(SanitizePathError::Empty);
    }

    let path = request_path.trim_start_matches('/');

    if path.is_empty() {
        let mut s: String<512> = String::new();
        let _ = s.push_str("index.html");
        return Ok(s);
    }

    for component in path.split('/') {
        if component == ".." || component == "." {
            return Err(SanitizePathError::Traversal);
        }
    }

    let mut result = String::new();
    let _ = result.push_str(path);
    Ok(result)
}

/// Detect the MIME content type from a file's extension.
pub fn mime_type_for_path(path: &str) -> &'static str {
    if path.ends_with(".html") || path.ends_with(".htm") {
        "text/html"
    } else if path.ends_with(".css") {
        "text/css"
    } else if path.ends_with(".js") || path.ends_with(".mjs") {
        "application/javascript"
    } else if path.ends_with(".json") {
        "application/json"
    } else if path.ends_with(".png") {
        "image/png"
    } else if path.ends_with(".jpg") || path.ends_with(".jpeg") {
        "image/jpeg"
    } else if path.ends_with(".gif") {
        "image/gif"
    } else if path.ends_with(".svg") {
        "image/svg+xml"
    } else if path.ends_with(".ico") {
        "image/x-icon"
    } else if path.ends_with(".txt") {
        "text/plain"
    } else if path.ends_with(".pdf") {
        "application/pdf"
    } else if path.ends_with(".sbsp") {
        "text/plain"
    } else if path.ends_with(".woff") {
        "font/woff"
    } else if path.ends_with(".woff2") {
        "font/woff2"
    } else {
        "application/octet-stream"
    }
}

// ---------------------------------------------------------------------------
// VFS IPC helpers
// ---------------------------------------------------------------------------

/// Pack a path string into IPC words[0..3] (up to 32 bytes, NUL-terminated).
fn path_msg(label: u64, path: &str) -> IpcMsg {
    let bytes = path.as_bytes();
    let mut msg = IpcMsg::with_label(label);
    for word_idx in 0..4 {
        let start = word_idx * 8;
        let end = (start + 8).min(bytes.len());
        if start < bytes.len() {
            let mut word = 0u64;
            for (i, &b) in bytes[start..end].iter().enumerate() {
                word |= (b as u64) << (i * 8);
            }
            msg = msg.word(word_idx, word);
        }
    }
    msg
}

/// Build the full VFS path from docroot + sanitized relative path.
///
/// Returns `None` if the combined path exceeds the 32-byte VFS limit.
fn build_vfs_path(sanitized: &str) -> Option<String<VFS_PATH_MAX>> {
    let mut path: String<VFS_PATH_MAX> = String::new();
    path.push_str(DOCROOT).ok()?;
    path.push_str(sanitized).ok()?;
    if path.len() > VFS_PATH_MAX {
        return None;
    }
    Some(path)
}

/// Open a file via VFS IPC. Returns the file handle on success.
fn vfs_open(vfs_cap: CapabilityToken, path: &str) -> Option<u32> {
    let msg = path_msg(VfsMsg::OPEN, path);
    let reply = ipc_call(vfs_cap, msg);
    if reply.label == VfsMsg::REPLY && reply.words[0] == 0 {
        Some(reply.words[1] as u32)
    } else {
        None
    }
}

/// Read a chunk from an open VFS file handle.
///
/// Supports both inline replies (≤16 bytes, packed in words[2..4])
/// and large DATA_SHARED replies (SHM grant for >48 bytes).
///
/// Returns `Some(n)` with `n > 0` for data, `Some(0)` for EOF,
/// and `None` on error.
fn vfs_read_chunk(
    vfs_cap: CapabilityToken,
    handle: u32,
    offset: usize,
    buf: &mut [u8],
) -> Option<usize> {
    let read_msg = IpcMsg::with_label(VfsMsg::READ)
        .word(0, handle as u64)
        .word(1, offset as u64)
        .word(2, 4096);
    let reply = ipc_call(vfs_cap, read_msg);

    if reply.label == VfsMsg::REPLY && reply.words[0] == 0 {
        // Inline data: words[2..4] carry up to 16 bytes
        let n = reply.words[1] as usize;
        if n == 0 {
            return Some(0);
        }
        let count = n.min(buf.len()).min(16);
        for i in 0..count {
            let word = if i < 8 { reply.words[2] } else { reply.words[3] };
            buf[i] = ((word >> ((i % 8) * 8)) & 0xFF) as u8;
        }
        Some(count)
    } else if reply.label == VfsMsg::DATA_SHARED && reply.words[0] == 0 {
        // Large read via shared memory grant
        let n = reply.words[1] as usize;
        if n == 0 {
            return Some(0);
        }
        let count = n.min(buf.len());
        let shm_token = reply.caps[0];
        if shm_token == CapabilityToken::INVALID {
            return None;
        }
        match sunlight_ipc::shm_map(shm_token) {
            Ok(ptr) => {
                unsafe {
                    core::ptr::copy_nonoverlapping(ptr as *const u8, buf.as_mut_ptr(), count);
                }
                let _ = sunlight_ipc::shm_free(shm_token);
                Some(count)
            }
            Err(_) => None,
        }
    } else {
        None
    }
}

/// Close an open VFS file handle (best-effort).
fn vfs_close(vfs_cap: CapabilityToken, handle: u32) {
    let msg = IpcMsg::with_label(VfsMsg::CLOSE).word(0, handle as u64);
    let _ = ipc_call(vfs_cap, msg);
}

// ---------------------------------------------------------------------------
// HTTP response helpers
// ---------------------------------------------------------------------------

/// Send an HTTP error response with a plain-text body.
fn send_error(
    stream: &mut TcpStream,
    code: u16,
    status: &str,
    msg: &str,
    shm_pool: &crate::ShmPagePool,
) {
    let mut response: String<256> = String::new();
    let _ = write!(
        &mut response,
        "HTTP/1.1 {} {}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        code, status, msg.len(), msg
    );
    let _ = stream.write_all(response.as_bytes(), shm_pool);
}

/// Send an HTTP 200 OK with a Content-Type header (streaming body follows).
fn send_headers(
    stream: &mut TcpStream,
    code: u16,
    status: &str,
    mime: &str,
    shm_pool: &crate::ShmPagePool,
) {
    let mut response: String<128> = String::new();
    let _ = write!(
        &mut response,
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nConnection: close\r\n\r\n",
        code, status, mime
    );
    let _ = stream.write_all(response.as_bytes(), shm_pool);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_root() {
        let result = sanitize_path("/").expect("root should be allowed");
        assert_eq!(result, "index.html");
    }

    #[test]
    fn test_sanitize_simple_file() {
        let result = sanitize_path("/index.html").expect("simple path should be allowed");
        assert_eq!(result, "index.html");
    }

    #[test]
    fn test_sanitize_directory_file() {
        let result = sanitize_path("/css/style.css").expect("nested path should be allowed");
        assert_eq!(result, "css/style.css");
    }

    #[test]
    fn test_sanitize_traversal_double_dot() {
        let result = sanitize_path("/../etc/passwd");
        assert!(matches!(result, Err(SanitizePathError::Traversal)));
    }

    #[test]
    fn test_sanitize_traversal_middle() {
        let result = sanitize_path("/dir/../etc/passwd");
        assert!(matches!(result, Err(SanitizePathError::Traversal)));
    }

    #[test]
    fn test_sanitize_dot_self() {
        let result = sanitize_path("/./index.html");
        assert!(matches!(result, Err(SanitizePathError::Traversal)));
    }

    #[test]
    fn test_mime_type_html() {
        assert_eq!(mime_type_for_path("index.html"), "text/html");
    }

    #[test]
    fn test_mime_type_css() {
        assert_eq!(mime_type_for_path("style.css"), "text/css");
    }

    #[test]
    fn test_mime_type_javascript() {
        assert_eq!(mime_type_for_path("app.js"), "application/javascript");
    }

    #[test]
    fn test_mime_type_sbsp() {
        assert_eq!(mime_type_for_path("hello.sbsp"), "text/plain");
    }

    #[test]
    fn test_mime_type_unknown() {
        assert_eq!(mime_type_for_path("unknown.xyz"), "application/octet-stream");
    }

    #[test]
    fn test_build_vfs_path_root() {
        let path = build_vfs_path("index.html").expect("root path should fit");
        assert_eq!(path, "/var/lib/sunlight/www/index.html");
    }

    #[test]
    fn test_build_vfs_path_nested() {
        let path = build_vfs_path("css/style.css").expect("nested path should fit");
        assert_eq!(path.len(), "/var/lib/sunlight/www/css/style.css".len());
        // 22 + 14 = 36 > 32 → should not fit in register IPC
        assert!(path.len() > VFS_PATH_MAX);
    }

    #[test]
    fn test_vfs_path_too_long() {
        let long = "a".repeat(11); // 22 + 11 = 33 > 32
        assert!(build_vfs_path(&long).is_none());
    }
}
