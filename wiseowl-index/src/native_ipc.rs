//! Native SunlightOS IPC envelope for wiseowl-index (Phase 3).
//!
//! Endpoint: `wiseowl.index.v1`
//! Op range: `0x4E00`–`0x4EFF` (adjacent to memorydb `0x4Dxx`).

use crate::error::IndexError;

/// Protocol version for the native envelope.
pub const NATIVE_PROTOCOL_VERSION: u16 = 1;

/// Maximum request body.
pub const MAX_REQUEST_BODY: u32 = 64 * 1024;
/// Maximum reply body.
pub const MAX_REPLY_BODY: u32 = 64 * 1024;
/// Inline threshold fitting one SHM page with framing.
pub const INLINE_PAYLOAD_THRESHOLD: u32 = 3072;
/// SHM page size.
pub const SHM_PAGE_SIZE: u32 = 4096;

/// Explicit owner-retained SHM lifecycle used by native Wise Owl requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ShmLeaseState {
    Allocated = 1,
    SharedReadOnly = 2,
    MappedByMemoryDb = 3,
    Consumed = 4,
    Unmapped = 5,
    ReleasedByOwner = 6,
}

impl ShmLeaseState {
    pub fn transition(self, next: Self) -> Result<Self, IndexError> {
        let valid = matches!(
            (self, next),
            (Self::Allocated, Self::SharedReadOnly)
                | (Self::SharedReadOnly, Self::MappedByMemoryDb)
                | (Self::MappedByMemoryDb, Self::Consumed)
                | (Self::Consumed, Self::Unmapped)
                | (Self::Unmapped, Self::ReleasedByOwner)
        );
        if valid {
            Ok(next)
        } else {
            Err(IndexError::InvalidRequest("invalid shm lease transition"))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ShmCounters {
    pub shm_allocations: u64,
    pub shm_shares: u64,
    pub shm_maps: u64,
    pub shm_unmaps: u64,
    pub shm_owner_frees: u64,
    pub shm_transfer_failures: u64,
    pub shm_invalid_handles: u64,
    pub shm_stale_handles: u64,
    pub shm_foreign_handles: u64,
    pub shm_bytes_active: u64,
    pub shm_bytes_peak: u64,
    pub active_shm_leases: u64,
}

/// Fixed header (24 bytes, little-endian).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexIpcHeader {
    pub protocol_version: u16,
    pub operation: u16,
    pub flags: u32,
    pub request_id: u64,
    pub body_len: u32,
    pub reserved: u32,
}

pub const INDEX_IPC_HEADER_LEN: usize = 24;
pub const REQUIRED_FLAGS_MASK: u32 = 0xFFFF_0000;

impl IndexIpcHeader {
    pub fn encode(&self) -> [u8; INDEX_IPC_HEADER_LEN] {
        let mut out = [0u8; INDEX_IPC_HEADER_LEN];
        out[0..2].copy_from_slice(&self.protocol_version.to_le_bytes());
        out[2..4].copy_from_slice(&self.operation.to_le_bytes());
        out[4..8].copy_from_slice(&self.flags.to_le_bytes());
        out[8..16].copy_from_slice(&self.request_id.to_le_bytes());
        out[16..20].copy_from_slice(&self.body_len.to_le_bytes());
        out[20..24].copy_from_slice(&self.reserved.to_le_bytes());
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, IndexError> {
        if bytes.len() < INDEX_IPC_HEADER_LEN {
            return Err(IndexError::InvalidRequest("truncated header"));
        }
        let h = Self {
            protocol_version: u16::from_le_bytes([bytes[0], bytes[1]]),
            operation: u16::from_le_bytes([bytes[2], bytes[3]]),
            flags: u32::from_le_bytes(bytes[4..8].try_into().unwrap()),
            request_id: u64::from_le_bytes(bytes[8..16].try_into().unwrap()),
            body_len: u32::from_le_bytes(bytes[16..20].try_into().unwrap()),
            reserved: u32::from_le_bytes(bytes[20..24].try_into().unwrap()),
        };
        h.validate()?;
        Ok(h)
    }

    pub fn validate(&self) -> Result<(), IndexError> {
        if self.protocol_version != NATIVE_PROTOCOL_VERSION {
            return Err(IndexError::UnsupportedProtocolVersion {
                got: self.protocol_version,
                want: NATIVE_PROTOCOL_VERSION,
            });
        }
        if self.flags & REQUIRED_FLAGS_MASK != 0 {
            return Err(IndexError::InvalidRequest("unknown required flags"));
        }
        if self.body_len > MAX_REQUEST_BODY {
            return Err(IndexError::PayloadTooLarge {
                size: self.body_len,
                max: MAX_REQUEST_BODY,
            });
        }
        Ok(())
    }
}

/// Stable native operation codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum IndexOp {
    RegisterRoot = 0x4E01,
    RemoveRoot = 0x4E02,
    ListRoots = 0x4E03,
    StartScan = 0x4E04,
    GetScanStatus = 0x4E05,
    ListSources = 0x4E06,
    InspectSource = 0x4E07,
    RetrySource = 0x4E08,
    ReindexSource = 0x4E09,
    ForgetSource = 0x4E0A,
    TokenizeText = 0x4E0B,
    SearchText = 0x4E0C,
    GetStats = 0x4E0D,
    GetHealth = 0x4E0E,
    ReleaseLease = 0x4E0F,
    /// Phase 3.5
    GetTransport = 0x4E10,
    GetMemoryDb = 0x4E11,
    GetPending = 0x4E12,
    Reconcile = 0x4E13,
    GetDigest = 0x4E14,
    /// Development/test build only: arm deterministic commit crash window.
    TestArmCommitCrash = 0x4EF0,
    TestNativeVerdict = 0x4EF1,
    TestArmShmCrash = 0x4EF2,
    /// Phase 3.875 soak / remaining Phase 4 readiness gates.
    TestPhase3875Soak = 0x4EF3,
    Reply = 0x4E80,
    Error = 0x4EFF,
}

impl IndexOp {
    pub const fn from_u16(v: u16) -> Option<Self> {
        match v {
            0x4E01 => Some(Self::RegisterRoot),
            0x4E02 => Some(Self::RemoveRoot),
            0x4E03 => Some(Self::ListRoots),
            0x4E04 => Some(Self::StartScan),
            0x4E05 => Some(Self::GetScanStatus),
            0x4E06 => Some(Self::ListSources),
            0x4E07 => Some(Self::InspectSource),
            0x4E08 => Some(Self::RetrySource),
            0x4E09 => Some(Self::ReindexSource),
            0x4E0A => Some(Self::ForgetSource),
            0x4E0B => Some(Self::TokenizeText),
            0x4E0C => Some(Self::SearchText),
            0x4E0D => Some(Self::GetStats),
            0x4E0E => Some(Self::GetHealth),
            0x4E0F => Some(Self::ReleaseLease),
            0x4E10 => Some(Self::GetTransport),
            0x4E11 => Some(Self::GetMemoryDb),
            0x4E12 => Some(Self::GetPending),
            0x4E13 => Some(Self::Reconcile),
            0x4E14 => Some(Self::GetDigest),
            0x4EF0 => Some(Self::TestArmCommitCrash),
            0x4EF1 => Some(Self::TestNativeVerdict),
            0x4EF2 => Some(Self::TestArmShmCrash),
            0x4EF3 => Some(Self::TestPhase3875Soak),
            0x4E80 => Some(Self::Reply),
            0x4EFF => Some(Self::Error),
            _ => None,
        }
    }
}

/// SHM descriptor for large payloads / results.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexShmDescriptor {
    pub offset: u32,
    pub length: u32,
    pub flags: u32,
}

impl IndexShmDescriptor {
    pub fn encode(&self) -> [u8; 12] {
        let mut out = [0u8; 12];
        out[0..4].copy_from_slice(&self.offset.to_le_bytes());
        out[4..8].copy_from_slice(&self.length.to_le_bytes());
        out[8..12].copy_from_slice(&self.flags.to_le_bytes());
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, IndexError> {
        if bytes.len() < 12 {
            return Err(IndexError::InvalidRequest("shm descriptor"));
        }
        let offset = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
        let length = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        let flags = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
        // Reject offset overflow combinations for page math.
        let end = offset
            .checked_add(length)
            .ok_or(IndexError::InvalidRequest("shm overflow"))?;
        if end > MAX_REPLY_BODY.saturating_mul(4) {
            return Err(IndexError::PayloadTooLarge {
                size: end,
                max: MAX_REPLY_BODY.saturating_mul(4),
            });
        }
        Ok(Self {
            offset,
            length,
            flags,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_roundtrip() {
        let h = IndexIpcHeader {
            protocol_version: NATIVE_PROTOCOL_VERSION,
            operation: IndexOp::TokenizeText as u16,
            flags: 0,
            request_id: 42,
            body_len: 16,
            reserved: 0,
        };
        let b = h.encode();
        let d = IndexIpcHeader::decode(&b).unwrap();
        assert_eq!(h, d);
    }

    #[test]
    fn rejects_unknown_version() {
        let h = IndexIpcHeader {
            protocol_version: 99,
            operation: 1,
            flags: 0,
            request_id: 1,
            body_len: 0,
            reserved: 0,
        };
        let b = h.encode();
        assert!(IndexIpcHeader::decode(&b).is_err());
    }

    #[test]
    fn shm_lifecycle_rejects_invalid_transitions() {
        let state = ShmLeaseState::Allocated
            .transition(ShmLeaseState::SharedReadOnly)
            .unwrap()
            .transition(ShmLeaseState::MappedByMemoryDb)
            .unwrap()
            .transition(ShmLeaseState::Consumed)
            .unwrap()
            .transition(ShmLeaseState::Unmapped)
            .unwrap()
            .transition(ShmLeaseState::ReleasedByOwner)
            .unwrap();
        assert_eq!(state, ShmLeaseState::ReleasedByOwner);
        assert!(ShmLeaseState::Allocated
            .transition(ShmLeaseState::ReleasedByOwner)
            .is_err());
    }
}
