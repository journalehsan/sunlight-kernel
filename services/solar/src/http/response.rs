//! HTTP/1.1 Response Building
//!
//! Constructs HTTP responses for static files and SBSP pages.
//! Minimal allocation: uses stack-allocated buffers for small responses.

use heapless::{String, Vec};

/// HTTP status codes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpStatus {
    Ok = 200,
    Created = 201,
    NoContent = 204,
    BadRequest = 400,
    NotFound = 404,
    MethodNotAllowed = 405,
    InternalServerError = 500,
}

impl HttpStatus {
    pub fn reason_phrase(&self) -> &'static str {
        match self {
            HttpStatus::Ok => "OK",
            HttpStatus::Created => "Created",
            HttpStatus::NoContent => "No Content",
            HttpStatus::BadRequest => "Bad Request",
            HttpStatus::NotFound => "Not Found",
            HttpStatus::MethodNotAllowed => "Method Not Allowed",
            HttpStatus::InternalServerError => "Internal Server Error",
        }
    }

    pub fn code(&self) -> u16 {
        *self as u16
    }
}

/// HTTP response builder
#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: HttpStatus,
    pub headers: Vec<(String<64>, String<256>), 32>,
    pub body: Vec<u8, 8192>,
}

impl HttpResponse {
    pub fn new(status: HttpStatus) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    /// Create a 200 OK response
    pub fn ok() -> Self {
        Self::new(HttpStatus::Ok)
    }

    /// Create a 404 Not Found response
    pub fn not_found() -> Self {
        Self::new(HttpStatus::NotFound)
    }

    /// Create a 500 Internal Server Error response
    pub fn internal_error() -> Self {
        Self::new(HttpStatus::InternalServerError)
    }

    /// Set a response header
    pub fn header(&mut self, key: &str, value: &str) -> &mut Self {
        let _ = self.headers.push((
            String::from(key),
            String::from(value),
        ));
        self
    }

    /// Set Content-Type header
    pub fn content_type(&mut self, mime: &str) -> &mut Self {
        self.header("Content-Type", mime)
    }

    /// Set the response body from bytes
    pub fn body(&mut self, data: &[u8]) -> &mut Self {
        let _ = self.body.extend_from_slice(data);
        self
    }

    /// Set the response body from a string
    pub fn text(&mut self, text: &str) -> &mut Self {
        let _ = self.body.extend_from_slice(text.as_bytes());
        self
    }

    /// Get Content-Length
    pub fn content_length(&self) -> usize {
        self.body.len()
    }
}

/// Detect MIME type by file extension
pub fn mime_type_for_path(path: &str) -> &'static str {
    if path.ends_with(".html") || path.ends_with(".htm") {
        "text/html"
    } else if path.ends_with(".css") {
        "text/css"
    } else if path.ends_with(".js") {
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
    } else {
        "application/octet-stream"
    }
}

/// Quick response builders for common cases (no allocation)
pub mod quick {
    /// Build a simple 404 response
    pub fn not_found_response() -> &'static [u8] {
        b"HTTP/1.1 404 Not Found\r\nContent-Type: text/plain\r\nContent-Length: 9\r\nConnection: close\r\n\r\nNot Found"
    }

    /// Build a simple 400 response
    pub fn bad_request_response() -> &'static [u8] {
        b"HTTP/1.1 400 Bad Request\r\nContent-Type: text/plain\r\nContent-Length: 11\r\nConnection: close\r\n\r\nBad Request"
    }

    /// Build a simple 500 response
    pub fn server_error_response() -> &'static [u8] {
        b"HTTP/1.1 500 Internal Server Error\r\nContent-Type: text/plain\r\nContent-Length: 21\r\nConnection: close\r\n\r\nInternal Server Error"
    }
}
