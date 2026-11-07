//! Bounded native TCP IPC. Each socket is closed on every return path.
use alloc::{string::String, vec::Vec};
use sunlight_account_broker::{Error, Result};
use sunlight_ipc::{
    ipc_call_timeout, monotonic_millis, nameserver_lookup, process_yield, shm_alloc, shm_free,
    CapabilityToken, IpcMsg,
};
use sunlight_net::netop::{NetOp, NetStatus};

pub struct Socket {
    cap: CapabilityToken,
    pub id: u64,
}
fn call(cap: CapabilityToken, msg: IpcMsg, deadline: u64) -> Result<IpcMsg> {
    let left = deadline.saturating_sub(monotonic_millis()).min(1000);
    if left == 0 {
        return Err(Error::Timeout);
    }
    ipc_call_timeout(cap, msg, left).map_err(|_| Error::Timeout)
}
impl Socket {
    pub fn new(deadline: u64) -> Result<Self> {
        let cap = nameserver_lookup("net").ok_or(Error::Tls)?;
        let r = call(cap, IpcMsg::with_label(NetOp::SOCKET), deadline)?;
        if r.words[0] == 0 {
            return Err(Error::Capacity);
        }
        Ok(Self {
            cap,
            id: r.words[0],
        })
    }
    pub fn bind_loopback(deadline: u64) -> Result<(Self, u16)> {
        Self::bind_loopback_port(0, deadline)
    }
    pub fn bind_loopback_port(port: u16, deadline: u64) -> Result<(Self, u16)> {
        let socket = Self::new(deadline)?;
        socket.ok(NetOp::BIND, 0x0100007f, port as u64, deadline)?;
        let (_, port) = socket.endpoint(false, deadline)?;
        socket.ok(NetOp::LISTEN, 1, 0, deadline)?;
        Ok((socket, port))
    }
    fn ok(&self, label: u64, a: u64, b: u64, deadline: u64) -> Result<IpcMsg> {
        let r = call(
            self.cap,
            IpcMsg::with_label(label)
                .word(0, self.id)
                .word(1, a)
                .word(2, b),
            deadline,
        )?;
        if r.words[1] != NetStatus::OK {
            return Err(Error::Tls);
        }
        Ok(r)
    }
    pub fn endpoint(&self, peer: bool, deadline: u64) -> Result<([u8; 4], u16)> {
        let r = call(
            self.cap,
            IpcMsg::with_label(if peer {
                NetOp::GET_PEERNAME
            } else {
                NetOp::GET_SOCKNAME
            })
            .word(0, self.id),
            deadline,
        )?;
        if r.words[2] != NetStatus::OK {
            return Err(Error::Tls);
        }
        Ok(((r.words[0] as u32).to_le_bytes(), r.words[1] as u16))
    }
    pub fn accept(&self, deadline: u64) -> Result<Option<Self>> {
        let r = call(
            self.cap,
            IpcMsg::with_label(NetOp::ACCEPT).word(0, self.id),
            deadline,
        )?;
        if r.words[1] == NetStatus::WOULD_BLOCK {
            return Ok(None);
        }
        if r.words[1] != NetStatus::OK || r.words[0] == 0 {
            return Err(Error::Tls);
        }
        Ok(Some(Self {
            cap: self.cap,
            id: r.words[0],
        }))
    }
    pub fn send(&self, data: &[u8], deadline: u64) -> Result<()> {
        let mut offset = 0;
        while offset < data.len() {
            let count = (data.len() - offset).min(16);
            let mut msg = IpcMsg::with_label(NetOp::SEND)
                .word(0, self.id)
                .word(1, count as u64);
            for (i, b) in data[offset..offset + count].iter().enumerate() {
                msg.words[2 + i / 8] |= (*b as u64) << (i % 8 * 8);
            }
            msg.word_count = 4;
            let r = call(self.cap, msg, deadline)?;
            if r.words[1] == NetStatus::WOULD_BLOCK {
                process_yield();
                continue;
            }
            if r.words[1] != NetStatus::OK || r.words[0] == 0 {
                return Err(Error::Tls);
            }
            offset += r.words[0] as usize;
        }
        Ok(())
    }
    pub fn read_request(&self, deadline: u64) -> Result<String> {
        let (ptr, token) = shm_alloc().map_err(|_| Error::Capacity)?;
        let result = (|| {
            let mut bytes = Vec::new();
            while bytes.len() < 8192 {
                let r = call(
                    self.cap,
                    IpcMsg::with_label(NetOp::RECV_SHM)
                        .word(0, self.id)
                        .word(1, 4096)
                        .with_cap(0, token),
                    deadline,
                )?;
                if r.words[1] == NetStatus::WOULD_BLOCK {
                    process_yield();
                    continue;
                }
                let n = (r.words[0] as usize).min(4096);
                if r.words[1] != NetStatus::OK || n == 0 {
                    return Err(Error::Callback);
                }
                bytes.extend_from_slice(unsafe { core::slice::from_raw_parts(ptr, n) });
                if bytes.windows(4).any(|w| w == b"\r\n\r\n") {
                    return String::from_utf8(bytes).map_err(|_| Error::Callback);
                }
            }
            Err(Error::Callback)
        })();
        let _ = shm_free(token);
        result
    }
    pub fn connect_loopback(&self, port: u16, deadline: u64) -> Result<()> {
        self.ok(NetOp::CONNECT, 0x0100007f, port as u64, deadline)?;
        Ok(())
    }
}
impl Drop for Socket {
    fn drop(&mut self) {
        let _ = ipc_call_timeout(
            self.cap,
            IpcMsg::with_label(NetOp::CLOSE).word(0, self.id),
            1000,
        );
    }
}

pub fn probe() -> Result<()> {
    let deadline = monotonic_millis() + 5000;
    let (listener, port) = Socket::bind_loopback(deadline)?;
    if port == 0 {
        return Err(Error::Callback);
    }
    let client = Socket::new(deadline)?;
    client.connect_loopback(port, deadline)?;
    let server = loop {
        if let Some(socket) = listener.accept(deadline)? {
            break socket;
        }
        process_yield();
    };
    if server.endpoint(true, deadline)?.0 != [127, 0, 0, 1] {
        return Err(Error::Callback);
    }
    client.send(b"GET /probe HTTP/1.1\r\nHost: localhost\r\n\r\n", deadline)?;
    if !server.read_request(deadline)?.starts_with("GET /probe ") {
        return Err(Error::Callback);
    }
    Ok(())
}
