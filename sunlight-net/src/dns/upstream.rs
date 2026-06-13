//! Phase 3.1: upstream DNS-over-UDP using the hand-written RFC 1035 wire
//! format from [`super::wire`] and smoltcp's UDP socket for transport.
//!
//! This mirrors the existing `dhcp::acquire_lease` / `icmp::ping` shape
//! (taking `&mut Interface`, `&mut SocketSet`, `&mut SunlightNetDevice`) so
//! it plugs into the same poll loop once net_server has a real device.

use super::wire::{BytePacketBuffer, DnsPacket, QueryType};
use super::DnsError;
use smoltcp::iface::{Interface, SocketHandle, SocketSet};
use smoltcp::phy::Device;
use smoltcp::socket::udp;
use smoltcp::time::Instant;
use smoltcp::wire::{IpAddress, IpEndpoint, IpListenEndpoint, Ipv4Address};

const DNS_PORT: u16 = 53;
const LOCAL_PORT: u16 = 53000;
/// Poll budget in "ticks" before giving up and (optionally) retrying once.
const POLL_TIMEOUT_TICKS: u32 = 2000;

/// Resolve `hostname` to an IPv4 address via the upstream `server` (e.g.
/// `[8, 8, 8, 8]`), sending a single A query over UDP/53.
///
/// Performs one retry on timeout, as required by Phase 3.1. Returns the
/// resolved address and the record's TTL (for cache insertion).
// smoltcp's `udp::PacketBuffer` borrows its storage (`ManagedSlice` without the
// `alloc` feature only accepts `&mut [T]`, not `Vec<T>`). A single in-flight
// query at a time is all net_server ever needs, so we keep the storage as
// process-static arrays and hand out `'static` slices.
static mut RX_META: [udp::PacketMetadata; 4] = [udp::PacketMetadata::EMPTY; 4];
static mut RX_PAYLOAD: [u8; 512] = [0u8; 512];
static mut TX_META: [udp::PacketMetadata; 4] = [udp::PacketMetadata::EMPTY; 4];
static mut TX_PAYLOAD: [u8; 512] = [0u8; 512];

pub fn query_a<D: Device>(
    hostname: &str,
    server: [u8; 4],
    iface: &mut Interface,
    sockets: &mut SocketSet,
    device: &mut D,
) -> Result<([u8; 4], u32), DnsError> {
    // SAFETY: net_server is single-threaded and `query_a` runs to completion
    // (including the socket removal below) before any other call can reuse
    // these buffers — no aliasing across calls.
    let (rx_buffer, tx_buffer) = unsafe {
        (
            udp::PacketBuffer::new(&mut RX_META[..], &mut RX_PAYLOAD[..]),
            udp::PacketBuffer::new(&mut TX_META[..], &mut TX_PAYLOAD[..]),
        )
    };
    let udp_socket = udp::Socket::new(rx_buffer, tx_buffer);
    let handle = sockets.add(udp_socket);

    let result = run_query(hostname, server, iface, sockets, device, handle, 0)
        .or_else(|_| run_query(hostname, server, iface, sockets, device, handle, 1));

    sockets.remove(handle);
    result
}

fn run_query<D: Device>(
    hostname: &str,
    server: [u8; 4],
    iface: &mut Interface,
    sockets: &mut SocketSet,
    device: &mut D,
    handle: SocketHandle,
    attempt: u16,
) -> Result<([u8; 4], u32), DnsError> {
    let server_addr = IpAddress::Ipv4(Ipv4Address::new(server[0], server[1], server[2], server[3]));
    let remote = IpEndpoint::new(server_addr, DNS_PORT);

    {
        let socket = sockets.get_mut::<udp::Socket>(handle);
        if !socket.is_open() {
            socket
                .bind(IpListenEndpoint { addr: None, port: LOCAL_PORT + attempt })
                .map_err(|_| DnsError::QueryFailed)?;
        }
    }

    // Build the query packet with our hand-written RFC 1035 serializer.
    let query_id = 0xD05 ^ attempt;
    let mut packet = DnsPacket::query(query_id, hostname, QueryType::A);
    let mut req_buf = BytePacketBuffer::new();
    packet.write(&mut req_buf).map_err(|_| DnsError::QueryFailed)?;

    {
        let socket = sockets.get_mut::<udp::Socket>(handle);
        socket
            .send_slice(&req_buf.buf[..req_buf.pos()], remote)
            .map_err(|_| DnsError::QueryFailed)?;
    }

    for tick in 0..POLL_TIMEOUT_TICKS {
        let now = Instant::from_millis(tick as i64);
        iface.poll(now, device, sockets);

        let socket = sockets.get_mut::<udp::Socket>(handle);
        if socket.can_recv() {
            let mut res_buf = BytePacketBuffer::new();
            let (n, _meta) = socket.recv_slice(&mut res_buf.buf).map_err(|_| DnsError::QueryFailed)?;
            let _ = n;

            let response = DnsPacket::from_buffer(&mut res_buf).map_err(|_| DnsError::QueryFailed)?;
            if response.header.id != query_id {
                // Stale/spoofed reply for a different query — keep waiting.
                continue;
            }
            return match response.first_a() {
                Some((addr, ttl)) => Ok((addr, ttl)),
                None => Err(DnsError::NotFound),
            };
        }

        // Each poll is a syscall round trip to the kernel's frame proxy;
        // yield so other processes get scheduled while we wait for a reply.
        sunlight_ipc::process_yield();
    }

    Err(DnsError::Timeout)
}
