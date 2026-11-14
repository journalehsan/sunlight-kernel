//! Explicit, bounded, whole-snapshot foreground relocation. No automatic GC.
use super::*;

// A changed/corrupt root can describe more blocks than the cached closure. Bound
// actual reads, rather than relying only on the pre-admission cost prediction.
struct VerificationDevice<'a, D> {
    device: &'a mut D,
    remaining: u64,
}
impl<D: BlockDevice> BlockDevice for VerificationDevice<'_, D> {
    fn info(&self) -> DeviceInfo {
        self.device.info()
    }
    fn read(&mut self, offset: u64, bytes: &mut [u8]) -> IoResult<()> {
        self.remaining = self
            .remaining
            .checked_sub(1)
            .ok_or_else(|| IoFailure::from(Error::Budget))?;
        self.device.read(offset, bytes)
    }
    fn write(&mut self, _: u64, _: &[u8]) -> IoResult<WriteCompletion> {
        Err(Error::Unsupported.into())
    }
    fn flush(&mut self) -> IoResult<()> {
        Err(Error::Unsupported.into())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SegmentState {
    Free,
    Active,
    Sealed,
    Cleaning,
    Retired,
    Quarantined,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SegmentLiveness {
    pub index: u64,
    pub state: SegmentState,
    pub current: Vec<u64>,
    pub fallback: Vec<u64>,
    /// These overlap current/fallback ownership; handles use private memory.
    pub runtime: Vec<u64>,
    pub reserved: Vec<u64>,
}
#[derive(Clone, Copy, Debug)]
pub struct CleanBudget {
    pub segments: u64,
    pub verification_reads: u64,
    pub dependency_blocks: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CleanPolicy {
    /// Only admit a transaction with strictly positive predicted reusable capacity.
    Reclaim,
    /// One explicit relocation, even when source retirement is deferred.
    Relocate,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CleanStop {
    Reclaimed,
    DeferredRetirement,
    NoCandidate,
    FallbackProtected,
    RuntimePinned,
    InsufficientReserve,
    Budget,
    Dirty,
    ReadOnly,
    DeviceFailure,
    Uncertain,
}
#[derive(Clone, Debug)]
pub struct CleanResult {
    pub segments_scanned: u64,
    pub candidates_considered: u64,
    pub victim: Option<u64>,
    pub segments_cleaned: u64,
    pub blocks_relocated: u64,
    pub data_blocks_relocated: u64,
    pub metadata_blocks_rewritten: u64,
    /// Blocks released by durable retirement, independent of target consumption.
    pub blocks_reclaimed: u64,
    pub net_reusable_blocks: i64,
    pub bytes_written: u64,
    /// Submitted bytes and attempted barriers include failed/uncertain operations.
    /// bytes_written counts device-confirmed completion, not durability.
    pub bytes_submitted: u64,
    pub checkpoints_committed: u64,
    pub flushes: u64,
    pub flushes_attempted: u64,
    pub verification_reads: u64,
    pub remaining_free_segments: u64,
    pub remaining_unscanned_segments: u64,
    pub cleaner_reserve_used: u64,
    pub recovery_floor: u64,
    pub stop: CleanStop,
    pub error: Option<FsError>,
}

impl<D: BlockDevice> Filesystem<D> {
    /// Actual closure-derived references. No state here changes the disk format.
    /// A frozen owner conservatively quarantines the entire segment area: damaged
    /// roots may own blocks beyond any closure that could still be reconstructed.
    pub fn segment_liveness(&self, limit: u64) -> Result<Vec<SegmentLiveness>> {
        let mut out = Vec::new();
        let newest = self.retained.last().ok_or(FsError::Corrupt)?;
        for index in 0..limit.min(self.sb.segment_count) {
            let start = index
                .checked_mul(self.sb.segment_blocks as u64)
                .and_then(|v| v.checked_add(self.sb.segment_start))
                .ok_or(Error::Overflow)?;
            let end = start
                .checked_add(self.sb.segment_blocks as u64)
                .ok_or(Error::Overflow)?;
            let current: Vec<_> = newest.owned.range(start..end).map(|(&b, _)| b).collect();
            let fallback: BTreeSet<_> = self
                .retained
                .iter()
                .take(self.retained.len() - 1)
                .flat_map(|s| s.owned.range(start..end).map(|(&b, _)| b))
                .collect();
            let runtime: BTreeSet<_> = self
                .retained
                .iter()
                .flat_map(|s| s.owned.range(start..end))
                .filter(|(_, o)| self.pins(&o.node) > 0)
                .map(|(&b, _)| b)
                .collect();
            let reserved: Vec<_> = self.quarantine.as_ref().map_or_else(Vec::new, |p| {
                p.admission
                    .blocks
                    .iter()
                    .copied()
                    .filter(|b| (start..end).contains(b))
                    .collect()
            });
            let state = if self.frozen || self.state != MountState::CleanWritable {
                SegmentState::Quarantined
            } else if !reserved.is_empty() {
                SegmentState::Cleaning
            } else if !current.is_empty() {
                SegmentState::Sealed
            } else if !fallback.is_empty() {
                SegmentState::Retired
            } else {
                SegmentState::Free
            };
            out.push(SegmentLiveness {
                index,
                state,
                current,
                fallback: fallback.into_iter().collect(),
                runtime: runtime.into_iter().collect(),
                reserved,
            });
        }
        Ok(out)
    }

    /// At most one full immutable replacement checkpoint, followed by normal R.
    /// Budgets are admission limits, never a request to interrupt an admitted
    /// durable sequence. Progress counters are acknowledged only after flush R.
    pub fn clean_segments(
        &mut self,
        budget: CleanBudget,
        policy: CleanPolicy,
    ) -> Result<CleanResult> {
        let before = self.accounting();
        let bps = self.sb.segment_blocks as u64;
        let floor = self
            .geometry
            .metadata_bound
            .checked_mul(2)
            .and_then(|n| n.checked_add(3))
            .ok_or(Error::Overflow)?;
        let mut r = CleanResult {
            segments_scanned: 0,
            candidates_considered: 0,
            victim: None,
            segments_cleaned: 0,
            blocks_relocated: 0,
            data_blocks_relocated: 0,
            metadata_blocks_rewritten: 0,
            blocks_reclaimed: 0,
            net_reusable_blocks: 0,
            bytes_written: 0,
            bytes_submitted: 0,
            checkpoints_committed: 0,
            flushes: 0,
            flushes_attempted: 0,
            verification_reads: 0,
            remaining_free_segments: before.available_blocks / bps,
            remaining_unscanned_segments: self.sb.segment_count,
            cleaner_reserve_used: 0,
            recovery_floor: floor,
            stop: CleanStop::NoCandidate,
            error: None,
        };
        if let Err(e) = self.writable() {
            r.stop = CleanStop::ReadOnly;
            r.error = Some(e);
            return Ok(r);
        }
        if self.dirty {
            r.stop = CleanStop::Dirty;
            return Ok(r);
        }
        let states = self.segment_liveness(budget.segments)?;
        r.segments_scanned = states.len() as u64;
        r.remaining_unscanned_segments = self
            .sb
            .segment_count
            .checked_sub(r.segments_scanned)
            .ok_or(Error::Overflow)?;
        // Equal-size segments: exact live count orders the ratio without floats.
        let mut candidates: Vec<_> = states
            .iter()
            .filter(|s| s.state == SegmentState::Sealed && s.fallback.is_empty())
            .collect();
        candidates.sort_by_key(|s| (s.current.len(), s.index));
        r.candidates_considered = candidates.len() as u64;
        let Some(victim) = candidates.first() else {
            r.stop = if r.remaining_unscanned_segments != 0 {
                CleanStop::Budget
            } else if states.iter().any(|s| !s.fallback.is_empty()) {
                CleanStop::FallbackProtected
            } else {
                CleanStop::NoCandidate
            };
            return Ok(r);
        };
        r.victim = Some(victim.index);
        // A read-only plan consumes no reservation until every check succeeds.
        let plan = match self.plan_for(true) {
            Ok(p) => p,
            Err(FsError::NoSpace) => {
                r.stop = CleanStop::InsufficientReserve;
                return Ok(r);
            }
            Err(e) => return Err(e),
        };
        let need = plan.admission.reserved_segment_blocks;
        if plan.admission.dependency_blocks > budget.dependency_blocks {
            r.stop = CleanStop::Budget;
            return Ok(r);
        }
        let tranche = self
            .sb
            .cleaner_reserve
            .checked_sub(floor)
            .ok_or(Error::Reserve)?;
        if need > tranche
            || before
                .available_blocks
                .checked_sub(need)
                .is_none_or(|n| n < floor)
        {
            r.stop = CleanStop::InsufficientReserve;
            return Ok(r);
        }
        let old_segments: BTreeSet<_> = self
            .retained
            .iter()
            .flat_map(|s| s.owned.keys())
            .map(|&b| (b - self.sb.segment_start) / bps)
            .collect();
        // publish retires one oldest root (two if R0 is required). Newest remains
        // fallback; protect it even if all its file bytes were copied successfully.
        let newest = self.retained.last().ok_or(FsError::Corrupt)?;
        let kept_segments: BTreeSet<_> = newest
            .owned
            .keys()
            .map(|&b| (b - self.sb.segment_start) / bps)
            .collect();
        let released_segments = old_segments.difference(&kept_segments).count() as u64;
        let released_capacity = released_segments.checked_mul(bps).ok_or(Error::Overflow)?;
        if policy == CleanPolicy::Reclaim && released_capacity <= need {
            r.stop = if self.runtime_accounting().pinned_orphans > 0 {
                CleanStop::RuntimePinned
            } else {
                CleanStop::FallbackProtected
            };
            return Ok(r);
        }
        let reads = self.retained.iter().try_fold(5u64, |n, s| {
            n.checked_add(s.owned.len() as u64).ok_or(Error::Overflow)
        })?;
        if reads > budget.verification_reads {
            r.stop = CleanStop::Budget;
            return Ok(r);
        }
        // Re-traverse every retained closure and read every source block before
        // admission. A valid header is insufficient, including on a live mount.
        let mut verification_device = VerificationDevice {
            device: &mut self.device,
            remaining: budget.verification_reads,
        };
        let verified = recovery::inspect(&mut verification_device).and_then(|v| {
            ensure(
                v.uuid == self.uuid
                    && v.sb == self.sb
                    && v.state == MountState::CleanWritable
                    && v.retained.len() == self.retained.len(),
            )?;
            for (a, b) in v.retained.iter().zip(&self.retained) {
                ensure(a.slot == b.slot && a.cp == b.cp && a.owned == b.owned)?;
            }
            Ok(())
        });
        if let Err(e) = verified {
            self.frozen = true;
            self.state = MountState::DegradedReadOnly;
            r.stop = match &e {
                FsError::Io(IoFailure {
                    kind: IoKind::Invalid(Error::Budget),
                    ..
                }) => CleanStop::Budget,
                FsError::Io(IoFailure {
                    certainty: Certainty::Known,
                    ..
                }) => CleanStop::DeviceFailure,
                _ => CleanStop::Uncertain,
            };
            r.error = Some(e);
            return Ok(r);
        }
        r.verification_reads = reads;
        r.cleaner_reserve_used = need.saturating_sub(
            before
                .available_blocks
                .saturating_sub(self.sb.cleaner_reserve),
        );
        let relocated = plan.admission.dependency_blocks;
        let data_blocks = plan.admission.data_blocks;
        let metadata_blocks = plan.admission.metadata_blocks;
        let bytes = (relocated
            .checked_add(1)
            .and_then(|n| n.checked_add(if self.retained.len() == 3 { 2 } else { 1 })))
        .and_then(|n| n.checked_mul(PAGE as u64))
        .ok_or(Error::Overflow)?;
        let flushes = if self.retained.len() == 3 { 4 } else { 3 };
        let old_blocks: BTreeSet<_> = self
            .retained
            .iter()
            .flat_map(|s| s.owned.keys().copied())
            .collect();
        self.quarantine = Some(plan);
        if let Err(e) = self.publish() {
            self.frozen = true;
            self.state = MountState::DegradedReadOnly;
            // I/O statistics do not grant a commit acknowledgment. Preserve exact
            // confirmed/attempted work even if persistence completion is unknown.
            if let Some(p) = &self.quarantine {
                r.bytes_written = p.written_bytes;
                r.bytes_submitted = p.submitted_bytes;
                r.flushes = p.flush_count;
                r.flushes_attempted = p.flush_attempts;
            }
            r.stop = match &e {
                FsError::Io(IoFailure {
                    certainty: Certainty::Known,
                    ..
                }) => CleanStop::DeviceFailure,
                _ => CleanStop::Uncertain,
            };
            r.error = Some(e);
            r.remaining_free_segments = self.accounting().available_blocks / bps;
            return Ok(r);
        }
        let after = self.accounting();
        let now: BTreeSet<_> = self
            .retained
            .iter()
            .flat_map(|s| s.owned.keys().copied())
            .collect();
        r.blocks_reclaimed = old_blocks.difference(&now).count() as u64;
        r.segments_cleaned = released_segments;
        r.blocks_relocated = relocated;
        r.data_blocks_relocated = data_blocks;
        r.metadata_blocks_rewritten = metadata_blocks;
        r.net_reusable_blocks = i64::try_from(after.available_blocks)
            .map_err(|_| Error::Overflow)?
            .checked_sub(i64::try_from(before.available_blocks).map_err(|_| Error::Overflow)?)
            .ok_or(Error::Overflow)?;
        r.bytes_written = bytes;
        r.bytes_submitted = bytes;
        r.checkpoints_committed = 1;
        r.flushes = flushes;
        r.flushes_attempted = flushes;
        r.remaining_free_segments = after.available_blocks / bps;
        r.stop = if r.net_reusable_blocks > 0 {
            CleanStop::Reclaimed
        } else {
            CleanStop::DeferredRetirement
        };
        Ok(r)
    }
}
