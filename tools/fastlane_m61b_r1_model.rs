//! Host-only reference model. This is not wired into the kernel or a native receipt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Principal {
    pid: u64,
    incarnation: u64,
    vm: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PolicyIdentity {
    id: u64,
    generation: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Policy {
    KernelOnly,
    Allowed,
    Required,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Transport {
    Kernel,
    FastLane,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Error {
    Exhausted,
    Stale,
    Denied,
    Duplicate,
    Terminal,
    Invalid,
}
#[derive(Clone, Copy)]
struct Entry {
    identity: PolicyIdentity,
    service: Principal,
    policy: Policy,
    live: bool,
}
struct Catalog {
    slots: [Option<Entry>; 4],
    next_generation: u64,
}
impl Catalog {
    fn new() -> Self {
        Self {
            slots: [None; 4],
            next_generation: 1,
        }
    }
    // Kernel-owned model API; callers of a native protocol must not gain this authority.
    fn issue(&mut self, service: Principal, policy: Policy) -> Result<PolicyIdentity, Error> {
        let slot = self
            .slots
            .iter()
            .position(|e| e.map_or(true, |x| !x.live))
            .ok_or(Error::Exhausted)?;
        let generation = self.next_generation;
        self.next_generation = generation.checked_add(1).ok_or(Error::Exhausted)?;
        let identity = PolicyIdentity {
            id: slot as u64 + 1,
            generation,
        };
        self.slots[slot] = Some(Entry {
            identity,
            service,
            policy,
            live: true,
        });
        Ok(identity)
    }
    fn resolve(&self, identity: PolicyIdentity, service: Principal) -> Result<Policy, Error> {
        let index = identity
            .id
            .checked_sub(1)
            .and_then(|n| usize::try_from(n).ok())
            .ok_or(Error::Stale)?;
        let e = self.slots.get(index).and_then(|e| *e).ok_or(Error::Stale)?;
        if !e.live || e.identity != identity {
            return Err(Error::Stale);
        }
        if e.service != service {
            return Err(Error::Denied);
        }
        Ok(e.policy)
    }
    fn retire(&mut self, identity: PolicyIdentity, service: Principal) -> Result<(), Error> {
        self.resolve(identity, service)?;
        self.slots[(identity.id - 1) as usize]
            .as_mut()
            .unwrap()
            .live = false;
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Preparing,
    Committed,
    Published,
    Completed,
    Failed,
}
struct Connection {
    transport: Transport,
    phase: Phase,
    request_id: Option<u64>,
}
impl Connection {
    fn prepare(
        policy: Policy,
        operation_authorized: bool,
        offer: Transport,
        fastlane_authority: bool,
    ) -> Result<Self, Error> {
        if !operation_authorized {
            return Err(Error::Denied);
        }
        match (policy, offer, fastlane_authority) {
            (Policy::KernelOnly, Transport::FastLane, _)
            | (Policy::Required, Transport::Kernel, _) => return Err(Error::Denied),
            (_, Transport::FastLane, false) => return Err(Error::Denied),
            _ => (),
        }
        Ok(Self {
            transport: offer,
            phase: Phase::Preparing,
            request_id: None,
        })
    }
    fn commit(&mut self) -> Result<(), Error> {
        if self.phase != Phase::Preparing {
            return Err(Error::Terminal);
        }
        self.phase = Phase::Committed;
        Ok(())
    }
    fn cancel(&mut self) -> Result<(), Error> {
        if self.phase != Phase::Preparing {
            return Err(Error::Terminal);
        }
        self.phase = Phase::Failed;
        Ok(())
    }
    // Request: nonzero u64 ID and at most 32 bytes. Response: same ID and bytes reversed.
    // One request per connection. Publication is irreversible; duplicates are rejected.
    fn execute(&mut self, id: u64, payload: &[u8]) -> Result<(u64, Vec<u8>), Error> {
        if self.request_id == Some(id) {
            return Err(Error::Duplicate);
        }
        if self.phase != Phase::Committed {
            return Err(Error::Terminal);
        }
        if id == 0 || payload.len() > 32 {
            return Err(Error::Invalid);
        }
        self.phase = Phase::Published;
        self.request_id = Some(id);
        let response = payload.iter().rev().copied().collect();
        self.phase = Phase::Completed;
        Ok((id, response))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn principal(n: u64) -> Principal {
        Principal {
            pid: n,
            incarnation: n + 10,
            vm: n + 20,
        }
    }
    #[test]
    fn identity_is_independent_and_revocation_fences_reuse() {
        let mut c = Catalog::new();
        let a = c.issue(principal(1), Policy::Allowed).unwrap();
        assert_eq!(c.resolve(a, principal(2)), Err(Error::Denied));
        assert_eq!(
            c.resolve(
                a,
                Principal {
                    incarnation: 99,
                    ..principal(1)
                }
            ),
            Err(Error::Denied)
        );
        c.retire(a, principal(1)).unwrap();
        assert_eq!(c.resolve(a, principal(1)), Err(Error::Stale));
        let b = c.issue(principal(1), Policy::Required).unwrap();
        assert_eq!(a.id, b.id);
        assert_ne!(a.generation, b.generation);
        assert_eq!(c.resolve(a, principal(1)), Err(Error::Stale));
    }
    #[test]
    fn bounded_and_generation_exhaustion_are_fail_closed() {
        let mut c = Catalog::new();
        for n in 0..4 {
            c.issue(principal(n), Policy::KernelOnly).unwrap();
        }
        assert_eq!(
            c.issue(principal(8), Policy::Allowed),
            Err(Error::Exhausted)
        );
        let mut c = Catalog::new();
        c.next_generation = u64::MAX;
        assert_eq!(
            c.issue(principal(1), Policy::Allowed),
            Err(Error::Exhausted)
        );
        assert!(c.slots.iter().all(Option::is_none));
    }
    #[test]
    fn operation_and_transport_authority_are_separate() {
        assert!(matches!(
            Connection::prepare(Policy::Allowed, false, Transport::FastLane, true),
            Err(Error::Denied)
        ));
        assert!(matches!(
            Connection::prepare(Policy::KernelOnly, true, Transport::FastLane, true),
            Err(Error::Denied)
        ));
        assert!(matches!(
            Connection::prepare(Policy::Required, true, Transport::Kernel, true),
            Err(Error::Denied)
        ));
        assert!(matches!(
            Connection::prepare(Policy::Required, true, Transport::FastLane, false),
            Err(Error::Denied)
        ));
    }
    #[test]
    fn same_operation_semantics_and_single_terminal_outcome() {
        for payload in [&b""[..], &b"abc"[..], &[42; 32][..]] {
            let mut k =
                Connection::prepare(Policy::Allowed, true, Transport::Kernel, false).unwrap();
            let mut f =
                Connection::prepare(Policy::Allowed, true, Transport::FastLane, true).unwrap();
            k.commit().unwrap();
            f.commit().unwrap();
            assert_eq!(k.execute(7, payload), f.execute(7, payload));
            assert_eq!(k.execute(7, payload), Err(Error::Duplicate));
            assert_eq!(f.execute(7, payload), Err(Error::Duplicate));
            assert_eq!(k.cancel(), Err(Error::Terminal));
            assert_eq!(f.cancel(), Err(Error::Terminal));
            assert_eq!(k.transport, Transport::Kernel);
            assert_eq!(f.transport, Transport::FastLane);
        }
        let mut c = Connection::prepare(Policy::Allowed, true, Transport::Kernel, false).unwrap();
        c.commit().unwrap();
        assert_eq!(c.execute(0, b"x"), Err(Error::Invalid));
        assert_eq!(c.execute(1, &[0; 33]), Err(Error::Invalid));
        assert_eq!(c.execute(1, b"x"), Ok((1, vec![b'x'])));
        let mut c = Connection::prepare(Policy::Allowed, true, Transport::FastLane, true).unwrap();
        c.cancel().unwrap();
        assert_eq!(c.commit(), Err(Error::Terminal));
        assert_eq!(c.cancel(), Err(Error::Terminal));
    }
}
