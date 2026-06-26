//! HTTP Request Types and Utilities
//!
//! Re-exports the zero-copy parser types and provides helper utilities
//! for routing and handling different request types.

pub use super::parser::{parse_request, HttpParseError, HttpRequest};

/// HTTP request method enum (for quick matching without string comparisons)
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
    /// Parse HTTP method from string
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

    /// Get the string representation
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

/// Routing helpers for HTTP requests
pub trait RequestRouter {
    /// Check if request is for a static file
    fn is_static(&self) -> bool;

    /// Check if request is for an SBSP script
    fn is_sbsp(&self) -> bool;

    /// Get the requested path without leading slash
    fn path_without_slash(&self) -> &str;
}

impl<'a> RequestRouter for HttpRequest<'a> {
    fn is_static(&self) -> bool {
        !self.is_sbsp()
    }

    fn is_sbsp(&self) -> bool {
        self.path.ends_with(".sbsp")
    }

    fn path_without_slash(&self) -> &str {
        self.path.trim_start_matches('/')
    }
}
