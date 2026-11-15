use super::pci::{inl, inw, outb, outl, outw};
use core::sync::atomic::{fence, Ordering};
use sunlight_block::contract::*;

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
    read_only: bool,
    // Permanent quarantine: no completion/fence API can safely reopen this queue.
    uncertain: bool,
    owner: Owner,
    last_outcome: Option<IoOutcome>,
    last_used_length: Option<u32>,
    #[cfg(test)]
    injection: Option<(u16, u32, u32, u8, bool)>,
}

fn queue_fits(size: u16) -> bool {
    let n = usize::from(size);
    let used = (n * 16 + 6 + n * 2 + 4095) & !4095;
    size >= 3 && size.is_power_of_two() && used + 6 + n * 8 <= QUEUE_PAGES * 4096
}

impl VirtioBlk {
    /// Initialize a legacy virtio-blk device.
    ///
    /// `queue_phys` / `queue_virt`: physically-contiguous 4-page region for virtqueue.
    /// `req_phys` / `req_virt`: 1-page region for the request buffer (header+data+status).
    ///
    /// SAFETY: All physical/virtual address pairs must be valid; caller holds ring-0 privilege.
    /// Queue and request frames must stay pinned for the device lifetime, including
    /// after timeout or dropping this handle. Dropping does not stop DMA.
    /// The caller must never reinitialize a live/quarantined attachment or reuse its frames.
    pub unsafe fn init(
        io_base: u16,
        queue_phys: u64,
        queue_virt: u64,
        req_phys: u64,
        req_virt: u64,
    ) -> Option<Self> {
        // No reinitialization of a live/quarantined queue is permitted by this API.
        if queue_phys & 4095 != 0 || req_phys & 4095 != 0 || queue_phys >> 12 > u32::MAX as u64 {
            return None;
        }
        let incarnation = new_incarnation()?;
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
        // Explicitly implemented: SIZE_MAX, RO, BLK_SIZE and FLUSH only.
        let accepted_features = features & ((1 << 1) | (1 << 5) | (1 << 6) | VIRTIO_BLK_F_FLUSH);
        outl(io_base + REG_DRIVER_FEATURES, accepted_features);

        // Legacy virtio-blk device-specific configuration starts at 0x14
        // when MSI-X is not in use. Capacity is expressed in 512-byte sectors.
        let mut capacity_sectors = None;
        for _ in 0..8 {
            let hi = inl(io_base + 0x18);
            let lo = inl(io_base + 0x14);
            let hi2 = inl(io_base + 0x18);
            let lo2 = inl(io_base + 0x14);
            if hi == hi2 && lo == lo2 {
                capacity_sectors = Some((hi as u64) << 32 | lo as u64);
                break;
            }
        }
        let capacity_sectors = capacity_sectors?;
        if capacity_sectors == 0 || capacity_sectors.checked_mul(512).is_none() {
            return None;
        }
        // This driver implements exactly one 512-byte data descriptor.
        if accepted_features & (1 << 6) != 0 && inl(io_base + 0x28) != 512 {
            return None;
        }
        if accepted_features & (1 << 1) != 0 {
            let limit = inl(io_base + 0x1c);
            if limit != 0 && limit < 512 {
                return None;
            }
        }

        // Select queue 0
        outw(io_base + REG_QUEUE_SEL, 0);
        let qsize = inw(io_base + REG_QUEUE_NUM);
        if !queue_fits(qsize) {
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
            read_only: features & (1 << 5) != 0,
            uncertain: false,
            owner: Owner::new(DeviceIdentity {
                backend: Backend::VirtioLegacy,
                incarnation,
            }),
            last_outcome: None,
            last_used_length: None,
            #[cfg(test)]
            injection: None,
        })
    }

    pub fn geometry(&self) -> Geometry {
        Geometry {
            identity: self.owner.device,
            persistent_identity: Knowledge::Unknown,
            logical_block: 512,
            physical_block: Knowledge::Unknown,
            blocks: self.capacity_sectors,
            max_blocks: 1,
            max_transfer: 512,
            io_alignment: 512,
            buffer_alignment: 1,
            read_only: Knowledge::Known(self.read_only),
            flush: Knowledge::Known(self.flush_supported),
            fua: Knowledge::Unsupported,
            write_cache: Knowledge::Unknown,
            fencing: FenceCapability::Unsupported,
        }
    }
    pub const fn last_outcome(&self) -> Option<IoOutcome> {
        self.last_outcome
    }
    pub const fn ownership(&self) -> Ownership {
        self.owner.state()
    }
    pub const fn outstanding(&self) -> Option<RequestId> {
        self.owner.active()
    }
    pub const fn last_used_length(&self) -> Option<u32> {
        self.last_used_length
    }

    fn receipt(
        &mut self,
        operation: Operation,
        lba: u64,
        request: Option<RequestId>,
        completion: Completion,
        source: StatusSource,
        durability: Durability,
    ) -> IoOutcome {
        let bytes = if operation == Operation::Flush {
            0
        } else {
            512
        };
        let result = IoOutcome {
            device: self.owner.device,
            request,
            operation,
            lba,
            requested_blocks: bytes / 512,
            requested_bytes: bytes,
            completed: if completion == Completion::Completed {
                Amount::Exact(bytes)
            } else if (matches!(completion, Completion::Rejected(_))
                || (completion == Completion::Unsupported && request.is_none()))
            {
                Amount::Exact(0)
            } else {
                Amount::Unknown
            },
            completion,
            source,
            durability,
        };
        self.last_outcome = Some(result);
        result
    }
    fn quarantine(&mut self, timed_out: bool) {
        self.uncertain = true;
        self.owner.quarantine(timed_out);
    }

    /// Safety: initialization's pinned coherent DMA allocation contract still holds.
    unsafe fn transfer(
        &mut self,
        operation: Operation,
        lba: u64,
        input: Option<&[u8; 512]>,
        output: Option<&mut [u8; 512]>,
    ) -> IoOutcome {
        if self.uncertain {
            return self.receipt(
                operation,
                lba,
                None,
                Completion::Unavailable,
                StatusSource::Queue,
                Durability::Uncertain,
            );
        }
        let validation = if operation == Operation::Flush {
            self.geometry().validate()
        } else {
            self.geometry().check(lba, 1, 512, 0).map(|_| ())
        };
        if let Err(e) = validation {
            return self.receipt(
                operation,
                lba,
                None,
                Completion::Rejected(e),
                StatusSource::Validation,
                Durability::NotEstablished,
            );
        }
        if operation == Operation::Write && self.read_only {
            return self.receipt(
                operation,
                lba,
                None,
                Completion::Rejected(Rejection::ReadOnly),
                StatusSource::Validation,
                Durability::NotEstablished,
            );
        }
        if operation == Operation::Flush && !self.flush_supported {
            return self.receipt(
                operation,
                lba,
                None,
                Completion::Unsupported,
                StatusSource::Validation,
                Durability::NotEstablished,
            );
        }
        // Preflight before touching either DMA buffer or descriptors.
        if ((self.used_virt + 2) as *const u16).read_volatile() != self.last_used_idx {
            self.quarantine(false);
            return self.receipt(
                operation,
                lba,
                None,
                Completion::Uncertain,
                StatusSource::Queue,
                Durability::Uncertain,
            );
        }
        let id = match self.owner.prepare(0) {
            Ok(id) => id,
            Err(e) => {
                return self.receipt(
                    operation,
                    lba,
                    None,
                    Completion::Rejected(e),
                    StatusSource::Validation,
                    Durability::NotEstablished,
                )
            }
        };
        if let Some(data) = input {
            core::ptr::copy_nonoverlapping(data.as_ptr(), (self.req_virt + 16) as *mut u8, 512);
        }
        let request_type = match operation {
            Operation::Read => VIRTIO_BLK_T_IN,
            Operation::Write => VIRTIO_BLK_T_OUT,
            Operation::Flush => VIRTIO_BLK_T_FLUSH,
        };
        let data_is_device_write = operation == Operation::Read;
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

        if !self.owner.submit(id) {
            self.quarantine(false);
            return self.receipt(
                operation,
                lba,
                Some(id),
                Completion::Uncertain,
                StatusSource::Queue,
                Durability::Uncertain,
            );
        }
        let avail_ring_ptr = (self.avail_virt + 4) as *mut u16;
        let slot = (self.avail_idx as usize) % (self.queue_size as usize);
        avail_ring_ptr.add(slot).write_volatile(0);
        fence(Ordering::SeqCst);

        let new_idx = self.avail_idx.wrapping_add(1);
        ((self.avail_virt + 2) as *mut u16).write_volatile(new_idx);
        self.avail_idx = new_idx;
        fence(Ordering::SeqCst);
        #[cfg(not(test))]
        outw(self.io_base + REG_QUEUE_NOTIFY, 0);

        #[cfg(test)]
        if let Some((delta, head, length, status, corrupt)) = self.injection.take() {
            let slot = self.last_used_idx as usize % self.queue_size as usize;
            ((self.used_virt + 4) as *mut u32)
                .add(slot * 2)
                .write_volatile(head);
            ((self.used_virt + 4) as *mut u32)
                .add(slot * 2 + 1)
                .write_volatile(length);
            status_ptr.write_volatile(status);
            if corrupt {
                (*(self.desc_virt as *mut VirtqDesc)).addr ^= 4096;
            }
            ((self.used_virt + 2) as *mut u16)
                .write_volatile(self.last_used_idx.wrapping_add(delta));
        }
        let used_idx_ptr = (self.used_virt + 2) as *const u16;
        let mut limit = if cfg!(test) { 16u32 } else { 50_000_000u32 };
        loop {
            fence(Ordering::SeqCst);
            if used_idx_ptr.read_volatile() != self.last_used_idx {
                break;
            }
            limit -= 1;
            if limit == 0 {
                self.quarantine(true);
                return self.receipt(
                    operation,
                    lba,
                    Some(id),
                    Completion::Uncertain,
                    StatusSource::Deadline,
                    Durability::Uncertain,
                );
            }
            core::hint::spin_loop();
        }
        let used_idx = used_idx_ptr.read_volatile();
        fence(Ordering::SeqCst);
        let used_slot = self.last_used_idx as usize % self.queue_size as usize;
        let head = ((self.used_virt + 4) as *const u32)
            .add(used_slot * 2)
            .read_volatile();
        let length = ((self.used_virt + 4) as *const u32)
            .add(used_slot * 2 + 1)
            .read_volatile();
        self.last_used_length = Some(length);
        // Legacy used.len is diagnostic only (VirtIO 1.2 section 5.2.6.2).
        // It cannot authorize a partial prefix or short-read success.
        let status = status_ptr.read_volatile();
        let resources = self.descriptors_match(operation);
        if used_idx != self.last_used_idx.wrapping_add(1)
            || head != u32::from(id.head)
            || status > 2
            || !self.owner.complete(id, resources)
        {
            self.quarantine(false);
            return self.receipt(
                operation,
                lba,
                Some(id),
                Completion::Uncertain,
                StatusSource::Queue,
                Durability::Uncertain,
            );
        }
        self.last_used_idx = used_idx;
        if !self.owner.retire(id) {
            self.quarantine(false);
            return self.receipt(
                operation,
                lba,
                Some(id),
                Completion::Uncertain,
                StatusSource::Queue,
                Durability::Uncertain,
            );
        }
        if status == VIRTIO_BLK_S_OK {
            if let Some(buf) = output {
                core::ptr::copy_nonoverlapping(
                    (self.req_virt + 16) as *const u8,
                    buf.as_mut_ptr(),
                    512,
                );
            }
            self.receipt(
                operation,
                lba,
                Some(id),
                Completion::Completed,
                StatusSource::DeviceStatus(status),
                if operation == Operation::Flush {
                    Durability::VirtualBarrier {
                        through_generation: id.generation - 1,
                    }
                } else if operation == Operation::Read {
                    Durability::NotApplicable
                } else {
                    Durability::NotEstablished
                },
            )
        } else {
            self.receipt(
                operation,
                lba,
                Some(id),
                if status == 2 {
                    Completion::Unsupported
                } else {
                    Completion::Failed
                },
                StatusSource::DeviceStatus(status),
                if operation == Operation::Read {
                    Durability::NotApplicable
                } else {
                    Durability::Uncertain
                },
            )
        }
    }

    unsafe fn descriptors_match(&self, operation: Operation) -> bool {
        let expected = [
            (self.req_phys, 16, DESC_F_NEXT, 1),
            if operation == Operation::Flush {
                (self.req_phys + 528, 1, DESC_F_WRITE, 0)
            } else {
                (
                    self.req_phys + 16,
                    512,
                    DESC_F_NEXT
                        | if operation == Operation::Read {
                            DESC_F_WRITE
                        } else {
                            0
                        },
                    2,
                )
            },
            (self.req_phys + 528, 1, DESC_F_WRITE, 0),
        ];
        for (index, (addr, len, flags, next)) in expected
            .iter()
            .enumerate()
            .take(if operation == Operation::Flush { 2 } else { 3 })
        {
            let d = (self.desc_virt as *const VirtqDesc).add(index);
            if core::ptr::addr_of!((*d).addr).read_volatile() != *addr {
                return false;
            }
            if core::ptr::addr_of!((*d).len).read_volatile() != *len
                || core::ptr::addr_of!((*d).flags).read_volatile() != *flags
                || core::ptr::addr_of!((*d).next).read_volatile() != *next
            {
                return false;
            }
        }
        true
    }
    /// Safety: same pinned-buffer and exclusive-access requirements as init.
    pub unsafe fn read_typed(&mut self, lba: u64, buf: &mut [u8; 512]) -> IoOutcome {
        self.transfer(Operation::Read, lba, None, Some(buf))
    }
    /// Completion is visibility only; a failed write may have side effects.
    /// Safety: same pinned-buffer and exclusive-access requirements as init.
    pub unsafe fn write_typed(&mut self, lba: u64, buf: &[u8; 512]) -> IoOutcome {
        self.transfer(Operation::Write, lba, Some(buf), None)
    }
    /// Real negotiated command; successful receipt covers this virtual queue only.
    /// Safety: same pinned-buffer and exclusive-access requirements as init.
    pub unsafe fn flush_typed(&mut self) -> IoOutcome {
        self.transfer(Operation::Flush, 0, None, None)
    }
    /// Legacy diagnostic bridge; use typed receipts when handling errors.
    /// Safety: same requirements as init.
    pub unsafe fn read_block(&mut self, lba: u64, buf: &mut [u8; 512]) -> bool {
        self.read_typed(lba, buf).legacy().is_ok()
    }
    /// Safety: same requirements as init.
    pub unsafe fn write_block(&mut self, lba: u64, buf: &[u8; 512]) -> bool {
        self.write_typed(lba, buf).legacy().is_ok()
    }
    /// Safety: same requirements as init.
    pub unsafe fn flush(&mut self) -> bool {
        self.flush_typed().legacy().is_ok()
    }

    pub const fn block_count(&self) -> u64 {
        self.capacity_sectors
    }

    /// A timeout/malformed completion retains DMA ownership permanently.
    /// The caller must keep both allocations pinned; this is not a DMA fence.
    pub const fn has_uncertain_io(&self) -> bool {
        self.uncertain
    }

    pub const fn supports_flush(&self) -> bool {
        self.flush_supported
    }

    pub const fn supports_write(&self) -> bool {
        !self.read_only
    }
}

// SAFETY: VirtioBlk wraps raw pointers into kernel-owned physical frames.
// The kernel is single-threaded during initialization.
unsafe impl Send for VirtioBlk {}

#[cfg(test)]
mod ownership_tests {
    use super::*;

    #[repr(C, align(4096))]
    struct Pages([u8; QUEUE_PAGES * 4096]);

    fn device(queue: &mut Pages, request: &mut Pages) -> VirtioBlk {
        let q = queue.0.as_mut_ptr() as u64;
        let r = request.0.as_mut_ptr() as u64;
        VirtioBlk {
            io_base: 0,
            queue_size: 8,
            desc_virt: q,
            avail_virt: q + 128,
            used_virt: q + 4096,
            req_phys: r,
            req_virt: r,
            avail_idx: 0,
            last_used_idx: 0,
            capacity_sectors: 8192,
            flush_supported: true,
            read_only: false,
            uncertain: false,
            owner: Owner::new(DeviceIdentity {
                backend: Backend::VirtioLegacy,
                incarnation: 1,
            }),
            last_outcome: None,
            last_used_length: None,
            injection: None,
        }
    }

    #[test]
    fn timeout_must_not_reuse_dma_page_or_descriptors() {
        let mut q = Pages([0; QUEUE_PAGES * 4096]);
        let mut r = Pages([0; QUEUE_PAGES * 4096]);
        let mut d = device(&mut q, &mut r);
        unsafe {
            assert!(!d.write_block(1, &[0x41; 512]));
            let before_q = q.0;
            let before_r = r.0;
            assert!(!d.write_block(2, &[0x42; 512]));
            assert!(r.0 == before_r, "timed-out DMA request was overwritten");
            assert!(q.0 == before_q, "timed-out descriptors were republished");
        }
    }

    #[test]
    fn late_completion_must_not_authorize_new_request() {
        let mut q = Pages([0; QUEUE_PAGES * 4096]);
        let mut r = Pages([0; QUEUE_PAGES * 4096]);
        let mut d = device(&mut q, &mut r);
        unsafe {
            assert!(!d.write_block(1, &[0x41; 512]));
            // Device finishes the old head after the CPU has timed out.
            ((d.used_virt + 2) as *mut u16).write_volatile(1);
            ((d.req_virt + 528) as *mut u8).write_volatile(0);
            let before_q = q.0;
            let before_r = r.0;
            let mut output = [0x99; 512];
            assert!(!d.read_block(2, &mut output));
            assert_eq!(output, [0x99; 512]);
            assert!(q.0 == before_q);
            assert!(r.0 == before_r);
        }
    }
    fn fixture() -> (Pages, Pages) {
        (
            Pages([0; QUEUE_PAGES * 4096]),
            Pages([0; QUEUE_PAGES * 4096]),
        )
    }
    #[test]
    fn queue_allocation_bounds() {
        assert!(!queue_fits(0));
        assert!(!queue_fits(2));
        assert!(!queue_fits(7));
        assert!(queue_fits(8));
        assert!(queue_fits(256));
        assert!(!queue_fits(512));
        assert!(!queue_fits(32768));
    }
    #[test]
    fn valid_completion_and_flush_provenance() {
        let (mut q, mut r) = fixture();
        let mut d = device(&mut q, &mut r);
        d.injection = Some((1, 0, 1, 0, false));
        let write = unsafe { d.write_typed(1, &[0x33; 512]) };
        assert_eq!(write.completion, Completion::Completed);
        assert_eq!(write.completed, Amount::Exact(512));
        assert_eq!(write.durability, Durability::NotEstablished);
        assert_eq!(d.ownership(), Ownership::Retired);
        d.injection = Some((1, 0, 1, 0, false));
        let flush = unsafe { d.flush_typed() };
        assert_eq!(
            flush.durability,
            Durability::VirtualBarrier {
                through_generation: write.request.unwrap().generation
            }
        );
        assert_eq!(flush.source, StatusSource::DeviceStatus(0));
    }
    #[test]
    fn t5_legacy_length_not_authoritative() {
        let (mut q, mut r) = fixture();
        let mut d = device(&mut q, &mut r);
        d.injection = Some((1, 0, 0, 0, false));
        let result = unsafe { d.read_typed(0, &mut [0; 512]) };
        // Specification's legacy exception: len cannot prove a short byte count.
        assert_eq!(d.last_used_length(), Some(0));
        assert_eq!(result.completed, Amount::Exact(512));
        assert_eq!(result.source, StatusSource::DeviceStatus(0));
    }
    #[test]
    fn t6_failed_write_and_read_do_not_invent_prefix() {
        let (mut q, mut r) = fixture();
        let mut d = device(&mut q, &mut r);
        d.injection = Some((1, 0, 1, 1, false));
        let result = unsafe { d.write_typed(0, &[9; 512]) };
        assert_eq!(result.completion, Completion::Failed);
        assert_eq!(result.completed, Amount::Unknown);
        assert_eq!(result.durability, Durability::Uncertain);
        assert_eq!(result.legacy(), Err(sunlight_block::BlockError::Uncertain));
        d.injection = Some((1, 0, 1, 1, false));
        let mut output = [0x91; 512];
        let result = unsafe { d.read_typed(0, &mut output) };
        assert_eq!(result.completion, Completion::Failed);
        assert_eq!(output, [0x91; 512]);
        assert_eq!(d.ownership(), Ownership::Retired);
    }
    #[test]
    fn t7_uncertain_write_and_retention() {
        let (mut q, mut r) = fixture();
        let mut d = device(&mut q, &mut r);
        let result = unsafe { d.write_typed(0, &[1; 512]) };
        assert_eq!(result.completion, Completion::Uncertain);
        assert_eq!(result.source, StatusSource::Deadline);
        assert_eq!(d.ownership(), Ownership::Quarantined);
        assert_eq!(d.outstanding(), result.request);
        let before = r.0;
        let next = unsafe { d.write_typed(1, &[2; 512]) };
        assert_eq!(next.legacy(), Err(sunlight_block::BlockError::Uncertain));
        assert!(r.0 == before);
    }
    #[test]
    fn t8_unsupported_flush_not_submitted() {
        let (mut q, mut r) = fixture();
        let mut d = device(&mut q, &mut r);
        d.flush_supported = false;
        let before = q.0;
        let result = unsafe { d.flush_typed() };
        assert_eq!(result.completion, Completion::Unsupported);
        assert_eq!(result.request, None);
        assert_eq!(result.completed, Amount::Exact(0));
        assert!(q.0 == before);
    }
    #[test]
    fn t9_failed_flush_preserves_write_uncertainty() {
        let (mut q, mut r) = fixture();
        let mut d = device(&mut q, &mut r);
        d.injection = Some((1, 0, 1, 1, false));
        let result = unsafe { d.flush_typed() };
        assert_eq!(result.completion, Completion::Failed);
        assert_eq!(result.durability, Durability::Uncertain);
        assert_eq!(result.completed, Amount::Unknown);
        assert_eq!(d.ownership(), Ownership::Retired);
    }
    #[test]
    fn t10_uncertain_flush_quarantines() {
        let (mut q, mut r) = fixture();
        let mut d = device(&mut q, &mut r);
        let result = unsafe { d.flush_typed() };
        assert_eq!(result.completion, Completion::Uncertain);
        assert_eq!(result.durability, Durability::Uncertain);
        assert_eq!(d.ownership(), Ownership::Quarantined);
    }
    #[test]
    fn t15_invalid_used_head_and_index() {
        for (delta, head) in [(1, 99), (2, 0)] {
            let (mut q, mut r) = fixture();
            let mut d = device(&mut q, &mut r);
            d.injection = Some((delta, head, 1, 0, false));
            let result = unsafe { d.flush_typed() };
            assert_eq!(result.completion, Completion::Uncertain);
            assert_eq!(d.ownership(), Ownership::Quarantined);
            assert!(d.outstanding().is_some());
        }
    }
    #[test]
    fn t16_descriptor_ownership_and_bad_status() {
        for (status, corrupt) in [(0, true), (255, false)] {
            let (mut q, mut r) = fixture();
            let mut d = device(&mut q, &mut r);
            d.injection = Some((1, 0, 1, status, corrupt));
            let result = unsafe { d.flush_typed() };
            assert_eq!(result.completion, Completion::Uncertain);
            assert_eq!(d.ownership(), Ownership::Quarantined);
        }
    }
    #[test]
    fn duplicate_before_submission_freezes_before_buffer_copy() {
        let (mut q, mut r) = fixture();
        let mut d = device(&mut q, &mut r);
        unsafe {
            ((d.used_virt + 2) as *mut u16).write_volatile(1);
        }
        let before = r.0;
        let result = unsafe { d.write_typed(0, &[2; 512]) };
        assert_eq!(result.completion, Completion::Uncertain);
        assert!(r.0 == before);
        assert_eq!(d.avail_idx, 0);
    }
    #[test]
    fn reused_wire_head_has_no_generation_echo_demonstrated_limit() {
        let (mut q, mut r) = fixture();
        let mut d = device(&mut q, &mut r);
        d.injection = Some((1, 0, 1, 0, false));
        let old = unsafe { d.flush_typed() };
        d.injection = Some((1, 0, 1, 0, false));
        // A forged duplicate arriving after publication is wire-identical to a new completion.
        let new = unsafe { d.flush_typed() };
        assert_ne!(old.request, new.request);
        assert_eq!(new.completion, Completion::Completed); // G4 remains PARTIAL.
    }
    #[test]
    fn ring_index_wrap_is_separate_from_generation_exhaustion() {
        let (mut q, mut r) = fixture();
        let mut d = device(&mut q, &mut r);
        d.avail_idx = u16::MAX;
        d.last_used_idx = u16::MAX;
        unsafe {
            ((d.used_virt + 2) as *mut u16).write_volatile(u16::MAX);
        }
        d.injection = Some((1, 0, 1, 0, false));
        let result = unsafe { d.flush_typed() };
        assert_eq!(result.completion, Completion::Completed);
        assert_eq!(d.avail_idx, 0);
        assert_eq!(d.last_used_idx, 0);
        assert_eq!(result.request.unwrap().generation, 1);
    }
}
