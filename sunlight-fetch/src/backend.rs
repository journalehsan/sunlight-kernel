use crate::error::{FetchError, FetchResult};
use crate::http::{HttpRequest, HttpResponse, ParsedUrl};
use crate::ipc::{self, ResolvedAddr, TcpHandle};
use crate::prelude::{String, Vec};

const MAX_REDIRECTS: usize = 10;

#[cfg(feature = "host-linux")]
type RequestTimer = std::time::Instant;
#[cfg(not(feature = "host-linux"))]
type RequestTimer = ();

#[derive(Debug, Clone)]
pub struct RequestResult {
    pub response: HttpResponse,
    pub body: Vec<u8>,
    pub final_url: Option<ParsedUrl>,
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Clone)]
pub(crate) enum RequestEvent {
    Connecting { url: ParsedUrl },
    Redirect { status: u16, next_url: ParsedUrl },
}

pub(crate) struct PendingRequest {
    pub response: HttpResponse,
    pub final_url: ParsedUrl,
    request_method: &'static str,
    handle: TcpHandle,
    initial_body: Vec<u8>,
    started_at: RequestTimer,
}

impl PendingRequest {
    pub fn read_body(
        mut self,
        progress: Option<&mut dyn FnMut(usize)>,
    ) -> FetchResult<RequestResult> {
        let body = ipc::read_body_full(
            &mut self.handle,
            self.request_method,
            &self.response,
            &self.initial_body,
            progress,
        )?;

        Ok(RequestResult {
            response: self.response,
            body,
            final_url: Some(self.final_url),
            duration_ms: elapsed_ms(self.started_at),
        })
    }
}

pub fn perform_request(url: ParsedUrl, request: HttpRequest) -> FetchResult<RequestResult> {
    ipc::acquire_capabilities()?;
    let addr = ipc::dns_resolve(&url.host)?;
    let pending = begin_request_from_resolved(url, addr, request, None)?;
    pending.read_body(None)
}

pub(crate) fn begin_request_from_resolved(
    start_url: ParsedUrl,
    start_addr: ResolvedAddr,
    request: HttpRequest,
    mut on_event: Option<&mut dyn FnMut(RequestEvent)>,
) -> FetchResult<PendingRequest> {
    let started_at = start_timer();
    let mut url = start_url;
    let mut addr = start_addr;
    let mut request = request;

    for redirect in 0..=MAX_REDIRECTS {
        emit_event(&mut on_event, RequestEvent::Connecting { url: url.clone() });

        let (response, mut handle, initial_body) =
            ipc::http_request(&url.host, &addr, url.port, url.uses_tls(), &request)?;

        if matches!(response.status_code, 301 | 302 | 303 | 307 | 308) {
            let location = response
                .header("location")
                .ok_or_else(|| FetchError::HttpError {
                    status: response.status_code,
                    message: String::from("redirect without Location header"),
                })?;

            if redirect == MAX_REDIRECTS {
                return Err(FetchError::HttpError {
                    status: response.status_code,
                    message: format!("too many redirects (>{MAX_REDIRECTS})"),
                });
            }

            let next_url = resolve_redirect_location(&url, location)?;
            emit_event(
                &mut on_event,
                RequestEvent::Redirect {
                    status: response.status_code,
                    next_url: next_url.clone(),
                },
            );
            addr = ipc::dns_resolve(&next_url.host)?;
            request = redirected_request(&request, &next_url);
            url = next_url;
            let _ = handle.close();
            continue;
        }

        return Ok(PendingRequest {
            response,
            final_url: url,
            request_method: request.method,
            handle,
            initial_body,
            started_at,
        });
    }

    Err(FetchError::HttpError {
        status: 0,
        message: String::from("redirect loop exhausted"),
    })
}

fn redirected_request(request: &HttpRequest, next_url: &ParsedUrl) -> HttpRequest {
    HttpRequest {
        method: request.method,
        path: next_url.path.clone(),
        host: next_url.host_header(),
        headers: request.headers.clone(),
        body: request.body.clone(),
    }
}

pub(crate) fn resolve_redirect_location(
    base_url: &ParsedUrl,
    location: &str,
) -> FetchResult<ParsedUrl> {
    if location.starts_with("http://") || location.starts_with("https://") {
        return ParsedUrl::parse(location).map_err(Into::into);
    }

    if location.starts_with('/') {
        return Ok(ParsedUrl {
            scheme: base_url.scheme,
            host: base_url.host.clone(),
            port: base_url.port,
            path: String::from(location),
        });
    }

    Err(FetchError::InvalidUrl(format!(
        "unsupported redirect location: {location}"
    )))
}

fn emit_event(on_event: &mut Option<&mut dyn FnMut(RequestEvent)>, event: RequestEvent) {
    if let Some(callback) = on_event {
        (**callback)(event);
    }
}

#[cfg(feature = "host-linux")]
fn start_timer() -> RequestTimer {
    std::time::Instant::now()
}

#[cfg(not(feature = "host-linux"))]
fn start_timer() -> RequestTimer {}

#[cfg(feature = "host-linux")]
fn elapsed_ms(started_at: RequestTimer) -> Option<u64> {
    let millis = started_at.elapsed().as_millis();
    Some(millis.min(u128::from(u64::MAX)) as u64)
}

#[cfg(not(feature = "host-linux"))]
fn elapsed_ms(_started_at: RequestTimer) -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "host-linux")]
    use std::io::{Read, Write};
    #[cfg(feature = "host-linux")]
    use std::net::TcpListener;
    #[cfg(feature = "host-linux")]
    use std::sync::{Arc, Mutex};
    #[cfg(feature = "host-linux")]
    use std::thread;

    #[test]
    fn resolve_redirect_absolute_url() {
        let base = ParsedUrl::parse("http://example.com/start").unwrap();
        let next = resolve_redirect_location(&base, "https://redirect.example.com/final").unwrap();
        assert_eq!(next.host, "redirect.example.com");
        assert_eq!(next.port, 443);
        assert_eq!(next.path, "/final");
        assert!(next.uses_tls());
    }

    #[test]
    fn resolve_redirect_root_relative_url() {
        let base = ParsedUrl::parse("http://example.com:8080/start").unwrap();
        let next = resolve_redirect_location(&base, "/next").unwrap();
        assert_eq!(next.host, "example.com");
        assert_eq!(next.port, 8080);
        assert_eq!(next.path, "/next");
    }

    #[test]
    fn reject_unsupported_relative_redirect() {
        let base = ParsedUrl::parse("http://example.com/start").unwrap();
        let err = resolve_redirect_location(&base, "../next").unwrap_err();
        match err {
            FetchError::InvalidUrl(msg) => assert!(msg.contains("unsupported redirect location")),
            other => panic!("expected InvalidUrl, got {other:?}"),
        }
    }

    #[cfg(feature = "host-linux")]
    #[test]
    fn perform_request_get_follows_redirects_and_preserves_headers() {
        let requests = Arc::new(Mutex::new(Vec::<String>::new()));
        let Some(listener) = bind_test_listener() else {
            return;
        };
        let port = listener.local_addr().unwrap().port();
        let captured = Arc::clone(&requests);

        let server = thread::spawn(move || {
            for response in [
                "HTTP/1.1 302 Found\r\nLocation: /final\r\nContent-Length: 0\r\n\r\n",
                "HTTP/1.1 200 OK\r\nContent-Length: 5\r\nX-Test: ok\r\n\r\nhello",
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                let request = read_http_request(&mut stream);
                captured.lock().unwrap().push(request);
                stream.write_all(response.as_bytes()).unwrap();
            }
        });

        let url = ParsedUrl::parse(&format!("http://127.0.0.1:{port}/start")).unwrap();
        let request = HttpRequest {
            method: "GET",
            path: url.path.clone(),
            host: url.host_header(),
            headers: vec![
                (String::from("accept"), String::from("text/plain")),
                (String::from("x-test-header"), String::from("redirect-me")),
            ],
            body: None,
        };

        let result = perform_request(url, request).unwrap();
        server.join().unwrap();

        assert_eq!(result.response.status_code, 200);
        assert_eq!(result.body, b"hello");
        assert_eq!(result.response.header("x-test"), Some("ok"));
        assert_eq!(result.final_url.unwrap().path, "/final");
        assert!(result.duration_ms.is_some());

        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[0].starts_with("GET /start HTTP/1.1\r\n"));
        assert!(requests[1].starts_with("GET /final HTTP/1.1\r\n"));
        assert!(requests[0].contains("x-test-header: redirect-me\r\n"));
        assert!(requests[1].contains("x-test-header: redirect-me\r\n"));
    }

    #[cfg(feature = "host-linux")]
    #[test]
    fn perform_request_post_sends_body_and_custom_headers() {
        let requests = Arc::new(Mutex::new(Vec::<String>::new()));
        let Some(listener) = bind_test_listener() else {
            return;
        };
        let port = listener.local_addr().unwrap().port();
        let captured = Arc::clone(&requests);

        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_http_request(&mut stream);
            captured.lock().unwrap().push(request);
            stream
                .write_all(b"HTTP/1.1 201 Created\r\nContent-Length: 2\r\n\r\nok")
                .unwrap();
        });

        let url = ParsedUrl::parse(&format!("http://127.0.0.1:{port}/submit")).unwrap();
        let request = HttpRequest {
            method: "POST",
            path: url.path.clone(),
            host: url.host_header(),
            headers: vec![
                (
                    String::from("content-type"),
                    String::from("application/x-www-form-urlencoded"),
                ),
                (String::from("x-api-key"), String::from("abc123")),
            ],
            body: Some(b"name=sunlight".to_vec()),
        };

        let result = perform_request(url, request).unwrap();
        server.join().unwrap();

        assert_eq!(result.response.status_code, 201);
        assert_eq!(result.body, b"ok");

        let requests = requests.lock().unwrap();
        let request = &requests[0];
        assert!(request.starts_with("POST /submit HTTP/1.1\r\n"));
        assert!(request.contains("content-type: application/x-www-form-urlencoded\r\n"));
        assert!(request.contains("x-api-key: abc123\r\n"));
        assert!(request.contains("Content-Length: 13\r\n"));
        assert!(request.ends_with("\r\n\r\nname=sunlight"));
    }

    #[cfg(feature = "host-linux")]
    #[test]
    fn https_error_surfaces_cleanly_against_plain_server() {
        let Some(listener) = bind_test_listener() else {
            return;
        };
        let port = listener.local_addr().unwrap().port();

        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok");
        });

        let url = ParsedUrl::parse(&format!("https://localhost:{port}/")).unwrap();
        let request = HttpRequest {
            method: "GET",
            path: url.path.clone(),
            host: url.host_header(),
            headers: vec![],
            body: None,
        };

        let err = perform_request(url, request).unwrap_err();
        server.join().unwrap();

        match err {
            FetchError::ConnectionFailed { .. }
            | FetchError::IoError(_)
            | FetchError::HttpError { .. } => {}
            other => panic!("unexpected error variant: {other:?}"),
        }
    }

    #[cfg(feature = "host-linux")]
    #[test]
    fn perform_request_decodes_chunked_body() {
        let Some(listener) = bind_test_listener() else {
            return;
        };
        let port = listener.local_addr().unwrap().port();

        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _ = read_http_request(&mut stream);
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Type: text/html\r\n\r\n4\r\nWiki\r\n6\r\npedia!\r\n0\r\nX-Test: done\r\n\r\n",
                )
                .unwrap();
        });

        let url = ParsedUrl::parse(&format!("http://127.0.0.1:{port}/chunked")).unwrap();
        let request = HttpRequest {
            method: "GET",
            path: url.path.clone(),
            host: url.host_header(),
            headers: vec![],
            body: None,
        };

        let result = perform_request(url, request).unwrap();
        server.join().unwrap();

        assert_eq!(result.body, b"Wikipedia!");
        assert!(core::str::from_utf8(&result.body)
            .unwrap()
            .starts_with("Wiki"));
    }

    #[cfg(feature = "host-linux")]
    #[test]
    fn head_request_ignores_declared_content_length_body() {
        let Some(listener) = bind_test_listener() else {
            return;
        };
        let port = listener.local_addr().unwrap().port();

        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _ = read_http_request(&mut stream);
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\n")
                .unwrap();
        });

        let url = ParsedUrl::parse(&format!("http://127.0.0.1:{port}/head")).unwrap();
        let request = HttpRequest {
            method: "HEAD",
            path: url.path.clone(),
            host: url.host_header(),
            headers: vec![],
            body: None,
        };

        let result = perform_request(url, request).unwrap();
        server.join().unwrap();

        assert!(result.body.is_empty());
    }

    #[cfg(feature = "host-linux")]
    fn read_http_request(stream: &mut std::net::TcpStream) -> String {
        let mut buf = Vec::new();
        let mut scratch = [0u8; 1024];

        loop {
            let n = stream.read(&mut scratch).unwrap();
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&scratch[..n]);

            if let Some(header_end) = header_end(&buf) {
                let header_len = header_end + 4;
                let headers = std::str::from_utf8(&buf[..header_len]).unwrap();
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        line.strip_prefix("Content-Length: ")
                            .and_then(|value| value.trim().parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                if buf.len() >= header_len + content_length {
                    break;
                }
            }
        }

        String::from_utf8(buf).unwrap()
    }

    #[cfg(feature = "host-linux")]
    fn header_end(data: &[u8]) -> Option<usize> {
        data.windows(4).position(|window| window == b"\r\n\r\n")
    }

    #[cfg(feature = "host-linux")]
    fn bind_test_listener() -> Option<TcpListener> {
        match TcpListener::bind("127.0.0.1:0") {
            Ok(listener) => Some(listener),
            Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => None,
            Err(err) => panic!("failed to bind test listener: {err}"),
        }
    }
}
