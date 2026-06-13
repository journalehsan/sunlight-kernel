//! Minimal HTTP types and URL parser.
//! No external crates — hand-rolled for SunlightOS constraints.

use std::string::String;
use std::vec::Vec;
use crate::error::{FetchError, FetchResult};

/// Parsed URL — only HTTP supported (no TLS in this phase)
#[derive(Debug, Clone)]
pub struct ParsedUrl {
    pub host: String,
    pub port: u16,
    pub path: String,
}

impl ParsedUrl {
    /// Parse a URL string into components.
    ///
    /// Supports: `http://host[:port][/path]`
    /// Does NOT support: https, auth, query params (future work).
    pub fn parse(url: &str) -> FetchResult<Self> {
        // Strip scheme
        let rest = url
            .strip_prefix("http://")
            .ok_or_else(|| FetchError::InvalidUrl(format!(
                "only http:// URLs supported, got: {url}"
            )))?;

        if rest.is_empty() {
            return Err(FetchError::InvalidUrl(String::from("empty host")));
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
            let port = port_str.parse::<u16>().map_err(|_| {
                FetchError::InvalidUrl(format!("invalid port: '{port_str}'"))
            })?;
            (host_part, port)
        } else {
            (host_port, 80)
        };

        if host.is_empty() {
            return Err(FetchError::InvalidUrl(String::from("empty hostname")));
        }

        Ok(Self {
            host: String::from(host),
            port,
            path: String::from(path),
        })
    }

    /// Infer a filename from the URL path.
    /// `/some/path/file.tar.gz` → `file.tar.gz`
    /// `/` or empty → `index.html`
    pub fn infer_filename(&self) -> String {
        let path = self.path.trim_end_matches('/');

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

    /// Build the Host header value
    pub fn host_header(&self) -> String {
        if self.port == 80 {
            self.host.clone()
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }
}

/// An HTTP request we'll serialize onto the wire
#[derive(Debug)]
pub struct HttpRequest {
    pub method: &'static str,
    pub path: String,
    pub host: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
}

impl HttpRequest {
    /// Serialize to wire format (HTTP/1.1)
    pub fn serialize(&self) -> Vec<u8> {
        use core::fmt::Write;

        let mut buf = String::with_capacity(512);

        // Request line
        let _ = write!(buf, "{} {} HTTP/1.1\r\n", self.method, self.path);

        // Host header (always first)
        let _ = write!(buf, "Host: {}\r\n", self.host);

        // User-Agent
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

/// Parsed HTTP response header
#[derive(Debug)]
pub struct HttpResponse {
    pub status_code: u16,
    pub status_text: String,
    pub headers: Vec<(String, String)>,
    pub header_len: usize, // bytes consumed by headers (for body offset)
}

impl HttpResponse {
    /// Parse response headers from raw bytes.
    /// Returns None if headers aren't complete yet (no \r\n\r\n found).
    pub fn parse(data: &[u8]) -> Option<FetchResult<Self>> {
        // Find end of headers
        let header_end = find_header_end(data)?;
        let header_bytes = &data[..header_end];

        // Parse as UTF-8 (HTTP headers are ASCII-compatible)
        let header_str = match core::str::from_utf8(header_bytes) {
            Ok(s) => s,
            Err(_) => {
                return Some(Err(FetchError::HttpError {
                    status: 0,
                    message: String::from("non-UTF8 response headers"),
                }));
            }
        };

        let mut lines = header_str.split("\r\n");

        // Status line: "HTTP/1.1 200 OK"
        let status_line = match lines.next() {
            Some(l) => l,
            None => {
                return Some(Err(FetchError::HttpError {
                    status: 0,
                    message: String::from("empty response"),
                }));
            }
        };

        let (status_code, status_text) = match parse_status_line(status_line) {
            Ok(v) => v,
            Err(e) => return Some(Err(e)),
        };

        // Parse headers
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

    /// Get a header value by lowercase key
    pub fn header(&self, key: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// Parse Content-Length header
    pub fn content_length(&self) -> Option<usize> {
        self.header("content-length")
            .and_then(|v| v.parse::<usize>().ok())
    }

    /// Check if server supports Range requests
    pub fn accepts_ranges(&self) -> bool {
        self.header("accept-ranges")
            .map(|v| v.contains("bytes"))
            .unwrap_or(false)
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
fn parse_status_line(line: &str) -> FetchResult<(u16, String)> {
    let mut parts = line.splitn(3, ' ');
    let _version = parts.next(); // "HTTP/1.1"
    let code_str = parts.next().ok_or_else(|| FetchError::HttpError {
        status: 0,
        message: String::from("missing status code in response"),
    })?;
    let text = parts.next().unwrap_or("");

    let code = code_str.parse::<u16>().map_err(|_| FetchError::HttpError {
        status: 0,
        message: format!("invalid status code: '{code_str}'"),
    })?;

    Ok((code, String::from(text)))
}

/// Extension trait for ASCII lowercase — avoids pulling in unicode tables
trait AsciiLowercase {
    fn to_ascii_lowercase_sunlight(&self) -> String;
}

impl AsciiLowercase for String {
    fn to_ascii_lowercase_sunlight(&self) -> String {
        let mut s = self.clone();
        // SAFETY: We only modify ASCII uppercase bytes (0x41–0x5A) to lowercase
        // (0x61–0x7A). These are single-byte UTF-8 characters, so modifying them
        // in place preserves UTF-8 validity. No safer alternative exists without
        // allocating a new string and copying — and we already own this string.
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn test_https_rejected() {
        assert!(ParsedUrl::parse("https://example.com").is_err());
    }
}
