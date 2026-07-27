//! Wise Owl Phase 2: durable long-term cognitive memory database.
//!
//! Service binary: `wiseowl-memorydb`  
//! CLI: `wiseowl-memorydbctl`  
//! Endpoint: `wiseowl.memorydb.v1`
//!
//! This crate is intentionally separate from `wiseowl-memory` / `wiseowl-memoryd`
//! (short-term Working/Hot/Cold). It reuses Phase 0 contracts (IDs, trust,
//! source kinds, LZ4, CRC32) without embedding the short-term service.
//!
//! # Non-goals (Phase 2)
//!
//! Tokenizers, document ingestion, embeddings, vector search, models, online AI,
//! patterns, reflexes, self-healing, and general-purpose SQL.

#![cfg_attr(not(feature = "host"), no_std)]

#[cfg(feature = "host")]
extern crate std;

extern crate alloc;

pub mod attributes;
pub mod caps;
pub mod census;
pub mod codec;
pub mod database;
pub mod error;
pub mod health;
pub mod index;
pub mod insert_wire;
pub mod native_ipc;
pub mod owlql;
pub mod protocol;
pub mod provenance;
pub mod query;
pub mod quotas;
pub mod record;
pub mod relationship;
pub mod segment;
pub mod stats;
pub mod tokens;
pub mod wal;

pub use caps::{DbCapability, DbCapabilitySet};
pub use database::{Database, DbCaller, DurableStore, InsertRequest, MemoryStore};
pub use error::DbError;
pub use health::{DbHealth, HealthState};
pub use protocol::{ENDPOINT_NAME, PROTOCOL_VERSION};
pub use query::{DedupPolicy, MemoryQuery, QueryResult};
pub use quotas::DbQuotaConfig;
pub use record::{
    LongTermMemoryKind, LongTermMemoryRecord, LongTermRecordState, MemoryScope, OwnerId,
};
pub use relationship::{MemoryRelationship, RelationshipKind};
pub use stats::DbStats;

#[cfg(feature = "host")]
pub use database::FsStore;
