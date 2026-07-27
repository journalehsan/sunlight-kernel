//! Native SunlightOS IPC envelope for wiseowl-memorydb (Phase 2).
//!
//! Endpoint: `wiseowl.memorydb.v1`
//!
//! Large payloads use validated SHM; small requests may use framed SHM pages
//! under [`INLINE_PAYLOAD_THRESHOLD`].

use crate::error::DbError;
use crate::query::{MemoryQuery, QueryCursor, QueryOrder, QueryResult};
use crate::record::MemoryScope;
use crate::tokens::{TokenMatchMode, TokenQuery};
use alloc::vec;
use alloc::vec::Vec;
use wiseowl_memory::MemoryId;

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
    ReconcileImport = 0x4D15,
    /// Phase 3.875: bounded generation census.
    GenerationCensus = 0x4D16,
    /// Phase 3.875: verify generation invariants.
    VerifyGenerations = 0x4D17,
    TestArmShmCrash = 0x4DF0,
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
            0x4D15 => Some(Self::ReconcileImport),
            0x4D16 => Some(Self::GenerationCensus),
            0x4D17 => Some(Self::VerifyGenerations),
            0x4DF0 => Some(Self::TestArmShmCrash),
            0x4D80 => Some(Self::Reply),
            0x4DFF => Some(Self::Error),
            _ => None,
        }
    }
}

pub const NATIVE_QUERY_FORMAT_VERSION: u16 = 1;
pub const MAX_NATIVE_QUERY_TOKENS: usize = 128;
const QUERY_MAGIC: u32 = 0x3159_5251; // "QRY1"
const RESULT_MAGIC: u32 = 0x3153_5251; // "QRS1"
const QUERY_HEADER_LEN: usize = 68;
const RESULT_HEADER_LEN: usize = 48;

/// Encode the lexical subset of the authoritative typed query model.
pub fn encode_native_query(q: &MemoryQuery) -> Result<Vec<u8>, DbError> {
    let tq = q
        .token_match
        .as_ref()
        .ok_or(DbError::InvalidRequest("native lexical query required"))?;
    if tq.token_ids.is_empty() || tq.token_ids.len() > MAX_NATIVE_QUERY_TOKENS {
        return Err(DbError::QuotaExceeded("native query tokens"));
    }
    if q.limit == 0 || q.limit > 64 {
        return Err(DbError::QuotaExceeded("native query results"));
    }
    let mut out = vec![0u8; QUERY_HEADER_LEN + tq.token_ids.len() * 8];
    out[0..4].copy_from_slice(&QUERY_MAGIC.to_le_bytes());
    out[4..6].copy_from_slice(&NATIVE_QUERY_FORMAT_VERSION.to_le_bytes());
    out[6..8].copy_from_slice(&(tq.token_ids.len() as u16).to_le_bytes());
    out[8..12].copy_from_slice(&tq.tokenizer_id.to_le_bytes());
    out[12..16].copy_from_slice(&tq.tokenizer_version.to_le_bytes());
    let (mode, minimum) = match tq.mode {
        TokenMatchMode::Any => (1, 0),
        TokenMatchMode::All => (2, 0),
        TokenMatchMode::MinimumCount(n) if n != 0 && n as usize <= tq.token_ids.len() => (3, n),
        TokenMatchMode::MinimumCount(_) => return Err(DbError::InvalidValue("token minimum")),
    };
    out[16] = mode;
    out[17] = q.scope.map(|s| s.as_u8()).unwrap_or(0);
    let mut flags = 0u8;
    if q.owner.is_some() {
        flags |= 1;
    }
    if q.cursor.is_some() {
        flags |= 2;
    }
    out[18] = flags;
    out[20..22].copy_from_slice(&minimum.to_le_bytes());
    out[24..28].copy_from_slice(&q.limit.to_le_bytes());
    out[28..36].copy_from_slice(&q.owner.unwrap_or(0).to_le_bytes());
    if let Some(cursor) = &q.cursor {
        out[36..68].copy_from_slice(&cursor.encode());
    }
    for (i, token) in tq.token_ids.iter().enumerate() {
        let start = QUERY_HEADER_LEN + i * 8;
        out[start..start + 8].copy_from_slice(&token.to_le_bytes());
    }
    Ok(out)
}

pub fn decode_native_query(data: &[u8]) -> Result<MemoryQuery, DbError> {
    if data.len() < QUERY_HEADER_LEN
        || u32::from_le_bytes(data[0..4].try_into().unwrap()) != QUERY_MAGIC
        || u16::from_le_bytes(data[4..6].try_into().unwrap()) != NATIVE_QUERY_FORMAT_VERSION
    {
        return Err(DbError::InvalidRequest("native query header"));
    }
    let count = u16::from_le_bytes(data[6..8].try_into().unwrap()) as usize;
    let expected = QUERY_HEADER_LEN
        .checked_add(count.checked_mul(8).ok_or(DbError::InvalidRequest("query overflow"))?)
        .ok_or(DbError::InvalidRequest("query overflow"))?;
    if count == 0 || count > MAX_NATIVE_QUERY_TOKENS || data.len() != expected {
        return Err(DbError::InvalidRequest("native query length"));
    }
    let minimum = u16::from_le_bytes(data[20..22].try_into().unwrap());
    let mode = match data[16] {
        1 => TokenMatchMode::Any,
        2 => TokenMatchMode::All,
        3 if minimum != 0 && minimum as usize <= count => TokenMatchMode::MinimumCount(minimum),
        _ => return Err(DbError::InvalidValue("native query match mode")),
    };
    let scope = match data[17] {
        0 => None,
        v => Some(MemoryScope::from_u8(v).ok_or(DbError::InvalidValue("query scope"))?),
    };
    let flags = data[18];
    if flags & !3 != 0 || data[19] != 0 || data[22] != 0 || data[23] != 0 {
        return Err(DbError::InvalidRequest("native query flags"));
    }
    let limit = u32::from_le_bytes(data[24..28].try_into().unwrap());
    if limit == 0 || limit > 64 {
        return Err(DbError::QuotaExceeded("native query results"));
    }
    let owner_raw = u64::from_le_bytes(data[28..36].try_into().unwrap());
    let owner = if flags & 1 != 0 { Some(owner_raw) } else { None };
    let cursor = if flags & 2 != 0 {
        Some(QueryCursor::decode(&data[36..68])?)
    } else {
        None
    };
    let mut token_ids = Vec::with_capacity(count);
    for chunk in data[QUERY_HEADER_LEN..].chunks_exact(8) {
        token_ids.push(u64::from_le_bytes(chunk.try_into().unwrap()));
    }
    Ok(MemoryQuery {
        scope,
        owner,
        token_match: Some(TokenQuery {
            tokenizer_id: u32::from_le_bytes(data[8..12].try_into().unwrap()),
            tokenizer_version: u32::from_le_bytes(data[12..16].try_into().unwrap()),
            token_ids,
            mode,
        }),
        order: QueryOrder::TokenRelevanceDesc,
        limit,
        cursor,
        ..MemoryQuery::default()
    })
}

pub fn encode_native_query_result(result: &QueryResult) -> Result<Vec<u8>, DbError> {
    if result.ids.len() > 64 {
        return Err(DbError::QuotaExceeded("native query results"));
    }
    let mut out = vec![0u8; RESULT_HEADER_LEN + result.ids.len() * 8];
    out[0..4].copy_from_slice(&RESULT_MAGIC.to_le_bytes());
    out[4..6].copy_from_slice(&NATIVE_QUERY_FORMAT_VERSION.to_le_bytes());
    let mut flags = if result.degraded { 1u16 } else { 0 };
    if result.next_cursor.is_some() {
        flags |= 2;
    }
    out[6..8].copy_from_slice(&flags.to_le_bytes());
    out[8..10].copy_from_slice(&(result.ids.len() as u16).to_le_bytes());
    out[12..16].copy_from_slice(&result.total_scanned.to_le_bytes());
    if let Some(cursor) = &result.next_cursor {
        out[16..48].copy_from_slice(&cursor.encode());
    }
    for (i, id) in result.ids.iter().enumerate() {
        let start = RESULT_HEADER_LEN + i * 8;
        out[start..start + 8].copy_from_slice(&id.get().to_le_bytes());
    }
    Ok(out)
}

pub fn decode_native_query_result(data: &[u8]) -> Result<QueryResult, DbError> {
    if data.len() < RESULT_HEADER_LEN
        || u32::from_le_bytes(data[0..4].try_into().unwrap()) != RESULT_MAGIC
        || u16::from_le_bytes(data[4..6].try_into().unwrap()) != NATIVE_QUERY_FORMAT_VERSION
    {
        return Err(DbError::InvalidRequest("native query result header"));
    }
    let flags = u16::from_le_bytes(data[6..8].try_into().unwrap());
    if flags & !3 != 0 {
        return Err(DbError::InvalidRequest("native query result flags"));
    }
    let count = u16::from_le_bytes(data[8..10].try_into().unwrap()) as usize;
    let expected = RESULT_HEADER_LEN
        .checked_add(count.checked_mul(8).ok_or(DbError::InvalidRequest("result overflow"))?)
        .ok_or(DbError::InvalidRequest("result overflow"))?;
    if count > 64 || data.len() != expected {
        return Err(DbError::InvalidRequest("native query result length"));
    }
    let mut ids = Vec::with_capacity(count);
    for chunk in data[RESULT_HEADER_LEN..].chunks_exact(8) {
        ids.push(MemoryId::from_raw(u64::from_le_bytes(chunk.try_into().unwrap()))
            .map_err(|_| DbError::InvalidValue("query result id"))?);
    }
    Ok(QueryResult {
        ids,
        next_cursor: if flags & 2 != 0 {
            Some(QueryCursor::decode(&data[16..48])?)
        } else {
            None
        },
        degraded: flags & 1 != 0,
        total_scanned: u32::from_le_bytes(data[12..16].try_into().unwrap()),
    })
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

    #[test]
    fn native_lexical_query_and_result_roundtrip() {
        let q = MemoryQuery {
            scope: Some(MemoryScope::User),
            owner: Some(7),
            token_match: Some(TokenQuery {
                tokenizer_id: 11,
                tokenizer_version: 2,
                token_ids: vec![3, 5, 8],
                mode: TokenMatchMode::MinimumCount(2),
            }),
            order: QueryOrder::TokenRelevanceDesc,
            limit: 17,
            ..MemoryQuery::default()
        };
        let encoded = encode_native_query(&q).unwrap();
        let decoded = decode_native_query(&encoded).unwrap();
        assert_eq!(decoded.token_match, q.token_match);
        assert_eq!(decoded.scope, q.scope);
        assert_eq!(decoded.owner, q.owner);
        assert_eq!(decoded.limit, q.limit);

        let result = QueryResult {
            ids: vec![MemoryId::from_raw_unchecked(1), MemoryId::from_raw_unchecked(9)],
            next_cursor: Some(QueryCursor::new(4, 5, 9)),
            degraded: false,
            total_scanned: 12,
        };
        let encoded = encode_native_query_result(&result).unwrap();
        assert_eq!(decode_native_query_result(&encoded).unwrap(), result);
    }

    #[test]
    fn native_query_rejects_bad_bounds_and_lengths() {
        let q = MemoryQuery {
            token_match: Some(TokenQuery {
                tokenizer_id: 1,
                tokenizer_version: 1,
                token_ids: vec![1],
                mode: TokenMatchMode::Any,
            }),
            limit: 1,
            ..MemoryQuery::default()
        };
        let mut encoded = encode_native_query(&q).unwrap();
        encoded[6..8].copy_from_slice(&2u16.to_le_bytes());
        assert!(decode_native_query(&encoded).is_err());
        let mut zero_limit = q;
        zero_limit.limit = 0;
        assert!(encode_native_query(&zero_limit).is_err());
    }
}
