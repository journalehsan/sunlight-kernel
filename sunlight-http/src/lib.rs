//! sunlight-http: Shared HTTP/1.1 types and client facade for SunlightOS.
//!
//! This crate contains the pure, reusable HTTP protocol logic:
//! - URL parsing
//! - HttpRequest serialization (wire format)
//! - HttpResponse header parsing
//! - Helpers: content_length, accepts_ranges, header access, host_header, infer_filename
//!
//! Transport, redirects, body collection, CLI, file I/O etc. remain in consumers
//! (e.g. sunlight-fetch).
//!
//! The crate is always `#![no_std]` + `alloc`.

#![no_std]
#![forbid(unsafe_op_in_unsafe_fn)]
#![deny(clippy::all, clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

#[cfg(test)]
extern crate std;

extern crate alloc;

use alloc::{format, string::String, vec::Vec};

/// URL scheme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UrlScheme {
    Http,
    Https,
}

/// Parsed URL — HTTP and HTTPS.
#[derive(Debug, Clone)]
pub struct ParsedUrl {
    pub scheme: UrlScheme,
    pub host: String,
    pub port: u16,
    pub path: String,
}

impl ParsedUrl {
    /// Parse a URL string into components.
    ///
    /// Supports: `http://host[:port][/path]` and `https://host[:port][/path]`
    /// Query strings are preserved in `path` (e.g. `/file?token=1`).
    pub fn parse(url: &str) -> Result<Self, HttpError> {
        let (scheme, rest) = if let Some(rest) = url.strip_prefix("https://") {
            (UrlScheme::Https, rest)
        } else if let Some(rest) = url.strip_prefix("http://") {
            (UrlScheme::Http, rest)
        } else {
            return Err(HttpError::InvalidUrl(format!(
                "only http:// and https:// URLs supported, got: {url}"
            )));
        };

        if rest.is_empty() {
            return Err(HttpError::InvalidUrl(String::from("empty host")));
        }

        // Split host+port from path at first '/'
        let (host_port, path) = match rest.find('/') {
            Some(idx) => (&rest[..idx], &rest[idx..]),
            None => (rest, "/"),
        };

        // Split host from port
        let (host, port) = if let Some(colon_idx) = host_port.rfind(':') {
            let host_part = &host_port[..colon_idx];
            let port_str = &host_port[colon_idx + 1..];
            let port = port_str
                .parse::<u16>()
                .map_err(|_| HttpError::InvalidUrl(format!("invalid port: '{port_str}'")))?;
            (host_part, port)
        } else {
            let default_port = match scheme {
                UrlScheme::Http => 80,
                UrlScheme::Https => 443,
            };
            (host_port, default_port)
        };

        if host.is_empty() {
            return Err(HttpError::InvalidUrl(String::from("empty hostname")));
        }

        Ok(Self {
            scheme,
            host: String::from(host),
            port,
            path: String::from(path),
        })
    }

    pub fn uses_tls(&self) -> bool {
        self.scheme == UrlScheme::Https
    }

    /// Infer a filename from the URL path.
    /// `/some/path/file.tar.gz` → `file.tar.gz`
    /// `/` or empty → `index.html`
    pub fn infer_filename(&self) -> String {
        let path = self.path.trim_end_matches('/');
        let path = path.split('?').next().unwrap_or(path);

        if path.is_empty() || path == "/" {
            return String::from("index.html");
        }

        // Take the last path segment
        match path.rfind('/') {
            Some(idx) => {
                let name = &path[idx + 1..];
                if name.is_empty() {
                    String::from("index.html")
                } else {
                    String::from(name)
                }
            }
            None => String::from(path),
        }
    }

    /// Build the Host header value.
    /// Matches historical behavior: only omits for port 80 (even for https:443).
    pub fn host_header(&self) -> String {
        if self.port == 80 {
            self.host.clone()
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }
}

/// An HTTP request we'll serialize onto the wire.
#[derive(Debug, Clone)]
pub struct HttpRequest {
    pub method: &'static str,
    pub path: String,
    pub host: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
}

impl HttpRequest {
    /// Serialize to wire format (HTTP/1.1).
    /// Always adds Host, a User-Agent, the provided headers, optional Content-Length,
    /// and "Connection: close".
    pub fn serialize(&self) -> Vec<u8> {
        use core::fmt::Write;

        let mut buf = String::with_capacity(512);

        // Request line
        let _ = write!(buf, "{} {} HTTP/1.1\r\n", self.method, self.path);

        // Host header (always first)
        let _ = write!(buf, "Host: {}\r\n", self.host);

        // User-Agent (historical value from fetch)
        let _ = buf.write_str("User-Agent: SunlightOS-fetch/0.1\r\n");

        // Additional headers
        for (key, value) in &self.headers {
            let _ = write!(buf, "{key}: {value}\r\n");
        }

        // Content-Length for bodies
        if let Some(ref body) = self.body {
            let _ = write!(buf, "Content-Length: {}\r\n", body.len());
        }

        // Connection close — we don't do keep-alive yet
        let _ = buf.write_str("Connection: close\r\n");

        // End of headers
        let _ = buf.write_str("\r\n");

        let mut result: Vec<u8> = buf.into_bytes();

        // Append body
        if let Some(ref body) = self.body {
            result.extend_from_slice(body);
        }

        result
    }
}

/// Parsed HTTP response header.
#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status_code: u16,
    pub status_text: String,
    pub headers: Vec<(String, String)>,
    pub header_len: usize, // bytes consumed by headers (for body offset)
}

impl HttpResponse {
    /// Parse response headers from raw bytes.
    /// Returns None if headers aren't complete yet (no \r\n\r\n found).
    pub fn parse(data: &[u8]) -> Option<Result<Self, HttpError>> {
        // Find end of headers
        let header_end = find_header_end(data)?;
        let header_bytes = &data[..header_end];

        // Parse as UTF-8 (HTTP headers are ASCII-compatible)
        let header_str = match core::str::from_utf8(header_bytes) {
            Ok(s) => s,
            Err(_) => {
                return Some(Err(HttpError::Protocol(String::from(
                    "non-UTF8 response headers",
                ))));
            }
        };

        let mut lines = header_str.split("\r\n");

        // Status line: "HTTP/1.1 200 OK"
        let status_line = match lines.next() {
            Some(l) => l,
            None => {
                return Some(Err(HttpError::Protocol(String::from("empty response"))));
            }
        };

        let (status_code, status_text) = match parse_status_line(status_line) {
            Ok(v) => v,
            Err(e) => return Some(Err(e)),
        };

        // Parse headers (keys lowercased for lookup convenience)
        let mut headers = Vec::new();
        for line in lines {
            if line.is_empty() {
                break;
            }
            if let Some(colon_idx) = line.find(':') {
                let key = line[..colon_idx].trim();
                let value = line[colon_idx + 1..].trim();
                headers.push((
                    String::from(key).to_ascii_lowercase_sunlight(),
                    String::from(value),
                ));
            }
        }

        Some(Ok(Self {
            status_code,
            status_text,
            headers,
            header_len: header_end + 4, // +4 for \r\n\r\n
        }))
    }

    /// Get a header value by lowercase key.
    pub fn header(&self, key: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// Parse Content-Length header.
    pub fn content_length(&self) -> Option<usize> {
        self.header("content-length")
            .and_then(|v| v.parse::<usize>().ok())
    }

    /// Check if server supports Range requests.
    pub fn accepts_ranges(&self) -> bool {
        self.header("accept-ranges")
            .map(|v| v.contains("bytes"))
            .unwrap_or(false)
    }
}

/// Minimal error type for the HTTP layer (reusable across backends).
#[derive(Debug, Clone)]
pub enum HttpError {
    /// Argument / URL parsing failure
    InvalidUrl(String),

    /// Network / transport level failure (connection, send/recv, DNS etc.)
    Transport(String),

    /// HTTP protocol / framing / parse error
    Protocol(String),

    /// Non-success status (use for cases where caller wants to surface it)
    Status { code: u16, text: String },

    /// TLS/HTTPS requested but not supported by this backend (policy: clean error)
    UnsupportedHttps,

    /// Other
    Other(String),
}

impl core::fmt::Display for HttpError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            HttpError::InvalidUrl(msg) => write!(f, "invalid url: {msg}"),
            HttpError::Transport(msg) => write!(f, "transport error: {msg}"),
            HttpError::Protocol(msg) => write!(f, "http protocol error: {msg}"),
            HttpError::Status { code, text } => write!(f, "http status {code}: {text}"),
            HttpError::UnsupportedHttps => write!(f, "https not supported by this backend"),
            HttpError::Other(msg) => write!(f, "{msg}"),
        }
    }
}

/// Find the `\r\n\r\n` boundary in raw bytes.
/// Returns the offset of the first `\r` in `\r\n\r\n`.
fn find_header_end(data: &[u8]) -> Option<usize> {
    if data.len() < 4 {
        return None;
    }
    for i in 0..data.len() - 3 {
        if &data[i..i + 4] == b"\r\n\r\n" {
            return Some(i);
        }
    }
    None
}

/// Parse "HTTP/1.1 200 OK" → (200, "OK")
fn parse_status_line(line: &str) -> Result<(u16, String), HttpError> {
    let mut parts = line.splitn(3, ' ');
    let _version = parts.next(); // "HTTP/1.1"
    let code_str = parts
        .next()
        .ok_or_else(|| HttpError::Protocol(String::from("missing status code in response")))?;
    let text = parts.next().unwrap_or("");

    let code = code_str
        .parse::<u16>()
        .map_err(|_| HttpError::Protocol(format!("invalid status code: '{code_str}'")))?;

    Ok((code, String::from(text)))
}

/// Extension trait for ASCII lowercase — avoids pulling in unicode tables.
trait AsciiLowercase {
    fn to_ascii_lowercase_sunlight(&self) -> String;
}

impl AsciiLowercase for String {
    fn to_ascii_lowercase_sunlight(&self) -> String {
        let mut s = self.clone();
        // SAFETY: We only modify ASCII uppercase bytes (0x41–0x5A) to lowercase
        // (0x61–0x7A). These are single-byte UTF-8 characters, so modifying them
        // in place preserves UTF-8 validity.
        unsafe {
            for byte in s.as_bytes_mut() {
                if *byte >= b'A' && *byte <= b'Z' {
                    *byte += 32;
                }
            }
        }
        s
    }
}

/// Placeholder for the recommended high-level shape. Real `get` (and transport)
/// is supplied by a backend such as sunlight-fetch.
pub fn get(_url: &str) -> Result<HttpResponse, HttpError> {
    Err(HttpError::Other(String::from(
        "get() requires a concrete backend; see sunlight-fetch or future consumers",
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn test_url_parse_basic() {
        let url = ParsedUrl::parse("http://example.com/file.txt").unwrap();
        assert_eq!(url.host, "example.com");
        assert_eq!(url.port, 80);
        assert_eq!(url.path, "/file.txt");
    }

    #[test]
    fn test_url_parse_with_port() {
        let url = ParsedUrl::parse("http://localhost:8080/api").unwrap();
        assert_eq!(url.host, "localhost");
        assert_eq!(url.port, 8080);
        assert_eq!(url.path, "/api");
    }

    #[test]
    fn test_url_parse_no_path() {
        let url = ParsedUrl::parse("http://example.com").unwrap();
        assert_eq!(url.path, "/");
    }

    #[test]
    fn test_filename_inference() {
        let url = ParsedUrl::parse("http://x.com/path/file.tar.gz").unwrap();
        assert_eq!(url.infer_filename(), "file.tar.gz");

        let url = ParsedUrl::parse("http://x.com/").unwrap();
        assert_eq!(url.infer_filename(), "index.html");

        let url = ParsedUrl::parse("http://x.com").unwrap();
        assert_eq!(url.infer_filename(), "index.html");
    }

    #[test]
    fn test_https_parse() {
        let url = ParsedUrl::parse("https://example.com/file").unwrap();
        assert_eq!(url.scheme, UrlScheme::Https);
        assert_eq!(url.port, 443);
        assert!(url.uses_tls());
    }

    #[test]
    fn test_query_in_path() {
        let url = ParsedUrl::parse("https://x.com/a/b.zip?token=1").unwrap();
        assert_eq!(url.path, "/a/b.zip?token=1");
        assert_eq!(url.infer_filename(), "b.zip");
    }

    #[test]
    fn test_request_serialize_get() {
        // Note: host_header only special-cases 80, so https default would include port.
        let mut req = HttpRequest {
            method: "GET",
            path: String::from("/"),
            host: String::from("example.com"),
            headers: vec![(String::from("accept"), String::from("*/*"))],
            body: None,
        };
        // Simulate what downloader does
        let wire = req.serialize();
        let s = core::str::from_utf8(&wire).unwrap();
        assert!(s.starts_with("GET / HTTP/1.1\r\n"));
        assert!(s.contains("Host: example.com\r\n"));
        assert!(s.contains("User-Agent: SunlightOS-fetch/0.1\r\n"));
        assert!(s.contains("accept: */*\r\n"));
        assert!(s.contains("Connection: close\r\n\r\n"));
        assert!(!s.contains("Content-Length"));
    }

    #[test]
    fn test_request_serialize_post_with_body() {
        let body = b"foo=bar".to_vec();
        let req = HttpRequest {
            method: "POST",
            path: String::from("/submit"),
            host: String::from("example.com"),
            headers: vec![(
                String::from("content-type"),
                String::from("application/x-www-form-urlencoded"),
            )],
            body: Some(body),
        };
        let wire = req.serialize();
        let s = core::str::from_utf8(&wire).unwrap();
        assert!(s.starts_with("POST /submit HTTP/1.1\r\n"));
        assert!(s.contains("Content-Length: 7\r\n"));
        assert!(s.ends_with("\r\n\r\nfoo=bar") || s.contains("foo=bar"));
    }

    #[test]
    fn test_response_parse_simple() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello";
        let resp = HttpResponse::parse(raw).unwrap().unwrap();
        assert_eq!(resp.status_code, 200);
        assert_eq!(resp.status_text, "OK");
        assert_eq!(resp.content_length(), Some(5));
        assert_eq!(resp.header_len, 38);
        assert_eq!(resp.header("content-length"), Some("5"));
    }

    #[test]
    fn test_response_headers_lowercased() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nAccept-Ranges: bytes\r\n\r\n";
        let resp = HttpResponse::parse(raw).unwrap().unwrap();
        assert_eq!(resp.header("content-type"), Some("text/html"));
        // keys are lowercased at parse time, so lookup must use lower
        assert_eq!(resp.header("content-type"), Some("text/html"));
        assert!(resp.accepts_ranges());
    }

    #[test]
    fn test_response_parse_incomplete() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n";
        assert!(HttpResponse::parse(raw).is_none());
    }

    #[test]
    fn test_malformed_response_status() {
        let raw = b"HTTP/1.1 ABC OK\r\n\r\n";
        let res = HttpResponse::parse(raw).unwrap();
        assert!(res.is_err());
        match res.unwrap_err() {
            HttpError::Protocol(msg) => assert!(msg.contains("invalid status code")),
            _ => panic!("expected Protocol error"),
        }
    }

    #[test]
    fn test_invalid_url_error() {
        let res = ParsedUrl::parse("ftp://example.com");
        assert!(res.is_err());
        match res.unwrap_err() {
            HttpError::InvalidUrl(_) => {}
            _ => panic!("expected InvalidUrl"),
        }
    }

    #[test]
    fn test_unsupported_https_error() {
        let err = HttpError::UnsupportedHttps;
        let msg = format!("{err}");
        assert!(msg.contains("https not supported"));
    }
}
