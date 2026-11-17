//! Fixed-size retention payloads. Decoding is structural only: transport must
//! authenticate kernel origin on requests and service registration on replies.
//! No runtime dispatcher is installed by this module.
use super::{Receipt, Scope};
use crate::vfs_rpc::{
    lifecycle::Caller,
    registration::{EndpointOwner, Registration},
    ProtocolError, VERSION,
};

pub const FRAME_BYTES: usize = 112;

/// Only failures that prove this request acquired no lease may be acknowledged.
/// Replay and identity failures must never become refusals: a replay can refer
/// to an already retained lease whose successful reply was lost.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum Refusal {
    Capacity = 1,
    Exhausted = 2,
    Resolve = 3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Frame {
    /// Trusted copied-channel lifecycle barrier. Namespace words are reserved.
    CallerRetired {
        registration: Registration,
        caller: Caller,
        event_id: u64,
    },
    CallerRetiredAck {
        registration: Registration,
        caller: Caller,
        event_id: u64,
    },
    Retain {
        registration: Registration,
        scope: Scope,
        request_id: u64,
    },
    Retained(Receipt),
    Release(Receipt),
    Released(Receipt),
    Refused {
        request: PendingRetain,
        reason: Refusal,
    },
}

impl Frame {
    pub fn encode(self) -> Result<[u8; FRAME_BYTES], ProtocolError> {
        let (op, registration, scope, request_id, generation) = match self {
            Self::CallerRetired {
                registration,
                caller,
                event_id,
            }
            | Self::CallerRetiredAck {
                registration,
                caller,
                event_id,
            } => (
                if matches!(self, Self::CallerRetired { .. }) {
                    6
                } else {
                    7
                },
                registration,
                Scope {
                    caller,
                    namespace_id: 0,
                    namespace_generation: 0,
                },
                event_id,
                0,
            ),
            Self::Retain {
                registration,
                scope,
                request_id,
            } => (1, registration, scope, request_id, 0),
            Self::Retained(r) => (2, r.registration, r.scope, r.request_id, r.generation),
            Self::Release(r) => (3, r.registration, r.scope, r.request_id, r.generation),
            Self::Released(r) => (4, r.registration, r.scope, r.request_id, r.generation),
            Self::Refused { request: p, reason } => {
                (5, p.registration, p.scope, p.request_id, reason as u64)
            }
        };
        let owner = registration.owner;
        let words = [
            VERSION,
            op,
            owner.endpoint,
            owner.pid,
            owner.incarnation,
            owner.address_space_generation,
            registration.epoch,
            scope.caller.pid,
            scope.caller.incarnation,
            scope.caller.address_space_generation,
            scope.namespace_id,
            scope.namespace_generation,
            request_id,
            generation,
        ];
        let mut bytes = [0; FRAME_BYTES];
        for (word, chunk) in words.iter().zip(bytes.chunks_exact_mut(8)) {
            chunk.copy_from_slice(&word.to_le_bytes());
        }
        Self::decode(&bytes)?;
        Ok(bytes)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, ProtocolError> {
        if bytes.len() != FRAME_BYTES {
            return Err(ProtocolError::Encoding);
        }
        let mut w = [0; 14];
        for (word, chunk) in w.iter_mut().zip(bytes.chunks_exact(8)) {
            *word = u64::from_le_bytes(chunk.try_into().unwrap());
        }
        if w[0] != VERSION {
            return Err(ProtocolError::Version);
        }
        if !(1..=7).contains(&w[1]) {
            return Err(ProtocolError::Operation);
        }
        // Endpoint zero is valid. Identity and correlation words are nonzero;
        // lifecycle records reserve the namespace and lease-generation words.
        let lifecycle = w[1] >= 6;
        if w[3..10].contains(&0)
            || w[12] == 0
            || if lifecycle {
                w[10] != 0 || w[11] != 0 || w[13] != 0
            } else {
                w[10..12].contains(&0) || (w[1] == 1) != (w[13] == 0)
            }
        {
            return Err(ProtocolError::Encoding);
        }
        let registration = Registration {
            owner: EndpointOwner {
                endpoint: w[2],
                pid: w[3],
                incarnation: w[4],
                address_space_generation: w[5],
            },
            epoch: w[6],
        };
        let scope = Scope {
            caller: Caller {
                pid: w[7],
                incarnation: w[8],
                address_space_generation: w[9],
            },
            namespace_id: w[10],
            namespace_generation: w[11],
        };
        let r = Receipt {
            registration,
            scope,
            request_id: w[12],
            generation: w[13],
        };
        Ok(match w[1] {
            6 => Self::CallerRetired {
                registration,
                caller: scope.caller,
                event_id: w[12],
            },
            7 => Self::CallerRetiredAck {
                registration,
                caller: scope.caller,
                event_id: w[12],
            },
            1 => Self::Retain {
                registration,
                scope,
                request_id: w[12],
            },
            2 => Self::Retained(r),
            3 => Self::Release(r),
            4 => Self::Released(r),
            5 => Self::Refused {
                request: PendingRetain {
                    registration,
                    scope,
                    request_id: w[12],
                },
                reason: match w[13] {
                    1 => Refusal::Capacity,
                    2 => Refusal::Exhausted,
                    3 => Refusal::Resolve,
                    _ => return Err(ProtocolError::Encoding),
                },
            },
            _ => unreachable!(),
        })
    }
}

/// Kernel-side correlation snapshot. Retain this across lost acknowledgements;
/// a timeout grants neither ownership nor permission to release an unknown lease.
/// Callers must revalidate lifecycle state under locks before publication. This
/// validator alone does not install a namespace or establish transport authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PendingRetain {
    registration: Registration,
    scope: Scope,
    request_id: u64,
}
impl PendingRetain {
    pub fn new(
        registration: Registration,
        scope: Scope,
        request_id: u64,
    ) -> Result<Self, ProtocolError> {
        Frame::Retain {
            registration,
            scope,
            request_id,
        }
        .encode()?;
        Ok(Self {
            registration,
            scope,
            request_id,
        })
    }
    pub const fn request(self) -> Frame {
        Frame::Retain {
            registration: self.registration,
            scope: self.scope,
            request_id: self.request_id,
        }
    }
    /// Validate a refusal against the exact pending request and transport identity.
    pub fn refuse(
        self,
        authenticated: Registration,
        bytes: &[u8],
    ) -> Result<Refusal, ProtocolError> {
        if authenticated != self.registration {
            return Err(ProtocolError::Encoding);
        }
        match Frame::decode(bytes)? {
            Frame::Refused { request, reason } if request == self => Ok(reason),
            _ => Err(ProtocolError::Encoding),
        }
    }

    /// `authenticated` must be captured independently of the reply payload.
    pub fn acknowledge(
        self,
        authenticated: Registration,
        bytes: &[u8],
    ) -> Result<Receipt, ProtocolError> {
        if authenticated != self.registration {
            return Err(ProtocolError::Encoding);
        }
        match Frame::decode(bytes)? {
            Frame::Retained(r)
                if r.registration == self.registration
                    && r.scope == self.scope
                    && r.request_id == self.request_id =>
            {
                Ok(r)
            }
            _ => Err(ProtocolError::Encoding),
        }
    }
}

/// Keep the receipt owned and charged until this succeeds or registration drain
/// proves the service can no longer access its references. Safe to retry release.
pub fn acknowledge_release(
    expected: Receipt,
    authenticated: Registration,
    bytes: &[u8],
) -> Result<(), ProtocolError> {
    if authenticated != expected.registration {
        return Err(ProtocolError::Encoding);
    }
    match Frame::decode(bytes)? {
        Frame::Released(r) if r == expected => Ok(()),
        _ => Err(ProtocolError::Encoding),
    }
}

#[cfg(test)]
mod tests {
    use super::super::Table;
    use super::*;
    fn pending() -> PendingRetain {
        PendingRetain::new(
            Registration {
                owner: EndpointOwner {
                    endpoint: 0,
                    pid: 2,
                    incarnation: 3,
                    address_space_generation: 4,
                },
                epoch: 5,
            },
            Scope {
                caller: Caller {
                    pid: 6,
                    incarnation: 7,
                    address_space_generation: 8,
                },
                namespace_id: 9,
                namespace_generation: 10,
            },
            11,
        )
        .unwrap()
    }
    #[test]
    fn lifecycle_frames_require_exact_identity_event_and_reserved_words() {
        let p = pending();
        for frame in [
            Frame::CallerRetired {
                registration: p.registration,
                caller: p.scope.caller,
                event_id: 1,
            },
            Frame::CallerRetiredAck {
                registration: p.registration,
                caller: p.scope.caller,
                event_id: 1,
            },
        ] {
            let bytes = frame.encode().unwrap();
            assert_eq!(Frame::decode(&bytes), Ok(frame));
            for word in [3, 4, 5, 6, 7, 8, 9, 12] {
                let mut invalid = bytes;
                invalid[word * 8..word * 8 + 8].fill(0);
                assert!(Frame::decode(&invalid).is_err());
            }
            for word in [10, 11, 13] {
                let mut invalid = bytes;
                invalid[word * 8] = 1;
                assert!(Frame::decode(&invalid).is_err());
            }
            assert!(Frame::decode(&bytes[..FRAME_BYTES - 1]).is_err());
        }
    }
    fn receipt(p: PendingRetain) -> Receipt {
        Table::<(), 1>::new(p.registration)
            .unwrap()
            .retain(p.registration, p.request_id, p.scope, || Ok::<_, ()>(()))
            .unwrap()
    }
    #[test]
    fn round_trip_and_retry_release() {
        let p = pending();
        let r = receipt(p);
        for frame in [
            p.request(),
            Frame::Retained(r),
            Frame::Release(r),
            Frame::Released(r),
        ] {
            let bytes = frame.encode().unwrap();
            assert_eq!(Frame::decode(&bytes), Ok(frame));
        }
        let mut table = Table::<(), 1>::new(p.registration).unwrap();
        let r = table
            .retain(p.registration, p.request_id, p.scope, || Ok::<_, ()>(()))
            .unwrap();
        let ack = Frame::Retained(r).encode().unwrap();
        assert_eq!(p.acknowledge(p.registration, &ack), Ok(r));
        for _ in 0..2 {
            let Frame::Release(decoded) =
                Frame::decode(&Frame::Release(r).encode().unwrap()).unwrap()
            else {
                panic!()
            };
            let released = table.release(p.registration, decoded).unwrap();
            assert_eq!(
                acknowledge_release(
                    r,
                    p.registration,
                    &Frame::Released(released).encode().unwrap()
                ),
                Ok(())
            );
        }
    }
    #[test]
    fn rejects_malformed_lengths_versions_operations_and_zero_fields() {
        let bytes = pending().request().encode().unwrap();
        for len in 0..FRAME_BYTES {
            assert!(Frame::decode(&bytes[..len]).is_err());
        }
        let mut long = [0; FRAME_BYTES + 1];
        long[..FRAME_BYTES].copy_from_slice(&bytes);
        assert!(Frame::decode(&long).is_err());
        for index in [0, 1, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12] {
            let mut altered = bytes;
            altered[index * 8..index * 8 + 8].fill(0);
            assert!(Frame::decode(&altered).is_err());
        }
        let mut altered = bytes;
        altered[104] = 1;
        assert!(Frame::decode(&altered).is_err());
        let mut altered = Frame::Retained(receipt(pending())).encode().unwrap();
        altered[104..112].fill(0);
        assert!(Frame::decode(&altered).is_err());
    }
    #[test]
    fn every_correlation_field_and_authenticated_registration_must_match() {
        let p = pending();
        let r = receipt(p);
        let ack = Frame::Retained(r).encode().unwrap();
        // Every identity word including endpoint, service VM and lease generation.
        for index in 2..14 {
            let mut altered = ack;
            altered[index * 8] ^= 16;
            if index == 13 {
                // The retain reply establishes the new lease; release must match it.
                assert!(p.acknowledge(p.registration, &altered).is_ok());
            } else {
                assert!(p.acknowledge(p.registration, &altered).is_err());
            }
            altered[8..16].copy_from_slice(&4u64.to_le_bytes());
            assert!(acknowledge_release(r, p.registration, &altered).is_err());
        }
        let changed = Registration {
            epoch: 99,
            ..p.registration
        };
        assert!(p.acknowledge(changed, &ack).is_err());
        assert!(acknowledge_release(r, changed, &Frame::Released(r).encode().unwrap()).is_err());
        for frame in [p.request(), Frame::Release(r), Frame::Released(r)] {
            assert!(p
                .acknowledge(p.registration, &frame.encode().unwrap())
                .is_err());
        }
        assert!(acknowledge_release(r, p.registration, &ack).is_err());
    }
}
