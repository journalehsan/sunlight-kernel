//! Bounded service-side directory retention contract and copied control API.
//! The adapter must authenticate kernel origin and the complete registration
//! from transport records, serialize operations, and resolve directories from
//! trusted mount policy. Decoded numbers alone grant no authority. Keep this
//! table for the entire registration lifetime; never reset it within an epoch.
use super::{lifecycle::Caller, registration::Registration};

#[path = "retention/wire.rs"]
pub mod wire;

#[path = "retention/transport.rs"]
pub mod transport;

#[path = "retention/ownership.rs"]
pub mod ownership;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scope {
    pub caller: Caller,
    pub namespace_id: u64,
    pub namespace_generation: u64,
}

/// Correlated acknowledgement. The generation is also the opaque object pair
/// identity: it is never a reusable filesystem inode or descriptor number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Receipt {
    registration: Registration,
    scope: Scope,
    request_id: u64,
    generation: u64,
}

impl Receipt {
    pub const fn registration(self) -> Registration {
        self.registration
    }
    pub const fn scope(self) -> Scope {
        self.scope
    }
    pub const fn request_id(self) -> u64 {
        self.request_id
    }
    pub const fn generation(self) -> u64 {
        self.generation
    }

    /// Reconstruct the correlation value a copied transport authenticated
    /// upstream. Callers must still prove it names a live lease; a decoded
    /// request alone never grants the referenced objects.
    pub fn from_parts(
        registration: Registration,
        scope: Scope,
        request_id: u64,
        generation: u64,
    ) -> Option<Self> {
        if request_id == 0
            || generation == 0
            || scope.namespace_id == 0
            || scope.namespace_generation == 0
            || [
                registration.epoch,
                scope.caller.pid,
                scope.caller.incarnation,
                scope.caller.address_space_generation,
            ]
            .contains(&0)
        {
            return None;
        }
        Some(Self {
            registration,
            scope,
            request_id,
            generation,
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Error<E> {
    Identity,
    Replay,
    Capacity,
    Exhausted,
    Stale,
    Resolve(E),
}

struct Entry<T> {
    receipt: Receipt,
    objects: T,
}

/// Each T must own real root/directory references, with Drop releasing them.
/// Capacity includes leases whose acknowledgement or release was lost. A timeout
/// cannot free a lease: retry release or retire the authenticated registration.
/// Requests use strictly increasing IDs in serialized transport order. Failed
/// resolution burns both the request ID and generation; retries need fresh IDs.
pub struct Table<T, const N: usize> {
    registration: Registration,
    entries: [Option<Entry<T>>; N],
    last_request: u64,
    next_generation: Option<u64>,
}

impl<T, const N: usize> Table<T, N> {
    pub fn new(registration: Registration) -> Result<Self, Error<core::convert::Infallible>> {
        let owner = registration.owner;
        if [
            registration.epoch,
            owner.pid,
            owner.incarnation,
            owner.address_space_generation,
        ]
        .contains(&0)
        {
            return Err(Error::Identity);
        }
        Ok(Self {
            registration,
            entries: [const { None }; N],
            last_request: 0,
            next_generation: Some(1),
        })
    }

    /// `authenticated` is captured by the transport, never copied from a frame.
    /// The callback must neither wait nor reenter this table. Success owns T
    /// before returning an acknowledgement; there is no fallible insertion.
    pub fn retain<E>(
        &mut self,
        authenticated: Registration,
        request_id: u64,
        scope: Scope,
        resolve: impl FnOnce() -> Result<T, E>,
    ) -> Result<Receipt, Error<E>> {
        if authenticated != self.registration
            || [
                scope.caller.pid,
                scope.caller.incarnation,
                scope.caller.address_space_generation,
                scope.namespace_id,
                scope.namespace_generation,
            ]
            .contains(&0)
        {
            return Err(Error::Identity);
        }
        if request_id == 0 || request_id <= self.last_request {
            return Err(Error::Replay);
        }
        // Even rejected capacity attempts cannot be replayed after release.
        self.last_request = request_id;
        let slot = self
            .entries
            .iter()
            .position(Option::is_none)
            .ok_or(Error::Capacity)?;
        let generation = self.next_generation.ok_or(Error::Exhausted)?;
        self.next_generation = generation.checked_add(1);
        let objects = resolve().map_err(Error::Resolve)?;
        let receipt = Receipt {
            registration: self.registration,
            scope,
            request_id,
            generation,
        };
        self.entries[slot] = Some(Entry { receipt, objects });
        Ok(receipt)
    }

    /// Produce a terminal refusal only when retain proved no lease was acquired.
    /// Identity/replay errors have no reply here; in particular a replay may own
    /// a live lease. Transport authentication and serialized delivery are required.
    pub fn retain_reply<E>(
        &mut self,
        authenticated: Registration,
        request: wire::PendingRetain,
        resolve: impl FnOnce() -> Result<T, E>,
    ) -> Result<wire::Frame, Error<E>> {
        let wire::Frame::Retain {
            registration,
            scope,
            request_id,
        } = request.request()
        else {
            unreachable!()
        };
        if registration != authenticated {
            return Err(Error::Identity);
        }
        let reason = match self.retain(authenticated, request_id, scope, resolve) {
            Ok(receipt) => return Ok(wire::Frame::Retained(receipt)),
            Err(Error::Capacity) => wire::Refusal::Capacity,
            Err(Error::Exhausted) => wire::Refusal::Exhausted,
            Err(Error::Resolve(_)) => wire::Refusal::Resolve,
            Err(error) => return Err(error),
        };
        Ok(wire::Frame::Refused { request, reason })
    }

    pub fn get(
        &self,
        authenticated: Registration,
        receipt: Receipt,
    ) -> Result<&T, Error<core::convert::Infallible>> {
        if authenticated != self.registration || receipt.registration != self.registration {
            return Err(Error::Identity);
        }
        self.entries
            .iter()
            .flatten()
            .find(|entry| entry.receipt == receipt)
            .map(|entry| &entry.objects)
            .ok_or(Error::Stale)
    }

    /// Resolve an opaque retained pair from an authenticated dispatch context.
    /// Numeric context fields alone never authenticate the caller or transport.
    pub fn get_scope(
        &self,
        authenticated: Registration,
        scope: Scope,
        generation: u64,
    ) -> Result<&T, Error<core::convert::Infallible>> {
        if authenticated != self.registration {
            return Err(Error::Identity);
        }
        self.entries
            .iter()
            .flatten()
            .find(|e| e.receipt.scope == scope && e.receipt.generation == generation)
            .map(|e| &e.objects)
            .ok_or(Error::Stale)
    }

    /// Replace the objects for an independently admitted scope when its
    /// authenticated shared-directory transaction commits. The control owner
    /// must fence dispatch and retain delivery against the replacement.
    pub fn replace_scope_objects(&mut self, scope: Scope, objects: &T)
    where
        T: Clone,
    {
        for entry in self.entries.iter_mut().flatten() {
            if entry.receipt.scope == scope {
                entry.objects = objects.clone();
            }
        }
    }

    /// Return an acknowledgement only after the owned references are dropped.
    /// Repeating an authentic release is harmless, including after slot reuse.
    pub fn release(
        &mut self,
        authenticated: Registration,
        receipt: Receipt,
    ) -> Result<Receipt, Error<core::convert::Infallible>> {
        if authenticated != self.registration || receipt.registration != self.registration {
            return Err(Error::Identity);
        }
        if let Some(slot) = self
            .entries
            .iter()
            .position(|entry| entry.as_ref().is_some_and(|entry| entry.receipt == receipt))
        {
            drop(self.entries[slot].take());
        }
        Ok(receipt)
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::super::registration::EndpointOwner;
    use super::*;
    use std::{cell::Cell, rc::Rc};

    fn registration() -> Registration {
        Registration {
            owner: EndpointOwner {
                endpoint: 0,
                pid: 2,
                incarnation: 3,
                address_space_generation: 4,
            },
            epoch: 5,
        }
    }
    fn scope() -> Scope {
        Scope {
            caller: Caller {
                pid: 6,
                incarnation: 7,
                address_space_generation: 8,
            },
            namespace_id: 9,
            namespace_generation: 10,
        }
    }
    struct Pin(Rc<Cell<usize>>);
    impl Drop for Pin {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }

    #[test]
    fn refusal_resolution_exhaustion_and_identity_rules() {
        use wire::{Frame, PendingRetain, Refusal};
        let r = registration();
        let mut table = Table::<u64, 1>::new(r).unwrap();
        let p = PendingRetain::new(r, scope(), 1).unwrap();
        assert_eq!(
            table.retain_reply(Registration { epoch: 99, ..r }, p, || -> Result<u64, ()> {
                panic!()
            }),
            Err(Error::Identity)
        );
        assert_eq!(
            table.retain_reply(r, p, || Err(())),
            Ok(Frame::Refused {
                request: p,
                reason: Refusal::Resolve
            })
        );
        assert_eq!(
            table.retain_reply(r, p, || -> Result<u64, ()> { panic!() }),
            Err(Error::Replay)
        );
        table.next_generation = None;
        let p = PendingRetain::new(r, scope(), 2).unwrap();
        assert_eq!(
            table.retain_reply(r, p, || -> Result<u64, ()> { panic!() }),
            Ok(Frame::Refused {
                request: p,
                reason: Refusal::Exhausted
            })
        );
    }

    #[test]
    fn ownership_release_retry_and_slot_reuse() {
        let drops = Rc::new(Cell::new(0));
        let r = registration();
        let mut table = Table::<Pin, 1>::new(r).unwrap();
        let a = table
            .retain(r, 1, scope(), || Ok::<_, ()>(Pin(drops.clone())))
            .unwrap();
        assert_eq!(a.request_id(), 1);
        assert_eq!(a.scope(), scope());
        assert_eq!(a.registration(), r);
        assert!(table.get(r, a).is_ok());
        assert_eq!(drops.get(), 0); // lost acknowledgement retains ownership
        assert_eq!(table.release(r, a), Ok(a));
        assert_eq!(drops.get(), 1);
        let b = table
            .retain(r, 2, scope(), || Ok::<_, ()>(Pin(drops.clone())))
            .unwrap();
        assert!(b.generation() > a.generation());
        assert_eq!(table.release(r, a), Ok(a));
        assert!(matches!(table.get(r, a), Err(Error::Stale)));
        assert!(table.get(r, b).is_ok());
        assert_eq!(drops.get(), 1);
        drop(table);
        assert_eq!(drops.get(), 2);
    }

    #[test]
    fn authentication_and_scope_mismatches_preserve_references() {
        let r = registration();
        let mut table = Table::<u64, 1>::new(r).unwrap();
        let a = table.retain(r, 1, scope(), || Ok::<_, ()>(42)).unwrap();
        for changed in [
            Registration { epoch: 6, ..r },
            Registration {
                owner: EndpointOwner {
                    address_space_generation: 99,
                    ..r.owner
                },
                ..r
            },
        ] {
            assert_eq!(table.release(changed, a), Err(Error::Identity));
            assert_eq!(table.get(changed, a), Err(Error::Identity));
            assert_eq!(
                table.retain(changed, 2, scope(), || -> Result<u64, ()> { panic!() }),
                Err(Error::Identity)
            );
        }
        let altered = Receipt {
            scope: Scope {
                namespace_generation: 11,
                ..scope()
            },
            ..a
        };
        assert_eq!(table.release(r, altered), Ok(altered));
        assert_eq!(table.get(r, altered), Err(Error::Stale));
        assert_eq!(table.get(r, a), Ok(&42));
        let other = Table::<u64, 1>::new(Registration { epoch: 6, ..r }).unwrap();
        assert_eq!(other.get(other.registration, a), Err(Error::Identity));
    }

    #[test]
    fn failures_replay_capacity_and_exhaustion_never_resolve() {
        let r = registration();
        let mut table = Table::<u64, 1>::new(r).unwrap();
        assert_eq!(
            table.retain(r, 1, scope(), || Err(())),
            Err(Error::Resolve(()))
        );
        assert_eq!(
            table.retain(r, 1, scope(), || -> Result<u64, ()> { panic!() }),
            Err(Error::Replay)
        );
        let a = table.retain(r, 2, scope(), || Ok::<_, ()>(42)).unwrap();
        assert_eq!(a.generation(), 2);
        assert_eq!(
            table.retain(r, 3, scope(), || -> Result<u64, ()> { panic!() }),
            Err(Error::Capacity)
        );
        table.release(r, a).unwrap();
        assert_eq!(
            table.retain(r, 3, scope(), || -> Result<u64, ()> { panic!() }),
            Err(Error::Replay)
        );
        table.next_generation = Some(u64::MAX);
        let last = table.retain(r, 4, scope(), || Ok::<_, ()>(42)).unwrap();
        assert_eq!(last.generation(), u64::MAX);
        table.release(r, last).unwrap();
        assert_eq!(
            table.retain(r, 5, scope(), || -> Result<u64, ()> { panic!() }),
            Err(Error::Exhausted)
        );
        assert!(matches!(
            Table::<u64, 1>::new(Registration { epoch: 0, ..r }),
            Err(Error::Identity)
        ));
        let mut empty = Table::<u64, 0>::new(r).unwrap();
        assert_eq!(
            empty.retain(r, 1, scope(), || -> Result<u64, ()> { panic!() }),
            Err(Error::Capacity)
        );
    }

    #[test]
    fn invalid_scope_excludes_resolution_and_request_ids_never_wrap() {
        let r = registration();
        let mut table = Table::<u64, 1>::new(r).unwrap();
        for field in 0..5 {
            let mut invalid = scope();
            match field {
                0 => invalid.caller.pid = 0,
                1 => invalid.caller.incarnation = 0,
                2 => invalid.caller.address_space_generation = 0,
                3 => invalid.namespace_id = 0,
                _ => invalid.namespace_generation = 0,
            }
            assert_eq!(
                table.retain(r, 1, invalid, || -> Result<u64, ()> { panic!() }),
                Err(Error::Identity)
            );
        }
        assert_eq!(
            table.retain(r, 0, scope(), || -> Result<u64, ()> { panic!() }),
            Err(Error::Replay)
        );
        let last = table
            .retain(r, u64::MAX, scope(), || Ok::<_, ()>(42))
            .unwrap();
        table.release(r, last).unwrap();
        assert_eq!(
            table.retain(r, 1, scope(), || -> Result<u64, ()> { panic!() }),
            Err(Error::Replay)
        );
        assert_eq!(
            table.retain(r, u64::MAX, scope(), || -> Result<u64, ()> { panic!() }),
            Err(Error::Replay)
        );
    }
}
