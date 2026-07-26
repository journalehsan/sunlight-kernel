//! Sealed segment design for cold short-term memory.
//!
//! Lifecycle: Open -> Sealed -> Compressed -> Spilled -> Rehydrated -> Expired/Deleted
//!
//! Only one owner may mutate an open segment. After sealing, records are
//! immutable and compression happens at most once.

use crate::compression::{compress_lz4, crc32_ieee, decompress_lz4_checked, COMPRESSION_LZ4};
use crate::error::MemoryError;
use crate::ids::{MemoryId, SegmentId, SessionId};

/// On-disk / wire format version for cold segments.
pub const SEGMENT_FORMAT_VERSION: u16 = 1;

/// Magic: "OWLS" (Owl Segment).
pub const SEGMENT_MAGIC: [u8; 4] = *b"OWLS";

/// Fixed header size (little-endian, architecture-independent).
/// magic(4) + version(2) + alg(1) + pad(1) + segment_id(8) + session_id(8)
/// + uncompressed(4) + compressed(4) + record_count(4) + checksum(4)
/// + created_ns(8) + expires_ns(8) = 56 bytes
pub const SEGMENT_HEADER_LEN: usize = 56;

/// Segment lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentState {
    Open,
    Sealed,
    Compressed,
    Spilled,
    Rehydrated,
    Expired,
    Deleted,
}

/// Cold segment header (validated before decompression).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColdSegmentHeader {
    pub version: u16,
    pub compression: u8,
    pub segment_id: SegmentId,
    pub session_id: SessionId,
    pub uncompressed_size: u32,
    pub compressed_size: u32,
    pub record_count: u32,
    pub checksum: u32,
    pub created_at_ns: u64,
    pub expires_at_ns: u64,
}

impl ColdSegmentHeader {
    pub fn encode(&self) -> [u8; SEGMENT_HEADER_LEN] {
        let mut out = [0u8; SEGMENT_HEADER_LEN];
        out[0..4].copy_from_slice(&SEGMENT_MAGIC);
        out[4..6].copy_from_slice(&self.version.to_le_bytes());
        out[6] = self.compression;
        out[7] = 0; // reserved
        out[8..16].copy_from_slice(&self.segment_id.to_le_bytes());
        out[16..24].copy_from_slice(&self.session_id.to_le_bytes());
        out[24..28].copy_from_slice(&self.uncompressed_size.to_le_bytes());
        out[28..32].copy_from_slice(&self.compressed_size.to_le_bytes());
        out[32..36].copy_from_slice(&self.record_count.to_le_bytes());
        out[36..40].copy_from_slice(&self.checksum.to_le_bytes());
        out[40..48].copy_from_slice(&self.created_at_ns.to_le_bytes());
        out[48..56].copy_from_slice(&self.expires_at_ns.to_le_bytes());
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, MemoryError> {
        if bytes.len() < SEGMENT_HEADER_LEN {
            return Err(MemoryError::SpillIncomplete);
        }
        if bytes[0..4] != SEGMENT_MAGIC {
            return Err(MemoryError::SpillCorrupt);
        }
        let version = u16::from_le_bytes([bytes[4], bytes[5]]);
        if version != SEGMENT_FORMAT_VERSION {
            return Err(MemoryError::UnsupportedProtocolVersion {
                got: version,
                want: SEGMENT_FORMAT_VERSION,
            });
        }
        let compression = bytes[6];
        let segment_id = SegmentId::from_le_bytes(bytes[8..16].try_into().unwrap())
            .map_err(|_| MemoryError::MalformedIdentifier("segment_id"))?;
        let session_id = SessionId::from_le_bytes(bytes[16..24].try_into().unwrap())
            .map_err(|_| MemoryError::MalformedIdentifier("session_id"))?;
        let uncompressed_size = u32::from_le_bytes(bytes[24..28].try_into().unwrap());
        let compressed_size = u32::from_le_bytes(bytes[28..32].try_into().unwrap());
        let record_count = u32::from_le_bytes(bytes[32..36].try_into().unwrap());
        let checksum = u32::from_le_bytes(bytes[36..40].try_into().unwrap());
        let created_at_ns = u64::from_le_bytes(bytes[40..48].try_into().unwrap());
        let expires_at_ns = u64::from_le_bytes(bytes[48..56].try_into().unwrap());
        Ok(Self {
            version,
            compression,
            segment_id,
            session_id,
            uncompressed_size,
            compressed_size,
            record_count,
            checksum,
            created_at_ns,
            expires_at_ns,
        })
    }
}

/// In-service segment object.
#[derive(Debug)]
pub struct Segment {
    pub id: SegmentId,
    pub session_id: SessionId,
    pub state: SegmentState,
    pub created_at_ns: u64,
    pub expires_at_ns: u64,
    /// Uncompressed sealed payload: sequence of [u32 id][u32 len][bytes].
    pub plain: Vec<u8>,
    pub compressed: Option<Vec<u8>>,
    pub header: Option<ColdSegmentHeader>,
    pub record_ids: Vec<MemoryId>,
    pub open: bool,
    /// Importance for eviction (min of member importance, lower = first).
    pub importance: u16,
    pub last_access_ns: u64,
}

impl Segment {
    pub fn new_open(
        id: SegmentId,
        session_id: SessionId,
        created_at_ns: u64,
        expires_at_ns: u64,
    ) -> Self {
        Self {
            id,
            session_id,
            state: SegmentState::Open,
            created_at_ns,
            expires_at_ns,
            plain: Vec::new(),
            compressed: None,
            header: None,
            record_ids: Vec::new(),
            open: true,
            importance: u16::MAX,
            last_access_ns: created_at_ns,
        }
    }

    /// Append a sealed record using checked arithmetic.
    pub fn append_record(
        &mut self,
        memory_id: MemoryId,
        payload: &[u8],
        importance: u16,
        max_segment_size: u32,
    ) -> Result<(), MemoryError> {
        if !self.open || self.state != SegmentState::Open {
            return Err(MemoryError::InvalidLifecycleTransition {
                from: "segment_not_open",
                op: crate::lifecycle::LifecycleOp::Append,
            });
        }
        // Layout: memory_id(8) + payload_len(4) + payload
        let rec_len = payload
            .len()
            .checked_add(12)
            .ok_or(MemoryError::InternalInvariantViolation("record size overflow"))?;
        let new_len = self
            .plain
            .len()
            .checked_add(rec_len)
            .ok_or(MemoryError::InternalInvariantViolation("segment size overflow"))?;
        if new_len > max_segment_size as usize {
            return Err(MemoryError::QuotaExceeded("max segment size"));
        }
        let id_bytes = memory_id.to_le_bytes();
        let len_bytes = (payload.len() as u32).to_le_bytes();
        self.plain.extend_from_slice(&id_bytes);
        self.plain.extend_from_slice(&len_bytes);
        self.plain.extend_from_slice(payload);
        self.record_ids.push(memory_id);
        self.importance = self.importance.min(importance);
        Ok(())
    }

    pub fn seal(&mut self) -> Result<(), MemoryError> {
        if self.state != SegmentState::Open {
            return Err(MemoryError::InvalidLifecycleTransition {
                from: "segment_not_open",
                op: crate::lifecycle::LifecycleOp::Seal,
            });
        }
        self.open = false;
        self.state = SegmentState::Sealed;
        Ok(())
    }

    /// Compress once. Idempotent if already compressed.
    pub fn compress_once(&mut self, max_decompress: u32) -> Result<(), MemoryError> {
        if self.state == SegmentState::Compressed
            || self.state == SegmentState::Spilled
            || self.state == SegmentState::Rehydrated
        {
            return Ok(());
        }
        if self.state != SegmentState::Sealed {
            return Err(MemoryError::InvalidLifecycleTransition {
                from: "segment_not_sealed",
                op: crate::lifecycle::LifecycleOp::SpillToCold,
            });
        }
        if self.plain.len() as u32 > max_decompress {
            return Err(MemoryError::PayloadTooLarge {
                size: self.plain.len() as u32,
                max: max_decompress,
            });
        }
        let compressed = compress_lz4(&self.plain)?;
        let checksum = crc32_ieee(&self.plain);
        let header = ColdSegmentHeader {
            version: SEGMENT_FORMAT_VERSION,
            compression: COMPRESSION_LZ4,
            segment_id: self.id,
            session_id: self.session_id,
            uncompressed_size: self.plain.len() as u32,
            compressed_size: compressed.len() as u32,
            record_count: self.record_ids.len() as u32,
            checksum,
            created_at_ns: self.created_at_ns,
            expires_at_ns: self.expires_at_ns,
        };
        self.compressed = Some(compressed);
        self.header = Some(header);
        self.state = SegmentState::Compressed;
        // Drop plain to free RAM after compression; rehydrate on demand.
        self.plain = Vec::new();
        Ok(())
    }

    /// Encode full spill blob: header || compressed payload.
    pub fn encode_spill_blob(&self) -> Result<Vec<u8>, MemoryError> {
        let header = self.header.as_ref().ok_or(MemoryError::SpillIncomplete)?;
        let compressed = self
            .compressed
            .as_ref()
            .ok_or(MemoryError::SpillIncomplete)?;
        let mut out = Vec::with_capacity(
            SEGMENT_HEADER_LEN
                .checked_add(compressed.len())
                .ok_or(MemoryError::InternalInvariantViolation("spill size"))?,
        );
        out.extend_from_slice(&header.encode());
        out.extend_from_slice(compressed);
        Ok(out)
    }

    /// Validate and rehydrate from a spill blob.
    pub fn from_spill_blob(
        blob: &[u8],
        max_decompress: u32,
    ) -> Result<(Self, Vec<u8>), MemoryError> {
        let header = ColdSegmentHeader::decode(blob)?;
        if header.uncompressed_size > max_decompress {
            return Err(MemoryError::PayloadTooLarge {
                size: header.uncompressed_size,
                max: max_decompress,
            });
        }
        let body = blob
            .get(SEGMENT_HEADER_LEN..)
            .ok_or(MemoryError::SpillIncomplete)?;
        if body.len() as u32 != header.compressed_size {
            return Err(MemoryError::SpillCorrupt);
        }
        let plain = if header.compression == COMPRESSION_LZ4 {
            decompress_lz4_checked(body, header.uncompressed_size, max_decompress)?
        } else if header.compression == crate::compression::COMPRESSION_NONE {
            if body.len() != header.uncompressed_size as usize {
                return Err(MemoryError::SpillCorrupt);
            }
            body.to_vec()
        } else {
            return Err(MemoryError::DecompressionFailure);
        };
        let actual = crc32_ieee(&plain);
        if actual != header.checksum {
            return Err(MemoryError::ChecksumMismatch);
        }
        let record_ids = parse_record_ids(&plain)?;
        if record_ids.len() as u32 != header.record_count {
            return Err(MemoryError::SpillCorrupt);
        }
        let seg = Self {
            id: header.segment_id,
            session_id: header.session_id,
            state: SegmentState::Rehydrated,
            created_at_ns: header.created_at_ns,
            expires_at_ns: header.expires_at_ns,
            plain: plain.clone(),
            compressed: Some(body.to_vec()),
            header: Some(header),
            record_ids,
            open: false,
            importance: 0,
            last_access_ns: 0,
        };
        Ok((seg, plain))
    }

    pub fn find_payload(&self, memory_id: MemoryId) -> Result<Option<Vec<u8>>, MemoryError> {
        if self.plain.is_empty() {
            return Err(MemoryError::InvalidRequest("segment not rehydrated"));
        }
        let mut off = 0usize;
        while off < self.plain.len() {
            if off
                .checked_add(12)
                .map(|n| n > self.plain.len())
                .unwrap_or(true)
            {
                return Err(MemoryError::SpillCorrupt);
            }
            let id = MemoryId::from_le_bytes(self.plain[off..off + 8].try_into().unwrap())
                .map_err(|_| MemoryError::MalformedIdentifier("record id"))?;
            let len = u32::from_le_bytes(self.plain[off + 8..off + 12].try_into().unwrap()) as usize;
            let payload_start = off
                .checked_add(12)
                .ok_or(MemoryError::InternalInvariantViolation("offset"))?;
            let payload_end = payload_start
                .checked_add(len)
                .ok_or(MemoryError::InternalInvariantViolation("offset"))?;
            if payload_end > self.plain.len() {
                return Err(MemoryError::SpillCorrupt);
            }
            if id == memory_id {
                return Ok(Some(self.plain[payload_start..payload_end].to_vec()));
            }
            off = payload_end;
        }
        Ok(None)
    }
}

fn parse_record_ids(plain: &[u8]) -> Result<Vec<MemoryId>, MemoryError> {
    let mut ids = Vec::new();
    let mut off = 0usize;
    while off < plain.len() {
        if off
            .checked_add(12)
            .map(|n| n > plain.len())
            .unwrap_or(true)
        {
            return Err(MemoryError::SpillCorrupt);
        }
        let id = MemoryId::from_le_bytes(plain[off..off + 8].try_into().unwrap())
            .map_err(|_| MemoryError::MalformedIdentifier("record id"))?;
        let len = u32::from_le_bytes(plain[off + 8..off + 12].try_into().unwrap()) as usize;
        let end = off
            .checked_add(12)
            .and_then(|s| s.checked_add(len))
            .ok_or(MemoryError::InternalInvariantViolation("offset"))?;
        if end > plain.len() {
            return Err(MemoryError::SpillCorrupt);
        }
        ids.push(id);
        off = end;
    }
    Ok(ids)
}

/// Checked layout calculation for tests / tooling.
pub fn checked_record_layout(
    payload_len: usize,
    existing_segment_len: usize,
    max_segment: usize,
) -> Result<usize, MemoryError> {
    let rec = payload_len
        .checked_add(12)
        .ok_or(MemoryError::InternalInvariantViolation("record layout"))?;
    let total = existing_segment_len
        .checked_add(rec)
        .ok_or(MemoryError::InternalInvariantViolation("segment layout"))?;
    if total > max_segment {
        return Err(MemoryError::QuotaExceeded("max segment size"));
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{MemoryId, SegmentId, SessionId};

    #[test]
    fn segment_layout_checked() {
        assert_eq!(checked_record_layout(10, 0, 100).unwrap(), 22);
        assert!(checked_record_layout(100, 0, 50).is_err());
        assert!(checked_record_layout(usize::MAX, 1, usize::MAX).is_err());
    }

    #[test]
    fn compress_and_rehydrate() {
        let mut seg = Segment::new_open(
            SegmentId::from_raw(1).unwrap(),
            SessionId::from_raw(2).unwrap(),
            100,
            9999,
        );
        let mid = MemoryId::from_raw(7).unwrap();
        seg.append_record(mid, b"payload-data", 50, 4096).unwrap();
        seg.seal().unwrap();
        seg.compress_once(4096).unwrap();
        assert!(seg.plain.is_empty());
        let blob = seg.encode_spill_blob().unwrap();
        let (restored, _) = Segment::from_spill_blob(&blob, 4096).unwrap();
        let p = restored.find_payload(mid).unwrap().unwrap();
        assert_eq!(p, b"payload-data");
    }

    #[test]
    fn checksum_mismatch() {
        let mut seg = Segment::new_open(
            SegmentId::from_raw(1).unwrap(),
            SessionId::from_raw(2).unwrap(),
            100,
            9999,
        );
        seg.append_record(MemoryId::from_raw(1).unwrap(), b"abc", 1, 4096)
            .unwrap();
        seg.seal().unwrap();
        seg.compress_once(4096).unwrap();
        let mut blob = seg.encode_spill_blob().unwrap();
        // Flip checksum bytes
        blob[36] ^= 0xFF;
        let r = Segment::from_spill_blob(&blob, 4096);
        assert!(matches!(r, Err(MemoryError::ChecksumMismatch)));
    }

    #[test]
    fn oversized_header_rejected() {
        let mut header = ColdSegmentHeader {
            version: SEGMENT_FORMAT_VERSION,
            compression: COMPRESSION_LZ4,
            segment_id: SegmentId::from_raw(1).unwrap(),
            session_id: SessionId::from_raw(1).unwrap(),
            uncompressed_size: 9_000_000,
            compressed_size: 0,
            record_count: 0,
            checksum: 0,
            created_at_ns: 0,
            expires_at_ns: 0,
        };
        let blob = header.encode().to_vec();
        // empty body with compressed_size 0
        let r = Segment::from_spill_blob(&blob, 1024);
        assert!(matches!(r, Err(MemoryError::PayloadTooLarge { .. })));
        // silence unused mut
        header.compressed_size = 0;
        let _ = header;
    }
}
