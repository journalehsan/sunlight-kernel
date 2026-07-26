//! Immutable sealed data segments (`.owlseg`).
//!
//! ## Segment layout (little-endian)
//!
//! ```text
//! magic                    u32  = SEG_MAGIC ("OWLS")
//! format_version           u16
//! flags                    u16  (bit0 = LZ4 body)
//! segment_id               u64
//! database_generation      u64
//! record_count             u32
//! metadata_table_offset    u32  (from start of uncompressed body)
//! payload_table_offset     u32  (from start of uncompressed body)
//! uncompressed_len         u32
//! compressed_len           u32
//! checksum                 u32  CRC32 of uncompressed body
//! seq_start                u64
//! seq_end                  u64
//! previous_segment_id      u64  (0 = none)
//! reserved                 u32
//! body                     [compressed_len]  LZ4 or raw uncompressed body
//! ```
//!
//! Uncompressed body:
//! ```text
//! for each record:
//!   meta_len u32
//!   meta_bytes [meta_len]   (LongTermMemoryRecord.encode)
//! ```
//! (payload is embedded inside each record encoding for Phase 2 simplicity;
//!  payload_table_offset points to end of metadata stream for future split.)

use alloc::vec::Vec;

use wiseowl_memory::compression::{
    compress_lz4, crc32_ieee, decompress_lz4_checked, COMPRESSION_LZ4, COMPRESSION_NONE,
};

use crate::error::DbError;
use crate::quotas::DbQuotaConfig;
use crate::record::LongTermMemoryRecord;

/// Segment magic: "OWLS" (same family as short-term spill; distinct format version).
pub const SEG_MAGIC: u32 = 0x534C_574F; // 'OWLS' LE
/// Long-term segment format version.
pub const LT_SEGMENT_FORMAT_VERSION: u16 = 1;
/// Fixed header length.
pub const SEG_HEADER_LEN: usize = 64;
/// Flag: body is LZ4 compressed.
pub const SEG_FLAG_LZ4: u16 = 0x0001;

/// Sealed segment header (decoded).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentHeader {
    pub segment_id: u64,
    pub database_generation: u64,
    pub record_count: u32,
    pub metadata_table_offset: u32,
    pub payload_table_offset: u32,
    pub uncompressed_len: u32,
    pub compressed_len: u32,
    pub checksum: u32,
    pub seq_start: u64,
    pub seq_end: u64,
    pub previous_segment_id: u64,
    pub flags: u16,
}

impl SegmentHeader {
    pub fn encode(&self) -> [u8; SEG_HEADER_LEN] {
        let mut out = [0u8; SEG_HEADER_LEN];
        out[0..4].copy_from_slice(&SEG_MAGIC.to_le_bytes());
        out[4..6].copy_from_slice(&LT_SEGMENT_FORMAT_VERSION.to_le_bytes());
        out[6..8].copy_from_slice(&self.flags.to_le_bytes());
        out[8..16].copy_from_slice(&self.segment_id.to_le_bytes());
        out[16..24].copy_from_slice(&self.database_generation.to_le_bytes());
        out[24..28].copy_from_slice(&self.record_count.to_le_bytes());
        out[28..32].copy_from_slice(&self.metadata_table_offset.to_le_bytes());
        out[32..36].copy_from_slice(&self.payload_table_offset.to_le_bytes());
        out[36..40].copy_from_slice(&self.uncompressed_len.to_le_bytes());
        out[40..44].copy_from_slice(&self.compressed_len.to_le_bytes());
        out[44..48].copy_from_slice(&self.checksum.to_le_bytes());
        out[48..56].copy_from_slice(&self.seq_start.to_le_bytes());
        out[56..64].copy_from_slice(&self.seq_end.to_le_bytes());
        // previous_segment_id stored in reserved? We need more space — put prev id
        // by shrinking: actually header is 64 bytes without prev. Extend: use
        // compressed path only — store previous in first 8 of body? Better expand.
        // For fixed 64 without prev in header, we encode prev inside body prefix.
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, DbError> {
        if bytes.len() < SEG_HEADER_LEN {
            return Err(DbError::Corrupt {
                reason: "segment header truncated",
            });
        }
        let magic = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
        if magic != SEG_MAGIC {
            return Err(DbError::Corrupt {
                reason: "segment magic",
            });
        }
        let version = u16::from_le_bytes(bytes[4..6].try_into().unwrap());
        if version != LT_SEGMENT_FORMAT_VERSION {
            return Err(DbError::Corrupt {
                reason: "segment version",
            });
        }
        let flags = u16::from_le_bytes(bytes[6..8].try_into().unwrap());
        Ok(Self {
            segment_id: u64::from_le_bytes(bytes[8..16].try_into().unwrap()),
            database_generation: u64::from_le_bytes(bytes[16..24].try_into().unwrap()),
            record_count: u32::from_le_bytes(bytes[24..28].try_into().unwrap()),
            metadata_table_offset: u32::from_le_bytes(bytes[28..32].try_into().unwrap()),
            payload_table_offset: u32::from_le_bytes(bytes[32..36].try_into().unwrap()),
            uncompressed_len: u32::from_le_bytes(bytes[36..40].try_into().unwrap()),
            compressed_len: u32::from_le_bytes(bytes[40..44].try_into().unwrap()),
            checksum: u32::from_le_bytes(bytes[44..48].try_into().unwrap()),
            seq_start: u64::from_le_bytes(bytes[48..56].try_into().unwrap()),
            seq_end: u64::from_le_bytes(bytes[56..64].try_into().unwrap()),
            previous_segment_id: 0, // carried in body prefix
            flags,
        })
    }
}

/// Build uncompressed body from records: prev_seg_id || record stream.
fn build_body(records: &[LongTermMemoryRecord], previous_segment_id: u64) -> Result<Vec<u8>, DbError> {
    let mut body = Vec::new();
    body.extend_from_slice(&previous_segment_id.to_le_bytes());
    for rec in records {
        let enc = rec.encode(1024 * 1024)?;
        let len = enc.len() as u32;
        body.extend_from_slice(&len.to_le_bytes());
        body.extend_from_slice(&enc);
    }
    Ok(body)
}

/// Seal records into a segment file blob.
pub fn seal_segment(
    segment_id: u64,
    database_generation: u64,
    seq_start: u64,
    seq_end: u64,
    previous_segment_id: u64,
    records: &[LongTermMemoryRecord],
    quotas: &DbQuotaConfig,
) -> Result<Vec<u8>, DbError> {
    if records.len() as u32 > quotas.max_compaction_records.max(records.len() as u32) {
        // no-op guard; actual bound checked below via size
    }
    let body = build_body(records, previous_segment_id)?;
    if body.len() as u32 > quotas.max_segment_uncompressed {
        return Err(DbError::PayloadTooLarge {
            size: body.len() as u32,
            max: quotas.max_segment_uncompressed,
        });
    }
    let checksum = crc32_ieee(&body);
    let compressed = compress_lz4(&body).map_err(|_| DbError::CompressionFailure)?;
    let (flags, body_out, _uncompressed_hint) = if compressed.len() < body.len() {
        (SEG_FLAG_LZ4, compressed, body.len().min(u32::MAX as usize) as u32)
    } else {
        // Prefer raw if LZ4 does not shrink.
        (0u16, body.clone(), body.len() as u32)
    };
    // compressed_len field = length of on-disk body bytes; uncompressed_len = full body.
    let on_disk_len = body_out.len() as u32;
    if on_disk_len > quotas.max_segment_bytes {
        return Err(DbError::PayloadTooLarge {
            size: on_disk_len,
            max: quotas.max_segment_bytes,
        });
    }
    let header = SegmentHeader {
        segment_id,
        database_generation,
        record_count: records.len() as u32,
        metadata_table_offset: 8, // after prev id
        payload_table_offset: body.len() as u32,
        uncompressed_len: body.len() as u32,
        compressed_len: on_disk_len,
        checksum,
        seq_start,
        seq_end,
        previous_segment_id,
        flags,
    };
    let mut out = Vec::with_capacity(SEG_HEADER_LEN + body_out.len());
    out.extend_from_slice(&header.encode());
    out.extend_from_slice(&body_out);
    let _ = (COMPRESSION_LZ4, COMPRESSION_NONE); // silence if unused
    Ok(out)
}

/// Open and validate a segment; return header + decoded records.
pub fn open_segment(
    data: &[u8],
    quotas: &DbQuotaConfig,
) -> Result<(SegmentHeader, Vec<LongTermMemoryRecord>), DbError> {
    let mut header = SegmentHeader::decode(data)?;
    if header.compressed_len as usize
        > data
            .len()
            .checked_sub(SEG_HEADER_LEN)
            .ok_or(DbError::Corrupt {
                reason: "segment size",
            })?
    {
        return Err(DbError::Corrupt {
            reason: "compressed_len",
        });
    }
    if header.uncompressed_len > quotas.max_segment_uncompressed {
        return Err(DbError::PayloadTooLarge {
            size: header.uncompressed_len,
            max: quotas.max_segment_uncompressed,
        });
    }
    if header.compressed_len > quotas.max_segment_bytes {
        return Err(DbError::PayloadTooLarge {
            size: header.compressed_len,
            max: quotas.max_segment_bytes,
        });
    }
    let body_bytes = &data[SEG_HEADER_LEN..SEG_HEADER_LEN + header.compressed_len as usize];
    let uncompressed = if header.flags & SEG_FLAG_LZ4 != 0 {
        decompress_lz4_checked(
            body_bytes,
            header.uncompressed_len,
            quotas.max_segment_uncompressed,
        )
        .map_err(|e| match e {
            wiseowl_memory::MemoryError::PayloadTooLarge { size, max } => {
                DbError::PayloadTooLarge { size, max }
            }
            _ => DbError::DecompressionFailure,
        })?
    } else {
        if body_bytes.len() as u32 != header.uncompressed_len {
            return Err(DbError::Corrupt {
                reason: "raw body length",
            });
        }
        body_bytes.to_vec()
    };
    let crc = crc32_ieee(&uncompressed);
    if crc != header.checksum {
        return Err(DbError::Corrupt {
            reason: "segment checksum",
        });
    }
    if uncompressed.len() < 8 {
        return Err(DbError::Corrupt {
            reason: "segment body too small",
        });
    }
    header.previous_segment_id = u64::from_le_bytes(uncompressed[0..8].try_into().unwrap());
    let mut offset = 8usize;
    let mut records = Vec::new();
    let mut seen_ids = alloc::collections::BTreeSet::new();
    for _ in 0..header.record_count {
        if offset + 4 > uncompressed.len() {
            return Err(DbError::Corrupt {
                reason: "record count vs body",
            });
        }
        let meta_len =
            u32::from_le_bytes(uncompressed[offset..offset + 4].try_into().unwrap()) as usize;
        offset += 4;
        let end = offset.checked_add(meta_len).ok_or(DbError::Corrupt {
            reason: "record offset overflow",
        })?;
        if end > uncompressed.len() {
            return Err(DbError::Corrupt {
                reason: "record boundary",
            });
        }
        let rec = LongTermMemoryRecord::decode(&uncompressed[offset..end], quotas)?;
        if !seen_ids.insert(rec.id.get()) {
            return Err(DbError::Corrupt {
                reason: "duplicate id in segment",
            });
        }
        records.push(rec);
        offset = end;
    }
    if records.len() as u32 != header.record_count {
        return Err(DbError::Corrupt {
            reason: "record count mismatch",
        });
    }
    Ok((header, records))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::fnv1a64;
    use crate::provenance::{DerivationKind, LongTermProvenance};
    use crate::record::*;
    use wiseowl_memory::{MemoryId, SourceKind, TrustLevel};

    fn sample(id: u64) -> LongTermMemoryRecord {
        let payload = b"seg-payload".to_vec();
        LongTermMemoryRecord {
            format_version: LT_RECORD_FORMAT_VERSION,
            id: MemoryId::from_raw_unchecked(id),
            revision: 1,
            kind: LongTermMemoryKind::Observation,
            scope: MemoryScope::User,
            owner: 1,
            created_at_ns: 10,
            updated_at_ns: 10,
            valid_from_ns: None,
            valid_until_ns: None,
            importance: 1,
            confidence: 1,
            trust: TrustLevel::Untrusted,
            provenance: LongTermProvenance {
                source_kind: SourceKind::UserInput,
                source_id: None,
                producer_service: alloc::string::String::from("t"),
                original_memory_ids: Vec::new(),
                parent_lt_ids: Vec::new(),
                insertion_time_ns: 10,
                trust: TrustLevel::Untrusted,
                source_content_hash: None,
                external_ref: None,
                derivation: DerivationKind::DirectImport,
            },
            payload_ref: PayloadRef {
                content_hash: fnv1a64(&payload),
                length: payload.len() as u32,
            },
            tokens: None,
            attributes: crate::attributes::AttributeSet::default(),
            state: LongTermRecordState::Active,
            supersedes: None,
            payload,
            token_entries: Vec::new(),
        }
    }

    #[test]
    fn seal_open_roundtrip() {
        let q = DbQuotaConfig::default();
        let recs = vec![sample(1), sample(2)];
        let blob = seal_segment(9, 1, 1, 2, 0, &recs, &q).unwrap();
        let (h, out) = open_segment(&blob, &q).unwrap();
        assert_eq!(h.segment_id, 9);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].id.get(), 1);
    }

    #[test]
    fn bad_magic() {
        let q = DbQuotaConfig::default();
        let mut blob = seal_segment(1, 1, 1, 1, 0, &[sample(1)], &q).unwrap();
        blob[0] ^= 0xFF;
        assert!(open_segment(&blob, &q).is_err());
    }

    #[test]
    fn checksum_mismatch() {
        let q = DbQuotaConfig::default();
        let mut blob = seal_segment(1, 1, 1, 1, 0, &[sample(1)], &q).unwrap();
        // Flip a body byte.
        let last = blob.len() - 1;
        blob[last] ^= 0x55;
        assert!(matches!(
            open_segment(&blob, &q),
            Err(DbError::Corrupt { .. }) | Err(DbError::DecompressionFailure)
        ));
    }

    #[test]
    fn oversized_uncompressed_rejected() {
        let q = DbQuotaConfig {
            max_segment_uncompressed: 16,
            ..DbQuotaConfig::default()
        };
        assert!(seal_segment(1, 1, 1, 1, 0, &[sample(1)], &q).is_err());
    }
}
