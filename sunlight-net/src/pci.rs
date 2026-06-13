// Re-use PCI scan + port I/O from sunlight-virtio (virtio-blk) to avoid duplication.
// All I/O requires ring-0; callers from kernel context only.

pub use sunlight_virtio::find_virtio_net;
pub use sunlight_virtio::pci::{
    inb, inl, inw, outb, outl, outw, pci_read32,
    VIRTIO_VENDOR_ID, VIRTIO_NET_LEGACY, VIRTIO_NET_MODERN,
};
