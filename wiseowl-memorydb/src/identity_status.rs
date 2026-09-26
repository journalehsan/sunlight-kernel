//! Bounded, sanitized read-only identity status shared over MemoryDB IPC.

use crate::identity::{IdentityValidationStatus, LoadedIdentity};

pub const IDENTITY_STATUS_VERSION: u16 = 1;
pub const IDENTITY_STATUS_WIRE_LEN: usize = 36;
const MAGIC: &[u8; 4] = b"WIDS";

/// Host serde wrapper uses fixed arrays supported by serde without a length prefix.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub struct IdentityStatusWire {
    pub first: [u8; 32],
    pub last: [u8; 4],
}

impl IdentityStatusWire {
    pub fn new(bytes: [u8; IDENTITY_STATUS_WIRE_LEN]) -> Self {
        let mut first = [0; 32];
        let mut last = [0; 4];
        first.copy_from_slice(&bytes[..32]);
        last.copy_from_slice(&bytes[32..]);
        Self { first, last }
    }

    pub fn as_bytes(&self) -> [u8; IDENTITY_STATUS_WIRE_LEN] {
        let mut bytes = [0; IDENTITY_STATUS_WIRE_LEN];
        bytes[..32].copy_from_slice(&self.first);
        bytes[32..].copy_from_slice(&self.last);
        bytes
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
#[repr(u8)]
pub enum IdentityStatusState {
    Ready = 1,
    PersistenceUnavailable = 2,
    IdentityMissing = 3,
    IdentityInvalid = 4,
    IdentityAmbiguous = 5,
    UnsupportedVersion = 6,
    Suspended = 7,
}

impl IdentityStatusState {
    fn decode(value: u8) -> Option<Self> {
        Some(match value {
            1 => Self::Ready,
            2 => Self::PersistenceUnavailable,
            3 => Self::IdentityMissing,
            4 => Self::IdentityInvalid,
            5 => Self::IdentityAmbiguous,
            6 => Self::UnsupportedVersion,
            7 => Self::Suspended,
            _ => return None,
        })
    }
}

/// Public sanitized status. Fingerprint only; no full ID or mutation handle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub struct IdentityStatus {
    pub state: IdentityStatusState,
    pub fingerprint: [u8; 8],
    pub lineage_sequence: u64,
    pub continuity_generation: u64,
    pub genesis_kind: u8,
    pub identity_format_version: u16,
    pub validation_status: u8,
    pub persistence_available: bool,
}

impl IdentityStatus {
    pub fn ready(identity: LoadedIdentity) -> Self {
        Self {
            state: IdentityStatusState::Ready,
            fingerprint: identity.diagnostic_fingerprint(),
            lineage_sequence: identity.lineage_sequence().get(),
            continuity_generation: identity.continuity_generation().get(),
            genesis_kind: identity.genesis_event_kind() as u8,
            identity_format_version: wiseowl_identity::IDENTITY_FORMAT_VERSION,
            validation_status: match identity.validation_status() {
                IdentityValidationStatus::Validated => 1,
            },
            persistence_available: true,
        }
    }

    pub fn validate(&self) -> bool {
        if self.state == IdentityStatusState::Ready {
            self.persistence_available
                && self.fingerprint != [0; 8]
                && self.lineage_sequence != 0
                && self.continuity_generation != 0
                && matches!(self.genesis_kind, 1 | 2)
                && self.identity_format_version == wiseowl_identity::IDENTITY_FORMAT_VERSION
                && self.validation_status == 1
        } else {
            !self.persistence_available
                && self.fingerprint == [0; 8]
                && self.lineage_sequence == 0
                && self.continuity_generation == 0
                && self.genesis_kind == 0
                && self.identity_format_version == 0
                && self.validation_status == 0
        }
    }

    pub fn encode(self) -> Option<[u8; IDENTITY_STATUS_WIRE_LEN]> {
        if !self.validate() {
            return None;
        }
        let mut out = [0u8; IDENTITY_STATUS_WIRE_LEN];
        out[0..4].copy_from_slice(MAGIC);
        out[4..6].copy_from_slice(&IDENTITY_STATUS_VERSION.to_le_bytes());
        out[6] = self.state as u8;
        out[7] = u8::from(self.persistence_available);
        out[8..16].copy_from_slice(&self.fingerprint);
        out[16..24].copy_from_slice(&self.lineage_sequence.to_le_bytes());
        out[24..32].copy_from_slice(&self.continuity_generation.to_le_bytes());
        out[32] = self.genesis_kind;
        out[33..35].copy_from_slice(&self.identity_format_version.to_le_bytes());
        out[35] = self.validation_status;
        Some(out)
    }

    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != IDENTITY_STATUS_WIRE_LEN
            || &bytes[0..4] != MAGIC
            || u16::from_le_bytes([bytes[4], bytes[5]]) != IDENTITY_STATUS_VERSION
            || bytes[7] > 1
        {
            return None;
        }
        let mut fingerprint = [0; 8];
        fingerprint.copy_from_slice(&bytes[8..16]);
        let out = Self {
            state: IdentityStatusState::decode(bytes[6])?,
            persistence_available: bytes[7] != 0,
            fingerprint,
            lineage_sequence: u64::from_le_bytes(bytes[16..24].try_into().ok()?),
            continuity_generation: u64::from_le_bytes(bytes[24..32].try_into().ok()?),
            genesis_kind: bytes[32],
            identity_format_version: u16::from_le_bytes([bytes[33], bytes[34]]),
            validation_status: bytes[35],
        };
        out.validate().then_some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_wire_is_fixed_bounded_and_rejects_invalid_frames() {
        let status = IdentityStatus {
            state: IdentityStatusState::Ready,
            fingerprint: *b"49DA20E3",
            lineage_sequence: 1,
            continuity_generation: 1,
            genesis_kind: 1,
            identity_format_version: 1,
            validation_status: 1,
            persistence_available: true,
        };
        let encoded = status.encode().unwrap();
        assert_eq!(encoded.len(), IDENTITY_STATUS_WIRE_LEN);
        assert_eq!(IdentityStatus::decode(&encoded), Some(status));
        assert!(IdentityStatus::decode(&encoded[..35]).is_none());
        let mut unknown_version = encoded;
        unknown_version[4] = 2;
        assert!(IdentityStatus::decode(&unknown_version).is_none());
        let mut malformed = encoded;
        malformed[6] = 99;
        assert!(IdentityStatus::decode(&malformed).is_none());
        let mut impossible = encoded;
        impossible[7] = 0;
        assert!(IdentityStatus::decode(&impossible).is_none());
        assert_eq!(IdentityStatusWire::new(encoded).as_bytes(), encoded);
    }
}
