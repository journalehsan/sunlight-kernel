use core::arch::asm;

const CONFIG_ADDRESS: u16 = 0xCF8;
const CONFIG_DATA: u16 = 0xCFC;

pub const VIRTIO_VENDOR_ID: u16 = 0x1AF4;
pub const VIRTIO_NET_LEGACY: u16 = 0x1000;
pub const VIRTIO_NET_MODERN: u16 = 0x1041;

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
