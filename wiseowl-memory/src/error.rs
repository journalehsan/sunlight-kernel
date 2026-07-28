//! Structured errors for Wise Owl short-term memory.
//!
//! Error messages never include payload contents or internal addresses.

use core::fmt;

use crate::lifecycle::LifecycleOp;

/// Stable structured error codes for IPC and diagnostics.
///
/// Static message slices are documentation-only labels (never payload content).
/// Host IPC serializes via a code + optional short label rather than lifetimes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryError {
    InvalidRequest(&'static str),
    UnsupportedProtocolVersion {
        got: u16,
        want: u16,
    },
    MalformedIdentifier(&'static str),
    PayloadTooLarge {
        size: u32,
        max: u32,
    },
    QuotaExceeded(&'static str),
    SessionQuotaExceeded,
    EntryNotFound,
    EntryExpired,
    InvalidLifecycleTransition {
        from: &'static str,
        op: LifecycleOp,
    },
    SharedMemoryValidationFailure(&'static str),
    CompressionFailure,
    DecompressionFailure,
    ChecksumMismatch,
    KvUnavailable,
    KvPromotionRejected(&'static str),
    PermissionDenied(&'static str),
    InternalInvariantViolation(&'static str),
    SegmentNotFound,
    SessionNotFound,
    EntrySealed,
    EntryDeleted,
    WorkBudgetExceeded,
    SpillCorrupt,
    SpillIncomplete,
    /// KV key exists but version/checksum/metadata do not match the local record.
    PromotionConflict {
        key: &'static str,
    },
    /// Service health is failed and cannot accept the request.
    ServiceFailed,
    /// Read lease or SHM mapping not found / expired.
    LeaseNotFound,
}

#[cfg(feature = "host")]
impl serde::Serialize for MemoryError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut st = serializer.serialize_struct("MemoryError", 2)?;
        st.serialize_field("code", &self.code())?;
        st.serialize_field("label", self.label())?;
        st.end()
    }
}

#[cfg(feature = "host")]
impl<'de> serde::Deserialize<'de> for MemoryError {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(serde::Deserialize)]
        struct Wire {
            code: u32,
            #[allow(dead_code)]
            label: String,
        }
        let w = Wire::deserialize(deserializer)?;
        Ok(match w.code {
            1 => Self::InvalidRequest("invalid request"),
            2 => Self::UnsupportedProtocolVersion { got: 0, want: 1 },
            3 => Self::MalformedIdentifier("identifier"),
            4 => Self::PayloadTooLarge { size: 0, max: 0 },
            5 => Self::QuotaExceeded("quota"),
            6 => Self::SessionQuotaExceeded,
            7 => Self::EntryNotFound,
            8 => Self::EntryExpired,
            9 => Self::InvalidLifecycleTransition {
                from: "unknown",
                op: LifecycleOp::Read,
            },
            10 => Self::SharedMemoryValidationFailure("shm"),
            11 => Self::CompressionFailure,
            12 => Self::DecompressionFailure,
            13 => Self::ChecksumMismatch,
            14 => Self::KvUnavailable,
            15 => Self::KvPromotionRejected("rejected"),
            16 => Self::PermissionDenied("denied"),
            17 => Self::InternalInvariantViolation("invariant"),
            18 => Self::SegmentNotFound,
            19 => Self::SessionNotFound,
            20 => Self::EntrySealed,
            21 => Self::EntryDeleted,
            22 => Self::WorkBudgetExceeded,
            23 => Self::SpillCorrupt,
            24 => Self::SpillIncomplete,
            25 => Self::PromotionConflict { key: "conflict" },
            26 => Self::ServiceFailed,
            27 => Self::LeaseNotFound,
            _ => Self::InvalidRequest("unknown error code"),
        })
    }
}

impl MemoryError {
    /// Stable numeric code for IPC wire encoding.
    pub const fn code(&self) -> u32 {
        match self {
            Self::InvalidRequest(_) => 1,
            Self::UnsupportedProtocolVersion { .. } => 2,
            Self::MalformedIdentifier(_) => 3,
            Self::PayloadTooLarge { .. } => 4,
            Self::QuotaExceeded(_) => 5,
            Self::SessionQuotaExceeded => 6,
            Self::EntryNotFound => 7,
            Self::EntryExpired => 8,
            Self::InvalidLifecycleTransition { .. } => 9,
            Self::SharedMemoryValidationFailure(_) => 10,
            Self::CompressionFailure => 11,
            Self::DecompressionFailure => 12,
            Self::ChecksumMismatch => 13,
            Self::KvUnavailable => 14,
            Self::KvPromotionRejected(_) => 15,
            Self::PermissionDenied(_) => 16,
            Self::InternalInvariantViolation(_) => 17,
            Self::SegmentNotFound => 18,
            Self::SessionNotFound => 19,
            Self::EntrySealed => 20,
            Self::EntryDeleted => 21,
            Self::WorkBudgetExceeded => 22,
            Self::SpillCorrupt => 23,
            Self::SpillIncomplete => 24,
            Self::PromotionConflict { .. } => 25,
            Self::ServiceFailed => 26,
            Self::LeaseNotFound => 27,
        }
    }

    /// Short label safe for logs (no payload data).
    pub fn label(&self) -> &'static str {
        match self {
            Self::InvalidRequest(_) => "invalid_request",
            Self::UnsupportedProtocolVersion { .. } => "unsupported_protocol_version",
            Self::MalformedIdentifier(_) => "malformed_identifier",
            Self::PayloadTooLarge { .. } => "payload_too_large",
            Self::QuotaExceeded(_) => "quota_exceeded",
            Self::SessionQuotaExceeded => "session_quota_exceeded",
            Self::EntryNotFound => "entry_not_found",
            Self::EntryExpired => "entry_expired",
            Self::InvalidLifecycleTransition { .. } => "invalid_lifecycle_transition",
            Self::SharedMemoryValidationFailure(_) => "shm_validation_failure",
            Self::CompressionFailure => "compression_failure",
            Self::DecompressionFailure => "decompression_failure",
            Self::ChecksumMismatch => "checksum_mismatch",
            Self::KvUnavailable => "kv_unavailable",
            Self::KvPromotionRejected(_) => "kv_promotion_rejected",
            Self::PermissionDenied(_) => "permission_denied",
            Self::InternalInvariantViolation(_) => "internal_invariant_violation",
            Self::SegmentNotFound => "segment_not_found",
            Self::SessionNotFound => "session_not_found",
            Self::EntrySealed => "entry_sealed",
            Self::EntryDeleted => "entry_deleted",
            Self::WorkBudgetExceeded => "work_budget_exceeded",
            Self::SpillCorrupt => "spill_corrupt",
            Self::SpillIncomplete => "spill_incomplete",
            Self::PromotionConflict { .. } => "promotion_conflict",
            Self::ServiceFailed => "service_failed",
            Self::LeaseNotFound => "lease_not_found",
        }
    }
}

impl fmt::Display for MemoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRequest(m) => write!(f, "invalid request: {m}"),
            Self::UnsupportedProtocolVersion { got, want } => {
                write!(f, "unsupported protocol version {got} (want {want})")
            }
            Self::MalformedIdentifier(m) => write!(f, "malformed identifier: {m}"),
            Self::PayloadTooLarge { size, max } => {
                write!(f, "payload too large: {size} > {max}")
            }
            Self::QuotaExceeded(m) => write!(f, "quota exceeded: {m}"),
            Self::SessionQuotaExceeded => write!(f, "session quota exceeded"),
            Self::EntryNotFound => write!(f, "entry not found"),
            Self::EntryExpired => write!(f, "entry expired"),
            Self::InvalidLifecycleTransition { from, op } => {
                write!(f, "invalid lifecycle transition: {from} + {op:?}")
            }
            Self::SharedMemoryValidationFailure(m) => write!(f, "shm validation failed: {m}"),
            Self::CompressionFailure => write!(f, "compression failure"),
            Self::DecompressionFailure => write!(f, "decompression failure"),
            Self::ChecksumMismatch => write!(f, "checksum mismatch"),
            Self::KvUnavailable => write!(f, "kv unavailable"),
            Self::KvPromotionRejected(m) => write!(f, "kv promotion rejected: {m}"),
            Self::PermissionDenied(m) => write!(f, "permission denied: {m}"),
            Self::InternalInvariantViolation(m) => {
                write!(f, "internal invariant violation: {m}")
            }
            Self::SegmentNotFound => write!(f, "segment not found"),
            Self::SessionNotFound => write!(f, "session not found"),
            Self::EntrySealed => write!(f, "entry is sealed"),
            Self::EntryDeleted => write!(f, "entry is deleted"),
            Self::WorkBudgetExceeded => write!(f, "maintenance work budget exceeded"),
            Self::SpillCorrupt => write!(f, "spill record corrupt"),
            Self::SpillIncomplete => write!(f, "spill record incomplete"),
            Self::PromotionConflict { key } => {
                write!(f, "promotion conflict for key context: {key}")
            }
            Self::ServiceFailed => write!(f, "service failed"),
            Self::LeaseNotFound => write!(f, "read lease not found"),
        }
    }
}

#[cfg(feature = "host")]
impl std::error::Error for MemoryError {}
