//! Stable, checksummed native operational-state snapshot.
//!
//! MemoryDB remains the only document store. This file contains only source
//! manifests and pending-import recovery metadata.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use wiseowl_memory::SourceId;
use wiseowl_memorydb::record::MemoryScope;

use crate::digest::{ContentDigest, FastFingerprint, LegacyFnvContentHash};
use crate::error::IndexError;
use crate::hash::fnv1a64;
use crate::import_key::ImportKey;
use crate::source::{
    FileIdentity, PendingImport, PendingImportState, PipelineVersions, SourceFailure,
    SourceFailureKind, SourceManifest, SourceState,
};
use crate::state::IndexerState;

const MAGIC: &[u8; 4] = b"OWIS";
/// v2: rejected-source confirmation_count + validator_version in failure records.
pub const OPERATIONAL_STATE_FORMAT_VERSION: u16 = 2;
const HEADER_LEN: usize = 18;
const MAX_STATE_BYTES: usize = 2 * 1024 * 1024;
const MAX_MANIFESTS: usize = 4096;

pub fn encode_state(state: &IndexerState) -> Result<Vec<u8>, IndexError> {
    if state.sources.len() > MAX_MANIFESTS {
        return Err(IndexError::QuotaExceeded("operational manifests"));
    }
    let mut body = Vec::new();
    put_u64(&mut body, state.next_source_counter);
    put_u16(&mut body, state.source_id_generation);
    put_u64(&mut body, state.last_successful_scan_ns);
    put_u64(&mut body, state.config_generation);
    put_u32(&mut body, state.sources.len() as u32);
    for manifest in state.sources.values() {
        encode_manifest(&mut body, manifest)?;
    }
    if body.len() > MAX_STATE_BYTES - HEADER_LEN {
        return Err(IndexError::QuotaExceeded("operational state bytes"));
    }
    let mut out = Vec::with_capacity(HEADER_LEN + body.len());
    out.extend_from_slice(MAGIC);
    put_u16(&mut out, OPERATIONAL_STATE_FORMAT_VERSION);
    put_u32(&mut out, body.len() as u32);
    put_u64(&mut out, fnv1a64(&body));
    out.extend_from_slice(&body);
    Ok(out)
}

pub fn decode_state(data: &[u8]) -> Result<IndexerState, IndexError> {
    if data.len() < HEADER_LEN || &data[..4] != MAGIC {
        return Err(IndexError::InvalidValue("operational state header"));
    }
    let mut header = Reader::new(&data[4..HEADER_LEN]);
    if header.u16()? != OPERATIONAL_STATE_FORMAT_VERSION {
        return Err(IndexError::InvalidValue("operational state version"));
    }
    let body_len = header.u32()? as usize;
    let checksum = header.u64()?;
    if body_len > MAX_STATE_BYTES - HEADER_LEN || data.len() != HEADER_LEN + body_len {
        return Err(IndexError::InvalidValue("operational state length"));
    }
    let body = &data[HEADER_LEN..];
    if fnv1a64(body) != checksum {
        return Err(IndexError::InvalidValue("operational state checksum"));
    }
    let mut r = Reader::new(body);
    let mut state = IndexerState::new();
    state.next_source_counter = r.u64()?.max(1);
    state.source_id_generation = r.u16()?.max(1);
    state.last_successful_scan_ns = r.u64()?;
    state.config_generation = r.u64()?;
    let count = r.u32()? as usize;
    if count > MAX_MANIFESTS {
        return Err(IndexError::QuotaExceeded("operational manifests"));
    }
    for _ in 0..count {
        state.insert_manifest(decode_manifest(&mut r)?);
    }
    if !r.done() {
        return Err(IndexError::InvalidValue("operational state trailing bytes"));
    }
    Ok(state)
}

fn encode_manifest(out: &mut Vec<u8>, m: &SourceManifest) -> Result<(), IndexError> {
    put_u16(out, m.manifest_version);
    put_u64(out, m.source_id.get()); put_u64(out, m.root_id); put_u8(out, m.scope.as_u8());
    put_u64(out, m.owner); put_string(out, &m.relative_path, 1024)?; put_u64(out, m.canonical_path_hash);
    match m.file_identity { Some(v) => { put_u8(out, 1); put_u64(out, v.device); put_u64(out, v.inode); }, None => put_u8(out, 0) }
    out.extend_from_slice(&m.content_digest.encode());
    put_opt_u64(out, m.fast_fingerprint.map(|v| v.get()));
    put_opt_u64(out, m.legacy_content_hash.map(|v| v.get()));
    put_u8(out, m.needs_digest_upgrade as u8); put_u64(out, m.size_bytes); put_opt_u64(out, m.modified_at_ns);
    for v in [m.parser_id, m.parser_version, m.tokenizer_id, m.tokenizer_version, m.chunking_id, m.chunking_version, m.ignore_config_version] { put_u32(out, v); }
    put_u64(out, m.indexed_at_ns); put_u8(out, m.state.as_u8()); put_u32(out, m.chunk_count);
    put_opt_u64(out, m.document_memory_id); put_u32(out, m.source_revision); put_u16(out, m.missing_confirmations);
    match &m.failure {
        Some(v) => {
            put_u8(out, 1);
            put_u8(out, v.kind as u8);
            put_u64(out, v.first_failure_ns);
            put_u64(out, v.latest_failure_ns);
            put_u32(out, v.attempt_count);
            put_u32(out, v.confirmation_count);
            put_u64(out, v.metadata_hash);
            put_u64(out, v.retry_after_ns);
            put_u32(out, v.validator_version);
        }
        None => put_u8(out, 0),
    }
    match &m.pending_import {
        Some(v) => { put_u8(out, 1); encode_pending(out, v); }
        None => put_u8(out, 0),
    }
    Ok(())
}

fn decode_manifest(r: &mut Reader<'_>) -> Result<SourceManifest, IndexError> {
    let manifest_version = r.u16()?;
    // Manifest record version is independent of the outer operational-state envelope.
    if manifest_version != SourceManifest::MANIFEST_VERSION {
        return Err(IndexError::InvalidValue("manifest version"));
    }
    let source_id = SourceId::from_raw(r.u64()?).map_err(|_| IndexError::InvalidValue("source id"))?;
    let root_id = r.u64()?;
    let scope = MemoryScope::from_u8(r.u8()?).ok_or(IndexError::InvalidValue("scope"))?;
    let owner = r.u64()?; let relative_path = r.string(1024)?; let canonical_path_hash = r.u64()?;
    let file_identity = match r.u8()? { 0 => None, 1 => Some(FileIdentity { device: r.u64()?, inode: r.u64()? }), _ => return Err(IndexError::InvalidValue("file identity flag")) };
    let content_digest = if r.peek_digest_version()? == 0 { r.unset_digest()? } else { ContentDigest::decode(r.take(35)?)? };
    let fast_fingerprint = r.opt_u64()?.map(FastFingerprint::new);
    let legacy_content_hash = r.opt_u64()?.map(LegacyFnvContentHash::new);
    let needs_digest_upgrade = r.bool()?; let size_bytes = r.u64()?; let modified_at_ns = r.opt_u64()?;
    let parser_id = r.u32()?; let parser_version = r.u32()?; let tokenizer_id = r.u32()?; let tokenizer_version = r.u32()?;
    let chunking_id = r.u32()?; let chunking_version = r.u32()?; let ignore_config_version = r.u32()?;
    let indexed_at_ns = r.u64()?; let state = SourceState::from_u8(r.u8()?).ok_or(IndexError::InvalidValue("source state"))?;
    let chunk_count = r.u32()?; let document_memory_id = r.opt_u64()?; let source_revision = r.u32()?; let missing_confirmations = r.u16()?;
    let failure = match r.u8()? {
        0 => None,
        1 => Some(SourceFailure {
            kind: SourceFailureKind::from_u8(r.u8()?).ok_or(IndexError::InvalidValue("failure kind"))?,
            first_failure_ns: r.u64()?,
            latest_failure_ns: r.u64()?,
            attempt_count: r.u32()?,
            confirmation_count: r.u32()?,
            metadata_hash: r.u64()?,
            retry_after_ns: r.u64()?,
            validator_version: r.u32()?,
        }),
        _ => return Err(IndexError::InvalidValue("failure flag")),
    };
    let pending_import = match r.u8()? { 0 => None, 1 => Some(decode_pending(r)?), _ => return Err(IndexError::InvalidValue("pending flag")) };
    Ok(SourceManifest { manifest_version, source_id, root_id, scope, owner, relative_path, canonical_path_hash, file_identity, content_digest, fast_fingerprint, legacy_content_hash, needs_digest_upgrade, size_bytes, modified_at_ns, parser_id, parser_version, tokenizer_id, tokenizer_version, chunking_id, chunking_version, ignore_config_version, indexed_at_ns, state, chunk_count, document_memory_id, source_revision, missing_confirmations, failure, pending_import })
}

fn encode_pending(out: &mut Vec<u8>, p: &PendingImport) {
    put_u16(out, p.format_version); out.extend_from_slice(&p.import_key.encode_canonical()); put_u64(out, p.source_id.get());
    put_u32(out, p.expected_revision); out.extend_from_slice(&p.content_digest.encode());
    let v = p.pipeline_versions; for n in [v.parser_id,v.parser_version,v.tokenizer_id,v.tokenizer_version,v.chunking_id,v.chunking_version,v.ignore_config_version] { put_u32(out,n); }
    put_u8(out, p.state as u8); put_u64(out, p.created_at); put_u64(out, p.latest_attempt_at); put_u16(out, p.attempt_count);
}
fn decode_pending(r: &mut Reader<'_>) -> Result<PendingImport, IndexError> {
    let format_version = r.u16()?; if format_version != 1 { return Err(IndexError::InvalidValue("pending version")); }
    let import_key = ImportKey::decode_canonical(r.take(crate::import_key::IMPORT_KEY_ENCODED_LEN)?)?;
    let source_id = SourceId::from_raw(r.u64()?).map_err(|_| IndexError::InvalidValue("pending source"))?;
    let expected_revision = r.u32()?; let content_digest = ContentDigest::decode(r.take(35)?)?;
    let pipeline_versions = PipelineVersions { parser_id:r.u32()?, parser_version:r.u32()?, tokenizer_id:r.u32()?, tokenizer_version:r.u32()?, chunking_id:r.u32()?, chunking_version:r.u32()?, ignore_config_version:r.u32()? };
    let state = PendingImportState::from_u8(r.u8()?).ok_or(IndexError::InvalidValue("pending state"))?;
    Ok(PendingImport { format_version, import_key, source_id, expected_revision, content_digest, pipeline_versions, state, created_at:r.u64()?, latest_attempt_at:r.u64()?, attempt_count:r.u16()? })
}

fn put_u8(out:&mut Vec<u8>,v:u8){out.push(v)} fn put_u16(out:&mut Vec<u8>,v:u16){out.extend_from_slice(&v.to_le_bytes())}
fn put_u32(out:&mut Vec<u8>,v:u32){out.extend_from_slice(&v.to_le_bytes())} fn put_u64(out:&mut Vec<u8>,v:u64){out.extend_from_slice(&v.to_le_bytes())}
fn put_opt_u64(out:&mut Vec<u8>,v:Option<u64>){match v{Some(n)=>{put_u8(out,1);put_u64(out,n)},None=>put_u8(out,0)}}
fn put_string(out:&mut Vec<u8>,v:&str,max:usize)->Result<(),IndexError>{if v.len()>max||v.len()>u16::MAX as usize{return Err(IndexError::InvalidValue("state string"));}put_u16(out,v.len() as u16);out.extend_from_slice(v.as_bytes());Ok(())}

struct Reader<'a>{data:&'a [u8],pos:usize}
impl<'a> Reader<'a>{fn new(data:&'a[u8])->Self{Self{data,pos:0}} fn take(&mut self,n:usize)->Result<&'a[u8],IndexError>{let end=self.pos.checked_add(n).ok_or(IndexError::InvalidValue("state overflow"))?;let s=self.data.get(self.pos..end).ok_or(IndexError::InvalidValue("state truncated"))?;self.pos=end;Ok(s)} fn u8(&mut self)->Result<u8,IndexError>{Ok(self.take(1)?[0])} fn u16(&mut self)->Result<u16,IndexError>{Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))} fn u32(&mut self)->Result<u32,IndexError>{Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))} fn u64(&mut self)->Result<u64,IndexError>{Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))} fn bool(&mut self)->Result<bool,IndexError>{match self.u8()?{0=>Ok(false),1=>Ok(true),_=>Err(IndexError::InvalidValue("state bool"))}} fn opt_u64(&mut self)->Result<Option<u64>,IndexError>{match self.u8()?{0=>Ok(None),1=>Ok(Some(self.u64()?)),_=>Err(IndexError::InvalidValue("state option"))}} fn string(&mut self,max:usize)->Result<String,IndexError>{let n=self.u16()? as usize;if n>max{return Err(IndexError::InvalidValue("state string length"));}core::str::from_utf8(self.take(n)?).map(|s|s.to_string()).map_err(|_|IndexError::InvalidValue("state utf8"))} fn done(&self)->bool{self.pos==self.data.len()} fn peek_digest_version(&self)->Result<u16,IndexError>{let b=self.data.get(self.pos+1..self.pos+3).ok_or(IndexError::InvalidValue("digest truncated"))?;Ok(u16::from_le_bytes(b.try_into().unwrap()))} fn unset_digest(&mut self)->Result<ContentDigest,IndexError>{let raw=self.take(35)?;if raw[0]!=1||raw[1..3]!=[0,0]||raw[3..]!=[0u8;32]{return Err(IndexError::InvalidValue("unset digest"));}Ok(ContentDigest::unset())}}

#[cfg(test)] mod tests { use super::*; use crate::digest::digest_bytes; use crate::source::SourceManifest; #[test] fn state_roundtrip_and_checksum(){let mut s=IndexerState::new();let m=SourceManifest::new_v2(SourceId::from_raw_unchecked(1),1,MemoryScope::User,7,String::from("a.txt"),9,digest_bytes(b"a"),Some(FastFingerprint::new(3)));s.insert_manifest(m);let bytes=encode_state(&s).unwrap();let decoded=decode_state(&bytes).unwrap();assert_eq!(decoded.sources.len(),1);assert_eq!(decoded.sources.get(&1).unwrap().relative_path,"a.txt");let mut bad=bytes;let last=bad.len()-1;bad[last]^=1;assert!(decode_state(&bad).is_err());}}
