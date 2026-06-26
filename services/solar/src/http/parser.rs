//! Zero-Allocation HTTP/1.1 Request Parser
//!
//! Parses incoming HTTP requests from a fixed 8 KB stack buffer.
//! Uses string slices (&'a str) that point directly into the buffer.
//! No heap allocations, no copies, O(n) time complexity.
//!
//! Example:
//! ```
//! let buffer = b"GET /index.html HTTP/1.1\r\nHost: localhost\r\n\r\n";
//! let request = parse_request(buffer)?;
//! assert_eq!(request.method, "GET");
//! assert_eq!(request.path, "/index.html");
//! ```

use heapless::Vec;

/// HTTP request parsing error
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpParseError {
    /// Buffer contains invalid UTF-8
    InvalidUtf8,
    /// Request line missing or malformed
    MalformedRequestLine,
    /// No CRLF CRLF separator between headers and body
    NoHeaderBodySeparator,
    /// HTTP method not recognized
    InvalidMethod,
    /// HTTP version not supported
    InvalidHttpVersion,
    /// Header line malformed (missing colon)
    MalformedHeader,
    /// Buffer too small for request
    BufferOverflow,
    /// Empty request
    EmptyRequest,
}

/// Zero-copy HTTP request with lifetime tied to buffer
#[derive(Debug, Clone)]
pub struct HttpRequest<'a> {
    /// HTTP method (GET, POST, etc.)
    pub method: &'a str,
    /// Request path with query string (e.g., "/index.html?page=1")
    pub path: &'a str,
    /// HTTP version (e.g., "HTTP/1.1")
    pub version: &'a str,
    /// Request headers (name, value) pairs
    /// Max 32 headers to prevent header spam attacks
    pub headers: Vec<(&'a str, &'a str), 32>,
    /// Request body as bytes
    pub body: &'a [u8],
}

impl<'a> HttpRequest<'a> {
    /// Get a header value by name (case-insensitive)
    pub fn get_header(&self, name: &str) -> Option<&'a str> {
        for (key, value) in &self.headers {
            if key.eq_ignore_ascii_case(name) {
                return Some(value);
            }
        }
        None
    }

    /// Get Content-Length header parsed as usize
    pub fn content_length(&self) -> Option<usize> {
        self.get_header("Content-Length").and_then(|s| {
            let mut num = 0usize;
            for ch in s.chars() {
                if let Some(digit) = ch.to_digit(10) {
                    num = num * 10 + digit as usize;
                } else {
                    return None;
                }
            }
            Some(num)
        })
    }

    /// Check if connection should remain open (HTTP/1.1 default is keep-alive)
    pub fn is_keep_alive(&self) -> bool {
        if let Some(conn) = self.get_header("Connection") {
            !conn.eq_ignore_ascii_case("close")
        } else {
            // HTTP/1.1 is keep-alive by default
            self.version.contains("1.1")
        }
    }

    /// Check if this is an SBSP script request
    pub fn is_sbsp(&self) -> bool {
        self.path.ends_with(".sbsp")
    }
}

/// Parse an HTTP request from a buffer containing raw bytes
///
/// # Arguments
/// * `buffer` - Slice of bytes read from TCP socket
///
/// # Returns
/// - `Ok(HttpRequest)` - Successfully parsed request with zero-copy slices
/// - `Err(HttpParseError)` - Parsing failed; send appropriate HTTP error response
///
/// # Safety
/// This function does NOT perform any dangerous operations:
/// - No unsafe blocks
/// - Buffer overruns prevented by slice bounds checking
/// - Lifetimes ensure slices don't outlive the buffer
pub fn parse_request(buffer: &[u8]) -> Result<HttpRequest, HttpParseError> {
    if buffer.is_empty() {
        return Err(HttpParseError::EmptyRequest);
    }

    // Convert bytes to string (fail fast on invalid UTF-8)
    let req_str = core::str::from_utf8(buffer).map_err(|_| HttpParseError::InvalidUtf8)?;

    // Split headers and body at "\r\n\r\n"
    let (headers_str, body_str) = match req_str.split_once("\r\n\r\n") {
        Some((h, b)) => (h, b),
        None => {
            // No body separator found; request may still be valid (no body)
            // But we require the separator for protocol compliance
            return Err(HttpParseError::NoHeaderBodySeparator);
        }
    };

    // Split first line from remaining headers
    let mut lines = headers_str.lines();
    let request_line = lines.next().ok_or(HttpParseError::MalformedRequestLine)?;

    // Parse request line: "GET /path HTTP/1.1"
    let mut parts = request_line.split_whitespace();
    let method = parts.next().ok_or(HttpParseError::MalformedRequestLine)?;
    let path = parts.next().ok_or(HttpParseError::MalformedRequestLine)?;
    let version = parts.next().ok_or(HttpParseError::MalformedRequestLine)?;

    // Validate HTTP version
    if !version.starts_with("HTTP/") {
        return Err(HttpParseError::InvalidHttpVersion);
    }

    // Parse headers
    let mut headers = Vec::new();
    for line in lines {
        if line.is_empty() {
            break;
        }

        match line.split_once(':') {
            Some((key, value)) => {
                // Headers are stored with trimmed key and value
                let trimmed_key = key.trim();
                let trimmed_value = value.trim();

                // Reject headers if we've hit the limit
                if headers.push((trimmed_key, trimmed_value)).is_err() {
                    return Err(HttpParseError::BufferOverflow);
                }
            }
            None => {
                return Err(HttpParseError::MalformedHeader);
            }
        }
    }

    Ok(HttpRequest {
        method,
        path,
        version,
        headers,
        body: body_str.as_bytes(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_get() {
        let buffer = b"GET /index.html HTTP/1.1\r\nHost: localhost\r\n\r\n";
        let req = parse_request(buffer).expect("parse failed");

        assert_eq!(req.method, "GET");
        assert_eq!(req.path, "/index.html");
        assert_eq!(req.version, "HTTP/1.1");
        assert_eq!(req.get_header("Host"), Some("localhost"));
        assert!(req.is_keep_alive());
        assert!(!req.is_sbsp());
    }

    #[test]
    fn test_parse_sbsp_request() {
        let buffer =
            b"POST /form.sbsp HTTP/1.1\r\nHost: localhost\r\nContent-Length: 5\r\n\r\nhello";
        let req = parse_request(buffer).expect("parse failed");

        assert_eq!(req.method, "POST");
        assert_eq!(req.path, "/form.sbsp");
        assert!(req.is_sbsp());
        assert_eq!(req.content_length(), Some(5));
        assert_eq!(req.body, b"hello");
    }

    #[test]
    fn test_parse_connection_close() {
        let buffer = b"GET / HTTP/1.0\r\nConnection: close\r\n\r\n";
        let req = parse_request(buffer).expect("parse failed");

        assert!(!req.is_keep_alive());
    }

    #[test]
    fn test_parse_multiple_headers() {
        let buffer =
            b"GET /test HTTP/1.1\r\nHost: example.com\r\nUser-Agent: curl\r\nAccept: */*\r\n\r\n";
        let req = parse_request(buffer).expect("parse failed");

        assert_eq!(req.get_header("Host"), Some("example.com"));
        assert_eq!(req.get_header("User-Agent"), Some("curl"));
        assert_eq!(req.get_header("Accept"), Some("*/*"));
    }

    #[test]
    fn test_parse_invalid_utf8() {
        let buffer = &[0xFF, 0xFE, 0xFD];
        assert_eq!(parse_request(buffer), Err(HttpParseError::InvalidUtf8));
    }

    #[test]
    fn test_parse_empty_request() {
        let buffer = b"";
        assert_eq!(parse_request(buffer), Err(HttpParseError::EmptyRequest));
    }

    #[test]
    fn test_parse_no_separator() {
        let buffer = b"GET / HTTP/1.1\r\n";
        assert_eq!(
            parse_request(buffer),
            Err(HttpParseError::NoHeaderBodySeparator)
        );
    }

    #[test]
    fn test_parse_malformed_request_line() {
        let buffer = b"GET HTTP/1.1\r\n\r\n"; // Missing path
        assert_eq!(
            parse_request(buffer),
            Err(HttpParseError::MalformedRequestLine)
        );
    }

    #[test]
    fn test_parse_invalid_header() {
        let buffer = b"GET / HTTP/1.1\r\nBroken Header\r\n\r\n"; // No colon
        assert_eq!(parse_request(buffer), Err(HttpParseError::MalformedHeader));
    }

    #[test]
    fn test_header_case_insensitive() {
        let buffer = b"GET / HTTP/1.1\r\nhost: example.com\r\nCONTENT-LENGTH: 42\r\n\r\n";
        let req = parse_request(buffer).expect("parse failed");

        // Header names are case-insensitive in HTTP
        assert_eq!(req.get_header("Host"), Some("example.com"));
        assert_eq!(req.get_header("Content-Length"), Some("42"));
    }
}
