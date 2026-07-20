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
    request_method: &str,
    response: &HttpResponse,
    initial_body: &[u8],
    progress: Option<&mut dyn FnMut(usize)>,
) -> FetchResult<Vec<u8>> {
    check_interrupted()?;
    read_body_full_impl(handle, request_method, response, initial_body, progress)
}

fn check_interrupted() -> FetchResult<()> {
    if is_interrupted() {
        Err(FetchError::Interrupted)
    } else {
        Ok(())
    }
}

fn response_has_no_body(request_method: &str, response: &HttpResponse) -> bool {
    request_method.eq_ignore_ascii_case("HEAD")
        || (100..200).contains(&response.status_code)
        || response.status_code == 204
        || response.status_code == 304
}

fn decode_chunked_body(encoded: &[u8]) -> FetchResult<Vec<u8>> {
    let mut decoded = Vec::new();
    let mut offset = 0usize;
    let mut chunk_id = 0usize;

    loop {
        let line_end = find_crlf(encoded, offset).ok_or(FetchError::HttpError {
            status: 0,
            message: String::from("invalid chunked body: missing chunk size line ending"),
        })?;
        let size_line = core::str::from_utf8(&encoded[offset..line_end]).map_err(|_| {
            FetchError::HttpError {
                status: 0,
                message: String::from("invalid chunked body: non-UTF8 chunk size"),
            }
        })?;
        let size_text = size_line.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_text, 16).map_err(|_| FetchError::HttpError {
            status: 0,
            message: format!("invalid chunked body: bad chunk size '{size_text}'"),
        })?;
        offset = line_end + 2;

        if size == 0 {
            consume_chunked_trailers(encoded, offset)?;
            return Ok(decoded);
        }

        let chunk_end = offset.saturating_add(size);
        if chunk_end > encoded.len() {
            return Err(FetchError::ChunkIntegrityError {
                chunk_id,
                expected: size,
                got: encoded.len().saturating_sub(offset),
            });
        }
        decoded.extend_from_slice(&encoded[offset..chunk_end]);
        offset = chunk_end;

        if encoded.get(offset..offset + 2) != Some(b"\r\n") {
            return Err(FetchError::HttpError {
                status: 0,
                message: String::from("invalid chunked body: missing chunk terminator"),
            });
        }
        offset += 2;
        chunk_id += 1;
    }
}

fn consume_chunked_trailers(encoded: &[u8], mut offset: usize) -> FetchResult<()> {
    loop {
        let line_end = find_crlf(encoded, offset).ok_or(FetchError::HttpError {
            status: 0,
            message: String::from("invalid chunked body: unterminated trailer"),
        })?;
        if line_end == offset {
            return Ok(());
        }
        offset = line_end + 2;
    }
}

fn find_crlf(bytes: &[u8], start: usize) -> Option<usize> {
    bytes[start..]
        .windows(2)
        .position(|window| window == b"\r\n")
        .map(|index| start + index)
}

// ── host-linux implementation ────────────────────────────────────────────────

#[cfg(feature = "host-linux")]
mod host {
    use super::*;
    use std::io::{ErrorKind, Read, Write};
    use std::net::{SocketAddr, TcpStream};
    use std::sync::Once;

    // ── rustls → serial (stderr) log bridge ──────────────────────────────────

    struct SerialLogger;

    impl log::Log for SerialLogger {
        fn enabled(&self, meta: &log::Metadata) -> bool {
            // Only rustls internals — avoids noise from other crates.
            meta.target().starts_with("rustls")
        }

        fn log(&self, record: &log::Record) {
            if self.enabled(record.metadata()) {
                eprintln!(
                    "[RUSTLS:{}] {}: {}",
                    record.level(),
                    record.target(),
                    record.args()
                );
            }
        }

        fn flush(&self) {}
    }

    static SERIAL_LOGGER: SerialLogger = SerialLogger;
    static LOGGER_INIT: Once = Once::new();

    fn init_serial_logger() {
        LOGGER_INIT.call_once(|| {
            // Ignore error: another logger may already be set (e.g. env_logger in tests).
            let _ = log::set_logger(&SERIAL_LOGGER);
            log::set_max_level(log::LevelFilter::Debug);
        });
    }

    // ─────────────────────────────────────────────────────────────────────────

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
            init_serial_logger();
            let _ = rustls::crypto::ring::default_provider().install_default();
        });
    }

    pub(super) fn resolve_impl(hostname: &str) -> FetchResult<ResolvedAddr> {
        let addrs: Vec<SocketAddr> = (hostname, 0)
            .to_socket_addrs()
            .map_err(|e| FetchError::DnsResolutionFailed(format!("{hostname}: {e}")))?
            .collect();

        let v4 = addrs.iter().find(|a| a.is_ipv4()).ok_or_else(|| {
            FetchError::DnsResolutionFailed(format!("{hostname}: no IPv4 address"))
        })?;

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

    fn connect_tls(hostname: &str, addr: &ResolvedAddr, port: u16) -> FetchResult<HostConnection> {
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

        let tls = rustls::ClientConnection::new(std::sync::Arc::new(config), server_name).map_err(
            |e| FetchError::ConnectionFailed {
                host: hostname.to_string(),
                port,
                reason: format!("TLS setup failed: {e}"),
            },
        )?;

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
            .map_err(Into::into)
    }

    pub(super) fn read_body_full_impl(
        handle: &mut TcpHandle,
        request_method: &str,
        response: &HttpResponse,
        initial_body: &[u8],
        mut progress: Option<&mut dyn FnMut(usize)>,
    ) -> FetchResult<Vec<u8>> {
        if response_has_no_body(request_method, response) {
            return Ok(Vec::new());
        }

        let mut body = Vec::from(initial_body);
        if let Some(progress) = progress.as_deref_mut() {
            progress(initial_body.len());
        }

        if response.is_chunked() {
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
            return decode_chunked_body(&body);
        }

        if let Some(expected) = response.content_length() {
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
    use sunlight_ipc::{
        debug_log, ipc_call, ipc_call_timeout, nameserver_lookup, shm_alloc, shm_free, shm_map,
        CapabilityToken, IpcMsg,
    };
    use sunlight_net::netop::{NetOp, NetReady, NetStatus};

    // Maximum bytes packed into one IPC call. Register IPC transports only
    // words[0..4] (r8/r9/r10/r12). words[0]=socket_id, words[1]=length, so
    // only words[2..4] = 2 words × 8 bytes = 16 bytes carry actual data.
    // words[4..7] are silently dropped by the kernel ABI.
    const IPC_CHUNK: usize = 16;

    // Bulk TCP I/O moves up to one shared 4 KiB page per IPC call (vs IPC_CHUNK
    // inline bytes), via NetOp::SEND_SHM / RECV_SHM.
    const SHM_PAGE: usize = 4096;

    // TLS IPC protocol labels — must match sunlight-tls/src/main.rs.
    // Design B: the daemon owns the socket + crypto; we exchange plaintext only.
    const TLS_CONNECT: u64 = 0x5401;
    const TLS_SEND: u64 = 0x5402;
    const TLS_RECV: u64 = 0x5403;
    const TLS_CLOSE: u64 = 0x5405;
    const TLS_REPLY: u64 = 0x54FF;
    const TLS_ERROR: u64 = 0x54EE;

    // TLS daemon error codes (word[0] in a TLS_ERROR reply).
    const TLS_ERR_SESSIONS_FULL: u64 = 3;
    const TLS_ERR_CERT_EXPIRED: u64 = 5;

    // One shared page per plaintext transfer to/from the daemon.
    const TLS_SHM_PAGE: usize = 4096;

    pub(super) enum OsConnection {
        Tcp { socket_id: u64 },
        // Plaintext-only TLS session in the daemon. `rx` buffers decrypted bytes
        // returned by a TLS_RECV (up to one page) that didn't fit the caller's
        // (48-byte) read buffer, so no data is lost across read_some calls.
        Tls { session_id: u64, rx: Vec<u8> },
    }

    impl OsConnection {
        pub(super) fn send_all(&mut self, data: &[u8]) -> FetchResult<()> {
            match self {
                Self::Tcp { socket_id } => net_send(*socket_id, data),
                Self::Tls { session_id, .. } => {
                    let sid = *session_id;
                    let mut offset = 0usize;
                    while offset < data.len() {
                        let end = (offset + TLS_SHM_PAGE).min(data.len());
                        tls_send(sid, &data[offset..end])?;
                        offset = end;
                    }
                    Ok(())
                }
            }
        }

        pub(super) fn read_some(&mut self, buf: &mut [u8]) -> FetchResult<usize> {
            match self {
                Self::Tcp { socket_id } => {
                    let chunk = net_recv(*socket_id, buf.len())?;
                    let n = chunk.len().min(buf.len());
                    buf[..n].copy_from_slice(&chunk[..n]);
                    Ok(n)
                }
                Self::Tls { session_id, rx } => {
                    if rx.is_empty() {
                        let (plain, _eof) = tls_recv(*session_id)?;
                        if plain.is_empty() {
                            return Ok(0); // EOF
                        }
                        *rx = plain;
                    }
                    let n = rx.len().min(buf.len());
                    buf[..n].copy_from_slice(&rx[..n]);
                    rx.drain(..n);
                    Ok(n)
                }
            }
        }

        pub(super) fn close(&mut self) -> FetchResult<()> {
            match self {
                Self::Tcp { socket_id } => net_close(*socket_id),
                Self::Tls { session_id, .. } => {
                    tls_close(*session_id);
                    Ok(())
                }
            }
        }
    }

    // ── net_server helpers ────────────────────────────────────────────────────

    fn net_cap() -> FetchResult<CapabilityToken> {
        nameserver_lookup("net")
            .ok_or_else(|| FetchError::IpcError(String::from("net service unavailable")))
    }

    fn net_socket() -> FetchResult<u64> {
        let cap = net_cap()?;
        let reply = ipc_call(cap, IpcMsg::with_label(NetOp::SOCKET));
        if reply.label != NetOp::SOCKET || reply.words[0] == 0 {
            return Err(FetchError::IpcError(String::from(
                "socket allocation failed",
            )));
        }
        Ok(reply.words[0])
    }

    fn net_connect(socket_id: u64, addr: &ResolvedAddr, port: u16) -> FetchResult<()> {
        let cap = net_cap()?;
        let reply = ipc_call(
            cap,
            IpcMsg::with_label(NetOp::CONNECT)
                .word(0, socket_id)
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
        net_wait_ready(socket_id, 20_000, NetReady::WRITE)
    }

    fn net_send(socket_id: u64, data: &[u8]) -> FetchResult<()> {
        let cap = net_cap()?;
        let mut offset = 0usize;
        while offset < data.len() {
            let chunk_len = (data.len() - offset).min(SHM_PAGE);
            let (ptr, tok) = shm_alloc()
                .map_err(|_| FetchError::IpcError(String::from("shm_alloc failed (TCP send)")))?;
            // SAFETY: page is one full 4 KiB page >= chunk_len bytes.
            unsafe {
                core::ptr::copy_nonoverlapping(data[offset..].as_ptr(), ptr, chunk_len);
            }
            let msg = IpcMsg::with_label(NetOp::SEND_SHM)
                .word(0, socket_id)
                .word(1, chunk_len as u64)
                .with_cap(0, tok);
            let reply = ipc_call(cap, msg);
            let _ = shm_free(tok);
            let sent = reply.words[0] as usize;
            if reply.words[1] == NetStatus::WOULD_BLOCK {
                net_wait_ready(socket_id, 8_000, NetReady::WRITE)?;
                continue;
            }
            if sent == 0 {
                return Err(FetchError::IoError(String::from("TCP send failed")));
            }
            offset += sent;
        }
        Ok(())
    }

    fn net_recv(socket_id: u64, max_len: usize) -> FetchResult<Vec<u8>> {
        let cap = net_cap()?;
        let want = max_len.min(SHM_PAGE).max(1);
        let (ptr, tok) = shm_alloc()
            .map_err(|_| FetchError::IpcError(String::from("shm_alloc failed (TCP recv)")))?;
        let reply = ipc_call(
            cap,
            IpcMsg::with_label(NetOp::RECV_SHM)
                .word(0, socket_id)
                .word(1, want as u64)
                .with_cap(0, tok),
        );
        if reply.label != NetOp::RECV_SHM {
            let _ = shm_free(tok);
            return Err(FetchError::IoError(String::from("TCP recv failed")));
        }
        if reply.words[1] == NetStatus::WOULD_BLOCK {
            let _ = shm_free(tok);
            net_wait_ready(socket_id, 8_000, NetReady::READ)?;
            return net_recv(socket_id, max_len);
        }
        if reply.words[1] != NetStatus::OK && reply.words[1] != NetStatus::EOF {
            let _ = shm_free(tok);
            return Err(FetchError::IoError(String::from("TCP recv failed")));
        }
        let len = (reply.words[0] as usize).min(SHM_PAGE);
        if len == 0 {
            let _ = shm_free(tok);
            return Ok(Vec::new()); // EOF / no data
        }
        // SAFETY: net_server copied `len` (<= SHM_PAGE) bytes into this page.
        let v = unsafe { core::slice::from_raw_parts(ptr, len) }.to_vec();
        let _ = shm_free(tok);
        Ok(v)
    }

    fn net_close(socket_id: u64) -> FetchResult<()> {
        let cap = net_cap()?;
        let _ = ipc_call(cap, IpcMsg::with_label(NetOp::CLOSE).word(0, socket_id));
        Ok(())
    }

    fn net_wait_ready(socket_id: u64, timeout_ms: u64, interest: u64) -> FetchResult<()> {
        let cap = net_cap()?;
        let (ptr, tok) = shm_alloc()
            .map_err(|_| FetchError::IpcError(String::from("shm_alloc failed (TCP wait)")))?;
        unsafe {
            *(ptr as *mut u64) = socket_id;
        }
        let reply = ipc_call_timeout(
            cap,
            IpcMsg::with_label(NetOp::WAIT)
                .word(0, 1)
                .word(1, timeout_ms)
                .word(2, interest)
                .with_cap(0, tok),
            timeout_ms.saturating_add(100),
        );
        let _ = shm_free(tok);
        let reply = reply.map_err(|_| FetchError::IoError(String::from("TCP wait timed out")))?;
        if reply.words[0] == NetStatus::OK {
            Ok(())
        } else {
            Err(FetchError::IoError(String::from("TCP wait failed")))
        }
    }

    fn pack_ipv4(ip: [u8; 4]) -> u64 {
        (ip[0] as u64) | ((ip[1] as u64) << 8) | ((ip[2] as u64) << 16) | ((ip[3] as u64) << 24)
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
        nameserver_lookup("sunlight-tls")
            .ok_or_else(|| FetchError::IpcError(String::from("sunlight-tls service unavailable")))
    }

    /// Open a real TLS session in the daemon: it connects the socket to
    /// `addr:port` and runs the rustls handshake with SNI = `host`. Returns the
    /// session id. (Design B: the daemon owns the socket + crypto.)
    fn tls_connect(host: &str, addr: &ResolvedAddr, port: u16) -> FetchResult<u64> {
        let cap = tls_cap()?;
        let mut msg = IpcMsg::with_label(TLS_CONNECT)
            .word(0, pack_ipv4(addr.octets))
            .word(1, port as u64);
        // Pack SNI hostname into words[2..7] (NUL-padded), up to 48 bytes.
        let hb = host.as_bytes();
        let mut i = 0usize;
        for w in 2..8 {
            let mut word = 0u64;
            for j in 0..8 {
                if i < hb.len() {
                    word |= (hb[i] as u64) << (j * 8);
                    i += 1;
                }
            }
            msg.words[w] = word;
            if i >= hb.len() {
                break;
            }
        }
        msg.word_count = 8;
        debug_log(&format!(
            "[FETCH-TLS] connect host={} port={}\n",
            host, port
        ));
        let reply = ipc_call(cap, msg);
        if reply.label == TLS_REPLY {
            let sid = reply.words[0];
            debug_log(&format!("[FETCH-TLS] hs_OK host={} sid={:#x}\n", host, sid));
            return Ok(sid);
        }
        if reply.label == TLS_ERROR {
            let code = reply.words[0];
            debug_log(&format!(
                "[FETCH-TLS] hs_FAIL host={} code={}\n",
                host, code
            ));
            return Err(match code {
                TLS_ERR_SESSIONS_FULL => {
                    FetchError::TlsHandshakeFailed(String::from("TLS sessions full on daemon"))
                }
                TLS_ERR_CERT_EXPIRED => FetchError::TlsCertExpired,
                _ => FetchError::TlsHandshakeFailed(format!("daemon error code {code}")),
            });
        }
        Err(FetchError::TlsHandshakeFailed(format!(
            "unexpected label {:#x} from sunlight-tls",
            reply.label
        )))
    }

    /// Send up to one shared page (<=4096 bytes) of plaintext; the daemon
    /// encrypts and transmits it over the session's socket.
    fn tls_send(sid: u64, data: &[u8]) -> FetchResult<()> {
        if data.is_empty() {
            return Ok(());
        }
        let cap = tls_cap()?;
        let (ptr, tok) = shm_alloc()
            .map_err(|_| FetchError::IpcError(String::from("shm_alloc failed (TLS send)")))?;
        let n = data.len().min(TLS_SHM_PAGE);
        unsafe {
            core::ptr::copy_nonoverlapping(data.as_ptr(), ptr, n);
        }
        let msg = IpcMsg::with_label(TLS_SEND)
            .word(0, sid)
            .word(1, n as u64)
            .with_cap(0, tok);
        let reply = ipc_call(cap, msg);
        let _ = shm_free(tok);
        if reply.label == TLS_REPLY && reply.words[0] == 0 {
            Ok(())
        } else {
            Err(FetchError::IoError(String::from("TLS_SEND failed")))
        }
    }

    /// Receive decrypted plaintext (up to one page) from the daemon. Returns
    /// (bytes, eof); empty bytes with eof=true means the peer closed.
    fn tls_recv(sid: u64) -> FetchResult<(Vec<u8>, bool)> {
        let cap = tls_cap()?;
        let reply = ipc_call(cap, IpcMsg::with_label(TLS_RECV).word(0, sid));
        if reply.label == TLS_ERROR {
            return Err(FetchError::IoError(String::from("TLS_RECV failed")));
        }
        let n = reply.words[0] as usize;
        let eof = reply.words[1] != 0;
        if n == 0 {
            return Ok((Vec::new(), eof));
        }
        let tok = reply.caps[0];
        if tok == CapabilityToken::INVALID {
            return Ok((Vec::new(), eof));
        }
        let ptr = shm_map(tok)
            .map_err(|_| FetchError::IoError(String::from("shm_map failed (TLS recv)")))?;
        let v = unsafe { core::slice::from_raw_parts(ptr, n.min(TLS_SHM_PAGE)) }.to_vec();
        let _ = shm_free(tok);
        Ok((v, eof))
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

    pub(super) fn recv_impl(handle: &mut TcpHandle, max_len: usize) -> FetchResult<Vec<u8>> {
        // One shared page is the bulk-transfer unit; never ask for more per call.
        let mut buf = [0u8; SHM_PAGE];
        let want = max_len.min(SHM_PAGE).max(1);
        let n = handle.conn.read_some(&mut buf[..want])?;
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
        // Establish the transport: a daemon-owned TLS session (rustls) for
        // https://, or a plain TCP socket for http://.
        let conn = if use_tls {
            let session_id = tls_connect(hostname, addr, port)?;
            OsConnection::Tls {
                session_id,
                rx: Vec::new(),
            }
        } else {
            let socket_id = net_socket()?;
            net_connect(socket_id, addr, port)?;
            OsConnection::Tcp { socket_id }
        };

        let mut handle = TcpHandle { conn };

        let wire = request.serialize();
        handle.conn.send_all(&wire)?;

        let mut buf = Vec::with_capacity(8192);
        let mut scratch = [0u8; SHM_PAGE];

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
            .map_err(Into::into)
    }

    pub(super) fn read_body_full_impl(
        handle: &mut TcpHandle,
        request_method: &str,
        response: &HttpResponse,
        initial_body: &[u8],
        mut progress: Option<&mut dyn FnMut(usize)>,
    ) -> FetchResult<Vec<u8>> {
        if response_has_no_body(request_method, response) {
            return Ok(Vec::new());
        }

        let mut body = Vec::from(initial_body);
        if let Some(progress) = progress.as_deref_mut() {
            progress(initial_body.len());
        }

        if response.is_chunked() {
            loop {
                let chunk = recv_impl(handle, SHM_PAGE)?;
                if chunk.is_empty() {
                    break;
                }
                body.extend_from_slice(&chunk);
                if let Some(progress) = progress.as_deref_mut() {
                    progress(chunk.len());
                }
            }
            return decode_chunked_body(&body);
        }

        if let Some(expected) = response.content_length() {
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
            let chunk = recv_impl(handle, SHM_PAGE)?;
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
