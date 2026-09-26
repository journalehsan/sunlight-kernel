//! Bounded, sanitized read-only identity status shared over MemoryDB IPC.

use crate::identity::{IdentityValidationStatus, LoadedIdentity};

pub const IDENTITY_STATUS_VERSION: u16 = 2;
pub const IDENTITY_STATUS_WIRE_LEN: usize = 48;
pub const IDENTITY_STATUS_NATIVE_WORDS: usize = 4;
const MAGIC: &[u8; 4] = b"WIDS";

/// Host serde wrapper uses fixed arrays supported by serde without a length prefix.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub struct IdentityStatusWire {
    pub first: [u8; 32],
    pub last: [u8; 16],
}

impl IdentityStatusWire {
    pub fn new(bytes: [u8; IDENTITY_STATUS_WIRE_LEN]) -> Self {
        let mut first = [0; 32];
        let mut last = [0; 16];
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
    pub activation_state: u8,
    pub activation_fingerprint: [u8; 8],
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
            activation_state: 3,
            // Host-only legacy construction represents a ready in-process
            // authority; native startup replaces this with the LOCAL ID.
            activation_fingerprint: identity.diagnostic_fingerprint(),
        }
    }

    pub fn ready_with_activation(identity: LoadedIdentity, activation_fingerprint: [u8; 8]) -> Self {
        let mut status = Self::ready(identity);
        status.activation_fingerprint = activation_fingerprint;
        status
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
                && self.lineage_sequence <= u32::MAX as u64
                && self.continuity_generation <= u32::MAX as u64
                && self.activation_state == 3
                && self.activation_fingerprint != [0; 8]
        } else {
            !self.persistence_available
                && self.fingerprint == [0; 8]
                && self.lineage_sequence == 0
                && self.continuity_generation == 0
                && self.genesis_kind == 0
                && self.identity_format_version == 0
                && self.validation_status == 0
                && self.activation_state == 5
                && self.activation_fingerprint == [0; 8]
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
        out[36] = self.activation_state;
        out[40..48].copy_from_slice(&self.activation_fingerprint);
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
            activation_state: bytes[36],
            activation_fingerprint: bytes[40..48].try_into().ok()?,
        };
        if bytes[37..40] != [0; 3] { return None; }
        out.validate().then_some(out)
    }

    /// Fixed four-register IPC representation for the sanitized status reply.
    pub fn encode_native_words(self) -> Option<[u64; IDENTITY_STATUS_NATIVE_WORDS]> {
        if !self.validate() || self.state != IdentityStatusState::Ready {
            return None;
        }
        Some([
            self.state as u64
                | ((IDENTITY_STATUS_VERSION as u64) << 8)
                | (u64::from(self.persistence_available) << 24)
                | ((self.genesis_kind as u64) << 25)
                | ((self.identity_format_version as u64) << 33)
                | ((self.validation_status as u64) << 49)
                | ((self.activation_state as u64) << 57),
            u64::from_le_bytes(self.fingerprint),
            u64::from_le_bytes(self.activation_fingerprint),
            ((self.continuity_generation & 0xffff_ffff) << 32) | (self.lineage_sequence & 0xffff_ffff),
        ])
    }

    /// Parse the fixed native reply and enforce the same semantic validation as the host frame.
    pub fn decode_native_words(label: u64, word_count: u32, words: [u64; 8]) -> Option<Self> {
        if label != 0x4D80 || word_count as usize != IDENTITY_STATUS_NATIVE_WORDS {
            return None;
        }
        let header = words[0];
        if header >> 60 != 0 || ((header >> 8) & 0xffff) != IDENTITY_STATUS_VERSION as u64 {
            return None;
        }
        let state = IdentityStatusState::decode((header & 0xff) as u8)?;
        let out = Self {
            state,
            fingerprint: words[1].to_le_bytes(),
            lineage_sequence: words[3] & 0xffff_ffff,
            continuity_generation: words[3] >> 32,
            genesis_kind: ((header >> 25) & 0xff) as u8,
            identity_format_version: ((header >> 33) & 0xffff) as u16,
            validation_status: ((header >> 49) & 0xff) as u8,
            persistence_available: ((header >> 24) & 1) != 0,
            activation_state: ((header >> 57) & 0x7) as u8,
            activation_fingerprint: words[2].to_le_bytes(),
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
            activation_state: 3,
            activation_fingerprint: *b"ACTIV001",
        };
        let encoded = status.encode().unwrap();
        assert_eq!(encoded.len(), IDENTITY_STATUS_WIRE_LEN);
        assert_eq!(IdentityStatus::decode(&encoded), Some(status));
        assert!(IdentityStatus::decode(&encoded[..35]).is_none());
        let mut extra = encoded.to_vec();
        extra.push(0);
        assert!(IdentityStatus::decode(&extra).is_none());
        let mut unknown_version = encoded;
        unknown_version[4] = 3;
        assert!(IdentityStatus::decode(&unknown_version).is_none());
        let mut malformed = encoded;
        malformed[6] = 99;
        assert!(IdentityStatus::decode(&malformed).is_none());
        let mut impossible = encoded;
        impossible[7] = 0;
        assert!(IdentityStatus::decode(&impossible).is_none());
        assert_eq!(IdentityStatusWire::new(encoded).as_bytes(), encoded);

        let native = status.encode_native_words().unwrap();
        let mut native_msg_words = [0; 8];
        native_msg_words[..IDENTITY_STATUS_NATIVE_WORDS].copy_from_slice(&native);
        assert_eq!(
            IdentityStatus::decode_native_words(0x4D80, 4, native_msg_words),
            Some(status)
        );
        let mut malformed_native = native;
        malformed_native[0] &= !(0x7 << 57);
        malformed_native[0] |= 2 << 57;
        native_msg_words[..IDENTITY_STATUS_NATIVE_WORDS].copy_from_slice(&malformed_native);
        assert!(IdentityStatus::decode_native_words(0x4D80, 4, native_msg_words).is_none());

        for bad_header in [
            native[0] ^ (1 << 8),                    // unknown protocol version
            native[0] & !0xff | 99,                  // invalid state enum
            native[0] & !(1 << 24),                  // Ready without persistence
            native[0] & !(0xff << 25) | (3 << 25),   // unknown genesis kind
            native[0] & !(0xffff << 33) | (2 << 33), // unsupported identity format
            native[0] & !(0xff << 49) | (2 << 49),   // unknown validation status
            native[0] & !(0x7 << 57) | (2 << 57),    // non-Active activation
        ] {
            native_msg_words[..IDENTITY_STATUS_NATIVE_WORDS].copy_from_slice(&native);
            native_msg_words[0] = bad_header;
            assert!(IdentityStatus::decode_native_words(0x4D80, 4, native_msg_words).is_none());
        }

        native_msg_words[..IDENTITY_STATUS_NATIVE_WORDS].copy_from_slice(&native);
        native_msg_words[1] = 0;
        assert!(IdentityStatus::decode_native_words(0x4D80, 4, native_msg_words).is_none());
        native_msg_words[..IDENTITY_STATUS_NATIVE_WORDS].copy_from_slice(&native);
        native_msg_words[2] = 0;
        assert!(IdentityStatus::decode_native_words(0x4D80, 4, native_msg_words).is_none());
        native_msg_words[..IDENTITY_STATUS_NATIVE_WORDS].copy_from_slice(&native);
        native_msg_words[3] = 0;
        assert!(IdentityStatus::decode_native_words(0x4D80, 4, native_msg_words).is_none());
        native_msg_words[..IDENTITY_STATUS_NATIVE_WORDS].copy_from_slice(&native);
        native_msg_words[4] = 1;
        assert!(IdentityStatus::decode_native_words(0x4D80, 5, native_msg_words).is_none());
        assert!(IdentityStatus::decode_native_words(0x4D80, 3, native_msg_words).is_none());
        assert!(IdentityStatus::decode_native_words(0x4D81, 4, native_msg_words).is_none());
    }

    #[test]
    fn boundary_values_round_trip_and_impossible_values_fail_closed() {
        let mut boundary = IdentityStatus {
            state: IdentityStatusState::Ready,
            fingerprint: *b"49DA20E3",
            lineage_sequence: u32::MAX as u64,
            continuity_generation: u32::MAX as u64,
            genesis_kind: 2,
            identity_format_version: 1,
            validation_status: 1,
            persistence_available: true,
            activation_state: 3,
            activation_fingerprint: *b"ACTIV001",
        };
        let encoded = boundary.encode().unwrap();
        assert_eq!(IdentityStatus::decode(&encoded), Some(boundary));
        assert_eq!(
            IdentityStatus::decode_native_words(0x4D80, 4, {
                let mut words = [0; 8];
                words[..IDENTITY_STATUS_NATIVE_WORDS]
                    .copy_from_slice(&boundary.encode_native_words().unwrap());
                words
            }),
            Some(boundary)
        );
        boundary.lineage_sequence = 0;
        assert!(boundary.encode().is_none());
        boundary.lineage_sequence = 1;
        boundary.continuity_generation = 0;
        assert!(boundary.encode().is_none());
    }

    #[test]
    fn native_status_matches_the_guest_identity_status_sample() {
        let expected = IdentityStatus {
            state: IdentityStatusState::Ready,
            fingerprint: *b"A77568FD",
            lineage_sequence: 1,
            continuity_generation: 1,
            genesis_kind: 1,
            identity_format_version: 1,
            validation_status: 1,
            persistence_available: true,
            activation_state: 3,
            activation_fingerprint: *b"ACTIV001",
        };
        let mut words = [0; 8];
        words[..4].copy_from_slice(&expected.encode_native_words().unwrap());
        let status = IdentityStatus::decode_native_words(0x4D80, 4, words).unwrap();
        assert_eq!(status, expected);
        assert_eq!(status.continuity_generation, 1);
    }

    #[test]
    fn personalized_ready_status_requires_active_activation() {
        let mut status = IdentityStatus {
            state: IdentityStatusState::Ready,
            fingerprint: *b"IDENTITY",
            lineage_sequence: 1,
            continuity_generation: 1,
            genesis_kind: 1,
            identity_format_version: 1,
            validation_status: 1,
            persistence_available: true,
            activation_state: 2,
            activation_fingerprint: *b"ACTIV001",
        };
        assert!(!status.validate());
        assert!(status.encode().is_none());
        status.activation_state = 3;
        assert!(status.validate());
    }
}
