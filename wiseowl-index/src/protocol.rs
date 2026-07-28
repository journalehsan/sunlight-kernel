//! Host IPC protocol types for wiseowl-index.

use alloc::string::String;
use alloc::vec::Vec;

use crate::config::IndexRootConfig;
use crate::health::IndexHealth;
use crate::source::SourceManifest;
use crate::stats::IndexStats;

/// Host bincode protocol version.
pub const PROTOCOL_VERSION: u16 = 1;

/// Endpoint name for nameserver registration.
pub const ENDPOINT_NAME: &str = "wiseowl.index.v1";

#[derive(Debug, Clone)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub enum IndexRequest {
    RegisterRoot {
        path: String,
        owner: u64,
        recursive: bool,
        maximum_depth: u16,
    },
    RemoveRoot {
        root_id: u64,
    },
    ListRoots,
    StartScan {
        root_id: Option<u64>,
    },
    GetScanStatus,
    ListSources {
        offset: u32,
        limit: u32,
    },
    InspectSource {
        source_id: u64,
    },
    RetrySource {
        source_id: u64,
    },
    ReindexSource {
        source_id: u64,
    },
    ForgetSource {
        source_id: u64,
        dry_run: bool,
    },
    TokenizeText {
        text: String,
    },
    SearchText {
        text: String,
        limit: u32,
    },
    GetStats,
    GetHealth,
    /// Phase 3.5 diagnostics
    GetTransport,
    GetMemoryDb,
    GetPending,
    Reconcile,
    GetDigest {
        source_id: u64,
    },
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub enum IndexResponse {
    Ok,
    RootId(u64),
    Roots(Vec<IndexRootConfig>),
    ScanStarted,
    ScanStatus {
        scanning: bool,
        last_scan_ns: u64,
    },
    Sources {
        items: Vec<SourceListItem>,
        more: bool,
    },
    Source(SourceManifest),
    Forget {
        deleted: u32,
        more: bool,
    },
    Tokens {
        tokenizer_id: u32,
        tokenizer_version: u32,
        tokens: Vec<TokenWire>,
    },
    Search {
        label: String,
        hits: Vec<SearchHit>,
    },
    Stats(IndexStats),
    Health(IndexHealth),
    Transport(TransportInfo),
    MemoryDb {
        ready: bool,
        state: String,
        generation: u64,
    },
    Pending {
        count: u64,
    },
    Reconciled {
        count: u32,
    },
    Digest {
        algorithm: String,
        version: u16,
        hex_abbrev: String,
        source_revision: u32,
        manifest_version: u16,
    },
    Error {
        code: String,
        message: String,
    },
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub struct TransportInfo {
    pub indexer_endpoint: String,
    pub memorydb_endpoint: String,
    pub memorydb_generation: u64,
    pub connection: String,
    pub ipc_protocol: String,
    pub shm: String,
    pub content_digest: String,
    pub manifest_format: u16,
    pub pending_imports: u64,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub struct SourceListItem {
    pub source_id: u64,
    pub root_id: u64,
    pub relative_path: String,
    pub state: String,
    pub content_digest_hex: String,
    pub fast_fingerprint: u64,
    pub chunk_count: u32,
    pub manifest_version: u16,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub struct TokenWire {
    pub token_id: u64,
    pub canonical: String,
    pub frequency: u16,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub struct SearchHit {
    pub memory_id: u64,
    pub source_id: Option<u64>,
    pub lexical_score: u32,
    pub preview: String,
}

impl IndexResponse {
    pub fn from_error(e: crate::error::IndexError) -> Self {
        Self::Error {
            code: alloc::format!("{e:?}"),
            message: alloc::format!("{e}"),
        }
    }
}
