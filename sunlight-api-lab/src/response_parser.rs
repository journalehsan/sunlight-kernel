extern crate alloc;

use alloc::{borrow::Cow, format, string::String};
use core::fmt::Write;

use sunlight_fetch::backend::RequestResult;
use sunlight_fetch::FetchError;

use crate::json_formatter::pretty_json;
use crate::request_builder::format_url;
use crate::HttpMethod;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeSeverity {
    Quiet,
    Warn,
    Error,
}

#[derive(Debug, Clone)]
pub struct ParsedResponseDisplay {
    pub status_label: String,
    pub body_text: String,
    pub headers_text: String,
    pub details_text: String,
    pub copy_response_text: String,
    pub console_text: String,
    pub console_severity: NoticeSeverity,
}

pub fn parse_response(
    method: HttpMethod,
    requested_url: &str,
    result: RequestResult,
    duration_ms: Option<u64>,
) -> ParsedResponseDisplay {
    let status_code = result.response.status_code;
    let status_text = if result.response.status_text.is_empty() {
        String::from("(no reason phrase)")
    } else {
        result.response.status_text.clone()
    };
    let final_url = result
        .final_url
        .as_ref()
        .map(format_url)
        .unwrap_or_else(|| String::from(requested_url));
    let content_type = result
        .response
        .header("content-type")
        .map_or(String::from("(missing)"), String::from);
    let body_size = result.body.len();
    let headers_text = format_headers(&result.response.headers);
    let body_text = format_body(
        method,
        status_code,
        result.response.header("content-type"),
        &result.body,
    );
    let details_text = format_details(
        status_code,
        &status_text,
        &final_url,
        duration_ms.or(result.duration_ms),
        body_size,
        &content_type,
        &headers_text,
    );
    let copy_response_text = format_copy_response(
        status_code,
        &status_text,
        &final_url,
        duration_ms.or(result.duration_ms),
        body_size,
        &content_type,
        &headers_text,
        &body_text,
    );
    let (mut console_text, mut console_severity) = http_status_notice(status_code, &status_text);
    if console_severity == NoticeSeverity::Quiet && body_text.starts_with("Binary response body") {
        console_text = String::from("Binary response body preview is unavailable.");
        console_severity = NoticeSeverity::Warn;
    }

    ParsedResponseDisplay {
        status_label: format!("HTTP {status_code} {status_text}"),
        body_text,
        headers_text,
        details_text,
        copy_response_text,
        console_text,
        console_severity,
    }
}

pub fn describe_fetch_error(err: &FetchError, duration_ms: Option<u64>) -> ParsedResponseDisplay {
    let (status_label, body_text, console_text, console_severity) = match err {
        FetchError::InvalidUrl(message) => (
            if message.contains("only http:// and https://") {
                String::from("Unsupported URL scheme")
            } else {
                String::from("Invalid URL")
            },
            String::new(),
            format!("Invalid URL: {message}"),
            NoticeSeverity::Warn,
        ),
        FetchError::DnsResolutionFailed(host) => (
            String::from("DNS failed"),
            String::from("The hostname could not be resolved."),
            format!("DNS lookup failed for {host}."),
            NoticeSeverity::Error,
        ),
        FetchError::TlsHandshakeFailed(reason) => (
            String::from("TLS failed"),
            String::from("A secure connection could not be established."),
            format!("TLS handshake failed: {reason}"),
            NoticeSeverity::Error,
        ),
        FetchError::TlsCertExpired => (
            String::from("TLS failed"),
            String::from("The server certificate has expired."),
            String::from("TLS certificate expired."),
            NoticeSeverity::Error,
        ),
        FetchError::ConnectionFailed { host, port, reason } => {
            let lower = reason.to_ascii_lowercase();
            if lower.contains("refused") {
                (
                    String::from("Connection refused"),
                    String::new(),
                    format!("Connection to {host}:{port} was refused."),
                    NoticeSeverity::Error,
                )
            } else if lower.contains("timed out") || lower.contains("timeout") {
                (
                    String::from("Timeout"),
                    String::new(),
                    format!("Connection to {host}:{port} timed out."),
                    NoticeSeverity::Error,
                )
            } else {
                (
                    String::from("Connection failed"),
                    String::new(),
                    format!("Connection to {host}:{port} failed: {reason}"),
                    NoticeSeverity::Error,
                )
            }
        }
        FetchError::IoError(message) | FetchError::IpcError(message) => {
            let lower = message.to_ascii_lowercase();
            if lower.contains("timed out") || lower.contains("timeout") {
                (
                    String::from("Timeout"),
                    String::new(),
                    format!("Request timed out: {message}"),
                    NoticeSeverity::Error,
                )
            } else {
                (
                    String::from("Request failed"),
                    String::new(),
                    String::from(message),
                    NoticeSeverity::Error,
                )
            }
        }
        FetchError::HttpError { status, message } => {
            let lower = message.to_ascii_lowercase();
            let title = if lower.contains("redirect") {
                "Redirect failed"
            } else if lower.contains("chunk")
                || lower.contains("decode")
                || lower.contains("response headers")
            {
                "Response decoding failed"
            } else {
                "HTTP error"
            };
            (
                String::from(title),
                String::new(),
                if *status == 0 {
                    format!("{title}: {message}")
                } else {
                    format!("HTTP {status}: {message}")
                },
                NoticeSeverity::Warn,
            )
        }
        FetchError::Interrupted => (
            String::from("Interrupted"),
            String::new(),
            String::from("Request interrupted."),
            NoticeSeverity::Warn,
        ),
        FetchError::CapabilityDenied { resource, detail } => (
            String::from("Permission denied"),
            String::new(),
            format!("Capability denied for {resource}: {detail}"),
            NoticeSeverity::Error,
        ),
        FetchError::RangeNotSupported => (
            String::from("Request failed"),
            String::new(),
            String::from("Range requests are not supported by the server."),
            NoticeSeverity::Warn,
        ),
        FetchError::VfsError(message) => (
            String::from("Request failed"),
            String::new(),
            format!("VFS error: {message}"),
            NoticeSeverity::Error,
        ),
        FetchError::InvalidArgs(message) => (
            String::from("Invalid request"),
            String::new(),
            String::from(message),
            NoticeSeverity::Warn,
        ),
        FetchError::UnknownContentLength => (
            String::from("Request failed"),
            String::new(),
            String::from("Content-Length is missing."),
            NoticeSeverity::Warn,
        ),
        FetchError::ChunkIntegrityError {
            chunk_id,
            expected,
            got,
        } => (
            String::from("Transfer failed"),
            String::from("The response body was incomplete."),
            format!("Body read failed on chunk {chunk_id}: expected {expected} bytes, got {got}."),
            NoticeSeverity::Error,
        ),
    };

    ParsedResponseDisplay {
        status_label,
        body_text: body_text.clone(),
        headers_text: String::from("(none)"),
        details_text: format_error_details(duration_ms),
        copy_response_text: body_text,
        console_text,
        console_severity,
    }
}

fn format_body(
    method: HttpMethod,
    status_code: u16,
    content_type: Option<&str>,
    body: &[u8],
) -> String {
    if body.is_empty() {
        return String::new();
    }

    if response_has_no_body(method, status_code) {
        return String::new();
    }

    if let Ok(text) = core::str::from_utf8(body) {
        if should_attempt_json_pretty_print(content_type, text) {
            if let Some(pretty) = pretty_json(text) {
                return pretty;
            }
        }
        return String::from(text);
    }

    if body_is_probably_text(content_type, body) {
        let lossy: Cow<'_, str> = String::from_utf8_lossy(body);
        return lossy.into_owned();
    }

    format!(
        "Binary response body ({} bytes).\nContent-Type: {}",
        body.len(),
        content_type.unwrap_or("(missing)")
    )
}

fn format_headers(headers: &[(String, String)]) -> String {
    if headers.is_empty() {
        return String::from("(none)");
    }

    let mut out = String::new();
    for (index, (key, value)) in headers.iter().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        let _ = write!(&mut out, "{key}: {value}");
    }
    out
}

fn format_details(
    status_code: u16,
    status_text: &str,
    final_url: &str,
    duration_ms: Option<u64>,
    content_length: usize,
    content_type: &str,
    headers_text: &str,
) -> String {
    let mut text = String::new();
    append_line(&mut text, "Status Code", &format!("{status_code}"));
    append_line(&mut text, "Status Text", status_text);
    append_line(&mut text, "Final URL", final_url);
    append_line(&mut text, "Duration", &format_duration(duration_ms));
    append_line(&mut text, "Body Size", &format!("{content_length} bytes"));
    append_line(&mut text, "Content Type", content_type);
    text.push('\n');
    text.push_str("Response Headers:\n");
    text.push_str(headers_text);
    text
}

#[allow(clippy::too_many_arguments)]
fn format_copy_response(
    status_code: u16,
    status_text: &str,
    final_url: &str,
    duration_ms: Option<u64>,
    content_length: usize,
    content_type: &str,
    headers_text: &str,
    body_text: &str,
) -> String {
    let mut text = format_details(
        status_code,
        status_text,
        final_url,
        duration_ms,
        content_length,
        content_type,
        headers_text,
    );
    text.push_str("\n\nBody:\n");
    text.push_str(body_text);
    text
}

fn format_error_details(duration_ms: Option<u64>) -> String {
    let mut text = String::new();
    append_line(&mut text, "Status Code", "failed");
    append_line(&mut text, "Status Text", "request not completed");
    append_line(&mut text, "Final URL", "");
    append_line(&mut text, "Duration", &format_duration(duration_ms));
    append_line(&mut text, "Body Size", "0 bytes");
    append_line(&mut text, "Content Type", "n/a");
    text.push_str("\nResponse Headers:\n(none)");
    text
}

fn http_status_notice(status_code: u16, status_text: &str) -> (String, NoticeSeverity) {
    if (400..500).contains(&status_code) {
        (
            format!("The server returned a client error: HTTP {status_code} {status_text}."),
            NoticeSeverity::Warn,
        )
    } else if status_code >= 500 {
        (
            format!("The server returned a server error: HTTP {status_code} {status_text}."),
            NoticeSeverity::Error,
        )
    } else {
        (String::new(), NoticeSeverity::Quiet)
    }
}

fn append_line(out: &mut String, label: &str, value: &str) {
    out.push_str(label);
    out.push_str(": ");
    out.push_str(value);
    out.push('\n');
}

fn should_attempt_json_pretty_print(content_type: Option<&str>, text: &str) -> bool {
    if let Some(content_type) = content_type {
        if is_json_content_type(content_type) {
            return true;
        }
    }

    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }

    matches!(
        trimmed.as_bytes()[0],
        b'{' | b'[' | b'"' | b'-' | b'0'..=b'9' | b't' | b'f' | b'n'
    )
}

fn is_json_content_type(content_type: &str) -> bool {
    let lower = content_type.to_ascii_lowercase();
    lower == "application/json"
        || lower.starts_with("application/json;")
        || lower.ends_with("+json")
        || lower.contains("+json;")
}

fn response_has_no_body(method: HttpMethod, status_code: u16) -> bool {
    method == HttpMethod::Head
        || (100..200).contains(&status_code)
        || status_code == 204
        || status_code == 304
}

fn format_duration(duration_ms: Option<u64>) -> String {
    match duration_ms {
        Some(ms) if ms < 1_000 => format!("{ms} ms"),
        Some(ms) => {
            let whole = ms / 1_000;
            let fractional = (ms % 1_000) / 10;
            format!("{whole}.{fractional:02} s")
        }
        None => String::from("n/a"),
    }
}

fn body_is_probably_text(content_type: Option<&str>, body: &[u8]) -> bool {
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
    use alloc::vec;
    use sunlight_http::HttpResponse;

    #[test]
    fn json_responses_are_pretty_printed() {
        let result = RequestResult {
            response: HttpResponse {
                status_code: 200,
                status_text: String::from("OK"),
                headers: vec![(
                    String::from("content-type"),
                    String::from("application/json"),
                )],
                header_len: 0,
            },
            body: br#"{"ok":true,"items":[1,2]}"#.to_vec(),
            final_url: None,
            duration_ms: Some(12),
        };

        let parsed = parse_response(HttpMethod::Get, "http://example.com", result, Some(12));
        assert!(parsed.body_text.contains("\n  \"ok\": true,"));
    }

    #[test]
    fn dns_errors_are_friendly() {
        let parsed = describe_fetch_error(
            &FetchError::DnsResolutionFailed(String::from("example.invalid")),
            Some(84),
        );
        assert_eq!(parsed.status_label, "DNS failed");
        assert_eq!(parsed.console_severity, NoticeSeverity::Error);
        assert!(parsed.details_text.contains("Duration: 84 ms"));
    }

    #[test]
    fn charset_and_suffix_json_are_pretty_printed() {
        for content_type in [
            "application/json; charset=utf-8",
            "application/problem+json",
        ] {
            let result = RequestResult {
                response: HttpResponse {
                    status_code: 200,
                    status_text: String::from("OK"),
                    headers: vec![(String::from("content-type"), String::from(content_type))],
                    header_len: 0,
                },
                body: br#"{"title":"bad","status":400}"#.to_vec(),
                final_url: None,
                duration_ms: None,
            };

            let parsed = parse_response(HttpMethod::Get, "http://example.com", result, Some(1200));
            assert!(parsed.body_text.contains("\n  \"title\": \"bad\","));
            assert!(parsed.details_text.contains("Duration: 1.20 s"));
        }
    }

    #[test]
    fn valid_json_without_content_type_is_pretty_printed() {
        let result = RequestResult {
            response: HttpResponse {
                status_code: 200,
                status_text: String::from("OK"),
                headers: vec![],
                header_len: 0,
            },
            body: br#"{"ok":true}"#.to_vec(),
            final_url: None,
            duration_ms: None,
        };

        let parsed = parse_response(HttpMethod::Get, "http://example.com", result, Some(4));
        assert_eq!(parsed.body_text, "{\n  \"ok\": true\n}");
    }

    #[test]
    fn malformed_json_stays_plain_text() {
        let result = RequestResult {
            response: HttpResponse {
                status_code: 200,
                status_text: String::from("OK"),
                headers: vec![(
                    String::from("content-type"),
                    String::from("application/json"),
                )],
                header_len: 0,
            },
            body: br#"{"broken":}"#.to_vec(),
            final_url: None,
            duration_ms: None,
        };

        let parsed = parse_response(HttpMethod::Get, "http://example.com", result, None);
        assert_eq!(parsed.body_text, "{\"broken\":}");
    }

    #[test]
    fn empty_and_head_bodies_stay_empty() {
        for (method, status_code) in [
            (HttpMethod::Head, 200),
            (HttpMethod::Get, 204),
            (HttpMethod::Get, 304),
            (HttpMethod::Get, 200),
        ] {
            let result = RequestResult {
                response: HttpResponse {
                    status_code,
                    status_text: String::from("OK"),
                    headers: vec![],
                    header_len: 0,
                },
                body: Vec::new(),
                final_url: None,
                duration_ms: None,
            };

            let parsed = parse_response(method, "http://example.com", result, None);
            assert!(parsed.body_text.is_empty());
            assert!(parsed.details_text.contains("Body Size: 0 bytes"));
        }
    }
}
