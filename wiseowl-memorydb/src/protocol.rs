//! Host IPC protocol (UDS + bincode) and shared request types.
//!
//! Native SunlightOS uses a separate LE envelope in `native_ipc`; this module
//! is the host development path and typed operation surface.

use alloc::string::String;
use alloc::vec::Vec;

use wiseowl_memory::{MemoryId, SourceId};

use crate::database::InsertRequest;
use crate::query::{MemoryQuery, QueryResult};
use crate::record::LongTermMemoryRecord;
use crate::relationship::MemoryRelationship;
use crate::stats::DbStats;

/// Host bincode protocol version.
pub const PROTOCOL_VERSION: u16 = 1;

/// Endpoint name for nameserver registration.
pub const ENDPOINT_NAME: &str = "wiseowl.memorydb.v1";

#[derive(Debug, Clone)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub enum DbRequest {
    BeginTransaction,
    InsertRecord { tx_id: u64, req: InsertRequestWire },
    InsertRelationship { tx_id: u64, rel: MemoryRelationship },
    Tombstone { tx_id: u64, id: MemoryId },
    Commit { tx_id: u64 },
    Abort { tx_id: u64 },
    Get { id: MemoryId, payload: bool },
    History { id: MemoryId },
    Source { source_id: SourceId, offset: u32, limit: u32 },
    Relationships { id: MemoryId },
    Query { query: MemoryQuery },
    OwlQl { text: String },
    DeleteSource { source_id: SourceId, batch: u32 },
    DeleteSourceDryRun { source_id: SourceId },
    Checkpoint,
    Compact,
    RebuildIndexes,
    Stats,
    Health,
    Verify { max_segments: u32 },
}

/// Wire form of insert request (payload as bytes).
#[derive(Debug, Clone)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub struct InsertRequestWire {
    pub kind: u8,
    pub scope: u8,
    pub owner: u64,
    pub payload: Vec<u8>,
    pub provenance: crate::provenance::LongTermProvenance,
    pub confidence: u16,
    pub importance: u16,
    pub trust: u8,
    pub valid_from_ns: Option<u64>,
    pub valid_until_ns: Option<u64>,
    pub tokens: Option<(crate::tokens::TokenSetRef, Vec<crate::tokens::IndexedToken>)>,
    pub attributes: crate::attributes::AttributeSet,
    pub supersedes: Option<MemoryId>,
    pub relationships: Vec<MemoryRelationship>,
    pub dedup: crate::query::DedupPolicy,
    pub id: Option<MemoryId>,
    pub revision: u32,
}

impl InsertRequestWire {
    pub fn into_request(self) -> Result<InsertRequest, crate::error::DbError> {
        use crate::record::{LongTermMemoryKind, MemoryScope};
        use wiseowl_memory::TrustLevel;
        Ok(InsertRequest {
            kind: LongTermMemoryKind::from_u8(self.kind)
                .ok_or(crate::error::DbError::InvalidValue("kind"))?,
            scope: MemoryScope::from_u8(self.scope)
                .ok_or(crate::error::DbError::InvalidValue("scope"))?,
            owner: self.owner,
            payload: self.payload,
            provenance: self.provenance,
            confidence: self.confidence,
            importance: self.importance,
            trust: TrustLevel::from_u8(self.trust)
                .ok_or(crate::error::DbError::InvalidValue("trust"))?,
            valid_from_ns: self.valid_from_ns,
            valid_until_ns: self.valid_until_ns,
            tokens: self.tokens,
            attributes: self.attributes,
            supersedes: self.supersedes,
            relationships: self.relationships,
            dedup: self.dedup,
            id: self.id,
            revision: self.revision,
        })
    }
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub enum DbResponse {
    Ok,
    TxId(u64),
    Sequence(u64),
    MemoryId(MemoryId),
    Record(LongTermMemoryRecord),
    Revisions(Vec<u32>),
    SourcePage { ids: Vec<MemoryId>, more: bool },
    Relationships(Vec<MemoryRelationship>),
    Query(QueryResult),
    SourceDelete { deleted: u32, more: bool },
    SourceCount(u32),
    Stats(DbStats),
    Health {
        ready: bool,
        state: String,
        reasons: Vec<String>,
    },
    Verify { ok: u32, bad: u32 },
    Compacted { reclaimed: u64 },
    Error { code: String, message: String },
}

impl DbResponse {
    pub fn from_error(e: crate::error::DbError) -> Self {
        Self::Error {
            code: alloc::format!("{e:?}"),
            message: alloc::format!("{e}"),
        }
    }
}
