use super::pci::{inb, inl, inw, outb, outl, outw};
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

/// Work bound, not a wall-clock deadline or evidence of DMA termination.
pub const RESET_READ_BUDGET: u8 = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResetState {
    Running,
    Waiting,
    /// Status zero observed. All allocations remain pinned; no reuse authority.
    VerifiedRetained,
    TimedOutRetained,
    UnexpectedStatusRetained,
}

/// Diagnostic receipt only. It cannot authorize frame release or reinitialization.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResetObservation {
    pub device: DeviceIdentity,
    pub queue_incarnation: u64,
    pub outstanding: Option<RequestId>,
    pub state: ResetState,
    pub reads: u8,
    pub status: Option<u8>,
}

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
    offered_features: u32,
    accepted_features: u32,
    reset_state: ResetState,
    reset_reads: u8,
    reset_status: Option<u8>,
    // First uncertain result survives subsequent refusal/reset observations.
    first_uncertain: Option<IoOutcome>,
    // Metadata owned by the attachment across bounded staged polls.
    pending: Option<(Operation, u64, RequestId)>,
    // SFS-5B.5: callback lifetime and DMA-mapping authority for the staged
    // request.  Completion commits exactly one terminal; timeout/cancel only
    // request cancellation and therefore retain both the slot and the mapping.
    slot: Option<sunlight_block::quiesce::OpSlot>,
    mapping: Option<sunlight_block::quiesce::MappingRecord>,
    #[cfg(test)]
    injection: Option<(u16, u32, u32, u8, bool)>,
    #[cfg(test)]
    transport_status: u8,
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
        // VirtIO 1.4 2.4: a write is not reset completion. Do not publish queue
        // memory or advance initialization unless status zero is observed.
        let mut reset_complete = false;
        for _ in 0..RESET_READ_BUDGET {
            if inb(io_base + REG_DEVICE_STATUS) == 0 {
                reset_complete = true;
                break;
            }
        }
        if !reset_complete {
            return None;
        }
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
            offered_features: features,
            accepted_features,
            reset_state: ResetState::Running,
            reset_reads: 0,
            reset_status: None,
            first_uncertain: None,
            pending: None,
            slot: None,
            mapping: None,
            #[cfg(test)]
            injection: None,
            #[cfg(test)]
            transport_status: 7,
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

    pub const fn features(&self) -> (u32, u32) {
        (self.offered_features, self.accepted_features)
    }

    pub const fn first_uncertain(&self) -> Option<IoOutcome> {
        self.first_uncertain
    }

    /// SFS-5B.5 callback lifetime of the staged request, if one was admitted.
    pub fn callback_state(&self) -> Option<sunlight_block::quiesce::SlotState> {
        self.slot.as_ref().map(|s| s.state())
    }

    /// Authorised users of the staged request's DMA mapping.
    pub fn mapping_users(&self) -> Option<u32> {
        self.mapping.as_ref().map(|m| m.users())
    }

    pub fn mapping_retired(&self) -> Option<bool> {
        self.mapping.as_ref().map(|m| m.is_retired())
    }

    /// Bounded drain over the staged request's callback lifetime.  A retained
    /// (timed-out or cancelled) request yields `Bounded`, never `Complete`.
    pub fn drain(&self, polls: u32) -> sunlight_block::quiesce::DrainOutcome {
        match self.slot.as_ref() {
            Some(slot) => sunlight_block::quiesce::drain_bounded(&[slot], polls),
            None => sunlight_block::quiesce::DrainOutcome::Complete,
        }
    }

    pub fn reset_observation(&self) -> ResetObservation {
        ResetObservation {
            device: self.owner.device,
            queue_incarnation: self.owner.queue_incarnation,
            outstanding: self.owner.active(),
            state: self.reset_state,
            reads: self.reset_reads,
            status: self.reset_status,
        }
    }

    /// Close admission and request ONE whole-device reset. Repeated calls are
    /// idempotent. Exclusive mutable ownership serializes I/O/reset initiation.
    /// No wait, allocation, ring mutation or request retirement occurs here.
    ///
    /// # Safety
    /// Caller owns this entire PCI device, at ring 0, with all DMA frames pinned.
    /// Do not invoke through a stale attachment or on an unrelated/boot device.
    pub unsafe fn begin_terminal_reset(&mut self) -> ResetObservation {
        if self.reset_state == ResetState::Running {
            self.quarantine_terminal();
            self.reset_state = ResetState::Waiting;
            fence(Ordering::SeqCst);
            #[cfg(not(test))]
            outb(self.io_base + REG_DEVICE_STATUS, 0);
        }
        self.reset_observation()
    }

    fn observe_reset_status(&mut self, status: u8) {
        if self.reset_state != ResetState::Waiting {
            return;
        }
        self.reset_reads += 1; // bounded by RESET_READ_BUDGET, never wraps
        self.reset_status = Some(status);
        self.reset_state = if status == 0 {
            fence(Ordering::SeqCst);
            ResetState::VerifiedRetained
        } else if status != STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_DRIVER_OK {
            // Includes 0xff (possible removal), NEEDS_RESET and FAILED.
            ResetState::UnexpectedStatusRetained
        } else if self.reset_reads == RESET_READ_BUDGET {
            ResetState::TimedOutRetained
        } else {
            ResetState::Waiting
        };
    }

    /// One register read per call; no interrupt/timer/completion dependency.
    /// VirtIO 1.4 2.4.1/2.4.2, legacy PCI status layout 4.1.4.10.
    /// Acknowledgment never resolves write/flush durability or releases DMA.
    ///
    /// # Safety
    /// Same exclusive PCI device and pinned allocation requirements as begin.
    pub unsafe fn poll_terminal_reset(&mut self) -> ResetObservation {
        if self.reset_state == ResetState::Waiting {
            #[cfg(not(test))]
            let status = inb(self.io_base + REG_DEVICE_STATUS);
            #[cfg(test)]
            let status = STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_DRIVER_OK;
            self.observe_reset_status(status);
        }
        self.reset_observation()
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
        if self.first_uncertain.is_none()
            && (completion == Completion::Uncertain
                || (request.is_some() && durability == Durability::Uncertain))
        {
            self.first_uncertain = Some(result);
        }
        self.last_outcome = Some(result);
        result
    }
    /// Permanently close this attachment without claiming reset or a fence.
    /// All active metadata and DMA allocations remain owned and pinned.
    pub fn quarantine_terminal(&mut self) {
        if self.first_uncertain.is_none() {
            if let Some((operation, lba, id)) = self.pending {
                self.receipt(
                    operation,
                    lba,
                    Some(id),
                    Completion::Uncertain,
                    StatusSource::Queue,
                    Durability::Uncertain,
                );
            }
        }
        self.quarantine(false);
        if let Some(slot) = self.slot.as_ref() {
            slot.cancel();
        }
    }

    fn quarantine(&mut self, timed_out: bool) {
        self.uncertain = true;
        self.owner.quarantine(timed_out);
    }

    unsafe fn device_running(&self) -> bool {
        #[cfg(not(test))]
        let status = inb(self.io_base + REG_DEVICE_STATUS);
        #[cfg(test)]
        let status = self.transport_status;
        status == STATUS_ACKNOWLEDGE | STATUS_DRIVER | STATUS_DRIVER_OK
    }

    /// Safety: initialization's pinned coherent DMA allocation contract still holds.
    unsafe fn transfer(
        &mut self,
        operation: Operation,
        lba: u64,
        input: Option<&[u8; 512]>,
        output: Option<&mut [u8; 512]>,
    ) -> IoOutcome {
        self.transfer_observed(operation, lba, input, output, || ())
    }

    unsafe fn transfer_observed<T>(
        &mut self,
        operation: Operation,
        lba: u64,
        input: Option<&[u8; 512]>,
        output: Option<&mut [u8; 512]>,
        before_publish: impl FnOnce() -> T,
    ) -> IoOutcome {
        let (id, _observation) = match self.start_transfer(operation, lba, input, before_publish) {
            Ok(id) => id,
            Err(outcome) => return outcome,
        };
        let budget = if cfg!(test) { 16 } else { 50_000_000 };
        for _ in 0..budget {
            // Only the final successful observation consumes the caller's buffer.
            fence(Ordering::SeqCst);
            if ((self.used_virt + 2) as *const u16).read_volatile() != self.last_used_idx {
                return self
                    .poll_transfer(id, output)
                    .expect("exclusive active request");
            }
            core::hint::spin_loop();
        }
        self.timeout_transfer(id).expect("exclusive active timeout")
    }

    unsafe fn start_transfer<T>(
        &mut self,
        operation: Operation,
        lba: u64,
        input: Option<&[u8; 512]>,
        before_publish: impl FnOnce() -> T,
    ) -> Result<(RequestId, T), IoOutcome> {
        if self.uncertain {
            return Err(self.receipt(
                operation,
                lba,
                None,
                Completion::Unavailable,
                StatusSource::Queue,
                Durability::Uncertain,
            ));
        }
        // A staged request owns the ring even when a completion has arrived.
        // A competing admission has no authority to interpret its used entry.
        if self.pending.is_some() {
            return Err(self.receipt(
                operation,
                lba,
                None,
                Completion::Rejected(Rejection::Exhausted),
                StatusSource::Validation,
                Durability::NotEstablished,
            ));
        }
        let validation = if operation == Operation::Flush {
            self.geometry().validate()
        } else {
            self.geometry().check(lba, 1, 512, 0).map(|_| ())
        };
        if let Err(e) = validation {
            return Err(self.receipt(
                operation,
                lba,
                None,
                Completion::Rejected(e),
                StatusSource::Validation,
                Durability::NotEstablished,
            ));
        }
        if operation == Operation::Write && self.read_only {
            return Err(self.receipt(
                operation,
                lba,
                None,
                Completion::Rejected(Rejection::ReadOnly),
                StatusSource::Validation,
                Durability::NotEstablished,
            ));
        }
        if operation == Operation::Flush && !self.flush_supported {
            return Err(self.receipt(
                operation,
                lba,
                None,
                Completion::Unsupported,
                StatusSource::Validation,
                Durability::NotEstablished,
            ));
        }
        // Preflight before touching either DMA buffer or descriptors.
        if !self.device_running()
            || ((self.used_virt + 2) as *const u16).read_volatile() != self.last_used_idx
        {
            self.quarantine(false);
            return Err(self.receipt(
                operation,
                lba,
                None,
                Completion::Uncertain,
                StatusSource::Queue,
                Durability::Uncertain,
            ));
        }
        let id = match self.owner.prepare(0) {
            Ok(id) => id,
            Err(e) => {
                return Err(self.receipt(
                    operation,
                    lba,
                    None,
                    Completion::Rejected(e),
                    StatusSource::Validation,
                    Durability::NotEstablished,
                ))
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
            return Err(self.receipt(
                operation,
                lba,
                Some(id),
                Completion::Uncertain,
                StatusSource::Queue,
                Durability::Uncertain,
            ));
        }
        let avail_ring_ptr = (self.avail_virt + 4) as *mut u16;
        let slot = (self.avail_idx as usize) % (self.queue_size as usize);
        avail_ring_ptr.add(slot).write_volatile(0);
        fence(Ordering::SeqCst);

        let new_idx = self.avail_idx.wrapping_add(1);
        let observation = before_publish();
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
        self.pending = Some((operation, lba, id));
        // SFS-5B.5: bind the staged request to an explicit callback lifetime and
        // an independent DMA-mapping authority.  Acquisition precedes any use.
        let epoch = id.queue_incarnation;
        let slot =
            sunlight_block::quiesce::OpSlot::new(id.generation, epoch, id.device.incarnation);
        let mapping = sunlight_block::quiesce::MappingRecord::new(id.generation, epoch, 512);
        let _ = slot.begin(id.generation.max(1));
        let _ = mapping.acquire(epoch);
        self.slot = Some(slot);
        self.mapping = Some(mapping);
        Ok((id, observation))
    }

    /// Submit one write without waiting. Caller retains the pinned-DMA contract.
    /// Admission and all later steps require exclusive attachment ownership.
    ///
    /// # Safety
    /// Same ring-0, coherent pinned DMA requirements as `write_typed`.
    pub unsafe fn start_write(
        &mut self,
        lba: u64,
        data: &[u8; 512],
    ) -> Result<RequestId, IoOutcome> {
        self.start_transfer(Operation::Write, lba, Some(data), || ())
            .map(|(id, ())| id)
    }

    /// One completion observation, without waiting. A stale token cannot consume
    /// a ring entry or alter ownership. None means pending or a rejected token;
    /// callers must also check `outstanding()` and terminal state.
    ///
    /// # Safety
    /// Same pinned exclusive attachment contract as `start_write`.
    pub unsafe fn poll_write(&mut self, id: RequestId) -> Option<IoOutcome> {
        if self.pending != Some((Operation::Write, self.pending?.1, id)) {
            return None;
        }
        self.poll_transfer(id, None)
    }

    /// Quarantine a matching pending write. Never reads or modifies DMA memory.
    /// A stale or already terminal token has no authority.
    pub fn timeout_write(&mut self, id: RequestId) -> Option<IoOutcome> {
        if self.pending != Some((Operation::Write, self.pending?.1, id)) {
            return None;
        }
        self.timeout_transfer(id)
    }

    fn timeout_transfer(&mut self, id: RequestId) -> Option<IoOutcome> {
        let (operation, lba, active) = self.pending?;
        if active != id || self.uncertain || self.owner.active() != Some(id) {
            return None;
        }
        self.quarantine(true);
        // Timeout requests cancellation only.  It is not cancellation
        // completion, so the slot stays non-terminal and the mapping keeps its
        // authorised user: nothing may be retired from this path.
        if let Some(slot) = self.slot.as_ref() {
            slot.cancel();
        }
        Some(self.receipt(
            operation,
            lba,
            Some(id),
            Completion::Uncertain,
            StatusSource::Deadline,
            Durability::Uncertain,
        ))
    }

    unsafe fn poll_transfer(
        &mut self,
        id: RequestId,
        output: Option<&mut [u8; 512]>,
    ) -> Option<IoOutcome> {
        let (operation, lba, active) = self.pending?;
        if active != id || self.uncertain || self.owner.active() != Some(id) {
            return None;
        }
        let used_idx_ptr = (self.used_virt + 2) as *const u16;
        fence(Ordering::SeqCst);
        if used_idx_ptr.read_volatile() == self.last_used_idx {
            return None;
        }
        Some(self.finish_transfer(operation, lba, id, output))
    }

    unsafe fn finish_transfer(
        &mut self,
        operation: Operation,
        lba: u64,
        id: RequestId,
        output: Option<&mut [u8; 512]>,
    ) -> IoOutcome {
        let used_idx_ptr = (self.used_virt + 2) as *const u16;
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
        // Legacy used.len is diagnostic only (VirtIO 1.4 section 5.2.6.3).
        // It cannot authorize a partial prefix or short-read success.
        let status = ((self.req_virt + 528) as *const u8).read_volatile();
        let resources = self.descriptors_match(operation);
        if used_idx != self.last_used_idx.wrapping_add(1)
            || head != u32::from(id.head)
            || status > 2
            || !self.device_running()
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
        self.pending = None;
        if let Some(slot) = self.slot.as_ref() {
            slot.begin_completion();
            slot.finish(sunlight_block::quiesce::Terminal::Completed);
        }
        if let Some(mapping) = self.mapping.as_ref() {
            mapping.release();
            mapping.retire(id.queue_incarnation);
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

    /// Observes a synchronous read from immediately before avail-index publication
    /// until the driver returns its outcome. Preflight rejections do not invoke
    /// the observer. Dropping its return value does not certify DMA quiescence.
    ///
    /// # Safety
    /// Same requirements as read_typed; the observer must not unwind or reenter
    /// the device.
    pub unsafe fn read_observed<T>(
        &mut self,
        lba: u64,
        buf: &mut [u8; 512],
        before_publish: impl FnOnce() -> T,
    ) -> IoOutcome {
        self.transfer_observed(Operation::Read, lba, None, Some(buf), before_publish)
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
// Moving exclusive ownership does not move or release the pinned frames.
// Shared access requires external synchronization; this is not a Sync promise.
unsafe impl Send for VirtioBlk {}

#[cfg(test)]
mod ownership_tests {
    use super::*;

    #[test]
    fn observed_read_retires_on_failure_and_timeout() {
        use core::cell::RefCell;
        use sunlight_block::busy_time::BusyTime;

        struct Observation<'a>(&'a RefCell<BusyTime>);
        impl Drop for Observation<'_> {
            fn drop(&mut self) {
                self.0.borrow_mut().finish(17);
            }
        }

        for fail_status in [true, false] {
            let mut queue = Pages([0; QUEUE_PAGES * 4096]);
            let mut request = Pages([0; QUEUE_PAGES * 4096]);
            let mut disk = device(&mut queue, &mut request);
            if fail_status {
                disk.injection = Some((1, 0, 1, 1, false));
            }
            let busy = RefCell::new(BusyTime::new());
            let mut buffer = [0xa5; 512];
            let rejected = unsafe {
                disk.read_observed::<()>(8192, &mut buffer, || panic!("rejected before publication"))
            };
            assert!(rejected.legacy().is_err());
            assert_eq!(busy.borrow().snapshot(9), 0);
            let outcome = unsafe {
                disk.read_observed(1, &mut buffer, || {
                    busy.borrow_mut().begin(10);
                    assert_eq!(busy.borrow().snapshot(12), 2);
                    Observation(&busy)
                })
            };
            assert!(outcome.legacy().is_err());
            assert_eq!(disk.has_uncertain_io(), !fail_status);
            assert_eq!(buffer, [0xa5; 512]);
            assert_eq!(busy.borrow().snapshot(17), 7);
            assert_eq!(busy.borrow().snapshot(100), 7);
        }
    }

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
            offered_features: VIRTIO_BLK_F_FLUSH,
            accepted_features: VIRTIO_BLK_F_FLUSH,
            reset_state: ResetState::Running,
            reset_reads: 0,
            reset_status: None,
            first_uncertain: None,
            pending: None,
            slot: None,
            mapping: None,
            injection: None,
            transport_status: 7,
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
    fn btop21_read_deadline_orders() {
        extern crate std;
        for order in ["poll-first", "used-before-timeout", "used-after-timeout"] {
            let (mut queue, mut request) = fixture();
            let mut disk = device(&mut queue, &mut request);
            let mut output = [0x99; 512];
            unsafe {
                let (id, ()) = disk.start_transfer(Operation::Read, 1, None, || ()).unwrap();
                assert_eq!(disk.poll_transfer(id, Some(&mut output)), None);
                let complete = |disk: &VirtioBlk| {
                    let entry = (disk.used_virt + 4) as *mut u32;
                    entry.write_volatile(0);
                    entry.add(1).write_volatile(513);
                    ((disk.req_virt + 528) as *mut u8).write_volatile(0);
                    ((disk.used_virt + 2) as *mut u16).write_volatile(1);
                };
                if order != "used-after-timeout" {
                    complete(&disk);
                }
                if order == "poll-first" {
                    assert!(disk
                        .poll_transfer(id, Some(&mut output))
                        .unwrap()
                        .legacy()
                        .is_ok());
                    assert_eq!(disk.last_used_idx, 1);
                    assert_eq!(disk.mapping_retired(), Some(true));
                    assert_eq!(disk.timeout_transfer(id), None);
                    assert_eq!(disk.poll_transfer(id, None), None);
                    let (next, ()) = disk.start_transfer(Operation::Read, 2, None, || ()).unwrap();
                    assert_eq!(next.head, id.head);
                    assert_ne!(next, id);
                    assert_eq!(disk.poll_transfer(id, None), None);
                    assert_eq!(disk.timeout_transfer(id), None);
                    assert_eq!(disk.poll_transfer(next, None), None);
                    assert_eq!(disk.pending, Some((Operation::Read, 2, next)));
                    assert_eq!(disk.last_used_idx, 1);
                    assert_eq!(disk.mapping_users(), Some(1));
                    assert_eq!(disk.mapping_retired(), Some(false));
                    std::println!("{order}: consumed=1 mapping_retired=true old_token_rejected=true");
                    continue;
                }
                let timeout = disk.timeout_transfer(id).unwrap();
                assert_eq!(timeout.source, StatusSource::Deadline);
                assert!(timeout.legacy().is_err());
                if order == "used-after-timeout" {
                    complete(&disk);
                }
                let queue_before = queue.0;
                let request_before = request.0;
                assert_eq!(disk.poll_transfer(id, Some(&mut output)), None);
                assert_eq!(disk.timeout_transfer(id), None);
                assert!(disk.start_transfer(Operation::Read, 2, None, || ()).is_err());
                assert_eq!(output, [0x99; 512]);
                assert_eq!(disk.pending, Some((Operation::Read, 1, id)));
                assert_eq!(disk.last_used_idx, 0);
                assert_eq!(disk.mapping_users(), Some(1));
                assert_eq!(disk.mapping_retired(), Some(false));
                assert_eq!(disk.begin_terminal_reset().state, ResetState::Waiting);
                disk.observe_reset_status(0);
                assert_eq!(
                    disk.reset_observation().state,
                    ResetState::VerifiedRetained
                );
                assert_eq!(disk.first_uncertain(), Some(timeout));
                assert_eq!(disk.pending, Some((Operation::Read, 1, id)));
                assert_eq!(disk.mapping_users(), Some(1));
                assert_eq!(disk.mapping_retired(), Some(false));
                assert_eq!(disk.poll_transfer(id, None), None);
                assert!(queue.0 == queue_before && request.0 == request_before);
                std::println!("{order}: Deadline consumed=0 users=1 reset=VerifiedRetained mapping_retired=false");
            }
        }
    }

    #[test]
    fn btop21_invalid_completion_retires_only_bookkeeping() {
        extern crate std;
        let (mut queue, mut request) = fixture();
        let mut disk = device(&mut queue, &mut request);
        let mut output = [0x99; 512];
        disk.injection = Some((1, 1, 513, 0, false));
        let outcome = unsafe { disk.read_typed(1, &mut output) };
        assert_eq!(outcome.completion, Completion::Uncertain);
        assert_eq!(outcome.source, StatusSource::Queue);
        assert_eq!(disk.last_used_idx, 0);
        assert!(disk.has_uncertain_io());
        let id = outcome.request.unwrap();
        assert_eq!(disk.pending, Some((Operation::Read, 1, id)));
        assert_eq!(disk.mapping_users(), Some(1));
        assert_eq!(disk.mapping_retired(), Some(false));
        assert_eq!(
            disk.callback_state(),
            Some(sunlight_block::quiesce::SlotState::Active)
        );
        assert_eq!(outcome.completed, Amount::Unknown);
        assert_eq!(unsafe { disk.poll_transfer(id, None) }, None);
        assert_eq!(output, [0x99; 512]);
        std::println!("invalid head: Queue/Uncertain consumed=0 callback=Active mapping_retired=false");
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

    #[test]
    fn f4_head_only_duplicate_cannot_supply_new_status() {
        let (mut q, mut r) = fixture();
        let mut d = device(&mut q, &mut r);
        d.injection = Some((1, 0, 1, 0, false));
        assert_eq!(unsafe { d.flush_typed() }.completion, Completion::Completed);
        // A used entry alone does not overwrite B's freshly initialized status.
        d.injection = Some((1, 0, 1, 255, false));
        assert_eq!(unsafe { d.flush_typed() }.completion, Completion::Uncertain);
        assert_eq!(d.ownership(), Ownership::Quarantined);
        assert!(d.outstanding().is_some());
    }

    #[test]
    fn f9_f21_f22_reset_retains_uncertain_io_and_all_dma_bytes() {
        for operation in [Operation::Write, Operation::Flush] {
            let (mut q, mut r) = fixture();
            let mut d = device(&mut q, &mut r);
            let outcome = unsafe {
                if operation == Operation::Write {
                    d.write_typed(1, &[0x71; 512])
                } else {
                    d.flush_typed()
                }
            };
            let before_q = q.0;
            let before_r = r.0;
            assert_eq!(outcome.completion, Completion::Uncertain);
            let begin = unsafe { d.begin_terminal_reset() };
            assert_eq!(begin.state, ResetState::Waiting);
            assert_eq!(begin.outstanding, outcome.request);
            d.observe_reset_status(0);
            let done = d.reset_observation();
            assert_eq!(done.state, ResetState::VerifiedRetained);
            assert_eq!(done.reads, 1);
            assert_eq!(done.outstanding, outcome.request);
            assert_eq!(d.first_uncertain(), Some(outcome));
            assert_eq!(d.last_outcome(), Some(outcome));
            assert_eq!(d.ownership(), Ownership::Quarantined);
            assert_eq!(
                unsafe { d.flush_typed() }.completion,
                Completion::Unavailable
            );
            assert_eq!(d.first_uncertain(), Some(outcome));
            assert_eq!(outcome.durability, Durability::Uncertain);
            assert!(q.0 == before_q);
            assert!(r.0 == before_r);
        }
    }

    #[test]
    fn f10_delayed_ack_has_exact_read_bound() {
        let (mut q, mut r) = fixture();
        let mut d = device(&mut q, &mut r);
        unsafe { d.begin_terminal_reset() };
        for _ in 0..RESET_READ_BUDGET - 1 {
            d.observe_reset_status(7);
        }
        assert_eq!(d.reset_observation().state, ResetState::Waiting);
        d.observe_reset_status(0);
        assert_eq!(d.reset_observation().state, ResetState::VerifiedRetained);
        assert_eq!(d.reset_observation().reads, RESET_READ_BUDGET);
    }

    #[test]
    fn f11_withheld_ack_is_terminal_and_not_a_fence() {
        let (mut q, mut r) = fixture();
        let mut d = device(&mut q, &mut r);
        unsafe { d.begin_terminal_reset() };
        for _ in 0..RESET_READ_BUDGET {
            unsafe { d.poll_terminal_reset() };
        }
        let done = d.reset_observation();
        assert_eq!(done.state, ResetState::TimedOutRetained);
        assert_eq!(done.reads, RESET_READ_BUDGET);
        d.observe_reset_status(0); // a later observation cannot rewrite the fault
        assert_eq!(unsafe { d.begin_terminal_reset() }, done);
        assert_eq!(unsafe { d.poll_terminal_reset() }, done);
        assert_eq!(
            unsafe { d.write_typed(1, &[0; 512]) }.completion,
            Completion::Unavailable
        );
    }

    #[test]
    fn f13_f16_unexpected_or_removed_device_cannot_ack() {
        for status in [1, 3, 8, 0x40, 0x80, 0xff] {
            let (mut q, mut r) = fixture();
            let mut d = device(&mut q, &mut r);
            unsafe { d.begin_terminal_reset() };
            d.observe_reset_status(status);
            assert_eq!(
                d.reset_observation().state,
                ResetState::UnexpectedStatusRetained
            );
            assert_eq!(d.reset_observation().status, Some(status));
            d.observe_reset_status(0);
            assert_eq!(d.reset_observation().status, Some(status));
        }
    }

    #[test]
    fn f15_f17_f18_reset_competitors_and_pressure_preserve_first_owner() {
        let (mut q, mut r) = fixture();
        let mut d = device(&mut q, &mut r);
        let fault = unsafe { d.write_typed(1, &[0x62; 512]) };
        let owner = d.outstanding();
        let before_q = q.0;
        let before_r = r.0;
        let begin = unsafe { d.begin_terminal_reset() };
        for _ in 0..1024 {
            assert_eq!(unsafe { d.begin_terminal_reset() }, begin);
            assert_eq!(
                unsafe { d.write_typed(2, &[0x63; 512]) }.completion,
                Completion::Unavailable
            );
        }
        assert_eq!(d.outstanding(), owner);
        assert_eq!(d.first_uncertain(), Some(fault));
        assert!(q.0 == before_q);
        assert!(r.0 == before_r);
    }

    #[test]
    fn f3_f8_late_ring_write_and_stale_notification_do_not_reopen_reset() {
        let (mut q, mut r) = fixture();
        let mut d = device(&mut q, &mut r);
        let fault = unsafe { d.flush_typed() };
        unsafe { d.begin_terminal_reset() };
        // Device activity before acknowledgment, not after a conforming reset.
        unsafe {
            ((d.used_virt + 2) as *mut u16).write_volatile(1);
            ((d.req_virt + 528) as *mut u8).write_volatile(0);
        }
        d.observe_reset_status(0);
        let before = q.0;
        assert_eq!(
            unsafe { d.flush_typed() }.completion,
            Completion::Unavailable
        );
        assert_eq!(d.outstanding(), fault.request);
        assert!(q.0 == before);
    }

    #[test]
    fn f19_normal_retirement_then_reset_has_no_second_retirement() {
        let (mut q, mut r) = fixture();
        let mut d = device(&mut q, &mut r);
        d.injection = Some((1, 0, 1, 0, false));
        let done = unsafe { d.flush_typed() };
        assert_eq!(d.ownership(), Ownership::Retired);
        unsafe { d.begin_terminal_reset() };
        d.observe_reset_status(0);
        assert_eq!(d.outstanding(), None);
        assert!(!d.owner.retire(done.request.unwrap()));
        assert_eq!(d.reset_observation().queue_incarnation, 1);
    }

    #[test]
    fn f13_device_lost_running_state_blocks_dma_publication() {
        for status in [0, 3, 0x47, 0x87, 0xff] {
            let (mut q, mut r) = fixture();
            let mut d = device(&mut q, &mut r);
            d.transport_status = status;
            let before_q = q.0;
            let before_r = r.0;
            assert_eq!(
                unsafe { d.write_typed(1, &[0x73; 512]) }.completion,
                Completion::Uncertain
            );
            assert_eq!(d.ownership(), Ownership::Quarantined);
            assert!(q.0 == before_q);
            assert!(r.0 == before_r);
            assert_eq!(d.outstanding(), None);
        }
    }

    #[test]
    fn f21_f22_device_error_provenance_survives_reset_and_refusal() {
        for operation in [Operation::Write, Operation::Flush] {
            for status in [1, 2] {
                let (mut q, mut r) = fixture();
                let mut d = device(&mut q, &mut r);
                d.injection = Some((1, 0, 1, status, false));
                let original = unsafe {
                    if operation == Operation::Write {
                        d.write_typed(1, &[0x74; 512])
                    } else {
                        d.flush_typed()
                    }
                };
                assert_eq!(original.source, StatusSource::DeviceStatus(status));
                assert_eq!(original.durability, Durability::Uncertain);
                assert_eq!(d.first_uncertain(), Some(original));
                assert_eq!(d.outstanding(), None); // authoritative terminal completion
                unsafe { d.begin_terminal_reset() };
                d.observe_reset_status(0);
                assert_eq!(
                    unsafe { d.flush_typed() }.completion,
                    Completion::Unavailable
                );
                assert_eq!(d.first_uncertain(), Some(original));
            }
        }
    }
    #[test]
    fn staged_write_rejects_stale_tokens_and_retires_once() {
        let (mut q, mut r) = fixture();
        let mut d = device(&mut q, &mut r);
        let id = unsafe { d.start_write(63, &[0xb4; 512]) }.unwrap();
        assert_eq!(unsafe { d.poll_write(id) }, None);
        let mut stale = id;
        stale.generation += 1;
        let before_q = q.0;
        let before_r = r.0;
        assert_eq!(d.timeout_write(stale), None);
        assert_eq!(unsafe { d.poll_write(stale) }, None);
        assert!(q.0 == before_q && r.0 == before_r);
        unsafe {
            ((d.used_virt + 4) as *mut u32).write_volatile(0);
            ((d.used_virt + 8) as *mut u32).write_volatile(1);
            ((d.req_virt + 528) as *mut u8).write_volatile(0);
            ((d.used_virt + 2) as *mut u16).write_volatile(1);
        }
        assert_eq!(unsafe { d.poll_write(stale) }, None);
        assert_eq!(
            unsafe { d.poll_write(id) }.unwrap().completion,
            Completion::Completed
        );
        assert_eq!(unsafe { d.poll_write(id) }, None);
        assert_eq!(d.timeout_write(id), None);
        assert_eq!(d.ownership(), Ownership::Retired);
        // SFS-5B.5: the completion path committed exactly one terminal outcome
        // and retired the mapping authority exactly once.
        assert_eq!(
            d.callback_state(),
            Some(sunlight_block::quiesce::SlotState::Terminal(
                sunlight_block::quiesce::Terminal::Completed
            ))
        );
        assert_eq!(d.mapping_users(), Some(0));
        assert_eq!(d.mapping_retired(), Some(true));
        assert_eq!(d.drain(4), sunlight_block::quiesce::DrainOutcome::Complete);
    }

    #[test]
    fn staged_timeout_reset_never_consumes_late_ring_or_reopens() {
        let (mut q, mut r) = fixture();
        let mut d = device(&mut q, &mut r);
        let id = unsafe { d.start_write(63, &[0xb4; 512]) }.unwrap();
        // SFS-5B.5: admission binds both a callback lifetime and a mapping.
        assert_eq!(
            d.callback_state(),
            Some(sunlight_block::quiesce::SlotState::Active)
        );
        assert_eq!(d.mapping_users(), Some(1));
        assert_eq!(d.mapping_retired(), Some(false));
        let outcome = d.timeout_write(id).unwrap();
        assert_eq!(outcome.completion, Completion::Uncertain);
        // Timeout requested cancellation only: the request is still non-terminal
        // and the mapping keeps its authorised user, so nothing is retired.
        assert_eq!(
            d.callback_state(),
            Some(sunlight_block::quiesce::SlotState::Cancelling)
        );
        assert_eq!(d.mapping_users(), Some(1));
        assert_eq!(d.mapping_retired(), Some(false));
        assert_eq!(
            d.drain(64),
            sunlight_block::quiesce::DrainOutcome::Bounded { pending: 1 }
        );
        unsafe { d.begin_terminal_reset() };
        d.observe_reset_status(0);
        unsafe { ((d.used_virt + 2) as *mut u16).write_volatile(1) };
        let before_q = q.0;
        let before_r = r.0;
        assert_eq!(unsafe { d.poll_write(id) }, None);
        assert_eq!(d.timeout_write(id), None);
        assert_eq!(
            unsafe { d.start_write(64, &[0xcc; 512]) }
                .unwrap_err()
                .completion,
            Completion::Unavailable
        );
        assert!(q.0 == before_q && r.0 == before_r);
        assert_eq!(d.first_uncertain(), Some(outcome));
        assert_eq!(d.outstanding(), Some(id));
    }
    #[test]
    fn staged_busy_admission_and_cancel_preserve_live_dma_inventory() {
        let (mut q, mut r) = fixture();
        let mut d = device(&mut q, &mut r);
        let id = unsafe { d.start_write(63, &[0xb4; 512]) }.unwrap();
        let before_q = q.0;
        let before_r = r.0;
        assert!(unsafe { d.start_write(64, &[0xcc; 512]) }.is_err());
        assert!(q.0 == before_q && r.0 == before_r);
        d.quarantine_terminal();
        assert_eq!(d.ownership(), Ownership::Quarantined);
        assert_eq!(d.outstanding(), Some(id));
        assert_eq!(unsafe { d.poll_write(id) }, None);
        assert_eq!(
            unsafe { d.start_write(64, &[0xcc; 512]) }
                .unwrap_err()
                .completion,
            Completion::Unavailable
        );
        assert!(q.0 == before_q && r.0 == before_r);
        assert_eq!(d.reset_observation().state, ResetState::Running);
    }
    #[test]
    fn staged_reset_or_cancel_before_timeout_records_write_uncertainty() {
        for cancel in [false, true] {
            let (mut q, mut r) = fixture();
            let mut d = device(&mut q, &mut r);
            let id = unsafe { d.start_write(63, &[0xb4; 512]) }.unwrap();
            if cancel {
                d.quarantine_terminal();
            } else {
                unsafe { d.begin_terminal_reset() };
            }
            let uncertain = d
                .first_uncertain()
                .expect("outstanding reset/cancel must preserve provenance");
            assert_eq!(uncertain.request, Some(id));
            assert_eq!(uncertain.operation, Operation::Write);
            assert_eq!(uncertain.lba, 63);
            assert_eq!(uncertain.completion, Completion::Uncertain);
            assert_eq!(uncertain.durability, Durability::Uncertain);
            assert_eq!(uncertain.completed, Amount::Unknown);
            assert_eq!(d.outstanding(), Some(id));
        }
    }
    #[test]
    fn staged_second_admission_cannot_poison_ready_first_completion() {
        let (mut q, mut r) = fixture();
        let mut d = device(&mut q, &mut r);
        let id = unsafe { d.start_write(63, &[0xb4; 512]) }.unwrap();
        unsafe {
            ((d.used_virt + 4) as *mut u32).write_volatile(0);
            ((d.used_virt + 8) as *mut u32).write_volatile(1);
            ((d.req_virt + 528) as *mut u8).write_volatile(0);
            ((d.used_virt + 2) as *mut u16).write_volatile(1);
        }
        let before_q = q.0;
        let before_r = r.0;
        assert_eq!(
            unsafe { d.start_write(64, &[0xcc; 512]) }
                .unwrap_err()
                .completion,
            Completion::Rejected(Rejection::Exhausted)
        );
        assert!(q.0 == before_q && r.0 == before_r);
        assert_eq!(
            unsafe { d.poll_write(id) }.unwrap().completion,
            Completion::Completed
        );
        assert_eq!(d.first_uncertain(), None);
    }
}
