//! Kernel-side retention bookkeeping. This is not transport authentication.
//! Keep one table for the registration lifetime, under lifecycle serialization.
//! Never discard it on timeout: pending retains can own unknown remote leases.
use super::{
    wire::{acknowledge_release, Frame, PendingRetain, Refusal},
    Receipt, Scope,
};
use crate::vfs_rpc::{registration::Registration, ProtocolError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Capacity,
    Exhausted,
    State,
    Protocol(ProtocolError),
}

/// A late success for an abandoned retain must never install a namespace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub enum Retained {
    Install(Receipt),
    Release(Frame),
}

#[derive(Clone, Copy)]
enum Lease {
    Pending {
        id: u64,
        request: PendingRetain,
        abandoned: bool,
    },
    Held(Receipt),
    Releasing(Receipt),
}
impl Lease {
    fn id(self) -> u64 {
        match self {
            Self::Pending { id, .. } => id,
            Self::Held(r) | Self::Releasing(r) => r.request_id(),
        }
    }
}

/// Capacity accounts for all sent retains, including those without receipts.
/// Receipt delivery is single-use; a timeout or failed publication must preserve
/// the entry. Once local namespace/request references are gone, begin release.
/// Dropping this bookkeeping does not release remote objects. The runtime owner
/// must keep it until empty or until authenticated registration drain is proven.
pub struct Ownership<const N: usize> {
    registration: Registration,
    entries: [Option<Lease>; N],
    next_id: Option<u64>,
}
impl<const N: usize> Ownership<N> {
    pub fn new(registration: Registration) -> Result<Self, ProtocolError> {
        // Reuse service registration validation without inventing a scope.
        super::Table::<(), 0>::new(registration).map_err(|_| ProtocolError::Encoding)?;
        Ok(Self {
            registration,
            entries: [None; N],
            next_id: Some(1),
        })
    }

    /// Reserve ownership before publication. The caller must serialize sends in
    /// this allocation order. Retain retransmission is forbidden by service replay
    /// rules; an uncertain send keeps its slot until receipt or registration drain.
    pub fn prepare(&mut self, scope: Scope) -> Result<Frame, Error> {
        let slot = self
            .entries
            .iter()
            .position(Option::is_none)
            .ok_or(Error::Capacity)?;
        let id = self.next_id.ok_or(Error::Exhausted)?;
        let request = PendingRetain::new(self.registration, scope, id).map_err(Error::Protocol)?;
        self.next_id = id.checked_add(1);
        self.entries[slot] = Some(Lease::Pending {
            id,
            request,
            abandoned: false,
        });
        Ok(request.request())
    }

    /// Abandon installation of a pending retain after caller/namespace teardown.
    /// Match the entire captured scope under lifecycle serialization. This does
    /// not cancel publication, free capacity, or retransmit the retain. A late
    /// success will enter release directly; a refusal can still free the slot.
    /// If success already won, this returns State: withdraw the installed binding
    /// and drain its local users before calling release instead.
    pub fn abandon(&mut self, id: u64, expected: Scope) -> Result<(), Error> {
        let slot = self.slot(id)?;
        let Some(Lease::Pending {
            request, abandoned, ..
        }) = &mut self.entries[slot]
        else {
            return Err(Error::State);
        };
        let Frame::Retain { scope, .. } = request.request() else {
            unreachable!()
        };
        if scope != expected {
            return Err(Error::State);
        }
        *abandoned = true;
        Ok(())
    }

    /// Returns the matched disposition once only. Authentication must come from
    /// transport records; revalidate caller/service lifecycle before installation.
    /// Abandoned requests were never installed and have no local users to drain.
    /// Their release remains owned even if the returned frame cannot be sent.
    pub fn retained(
        &mut self,
        id: u64,
        authenticated: Registration,
        bytes: &[u8],
    ) -> Result<Retained, Error> {
        let slot = self.slot(id)?;
        let Some(Lease::Pending {
            request, abandoned, ..
        }) = self.entries[slot]
        else {
            return Err(Error::State);
        };
        let receipt = request
            .acknowledge(authenticated, bytes)
            .map_err(Error::Protocol)?;
        if abandoned {
            self.entries[slot] = Some(Lease::Releasing(receipt));
            Ok(Retained::Release(Frame::Release(receipt)))
        } else {
            self.entries[slot] = Some(Lease::Held(receipt));
            Ok(Retained::Install(receipt))
        }
    }

    /// Free only a pending slot on a correlated, authenticated proof of refusal.
    /// Timeout, replay errors and malformed replies provide no such proof.
    pub fn refused(
        &mut self,
        id: u64,
        authenticated: Registration,
        bytes: &[u8],
    ) -> Result<Refusal, Error> {
        let slot = self.slot(id)?;
        let Some(Lease::Pending { request, .. }) = self.entries[slot] else {
            return Err(Error::State);
        };
        let reason = request
            .refuse(authenticated, bytes)
            .map_err(Error::Protocol)?;
        self.entries[slot] = None;
        Ok(reason)
    }

    /// Caller must first withdraw publication and drain all local users. This
    /// model cannot prove that prerequisite. Repeated calls retry the same lease.
    pub fn release(&mut self, id: u64) -> Result<Frame, Error> {
        let slot = self.slot(id)?;
        let receipt = match self.entries[slot] {
            Some(Lease::Held(r) | Lease::Releasing(r)) => r,
            _ => return Err(Error::State),
        };
        self.entries[slot] = Some(Lease::Releasing(receipt));
        Ok(Frame::Release(receipt))
    }

    pub fn released(
        &mut self,
        id: u64,
        authenticated: Registration,
        bytes: &[u8],
    ) -> Result<(), Error> {
        let slot = self.slot(id)?;
        let Some(Lease::Releasing(receipt)) = self.entries[slot] else {
            return Err(Error::State);
        };
        acknowledge_release(receipt, authenticated, bytes).map_err(Error::Protocol)?;
        self.entries[slot] = None;
        Ok(())
    }

    pub fn outstanding(&self) -> usize {
        self.entries.iter().flatten().count()
    }

    /// Bounded retry inventory; enumeration never transfers or frees ownership.
    /// Only releases are retryable. Pending retains must not be retransmitted.
    pub fn releases(&self) -> impl Iterator<Item = Frame> + '_ {
        self.entries.iter().filter_map(|entry| match entry {
            Some(Lease::Releasing(receipt)) => Some(Frame::Release(*receipt)),
            _ => None,
        })
    }

    fn slot(&self, id: u64) -> Result<usize, Error> {
        self.entries
            .iter()
            .position(|entry| entry.is_some_and(|lease| lease.id() == id))
            .ok_or(Error::State)
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use crate::vfs_rpc::{lifecycle::Caller, registration::EndpointOwner};
    use std::{cell::Cell, rc::Rc, vec::Vec};
    fn registration() -> Registration {
        Registration {
            owner: EndpointOwner {
                endpoint: 0,
                pid: 1,
                incarnation: 2,
                address_space_generation: 3,
            },
            epoch: 4,
        }
    }
    fn scope() -> Scope {
        Scope {
            caller: Caller {
                pid: 5,
                incarnation: 6,
                address_space_generation: 7,
            },
            namespace_id: 8,
            namespace_generation: 9,
        }
    }
    fn retain(owner: &mut Ownership<1>, service: &mut super::super::Table<(), 1>) -> Receipt {
        let Frame::Retain {
            registration,
            scope,
            request_id,
        } = owner.prepare(scope()).unwrap()
        else {
            panic!()
        };
        service
            .retain(registration, request_id, scope, || Ok::<_, ()>(()))
            .unwrap()
    }

    #[test]
    fn abandoned_late_success_owns_release_through_send_failure_and_lost_ack() {
        struct Pin(Rc<Cell<usize>>);
        impl Drop for Pin {
            fn drop(&mut self) {
                self.0.set(self.0.get() + 1);
            }
        }
        let drops = Rc::new(Cell::new(0));
        let r = registration();
        let mut owner = Ownership::<1>::new(r).unwrap();
        let mut service = super::super::Table::<Pin, 1>::new(r).unwrap();
        owner.prepare(scope()).unwrap();
        let receipt = service
            .retain(r, 1, scope(), || Ok::<_, ()>(Pin(drops.clone())))
            .unwrap();
        owner.abandon(1, scope()).unwrap();
        owner.abandon(1, scope()).unwrap();
        assert_eq!(owner.outstanding(), 1);
        assert_eq!(owner.prepare(scope()), Err(Error::Capacity));
        assert_eq!(owner.release(1), Err(Error::State));
        assert_eq!(owner.releases().count(), 0);

        let ack = Frame::Retained(receipt).encode().unwrap();
        let release = Frame::Release(receipt);
        assert_eq!(owner.retained(1, r, &ack), Ok(Retained::Release(release)));
        assert_eq!(owner.retained(1, r, &ack), Err(Error::State));
        assert_eq!(drops.get(), 0);
        // The initial send is lost. The inventory preserves the exact release.
        assert_eq!(owner.releases().collect::<Vec<_>>(), [release]);
        assert_eq!(owner.release(1), Ok(release));
        service.release(r, receipt).unwrap();
        assert_eq!(drops.get(), 1);
        // The acknowledgement is lost; replaying release drops nothing twice.
        assert_eq!(owner.releases().collect::<Vec<_>>(), [release]);
        service.release(r, receipt).unwrap();
        assert_eq!(drops.get(), 1);
        let released = Frame::Released(receipt).encode().unwrap();
        assert!(owner
            .released(1, Registration { epoch: 99, ..r }, &released)
            .is_err());
        assert_eq!(owner.outstanding(), 1);
        owner.released(1, r, &released).unwrap();
        assert_eq!(owner.releases().count(), 0);
        assert_eq!(owner.outstanding(), 0);
        assert_eq!(owner.released(1, r, &released), Err(Error::State));
        assert_eq!(drops.get(), 1);
    }

    #[test]
    fn abandonment_matches_full_scope_and_success_winner_still_requires_drain() {
        let r = registration();
        let s = scope();
        let mut owner = Ownership::<1>::new(r).unwrap();
        let mut service = super::super::Table::<(), 1>::new(r).unwrap();
        let receipt = retain(&mut owner, &mut service);
        for field in 0..5 {
            let mut wrong = s;
            match field {
                0 => wrong.caller.pid += 1,
                1 => wrong.caller.incarnation += 1,
                2 => wrong.caller.address_space_generation += 1,
                3 => wrong.namespace_id += 1,
                _ => wrong.namespace_generation += 1,
            }
            assert_eq!(owner.abandon(1, wrong), Err(Error::State));
        }
        assert_eq!(owner.abandon(2, s), Err(Error::State));
        let ack = Frame::Retained(receipt).encode().unwrap();
        assert_eq!(owner.retained(1, r, &ack), Ok(Retained::Install(receipt)));
        // Success won serialization. Abandon cannot release a published lease.
        assert_eq!(owner.abandon(1, s), Err(Error::State));
        assert_eq!(owner.releases().count(), 0);
        assert_eq!(owner.outstanding(), 1);
        owner.release(1).unwrap(); // Caller has separately established local drain.
        assert_eq!(owner.abandon(1, s), Err(Error::State));
        assert_eq!(owner.releases().count(), 1);
    }

    #[test]
    fn abandoned_pending_rejects_bad_replies_and_accepts_correlated_refusal() {
        let r = registration();
        let mut owner = Ownership::<1>::new(r).unwrap();
        let mut service = super::super::Table::<(), 1>::new(r).unwrap();
        let receipt = retain(&mut owner, &mut service);
        owner.abandon(1, scope()).unwrap();
        let ack = Frame::Retained(receipt).encode().unwrap();
        for len in 0..ack.len() {
            assert!(owner.retained(1, r, &ack[..len]).is_err());
        }
        // Generation is service-assigned; only the preceding fields correlate
        // with a pending request.
        for index in 2..13 {
            let mut wrong = ack;
            wrong[index * 8] ^= 16;
            assert!(owner.retained(1, r, &wrong).is_err());
        }
        assert!(owner
            .retained(1, Registration { epoch: 99, ..r }, &ack)
            .is_err());
        let released = Frame::Released(receipt).encode().unwrap();
        assert!(owner.retained(1, r, &released).is_err());
        assert_eq!(owner.released(1, r, &released), Err(Error::State));
        assert_eq!(owner.outstanding(), 1);
        assert_eq!(owner.releases().count(), 0);
        // Resolve this live lease, then exercise actual refusal on a new request.
        assert_eq!(
            owner.retained(1, r, &ack),
            Ok(Retained::Release(Frame::Release(receipt)))
        );
        service.release(r, receipt).unwrap();
        owner.released(1, r, &released).unwrap();
        owner.prepare(scope()).unwrap();
        owner.abandon(2, scope()).unwrap();
        let pending = PendingRetain::new(r, scope(), 2).unwrap();
        let refusal = service.retain_reply(r, pending, || Err(())).unwrap();
        let bytes = refusal.encode().unwrap();
        assert!(owner.retained(2, r, &bytes).is_err());
        assert!(owner.refused(2, r, &ack).is_err());
        assert_eq!(owner.refused(2, r, &bytes), Ok(Refusal::Resolve));
        assert_eq!(owner.outstanding(), 0);
        assert_eq!(owner.refused(2, r, &bytes), Err(Error::State));
        let Frame::Retain { request_id, .. } = owner.prepare(scope()).unwrap() else {
            panic!()
        };
        assert_eq!(request_id, 3);
        assert_eq!(owner.abandon(2, scope()), Err(Error::State));
        assert!(owner.refused(3, r, &bytes).is_err());
        assert_eq!(owner.outstanding(), 1);
    }

    #[test]
    fn release_inventory_excludes_pending_and_held_entries() {
        let r = registration();
        let mut owner = Ownership::<4>::new(r).unwrap();
        let mut service = super::super::Table::<(), 4>::new(r).unwrap();
        let mut expected = Vec::new();
        for id in 1..=4 {
            owner.prepare(scope()).unwrap();
            let receipt = service.retain(r, id, scope(), || Ok::<_, ()>(())).unwrap();
            if id == 1 {
                continue; // Lost acknowledgement: never retry this retain.
            }
            if id == 3 {
                owner.abandon(id, scope()).unwrap();
            }
            assert_eq!(
                owner.retained(id, r, &Frame::Retained(receipt).encode().unwrap()),
                Ok(if id == 3 {
                    Retained::Release(Frame::Release(receipt))
                } else {
                    Retained::Install(receipt)
                })
            );
            if id == 4 {
                owner.release(id).unwrap();
            }
            if id >= 3 {
                expected.push(Frame::Release(receipt));
            }
        }
        assert_eq!(owner.outstanding(), 4);
        for _ in 0..2 {
            assert_eq!(owner.releases().collect::<Vec<_>>(), expected);
            assert_eq!(owner.outstanding(), 4);
        }
    }
    #[test]
    fn refusal_releases_only_exact_pending_request_and_never_reuses_id() {
        let r = registration();
        let mut owner = Ownership::<1>::new(r).unwrap();
        let mut service = super::super::Table::<(), 0>::new(r).unwrap();
        let frame = owner.prepare(scope()).unwrap();
        let Frame::Retain { request_id, .. } = frame else {
            panic!()
        };
        let pending = PendingRetain::new(r, scope(), request_id).unwrap();
        let refusal = service
            .retain_reply(r, pending, || -> Result<(), ()> { panic!() })
            .unwrap();
        let bytes = refusal.encode().unwrap();
        for len in 0..bytes.len() {
            assert!(owner.refused(request_id, r, &bytes[..len]).is_err());
        }
        assert!(owner
            .refused(request_id, r, &frame.encode().unwrap())
            .is_err());
        assert!(owner.retained(request_id, r, &bytes).is_err());

        assert_eq!(Frame::decode(&bytes), Ok(refusal));
        for index in 2..13 {
            let mut wrong = bytes;
            wrong[index * 8] ^= 16;
            assert!(owner.refused(request_id, r, &wrong).is_err());
            assert_eq!(owner.outstanding(), 1);
        }
        for value in [0u64, 4, u64::MAX] {
            let mut wrong = bytes;
            wrong[104..112].copy_from_slice(&value.to_le_bytes());
            assert!(owner.refused(request_id, r, &wrong).is_err());
        }
        assert!(owner
            .refused(request_id, Registration { epoch: 99, ..r }, &bytes)
            .is_err());
        assert_eq!(owner.refused(request_id, r, &bytes), Ok(Refusal::Capacity));
        assert_eq!(owner.refused(request_id, r, &bytes), Err(Error::State));
        assert_eq!(owner.outstanding(), 0);
        let Frame::Retain {
            request_id: next, ..
        } = owner.prepare(scope()).unwrap()
        else {
            panic!()
        };
        assert_eq!(next, request_id + 1);
        assert!(owner.refused(next, r, &bytes).is_err());
        assert_eq!(owner.outstanding(), 1);
    }
    #[test]
    fn replay_cannot_refuse_a_lease_and_refusal_cannot_clear_held_or_releasing() {
        let r = registration();
        let mut owner = Ownership::<1>::new(r).unwrap();
        owner.prepare(scope()).unwrap();
        let p = PendingRetain::new(r, scope(), 1).unwrap();
        let mut service = super::super::Table::<(), 1>::new(r).unwrap();
        let ack = service.retain_reply(r, p, || Ok::<_, ()>(())).unwrap();
        assert_eq!(
            service.retain_reply(r, p, || -> Result<(), ()> { panic!() }),
            Err(super::super::Error::Replay)
        );
        let refusal = Frame::Refused {
            request: p,
            reason: Refusal::Resolve,
        }
        .encode()
        .unwrap();
        assert!(matches!(
            owner.retained(1, r, &ack.encode().unwrap()),
            Ok(Retained::Install(_))
        ));
        assert_eq!(owner.refused(1, r, &refusal), Err(Error::State));
        owner.release(1).unwrap();
        assert_eq!(owner.refused(1, r, &refusal), Err(Error::State));
        assert_eq!(owner.outstanding(), 1);
    }

    #[test]
    fn lost_replies_duplicate_publication_and_release_retries() {
        let r = registration();
        let mut owner = Ownership::<1>::new(r).unwrap();
        let mut service = super::super::Table::<(), 1>::new(r).unwrap();
        let receipt = retain(&mut owner, &mut service);
        assert_eq!(owner.outstanding(), 1);
        assert_eq!(owner.prepare(scope()), Err(Error::Capacity));
        assert_eq!(owner.release(1), Err(Error::State));
        let ack = Frame::Retained(receipt).encode().unwrap();
        let stale = Registration { epoch: 99, ..r };
        assert!(owner.retained(1, stale, &ack).is_err());
        assert!(owner.retained(2, r, &ack).is_err());
        assert_eq!(owner.retained(1, r, &ack), Ok(Retained::Install(receipt)));
        assert_eq!(owner.retained(1, r, &ack), Err(Error::State));
        let released = Frame::Released(receipt).encode().unwrap();
        assert_eq!(owner.released(1, r, &released), Err(Error::State));
        for _ in 0..2 {
            assert_eq!(owner.release(1), Ok(Frame::Release(receipt)));
            service.release(r, receipt).unwrap();
            assert!(owner.released(1, stale, &released).is_err());
            assert!(owner.released(1, r, &ack).is_err());
            assert_eq!(owner.outstanding(), 1);
        }
        owner.released(1, r, &released).unwrap();
        assert_eq!(owner.outstanding(), 0);
        let replacement = retain(&mut owner, &mut service);
        assert_eq!(replacement.request_id(), 2);
        assert!(owner.released(1, r, &released).is_err());
        assert!(owner.retained(2, r, &ack).is_err());
        assert_eq!(owner.outstanding(), 1);
    }
    #[test]
    fn invalid_scope_capacity_and_permanent_exhaustion() {
        let r = registration();
        assert!(Ownership::<1>::new(Registration { epoch: 0, ..r }).is_err());
        let mut owner = Ownership::<1>::new(r).unwrap();
        assert!(owner
            .prepare(Scope {
                namespace_id: 0,
                ..scope()
            })
            .is_err());
        owner.next_id = Some(u64::MAX);
        let mut service = super::super::Table::<(), 1>::new(r).unwrap();
        let receipt = retain(&mut owner, &mut service);
        let id = receipt.request_id();
        assert_eq!(
            owner.retained(id, r, &Frame::Retained(receipt).encode().unwrap()),
            Ok(Retained::Install(receipt))
        );
        owner.release(id).unwrap();
        service.release(r, receipt).unwrap();
        owner
            .released(id, r, &Frame::Released(receipt).encode().unwrap())
            .unwrap();
        assert_eq!(owner.prepare(scope()), Err(Error::Exhausted));
        assert_eq!(
            Ownership::<0>::new(r).unwrap().prepare(scope()),
            Err(Error::Capacity)
        );
    }
}
