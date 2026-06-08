use core::sync::atomic::{fence, Ordering};
use super::pci::{inl, inw, outb, outl, outw};

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

// virtio-blk request type: read
const VIRTIO_BLK_T_IN: u32 = 0;
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
        outb(io_base + REG_DEVICE_STATUS, STATUS_ACKNOWLEDGE | STATUS_DRIVER);

        // Read and echo features (we accept all; we only use basic read)
        let features = inl(io_base + REG_DEVICE_FEATURES);
        outl(io_base + REG_DRIVER_FEATURES, features & !((1 << 5) | (1 << 7)));

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
        })
    }

    /// Read a 512-byte sector at `lba` into `buf`.
    ///
    /// SAFETY: All pointers initialized in `init` must still be valid.
    pub unsafe fn read_block(&mut self, lba: u64, buf: &mut [u8; 512]) -> bool {
        // --- Build request header at req_virt[0..16] ---
        // type (u32): 0 = IN (read)
        // ioprio (u32): 0
        // sector (u64): lba
        // SAFETY: req_virt points to a valid writable page initialized in init.
        (self.req_virt as *mut u32).write_volatile(VIRTIO_BLK_T_IN);
        ((self.req_virt + 4) as *mut u32).write_volatile(0);
        ((self.req_virt + 8) as *mut u64).write_volatile(lba);

        // Status byte at req_virt + 16 + 512 = req_virt + 528; device writes here
        let status_ptr = (self.req_virt + 528) as *mut u8;
        status_ptr.write_volatile(0xFF); // sentinel

        // --- Fill descriptor table entries (3 descriptors starting at index 0) ---
        // Descriptor 0: request header (device reads)
        let d0 = self.desc_virt as *mut VirtqDesc;
        (*d0).addr = self.req_phys;
        (*d0).len = 16;
        (*d0).flags = DESC_F_NEXT;
        (*d0).next = 1;

        // Descriptor 1: 512-byte data buffer (device writes)
        let d1 = (self.desc_virt + 16) as *mut VirtqDesc;
        (*d1).addr = self.req_phys + 16;
        (*d1).len = 512;
        (*d1).flags = DESC_F_WRITE | DESC_F_NEXT;
        (*d1).next = 2;

        // Descriptor 2: status byte (device writes)
        let d2 = (self.desc_virt + 32) as *mut VirtqDesc;
        (*d2).addr = self.req_phys + 528;
        (*d2).len = 1;
        (*d2).flags = DESC_F_WRITE;
        (*d2).next = 0;

        // --- Push to available ring ---
        // Available ring layout: [flags: u16][idx: u16][ring: u16 * qsize]...
        let avail_ring_ptr = (self.avail_virt + 4) as *mut u16; // ring array starts at offset 4
        let slot = (self.avail_idx as usize) % (self.queue_size as usize);
        avail_ring_ptr.add(slot).write_volatile(0); // descriptor chain head = 0

        fence(Ordering::SeqCst);

        let avail_idx_ptr = (self.avail_virt + 2) as *mut u16;
        let new_idx = self.avail_idx.wrapping_add(1);
        avail_idx_ptr.write_volatile(new_idx);
        self.avail_idx = new_idx;

        fence(Ordering::SeqCst);

        // Notify device that queue 0 has new entries
        // SAFETY: ring-0 I/O port access.
        outw(self.io_base + REG_QUEUE_NOTIFY, 0);

        // --- Poll used ring until device completes ---
        // Used ring layout: [flags: u16][idx: u16][ring: {id: u32, len: u32} * qsize]...
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

        // Check the status byte written by the device
        fence(Ordering::SeqCst);
        if status_ptr.read_volatile() != VIRTIO_BLK_S_OK {
            return false;
        }

        // Copy data from request buffer to caller's buffer
        // SAFETY: req_virt + 16 points to the 512-byte data region.
        core::ptr::copy_nonoverlapping((self.req_virt + 16) as *const u8, buf.as_mut_ptr(), 512);
        true
    }
}

// SAFETY: VirtioBlk wraps raw pointers into kernel-owned physical frames.
// The kernel is single-threaded during initialization.
unsafe impl Send for VirtioBlk {}
