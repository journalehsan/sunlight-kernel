//! Session authority, always under SCHEDULER -> CAP_BROKER in production.
//! No callback, IPC, waiting, or user-memory access occurs in this module.
use super::{CapabilityBroker, CapabilityToken, GrantDelivery, RecipientIdentity};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};
use sunlight_ipc::{
    account_session::SessionRef,
    accounts::{Error, GrantBinding, GrantScope},
};

static NEXT_EPOCH: AtomicU64 = AtomicU64::new(1);

fn allocate_epoch(counter: &AtomicU64) -> Result<u64, Error> {
    counter
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
        .map_err(|_| Error::SessionAuthorityUnavailable)
}
const SESSION_LIMIT: usize = 16;
const REQUEST_LIMIT: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Suspended,
    Active,
}

struct Request {
    delivery: GrantDelivery,
    scope: GrantScope,
    token: CapabilityToken,
}

pub(super) struct Session {
    reference: SessionRef,
    pub(super) owner: RecipientIdentity,
    pub(super) issuer: RecipientIdentity,
    uid: u32,
    gid: u32,
    deadline: u64,
    registration: GrantDelivery,
    state: State,
    requests: Vec<Request>,
}

pub(super) struct SessionTable {
    epoch: u64,
    next: u64,
    rows: Vec<Session>,
    registrations: usize,
    revoked: usize,
    expired: usize,
    owner_retired: usize,
}

impl SessionTable {
    pub(super) const fn new() -> Self {
        Self {
            epoch: 0,
            next: 1,
            rows: Vec::new(),
            registrations: 0,
            revoked: 0,
            expired: 0,
            owner_retired: 0,
        }
    }

    fn reserve(&mut self) -> Result<SessionRef, Error> {
        if self.rows.len() >= SESSION_LIMIT {
            return Err(Error::BrokerUnavailable);
        }
        self.rows
            .try_reserve(1)
            .map_err(|_| Error::BrokerUnavailable)?;
        if self.epoch == 0 {
            self.epoch = allocate_epoch(&NEXT_EPOCH)?;
        }
        let next = self
            .next
            .checked_add(1)
            .ok_or(Error::SessionAuthorityUnavailable)?;
        let reference = SessionRef {
            id: self.next,
            epoch: self.epoch,
        };
        self.next = next; // Never refund identities, even on a later failure.
        Ok(reference)
    }
}

impl CapabilityBroker {
    #[cfg(feature = "native_session_race_test")]
    pub(super) fn session_metadata_observation(&self) -> (usize, usize) {
        let bytes = self.sessions.rows.capacity() * core::mem::size_of::<Session>()
            + self
                .sessions
                .rows
                .iter()
                .map(|s| s.requests.capacity() * core::mem::size_of::<Request>())
                .sum::<usize>();
        // These are stored identity values, not allocator or process Arc pins.
        (self.sessions.rows.len() * 2, bytes)
    }

    /// The scheduler must validate both service identities and the exact login
    /// request before entering. Reservation precedes one-use proof consumption.
    pub(crate) fn register_account_session(
        &mut self,
        owner: RecipientIdentity,
        issuer: RecipientIdentity,
        request: GrantDelivery,
        proof: CapabilityToken,
        now: u64,
        deadline: u64,
    ) -> Result<(SessionRef, u32, u32), Error> {
        self.expire_sessions(now);
        if let Some(s) = self
            .sessions
            .rows
            .iter()
            .find(|s| s.owner == owner && s.issuer == issuer && s.registration == request)
        {
            return Ok((s.reference, s.uid, s.gid));
        }
        let g = self
            .auth_session_grants
            .iter()
            .find(|g| g.token == proof)
            .ok_or(Error::CapabilityRejected)?;
        if !g.delivery.is_some_and(|d| {
            d.usable(request.recipient)
                && d.issuer_pid == issuer.pid
                && d.issuer_incarnation == issuer.incarnation
                && d.issuer_vm_generation == issuer.vm_generation
        }) || now > g.expires_at_tick
            || now >= deadline
        {
            return Err(Error::CapabilityRejected);
        }
        let (uid, gid) = (g.uid, g.gid);
        let reference = self.sessions.reserve()?;
        self.consume_live_auth_grant(proof, request.recipient, now)
            .ok_or(Error::CapabilityRejected)?;
        self.sessions.registrations = self.sessions.registrations.saturating_add(1);
        self.sessions.rows.push(Session {
            reference,
            owner,
            issuer,
            uid,
            gid,
            deadline,
            registration: request,
            state: State::Suspended,
            requests: Vec::new(),
        });
        Ok((reference, uid, gid))
    }

    pub(super) fn registration_issuer(
        &self,
        owner: RecipientIdentity,
        request: GrantDelivery,
    ) -> Option<RecipientIdentity> {
        self.sessions
            .rows
            .iter()
            .find(|s| s.owner == owner && s.registration == request)
            .map(|s| s.issuer)
    }

    pub(crate) fn account_session_participants(
        &self,
        reference: SessionRef,
    ) -> Option<(RecipientIdentity, RecipientIdentity)> {
        self.sessions
            .rows
            .iter()
            .find(|s| s.reference == reference)
            .map(|s| (s.owner, s.issuer))
    }

    /// Missing terminal records already confer no authority; revoke is
    /// idempotent. No operation can recreate a removed identity.
    pub(crate) fn transition_account_session(
        &mut self,
        owner: RecipientIdentity,
        reference: SessionRef,
        operation: u64,
        now: u64,
    ) -> Result<(), Error> {
        use sunlight_ipc::account_session::{ACTIVATE, REVOKE, SUSPEND};
        if !matches!(operation, ACTIVATE | SUSPEND | REVOKE) {
            return Err(Error::InvalidInput);
        }
        self.expire_sessions(now);
        let Some(index) = self
            .sessions
            .rows
            .iter()
            .position(|s| s.reference == reference)
        else {
            return if operation == REVOKE {
                Ok(())
            } else {
                Err(Error::SessionChanged)
            };
        };
        if self.sessions.rows[index].owner != owner {
            return Err(Error::Denied);
        }
        if operation == REVOKE {
            self.sessions.revoked = self.sessions.revoked.saturating_add(1);
            self.remove_session(index);
        } else {
            self.sessions.rows[index].state = if operation == ACTIVATE {
                State::Active
            } else {
                State::Suspended
            };
            if operation == SUSPEND {
                self.remove_session_grants(reference);
            }
        }
        Ok(())
    }

    fn remove_session_grants(&mut self, reference: SessionRef) {
        self.account_grants.retain(|(_, g)| {
            let scope = g.grant.binding.scope;
            scope.session != reference.id || scope.session_generation != reference.epoch
        });
    }

    fn remove_session(&mut self, index: usize) {
        let reference = self.sessions.rows.swap_remove(index).reference;
        self.remove_session_grants(reference);
    }

    pub(crate) fn expire_sessions(&mut self, now: u64) {
        let mut index = 0;
        while index < self.sessions.rows.len() {
            if now >= self.sessions.rows[index].deadline {
                self.sessions.expired = self.sessions.expired.saturating_add(1);
                self.remove_session(index);
            } else {
                index += 1;
            }
        }
    }

    pub(super) fn retire_sessions_for(&mut self, pid: usize) {
        let mut index = 0;
        while index < self.sessions.rows.len() {
            let s = &self.sessions.rows[index];
            if s.owner.pid == pid || s.issuer.pid == pid {
                #[cfg(feature = "native_session_race_test")]
                let retired = (s.reference, s.issuer);
                self.sessions.owner_retired = self.sessions.owner_retired.saturating_add(1);
                self.remove_session(index);
                #[cfg(feature = "native_session_race_test")]
                super::session_h3_trace::record(20, retired.1.pid as u64, retired.1.incarnation, retired.0.id, retired.0.epoch, pid as u64);
                #[cfg(feature = "native_session_h4_test")]
                super::session_h4::retired(pid);
                #[cfg(feature = "native_session_race_test")]
                {
                    crate::serial_println!("[SESSION-E3] seq={} checkpoint=service-retirement session={} epoch={} account_pid={} account_inc={} account_vm={} retiring_pid={} cpu={} account_grants={}", super::session_race::sequence(), retired.0.id, retired.0.epoch, retired.1.pid, retired.1.incarnation, retired.1.vm_generation, pid, crate::sched::current_cpu_id(), self.account_grants.len());
                    super::session_race::inventory(self);
                }
            } else {
                self.sessions.rows[index]
                    .requests
                    .retain(|r| r.delivery.recipient.pid != pid);
                index += 1;
            }
        }
    }

    /// Authorization, retry receipt, token preparation, insertion and ownership
    /// attachment are one transaction under the caller's broker guard.
    pub(crate) fn commit_account_session_grant(
        &mut self,
        issuer: RecipientIdentity,
        delivery: GrantDelivery,
        uid: u32,
        binding: GrantBinding,
        consume: Option<CapabilityToken>,
        now: u64,
    ) -> Result<CapabilityToken, Error> {
        self.expire_sessions(now);
        let reference = SessionRef {
            id: binding.scope.session,
            epoch: binding.scope.session_generation,
        };
        let index = self
            .sessions
            .rows
            .iter()
            .position(|s| s.reference == reference)
            .ok_or(Error::SessionAuthorityUnavailable)?;
        let s = &self.sessions.rows[index];
        if s.issuer != issuer || s.state != State::Active || s.uid != uid {
            return Err(Error::Denied);
        }
        use sunlight_ipc::accounts::Operation;
        match binding.scope.operation {
            Operation::ChangeOwnPassword | Operation::UpdateOwnProfile
                if binding.scope.target == uid && binding.scope.member == uid => {}
            Operation::GroupCreate
            | Operation::GroupAddMember
            | Operation::GroupRemoveMember
            | Operation::GroupDelete
                if uid == 0 => {}
            _ => return Err(Error::Denied),
        }
        if let Some(token) = consume {
            return self
                .consume_live_account_grant(token, delivery, binding, now)
                .then_some(token)
                .ok_or(Error::CapabilityRejected);
        }
        // Keep one receipt per recipient incarnation, not per operation. A
        // monotonic call generation replaces its predecessor only after commit.
        let receipt = s
            .requests
            .iter()
            .position(|r| r.delivery.recipient == delivery.recipient);
        if let Some(position) = receipt {
            let r = &s.requests[position];
            if delivery.request <= r.delivery.request {
                return if r.delivery == delivery
                    && r.scope == binding.scope
                    && self
                        .account_grants
                        .iter()
                        .any(|(t, g)| *t == r.token && now < g.grant.expires)
                {
                    Ok(r.token)
                } else {
                    Err(Error::ConcurrentChange)
                };
            }
        } else {
            if s.requests.len() >= REQUEST_LIMIT {
                return Err(Error::BrokerUnavailable);
            }
            self.sessions.rows[index]
                .requests
                .try_reserve(1)
                .map_err(|_| Error::BrokerUnavailable)?;
        }
        let token = self
            .mint_account_grant(binding, now)
            .ok_or(Error::BrokerUnavailable)?;
        self.account_grants
            .last_mut()
            .ok_or(Error::BrokerUnavailable)?
            .1
            .delivery = Some(delivery);
        let receipt_value = Request {
            delivery,
            scope: binding.scope,
            token,
        };
        if let Some(position) = receipt {
            self.sessions.rows[index].requests[position] = receipt_value;
        } else {
            self.sessions.rows[index].requests.push(receipt_value);
        }
        Ok(token)
    }

    #[cfg(any(
        test,
        feature = "cap_issuance_test",
        feature = "production_grant_test",
        feature = "native_session_race_test"
    ))]
    pub(crate) fn session_accounting(&self) -> [usize; 8] {
        [
            self.sessions.rows.len(),
            self.sessions.rows.capacity(),
            self.sessions.registrations,
            self.sessions.revoked,
            self.sessions.expired,
            self.sessions.owner_retired,
            self.sessions.rows.iter().map(|s| s.requests.len()).sum(),
            self.account_grants.len(),
        ]
    }

    #[cfg(any(test, feature = "cap_issuance_test"))]
    pub(crate) fn session_inventory(&self) -> (usize, usize, usize) {
        (
            self.sessions.rows.len(),
            self.sessions.rows.iter().map(|s| s.requests.len()).sum(),
            self.account_grants.len(),
        )
    }
}

#[cfg(test)]
impl CapabilityBroker {
    pub(super) fn exhaust_session_ids_for_test(&mut self) {
        self.sessions.next = u64::MAX;
    }
    pub(super) fn reserve_session_for_test(&mut self) -> Result<SessionRef, Error> {
        self.sessions.reserve()
    }
}

/// Sequential native boundary witness only. It uses an isolated production
/// broker and synthetic identities; it does not certify service authentication
/// or remote commit/revocation races. Never install roles on live services.
#[cfg(feature = "cap_issuance_test")]
pub(crate) fn native_sequential_probe() {
    use sunlight_ipc::account_session::{ACTIVATE, REVOKE};
    use sunlight_ipc::accounts::Operation;
    let _lifecycle = crate::sched::SCHEDULER.lock();
    let issuer = RecipientIdentity {
        pid: 6,
        incarnation: 60,
        vm_generation: 61,
    };
    let owner = RecipientIdentity {
        pid: 8,
        incarnation: 80,
        vm_generation: 81,
    };
    let recipient = RecipientIdentity {
        pid: 7,
        incarnation: 70,
        vm_generation: 71,
    };
    let mut b = CapabilityBroker::new();
    for case in 0..4 {
        let login = b.mint_auth_session_grant(7, 0, 0, 100).unwrap();
        b.auth_session_grants.last_mut().unwrap().delivery = Some(GrantDelivery {
            recipient,
            issuer_pid: 6,
            issuer_incarnation: 60,
            issuer_vm_generation: 61,
            request: case * 2 + 1,
            delivered: true,
        });
        let registration = GrantDelivery {
            recipient,
            issuer_pid: 8,
            issuer_incarnation: 80,
            issuer_vm_generation: 81,
            request: case * 2 + 2,
            delivered: false,
        };
        let (reference, _, _) = b
            .register_account_session(owner, issuer, registration, login, 1, 5)
            .unwrap();
        b.transition_account_session(owner, reference, ACTIVATE, 1)
            .unwrap();
        let delivery = GrantDelivery {
            issuer_pid: 6,
            issuer_incarnation: 60,
            issuer_vm_generation: 61,
            ..registration
        };
        let binding = GrantBinding {
            pid: 7,
            process_generation: 71,
            scope: GrantScope {
                revision: 1,
                operation: Operation::GroupCreate,
                target: 0,
                member: 0,
                session: reference.id,
                session_generation: reference.epoch,
            },
        };
        if case == 0 {
            b.transition_account_session(owner, reference, REVOKE, 1)
                .unwrap();
            assert!(b
                .commit_account_session_grant(issuer, delivery, 0, binding, None, 1)
                .is_err());
        } else {
            let token = b
                .commit_account_session_grant(issuer, delivery, 0, binding, None, 1)
                .unwrap();
            assert!(b
                .commit_account_session_grant(issuer, delivery, 0, binding, None, 1)
                .is_ok());
            match case {
                1 => b
                    .transition_account_session(owner, reference, REVOKE, 1)
                    .unwrap(),
                2 => b.expire_sessions(5),
                _ => b.retire_sessions_for(6),
            }
            assert!(b
                .commit_account_session_grant(issuer, delivery, 0, binding, Some(token), 5)
                .is_err());
        }
        assert_eq!(b.session_inventory(), (0, 0, 0));
        crate::serial_println!("[SESSION-E2] sequential case={} epoch={} session={} issuer_inc=60 owner_inc=80 recipient_inc=70 vm=71 authority=retired live=0 cpu={} scope=isolated-production-broker authentication=NOT_TESTED", case, reference.epoch, reference.id, crate::sched::current_cpu_id());
    }
    b.sessions.next = u64::MAX;
    assert!(b.sessions.reserve().is_err());
    assert!(allocate_epoch(&AtomicU64::new(u64::MAX)).is_err());
    let old_epoch = b.sessions.epoch;
    let mut replacement = CapabilityBroker::new();
    let fresh = replacement.sessions.reserve().unwrap();
    assert_ne!(fresh.epoch, old_epoch);
    assert!(replacement
        .account_session_participants(SessionRef {
            id: 1,
            epoch: old_epoch
        })
        .is_none());
    let counts = b.session_accounting();
    assert_eq!(
        (counts[0], counts[2], counts[3], counts[4], counts[5], counts[6], counts[7]),
        (0, 4, 2, 1, 1, 0, 0)
    );
    crate::serial_println!("[SESSION-E2] boundary PASS registrations=4 revoked=2 expired=1 owner_retired=1 live=0 receipts=0 grants=0 capacity={} pending_commit=0 consumption_refs=0 revocation_ops=0 ipc_completions=0 epoch_allocations=2 exhausted=denied old_epoch=denied cpu={} scope=isolated-production-broker remote_races=NOT_FORCED", counts[1], crate::sched::current_cpu_id());
}

#[test]
fn epoch_exhaustion_never_wraps_or_changes_the_allocator() {
    let counter = AtomicU64::new(u64::MAX - 1);
    assert_eq!(allocate_epoch(&counter), Ok(u64::MAX - 1));
    assert!(allocate_epoch(&counter).is_err());
    assert!(allocate_epoch(&counter).is_err());
    assert_eq!(counter.load(Ordering::Relaxed), u64::MAX);
}
