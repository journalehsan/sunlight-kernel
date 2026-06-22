//! Static File Handler
//!
//! Phase 3: Serve static files via VFS capability
//! - Path sanitization (prevent `/../etc/passwd` traversal)
//! - VFS capability-based file reading (zero direct syscalls)
//! - Content-Type detection by extension
//! - Streaming response directly to TCP socket
//!
//! Security Model:
//! - Strict whitelist: only /var/lib/sunlight/www/* allowed
//! - All reads mediated through VFS capability token
//! - Path canonicalization: reject any `..` escapes
//! - 404 for missing files (no "file exists" information leak)

use heapless::String;

/// Result of path sanitization
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SanitizePathError {
    /// Path attempts to escape the document root
    Traversal,
    /// Path is empty or invalid
    Empty,
}

/// Sanitize a request path to prevent directory traversal
///
/// # Example
/// ```
/// assert!(sanitize_path("/../etc/passwd").is_err()); // ✗ Blocked
/// assert!(sanitize_path("/index.html").is_ok());      // ✓ Allowed
/// assert!(sanitize_path("/css/style.css").is_ok());   // ✓ Allowed
/// ```
pub fn sanitize_path(request_path: &str) -> Result<String<512>, SanitizePathError> {
    // Request path must start with /
    if !request_path.starts_with('/') {
        return Err(SanitizePathError::Empty);
    }

    // Remove leading slash for document root joining
    let path = request_path.trim_start_matches('/');

    if path.is_empty() {
        // Root request → serve index.html
        return Ok(String::from("index.html"));
    }

    // Reject any path component that is ".." (directory escape)
    for component in path.split('/') {
        if component == ".." || component == "." {
            return Err(SanitizePathError::Traversal);
        }
    }

    let mut result = String::new();
    let _ = result.push_str(path);
    Ok(result)
}

/// MIME type detection by file extension
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
    } else if path.ends_with(".woff") {
        "font/woff"
    } else if path.ends_with(".woff2") {
        "font/woff2"
    } else {
        "application/octet-stream"
    }
}

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
    fn test_mime_type_unknown() {
        assert_eq!(mime_type_for_path("unknown.xyz"), "application/octet-stream");
    }
}
