//! Bounded software policy ledger for one user address space.
//!
//! The page tables remain the hardware truth. This module records range policy
//! without owning physical frames or duplicating SHM accounting.

pub const PAGE_SIZE: u64 = 4096;
pub const MAX_REGIONS_PER_ADDRESS_SPACE: usize = 128;
pub const MAX_PENDING_REGION_INSERTIONS: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MappingKind {
    Anonymous,
    Brk,
    UserStack,
    ElfSegment,
    SharedMemory,
    Framebuffer,
    Telemetry,
    BootSharedData,
    InternalUserMapping,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct RegionProtection(u8);

impl RegionProtection {
    const READ: u8 = 1 << 0;
    const WRITE: u8 = 1 << 1;
    const EXECUTE: u8 = 1 << 2;

    pub const NONE: Self = Self(0);
    pub const READ_ONLY: Self = Self(Self::READ);
    pub const READ_WRITE: Self = Self(Self::READ | Self::WRITE);
    pub const READ_EXECUTE: Self = Self(Self::READ | Self::EXECUTE);

    pub const fn new(read: bool, write: bool, execute: bool) -> Result<Self, LedgerError> {
        if write && execute {
            return Err(LedgerError::PermissionRejected);
        }
        Ok(Self(
            (if read { Self::READ } else { 0 })
                | (if write { Self::WRITE } else { 0 })
                | (if execute { Self::EXECUTE } else { 0 }),
        ))
    }

    pub const fn readable(self) -> bool {
        self.0 & Self::READ != 0
    }

    pub const fn writable(self) -> bool {
        self.0 & Self::WRITE != 0
    }

    pub const fn executable(self) -> bool {
        self.0 & Self::EXECUTE != 0
    }

    pub const fn is_valid(self) -> bool {
        !(self.writable() && self.executable()) && self.0 & !0x7 == 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct RegionPolicy(u8);

impl RegionPolicy {
    pub const MAY_UNMAP: Self = Self(1 << 0);
    pub const MAY_CHANGE_PROTECTION: Self = Self(1 << 1);
    pub const SHARED: Self = Self(1 << 2);
    pub const SYSTEM: Self = Self(1 << 3);
    pub const OWNER_MANAGED: Self = Self(1 << 4);
    /// May be atomically displaced by a user MAP_FIXED request.
    pub const MAY_REPLACE: Self = Self(1 << 5);

    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionBacking {
    None,
    AnonymousOwner(u32),
    ElfImage(u64),
    SharedMemory(u64),
    Internal(u64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MappingRegion {
    pub start: u64,
    pub end: u64,
    pub protection: RegionProtection,
    pub kind: MappingKind,
    pub policy: RegionPolicy,
    pub backing: RegionBacking,
}

impl MappingRegion {
    const EMPTY: Self = Self {
        start: 0,
        end: 0,
        protection: RegionProtection::READ_ONLY,
        kind: MappingKind::InternalUserMapping,
        policy: RegionPolicy::empty(),
        backing: RegionBacking::None,
    };

    pub const fn new(
        start: u64,
        end: u64,
        protection: RegionProtection,
        kind: MappingKind,
        policy: RegionPolicy,
        backing: RegionBacking,
    ) -> Result<Self, LedgerError> {
        if start >= end || start & (PAGE_SIZE - 1) != 0 || end & (PAGE_SIZE - 1) != 0 {
            return Err(LedgerError::InvalidRange);
        }
        if !protection.is_valid() {
            return Err(LedgerError::PermissionRejected);
        }
        Ok(Self {
            start,
            end,
            protection,
            kind,
            policy,
            backing,
        })
    }

    pub const fn contains_address(self, address: u64) -> bool {
        self.start <= address && address < self.end
    }

    pub fn compatible_for_merge(self, other: Self) -> bool {
        if self.kind != other.kind
            || self.protection != other.protection
            || self.policy != other.policy
        {
            return false;
        }
        match (self.kind, self.backing, other.backing) {
            // Even repeated views of one object have independent broker map
            // counts and therefore remain independent lifecycle records.
            (MappingKind::SharedMemory, _, _) => false,
            (_, left, right) => left == right,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LedgerError {
    InvalidRange,
    PermissionRejected,
    PolicyRejected,
    Overlap,
    CapacityExhausted,
    TooManyPending,
    StaleReservation,
    ExactRecordNotFound,
    Hole,
    Inconsistent,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UnmapEffects {
    pub pages_covered: u64,
    pub hole_pages: u64,
    pub full_regions: u64,
    pub prefix_regions: u64,
    pub suffix_regions: u64,
    pub middle_splits: u64,
}

/// Complete fixed-capacity ledger image staged before an unmap publishes any
/// PTE changes. Its contents are intentionally opaque outside this module.
pub struct UnmapPlan {
    records: [MappingRegion; MAX_REGIONS_PER_ADDRESS_SPACE],
    len: usize,
    effects: UnmapEffects,
}

/// Complete fixed-capacity ledger image staged for a MAP_FIXED replacement.
/// The replacement is inserted into the image before PTE changes begin, so a
/// successful PTE transaction never publishes an intermediate ledger hole.
pub struct ReplacePlan {
    records: [MappingRegion; MAX_REGIONS_PER_ADDRESS_SPACE],
    len: usize,
}

/// Complete fixed-capacity ledger image staged before an mprotect publishes
/// any PTE permission changes.
pub struct ProtectPlan {
    records: [MappingRegion; MAX_REGIONS_PER_ADDRESS_SPACE],
    len: usize,
}

impl UnmapPlan {
    pub const fn effects(&self) -> UnmapEffects {
        self.effects
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegionReservation {
    nonce: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PendingRegion {
    nonce: u64,
    region: MappingRegion,
}

impl PendingRegion {
    const EMPTY: Self = Self {
        nonce: 0,
        region: MappingRegion::EMPTY,
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeLookup {
    Contained(MappingRegion),
    CompatibleAdjacent,
    Hole,
    Incompatible,
}

pub struct RegionLedger {
    records: [MappingRegion; MAX_REGIONS_PER_ADDRESS_SPACE],
    len: usize,
    pending: [PendingRegion; MAX_PENDING_REGION_INSERTIONS],
    pending_len: usize,
    next_nonce: u64,
}

impl RegionLedger {
    pub const fn new() -> Self {
        Self {
            records: [MappingRegion::EMPTY; MAX_REGIONS_PER_ADDRESS_SPACE],
            len: 0,
            pending: [PendingRegion::EMPTY; MAX_PENDING_REGION_INSERTIONS],
            pending_len: 0,
            next_nonce: 1,
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn record_at(&self, index: usize) -> Option<MappingRegion> {
        (index < self.len).then(|| self.records[index])
    }

    pub fn preflight(&mut self, region: MappingRegion) -> Result<RegionReservation, LedgerError> {
        Self::validate_region(region)?;
        if self.overlaps_committed(region) || self.overlaps_pending(region) {
            return Err(LedgerError::Overlap);
        }
        if self.pending_len == MAX_PENDING_REGION_INSERTIONS {
            return Err(LedgerError::TooManyPending);
        }
        // Capacity is reserved pessimistically. A later merge may return the
        // slot, but publication never depends on that optimization succeeding.
        if self.len + self.pending_len >= MAX_REGIONS_PER_ADDRESS_SPACE {
            return Err(LedgerError::CapacityExhausted);
        }
        let nonce = self.next_nonce;
        self.next_nonce = self.next_nonce.checked_add(1).unwrap_or(1);
        if self.next_nonce == 0 {
            self.next_nonce = 1;
        }
        self.pending[self.pending_len] = PendingRegion { nonce, region };
        self.pending_len += 1;
        Ok(RegionReservation { nonce })
    }

    /// Commit is infallible after a successful preflight unless the caller
    /// supplies a stale/cancelled token or corrupts operation ordering.
    pub fn commit(&mut self, reservation: RegionReservation) -> Result<MappingRegion, LedgerError> {
        let index = self
            .pending_index(reservation)
            .ok_or(LedgerError::StaleReservation)?;
        let region = self.pending[index].region;
        self.remove_pending(index);
        self.insert_committed(region)
    }

    pub fn cancel(&mut self, reservation: RegionReservation) -> Result<(), LedgerError> {
        let index = self
            .pending_index(reservation)
            .ok_or(LedgerError::StaleReservation)?;
        self.remove_pending(index);
        Ok(())
    }

    pub fn remove_exact(&mut self, expected: MappingRegion) -> Result<(), LedgerError> {
        let index = self.records[..self.len]
            .iter()
            .position(|record| *record == expected)
            .ok_or(LedgerError::ExactRecordNotFound)?;
        self.remove_committed(index);
        Ok(())
    }

    /// Resolve and stage a policy-authorized anonymous unmap. The complete
    /// final layout is constructed here, so commit performs no allocation and
    /// cannot encounter an expected capacity or policy failure.
    pub fn preflight_unmap(&self, start: u64, end: u64) -> Result<UnmapPlan, LedgerError> {
        let requested = MappingRegion::new(
            start,
            end,
            RegionProtection::READ_ONLY,
            MappingKind::InternalUserMapping,
            RegionPolicy::empty(),
            RegionBacking::None,
        )?;
        if self.pending_len != 0 {
            return Err(LedgerError::Inconsistent);
        }

        let requested_pages = (requested.end - requested.start) / PAGE_SIZE;
        let mut effects = UnmapEffects::default();
        let mut final_len = self.len;

        // First pass performs every fallible policy and capacity decision.
        for region in self.records[..self.len].iter().copied() {
            let overlap_start = region.start.max(requested.start);
            let overlap_end = region.end.min(requested.end);
            if overlap_start >= overlap_end {
                continue;
            }
            if region.kind != MappingKind::Anonymous
                || !region.policy.contains(RegionPolicy::MAY_UNMAP)
            {
                return Err(LedgerError::PolicyRejected);
            }
            effects.pages_covered += (overlap_end - overlap_start) / PAGE_SIZE;
            match (overlap_start == region.start, overlap_end == region.end) {
                (true, true) => {
                    effects.full_regions += 1;
                    final_len -= 1;
                }
                (true, false) => effects.prefix_regions += 1,
                (false, true) => effects.suffix_regions += 1,
                (false, false) => {
                    effects.middle_splits += 1;
                    final_len = final_len
                        .checked_add(1)
                        .ok_or(LedgerError::CapacityExhausted)?;
                }
            }
        }
        effects.hole_pages = requested_pages
            .checked_sub(effects.pages_covered)
            .ok_or(LedgerError::Inconsistent)?;
        if final_len + self.pending_len > MAX_REGIONS_PER_ADDRESS_SPACE {
            return Err(LedgerError::CapacityExhausted);
        }

        // Second pass builds the exact final sorted layout.
        let mut records = [MappingRegion::EMPTY; MAX_REGIONS_PER_ADDRESS_SPACE];
        let mut output = 0usize;
        for region in self.records[..self.len].iter().copied() {
            let overlap_start = region.start.max(requested.start);
            let overlap_end = region.end.min(requested.end);
            if overlap_start >= overlap_end {
                records[output] = region;
                output += 1;
                continue;
            }

            if region.start < overlap_start {
                let mut left = region;
                left.end = overlap_start;
                records[output] = left;
                output += 1;
            }
            if overlap_end < region.end {
                let mut right = region;
                right.start = overlap_end;
                records[output] = right;
                output += 1;
            }
        }
        if output != final_len {
            return Err(LedgerError::Inconsistent);
        }
        Ok(UnmapPlan {
            records,
            len: final_len,
            effects,
        })
    }

    /// Stage a MAP_FIXED replacement. Only mappings which opted in through
    /// MAY_REPLACE may be displaced; system, shared, and protected ranges are
    /// rejected before any page-table mutation.
    pub fn preflight_replace(&self, replacement: MappingRegion) -> Result<ReplacePlan, LedgerError> {
        Self::validate_region(replacement)?;
        if self.pending_len != 0 {
            return Err(LedgerError::Inconsistent);
        }

        let mut records = [MappingRegion::EMPTY; MAX_REGIONS_PER_ADDRESS_SPACE];
        let mut output = 0usize;
        let mut inserted = false;
        for region in self.records[..self.len].iter().copied() {
            let overlap_start = region.start.max(replacement.start);
            let overlap_end = region.end.min(replacement.end);
            if overlap_start < overlap_end && !region.policy.contains(RegionPolicy::MAY_REPLACE) {
                return Err(LedgerError::PolicyRejected);
            }
            if overlap_start >= overlap_end {
                if !inserted && replacement.end <= region.start {
                    Self::append_staged(&mut records, &mut output, replacement)?;
                    inserted = true;
                }
                Self::append_staged(&mut records, &mut output, region)?;
                continue;
            }
            if region.start < overlap_start {
                let mut left = region;
                left.end = overlap_start;
                Self::append_staged(&mut records, &mut output, left)?;
            }
            if !inserted {
                Self::append_staged(&mut records, &mut output, replacement)?;
                inserted = true;
            }
            if overlap_end < region.end {
                let mut right = region;
                right.start = overlap_end;
                Self::append_staged(&mut records, &mut output, right)?;
            }
        }
        if !inserted {
            Self::append_staged(&mut records, &mut output, replacement)?;
        }
        Ok(ReplacePlan { records, len: output })
    }

    /// Publish a fully staged unmap layout. The scheduler serializes mapping
    /// transactions for an address space, and preflight rejects pending ones.
    pub fn commit_unmap(&mut self, plan: UnmapPlan) {
        assert_eq!(self.pending_len, 0, "ledger changed during staged unmap");
        self.records = plan.records;
        self.len = plan.len;
        debug_assert_eq!(self.validate(), Ok(()));
    }

    pub fn commit_replace(&mut self, plan: ReplacePlan) {
        assert_eq!(self.pending_len, 0, "ledger changed during staged replacement");
        self.records = plan.records;
        self.len = plan.len;
        debug_assert_eq!(self.validate(), Ok(()));
    }

    /// Resolve and stage a fully covered, policy-authorized anonymous
    /// protection change. Splits and compatible merges are computed into the
    /// final bounded image, so no capacity decision remains after PTE writes.
    pub fn preflight_protect(
        &self,
        start: u64,
        end: u64,
        protection: RegionProtection,
    ) -> Result<ProtectPlan, LedgerError> {
        let requested = MappingRegion::new(
            start,
            end,
            protection,
            MappingKind::InternalUserMapping,
            RegionPolicy::empty(),
            RegionBacking::None,
        )?;
        if self.pending_len != 0 {
            return Err(LedgerError::Inconsistent);
        }

        // Authorize the entire range first. A gap or protected record rejects
        // the request before the staging pass makes a capacity decision.
        let mut covered_end = requested.start;
        for region in self.records[..self.len].iter().copied() {
            if region.end <= requested.start {
                continue;
            }
            if region.start >= requested.end {
                break;
            }
            if region.start > covered_end {
                return Err(LedgerError::Hole);
            }
            if region.kind != MappingKind::Anonymous
                || !region.policy.contains(RegionPolicy::MAY_CHANGE_PROTECTION)
            {
                return Err(LedgerError::PolicyRejected);
            }
            covered_end = region.end.min(requested.end);
        }
        if covered_end != requested.end {
            return Err(LedgerError::Hole);
        }

        let mut records = [MappingRegion::EMPTY; MAX_REGIONS_PER_ADDRESS_SPACE];
        let mut output = 0usize;
        for region in self.records[..self.len].iter().copied() {
            let overlap_start = region.start.max(requested.start);
            let overlap_end = region.end.min(requested.end);
            if overlap_start >= overlap_end {
                Self::append_staged(&mut records, &mut output, region)?;
                continue;
            }

            if region.start < overlap_start {
                let mut left = region;
                left.end = overlap_start;
                Self::append_staged(&mut records, &mut output, left)?;
            }
            let mut protected = region;
            protected.start = overlap_start;
            protected.end = overlap_end;
            protected.protection = protection;
            Self::append_staged(&mut records, &mut output, protected)?;
            if overlap_end < region.end {
                let mut right = region;
                right.start = overlap_end;
                Self::append_staged(&mut records, &mut output, right)?;
            }
        }

        Ok(ProtectPlan {
            records,
            len: output,
        })
    }

    /// Publish a fully staged protection layout. The scheduler serializes
    /// mapping transactions, and preflight rejects pending insertions.
    pub fn commit_protect(&mut self, plan: ProtectPlan) {
        assert_eq!(self.pending_len, 0, "ledger changed during staged mprotect");
        self.records = plan.records;
        self.len = plan.len;
        debug_assert_eq!(self.validate(), Ok(()));
    }

    pub fn replace_backing(
        &mut self,
        start: u64,
        end: u64,
        kind: MappingKind,
        expected: RegionBacking,
        replacement: RegionBacking,
    ) -> Result<MappingRegion, LedgerError> {
        let record = self.records[..self.len]
            .iter_mut()
            .find(|record| {
                record.start == start
                    && record.end == end
                    && record.kind == kind
                    && record.backing == expected
            })
            .ok_or(LedgerError::ExactRecordNotFound)?;
        record.backing = replacement;
        Ok(*record)
    }

    pub fn lookup_address(&self, address: u64) -> Option<MappingRegion> {
        let index = self.records[..self.len].partition_point(|region| region.end <= address);
        self.records[..self.len]
            .get(index)
            .copied()
            .filter(|region| region.contains_address(address))
    }

    pub fn lookup_range(&self, start: u64, end: u64) -> Result<RangeLookup, LedgerError> {
        let requested = MappingRegion::new(
            start,
            end,
            RegionProtection::READ_ONLY,
            MappingKind::InternalUserMapping,
            RegionPolicy::empty(),
            RegionBacking::None,
        )?;
        let Some(mut index) = self.records[..self.len]
            .iter()
            .position(|region| region.contains_address(requested.start))
        else {
            return Ok(RangeLookup::Hole);
        };
        let first = self.records[index];
        if requested.end <= first.end {
            return Ok(RangeLookup::Contained(first));
        }
        let mut covered_end = first.end;
        while covered_end < requested.end {
            index += 1;
            let Some(next) = self.records[..self.len].get(index).copied() else {
                return Ok(RangeLookup::Hole);
            };
            if next.start != covered_end {
                return Ok(RangeLookup::Hole);
            }
            if !first.compatible_for_merge(next) {
                return Ok(RangeLookup::Incompatible);
            }
            covered_end = next.end;
        }
        Ok(RangeLookup::CompatibleAdjacent)
    }

    pub fn validate(&self) -> Result<(), LedgerError> {
        if self.len > MAX_REGIONS_PER_ADDRESS_SPACE
            || self.pending_len > MAX_PENDING_REGION_INSERTIONS
        {
            return Err(LedgerError::Inconsistent);
        }
        for (index, region) in self.records[..self.len].iter().copied().enumerate() {
            Self::validate_region(region)?;
            if index != 0 && self.records[index - 1].end > region.start {
                return Err(LedgerError::Inconsistent);
            }
        }
        Ok(())
    }

    pub fn clear(&mut self) -> usize {
        let removed = self.len;
        self.len = 0;
        self.pending_len = 0;
        removed
    }

    fn validate_region(region: MappingRegion) -> Result<(), LedgerError> {
        MappingRegion::new(
            region.start,
            region.end,
            region.protection,
            region.kind,
            region.policy,
            region.backing,
        )
        .map(|_| ())
    }

    fn overlaps_committed(&self, region: MappingRegion) -> bool {
        self.records[..self.len]
            .iter()
            .any(|current| current.start < region.end && region.start < current.end)
    }

    fn overlaps_pending(&self, region: MappingRegion) -> bool {
        self.pending[..self.pending_len]
            .iter()
            .any(|current| current.region.start < region.end && region.start < current.region.end)
    }

    fn pending_index(&self, reservation: RegionReservation) -> Option<usize> {
        self.pending[..self.pending_len]
            .iter()
            .position(|pending| pending.nonce == reservation.nonce)
    }

    fn remove_pending(&mut self, index: usize) {
        self.pending_len -= 1;
        self.pending
            .copy_within(index + 1..=self.pending_len, index);
        self.pending[self.pending_len] = PendingRegion::EMPTY;
    }

    fn insert_committed(&mut self, region: MappingRegion) -> Result<MappingRegion, LedgerError> {
        if self.overlaps_committed(region) {
            return Err(LedgerError::Overlap);
        }
        let mut index =
            self.records[..self.len].partition_point(|current| current.start < region.start);
        let merge_left = index != 0
            && self.records[index - 1].end == region.start
            && self.records[index - 1].compatible_for_merge(region);
        let merge_right = index < self.len
            && region.end == self.records[index].start
            && region.compatible_for_merge(self.records[index]);

        if merge_left {
            index -= 1;
            self.records[index].end = region.end;
            if merge_right {
                self.records[index].end = self.records[index + 1].end;
                self.remove_committed(index + 1);
            }
            return Ok(self.records[index]);
        }
        if merge_right {
            self.records[index].start = region.start;
            return Ok(self.records[index]);
        }
        if self.len == MAX_REGIONS_PER_ADDRESS_SPACE {
            return Err(LedgerError::CapacityExhausted);
        }
        self.records.copy_within(index..self.len, index + 1);
        self.records[index] = region;
        self.len += 1;
        Ok(region)
    }

    fn append_staged(
        records: &mut [MappingRegion; MAX_REGIONS_PER_ADDRESS_SPACE],
        len: &mut usize,
        region: MappingRegion,
    ) -> Result<(), LedgerError> {
        if *len != 0 {
            let previous = records[*len - 1];
            if previous.end == region.start && previous.compatible_for_merge(region) {
                records[*len - 1].end = region.end;
                return Ok(());
            }
        }
        if *len == MAX_REGIONS_PER_ADDRESS_SPACE {
            return Err(LedgerError::CapacityExhausted);
        }
        records[*len] = region;
        *len += 1;
        Ok(())
    }

    fn remove_committed(&mut self, index: usize) {
        self.len -= 1;
        self.records.copy_within(index + 1..=self.len, index);
        self.records[self.len] = MappingRegion::EMPTY;
    }
}

impl Default for RegionLedger {
    fn default() -> Self {
        Self::new()
    }
}
