//! Little-endian wire encoding for native InsertRequest transport (Phase 3.5).
//!
//! Used by independent services over SHM; not a second database protocol —
//! encodes the existing Phase 2 [`InsertRequest`] fields only.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use wiseowl_memory::{MemoryId, TrustLevel};

use crate::attributes::AttributeSet;
use crate::codec::{BufReader, BufWriter};
use crate::database::InsertRequest;
use crate::error::DbError;
use crate::provenance::LongTermProvenance;
use crate::query::DedupPolicy;
use crate::quotas::DbQuotaConfig;
use crate::record::{LongTermMemoryKind, MemoryScope};
use crate::relationship::MemoryRelationship;
use crate::tokens::{IndexedToken, TokenSetRef};

/// Wire format version for insert envelopes.
pub const INSERT_WIRE_VERSION: u16 = 1;

/// Encode an insert request for native SHM transport.
pub fn encode_insert_request(req: &InsertRequest, max: usize) -> Result<Vec<u8>, DbError> {
    let mut w = BufWriter::with_capacity(max);
    w.write_u16(INSERT_WIRE_VERSION)?;
    w.write_u8(req.kind.as_u8())?;
    w.write_u8(req.scope.as_u8())?;
    w.write_u64(req.owner)?;
    w.write_bytes_len_u32(&req.payload)?;
    req.provenance.encode(&mut w)?;
    w.write_u16(req.confidence)?;
    w.write_u16(req.importance)?;
    w.write_u8(req.trust.as_u8())?;
    match req.valid_from_ns {
        None => w.write_u8(0)?,
        Some(v) => {
            w.write_u8(1)?;
            w.write_u64(v)?;
        }
    }
    match req.valid_until_ns {
        None => w.write_u8(0)?,
        Some(v) => {
            w.write_u8(1)?;
            w.write_u64(v)?;
        }
    }
    match &req.tokens {
        None => w.write_u8(0)?,
        Some((ts, tokens)) => {
            w.write_u8(1)?;
            ts.encode(&mut w)?;
            w.write_u16(tokens.len() as u16)?;
            for t in tokens {
                t.encode(&mut w)?;
            }
        }
    }
    req.attributes.encode(&mut w)?;
    match req.supersedes {
        None => w.write_u8(0)?,
        Some(id) => {
            w.write_u8(1)?;
            w.write_u64(id.get())?;
        }
    }
    w.write_u16(req.relationships.len() as u16)?;
    for rel in &req.relationships {
        rel.encode(&mut w)?;
    }
    w.write_u8(match req.dedup {
        DedupPolicy::Allow => 0,
        DedupPolicy::RejectExactPayload => 1,
        DedupPolicy::ReturnExistingExactPayload => 2,
        DedupPolicy::RejectSameSourceRevision => 3,
    })?;
    match req.id {
        None => w.write_u8(0)?,
        Some(id) => {
            w.write_u8(1)?;
            w.write_u64(id.get())?;
        }
    }
    w.write_u32(req.revision)?;
    Ok(w.into_vec())
}

/// Decode an insert request from native SHM transport.
pub fn decode_insert_request(
    data: &[u8],
    quotas: &DbQuotaConfig,
) -> Result<InsertRequest, DbError> {
    let mut r = BufReader::new(data);
    let ver = r.read_u16()?;
    if ver != INSERT_WIRE_VERSION {
        return Err(DbError::UnsupportedProtocolVersion {
            got: ver,
            want: INSERT_WIRE_VERSION,
        });
    }
    let kind = LongTermMemoryKind::from_u8(r.read_u8()?)
        .ok_or(DbError::InvalidValue("kind"))?;
    let scope = MemoryScope::from_u8(r.read_u8()?)
        .ok_or(DbError::InvalidValue("scope"))?;
    let owner = r.read_u64()?;
    let payload = r.read_bytes_len_u32()?.to_vec();
    if payload.len() as u32 > quotas.max_payload_bytes {
        return Err(DbError::PayloadTooLarge {
            size: payload.len() as u32,
            max: quotas.max_payload_bytes,
        });
    }
    let provenance = LongTermProvenance::decode(&mut r, quotas)?;
    let confidence = r.read_u16()?;
    let importance = r.read_u16()?;
    let trust = TrustLevel::from_u8(r.read_u8()?).ok_or(DbError::InvalidValue("trust"))?;
    let valid_from_ns = match r.read_u8()? {
        0 => None,
        1 => Some(r.read_u64()?),
        _ => return Err(DbError::InvalidValue("valid_from")),
    };
    let valid_until_ns = match r.read_u8()? {
        0 => None,
        1 => Some(r.read_u64()?),
        _ => return Err(DbError::InvalidValue("valid_until")),
    };
    let tokens = match r.read_u8()? {
        0 => None,
        1 => {
            let ts = TokenSetRef::decode(&mut r)?;
            let n = r.read_u16()? as usize;
            if n as u32 > quotas.max_tokens_per_record {
                return Err(DbError::QuotaExceeded("tokens"));
            }
            let mut v = Vec::with_capacity(n);
            for _ in 0..n {
                v.push(IndexedToken::decode(&mut r, quotas)?);
            }
            Some((ts, v))
        }
        _ => return Err(DbError::InvalidValue("tokens tag")),
    };
    let attributes = AttributeSet::decode(&mut r, quotas)?;
    let supersedes = match r.read_u8()? {
        0 => None,
        1 => Some(
            MemoryId::from_raw(r.read_u64()?).map_err(|_| DbError::InvalidValue("supersedes"))?,
        ),
        _ => return Err(DbError::InvalidValue("supersedes tag")),
    };
    let n_rel = r.read_u16()? as usize;
    if n_rel as u32 > quotas.max_relationships_per_transaction {
        return Err(DbError::QuotaExceeded("relationships"));
    }
    let mut relationships = Vec::with_capacity(n_rel);
    for _ in 0..n_rel {
        relationships.push(MemoryRelationship::decode(&mut r)?);
    }
    let dedup = match r.read_u8()? {
        0 => DedupPolicy::Allow,
        1 => DedupPolicy::RejectExactPayload,
        2 => DedupPolicy::ReturnExistingExactPayload,
        3 => DedupPolicy::RejectSameSourceRevision,
        _ => return Err(DbError::InvalidValue("dedup")),
    };
    let id = match r.read_u8()? {
        0 => None,
        1 => Some(MemoryId::from_raw(r.read_u64()?).map_err(|_| DbError::InvalidValue("id"))?),
        _ => return Err(DbError::InvalidValue("id tag")),
    };
    let revision = r.read_u32()?;
    let _ = String::new(); // keep alloc used for no_std hygiene in some builds
    Ok(InsertRequest {
        kind,
        scope,
        owner,
        payload,
        provenance,
        confidence,
        importance,
        trust,
        valid_from_ns,
        valid_until_ns,
        tokens,
        attributes,
        supersedes,
        relationships,
        dedup,
        id,
        revision,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provenance::DerivationKind;
    use wiseowl_memory::SourceKind;

    #[test]
    fn insert_wire_roundtrip() {
        let req = InsertRequest {
            kind: LongTermMemoryKind::ImportedRecord,
            scope: MemoryScope::User,
            owner: 7,
            payload: b"hello".to_vec(),
            provenance: LongTermProvenance {
                source_kind: SourceKind::UserInput,
                source_id: None,
                producer_service: String::from("wiseowl-indexd"),
                original_memory_ids: Vec::new(),
                parent_lt_ids: Vec::new(),
                insertion_time_ns: 100,
                trust: TrustLevel::Untrusted,
                source_content_hash: Some(0xabc),
                external_ref: Some(String::from("a.txt")),
                derivation: DerivationKind::DirectImport,
            },
            confidence: 5000,
            importance: 3000,
            trust: TrustLevel::Untrusted,
            valid_from_ns: None,
            valid_until_ns: None,
            tokens: None,
            attributes: AttributeSet::default(),
            supersedes: None,
            relationships: Vec::new(),
            dedup: DedupPolicy::Allow,
            id: None,
            revision: 1,
        };
        let q = DbQuotaConfig::default();
        let enc = encode_insert_request(&req, 64 * 1024).unwrap();
        let dec = decode_insert_request(&enc, &q).unwrap();
        assert_eq!(dec.payload, b"hello");
        assert_eq!(dec.owner, 7);
        assert_eq!(dec.revision, 1);
    }
}
