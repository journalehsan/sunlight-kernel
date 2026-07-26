//! Durable long-term memory record model (Phase 2).
//!
//! Independent of model implementations. Records are normally immutable;
//! corrections produce a new revision rather than mutating historical bytes.

use alloc::string::String;
use alloc::vec::Vec;

use wiseowl_memory::{MemoryId, TrustLevel};

use crate::attributes::AttributeSet;
use crate::codec::{BufReader, BufWriter};
use crate::error::DbError;
use crate::provenance::LongTermProvenance;
use crate::quotas::DbQuotaConfig;
use crate::tokens::{IndexedToken, TokenSetRef};

/// On-disk / wire format version for long-term records.
pub const LT_RECORD_FORMAT_VERSION: u16 = 1;

/// Long-term memory kinds. Deliberately excludes Pattern (Phase 3+).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
#[repr(u8)]
pub enum LongTermMemoryKind {
    ImportedRecord = 1,
    Observation = 2,
    UserProvidedKnowledge = 3,
    ToolVerifiedKnowledge = 4,
    RemoteUnverifiedKnowledge = 5,
    SessionSummary = 6,
    Preference = 7,
    Procedure = 8,
    DiagnosticHistory = 9,
}

impl LongTermMemoryKind {
    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::ImportedRecord),
            2 => Some(Self::Observation),
            3 => Some(Self::UserProvidedKnowledge),
            4 => Some(Self::ToolVerifiedKnowledge),
            5 => Some(Self::RemoteUnverifiedKnowledge),
            6 => Some(Self::SessionSummary),
            7 => Some(Self::Preference),
            8 => Some(Self::Procedure),
            9 => Some(Self::DiagnosticHistory),
            _ => None,
        }
    }

    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ImportedRecord => "imported_record",
            Self::Observation => "observation",
            Self::UserProvidedKnowledge => "user_provided_knowledge",
            Self::ToolVerifiedKnowledge => "tool_verified_knowledge",
            Self::RemoteUnverifiedKnowledge => "remote_unverified_knowledge",
            Self::SessionSummary => "session_summary",
            Self::Preference => "preference",
            Self::Procedure => "procedure",
            Self::DiagnosticHistory => "diagnostic_history",
        }
    }
}

/// Visibility / lifecycle state of a durable record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
#[repr(u8)]
pub enum LongTermRecordState {
    Active = 1,
    Superseded = 2,
    Tombstoned = 3,
    Quarantined = 4,
}

impl LongTermRecordState {
    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::Active),
            2 => Some(Self::Superseded),
            3 => Some(Self::Tombstoned),
            4 => Some(Self::Quarantined),
            _ => None,
        }
    }

    pub const fn as_u8(self) -> u8 {
        self as u8
    }
}

/// Access scope for a long-term record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
#[repr(u8)]
pub enum MemoryScope {
    System = 1,
    User = 2,
    SessionDerived = 3,
    Application = 4,
}

impl MemoryScope {
    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::System),
            2 => Some(Self::User),
            3 => Some(Self::SessionDerived),
            4 => Some(Self::Application),
            _ => None,
        }
    }

    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::SessionDerived => "session_derived",
            Self::Application => "application",
        }
    }
}

/// Owner / namespace identifier (0 = none / system).
pub type OwnerId = u64;

/// Payload reference: inline bytes live next to the record in the segment payload table.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub struct PayloadRef {
    /// FNV-1a of payload bytes (stable).
    pub content_hash: u64,
    pub length: u32,
}

/// Full durable long-term memory record (metadata + optional payload + tokens).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub struct LongTermMemoryRecord {
    pub format_version: u16,
    pub id: MemoryId,
    pub revision: u32,
    pub kind: LongTermMemoryKind,
    pub scope: MemoryScope,
    pub owner: OwnerId,
    pub created_at_ns: u64,
    pub updated_at_ns: u64,
    pub valid_from_ns: Option<u64>,
    pub valid_until_ns: Option<u64>,
    pub importance: u16,
    pub confidence: u16,
    pub trust: TrustLevel,
    pub provenance: LongTermProvenance,
    pub payload_ref: PayloadRef,
    pub tokens: Option<TokenSetRef>,
    pub attributes: AttributeSet,
    pub state: LongTermRecordState,
    /// Supersedes this older record id (if any).
    pub supersedes: Option<MemoryId>,
    /// Payload bytes (not logged; stored in segment payload table).
    pub payload: Vec<u8>,
    /// Caller-supplied indexed tokens (optional).
    pub token_entries: Vec<IndexedToken>,
}

impl LongTermMemoryRecord {
    pub fn validate(&self, quotas: &DbQuotaConfig) -> Result<(), DbError> {
        if self.format_version != LT_RECORD_FORMAT_VERSION {
            return Err(DbError::InvalidValue("record format version"));
        }
        if self.payload.len() as u32 != self.payload_ref.length {
            return Err(DbError::InvalidValue("payload length mismatch"));
        }
        if self.payload.len() as u32 > quotas.max_payload_bytes {
            return Err(DbError::PayloadTooLarge {
                size: self.payload.len() as u32,
                max: quotas.max_payload_bytes,
            });
        }
        if self.importance > 10_000 || self.confidence > 10_000 {
            return Err(DbError::InvalidValue("importance/confidence range"));
        }
        if let Some(ts) = &self.tokens {
            if ts.token_count as usize != self.token_entries.len() {
                return Err(DbError::InvalidValue("token count mismatch"));
            }
            if self.token_entries.len() as u32 > quotas.max_tokens_per_record {
                return Err(DbError::QuotaExceeded("tokens per record"));
            }
        } else if !self.token_entries.is_empty() {
            return Err(DbError::InvalidValue("tokens without TokenSetRef"));
        }
        self.attributes.validate(quotas)?;
        self.provenance.validate(quotas)?;
        for t in &self.token_entries {
            t.validate(quotas)?;
        }
        Ok(())
    }

    /// Encode metadata + payload + tokens into a stable LE blob.
    pub fn encode(&self, max: usize) -> Result<Vec<u8>, DbError> {
        let mut w = BufWriter::with_capacity(max);
        w.write_u16(self.format_version)?;
        w.write_u64(self.id.get())?;
        w.write_u32(self.revision)?;
        w.write_u8(self.kind.as_u8())?;
        w.write_u8(self.scope.as_u8())?;
        w.write_u64(self.owner)?;
        w.write_u64(self.created_at_ns)?;
        w.write_u64(self.updated_at_ns)?;
        BufReader::write_opt_u64(&mut w, self.valid_from_ns)?;
        BufReader::write_opt_u64(&mut w, self.valid_until_ns)?;
        w.write_u16(self.importance)?;
        w.write_u16(self.confidence)?;
        w.write_u8(self.trust.as_u8())?;
        self.provenance.encode(&mut w)?;
        w.write_u64(self.payload_ref.content_hash)?;
        w.write_u32(self.payload_ref.length)?;
        match &self.tokens {
            None => w.write_u8(0)?,
            Some(ts) => {
                w.write_u8(1)?;
                ts.encode(&mut w)?;
            }
        }
        self.attributes.encode(&mut w)?;
        w.write_u8(self.state.as_u8())?;
        match self.supersedes {
            None => w.write_u8(0)?,
            Some(id) => {
                w.write_u8(1)?;
                w.write_u64(id.get())?;
            }
        }
        w.write_bytes_len_u32(&self.payload)?;
        w.write_u32(self.token_entries.len() as u32)?;
        for t in &self.token_entries {
            t.encode(&mut w)?;
        }
        Ok(w.into_vec())
    }

    pub fn decode(data: &[u8], quotas: &DbQuotaConfig) -> Result<Self, DbError> {
        let mut r = BufReader::new(data);
        let format_version = r.read_u16()?;
        if format_version != LT_RECORD_FORMAT_VERSION {
            return Err(DbError::InvalidValue("record format version"));
        }
        let id = MemoryId::from_raw(r.read_u64()?).map_err(|_| DbError::InvalidValue("memory id"))?;
        let revision = r.read_u32()?;
        let kind = LongTermMemoryKind::from_u8(r.read_u8()?)
            .ok_or(DbError::InvalidValue("kind"))?;
        let scope =
            MemoryScope::from_u8(r.read_u8()?).ok_or(DbError::InvalidValue("scope"))?;
        let owner = r.read_u64()?;
        let created_at_ns = r.read_u64()?;
        let updated_at_ns = r.read_u64()?;
        let valid_from_ns = r.read_opt_u64()?;
        let valid_until_ns = r.read_opt_u64()?;
        let importance = r.read_u16()?;
        let confidence = r.read_u16()?;
        let trust =
            TrustLevel::from_u8(r.read_u8()?).ok_or(DbError::InvalidValue("trust"))?;
        let provenance = LongTermProvenance::decode(&mut r, quotas)?;
        let content_hash = r.read_u64()?;
        let length = r.read_u32()?;
        let tokens = match r.read_u8()? {
            0 => None,
            1 => Some(TokenSetRef::decode(&mut r)?),
            _ => return Err(DbError::InvalidValue("tokens tag")),
        };
        let attributes = AttributeSet::decode(&mut r, quotas)?;
        let state = LongTermRecordState::from_u8(r.read_u8()?)
            .ok_or(DbError::InvalidValue("state"))?;
        let supersedes = match r.read_u8()? {
            0 => None,
            1 => Some(
                MemoryId::from_raw(r.read_u64()?)
                    .map_err(|_| DbError::InvalidValue("supersedes id"))?,
            ),
            _ => return Err(DbError::InvalidValue("supersedes tag")),
        };
        let payload = r.read_bytes_len_u32()?.to_vec();
        if payload.len() as u32 != length {
            return Err(DbError::InvalidValue("payload length mismatch"));
        }
        let n_tokens = r.read_u32()? as usize;
        if n_tokens as u32 > quotas.max_tokens_per_record {
            return Err(DbError::QuotaExceeded("tokens per record"));
        }
        let mut token_entries = Vec::with_capacity(n_tokens);
        for _ in 0..n_tokens {
            token_entries.push(IndexedToken::decode(&mut r, quotas)?);
        }
        let rec = Self {
            format_version,
            id,
            revision,
            kind,
            scope,
            owner,
            created_at_ns,
            updated_at_ns,
            valid_from_ns,
            valid_until_ns,
            importance,
            confidence,
            trust,
            provenance,
            payload_ref: PayloadRef {
                content_hash,
                length,
            },
            tokens,
            attributes,
            state,
            supersedes,
            payload,
            token_entries,
        };
        rec.validate(quotas)?;
        Ok(rec)
    }

    /// Metadata-only clone without payload bytes (for list/query results).
    pub fn metadata_only(&self) -> Self {
        let mut c = self.clone();
        c.payload.clear();
        c
    }
}

/// Kind bitmask for queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub struct KindMask(pub u32);

impl KindMask {
    pub fn empty() -> Self {
        Self(0)
    }

    pub fn all() -> Self {
        Self(0xFFFF_FFFF)
    }

    pub fn with(mut self, k: LongTermMemoryKind) -> Self {
        self.0 |= 1u32 << (k.as_u8() as u32);
        self
    }

    pub fn contains(self, k: LongTermMemoryKind) -> bool {
        if self.0 == 0 || self.0 == 0xFFFF_FFFF {
            return true;
        }
        self.0 & (1u32 << (k.as_u8() as u32)) != 0
    }
}

/// Human-readable summary for CLI (no payload content).
pub fn record_summary(r: &LongTermMemoryRecord) -> String {
    alloc::format!(
        "id={} rev={} kind={} scope={} state={} conf={} trust={:?} payload_len={}",
        r.id.get(),
        r.revision,
        r.kind.as_str(),
        r.scope.as_str(),
        match r.state {
            LongTermRecordState::Active => "active",
            LongTermRecordState::Superseded => "superseded",
            LongTermRecordState::Tombstoned => "tombstoned",
            LongTermRecordState::Quarantined => "quarantined",
        },
        r.confidence,
        r.trust,
        r.payload_ref.length
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::fnv1a64;
    use crate::provenance::{DerivationKind, LongTermProvenance};
    use wiseowl_memory::SourceKind;

    fn sample() -> LongTermMemoryRecord {
        let payload = b"hello long-term".to_vec();
        LongTermMemoryRecord {
            format_version: LT_RECORD_FORMAT_VERSION,
            id: MemoryId::from_raw_unchecked(0x0001_0000_0000_0001),
            revision: 1,
            kind: LongTermMemoryKind::Observation,
            scope: MemoryScope::User,
            owner: 42,
            created_at_ns: 100,
            updated_at_ns: 100,
            valid_from_ns: None,
            valid_until_ns: None,
            importance: 500,
            confidence: 800,
            trust: TrustLevel::Untrusted,
            provenance: LongTermProvenance {
                source_kind: SourceKind::UserInput,
                source_id: None,
                producer_service: alloc::string::String::from("test"),
                original_memory_ids: Vec::new(),
                parent_lt_ids: Vec::new(),
                insertion_time_ns: 100,
                trust: TrustLevel::Untrusted,
                source_content_hash: None,
                external_ref: None,
                derivation: DerivationKind::DirectImport,
            },
            payload_ref: PayloadRef {
                content_hash: fnv1a64(&payload),
                length: payload.len() as u32,
            },
            tokens: None,
            attributes: AttributeSet::default(),
            state: LongTermRecordState::Active,
            supersedes: None,
            payload,
            token_entries: Vec::new(),
        }
    }

    #[test]
    fn encode_decode_roundtrip() {
        let q = DbQuotaConfig::default();
        let r = sample();
        let bytes = r.encode(64 * 1024).unwrap();
        let d = LongTermMemoryRecord::decode(&bytes, &q).unwrap();
        assert_eq!(d, r);
    }

    #[test]
    fn rejects_invalid_kind() {
        let q = DbQuotaConfig::default();
        let mut bytes = sample().encode(64 * 1024).unwrap();
        // kind is at fixed offset after version+id+rev: 2+8+4 = 14
        bytes[14] = 99;
        assert!(LongTermMemoryRecord::decode(&bytes, &q).is_err());
    }

    #[test]
    fn kind_mask() {
        let m = KindMask::empty()
            .with(LongTermMemoryKind::Observation)
            .with(LongTermMemoryKind::Preference);
        assert!(m.contains(LongTermMemoryKind::Observation));
        assert!(!m.contains(LongTermMemoryKind::Procedure));
    }
}
