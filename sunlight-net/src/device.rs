use crate::virtio_net::VirtioNet;
use smoltcp::phy::{Device, DeviceCapabilities, Medium, TxToken, RxToken};
use smoltcp::time::Instant;
use core::cell::RefCell;

/// RX token for smoltcp device (stub for Phase 5.0)
pub struct SunlightRxToken<'a> {
    _buffer: &'a [u8],
}

impl<'a> RxToken for SunlightRxToken<'a> {
    fn consume<R, F>(self, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        // Phase 5.1: implement actual RX handling
        let mut buf = [0u8; 1514];
        f(&mut buf)
    }
}

/// TX token for smoltcp device (stub for Phase 5.0)
pub struct SunlightTxToken<'a> {
    _buffer: &'a mut [u8],
}

impl<'a> TxToken for SunlightTxToken<'a> {
    fn consume<R, F>(self, _len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        // Phase 5.1: implement actual TX handling
        let mut buf = [0u8; 1514];
        f(&mut buf)
    }
}

/// Wrapper for VirtioNet implementing smoltcp Device trait
pub struct SunlightNetDevice {
    virtio: RefCell<VirtioNet>,
}

impl SunlightNetDevice {
    pub fn new(virtio: VirtioNet) -> Self {
        SunlightNetDevice {
            virtio: RefCell::new(virtio),
        }
    }

    pub fn mac_address(&self) -> [u8; 6] {
        self.virtio.borrow().mac()
    }
}

impl Device for SunlightNetDevice {
    type RxToken<'a> = SunlightRxToken<'a>;
    type TxToken<'a> = SunlightTxToken<'a>;

    fn receive(&mut self, _timestamp: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        // Poll virtio RX queue for incoming packets
        // For Phase 5.0, just return None (no packets yet)
        None
    }

    fn transmit(&mut self, _timestamp: Instant) -> Option<Self::TxToken<'_>> {
        // Return a TX token if we can queue a packet
        // For Phase 5.0, just return None (TX not yet implemented)
        None
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        caps.max_transmission_unit = 1514;
        caps.medium = Medium::Ethernet;
        caps
    }
}
