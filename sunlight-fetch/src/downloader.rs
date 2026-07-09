//! Chunked download engine with Range-request parallelism.

use crate::backend::{self, RequestEvent};
use crate::cli::{FetchConfig, HttpMethod};
use crate::error::{FetchError, FetchResult};
use crate::http::{HttpRequest, ParsedUrl};
use crate::ipc::{self, ResolvedAddr};
use crate::prelude::{String, ToString, Vec};
use crate::progress::ProgressTracker;

/// Main download entry point — routes by URL scheme.
pub fn execute_download(config: &FetchConfig) -> FetchResult<()> {
    ipc::acquire_capabilities()?;

    if config.url.starts_with("https://") {
        do_https_fetch(config)
    } else if config.url.starts_with("http://") {
        do_plain_http_fetch(config)
    } else {
        Err(FetchError::InvalidUrl(format!(
            "only http:// and https:// are supported: {}",
            config.url
        )))
    }
}

/// Plain HTTP fetch — the complete, verified working path. Do not modify.
fn do_plain_http_fetch(config: &FetchConfig) -> FetchResult<()> {
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

/// HTTPS fetch — delegates to the same execute_get/execute_post machinery as
/// the plain path.  TLS is transparent: ParsedUrl sets uses_tls()=true which
/// makes ipc::http_request route through the sunlight-tls daemon
/// (tls_connect → TLS_{SEND,RECV} IPC) instead of a raw TCP socket.
fn do_https_fetch(config: &FetchConfig) -> FetchResult<()> {
    let url = ParsedUrl::parse(&config.url)?;

    let output_name = config
        .output
        .clone()
        .unwrap_or_else(|| url.infer_filename());

    eprintln_fetch(&format!("Resolving {} (HTTPS)...", url.host));

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
    _config: &FetchConfig,
    url: &ParsedUrl,
    addr: &ResolvedAddr,
    output_name: &str,
) -> FetchResult<()> {
    let request = HttpRequest {
        method: HttpMethod::Get.as_str(),
        path: url.path.clone(),
        host: url.host_header(),
        headers: vec![(String::from("accept"), String::from("*/*"))],
        body: None,
    };
    let mut on_event = |event: RequestEvent| match event {
        RequestEvent::Connecting { url } => eprintln_fetch(&format!(
            "Connecting to {}:{} ({})...",
            url.host,
            url.port,
            if url.uses_tls() { "TLS" } else { "plain" }
        )),
        RequestEvent::Redirect { status, next_url } => eprintln_fetch(&format!(
            "Following redirect {} -> {}:{}{}",
            status, next_url.host, next_url.port, next_url.path
        )),
    };
    let pending = backend::begin_request_from_resolved(
        url.clone(),
        addr.clone(),
        request,
        Some(&mut on_event),
    )?;
    let response = &pending.response;

    if response.status_code != 200 {
        return Err(FetchError::HttpError {
            status: response.status_code,
            message: response.status_text.clone(),
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

    let result = pending.read_body(Some(&mut on_progress))?;

    progress.finish();
    progress.render(&mut render_buf);
    eprint_progress(&render_buf);

    write_atomic(output_name, &result.body)?;

    eprintln_fetch(&format!(
        "Saved {} ({} bytes)",
        output_name,
        result.body.len()
    ));
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

    let request = HttpRequest {
        method: HttpMethod::Post.as_str(),
        path: url.path.clone(),
        host: url.host_header(),
        headers: vec![(
            String::from("content-type"),
            String::from("application/x-www-form-urlencoded"),
        )],
        body: Some(body_data),
    };
    let mut on_event = |event: RequestEvent| match event {
        RequestEvent::Connecting { url } => eprintln_fetch(&format!(
            "Connecting to {}:{} ({})...",
            url.host,
            url.port,
            if url.uses_tls() { "TLS" } else { "plain" }
        )),
        RequestEvent::Redirect { status, next_url } => eprintln_fetch(&format!(
            "Following redirect {} -> {}:{}{}",
            status, next_url.host, next_url.port, next_url.path
        )),
    };
    let pending = backend::begin_request_from_resolved(
        url.clone(),
        addr.clone(),
        request,
        Some(&mut on_event),
    )?;
    let status_code = pending.response.status_code;
    let status_text = pending.response.status_text.clone();

    if status_code >= 400 {
        return Err(FetchError::HttpError {
            status: status_code,
            message: status_text,
        });
    }

    let result = pending.read_body(None)?;
    write_atomic(output_name, &result.body)?;

    eprintln_fetch(&format!(
        "HTTP {} {} — saved {} ({} bytes)",
        status_code,
        status_text,
        output_name,
        result.body.len()
    ));

    Ok(())
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

/// Resolve a download output name to an absolute, writable path.
///
/// SunlightOS has no current-working-directory concept and an immutable root
/// filesystem — only the home trees (`/root`, `/home/<user>`) are writable. A
/// bare/relative name like the default `index.html` would resolve against the
/// read-only root and fail with an I/O error, so anchor relative names in
/// root's home. Callers that pass an absolute `-o /path` are left untouched.
#[cfg(not(feature = "host-linux"))]
fn resolve_out_path(path: &str) -> String {
    if path.starts_with('/') {
        String::from(path)
    } else {
        format!("/root/{path}")
    }
}

#[cfg(not(feature = "host-linux"))]
fn platform_write_file(path: &str, data: &[u8]) -> FetchResult<()> {
    use sunlight_libc::{self as libc, Errno};

    // Use create() (O_WRONLY | O_CREAT), not open() — open() passes flags=0 and
    // cannot create a new file, so every download failed with an I/O error.
    let abs = resolve_out_path(path);
    let fd = libc::create(abs.as_bytes()).map_err(|e| FetchError::IoError(errno_str(e)))?;
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

    // Match the anchoring done by platform_write_file so the rename read-back
    // finds the `.part` file it just wrote.
    let abs = resolve_out_path(path);
    let fd = libc::open(abs.as_bytes()).map_err(|e| FetchError::IoError(errno_str(e)))?;
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
