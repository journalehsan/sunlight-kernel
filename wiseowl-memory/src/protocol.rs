//! Versioned IPC protocol types for wiseowl-memoryd.
//!
//! Host transport uses length-prefixed bincode (same framing as sunlight-kv).
//! On SunlightOS, labels map to these request variants (see documentation).

use crate::caps::CapabilitySet;
use crate::entry::{MemoryEntryHeader, MemoryState, TokenStreamRef};
use crate::error::MemoryError;
use crate::ids::{ClientId, MemoryId, SessionId};
use crate::kinds::{MemoryClass, MemoryKind};
use crate::provenance::Provenance;
use crate::stats::ServiceStats;

/// Wire protocol version.
pub const PROTOCOL_VERSION: u16 = 1;

/// Explicit promotion request.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub struct PromoteRequest {
    pub memory_id: MemoryId,
    /// Destination namespace prefix, e.g. `owl.v1.shortterm`.
    pub namespace: String,
    pub expected_record_version: u16,
    /// Optional retention hint stored as metadata (not a local memory class).
    pub retention_hint: String,
    pub reason: String,
    /// If true, delete local copy only after confirmed KV write.
    pub delete_local_after: bool,
}

/// Result of a promotion attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub enum PromoteResult {
    /// Newly written to KV.
    Written { key: String },
    /// Already present (idempotent retry).
    AlreadyPresent { key: String },
}

/// Filter for list operations (always hard-capped by quota).
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub struct ListFilter {
    pub session_id: Option<SessionId>,
    pub class: Option<MemoryClass>,
    pub kind: Option<MemoryKind>,
    pub max_results: Option<u32>,
}

/// Bounded maintenance work budget.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub struct MaintenanceBudget {
    pub max_entries_scanned: u32,
    pub max_segments_compressed: u32,
    pub max_bytes_reclaimed: u64,
    pub max_elapsed_ns: u64,
}

impl Default for MaintenanceBudget {
    fn default() -> Self {
        Self {
            max_entries_scanned: 64,
            max_segments_compressed: 4,
            max_bytes_reclaimed: 256 * 1024,
            max_elapsed_ns: 5_000_000, // 5 ms budget
        }
    }
}

/// Client -> service requests.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub enum ProtocolRequest {
    CreateEntry {
        protocol_version: u16,
        session_id: SessionId,
        class: MemoryClass,
        kind: MemoryKind,
        importance: u16,
        confidence: u16,
        ttl_ns: Option<u64>,
        payload: Vec<u8>,
        token_stream: Option<TokenStreamRef>,
        provenance: Provenance,
    },
    AppendEntry {
        protocol_version: u16,
        memory_id: MemoryId,
        data: Vec<u8>,
    },
    ReadEntry {
        protocol_version: u16,
        memory_id: MemoryId,
        /// When true, include payload (requires ReadPayload capability).
        include_payload: bool,
    },
    TouchEntry {
        protocol_version: u16,
        memory_id: MemoryId,
    },
    SealEntry {
        protocol_version: u16,
        memory_id: MemoryId,
        /// After seal, optionally move class Working -> Hot or Hot -> Cold path.
        promote_class_to_hot: bool,
    },
    DeleteEntry {
        protocol_version: u16,
        memory_id: MemoryId,
    },
    PromoteEntry {
        protocol_version: u16,
        request: PromoteRequest,
    },
    ListEntries {
        protocol_version: u16,
        filter: ListFilter,
    },
    GetStats {
        protocol_version: u16,
    },
    RunMaintenance {
        protocol_version: u16,
        budget: MaintenanceBudget,
    },
    /// Register / heartbeat a client connection.
    RegisterClient {
        protocol_version: u16,
        name: String,
    },
    /// Drop all working entries owned by a disconnected client.
    ClientDisconnect {
        protocol_version: u16,
        client_id: ClientId,
    },
    /// Create a new session owned by the caller.
    CreateSession {
        protocol_version: u16,
    },
    ListSessions {
        protocol_version: u16,
    },
}

/// Service -> client responses.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub enum ProtocolResponse {
    Ok,
    Created {
        memory_id: MemoryId,
        session_id: SessionId,
    },
    Entry {
        header: MemoryEntryHeader,
        state: MemoryState,
        payload: Option<Vec<u8>>,
        promoted: bool,
        segment_id: Option<u64>,
    },
    Listed {
        headers: Vec<(MemoryEntryHeader, MemoryState)>,
    },
    Stats(ServiceStats),
    Promoted(PromoteResult),
    Maintenance {
        entries_scanned: u32,
        segments_compressed: u32,
        bytes_reclaimed: u64,
        expired: u32,
        evicted: u32,
    },
    ClientRegistered {
        client_id: ClientId,
    },
    SessionCreated {
        session_id: SessionId,
    },
    Sessions {
        ids: Vec<SessionId>,
    },
    Error(MemoryError),
}

/// Validate protocol version on every request.
pub fn check_protocol_version(got: u16) -> Result<(), MemoryError> {
    if got != PROTOCOL_VERSION {
        Err(MemoryError::UnsupportedProtocolVersion {
            got,
            want: PROTOCOL_VERSION,
        })
    } else {
        Ok(())
    }
}

/// Extract protocol version from a request.
pub fn request_version(req: &ProtocolRequest) -> u16 {
    match req {
        ProtocolRequest::CreateEntry {
            protocol_version, ..
        }
        | ProtocolRequest::AppendEntry {
            protocol_version, ..
        }
        | ProtocolRequest::ReadEntry {
            protocol_version, ..
        }
        | ProtocolRequest::TouchEntry {
            protocol_version, ..
        }
        | ProtocolRequest::SealEntry {
            protocol_version, ..
        }
        | ProtocolRequest::DeleteEntry {
            protocol_version, ..
        }
        | ProtocolRequest::PromoteEntry {
            protocol_version, ..
        }
        | ProtocolRequest::ListEntries {
            protocol_version, ..
        }
        | ProtocolRequest::GetStats {
            protocol_version, ..
        }
        | ProtocolRequest::RunMaintenance {
            protocol_version, ..
        }
        | ProtocolRequest::RegisterClient {
            protocol_version, ..
        }
        | ProtocolRequest::ClientDisconnect {
            protocol_version, ..
        }
        | ProtocolRequest::CreateSession {
            protocol_version, ..
        }
        | ProtocolRequest::ListSessions {
            protocol_version, ..
        } => *protocol_version,
    }
}

/// Caller context attached to each request by the transport.
#[derive(Debug, Clone)]
pub struct RequestContext {
    pub client_id: Option<ClientId>,
    pub caps: CapabilitySet,
    /// Session ownership map is maintained by the service; this is the
    /// set of sessions the caller may treat as "own".
    pub owned_sessions: Vec<SessionId>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_bad_version() {
        assert!(check_protocol_version(PROTOCOL_VERSION).is_ok());
        assert!(matches!(
            check_protocol_version(99),
            Err(MemoryError::UnsupportedProtocolVersion { got: 99, .. })
        ));
    }
}
