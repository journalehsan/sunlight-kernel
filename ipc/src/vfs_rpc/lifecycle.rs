//! Allocation-free pending-operation model for the kernel boundary.
//! This module does not authenticate identities or own buffers/descriptions.
//! Integration must retain those resources and serialize transitions under a
//! short kernel lock; never hold that lock while waiting for the service.
use super::codec::Request;
use super::response::{ReplyContract, ReplyMessage};
use super::{Completion, ProtocolError, Transfer};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Caller {
    pub pid: u64,
    pub incarnation: u64,
    pub address_space_generation: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Service {
    pub endpoint: u64,
    pub owner_pid: u64,
    pub owner_incarnation: u64,
    pub mount_epoch: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    SubmissionFailed,
    Cancelled,
    TimedOut,
    CallerRetired,
    ServiceRetired,
    MalformedReply,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Waiting,
    Completed(Completion),
    Failed(Failure),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransitionError {
    StaleCaller,
    StaleService,
    Terminal,
    Protocol(ProtocolError),
}

/// Monotonic, boot-scoped IDs. Exhaustion is permanent, including after the
/// largest ID has been handed out; an old request cannot be resurrected.
#[derive(Debug)]
pub struct RequestIds {
    next: Option<core::num::NonZeroU64>,
}
impl Default for RequestIds {
    fn default() -> Self {
        Self::new()
    }
}
impl RequestIds {
    pub const fn new() -> Self {
        Self {
            next: core::num::NonZeroU64::new(1),
        }
    }
    pub fn allocate(&mut self) -> Option<u64> {
        let id = self.next?.get();
        self.next = id.checked_add(1).and_then(core::num::NonZeroU64::new);
        Some(id)
    }
}

#[derive(Debug)]
pub struct Pending {
    caller: Caller,
    service: Service,
    transfer: Transfer,
    contract: ReplyContract,
    state: State,
}
impl Pending {
    /// Identity values must come from the kernel's authenticated process and
    /// endpoint records, never from a service/client request payload.
    pub fn new(
        caller: Caller,
        service: Service,
        transfer: Transfer,
        request: Request<'_>,
    ) -> Result<Self, ProtocolError> {
        let contract = ReplyContract::for_request(request)?;
        Self::with_contract(caller, service, transfer, contract, contract.buffer_bytes())
    }

    /// Separate immutable input and writable output. The adapter must retain
    /// the input independently; transfer words bound only the result allocation.
    pub fn new_copied(
        caller: Caller,
        service: Service,
        transfer: Transfer,
        request: Request<'_>,
    ) -> Result<Self, ProtocolError> {
        let contract = ReplyContract::for_request(request)?;
        Self::with_contract(
            caller,
            service,
            transfer,
            contract,
            contract.output_capacity,
        )
    }

    fn with_contract(
        caller: Caller,
        service: Service,
        transfer: Transfer,
        contract: ReplyContract,
        required: usize,
    ) -> Result<Self, ProtocolError> {
        if transfer.words()[3] < required as u64 {
            return Err(ProtocolError::Bounds);
        }
        Ok(Self {
            caller,
            service,
            transfer,
            contract,
            state: State::Waiting,
        })
    }
    pub const fn state(&self) -> State {
        self.state
    }
    fn waiting(&self) -> Result<(), TransitionError> {
        if self.state != State::Waiting {
            return Err(TransitionError::Terminal);
        }
        Ok(())
    }
    pub fn fail(&mut self, failure: Failure) -> Result<(), TransitionError> {
        self.waiting()?;
        self.state = State::Failed(failure);
        Ok(())
    }
    /// Only the bound service may submit a completion. A different endpoint or
    /// incarnation cannot cancel the actual operation with a forged bad reply.
    /// Caller retirement makes any completion permanently unusable.
    pub fn complete(
        &mut self,
        caller: Caller,
        service: Service,
        reply: ReplyMessage<'_>,
    ) -> Result<Completion, TransitionError> {
        self.waiting()?;
        if service != self.service {
            return Err(TransitionError::StaleService);
        }
        // An unknown ID belongs to no operation here. Even an authenticated
        // service cannot terminate a different pending call by misrouting it.
        if reply.words[1] != self.transfer.words()[1] {
            return Err(TransitionError::Protocol(ProtocolError::RequestId));
        }
        if caller != self.caller {
            self.state = State::Failed(Failure::CallerRetired);
            return Err(TransitionError::StaleCaller);
        }
        match self.contract.complete(self.transfer, reply) {
            Ok(completion) => {
                self.state = State::Completed(completion);
                Ok(completion)
            }
            Err(error) => {
                self.state = State::Failed(Failure::MalformedReply);
                Err(TransitionError::Protocol(error))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const CALLER: Caller = Caller {
        pid: 5,
        incarnation: 2,
        address_space_generation: 3,
    };
    const SERVICE: Service = Service {
        endpoint: 8,
        owner_pid: 3,
        owner_incarnation: 1,
        mount_epoch: 9,
    };
    const REPLY: [u64; 4] = [1, 7, 0, 4];
    fn pending() -> Pending {
        Pending::new(
            CALLER,
            SERVICE,
            Transfer::decode([1, 7, 0, 22], 22).unwrap(),
            Request::Read {
                handle: 1,
                offset: 0,
                count: 8,
            },
        )
        .unwrap()
    }
    fn reply(words: [u64; 4]) -> ReplyMessage<'static> {
        ReplyMessage {
            label: super::super::REPLY,
            words,
            word_count: 4,
            cap_count: 0,
            output: b"test",
        }
    }
    #[test]
    fn first_terminal_transition_wins_in_either_order() {
        for failure in [
            Failure::SubmissionFailed,
            Failure::Cancelled,
            Failure::TimedOut,
            Failure::ServiceRetired,
            Failure::CallerRetired,
        ] {
            let mut p = pending();
            p.fail(failure).unwrap();
            assert_eq!(
                p.complete(CALLER, SERVICE, reply(REPLY)),
                Err(TransitionError::Terminal)
            );
            assert_eq!(p.state(), State::Failed(failure));
            let mut p = pending();
            p.complete(CALLER, SERVICE, reply(REPLY)).unwrap();
            assert_eq!(p.fail(failure), Err(TransitionError::Terminal));
            assert_eq!(
                p.complete(CALLER, SERVICE, reply(REPLY)),
                Err(TransitionError::Terminal)
            );
        }
    }
    #[test]
    fn exec_pid_reuse_and_service_replacement_cannot_complete_old_calls() {
        for caller in [
            Caller {
                incarnation: 3,
                ..CALLER
            },
            Caller {
                address_space_generation: 4,
                ..CALLER
            },
            Caller { pid: 6, ..CALLER },
        ] {
            let mut p = pending();
            assert_eq!(
                p.complete(caller, SERVICE, reply(REPLY)),
                Err(TransitionError::StaleCaller)
            );
            assert_eq!(p.state(), State::Failed(Failure::CallerRetired));
        }
        for service in [
            Service {
                endpoint: 9,
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
            let mut p = pending();
            assert_eq!(
                p.complete(CALLER, service, reply(REPLY)),
                Err(TransitionError::StaleService)
            );
            assert_eq!(p.state(), State::Waiting);
            assert!(p.complete(CALLER, SERVICE, reply(REPLY)).is_ok());
        }
    }
    #[test]
    fn malformed_authenticated_reply_fails_once() {
        let mut p = pending();
        assert_eq!(
            p.complete(CALLER, SERVICE, reply([1, 7, 0, 23])),
            Err(TransitionError::Protocol(ProtocolError::Bounds))
        );
        assert_eq!(p.state(), State::Failed(Failure::MalformedReply));
    }
    #[test]
    fn reply_that_fits_transfer_but_exceeds_operation_fails_once() {
        // Input is 22 bytes while requested output is only eight. The transport
        // alone would accept nine initialized bytes in this allocation.
        let mut p = pending();
        let message = ReplyMessage {
            output: b"123456789",
            ..reply([1, 7, 0, 9])
        };
        assert_eq!(
            p.complete(CALLER, SERVICE, message),
            Err(TransitionError::Protocol(ProtocolError::Bounds))
        );
        assert_eq!(p.state(), State::Failed(Failure::MalformedReply));
        assert_eq!(
            p.complete(CALLER, SERVICE, reply(REPLY)),
            Err(TransitionError::Terminal)
        );
    }
    #[test]
    fn pending_admission_requires_space_for_both_input_and_output() {
        for (request, required) in [
            (
                Request::Read {
                    handle: 1,
                    offset: 0,
                    count: 8,
                },
                22,
            ),
            (
                Request::Stat { path: "/" },
                super::super::response::METADATA_BYTES,
            ),
            (
                Request::Open {
                    path: "/",
                    flags: 0,
                    mode: 0,
                },
                super::super::response::OPEN_BYTES,
            ),
        ] {
            assert!(Pending::new(
                CALLER,
                SERVICE,
                Transfer::decode([1, 7, 0, required as u64 - 1], required as u64).unwrap(),
                request
            )
            .is_err());
            assert!(Pending::new(
                CALLER,
                SERVICE,
                Transfer::decode([1, 7, 0, required as u64], required as u64).unwrap(),
                request
            )
            .is_ok());
        }
    }
    #[test]
    fn malformed_envelopes_and_snapshot_length_fail_only_authenticated_operations() {
        for message in [
            ReplyMessage {
                label: super::super::REQUEST,
                ..reply(REPLY)
            },
            ReplyMessage {
                cap_count: 1,
                ..reply(REPLY)
            },
            ReplyMessage {
                word_count: 3,
                ..reply(REPLY)
            },
            ReplyMessage {
                output: b"",
                ..reply(REPLY)
            },
            reply([1, 7, 5, 4]),
        ] {
            let mut p = pending();
            assert_eq!(
                p.complete(
                    CALLER,
                    Service {
                        owner_incarnation: 2,
                        ..SERVICE
                    },
                    ReplyMessage {
                        output: message.output,
                        ..reply(message.words)
                    }
                ),
                Err(TransitionError::StaleService)
            );
            assert_eq!(p.state(), State::Waiting);
            assert!(p.complete(CALLER, SERVICE, message).is_err());
            assert_eq!(p.state(), State::Failed(Failure::MalformedReply));
        }
        let mut p = pending();
        assert_eq!(
            p.complete(
                CALLER,
                SERVICE,
                ReplyMessage {
                    output: b"",
                    ..reply([1, 7, 5, 0])
                }
            ),
            Ok(Completion {
                errno: 5,
                output_bytes: 0
            })
        );
    }
    #[test]
    fn request_ids_never_wrap() {
        let mut ids = RequestIds::new();
        assert_eq!(ids.allocate(), Some(1));
        assert_eq!(ids.allocate(), Some(2));
        ids.next = core::num::NonZeroU64::new(u64::MAX);
        assert_eq!(ids.allocate(), Some(u64::MAX));
        assert_eq!(ids.allocate(), None);
        assert_eq!(ids.allocate(), None);
    }

    #[test]
    fn unknown_reply_identity_preserves_the_actual_pending_operation() {
        let mut p = pending();
        assert_eq!(
            p.complete(CALLER, SERVICE, reply([1, 8, 0, 4])),
            Err(TransitionError::Protocol(ProtocolError::RequestId))
        );
        assert_eq!(p.state(), State::Waiting);
        assert!(p.complete(CALLER, SERVICE, reply(REPLY)).is_ok());
    }
}
