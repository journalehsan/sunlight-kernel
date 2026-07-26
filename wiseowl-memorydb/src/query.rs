//! Typed query core (authoritative). OwlQL compiles into these structures.

use alloc::vec::Vec;

use wiseowl_memory::{MemoryId, SourceId, TrustLevel};

use crate::attributes::BoundedAttributeFilters;
use crate::record::{KindMask, MemoryScope, OwnerId};
use crate::relationship::RelationshipQuery;
use crate::tokens::TokenQuery;

/// Query ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub enum QueryOrder {
    #[default]
    IdAsc,
    ConfidenceDesc,
    ImportanceDesc,
    RecencyDesc,
    TokenRelevanceDesc,
}

/// Opaque versioned cursor (not a raw index pointer).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub struct QueryCursor {
    pub database_generation: u64,
    pub index_generation: u64,
    /// Last seen MemoryId for keyset pagination.
    pub after_id: u64,
    /// Tamper check: FNV of (generation, index_gen, after_id, magic).
    pub checksum: u64,
}

const CURSOR_MAGIC: u64 = 0x4F57_4C43_5552_5300; // "OWLCURS\0"

impl QueryCursor {
    pub fn new(database_generation: u64, index_generation: u64, after_id: u64) -> Self {
        let checksum = crate::codec::fnv1a64(
            &[
                database_generation.to_le_bytes(),
                index_generation.to_le_bytes(),
                after_id.to_le_bytes(),
                CURSOR_MAGIC.to_le_bytes(),
            ]
            .concat(),
        );
        Self {
            database_generation,
            index_generation,
            after_id,
            checksum,
        }
    }

    pub fn validate(&self, database_generation: u64, index_generation: u64) -> Result<(), crate::error::DbError> {
        let expect = Self::new(
            self.database_generation,
            self.index_generation,
            self.after_id,
        )
        .checksum;
        if self.checksum != expect {
            return Err(crate::error::DbError::StaleCursor);
        }
        if self.database_generation != database_generation
            || self.index_generation != index_generation
        {
            return Err(crate::error::DbError::StaleCursor);
        }
        Ok(())
    }

    pub fn encode(&self) -> [u8; 32] {
        let mut out = [0u8; 32];
        out[0..8].copy_from_slice(&self.database_generation.to_le_bytes());
        out[8..16].copy_from_slice(&self.index_generation.to_le_bytes());
        out[16..24].copy_from_slice(&self.after_id.to_le_bytes());
        out[24..32].copy_from_slice(&self.checksum.to_le_bytes());
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, crate::error::DbError> {
        if bytes.len() < 32 {
            return Err(crate::error::DbError::InvalidRequest("cursor length"));
        }
        Ok(Self {
            database_generation: u64::from_le_bytes(bytes[0..8].try_into().unwrap()),
            index_generation: u64::from_le_bytes(bytes[8..16].try_into().unwrap()),
            after_id: u64::from_le_bytes(bytes[16..24].try_into().unwrap()),
            checksum: u64::from_le_bytes(bytes[24..32].try_into().unwrap()),
        })
    }
}

/// Source lookup filter.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub struct SourceQuery {
    pub source_id: Option<SourceId>,
    pub source_content_hash: Option<u64>,
}

/// Trust filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub enum TrustFilter {
    Exact(TrustLevel),
    MinSystemDerived,
}

/// Typed memory query.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub struct MemoryQuery {
    pub kinds: KindMask,
    pub scope: Option<MemoryScope>,
    pub owner: Option<OwnerId>,
    pub token_match: Option<TokenQuery>,
    pub source: Option<SourceQuery>,
    pub relationship: Option<RelationshipQuery>,
    pub attributes: BoundedAttributeFilters,
    pub min_confidence: Option<u16>,
    pub trust: Option<TrustFilter>,
    pub created_after_ns: Option<u64>,
    pub created_before_ns: Option<u64>,
    pub include_superseded: bool,
    pub include_tombstoned_metadata: bool,
    pub order: QueryOrder,
    pub limit: u32,
    pub cursor: Option<QueryCursor>,
}

impl Default for MemoryQuery {
    fn default() -> Self {
        Self {
            kinds: KindMask::all(),
            scope: None,
            owner: None,
            token_match: None,
            source: None,
            relationship: None,
            attributes: BoundedAttributeFilters::default(),
            min_confidence: None,
            trust: None,
            created_after_ns: None,
            created_before_ns: None,
            include_superseded: false,
            include_tombstoned_metadata: false,
            order: QueryOrder::IdAsc,
            limit: 20,
            cursor: None,
        }
    }
}

/// Query result page.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub struct QueryResult {
    pub ids: Vec<MemoryId>,
    pub next_cursor: Option<QueryCursor>,
    /// True when indexes were degraded; results may be incomplete.
    pub degraded: bool,
    pub total_scanned: u32,
}

/// Deduplication policy on insert.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub enum DedupPolicy {
    #[default]
    Allow,
    RejectExactPayload,
    ReturnExistingExactPayload,
    RejectSameSourceRevision,
}
