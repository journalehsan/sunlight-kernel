//! HTTP/1.1 Request Parsing
//!
//! Parses incoming HTTP requests into structured data.
//! Example:
//!   GET /index.html HTTP/1.1
//!   Host: localhost:8080
//!   Content-Length: 42

use heapless::{String, Vec};

/// Maximum request line length (e.g., "GET /path/to/file.html?query=value HTTP/1.1")
const MAX_REQUEST_LINE: usize = 512;
/// Maximum number of headers per request
const MAX_HEADERS: usize = 32;
/// Maximum header value length
const MAX_HEADER_VALUE: usize = 256;

/// HTTP request method
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Delete,
    Head,
    Options,
}

impl HttpMethod {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "GET" => Some(HttpMethod::Get),
            "POST" => Some(HttpMethod::Post),
            "PUT" => Some(HttpMethod::Put),
            "DELETE" => Some(HttpMethod::Delete),
            "HEAD" => Some(HttpMethod::Head),
            "OPTIONS" => Some(HttpMethod::Options),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            HttpMethod::Get => "GET",
            HttpMethod::Post => "POST",
            HttpMethod::Put => "PUT",
            HttpMethod::Delete => "DELETE",
            HttpMethod::Head => "HEAD",
            HttpMethod::Options => "OPTIONS",
        }
    }
}

/// HTTP request
#[derive(Debug, Clone)]
pub struct HttpRequest {
    pub method: HttpMethod,
    pub path: String<512>,
    pub http_version: String<16>,
    pub headers: Vec<(String<64>, String<256>), 32>,
    pub body: Vec<u8, 4096>,
}

impl HttpRequest {
    pub fn new() -> Self {
        Self {
            method: HttpMethod::Get,
            path: String::new(),
            http_version: String::from("HTTP/1.1"),
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    /// Get a header value by name (case-insensitive)
    pub fn get_header(&self, name: &str) -> Option<&str> {
        for (key, value) in &self.headers {
            if key.eq_ignore_ascii_case(name) {
                return Some(value);
            }
        }
        None
    }

    /// Get Content-Length if present
    pub fn content_length(&self) -> Option<usize> {
        self.get_header("Content-Length")
            .and_then(|s| s.parse::<usize>().ok())
    }

    /// Check if this is a keep-alive connection (default for HTTP/1.1)
    pub fn is_keep_alive(&self) -> bool {
        if let Some(conn) = self.get_header("Connection") {
            !conn.eq_ignore_ascii_case("close")
        } else {
            true // HTTP/1.1 default is keep-alive
        }
    }
}
