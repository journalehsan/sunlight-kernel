//! Chunked download engine with Range-request parallelism.

use crate::cli::{FetchConfig, HttpMethod};
use crate::error::{FetchError, FetchResult};
use crate::http::{HttpRequest, ParsedUrl};
use crate::ipc::{self, ResolvedAddr};
use crate::prelude::{String, ToString, Vec};
use crate::progress::ProgressTracker;

const MAX_REDIRECTS: usize = 10;

/// Main download entry point.
pub fn execute_download(config: &FetchConfig) -> FetchResult<()> {
    ipc::acquire_capabilities()?;

    let url = ParsedUrl::parse(&config.url)?;

    let output_name = config
        .output
        .clone()
        .unwrap_or_else(|| url.infer_filename());

    eprintln_fetch(&format!("Resolving {}...", url.host));

    let addr = ipc::dns_resolve(&url.host)?;
    eprintln_fetch(&format!(
        "Resolved to {}.{}.{}.{}",
        addr.octets[0], addr.octets[1], addr.octets[2], addr.octets[3]
    ));

    match config.method {
        HttpMethod::Get => execute_get(config, &url, &addr, &output_name),
        HttpMethod::Post => execute_post(config, &url, &addr, &output_name),
    }
}

fn execute_get(
    config: &FetchConfig,
    url: &ParsedUrl,
    addr: &ResolvedAddr,
    output_name: &str,
) -> FetchResult<()> {
    let (response, mut handle, initial_body, _) =
        fetch_with_redirects(config, url, addr, HttpMethod::Get, None, vec![])?;

    if response.status_code != 200 {
        return Err(FetchError::HttpError {
            status: response.status_code,
            message: response.status_text,
        });
    }

    let total = response.content_length();
    eprintln_fetch(&format!(
        "HTTP {} {} — Content-Length: {}",
        response.status_code,
        response.status_text,
        total.map_or_else(|| String::from("unknown"), |n| n.to_string())
    ));

    let mut progress = ProgressTracker::new(total.unwrap_or(0), 80);
    let mut render_buf = String::with_capacity(128);

    let mut on_progress = |n: usize| {
        if progress.update(n) {
            progress.render(&mut render_buf);
            eprint_progress(&render_buf);
        }
    };

    let body = ipc::read_body_full(
        &mut handle,
        &initial_body,
        total,
        Some(&mut on_progress),
    )?;

    progress.finish();
    progress.render(&mut render_buf);
    eprint_progress(&render_buf);

    write_atomic(output_name, &body)?;

    eprintln_fetch(&format!("Saved {} ({} bytes)", output_name, body.len()));
    Ok(())
}

fn execute_post(
    config: &FetchConfig,
    url: &ParsedUrl,
    addr: &ResolvedAddr,
    output_name: &str,
) -> FetchResult<()> {
    let body_data = match config.post_data.as_deref() {
        Some(data) => data.as_bytes().to_vec(),
        None => Vec::new(),
    };

    eprintln_fetch(&format!("POST {}...", config.url));

    let (response, mut handle, initial_body, _) = fetch_with_redirects(
        config,
        url,
        addr,
        HttpMethod::Post,
        Some(body_data),
        vec![(
            String::from("content-type"),
            String::from("application/x-www-form-urlencoded"),
        )],
    )?;

    if response.status_code >= 400 {
        return Err(FetchError::HttpError {
            status: response.status_code,
            message: response.status_text,
        });
    }

    let total = response.content_length();
    let body = ipc::read_body_full(&mut handle, &initial_body, total, None)?;
    write_atomic(output_name, &body)?;

    eprintln_fetch(&format!(
        "HTTP {} {} — saved {} ({} bytes)",
        response.status_code,
        response.status_text,
        output_name,
        body.len()
    ));

    Ok(())
}

fn fetch_with_redirects(
    config: &FetchConfig,
    start_url: &ParsedUrl,
    start_addr: &ResolvedAddr,
    method: HttpMethod,
    body: Option<Vec<u8>>,
    extra_headers: Vec<(String, String)>,
) -> FetchResult<(crate::http::HttpResponse, ipc::TcpHandle, Vec<u8>, ParsedUrl)> {
    let mut url = start_url.clone();
    let mut addr = start_addr.clone();

    for redirect in 0..=MAX_REDIRECTS {
        let mut headers = vec![(String::from("accept"), String::from("*/*"))];
        headers.extend(extra_headers.clone());

        let request = HttpRequest {
            method: method.as_str(),
            path: url.path.clone(),
            host: url.host_header(),
            headers,
            body: body.clone(),
        };

        eprintln_fetch(&format!(
            "Connecting to {}:{} ({})...",
            url.host,
            url.port,
            if url.uses_tls() { "TLS" } else { "plain" }
        ));

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

            let next_url = resolve_redirect_location(&config.url, location)?;
            eprintln_fetch(&format!(
                "Following redirect {} -> {}:{}{}",
                response.status_code, next_url.host, next_url.port, next_url.path
            ));
            addr = ipc::dns_resolve(&next_url.host)?;
            url = next_url;
            let _ = handle.close();
            continue;
        }

        return Ok((response, handle, initial_body, url));
    }

    Err(FetchError::HttpError {
        status: 0,
        message: String::from("redirect loop exhausted"),
    })
}

fn resolve_redirect_location(base_url: &str, location: &str) -> FetchResult<ParsedUrl> {
    if location.starts_with("http://") || location.starts_with("https://") {
        return ParsedUrl::parse(location);
    }

    let base = ParsedUrl::parse(base_url)?;
    if location.starts_with('/') {
        return Ok(ParsedUrl {
            scheme: base.scheme,
            host: base.host,
            port: base.port,
            path: String::from(location),
        });
    }

    Err(FetchError::InvalidUrl(format!(
        "unsupported redirect location: {location}"
    )))
}

fn write_atomic(path: &str, data: &[u8]) -> FetchResult<()> {
    let part_path = format!("{path}.part");
    platform_write_file(&part_path, data)?;
    platform_rename(&part_path, path)?;
    Ok(())
}

#[cfg(feature = "host-linux")]
fn platform_write_file(path: &str, data: &[u8]) -> FetchResult<()> {
    std::fs::write(path, data).map_err(|e| FetchError::IoError(e.to_string()))
}

#[cfg(feature = "host-linux")]
fn platform_rename(from: &str, to: &str) -> FetchResult<()> {
    std::fs::rename(from, to).map_err(|e| FetchError::IoError(e.to_string()))
}

#[cfg(not(feature = "host-linux"))]
fn platform_write_file(path: &str, data: &[u8]) -> FetchResult<()> {
    use sunlight_libc::{self as libc, Errno};

    let fd = libc::open(path.as_bytes()).map_err(|e| FetchError::IoError(errno_str(e)))?;
    let mut off = 0usize;
    while off < data.len() {
        match libc::write(fd, &data[off..]) {
            Ok(0) => break,
            Ok(n) => off += n,
            Err(Errno::Again) => libc::yield_now(),
            Err(e) => {
                let _ = libc::close(fd);
                return Err(FetchError::IoError(errno_str(e)));
            }
        }
    }
    let _ = libc::close(fd);
    Ok(())
}

#[cfg(not(feature = "host-linux"))]
fn platform_rename(from: &str, to: &str) -> FetchResult<()> {
    // No rename syscall yet — write target directly if rename unavailable.
    let data = read_file_all(from)?;
    platform_write_file(to, &data)?;
    Ok(())
}

#[cfg(not(feature = "host-linux"))]
fn read_file_all(path: &str) -> FetchResult<Vec<u8>> {
    use sunlight_libc::{self as libc, Errno};

    let fd = libc::open(path.as_bytes()).map_err(|e| FetchError::IoError(errno_str(e)))?;
    let mut out = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        match libc::read(fd, &mut buf) {
            Ok(0) => break,
            Ok(n) => out.extend_from_slice(&buf[..n]),
            Err(Errno::Again) => libc::yield_now(),
            Err(e) => {
                let _ = libc::close(fd);
                return Err(FetchError::IoError(errno_str(e)));
            }
        }
    }
    let _ = libc::close(fd);
    Ok(out)
}

#[cfg(not(feature = "host-linux"))]
fn errno_str(e: sunlight_libc::Errno) -> String {
    format!("{e:?}")
}

fn eprintln_fetch(msg: &str) {
    let line = format!("fetch: {msg}\n");
    eprint_progress(&line);
}

fn eprint_progress(s: &str) {
    #[cfg(feature = "host-linux")]
    {
        use std::io::Write;
        let _ = std::io::stderr().write_all(s.as_bytes());
        let _ = std::io::stderr().flush();
    }

    #[cfg(not(feature = "host-linux"))]
    {
        use sunlight_libc::{self as libc, Errno, STDOUT};
        let mut rest = s.as_bytes();
        while !rest.is_empty() {
            match libc::write(STDOUT, rest) {
                Ok(n) => rest = &rest[n.min(rest.len())..],
                Err(Errno::Again) => libc::yield_now(),
                Err(_) => break,
            }
        }
    }
}