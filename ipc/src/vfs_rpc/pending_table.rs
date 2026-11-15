//! Bounded ownership and reply routing for a future kernel dispatch adapter.
//! Identities and resources MUST be authenticated/acquired by that adapter.
//! Serialize all methods with caller/service teardown and publication. This
//! table performs no IPC, memory mapping, credential derivation, or wakeups.
use super::codec::Request;
use super::lifecycle::{Caller, Failure, Pending, RequestIds, Service, State, TransitionError};
use super::response::ReplyMessage;
use super::{ProtocolError, Transfer};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdmissionError {
    Full,
    Exhausted,
    Protocol(ProtocolError),
}

/// Preparation failure leaves no entry; allocated IDs are never returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrepareError<E> {
    Admission(AdmissionError),
    Resource(E),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RouteError {
    UnknownRequest,
    Transition(TransitionError),
}

struct Entry<R> {
    id: u64,
    caller: Caller,
    service: Service,
    pending: Pending,
    resources: R,
}

/// Owns resources until explicitly reaped, including after terminal completion.
/// A terminal entry still consumes capacity so result delivery cannot lose its
/// resources to slot reuse. R should own retained buffers, caller/VM and namespace
/// references; it need not implement Clone or Copy. Dropping the table drops R.
/// The adapter must retain one table for the entire boot, including service
/// replacement. Reconstructing it resets IDs and is forbidden during operation.
pub struct Table<R, const N: usize> {
    ids: RequestIds,
    entries: [Option<Entry<R>>; N],
}

pub struct Retired<R> {
    pub request_id: u64,
    pub state: State,
    pub resources: R,
}

impl<R, const N: usize> Default for Table<R, N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<R, const N: usize> Table<R, N> {
    pub const fn new() -> Self {
        Self {
            ids: RequestIds::new(),
            entries: [const { None }; N],
        }
    }

    pub fn len(&self) -> usize {
        self.entries.iter().flatten().count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns ownership on every admission failure. The generated transfer is
    /// for a separate output allocation at offset zero. The caller must frame
    /// immutable input using this ID, then publish only after admission. A
    /// publication failure terminalizes this entry; it never retries/reuses IDs.
    pub fn admit(
        &mut self,
        caller: Caller,
        service: Service,
        request: Request<'_>,
        output_bytes: u64,
        resources: R,
    ) -> Result<Transfer, (AdmissionError, R)> {
        let mut resources = Some(resources);
        match self.admit_with(caller, service, request, output_bytes, |_| {
            Ok::<R, core::convert::Infallible>(resources.take().unwrap())
        }) {
            Ok(transfer) => Ok(transfer),
            Err(PrepareError::Admission(error)) => Err((error, resources.unwrap())),
            Err(PrepareError::Resource(never)) => match never {},
        }
    }

    /// Prepare resources using only the table-generated ID. Capacity and wire
    /// contract validation precede the callback. Callback failure leaves no
    /// entry and burns the ID. Success transfers ownership directly to the
    /// entry; no fallible work follows preparation. This does not publish IPC.
    ///
    /// The callback must not reenter this table or wait for a remote service.
    /// Caller/service and namespace lifecycle serialization remains the adapter's
    /// responsibility. Any partial resources must roll back through ownership.
    pub fn admit_with<E>(
        &mut self,
        caller: Caller,
        service: Service,
        request: Request<'_>,
        output_bytes: u64,
        prepare: impl FnOnce(Transfer) -> Result<R, E>,
    ) -> Result<Transfer, PrepareError<E>> {
        if [
            caller.pid,
            caller.incarnation,
            caller.address_space_generation,
            service.owner_pid,
            service.owner_incarnation,
            service.mount_epoch,
        ]
        .contains(&0)
        {
            return Err(PrepareError::Admission(AdmissionError::Protocol(
                ProtocolError::Encoding,
            )));
        }
        let slot = self
            .entries
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(PrepareError::Admission(AdmissionError::Full))?;
        let id = self
            .ids
            .allocate()
            .ok_or(PrepareError::Admission(AdmissionError::Exhausted))?;
        let transfer = Transfer::decode([super::VERSION, id, 0, output_bytes], output_bytes)
            .map_err(|error| PrepareError::Admission(AdmissionError::Protocol(error)))?;
        let pending = Pending::new(caller, service, transfer, request)
            .map_err(|error| PrepareError::Admission(AdmissionError::Protocol(error)))?;
        let resources = prepare(transfer).map_err(PrepareError::Resource)?;
        *slot = Some(Entry {
            id,
            caller,
            service,
            pending,
            resources,
        });
        Ok(transfer)
    }

    pub fn state(&self, id: u64) -> Option<State> {
        self.entries
            .iter()
            .flatten()
            .find(|entry| entry.id == id)
            .map(|entry| entry.pending.state())
    }

    /// Source is the authenticated transport identity, never reply payload data.
    /// Wrong sources/IDs leave all entries unchanged. The retained caller is
    /// checked by lifecycle teardown, not supplied by the replying service.
    pub fn complete(
        &mut self,
        source: Service,
        reply: ReplyMessage<'_>,
    ) -> Result<State, RouteError> {
        let entry = self
            .entries
            .iter_mut()
            .flatten()
            .find(|entry| entry.id == reply.words[1])
            .ok_or(RouteError::UnknownRequest)?;
        entry
            .pending
            .complete(entry.caller, source, reply)
            .map_err(RouteError::Transition)?;
        Ok(entry.pending.state())
    }

    pub fn fail(&mut self, id: u64, failure: Failure) -> Result<(), RouteError> {
        self.entries
            .iter_mut()
            .flatten()
            .find(|entry| entry.id == id)
            .ok_or(RouteError::UnknownRequest)?
            .pending
            .fail(failure)
            .map_err(RouteError::Transition)
    }

    pub fn retire_caller(&mut self, caller: Caller) -> usize {
        self.fail_matching(|entry| entry.caller == caller, Failure::CallerRetired)
    }

    pub fn retire_service(&mut self, service: Service) -> usize {
        self.fail_matching(|entry| entry.service == service, Failure::ServiceRetired)
    }

    fn fail_matching(&mut self, matches: impl Fn(&Entry<R>) -> bool, failure: Failure) -> usize {
        let mut count = 0;
        for entry in self.entries.iter_mut().flatten() {
            if matches(entry) && entry.pending.fail(failure).is_ok() {
                count += 1;
            }
        }
        count
    }

    /// Transfer terminal resources exactly once to delivery/cleanup. Never call
    /// resource destructors or copy out while holding the future kernel lock.
    pub fn reap(&mut self, id: u64) -> Option<Retired<R>> {
        let slot = self.entries.iter_mut().find(|slot| {
            slot.as_ref()
                .is_some_and(|entry| entry.id == id && entry.pending.state() != State::Waiting)
        })?;
        let entry = slot.take()?;
        Some(Retired {
            request_id: entry.id,
            state: entry.pending.state(),
            resources: entry.resources,
        })
    }

    /// Drain terminal cleanup without a separate index of request IDs. Waiting
    /// entries remain owned even when interleaved with completed entries.
    pub fn reap_next(&mut self) -> Option<Retired<R>> {
        let id = self
            .entries
            .iter()
            .flatten()
            .find(|entry| entry.pending.state() != State::Waiting)?
            .id;
        self.reap(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::cell::Cell;
    const CALLER: Caller = Caller {
        pid: 5,
        incarnation: 2,
        address_space_generation: 3,
    };
    const SERVICE: Service = Service {
        endpoint: 0,
        owner_pid: 3,
        owner_incarnation: 1,
        mount_epoch: 9,
    };
    const REQUEST: Request<'static> = Request::Stat { path: "/" };
    const BYTES: u64 = super::super::response::METADATA_BYTES as u64;
    fn admit<R, const N: usize>(table: &mut Table<R, N>, resource: R) -> u64 {
        match table.admit(CALLER, SERVICE, REQUEST, BYTES, resource) {
            Ok(transfer) => transfer.words()[1],
            Err(_) => panic!("admission failed"),
        }
    }
    fn reply(id: u64) -> ReplyMessage<'static> {
        // Valid service error, no metadata body required.
        ReplyMessage {
            label: super::super::REPLY,
            words: [1, id, 2, 0],
            word_count: 4,
            cap_count: 0,
            output: &[],
        }
    }
    #[test]
    fn transactional_preparation_failure_and_capacity() {
        let mut table = Table::<u64, 1>::new();
        let mut failed_id = 0;
        assert_eq!(
            table.admit_with(CALLER, SERVICE, REQUEST, BYTES, |transfer| {
                failed_id = transfer.words()[1];
                Err("allocation")
            }),
            Err(PrepareError::Resource("allocation"))
        );
        assert!(table.is_empty());
        assert_eq!(table.state(failed_id), None);
        // Invalid contracts do not invoke preparation.
        assert!(matches!(
            table.admit_with(CALLER, SERVICE, REQUEST, 0, |_| {
                panic!("invalid contract prepared")
            }),
            Err(PrepareError::<()>::Admission(AdmissionError::Protocol(_)))
        ));
        let transfer = table
            .admit_with(CALLER, SERVICE, REQUEST, BYTES, |transfer| {
                Ok::<_, ()>(transfer.words()[1])
            })
            .unwrap();
        let id = transfer.words()[1];
        assert!(id > failed_id);
        assert_eq!(
            table.admit_with(CALLER, SERVICE, REQUEST, BYTES, |_| {
                panic!("full table prepared")
            }),
            Err(PrepareError::<()>::Admission(AdmissionError::Full))
        );
        table.fail(id, Failure::SubmissionFailed).unwrap();
        // Terminal entries retain capacity until reap.
        assert_eq!(
            table.admit_with(CALLER, SERVICE, REQUEST, BYTES, |_| {
                panic!("terminal entry lost ownership")
            }),
            Err(PrepareError::<()>::Admission(AdmissionError::Full))
        );
        assert_eq!(table.reap(id).unwrap().resources, id);
        let next = table
            .admit_with(CALLER, SERVICE, REQUEST, BYTES, |t| {
                Ok::<_, ()>(t.words()[1])
            })
            .unwrap();
        assert!(next.words()[1] > id);
    }

    struct Resource<'a>(&'a Cell<usize>);
    impl Drop for Resource<'_> {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }

    #[test]
    fn capacity_includes_terminal_entries_until_resources_are_reaped() {
        let drops = Cell::new(0);
        let mut table = Table::<_, 1>::new();
        let id = admit(&mut table, Resource(&drops));
        assert!(table.reap(id).is_none());
        assert_eq!(table.fail(id, Failure::SubmissionFailed), Ok(()));
        let (error, resource) = table
            .admit(CALLER, SERVICE, REQUEST, BYTES, Resource(&drops))
            .err()
            .unwrap();
        assert_eq!(error, AdmissionError::Full);
        assert_eq!(drops.get(), 0);
        drop(resource);
        let retired = table.reap(id).unwrap();
        assert_eq!(retired.request_id, id);
        assert_eq!(retired.state, State::Failed(Failure::SubmissionFailed));
        assert!(table.is_empty());
        assert!(table.reap(id).is_none());
        assert_eq!(drops.get(), 1);
        drop(retired.resources);
        assert_eq!(drops.get(), 2);
        let replacement = admit(&mut table, Resource(&drops));
        assert!(replacement > id);
        assert_eq!(
            table.complete(SERVICE, reply(id)),
            Err(RouteError::UnknownRequest)
        );
        assert_eq!(table.state(replacement), Some(State::Waiting));
        drop(table);
        assert_eq!(drops.get(), 3);
    }

    #[test]
    fn wrong_sources_and_unknown_ids_preserve_multiple_pending_calls() {
        let mut table = Table::<_, 2>::new();
        let a = admit(&mut table, 10);
        let b = admit(&mut table, 20);
        for source in [
            Service {
                endpoint: 1,
                ..SERVICE
            },
            Service {
                owner_pid: 4,
                ..SERVICE
            },
            Service {
                owner_incarnation: 2,
                ..SERVICE
            },
            Service {
                mount_epoch: 10,
                ..SERVICE
            },
        ] {
            assert_eq!(
                table.complete(source, reply(a)),
                Err(RouteError::Transition(TransitionError::StaleService))
            );
            assert_eq!(table.state(a), Some(State::Waiting));
            assert_eq!(table.state(b), Some(State::Waiting));
        }
        assert_eq!(
            table.complete(SERVICE, reply(b + 1)),
            Err(RouteError::UnknownRequest)
        );
        table.complete(SERVICE, reply(b)).unwrap();
        assert_eq!(
            table.complete(SERVICE, reply(b)),
            Err(RouteError::Transition(TransitionError::Terminal))
        );
        assert_eq!(table.state(a), Some(State::Waiting));
        assert_eq!(table.reap(b).unwrap().resources, 20);
        table.complete(SERVICE, reply(a)).unwrap();
        assert_eq!(table.reap(a).unwrap().resources, 10);
    }

    #[test]
    fn malformed_reply_fails_only_its_selected_operation() {
        for malformed in 0..5 {
            let mut table = Table::<_, 2>::new();
            let a = admit(&mut table, ());
            let b = admit(&mut table, ());
            let mut message = reply(a);
            match malformed {
                0 => message.label = super::super::REQUEST,
                1 => message.word_count = 3,
                2 => message.cap_count = 1,
                3 => message.words[3] = BYTES + 1,
                _ => message.words[2] = 0, // missing successful stat body
            }
            assert!(matches!(
                table.complete(SERVICE, message),
                Err(RouteError::Transition(TransitionError::Protocol(_)))
            ));
            assert_eq!(table.state(a), Some(State::Failed(Failure::MalformedReply)));
            assert_eq!(table.state(b), Some(State::Waiting));
            table.complete(SERVICE, reply(b)).unwrap();
        }
    }

    #[test]
    fn teardown_matches_full_identity_and_preserves_first_terminal_state() {
        let mut table = Table::<_, 3>::new();
        let a = admit(&mut table, ());
        let new_caller = Caller {
            address_space_generation: 4,
            ..CALLER
        };
        let new_service = Service {
            mount_epoch: 10,
            ..SERVICE
        };
        let b = table
            .admit(new_caller, SERVICE, REQUEST, BYTES, ())
            .unwrap()
            .words()[1];
        let c = table
            .admit(CALLER, new_service, REQUEST, BYTES, ())
            .unwrap()
            .words()[1];
        table.complete(new_service, reply(c)).unwrap();
        assert_eq!(table.retire_caller(CALLER), 1);
        assert_eq!(table.retire_caller(CALLER), 0);
        assert_eq!(table.state(a), Some(State::Failed(Failure::CallerRetired)));
        assert_eq!(table.state(b), Some(State::Waiting));
        assert!(matches!(table.state(c), Some(State::Completed(_))));
        assert_eq!(table.retire_service(SERVICE), 1);
        assert_eq!(table.retire_service(SERVICE), 0);
        assert_eq!(table.state(b), Some(State::Failed(Failure::ServiceRetired)));
        assert!(matches!(table.state(c), Some(State::Completed(_))));
        assert_eq!(
            table.complete(SERVICE, reply(a)),
            Err(RouteError::Transition(TransitionError::Terminal))
        );
    }

    #[test]
    fn failed_admission_returns_resources_and_does_not_occupy_slots() {
        let mut table = Table::<_, 1>::new();
        for bytes in [BYTES - 1, super::super::MAX_TRANSFER + 1] {
            assert_eq!(
                table.admit(CALLER, SERVICE, REQUEST, bytes, 99).err(),
                Some((AdmissionError::Protocol(ProtocolError::Bounds), 99))
            );
            assert!(table.is_empty());
        }
        assert_eq!(
            table
                .admit(
                    Caller {
                        incarnation: 0,
                        ..CALLER
                    },
                    SERVICE,
                    REQUEST,
                    BYTES,
                    99
                )
                .err(),
            Some((AdmissionError::Protocol(ProtocolError::Encoding), 99))
        );
        let mut empty = Table::<_, 0>::new();
        assert_eq!(
            empty.admit(CALLER, SERVICE, REQUEST, BYTES, 99).err(),
            Some((AdmissionError::Full, 99))
        );
        assert!(empty.is_empty());
        admit(&mut table, 99);
        assert_eq!(table.len(), 1);
    }

    #[test]
    fn successful_stat_and_teardown_obey_both_terminal_orders() {
        use super::super::response::{Kind, Metadata, Provenance, Reply, ReplyContract};
        let metadata = Metadata {
            provenance: Provenance {
                mount_id: 1,
                mount_generation: 2,
                object_id: 3,
                object_generation: 4,
            },
            kind: Kind::File,
            mode: 0o640,
            uid: 5,
            gid: 6,
            links: 1,
            size: 99,
        };
        let mut output = [0; BYTES as usize];
        ReplyContract::for_request(REQUEST)
            .unwrap()
            .encode(Reply::Metadata(metadata), &mut output)
            .unwrap();
        for reply_first in [false, true] {
            let drops = Cell::new(0);
            let mut table = Table::<_, 1>::new();
            let id = admit(&mut table, Resource(&drops));
            let message = ReplyMessage {
                words: [1, id, 0, BYTES],
                output: &output,
                ..reply(id)
            };
            if reply_first {
                table.complete(SERVICE, message).unwrap();
                assert_eq!(table.retire_service(SERVICE), 0);
                assert_eq!(table.retire_caller(CALLER), 0);
                assert!(matches!(
                    table.state(id),
                    Some(State::Completed(super::super::Completion {
                        errno: 0,
                        output_bytes: BYTES
                    }))
                ));
            } else {
                assert_eq!(table.retire_service(SERVICE), 1);
                assert_eq!(
                    table.complete(SERVICE, message),
                    Err(RouteError::Transition(TransitionError::Terminal))
                );
                assert_eq!(
                    table.state(id),
                    Some(State::Failed(Failure::ServiceRetired))
                );
            }
            assert_eq!(drops.get(), 0);
            drop(table.reap(id).unwrap());
            assert_eq!(drops.get(), 1);
            assert!(table.reap(id).is_none());
            drop(table);
            assert_eq!(drops.get(), 1);
        }
    }

    #[test]
    fn teardown_drain_skips_waiters_and_releases_each_terminal_bundle_once() {
        let drops = Cell::new(0);
        let mut table = Table::<_, 3>::new();
        let a = admit(&mut table, Resource(&drops));
        let b = admit(&mut table, Resource(&drops));
        let c = admit(&mut table, Resource(&drops));
        table.complete(SERVICE, reply(b)).unwrap();
        table.fail(c, Failure::Cancelled).unwrap();
        assert_eq!(table.reap_next().map(|entry| entry.request_id), Some(b));
        assert_eq!(table.reap_next().map(|entry| entry.request_id), Some(c));
        assert!(table.reap_next().is_none());
        assert_eq!(drops.get(), 2);
        assert_eq!(table.state(a), Some(State::Waiting));
        assert_eq!(table.retire_service(SERVICE), 1);
        assert_eq!(table.reap_next().map(|entry| entry.request_id), Some(a));
        assert!(table.is_empty());
        assert!(table.reap_next().is_none());
        assert_eq!(drops.get(), 3);
    }
}
