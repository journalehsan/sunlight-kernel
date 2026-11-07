//! Phase 3.4: smoltcp `Device` backed by the kernel's frame-proxy syscalls
//! (`NetTx`/`NetRx`), used by `net_server` since ring-3 cannot map the
//! NIC device's registers directly. The kernel keeps the selected Layer-2
//! backend alive (see `kernel::NET_DEVICE`) and copies raw Ethernet
//! frames in and out on net_server's behalf.

use alloc::collections::VecDeque;

use smoltcp::phy::{ChecksumCapabilities, Device, DeviceCapabilities, Medium, RxToken, TxToken};
use smoltcp::time::Instant;

pub struct ProxyRxToken {
    buffer: [u8; 1514],
    len: usize,
}

impl RxToken for ProxyRxToken {
    fn consume<R, F>(self, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let mut buf = self.buffer;
        f(&mut buf[..self.len])
    }
}

pub struct ProxyTxToken<'a> {
    local: &'a mut VecDeque<ProxyRxToken>,
}

impl TxToken for ProxyTxToken<'_> {
    fn consume<R, F>(self, len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let mut scratch = [0u8; 1514];
        let n = len.min(scratch.len());
        let written = f(&mut scratch[..n]);
        if loopback_frame(&scratch[..n]) {
            // Loopback never reaches the physical NIC, including ARP. A full
            // queue drops packets; TCP retransmission supplies backpressure.
            if self.local.len() < 64 {
                self.local.push_back(ProxyRxToken {
                    buffer: scratch,
                    len: n,
                });
            }
        } else {
            sunlight_ipc::net_tx(&scratch[..n]);
        }
        written
    }
}

/// Userspace-side frame proxy device. Each `receive`/`transmit` call costs
/// one syscall round trip to the kernel-owned Layer-2 backend.
pub struct ProxyNetDevice {
    mac: [u8; 6],
    local: VecDeque<ProxyRxToken>,
}

impl ProxyNetDevice {
    pub fn new(mac: [u8; 6]) -> Self {
        ProxyNetDevice {
            mac,
            local: VecDeque::new(),
        }
    }

    pub fn mac_address(&self) -> [u8; 6] {
        self.mac
    }
}

impl Device for ProxyNetDevice {
    type RxToken<'a> = ProxyRxToken;
    type TxToken<'a> = ProxyTxToken<'a>;

    fn receive(&mut self, _timestamp: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        if let Some(packet) = self.local.pop_front() {
            return Some((
                packet,
                ProxyTxToken {
                    local: &mut self.local,
                },
            ));
        }
        let mut buf = [0u8; 1514];
        let n = sunlight_ipc::net_rx(&mut buf);
        // Never accept a physical packet claiming loopback provenance.
        if n > 0 && !loopback_frame(&buf[..n]) {
            Some((
                ProxyRxToken {
                    buffer: buf,
                    len: n,
                },
                ProxyTxToken {
                    local: &mut self.local,
                },
            ))
        } else {
            None
        }
    }

    fn transmit(&mut self, _timestamp: Instant) -> Option<Self::TxToken<'_>> {
        Some(ProxyTxToken {
            local: &mut self.local,
        })
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        caps.max_transmission_unit = 1514;
        caps.medium = Medium::Ethernet;
        // Force software checksums. We do NOT offload to the kernel virtio device,
        // so smoltcp must compute IP/TCP/UDP/ICMP checksums itself on tx and verify
        // them on rx. That is exactly `Checksum::Both`, which is the *default*.
        //
        // NB: do NOT use `ChecksumCapabilities::ignored()` here — `ignored()` sets
        // every protocol to `Checksum::None`, telling smoltcp to skip checksums
        // entirely (it assumes hardware offload). With no offload, outbound frames
        // then ship with a zero/invalid IPv4 header checksum and QEMU slirp drops
        // every IP packet (ARP still works — it has no checksum), which silently
        // breaks DNS, ping and TCP/TLS.
        caps.checksum = ChecksumCapabilities::default();
        caps
    }
}

/// Classify both source and destination, so neither outbound nor inbound
/// loopback traffic can cross the NIC boundary. Ethernet IPv4 and Ethernet/IPv4 ARP.
fn loopback_frame(frame: &[u8]) -> bool {
    if frame.len() >= 34 && frame[12..14] == [8, 0] {
        frame[26] == 127 || frame[30] == 127
    } else if frame.len() >= 42 && frame[12..14] == [8, 6] {
        frame[28] == 127 || frame[38] == 127
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn loopback_packets_are_queued_without_a_nic() {
        let mut device = ProxyNetDevice::new([2, 0, 0, 0, 0, 1]);
        let mut packet = [0u8; 42];
        packet[12..14].copy_from_slice(&[8, 0]);
        packet[30..34].copy_from_slice(&[127, 0, 0, 1]);
        device
            .transmit(Instant::from_millis(0))
            .unwrap()
            .consume(packet.len(), |out| out.copy_from_slice(&packet));
        let (rx, _) = device.receive(Instant::from_millis(0)).unwrap();
        rx.consume(|bytes| assert_eq!(bytes, packet));
        packet[30] = 10;
        assert!(!loopback_frame(&packet));
        packet[26] = 127;
        assert!(loopback_frame(&packet));
        packet[12..14].copy_from_slice(&[8, 6]);
        packet[28] = 127;
        assert!(loopback_frame(&packet));
    }
}
