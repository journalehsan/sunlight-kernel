//! Unified error type for fetch operations.
//! No dynamic dispatch — enum-based, size-optimized.

use std::string::String;
use std::fmt;

/// Every error fetch can produce, with enough context to diagnose.
#[derive(Debug)]
pub enum FetchError {
    /// Argument parsing failure
    InvalidArgs(String),

    /// DNS resolution failed for hostname
    DnsResolutionFailed(String),

    /// Capability acquisition failed
    CapabilityDenied {
        resource: &'static str,
        detail: String,
    },

    /// IPC communication failure with net_server
    IpcError(String),

    /// TCP connection failed
    ConnectionFailed {
        host: String,
        port: u16,
        reason: String,
    },

    /// HTTP protocol error (malformed response, unexpected status)
    HttpError {
        status: u16,
        message: String,
    },

    /// Server does not support Range requests (informational, triggers fallback)
    RangeNotSupported,

    /// VFS / filesystem error
    VfsError(String),

    /// I/O error during download
    IoError(String),

    /// Download was interrupted (Ctrl+C or signal)
    Interrupted,

    /// URL parsing failure
    InvalidUrl(String),

    /// Content-Length missing and chunked not supported
    UnknownContentLength,

    /// Chunk reassembly integrity failure
    ChunkIntegrityError {
        chunk_id: usize,
        expected: usize,
        got: usize,
    },
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidArgs(msg) => write!(f, "invalid arguments: {msg}"),
            Self::DnsResolutionFailed(host) => {
                write!(f, "DNS resolution failed for '{host}'")
            }
            Self::CapabilityDenied { resource, detail } => {
                write!(f, "capability denied for {resource}: {detail}")
            }
            Self::IpcError(msg) => write!(f, "IPC error: {msg}"),
            Self::ConnectionFailed { host, port, reason } => {
                write!(f, "connection to {host}:{port} failed: {reason}")
            }
            Self::HttpError { status, message } => {
                write!(f, "HTTP {status}: {message}")
            }
            Self::RangeNotSupported => {
                write!(f, "server does not support Range requests")
            }
            Self::VfsError(msg) => write!(f, "VFS error: {msg}"),
            Self::IoError(msg) => write!(f, "I/O error: {msg}"),
            Self::Interrupted => write!(f, "download interrupted"),
            Self::InvalidUrl(msg) => write!(f, "invalid URL: {msg}"),
            Self::UnknownContentLength => {
                write!(f, "server did not provide Content-Length")
            }
            Self::ChunkIntegrityError {
                chunk_id,
                expected,
                got,
            } => {
                write!(
                    f,
                    "chunk {chunk_id} integrity error: expected {expected} bytes, got {got}"
                )
            }
        }
    }
}

pub type FetchResult<T> = Result<T, FetchError>;
