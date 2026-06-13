//! IPC interface to SunlightOS net_server.
//!
//! Extends the existing NetOp protocol with HTTP-specific operations.
//! All network I/O goes through this — no direct socket access.

use std::string::String;
use std::vec::Vec;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::error::{FetchError, FetchResult};
use crate::http::{HttpRequest, HttpResponse};

/// IPC operation codes for net_server HTTP extension
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetHttpOp {
    /// Resolve hostname → IPv4 address
    DnsResolve = 0x1001,
    /// Open TCP connection (returns connection handle)
    TcpConnect = 0x1002,
    /// Send data on TCP connection
    TcpSend = 0x1003,
    /// Receive data from TCP connection
    TcpRecv = 0x1004,
    /// Close TCP connection
    TcpClose = 0x1005,
}

/// DNS resolution result
#[derive(Debug, Clone)]
pub struct ResolvedAddr {
    pub octets: [u8; 4],
}

impl ResolvedAddr {
    pub fn as_u32(&self) -> u32 {
        u32::from_be_bytes(self.octets)
    }
}

/// Handle to an open TCP connection via net_server
#[derive(Debug, Clone)]
pub struct TcpHandle {
    id: u64,
    closed: bool,
}

impl Drop for TcpHandle {
    fn drop(&mut self) {
        // Best-effort close on drop — don't leak server-side state
        let _ = self.close();
    }
}

/// Global interrupt flag for Ctrl+C handling
static INTERRUPTED: AtomicBool = AtomicBool::new(false);

/// Set the interrupt flag (called from signal handler)
pub fn set_interrupted() {
    INTERRUPTED.store(true, Ordering::Release);
}

/// Check if we've been interrupted
pub fn is_interrupted() -> bool {
    INTERRUPTED.load(Ordering::Acquire)
}

/// Acquire the required capabilities for fetch operations.
pub fn acquire_capabilities() -> FetchResult<()> {
    // TODO: Implement capability acquisition
    // For now, assume capabilities are available
    Ok(())
}

/// Resolve a hostname to an IPv4 address via net_server.
pub fn dns_resolve(hostname: &str) -> FetchResult<ResolvedAddr> {
    // TODO: Implement DNS resolution via IPC
    // For now, return a placeholder
    Err(FetchError::DnsResolutionFailed(String::from(hostname)))
}

/// Open a TCP connection via net_server.
pub fn tcp_connect(_addr: &ResolvedAddr, _port: u16) -> FetchResult<TcpHandle> {
    Ok(TcpHandle { id: 0, closed: false })
}

impl TcpHandle {
    /// Send data on this TCP connection.
    pub fn send(&self, _data: &[u8]) -> FetchResult<usize> {
        if is_interrupted() {
            return Err(FetchError::Interrupted);
        }
        // TODO: Implement TCP send via IPC
        Ok(0)
    }

    /// Receive data from this TCP connection.
    pub fn recv(&self, _max_len: usize) -> FetchResult<Vec<u8>> {
        if is_interrupted() {
            return Err(FetchError::Interrupted);
        }
        // TODO: Implement TCP recv via IPC
        Ok(Vec::new())
    }

    /// Close this TCP connection.
    pub fn close(&mut self) -> FetchResult<()> {
        if self.closed {
            return Ok(());
        }
        // TODO: Implement TCP close via IPC
        self.closed = true;
        Ok(())
    }
}

/// Perform a complete HTTP request and return the response.
pub fn http_request(
    _addr: &ResolvedAddr,
    _port: u16,
    _request: &HttpRequest,
) -> FetchResult<(HttpResponse, TcpHandle)> {
    // TODO: Implement full HTTP request/response cycle
    Err(FetchError::IpcError(String::from("HTTP request not implemented")))
}

/// Read the remaining body from a TCP handle.
pub fn read_body_full(
    _handle: &TcpHandle,
    _initial_body: &[u8],
    _content_length: Option<usize>,
) -> FetchResult<Vec<u8>> {
    // TODO: Implement body reading
    Ok(Vec::new())
}
