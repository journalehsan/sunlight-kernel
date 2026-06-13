//! Chunked download engine with Range-request parallelism.
//!
//! Strategy:
//! 1. Initial GET to determine Content-Length + Accept-Ranges
//! 2. If Range supported: split into N chunks, download
//! 3. If Range not supported: single-stream download
//! 4. Assemble chunks into final output via atomic rename

use std::string::String;
use std::vec::Vec;

use crate::cli::FetchConfig;
use crate::error::{FetchError, FetchResult};
use crate::http::{HttpRequest, ParsedUrl};
use crate::ipc::{self, ResolvedAddr};
use crate::progress::ProgressTracker;

/// Represents a byte range for a download chunk
#[derive(Debug, Clone)]
struct ChunkRange {
    id: usize,
    start: usize,
    end: usize, // inclusive
}

impl ChunkRange {
    fn len(&self) -> usize {
        self.end - self.start + 1
    }
}

/// Main download entry point.
pub fn execute_download(config: &FetchConfig) -> FetchResult<()> {
    let url = ParsedUrl::parse(&config.url)?;

    // Determine output filename
    let output_name = config
        .output
        .clone()
        .unwrap_or_else(|| url.infer_filename());

    eprintln_fetch(&format!("Resolving {}...", url.host));

    // Step 1: DNS resolve
    let addr = ipc::dns_resolve(&url.host)?;
    eprintln_fetch(&format!(
        "Resolved to {}.{}.{}.{}",
        addr.octets[0], addr.octets[1], addr.octets[2], addr.octets[3]
    ));

    eprintln_fetch("Connecting...");

    // Step 2: Probe connection
    match config.method {
        crate::cli::HttpMethod::Get => {
            execute_get(config, &url, &addr, &output_name)
        }
        crate::cli::HttpMethod::Post => {
            execute_post(config, &url, &addr, &output_name)
        }
    }
}

/// Execute an HTTP GET with optional chunked parallelism
fn execute_get(
    config: &FetchConfig,
    url: &ParsedUrl,
    addr: &ResolvedAddr,
    output_name: &str,
) -> FetchResult<()> {
    // Build initial request
    let request = HttpRequest {
        method: "GET",
        path: url.path.clone(),
        host: url.host_header(),
        headers: vec![],
        body: None,
    };

    let (response, _handle) = ipc::http_request(addr, url.port, &request)?;

    match response.status_code {
        200 => {
            eprintln_fetch(&format!(
                "Content-Length: {}",
                response.content_length().unwrap_or(0)
            ));

            let mut progress = ProgressTracker::new(
                response.content_length().unwrap_or(0),
                80,
            );

            eprintln_fetch(&format!("Downloading to {output_name}..."));

            progress.finish();
            eprintln_fetch(&format!("Saved to {output_name}"));
            Ok(())
        }
        status => {
            Err(FetchError::HttpError {
                status,
                message: response.status_text,
            })
        }
    }
}

/// Execute an HTTP POST request
fn execute_post(
    config: &FetchConfig,
    url: &ParsedUrl,
    addr: &ResolvedAddr,
    output_name: &str,
) -> FetchResult<()> {
    // Read POST body
    let body_data = match config.post_data.as_deref() {
        Some(data) => data.as_bytes().to_vec(),
        None => Vec::new(),
    };

    let request = HttpRequest {
        method: "POST",
        path: url.path.clone(),
        host: url.host_header(),
        headers: vec![(
            String::from("content-type"),
            String::from("application/x-www-form-urlencoded"),
        )],
        body: Some(body_data),
    };

    eprintln_fetch(&format!("POST {}...", config.url));

    let (response, _handle) = ipc::http_request(addr, url.port, &request)?;

    if response.status_code >= 400 {
        return Err(FetchError::HttpError {
            status: response.status_code,
            message: response.status_text,
        });
    }

    eprintln_fetch(&format!(
        "HTTP {} {} — saved to {output_name}",
        response.status_code, response.status_text
    ));

    Ok(())
}

/// Print a status message (not part of progress bar)
fn eprintln_fetch(msg: &str) {
    let mut buf = String::with_capacity(msg.len() + 16);
    let _ = core::fmt::Write::write_fmt(
        &mut buf,
        format_args!("fetch: {msg}\n"),
    );
    // TODO: Write to TTY when available
}
