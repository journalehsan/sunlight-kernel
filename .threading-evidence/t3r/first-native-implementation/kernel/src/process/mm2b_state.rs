use core::sync::atomic::{AtomicU64, Ordering};

pub const MAX_TRACKED_CPUS: usize = 64;
pub const FULL_FLUSH_PAGES: u64 = u64::MAX;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AddressSpaceIdentity {
    pub pml4_phys: u64,
    pub generation: u64,
}

impl AddressSpaceIdentity {
    pub const INVALID: Self = Self {
        pml4_phys: 0,
        generation: 0,
    };

    pub const fn is_valid(self) -> bool {
        self.pml4_phys != 0 && self.generation != 0
    }
}

pub fn allocate_identity(next_generation: &AtomicU64, pml4_phys: u64) -> AddressSpaceIdentity {
    assert_ne!(pml4_phys, 0, "address-space root must be nonzero");
    let generation = next_generation
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
            current.checked_add(1)
        })
        .expect("address-space generation exhausted");
    assert_ne!(generation, 0, "address-space generation zero is reserved");
    AddressSpaceIdentity {
        pml4_phys,
        generation,
    }
}

struct ActiveCpuSlot {
    pml4_phys: AtomicU64,
    generation: AtomicU64,
}

impl ActiveCpuSlot {
    const fn new() -> Self {
        Self {
            pml4_phys: AtomicU64::new(0),
            generation: AtomicU64::new(0),
        }
    }

    fn enter(&self, identity: AddressSpaceIdentity) {
        debug_assert!(identity.is_valid());
        // Generation zero is the transition marker. Readers that overlap a
        // switch retry rather than combining fields from two identities.
        self.generation.store(0, Ordering::Release);
        self.pml4_phys.store(identity.pml4_phys, Ordering::Relaxed);
        self.generation
            .store(identity.generation, Ordering::Release);
    }

    fn leave(&self) {
        self.generation.store(0, Ordering::Release);
        self.pml4_phys.store(0, Ordering::Relaxed);
    }

    fn current(&self) -> (Option<AddressSpaceIdentity>, u64) {
        let mut retries = 0;
        loop {
            let generation = self.generation.load(Ordering::Acquire);
            if generation == 0 {
                return (None, retries);
            }
            let pml4_phys = self.pml4_phys.load(Ordering::Relaxed);
            if self.generation.load(Ordering::Acquire) == generation {
                return (
                    Some(AddressSpaceIdentity {
                        pml4_phys,
                        generation,
                    }),
                    retries,
                );
            }
            retries += 1;
            core::hint::spin_loop();
        }
    }
}

pub struct ActiveCpuSet {
    slots: [ActiveCpuSlot; MAX_TRACKED_CPUS],
}

impl ActiveCpuSet {
    pub const fn new() -> Self {
        Self {
            slots: [const { ActiveCpuSlot::new() }; MAX_TRACKED_CPUS],
        }
    }

    pub fn enter(&self, cpu_id: usize, identity: AddressSpaceIdentity) {
        assert!(cpu_id < MAX_TRACKED_CPUS, "active CPU index out of range");
        self.slots[cpu_id].enter(identity);
    }

    pub fn leave(&self, cpu_id: usize) {
        assert!(cpu_id < MAX_TRACKED_CPUS, "active CPU index out of range");
        self.slots[cpu_id].leave();
    }

    pub fn current(&self, cpu_id: usize) -> (Option<AddressSpaceIdentity>, u64) {
        assert!(cpu_id < MAX_TRACKED_CPUS, "active CPU index out of range");
        self.slots[cpu_id].current()
    }

    pub fn mask(&self, identity: AddressSpaceIdentity, online_mask: u64) -> (u64, u64) {
        let mut mask = 0;
        let mut retries = 0;
        for cpu_id in 0..MAX_TRACKED_CPUS {
            let bit = 1u64 << cpu_id;
            if online_mask & bit == 0 {
                continue;
            }
            let (current, slot_retries) = self.slots[cpu_id].current();
            retries += slot_retries;
            if current == Some(identity) {
                mask |= bit;
            }
        }
        (mask, retries)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MailboxRequest {
    pub sequence: u64,
    pub target: AddressSpaceIdentity,
    pub start: u64,
    pub pages: u64,
}

impl MailboxRequest {
    pub const fn is_full_flush(self) -> bool {
        self.pages == FULL_FLUSH_PAGES
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublishError {
    Busy,
    InvalidSequence,
}

pub struct ShootdownMailbox {
    request_sequence: AtomicU64,
    acknowledgement: AtomicU64,
    claimed_sequence: AtomicU64,
    target_pml4: AtomicU64,
    target_generation: AtomicU64,
    start: AtomicU64,
    pages: AtomicU64,
}

impl ShootdownMailbox {
    pub const fn new() -> Self {
        Self {
            request_sequence: AtomicU64::new(0),
            acknowledgement: AtomicU64::new(0),
            claimed_sequence: AtomicU64::new(0),
            target_pml4: AtomicU64::new(0),
            target_generation: AtomicU64::new(0),
            start: AtomicU64::new(0),
            pages: AtomicU64::new(0),
        }
    }

    pub fn try_publish(&self, request: MailboxRequest) -> Result<(), PublishError> {
        let previous = self.request_sequence.load(Ordering::Acquire);
        if self.acknowledgement.load(Ordering::Acquire) != previous {
            return Err(PublishError::Busy);
        }
        if request.sequence == 0 || request.sequence <= previous || !request.target.is_valid() {
            return Err(PublishError::InvalidSequence);
        }

        self.target_pml4
            .store(request.target.pml4_phys, Ordering::Relaxed);
        self.target_generation
            .store(request.target.generation, Ordering::Relaxed);
        self.start.store(request.start, Ordering::Relaxed);
        self.pages.store(request.pages, Ordering::Relaxed);
        self.claimed_sequence.store(0, Ordering::Relaxed);
        self.request_sequence
            .store(request.sequence, Ordering::Release);
        Ok(())
    }

    pub fn pending(&self) -> Option<MailboxRequest> {
        let sequence = self.request_sequence.load(Ordering::Acquire);
        if sequence == 0 || self.acknowledgement.load(Ordering::Acquire) == sequence {
            return None;
        }
        Some(MailboxRequest {
            sequence,
            target: AddressSpaceIdentity {
                pml4_phys: self.target_pml4.load(Ordering::Relaxed),
                generation: self.target_generation.load(Ordering::Relaxed),
            },
            start: self.start.load(Ordering::Relaxed),
            pages: self.pages.load(Ordering::Relaxed),
        })
    }

    pub fn try_claim(&self, sequence: u64) -> bool {
        self.claimed_sequence
            .compare_exchange(0, sequence, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    pub fn acknowledge(&self, sequence: u64) {
        self.acknowledgement.store(sequence, Ordering::Release);
    }

    pub fn acknowledged(&self, sequence: u64) -> bool {
        self.acknowledgement.load(Ordering::Acquire) == sequence
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destroyed_and_recreated_roots_have_distinct_identity() {
        let next = AtomicU64::new(1);
        let first = allocate_identity(&next, 0x4000);
        let recreated = allocate_identity(&next, 0x4000);
        assert_ne!(first, recreated);
        assert_eq!(first.pml4_phys, recreated.pml4_phys);
    }

    #[test]
    fn owner_and_borrower_share_identity() {
        let next = AtomicU64::new(1);
        let owner = allocate_identity(&next, 0x8000);
        let borrower = owner;
        assert_eq!(owner, borrower);
    }

    #[test]
    fn active_mask_tracks_switches_and_online_cpus() {
        let active = ActiveCpuSet::new();
        let identity = AddressSpaceIdentity {
            pml4_phys: 0x1000,
            generation: 9,
        };
        active.enter(0, identity);
        active.enter(3, identity);
        assert_eq!(active.mask(identity, u64::MAX).0, 0b1001);
        active.leave(0);
        assert_eq!(active.mask(identity, u64::MAX).0, 0b1000);
        assert_eq!(active.mask(identity, 0b0111).0, 0);
    }

    #[test]
    fn mailbox_rejects_overwrite_and_stale_sequence() {
        let mailbox = ShootdownMailbox::new();
        let target = AddressSpaceIdentity {
            pml4_phys: 0x2000,
            generation: 4,
        };
        let first = MailboxRequest {
            sequence: 1,
            target,
            start: 0x4000,
            pages: 1,
        };
        mailbox.try_publish(first).unwrap();
        assert_eq!(mailbox.pending(), Some(first));
        assert_eq!(
            mailbox.try_publish(MailboxRequest {
                sequence: 2,
                ..first
            }),
            Err(PublishError::Busy)
        );
        mailbox.acknowledge(1);
        mailbox
            .try_publish(MailboxRequest {
                sequence: 2,
                ..first
            })
            .unwrap();
        assert!(!mailbox.acknowledged(2));
        mailbox.acknowledge(2);
        assert_eq!(mailbox.pending(), None);
        assert_eq!(
            mailbox.try_publish(first),
            Err(PublishError::InvalidSequence)
        );
    }

    #[test]
    fn multiple_mailboxes_acknowledge_one_sequence_independently() {
        let mailboxes = [ShootdownMailbox::new(), ShootdownMailbox::new()];
        let request = MailboxRequest {
            sequence: 7,
            target: AddressSpaceIdentity {
                pml4_phys: 0x3000,
                generation: 5,
            },
            start: 0,
            pages: FULL_FLUSH_PAGES,
        };
        for mailbox in &mailboxes {
            mailbox.try_publish(request).unwrap();
        }
        mailboxes[0].acknowledge(7);
        assert!(mailboxes[0].acknowledged(7));
        assert!(!mailboxes[1].acknowledged(7));
        mailboxes[1].acknowledge(7);
        assert!(mailboxes.iter().all(|mailbox| mailbox.acknowledged(7)));
    }

    #[test]
    fn repeated_requests_are_neither_lost_nor_overwritten() {
        let mailbox = ShootdownMailbox::new();
        let target = AddressSpaceIdentity {
            pml4_phys: 0x9000,
            generation: 12,
        };
        for sequence in 1..=128 {
            let request = MailboxRequest {
                sequence,
                target,
                start: 0x20_0000 + sequence * 4096,
                pages: 1,
            };
            mailbox.try_publish(request).unwrap();
            assert_eq!(mailbox.pending(), Some(request));
            assert!(mailbox.try_claim(sequence));
            assert!(!mailbox.try_claim(sequence));
            mailbox.acknowledge(sequence);
            assert!(mailbox.acknowledged(sequence));
        }
    }
}
