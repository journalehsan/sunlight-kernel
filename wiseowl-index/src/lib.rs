//! Wise Owl Phase 3.5: secure incremental document ingestion with strong content
//! digests and independent MemoryDB service integration.
//!
//! Service binary: `wiseowl-indexd`
//! CLI: `wiseowl-indexctl`
//! Endpoint: `wiseowl.index.v1`
//! Crate: `wiseowl-index`
//!
//! This crate is intentionally separate from:
//! - `wiseowl-memory` / `wiseowl-memoryd` (short-term Working/Hot/Cold)
//! - `wiseowl-memorydb` (durable long-term storage — **consumer of** its APIs only)
//! - `sunlight-kv` (optional operational state only)
//!
//! # Phase 3.5 architecture (native production)
//!
//! ```text
//! wiseowl-indexctl → wiseowl-indexd → wiseowl.memorydb.v1 (IPC + SHM)
//! ```
//!
//! Native production must **not** embed MemoryDB in-process.
//! Host tests may use [`HostMemoryDbBackend`].
//!
//! # Explicit non-goals
//!
//! Model training, embeddings, vector search, OCR, PDF/DOCX, web crawling,
//! pattern recognition, reflexes, online AI, answer generation, self-healing.

#![cfg_attr(not(feature = "host"), no_std)]

#[cfg(feature = "host")]
extern crate std;

extern crate alloc;

pub mod caps;
pub mod chunk;
pub mod config;
pub mod digest;
pub mod discover;
pub mod error;
pub mod hash;
pub mod health;
pub mod ignore;
pub mod import_key;
pub mod ingest;
pub mod memorydb_backend;
pub mod native_ipc;
pub mod parse;
pub mod path_security;
pub mod protocol;
pub mod quotas;
pub mod scan;
pub mod service;
pub mod source;
pub mod stable_file;
pub mod state;
pub mod stats;
pub mod text_validate;
pub mod tokenize;

#[cfg(feature = "sunlightos")]
pub mod memorydb_client;

pub use caps::{IndexCapability, IndexCapabilitySet};
pub use config::{IndexRootConfig, IndexerConfig};
pub use digest::{ContentDigest, ContentDigestAlgorithm, ContentDigestHasher};
pub use error::IndexError;
pub use health::{DegradedReason, HealthState, IndexHealth};
pub use memorydb_backend::{HostMemoryDbBackend, IndexMemoryDb, MemoryDbHealth};
pub use protocol::{ENDPOINT_NAME, PROTOCOL_VERSION};
pub use quotas::IndexQuotaConfig;
pub use service::{IndexCaller, IndexerService};
pub use stats::IndexStats;
pub use tokenize::{RetrievalTokenizer, WiseOwlLexicalV1};
