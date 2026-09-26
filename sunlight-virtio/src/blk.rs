use super::pci::{inl, inw, outb, outl, outw};
use core::sync::atomic::{fence, Ordering};

// Legacy virtio-blk I/O register offsets from io_base
const REG_DEVICE_FEATURES: u16 = 0x00;
const REG_DRIVER_FEATURES: u16 = 0x04;
const REG_QUEUE_PFN: u16 = 0x08;
const REG_QUEUE_NUM: u16 = 0x0C;
const REG_QUEUE_SEL: u16 = 0x0E;
const REG_QUEUE_NOTIFY: u16 = 0x10;
const REG_DEVICE_STATUS: u16 = 0x12;

// Device status bits
const STATUS_ACKNOWLEDGE: u8 = 1;
const STATUS_DRIVER: u8 = 2;
const STATUS_DRIVER_OK: u8 = 4;

// Virtqueue descriptor flags
const DESC_F_NEXT: u16 = 1;
const DESC_F_WRITE: u16 = 2;

// virtio-blk request types.
const VIRTIO_BLK_T_IN: u32 = 0;
const VIRTIO_BLK_T_OUT: u32 = 1;
const VIRTIO_BLK_T_FLUSH: u32 = 4;
const VIRTIO_BLK_F_FLUSH: u32 = 1 << 9;
// virtio-blk status: OK
const VIRTIO_BLK_S_OK: u8 = 0;

// We allocate 4 pages (16384 bytes) for the virtqueue.
// This is enough for QUEUE_NUM up to ~500.
pub const QUEUE_PAGES: usize = 4;

#[repr(C)]
struct VirtqDesc {
    addr: u64,
    len: u32,
    flags: u16,
    next: u16,
}

pub enum BlkError {
    QueueEmpty,
    Timeout,
    DeviceError,
}

pub struct VirtioBlk {
    io_base: u16,
    queue_size: u16,
    // Virtual addresses of virtqueue sections
    desc_virt: u64,
    avail_virt: u64,
    used_virt: u64,
    // Request buffer (virtual and physical)
    req_phys: u64,
    req_virt: u64,
    // Tracking
    avail_idx: u16,
    last_used_idx: u16,
    capacity_sectors: u64,
    flush_supported: bool,
}

impl VirtioBlk {
    /// Initialize a legacy virtio-blk device.
    ///
    /// `queue_phys` / `queue_virt`: physically-contiguous 4-page region for virtqueue.
    /// `req_phys` / `req_virt`: 1-page region for the request buffer (header+data+status).
    ///
    /// SAFETY: All physical/virtual address pairs must be valid; caller holds ring-0 privilege.
    pub unsafe fn init(
        io_base: u16,
        queue_phys: u64,
        queue_virt: u64,
        req_phys: u64,
        req_virt: u64,
    ) -> Option<Self> {
        // Reset the device
        outb(io_base + REG_DEVICE_STATUS, 0);
        // Acknowledge device existence and load our driver
        outb(
            io_base + REG_DEVICE_STATUS,
            STATUS_ACKNOWLEDGE | STATUS_DRIVER,
        );

        // Preserve the legacy feature policy, but remember whether the device
        // negotiated the stable-media barrier required by persistent state.
        let features = inl(io_base + REG_DEVICE_FEATURES);
        let accepted_features = features & !((1 << 5) | (1 << 7));
        outl(io_base + REG_DRIVER_FEATURES, accepted_features);

        // Legacy virtio-blk device-specific configuration starts at 0x14
        // when MSI-X is not in use. Capacity is expressed in 512-byte sectors.
        let capacity_sectors = (inl(io_base + 0x18) as u64) << 32 | inl(io_base + 0x14) as u64;

        // Select queue 0
        outw(io_base + REG_QUEUE_SEL, 0);
        let qsize = inw(io_base + REG_QUEUE_NUM);
        if qsize == 0 {
            return None;
        }

        // Compute ring offsets based on the reported queue size
        let avail_off = (qsize as u64) * 16; // desc table = qsize * sizeof(VirtqDesc)
        let avail_end = avail_off + 6 + (qsize as u64) * 2; // flags + idx + ring + used_event
        let used_off = (avail_end + 4095) & !4095; // align to page

        // Zero the entire queue region
        // SAFETY: queue_virt points to QUEUE_PAGES * 4096 bytes of valid writable kernel memory.
        (queue_virt as *mut u8).write_bytes(0, QUEUE_PAGES * 4096);
        // Zero the request buffer
        // SAFETY: req_virt points to 4096 bytes of valid writable kernel memory.
        (req_virt as *mut u8).write_bytes(0, 4096);

        // Tell device the queue address (physical page frame number)
        outl(io_base + REG_QUEUE_PFN, (queue_phys >> 12) as u32);

        // Signal driver ready
        outb(
            io_base + REG_DEVICE_STATUS,
            STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_DRIVER_OK,
        );

        Some(VirtioBlk {
            io_base,
            queue_size: qsize,
            desc_virt: queue_virt,
            avail_virt: queue_virt + avail_off,
            used_virt: queue_virt + used_off,
            req_phys,
            req_virt,
            avail_idx: 0,
            last_used_idx: 0,
            capacity_sectors,
            flush_supported: accepted_features & VIRTIO_BLK_F_FLUSH != 0,
        })
    }

    unsafe fn submit(&mut self, request_type: u32, lba: u64, data_is_device_write: bool) -> bool {
        (self.req_virt as *mut u32).write_volatile(request_type);
        ((self.req_virt + 4) as *mut u32).write_volatile(0);
        ((self.req_virt + 8) as *mut u64).write_volatile(lba);

        let status_ptr = (self.req_virt + 528) as *mut u8;
        status_ptr.write_volatile(0xFF);

        let d0 = self.desc_virt as *mut VirtqDesc;
        (*d0).addr = self.req_phys;
        (*d0).len = 16;
        (*d0).flags = DESC_F_NEXT;
        (*d0).next = 1;

        if request_type == VIRTIO_BLK_T_FLUSH {
            let d1 = (self.desc_virt + 16) as *mut VirtqDesc;
            (*d1).addr = self.req_phys + 528;
            (*d1).len = 1;
            (*d1).flags = DESC_F_WRITE;
            (*d1).next = 0;
        } else {
            let d1 = (self.desc_virt + 16) as *mut VirtqDesc;
            (*d1).addr = self.req_phys + 16;
            (*d1).len = 512;
            (*d1).flags = DESC_F_NEXT
                | if data_is_device_write {
                    DESC_F_WRITE
                } else {
                    0
                };
            (*d1).next = 2;

            let d2 = (self.desc_virt + 32) as *mut VirtqDesc;
            (*d2).addr = self.req_phys + 528;
            (*d2).len = 1;
            (*d2).flags = DESC_F_WRITE;
            (*d2).next = 0;
        }

        let avail_ring_ptr = (self.avail_virt + 4) as *mut u16;
        let slot = (self.avail_idx as usize) % (self.queue_size as usize);
        avail_ring_ptr.add(slot).write_volatile(0);
        fence(Ordering::SeqCst);

        let new_idx = self.avail_idx.wrapping_add(1);
        ((self.avail_virt + 2) as *mut u16).write_volatile(new_idx);
        self.avail_idx = new_idx;
        fence(Ordering::SeqCst);
        outw(self.io_base + REG_QUEUE_NOTIFY, 0);

        let used_idx_ptr = (self.used_virt + 2) as *const u16;
        let mut limit = 50_000_000u32;
        loop {
            fence(Ordering::SeqCst);
            if used_idx_ptr.read_volatile() != self.last_used_idx {
                break;
            }
            limit -= 1;
            if limit == 0 {
                return false;
            }
            core::hint::spin_loop();
        }
        self.last_used_idx = self.last_used_idx.wrapping_add(1);
        fence(Ordering::SeqCst);
        status_ptr.read_volatile() == VIRTIO_BLK_S_OK
    }

    /// Read a 512-byte sector at `lba` into `buf`.
    ///
    /// SAFETY: All pointers initialized in `init` must still be valid.
    pub unsafe fn read_block(&mut self, lba: u64, buf: &mut [u8; 512]) -> bool {
        if lba >= self.capacity_sectors || !self.submit(VIRTIO_BLK_T_IN, lba, true) {
            return false;
        }
        core::ptr::copy_nonoverlapping((self.req_virt + 16) as *const u8, buf.as_mut_ptr(), 512);
        true
    }

    /// Write one 512-byte sector. Completion means accepted by the device;
    /// call [`VirtioBlk::flush`] for stable-media durability.
    pub unsafe fn write_block(&mut self, lba: u64, buf: &[u8; 512]) -> bool {
        if lba >= self.capacity_sectors {
            return false;
        }
        core::ptr::copy_nonoverlapping(buf.as_ptr(), (self.req_virt + 16) as *mut u8, 512);
        self.submit(VIRTIO_BLK_T_OUT, lba, false)
    }

    /// Issue the negotiated virtio-blk stable-media barrier.
    pub unsafe fn flush(&mut self) -> bool {
        self.flush_supported && self.submit(VIRTIO_BLK_T_FLUSH, 0, false)
    }

    pub const fn block_count(&self) -> u64 {
        self.capacity_sectors
    }

    pub const fn supports_flush(&self) -> bool {
        self.flush_supported
    }
}

// SAFETY: VirtioBlk wraps raw pointers into kernel-owned physical frames.
// The kernel is single-threaded during initialization.
unsafe impl Send for VirtioBlk {}
