//! Re-exports pure HTTP protocol types from sunlight-http.
//!
//! sunlight-fetch keeps its higher level orchestration (redirects, body reading,
//! transport via ipc, file output, etc.) here and in downloader/ipc.
//!
//! This module preserves the historical API surface used by the rest of fetch
//! (so call sites using `?` and FetchResult continue to work via From).

pub use sunlight_http::{HttpRequest, HttpResponse, ParsedUrl, UrlScheme};

use crate::error::FetchError;
use crate::prelude::String;

/// Bridge sunlight-http errors into fetch's FetchError so that existing `?` usage
/// and `HttpResponse::parse(...) ?` patterns continue to compile and behave
/// without changing call sites in downloader.rs and ipc.rs.
impl From<sunlight_http::HttpError> for FetchError {
    fn from(e: sunlight_http::HttpError) -> Self {
        match e {
            sunlight_http::HttpError::InvalidUrl(msg) => FetchError::InvalidUrl(msg),
            sunlight_http::HttpError::Protocol(msg) => FetchError::HttpError { status: 0, message: msg },
            sunlight_http::HttpError::Status { code, text } => FetchError::HttpError { status: code, message: text },
            sunlight_http::HttpError::Transport(msg) => FetchError::IoError(msg), // transport errors surface as I/O at this layer
            sunlight_http::HttpError::UnsupportedHttps => FetchError::HttpError {
                status: 0,
                message: String::from("https not supported by backend"),
            },
            sunlight_http::HttpError::Other(msg) => FetchError::HttpError { status: 0, message: msg },
        }
    }
}

// Note: ParsedUrl::parse, HttpResponse::parse etc. are re-exported directly.
// Their error types convert via the From impl above when used with `?` in
// FetchResult contexts.

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
}
