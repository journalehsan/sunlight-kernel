//! M1 authority, with separately gated M2 backing and M4 notification state.
//! All native callers hold SCHEDULER -> CAP_BROKER. An object handle is never
//! bearer authority: each operation also checks its immutable recipient/role.
use super::{CapabilityBroker, CapabilityToken, RecipientIdentity};
use alloc::vec::Vec;
use sunlight_ipc::account_session::SessionRef;

pub(crate) const LIMIT: usize = 8;
const MAX_TTL: u64 = 30_000;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub(crate) enum Error {
    Denied = 1,
    Stale = 2,
    Duplicate = 3,
    Terminal = 4,
    Capacity = 5,
    Exhausted = 6,
    Incomplete = 7,
    #[cfg(any(test, feature = "fastlane_doorbell_test"))]
    Cancelled = 9,
    #[cfg(any(test, feature = "fastlane_doorbell_test"))]
    Changed = 10,
    #[cfg(any(
        test,
        feature = "fastlane_doorbell_test",
        feature = "restricted_remote_test"
    ))]
    Pending = 11,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct Handle {
    pub token: CapabilityToken,
    pub generation: u64,
}
impl core::fmt::Debug for Handle {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Handle")
            .field("token", &"REDACTED")
            .field("generation", &self.generation)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub(crate) enum State {
    Proposed,
    Paired,
    Created,
    Attached,
    Retired,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub(crate) enum Terminal {
    Revoked = 1,
    ProcessRetired,
    SessionRetired,
    Expired,
    IdentityChanged,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RoleState {
    Issued,
    Attached,
}

pub(super) struct Channel {
    pub handle: Handle,
    pub issuer: RecipientIdentity,
    pub session: SessionRef,
    pub creator: RecipientIdentity, // also the sole producer; never reassigned
    pub consumer: Option<RecipientIdentity>,
    // One-use pair confirmation, delivered only to the consenting requester.
    pub consumer_receipt: Option<CapabilityToken>,
    pub producer_role: RoleState,
    pub consumer_role: RoleState,
    pub state: State,
    pub terminal: Option<Terminal>,
    pub deadline: u64,
    // Kernel-private SHM identities; never exported as bearer grants.
    pub backing: [Option<CapabilityToken>; 3],
    #[cfg(any(test, all(debug_assertions, feature = "fastlane_contract_test")))]
    pub contract: super::channel_contract::Contract,
    // M5 single use: 0 writable, 1 published, 2 acknowledged. Never reset.
    #[cfg(feature = "fastlane_transfer_test")]
    pub transfer: u8,
    #[cfg(feature = "fastlane_shared_ring_test")]
    pub native_ring: [bool; 2],
    #[cfg(any(test, feature = "fastlane_repeated_test"))]
    pub repeated: super::channel_repeated::Repeated,
    #[cfg(any(test, feature = "fastlane_doorbell_test"))]
    pub bell: Option<sunlight_fastlane_doorbell::Doorbell>,
    #[cfg(any(test, feature = "fastlane_doorbell_test"))]
    pub bell_pending: Option<sunlight_fastlane_doorbell::Identity>,
    #[cfg(any(test, feature = "fastlane_mapping_test"))]
    pub mapped: [[bool; 3]; 2],
}
impl Channel {
    pub(super) fn live(&self) -> bool {
        self.state != State::Retired
    }
    pub(super) fn retire(&mut self, reason: Terminal) {
        if self.live() {
            #[cfg(all(debug_assertions, feature = "fastlane_contract_test"))]
            self.contract_retire(reason);
            #[cfg(all(test, not(feature = "fastlane_contract_test")))]
            self.contract.retire();
            #[cfg(all(debug_assertions, feature = "fastlane_doorbell_trace"))]
            crate::sched::bell_trace::event("close", self.handle.generation, self.creator.pid,
                self.creator.incarnation, self.creator.vm_generation, reason as u64,
                u64::from(self.bell.as_ref().is_some_and(|b| b.has_waiter())));
            #[cfg(any(test, feature = "fastlane_doorbell_test"))]
            if let Some(bell) = self.bell.as_mut() {
                self.bell_pending = bell.revoke(super::channel_doorbell::key(self.handle), super::channel_doorbell::identity(self.creator)).expect("bound channel owner").wake;
            }
            self.terminal = Some(reason);
            self.consumer_receipt = None;
            self.state = State::Retired;
        }
    }
    fn member(&self, who: RecipientIdentity) -> bool {
        self.creator == who || self.consumer == Some(who)
    }
}
pub(super) struct ChannelTable {
    pub rows: Vec<Channel>,
    #[cfg(any(test, all(debug_assertions, feature = "fastlane_contract_test")))]
    pub policies: super::channel_contract::PolicyCatalog,
    next_generation: u64,
}
impl ChannelTable {
    pub const fn new() -> Self {
        Self {
            rows: Vec::new(),
            #[cfg(any(test, all(debug_assertions, feature = "fastlane_contract_test")))]
            policies: super::channel_contract::PolicyCatalog::new(),
            next_generation: 1,
        }
    }
    pub fn live_count(&self) -> usize {
        self.rows.iter().filter(|r| r.live()).count()
    }
    pub fn token_count(&self) -> usize {
        self.rows
            .iter()
            .filter(|r| r.live())
            .map(|r| 1 + usize::from(r.consumer_receipt.is_some()))
            .sum()
    }
    pub fn vm_member(&self, generation: u64) -> bool {
        self.rows.iter().any(|r| {
            (r.live() || r.backing.iter().any(Option::is_some))
                && (r.creator.vm_generation == generation
                    || r.consumer.is_some_and(|c| c.vm_generation == generation))
        })
    }
    pub fn retire_process(&mut self, pid: usize) {
        #[cfg(any(test, all(debug_assertions, feature = "fastlane_contract_test")))]
        self.policies.retire_process(pid);
        for row in &mut self.rows {
            if row.creator.pid == pid
                || row.consumer.is_some_and(|p| p.pid == pid)
                || row.issuer.pid == pid
            {
                row.retire(Terminal::ProcessRetired);
            }
        }
    }
    pub fn retire_session(&mut self, session: SessionRef) {
        for row in &mut self.rows {
            if row.session == session {
                row.retire(Terminal::SessionRetired);
            }
        }
    }
    pub fn expire(&mut self, now: u64) {
        for row in &mut self.rows {
            if now >= row.deadline {
                row.retire(Terminal::Expired);
            }
        }
    }
    pub(super) fn row_mut(&mut self, h: Handle) -> Result<&mut Channel, Error> {
        self.rows
            .iter_mut()
            .find(|r| r.handle == h)
            .ok_or(Error::Stale)
    }
    fn occupied(&self, who: RecipientIdentity) -> bool {
        self.rows.iter().any(|r| r.live() && r.member(who))
    }
}
impl CapabilityBroker {
    /// New explicit policy: an active root Account session may authorize the
    /// actual synchronous requester's producer proposal, then another actual
    /// requester's consumer consent to that exact proposal. No PID selection.
    pub(super) fn channel_propose(
        &mut self,
        issuer: RecipientIdentity,
        producer: RecipientIdentity,
        session: SessionRef,
        uid: u32,
        now: u64,
        ttl: u64,
    ) -> Result<Handle, Error> {
        self.expire_sessions(now);
        self.channels.expire(now);
        if !self.channel_session_authorized(session, issuer, uid, now)
            || issuer == producer
            || producer.incarnation == 0
            || producer.vm_generation == 0
            || ttl == 0
            || ttl > MAX_TTL
        {
            return Err(Error::Denied);
        }
        if self.channels.occupied(producer) {
            return Err(Error::Duplicate);
        }
        let slot = self.channels.rows.iter().position(|r| {
            #[cfg(any(test, all(debug_assertions, feature = "fastlane_contract_test")))]
            if r.contract.hybrid_reply.is_some() { return false; }
            #[cfg(any(test, feature = "fastlane_doorbell_test"))]
            if r.bell_pending.is_some() { return false; }
            !r.live() && r.backing.iter().all(Option::is_none)
        });
        if slot.is_none() && self.channels.rows.len() >= LIMIT {
            return Err(Error::Capacity);
        }
        let deadline = now.checked_add(ttl).ok_or(Error::Exhausted)?;
        let next = self
            .channels
            .next_generation
            .checked_add(1)
            .ok_or(Error::Exhausted)?;
        self.check_capacity(1).map_err(|_| Error::Capacity)?;
        if self.channels.rows.capacity() == 0 {
            self.channels
                .rows
                .try_reserve_exact(LIMIT)
                .map_err(|_| Error::Capacity)?;
        }
        // The broker's existing entropy + boot-lifetime nonreuse ledger issues
        // the only unpredictable identity. Failure publishes no channel/role.
        let token = self
            .issue_token(CapabilityToken::TAG_FASTLANE)
            .map_err(|_| Error::Capacity)?;
        let handle = Handle {
            token,
            generation: self.channels.next_generation,
        };
        self.channels.next_generation = next;
        let row = Channel {
            handle,
            issuer,
            session,
            creator: producer,
            consumer: None,
            consumer_receipt: None,
            producer_role: RoleState::Issued,
            consumer_role: RoleState::Issued,
            state: State::Proposed,
            terminal: None,
            deadline,
            backing: [None; 3],
            #[cfg(any(test, all(debug_assertions, feature = "fastlane_contract_test")))]
            contract: super::channel_contract::Contract::new(),
            #[cfg(feature = "fastlane_transfer_test")]
            transfer: 0,
            #[cfg(feature = "fastlane_shared_ring_test")]
            native_ring: [false; 2],
            #[cfg(any(test, feature = "fastlane_repeated_test"))]
            repeated: super::channel_repeated::Repeated::new(),
            #[cfg(any(test, feature = "fastlane_doorbell_test"))]
            bell: None,
            #[cfg(any(test, feature = "fastlane_doorbell_test"))]
            bell_pending: None,
            #[cfg(any(test, feature = "fastlane_mapping_test"))]
            mapped: [[false; 3]; 2],
        };
        if let Some(i) = slot {
            self.channels.rows[i] = row;
        } else {
            self.channels.rows.push(row);
        }
        Ok(handle)
    }
    pub(super) fn channel_consent(
        &mut self,
        h: Handle,
        issuer: RecipientIdentity,
        consumer: RecipientIdentity,
        session: SessionRef,
        uid: u32,
        now: u64,
    ) -> Result<CapabilityToken, Error> {
        self.expire_sessions(now);
        self.channels.expire(now);
        if !self.channel_session_authorized(session, issuer, uid, now)
            || consumer.incarnation == 0
            || consumer.vm_generation == 0
            || consumer == issuer
        {
            return Err(Error::Denied);
        }
        if self.channels.occupied(consumer) {
            return Err(Error::Duplicate);
        }
        let row = self.channels.row_mut(h)?;
        if !row.live() {
            return Err(Error::Terminal);
        }
        if row.issuer != issuer
            || row.session != session
            || row.creator == consumer
            || row.creator.vm_generation == consumer.vm_generation
        {
            return Err(Error::Denied);
        }
        if row.state != State::Proposed {
            return Err(Error::Duplicate);
        }
        self.check_capacity(1).map_err(|_| Error::Capacity)?;
        let receipt = self
            .issue_token(CapabilityToken::TAG_FASTLANE)
            .map_err(|_| Error::Capacity)?;
        let row = self.channels.row_mut(h)?;
        row.consumer = Some(consumer);
        row.consumer_receipt = Some(receipt);
        row.state = State::Paired;
        Ok(receipt)
    }
    pub(super) fn channel_create(
        &mut self,
        h: Handle,
        who: RecipientIdentity,
        receipt: CapabilityToken,
        now: u64,
    ) -> Result<(), Error> {
        self.channels.expire(now);
        let row = self.channels.row_mut(h)?;
        if row.creator != who {
            return Err(Error::Denied);
        }
        if !row.live() {
            return Err(Error::Terminal);
        }
        if row.state == State::Proposed {
            return Err(Error::Incomplete);
        }
        if row.state != State::Paired {
            return Err(Error::Duplicate);
        }
        if row.consumer_receipt != Some(receipt) {
            return Err(Error::Denied);
        }
        row.consumer_receipt = None; // consumed exactly once; history is retained
        row.state = State::Created;
        Ok(())
    }
    pub(super) fn channel_attach(
        &mut self,
        h: Handle,
        who: RecipientIdentity,
        role: u64,
        now: u64,
    ) -> Result<(), Error> {
        self.channels.expire(now);
        let row = self.channels.row_mut(h)?;
        if !row.live() {
            return Err(Error::Terminal);
        }
        if !(role == 0 && row.creator == who || role == 1 && row.consumer == Some(who)) {
            return Err(Error::Denied);
        }
        if !matches!(row.state, State::Created | State::Attached) {
            return Err(Error::Incomplete);
        }
        let state = if role == 0 {
            &mut row.producer_role
        } else {
            &mut row.consumer_role
        };
        if *state == RoleState::Attached {
            return Err(Error::Duplicate);
        }
        *state = RoleState::Attached;
        if row.producer_role == RoleState::Attached && row.consumer_role == RoleState::Attached {
            row.state = State::Attached;
        }
        Ok(())
    }
    #[cfg(any(test, feature = "fastlane_mapping_test"))]
    pub(super) fn channel_mapping_authorize(
        &mut self, h: Handle, who: RecipientIdentity, arg: u64,
    ) -> Result<(usize, usize, bool), Error> {
        if arg >= 6 { return Err(Error::Denied); }
        let role = (arg / 3) as usize;
        let region = (arg % 3) as usize;
        let row = self.channels.row_mut(h)?;
        if !row.live() { return Err(Error::Terminal); }
        if !(role == 0 && row.creator == who || role == 1 && row.consumer == Some(who)) {
            return Err(Error::Denied);
        }
        let attached = if role == 0 { row.producer_role } else { row.consumer_role };
        if attached != RoleState::Attached { return Err(Error::Incomplete); }
        if row.mapped[role][region] { return Err(Error::Duplicate); }
        Ok((role, region, if region == 1 { role == 1 } else { role == 0 }))
    }
    pub(super) fn channel_revoke(
        &mut self,
        h: Handle,
        who: RecipientIdentity,
    ) -> Result<(), Error> {
        let row = self.channels.row_mut(h)?;
        if !row.member(who) {
            return Err(Error::Denied);
        }
        row.retire(Terminal::Revoked);
        Ok(())
    }
    pub(super) fn channel_status(
        &mut self,
        h: Handle,
        who: RecipientIdentity,
        now: u64,
    ) -> Result<u64, Error> {
        self.channels.expire(now);
        let row = self.channels.row_mut(h)?;
        if !row.member(who) {
            return Err(Error::Denied);
        }
        Ok(row.state as u64
            | ((row.terminal.map_or(0, |t| t as u64)) << 8)
            | (u64::from(row.producer_role == RoleState::Attached) << 16)
            | (u64::from(row.consumer_role == RoleState::Attached) << 17))
    }
}
#[cfg(test)]
#[path = "channel_tests.rs"]
mod tests;
