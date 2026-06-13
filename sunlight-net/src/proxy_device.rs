//! Phase 3.4: smoltcp `Device` backed by the kernel's frame-proxy syscalls
//! (`NetTx`/`NetRx`), used by `net_server` since ring-3 cannot map the
//! virtio-net device's I/O ports directly. The kernel keeps the real
//! `VirtioNet` alive (see `kernel::NET_DEVICE`) and copies raw Ethernet
//! frames in and out on net_server's behalf.

use smoltcp::phy::{Device, DeviceCapabilities, Medium, RxToken, TxToken};
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

pub struct ProxyTxToken;

impl TxToken for ProxyTxToken {
    fn consume<R, F>(self, len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let mut scratch = [0u8; 1514];
        let n = len.min(scratch.len());
        let written = f(&mut scratch[..n]);
        sunlight_ipc::net_tx(&scratch[..n]);
        written
    }
}

/// Userspace-side frame proxy device. Each `receive`/`transmit` call costs
/// one syscall round trip to the kernel-owned `VirtioNet`.
pub struct ProxyNetDevice {
    mac: [u8; 6],
}

impl ProxyNetDevice {
    pub fn new(mac: [u8; 6]) -> Self {
        ProxyNetDevice { mac }
    }

    pub fn mac_address(&self) -> [u8; 6] {
        self.mac
    }
}

impl Device for ProxyNetDevice {
    type RxToken<'a> = ProxyRxToken;
    type TxToken<'a> = ProxyTxToken;

    fn receive(&mut self, _timestamp: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        let mut buf = [0u8; 1514];
        let n = sunlight_ipc::net_rx(&mut buf);
        if n > 0 {
            Some((ProxyRxToken { buffer: buf, len: n }, ProxyTxToken))
        } else {
            None
        }
    }

    fn transmit(&mut self, _timestamp: Instant) -> Option<Self::TxToken<'_>> {
        Some(ProxyTxToken)
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        caps.max_transmission_unit = 1514;
        caps.medium = Medium::Ethernet;
        caps
    }
}
