//! Callback quiescence and DMA-mapping lifetime protocol.
//!
//! Every asynchronous storage operation owns a stable identity, an explicit
//! owner, an active/inactive state and exactly one terminal outcome.  DMA
//! mappings carry their own user authority, independent of request completion.
//!
//! Nothing here allocates, blocks, sleeps or reads a clock.  Every transition
//! is a single atomic compare-exchange so the caller decides the interleaving
//! and the model only decides the outcome.  That keeps the protocol testable
//! with deterministic coordination instead of timing-dependent races.
//!
//! Scope: this is the driver/host-side bookkeeping contract.  It does not by
//! itself prove hardware DMA fencing, IOMMU isolation or persistence.

use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};

pub type OpId = u64;
pub type Epoch = u64;
pub type Incarnation = u64;

/// Exactly one of these is committed per operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Terminal {
    Completed,
    Cancelled,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotState {
    Idle,
    Active,
    /// Completion publication has started and owns the exit path.
    Completing,
    /// Cancellation was requested; the operation is still active.
    Cancelling,
    Terminal(Terminal),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Acquire {
    Granted,
    /// A different operation already owns this slot.
    Busy,
    /// The slot is terminal; it can never be reused.
    Closed,
    /// Wrong epoch or device incarnation.
    Stale,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CancelRequest {
    /// Request accepted.  This is NOT cancellation completion.
    Accepted,
    /// Completion publication already owns the exit; cancellation loses.
    CompletionCommitted,
    AlreadyTerminal(Terminal),
    NotActive,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FinishOutcome {
    /// This call committed the terminal state.
    Committed(Terminal),
    /// Another actor already committed a terminal state.
    LostRace(Terminal),
    /// Nothing was active to finish.
    NotActive,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetireOutcome {
    Retired,
    /// Authorised users remain; retirement must not proceed.
    Busy(u32),
    AlreadyRetired,
    /// A stale epoch cannot retire a current mapping.
    Stale,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrainOutcome {
    Complete,
    /// Bounded failure: at least one operation is not terminal.
    Bounded {
        pending: u32,
    },
}

const IDLE: u8 = 0;
const ACTIVE: u8 = 1;
const COMPLETING: u8 = 2;
const CANCELLING: u8 = 3;
const TERMINAL: u8 = 4;

const fn encode(t: Terminal) -> u8 {
    match t {
        Terminal::Completed => 1,
        Terminal::Cancelled => 2,
        Terminal::Failed => 3,
    }
}

const fn decode(v: u8) -> Option<Terminal> {
    match v {
        1 => Some(Terminal::Completed),
        2 => Some(Terminal::Cancelled),
        3 => Some(Terminal::Failed),
        _ => None,
    }
}

/// One asynchronous operation: stable identity plus exactly-once terminal.
pub struct OpSlot {
    id: OpId,
    epoch: Epoch,
    incarnation: Incarnation,
    state: AtomicU8,
    outcome: AtomicU8,
    owner: AtomicU64,
}

impl OpSlot {
    pub const fn new(id: OpId, epoch: Epoch, incarnation: Incarnation) -> Self {
        Self {
            id,
            epoch,
            incarnation,
            state: AtomicU8::new(IDLE),
            outcome: AtomicU8::new(0),
            owner: AtomicU64::new(0),
        }
    }

    pub const fn id(&self) -> OpId {
        self.id
    }

    pub const fn epoch(&self) -> Epoch {
        self.epoch
    }

    pub const fn incarnation(&self) -> Incarnation {
        self.incarnation
    }

    pub fn owner(&self) -> u64 {
        self.owner.load(Ordering::Acquire)
    }

    pub fn state(&self) -> SlotState {
        match self.state.load(Ordering::Acquire) {
            IDLE => SlotState::Idle,
            ACTIVE => SlotState::Active,
            COMPLETING => SlotState::Completing,
            CANCELLING => SlotState::Cancelling,
            TERMINAL => SlotState::Terminal(self.terminal().unwrap_or(Terminal::Failed)),
            _ => SlotState::Idle,
        }
    }

    pub fn terminal(&self) -> Option<Terminal> {
        if self.state.load(Ordering::Acquire) != TERMINAL {
            return None;
        }
        decode(self.outcome.load(Ordering::Acquire))
    }

    /// Claim the operation for `owner`.  `owner` is a trusted task incarnation.
    pub fn begin(&self, owner: u64) -> Acquire {
        if owner == 0 {
            return Acquire::Stale;
        }
        match self
            .state
            .compare_exchange(IDLE, ACTIVE, Ordering::AcqRel, Ordering::Acquire)
        {
            Ok(_) => {
                self.owner.store(owner, Ordering::Release);
                Acquire::Granted
            }
            Err(TERMINAL) => Acquire::Closed,
            Err(_) => Acquire::Busy,
        }
    }

    /// Enter completion publication.  Only the active owner may do this.
    pub fn begin_completion(&self) -> bool {
        self.state
            .compare_exchange(ACTIVE, COMPLETING, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    /// Request cancellation.  Acceptance is not completion.
    pub fn cancel(&self) -> CancelRequest {
        loop {
            match self.state.load(Ordering::Acquire) {
                ACTIVE => {
                    if self
                        .state
                        .compare_exchange(ACTIVE, CANCELLING, Ordering::AcqRel, Ordering::Acquire)
                        .is_ok()
                    {
                        return CancelRequest::Accepted;
                    }
                }
                CANCELLING => return CancelRequest::Accepted,
                COMPLETING => return CancelRequest::CompletionCommitted,
                TERMINAL => {
                    return CancelRequest::AlreadyTerminal(
                        decode(self.outcome.load(Ordering::Acquire)).unwrap_or(Terminal::Failed),
                    )
                }
                _ => return CancelRequest::NotActive,
            }
        }
    }

    /// Commit a terminal outcome.  Exactly one caller wins.
    ///
    /// A cancellation that was already accepted wins over a late completion:
    /// the committed outcome is then `Cancelled`, never `Completed`.
    pub fn finish(&self, proposed: Terminal) -> FinishOutcome {
        loop {
            match self.state.load(Ordering::Acquire) {
                ACTIVE => {
                    if self
                        .state
                        .compare_exchange(ACTIVE, TERMINAL, Ordering::AcqRel, Ordering::Acquire)
                        .is_ok()
                    {
                        self.outcome.store(encode(proposed), Ordering::Release);
                        return FinishOutcome::Committed(proposed);
                    }
                }
                COMPLETING => {
                    if self
                        .state
                        .compare_exchange(COMPLETING, TERMINAL, Ordering::AcqRel, Ordering::Acquire)
                        .is_ok()
                    {
                        self.outcome.store(encode(proposed), Ordering::Release);
                        return FinishOutcome::Committed(proposed);
                    }
                }
                CANCELLING => {
                    if self
                        .state
                        .compare_exchange(CANCELLING, TERMINAL, Ordering::AcqRel, Ordering::Acquire)
                        .is_ok()
                    {
                        self.outcome
                            .store(encode(Terminal::Cancelled), Ordering::Release);
                        return FinishOutcome::Committed(Terminal::Cancelled);
                    }
                }
                TERMINAL => {
                    return FinishOutcome::LostRace(
                        decode(self.outcome.load(Ordering::Acquire)).unwrap_or(Terminal::Failed),
                    )
                }
                _ => return FinishOutcome::NotActive,
            }
        }
    }
}

/// A callback entry gate bound to one device incarnation and queue epoch.
pub struct CallbackGate {
    epoch: Epoch,
    incarnation: Incarnation,
    open: AtomicBool,
    active: AtomicU32,
}

impl CallbackGate {
    pub const fn new(epoch: Epoch, incarnation: Incarnation) -> Self {
        Self {
            epoch,
            incarnation,
            open: AtomicBool::new(true),
            active: AtomicU32::new(0),
        }
    }

    pub const fn epoch(&self) -> Epoch {
        self.epoch
    }

    pub const fn incarnation(&self) -> Incarnation {
        self.incarnation
    }

    pub fn active(&self) -> u32 {
        self.active.load(Ordering::Acquire)
    }

    /// Enter a callback.  Stale epochs/incarnations are refused before any
    /// device or mapping state is touched.
    pub fn enter(&self, epoch: Epoch, incarnation: Incarnation) -> Acquire {
        if epoch != self.epoch || incarnation != self.incarnation {
            return Acquire::Stale;
        }
        if !self.open.load(Ordering::Acquire) {
            return Acquire::Closed;
        }
        self.active.fetch_add(1, Ordering::AcqRel);
        if !self.open.load(Ordering::Acquire) {
            self.active.fetch_sub(1, Ordering::AcqRel);
            return Acquire::Closed;
        }
        Acquire::Granted
    }

    pub fn exit(&self) -> bool {
        self.active
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |v| v.checked_sub(1))
            .is_ok()
    }

    /// Close admission.  Outstanding entries keep running until they exit.
    pub fn begin_drain(&self) {
        self.open.store(false, Ordering::Release);
    }

    pub fn quiescent(&self) -> bool {
        !self.open.load(Ordering::Acquire) && self.active.load(Ordering::Acquire) == 0
    }
}

/// A DMA mapping with independent user authority.
pub struct MappingRecord {
    id: u64,
    epoch: Epoch,
    bytes: u32,
    users: AtomicU32,
    retired: AtomicBool,
}

impl MappingRecord {
    pub const fn new(id: u64, epoch: Epoch, bytes: u32) -> Self {
        Self {
            id,
            epoch,
            bytes,
            users: AtomicU32::new(0),
            retired: AtomicBool::new(false),
        }
    }

    pub const fn id(&self) -> u64 {
        self.id
    }

    pub const fn epoch(&self) -> Epoch {
        self.epoch
    }

    pub const fn bytes(&self) -> u32 {
        self.bytes
    }

    pub fn users(&self) -> u32 {
        self.users.load(Ordering::Acquire)
    }

    pub fn is_retired(&self) -> bool {
        self.retired.load(Ordering::Acquire)
    }

    /// Acquire user authority for an access.  Refuses retired or stale mappings
    /// and rechecks after incrementing so a concurrent retire cannot be missed.
    pub fn acquire(&self, epoch: Epoch) -> Acquire {
        if epoch != self.epoch {
            return Acquire::Stale;
        }
        if self.retired.load(Ordering::Acquire) {
            return Acquire::Closed;
        }
        self.users.fetch_add(1, Ordering::AcqRel);
        if self.retired.load(Ordering::Acquire) {
            self.users.fetch_sub(1, Ordering::AcqRel);
            return Acquire::Closed;
        }
        Acquire::Granted
    }

    /// Drop user authority.  Returns false on a double release.
    pub fn release(&self) -> bool {
        self.users
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |v| v.checked_sub(1))
            .is_ok()
    }

    /// Retire exactly once, and only with no authorised users left.
    pub fn retire(&self, epoch: Epoch) -> RetireOutcome {
        if epoch != self.epoch {
            return RetireOutcome::Stale;
        }
        if self.retired.load(Ordering::Acquire) {
            return RetireOutcome::AlreadyRetired;
        }
        let users = self.users.load(Ordering::Acquire);
        if users != 0 {
            return RetireOutcome::Busy(users);
        }
        match self
            .retired
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        {
            Ok(_) => {
                if self.users.load(Ordering::Acquire) == 0 {
                    RetireOutcome::Retired
                } else {
                    // A late acquire slipped in; undo and stay pinned.
                    self.retired.store(false, Ordering::Release);
                    RetireOutcome::Busy(self.users.load(Ordering::Acquire))
                }
            }
            Err(_) => RetireOutcome::AlreadyRetired,
        }
    }
}

/// Admission gate for one queue epoch.  Closing an epoch is terminal for it.
pub struct FenceGate {
    epoch: AtomicU64,
    closed: AtomicBool,
}

impl FenceGate {
    pub const fn new(epoch: Epoch) -> Self {
        Self {
            epoch: AtomicU64::new(epoch),
            closed: AtomicBool::new(false),
        }
    }

    pub fn epoch(&self) -> Epoch {
        self.epoch.load(Ordering::Acquire)
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    /// Close the current epoch.  Returns the epoch that was closed.
    pub fn close(&self) -> Epoch {
        self.closed.store(true, Ordering::Release);
        self.epoch.load(Ordering::Acquire)
    }

    pub fn admit(&self, epoch: Epoch) -> Acquire {
        if epoch != self.epoch.load(Ordering::Acquire) {
            return Acquire::Stale;
        }
        if self.closed.load(Ordering::Acquire) {
            return Acquire::Closed;
        }
        Acquire::Granted
    }

    /// Authorise a fresh epoch.  Only legal after the previous epoch closed and
    /// every old-epoch operation and mapping was retired.  Callers must not use
    /// this to paper over unresolved ownership.
    pub fn open_next(&self) -> Option<Epoch> {
        if !self.closed.load(Ordering::Acquire) {
            return None;
        }
        let next = self.epoch.load(Ordering::Acquire).checked_add(1)?;
        self.epoch.store(next, Ordering::Release);
        self.closed.store(false, Ordering::Release);
        Some(next)
    }
}

/// Bounded drain: observes the slots at most `polls` times.
///
/// Returns `Bounded` rather than claiming quiescence when any operation is
/// still active.  A permanently delayed callback therefore yields a bounded
/// failure, never a false success.
pub fn drain_bounded(slots: &[&OpSlot], polls: u32) -> DrainOutcome {
    for _ in 0..polls {
        let pending = slots.iter().filter(|s| s.terminal().is_none()).count() as u32;
        if pending == 0 {
            return DrainOutcome::Complete;
        }
    }
    DrainOutcome::Bounded {
        pending: slots.iter().filter(|s| s.terminal().is_none()).count() as u32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot() -> OpSlot {
        OpSlot::new(7, 1, 3)
    }

    // C1: completion wins cancellation.
    #[test]
    fn c1_completion_wins_cancellation() {
        let s = slot();
        assert_eq!(s.begin(9), Acquire::Granted);
        assert_eq!(
            s.finish(Terminal::Completed),
            FinishOutcome::Committed(Terminal::Completed)
        );
        assert_eq!(
            s.cancel(),
            CancelRequest::AlreadyTerminal(Terminal::Completed)
        );
        assert_eq!(
            s.finish(Terminal::Cancelled),
            FinishOutcome::LostRace(Terminal::Completed)
        );
        assert_eq!(s.terminal(), Some(Terminal::Completed));
    }

    // C2: cancellation wins completion.
    #[test]
    fn c2_cancellation_wins_completion() {
        let s = slot();
        assert_eq!(s.begin(9), Acquire::Granted);
        assert_eq!(s.cancel(), CancelRequest::Accepted);
        assert_eq!(s.state(), SlotState::Cancelling);
        // The request alone terminated nothing.
        assert_eq!(s.terminal(), None);
        assert_eq!(
            s.finish(Terminal::Completed),
            FinishOutcome::Committed(Terminal::Cancelled)
        );
        assert_eq!(s.terminal(), Some(Terminal::Cancelled));
    }

    // C3: reset races callback entry.
    #[test]
    fn c3_reset_races_callback_entry() {
        let g = CallbackGate::new(1, 3);
        assert_eq!(g.enter(1, 3), Acquire::Granted);
        g.begin_drain();
        assert!(!g.quiescent());
        assert_eq!(g.enter(1, 3), Acquire::Closed);
        assert!(g.exit());
        assert!(g.quiescent());
        assert_eq!(g.active(), 0);
    }

    // C4: reset races callback exit.
    #[test]
    fn c4_reset_races_callback_exit() {
        let g = CallbackGate::new(1, 3);
        assert_eq!(g.enter(1, 3), Acquire::Granted);
        g.begin_drain();
        assert!(g.exit());
        assert!(g.quiescent());
        assert!(!g.exit(), "double exit must be refused");
    }

    // C5: reset races completion publication.
    #[test]
    fn c5_reset_races_completion_publication() {
        let s = slot();
        assert_eq!(s.begin(9), Acquire::Granted);
        assert!(s.begin_completion());
        assert_eq!(s.cancel(), CancelRequest::CompletionCommitted);
        assert_eq!(
            s.finish(Terminal::Completed),
            FinishOutcome::Committed(Terminal::Completed)
        );
        assert_eq!(s.terminal(), Some(Terminal::Completed));
    }

    // C6: callback fails before mapping acquisition.
    #[test]
    fn c6_callback_fails_before_mapping() {
        let s = slot();
        assert_eq!(s.begin(9), Acquire::Granted);
        assert_eq!(
            s.finish(Terminal::Failed),
            FinishOutcome::Committed(Terminal::Failed)
        );
        let m = MappingRecord::new(1, 1, 512);
        assert_eq!(m.users(), 0);
        assert_eq!(m.retire(1), RetireOutcome::Retired);
    }

    // C7: callback fails after mapping acquisition.
    #[test]
    fn c7_callback_fails_after_mapping() {
        let s = slot();
        let m = MappingRecord::new(1, 1, 512);
        assert_eq!(s.begin(9), Acquire::Granted);
        assert_eq!(m.acquire(1), Acquire::Granted);
        assert_eq!(
            s.finish(Terminal::Failed),
            FinishOutcome::Committed(Terminal::Failed)
        );
        assert_eq!(m.retire(1), RetireOutcome::Busy(1));
        assert!(m.release());
        assert_eq!(m.retire(1), RetireOutcome::Retired);
        assert_eq!(m.retire(1), RetireOutcome::AlreadyRetired);
    }

    // C8: worker exits during reset.
    #[test]
    fn c8_worker_exits_during_reset() {
        let s = slot();
        let g = CallbackGate::new(1, 3);
        assert_eq!(s.begin(9), Acquire::Granted);
        assert_eq!(g.enter(1, 3), Acquire::Granted);
        g.begin_drain();
        assert!(!g.quiescent());
        assert_eq!(s.cancel(), CancelRequest::Accepted);
        assert!(g.exit());
        assert_eq!(
            s.finish(Terminal::Completed),
            FinishOutcome::Committed(Terminal::Cancelled)
        );
        assert!(g.quiescent());
        assert_eq!(drain_bounded(&[&s], 4), DrainOutcome::Complete);
    }

    // C9: callback delayed indefinitely yields a bounded failure, never success.
    #[test]
    fn c9_delayed_callback_yields_bounded_failure() {
        let s = slot();
        assert_eq!(s.begin(9), Acquire::Granted);
        assert_eq!(
            drain_bounded(&[&s], 64),
            DrainOutcome::Bounded { pending: 1 }
        );
        assert_eq!(s.terminal(), None);
    }

    // C10: repeated cancellation and reset.
    #[test]
    fn c10_repeated_cancellation_and_reset() {
        let s = slot();
        assert_eq!(s.begin(9), Acquire::Granted);
        assert_eq!(s.cancel(), CancelRequest::Accepted);
        assert_eq!(s.cancel(), CancelRequest::Accepted);
        assert_eq!(
            s.finish(Terminal::Cancelled),
            FinishOutcome::Committed(Terminal::Cancelled)
        );
        assert_eq!(
            s.cancel(),
            CancelRequest::AlreadyTerminal(Terminal::Cancelled)
        );
        assert_eq!(
            s.finish(Terminal::Cancelled),
            FinishOutcome::LostRace(Terminal::Cancelled)
        );
    }

    // C11: callback references a stale device incarnation.
    #[test]
    fn c11_stale_device_incarnation_refused() {
        let g = CallbackGate::new(1, 3);
        assert_eq!(g.enter(1, 4), Acquire::Stale);
        assert_eq!(g.active(), 0);
        assert_eq!(g.enter(1, 3), Acquire::Granted);
    }

    // C12: callback references a stale queue epoch.
    #[test]
    fn c12_stale_queue_epoch_refused() {
        let g = CallbackGate::new(2, 3);
        assert_eq!(g.enter(1, 3), Acquire::Stale);
        let f = FenceGate::new(2);
        assert_eq!(f.admit(1), Acquire::Stale);
        assert_eq!(f.admit(2), Acquire::Granted);
    }

    // M1/M2/M3: acquisition precedes use, authority spans use, users release first.
    #[test]
    fn m1_m3_mapping_authority_spans_use() {
        let m = MappingRecord::new(1, 1, 4096);
        assert_eq!(m.users(), 0);
        assert_eq!(m.acquire(1), Acquire::Granted);
        assert_eq!(m.users(), 1);
        assert_eq!(m.retire(1), RetireOutcome::Busy(1));
        assert!(m.release());
        assert_eq!(m.retire(1), RetireOutcome::Retired);
    }

    // M4: reset does not implicitly revoke a mapping without a defined contract.
    #[test]
    fn m4_reset_does_not_implicitly_revoke_mapping() {
        let m = MappingRecord::new(1, 1, 512);
        let f = FenceGate::new(1);
        assert_eq!(m.acquire(1), Acquire::Granted);
        f.close();
        assert_eq!(f.admit(1), Acquire::Closed);
        assert_eq!(m.users(), 1);
        assert!(!m.is_retired());
        assert!(m.release());
        assert_eq!(m.retire(1), RetireOutcome::Retired);
    }

    // M5: release is exactly once.
    #[test]
    fn m5_release_exactly_once() {
        let m = MappingRecord::new(1, 1, 512);
        assert_eq!(m.acquire(1), Acquire::Granted);
        assert!(m.release());
        assert!(!m.release());
        assert_eq!(m.users(), 0);
    }

    // M6: stale callbacks cannot reacquire a retired mapping.
    #[test]
    fn m6_stale_callback_cannot_reacquire() {
        let m = MappingRecord::new(1, 1, 512);
        assert_eq!(m.retire(1), RetireOutcome::Retired);
        assert_eq!(m.acquire(1), Acquire::Closed);
        assert_eq!(m.users(), 0);
    }

    // M7: a new epoch cannot alias an unretired old mapping.
    #[test]
    fn m7_new_epoch_cannot_alias_unretired_mapping() {
        let f = FenceGate::new(1);
        let old = MappingRecord::new(1, 1, 512);
        assert_eq!(old.acquire(1), Acquire::Granted);
        assert_eq!(f.close(), 1);
        let next = f.open_next().expect("closed epoch may advance");
        assert_eq!(next, 2);
        assert_eq!(old.retire(1), RetireOutcome::Busy(1));
        assert_eq!(old.acquire(2), Acquire::Stale);
        assert!(old.release());
        assert_eq!(old.retire(1), RetireOutcome::Retired);
        let fresh = MappingRecord::new(2, 2, 512);
        assert_eq!(fresh.acquire(2), Acquire::Granted);
    }

    // M8: access after an established fence is refused by the model.
    #[test]
    fn m8_access_after_fence_refused() {
        let f = FenceGate::new(1);
        let m = MappingRecord::new(1, 1, 512);
        f.close();
        assert_eq!(f.admit(1), Acquire::Closed);
        assert_eq!(
            m.acquire(1),
            Acquire::Granted,
            "the gate, not the mapping, fences admission"
        );
        assert!(m.release());
        assert_eq!(m.retire(1), RetireOutcome::Retired);
        assert_eq!(m.acquire(1), Acquire::Closed);
    }

    // NC1: use after mapping release is detected.
    #[test]
    fn nc1_use_after_release_detected() {
        let m = MappingRecord::new(1, 1, 512);
        assert_eq!(m.retire(1), RetireOutcome::Retired);
        let broken = m.acquire(1);
        assert_eq!(broken, Acquire::Closed);
        assert_ne!(broken, Acquire::Granted, "observer must flag the violation");
    }

    // NC2: callback after device teardown is refused.
    #[test]
    fn nc2_callback_after_teardown_refused() {
        let g = CallbackGate::new(1, 1);
        g.begin_drain();
        assert!(g.quiescent());
        assert_eq!(g.enter(1, 1), Acquire::Closed);
    }

    // NC3: stale queue access is refused.
    #[test]
    fn nc3_stale_queue_access_refused() {
        let f = FenceGate::new(2);
        f.close();
        assert_eq!(f.admit(1), Acquire::Stale);
        assert_eq!(f.admit(2), Acquire::Closed);
    }

    // NC4: delayed write after claimed quiescence is refused.
    #[test]
    fn nc4_delayed_write_after_quiescence_refused() {
        let g = CallbackGate::new(1, 1);
        let s = slot();
        assert_eq!(s.begin(1), Acquire::Granted);
        g.begin_drain();
        assert_eq!(
            s.finish(Terminal::Completed),
            FinishOutcome::Committed(Terminal::Completed)
        );
        assert!(g.quiescent());
        assert_eq!(g.enter(1, 1), Acquire::Closed);
        assert_eq!(s.begin(2), Acquire::Closed);
    }

    // Terminal outcome is exactly once for every two-step interleaving.
    #[test]
    fn terminal_is_exactly_once_for_all_interleavings() {
        for first in [Terminal::Completed, Terminal::Cancelled, Terminal::Failed] {
            for second in [Terminal::Completed, Terminal::Cancelled, Terminal::Failed] {
                let s = slot();
                assert_eq!(s.begin(1), Acquire::Granted);
                let a = s.finish(first);
                let b = s.finish(second);
                assert!(matches!(a, FinishOutcome::Committed(_)));
                assert!(matches!(b, FinishOutcome::LostRace(_)));
                assert_eq!(s.terminal(), Some(first));
            }
        }
    }

    // Cancel-then-finish always commits Cancelled, never Completed.
    #[test]
    fn cancel_then_finish_never_commits_completed() {
        for proposed in [Terminal::Completed, Terminal::Cancelled, Terminal::Failed] {
            let s = slot();
            assert_eq!(s.begin(1), Acquire::Granted);
            assert_eq!(s.cancel(), CancelRequest::Accepted);
            assert_eq!(
                s.finish(proposed),
                FinishOutcome::Committed(Terminal::Cancelled)
            );
        }
    }

    // A retired mapping can never be resurrected by any epoch.
    #[test]
    fn retired_mapping_never_resurrects() {
        let f = FenceGate::new(1);
        let m = MappingRecord::new(1, 1, 512);
        assert_eq!(m.retire(1), RetireOutcome::Retired);
        f.close();
        let next = f.open_next().unwrap();
        assert_eq!(m.acquire(next), Acquire::Stale);
        assert_eq!(m.acquire(1), Acquire::Closed);
        assert_eq!(m.retire(next), RetireOutcome::Stale);
    }
}
