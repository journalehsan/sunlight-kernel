//! Phase 3.1: upstream DNS-over-UDP using the hand-written RFC 1035 wire
//! format from [`super::wire`] and smoltcp's UDP socket for transport.
//!
//! This mirrors the existing `dhcp::acquire_lease` / `icmp::ping` shape
//! (taking `&mut Interface`, `&mut SocketSet`, `&mut SunlightNetDevice`) so
//! it plugs into the same poll loop once net_server has a real device.

use super::wire::{BytePacketBuffer, DnsPacket, QueryType};
use super::DnsError;
use core::sync::atomic::{AtomicU16, Ordering};
use smoltcp::iface::{Interface, SocketHandle, SocketSet};
use smoltcp::phy::Device;
use smoltcp::socket::udp;
use smoltcp::time::Instant;
use smoltcp::wire::{IpAddress, IpEndpoint, IpListenEndpoint, Ipv4Address};

const DNS_PORT: u16 = 53;
const LOCAL_PORT: u16 = 53000;
static NEXT_QUERY_ID: AtomicU16 = AtomicU16::new(0xD05);
/// Wall-clock budget (seconds) for a single attempt before giving up and
/// (optionally) retrying once.
///
/// This MUST be measured against real time, not a fixed poll-iteration count:
/// `process_yield()` returns as soon as the scheduler re-runs net_server (often
/// immediately, since it is usually the only runnable process), so a fixed
/// iteration budget burns through in microseconds — long before a reply can
/// traverse the QEMU slirp NAT and the host's recursive resolver. Names the
/// host already has cached (e.g. google.com) used to "work" only because they
/// happened to reply within that microsecond window; uncached names always
/// raced out to a spurious `Timeout`.
const DNS_TIMEOUT_SECS: u64 = 3;

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
                .bind(IpListenEndpoint {
                    addr: None,
                    port: LOCAL_PORT + attempt,
                })
                .map_err(|_| DnsError::QueryFailed)?;
        }
    }

    // Build the query packet with our hand-written RFC 1035 serializer.
    // A late response from a previous hostname or retry must not satisfy this
    // query. The echoed question and source are checked as well as this ID.
    let query_id = NEXT_QUERY_ID.fetch_add(1, Ordering::Relaxed);
    let mut packet = DnsPacket::query(query_id, hostname, QueryType::A);
    let mut req_buf = BytePacketBuffer::new();
    packet
        .write(&mut req_buf)
        .map_err(|_| DnsError::QueryFailed)?;

    {
        let socket = sockets.get_mut::<udp::Socket>(handle);
        socket
            .send_slice(&req_buf.buf[..req_buf.pos()], remote)
            .map_err(|_| DnsError::QueryFailed)?;
    }

    // Relative network timeouts are monotonic: an RTC correction must not make
    // an in-flight DNS query expire early or wait indefinitely.
    let timeout_ms = DNS_TIMEOUT_SECS
        .checked_mul(1_000)
        .ok_or(DnsError::Timeout)?;
    let deadline = sunlight_ipc::monotonic_millis()
        .checked_add(timeout_ms)
        .ok_or(DnsError::Timeout)?;
    loop {
        let now = Instant::from_millis(sunlight_ipc::monotonic_millis() as i64);
        iface.poll(now, device, sockets);

        let socket = sockets.get_mut::<udp::Socket>(handle);
        if socket.can_recv() {
            let mut res_buf = BytePacketBuffer::new();
            // A failed recv or a malformed/unparseable packet is not fatal: keep
            // waiting for a well-formed reply until the deadline (a single bad
            // datagram must not abort the query before the fallback logic, which
            // keys on `Timeout`, ever gets a chance to run).
            if let Ok((_n, meta)) = socket.recv_slice(&mut res_buf.buf) {
                if let Ok(response) = DnsPacket::from_buffer(&mut res_buf) {
                    if matches_response(&response, meta.endpoint, remote, query_id, hostname) {
                        return match response.first_a() {
                            Some((addr, ttl)) => Ok((addr, ttl)),
                            None => Err(DnsError::NotFound),
                        };
                    }
                    // Stale/spoofed reply for a different query — keep waiting.
                }
            }
        }

        if sunlight_ipc::monotonic_millis() >= deadline {
            return Err(DnsError::Timeout);
        }

        // Each poll is a syscall round trip to the kernel's frame proxy;
        // yield so other processes get scheduled while we wait for a reply.
        sunlight_ipc::process_yield();
    }
}

fn matches_response(
    response: &DnsPacket,
    source: IpEndpoint,
    server: IpEndpoint,
    query_id: u16,
    hostname: &str,
) -> bool {
    source == server
        && response.header.id == query_id
        && response.header.response
        && response.header.opcode == 0
        && !response.header.truncated_message
        && response.questions.len() == 1
        && response.questions[0].qtype == QueryType::A
        && response.questions[0]
            .name
            .trim_end_matches('.')
            .eq_ignore_ascii_case(hostname.trim_end_matches('.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server() -> IpEndpoint {
        IpEndpoint::new(IpAddress::Ipv4(Ipv4Address::new(10, 0, 2, 3)), DNS_PORT)
    }

    #[test]
    fn late_ntp_dns_reply_cannot_resolve_https_hostname() {
        // Previously every first attempt used 0xD05, so a delayed NTP reply
        // could poison the HTTPS hostname's cache entry with an NTP server IP.
        let mut response = DnsPacket::query(0xD05, "3.pool.ntp.org", QueryType::A);
        response.header.response = true;
        assert!(!matches_response(
            &response,
            server(),
            server(),
            0xD05,
            "archlinux.org"
        ));
        assert!(!matches_response(
            &response,
            server(),
            server(),
            0xD06,
            "3.pool.ntp.org"
        ));
    }

    #[test]
    fn response_requires_expected_source_question_and_flags() {
        let mut response = DnsPacket::query(42, "ARCHLINUX.ORG", QueryType::A);
        response.header.response = true;
        assert!(matches_response(
            &response,
            server(),
            server(),
            42,
            "archlinux.org."
        ));
        let wrong_source = IpEndpoint::new(server().addr, 123);
        assert!(!matches_response(
            &response,
            wrong_source,
            server(),
            42,
            "archlinux.org"
        ));
        response.questions[0].qtype = QueryType::AAAA;
        assert!(!matches_response(
            &response,
            server(),
            server(),
            42,
            "archlinux.org"
        ));
        response.questions[0].qtype = QueryType::A;
        response.header.response = false;
        assert!(!matches_response(
            &response,
            server(),
            server(),
            42,
            "archlinux.org"
        ));
        response.header.response = true;
        response.header.truncated_message = true;
        assert!(!matches_response(
            &response,
            server(),
            server(),
            42,
            "archlinux.org"
        ));
    }
}
