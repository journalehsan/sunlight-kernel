//! Network transport for fetch.
//!
//! On Linux host builds (`host-linux` feature): DNS/TCP/TLS via std + rustls.
//! On SunlightOS (`sunlightos` / no default features): IPC to net_server.

#[cfg(feature = "host-linux")]
use std::net::{SocketAddr, ToSocketAddrs};
#[cfg(feature = "host-linux")]
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(not(feature = "host-linux"))]
use core::sync::atomic::{AtomicBool, Ordering};

use crate::prelude::{format, String, Vec};

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

    #[cfg(feature = "host-linux")]
    fn socket_addr(&self, port: u16) -> SocketAddr {
        SocketAddr::from((self.octets, port))
    }
}

/// Handle to an open TCP connection
pub struct TcpHandle {
    #[cfg(feature = "host-linux")]
    conn: host::HostConnection,
    #[cfg(not(feature = "host-linux"))]
    conn: sunlight::OsConnection,
}

impl Drop for TcpHandle {
    fn drop(&mut self) {
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
    Ok(())
}

/// Resolve a hostname to an IPv4 address.
pub fn dns_resolve(hostname: &str) -> FetchResult<ResolvedAddr> {
    check_interrupted()?;
    resolve_impl(hostname)
}

/// Open a TCP connection.
pub fn tcp_connect(addr: &ResolvedAddr, port: u16) -> FetchResult<TcpHandle> {
    check_interrupted()?;
    tcp_connect_impl(addr, port)
}

impl TcpHandle {
    /// Send data on this TCP connection.
    pub fn send(&mut self, data: &[u8]) -> FetchResult<usize> {
        check_interrupted()?;
        send_impl(self, data)
    }

    /// Receive data from this TCP connection.
    pub fn recv(&mut self, max_len: usize) -> FetchResult<Vec<u8>> {
        check_interrupted()?;
        recv_impl(self, max_len)
    }

    /// Close this TCP connection.
    pub fn close(&mut self) -> FetchResult<()> {
        close_impl(self)
    }
}

/// Perform a complete HTTP request and return parsed headers plus the connection.
pub fn http_request(
    hostname: &str,
    addr: &ResolvedAddr,
    port: u16,
    use_tls: bool,
    request: &HttpRequest,
) -> FetchResult<(HttpResponse, TcpHandle, Vec<u8>)> {
    check_interrupted()?;
    http_request_impl(hostname, addr, port, use_tls, request)
}

/// Read the remaining body from a TCP handle.
pub fn read_body_full(
    handle: &mut TcpHandle,
    initial_body: &[u8],
    content_length: Option<usize>,
    progress: Option<&mut dyn FnMut(usize)>,
) -> FetchResult<Vec<u8>> {
    check_interrupted()?;
    read_body_full_impl(handle, initial_body, content_length, progress)
}

fn check_interrupted() -> FetchResult<()> {
    if is_interrupted() {
        Err(FetchError::Interrupted)
    } else {
        Ok(())
    }
}

// ── host-linux implementation ────────────────────────────────────────────────

#[cfg(feature = "host-linux")]
mod host {
    use super::*;
    use std::io::{ErrorKind, Read, Write};
    use std::net::{SocketAddr, TcpStream};
    use std::sync::Once;

    pub(crate) enum HostConnection {
        Plain(TcpStream),
        Tls(rustls::StreamOwned<rustls::ClientConnection, TcpStream>),
    }

    impl HostConnection {
        fn send_all(&mut self, data: &[u8]) -> FetchResult<()> {
            let mut written = 0;
            while written < data.len() {
                let n = match self {
                    Self::Plain(stream) => stream.write(&data[written..]),
                    Self::Tls(stream) => stream.write(&data[written..]),
                }
                .map_err(|e| FetchError::IoError(e.to_string()))?;
                if n == 0 {
                    return Err(FetchError::IoError(String::from(
                        "connection closed while sending request",
                    )));
                }
                written += n;
            }
            Ok(())
        }

        fn read_some(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            match self {
                Self::Plain(stream) => stream.read(buf),
                Self::Tls(stream) => stream.read(buf),
            }
        }
    }

    static TLS_INIT: Once = Once::new();

    fn init_tls() {
        TLS_INIT.call_once(|| {
            let _ = rustls::crypto::ring::default_provider().install_default();
        });
    }

    pub(super) fn resolve_impl(hostname: &str) -> FetchResult<ResolvedAddr> {
        let addrs: Vec<SocketAddr> = (hostname, 0)
            .to_socket_addrs()
            .map_err(|e| FetchError::DnsResolutionFailed(format!("{hostname}: {e}")))?
            .collect();

        let v4 = addrs
            .iter()
            .find(|a| a.is_ipv4())
            .ok_or_else(|| FetchError::DnsResolutionFailed(format!("{hostname}: no IPv4 address")))?;

        match v4.ip() {
            std::net::IpAddr::V4(ip) => Ok(ResolvedAddr {
                octets: ip.octets(),
            }),
            _ => Err(FetchError::DnsResolutionFailed(format!(
                "{hostname}: no IPv4 address"
            ))),
        }
    }

    pub(super) fn tcp_connect_impl(addr: &ResolvedAddr, port: u16) -> FetchResult<TcpHandle> {
        let socket_addr = addr.socket_addr(port);
        let stream = TcpStream::connect(socket_addr).map_err(|e| FetchError::ConnectionFailed {
            host: format!(
                "{}.{}.{}.{}",
                addr.octets[0], addr.octets[1], addr.octets[2], addr.octets[3]
            ),
            port,
            reason: e.to_string(),
        })?;
        let _ = stream.set_read_timeout(None);
        let _ = stream.set_write_timeout(None);
        let _ = stream.set_nodelay(true);

        Ok(TcpHandle {
            conn: HostConnection::Plain(stream),
        })
    }

    fn connect_tls(
        hostname: &str,
        addr: &ResolvedAddr,
        port: u16,
    ) -> FetchResult<HostConnection> {
        init_tls();

        let socket_addr = addr.socket_addr(port);
        let tcp = TcpStream::connect(socket_addr).map_err(|e| FetchError::ConnectionFailed {
            host: hostname.to_string(),
            port,
            reason: e.to_string(),
        })?;

        let mut root_store = rustls::RootCertStore::empty();
        root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

        let config = rustls::ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth();

        let server_name = rustls::pki_types::ServerName::try_from(hostname.to_string())
            .map_err(|_| FetchError::InvalidUrl(format!("invalid TLS server name: {hostname}")))?
            .to_owned();

        let tls = rustls::ClientConnection::new(std::sync::Arc::new(config), server_name)
            .map_err(|e| FetchError::ConnectionFailed {
                host: hostname.to_string(),
                port,
                reason: format!("TLS setup failed: {e}"),
            })?;

        Ok(HostConnection::Tls(rustls::StreamOwned::new(tls, tcp)))
    }

    pub(super) fn send_impl(handle: &mut TcpHandle, data: &[u8]) -> FetchResult<usize> {
        handle.conn.send_all(data)?;
        Ok(data.len())
    }

    pub(super) fn recv_impl(handle: &mut TcpHandle, max_len: usize) -> FetchResult<Vec<u8>> {
        let mut buf = vec![0u8; max_len];
        match handle.conn.read_some(&mut buf) {
            Ok(0) => Ok(Vec::new()),
            Ok(n) => {
                buf.truncate(n);
                Ok(buf)
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut => {
                Ok(Vec::new())
            }
            Err(e) => Err(FetchError::IoError(e.to_string())),
        }
    }

    pub(super) fn close_impl(handle: &mut TcpHandle) -> FetchResult<()> {
        match &mut handle.conn {
            HostConnection::Plain(stream) => {
                let _ = stream.shutdown(std::net::Shutdown::Both);
            }
            HostConnection::Tls(stream) => {
                let _ = stream.sock.shutdown(std::net::Shutdown::Both);
            }
        }
        Ok(())
    }

    pub(super) fn http_request_impl(
        hostname: &str,
        addr: &ResolvedAddr,
        port: u16,
        use_tls: bool,
        request: &HttpRequest,
    ) -> FetchResult<(HttpResponse, TcpHandle, Vec<u8>)> {
        let mut handle = if use_tls {
            TcpHandle {
                conn: connect_tls(hostname, addr, port)?,
            }
        } else {
            tcp_connect_impl(addr, port)?
        };

        let wire = request.serialize();
        handle.conn.send_all(&wire)?;

        let mut buf = Vec::with_capacity(8192);
        let mut scratch = [0u8; 8192];

        loop {
            let n = handle
                .conn
                .read_some(&mut scratch)
                .map_err(|e| FetchError::IoError(e.to_string()))?;
            if n == 0 {
                if buf.is_empty() {
                    return Err(FetchError::HttpError {
                        status: 0,
                        message: String::from("empty response from server"),
                    });
                }
                break;
            }
            buf.extend_from_slice(&scratch[..n]);

            if let Some(result) = HttpResponse::parse(&buf) {
                let response = result?;
                let body_start = response.header_len;
                let initial_body = buf[body_start..].to_vec();
                return Ok((response, handle, initial_body));
            }

            if buf.len() > 1024 * 1024 {
                return Err(FetchError::HttpError {
                    status: 0,
                    message: String::from("response headers too large"),
                });
            }
        }

        HttpResponse::parse(&buf)
            .ok_or_else(|| FetchError::HttpError {
                status: 0,
                message: String::from("incomplete response headers"),
            })?
            .map(|response| {
                let body_start = response.header_len;
                let initial_body = buf[body_start..].to_vec();
                (response, handle, initial_body)
            })
    }

    pub(super) fn read_body_full_impl(
        handle: &mut TcpHandle,
        initial_body: &[u8],
        content_length: Option<usize>,
        mut progress: Option<&mut dyn FnMut(usize)>,
    ) -> FetchResult<Vec<u8>> {
        let mut body = Vec::from(initial_body);
        if let Some(progress) = progress.as_deref_mut() {
            progress(initial_body.len());
        }

        if let Some(expected) = content_length {
            while body.len() < expected {
                let chunk = recv_impl(handle, 64 * 1024)?;
                if chunk.is_empty() {
                    break;
                }
                body.extend_from_slice(&chunk);
                if let Some(progress) = progress.as_deref_mut() {
                    progress(chunk.len());
                }
            }
            if body.len() > expected {
                body.truncate(expected);
            }
            if body.len() != expected {
                return Err(FetchError::ChunkIntegrityError {
                    chunk_id: 0,
                    expected,
                    got: body.len(),
                });
            }
            return Ok(body);
        }

        loop {
            let chunk = recv_impl(handle, 64 * 1024)?;
            if chunk.is_empty() {
                break;
            }
            body.extend_from_slice(&chunk);
            if let Some(progress) = progress.as_deref_mut() {
                progress(chunk.len());
            }
        }

        Ok(body)
    }
}

#[cfg(feature = "host-linux")]
use host::{
    close_impl, http_request_impl, read_body_full_impl, recv_impl, resolve_impl, send_impl,
    tcp_connect_impl,
};

// ── SunlightOS IPC via net_server ────────────────────────────────────────────

#[cfg(not(feature = "host-linux"))]
mod sunlight {
    use super::*;
    use sunlight_ipc::{ipc_call, nameserver_lookup, CapabilityToken, IpcMsg};
    use sunlight_net::netop::NetOp;

    const IPC_CHUNK: usize = 48;

    pub(super) enum OsConnection {
        Tcp { socket_id: u32 },
    }

    impl OsConnection {
        pub(super) fn send_all(&mut self, data: &[u8]) -> FetchResult<()> {
            match self {
                Self::Tcp { socket_id } => net_send(*socket_id, data),
            }
        }

        pub(super) fn read_some(&mut self, buf: &mut [u8]) -> FetchResult<usize> {
            match self {
                Self::Tcp { socket_id } => {
                    let chunk = net_recv(*socket_id, buf.len().min(IPC_CHUNK))?;
                    let n = chunk.len().min(buf.len());
                    buf[..n].copy_from_slice(&chunk[..n]);
                    Ok(n)
                }
            }
        }

        pub(super) fn close(&mut self) -> FetchResult<()> {
            match self {
                Self::Tcp { socket_id } => net_close(*socket_id),
            }
        }
    }

    fn net_cap() -> FetchResult<CapabilityToken> {
        nameserver_lookup("net").ok_or_else(|| {
            FetchError::IpcError(String::from("net service unavailable"))
        })
    }

    fn net_socket() -> FetchResult<u32> {
        let cap = net_cap()?;
        let reply = ipc_call(cap, IpcMsg::with_label(NetOp::SOCKET));
        if reply.label != NetOp::SOCKET || reply.words[0] == 0 {
            return Err(FetchError::IpcError(String::from("socket allocation failed")));
        }
        Ok(reply.words[0] as u32)
    }

    fn net_connect(socket_id: u32, addr: &ResolvedAddr, port: u16) -> FetchResult<()> {
        let cap = net_cap()?;
        let reply = ipc_call(
            cap,
            IpcMsg::with_label(NetOp::CONNECT)
                .word(0, socket_id as u64)
                .word(1, pack_ipv4(addr.octets))
                .word(2, port as u64),
        );
        if reply.label != NetOp::CONNECT || reply.words[0] == 0 {
            return Err(FetchError::ConnectionFailed {
                host: format!(
                    "{}.{}.{}.{}",
                    addr.octets[0], addr.octets[1], addr.octets[2], addr.octets[3]
                ),
                port,
                reason: String::from("TCP connect failed"),
            });
        }
        Ok(())
    }

    fn net_send(socket_id: u32, data: &[u8]) -> FetchResult<()> {
        let cap = net_cap()?;
        let mut offset = 0usize;
        while offset < data.len() {
            let chunk_len = (data.len() - offset).min(IPC_CHUNK);
            let mut msg = IpcMsg::with_label(NetOp::SEND)
                .word(0, socket_id as u64)
                .word(1, chunk_len as u64);
            for i in 0..chunk_len {
                let word_idx = 2 + i / 8;
                let byte_idx = i % 8;
                let byte = data[offset + i] as u64;
                msg.words[word_idx] |= byte << (byte_idx * 8);
            }
            let reply = ipc_call(cap, msg);
            let sent = reply.words[0] as usize;
            if sent == 0 {
                return Err(FetchError::IoError(String::from("TCP send failed")));
            }
            offset += sent;
        }
        Ok(())
    }

    fn net_recv(socket_id: u32, max_len: usize) -> FetchResult<Vec<u8>> {
        let cap = net_cap()?;
        let reply = ipc_call(
            cap,
            IpcMsg::with_label(NetOp::RECV)
                .word(0, socket_id as u64)
                .word(1, max_len as u64),
        );
        if reply.label != NetOp::RECV {
            return Err(FetchError::IoError(String::from("TCP recv failed")));
        }
        let len = reply.words[0] as usize;
        Ok(unpack_chunk(&reply.words, len))
    }

    fn net_close(socket_id: u32) -> FetchResult<()> {
        let cap = net_cap()?;
        let _ = ipc_call(
            cap,
            IpcMsg::with_label(NetOp::CLOSE).word(0, socket_id as u64),
        );
        Ok(())
    }

    fn pack_ipv4(ip: [u8; 4]) -> u64 {
        (ip[0] as u64)
            | ((ip[1] as u64) << 8)
            | ((ip[2] as u64) << 16)
            | ((ip[3] as u64) << 24)
    }

    fn unpack_chunk(words: &[u64; 8], len: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(len);
        for i in 0..len {
            let word = words[2 + i / 8];
            let byte_idx = i % 8;
            out.push(((word >> (byte_idx * 8)) & 0xff) as u8);
        }
        out
    }

    fn resolve_via_net(hostname: &str) -> FetchResult<ResolvedAddr> {
        let cap = net_cap()?;
        let bytes = hostname.as_bytes();
        let name_len = bytes.len().min(48);
        let mut msg = IpcMsg::with_label(NetOp::RESOLVE).word(0, name_len as u64);
        let mut w_idx = 1usize;
        let mut b_idx = 0usize;
        while b_idx < name_len && w_idx < 8 {
            let mut w = 0u64;
            for j in 0..8 {
                if b_idx >= name_len {
                    break;
                }
                w |= (bytes[b_idx] as u64) << (j * 8);
                b_idx += 1;
            }
            msg = msg.word(w_idx, w);
            w_idx += 1;
        }
        let reply = ipc_call(cap, msg);
        if reply.label != NetOp::RESOLVE || reply.word_count == 0 || reply.words[0] == 0 {
            return Err(FetchError::DnsResolutionFailed(String::from(hostname)));
        }
        let v = reply.words[0];
        Ok(ResolvedAddr {
            octets: [
                (v & 0xff) as u8,
                ((v >> 8) & 0xff) as u8,
                ((v >> 16) & 0xff) as u8,
                ((v >> 24) & 0xff) as u8,
            ],
        })
    }

    pub(super) fn resolve_impl(hostname: &str) -> FetchResult<ResolvedAddr> {
        resolve_via_net(hostname)
    }

    pub(super) fn tcp_connect_impl(addr: &ResolvedAddr, port: u16) -> FetchResult<TcpHandle> {
        let socket_id = net_socket()?;
        net_connect(socket_id, addr, port)?;
        Ok(TcpHandle {
            conn: OsConnection::Tcp { socket_id },
        })
    }

    pub(super) fn send_impl(handle: &mut TcpHandle, data: &[u8]) -> FetchResult<usize> {
        handle.conn.send_all(data)?;
        Ok(data.len())
    }

    pub(super) fn recv_impl(handle: &mut TcpHandle, max_len: usize) -> FetchResult<Vec<u8>> {
        let mut buf = vec![0u8; max_len];
        let n = handle.conn.read_some(&mut buf)?;
        buf.truncate(n);
        Ok(buf)
    }

    pub(super) fn close_impl(handle: &mut TcpHandle) -> FetchResult<()> {
        handle.conn.close()
    }

    pub(super) fn http_request_impl(
        _hostname: &str,
        addr: &ResolvedAddr,
        port: u16,
        use_tls: bool,
        request: &HttpRequest,
    ) -> FetchResult<(HttpResponse, TcpHandle, Vec<u8>)> {
        if use_tls {
            return Err(FetchError::IpcError(String::from(
                "HTTPS not yet supported in SunlightOS fetch (use http:// URLs for now)",
            )));
        }

        let socket_id = net_socket()?;
        net_connect(socket_id, addr, port)?;

        let mut handle = TcpHandle {
            conn: OsConnection::Tcp { socket_id },
        };

        let wire = request.serialize();
        handle.conn.send_all(&wire)?;

        let mut buf = Vec::with_capacity(8192);
        let mut scratch = [0u8; IPC_CHUNK];

        loop {
            let n = handle.conn.read_some(&mut scratch)?;
            if n == 0 {
                if buf.is_empty() {
                    return Err(FetchError::HttpError {
                        status: 0,
                        message: String::from("empty response from server"),
                    });
                }
                break;
            }
            buf.extend_from_slice(&scratch[..n]);

            if let Some(result) = HttpResponse::parse(&buf) {
                let response = result?;
                let body_start = response.header_len;
                let initial_body = buf[body_start..].to_vec();
                return Ok((response, handle, initial_body));
            }

            if buf.len() > 1024 * 1024 {
                return Err(FetchError::HttpError {
                    status: 0,
                    message: String::from("response headers too large"),
                });
            }
        }

        HttpResponse::parse(&buf)
            .ok_or_else(|| FetchError::HttpError {
                status: 0,
                message: String::from("incomplete response headers"),
            })?
            .map(|response| {
                let body_start = response.header_len;
                let initial_body = buf[body_start..].to_vec();
                (response, handle, initial_body)
            })
    }

    pub(super) fn read_body_full_impl(
        handle: &mut TcpHandle,
        initial_body: &[u8],
        content_length: Option<usize>,
        mut progress: Option<&mut dyn FnMut(usize)>,
    ) -> FetchResult<Vec<u8>> {
        let mut body = Vec::from(initial_body);
        if let Some(progress) = progress.as_deref_mut() {
            progress(initial_body.len());
        }

        if let Some(expected) = content_length {
            while body.len() < expected {
                let chunk = recv_impl(handle, 64 * 1024)?;
                if chunk.is_empty() {
                    break;
                }
                body.extend_from_slice(&chunk);
                if let Some(progress) = progress.as_deref_mut() {
                    progress(chunk.len());
                }
            }
            if body.len() > expected {
                body.truncate(expected);
            }
            if body.len() != expected {
                return Err(FetchError::ChunkIntegrityError {
                    chunk_id: 0,
                    expected,
                    got: body.len(),
                });
            }
            return Ok(body);
        }

        loop {
            let chunk = recv_impl(handle, 64 * 1024)?;
            if chunk.is_empty() {
                break;
            }
            body.extend_from_slice(&chunk);
            if let Some(progress) = progress.as_deref_mut() {
                progress(chunk.len());
            }
        }

        Ok(body)
    }
}

#[cfg(not(feature = "host-linux"))]
use sunlight::{
    close_impl, http_request_impl, read_body_full_impl, recv_impl, resolve_impl, send_impl,
    tcp_connect_impl,
};