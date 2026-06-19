//! TCP socket pool backed by smoltcp — used by net_server for fetch/wget IPC.

use smoltcp::iface::{Interface, SocketHandle, SocketSet};
use smoltcp::phy::Device;
use smoltcp::socket::tcp;
use smoltcp::time::Instant;
use smoltcp::wire::{IpAddress, IpEndpoint, Ipv4Address};

const RX_BUF: usize = 8192;
const TX_BUF: usize = 4096;
const MAX_SOCKETS: usize = 4;
/// Real wall-clock budget for the TCP three-way handshake. QEMU's slirp NAT
/// opens the actual connection to the external host on our behalf, so this must
/// cover a full real RTT to the remote (filtered/overseas routes can be slow).
/// Set to 20s: speedtest-class servers can take 8–10 s to respond to the first
/// SYN (server-side SYN cookies + rate limiting), and our smoltcp initial RTO
/// schedule exhausts 7 retransmits in ~6.7 s, leaving the 8th SYN and ACK
/// exchange to complete by ~9 s on a typical overseas host.
const CONNECT_TIMEOUT_MS: u64 = 20_000;
/// Real wall-clock budget for flushing one `send` into the TX window.
const SEND_TIMEOUT_MS: u64 = 8000;
/// Real wall-clock budget to wait for inbound data on one `recv`. Bounds the
/// idle gap between segments; an actual peer close is detected immediately and
/// returns early regardless of this value.
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
        yield_fn: Option<fn()>,
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

        // Drive the handshake on real wall-clock time, exactly like the ICMP
        // ping and DNS wait loops (icmp::ping / dns::upstream::query_a): feed
        // smoltcp real elapsed milliseconds so its SYN retransmit timers track
        // reality, bound the wait by a real monotonic deadline, and yield on
        // *every* poll so the kernel frame proxy can deliver the SYN-ACK.
        //
        // The previous loop spun a fixed POLL_ROUNDS of synthetic 5 ms steps and
        // yielded only every 10th round; with nothing forcing real time to pass
        // it exhausted all 2000 polls in a few ms — far less than the real RTT
        // slirp needs to complete the external handshake — so every connect to a
        // real host timed out even though ping/DNS (which wait on real time)
        // worked.
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
            }
        }
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

            // Flush on real time, yielding every poll so the proxy device can
            // hand the queued segments to the kernel NIC and pick up ACKs —
            // same discipline as the connect/recv loops below.
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
                sunlight_ipc::process_yield();
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

        // Wait for inbound data on real time, yielding every poll so the proxy
        // device can pull freshly-arrived frames from the kernel NIC. The old
        // fixed-round spin without any yield raced past the data window and
        // returned empty mid-transfer, which the HTTP client misread as EOF
        // (truncated body / "empty response").
        // Allocate once outside the loop so the bump allocator is not drained
        // by a Vec-per-iteration leak on every poll that finds no data.
        let mut out = alloc::vec::Vec::with_capacity(max_len.min(RX_BUF));
        let start = sunlight_ipc::monotonic_millis();
        let deadline = start.wrapping_add(RECV_TIMEOUT_MS);
        loop {
            out.clear();
            let elapsed = sunlight_ipc::monotonic_millis().wrapping_sub(start);
            iface.poll(Instant::from_millis(elapsed as i64), device, sockets);

            {
                let socket = sockets.get_mut::<tcp::Socket>(handle);
                while out.len() < max_len {
                    let space = max_len - out.len();
                    let mut chunk = [0u8; 512];
                    let limit = space.min(chunk.len());
                    match socket.recv_slice(&mut chunk[..limit]) {
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
                // Peer closed and the RX buffer is drained — genuine EOF.
                return Ok(alloc::vec::Vec::new());
            }

            if sunlight_ipc::monotonic_millis() >= deadline {
                // Return an explicit Timeout so callers distinguish a stalled
                // connection from a clean peer close (which returns Ok(empty)).
                return Err(TcpError::Timeout);
            }

            sunlight_ipc::process_yield();
        }
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
            // Initiate graceful FIN before removing the socket. smoltcp will
            // queue the FIN segment; the next iface.poll() (in the caller's
            // loop) will transmit it. Without this call, smoltcp drops the
            // socket immediately and the peer receives RST instead of FIN.
            sockets.get_mut::<tcp::Socket>(handle).close();
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