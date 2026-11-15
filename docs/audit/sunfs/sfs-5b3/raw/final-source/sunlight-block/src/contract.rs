//! Native block metadata and synchronous receipts. Identity never fences DMA.
use core::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Knowledge<T> {
    Known(T),
    Unknown,
    Unsupported,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    VirtioLegacy,
    UsbBot,
    MemDisk,
}
/// Boot-local attachment identity, never a persistent media serial number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceIdentity {
    pub backend: Backend,
    pub incarnation: u64,
}
static NEXT_INCARNATION: AtomicU64 = AtomicU64::new(1);
pub fn new_incarnation() -> Option<u64> {
    NEXT_INCARNATION
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
        .ok()
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FenceCapability {
    Unsupported,
    TransportResetOnly,
    NotApplicable,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    pub identity: DeviceIdentity,
    pub persistent_identity: Knowledge<[u8; 20]>,
    pub logical_block: u32,
    pub physical_block: Knowledge<u32>,
    pub blocks: u64,
    pub max_blocks: u64,
    pub max_transfer: u64,
    /// Byte offset and length alignment. Caller pointer may be unaligned: bounce I/O.
    pub io_alignment: u32,
    pub buffer_alignment: u32,
    pub read_only: Knowledge<bool>,
    pub flush: Knowledge<bool>,
    pub fua: Knowledge<bool>,
    pub write_cache: Knowledge<bool>,
    pub fencing: FenceCapability,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rejection {
    Geometry,
    Overflow,
    Bounds,
    Alignment,
    TooLarge,
    ZeroLength,
    BufferLength,
    ReadOnly,
    Exhausted,
}
impl Geometry {
    pub fn validate(&self) -> Result<(), Rejection> {
        if self.identity.incarnation == 0
            || !self.logical_block.is_power_of_two()
            || !self.io_alignment.is_power_of_two()
            || !self.buffer_alignment.is_power_of_two()
            || self.blocks == 0
            || self.max_blocks == 0
            || self.max_transfer < u64::from(self.logical_block)
            || !self
                .max_transfer
                .is_multiple_of(u64::from(self.logical_block))
            || matches!(self.physical_block, Knowledge::Known(n) if !n.is_power_of_two() || n < self.logical_block)
        {
            return Err(Rejection::Geometry);
        }
        self.blocks
            .checked_mul(u64::from(self.logical_block))
            .ok_or(Rejection::Overflow)?;
        Ok(())
    }
    pub fn check(
        &self,
        lba: u64,
        count: u64,
        length: usize,
        address: usize,
    ) -> Result<u64, Rejection> {
        self.validate()?;
        if count == 0 {
            return Err(Rejection::ZeroLength);
        }
        let end = lba.checked_add(count).ok_or(Rejection::Overflow)?;
        let bytes = count
            .checked_mul(u64::from(self.logical_block))
            .ok_or(Rejection::Overflow)?;
        let offset = lba
            .checked_mul(u64::from(self.logical_block))
            .ok_or(Rejection::Overflow)?;
        if end > self.blocks {
            return Err(Rejection::Bounds);
        }
        if count > self.max_blocks || bytes > self.max_transfer {
            return Err(Rejection::TooLarge);
        }
        if bytes != length as u64 {
            return Err(Rejection::BufferLength);
        }
        if !offset.is_multiple_of(u64::from(self.io_alignment))
            || !bytes.is_multiple_of(u64::from(self.io_alignment))
            || !address.is_multiple_of(self.buffer_alignment as usize)
        {
            return Err(Rejection::Alignment);
        }
        Ok(bytes)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    Read,
    Write,
    Flush,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RequestId {
    pub device: DeviceIdentity,
    pub queue_incarnation: u64,
    /// Also the descriptor ownership epoch and serial submission sequence.
    pub generation: u64,
    pub head: u16,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ownership {
    Created,
    Prepared,
    Submitted,
    Completed,
    TimedOutUncertain,
    Quarantined,
    Retired,
}
/// Single outstanding request. No reset/fenced transitions exist without a fence.
#[derive(Debug)]
pub struct Owner {
    pub device: DeviceIdentity,
    pub queue_incarnation: u64,
    next: u64,
    active: Option<RequestId>,
    state: Ownership,
}
impl Owner {
    pub const fn new(device: DeviceIdentity) -> Self {
        Self {
            device,
            queue_incarnation: 1,
            next: 1,
            active: None,
            state: Ownership::Created,
        }
    }
    pub fn prepare(&mut self, head: u16) -> Result<RequestId, Rejection> {
        if self.active.is_some() || !matches!(self.state, Ownership::Created | Ownership::Retired) {
            return Err(Rejection::Exhausted);
        }
        let next = self.next.checked_add(1).ok_or(Rejection::Exhausted)?;
        let id = RequestId {
            device: self.device,
            queue_incarnation: self.queue_incarnation,
            generation: self.next,
            head,
        };
        self.next = next;
        self.active = Some(id);
        self.state = Ownership::Prepared;
        Ok(id)
    }
    pub fn submit(&mut self, id: RequestId) -> bool {
        if self.active != Some(id) || self.state != Ownership::Prepared {
            return false;
        }
        self.state = Ownership::Submitted;
        true
    }
    pub fn complete(&mut self, id: RequestId, resources_match: bool) -> bool {
        if self.active != Some(id)
            || id.device != self.device
            || id.queue_incarnation != self.queue_incarnation
            || self.state != Ownership::Submitted
            || !resources_match
        {
            return false;
        }
        self.state = Ownership::Completed;
        true
    }
    pub fn retire(&mut self, id: RequestId) -> bool {
        if self.active != Some(id) || self.state != Ownership::Completed {
            return false;
        }
        self.active = None;
        self.state = Ownership::Retired;
        true
    }
    pub fn quarantine(&mut self, timed_out: bool) {
        if timed_out {
            self.state = Ownership::TimedOutUncertain;
        }
        self.state = Ownership::Quarantined;
    }
    pub const fn state(&self) -> Ownership {
        self.state
    }
    pub const fn active(&self) -> Option<RequestId> {
        self.active
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Completion {
    Submitted,
    Completed,
    Partial,
    Failed,
    Uncertain,
    Unsupported,
    Unavailable,
    Rejected(Rejection),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Amount {
    Unknown,
    Exact(u64),
    AtLeast(u64),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusSource {
    Validation,
    Queue,
    DeviceStatus(u8),
    Deadline,
    Transport,
    VolatileMemory,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Durability {
    NotApplicable,
    NotEstablished,
    /// Virtual-device barrier, through this serialized queue generation only.
    VirtualBarrier {
        through_generation: u64,
    },
    Uncertain,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IoOutcome {
    pub device: DeviceIdentity,
    pub request: Option<RequestId>,
    pub operation: Operation,
    pub lba: u64,
    pub requested_blocks: u64,
    pub requested_bytes: u64,
    pub completed: Amount,
    pub completion: Completion,
    pub source: StatusSource,
    pub durability: Durability,
}
impl IoOutcome {
    /// Narrow legacy bridge. Ambiguity remains distinguishable and never succeeds.
    pub fn legacy(self) -> Result<(), crate::BlockError> {
        use crate::BlockError;
        if self.durability == Durability::Uncertain {
            return Err(BlockError::Uncertain);
        }
        match self.completion {
            Completion::Completed => Ok(()),
            Completion::Unsupported => Err(BlockError::Unsupported),
            Completion::Unavailable => Err(BlockError::Unavailable),
            Completion::Rejected(_) => Err(BlockError::OutOfRange),
            Completion::Uncertain | Completion::Submitted | Completion::Partial => {
                Err(BlockError::Uncertain)
            }
            Completion::Failed if self.durability == Durability::Uncertain => {
                Err(BlockError::Uncertain)
            }
            Completion::Failed => Err(BlockError::Failed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn geometry() -> Geometry {
        Geometry {
            identity: DeviceIdentity {
                backend: Backend::MemDisk,
                incarnation: 1,
            },
            persistent_identity: Knowledge::Unsupported,
            logical_block: 512,
            physical_block: Knowledge::Unknown,
            blocks: 64,
            max_blocks: 8,
            max_transfer: 4096,
            io_alignment: 512,
            buffer_alignment: 1,
            read_only: Knowledge::Known(false),
            flush: Knowledge::Unsupported,
            fua: Knowledge::Unsupported,
            write_cache: Knowledge::Unknown,
            fencing: FenceCapability::NotApplicable,
        }
    }
    #[test]
    fn t1_invalid_geometry() {
        let mut g = geometry();
        g.logical_block = 0;
        assert_eq!(g.validate(), Err(Rejection::Geometry));
        g = geometry();
        g.physical_block = Knowledge::Known(256);
        assert_eq!(g.validate(), Err(Rejection::Geometry));
    }
    #[test]
    fn t2_overflow() {
        assert_eq!(
            geometry().check(u64::MAX, 2, 1024, 0),
            Err(Rejection::Overflow)
        );
        let mut g = geometry();
        g.blocks = u64::MAX;
        assert_eq!(g.validate(), Err(Rejection::Overflow));
    }
    #[test]
    fn t3_alignment() {
        let mut g = geometry();
        g.io_alignment = 1024;
        assert_eq!(g.check(1, 1, 512, 0), Err(Rejection::Alignment));
        g = geometry();
        g.buffer_alignment = 8;
        assert_eq!(g.check(0, 1, 512, 1), Err(Rejection::Alignment));
    }
    #[test]
    fn t4_oversized_and_zero_and_buffer() {
        let g = geometry();
        assert_eq!(g.check(0, 9, 4608, 0), Err(Rejection::TooLarge));
        assert_eq!(g.check(0, 0, 0, 0), Err(Rejection::ZeroLength));
        assert_eq!(g.check(0, 1, 511, 0), Err(Rejection::BufferLength));
    }
    fn submitted() -> (Owner, RequestId) {
        let mut o = Owner::new(geometry().identity);
        let id = o.prepare(0).unwrap();
        assert!(o.submit(id));
        (o, id)
    }
    #[test]
    fn normal_exactly_once_retirement() {
        let (mut o, id) = submitted();
        assert!(o.complete(id, true));
        assert!(o.retire(id));
        assert!(!o.retire(id));
    }
    #[test]
    fn t11_late_and_t18_quarantine() {
        let (mut o, id) = submitted();
        o.quarantine(true);
        assert!(!o.complete(id, true));
        assert!(!o.retire(id));
        assert_eq!(o.active(), Some(id));
        assert_eq!(o.state(), Ownership::Quarantined);
        assert!(o.prepare(0).is_err());
    }
    #[test]
    fn t12_duplicate() {
        let (mut o, id) = submitted();
        assert!(o.complete(id, true));
        assert!(!o.complete(id, true));
        assert!(o.retire(id));
        assert!(!o.complete(id, true));
    }
    #[test]
    fn t13_wrong_queue() {
        let (mut o, mut id) = submitted();
        id.queue_incarnation += 1;
        assert!(!o.complete(id, true));
    }
    #[test]
    fn t14_wrong_media() {
        let (mut o, mut id) = submitted();
        id.device.incarnation += 1;
        assert!(!o.complete(id, true));
    }
    #[test]
    fn t16_wrong_resources() {
        let (mut o, id) = submitted();
        assert!(!o.complete(id, false));
        assert!(!o.retire(id));
    }
    #[test]
    fn t17_reuse_old_identity() {
        let (mut o, id) = submitted();
        assert!(o.prepare(0).is_err());
        assert!(o.complete(id, true));
        assert!(o.retire(id));
        let new = o.prepare(0).unwrap();
        assert_ne!(new, id);
        assert!(o.submit(new));
        assert!(!o.complete(id, true));
        assert_eq!(o.active(), Some(new));
    }
    #[test]
    fn t19_exhaustion() {
        let mut o = Owner::new(geometry().identity);
        o.next = u64::MAX;
        assert_eq!(o.prepare(0), Err(Rejection::Exhausted));
        assert_eq!(o.active(), None);
    }
    #[test]
    fn t20_removal_and_shutdown() {
        let (mut o, id) = submitted();
        o.quarantine(false);
        assert!(!o.retire(id));
        assert_eq!(o.active(), Some(id));
        assert!(o.prepare(0).is_err());
    }
}
