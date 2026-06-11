use crate::pci;

// Virtio feature bits for networking
pub const VIRTIO_NET_F_MAC: u32 = 1 << 5;
pub const VIRTIO_NET_F_STATUS: u32 = 1 << 16;
pub const VIRTIO_NET_F_CTRL_VQ: u32 = 1 << 17;
pub const VIRTIO_NET_F_MRG_RXBUF: u32 = 1 << 15;

// Virtio device registers (I/O BAR)
const VIRTIO_REG_DEVICE_FEATURES: u16 = 0x00;
const VIRTIO_REG_DRIVER_FEATURES: u16 = 0x04;
const VIRTIO_REG_QUEUE_ADDRESS: u16 = 0x08;
const VIRTIO_REG_QUEUE_SIZE: u16 = 0x0C;
const VIRTIO_REG_QUEUE_SELECT: u16 = 0x0E;
const VIRTIO_REG_QUEUE_NOTIFY: u16 = 0x10;
const VIRTIO_REG_STATUS: u16 = 0x12;
const VIRTIO_REG_CONFIG: u16 = 0x14;

// Virtio status values
const STATUS_RESET: u8 = 0x00;
const STATUS_ACKNOWLEDGE: u8 = 0x01;
const STATUS_DRIVER: u8 = 0x02;
const STATUS_DRIVER_OK: u8 = 0x04;
const STATUS_FEATURES_OK: u8 = 0x08;

/// Virtio-net packet header (before Ethernet frame)
#[repr(C)]
pub struct VirtioNetHeader {
    pub flags: u8,
    pub gso_type: u8,
    pub hdr_len: u16,
    pub gso_size: u16,
    pub csum_start: u16,
    pub csum_offset: u16,
}

/// Virtio-net device
pub struct VirtioNet {
    io_base: u16,
    mac: [u8; 6],
    rx_queue: Option<u32>,
    tx_queue: Option<u32>,
    bus: u8,
    slot: u8,
}

#[derive(Debug)]
pub enum NetError {
    NotFound,
    InitFailed,
}

impl VirtioNet {
    /// Initialize from PCI — reuse sunlight-virtio PCI scan
    pub unsafe fn init() -> Result<Self, NetError> {
        let (bus, slot, _func, io_base) = pci::find_virtio_net()
            .ok_or(NetError::NotFound)?;

        let mut dev = VirtioNet {
            io_base,
            mac: [0u8; 6],
            rx_queue: None,
            tx_queue: None,
            bus,
            slot,
        };

        // Reset device
        pci::outb(io_base + VIRTIO_REG_STATUS, STATUS_RESET);

        // Acknowledge device
        pci::outb(io_base + VIRTIO_REG_STATUS, STATUS_ACKNOWLEDGE);

        // Tell device we're a driver
        let status = pci::inb(io_base + VIRTIO_REG_STATUS);
        pci::outb(io_base + VIRTIO_REG_STATUS, status | STATUS_DRIVER);

        // Read and negotiate features
        let features = pci::inl(io_base + VIRTIO_REG_DEVICE_FEATURES);
        let supported_features = VIRTIO_NET_F_MAC | VIRTIO_NET_F_STATUS;
        let driver_features = features & supported_features;

        pci::outl(io_base + VIRTIO_REG_DRIVER_FEATURES, driver_features);

        // Features OK
        let status = pci::inb(io_base + VIRTIO_REG_STATUS);
        pci::outb(io_base + VIRTIO_REG_STATUS, status | STATUS_FEATURES_OK);

        // Read MAC address from device config (offset 0x14)
        for i in 0..6 {
            dev.mac[i] = pci::inb(io_base + VIRTIO_REG_CONFIG + (i as u16));
        }

        // Initialize RX and TX queues (queue 0 = RX, queue 1 = TX)
        // For now, just mark them as "initialized" — actual queue setup deferred
        dev.rx_queue = Some(0);
        dev.tx_queue = Some(1);

        // Driver OK
        let status = pci::inb(io_base + VIRTIO_REG_STATUS);
        pci::outb(io_base + VIRTIO_REG_STATUS, status | STATUS_DRIVER_OK);

        Ok(dev)
    }

    /// Get MAC address
    pub fn mac(&self) -> [u8; 6] {
        self.mac
    }

    /// Get PCI bus identifier
    pub fn pci_location(&self) -> (u8, u8) {
        (self.bus, self.slot)
    }
}
