//! Sealed segment design for cold short-term memory.
//!
//! Lifecycle: Open -> Sealed -> Compressed -> Spilled -> Rehydrated -> Expired/Deleted
//!
//! Only one owner may mutate an open segment. After sealing, records are
//! immutable and compression happens at most once.
//!
//! # Spill body format (Phase 1.1 / segment version 2)
//!
//! Each record in the uncompressed body carries a versioned metadata header so
//! restart recovery does not depend on RAM-only entry tables. Segment format
//! version 1 (body was only `id|len|payload`) is **rejected** on recovery
//! (quarantined). Compatibility: neither forward nor backward.

extern crate alloc;

use alloc::vec::Vec;

use crate::compression::{compress_lz4, crc32_ieee, decompress_lz4_checked, COMPRESSION_LZ4};
use crate::entry::{MemoryEntryHeader, MemoryState, TokenStreamRef, ENTRY_HEADER_VERSION};
use crate::error::MemoryError;
use crate::ids::{MemoryId, SegmentId, SessionId, TokenStreamId};
use crate::kinds::{MemoryClass, MemoryKind, SourceKind, TrustLevel};
use crate::provenance::{Provenance, MAX_PRODUCER_LEN, MAX_PROVENANCE_PARENTS};

/// On-disk / wire format version for cold segments (v2 = full metadata records).
pub const SEGMENT_FORMAT_VERSION: u16 = 2;
/// Previous format (id|len|payload only). Rejected safely on recovery.
pub const SEGMENT_FORMAT_VERSION_V1: u16 = 1;
/// Per-record body format version embedded in each record.
pub const RECORD_FORMAT_VERSION: u16 = 2;
/// Fixed prefix length before variable provenance/token/payload fields.
/// version(2)+id(8)+session(8)+class/kind/state/flags(4)+created(8)+last(8)+exp(8)
/// +importance/confidence(4) + prov header(4)+prov_created(8) = 62
pub const RECORD_FIXED_PREFIX_LEN: usize = 62;

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
        // Accept only current format. V1 is intentionally not migrated.
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

    /// Append a sealed record with full recovery metadata (Phase 1.1).
    pub fn append_record_full(
        &mut self,
        header: &MemoryEntryHeader,
        state: MemoryState,
        payload: &[u8],
        max_segment_size: u32,
    ) -> Result<(), MemoryError> {
        if !self.open || self.state != SegmentState::Open {
            return Err(MemoryError::InvalidLifecycleTransition {
                from: "segment_not_open",
                op: crate::lifecycle::LifecycleOp::Append,
            });
        }
        let encoded = encode_record_v2(header, state, payload)?;
        let new_len = self
            .plain
            .len()
            .checked_add(encoded.len())
            .ok_or(MemoryError::InternalInvariantViolation("segment size overflow"))?;
        if new_len > max_segment_size as usize {
            return Err(MemoryError::QuotaExceeded("max segment size"));
        }
        self.plain.extend_from_slice(&encoded);
        self.record_ids.push(header.id);
        self.importance = self.importance.min(header.importance);
        Ok(())
    }

    /// Convenience for tests: minimal metadata around a payload.
    pub fn append_record(
        &mut self,
        memory_id: MemoryId,
        payload: &[u8],
        importance: u16,
        max_segment_size: u32,
    ) -> Result<(), MemoryError> {
        let header = MemoryEntryHeader {
            version: ENTRY_HEADER_VERSION,
            id: memory_id,
            session_id: self.session_id,
            class: MemoryClass::Cold,
            kind: MemoryKind::Diagnostic,
            created_at_ns: self.created_at_ns,
            last_access_ns: self.created_at_ns,
            expires_at_ns: if self.expires_at_ns == u64::MAX {
                None
            } else {
                Some(self.expires_at_ns)
            },
            importance,
            confidence: 0,
            payload_len: payload.len() as u32,
            token_stream: None,
            provenance: Provenance::new(
                SourceKind::LocalService,
                None,
                self.created_at_ns,
                "wiseowl-memoryd",
                TrustLevel::Trusted,
            ),
        };
        self.append_record_full(&header, MemoryState::Cold, payload, max_segment_size)
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
        for rec in iter_records_v2(&self.plain)? {
            if rec.header.id == memory_id {
                return Ok(Some(rec.payload));
            }
        }
        Ok(None)
    }

    /// Iterate recovered records with full metadata (for restart reconstruction).
    pub fn recovered_records(&self) -> Result<Vec<RecoveredRecord>, MemoryError> {
        if self.plain.is_empty() {
            return Err(MemoryError::InvalidRequest("segment not rehydrated"));
        }
        iter_records_v2(&self.plain)
    }
}

/// A fully reconstructed cold record from spill.
#[derive(Debug, Clone)]
pub struct RecoveredRecord {
    pub header: MemoryEntryHeader,
    pub state: MemoryState,
    pub payload: Vec<u8>,
    pub payload_checksum: u32,
}

/// Encode a versioned record for the segment body.
pub fn encode_record_v2(
    header: &MemoryEntryHeader,
    state: MemoryState,
    payload: &[u8],
) -> Result<Vec<u8>, MemoryError> {
    if payload.len() as u32 != header.payload_len && header.payload_len != 0 {
        // Prefer actual payload length; header.payload_len may be stale after shrink.
    }
    let payload_len = payload.len() as u32;
    let checksum = crc32_ieee(payload);
    let mut flags: u8 = 0;
    if header.token_stream.is_some() {
        flags |= 0x01;
    }

    let producer = header.provenance.producer_service.as_str().as_bytes();
    let producer_len = producer.len().min(MAX_PRODUCER_LEN) as u8;
    let parent_count = header.provenance.parent_count().min(MAX_PROVENANCE_PARENTS);

    let size = RECORD_FIXED_PREFIX_LEN
        .checked_add(parent_count * 8)
        .and_then(|s| s.checked_add(1 + producer_len as usize))
        .and_then(|s| {
            if flags & 0x01 != 0 {
                s.checked_add(8 + 4 + 4 + 4)
            } else {
                Some(s)
            }
        })
        .and_then(|s| s.checked_add(4 + 4))
        .and_then(|s| s.checked_add(payload.len()))
        .ok_or(MemoryError::InternalInvariantViolation("record size overflow"))?;

    let mut out = Vec::with_capacity(size);
    out.extend_from_slice(&RECORD_FORMAT_VERSION.to_le_bytes());
    out.extend_from_slice(&header.id.to_le_bytes());
    out.extend_from_slice(&header.session_id.to_le_bytes());
    out.push(header.class.as_u8());
    out.push(header.kind.as_u8());
    out.push(state as u8);
    out.push(flags);
    out.extend_from_slice(&header.created_at_ns.to_le_bytes());
    out.extend_from_slice(&header.last_access_ns.to_le_bytes());
    let exp = header.expires_at_ns.unwrap_or(0);
    out.extend_from_slice(&exp.to_le_bytes());
    out.extend_from_slice(&header.importance.to_le_bytes());
    out.extend_from_slice(&header.confidence.to_le_bytes());
    out.push(header.provenance.source_kind.as_u8());
    out.push(header.provenance.trust.as_u8());
    out.push(parent_count as u8);
    out.push(0); // pad
    out.extend_from_slice(&header.provenance.created_at_ns.to_le_bytes());
    for p in header.provenance.parents.iter().take(parent_count) {
        out.extend_from_slice(&p.to_le_bytes());
    }
    out.push(producer_len);
    out.extend_from_slice(&producer[..producer_len as usize]);
    if let Some(ts) = header.token_stream {
        out.extend_from_slice(&ts.id.to_le_bytes());
        out.extend_from_slice(&ts.tokenizer_id.to_le_bytes());
        out.extend_from_slice(&ts.tokenizer_version.to_le_bytes());
        out.extend_from_slice(&ts.token_count.to_le_bytes());
    }
    out.extend_from_slice(&payload_len.to_le_bytes());
    out.extend_from_slice(&checksum.to_le_bytes());
    out.extend_from_slice(payload);
    debug_assert_eq!(out.len(), size);
    let _ = size;
    Ok(out)
}

/// Parse all v2 records from an uncompressed segment body.
pub fn parse_records_v2(plain: &[u8]) -> Result<Vec<RecoveredRecord>, MemoryError> {
    iter_records_v2(plain)
}

fn iter_records_v2(plain: &[u8]) -> Result<Vec<RecoveredRecord>, MemoryError> {
    let mut out = Vec::new();
    let mut off = 0usize;
    while off < plain.len() {
        let (rec, next) = decode_record_v2_at(plain, off)?;
        out.push(rec);
        off = next;
    }
    Ok(out)
}

fn decode_record_v2_at(plain: &[u8], off: usize) -> Result<(RecoveredRecord, usize), MemoryError> {
    let need = |n: usize| -> Result<(), MemoryError> {
        if off.checked_add(n).map(|e| e > plain.len()).unwrap_or(true) {
            Err(MemoryError::SpillCorrupt)
        } else {
            Ok(())
        }
    };
    need(RECORD_FIXED_PREFIX_LEN)?;
    let version = u16::from_le_bytes(plain[off..off + 2].try_into().unwrap());
    if version != RECORD_FORMAT_VERSION {
        return Err(MemoryError::UnsupportedProtocolVersion {
            got: version,
            want: RECORD_FORMAT_VERSION,
        });
    }
    let mut p = off + 2;
    let memory_id = MemoryId::from_le_bytes(plain[p..p + 8].try_into().unwrap())
        .map_err(|_| MemoryError::MalformedIdentifier("record id"))?;
    p += 8;
    let session_id = SessionId::from_le_bytes(plain[p..p + 8].try_into().unwrap())
        .map_err(|_| MemoryError::MalformedIdentifier("session id"))?;
    p += 8;
    let class = MemoryClass::from_u8(plain[p]).ok_or(MemoryError::SpillCorrupt)?;
    let kind = MemoryKind::from_u8(plain[p + 1]).ok_or(MemoryError::SpillCorrupt)?;
    let state = MemoryState::from_u8(plain[p + 2]).ok_or(MemoryError::SpillCorrupt)?;
    let flags = plain[p + 3];
    p += 4;
    let created_at_ns = u64::from_le_bytes(plain[p..p + 8].try_into().unwrap());
    p += 8;
    let last_access_ns = u64::from_le_bytes(plain[p..p + 8].try_into().unwrap());
    p += 8;
    let expires_raw = u64::from_le_bytes(plain[p..p + 8].try_into().unwrap());
    p += 8;
    let importance = u16::from_le_bytes(plain[p..p + 2].try_into().unwrap());
    p += 2;
    let confidence = u16::from_le_bytes(plain[p..p + 2].try_into().unwrap());
    p += 2;
    let source_kind = SourceKind::from_u8(plain[p]).ok_or(MemoryError::SpillCorrupt)?;
    let trust = TrustLevel::from_u8(plain[p + 1]).ok_or(MemoryError::SpillCorrupt)?;
    let parent_count = plain[p + 2] as usize;
    p += 4; // includes pad
    if parent_count > MAX_PROVENANCE_PARENTS {
        return Err(MemoryError::SpillCorrupt);
    }
    need(p - off + 8 + parent_count * 8 + 1)?;
    let prov_created = u64::from_le_bytes(plain[p..p + 8].try_into().unwrap());
    p += 8;
    let mut provenance = Provenance::new(source_kind, None, prov_created, "", trust);
    for _ in 0..parent_count {
        let pid = MemoryId::from_le_bytes(plain[p..p + 8].try_into().unwrap())
            .map_err(|_| MemoryError::MalformedIdentifier("parent id"))?;
        let _ = provenance.push_parent(pid);
        p += 8;
    }
    let producer_len = plain[p] as usize;
    p += 1;
    if producer_len > MAX_PRODUCER_LEN {
        return Err(MemoryError::SpillCorrupt);
    }
    need(p - off + producer_len)?;
    let producer = core::str::from_utf8(&plain[p..p + producer_len]).unwrap_or("");
    provenance.producer_service = crate::provenance::heapless_string::HeaplessString::from_str(producer);
    p += producer_len;

    let token_stream = if flags & 0x01 != 0 {
        need(p - off + 20)?;
        let id = TokenStreamId::from_le_bytes(plain[p..p + 8].try_into().unwrap())
            .map_err(|_| MemoryError::MalformedIdentifier("token stream"))?;
        p += 8;
        let tokenizer_id = u32::from_le_bytes(plain[p..p + 4].try_into().unwrap());
        p += 4;
        let tokenizer_version = u32::from_le_bytes(plain[p..p + 4].try_into().unwrap());
        p += 4;
        let token_count = u32::from_le_bytes(plain[p..p + 4].try_into().unwrap());
        p += 4;
        Some(TokenStreamRef {
            id,
            tokenizer_id,
            tokenizer_version,
            token_count,
        })
    } else {
        None
    };

    need(p - off + 8)?;
    let payload_len = u32::from_le_bytes(plain[p..p + 4].try_into().unwrap()) as usize;
    p += 4;
    let payload_checksum = u32::from_le_bytes(plain[p..p + 4].try_into().unwrap());
    p += 4;
    need(p - off + payload_len)?;
    let payload = plain[p..p + payload_len].to_vec();
    p += payload_len;
    if crc32_ieee(&payload) != payload_checksum {
        return Err(MemoryError::ChecksumMismatch);
    }

    let header = MemoryEntryHeader {
        version: ENTRY_HEADER_VERSION,
        id: memory_id,
        session_id,
        class,
        kind,
        created_at_ns,
        last_access_ns,
        expires_at_ns: if expires_raw == 0 {
            None
        } else {
            Some(expires_raw)
        },
        importance,
        confidence,
        payload_len: payload_len as u32,
        token_stream,
        provenance,
    };
    Ok((
        RecoveredRecord {
            header,
            state,
            payload,
            payload_checksum,
        },
        p,
    ))
}

fn parse_record_ids(plain: &[u8]) -> Result<Vec<MemoryId>, MemoryError> {
    Ok(iter_records_v2(plain)?
        .into_iter()
        .map(|r| r.header.id)
        .collect())
}

/// Checked layout calculation for tests / tooling (approximate fixed overhead).
pub fn checked_record_layout(
    payload_len: usize,
    existing_segment_len: usize,
    max_segment: usize,
) -> Result<usize, MemoryError> {
    // Minimal record: fixed prefix + producer_len(1) + payload_len(4)+checksum(4)+payload
    let rec = payload_len
        .checked_add(RECORD_FIXED_PREFIX_LEN + 1 + 8)
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
        // v2 records have larger fixed overhead than the old 12-byte header.
        let n = checked_record_layout(10, 0, 256).unwrap();
        assert!(n > 10);
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
