//! Native SunlightOS IPC envelope for wiseowl-memorydb (Phase 2).
//!
//! Endpoint: `wiseowl.memorydb.v1`
//!
//! Large payloads use validated SHM; small requests may use framed SHM pages
//! under [`INLINE_PAYLOAD_THRESHOLD`].

use crate::error::DbError;

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

/// Fixed header (24 bytes, little-endian).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryDbIpcHeader {
    pub protocol_version: u16,
    pub operation: u16,
    pub flags: u32,
    pub request_id: u64,
    pub body_len: u32,
    pub reserved: u32,
}

pub const MEMORYDB_IPC_HEADER_LEN: usize = 24;
pub const REQUIRED_FLAGS_MASK: u32 = 0xFFFF_0000;

impl MemoryDbIpcHeader {
    pub fn encode(&self) -> [u8; MEMORYDB_IPC_HEADER_LEN] {
        let mut out = [0u8; MEMORYDB_IPC_HEADER_LEN];
        out[0..2].copy_from_slice(&self.protocol_version.to_le_bytes());
        out[2..4].copy_from_slice(&self.operation.to_le_bytes());
        out[4..8].copy_from_slice(&self.flags.to_le_bytes());
        out[8..16].copy_from_slice(&self.request_id.to_le_bytes());
        out[16..20].copy_from_slice(&self.body_len.to_le_bytes());
        out[20..24].copy_from_slice(&self.reserved.to_le_bytes());
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, DbError> {
        if bytes.len() < MEMORYDB_IPC_HEADER_LEN {
            return Err(DbError::InvalidRequest("truncated header"));
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

    pub fn validate(&self) -> Result<(), DbError> {
        if self.protocol_version != NATIVE_PROTOCOL_VERSION {
            return Err(DbError::UnsupportedProtocolVersion {
                got: self.protocol_version,
                want: NATIVE_PROTOCOL_VERSION,
            });
        }
        if self.flags & REQUIRED_FLAGS_MASK != 0 {
            return Err(DbError::InvalidRequest("unknown required flags"));
        }
        if self.body_len > MAX_REQUEST_BODY {
            return Err(DbError::PayloadTooLarge {
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
pub enum MemoryDbOp {
    BeginTransaction = 0x4D01,
    InsertRecord = 0x4D02,
    InsertRelationship = 0x4D03,
    CommitTransaction = 0x4D04,
    AbortTransaction = 0x4D05,
    GetRecord = 0x4D06,
    Query = 0x4D07,
    ListRevisions = 0x4D08,
    GetRelationships = 0x4D09,
    TombstoneRecord = 0x4D0A,
    DeleteSource = 0x4D0B,
    CreateCheckpoint = 0x4D0C,
    RunCompaction = 0x4D0D,
    GetStats = 0x4D0E,
    GetHealth = 0x4D0F,
    SourceLookup = 0x4D10,
    RebuildIndexes = 0x4D11,
    Verify = 0x4D12,
    OwlQl = 0x4D13,
    ReleaseLease = 0x4D14,
    Reply = 0x4D80,
    Error = 0x4DFF,
}

impl MemoryDbOp {
    pub const fn from_u16(v: u16) -> Option<Self> {
        match v {
            0x4D01 => Some(Self::BeginTransaction),
            0x4D02 => Some(Self::InsertRecord),
            0x4D03 => Some(Self::InsertRelationship),
            0x4D04 => Some(Self::CommitTransaction),
            0x4D05 => Some(Self::AbortTransaction),
            0x4D06 => Some(Self::GetRecord),
            0x4D07 => Some(Self::Query),
            0x4D08 => Some(Self::ListRevisions),
            0x4D09 => Some(Self::GetRelationships),
            0x4D0A => Some(Self::TombstoneRecord),
            0x4D0B => Some(Self::DeleteSource),
            0x4D0C => Some(Self::CreateCheckpoint),
            0x4D0D => Some(Self::RunCompaction),
            0x4D0E => Some(Self::GetStats),
            0x4D0F => Some(Self::GetHealth),
            0x4D10 => Some(Self::SourceLookup),
            0x4D11 => Some(Self::RebuildIndexes),
            0x4D12 => Some(Self::Verify),
            0x4D13 => Some(Self::OwlQl),
            0x4D14 => Some(Self::ReleaseLease),
            0x4D80 => Some(Self::Reply),
            0x4DFF => Some(Self::Error),
            _ => None,
        }
    }
}

/// SHM descriptor for large payloads / results.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryDbShmDescriptor {
    pub offset: u32,
    pub length: u32,
    pub flags: u32,
}

impl MemoryDbShmDescriptor {
    pub fn validate(&self, max: u32) -> Result<(), DbError> {
        let end = self
            .offset
            .checked_add(self.length)
            .ok_or(DbError::InvalidRequest("shm overflow"))?;
        if self.length > max || end > max {
            return Err(DbError::PayloadTooLarge {
                size: self.length,
                max,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_roundtrip() {
        let h = MemoryDbIpcHeader {
            protocol_version: NATIVE_PROTOCOL_VERSION,
            operation: MemoryDbOp::GetStats as u16,
            flags: 0,
            request_id: 42,
            body_len: 0,
            reserved: 0,
        };
        let e = h.encode();
        let d = MemoryDbIpcHeader::decode(&e).unwrap();
        assert_eq!(d, h);
    }

    #[test]
    fn rejects_unknown_version() {
        let mut h = MemoryDbIpcHeader {
            protocol_version: 99,
            operation: 0,
            flags: 0,
            request_id: 0,
            body_len: 0,
            reserved: 0,
        };
        let e = h.encode();
        // bypass validate in encode path
        h.protocol_version = 99;
        let mut e = h.encode();
        e[0] = 99;
        e[1] = 0;
        assert!(MemoryDbIpcHeader::decode(&e).is_err());
    }
}
