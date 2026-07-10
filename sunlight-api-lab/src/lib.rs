#![cfg_attr(not(test), no_std)]

#[cfg(test)]
extern crate std;

extern crate alloc;

use alloc::{format, string::String, vec::Vec};

use sunlight_http::{HttpError, HttpRequest, ParsedUrl, UrlScheme};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Post,
}

impl HttpMethod {
    pub const ALL: [Self; 2] = [Self::Get, Self::Post];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Get => Self::Post,
            Self::Post => Self::Get,
        }
    }
}

pub fn normalize_url_input(input: &str) -> Result<String, HttpError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(HttpError::InvalidUrl(String::from("enter a URL")));
    }

    if let Some(idx) = trimmed.find("://") {
        let scheme = &trimmed[..idx];
        let rest = &trimmed[idx + 3..];
        if scheme.eq_ignore_ascii_case("http") {
            return Ok(format!("http://{rest}"));
        }
        if scheme.eq_ignore_ascii_case("https") {
            return Ok(format!("https://{rest}"));
        }
        return Ok(String::from(trimmed));
    }

    Ok(format!("http://{trimmed}"))
}

pub fn build_request(
    method: HttpMethod,
    url: &ParsedUrl,
    content_type: &str,
    body_text: &str,
) -> HttpRequest {
    let mut headers = Vec::new();
    let body = match method {
        HttpMethod::Get => None,
        HttpMethod::Post => {
            let trimmed_body = body_text.trim();
            if !content_type.trim().is_empty() {
                headers.push((
                    String::from("content-type"),
                    String::from(content_type.trim()),
                ));
            } else if !trimmed_body.is_empty() {
                headers.push((String::from("content-type"), String::from("text/plain")));
            }
            if trimmed_body.is_empty() {
                None
            } else {
                Some(trimmed_body.as_bytes().to_vec())
            }
        }
    };

    HttpRequest {
        method: method.as_str(),
        path: url.path.clone(),
        host: url.host_header(),
        headers,
        body,
    }
}

pub fn format_url(url: &ParsedUrl) -> String {
    let scheme = match url.scheme {
        UrlScheme::Http => "http",
        UrlScheme::Https => "https",
    };
    let default_port = match url.scheme {
        UrlScheme::Http => 80,
        UrlScheme::Https => 443,
    };
    if url.port == default_port {
        format!("{scheme}://{}{}", url.host, url.path)
    } else {
        format!("{scheme}://{}:{}{}", url.host, url.port, url.path)
    }
}

pub fn body_is_probably_text(content_type: Option<&str>, body: &[u8]) -> bool {
    if body.is_empty() {
        return true;
    }
    if let Some(content_type) = content_type {
        let lower = content_type.to_ascii_lowercase();
        if lower.starts_with("text/")
            || lower.contains("json")
            || lower.contains("xml")
            || lower.contains("javascript")
            || lower.contains("ecmascript")
            || lower.contains("svg")
            || lower.contains("x-www-form-urlencoded")
        {
            return true;
        }
    }

    let sample = &body[..body.len().min(4096)];
    if sample.contains(&0) {
        return false;
    }

    let printable = sample
        .iter()
        .filter(|byte| matches!(**byte, b'\n' | b'\r' | b'\t' | 0x20..=0x7E))
        .count();
    printable * 100 >= sample.len().saturating_mul(85)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_url_adds_http_scheme() {
        let normalized = normalize_url_input("example.com/demo").unwrap();
        assert_eq!(normalized, "http://example.com/demo");
    }

    #[test]
    fn build_post_request_sets_content_type() {
        let url = ParsedUrl::parse("http://example.com/submit").unwrap();
        let request = build_request(HttpMethod::Post, &url, "", "hello");
        assert_eq!(request.method, "POST");
        assert_eq!(request.body.as_deref(), Some(b"hello".as_slice()));
        assert_eq!(
            request.headers,
            vec![(String::from("content-type"), String::from("text/plain"))]
        );
    }

    #[test]
    fn build_get_request_ignores_body() {
        let url = ParsedUrl::parse("http://example.com/").unwrap();
        let request = build_request(HttpMethod::Get, &url, "text/plain", "ignored");
        assert_eq!(request.method, "GET");
        assert!(request.body.is_none());
        assert!(request.headers.is_empty());
    }
}