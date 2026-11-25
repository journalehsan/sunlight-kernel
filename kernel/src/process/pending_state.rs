//! Private PID claims and ownership arbitration, serialized by the scheduler.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Parent {
    pub pid: usize,
    pub incarnation: u64,
    pub image_generation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Ticket {
    pub pid: usize,
    serial: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Identity {
    pub ticket: Ticket,
    pub incarnation: u64,
    pub image_generation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RemoteClaim {
    pub child: Identity,
    pub registration: sunlight_ipc::vfs_rpc::registration::Registration,
    pub request_id: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Error {
    Capacity,
    Collision,
    Exhausted,
    Stale,
    Cancelled,
    ParentGone,
    NoMemory,
    Image,
    UnsupportedDescriptor,
}

enum State<T> {
    Constructing,
    Private(Identity, T),
    // A cancelled constructor still owns its partial resources.
    CancellingConstruction,
    // The cancellation winner owns teardown until it reports completion.
    Reclaiming,
    ReclaimedWithRemote,
    CommitEligible(Identity, T),
    Transferred,
}

struct Entry<T> {
    ticket: Ticket,
    parent: Parent,
    remote: Option<RemoteClaim>,
    state: State<T>,
}

pub(crate) struct Pending<T, const N: usize> {
    entries: [Option<Entry<T>>; N],
    serial: u64,
}

impl<T, const N: usize> Pending<T, N> {
    pub const fn new() -> Self {
        Self {
            entries: [const { None }; N],
            serial: 0,
        }
    }

    pub fn pids(&self) -> impl Iterator<Item = usize> + '_ {
        self.entries.iter().flatten().map(|entry| entry.ticket.pid)
    }

    pub fn reserve(&mut self, pid: usize, parent: Parent, occupied: bool) -> Result<Ticket, Error> {
        if pid == 0 || occupied || self.pids().any(|reserved| reserved == pid) {
            return Err(Error::Collision);
        }
        let slot = self
            .entries
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(Error::Capacity)?;
        self.serial = self.serial.checked_add(1).ok_or(Error::Exhausted)?;
        let ticket = Ticket {
            pid,
            serial: self.serial,
        };
        *slot = Some(Entry {
            ticket,
            parent,
            remote: None,
            state: State::Constructing,
        });
        Ok(ticket)
    }

    fn entry(&mut self, ticket: Ticket) -> Result<&mut Entry<T>, Error> {
        self.entries
            .iter_mut()
            .flatten()
            .find(|entry| entry.ticket == ticket)
            .ok_or(Error::Stale)
    }

    pub fn finish(
        &mut self,
        identity: Identity,
        child: T,
        parent_live: impl FnOnce(Parent) -> bool,
    ) -> Result<(), (Error, T)> {
        let entry = match self.entry(identity.ticket) {
            Ok(entry) => entry,
            Err(error) => return Err((error, child)),
        };
        match entry.state {
            State::Constructing if parent_live(entry.parent) => {
                entry.state = State::Private(identity, child);
                Ok(())
            }
            State::Constructing => {
                entry.state = State::Reclaiming;
                Err((Error::ParentGone, child))
            }
            State::CancellingConstruction => {
                entry.state = State::Reclaiming;
                Err((Error::Cancelled, child))
            }
            _ => Err((Error::Stale, child)),
        }
    }

    pub fn parent(&self, ticket: Ticket) -> Result<Parent, Error> {
        self.entries
            .iter()
            .flatten()
            .find(|entry| entry.ticket == ticket)
            .map(|entry| entry.parent)
            .ok_or(Error::Stale)
    }

    pub fn identity(&self, ticket: Ticket) -> Result<Identity, Error> {
        self.entries
            .iter()
            .flatten()
            .find_map(|entry| match entry.state {
                State::Private(identity, _) if entry.ticket == ticket => Some(identity),
                _ => None,
            })
            .ok_or(Error::Stale)
    }

    pub fn private(&self, expected: Identity) -> Option<&T> {
        self.entries
            .iter()
            .flatten()
            .find_map(|entry| match &entry.state {
                State::Private(actual, child) if *actual == expected => Some(child),
                _ => None,
            })
    }

    pub fn remote(&self, child: Identity) -> Option<RemoteClaim> {
        self.entries
            .iter()
            .flatten()
            .filter_map(|entry| entry.remote)
            .find(|claim| claim.child == child)
    }

    pub fn attach_remote(&mut self, claim: RemoteClaim) {
        let entry = self
            .entry(claim.child.ticket)
            .expect("staged private child");
        assert!(matches!(entry.state, State::Private(identity, _) if identity == claim.child));
        assert!(entry.remote.replace(claim).is_none());
    }

    pub fn reap_remote(&mut self, settled: impl Fn(RemoteClaim) -> bool) {
        for slot in &mut self.entries {
            if slot.as_ref().is_some_and(|entry| {
                matches!(entry.state, State::ReclaimedWithRemote)
                    && entry.remote.is_some_and(&settled)
            }) {
                *slot = None;
            }
        }
    }

    pub fn cancel(
        &mut self,
        ticket: Ticket,
        expected: Option<Identity>,
    ) -> Result<Option<T>, Error> {
        let entry = self.entry(ticket)?;
        if let Some(expected) = expected {
            match entry.state {
                State::Private(identity, _) if identity == expected => {}
                State::CommitEligible(identity, _) if identity == expected => {}
                _ => return Err(Error::Stale),
            }
        }
        match entry.state {
            State::Constructing => {
                entry.state = State::CancellingConstruction;
                Ok(None)
            }
            State::Private(..) => match core::mem::replace(&mut entry.state, State::Reclaiming) {
                State::Private(_, child) => Ok(Some(child)),
                _ => unreachable!(),
            },
            State::CommitEligible(..) => {
                match core::mem::replace(&mut entry.state, State::Reclaiming) {
                    State::CommitEligible(_, child) => Ok(Some(child)),
                    _ => unreachable!(),
                }
            }
            _ => Err(Error::Cancelled),
        }
    }

    /// Call after the only local owner has destroyed all resources.
    pub fn reclaimed(&mut self, ticket: Ticket) {
        let slot = self
            .entries
            .iter_mut()
            .find(|slot| slot.as_ref().is_some_and(|entry| entry.ticket == ticket))
            .expect("pending identity released twice");
        assert!(matches!(
            slot.as_ref().unwrap().state,
            State::Constructing | State::CancellingConstruction | State::Reclaiming
        ));
        if let Some(entry) = slot.as_mut().filter(|entry| entry.remote.is_some()) {
            entry.state = State::ReclaimedWithRemote;
        } else {
            *slot = None;
        }
    }

    pub fn eligible(&mut self, identity: Identity, parent_live: bool) -> Result<(), Error> {
        let entry = self.entry(identity.ticket)?;
        if !parent_live {
            return Err(Error::ParentGone);
        }
        if !matches!(entry.state, State::Private(actual, _) if actual == identity) {
            return Err(Error::Stale);
        }
        if let State::Private(_, child) = core::mem::replace(&mut entry.state, State::Transferred) {
            entry.state = State::CommitEligible(identity, child);
        }
        Ok(())
    }

    pub fn transfer(&mut self, identity: Identity, parent_live: bool) -> Result<T, Error> {
        let entry = self.entry(identity.ticket)?;
        if !parent_live {
            return Err(Error::ParentGone);
        }
        if !matches!(entry.state, State::CommitEligible(actual, _) if actual == identity) {
            return Err(Error::Stale);
        }
        match core::mem::replace(&mut entry.state, State::Transferred) {
            State::CommitEligible(_, child) => Ok(child),
            _ => unreachable!(),
        }
    }

    pub fn published(&mut self, ticket: Ticket) {
        let slot = self.entries.iter_mut().find(|slot| {
            slot.as_ref().is_some_and(|entry| entry.ticket == ticket)
        }).expect("transferred child retains its slot until scheduler handoff");
        assert!(matches!(slot.as_ref().unwrap().state, State::Transferred));
        *slot = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{mpsc, Arc, Mutex};

    const PARENT: Parent = Parent {
        pid: 1,
        incarnation: 7,
        image_generation: 11,
    };
    const PID: usize = 12;

    struct Resource(Arc<AtomicUsize>);
    impl Drop for Resource {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn identity(ticket: Ticket, incarnation: u64) -> Identity {
        Identity {
            ticket,
            incarnation,
            image_generation: incarnation + 1000,
        }
    }

    #[test]
    fn bounded_admission_collision_and_reuse_over_100_cycles() {
        let mut pending = Pending::<Resource, 1>::new();
        let drops = Arc::new(AtomicUsize::new(0));
        let mut prior = None;
        for n in 1..=100 {
            assert_eq!(pending.reserve(PID, PARENT, true), Err(Error::Collision));
            let ticket = pending.reserve(PID, PARENT, false).unwrap();
            assert_eq!(pending.reserve(PID, PARENT, false), Err(Error::Collision));
            assert_eq!(
                pending.reserve(PID + 1, PARENT, false),
                Err(Error::Capacity)
            );
            assert_eq!(pending.identity(ticket), Err(Error::Stale));
            assert_eq!(pending.parent(ticket), Ok(PARENT));
            let current = identity(ticket, n);
            assert!(pending
                .finish(current, Resource(drops.clone()), |p| p == PARENT)
                .is_ok());
            assert_eq!(pending.identity(ticket), Ok(current));
            if let Some(previous) = prior {
                assert!(matches!(pending.cancel(previous, None), Err(Error::Stale)));
            }
            let mut stale = current;
            stale.incarnation += 1;
            assert!(matches!(
                pending.cancel(ticket, Some(stale)),
                Err(Error::Stale)
            ));
            stale = current;
            stale.image_generation += 1;
            assert!(matches!(
                pending.cancel(ticket, Some(stale)),
                Err(Error::Stale)
            ));
            let owner = pending.cancel(ticket, Some(current)).unwrap().unwrap();
            assert!(pending.cancel(ticket, Some(current)).is_err());
            assert_eq!(pending.reserve(PID, PARENT, false), Err(Error::Collision));
            drop(owner);
            pending.reclaimed(ticket);
            assert_eq!(drops.load(Ordering::Relaxed), n as usize);
            assert_eq!(pending.pids().count(), 0);
            prior = Some(ticket);
        }
    }

    #[test]
    fn full_pending_table_keeps_reclaiming_pid_charged_until_teardown() {
        let mut pending = Pending::<Resource, 8>::new();
        let drops = Arc::new(AtomicUsize::new(0));
        let identities: [Identity; 8] = core::array::from_fn(|index| {
            let ticket = pending.reserve(PID + index, PARENT, false).unwrap();
            let current = identity(ticket, index as u64 + 1);
            assert!(pending
                .finish(current, Resource(drops.clone()), |_| true)
                .is_ok());
            current
        });
        let first = identities[0];
        let owner = pending.cancel(first.ticket, Some(first)).unwrap().unwrap();
        assert_eq!(
            pending.reserve(PID + 8, PARENT, false),
            Err(Error::Capacity)
        );
        assert_eq!(pending.reserve(PID, PARENT, false), Err(Error::Collision));
        assert_eq!(drops.load(Ordering::Relaxed), 0);
        drop(owner);
        pending.reclaimed(first.ticket);

        let reused = pending.reserve(PID, PARENT, false).unwrap();
        assert_ne!(reused, first.ticket);
        assert!(matches!(
            pending.cancel(first.ticket, Some(first)),
            Err(Error::Stale)
        ));
        pending.reclaimed(reused);
        for current in identities.into_iter().skip(1) {
            drop(
                pending
                    .cancel(current.ticket, Some(current))
                    .unwrap()
                    .unwrap(),
            );
            pending.reclaimed(current.ticket);
        }
        assert_eq!(drops.load(Ordering::Relaxed), 8);
        assert_eq!(pending.pids().count(), 0);
    }

    #[test]
    fn construction_cancellation_and_parent_disappearance_preserve_the_claim() {
        let mut pending = Pending::<Resource, 1>::new();
        let drops = Arc::new(AtomicUsize::new(0));
        for (n, error) in [(1, Error::ParentGone), (2, Error::Cancelled)] {
            let ticket = pending.reserve(PID, PARENT, false).unwrap();
            if error == Error::Cancelled {
                assert!(matches!(pending.cancel(ticket, None), Ok(None)));
                assert!(matches!(
                    pending.cancel(ticket, None),
                    Err(Error::Cancelled)
                ));
            }
            let result = pending.finish(identity(ticket, n), Resource(drops.clone()), |parent| {
                parent.incarnation == PARENT.incarnation + 1
            });
            let (actual, resources) = result.err().unwrap();
            assert_eq!(actual, error);
            assert_eq!(pending.reserve(PID, PARENT, false), Err(Error::Collision));
            drop(resources);
            pending.reclaimed(ticket);
            assert_eq!(drops.load(Ordering::Relaxed), n as usize);
        }
    }

    #[test]
    fn cancellation_and_test_transfer_have_one_winner_in_both_orders() {
        for transfer_first in [true, false] {
            let pending = Mutex::new(Pending::<Resource, 1>::new());
            let drops = Arc::new(AtomicUsize::new(0));
            let ticket = pending.lock().unwrap().reserve(PID, PARENT, false).unwrap();
            let current = identity(ticket, 1);
            assert!(pending
                .lock()
                .unwrap()
                .finish(current, Resource(drops.clone()), |_| true)
                .is_ok());
            assert_eq!(
                pending.lock().unwrap().eligible(current, false),
                Err(Error::ParentGone)
            );
            pending.lock().unwrap().eligible(current, true).unwrap();
            assert!(matches!(
                pending.lock().unwrap().transfer(current, false),
                Err(Error::ParentGone)
            ));
            let (finished, wait) = mpsc::channel();
            std::thread::scope(|scope| {
                let pending = &pending;
                let winner = scope.spawn(move || {
                    let resource = if transfer_first {
                        pending.lock().unwrap().transfer(current, true).unwrap()
                    } else {
                        pending
                            .lock()
                            .unwrap()
                            .cancel(ticket, Some(current))
                            .unwrap()
                            .unwrap()
                    };
                    finished.send(()).unwrap();
                    resource
                });
                wait.recv().unwrap();
                let mut pending = pending.lock().unwrap();
                assert!(pending.cancel(ticket, Some(current)).is_err());
                assert!(pending.transfer(current, true).is_err());
                assert_eq!(pending.reserve(PID, PARENT, false), Err(Error::Collision));
                drop(winner.join().unwrap());
                if !transfer_first {
                    pending.reclaimed(ticket);
                }
            });
            assert_eq!(drops.load(Ordering::Relaxed), 1);
        }
    }
}
