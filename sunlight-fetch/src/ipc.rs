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

use crate::prelude::{String, Vec};

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

// ── SunlightOS IPC via net_server + sunlight-tls ─────────────────────────────

#[cfg(not(feature = "host-linux"))]
mod sunlight {
    use super::*;
    use sunlight_ipc::{ipc_call, nameserver_lookup, CapabilityToken, IpcMsg};
    use sunlight_net::netop::NetOp;

    // Maximum bytes packed into one IPC call (words[2..7] = 6 words × 8 bytes).
    const IPC_CHUNK: usize = 48;

    // TLS IPC protocol labels — must match sunlight-tls/src/main.rs.
    const TLS_HANDSHAKE: u64 = 0x5401;
    const TLS_FEED: u64 = 0x5402;
    const TLS_GET_WRITE: u64 = 0x5403;
    const TLS_GET_PLAIN: u64 = 0x5404;
    const TLS_CLOSE: u64 = 0x5405;
    const TLS_REPLY: u64 = 0x54FF;
    const TLS_ERROR: u64 = 0x54EE;

    // TLS daemon error codes (word[0] in a TLS_ERROR reply).
    const TLS_ERR_SESSIONS_FULL: u64 = 3;
    const TLS_ERR_CERT_EXPIRED: u64 = 5;

    // Data bytes per TLS_FEED / TLS_GET_WRITE / TLS_GET_PLAIN call:
    // word[0] = session_id or length, words[1..7] = 6 × 8 = 48 bytes.
    const TLS_DATA_CHUNK: usize = IPC_CHUNK;

    pub(super) enum OsConnection {
        Tcp { socket_id: u32 },
        Tls { socket_id: u32, session_id: u64 },
    }

    impl OsConnection {
        pub(super) fn send_all(&mut self, data: &[u8]) -> FetchResult<()> {
            match self {
                Self::Tcp { socket_id } => net_send(*socket_id, data),
                Self::Tls { socket_id, session_id } => {
                    let sid = *session_id;
                    let sock = *socket_id;
                    let mut offset = 0usize;
                    while offset < data.len() {
                        let end = (offset + TLS_DATA_CHUNK).min(data.len());
                        tls_feed(sid, &data[offset..end])?;
                        let encrypted = tls_get_write(sid)?;
                        if !encrypted.is_empty() {
                            net_send(sock, &encrypted)?;
                        }
                        offset = end;
                    }
                    Ok(())
                }
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
                Self::Tls { socket_id, session_id } => {
                    let raw = net_recv(*socket_id, IPC_CHUNK)?;
                    if raw.is_empty() {
                        return Ok(0);
                    }
                    tls_feed(*session_id, &raw)?;
                    let plain = tls_get_plain(*session_id)?;
                    let n = plain.len().min(buf.len());
                    buf[..n].copy_from_slice(&plain[..n]);
                    Ok(n)
                }
            }
        }

        pub(super) fn close(&mut self) -> FetchResult<()> {
            match self {
                Self::Tcp { socket_id } => net_close(*socket_id),
                Self::Tls { socket_id, session_id } => {
                    tls_close(*session_id);
                    net_close(*socket_id)
                }
            }
        }
    }

    // ── net_server helpers ────────────────────────────────────────────────────

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

    // ── sunlight-tls IPC helpers ──────────────────────────────────────────────

    fn tls_cap() -> FetchResult<CapabilityToken> {
        nameserver_lookup("sunlight-tls").ok_or_else(|| {
            FetchError::IpcError(String::from("sunlight-tls service unavailable"))
        })
    }

    /// Pack up to TLS_DATA_CHUNK bytes into words[1..7] of msg.
    fn pack_tls_data(msg: &mut IpcMsg, data: &[u8]) {
        let len = data.len().min(TLS_DATA_CHUNK);
        for i in 0..len {
            let word_idx = 1 + i / 8;
            let byte_idx = i % 8;
            if word_idx < 8 {
                msg.words[word_idx] |= (data[i] as u64) << (byte_idx * 8);
            }
        }
    }

    /// Unpack data from a TLS_GET_WRITE / TLS_GET_PLAIN reply.
    /// words[0] = byte count; words[1..7] = data.
    fn unpack_tls_reply_data(words: &[u64; 8]) -> Vec<u8> {
        let len = (words[0] as usize).min(TLS_DATA_CHUNK);
        let mut out = Vec::with_capacity(len);
        for i in 0..len {
            let word_idx = 1 + i / 8;
            let byte_idx = i % 8;
            if word_idx < 8 {
                out.push(((words[word_idx] >> (byte_idx * 8)) & 0xff) as u8);
            }
        }
        out
    }

    /// Initiate a client-mode TLS handshake with the given SNI hostname.
    /// Returns the session_id on success.
    fn tls_handshake(hostname: &str) -> FetchResult<u64> {
        let cap = tls_cap()?;
        let sni = hostname.as_bytes();
        let mut msg = IpcMsg::with_label(TLS_HANDSHAKE).word(0, 1u64); // 1 = client
        pack_tls_data(&mut msg, sni);
        let reply = ipc_call(cap, msg);
        if reply.label == TLS_ERROR {
            return Err(match reply.words[0] {
                TLS_ERR_SESSIONS_FULL => {
                    FetchError::TlsHandshakeFailed(String::from("TLS sessions full on daemon"))
                }
                TLS_ERR_CERT_EXPIRED => FetchError::TlsCertExpired,
                code => FetchError::TlsHandshakeFailed(format!("daemon error code {code:#x}")),
            });
        }
        if reply.label != TLS_REPLY {
            return Err(FetchError::TlsHandshakeFailed(format!(
                "unexpected label {:#x} from sunlight-tls",
                reply.label
            )));
        }
        Ok(reply.words[0]) // session_id
    }

    /// Feed a plaintext or ciphertext chunk into the TLS daemon for processing.
    /// Chunks larger than TLS_DATA_CHUNK are split across multiple IPC calls.
    fn tls_feed(sid: u64, data: &[u8]) -> FetchResult<()> {
        if data.is_empty() {
            return Ok(());
        }
        let cap = tls_cap()?;
        let mut offset = 0usize;
        while offset < data.len() {
            let end = (offset + TLS_DATA_CHUNK).min(data.len());
            let mut msg = IpcMsg::with_label(TLS_FEED).word(0, sid);
            pack_tls_data(&mut msg, &data[offset..end]);
            let reply = ipc_call(cap, msg);
            if reply.label == TLS_ERROR {
                return Err(FetchError::TlsHandshakeFailed(String::from(
                    "TLS_FEED rejected by daemon",
                )));
            }
            offset = end;
        }
        Ok(())
    }

    /// Drain the "to-write" (outbound ciphertext) buffer from the TLS daemon.
    fn tls_get_write(sid: u64) -> FetchResult<Vec<u8>> {
        let cap = tls_cap()?;
        let reply = ipc_call(cap, IpcMsg::with_label(TLS_GET_WRITE).word(0, sid));
        if reply.label == TLS_ERROR {
            return Err(FetchError::TlsHandshakeFailed(String::from(
                "TLS_GET_WRITE failed",
            )));
        }
        Ok(unpack_tls_reply_data(&reply.words))
    }

    /// Drain the "to-plain" (inbound plaintext) buffer from the TLS daemon.
    fn tls_get_plain(sid: u64) -> FetchResult<Vec<u8>> {
        let cap = tls_cap()?;
        let reply = ipc_call(cap, IpcMsg::with_label(TLS_GET_PLAIN).word(0, sid));
        if reply.label == TLS_ERROR {
            return Err(FetchError::TlsHandshakeFailed(String::from(
                "TLS_GET_PLAIN failed",
            )));
        }
        Ok(unpack_tls_reply_data(&reply.words))
    }

    /// Close a TLS session in the daemon (best-effort, ignores errors).
    fn tls_close(sid: u64) {
        if let Some(cap) = nameserver_lookup("sunlight-tls") {
            let _ = ipc_call(cap, IpcMsg::with_label(TLS_CLOSE).word(0, sid));
        }
    }

    // ── DNS / TCP / HTTP public impls ─────────────────────────────────────────

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

    pub(super) fn recv_impl(handle: &mut TcpHandle, _max_len: usize) -> FetchResult<Vec<u8>> {
        // Cap allocation to IPC_CHUNK — we can never receive more than that per
        // kernel IPC call, so allocating 64 KiB would just drain the bump heap.
        let mut buf = [0u8; IPC_CHUNK];
        let n = handle.conn.read_some(&mut buf)?;
        Ok(buf[..n].to_vec())
    }

    pub(super) fn close_impl(handle: &mut TcpHandle) -> FetchResult<()> {
        handle.conn.close()
    }

    pub(super) fn http_request_impl(
        hostname: &str,
        addr: &ResolvedAddr,
        port: u16,
        use_tls: bool,
        request: &HttpRequest,
    ) -> FetchResult<(HttpResponse, TcpHandle, Vec<u8>)> {
        // sunlight-tls is a stub that echoes bytes without a real handshake.
        // Sending raw HTTP to port 443 causes the remote to close/reset the
        // connection and net_server to spin in an 8-second recv timeout.
        // Fail fast until rustls + ring land on x86_64-unknown-none (see TODO.md).
        if use_tls {
            return Err(FetchError::TlsHandshakeFailed(String::from(
                "HTTPS not yet supported on SunlightOS; use http:// for now",
            )));
        }

        let socket_id = net_socket()?;
        net_connect(socket_id, addr, port)?;

        let conn = OsConnection::Tcp { socket_id };

        let mut handle = TcpHandle { conn };

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
                let chunk = recv_impl(handle, expected - body.len())?;
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
            let chunk = recv_impl(handle, IPC_CHUNK)?;
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