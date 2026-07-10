extern crate alloc;

use alloc::{format, string::String, vec::Vec};
use core::fmt;

use sunlight_http::{HttpError, HttpRequest, ParsedUrl, UrlScheme};

use crate::{BodyFormat, HttpMethod};

const MANAGED_HEADERS: [&str; 4] = ["host", "user-agent", "connection", "content-length"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyValueEntry {
    pub enabled: bool,
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, Copy)]
pub struct BasicAuthInput<'a> {
    pub username: &'a str,
    pub password: &'a str,
}

impl<'a> BasicAuthInput<'a> {
    fn is_enabled(self) -> bool {
        !self.username.is_empty() || !self.password.is_empty()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RequestBuildInput<'a> {
    pub method: HttpMethod,
    pub url_input: &'a str,
    pub parameters: &'a [KeyValueEntry],
    pub headers: &'a [KeyValueEntry],
    pub auth: BasicAuthInput<'a>,
    pub body_format: BodyFormat,
    pub body_text: &'a str,
}

#[derive(Debug, Clone)]
pub struct BuiltRequest {
    pub normalized_url: String,
    pub parsed_url: ParsedUrl,
    pub request: HttpRequest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestBuildError {
    InvalidUrl(String),
    DuplicateHeader(String),
    ManagedHeader(String),
}

impl fmt::Display for RequestBuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidUrl(message) => write!(f, "{message}"),
            Self::DuplicateHeader(name) => write!(f, "duplicate header: {name}"),
            Self::ManagedHeader(name) => write!(f, "header is managed automatically: {name}"),
        }
    }
}

impl From<HttpError> for RequestBuildError {
    fn from(err: HttpError) -> Self {
        Self::InvalidUrl(format!("{err}"))
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

pub fn build_request(input: RequestBuildInput<'_>) -> Result<BuiltRequest, RequestBuildError> {
    let normalized_url = normalize_url_input(input.url_input)?;
    let mut parsed_url = ParsedUrl::parse(&normalized_url)?;
    parsed_url.path = append_query_params(&parsed_url.path, input.parameters);

    let mut headers = Vec::new();
    let mut seen = Vec::<String>::new();
    collect_custom_headers(input.headers, &mut headers, &mut seen)?;

    if input.auth.is_enabled() && !seen.iter().any(|name| name == "authorization") {
        push_unique_header(
            &mut headers,
            &mut seen,
            "authorization",
            &format!(
                "Basic {}",
                encode_base64(&format!("{}:{}", input.auth.username, input.auth.password))
            ),
        )?;
    }

    let body = if input.method.allows_body() && !input.body_text.is_empty() {
        let body = input.body_text.as_bytes().to_vec();
        if !seen.iter().any(|name| name == "content-type") {
            headers.push((
                String::from("content-type"),
                String::from(input.body_format.default_content_type()),
            ));
            seen.push(String::from("content-type"));
        }
        Some(body)
    } else {
        None
    };

    let host = parsed_url.host_header();
    let path = strip_fragment(&parsed_url.path);

    Ok(BuiltRequest {
        normalized_url,
        parsed_url,
        request: HttpRequest {
            method: input.method.as_str(),
            path,
            host,
            headers,
            body,
        },
    })
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

fn collect_custom_headers(
    input: &[KeyValueEntry],
    headers: &mut Vec<(String, String)>,
    seen: &mut Vec<String>,
) -> Result<(), RequestBuildError> {
    for entry in input {
        if !entry.enabled {
            continue;
        }

        let key = entry.key.trim();
        if key.is_empty() {
            continue;
        }

        let normalized = key.to_ascii_lowercase();
        if MANAGED_HEADERS.iter().any(|name| *name == normalized) {
            return Err(RequestBuildError::ManagedHeader(String::from(key)));
        }
        if seen.contains(&normalized) {
            return Err(RequestBuildError::DuplicateHeader(String::from(key)));
        }

        seen.push(normalized.clone());
        headers.push((normalized, String::from(entry.value.as_str())));
    }

    Ok(())
}

fn push_unique_header(
    headers: &mut Vec<(String, String)>,
    seen: &mut Vec<String>,
    key: &str,
    value: &str,
) -> Result<(), RequestBuildError> {
    let normalized = key.to_ascii_lowercase();
    if seen.contains(&normalized) {
        return Err(RequestBuildError::DuplicateHeader(String::from(key)));
    }
    seen.push(normalized.clone());
    headers.push((normalized, String::from(value)));
    Ok(())
}

fn append_query_params(path: &str, params: &[KeyValueEntry]) -> String {
    let (path_without_fragment, fragment) = split_fragment(path);
    let (base_path, existing_query) = match path_without_fragment.split_once('?') {
        Some((base, query)) => (base, query),
        None => (path_without_fragment, ""),
    };

    let mut combined_query = String::from(existing_query);
    for entry in params {
        if !entry.enabled {
            continue;
        }
        let key = entry.key.trim();
        if key.is_empty() {
            continue;
        }
        if !combined_query.is_empty() {
            combined_query.push('&');
        }
        append_query_component(&mut combined_query, key, false);
        combined_query.push('=');
        append_query_component(&mut combined_query, entry.value.as_str(), false);
    }

    if combined_query.is_empty() {
        return rebuild_path(base_path, "", fragment);
    }

    rebuild_path(base_path, &combined_query, fragment)
}

fn append_query_component(out: &mut String, value: &str, form_space_plus: bool) {
    for byte in value.bytes() {
        if is_unreserved(byte) {
            out.push(char::from(byte));
        } else if form_space_plus && byte == b' ' {
            out.push('+');
        } else {
            out.push('%');
            out.push(hex(byte >> 4));
            out.push(hex(byte & 0x0F));
        }
    }
}

fn is_unreserved(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~')
}

fn hex(value: u8) -> char {
    match value {
        0..=9 => char::from(b'0' + value),
        _ => char::from(b'A' + (value - 10)),
    }
}

fn split_fragment(path: &str) -> (&str, &str) {
    match path.split_once('#') {
        Some((path, fragment)) => (path, fragment),
        None => (path, ""),
    }
}

fn rebuild_path(base_path: &str, query: &str, fragment: &str) -> String {
    let mut out = String::from(base_path);
    if !query.is_empty() {
        out.push('?');
        out.push_str(query);
    }
    if !fragment.is_empty() {
        out.push('#');
        out.push_str(fragment);
    }
    out
}

fn strip_fragment(path: &str) -> String {
    String::from(path.split_once('#').map_or(path, |(base, _)| base))
}

fn encode_base64(input: &str) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    let mut index = 0usize;
    while index < bytes.len() {
        let b0 = bytes[index];
        let b1 = *bytes.get(index + 1).unwrap_or(&0);
        let b2 = *bytes.get(index + 2).unwrap_or(&0);

        out.push(char::from(TABLE[(b0 >> 2) as usize]));
        out.push(char::from(TABLE[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize]));

        if index + 1 < bytes.len() {
            out.push(char::from(TABLE[(((b1 & 0x0F) << 2) | (b2 >> 6)) as usize]));
        } else {
            out.push('=');
        }

        if index + 2 < bytes.len() {
            out.push(char::from(TABLE[(b2 & 0x3F) as usize]));
        } else {
            out.push('=');
        }

        index += 3;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn normalize_url_adds_http_scheme() {
        let normalized = normalize_url_input("example.com/demo").unwrap();
        assert_eq!(normalized, "http://example.com/demo");
    }

    #[test]
    fn query_params_are_appended() {
        let built = build_request(RequestBuildInput {
            method: HttpMethod::Get,
            url_input: "https://example.com/api?existing=1",
            parameters: &[KeyValueEntry {
                enabled: true,
                key: String::from("q"),
                value: String::from("hello world"),
            }],
            headers: &[],
            auth: BasicAuthInput {
                username: "",
                password: "",
            },
            body_format: BodyFormat::RawText,
            body_text: "",
        })
        .unwrap();

        assert_eq!(built.request.path, "/api?existing=1&q=hello%20world");
    }

    #[test]
    fn query_params_preserve_fragment_and_duplicate_keys() {
        let built = build_request(RequestBuildInput {
            method: HttpMethod::Get,
            url_input: "https://example.com/api?existing=1#frag",
            parameters: &[
                KeyValueEntry {
                    enabled: true,
                    key: String::from("tag"),
                    value: String::from("a/b"),
                },
                KeyValueEntry {
                    enabled: true,
                    key: String::from("tag"),
                    value: String::from("c d"),
                },
            ],
            headers: &[],
            auth: BasicAuthInput {
                username: "",
                password: "",
            },
            body_format: BodyFormat::RawText,
            body_text: "",
        })
        .unwrap();

        assert_eq!(built.request.path, "/api?existing=1&tag=a%2Fb&tag=c%20d");
        assert_eq!(
            built.parsed_url.path,
            "/api?existing=1&tag=a%2Fb&tag=c%20d#frag"
        );
    }

    #[test]
    fn build_post_request_sets_auth_and_content_type() {
        let built = build_request(RequestBuildInput {
            method: HttpMethod::Post,
            url_input: "http://example.com/submit",
            parameters: &[],
            headers: &[],
            auth: BasicAuthInput {
                username: "dev",
                password: "secret",
            },
            body_format: BodyFormat::Json,
            body_text: "{\"ok\":true}",
        })
        .unwrap();

        assert_eq!(built.request.method, "POST");
        assert_eq!(
            built.request.body.as_deref(),
            Some(b"{\"ok\":true}".as_slice())
        );
        assert_eq!(
            built.request.headers,
            vec![
                (
                    String::from("authorization"),
                    String::from("Basic ZGV2OnNlY3JldA=="),
                ),
                (
                    String::from("content-type"),
                    String::from("application/json"),
                ),
            ]
        );
    }

    #[test]
    fn duplicate_headers_are_rejected() {
        let err = build_request(RequestBuildInput {
            method: HttpMethod::Get,
            url_input: "http://example.com",
            parameters: &[],
            headers: &[
                KeyValueEntry {
                    enabled: true,
                    key: String::from("Accept"),
                    value: String::from("application/json"),
                },
                KeyValueEntry {
                    enabled: true,
                    key: String::from("accept"),
                    value: String::from("text/plain"),
                },
            ],
            auth: BasicAuthInput {
                username: "",
                password: "",
            },
            body_format: BodyFormat::RawText,
            body_text: "",
        })
        .unwrap_err();

        assert_eq!(
            err,
            RequestBuildError::DuplicateHeader(String::from("accept"))
        );
    }

    #[test]
    fn managed_headers_are_rejected() {
        let err = build_request(RequestBuildInput {
            method: HttpMethod::Get,
            url_input: "http://example.com",
            parameters: &[],
            headers: &[KeyValueEntry {
                enabled: true,
                key: String::from("Host"),
                value: String::from("override"),
            }],
            auth: BasicAuthInput {
                username: "",
                password: "",
            },
            body_format: BodyFormat::RawText,
            body_text: "",
        })
        .unwrap_err();

        assert_eq!(err, RequestBuildError::ManagedHeader(String::from("Host")));
    }

    #[test]
    fn explicit_authorization_header_wins_over_generated_basic_auth() {
        let built = build_request(RequestBuildInput {
            method: HttpMethod::Get,
            url_input: "http://example.com",
            parameters: &[],
            headers: &[KeyValueEntry {
                enabled: true,
                key: String::from("Authorization"),
                value: String::from("Bearer token"),
            }],
            auth: BasicAuthInput {
                username: "dev",
                password: "secret",
            },
            body_format: BodyFormat::RawText,
            body_text: "",
        })
        .unwrap();

        assert_eq!(
            built.request.headers,
            vec![(String::from("authorization"), String::from("Bearer token"))]
        );
    }

    #[test]
    fn get_and_head_requests_do_not_send_bodies() {
        for method in [HttpMethod::Get, HttpMethod::Head] {
            let built = build_request(RequestBuildInput {
                method,
                url_input: "http://example.com",
                parameters: &[],
                headers: &[],
                auth: BasicAuthInput {
                    username: "",
                    password: "",
                },
                body_format: BodyFormat::Json,
                body_text: "{\"ignored\":true}",
            })
            .unwrap();

            assert!(built.request.body.is_none());
            assert!(built
                .request
                .headers
                .iter()
                .all(|(key, _)| key != "content-type"));
        }
    }
}
