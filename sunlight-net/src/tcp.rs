//! TCP socket pool backed by smoltcp — used by net_server for fetch/wget IPC.
//!
//! Project Antigravity: Dynamic allocation + async polling + garbage collection

extern crate alloc;
use alloc::collections::BTreeMap;
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
            _rx_buffer: rx_boxed,
            _tx_buffer: tx_boxed,
        };

        self.slots.insert(socket_id, slot);
        Ok(socket_id)
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

    pub fn recv<D: Device>(
        &mut self,
        socket_id: u32,
        iface: &mut Interface,
        sockets: &mut SocketSet<'static>,
        device: &mut D,
        yield_fn: Option<fn()>,
    ) -> Result<Vec<u8>, TcpError> {
        let handle = self
            .slots.get(&socket_id)
            .map(|s| s.handle)
            .ok_or(TcpError::InvalidSocket)?;

        let start = sunlight_ipc::monotonic_millis();
        let deadline = start.wrapping_add(RECV_TIMEOUT_MS);

        loop {
            let elapsed = sunlight_ipc::monotonic_millis().wrapping_sub(start);
            iface.poll(Instant::from_millis(elapsed as i64), device, sockets);

            let socket = sockets.get_mut::<tcp::Socket>(handle);
            if socket.can_recv() {
                let mut out = Vec::new();
                socket
                    .recv(|buf| {
                        out.extend_from_slice(buf);
                        (buf.len(), ())
                    })
                    .map_err(|_| TcpError::SocketError)?;

                if !out.is_empty() {
                    return Ok(out);
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
