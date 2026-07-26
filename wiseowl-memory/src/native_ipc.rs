//! Native SunlightOS IPC envelope for wiseowl-memory (Phase 1.1).
//!
//! This module defines the **target ABI**. Host UDS+bincode is separate and
//! must not be treated as the native wire format.
//!
//! # Inline payload threshold
//!
//! `IpcMsg` carries at most [`sunlight_ipc::IPC_MAX_WORDS`] words (8 × u64) plus
//! up to two capability tokens. Large payloads use SHM pages of
//! [`INLINE_PAYLOAD_THRESHOLD`] bytes or less may be placed in a single SHM
//! page transferred by capability; the conservative inline body limit used for
//! the framed body-in-SHM path is **3072 bytes** so a request header + body
//! always fits in one 4096-byte SHM page with room for framing.
//!
//! Requests larger than [`INLINE_PAYLOAD_THRESHOLD`] must use an explicit
//! [`MemoryShmDescriptor`].

#![allow(dead_code)]

use crate::error::MemoryError;

/// Protocol version for the native envelope (independent of bincode host protocol).
pub const NATIVE_PROTOCOL_VERSION: u16 = 1;

/// Maximum body size accepted in a single request (hard cap).
pub const MAX_REQUEST_BODY: u32 = 64 * 1024;
/// Maximum reply body size (hard cap).
pub const MAX_REPLY_BODY: u32 = 64 * 1024;
/// Inline threshold: payloads at or below this may travel in the request SHM
/// page without a separate large-payload descriptor dance.
/// Chosen so header (24) + body ≤ one 4096-byte SHM page with margin.
pub const INLINE_PAYLOAD_THRESHOLD: u32 = 3072;
/// SHM page size used by SunlightOS (matches `sunlight_ipc::SHM_PAGE`).
pub const SHM_PAGE_SIZE: u32 = 4096;

/// Fixed-size native request/reply header (little-endian on the wire).
///
/// Layout (24 bytes):
/// ```text
/// protocol_version u16
/// operation        u16
/// flags            u32
/// request_id       u64
/// body_len         u32
/// reserved         u32
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct MemoryIpcHeader {
    pub protocol_version: u16,
    pub operation: u16,
    pub flags: u32,
    pub request_id: u64,
    pub body_len: u32,
    pub reserved: u32,
}

pub const MEMORY_IPC_HEADER_LEN: usize = 24;

/// Unknown required flags mask: bits that clients must not set until defined.
pub const REQUIRED_FLAGS_MASK: u32 = 0xFFFF_0000;

impl MemoryIpcHeader {
    pub fn encode(&self) -> [u8; MEMORY_IPC_HEADER_LEN] {
        let mut out = [0u8; MEMORY_IPC_HEADER_LEN];
        out[0..2].copy_from_slice(&self.protocol_version.to_le_bytes());
        out[2..4].copy_from_slice(&self.operation.to_le_bytes());
        out[4..8].copy_from_slice(&self.flags.to_le_bytes());
        out[8..16].copy_from_slice(&self.request_id.to_le_bytes());
        out[16..20].copy_from_slice(&self.body_len.to_le_bytes());
        out[20..24].copy_from_slice(&self.reserved.to_le_bytes());
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, MemoryError> {
        if bytes.len() < MEMORY_IPC_HEADER_LEN {
            return Err(MemoryError::InvalidRequest("truncated header"));
        }
        let protocol_version = u16::from_le_bytes([bytes[0], bytes[1]]);
        let operation = u16::from_le_bytes([bytes[2], bytes[3]]);
        let flags = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        let request_id = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
        let body_len = u32::from_le_bytes(bytes[16..20].try_into().unwrap());
        let reserved = u32::from_le_bytes(bytes[20..24].try_into().unwrap());
        let h = Self {
            protocol_version,
            operation,
            flags,
            request_id,
            body_len,
            reserved,
        };
        h.validate()?;
        Ok(h)
    }

    pub fn validate(&self) -> Result<(), MemoryError> {
        if self.protocol_version != NATIVE_PROTOCOL_VERSION {
            return Err(MemoryError::UnsupportedProtocolVersion {
                got: self.protocol_version,
                want: NATIVE_PROTOCOL_VERSION,
            });
        }
        if self.flags & REQUIRED_FLAGS_MASK != 0 {
            return Err(MemoryError::InvalidRequest("unknown required flags"));
        }
        if self.body_len > MAX_REQUEST_BODY {
            return Err(MemoryError::PayloadTooLarge {
                size: self.body_len,
                max: MAX_REQUEST_BODY,
            });
        }
        Ok(())
    }
}

/// Stable native operation codes (message labels).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum MemoryOp {
    RegisterClient = 0x4F01,
    CreateSession = 0x4F02,
    CreateEntry = 0x4F03,
    AppendEntry = 0x4F04,
    ReadEntry = 0x4F05,
    TouchEntry = 0x4F06,
    SealEntry = 0x4F07,
    DeleteEntry = 0x4F08,
    PromoteEntry = 0x4F09,
    ListEntries = 0x4F0A,
    ListSessions = 0x4F0B,
    GetStats = 0x4F0C,
    RunMaintenance = 0x4F0D,
    ClientDisconnect = 0x4F0E,
    ReleaseLease = 0x4F0F,
    TransportInfo = 0x4F10,
    /// Reply label
    Reply = 0x4F80,
    /// Error reply label
    Error = 0x4FFF,
}

impl MemoryOp {
    pub const fn from_u16(v: u16) -> Option<Self> {
        match v {
            0x4F01 => Some(Self::RegisterClient),
            0x4F02 => Some(Self::CreateSession),
            0x4F03 => Some(Self::CreateEntry),
            0x4F04 => Some(Self::AppendEntry),
            0x4F05 => Some(Self::ReadEntry),
            0x4F06 => Some(Self::TouchEntry),
            0x4F07 => Some(Self::SealEntry),
            0x4F08 => Some(Self::DeleteEntry),
            0x4F09 => Some(Self::PromoteEntry),
            0x4F0A => Some(Self::ListEntries),
            0x4F0B => Some(Self::ListSessions),
            0x4F0C => Some(Self::GetStats),
            0x4F0D => Some(Self::RunMaintenance),
            0x4F0E => Some(Self::ClientDisconnect),
            0x4F0F => Some(Self::ReleaseLease),
            0x4F10 => Some(Self::TransportInfo),
            0x4F80 => Some(Self::Reply),
            0x4FFF => Some(Self::Error),
            _ => None,
        }
    }

    pub const fn as_u16(self) -> u16 {
        self as u16
    }

    /// IPC message label (same as op code for this service).
    pub const fn label(self) -> u64 {
        self as u16 as u64
    }
}

/// SHM access mode for validated descriptors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ShmAccess {
    ReadOnly = 1,
    // Write from client is never accepted for service-owned content.
}

/// Validated SHM payload descriptor (native path).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryShmDescriptor {
    /// Capability token value (opaque u64 on wire; kernel validates ownership).
    pub handle: u64,
    pub offset: u32,
    pub length: u32,
    pub access: ShmAccess,
    pub checksum: Option<u32>,
}

impl MemoryShmDescriptor {
    pub fn validate(&self, mapping_len: u32) -> Result<(), MemoryError> {
        if self.handle == 0 {
            return Err(MemoryError::SharedMemoryValidationFailure("invalid handle"));
        }
        if self.access != ShmAccess::ReadOnly {
            return Err(MemoryError::SharedMemoryValidationFailure(
                "write access not permitted",
            ));
        }
        let end = self
            .offset
            .checked_add(self.length)
            .ok_or(MemoryError::SharedMemoryValidationFailure(
                "offset+length overflow",
            ))?;
        if end > mapping_len {
            return Err(MemoryError::SharedMemoryValidationFailure(
                "out of range mapping",
            ));
        }
        if self.length > MAX_REQUEST_BODY {
            return Err(MemoryError::PayloadTooLarge {
                size: self.length,
                max: MAX_REQUEST_BODY,
            });
        }
        Ok(())
    }
}

/// Encode a simple error reply body: error code (u32) + request_id (u64).
pub fn encode_error_body(code: u32, request_id: u64) -> [u8; 12] {
    let mut out = [0u8; 12];
    out[0..4].copy_from_slice(&code.to_le_bytes());
    out[4..12].copy_from_slice(&request_id.to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_roundtrip() {
        let h = MemoryIpcHeader {
            protocol_version: NATIVE_PROTOCOL_VERSION,
            operation: MemoryOp::GetStats.as_u16(),
            flags: 0,
            request_id: 42,
            body_len: 16,
            reserved: 0,
        };
        let enc = h.encode();
        let dec = MemoryIpcHeader::decode(&enc).unwrap();
        assert_eq!(dec, h);
    }

    #[test]
    fn rejects_unknown_version() {
        let h = MemoryIpcHeader {
            protocol_version: 99,
            operation: 1,
            flags: 0,
            request_id: 1,
            body_len: 0,
            reserved: 0,
        };
        assert!(h.validate().is_err());
    }

    #[test]
    fn rejects_unknown_required_flags() {
        let h = MemoryIpcHeader {
            protocol_version: NATIVE_PROTOCOL_VERSION,
            operation: 1,
            flags: 0x0001_0000,
            request_id: 1,
            body_len: 0,
            reserved: 0,
        };
        assert!(h.validate().is_err());
    }

    #[test]
    fn shm_descriptor_bounds() {
        let d = MemoryShmDescriptor {
            handle: 1,
            offset: 4000,
            length: 200,
            access: ShmAccess::ReadOnly,
            checksum: None,
        };
        assert!(d.validate(4096).is_err());
        let d2 = MemoryShmDescriptor {
            handle: 1,
            offset: 0,
            length: 100,
            access: ShmAccess::ReadOnly,
            checksum: Some(1),
        };
        assert!(d2.validate(4096).is_ok());
    }

    #[test]
    fn inline_threshold_fits_page() {
        assert!(
            (MEMORY_IPC_HEADER_LEN as u32 + INLINE_PAYLOAD_THRESHOLD) <= SHM_PAGE_SIZE
        );
    }
}
