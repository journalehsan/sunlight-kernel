//! HTTP/1.1 Protocol Implementation
//!
//! Phase 2: HTTP request parsing, response building, static file serving
//! - Request line: GET /path HTTP/1.1
//! - Header parsing: Content-Length, Transfer-Encoding: chunked
//! - Keep-alive connection handling
//! - Static file streaming with proper Content-Type detection

pub mod request;
pub mod response;

pub use request::HttpRequest;
pub use response::HttpResponse;
