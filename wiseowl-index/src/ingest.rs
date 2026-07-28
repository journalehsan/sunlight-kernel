//! Transactional ingestion into wiseowl-memorydb (public APIs only).
//!
//! Uses [`IndexMemoryDb`] so host and native transports share one engine.
//! Strong content digests are stored as attributes; provenance retains an
//! optional FNV fingerprint for historical compatibility only.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use wiseowl_memory::{MemoryId, SourceId, SourceKind, TrustLevel};
use wiseowl_memorydb::attributes::{Attribute, AttributeSet, AttributeValue};
use wiseowl_memorydb::database::InsertRequest;
use wiseowl_memorydb::provenance::{DerivationKind, LongTermProvenance};
use wiseowl_memorydb::query::DedupPolicy;
use wiseowl_memorydb::record::LongTermMemoryKind;
use wiseowl_memorydb::relationship::{MemoryRelationship, RelationshipKind};
use wiseowl_memorydb::tokens::{IndexedToken, TokenSetRef};

use crate::chunk::DocumentChunk;
use crate::digest::ContentDigest;
use crate::error::IndexError;
use crate::import_key::{build_import_key, ImportKey, ImportState};
use crate::memorydb_backend::IndexMemoryDb;
use crate::quotas::IndexQuotaConfig;
use crate::source::{PendingImport, PendingImportState, PipelineVersions, SourceManifest};
use crate::tokenize::{to_indexed_tokens, TokenSink};

/// Producer service name stored in provenance.
pub const PRODUCER_SERVICE: &str = "wiseowl-indexd";

/// Prepared chunk ready for insert.
#[derive(Debug, Clone)]
pub struct PreparedChunk {
    pub chunk: DocumentChunk,
    pub tokens: Vec<IndexedToken>,
    pub tokenizer_id: u32,
    pub tokenizer_version: u32,
}

/// Result of a successful ingest.
#[derive(Debug, Clone)]
pub struct IngestResult {
    pub document_id: MemoryId,
    pub chunk_ids: Vec<MemoryId>,
    pub source_revision: u32,
    pub import_key: ImportKey,
    /// True if MemoryDB already had this generation (idempotent).
    pub already_committed: bool,
}

/// Build provenance for imported source material (claims, not verified facts).
///
/// `legacy_fnv` is optional historical fingerprint stored in provenance's
/// `source_content_hash` field (u64). Strong identity is in attributes.
pub fn import_provenance(
    source_id: SourceId,
    legacy_fnv: Option<u64>,
    external_ref: Option<String>,
    now_ns: u64,
) -> LongTermProvenance {
    LongTermProvenance {
        source_kind: SourceKind::UserInput,
        source_id: Some(source_id),
        producer_service: String::from(PRODUCER_SERVICE),
        original_memory_ids: Vec::new(),
        parent_lt_ids: Vec::new(),
        insertion_time_ns: now_ns,
        trust: TrustLevel::Untrusted,
        source_content_hash: legacy_fnv,
        external_ref,
        derivation: DerivationKind::DirectImport,
    }
}

fn attrs(entries: &[(&str, AttributeValue)]) -> AttributeSet {
    let mut set = AttributeSet {
        entries: entries
            .iter()
            .map(|(k, v)| Attribute {
                key: String::from(*k),
                value: v.clone(),
            })
            .collect(),
    };
    set.normalize();
    set
}

/// Build document attributes including strong digest + import key.
fn document_attrs(
    manifest: &SourceManifest,
    relative_path: &str,
    prepared_len: usize,
    revision: u32,
    import_key_hex: &str,
) -> AttributeSet {
    // Keep ≤ max_attributes_per_record (16). Strong digest is authoritative.
    let dig = &manifest.content_digest;
    // Pack alg+ver into one unsigned: (alg << 16) | ver
    let digest_meta = ((dig.algorithm.as_u8() as u64) << 16) | (dig.version as u64);
    attrs(&[
        (
            "record_role",
            AttributeValue::Text(String::from("document")),
        ),
        ("root_id", AttributeValue::Unsigned(manifest.root_id)),
        (
            "relative_path",
            AttributeValue::Text(truncate_attr(relative_path, 120)),
        ),
        ("content_digest_meta", AttributeValue::Unsigned(digest_meta)),
        ("content_digest", AttributeValue::Text(dig.to_hex())),
        (
            "parser_id",
            AttributeValue::Unsigned(manifest.parser_id as u64),
        ),
        (
            "parser_version",
            AttributeValue::Unsigned(manifest.parser_version as u64),
        ),
        (
            "tokenizer_id",
            AttributeValue::Unsigned(manifest.tokenizer_id as u64),
        ),
        (
            "tokenizer_version",
            AttributeValue::Unsigned(manifest.tokenizer_version as u64),
        ),
        (
            "chunking_id",
            AttributeValue::Unsigned(manifest.chunking_id as u64),
        ),
        (
            "chunking_version",
            AttributeValue::Unsigned(manifest.chunking_version as u64),
        ),
        ("chunk_count", AttributeValue::Unsigned(prepared_len as u64)),
        ("source_revision", AttributeValue::Unsigned(revision as u64)),
        (
            "import_key",
            AttributeValue::Text(String::from(import_key_hex)),
        ),
        (
            "fast_fingerprint",
            AttributeValue::Unsigned(manifest.fast_fingerprint.map(|v| v.get()).unwrap_or(0)),
        ),
        (
            "manifest_version",
            AttributeValue::Unsigned(manifest.manifest_version as u64),
        ),
    ])
}

/// Ingest one source as a single atomic transaction when it fits quotas.
///
/// Phase 3.5: reconcile ImportKey first; persist pending metadata is caller's job.
/// Partial generations are never committed.
pub fn ingest_source_atomic<B: IndexMemoryDb>(
    backend: &mut B,
    manifest: &SourceManifest,
    relative_path: &str,
    prepared: &[PreparedChunk],
    quotas: &IndexQuotaConfig,
    now_ns: u64,
) -> Result<IngestResult, IndexError> {
    if prepared.len() as u32 > quotas.max_chunks_per_file {
        return Err(IndexError::QuotaExceeded("chunks per file"));
    }
    let ops = 1u32.saturating_add(prepared.len() as u32);
    if ops > quotas.max_ingest_ops_per_tx {
        return Err(IndexError::QuotaExceeded("ingest ops"));
    }
    if !manifest.content_digest.is_set() {
        return Err(IndexError::InvalidValue("strong content digest required"));
    }

    let revision = manifest.source_revision.saturating_add(1).max(1);
    let import_key = build_import_key(
        manifest.source_id,
        revision,
        manifest.content_digest,
        manifest.parser_id,
        manifest.parser_version,
        manifest.tokenizer_id,
        manifest.tokenizer_version,
        manifest.chunking_id,
        manifest.chunking_version,
        manifest.scope,
        manifest.owner,
        manifest.ignore_config_version,
    );

    // Reconcile before starting a replacement transaction.
    let recon = backend.reconcile_import(&import_key)?;
    match recon.state {
        ImportState::AlreadyCommitted | ImportState::Committed => {
            if let Some(doc_id) = recon.document_memory_id {
                return Ok(IngestResult {
                    document_id: MemoryId::from_raw_unchecked(doc_id),
                    chunk_ids: Vec::new(),
                    source_revision: recon.source_revision.unwrap_or(revision),
                    import_key,
                    already_committed: true,
                });
            }
        }
        ImportState::Conflict => {
            return Err(IndexError::TransactionRejected(String::from(
                "import key conflict",
            )));
        }
        ImportState::InProgress => {
            return Err(IndexError::TransactionRejected(String::from(
                "import in progress",
            )));
        }
        ImportState::NotFound | ImportState::Aborted => {}
    }

    let prev_doc = manifest
        .document_memory_id
        .and_then(|id| MemoryId::from_raw(id).ok());
    let key_hex = import_key.key_hex();
    let legacy_fnv = manifest
        .legacy_content_hash
        .map(|v| v.get())
        .or_else(|| manifest.fast_fingerprint.map(|v| v.get()));

    let tx = backend.begin_transaction()?;

    let doc_payload = alloc::format!(
        "document source_id={} path={} digest={} chunks={}",
        manifest.source_id.get(),
        relative_path,
        manifest.content_digest.abbreviated_hex(),
        prepared.len()
    );
    let doc_attrs = document_attrs(manifest, relative_path, prepared.len(), revision, &key_hex);

    let doc_req = InsertRequest {
        kind: LongTermMemoryKind::ImportedRecord,
        scope: manifest.scope,
        owner: manifest.owner,
        payload: doc_payload.into_bytes(),
        provenance: import_provenance(
            manifest.source_id,
            legacy_fnv,
            Some(truncate_attr(relative_path, 120)),
            now_ns,
        ),
        confidence: 5000,
        importance: 3000,
        trust: TrustLevel::Untrusted,
        valid_from_ns: None,
        valid_until_ns: None,
        tokens: None,
        attributes: doc_attrs,
        supersedes: prev_doc,
        relationships: Vec::new(),
        dedup: DedupPolicy::Allow,
        id: None,
        revision,
    };

    let document_id = match backend.insert_record(tx, doc_req) {
        Ok(id) => id,
        Err(e) => {
            let _ = backend.abort_transaction(tx);
            return Err(e);
        }
    };

    let mut chunk_ids = Vec::with_capacity(prepared.len());
    for pc in prepared {
        let ch = &pc.chunk;
        let digest_meta = ((ch.content_digest.algorithm.as_u8() as u64) << 16)
            | (ch.content_digest.version as u64);
        let chunk_attrs = attrs(&[
            (
                "record_role",
                AttributeValue::Text(String::from("document_chunk")),
            ),
            ("chunk_ordinal", AttributeValue::Unsigned(ch.ordinal as u64)),
            ("byte_start", AttributeValue::Unsigned(ch.byte_start)),
            ("byte_end", AttributeValue::Unsigned(ch.byte_end)),
            ("line_start", AttributeValue::Unsigned(ch.line_start as u64)),
            ("line_end", AttributeValue::Unsigned(ch.line_end as u64)),
            ("content_digest_meta", AttributeValue::Unsigned(digest_meta)),
            (
                "content_digest",
                AttributeValue::Text(ch.content_digest.to_hex()),
            ),
            ("source_revision", AttributeValue::Unsigned(revision as u64)),
            ("import_key", AttributeValue::Text(key_hex.clone())),
        ]);

        let rel = MemoryRelationship {
            source: MemoryId::from_raw_unchecked(1),
            target: document_id,
            kind: RelationshipKind::DerivedFrom,
            confidence: 10000,
            created_at_ns: now_ns,
            provenance: wiseowl_memorydb::provenance::RelationshipProvenance {
                producer_service: String::from(PRODUCER_SERVICE),
                created_at_ns: now_ns,
                trust: TrustLevel::Untrusted,
            },
            tombstoned: false,
        };

        let token_ref = TokenSetRef {
            tokenizer_id: pc.tokenizer_id,
            tokenizer_version: pc.tokenizer_version,
            token_count: pc.tokens.len() as u32,
        };

        let req = InsertRequest {
            kind: LongTermMemoryKind::ImportedRecord,
            scope: manifest.scope,
            owner: manifest.owner,
            payload: ch.text.as_bytes().to_vec(),
            provenance: {
                let mut p = import_provenance(
                    manifest.source_id,
                    Some(ch.content_digest.fingerprint64()),
                    Some(truncate_attr(relative_path, 120)),
                    now_ns,
                );
                p.parent_lt_ids = alloc::vec![document_id];
                p
            },
            confidence: 5000,
            importance: 2000,
            trust: TrustLevel::Untrusted,
            valid_from_ns: None,
            valid_until_ns: None,
            tokens: Some((token_ref, pc.tokens.clone())),
            attributes: chunk_attrs,
            supersedes: None,
            relationships: alloc::vec![rel],
            dedup: DedupPolicy::Allow,
            id: None,
            revision: 1,
        };

        match backend.insert_record(tx, req) {
            Ok(id) => chunk_ids.push(id),
            Err(e) => {
                let _ = backend.abort_transaction(tx);
                return Err(e);
            }
        }
    }

    match backend.commit_transaction(tx) {
        Ok(_) => Ok(IngestResult {
            document_id,
            chunk_ids,
            source_revision: revision,
            import_key,
            already_committed: false,
        }),
        Err(e) => Err(e),
    }
}

/// Build pending import metadata (persist before begin_import).
pub fn pending_for_manifest(
    import_key: &ImportKey,
    now_ns: u64,
    _local_tx_id: Option<u64>,
) -> PendingImport {
    PendingImport {
        format_version: 1,
        import_key: import_key.clone(),
        source_id: import_key.source_id,
        expected_revision: import_key.source_revision,
        content_digest: import_key.content_digest,
        pipeline_versions: PipelineVersions {
            parser_id: import_key.parser_id,
            parser_version: import_key.parser_version,
            tokenizer_id: import_key.tokenizer_id,
            tokenizer_version: import_key.tokenizer_version,
            chunking_id: import_key.chunking_id,
            chunking_version: import_key.chunking_version,
            ignore_config_version: import_key.ingestion_config_generation,
        },
        state: PendingImportState::Prepared,
        created_at: now_ns,
        latest_attempt_at: now_ns,
        attempt_count: 1,
    }
}

fn truncate_attr(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        s.chars().take(max).collect()
    }
}

/// Tokenize all chunks with the given tokenizer.
pub fn prepare_chunks_from_text(
    chunks: Vec<DocumentChunk>,
    tokenizer: &dyn crate::tokenize::RetrievalTokenizer,
    dict: &mut crate::tokenize::TokenDictionary,
    quotas: &IndexQuotaConfig,
) -> Result<Vec<PreparedChunk>, IndexError> {
    let mut out = Vec::with_capacity(chunks.len());
    for chunk in chunks {
        let mut norm = crate::tokenize::NormalizedTextBuffer::default();
        tokenizer.normalize(&chunk.text, &mut norm)?;
        let mut sink = TokenSink::default();
        tokenizer.tokenize(&norm.text, dict, quotas, &mut sink)?;
        let tokens = to_indexed_tokens(&sink);
        out.push(PreparedChunk {
            chunk,
            tokens,
            tokenizer_id: tokenizer.tokenizer_id(),
            tokenizer_version: tokenizer.version(),
        });
    }
    Ok(out)
}

/// Bound source deletion via memorydb backend.
pub fn delete_source_bounded<B: IndexMemoryDb>(
    backend: &mut B,
    source_id: SourceId,
    batch: u32,
) -> Result<(u32, bool), IndexError> {
    backend.delete_source(source_id, batch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::{chunk_blocks, ChunkingProfile};
    use crate::digest::digest_bytes;
    use crate::memorydb_backend::HostMemoryDbBackend;
    use crate::parse::{ParsedBlock, ParsedBlockKind};
    use crate::source::{SourceManifest, SourceState};
    use crate::tokenize::{TokenDictionary, WiseOwlLexicalV1};
    use wiseowl_memorydb::database::{DbCaller, MemoryStore};
    use wiseowl_memorydb::record::MemoryScope;
    use wiseowl_memorydb::DbQuotaConfig;

    fn base_manifest(sid: SourceId) -> SourceManifest {
        let body = b"hello world";
        SourceManifest {
            manifest_version: 2,
            source_id: sid,
            root_id: 1,
            scope: MemoryScope::User,
            owner: 1,
            relative_path: String::from("notes.txt"),
            canonical_path_hash: 1,
            file_identity: None,
            content_digest: digest_bytes(body),
            fast_fingerprint: Some(crate::digest::fast_fingerprint(body)),
            legacy_content_hash: None,
            needs_digest_upgrade: false,
            size_bytes: 11,
            modified_at_ns: None,
            parser_id: 1,
            parser_version: 1,
            tokenizer_id: 1,
            tokenizer_version: 1,
            chunking_id: 1,
            chunking_version: 1,
            ignore_config_version: 1,
            indexed_at_ns: 0,
            state: SourceState::Stable,
            chunk_count: 0,
            document_memory_id: None,
            source_revision: 0,
            missing_confirmations: 0,
            failure: None,
            pending_import: None,
        }
    }

    #[test]
    fn atomic_ingest_and_idempotent_reconcile() {
        let db = wiseowl_memorydb::Database::<MemoryStore>::open_memory(DbQuotaConfig::default())
            .unwrap();
        let mut backend = HostMemoryDbBackend::new(db, DbCaller::user(1));
        let sid = SourceId::from_raw_unchecked(42);
        let mut manifest = base_manifest(sid);
        let blocks = vec![ParsedBlock {
            block_kind: ParsedBlockKind::Paragraph,
            byte_start: 0,
            byte_end: 11,
            line_start: 1,
            line_end: 1,
            heading_path: String::new(),
            text: String::from("hello world"),
        }];
        let q = IndexQuotaConfig::default();
        let chunks = chunk_blocks(sid, 1, 1, 1, &ChunkingProfile::default(), &blocks, &q).unwrap();
        let mut dict = TokenDictionary::new();
        let prepared = prepare_chunks_from_text(chunks, &WiseOwlLexicalV1, &mut dict, &q).unwrap();
        let res = ingest_source_atomic(&mut backend, &manifest, "notes.txt", &prepared, &q, 1000)
            .unwrap();
        assert!(!res.already_committed);
        assert!(backend.get_record(res.document_id, false).is_ok());
        manifest.source_revision = res.source_revision;
        manifest.document_memory_id = Some(res.document_id.get());

        // Same import key → already committed (no duplicate generation)
        // Roll back revision so the same generation key is retried.
        let saved_rev = manifest.source_revision;
        manifest.source_revision = saved_rev.saturating_sub(1);
        let res2 = ingest_source_atomic(&mut backend, &manifest, "notes.txt", &prepared, &q, 1001)
            .unwrap();
        assert!(res2.already_committed);
        assert_eq!(res.document_id, res2.document_id);
        manifest.source_revision = saved_rev;

        // Content change → new revision (source_revision advances to planned = saved+1)
        let body2 = b"changed";
        manifest.content_digest = digest_bytes(body2);
        manifest.fast_fingerprint = Some(crate::digest::fast_fingerprint(body2));
        let blocks2 = vec![ParsedBlock {
            block_kind: ParsedBlockKind::Paragraph,
            byte_start: 0,
            byte_end: 7,
            line_start: 1,
            line_end: 1,
            heading_path: String::new(),
            text: String::from("changed"),
        }];
        let chunks2 =
            chunk_blocks(sid, 2, 1, 1, &ChunkingProfile::default(), &blocks2, &q).unwrap();
        let prepared2 =
            prepare_chunks_from_text(chunks2, &WiseOwlLexicalV1, &mut dict, &q).unwrap();
        let res3 = ingest_source_atomic(&mut backend, &manifest, "notes.txt", &prepared2, &q, 2000)
            .unwrap();
        assert_ne!(res.document_id, res3.document_id);
        assert!(!res3.already_committed);
    }
}
