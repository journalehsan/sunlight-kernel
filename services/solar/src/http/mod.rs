//! HTTP/1.1 Protocol Implementation with Zero-Allocation Parser
//!
//! Phase 2: High-performance HTTP request parsing using:
//! - Stack-allocated 8 KB buffer (no heap)
//! - String slices (&'a str) pointing directly into buffer
//! - Manual \r\n\r\n scanning (explicit buffer control)
//! - heapless::Vec for headers (max 32 headers per request)
//! - Zero copies, maximum speed, isolation per connection
//!
//! Architecture:
//! - parse_request(): Raw bytes → HttpRequest<'a> (all zero-copy slices)
//! - handle_http_request(): Routing logic (SBSP vs static files)
//! - Per-thread isolation: malformed request → thread exits cleanly
//!
//! Security:
//! - Rust's borrow checker prevents buffer overflows
//! - Heapless collections guarantee stack-only allocation
//! - No unwrap(), all errors handled explicitly
//! - Malformed requests → 400 Bad Request → connection close

pub mod connection;
pub mod form;
pub mod parser;
pub mod request;
pub mod response;

pub use connection::handle_http_request;
pub use form::parse_form_urlencoded;
pub use parser::{parse_request, HttpParseError};
pub use request::{HttpRequest, RequestRouter};
pub use response::HttpResponse;
