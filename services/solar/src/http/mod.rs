//! HTTP/1.1 Protocol Implementation with Zero-Allocation Parser
//!
//! Phase 2: High-performance HTTP request parsing using:
//! - Stack-allocated 8 KB buffer (no heap)
//! - String slices (&'a str) pointing directly into buffer
//! - Simple state machine (split on \r\n and \r\n\r\n)
//! - heapless::Vec for headers (max 32 headers per request)
//! - Zero copies, maximum speed, isolation per connection
//!
//! Security:
//! - Rust's borrow checker prevents buffer overflows
//! - Malformed requests → clean error response → thread exits
//! - No unwrap(), all errors handled explicitly

pub mod request;
pub mod response;
pub mod parser;

pub use request::HttpRequest;
pub use response::HttpResponse;
pub use parser::{parse_request, HttpParseError};
