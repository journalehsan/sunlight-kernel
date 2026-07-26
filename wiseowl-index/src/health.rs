//! Indexer health states and degraded reasons.

use alloc::string::String;
use alloc::vec::Vec;

/// Service health state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
#[repr(u8)]
pub enum HealthState {
    Starting = 1,
    Ready = 2,
    Scanning = 3,
    Degraded = 4,
    Stopping = 5,
    Failed = 6,
}

impl HealthState {
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Ready => "ready",
            Self::Scanning => "scanning",
            Self::Degraded => "degraded",
            Self::Stopping => "stopping",
            Self::Failed => "failed",
        }
    }
}

/// Degraded reasons (bounded labels).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
#[repr(u8)]
pub enum DegradedReason {
    MemoryDbUnavailable = 1,
    MemoryDbRecovering = 2,
    MemoryDbProtocolMismatch = 3,
    PendingImportConflict = 4,
    RootUnavailable = 5,
    OperationalStateUnavailable = 6,
    ShmUnavailable = 7,
}

impl DegradedReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MemoryDbUnavailable => "MemoryDbUnavailable",
            Self::MemoryDbRecovering => "MemoryDbRecovering",
            Self::MemoryDbProtocolMismatch => "MemoryDbProtocolMismatch",
            Self::PendingImportConflict => "PendingImportConflict",
            Self::RootUnavailable => "RootUnavailable",
            Self::OperationalStateUnavailable => "OperationalStateUnavailable",
            Self::ShmUnavailable => "ShmUnavailable",
        }
    }
}

/// Health report.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub struct IndexHealth {
    pub ready: bool,
    pub state: HealthState,
    pub reasons: Vec<String>,
    /// MemoryDB endpoint connection state (Ready / Unavailable / Degraded).
    pub memorydb_connection: String,
    pub memorydb_generation: u64,
    pub content_digest_label: String,
    pub manifest_format: u16,
    pub pending_imports: u64,
}

impl Default for IndexHealth {
    fn default() -> Self {
        Self {
            ready: false,
            state: HealthState::Starting,
            reasons: Vec::new(),
            memorydb_connection: String::from("unknown"),
            memorydb_generation: 0,
            content_digest_label: String::from("SHA-256 v1"),
            manifest_format: 2,
            pending_imports: 0,
        }
    }
}

impl IndexHealth {
    pub fn set_degraded(&mut self, reason: DegradedReason) {
        self.state = HealthState::Degraded;
        // Control plane remains usable while degraded for MemoryDB outages.
        self.ready = true;
        let label = String::from(reason.as_str());
        if !self.reasons.iter().any(|r| r == &label) {
            self.reasons.push(label);
        }
    }

    pub fn clear_reason(&mut self, reason: DegradedReason) {
        let label = reason.as_str();
        self.reasons.retain(|r| r != label);
        if self.reasons.is_empty() && self.state == HealthState::Degraded {
            self.state = HealthState::Ready;
            self.ready = true;
        }
    }
}
