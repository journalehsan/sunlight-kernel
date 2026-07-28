//! Wise Owl cognitive memory foundation (Phase 0 contracts + Phase 1 short-term service).
//!
//! This crate deliberately excludes language models, embeddings, long-term
//! databases, pattern recognition, online providers, and self-healing.
//!
//! # Service name
//!
//! The short-term memory daemon is **`wiseowl-memoryd`** (not `sunlight-memoryd`)
//! to avoid confusion with kernel physical/virtual memory management.

#![cfg_attr(not(feature = "host"), no_std)]

#[cfg(feature = "host")]
extern crate std;

extern crate alloc;

pub mod caller;
pub mod caps;
pub mod compression;
pub mod entry;
pub mod error;
pub mod health;
pub mod ids;
pub mod kinds;
pub mod lifecycle;
pub mod native_ipc;
pub mod protocol;
pub mod provenance;
pub mod quotas;
pub mod segments;
#[cfg(feature = "host")]
pub mod service;
#[cfg(feature = "host")]
pub mod spill;
pub mod stats;
pub mod sunlightos_engine;

pub use caller::CallerIdentity;
pub use caps::{CapabilitySet, MemoryCapability};
pub use compression::{compress_lz4, decompress_lz4_checked, COMPRESSION_LZ4, COMPRESSION_NONE};
pub use entry::{
    MemoryEntryHeader, MemoryState, TokenStreamRef, CONFIDENCE_MAX, ENTRY_HEADER_VERSION,
    IMPORTANCE_MAX,
};
pub use error::MemoryError;
pub use health::{degraded, ServiceHealth};
pub use ids::{pack_id, unpack_id, IdAllocator, COUNTER_BITS, GENERATION_BITS};
pub use ids::{
    ClientId, EpisodeId, IdError, MemoryId, SegmentId, SessionId, SourceId, TokenStreamId,
};
pub use kinds::{MemoryClass, MemoryKind, SourceKind, TrustLevel};
pub use lifecycle::{LifecycleOp, TransitionCheck};
pub use protocol::{
    ListFilter, MaintenanceBudget, PromoteRequest, PromoteResult, ProtocolRequest,
    ProtocolResponse, PROTOCOL_VERSION,
};
pub use provenance::{Provenance, MAX_PROVENANCE_PARENTS};
pub use quotas::{QuotaConfig, QuotaSnapshot};
pub use segments::{encode_record_v2, parse_records_v2, RecoveredRecord, RECORD_FORMAT_VERSION};
pub use segments::{ColdSegmentHeader, Segment, SegmentState, SEGMENT_FORMAT_VERSION};
#[cfg(feature = "host")]
pub use service::{
    InMemoryKv, KvBackend, KvPutOutcome, MemoryService, ServiceConfig, PROMOTION_RECORD_VERSION,
};
#[cfg(feature = "host")]
pub use spill::{
    QuarantineConfig, SpillRecordMeta, SpillStore, MAX_QUARANTINE_BYTES, MAX_QUARANTINE_FILES,
};
pub use stats::ServiceStats;
pub use sunlightos_engine::{NativeKvBackend, NativeKvPut, NativeMemoryEngine, RamKv};
