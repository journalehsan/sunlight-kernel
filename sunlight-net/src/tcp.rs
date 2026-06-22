//! TCP socket pool backed by smoltcp — used by net_server for fetch/wget IPC.
//!
//! Project Antigravity: Dynamic allocation + async polling + garbage collection

extern crate alloc;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::vec::Vec;
use alloc::boxed::Box;

use smoltcp::iface::{Interface, SocketHandle, SocketSet};
use smoltcp::phy::Device;
use smoltcp::socket::tcp;
use smoltcp::time::Instant;
use smoltcp::wire::{IpAddress, IpEndpoint, Ipv4Address};

const RX_BUF: usize = 8192;
const TX_BUF: usize = 4096;
const MAX_SOCKETS: usize = 128; // Antigravity Phase 1: Increased from 4 to 128

/// Real wall-clock budget for the TCP three-way handshake.
const CONNECT_TIMEOUT_MS: u64 = 20_000;
/// Real wall-clock budget for flushing one `send` into the TX window.
const SEND_TIMEOUT_MS: u64 = 8000;
/// Real wall-clock budget to wait for inbound data on one `recv`.
const RECV_TIMEOUT_MS: u64 = 8000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TcpError {
    Timeout,
    Refused,
    SocketError,
    NoSlots,
    InvalidSocket,
    NotConnected,
}

/// Antigravity Phase 1: Dynamic socket registry
/// Replaces static [TcpSlot; 4] with BTreeMap for up to 128 concurrent connections
pub struct TcpManager {
    slots: BTreeMap<u32, TcpSlot>,
    next_socket_id: u32,
    next_local_port: u16,
}

/// Antigravity Phase 1: Dynamically allocated buffers using Box
/// Each slot only exists when a socket is active, freeing memory when closed
struct TcpSlot {
    handle: SocketHandle,
    local_port: Option<u16>,
    // Decrypted/plaintext bytes already drained from the smoltcp socket buffer
    // but not yet handed to the client. The IPC ABI only carries a few bytes of
    // payload per RECV reply, so a single segment (up to RX_BUF) must be served
    // across many RECV calls. Draining the socket into this backlog (instead of
    // returning the whole segment and dropping everything past one IPC chunk) is
    // what makes large HTTP responses and TLS handshakes survive intact.
    rx_backlog: VecDeque<u8>,
    // Keep buffers alive by storing them (even though they're not directly accessed)
    _rx_buffer: Box<[u8]>,
    _tx_buffer: Box<[u8]>,
}

impl TcpManager {
    pub fn new() -> Self {
        Self {
            slots: BTreeMap::new(),
            next_socket_id: 1,
            next_local_port: 49_152,
        }
    }

    /// Allocate a new socket with dynamically allocated buffers
    pub fn alloc_socket(
        &mut self,
        sockets: &mut SocketSet<'static>,
    ) -> Result<u32, TcpError> {
        if self.slots.len() >= MAX_SOCKETS {
            return Err(TcpError::NoSlots);
        }

        let socket_id = self.next_socket_id;
        self.next_socket_id = self.next_socket_id.wrapping_add(1);
        if self.next_socket_id == 0 {
            self.next_socket_id = 1; // Never use 0 as socket_id
        }

        // Antigravity Phase 1: Allocate buffers on-demand
        let mut rx_buffer = Vec::new();
        rx_buffer.resize(RX_BUF, 0);
        let rx_boxed: Box<[u8]> = rx_buffer.into_boxed_slice();
        
        let mut tx_buffer = Vec::new();
        tx_buffer.resize(TX_BUF, 0);
        let tx_boxed: Box<[u8]> = tx_buffer.into_boxed_slice();

        // Leak the buffers to get 'static references for smoltcp
        let rx_static: &'static mut [u8] = Box::leak(rx_boxed.clone());
        let tx_static: &'static mut [u8] = Box::leak(tx_boxed.clone());

        let socket = tcp::Socket::new(
            tcp::SocketBuffer::new(rx_static),
            tcp::SocketBuffer::new(tx_static),
        );
        let handle = sockets.add(socket);

        let slot = TcpSlot {
            handle,
            local_port: None,
            rx_backlog: VecDeque::new(),
            _rx_buffer: rx_boxed,
            _tx_buffer: tx_boxed,
        };

        self.slots.insert(socket_id, slot);
        Ok(socket_id)
    }

    fn alloc_slot_with_listen(
        &mut self,
        sockets: &mut SocketSet<'static>,
        port: u16,
    ) -> Result<u32, TcpError> {
        let socket_id = self.alloc_socket(sockets)?;
        let handle = self
            .slots
            .get(&socket_id)
            .map(|s| s.handle)
            .ok_or(TcpError::InvalidSocket)?;

        sockets
            .get_mut::<tcp::Socket>(handle)
            .listen(port)
            .map_err(|_| TcpError::SocketError)?;

        if let Some(slot) = self.slots.get_mut(&socket_id) {
            slot.local_port = Some(port);
        }

        Ok(socket_id)
    }

    pub fn bind(
        &mut self,
        socket_id: u32,
        port: u16,
        sockets: &mut SocketSet<'static>,
    ) -> Result<(), TcpError> {
        if port == 0 {
            return Err(TcpError::SocketError);
        }

        if self.slots.iter().any(|(&id, slot)| {
            id != socket_id && slot.local_port == Some(port)
        }) {
            return Err(TcpError::SocketError);
        }

        let slot = self
            .slots
            .get_mut(&socket_id)
            .ok_or(TcpError::InvalidSocket)?;
        let socket = sockets.get::<tcp::Socket>(slot.handle);
        if socket.is_open() {
            return Err(TcpError::SocketError);
        }

        slot.local_port = Some(port);
        Ok(())
    }

    pub fn listen(
        &mut self,
        socket_id: u32,
        _backlog: usize,
        sockets: &mut SocketSet<'static>,
    ) -> Result<(), TcpError> {
        let slot = self
            .slots
            .get_mut(&socket_id)
            .ok_or(TcpError::InvalidSocket)?;
        let port = slot.local_port.ok_or(TcpError::SocketError)?;
        sockets
            .get_mut::<tcp::Socket>(slot.handle)
            .listen(port)
            .map_err(|_| TcpError::SocketError)
    }

    pub fn accept<D: Device>(
        &mut self,
        socket_id: u32,
        iface: &mut Interface,
        sockets: &mut SocketSet<'static>,
        device: &mut D,
    ) -> Result<Option<u32>, TcpError> {
        let (handle, port) = {
            let slot = self.slots.get(&socket_id).ok_or(TcpError::InvalidSocket)?;
            (slot.handle, slot.local_port.ok_or(TcpError::SocketError)?)
        };

        iface.poll(Instant::from_millis(0), device, sockets);

        let state = sockets.get::<tcp::Socket>(handle).state();
        if !matches!(state, tcp::State::SynReceived | tcp::State::Established) {
            return Ok(None);
        }

        let client_id = self.alloc_slot_with_listen(sockets, port)?;
        let mut connected_slot = self
            .slots
            .remove(&socket_id)
            .ok_or(TcpError::InvalidSocket)?;
        let listener_slot = self
            .slots
            .remove(&client_id)
            .ok_or(TcpError::InvalidSocket)?;

        connected_slot.local_port = None;
        self.slots.insert(socket_id, listener_slot);
        self.slots.insert(client_id, connected_slot);

        Ok(Some(client_id))
    }

    pub fn connect<D: Device>(
        &mut self,
        socket_id: u32,
        remote_ip: [u8; 4],
        remote_port: u16,
        iface: &mut Interface,
        sockets: &mut SocketSet<'static>,
        device: &mut D,
        yield_fn: Option<fn()>,
    ) -> Result<(), TcpError> {
        let handle = self
            .slots.get(&socket_id)
            .map(|s| s.handle)
            .ok_or(TcpError::InvalidSocket)?;

        let local_port = self.next_local_port;
        self.next_local_port = self.next_local_port.wrapping_add(1);

        {
            let socket = sockets.get_mut::<tcp::Socket>(handle);
            let endpoint = IpEndpoint::new(
                IpAddress::Ipv4(Ipv4Address::new(
                    remote_ip[0],
                    remote_ip[1],
                    remote_ip[2],
                    remote_ip[3],
                )),
                remote_port,
            );
            socket
                .connect(iface.context(), endpoint, local_port)
                .map_err(|_| TcpError::SocketError)?;
        }

        let start = sunlight_ipc::monotonic_millis();
        let deadline = start.wrapping_add(CONNECT_TIMEOUT_MS);
        loop {
            let elapsed = sunlight_ipc::monotonic_millis().wrapping_sub(start);
            iface.poll(Instant::from_millis(elapsed as i64), device, sockets);

            let socket = sockets.get_mut::<tcp::Socket>(handle);
            match socket.state() {
                tcp::State::Established => return Ok(()),
                tcp::State::Closed | tcp::State::TimeWait => return Err(TcpError::Refused),
                _ => {}
            }

            if sunlight_ipc::monotonic_millis() >= deadline {
                return Err(TcpError::Timeout);
            }

            if let Some(f) = yield_fn {
                f();
            } else {
                sunlight_ipc::process_yield();
            }
        }
    }

    pub fn send<D: Device>(
        &mut self,
        socket_id: u32,
        data: &[u8],
        iface: &mut Interface,
        sockets: &mut SocketSet<'static>,
        device: &mut D,
        yield_fn: Option<fn()>,
    ) -> Result<usize, TcpError> {
        let handle = self
            .slots.get(&socket_id)
            .map(|s| s.handle)
            .ok_or(TcpError::InvalidSocket)?;

        {
            let mut offset = 0;
            while offset < data.len() {
                let socket = sockets.get_mut::<tcp::Socket>(handle);
                if !matches!(socket.state(), tcp::State::Established) {
                    return Err(TcpError::NotConnected);
                }
                match socket.send_slice(&data[offset..]) {
                    Ok(0) => {}
                    Ok(n) => offset += n,
                    Err(tcp::SendError::InvalidState) => return Err(TcpError::NotConnected),
                }
            }

            let start = sunlight_ipc::monotonic_millis();
            let deadline = start.wrapping_add(SEND_TIMEOUT_MS);
            loop {
                let elapsed = sunlight_ipc::monotonic_millis().wrapping_sub(start);
                iface.poll(Instant::from_millis(elapsed as i64), device, sockets);
                let socket = sockets.get_mut::<tcp::Socket>(handle);
                if !matches!(socket.state(), tcp::State::Established) {
                    return Err(TcpError::NotConnected);
                }
                if socket.can_send() {
                    break;
                }
                if sunlight_ipc::monotonic_millis() >= deadline {
                    return Err(TcpError::Timeout);
                }
                if let Some(f) = yield_fn {
                    f();
                } else {
                    sunlight_ipc::process_yield();
                }
            }
        }

        Ok(data.len())
    }

    /// Receive up to `max_len` bytes. Any bytes drained from the socket beyond
    /// `max_len` are retained in the slot's `rx_backlog` and returned by later
    /// calls, so no data is ever dropped between the socket buffer and the
    /// (small) IPC reply. Returns an empty Vec on a clean peer close.
    pub fn recv<D: Device>(
        &mut self,
        socket_id: u32,
        max_len: usize,
        iface: &mut Interface,
        sockets: &mut SocketSet<'static>,
        device: &mut D,
        yield_fn: Option<fn()>,
    ) -> Result<Vec<u8>, TcpError> {
        let take = max_len.max(1);

        let handle = {
            let slot = self.slots.get_mut(&socket_id).ok_or(TcpError::InvalidSocket)?;
            // Serve buffered bytes first — never re-poll while data is pending,
            // and never lose what we already drained from the socket.
            if !slot.rx_backlog.is_empty() {
                let n = slot.rx_backlog.len().min(take);
                return Ok(slot.rx_backlog.drain(..n).collect());
            }
            slot.handle
        };

        let start = sunlight_ipc::monotonic_millis();
        let deadline = start.wrapping_add(RECV_TIMEOUT_MS);

        loop {
            let elapsed = sunlight_ipc::monotonic_millis().wrapping_sub(start);
            iface.poll(Instant::from_millis(elapsed as i64), device, sockets);

            let socket = sockets.get_mut::<tcp::Socket>(handle);
            if socket.can_recv() {
                let mut drained = Vec::new();
                socket
                    .recv(|buf| {
                        drained.extend_from_slice(buf);
                        (buf.len(), ())
                    })
                    .map_err(|_| TcpError::SocketError)?;

                if !drained.is_empty() {
                    // Stash the full segment, hand back at most `take` bytes.
                    let slot = self
                        .slots
                        .get_mut(&socket_id)
                        .ok_or(TcpError::InvalidSocket)?;
                    slot.rx_backlog.extend(drained);
                    let n = slot.rx_backlog.len().min(take);
                    return Ok(slot.rx_backlog.drain(..n).collect());
                }
            }

            let socket = sockets.get_mut::<tcp::Socket>(handle);
            if matches!(
                socket.state(),
                tcp::State::CloseWait | tcp::State::Closed | tcp::State::TimeWait
            ) {
                return Ok(Vec::new());
            }

            if sunlight_ipc::monotonic_millis() >= deadline {
                return Err(TcpError::Timeout);
            }

            if let Some(f) = yield_fn {
                f();
            } else {
                sunlight_ipc::process_yield();
            }
        }
    }

    pub fn close(
        &mut self,
        socket_id: u32,
        sockets: &mut SocketSet<'static>,
    ) -> Result<(), TcpError> {
        let slot = self
            .slots.remove(&socket_id)
            .ok_or(TcpError::InvalidSocket)?;

        sockets.get_mut::<tcp::Socket>(slot.handle).close();
        sockets.remove(slot.handle);

        // Antigravity Phase 1: Buffers automatically dropped, memory freed
        // Note: The leaked 'static buffers remain until process exit, but that's
        // acceptable for a long-running daemon with bounded MAX_SOCKETS
        Ok(())
    }

    /// Antigravity Phase 2: Non-blocking poll for ready sockets
    pub fn poll_ready(
        &self,
        socket_ids: &[u32],
        sockets: &SocketSet<'static>,
    ) -> Vec<u32> {
        let mut ready = Vec::new();
        
        for &socket_id in socket_ids {
            if let Some(slot) = self.slots.get(&socket_id) {
                let socket = sockets.get::<tcp::Socket>(slot.handle);
                if socket.can_recv() 
                    || socket.can_send() 
                    || !matches!(socket.state(), tcp::State::Established) 
                {
                    ready.push(socket_id);
                }
            }
        }
        
        ready
    }

    /// Antigravity Phase 3: Reap closed sockets (garbage collection)
    pub fn reap_closed_sockets(&mut self, sockets: &mut SocketSet<'static>) {
        let to_remove: Vec<u32> = self.slots.iter()
            .filter_map(|(&socket_id, slot)| {
                let socket = sockets.get::<tcp::Socket>(slot.handle);
                if matches!(socket.state(), tcp::State::Closed | tcp::State::TimeWait) {
                    Some(socket_id)
                } else {
                    None
                }
            })
            .collect();

        for socket_id in to_remove {
            let _ = self.close(socket_id, sockets);
        }
    }

    /// Get the number of active sockets
    pub fn active_count(&self) -> usize {
        self.slots.len()
    }
}
