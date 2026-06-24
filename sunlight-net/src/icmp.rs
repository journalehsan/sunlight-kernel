//! ICMP echo (ping) over the kernel frame proxy.
//!
//! Real send/receive of ICMPv4 echo requests via smoltcp's ICMP socket,
//! mirroring the `dns::upstream::query_a` shape (generic over `Device`, driven
//! by the same `Interface` + proxy device + `SocketSet`). RTT is measured with
//! `sunlight_ipc::monotonic_millis()` (~10 ms resolution), so reported times
//! reflect the actual network round trip rather than a hardcoded constant.

use smoltcp::iface::{Interface, SocketSet};
use smoltcp::phy::{Device, DeviceCapabilities};
use smoltcp::socket::icmp;
use smoltcp::time::Instant;
use smoltcp::wire::{Icmpv4Packet, Icmpv4Repr, IpAddress, Ipv4Address};

/// Per-attempt wall-clock budget before a packet is counted as lost. Iran's
/// filtered routes can push real RTTs well past 100 ms, so keep this generous.
const PING_TIMEOUT_MS: u64 = 2000;
/// ICMP echo payload size (classic `ping` default minus headers ≈ 56 bytes).
const ECHO_PAYLOAD: usize = 56;
const ICMP_IDENT: u16 = 0x53AA;

#[derive(Debug)]
pub struct PingStats {
    pub target_ip: [u8; 4],
    pub packets_sent: u32,
    pub packets_received: u32,
    pub total_rtt_ms: u64,
    /// Per-sequence RTT in ms; `0` means lost/timed out.
    pub rtts_ms: [u32; 16],
}

#[derive(Debug)]
pub enum IcmpError {
    Timeout,
    SocketError,
}

impl PingStats {
    pub fn new(target: [u8; 4]) -> Self {
        PingStats {
            target_ip: target,
            packets_sent: 0,
            packets_received: 0,
            total_rtt_ms: 0,
            rtts_ms: [0; 16],
        }
    }

    pub fn get_loss_percent(&self) -> u32 {
        if self.packets_sent == 0 {
            0
        } else {
            ((self.packets_sent - self.packets_received) * 100) / self.packets_sent
        }
    }

    pub fn get_avg_rtt(&self) -> u64 {
        if self.packets_received == 0 {
            0
        } else {
            self.total_rtt_ms / (self.packets_received as u64)
        }
    }
}

// Single in-flight echo at a time, so static borrow-able storage is enough
// (smoltcp's no-alloc PacketBuffer needs `&mut [T]`, see dns::upstream).
static mut RX_META: [icmp::PacketMetadata; 4] = [icmp::PacketMetadata::EMPTY; 4];
static mut RX_PAYLOAD: [u8; 256] = [0u8; 256];
static mut TX_META: [icmp::PacketMetadata; 4] = [icmp::PacketMetadata::EMPTY; 4];
static mut TX_PAYLOAD: [u8; 256] = [0u8; 256];

/// Send `count` ICMP echo requests to `target`, measuring real RTT per packet.
pub fn ping<D: Device>(
    target: [u8; 4],
    count: u32,
    iface: &mut Interface,
    sockets: &mut SocketSet,
    device: &mut D,
) -> Result<PingStats, IcmpError> {
    // SAFETY: net_server is single-threaded and runs this to completion
    // (socket removed below) before any reuse — no aliasing across calls.
    let (rx_buffer, tx_buffer) = unsafe {
        (
            icmp::PacketBuffer::new(&mut RX_META[..], &mut RX_PAYLOAD[..]),
            icmp::PacketBuffer::new(&mut TX_META[..], &mut TX_PAYLOAD[..]),
        )
    };
    let mut socket = icmp::Socket::new(rx_buffer, tx_buffer);
    socket
        .bind(icmp::Endpoint::Ident(ICMP_IDENT))
        .map_err(|_| IcmpError::SocketError)?;
    let handle = sockets.add(socket);

    let dst = IpAddress::Ipv4(Ipv4Address::new(target[0], target[1], target[2], target[3]));
    let mut stats = PingStats::new(target);
    let mut tick: i64 = 0;

    for seq in 0..count.min(16) {
        stats.packets_sent += 1;

        // Send one echo request.
        let send_at = sunlight_ipc::monotonic_millis();
        {
            let socket = sockets.get_mut::<icmp::Socket>(handle);
            let repr = Icmpv4Repr::EchoRequest {
                ident: ICMP_IDENT,
                seq_no: seq as u16,
                data: &[0u8; ECHO_PAYLOAD],
            };
            if let Ok(payload) = socket.send(repr.buffer_len(), dst) {
                let mut pkt = Icmpv4Packet::new_unchecked(payload);
                repr.emit(&mut pkt, &device.capabilities().checksum);
            } else {
                continue; // tx buffer full — count as lost
            }
        }

        // Wait for the matching echo reply or time out.
        let deadline = send_at.wrapping_add(PING_TIMEOUT_MS);
        loop {
            iface.poll(Instant::from_millis(tick), device, sockets);
            tick = tick.wrapping_add(1);

            let socket = sockets.get_mut::<icmp::Socket>(handle);
            if socket.can_recv() {
                if let Ok((payload, _src)) = socket.recv() {
                    if let Ok(pkt) = Icmpv4Packet::new_checked(payload) {
                        if let Ok(Icmpv4Repr::EchoReply { ident, seq_no, .. }) =
                            Icmpv4Repr::parse(&pkt, &device.capabilities().checksum)
                        {
                            if ident == ICMP_IDENT && seq_no == seq as u16 {
                                let rtt = sunlight_ipc::monotonic_millis()
                                    .wrapping_sub(send_at)
                                    .max(1); // never report 0 ms for a real reply
                                stats.packets_received += 1;
                                stats.total_rtt_ms += rtt;
                                stats.rtts_ms[seq as usize] = rtt.min(u32::MAX as u64) as u32;
                                break;
                            }
                            // Reply for a different seq/ident — keep waiting.
                        }
                    }
                }
            }

            if sunlight_ipc::monotonic_millis() >= deadline {
                break; // lost: rtts_ms[seq] stays 0
            }
            sunlight_ipc::process_yield();
        }
    }

    sockets.remove(handle);
    Ok(stats)
}
