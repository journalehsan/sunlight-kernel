//! Executable host loopback callback transport, for local integration testing.
//! Native TCP rejects bind(port=0); this is NOT a deployed SunlightOS listener.
use crate::{
    AccessPolicy, AccountResult, Broker, Entropy, Error, KvClient, Result, Started, TokenTransport,
};
use alloc::{format, string::String};
use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
    time::{Duration, Instant},
};

pub struct OsEntropy;
impl Entropy for OsEntropy {
    fn fill(&mut self, bytes: &mut [u8]) -> Result<()> {
        getrandom::getrandom(bytes).map_err(|_| Error::Entropy)
    }
}
pub struct Loopback {
    listener: TcpListener,
    pub redirect: String,
}
impl Loopback {
    pub fn bind() -> Result<Self> {
        let listener =
            TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).map_err(|_| Error::Callback)?;
        listener
            .set_nonblocking(true)
            .map_err(|_| Error::Callback)?;
        let port = listener.local_addr().map_err(|_| Error::Callback)?.port();
        Ok(Self {
            listener,
            redirect: format!("http://127.0.0.1:{port}/oauth/callback"),
        })
    }
    /// Bind happened before authorization URL generation or browser launch.
    /// The opener must use a trusted browser launch path, never page JavaScript.
    pub fn start<K: KvClient, E: Entropy, T: TokenTransport, P: AccessPolicy>(
        &self,
        broker: &mut Broker<K, E, T, P>,
        scopes: &[&str],
        now: u64,
        open_browser: impl FnOnce(&str) -> Result<()>,
    ) -> Result<Started> {
        let started = broker.start_authorization(scopes, &self.redirect, now)?;
        if let Err(error) = open_browser(&started.authorization_url) {
            let _ = broker.cancel_authorization(&started.session);
            return Err(error);
        }
        Ok(started)
    }
    pub fn complete<K: KvClient, E: Entropy, T: TokenTransport, P: AccessPolicy>(
        self,
        broker: &mut Broker<K, E, T, P>,
        now: u64,
        timeout: Duration,
    ) -> Result<AccountResult> {
        let began = Instant::now();
        loop {
            if began.elapsed() >= timeout {
                return Err(Error::Timeout);
            }
            match self.listener.accept() {
                Ok((mut stream, peer)) => {
                    if !peer.ip().is_loopback() {
                        return Err(Error::Callback);
                    }
                    let left = timeout
                        .saturating_sub(began.elapsed())
                        .max(Duration::from_millis(1));
                    stream
                        .set_read_timeout(Some(left))
                        .map_err(|_| Error::Callback)?;
                    stream
                        .set_write_timeout(Some(left))
                        .map_err(|_| Error::Callback)?;
                    let mut buf = [0; 8192];
                    let mut n = 0;
                    while n < buf.len() && !buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
                        let remaining = timeout
                            .checked_sub(began.elapsed())
                            .filter(|d| !d.is_zero())
                            .ok_or(Error::Timeout)?;
                        stream
                            .set_read_timeout(Some(remaining))
                            .map_err(|_| Error::Callback)?;
                        let count = stream.read(&mut buf[n..]).map_err(|_| Error::Timeout)?;
                        if count == 0 {
                            break;
                        }
                        n += count;
                    }
                    let text = core::str::from_utf8(&buf[..n]).map_err(|_| Error::Callback)?;
                    let (line, _) = text.split_once("\r\n").ok_or(Error::Callback)?;
                    let mut fields = line.split(' ');
                    if fields.next() != Some("GET") {
                        return Err(Error::Callback);
                    }
                    let path = fields.next().ok_or(Error::Callback)?;
                    if fields.next() != Some("HTTP/1.1")
                        || fields.next().is_some()
                        || !text.ends_with("\r\n\r\n")
                    {
                        return Err(Error::Callback);
                    }
                    let authority = self
                        .redirect
                        .strip_prefix("http://")
                        .unwrap()
                        .split('/')
                        .next()
                        .unwrap();
                    let hosts: alloc::vec::Vec<_> = text
                        .split("\r\n")
                        .filter_map(|line| line.split_once(':'))
                        .filter(|(k, _)| k.eq_ignore_ascii_case("host"))
                        .collect();
                    if hosts.len() != 1
                        || hosts[0].1.trim() != authority
                        || !path.starts_with("/oauth/callback?")
                    {
                        return Err(Error::Callback);
                    }
                    let callback = format!("http://{authority}{path}");
                    let elapsed = began.elapsed().as_secs();
                    let result =
                        broker.complete_authorization(&callback, now.saturating_add(elapsed));
                    // Never echo the URL, code, state, provider response or tokens.
                    // A 303 clears the credential URL and the listener is then dropped.
                    let _ = stream.write_all(b"HTTP/1.1 303 See Other\r\nLocation: about:blank\r\nContent-Length: 0\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\n\r\n");
                    return result;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(2))
                }
                Err(_) => return Err(Error::Callback),
            }
        }
    }
}
