//! Index families for long-term memory.
//!
//! **Strategy (hybrid):**
//! - Data records in sealed segments are the source of truth.
//! - Primary / source / token / relationship indexes are rebuildable derived
//!   structures held in RAM and optionally snapshotted.
//! - Index corruption never mutates records; rebuild from segments + WAL.
//! - Queries that require a missing index return [`DbError::IndexDegraded`].

mod primary;
mod relationship;
mod source;
mod token;

pub use primary::{PrimaryEntry, PrimaryIndex, RecordLocation};
pub use relationship::RelationshipIndex;
pub use source::SourceIndex;
pub use token::TokenIndex;

use crate::error::DbError;
use crate::record::LongTermMemoryRecord;
use crate::relationship::MemoryRelationship;

/// Aggregate index set.
#[derive(Debug, Default)]
pub struct IndexSet {
    pub primary: PrimaryIndex,
    pub source: SourceIndex,
    pub token: TokenIndex,
    pub relationship: RelationshipIndex,
    /// True when indexes are known incomplete.
    pub degraded: bool,
    pub degrade_reason: Option<&'static str>,
}

impl IndexSet {
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn mark_degraded(&mut self, reason: &'static str) {
        self.degraded = true;
        self.degrade_reason = Some(reason);
    }

    pub fn require_ready(&self) -> Result<(), DbError> {
        if self.degraded {
            Err(DbError::IndexDegraded(
                self.degrade_reason.unwrap_or("indexes degraded"),
            ))
        } else {
            Ok(())
        }
    }

    /// Apply a committed record into all indexes.
    pub fn apply_record(&mut self, rec: &LongTermMemoryRecord, loc: RecordLocation) {
        self.primary.upsert(rec, loc);
        self.source.index_record(rec);
        self.token.index_record(rec);
    }

    pub fn apply_relationship(&mut self, rel: &MemoryRelationship) {
        self.relationship.insert(rel.clone());
    }

    /// Rebuild all indexes from a set of segment records + relationships.
    pub fn rebuild_from(
        records: &[(RecordLocation, LongTermMemoryRecord)],
        relationships: &[MemoryRelationship],
    ) -> Self {
        let mut set = Self::default();
        for (loc, rec) in records {
            set.apply_record(rec, *loc);
        }
        for rel in relationships {
            if !rel.tombstoned {
                set.apply_relationship(rel);
            }
        }
        set.degraded = false;
        set.degrade_reason = None;
        set
    }
}
