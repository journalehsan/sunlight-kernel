use crate::virtio_net::VirtioNet;
use smoltcp::phy::{Device, DeviceCapabilities, Medium, TxToken, RxToken};
use smoltcp::time::Instant;
use core::cell::RefCell;

/// RX token carrying a buffer that was filled by VirtioNet::recv (or a scratch for simulation).
pub struct SunlightRxToken<'a> {
    buffer: [u8; 1514],
    len: usize,
    _marker: core::marker::PhantomData<&'a ()>,
}

impl<'a> RxToken for SunlightRxToken<'a> {
    fn consume<R, F>(self, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let mut buf = self.buffer;
        f(&mut buf[..self.len])
    }
}

/// TX token that will push the filled bytes into VirtioNet::send on consume.
pub struct SunlightTxToken<'a> {
    virtio: &'a RefCell<VirtioNet>,
    _marker: core::marker::PhantomData<&'a ()>,
}

impl<'a> TxToken for SunlightTxToken<'a> {
    fn consume<R, F>(self, len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let mut scratch = [0u8; 1514];
        let n = len.min(scratch.len());
        let written = f(&mut scratch[..n]);
        // SAFETY: The underlying VirtioNet was initialized with valid rings and we are
        // the sole driver; send performs its own bounded poll on the used ring.
        // Alternative (safer but slower): always copy to a dedicated per-TX buffer.
        let _ = unsafe { self.virtio.borrow_mut().send(&scratch[..n]) };
        written
    }
}

/// Wrapper for VirtioNet implementing smoltcp Device trait.
/// 
/// This bridges the kernel-owned VirtioNet (ring-0 queues + port I/O) into smoltcp's
/// poll model used by net_server (or kernel DHCP at boot).
///
/// For net_server userspace: a real VirtioNet cannot live here (no port I/O). In that
/// case callers should use a simulation or an IPC-driven "remote device" (future).
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
        // Poll the real virtio RX queue. If a frame arrived, hand a token containing it.
        // SAFETY: VirtioNet::recv is unsafe only because it derefs rings initialized at
        // device creation time; those rings are still valid here.
        let mut tmp = [0u8; 1514];
        let n = unsafe { self.virtio.borrow_mut().recv(&mut tmp) };
        if n > 0 {
            let mut buf = [0u8; 1514];
            buf[..n].copy_from_slice(&tmp[..n]);
            let rx = SunlightRxToken { buffer: buf, len: n, _marker: core::marker::PhantomData };
            let tx = SunlightTxToken { virtio: &self.virtio, _marker: core::marker::PhantomData };
            Some((rx, tx))
        } else {
            None
        }
    }

    fn transmit(&mut self, _timestamp: Instant) -> Option<Self::TxToken<'_>> {
        // Always allow a TX token; the consume will attempt the actual send.
        // We return a token unconditionally because the virtqueue has space in MVI model.
        Some(SunlightTxToken { virtio: &self.virtio, _marker: core::marker::PhantomData })
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        caps.max_transmission_unit = 1514;
        caps.medium = Medium::Ethernet;
        caps
    }
}
