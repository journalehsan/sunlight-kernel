use crate::memory::pmm::PhysicalMemoryManager;
use crate::process::mm2b_state::{allocate_identity, AddressSpaceIdentity};
use crate::process::region::{
    LedgerError, MappingKind, MappingRegion, RangeLookup, RegionBacking, RegionLedger,
    RegionPolicy, RegionProtection, RegionReservation, UnmapPlan,
};
use core::sync::atomic::{AtomicU64, Ordering};
use x86_64::{
    structures::paging::{
        page_table::PageTableEntry, Page, PageTable, PageTableFlags, PhysFrame, Size4KiB,
    },
    PhysAddr, VirtAddr,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MappingError {
    InvalidAddress,
    NonCanonical,
    Overflow,
    Misaligned,
    AlreadyMapped,
    NotMapped,
    FrameAllocationFailed,
    PageTableAllocationFailed,
    PermissionRejected,
    ProtectedRegion,
    UnsupportedReplacement,
    LedgerCapacityExhausted,
    LedgerOverlap,
    LedgerUnavailable,
    InternalInvariant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpectedMapping {
    Present {
        frame: PhysAddr,
        flags: PageTableFlags,
    },
    Swapped {
        block_id: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplacementMapping {
    Present {
        frame: PhysFrame<Size4KiB>,
        flags: PageTableFlags,
    },
    Swapped {
        block_id: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnershipTransition {
    RetainOld,
    ReleaseOldFrame,
}

static MAPPING_COLLISIONS: AtomicU64 = AtomicU64::new(0);
static MMAP_ROLLBACKS: AtomicU64 = AtomicU64::new(0);
static SHM_CREATE_ROLLBACKS: AtomicU64 = AtomicU64::new(0);
static SHM_PEER_ROLLBACKS: AtomicU64 = AtomicU64::new(0);
static FRAME_ALLOCATION_FAILURES: AtomicU64 = AtomicU64::new(0);
static PAGE_TABLE_ALLOCATION_FAILURES: AtomicU64 = AtomicU64::new(0);
static INTENTIONAL_REPLACEMENTS: AtomicU64 = AtomicU64::new(0);
static ROLLBACK_INVARIANT_FAILURES: AtomicU64 = AtomicU64::new(0);
static NEXT_ADDRESS_SPACE_GENERATION: AtomicU64 = AtomicU64::new(1);

static REGION_INSERTIONS: AtomicU64 = AtomicU64::new(0);
static REGION_ADJACENT_MERGES: AtomicU64 = AtomicU64::new(0);
static REGION_OVERLAP_REJECTIONS: AtomicU64 = AtomicU64::new(0);
static REGION_CAPACITY_FAILURES: AtomicU64 = AtomicU64::new(0);
static REGION_ROLLBACK_REMOVALS: AtomicU64 = AtomicU64::new(0);
static REGION_TEARDOWN_REMOVALS: AtomicU64 = AtomicU64::new(0);
static REGION_PTE_CONSISTENCY_FAILURES: AtomicU64 = AtomicU64::new(0);
static BORROWER_LEDGER_LOOKUPS: AtomicU64 = AtomicU64::new(0);
static STALE_LEDGER_LOOKUPS: AtomicU64 = AtomicU64::new(0);

/// Matches the existing bounded telemetry process table. Borrowers consume no
/// slot because they retain the owner's checked handle.
pub const MAX_ADDRESS_SPACE_LEDGERS: usize = 64;
const SHARED_REGION_BASE: u64 = 0x0000_0003_0000_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LedgerHandle {
    slot: u8,
    identity: AddressSpaceIdentity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SharedAddressSpaceHandle {
    pml4_phys: PhysAddr,
    identity: AddressSpaceIdentity,
    ledger: LedgerHandle,
}

struct LedgerSlot {
    identity: AddressSpaceIdentity,
    shared_bump: u64,
    ledger: RegionLedger,
}

impl LedgerSlot {
    const fn empty() -> Self {
        Self {
            identity: AddressSpaceIdentity::INVALID,
            shared_bump: SHARED_REGION_BASE,
            ledger: RegionLedger::new(),
        }
    }
}

static REGION_LEDGERS: spin::Mutex<[LedgerSlot; MAX_ADDRESS_SPACE_LEDGERS]> =
    spin::Mutex::new([const { LedgerSlot::empty() }; MAX_ADDRESS_SPACE_LEDGERS]);

pub fn note_mmap_rollback() {
    MMAP_ROLLBACKS.fetch_add(1, Ordering::Relaxed);
}

pub fn note_mapping_collision() {
    MAPPING_COLLISIONS.fetch_add(1, Ordering::Relaxed);
}

pub fn note_shm_create_rollback() {
    SHM_CREATE_ROLLBACKS.fetch_add(1, Ordering::Relaxed);
}

pub fn note_shm_peer_rollback() {
    SHM_PEER_ROLLBACKS.fetch_add(1, Ordering::Relaxed);
}

pub fn note_frame_allocation_failure() {
    FRAME_ALLOCATION_FAILURES.fetch_add(1, Ordering::Relaxed);
}

pub fn note_rollback_invariant_failure() {
    ROLLBACK_INVARIANT_FAILURES.fetch_add(1, Ordering::Relaxed);
}

pub fn diagnostic_report() {
    crate::serial_println!(
        "[MM-2A-DIAG] collisions={} mmap_rollbacks={} shm_create_rollbacks={} shm_peer_rollbacks={} frame_alloc_failures={} pt_alloc_failures={} replacements={} rollback_invariant_failures={}",
        MAPPING_COLLISIONS.load(Ordering::Relaxed),
        MMAP_ROLLBACKS.load(Ordering::Relaxed),
        SHM_CREATE_ROLLBACKS.load(Ordering::Relaxed),
        SHM_PEER_ROLLBACKS.load(Ordering::Relaxed),
        FRAME_ALLOCATION_FAILURES.load(Ordering::Relaxed),
        PAGE_TABLE_ALLOCATION_FAILURES.load(Ordering::Relaxed),
        INTENTIONAL_REPLACEMENTS.load(Ordering::Relaxed),
        ROLLBACK_INVARIANT_FAILURES.load(Ordering::Relaxed),
    );
    crate::serial_println!(
        "[MM-2C-DIAG] insertions={} merges={} overlap_rejections={} capacity_failures={} rollback_removals={} teardown_removals={} consistency_failures={} borrower_lookups={} stale_lookups={}",
        REGION_INSERTIONS.load(Ordering::Relaxed),
        REGION_ADJACENT_MERGES.load(Ordering::Relaxed),
        REGION_OVERLAP_REJECTIONS.load(Ordering::Relaxed),
        REGION_CAPACITY_FAILURES.load(Ordering::Relaxed),
        REGION_ROLLBACK_REMOVALS.load(Ordering::Relaxed),
        REGION_TEARDOWN_REMOVALS.load(Ordering::Relaxed),
        REGION_PTE_CONSISTENCY_FAILURES.load(Ordering::Relaxed),
        BORROWER_LEDGER_LOOKUPS.load(Ordering::Relaxed),
        STALE_LEDGER_LOOKUPS.load(Ordering::Relaxed),
    );
}

pub struct AddressSpace {
    pub pml4_phys: PhysAddr,
    identity: AddressSpaceIdentity,
    ledger: Option<LedgerHandle>,
    borrower: bool,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ReclaimStats {
    pub user_frames: usize,
    pub page_tables: usize,
    pub swap_blocks: usize,
}

impl AddressSpace {
    /// Create a new address space, copying kernel higher-half mappings.
    /// SAFETY: `hhdm_offset` must be the correct HHDM base.
    pub unsafe fn new(pmm: &mut PhysicalMemoryManager, hhdm_offset: VirtAddr) -> Self {
        Self::try_new(pmm, hhdm_offset).expect("boot PML4 allocation failed")
    }

    /// Fallible address-space construction for user-triggered process creation.
    /// SAFETY: `hhdm_offset` must be the correct HHDM base.
    pub unsafe fn try_new(
        pmm: &mut PhysicalMemoryManager,
        hhdm_offset: VirtAddr,
    ) -> Result<Self, MappingError> {
        let pml4_phys = pmm.alloc_frame().ok_or_else(|| {
            PAGE_TABLE_ALLOCATION_FAILURES.fetch_add(1, Ordering::Relaxed);
            MappingError::PageTableAllocationFailed
        })?;

        // Map the PML4 via HHDM to initialize it.
        let pml4_virt = hhdm_offset + pml4_phys.as_u64();
        let pml4 = &mut *(pml4_virt.as_mut_ptr::<PageTable>());

        // Zero the PML4.
        for entry in pml4.iter_mut() {
            entry.set_unused();
        }

        // Copy kernel higher-half mappings (indices 256..512).
        let current_pml4 = &*get_current_pml4(hhdm_offset);
        for i in 256..512 {
            pml4[i].set_addr(current_pml4[i].addr(), current_pml4[i].flags());
        }

        let identity = allocate_identity(&NEXT_ADDRESS_SPACE_GENERATION, pml4_phys.as_u64());
        let ledger = match Self::allocate_ledger(identity) {
            Ok(handle) => handle,
            Err(error) => {
                pmm.free_frame(pml4_phys);
                return Err(error);
            }
        };
        Ok(Self {
            pml4_phys,
            identity,
            ledger: Some(ledger),
            borrower: false,
        })
    }

    /// Construct a read-only diagnostic wrapper for an already-allocated PML4.
    /// The invalid identity deliberately prevents activation or mutation.
    pub fn from_pml4(pml4_phys: PhysAddr) -> Self {
        Self {
            pml4_phys,
            identity: AddressSpaceIdentity::INVALID,
            ledger: None,
            borrower: false,
        }
    }

    /// Construct a borrower wrapper for an existing address-space instance.
    pub(crate) fn from_shared(shared: SharedAddressSpaceHandle) -> Self {
        assert_eq!(shared.pml4_phys.as_u64(), shared.identity.pml4_phys);
        assert!(shared.identity.is_valid());
        Self {
            pml4_phys: shared.pml4_phys,
            identity: shared.identity,
            ledger: Some(shared.ledger),
            borrower: true,
        }
    }

    pub(crate) fn shared_handle(&self) -> SharedAddressSpaceHandle {
        SharedAddressSpaceHandle {
            pml4_phys: self.pml4_phys,
            identity: self.identity,
            ledger: self.ledger.expect("valid address space missing ledger"),
        }
    }

    #[cfg(feature = "mm2b_smp_test")]
    pub fn test_root(pml4_phys: PhysAddr) -> Self {
        let identity = allocate_identity(&NEXT_ADDRESS_SPACE_GENERATION, pml4_phys.as_u64());
        Self {
            pml4_phys,
            identity,
            ledger: Some(Self::allocate_ledger(identity).expect("MM-2B test ledger allocation")),
            borrower: false,
        }
    }

    pub const fn identity(&self) -> AddressSpaceIdentity {
        self.identity
    }

    /// Convert software protection policy to user PTE flags in one place.
    pub fn protection_to_pte_flags(
        protection: RegionProtection,
    ) -> Result<PageTableFlags, MappingError> {
        if !protection.is_valid() {
            return Err(MappingError::PermissionRejected);
        }
        let mut flags = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE;
        if protection.writable() {
            flags |= PageTableFlags::WRITABLE;
        }
        if !protection.executable() {
            flags |= PageTableFlags::NO_EXECUTE;
        }
        Ok(flags)
    }

    pub fn protection_from_pte_flags(
        flags: PageTableFlags,
    ) -> Result<RegionProtection, MappingError> {
        if !flags.contains(PageTableFlags::PRESENT)
            || !flags.contains(PageTableFlags::USER_ACCESSIBLE)
        {
            return Err(MappingError::PermissionRejected);
        }
        RegionProtection::new(
            true,
            flags.contains(PageTableFlags::WRITABLE),
            !flags.contains(PageTableFlags::NO_EXECUTE),
        )
        .map_err(|_| MappingError::PermissionRejected)
    }

    /// Callers may already hold SCHEDULER, PMM, and the SHM broker. The ledger
    /// lock is terminal: it never allocates and is always released before PMM,
    /// broker, swap, or synchronous TLB operations are invoked.
    pub fn preflight_region(
        &self,
        region: MappingRegion,
    ) -> Result<RegionReservation, MappingError> {
        self.with_ledger_mut(|ledger| ledger.preflight(region))
            .and_then(|result| result.map_err(Self::map_ledger_error))
    }

    pub fn commit_region(
        &self,
        reservation: RegionReservation,
    ) -> Result<MappingRegion, MappingError> {
        self.with_ledger_mut(|ledger| {
            let before = ledger.len();
            let committed = ledger.commit(reservation)?;
            REGION_INSERTIONS.fetch_add(1, Ordering::Relaxed);
            if ledger.len() == before {
                REGION_ADJACENT_MERGES.fetch_add(1, Ordering::Relaxed);
            }
            Ok(committed)
        })
        .and_then(|result| result.map_err(Self::map_ledger_error))
    }

    pub fn cancel_region(&self, reservation: RegionReservation) {
        if self
            .with_ledger_mut(|ledger| ledger.cancel(reservation))
            .and_then(|result| result.map_err(Self::map_ledger_error))
            .is_err()
        {
            ROLLBACK_INVARIANT_FAILURES.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn remove_region_exact(&self, expected: MappingRegion) -> Result<(), MappingError> {
        self.with_ledger_mut(|ledger| ledger.remove_exact(expected))
            .and_then(|result| result.map_err(Self::map_ledger_error))?;
        REGION_ROLLBACK_REMOVALS.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    pub fn lookup_region(&self, address: u64) -> Option<MappingRegion> {
        self.with_ledger(|ledger| ledger.lookup_address(address))
            .ok()
            .flatten()
    }

    pub fn lookup_region_range(&self, start: u64, end: u64) -> Result<RangeLookup, MappingError> {
        self.with_ledger(|ledger| ledger.lookup_range(start, end))
            .and_then(|result| result.map_err(Self::map_ledger_error))
    }

    pub fn preflight_unmap(&self, start: u64, end: u64) -> Result<UnmapPlan, MappingError> {
        self.with_ledger(|ledger| ledger.preflight_unmap(start, end))
            .and_then(|result| result.map_err(Self::map_ledger_error))
    }

    /// Publish the ledger image staged before PTE removal. Expected failures
    /// cannot occur here; a concurrent ledger mutation is a fail-stop bug.
    pub fn commit_unmap(&self, plan: UnmapPlan) {
        if self
            .with_ledger_mut(|ledger| ledger.commit_unmap(plan))
            .is_err()
        {
            REGION_PTE_CONSISTENCY_FAILURES.fetch_add(1, Ordering::Relaxed);
            panic!("address-space ledger disappeared during munmap commit");
        }
    }

    pub fn region_count(&self) -> usize {
        self.with_ledger(RegionLedger::len).unwrap_or(0)
    }

    pub fn region_at(&self, index: usize) -> Option<MappingRegion> {
        self.with_ledger(|ledger| ledger.record_at(index))
            .ok()
            .flatten()
    }

    /// Bounded debug/gate validation. Residency transitions are accepted, but
    /// present boundary pages must match ledger W/X protection.
    pub unsafe fn validate_ledger_ptes(&self, hhdm_offset: VirtAddr) -> bool {
        let mut valid = true;
        for index in 0..self.region_count() {
            let Some(region) = self.region_at(index) else {
                valid = false;
                break;
            };
            for address in [region.start, region.end - 4096] {
                let Ok(page) = Page::<Size4KiB>::from_start_address(VirtAddr::new(address)) else {
                    valid = false;
                    continue;
                };
                let Some((_frame, flags)) = self.lookup_entry(page, hhdm_offset) else {
                    valid = false;
                    continue;
                };
                if flags.contains(PageTableFlags::PRESENT) {
                    let Ok(actual) = Self::protection_from_pte_flags(flags) else {
                        valid = false;
                        continue;
                    };
                    if actual.writable() != region.protection.writable()
                        || actual.executable() != region.protection.executable()
                    {
                        valid = false;
                    }
                }
            }
        }
        if !valid {
            REGION_PTE_CONSISTENCY_FAILURES.fetch_add(1, Ordering::Relaxed);
        }
        valid
    }

    pub fn update_region_backing(
        &self,
        start: u64,
        end: u64,
        kind: MappingKind,
        expected: RegionBacking,
        replacement: RegionBacking,
    ) -> Result<MappingRegion, MappingError> {
        self.with_ledger_mut(|ledger| {
            ledger.replace_backing(start, end, kind, expected, replacement)
        })
        .and_then(|result| result.map_err(Self::map_ledger_error))
    }

    fn allocate_ledger(identity: AddressSpaceIdentity) -> Result<LedgerHandle, MappingError> {
        let mut slots = REGION_LEDGERS.lock();
        let Some(index) = slots.iter().position(|slot| !slot.identity.is_valid()) else {
            REGION_CAPACITY_FAILURES.fetch_add(1, Ordering::Relaxed);
            return Err(MappingError::LedgerCapacityExhausted);
        };
        slots[index].identity = identity;
        slots[index].shared_bump = SHARED_REGION_BASE;
        slots[index].ledger.clear();
        Ok(LedgerHandle {
            slot: index as u8,
            identity,
        })
    }

    fn with_ledger<R>(
        &self,
        operation: impl FnOnce(&RegionLedger) -> R,
    ) -> Result<R, MappingError> {
        if self.borrower {
            BORROWER_LEDGER_LOOKUPS.fetch_add(1, Ordering::Relaxed);
        }
        let handle = self.ledger.ok_or_else(|| {
            STALE_LEDGER_LOOKUPS.fetch_add(1, Ordering::Relaxed);
            MappingError::LedgerUnavailable
        })?;
        let slots = REGION_LEDGERS.lock();
        let slot = slots.get(handle.slot as usize).ok_or_else(|| {
            STALE_LEDGER_LOOKUPS.fetch_add(1, Ordering::Relaxed);
            MappingError::LedgerUnavailable
        })?;
        if slot.identity != handle.identity || slot.identity != self.identity {
            STALE_LEDGER_LOOKUPS.fetch_add(1, Ordering::Relaxed);
            return Err(MappingError::LedgerUnavailable);
        }
        Ok(operation(&slot.ledger))
    }

    fn with_ledger_mut<R>(
        &self,
        operation: impl FnOnce(&mut RegionLedger) -> R,
    ) -> Result<R, MappingError> {
        if self.borrower {
            BORROWER_LEDGER_LOOKUPS.fetch_add(1, Ordering::Relaxed);
        }
        let handle = self.ledger.ok_or_else(|| {
            STALE_LEDGER_LOOKUPS.fetch_add(1, Ordering::Relaxed);
            MappingError::LedgerUnavailable
        })?;
        let mut slots = REGION_LEDGERS.lock();
        let slot = slots.get_mut(handle.slot as usize).ok_or_else(|| {
            STALE_LEDGER_LOOKUPS.fetch_add(1, Ordering::Relaxed);
            MappingError::LedgerUnavailable
        })?;
        if slot.identity != handle.identity || slot.identity != self.identity {
            STALE_LEDGER_LOOKUPS.fetch_add(1, Ordering::Relaxed);
            return Err(MappingError::LedgerUnavailable);
        }
        Ok(operation(&mut slot.ledger))
    }

    fn map_ledger_error(error: LedgerError) -> MappingError {
        match error {
            LedgerError::Overlap => {
                REGION_OVERLAP_REJECTIONS.fetch_add(1, Ordering::Relaxed);
                MappingError::LedgerOverlap
            }
            LedgerError::CapacityExhausted | LedgerError::TooManyPending => {
                REGION_CAPACITY_FAILURES.fetch_add(1, Ordering::Relaxed);
                MappingError::LedgerCapacityExhausted
            }
            LedgerError::InvalidRange => MappingError::InvalidAddress,
            LedgerError::PermissionRejected => MappingError::PermissionRejected,
            LedgerError::PolicyRejected => MappingError::ProtectedRegion,
            LedgerError::StaleReservation
            | LedgerError::ExactRecordNotFound
            | LedgerError::Inconsistent => MappingError::InternalInvariant,
        }
    }

    fn shared_bump(&self) -> Result<u64, MappingError> {
        let handle = self.ledger.ok_or(MappingError::LedgerUnavailable)?;
        let slots = REGION_LEDGERS.lock();
        let slot = slots
            .get(handle.slot as usize)
            .filter(|slot| slot.identity == handle.identity && slot.identity == self.identity)
            .ok_or_else(|| {
                STALE_LEDGER_LOOKUPS.fetch_add(1, Ordering::Relaxed);
                MappingError::LedgerUnavailable
            })?;
        Ok(slot.shared_bump)
    }

    fn commit_shared_bump(&self, expected: u64, replacement: u64) -> Result<(), MappingError> {
        let handle = self.ledger.ok_or(MappingError::LedgerUnavailable)?;
        let mut slots = REGION_LEDGERS.lock();
        let slot = slots
            .get_mut(handle.slot as usize)
            .filter(|slot| slot.identity == handle.identity && slot.identity == self.identity)
            .ok_or_else(|| {
                STALE_LEDGER_LOOKUPS.fetch_add(1, Ordering::Relaxed);
                MappingError::LedgerUnavailable
            })?;
        if slot.shared_bump != expected {
            return Err(MappingError::InternalInvariant);
        }
        slot.shared_bump = replacement;
        Ok(())
    }

    fn destroy_ledger(&mut self) -> Result<usize, MappingError> {
        let handle = self.ledger.take().ok_or(MappingError::LedgerUnavailable)?;
        let mut slots = REGION_LEDGERS.lock();
        let slot = slots
            .get_mut(handle.slot as usize)
            .filter(|slot| slot.identity == handle.identity && slot.identity == self.identity)
            .ok_or_else(|| {
                STALE_LEDGER_LOOKUPS.fetch_add(1, Ordering::Relaxed);
                MappingError::LedgerUnavailable
            })?;
        if slot.ledger.validate().is_err() {
            REGION_PTE_CONSISTENCY_FAILURES.fetch_add(1, Ordering::Relaxed);
            return Err(MappingError::InternalInvariant);
        }
        let removed = slot.ledger.clear();
        slot.identity = AddressSpaceIdentity::INVALID;
        slot.shared_bump = SHARED_REGION_BASE;
        REGION_TEARDOWN_REMOVALS.fetch_add(removed as u64, Ordering::Relaxed);
        Ok(removed)
    }

    /// True after the address space root has been freed during process teardown.
    pub fn is_reclaimed(&self) -> bool {
        self.pml4_phys.as_u64() == 0
    }

    /// Map a page in this address space.
    /// SAFETY: `hhdm_offset` must be the correct HHDM base.
    pub unsafe fn map_page(
        &mut self,
        page: Page<Size4KiB>,
        phys: PhysFrame<Size4KiB>,
        flags: PageTableFlags,
        pmm: &mut PhysicalMemoryManager,
        hhdm_offset: VirtAddr,
    ) -> Result<(), MappingError> {
        if flags.contains(PageTableFlags::USER_ACCESSIBLE)
            && flags.contains(PageTableFlags::WRITABLE)
            && !flags.contains(PageTableFlags::NO_EXECUTE)
        {
            return Err(MappingError::PermissionRejected);
        }
        let pml4 = &mut *((hhdm_offset + self.pml4_phys.as_u64()).as_mut_ptr::<PageTable>());

        let mut created: [Option<(*mut PageTableEntry, PhysAddr)>; 3] = [None, None, None];

        let p4_entry = &mut pml4[page.p4_index()];
        let p3_table = match Self::create_next_table(p4_entry, pmm, hhdm_offset, &mut created[0]) {
            Ok(table) => table,
            Err(error) => return Err(error),
        };

        let p3_entry = &mut p3_table[page.p3_index()];
        let p2_table = match Self::create_next_table(p3_entry, pmm, hhdm_offset, &mut created[1]) {
            Ok(table) => table,
            Err(error) => {
                Self::rollback_created_tables(&mut created, self.identity, page, pmm);
                return Err(error);
            }
        };

        let p2_entry = &mut p2_table[page.p2_index()];
        let p1_table = match Self::create_next_table(p2_entry, pmm, hhdm_offset, &mut created[2]) {
            Ok(table) => table,
            Err(error) => {
                Self::rollback_created_tables(&mut created, self.identity, page, pmm);
                return Err(error);
            }
        };

        let p1_entry = &mut p1_table[page.p1_index()];
        if !p1_entry.is_unused() {
            MAPPING_COLLISIONS.fetch_add(1, Ordering::Relaxed);
            Self::rollback_created_tables(&mut created, self.identity, page, pmm);
            return Err(MappingError::AlreadyMapped);
        }
        p1_entry.set_frame(phys, flags);
        Ok(())
    }

    /// Return whether a leaf is occupied by either a present mapping or a
    /// non-present marker such as a swapped page.
    pub unsafe fn is_occupied(&self, page: Page<Size4KiB>, hhdm_offset: VirtAddr) -> bool {
        if self.is_reclaimed() {
            return true;
        }
        let p4 = &*((hhdm_offset + self.pml4_phys.as_u64()).as_ptr::<PageTable>());
        let p4e = &p4[page.p4_index()];
        if p4e.is_unused() {
            return false;
        }
        if !p4e.flags().contains(PageTableFlags::PRESENT)
            || p4e.flags().contains(PageTableFlags::HUGE_PAGE)
        {
            return true;
        }
        let p3 = &*((hhdm_offset + p4e.addr().as_u64()).as_ptr::<PageTable>());
        let p3e = &p3[page.p3_index()];
        if p3e.is_unused() {
            return false;
        }
        if !p3e.flags().contains(PageTableFlags::PRESENT)
            || p3e.flags().contains(PageTableFlags::HUGE_PAGE)
        {
            return true;
        }
        let p2 = &*((hhdm_offset + p3e.addr().as_u64()).as_ptr::<PageTable>());
        let p2e = &p2[page.p2_index()];
        if p2e.is_unused() {
            return false;
        }
        if !p2e.flags().contains(PageTableFlags::PRESENT)
            || p2e.flags().contains(PageTableFlags::HUGE_PAGE)
        {
            return true;
        }
        let p1 = &*((hhdm_offset + p2e.addr().as_u64()).as_ptr::<PageTable>());
        !p1[page.p1_index()].is_unused()
    }

    /// Replace exactly the expected leaf state. This is the only API for CoW
    /// and swap transitions; ordinary mapping never replaces a leaf.
    pub unsafe fn replace_mapping(
        &mut self,
        page: Page<Size4KiB>,
        expected: ExpectedMapping,
        replacement: ReplacementMapping,
        ownership: OwnershipTransition,
        pmm: &mut PhysicalMemoryManager,
        hhdm_offset: VirtAddr,
    ) -> Result<(), MappingError> {
        let ptr = self
            .p1_entry_ptr(page, hhdm_offset)
            .ok_or(MappingError::NotMapped)?;
        let entry = &mut *ptr;
        let current_matches = match expected {
            ExpectedMapping::Present { frame, flags } => {
                entry.flags().contains(PageTableFlags::PRESENT)
                    && entry.addr() == frame
                    && entry.flags() == flags
            }
            ExpectedMapping::Swapped { block_id } => {
                let encoded = block_id
                    .checked_add(1)
                    .and_then(|value| value.checked_shl(12))
                    .ok_or(MappingError::Overflow)?;
                !entry.is_unused()
                    && !entry.flags().contains(PageTableFlags::PRESENT)
                    && entry.addr().as_u64() == encoded
            }
        };
        if !current_matches {
            return Err(MappingError::UnsupportedReplacement);
        }
        if ownership == OwnershipTransition::ReleaseOldFrame
            && !matches!(expected, ExpectedMapping::Present { .. })
        {
            return Err(MappingError::InternalInvariant);
        }

        match replacement {
            ReplacementMapping::Present { frame, flags } => {
                if flags.contains(PageTableFlags::USER_ACCESSIBLE)
                    && flags.contains(PageTableFlags::WRITABLE)
                    && !flags.contains(PageTableFlags::NO_EXECUTE)
                {
                    return Err(MappingError::PermissionRejected);
                }
                entry.set_frame(frame, flags);
            }
            ReplacementMapping::Swapped { block_id } => {
                let encoded = block_id
                    .checked_add(1)
                    .and_then(|value| value.checked_shl(12))
                    .ok_or(MappingError::Overflow)?;
                entry.set_addr(PhysAddr::new(encoded), PageTableFlags::empty());
            }
        }
        crate::memory::tlb::invalidate_page(self.identity, page.start_address());
        if ownership == OwnershipTransition::ReleaseOldFrame {
            if let ExpectedMapping::Present { frame, .. } = expected {
                pmm.free_frame(frame);
            }
        }
        INTENTIONAL_REPLACEMENTS.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Look up the physical address mapped for `page`, if any.
    /// SAFETY: `hhdm_offset` must be the correct HHDM base.
    pub unsafe fn lookup_phys(
        &self,
        page: Page<Size4KiB>,
        hhdm_offset: VirtAddr,
    ) -> Option<PhysAddr> {
        unsafe { self.lookup_entry(page, hhdm_offset).map(|(phys, _)| phys) }
    }

    /// Look up the physical address and flags mapped for `page`, if any.
    /// SAFETY: `hhdm_offset` must be the correct HHDM base.
    pub unsafe fn lookup_entry(
        &self,
        page: Page<Size4KiB>,
        hhdm_offset: VirtAddr,
    ) -> Option<(PhysAddr, PageTableFlags)> {
        if self.is_reclaimed() {
            return None;
        }
        let pml4 = &*((hhdm_offset + self.pml4_phys.as_u64()).as_ptr::<PageTable>());
        let p4_entry = &pml4[page.p4_index()];
        if p4_entry.is_unused()
            || !p4_entry.flags().contains(PageTableFlags::PRESENT)
            || p4_entry.flags().contains(PageTableFlags::HUGE_PAGE)
        {
            return None;
        }
        let p3_table = &*((hhdm_offset + p4_entry.addr().as_u64()).as_ptr::<PageTable>());
        let p3_entry = &p3_table[page.p3_index()];
        if p3_entry.is_unused()
            || !p3_entry.flags().contains(PageTableFlags::PRESENT)
            || p3_entry.flags().contains(PageTableFlags::HUGE_PAGE)
        {
            return None;
        }
        let p2_table = &*((hhdm_offset + p3_entry.addr().as_u64()).as_ptr::<PageTable>());
        let p2_entry = &p2_table[page.p2_index()];
        if p2_entry.is_unused()
            || !p2_entry.flags().contains(PageTableFlags::PRESENT)
            || p2_entry.flags().contains(PageTableFlags::HUGE_PAGE)
        {
            return None;
        }
        let p1_table = &*((hhdm_offset + p2_entry.addr().as_u64()).as_ptr::<PageTable>());
        let p1_entry = &p1_table[page.p1_index()];
        if p1_entry.is_unused() {
            return None;
        }
        Some((p1_entry.addr(), p1_entry.flags()))
    }

    /// Replace flags only if the leaf still has the expected flags.
    /// SAFETY: `hhdm_offset` must be the correct HHDM base.
    pub unsafe fn update_flags(
        &mut self,
        page: Page<Size4KiB>,
        expected_flags: PageTableFlags,
        flags: PageTableFlags,
        hhdm_offset: VirtAddr,
    ) -> Result<(), MappingError> {
        if flags.contains(PageTableFlags::USER_ACCESSIBLE)
            && flags.contains(PageTableFlags::WRITABLE)
            && !flags.contains(PageTableFlags::NO_EXECUTE)
        {
            return Err(MappingError::PermissionRejected);
        }
        let entry = &mut *self
            .p1_entry_ptr(page, hhdm_offset)
            .ok_or(MappingError::NotMapped)?;
        if !entry.flags().contains(PageTableFlags::PRESENT) {
            return Err(MappingError::NotMapped);
        }
        if entry.flags() != expected_flags {
            return Err(MappingError::UnsupportedReplacement);
        }
        let frame = entry.addr();
        entry.set_addr(frame, flags);
        crate::memory::tlb::invalidate_page(self.identity, page.start_address());
        Ok(())
    }

    /// Walk to the leaf (P1) entry for `page`, returning a raw pointer to it
    /// if every higher-level table is already present. Works for both
    /// present and not-present (e.g. swapped) leaf entries.
    /// SAFETY: `hhdm_offset` must be the correct HHDM base.
    unsafe fn p1_entry_ptr(
        &self,
        page: Page<Size4KiB>,
        hhdm_offset: VirtAddr,
    ) -> Option<*mut x86_64::structures::paging::page_table::PageTableEntry> {
        if self.is_reclaimed() {
            return None;
        }
        let pml4 = &*((hhdm_offset + self.pml4_phys.as_u64()).as_ptr::<PageTable>());
        let p4_entry = &pml4[page.p4_index()];
        if p4_entry.is_unused()
            || !p4_entry.flags().contains(PageTableFlags::PRESENT)
            || p4_entry.flags().contains(PageTableFlags::HUGE_PAGE)
        {
            return None;
        }
        let p3_table = &*((hhdm_offset + p4_entry.addr().as_u64()).as_ptr::<PageTable>());
        let p3_entry = &p3_table[page.p3_index()];
        if p3_entry.is_unused()
            || !p3_entry.flags().contains(PageTableFlags::PRESENT)
            || p3_entry.flags().contains(PageTableFlags::HUGE_PAGE)
        {
            return None;
        }
        let p2_table = &*((hhdm_offset + p3_entry.addr().as_u64()).as_ptr::<PageTable>());
        let p2_entry = &p2_table[page.p2_index()];
        if p2_entry.is_unused()
            || !p2_entry.flags().contains(PageTableFlags::PRESENT)
            || p2_entry.flags().contains(PageTableFlags::HUGE_PAGE)
        {
            return None;
        }
        let p1_table = &*((hhdm_offset + p2_entry.addr().as_u64()).as_ptr::<PageTable>());
        Some(&p1_table[page.p1_index()] as *const _ as *mut _)
    }

    /// If `page`'s leaf entry is a swapped marker (not present, non-zero
    /// address field), return the encoded ZRAM block id.
    /// SAFETY: `hhdm_offset` must be the correct HHDM base.
    pub unsafe fn swapped_block_id(
        &self,
        page: Page<Size4KiB>,
        hhdm_offset: VirtAddr,
    ) -> Option<u64> {
        let entry = &*self.p1_entry_ptr(page, hhdm_offset)?;
        if entry.is_unused() || entry.flags().contains(PageTableFlags::PRESENT) {
            return None;
        }
        (entry.addr().as_u64() >> 12).checked_sub(1)
    }

    /// Clear a leaf only if it still has the exact state established by the
    /// munmap preflight. TLB invalidation and ownership release are separate so
    /// callers can batch a bounded range into one synchronous shootdown.
    pub unsafe fn remove_expected_mapping(
        &mut self,
        page: Page<Size4KiB>,
        expected: ExpectedMapping,
        hhdm_offset: VirtAddr,
    ) -> Result<(), MappingError> {
        let entry = &mut *self
            .p1_entry_ptr(page, hhdm_offset)
            .ok_or(MappingError::NotMapped)?;
        let matches = match expected {
            ExpectedMapping::Present { frame, flags } => {
                entry.flags().contains(PageTableFlags::PRESENT)
                    && entry.addr() == frame
                    && entry.flags() == flags
            }
            ExpectedMapping::Swapped { block_id } => {
                let encoded = block_id
                    .checked_add(1)
                    .and_then(|value| value.checked_shl(12))
                    .ok_or(MappingError::Overflow)?;
                !entry.is_unused()
                    && !entry.flags().contains(PageTableFlags::PRESENT)
                    && entry.addr().as_u64() == encoded
            }
        };
        if !matches {
            return Err(MappingError::UnsupportedReplacement);
        }
        entry.set_unused();
        Ok(())
    }

    /// Reclaim empty lower-half page tables after the covering leaf shootdown
    /// has completed. Tables shared by any surviving mapping are retained.
    pub unsafe fn reclaim_empty_tables_for_page(
        &mut self,
        page: Page<Size4KiB>,
        pmm: &mut PhysicalMemoryManager,
        hhdm_offset: VirtAddr,
    ) -> usize {
        if self.is_reclaimed() {
            return 0;
        }
        let pml4 = &mut *((hhdm_offset + self.pml4_phys.as_u64()).as_mut_ptr::<PageTable>());
        let p4e = &mut pml4[page.p4_index()];
        if p4e.is_unused()
            || !p4e.flags().contains(PageTableFlags::PRESENT)
            || p4e.flags().contains(PageTableFlags::HUGE_PAGE)
        {
            return 0;
        }
        let p3_phys = p4e.addr();
        let p3 = &mut *((hhdm_offset + p3_phys.as_u64()).as_mut_ptr::<PageTable>());
        let p3e = &mut p3[page.p3_index()];
        if p3e.is_unused()
            || !p3e.flags().contains(PageTableFlags::PRESENT)
            || p3e.flags().contains(PageTableFlags::HUGE_PAGE)
        {
            return 0;
        }
        let p2_phys = p3e.addr();
        let p2 = &mut *((hhdm_offset + p2_phys.as_u64()).as_mut_ptr::<PageTable>());
        let p2e = &mut p2[page.p2_index()];
        if p2e.is_unused()
            || !p2e.flags().contains(PageTableFlags::PRESENT)
            || p2e.flags().contains(PageTableFlags::HUGE_PAGE)
        {
            return 0;
        }
        let p1_phys = p2e.addr();
        let p1 = &mut *((hhdm_offset + p1_phys.as_u64()).as_mut_ptr::<PageTable>());
        if !p1.iter().all(PageTableEntry::is_unused) {
            return 0;
        }

        let mut reclaimed = 1usize;
        p2e.set_unused();
        pmm.free_frame(p1_phys);
        if p2.iter().all(PageTableEntry::is_unused) {
            reclaimed += 1;
            p3e.set_unused();
            pmm.free_frame(p2_phys);
            if p3.iter().all(PageTableEntry::is_unused) {
                reclaimed += 1;
                p4e.set_unused();
                pmm.free_frame(p3_phys);
            }
        }
        reclaimed
    }

    /// Switch to this address space (write PML4 phys addr to CR3).
    /// SAFETY: `pml4_phys` must be a valid, page-aligned physical address.
    pub unsafe fn activate(&self) {
        crate::memory::tlb::activate(self.identity);
    }

    /// Map a single shared physical frame (compat wrapper).
    /// SAFETY: hhdm_offset must be correct HHDM base; phys must be a valid allocated frame.
    pub unsafe fn map_shared_page(
        &mut self,
        phys: PhysAddr,
        pmm: &mut crate::memory::pmm::PhysicalMemoryManager,
        hhdm_offset: VirtAddr,
    ) -> Result<VirtAddr, crate::memory::shared::SharedMemError> {
        let frame = PhysFrame::<Size4KiB>::from_start_address(phys)
            .map_err(|_| crate::memory::shared::SharedMemError::InvalidAddress)?;
        self.map_shared_region(&[frame], RegionBacking::None, pmm, hhdm_offset)
    }

    /// Map a (possibly multi-page) shared memory region contiguously into the dedicated shared area.
    /// Returns the base virtual address of the region.
    /// SAFETY: hhdm_offset must be correct HHDM base; frames must be valid.
    pub unsafe fn map_shared_region(
        &mut self,
        frames: &[PhysFrame<Size4KiB>],
        backing: RegionBacking,
        pmm: &mut crate::memory::pmm::PhysicalMemoryManager,
        hhdm_offset: VirtAddr,
    ) -> Result<VirtAddr, crate::memory::shared::SharedMemError> {
        const PAGE_SIZE: u64 = 4096;
        let n = frames.len();
        if n == 0 {
            return Err(crate::memory::shared::SharedMemError::InvalidArgument);
        }
        let total = (n as u64)
            .checked_mul(PAGE_SIZE)
            .ok_or(crate::memory::shared::SharedMemError::InvalidAddress)?;
        let base_u64 = self.shared_bump()?;
        let end = base_u64
            .checked_add(total)
            .ok_or(crate::memory::shared::SharedMemError::InvalidAddress)?;
        if end > 0x0000_0004_0000_0000 {
            return Err(crate::memory::shared::SharedMemError::OutOfMemory);
        }
        let base = VirtAddr::new(base_u64);
        let protection = RegionProtection::READ_WRITE;
        let region = MappingRegion::new(
            base_u64,
            end,
            protection,
            MappingKind::SharedMemory,
            RegionPolicy::SHARED.union(RegionPolicy::OWNER_MANAGED),
            backing,
        )
        .map_err(|_| crate::memory::shared::SharedMemError::InvalidAddress)?;
        let reservation = self.preflight_region(region)?;
        let flags = Self::protection_to_pte_flags(protection)?;

        for i in 0..n {
            let offset = (i as u64)
                .checked_mul(PAGE_SIZE)
                .ok_or(crate::memory::shared::SharedMemError::InvalidAddress)?;
            let page = Page::<Size4KiB>::from_start_address(base + offset).map_err(|_| {
                self.cancel_region(reservation);
                crate::memory::shared::SharedMemError::InvalidAddress
            })?;
            if self.is_occupied(page, hhdm_offset) {
                MAPPING_COLLISIONS.fetch_add(1, Ordering::Relaxed);
                self.cancel_region(reservation);
                return Err(crate::memory::shared::SharedMemError::AlreadyMapped);
            }
        }

        let mut installed = 0usize;
        for (i, frame) in frames.iter().enumerate() {
            let v = base + (i as u64) * PAGE_SIZE;
            let page = Page::<Size4KiB>::from_start_address(v)
                .map_err(|_| crate::memory::shared::SharedMemError::InvalidAddress)?;
            if let Err(error) = self.map_page(page, *frame, flags, pmm, hhdm_offset) {
                for rollback_index in (0..installed).rev() {
                    let Ok(rollback_page) = Page::<Size4KiB>::from_start_address(
                        base + (rollback_index as u64) * PAGE_SIZE,
                    ) else {
                        ROLLBACK_INVARIANT_FAILURES.fetch_add(1, Ordering::Relaxed);
                        continue;
                    };
                    let rollback_frame = frames[rollback_index].start_address();
                    if self
                        .rollback_mapped_page(rollback_page, rollback_frame, pmm, hhdm_offset)
                        .is_err()
                    {
                        ROLLBACK_INVARIANT_FAILURES.fetch_add(1, Ordering::Relaxed);
                    }
                }
                self.cancel_region(reservation);
                return Err(crate::memory::shared::SharedMemError::from(error));
            }
            installed += 1;
        }
        if let Err(error) = self.commit_region(reservation) {
            for rollback_index in (0..installed).rev() {
                let rollback_page = Page::<Size4KiB>::from_start_address(
                    base + (rollback_index as u64) * PAGE_SIZE,
                )
                .map_err(|_| crate::memory::shared::SharedMemError::InternalInvariant)?;
                let rollback_frame = frames[rollback_index].start_address();
                self.rollback_mapped_page(rollback_page, rollback_frame, pmm, hhdm_offset)
                    .map_err(crate::memory::shared::SharedMemError::from)?;
            }
            return Err(crate::memory::shared::SharedMemError::from(error));
        }
        self.commit_shared_bump(base_u64, end)?;
        for _ in 0..installed {
            crate::memory::security::note_nx_shm_mapping();
        }
        Ok(base)
    }

    /// Unmap a page previously mapped in this address space. Returns the phys addr if it was present.
    /// Does not free page tables or the frame itself.
    /// SAFETY: hhdm_offset correct.
    pub unsafe fn unmap_page(
        &mut self,
        page: Page<Size4KiB>,
        hhdm_offset: VirtAddr,
    ) -> Option<PhysAddr> {
        if self.is_reclaimed() {
            return None;
        }
        let pml4 = &mut *((hhdm_offset + self.pml4_phys.as_u64()).as_mut_ptr::<PageTable>());
        let p4e = &mut pml4[page.p4_index()];
        if p4e.is_unused() {
            return None;
        }
        let p3 = &mut *((hhdm_offset + p4e.addr().as_u64()).as_mut_ptr::<PageTable>());
        let p3e = &mut p3[page.p3_index()];
        if p3e.is_unused() {
            return None;
        }
        let p2 = &mut *((hhdm_offset + p3e.addr().as_u64()).as_mut_ptr::<PageTable>());
        let p2e = &mut p2[page.p2_index()];
        if p2e.is_unused() {
            return None;
        }
        let p1 = &mut *((hhdm_offset + p2e.addr().as_u64()).as_mut_ptr::<PageTable>());
        let p1e = &mut p1[page.p1_index()];
        if p1e.is_unused() || !p1e.flags().contains(PageTableFlags::PRESENT) {
            return None;
        }
        let phys = p1e.addr();
        p1e.set_unused();
        crate::memory::tlb::invalidate_page(self.identity, page.start_address());
        Some(phys)
    }

    /// Roll back a leaf only when it still maps the exact frame installed by
    /// the caller, then reclaim empty lower-level tables. Existing mappings
    /// and non-present markers are never removed.
    pub unsafe fn rollback_mapped_page(
        &mut self,
        page: Page<Size4KiB>,
        expected_frame: PhysAddr,
        pmm: &mut PhysicalMemoryManager,
        hhdm_offset: VirtAddr,
    ) -> Result<(), MappingError> {
        let pml4 = &mut *((hhdm_offset + self.pml4_phys.as_u64()).as_mut_ptr::<PageTable>());
        let p4e = &mut pml4[page.p4_index()];
        if p4e.is_unused() {
            return Err(MappingError::NotMapped);
        }
        let p3_phys = p4e.addr();
        let p3 = &mut *((hhdm_offset + p3_phys.as_u64()).as_mut_ptr::<PageTable>());
        let p3e = &mut p3[page.p3_index()];
        if p3e.is_unused() || p3e.flags().contains(PageTableFlags::HUGE_PAGE) {
            return Err(MappingError::NotMapped);
        }
        let p2_phys = p3e.addr();
        let p2 = &mut *((hhdm_offset + p2_phys.as_u64()).as_mut_ptr::<PageTable>());
        let p2e = &mut p2[page.p2_index()];
        if p2e.is_unused() || p2e.flags().contains(PageTableFlags::HUGE_PAGE) {
            return Err(MappingError::NotMapped);
        }
        let p1_phys = p2e.addr();
        let p1 = &mut *((hhdm_offset + p1_phys.as_u64()).as_mut_ptr::<PageTable>());
        let p1e = &mut p1[page.p1_index()];
        if !p1e.flags().contains(PageTableFlags::PRESENT) || p1e.addr() != expected_frame {
            ROLLBACK_INVARIANT_FAILURES.fetch_add(1, Ordering::Relaxed);
            return Err(MappingError::InternalInvariant);
        }
        p1e.set_unused();
        crate::memory::tlb::invalidate_page(self.identity, page.start_address());

        if p1.iter().all(PageTableEntry::is_unused) {
            p2e.set_unused();
            pmm.free_frame(p1_phys);
            if p2.iter().all(PageTableEntry::is_unused) {
                p3e.set_unused();
                pmm.free_frame(p2_phys);
                if p3.iter().all(PageTableEntry::is_unused) {
                    p4e.set_unused();
                    pmm.free_frame(p3_phys);
                }
            }
        }
        Ok(())
    }

    /// Free all lower-half user mappings and page tables. The root PML4 frame
    /// is freed when `free_root` is true; callers must ensure CR3 no longer
    /// points at this address space before setting that flag.
    pub unsafe fn reclaim_user_space(
        &mut self,
        pmm: &mut PhysicalMemoryManager,
        hhdm_offset: VirtAddr,
        free_root: bool,
    ) -> ReclaimStats {
        let mut stats = ReclaimStats::default();
        if self.is_reclaimed() {
            return stats;
        }
        assert_eq!(
            crate::memory::tlb::active_cpu_mask(self.identity),
            0,
            "attempted to reclaim an active address space"
        );
        debug_assert!(self.validate_ledger_ptes(hhdm_offset));
        let pml4 = &mut *((hhdm_offset + self.pml4_phys.as_u64()).as_mut_ptr::<PageTable>());

        for p4_idx in 0..256 {
            let p4e = &mut pml4[p4_idx];
            if p4e.is_unused() {
                continue;
            }

            let p3_phys = p4e.addr();
            let p3 = &mut *((hhdm_offset + p3_phys.as_u64()).as_mut_ptr::<PageTable>());
            for (p3_idx, p3e) in p3.iter_mut().enumerate() {
                if p3e.is_unused() {
                    continue;
                }

                let p2_phys = p3e.addr();
                let p2 = &mut *((hhdm_offset + p2_phys.as_u64()).as_mut_ptr::<PageTable>());
                for (p2_idx, p2e) in p2.iter_mut().enumerate() {
                    if p2e.is_unused() {
                        continue;
                    }

                    let p1_phys = p2e.addr();
                    let p1 = &mut *((hhdm_offset + p1_phys.as_u64()).as_mut_ptr::<PageTable>());
                    for (p1_idx, p1e) in p1.iter_mut().enumerate() {
                        if p1e.is_unused() {
                            continue;
                        }
                        if p1e.flags().contains(PageTableFlags::PRESENT) {
                            let phys = p1e.addr();
                            crate::memory::swap::untrack(phys);
                            let virtual_address = ((p4_idx as u64) << 39)
                                | ((p3_idx as u64) << 30)
                                | ((p2_idx as u64) << 21)
                                | ((p1_idx as u64) << 12);
                            let externally_owned =
                                self.lookup_region(virtual_address).is_some_and(|region| {
                                    matches!(
                                        region.kind,
                                        MappingKind::SharedMemory
                                            | MappingKind::Framebuffer
                                            | MappingKind::Telemetry
                                            | MappingKind::BootSharedData
                                    )
                                });
                            if !externally_owned {
                                pmm.free_frame(phys);
                                stats.user_frames += 1;
                            }
                        } else if p1e.addr().as_u64() != 0 {
                            let _ = crate::memory::zram::discard_block(
                                ((p1e.addr().as_u64() >> 12) - 1) as usize,
                            );
                            stats.swap_blocks += 1;
                        }
                        p1e.set_unused();
                    }

                    p2e.set_unused();
                    pmm.free_frame(p1_phys);
                    stats.page_tables += 1;
                }

                p3e.set_unused();
                pmm.free_frame(p2_phys);
                stats.page_tables += 1;
            }

            p4e.set_unused();
            pmm.free_frame(p3_phys);
            stats.page_tables += 1;
        }

        if free_root {
            self.destroy_ledger()
                .expect("address-space ledger teardown failed");
            pmm.free_frame(self.pml4_phys);
            stats.page_tables += 1;
            self.pml4_phys = PhysAddr::new(0);
            self.identity = AddressSpaceIdentity::INVALID;
        }

        stats
    }

    /// Count the number of present user-space 4 KiB pages mapped in this
    /// address space (an RSS-like measure). Walks only the lower half of the
    /// PML4 (indices 0..256); the kernel higher half is shared and excluded.
    /// Huge pages are counted by the number of 4 KiB pages they span.
    ///
    /// SAFETY: `hhdm_offset` must be the correct HHDM base and the page tables
    /// must be quiescent (caller holds the scheduler lock).
    pub unsafe fn count_user_pages(&self, hhdm_offset: VirtAddr) -> usize {
        if self.is_reclaimed() {
            return 0;
        }
        let mut total = 0usize;
        let pml4 = &*((hhdm_offset + self.pml4_phys.as_u64()).as_ptr::<PageTable>());
        for p4e in pml4.iter().take(256) {
            if p4e.is_unused() {
                continue;
            }
            let p3 = &*((hhdm_offset + p4e.addr().as_u64()).as_ptr::<PageTable>());
            for p3e in p3.iter() {
                if p3e.is_unused() {
                    continue;
                }
                // 1 GiB huge page
                if p3e.flags().contains(PageTableFlags::HUGE_PAGE) {
                    total += 512 * 512;
                    continue;
                }
                let p2 = &*((hhdm_offset + p3e.addr().as_u64()).as_ptr::<PageTable>());
                for p2e in p2.iter() {
                    if p2e.is_unused() {
                        continue;
                    }
                    // 2 MiB huge page
                    if p2e.flags().contains(PageTableFlags::HUGE_PAGE) {
                        total += 512;
                        continue;
                    }
                    let p1 = &*((hhdm_offset + p2e.addr().as_u64()).as_ptr::<PageTable>());
                    for p1e in p1.iter() {
                        if !p1e.is_unused() && p1e.flags().contains(PageTableFlags::PRESENT) {
                            total += 1;
                        }
                    }
                }
            }
        }
        total
    }

    /// Create or get the next-level page table for an entry.
    fn create_next_table(
        entry: &mut PageTableEntry,
        pmm: &mut PhysicalMemoryManager,
        hhdm_offset: VirtAddr,
        created: &mut Option<(*mut PageTableEntry, PhysAddr)>,
    ) -> Result<&'static mut PageTable, MappingError> {
        if entry.is_unused() {
            let frame_addr = match pmm.alloc_frame() {
                Some(frame) => frame,
                None => {
                    PAGE_TABLE_ALLOCATION_FAILURES.fetch_add(1, Ordering::Relaxed);
                    return Err(MappingError::PageTableAllocationFailed);
                }
            };
            debug_assert!(
                frame_addr.as_u64() & 0xFFF == 0,
                "PMM returned unaligned frame_addr {:#x}",
                frame_addr.as_u64()
            );
            let flags = PageTableFlags::PRESENT
                | PageTableFlags::WRITABLE
                | PageTableFlags::USER_ACCESSIBLE;
            entry.set_addr(frame_addr, flags);
            let virt = hhdm_offset + frame_addr.as_u64();
            let table = unsafe { &mut *(virt.as_mut_ptr::<PageTable>()) };
            for e in table.iter_mut() {
                e.set_unused();
            }
            *created = Some((entry as *mut PageTableEntry, frame_addr));
            Ok(table)
        } else {
            if entry.flags().contains(PageTableFlags::HUGE_PAGE) {
                MAPPING_COLLISIONS.fetch_add(1, Ordering::Relaxed);
                return Err(MappingError::AlreadyMapped);
            }
            if !entry.flags().contains(PageTableFlags::PRESENT) {
                return Err(MappingError::InternalInvariant);
            }
            let phys = entry.addr();
            let virt = hhdm_offset + phys.as_u64();
            Ok(unsafe { &mut *(virt.as_mut_ptr::<PageTable>()) })
        }
    }

    fn rollback_created_tables(
        created: &mut [Option<(*mut PageTableEntry, PhysAddr)>; 3],
        identity: AddressSpaceIdentity,
        page: Page<Size4KiB>,
        pmm: &mut PhysicalMemoryManager,
    ) {
        let mut removed_any = false;
        for item in created.iter_mut().rev() {
            if let Some((entry, _)) = item {
                unsafe {
                    (**entry).set_unused();
                }
                removed_any = true;
            }
        }
        if removed_any {
            crate::memory::tlb::invalidate_page(identity, page.start_address());
        }
        for item in created.iter_mut().rev() {
            if let Some((_, frame)) = item.take() {
                pmm.free_frame(frame);
            }
        }
    }
}

/// Get the currently active PML4 as a mutable pointer via HHDM.
/// SAFETY: `hhdm_offset` must be the correct HHDM base.
unsafe fn get_current_pml4(hhdm_offset: VirtAddr) -> *mut PageTable {
    let phys = x86_64::registers::control::Cr3::read().0.start_address();
    let virt = hhdm_offset + phys.as_u64();
    virt.as_mut_ptr::<PageTable>()
}
