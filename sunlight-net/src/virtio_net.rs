use crate::pci;
use core::sync::atomic::{fence, Ordering};

// Virtio feature bits for networking (Phase 5.0: only MAC + STATUS required)
pub const VIRTIO_NET_F_MAC: u32 = 1 << 5;
pub const VIRTIO_NET_F_STATUS: u32 = 1 << 16;
pub const VIRTIO_NET_F_CTRL_VQ: u32 = 1 << 17;
pub const VIRTIO_NET_F_MRG_RXBUF: u32 = 1 << 15;

// Virtio device registers (legacy I/O BAR)
const VIRTIO_REG_DEVICE_FEATURES: u16 = 0x00;
const VIRTIO_REG_DRIVER_FEATURES: u16 = 0x04;
const VIRTIO_REG_QUEUE_PFN: u16 = 0x08; // PFN for legacy
const VIRTIO_REG_QUEUE_NUM: u16 = 0x0C;
const VIRTIO_REG_QUEUE_SEL: u16 = 0x0E;
const VIRTIO_REG_QUEUE_NOTIFY: u16 = 0x10;
const VIRTIO_REG_DEVICE_STATUS: u16 = 0x12;
const VIRTIO_REG_CONFIG: u16 = 0x14;

// Status bits
const STATUS_ACKNOWLEDGE: u8 = 0x01;
const STATUS_DRIVER: u8 = 0x02;
const STATUS_DRIVER_OK: u8 = 0x04;
const STATUS_FEATURES_OK: u8 = 0x08;

// Virtqueue descriptor flags (same as virtio-blk)
const DESC_F_WRITE: u16 = 2;

/// Virtio-net packet header (must precede every Ethernet frame on RX/TX).
/// 10 bytes for basic; 12 bytes when VIRTIO_NET_F_MRG_RXBUF negotiated (we do not).
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct VirtioNetHeader {
    pub flags: u8,
    pub gso_type: u8,
    pub hdr_len: u16,
    pub gso_size: u16,
    pub csum_start: u16,
    pub csum_offset: u16,
}

/// We allocate 4 pages per queue (RX + TX = 8 pages total) — same sizing as blk.
pub const QUEUE_PAGES_PER_NET_QUEUE: usize = 4;

/// Queue layout state for one virtqueue (RX or TX).
struct NetVirtq {
    queue_size: u16,
    // Virtual (HHDM-mapped) addresses of the three rings
    desc_virt: u64,
    avail_virt: u64,
    used_virt: u64,
    // Tracking indices (driver side)
    avail_idx: u16,
    last_used_idx: u16,
}

/// Virtio-net device driver (kernel ring-0 only).
///
/// The driver owns two virtqueues and performs all port I/O + DMA ring manipulation.
/// Packet buffers passed to send/recv must be physically contiguous and their
/// physical addresses known to the caller (the kernel maps them).
pub struct VirtioNet {
    io_base: u16,
    mac: [u8; 6],
    bus: u8,
    slot: u8,

    // RX queue (index 0)
    rx: NetVirtq,
    // We keep a small set of pre-supplied RX buffer physical addresses.
    // For MVI we use a single RX buffer that we re-arm after consume.
    rx_buf_phys: u64,
    rx_buf_virt: u64,
    rx_buf_len: usize,

    // TX queue (index 1)
    tx: NetVirtq,
    // Dedicated TX staging buffer, separate from the RX buffer above. The RX
    // descriptor stays armed (DESC_F_WRITE) for the whole device lifetime, so
    // reusing it as TX staging would let an inbound DMA write race with our
    // outbound frame write/read — see send().
    tx_buf_phys: u64,
    tx_buf_virt: u64,
}

#[derive(Debug)]
pub enum NetError {
    NotFound,
    InitFailed,
    QueueError,
    NoPacket,
}

/// SAFETY: VirtioNet holds raw pointers (via virt addresses) into kernel-owned
/// physically contiguous frames that live for the lifetime of the kernel.
/// Access is expected to be serialized (single-threaded boot + later mutex if needed).
unsafe impl Send for VirtioNet {}

impl VirtioNet {
    /// Initialize a legacy virtio-net device and set up its RX (q0) + TX (q1) virtqueues.
    ///
    /// `rx_queue_phys/virt`, `tx_queue_phys/virt`: two separate physically-contiguous
    /// regions of QUEUE_PAGES_PER_NET_QUEUE * 4096 bytes each (caller allocates via PMM + HHDM).
    ///
    /// `rx_buf_phys/virt`: at least 1514+header bytes physically contiguous buffer for RX.
    /// The driver will arm the RX queue with this buffer.
    ///
    /// All addresses must be valid; caller must be ring 0.
    ///
    /// SAFETY: The physical and virtual addresses must remain valid and the memory
    /// must not be repurposed while this device is in use. Port I/O is privileged.
    pub unsafe fn init(
        io_base: u16,
        bus: u8,
        slot: u8,
        rx_queue_phys: u64,
        rx_queue_virt: u64,
        tx_queue_phys: u64,
        tx_queue_virt: u64,
        rx_buf_phys: u64,
        rx_buf_virt: u64,
        rx_buf_len: usize,
        tx_buf_phys: u64,
        tx_buf_virt: u64,
    ) -> Option<Self> {
        // --- Reset + feature negotiation (identical pattern to virtio-blk) ---
        // SAFETY: io_base is a valid legacy virtio I/O BAR; ring-0 required.
        pci::outb(io_base + VIRTIO_REG_DEVICE_STATUS, 0); // reset

        pci::outb(io_base + VIRTIO_REG_DEVICE_STATUS, STATUS_ACKNOWLEDGE | STATUS_DRIVER);

        let features = pci::inl(io_base + VIRTIO_REG_DEVICE_FEATURES);
        // We only require/ack MAC + STATUS for Phase 5.0. Drop MRG/CTRL for simplicity.
        let supported = VIRTIO_NET_F_MAC | VIRTIO_NET_F_STATUS;
        let driver_features = features & supported;
        pci::outl(io_base + VIRTIO_REG_DRIVER_FEATURES, driver_features);

        pci::outb(
            io_base + VIRTIO_REG_DEVICE_STATUS,
            STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK,
        );

        // --- Read MAC from config space ---
        let mut mac = [0u8; 6];
        for i in 0..6 {
            mac[i] = pci::inb(io_base + VIRTIO_REG_CONFIG + (i as u16));
        }

        // --- Initialize RX queue (sel 0) ---
        pci::outw(io_base + VIRTIO_REG_QUEUE_SEL, 0);
        let qsize = pci::inw(io_base + VIRTIO_REG_QUEUE_NUM);
        if qsize == 0 || qsize > 256 {
            return None;
        }

        // Layout inside the supplied queue memory (same math as blk):
        // desc table: qsize * 16 bytes
        // avail: 6 + qsize*2 , then align for used
        let avail_off = (qsize as u64) * 16;
        let avail_end = avail_off + 6 + (qsize as u64) * 2;
        let used_off = (avail_end + 4095) & !4095;

        // Zero the rings
        // SAFETY: rx_queue_virt points to caller-allocated physically contiguous pages.
        unsafe {
            (rx_queue_virt as *mut u8).write_bytes(0, QUEUE_PAGES_PER_NET_QUEUE * 4096);
        }

        // SAFETY: ring-0 I/O to tell device the queue physical page frame number.
        unsafe {
            pci::outl(io_base + VIRTIO_REG_QUEUE_PFN, (rx_queue_phys >> 12) as u32);
        }

        let mut rx = NetVirtq {
            queue_size: qsize,
            desc_virt: rx_queue_virt,
            avail_virt: rx_queue_virt + avail_off,
            used_virt: rx_queue_virt + used_off,
            avail_idx: 0,
            last_used_idx: 0,
        };

        // Arm one RX descriptor pointing at our RX buffer (header + frame space)
        // Descriptor 0: RX buffer (device writes)
        // SAFETY: The desc table lives in the rx_queue memory we just zeroed and which
        // remains valid for the device lifetime. All fields are plain integers.
        unsafe {
            let d0 = rx.desc_virt as *mut VirtqDesc;
            (*d0).addr = rx_buf_phys;
            (*d0).len = (core::mem::size_of::<VirtioNetHeader>() + rx_buf_len) as u32;
            (*d0).flags = DESC_F_WRITE; // device writes the whole thing
            (*d0).next = 0;
        }

        // Push to avail ring (index 0)
        // SAFETY: avail ring is inside our queue allocation; write_volatile + fence for device visibility.
        unsafe {
            let avail_ring_ptr = (rx.avail_virt + 4) as *mut u16;
            avail_ring_ptr.write_volatile(0);
        }

        fence(Ordering::SeqCst);

        // SAFETY: update driver-tracked avail index in the ring and notify device.
        unsafe {
            let avail_idx_ptr = (rx.avail_virt + 2) as *mut u16;
            avail_idx_ptr.write_volatile(1);
            pci::outw(io_base + VIRTIO_REG_QUEUE_NOTIFY, 0); // notify RX queue
        }
        rx.avail_idx = 1; // keep driver tracking in sync with what we wrote to the ring.

        // --- Initialize TX queue (sel 1) ---
        pci::outw(io_base + VIRTIO_REG_QUEUE_SEL, 1);
        let qsize_tx = pci::inw(io_base + VIRTIO_REG_QUEUE_NUM);
        if qsize_tx == 0 || qsize_tx > 256 {
            return None;
        }

        let avail_off_tx = (qsize_tx as u64) * 16;
        let avail_end_tx = avail_off_tx + 6 + (qsize_tx as u64) * 2;
        let used_off_tx = (avail_end_tx + 4095) & !4095;

        // SAFETY: tx_queue_virt points to caller-allocated pages.
        unsafe {
            (tx_queue_virt as *mut u8).write_bytes(0, QUEUE_PAGES_PER_NET_QUEUE * 4096);
        }

        // SAFETY: ring-0 I/O for TX queue PFN.
        unsafe {
            pci::outl(io_base + VIRTIO_REG_QUEUE_PFN, (tx_queue_phys >> 12) as u32);
        }

        let tx = NetVirtq {
            queue_size: qsize_tx,
            desc_virt: tx_queue_virt,
            avail_virt: tx_queue_virt + avail_off_tx,
            used_virt: tx_queue_virt + used_off_tx,
            avail_idx: 0,
            last_used_idx: 0,
        };

        // Driver OK
        // SAFETY: final status write to complete initialization handshake.
        unsafe {
            pci::outb(
                io_base + VIRTIO_REG_DEVICE_STATUS,
                STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_FEATURES_OK | STATUS_DRIVER_OK,
            );
        }

        Some(VirtioNet {
            io_base,
            mac,
            bus,
            slot,
            rx,
            rx_buf_phys,
            rx_buf_virt,
            rx_buf_len,
            tx,
            tx_buf_phys,
            tx_buf_virt,
        })
    }

    pub fn mac(&self) -> [u8; 6] {
        self.mac
    }

    pub fn pci_location(&self) -> (u8, u8) {
        (self.bus, self.slot)
    }

    /// Try to receive one packet into `buf` (after the header space in our RX buffer).
    /// Returns number of Ethernet bytes copied (0 if none ready).
    ///
    /// The caller sees only the Ethernet frame; header is stripped.
    ///
    /// SAFETY: Must only be called while the queues and buffers passed to init remain valid.
    pub unsafe fn recv(&mut self, buf: &mut [u8]) -> usize {
        // Check used ring for completed RX
        // SAFETY: used ring is part of the rx queue memory supplied at init and remains valid.
        let used_idx_ptr = (self.rx.used_virt + 2) as *const u16;
        fence(Ordering::SeqCst);
        if unsafe { *used_idx_ptr } == self.rx.last_used_idx {
            return 0; // nothing
        }

        // We only ever arm descriptor 0, but the device writes each completion
        // to the used ring slot at (used.idx % queue_size), not always slot 0.
        // Read the slot matching our tracked last_used_idx.
        // Used ring entry layout after flags+idx: {id:u32, len:u32}, 8 bytes/entry.
        let slot = (self.rx.last_used_idx as usize) % (self.rx.queue_size as usize);
        let used_entry_base = self.rx.used_virt + 4 + (slot as u64) * 8;
        // SAFETY: volatile reads from the used ring written by the device.
        let used0_id = unsafe { ((used_entry_base) as *const u32).read_volatile() };
        let used0_len = unsafe { ((used_entry_base + 4) as *const u32).read_volatile() };

        self.rx.last_used_idx = self.rx.last_used_idx.wrapping_add(1);

        if used0_id != 0 {
            // Unexpected descriptor chain; re-arm and bail
            self.rearm_rx();
            return 0;
        }

        // The device wrote VirtioNetHeader + frame into rx_buf_virt.
        // Length reported by device includes the header.
        let total_written = used0_len as usize;
        let hdr_sz = core::mem::size_of::<VirtioNetHeader>();
        if total_written <= hdr_sz {
            self.rearm_rx();
            return 0;
        }

        let frame_len = (total_written - hdr_sz).min(buf.len());
        let frame_src = (self.rx_buf_virt as *const u8).add(hdr_sz);
        // SAFETY: copy from our RX buffer (device-written) into caller buffer.
        unsafe {
            core::ptr::copy_nonoverlapping(frame_src, buf.as_mut_ptr(), frame_len);
        }

        self.rearm_rx();
        frame_len
    }

    fn rearm_rx(&mut self) {
        // Re-arm descriptor 0 with same buffer for next packet.
        // SAFETY: All pointers and memory regions were validated at init time and the
        // caller of recv (which calls us) guarantees the device/queues are still live.
        // We are inside an unsafe fn (recv) so the contract holds.
        unsafe {
            let d0 = self.rx.desc_virt as *mut VirtqDesc;
            (*d0).addr = self.rx_buf_phys;
            (*d0).len = (core::mem::size_of::<VirtioNetHeader>() + self.rx_buf_len) as u32;
            (*d0).flags = DESC_F_WRITE;
            (*d0).next = 0;

            let slot = (self.rx.avail_idx as usize) % (self.rx.queue_size as usize);
            let avail_ring_ptr = (self.rx.avail_virt + 4) as *mut u16;
            avail_ring_ptr.add(slot).write_volatile(0);

            fence(Ordering::SeqCst);

            let avail_idx_ptr = (self.rx.avail_virt + 2) as *mut u16;
            let new_idx = self.rx.avail_idx.wrapping_add(1);
            avail_idx_ptr.write_volatile(new_idx);
            self.rx.avail_idx = new_idx;

            fence(Ordering::SeqCst);

            pci::outw(self.io_base + VIRTIO_REG_QUEUE_NOTIFY, 0);
        }
    }

    /// Transmit an Ethernet frame. Prepends the virtio-net header internally.
    /// Returns Ok(()) on success (we wait for used entry for MVI simplicity).
    ///
    /// SAFETY: `frame` data must remain valid for the duration of the (short) TX.
    /// The internal TX scratch must not alias with caller memory.
    pub unsafe fn send(&mut self, frame: &[u8]) -> Result<(), NetError> {
        if frame.len() > self.rx_buf_len {
            // Reuse rx_buf_len as a reasonable MTU proxy for the TX buffer size.
            return Err(NetError::QueueError);
        }

        // Dedicated TX staging buffer (separate from the RX buffer, which stays
        // armed with DESC_F_WRITE for the device to DMA incoming frames into at
        // any time).
        let tx_buf_virt = self.tx_buf_virt;
        let tx_buf_phys = self.tx_buf_phys;

        // Write header + frame into the staging area.
        let hdr = VirtioNetHeader {
            flags: 0,
            gso_type: 0,
            hdr_len: 0,
            gso_size: 0,
            csum_start: 0,
            csum_offset: 0,
        };
        // SAFETY: tx staging is valid kernel memory supplied at init (we reuse the rx scratch for MVI).
        unsafe {
            core::ptr::write_volatile(tx_buf_virt as *mut VirtioNetHeader, hdr);
        }
        let frame_dst = (tx_buf_virt as *mut u8).add(core::mem::size_of::<VirtioNetHeader>());
        // SAFETY: copy caller's frame into our controlled TX staging area.
        unsafe {
            core::ptr::copy_nonoverlapping(frame.as_ptr(), frame_dst, frame.len());
        }

        let total_len = core::mem::size_of::<VirtioNetHeader>() + frame.len();

        // Build descriptor chain (single desc for the whole buffer for MVI)
        // SAFETY: TX desc/avail rings are the ones we set up in init and that remain valid.
        unsafe {
            let d0 = self.tx.desc_virt as *mut VirtqDesc;
            (*d0).addr = tx_buf_phys;
            (*d0).len = total_len as u32;
            (*d0).flags = 0; // device reads
            (*d0).next = 0;

            let slot = (self.tx.avail_idx as usize) % (self.tx.queue_size as usize);
            let avail_ring_ptr = (self.tx.avail_virt + 4) as *mut u16;
            avail_ring_ptr.add(slot).write_volatile(0);

            fence(Ordering::SeqCst);

            let avail_idx_ptr = (self.tx.avail_virt + 2) as *mut u16;
            let new_idx = self.tx.avail_idx.wrapping_add(1);
            avail_idx_ptr.write_volatile(new_idx);
            self.tx.avail_idx = new_idx;

            fence(Ordering::SeqCst);

            pci::outw(self.io_base + VIRTIO_REG_QUEUE_NOTIFY, 1);
        }

        // Poll used ring (bounded) — MVI simple blocking TX
        // SAFETY: read from used ring + notify are device-visible via fences.
        unsafe {
            let used_idx_ptr = (self.tx.used_virt + 2) as *const u16;
            let mut limit = 50_000_000u32;
            loop {
                fence(Ordering::SeqCst);
                if (*used_idx_ptr) != self.tx.last_used_idx {
                    self.tx.last_used_idx = self.tx.last_used_idx.wrapping_add(1);
                    break;
                }
                limit -= 1;
                if limit == 0 {
                    return Err(NetError::QueueError);
                }
                core::hint::spin_loop();
            }
        }

        Ok(())
    }
}

/// Descriptor (same layout as blk)
#[repr(C)]
struct VirtqDesc {
    addr: u64,
    len: u32,
    flags: u16,
    next: u16,
}
