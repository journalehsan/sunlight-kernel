//! HTTP/1.1 Response Building
//!
//! Constructs and serializes HTTP responses for static files and SBSP pages.
//! Example:
//!   HTTP/1.1 200 OK
//!   Content-Type: text/html
//!   Content-Length: 1234
//!
//!   <html>...</html>

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

/// HTTP response
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

    /// Set the response body
    pub fn body(&mut self, data: &[u8]) -> &mut Self {
        let _ = self.body.extend_from_slice(data);
        self
    }

    /// Set the response body from a string
    pub fn text(&mut self, text: &str) -> &mut Self {
        let _ = self.body.extend_from_slice(text.as_bytes());
        self
    }

    /// Serialize the response to bytes suitable for TCP write
    pub fn serialize(&self) -> Vec<u8, 16384> {
        let mut buf = Vec::new();

        // Status line: "HTTP/1.1 200 OK\r\n"
        let status_line = heapless::String::<128>::from("");
        let status_str = if let Ok(s) = core::fmt::write(
            &mut {
                struct Buf<'a>(&'a mut heapless::String<128>);
                impl<'a> core::fmt::Write for Buf<'a> {
                    fn write_str(&mut self, s: &str) -> core::fmt::Result {
                        self.0.push_str(s).map_err(|_| core::fmt::Error)
                    }
                }
                Buf(&mut heapless::String::<128>::new())
            },
            format_args!(
                "HTTP/1.1 {} {}\r\n",
                self.status.code(),
                self.status.reason_phrase()
            ),
        ) {
            "HTTP/1.1 ".as_bytes()
        } else {
            b"HTTP/1.1 500 Internal Server Error\r\n"
        };

        let _ = buf.extend_from_slice(status_str);
        // TODO: Complete serialization in Phase 2

        buf
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
