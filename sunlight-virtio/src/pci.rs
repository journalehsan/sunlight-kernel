use core::arch::asm;

const CONFIG_ADDRESS: u16 = 0xCF8;
const CONFIG_DATA: u16 = 0xCFC;

pub const VIRTIO_VENDOR_ID: u16 = 0x1AF4;
pub const VIRTIO_BLK_LEGACY: u16 = 0x1001;
pub const VIRTIO_BLK_MODERN: u16 = 0x1042;
pub const VIRTIO_NET_LEGACY: u16 = 0x1000;
pub const VIRTIO_NET_MODERN: u16 = 0x1041;
pub const VIRTIO_GPU_MODERN: u16 = 0x1050;

// VirtIO PCI capability cfg_type values (VirtIO 1.0 spec §4.1.4)
const VIRTIO_PCI_CAP_COMMON_CFG: u8 = 1;
const VIRTIO_PCI_CAP_NOTIFY_CFG: u8 = 2;
const VIRTIO_PCI_CAP_ISR_CFG: u8 = 3;
const VIRTIO_PCI_CAP_DEVICE_CFG: u8 = 4;

/// Info returned by `find_virtio_gpu()` — MMIO base addresses ready for use via HHDM.
pub struct VirtioGpuPciInfo {
    pub bus: u8,
    pub slot: u8,
    pub func: u8,
    /// Physical base of the common config BAR region (map via HHDM + offset).
    pub common_cfg_phys: u64,
    /// Byte offset within the BAR to the common config struct.
    pub common_cfg_off: u32,
    /// Length of the common config capability window.
    pub common_cfg_len: u32,
    /// Physical base of the notify BAR region.
    pub notify_phys: u64,
    /// Byte offset within the BAR for queue notifications.
    pub notify_off: u32,
    /// Length of the notify capability window.
    pub notify_len: u32,
    /// Multiplier: notify address for queue N = notify_base + N * notify_off_multiplier.
    pub notify_off_multiplier: u32,
    /// Physical base of the ISR status BAR region.
    pub isr_phys: u64,
    pub isr_off: u32,
    /// Length of the ISR capability window.
    pub isr_len: u32,
}

/// Read a single byte from PCI config space.
///
/// SAFETY: caller must be at ring 0.
pub unsafe fn pci_read8(bus: u8, slot: u8, func: u8, offset: u8) -> u8 {
    let dword_off = offset & !3;
    let byte_sel = (offset & 3) as u32;
    let word = pci_read32(bus, slot, func, dword_off);
    ((word >> (byte_sel * 8)) & 0xFF) as u8
}

/// Read the physical base address of a 32-bit or 64-bit MMIO BAR.
/// Returns `None` if the BAR is an I/O BAR or the base is zero.
///
/// SAFETY: caller must be at ring 0.  The function briefly writes 0xFFFF_FFFF to the BAR
/// for sizing, then restores the original value; this must not race with any active DMA.
pub unsafe fn read_bar_mmio_base(bus: u8, slot: u8, func: u8, bar_idx: u8) -> Option<u64> {
    let reg_off = 0x10 + bar_idx * 4;
    let bar0 = pci_read32(bus, slot, func, reg_off);
    if bar0 & 1 != 0 {
        return None; // I/O BAR — not what we want
    }
    let bar_type = (bar0 >> 1) & 0x3;
    let base: u64 = if bar_type == 2 {
        // 64-bit BAR: high 32 bits in the next register
        let bar1 = pci_read32(bus, slot, func, reg_off + 4);
        ((bar1 as u64) << 32) | (bar0 as u64 & !0xF)
    } else {
        bar0 as u64 & !0xF
    };
    if base == 0 {
        None
    } else {
        Some(base)
    }
}

/// Walk the PCI capability list for a modern VirtIO GPU (device 0x1050) and return
/// the MMIO info needed to drive it.
///
/// SAFETY: caller must be at ring 0.
pub unsafe fn find_virtio_gpu() -> Option<VirtioGpuPciInfo> {
    for bus in 0u8..8 {
        for slot in 0u8..32 {
            let ids = pci_read32(bus, slot, 0, 0x00);
            if ids == 0xFFFF_FFFF {
                continue;
            }
            let vendor = (ids & 0xFFFF) as u16;
            let device = ((ids >> 16) & 0xFFFF) as u16;
            if vendor != VIRTIO_VENDOR_ID || device != VIRTIO_GPU_MODERN {
                continue;
            }

            // Found the device. Walk the PCI capability list (status bit 4 indicates caps present).
            let status = pci_read32(bus, slot, 0, 0x04) >> 16;
            if status & (1 << 4) == 0 {
                continue; // No capabilities list
            }

            let mut cap_off = pci_read8(bus, slot, 0, 0x34); // cap pointer
            let mut common_cfg_phys: Option<u64> = None;
            let mut common_cfg_off = 0u32;
            let mut common_cfg_len = 0u32;
            let mut notify_phys: Option<u64> = None;
            let mut notify_off = 0u32;
            let mut notify_len = 0u32;
            let mut notify_off_multiplier = 4u32;
            let mut isr_phys: Option<u64> = None;
            let mut isr_off = 0u32;
            let mut isr_len = 0u32;

            while cap_off != 0 && cap_off >= 0x40 {
                let cap_id = pci_read8(bus, slot, 0, cap_off);
                let cap_next = pci_read8(bus, slot, 0, cap_off + 1);

                if cap_id == 0x09 {
                    // Vendor-specific: check VirtIO capability structure
                    let cfg_type = pci_read8(bus, slot, 0, cap_off + 3);
                    let bar = pci_read8(bus, slot, 0, cap_off + 4);
                    // offset and length are u32 at cap_off+8 and cap_off+12
                    let bar_offset = pci_read32(bus, slot, 0, cap_off + 8);
                    let cap_len = pci_read32(bus, slot, 0, cap_off + 12);
                    let bar_phys = read_bar_mmio_base(bus, slot, 0, bar);

                    match cfg_type {
                        VIRTIO_PCI_CAP_COMMON_CFG => {
                            if let Some(phys) = bar_phys {
                                common_cfg_phys = Some(phys);
                                common_cfg_off = bar_offset;
                                common_cfg_len = cap_len;
                            }
                        }
                        VIRTIO_PCI_CAP_NOTIFY_CFG => {
                            if let Some(phys) = bar_phys {
                                notify_phys = Some(phys);
                                notify_off = bar_offset;
                                notify_len = cap_len;
                                // notify_off_multiplier is a u32 at cap_off+16
                                notify_off_multiplier = pci_read32(bus, slot, 0, cap_off + 16);
                            }
                        }
                        VIRTIO_PCI_CAP_ISR_CFG => {
                            if let Some(phys) = bar_phys {
                                isr_phys = Some(phys);
                                isr_off = bar_offset;
                                isr_len = cap_len;
                            }
                        }
                        VIRTIO_PCI_CAP_DEVICE_CFG => {
                            // Device-specific config (display resolution info). Not needed
                            // separately — we read it via GET_DISPLAY_INFO command.
                        }
                        _ => {}
                    }
                }

                cap_off = cap_next;
            }

            if let (Some(cp), Some(np), Some(ip)) = (common_cfg_phys, notify_phys, isr_phys) {
                return Some(VirtioGpuPciInfo {
                    bus,
                    slot,
                    func: 0,
                    common_cfg_phys: cp,
                    common_cfg_off,
                    common_cfg_len,
                    notify_phys: np,
                    notify_off,
                    notify_len,
                    notify_off_multiplier,
                    isr_phys: ip,
                    isr_off,
                    isr_len,
                });
            }
        }
    }
    None
}

/// Scan PCI buses 0-7 for a virtio-blk device.
/// Returns (bus, slot, func, io_base) on success.
///
/// SAFETY: Caller must be running at ring 0 (PCI port I/O requires privilege).
pub unsafe fn find_virtio_blk() -> Option<(u8, u8, u8, u16)> {
    for bus in 0u8..8 {
        for slot in 0u8..32 {
            let ids = pci_read32(bus, slot, 0, 0x00);
            if ids == 0xFFFF_FFFF {
                continue;
            }
            let vendor = (ids & 0xFFFF) as u16;
            let device = ((ids >> 16) & 0xFFFF) as u16;
            if vendor == VIRTIO_VENDOR_ID
                && (device == VIRTIO_BLK_LEGACY || device == VIRTIO_BLK_MODERN)
            {
                let bar0 = pci_read32(bus, slot, 0, 0x10);
                // Bit 0 = 1 means I/O BAR (legacy virtio uses I/O space)
                if bar0 & 1 == 1 {
                    let io_base = (bar0 & !0x3) as u16;
                    return Some((bus, slot, 0, io_base));
                }
            }
        }
    }
    None
}

/// Scan PCI buses 0-7 for a virtio-net device.
/// Returns (bus, slot, func, io_base) on success.
///
/// SAFETY: Caller must be running at ring 0 (PCI port I/O requires privilege).
pub unsafe fn find_virtio_net() -> Option<(u8, u8, u8, u16)> {
    for bus in 0u8..8 {
        for slot in 0u8..32 {
            let ids = pci_read32(bus, slot, 0, 0x00);
            if ids == 0xFFFF_FFFF {
                continue;
            }
            let vendor = (ids & 0xFFFF) as u16;
            let device = ((ids >> 16) & 0xFFFF) as u16;
            if vendor == VIRTIO_VENDOR_ID
                && (device == VIRTIO_NET_LEGACY || device == VIRTIO_NET_MODERN)
            {
                let bar0 = pci_read32(bus, slot, 0, 0x10);
                // Bit 0 = 1 means I/O BAR (legacy virtio uses I/O space)
                if bar0 & 1 == 1 {
                    let io_base = (bar0 & !0x3) as u16;
                    return Some((bus, slot, 0, io_base));
                }
            }
        }
    }
    None
}

pub unsafe fn pci_read32(bus: u8, slot: u8, func: u8, offset: u8) -> u32 {
    let addr: u32 = 0x8000_0000
        | ((bus as u32) << 16)
        | ((slot as u32) << 11)
        | ((func as u32) << 8)
        | ((offset as u32) & 0xFC);
    outl(CONFIG_ADDRESS, addr);
    inl(CONFIG_DATA)
}

// --- Port I/O primitives ---

pub unsafe fn outl(port: u16, val: u32) {
    // SAFETY: caller guarantees ring-0 privilege for port I/O.
    asm!(
        "out dx, eax",
        in("dx") port,
        in("eax") val,
        options(nomem, nostack, preserves_flags)
    );
}

pub unsafe fn inl(port: u16) -> u32 {
    let val: u32;
    // SAFETY: caller guarantees ring-0 privilege for port I/O.
    asm!(
        "in eax, dx",
        out("eax") val,
        in("dx") port,
        options(nomem, nostack, preserves_flags)
    );
    val
}

pub unsafe fn outb(port: u16, val: u8) {
    // SAFETY: caller guarantees ring-0 privilege for port I/O.
    asm!(
        "out dx, al",
        in("dx") port,
        in("al") val,
        options(nomem, nostack, preserves_flags)
    );
}

pub unsafe fn inb(port: u16) -> u8 {
    let val: u8;
    // SAFETY: caller guarantees ring-0 privilege for port I/O.
    asm!(
        "in al, dx",
        out("al") val,
        in("dx") port,
        options(nomem, nostack, preserves_flags)
    );
    val
}

pub unsafe fn outw(port: u16, val: u16) {
    // SAFETY: caller guarantees ring-0 privilege for port I/O.
    asm!(
        "out dx, ax",
        in("dx") port,
        in("ax") val,
        options(nomem, nostack, preserves_flags)
    );
}

pub unsafe fn inw(port: u16) -> u16 {
    let val: u16;
    // SAFETY: caller guarantees ring-0 privilege for port I/O.
    asm!(
        "in ax, dx",
        out("ax") val,
        in("dx") port,
        options(nomem, nostack, preserves_flags)
    );
    val
}
