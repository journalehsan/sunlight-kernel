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

pub mod caps;
pub mod compression;
pub mod entry;
pub mod error;
pub mod ids;
pub mod kinds;
pub mod lifecycle;
pub mod protocol;
pub mod provenance;
pub mod quotas;
pub mod segments;
pub mod service;
pub mod spill;
pub mod stats;

pub use caps::{CapabilitySet, MemoryCapability};
pub use compression::{compress_lz4, decompress_lz4_checked, COMPRESSION_LZ4, COMPRESSION_NONE};
pub use entry::{
    MemoryEntryHeader, MemoryState, TokenStreamRef, ENTRY_HEADER_VERSION, IMPORTANCE_MAX,
    CONFIDENCE_MAX,
};
pub use error::MemoryError;
pub use ids::{
    ClientId, EpisodeId, MemoryId, SegmentId, SessionId, SourceId, TokenStreamId, IdError,
};
pub use kinds::{MemoryClass, MemoryKind, SourceKind, TrustLevel};
pub use lifecycle::{LifecycleOp, TransitionCheck};
pub use protocol::{
    ListFilter, MaintenanceBudget, PromoteRequest, PromoteResult, ProtocolRequest,
    ProtocolResponse, PROTOCOL_VERSION,
};
pub use provenance::{Provenance, MAX_PROVENANCE_PARENTS};
pub use quotas::{QuotaConfig, QuotaSnapshot};
pub use segments::{ColdSegmentHeader, Segment, SegmentState, SEGMENT_FORMAT_VERSION};
pub use service::{CallerIdentity, MemoryService, ServiceConfig};
pub use spill::{SpillStore, SpillRecordMeta};
pub use stats::ServiceStats;
