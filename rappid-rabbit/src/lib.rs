#![cfg_attr(not(test), no_std)]

extern crate alloc;

use alloc::{format, string::String, vec::Vec};

use sunlight_http::{HttpError, HttpRequest, ParsedUrl, UrlScheme};

pub mod developer_tools;
pub mod document_lifecycle;
pub mod resources;

pub const MAX_DISCOVERED_RESOURCES: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceLink {
    pub kind: &'static str,
    pub url: String,
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

pub fn build_get_request(url: &ParsedUrl) -> HttpRequest {
    HttpRequest {
        method: "GET",
        path: url.path.clone(),
        host: url.host_header(),
        headers: Vec::new(),
        body: None,
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

pub fn looks_like_html(content_type: Option<&str>, body_text: &str) -> bool {
    if let Some(content_type) = content_type {
        if content_type.to_ascii_lowercase().contains("text/html") {
            return true;
        }
    }

    contains_ascii_case_insensitive(body_text, "<html")
        || contains_ascii_case_insensitive(body_text, "<!doctype html")
        || contains_ascii_case_insensitive(body_text, "<body")
}

pub fn scan_html_resources(html: &str) -> Vec<ResourceLink> {
    let mut resources = Vec::new();
    scan_tag_attr(html, "a", "href", "a", &mut resources);
    scan_tag_attr(html, "img", "src", "img", &mut resources);
    scan_tag_attr(html, "link", "href", "link", &mut resources);
    scan_tag_attr(html, "script", "src", "script", &mut resources);
    scan_tag_attr(html, "form", "action", "form", &mut resources);
    resources
}

fn scan_tag_attr(
    html: &str,
    tag: &str,
    attr: &str,
    kind: &'static str,
    out: &mut Vec<ResourceLink>,
) {
    let mut offset = 0usize;
    let tag_open = format!("<{tag}");
    while out.len() < MAX_DISCOVERED_RESOURCES {
        let Some(start_rel) = find_ascii_case_insensitive(&html[offset..], &tag_open) else {
            break;
        };
        let start = offset + start_rel;
        let Some(end_rel) = html[start..].find('>') else {
            break;
        };
        let end = start + end_rel + 1;
        if let Some(value) = extract_attr_value(&html[start..end], attr) {
            out.push(ResourceLink { kind, url: value });
        }
        offset = end;
    }
}

fn extract_attr_value(element: &str, attr: &str) -> Option<String> {
    let mut offset = 0usize;
    while let Some(found_rel) = find_ascii_case_insensitive(&element[offset..], attr) {
        let start = offset + found_rel;
        let before_ok = start == 0
            || !element
                .as_bytes()
                .get(start.wrapping_sub(1))
                .copied()
                .unwrap_or_default()
                .is_ascii_alphanumeric();
        let after = start + attr.len();
        let after_ok = after >= element.len()
            || !element
                .as_bytes()
                .get(after)
                .copied()
                .unwrap_or_default()
                .is_ascii_alphanumeric();
        if !before_ok || !after_ok {
            offset = after;
            continue;
        }

        let bytes = element.as_bytes();
        let mut idx = after;
        while let Some(byte) = bytes.get(idx) {
            if byte.is_ascii_whitespace() {
                idx += 1;
                continue;
            }
            break;
        }
        if bytes.get(idx).copied() != Some(b'=') {
            offset = after;
            continue;
        }
        idx += 1;
        while let Some(byte) = bytes.get(idx) {
            if byte.is_ascii_whitespace() {
                idx += 1;
                continue;
            }
            break;
        }

        let first = *bytes.get(idx)?;
        if first == b'"' || first == b'\'' {
            idx += 1;
            let end = element[idx..].find(first as char)?;
            return Some(String::from(&element[idx..idx + end]));
        }

        let start_value = idx;
        while let Some(byte) = bytes.get(idx) {
            if byte.is_ascii_whitespace() || *byte == b'>' {
                break;
            }
            idx += 1;
        }
        if idx > start_value {
            return Some(String::from(&element[start_value..idx]));
        }
        offset = after;
    }
    None
}

fn contains_ascii_case_insensitive(haystack: &str, needle: &str) -> bool {
    find_ascii_case_insensitive(haystack, needle).is_some()
}

fn find_ascii_case_insensitive(haystack: &str, needle: &str) -> Option<usize> {
    let haystack_bytes = haystack.as_bytes();
    let needle_bytes = needle.as_bytes();
    if needle_bytes.is_empty() {
        return Some(0);
    }
    if haystack_bytes.len() < needle_bytes.len() {
        return None;
    }

    for start in 0..=haystack_bytes.len() - needle_bytes.len() {
        if haystack_bytes[start..start + needle_bytes.len()]
            .iter()
            .zip(needle_bytes.iter())
            .all(|(left, right)| left.eq_ignore_ascii_case(right))
        {
            return Some(start);
        }
    }
    None
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
    fn normalize_url_lowercases_http_scheme() {
        let normalized = normalize_url_input("HTTPS://example.com/").unwrap();
        assert_eq!(normalized, "https://example.com/");
    }

    #[test]
    fn invalid_url_does_not_panic() {
        let normalized = normalize_url_input("ftp://example.com").unwrap();
        let result = ParsedUrl::parse(&normalized);
        assert!(matches!(result, Err(HttpError::InvalidUrl(_))));
    }

    #[test]
    fn extract_resources_from_html() {
        let html = r#"
            <html>
                <a href="/docs">Docs</a>
                <img src="hero.png">
                <link href="/site.css" rel="stylesheet">
                <script src="/app.js"></script>
                <form action="/submit" method="post"></form>
            </html>
        "#;
        let links = scan_html_resources(html);
        let pairs: Vec<(&str, &str)> = links
            .iter()
            .map(|item| (item.kind, item.url.as_str()))
            .collect();
        assert_eq!(
            pairs,
            vec![
                ("a", "/docs"),
                ("img", "hero.png"),
                ("link", "/site.css"),
                ("script", "/app.js"),
                ("form", "/submit")
            ]
        );
    }
}
