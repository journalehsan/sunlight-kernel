//! Indexer health states and degraded reasons.
//!
//! MemoryDB readiness is owned by a single authoritative
//! [`MemoryDbConnectionState`]. Independent booleans must not contradict it.

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

/// Authoritative MemoryDB connection-health state machine.
///
/// Direct successful health responses update this state. Endpoint generation
/// changes invalidate stale readiness. Failed mutations do not permanently
/// override a newer successful health observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub enum MemoryDbConnectionState {
    Unknown,
    Discovering,
    Connecting,
    Ready {
        endpoint_generation: u64,
        database_generation: u64,
        last_success_at: u64,
    },
    Degraded {
        reason: MemoryDbDegradedReason,
        last_success_at: Option<u64>,
        retry_after: Option<u64>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
#[repr(u8)]
pub enum MemoryDbDegradedReason {
    Unavailable = 1,
    Recovering = 2,
    ProtocolMismatch = 3,
}

impl MemoryDbDegradedReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unavailable => "Unavailable",
            Self::Recovering => "Recovering",
            Self::ProtocolMismatch => "ProtocolMismatch",
        }
    }
}

impl MemoryDbConnectionState {
    pub const fn as_label(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Discovering => "discovering",
            Self::Connecting => "connecting",
            Self::Ready { .. } => "Ready",
            Self::Degraded { reason, .. } => match reason {
                MemoryDbDegradedReason::Unavailable => "Unavailable",
                MemoryDbDegradedReason::Recovering => "Recovering",
                MemoryDbDegradedReason::ProtocolMismatch => "ProtocolMismatch",
            },
        }
    }

    pub const fn is_ready(self) -> bool {
        matches!(self, Self::Ready { .. })
    }

    pub const fn memorydb_ready_flag(self) -> u64 {
        if self.is_ready() {
            1
        } else {
            0
        }
    }

    pub const fn database_generation(self) -> u64 {
        match self {
            Self::Ready {
                database_generation,
                ..
            } => database_generation,
            _ => 0,
        }
    }

    pub const fn endpoint_generation(self) -> u64 {
        match self {
            Self::Ready {
                endpoint_generation,
                ..
            } => endpoint_generation,
            _ => 0,
        }
    }

    /// Apply a successful direct MemoryDB health observation.
    pub fn observe_success(
        &mut self,
        endpoint_generation: u64,
        database_generation: u64,
        now_ns: u64,
    ) {
        // Endpoint generation change always installs the newer ready snapshot.
        *self = Self::Ready {
            endpoint_generation,
            database_generation,
            last_success_at: now_ns,
        };
    }

    /// Apply a failed health or transport observation.
    pub fn observe_failure(
        &mut self,
        reason: MemoryDbDegradedReason,
        now_ns: u64,
        retry_after: Option<u64>,
    ) {
        let last_success_at = match *self {
            Self::Ready {
                last_success_at, ..
            } => Some(last_success_at),
            Self::Degraded {
                last_success_at, ..
            } => last_success_at,
            _ => None,
        };
        // A newer Ready observation is never permanently locked out by an older
        // failure once observe_success is called again; this only records current
        // unavailability.
        let _ = now_ns;
        *self = Self::Degraded {
            reason,
            last_success_at,
            retry_after,
        };
    }
}

/// Health report.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub struct IndexHealth {
    pub ready: bool,
    pub state: HealthState,
    pub reasons: Vec<String>,
    /// Authoritative MemoryDB connection state.
    pub memorydb: MemoryDbConnectionState,
    pub content_digest_label: String,
    pub manifest_format: u16,
    pub pending_imports: u64,
}

impl IndexHealth {
    /// Derived string label for CLI/logging (must match [`Self::memorydb`]).
    pub fn memorydb_connection_label(&self) -> &'static str {
        self.memorydb.as_label()
    }

    pub fn memorydb_ready(&self) -> bool {
        self.memorydb.is_ready()
    }

    pub fn memorydb_generation(&self) -> u64 {
        self.memorydb.database_generation()
    }
}

impl Default for IndexHealth {
    fn default() -> Self {
        Self {
            ready: false,
            state: HealthState::Starting,
            reasons: Vec::new(),
            memorydb: MemoryDbConnectionState::Unknown,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ready_and_flag_consistent() {
        let mut h = IndexHealth::default();
        assert_eq!(h.memorydb_ready(), false);
        assert_eq!(h.memorydb.memorydb_ready_flag(), 0);
        h.memorydb.observe_success(3, 7, 100);
        assert!(h.memorydb_ready());
        assert_eq!(h.memorydb.memorydb_ready_flag(), 1);
        assert_eq!(h.memorydb_generation(), 7);
        h.memorydb
            .observe_failure(MemoryDbDegradedReason::Unavailable, 200, None);
        assert!(!h.memorydb_ready());
        assert_eq!(h.memorydb_connection_label(), "Unavailable");
        // Later success overrides permanent-looking failure.
        h.memorydb.observe_success(4, 8, 300);
        assert!(h.memorydb_ready());
        assert_eq!(h.memorydb_generation(), 8);
    }
}
