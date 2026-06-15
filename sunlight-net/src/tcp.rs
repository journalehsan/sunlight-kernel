//! TCP socket pool backed by smoltcp — used by net_server for fetch/wget IPC.

use smoltcp::iface::{Interface, SocketHandle, SocketSet};
use smoltcp::phy::Device;
use smoltcp::socket::tcp;
use smoltcp::time::Instant;
use smoltcp::wire::{IpAddress, IpEndpoint, Ipv4Address};

const RX_BUF: usize = 8192;
const TX_BUF: usize = 4096;
const MAX_SOCKETS: usize = 4;
const POLL_ROUNDS: usize = 2000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TcpError {
    Timeout,
    Refused,
    SocketError,
    NoSlots,
    InvalidSocket,
    NotConnected,
}

pub struct TcpManager {
    slots: [TcpSlot; MAX_SOCKETS],
    next_local_port: u16,
}

struct TcpSlot {
    active: bool,
    handle: Option<SocketHandle>,
    rx_buffer: [u8; RX_BUF],
    tx_buffer: [u8; TX_BUF],
}

impl TcpManager {
    pub const fn new() -> Self {
        Self {
            slots: [TcpSlot::EMPTY; MAX_SOCKETS],
            next_local_port: 49_152,
        }
    }

    pub fn alloc_socket(
        manager: &'static mut Self,
        sockets: &mut SocketSet<'static>,
    ) -> Result<u32, TcpError> {
        for (idx, slot) in manager.slots.iter_mut().enumerate() {
            if !slot.active {
                let socket = tcp::Socket::new(
                    tcp::SocketBuffer::new(&mut slot.rx_buffer[..]),
                    tcp::SocketBuffer::new(&mut slot.tx_buffer[..]),
                );
                let handle = sockets.add(socket);
                slot.active = true;
                slot.handle = Some(handle);
                return Ok((idx + 1) as u32);
            }
        }
        Err(TcpError::NoSlots)
    }

    pub fn connect<D: Device>(
        manager: &'static mut Self,
        socket_id: u32,
        remote_ip: [u8; 4],
        remote_port: u16,
        iface: &mut Interface,
        sockets: &mut SocketSet<'static>,
        device: &mut D,
    ) -> Result<(), TcpError> {
        let handle = manager
            .slot_mut(socket_id)
            .and_then(|s| s.handle)
            .ok_or(TcpError::InvalidSocket)?;

        let local_port = manager.next_local_port;
        manager.next_local_port = manager.next_local_port.wrapping_add(1);

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

        for round in 0..POLL_ROUNDS {
            let timestamp = Instant::from_millis(round as i64 * 5);
            iface.poll(timestamp, device, sockets);
            let socket = sockets.get_mut::<tcp::Socket>(handle);
            match socket.state() {
                tcp::State::Established => return Ok(()),
                tcp::State::Closed | tcp::State::TimeWait => return Err(TcpError::Refused),
                _ => {}
            }
        }

        Err(TcpError::Timeout)
    }

    pub fn send<D: Device>(
        manager: &'static mut Self,
        socket_id: u32,
        data: &[u8],
        iface: &mut Interface,
        sockets: &mut SocketSet<'static>,
        device: &mut D,
    ) -> Result<usize, TcpError> {
        let handle = manager
            .slot_mut(socket_id)
            .and_then(|s| s.handle)
            .ok_or(TcpError::InvalidSocket)?;

        let mut offset = 0usize;
        while offset < data.len() {
            {
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

            for round in 0..32 {
                let timestamp = Instant::from_millis(round as i64);
                iface.poll(timestamp, device, sockets);
                let socket = sockets.get_mut::<tcp::Socket>(handle);
                if socket.can_send() {
                    break;
                }
            }
        }

        Ok(data.len())
    }

    pub fn recv<D: Device>(
        manager: &'static mut Self,
        socket_id: u32,
        max_len: usize,
        iface: &mut Interface,
        sockets: &mut SocketSet<'static>,
        device: &mut D,
    ) -> Result<alloc::vec::Vec<u8>, TcpError> {
        let handle = manager
            .slot_mut(socket_id)
            .and_then(|s| s.handle)
            .ok_or(TcpError::InvalidSocket)?;

        for round in 0..POLL_ROUNDS {
            let timestamp = Instant::from_millis(round as i64 * 2);
            iface.poll(timestamp, device, sockets);

            let mut out = alloc::vec::Vec::new();
            out.reserve(max_len.min(RX_BUF));
            {
                let socket = sockets.get_mut::<tcp::Socket>(handle);
                while out.len() < max_len {
                    let mut chunk = [0u8; 512];
                    match socket.recv_slice(&mut chunk) {
                        Ok(0) => break,
                        Ok(n) => out.extend_from_slice(&chunk[..n]),
                        Err(tcp::RecvError::Finished) => break,
                        Err(tcp::RecvError::InvalidState) => return Err(TcpError::NotConnected),
                    }
                }
            }
            if !out.is_empty() {
                return Ok(out);
            }

            let socket = sockets.get_mut::<tcp::Socket>(handle);
            if matches!(
                socket.state(),
                tcp::State::CloseWait | tcp::State::Closed | tcp::State::TimeWait
            ) {
                return Ok(alloc::vec::Vec::new());
            }
        }

        Ok(alloc::vec::Vec::new())
    }

    pub fn close(
        manager: &'static mut Self,
        socket_id: u32,
        sockets: &mut SocketSet<'static>,
    ) -> Result<(), TcpError> {
        let slot = manager
            .slot_mut(socket_id)
            .ok_or(TcpError::InvalidSocket)?;
        if let Some(handle) = slot.handle.take() {
            sockets.remove(handle);
        }
        slot.active = false;
        Ok(())
    }

    fn slot_mut(&mut self, socket_id: u32) -> Option<&mut TcpSlot> {
        if socket_id == 0 || socket_id as usize > MAX_SOCKETS {
            return None;
        }
        let slot = &mut self.slots[(socket_id - 1) as usize];
        if slot.active {
            Some(slot)
        } else {
            None
        }
    }
}

impl TcpSlot {
    const EMPTY: Self = Self {
        active: false,
        handle: None,
        rx_buffer: [0; RX_BUF],
        tx_buffer: [0; TX_BUF],
    };
}