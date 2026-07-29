//! Fixed-purpose Wise Owl delegated caller capabilities.
//!
//! The opaque values are bearer references into this bounded kernel table.
//! Caller identity is captured from the mediator's active IPC reply target;
//! no user-supplied PID, UID, session id, or process name enters a record.

use alloc::vec::Vec;
use spin::Mutex;

pub const MAX_DELEGATIONS_IN_FLIGHT: usize = 32;
pub const MAX_AUTHORITY_PROOFS: usize = 32;
pub const MAX_DELEGATION_LIFETIME_MS: u64 = 5_000;
pub const DELEGATED_OPERATION_WISEOWL_SESSION_ATTESTATION: u64 = 1;

#[derive(Clone, Copy)]
pub struct DelegationRecord {
    pub opaque_id: u64,
    pub integrity_tag: u64,
    pub caller_pid: usize,
    pub caller_process_generation: u64,
    pub caller_uid: u32,
    pub mediator_pid: usize,
    pub mediator_process_generation: u64,
    pub session_id: u64,
    pub session_generation: u64,
    pub issued_at_ms: u64,
    pub expires_at_ms: u64,
}

#[derive(Clone, Copy, Default)]
pub struct ActiveSessionBinding {
    pub session_id: u64,
    pub generation: u64,
    pub uid: u32,
    pub active: bool,
}

pub struct DelegationRegistry {
    records: Vec<DelegationRecord>,
    proofs: Vec<SessionAuthorityProofRecord>,
    active_session: ActiveSessionBinding,
}

impl DelegationRegistry {
    pub const fn new() -> Self {
        Self {
            records: Vec::new(),
            proofs: Vec::new(),
            active_session: ActiveSessionBinding {
                session_id: 0,
                generation: 0,
                uid: 0,
                active: false,
            },
        }
    }

    pub fn active_session(&self) -> ActiveSessionBinding {
        self.active_session
    }

    pub fn set_active_session(&mut self, binding: ActiveSessionBinding) {
        if self.active_session.session_id != binding.session_id
            || self.active_session.generation != binding.generation
            || self.active_session.uid != binding.uid
            || self.active_session.active != binding.active
        {
            self.records.clear();
            self.proofs.clear();
        }
        self.active_session = binding;
    }

    pub fn issue(&mut self, record: DelegationRecord) -> bool {
        self.records
            .retain(|entry| entry.expires_at_ms >= record.issued_at_ms);
        if !self.active_session.active
            || record.session_id != self.active_session.session_id
            || record.session_generation != self.active_session.generation
            || record.caller_uid != self.active_session.uid
            || self.records.len() >= MAX_DELEGATIONS_IN_FLIGHT
        {
            return false;
        }
        self.records.push(record);
        true
    }

    /// Removes before returning: every validation attempt is consuming and
    /// therefore replay-safe even when a later authority check fails.
    pub fn take(&mut self, opaque_id: u64, integrity_tag: u64) -> Option<DelegationRecord> {
        let index = self.records.iter().position(|entry| {
            entry.opaque_id == opaque_id && entry.integrity_tag == integrity_tag
        })?;
        Some(self.records.swap_remove(index))
    }

    pub fn issue_proof(&mut self, proof: SessionAuthorityProofRecord) -> bool {
        self.proofs
            .retain(|entry| entry.expires_at_ms >= proof.issued_at_ms);
        if self.proofs.len() >= MAX_AUTHORITY_PROOFS {
            return false;
        }
        self.proofs.push(proof);
        true
    }

    pub fn take_proof(
        &mut self,
        opaque_id: u64,
        integrity_tag: u64,
    ) -> Option<SessionAuthorityProofRecord> {
        let index = self.proofs.iter().position(|entry| {
            entry.opaque_id == opaque_id && entry.integrity_tag == integrity_tag
        })?;
        Some(self.proofs.swap_remove(index))
    }
}

#[derive(Clone, Copy)]
pub struct SessionAuthorityProofRecord {
    pub opaque_id: u64,
    pub integrity_tag: u64,
    pub caller_pid: usize,
    pub caller_process_generation: u64,
    pub caller_uid: u32,
    pub mediator_pid: usize,
    pub mediator_process_generation: u64,
    pub client_instance_id: u64,
    pub session_id: u64,
    pub session_generation: u64,
    pub issued_at_ms: u64,
    pub expires_at_ms: u64,
}

pub static DELEGATED_CALLERS: Mutex<DelegationRegistry> = Mutex::new(DelegationRegistry::new());

#[cfg(test)]
mod tests {
    use super::*;

    fn binding(generation: u64) -> ActiveSessionBinding {
        ActiveSessionBinding {
            session_id: 7,
            generation,
            uid: 1000,
            active: true,
        }
    }

    fn record(id: u64) -> DelegationRecord {
        DelegationRecord {
            opaque_id: id,
            integrity_tag: id ^ 99,
            caller_pid: 40,
            caller_process_generation: 4,
            caller_uid: 1000,
            mediator_pid: 50,
            mediator_process_generation: 8,
            session_id: 7,
            session_generation: 3,
            issued_at_ms: 10,
            expires_at_ms: 20,
        }
    }

    #[test]
    fn one_shot_take_rejects_replay_and_wrong_tag() {
        let mut table = DelegationRegistry::new();
        table.set_active_session(binding(3));
        assert!(table.issue(record(1)));
        assert!(table.take(1, 7).is_none());
        assert!(table.take(1, 98).is_some());
        assert!(table.take(1, 98).is_none());
    }

    #[test]
    fn session_generation_change_and_logout_revoke_all() {
        let mut table = DelegationRegistry::new();
        table.set_active_session(binding(3));
        assert!(table.issue(record(1)));
        table.set_active_session(binding(4));
        assert!(table.take(1, 98).is_none());
        table.set_active_session(ActiveSessionBinding::default());
        assert!(!table.issue(record(2)));
    }

    #[test]
    fn capacity_is_bounded_and_fails_closed() {
        let mut table = DelegationRegistry::new();
        table.set_active_session(binding(3));
        for id in 1..=MAX_DELEGATIONS_IN_FLIGHT as u64 {
            assert!(table.issue(record(id)));
        }
        assert!(!table.issue(record(100)));
    }
}
