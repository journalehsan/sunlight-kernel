//! Bounded TCP server wrappers for Solar.

use sunlight_ipc::{
    ipc_call, ipc_call_timeout, nameserver_lookup, shm_alloc, shm_free, CapabilityToken, IpcMsg,
};
use sunlight_net::netop::NetReady;
use sunlight_net::netop::{NetOp, NetStatus};

/// `net_server` accepts at most this many opaque identities in one wait-set.
pub const MAX_WAIT_SOCKETS: usize = 32;
/// Solar reserves one wait-set entry for its listener.
pub const MAX_ACTIVE_CONNS: usize = MAX_WAIT_SOCKETS - 1;

pub struct TcpListener {
    socket_id: u64,
    net_endpoint: CapabilityToken,
}

impl TcpListener {
    pub fn bind(port: u16) -> Result<Self, &'static str> {
        Self::bind_addr([0, 0, 0, 0], port)
    }

    pub fn bind_addr(addr: [u8; 4], port: u16) -> Result<Self, &'static str> {
        let net_endpoint =
            nameserver_lookup("net").ok_or("Could not find net_server in nameserver")?;
        let reply = ipc_call(net_endpoint, IpcMsg::with_label(NetOp::SOCKET));
        let socket_id = reply.words[0];
        if socket_id == 0 || reply.words[1] != NetStatus::OK {
            return Err("socket allocation failed");
        }

        let reply = ipc_call(
            net_endpoint,
            IpcMsg::with_label(NetOp::BIND)
                .word(0, socket_id)
                .word(1, pack_ipv4(addr))
                .word(2, port as u64),
        );
        if reply.words[1] != NetStatus::OK {
            close_socket(net_endpoint, socket_id);
            return Err("bind failed");
        }

        let reply = ipc_call(
            net_endpoint,
            IpcMsg::with_label(NetOp::LISTEN)
                .word(0, socket_id)
                .word(1, MAX_ACTIVE_CONNS as u64),
        );
        if reply.words[1] != NetStatus::OK {
            close_socket(net_endpoint, socket_id);
            return Err("listen failed");
        }

        Ok(Self {
            socket_id,
            net_endpoint,
        })
    }

    pub fn try_accept(&self) -> Result<Option<TcpStream>, &'static str> {
        let reply = ipc_call(
            self.net_endpoint,
            IpcMsg::with_label(NetOp::ACCEPT).word(0, self.socket_id),
        );
        match reply.words[1] {
            NetStatus::OK if reply.words[0] != 0 => Ok(Some(TcpStream {
                socket_id: reply.words[0],
                net_endpoint: self.net_endpoint,
            })),
            NetStatus::WOULD_BLOCK => Ok(None),
            _ => Err("accept failed"),
        }
    }

    pub fn socket_id(&self) -> u64 {
        self.socket_id
    }

    pub fn net_endpoint(&self) -> CapabilityToken {
        self.net_endpoint
    }

    pub fn close(&mut self) {
        if self.socket_id != 0 {
            close_socket(self.net_endpoint, self.socket_id);
            self.socket_id = 0;
        }
    }
}

pub struct TcpStream {
    pub socket_id: u64,
    pub net_endpoint: CapabilityToken,
}

impl TcpStream {
    pub fn read(&mut self, buffer: &mut [u8]) -> Result<usize, &'static str> {
        let (ptr, token) = shm_alloc().map_err(|_| "could not allocate read page")?;
        let reply = ipc_call(
            self.net_endpoint,
            IpcMsg::with_label(NetOp::RECV_SHM)
                .word(0, self.socket_id)
                .word(1, buffer.len().min(4096) as u64)
                .with_cap(0, token),
        );
        match reply.words[1] {
            NetStatus::EOF => {
                let _ = shm_free(token);
                return Ok(0);
            }
            NetStatus::WOULD_BLOCK => {
                let _ = shm_free(token);
                return Err("read would block");
            }
            NetStatus::OK => {}
            _ => {
                let _ = shm_free(token);
                return Err("read failed");
            }
        }
        let bytes_read = reply.words[0] as usize;
        if bytes_read == 0 {
            let _ = shm_free(token);
            return Ok(0);
        }
        let copy_len = bytes_read.min(buffer.len()).min(4096);
        unsafe {
            core::ptr::copy_nonoverlapping(ptr, buffer.as_mut_ptr(), copy_len);
        }
        let _ = shm_free(token);
        Ok(copy_len)
    }

    pub fn write_all(
        &mut self,
        data: &[u8],
        shm_pool: &crate::ShmPagePool,
    ) -> Result<(), &'static str> {
        let mut offset = 0usize;
        while offset < data.len() {
            let chunk_len = (data.len() - offset).min(4096);
            let page = shm_pool.acquire().ok_or("SHM pool exhausted")?;
            unsafe {
                core::ptr::copy_nonoverlapping(data[offset..].as_ptr(), page.ptr, chunk_len);
            }
            let reply = ipc_call(
                self.net_endpoint,
                IpcMsg::with_label(NetOp::SEND_SHM)
                    .word(0, self.socket_id)
                    .word(1, chunk_len as u64)
                    .with_cap(0, page.token),
            );
            shm_pool.release(page);
            if reply.words[1] == NetStatus::WOULD_BLOCK {
                match wait_socket_ready(self.net_endpoint, self.socket_id, 8_000, NetReady::WRITE)?
                {
                    Some(event) if event.readiness & NetReady::WRITE != 0 => continue,
                    Some(_) => return Err("write failed"),
                    None => return Err("write timed out"),
                }
            }
            if reply.words[1] != NetStatus::OK || reply.words[0] == 0 {
                return Err("write failed");
            }
            offset += reply.words[0] as usize;
        }
        Ok(())
    }

    pub fn close(&mut self) {
        if self.socket_id != 0 {
            let _ = ipc_call(
                self.net_endpoint,
                IpcMsg::with_label(NetOp::CLOSE).word(0, self.socket_id),
            );
            self.socket_id = 0;
        }
    }
}

pub struct WaitEvent {
    pub socket_id: u64,
    pub readiness: u64,
}

/// Sleep until a watched socket becomes ready. The identities live in one
/// shared page so the ABI never truncates the wait-set.
pub fn wait_sockets(
    net_endpoint: CapabilityToken,
    socket_ids: &[u64],
    timeout_ms: u64,
) -> Result<Option<WaitEvent>, &'static str> {
    wait_sockets_with_interest(
        net_endpoint,
        socket_ids,
        timeout_ms,
        NetReady::ACCEPT | NetReady::READ,
    )
}

fn wait_socket_ready(
    net_endpoint: CapabilityToken,
    socket_id: u64,
    timeout_ms: u64,
    interest: u64,
) -> Result<Option<WaitEvent>, &'static str> {
    wait_sockets_with_interest(net_endpoint, &[socket_id], timeout_ms, interest)
}

fn wait_sockets_with_interest(
    net_endpoint: CapabilityToken,
    socket_ids: &[u64],
    timeout_ms: u64,
    interest: u64,
) -> Result<Option<WaitEvent>, &'static str> {
    let count = socket_ids.len().min(MAX_WAIT_SOCKETS);
    if count == 0 {
        return Ok(None);
    }
    let (ptr, token) = shm_alloc().map_err(|_| "could not allocate wait-set page")?;
    unsafe {
        let out = core::slice::from_raw_parts_mut(ptr as *mut u64, count);
        out.copy_from_slice(&socket_ids[..count]);
    }
    let reply = ipc_call_timeout(
        net_endpoint,
        IpcMsg::with_label(NetOp::WAIT)
            .word(0, count as u64)
            .word(1, timeout_ms)
            .word(2, interest)
            .with_cap(0, token),
        timeout_ms.saturating_add(100),
    );
    let _ = shm_free(token);
    let reply = reply.map_err(|_| "wait IPC timed out")?;
    match reply.words[0] {
        NetStatus::OK => Ok(Some(WaitEvent {
            socket_id: reply.words[1],
            readiness: reply.words[2],
        })),
        NetStatus::TIMEOUT | NetStatus::WOULD_BLOCK => Ok(None),
        _ => Err("socket wait failed"),
    }
}

fn pack_ipv4(ip: [u8; 4]) -> u64 {
    (ip[0] as u64) | ((ip[1] as u64) << 8) | ((ip[2] as u64) << 16) | ((ip[3] as u64) << 24)
}

fn close_socket(net_endpoint: CapabilityToken, socket_id: u64) {
    let _ = ipc_call(
        net_endpoint,
        IpcMsg::with_label(NetOp::CLOSE).word(0, socket_id),
    );
}

impl Drop for TcpListener {
    fn drop(&mut self) {
        self.close();
    }
}

impl Drop for TcpStream {
    fn drop(&mut self) {
        self.close();
    }
}
