use crate::pci;
use crate::NetDeviceCounters;
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

/// Number of pre-posted RX buffers / descriptors. Having >1 greatly reduces
/// the chance of the virtio device dropping inbound packets while the driver
/// is processing + re-arming a single buffer (the previous design).
pub const MAX_RX_BUFFERS: usize = 4;

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
    // Multiple pre-supplied RX buffers (posted via distinct descriptors).
    // This allows the device to deliver several inbound frames without
    // requiring an immediate re-arm from the driver for each one.
    rx_buf_phys: [u64; MAX_RX_BUFFERS],
    rx_buf_virt: [u64; MAX_RX_BUFFERS],
    rx_buf_len: usize,

    // TX queue (index 1)
    tx: NetVirtq,
    // Dedicated TX staging buffer, separate from the RX buffer above. The RX
    // descriptor stays armed (DESC_F_WRITE) for the whole device lifetime, so
    // reusing it as TX staging would let an inbound DMA write race with our
    // outbound frame write/read — see send().
    tx_buf_phys: u64,
    tx_buf_virt: u64,
    counters: NetDeviceCounters,
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
    /// `rx_buf_phys/virt` (arrays): RX data buffers for the descriptors we will arm.
    /// We pre-arm MAX_RX_BUFFERS of them to avoid dropping inbound packets during re-arm.
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
        rx_buf_phys: [u64; MAX_RX_BUFFERS],
        rx_buf_virt: [u64; MAX_RX_BUFFERS],
        rx_buf_len: usize,
        tx_buf_phys: u64,
        tx_buf_virt: u64,
    ) -> Option<Self> {
        // --- Reset + feature negotiation (identical pattern to virtio-blk) ---
        // SAFETY: io_base is a valid legacy virtio I/O BAR; ring-0 required.
        pci::outb(io_base + VIRTIO_REG_DEVICE_STATUS, 0); // reset

        pci::outb(
            io_base + VIRTIO_REG_DEVICE_STATUS,
            STATUS_ACKNOWLEDGE | STATUS_DRIVER,
        );

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

        // Arm multiple RX descriptors (one per buffer). We use desc ids 0..MAX_RX_BUFFERS-1.
        // Having several available lets the device DMA multiple inbound packets
        // (e.g. during a transfer or when ARP + data arrive close together) without
        // the driver having to immediately re-supply after each consume.
        unsafe {
            for i in 0..MAX_RX_BUFFERS {
                let d = (rx.desc_virt as *mut VirtqDesc).add(i);
                (*d).addr = rx_buf_phys[i];
                (*d).len = (core::mem::size_of::<VirtioNetHeader>() + rx_buf_len) as u32;
                (*d).flags = DESC_F_WRITE;
                (*d).next = 0;

                let avail_ring_ptr = (rx.avail_virt + 4) as *mut u16;
                avail_ring_ptr.add(i).write_volatile(i as u16);
            }
        }

        fence(Ordering::SeqCst);

        // SAFETY: set initial avail head past the ones we just supplied and notify.
        unsafe {
            let avail_idx_ptr = (rx.avail_virt + 2) as *mut u16;
            avail_idx_ptr.write_volatile(MAX_RX_BUFFERS as u16);
            pci::outw(io_base + VIRTIO_REG_QUEUE_NOTIFY, 0); // notify RX queue
        }
        rx.avail_idx = MAX_RX_BUFFERS as u16;

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
            counters: NetDeviceCounters::default(),
        })
    }

    pub fn mac(&self) -> [u8; 6] {
        self.mac
    }

    pub fn pci_location(&self) -> (u8, u8) {
        (self.bus, self.slot)
    }

    pub fn counters(&self) -> NetDeviceCounters {
        self.counters
    }

    /// Try to receive one packet into `buf` (after the header space in our RX buffer).
    /// Returns number of Ethernet bytes copied (0 if none ready).
    ///
    /// The caller sees only the Ethernet frame; header is stripped.
    ///
    /// SAFETY: Must only be called while the queues and buffers passed to init remain valid.
    pub unsafe fn recv(&mut self, buf: &mut [u8]) -> usize {
        self.counters.polls = self.counters.polls.saturating_add(1);
        // Check used ring for completed RX
        // SAFETY: used ring is part of the rx queue memory supplied at init and remains valid.
        let used_idx_ptr = (self.rx.used_virt + 2) as *const u16;
        fence(Ordering::SeqCst);
        if unsafe { used_idx_ptr.read_volatile() } == self.rx.last_used_idx {
            return 0; // nothing
        }

        // Read the used entry for the next driver-owned slot.
        // We now support multiple RX buffers (desc ids 0..MAX_RX_BUFFERS-1).
        let slot = (self.rx.last_used_idx as usize) % (self.rx.queue_size as usize);
        let used_entry_base = self.rx.used_virt + 4 + (slot as u64) * 8;
        // SAFETY: volatile reads from the used ring written by the device.
        let used_id = unsafe { ((used_entry_base) as *const u32).read_volatile() };
        let used_len = unsafe { ((used_entry_base + 4) as *const u32).read_volatile() };

        self.rx.last_used_idx = self.rx.last_used_idx.wrapping_add(1);
        self.counters.rx_completed = self.counters.rx_completed.saturating_add(1);

        let buf_idx = used_id as usize;
        if buf_idx >= MAX_RX_BUFFERS {
            // Unexpected desc id (should never happen with our arming); re-arm the id we saw and bail.
            self.rearm_desc(used_id);
            self.counters.rx_bad_completion = self.counters.rx_bad_completion.saturating_add(1);
            return 0;
        }

        // The device wrote VirtioNetHeader + frame into the chosen rx_buf.
        let total_written = used_len as usize;
        let hdr_sz = core::mem::size_of::<VirtioNetHeader>();
        if total_written <= hdr_sz {
            self.rearm_desc(used_id);
            self.counters.rx_bad_completion = self.counters.rx_bad_completion.saturating_add(1);
            return 0;
        }

        let frame_len = (total_written - hdr_sz).min(buf.len());
        let frame_src = (self.rx_buf_virt[buf_idx] as *const u8).add(hdr_sz);
        // SAFETY: copy from the correct RX buffer (device-written) into caller buffer.
        unsafe {
            core::ptr::copy_nonoverlapping(frame_src, buf.as_mut_ptr(), frame_len);
        }

        self.rearm_desc(used_id);
        self.counters.rx_delivered = self.counters.rx_delivered.saturating_add(1);
        self.counters.rx_bytes = self.counters.rx_bytes.saturating_add(frame_len as u64);
        frame_len
    }

    /// Re-arm (re-supply) the given RX descriptor id with its associated buffer.
    /// Called after consuming a used entry for that desc.
    fn rearm_desc(&mut self, desc_id: u32) {
        let idx = desc_id as usize;
        if idx >= MAX_RX_BUFFERS {
            return;
        }
        // SAFETY: pointers validated at init; we only re-arm a desc the device just
        // returned via the used ring.
        unsafe {
            let d = (self.rx.desc_virt as *mut VirtqDesc).add(idx);
            (*d).addr = self.rx_buf_phys[idx];
            (*d).len = (core::mem::size_of::<VirtioNetHeader>() + self.rx_buf_len) as u32;
            (*d).flags = DESC_F_WRITE;
            (*d).next = 0;

            let slot = (self.rx.avail_idx as usize) % (self.rx.queue_size as usize);
            let avail_ring_ptr = (self.rx.avail_virt + 4) as *mut u16;
            avail_ring_ptr.add(slot).write_volatile(desc_id as u16);

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
        self.counters.tx_submitted = self.counters.tx_submitted.saturating_add(1);
        self.counters.tx_bytes = self.counters.tx_bytes.saturating_add(frame.len() as u64);

        // Poll used ring (bounded) — MVI simple blocking TX
        // SAFETY: read from used ring + notify are device-visible via fences.
        unsafe {
            let used_idx_ptr = (self.tx.used_virt + 2) as *const u16;
            let mut limit = 50_000_000u32;
            loop {
                fence(Ordering::SeqCst);
                if used_idx_ptr.read_volatile() != self.tx.last_used_idx {
                    self.tx.last_used_idx = self.tx.last_used_idx.wrapping_add(1);
                    self.counters.tx_completed = self.counters.tx_completed.saturating_add(1);
                    break;
                }
                limit -= 1;
                if limit == 0 {
                    self.counters.tx_ring_full = self.counters.tx_ring_full.saturating_add(1);
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
