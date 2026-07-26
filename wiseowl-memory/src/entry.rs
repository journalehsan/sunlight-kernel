//! Memory entry headers and lifecycle state.

use crate::error::MemoryError;
use crate::ids::{MemoryId, SessionId, TokenStreamId};
use crate::kinds::{MemoryClass, MemoryKind};
use crate::provenance::Provenance;

/// Protocol / record version for [`MemoryEntryHeader`].
pub const ENTRY_HEADER_VERSION: u16 = 1;

/// Inclusive maximum for importance scores.
pub const IMPORTANCE_MAX: u16 = 10_000;
/// Inclusive maximum for confidence scores.
pub const CONFIDENCE_MAX: u16 = 10_000;

/// Opaque versioned token-stream reference (no tokenizer in Phase 0/1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub struct TokenStreamRef {
    pub id: TokenStreamId,
    pub tokenizer_id: u32,
    pub tokenizer_version: u32,
    pub token_count: u32,
}

/// Stable entry header shared across IPC and spill metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub struct MemoryEntryHeader {
    pub version: u16,
    pub id: MemoryId,
    pub session_id: SessionId,
    pub class: MemoryClass,
    pub kind: MemoryKind,
    pub created_at_ns: u64,
    pub last_access_ns: u64,
    /// Absolute monotonic expiry; `None` means no TTL (still subject to quotas).
    pub expires_at_ns: Option<u64>,
    /// Fixed range 0..=IMPORTANCE_MAX.
    pub importance: u16,
    /// Fixed range 0..=CONFIDENCE_MAX.
    pub confidence: u16,
    pub payload_len: u32,
    pub token_stream: Option<TokenStreamRef>,
    pub provenance: Provenance,
}

impl MemoryEntryHeader {
    pub fn validate_scores(importance: u16, confidence: u16) -> Result<(), MemoryError> {
        if importance > IMPORTANCE_MAX {
            return Err(MemoryError::InvalidRequest("importance out of range"));
        }
        if confidence > CONFIDENCE_MAX {
            return Err(MemoryError::InvalidRequest("confidence out of range"));
        }
        Ok(())
    }

    pub fn validate_payload_len(len: u32, max: u32) -> Result<(), MemoryError> {
        if len > max {
            return Err(MemoryError::PayloadTooLarge { size: len, max });
        }
        Ok(())
    }
}

/// Runtime lifecycle state of a memory entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
#[repr(u8)]
pub enum MemoryState {
    /// Open working entry; mutable.
    Open = 1,
    /// Sealed immutable; may still be hot (RAM) or cold.
    Sealed = 2,
    /// Sealed and compressed into a cold segment.
    Cold = 3,
    /// Explicitly deleted (not returned by reads/lists).
    Deleted = 4,
    /// Expired by TTL.
    Expired = 5,
    /// Successfully promoted to sunlight-kv (local copy may still exist).
    Promoted = 6,
}

impl MemoryState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Sealed => "sealed",
            Self::Cold => "cold",
            Self::Deleted => "deleted",
            Self::Expired => "expired",
            Self::Promoted => "promoted",
        }
    }

    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::Open),
            2 => Some(Self::Sealed),
            3 => Some(Self::Cold),
            4 => Some(Self::Deleted),
            5 => Some(Self::Expired),
            6 => Some(Self::Promoted),
            _ => None,
        }
    }

    /// True when the entry is still addressable (not deleted/expired).
    pub const fn is_live(self) -> bool {
        !matches!(self, Self::Deleted | Self::Expired)
    }
}

/// Full in-memory entry (header + payload).
#[derive(Debug, Clone)]
pub struct MemoryEntry {
    pub header: MemoryEntryHeader,
    pub state: MemoryState,
    pub payload: Vec<u8>,
    /// Number of active reader pins (protects from eviction).
    pub pin_count: u32,
    /// Cold segment containing this entry, if spilled.
    pub segment_id: Option<crate::ids::SegmentId>,
    /// KV promotion key if promoted (for idempotency).
    pub kv_key: Option<String>,
    /// Whether promotion already succeeded.
    pub promoted: bool,
    /// Client that created the entry (for disconnect cleanup).
    pub owner_client: Option<crate::ids::ClientId>,
}

impl MemoryEntry {
    pub fn is_live(&self) -> bool {
        !matches!(self.state, MemoryState::Deleted | MemoryState::Expired)
    }

    pub fn is_mutable(&self) -> bool {
        self.state == MemoryState::Open
    }

    pub fn is_pinned(&self) -> bool {
        self.pin_count > 0
    }

    pub fn touch(&mut self, now_ns: u64) -> Result<(), MemoryError> {
        if !self.is_live() {
            return Err(if self.state == MemoryState::Expired {
                MemoryError::EntryExpired
            } else {
                MemoryError::EntryDeleted
            });
        }
        if let Some(exp) = self.header.expires_at_ns {
            if now_ns >= exp {
                self.state = MemoryState::Expired;
                return Err(MemoryError::EntryExpired);
            }
        }
        self.header.last_access_ns = now_ns;
        Ok(())
    }

    pub fn check_not_expired(&mut self, now_ns: u64) -> Result<(), MemoryError> {
        if self.state == MemoryState::Deleted {
            return Err(MemoryError::EntryDeleted);
        }
        if self.state == MemoryState::Expired {
            return Err(MemoryError::EntryExpired);
        }
        if let Some(exp) = self.header.expires_at_ns {
            if now_ns >= exp {
                self.state = MemoryState::Expired;
                return Err(MemoryError::EntryExpired);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{MemoryId, SessionId};
    use crate::kinds::{SourceKind, TrustLevel};
    use crate::provenance::Provenance;

    fn sample_header() -> MemoryEntryHeader {
        MemoryEntryHeader {
            version: ENTRY_HEADER_VERSION,
            id: MemoryId::from_raw(1).unwrap(),
            session_id: SessionId::from_raw(1).unwrap(),
            class: MemoryClass::Working,
            kind: MemoryKind::Input,
            created_at_ns: 100,
            last_access_ns: 100,
            expires_at_ns: Some(1000),
            importance: 100,
            confidence: 100,
            payload_len: 0,
            token_stream: None,
            provenance: Provenance::new(
                SourceKind::UserInput,
                None,
                100,
                "test",
                TrustLevel::Untrusted,
            ),
        }
    }

    #[test]
    fn importance_bounds() {
        assert!(MemoryEntryHeader::validate_scores(0, 0).is_ok());
        assert!(MemoryEntryHeader::validate_scores(IMPORTANCE_MAX, CONFIDENCE_MAX).is_ok());
        assert!(MemoryEntryHeader::validate_scores(IMPORTANCE_MAX + 1, 0).is_err());
        assert!(MemoryEntryHeader::validate_scores(0, CONFIDENCE_MAX + 1).is_err());
    }

    #[test]
    fn expired_touch_fails() {
        let mut e = MemoryEntry {
            header: sample_header(),
            state: MemoryState::Open,
            payload: Vec::new(),
            pin_count: 0,
            segment_id: None,
            kv_key: None,
            promoted: false,
            owner_client: None,
        };
        assert!(e.touch(1000).is_err());
        assert_eq!(e.state, MemoryState::Expired);
    }
}
