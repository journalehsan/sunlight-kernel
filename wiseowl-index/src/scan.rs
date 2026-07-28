//! Incremental scan algorithm (metadata prefilter → strong hash → parse → tokenize → ingest).
//!
//! Fast path:
//! ```text
//! metadata unchanged → optional fast skip when strong digest already known
//! metadata changed   → strong hash
//! strong hash same   → metadata update only (no parse / tokenize / new generation)
//! strong hash change → full re-index
//! ```
//! FNV fingerprints never suppress strong-digest verification when metadata
//! indicates a possible change.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::chunk::{chunk_blocks, ChunkingProfile};
use crate::config::{IndexerConfig, PARSER_PLAIN};
use crate::digest::{digest_bytes, fast_fingerprint, ContentDigest};
use crate::error::IndexError;
use crate::hash::fnv1a64;
use crate::ignore::IgnoreSet;
use crate::import_key::{build_import_key, ImportState};
use crate::ingest::{
    delete_source_bounded, ingest_source_atomic, pending_for_manifest, prepare_chunks_from_text,
};
use crate::memorydb_backend::IndexMemoryDb;
use crate::parse::select_parser;
use crate::source::{
    PipelineVersions, SourceFailure, SourceFailureKind, SourceManifest, SourceState,
    VALIDATOR_VERSION,
};
use crate::stable_file::{hash_bytes, FileMetaSnapshot};
use crate::state::IndexerState;
use crate::stats::IndexStats;
use crate::text_validate::{normalize_newlines_owned, validate_utf8_text};
use crate::tokenize::{TokenDictionary, WiseOwlLexicalV1};

/// Scan request parameters.
#[derive(Debug, Clone, Default)]
pub struct ScanRequest {
    pub root_id: Option<u64>,
    pub force_reindex: bool,
}

/// Outcome of one scan quantum.
#[derive(Debug, Clone, Default)]
pub struct ScanOutcome {
    pub files_processed: u32,
    pub budget_exhausted: bool,
    pub roots_unavailable: u32,
}

/// Core scan engine operating on in-memory state + MemoryDB backend.
pub struct ScanEngine {
    pub config: IndexerConfig,
    pub ignore: IgnoreSet,
    pub dict: TokenDictionary,
    pub scanning: bool,
}

impl ScanEngine {
    pub fn new(config: IndexerConfig) -> Self {
        Self {
            config,
            ignore: IgnoreSet::builtin(),
            dict: TokenDictionary::new(),
            scanning: false,
        }
    }

    pub fn pipeline_versions(&self, parser_id: u32) -> PipelineVersions {
        PipelineVersions {
            parser_id,
            parser_version: self.config.parser_version,
            tokenizer_id: self.config.tokenizer_id,
            tokenizer_version: self.config.tokenizer_version,
            chunking_id: self.config.chunking_id,
            chunking_version: self.config.chunking_version,
            ignore_config_version: self.config.ignore_config_version,
        }
    }

    /// Reconcile pending imports (crash window: commit before manifest update).
    pub fn reconcile_pending<B: IndexMemoryDb>(
        &mut self,
        state: &mut IndexerState,
        backend: &mut B,
        stats: &mut IndexStats,
        now_ns: u64,
        max: u32,
    ) -> Result<u32, IndexError> {
        let mut done = 0u32;
        let pending: Vec<u64> = state
            .sources
            .values()
            .filter(|m| m.pending_import.is_some())
            .map(|m| m.source_id.get())
            .take(max as usize)
            .collect();
        for sid in pending {
            let Some(man) = state.sources.get(&sid).cloned() else {
                continue;
            };
            let Some(ref pend) = man.pending_import else {
                continue;
            };
            let key = pend.import_key.clone();
            IndexStats::sat_add(&mut stats.uncertain_imports, 1);
            match backend.reconcile_import(&key) {
                Ok(r) => match r.state {
                    ImportState::AlreadyCommitted | ImportState::Committed => {
                        if let Some(m) = state.sources.get_mut(&sid) {
                            m.document_memory_id = r.document_memory_id;
                            m.source_revision = r.source_revision.unwrap_or(pend.expected_revision);
                            m.content_digest = pend.content_digest;
                            m.pending_import = None;
                            m.state = SourceState::Indexed;
                            m.indexed_at_ns = now_ns;
                            m.needs_digest_upgrade = false;
                            m.manifest_version = SourceManifest::MANIFEST_VERSION;
                        }
                        IndexStats::sat_add(&mut stats.imports_reconciled_committed, 1);
                        let _ = backend.clear_prepared_import(man.source_id);
                        done = done.saturating_add(1);
                    }
                    ImportState::NotFound | ImportState::Aborted => {
                        if let Some(m) = state.sources.get_mut(&sid) {
                            m.pending_import = None;
                            m.state = SourceState::Changed;
                        }
                        IndexStats::sat_add(&mut stats.imports_reconciled_retried, 1);
                        let _ = backend.clear_prepared_import(man.source_id);
                        done = done.saturating_add(1);
                    }
                    ImportState::Conflict => {
                        if let Some(m) = state.sources.get_mut(&sid) {
                            m.state = SourceState::Failed;
                            m.failure = Some(SourceFailure {
                                kind: SourceFailureKind::ImportConflict,
                                first_failure_ns: now_ns,
                                latest_failure_ns: now_ns,
                                attempt_count: 1,
                                confirmation_count: 0,
                                metadata_hash: fnv1a64(m.relative_path.as_bytes()),
                                retry_after_ns: u64::MAX,
                                validator_version: VALIDATOR_VERSION,
                            });
                        }
                        IndexStats::sat_add(&mut stats.imports_reconciled_conflict, 1);
                        done = done.saturating_add(1);
                    }
                    ImportState::InProgress => {
                        // Bounded wait: leave pending for next maintenance.
                    }
                },
                Err(IndexError::DatabaseUnavailable) => {
                    IndexStats::sat_add(&mut stats.memorydb_unavailable_operations, 1);
                    return Err(IndexError::DatabaseUnavailable);
                }
                Err(_) => {}
            }
        }
        stats.pending_imports = state.pending_import_count();
        Ok(done)
    }

    /// Run a bounded scan using a provided discovery listing (tests / virtual FS).
    pub fn scan_listing<B: IndexMemoryDb>(
        &mut self,
        state: &mut IndexerState,
        backend: &mut B,
        root_id: u64,
        listing: &[(String, Vec<u8>, Option<u64>)],
        stats: &mut IndexStats,
        now_ns: u64,
    ) -> Result<ScanOutcome, IndexError> {
        if self.scanning {
            return Err(IndexError::ScanAlreadyRunning);
        }
        self.scanning = true;
        // Reconcile uncertain imports first (bounded).
        let _ = self.reconcile_pending(state, backend, stats, now_ns, 16);
        let result = self.scan_listing_inner(state, backend, root_id, listing, stats, now_ns);
        self.scanning = false;
        result
    }

    fn scan_listing_inner<B: IndexMemoryDb>(
        &mut self,
        state: &mut IndexerState,
        backend: &mut B,
        root_id: u64,
        listing: &[(String, Vec<u8>, Option<u64>)],
        stats: &mut IndexStats,
        now_ns: u64,
    ) -> Result<ScanOutcome, IndexError> {
        let root = state
            .roots
            .get(&root_id)
            .cloned()
            .ok_or(IndexError::RootNotFound)?;
        if !root.available {
            return Ok(ScanOutcome {
                roots_unavailable: 1,
                ..Default::default()
            });
        }

        // MemoryDB readiness gate — do not embed fallback.
        match backend.health() {
            Ok(h) if h.ready => {}
            Ok(_) | Err(_) => {
                IndexStats::sat_add(&mut stats.memorydb_unavailable_operations, 1);
                return Err(IndexError::DatabaseUnavailable);
            }
        }

        let mut outcome = ScanOutcome::default();
        let mut seen_paths: Vec<String> = Vec::new();
        let mut hashed = 0u32;
        let mut parsed = 0u32;
        let mut hash_bytes_total = 0u64;
        let mut parse_bytes_total = 0u64;
        let mut inserted = 0u32;

        let mut items = listing.to_vec();
        items.sort_by(|a, b| a.0.cmp(&b.0));

        for (rel, content, mtime) in &items {
            if self.ignore.is_ignored(rel, false) {
                IndexStats::sat_add(&mut stats.files_excluded, 1);
                continue;
            }
            seen_paths.push(rel.clone());
            IndexStats::sat_add(&mut stats.files_discovered, 1);

            if hashed >= self.config.quotas.max_files_hashed_per_scan
                || parsed >= self.config.quotas.max_files_parsed_per_scan
                || hash_bytes_total >= self.config.quotas.max_hash_bytes_per_scan
            {
                outcome.budget_exhausted = true;
                IndexStats::sat_add(&mut stats.scan_budget_exhaustions, 1);
                break;
            }

            let ext = crate::path_security::extension_of(rel).unwrap_or_default();
            if !root.allowed_extensions.contains(&ext) {
                IndexStats::sat_add(&mut stats.files_excluded, 1);
                continue;
            }

            if content.len() as u64 > root.maximum_file_size {
                // Oversized files still get a strong digest so rejection cache can
                // confirm them without re-reading as "new" after the first reject.
                let dig = hash_bytes(content);
                IndexStats::sat_add(&mut stats.files_hashed, 1);
                IndexStats::sat_add(&mut stats.strong_hash_files, 1);
                IndexStats::sat_add(&mut stats.strong_hash_bytes, content.len() as u64);
                IndexStats::sat_add(&mut stats.hash_bytes, content.len() as u64);
                hashed = hashed.saturating_add(1);
                hash_bytes_total = hash_bytes_total.saturating_add(content.len() as u64);
                let pv = self.pipeline_versions(PARSER_PLAIN);
                if let Some(existing) = state.get_by_path(root_id, rel).cloned() {
                    if existing.can_reuse_rejection(
                        &dig,
                        content.len() as u64,
                        &pv,
                        VALIDATOR_VERSION,
                    ) {
                        self.confirm_rejection(state, existing.source_id.get(), now_ns, stats);
                        outcome.files_processed = outcome.files_processed.saturating_add(1);
                        continue;
                    }
                }
                self.record_failure(
                    state,
                    &root,
                    rel,
                    SourceFailureKind::FileTooLarge,
                    now_ns,
                    stats,
                    Some(dig),
                    content.len() as u64,
                    *mtime,
                    Some(fast_fingerprint(content)),
                );
                continue;
            }

            let meta = FileMetaSnapshot {
                size_bytes: content.len() as u64,
                modified_at_ns: *mtime,
                identity: None,
            };

            // Size prefilter (mtime optional — native ABI has no mtime):
            // when size matches and a strong digest is already known, hash and
            // prove identity. Matching FNV never suppresses strong digest proof.
            if let Some(existing) = state.get_by_path(root_id, rel) {
                if existing.state == SourceState::Indexed
                    && existing.has_strong_digest()
                    && existing.size_bytes == meta.size_bytes
                {
                    // Optional fingerprint prefilter only; never final identity.
                    if let Some(ff) = existing.fast_fingerprint {
                        let _ = ff == fast_fingerprint(content);
                    }
                    let dig = hash_bytes(content);
                    IndexStats::sat_add(&mut stats.files_hashed, 1);
                    IndexStats::sat_add(&mut stats.strong_hash_files, 1);
                    IndexStats::sat_add(&mut stats.strong_hash_bytes, content.len() as u64);
                    IndexStats::sat_add(&mut stats.hash_bytes, content.len() as u64);
                    hashed = hashed.saturating_add(1);
                    hash_bytes_total = hash_bytes_total.saturating_add(content.len() as u64);
                    let pv = self.pipeline_versions(existing.parser_id);
                    if existing.can_skip_reparse(&dig, &pv) {
                        IndexStats::sat_add(&mut stats.files_unchanged, 1);
                        IndexStats::sat_add(&mut stats.strong_hash_unchanged, 1);
                        IndexStats::sat_add(&mut stats.metadata_fast_skips, 1);
                        IndexStats::sat_add(&mut stats.files_prefilter_skipped, 1);
                        if let Some(m) = state.sources.get_mut(&existing.source_id.get()) {
                            m.modified_at_ns = meta.modified_at_ns;
                        }
                        outcome.files_processed = outcome.files_processed.saturating_add(1);
                        continue;
                    }
                    // Fall through with dig already computed — recompute below is fine.
                }
            }

            let dig = hash_bytes(content);
            let ff = fast_fingerprint(content);
            IndexStats::sat_add(&mut stats.files_hashed, 1);
            IndexStats::sat_add(&mut stats.strong_hash_files, 1);
            IndexStats::sat_add(&mut stats.strong_hash_bytes, content.len() as u64);
            IndexStats::sat_add(&mut stats.hash_bytes, content.len() as u64);
            IndexStats::sat_add(&mut stats.bytes_read, content.len() as u64);
            hashed = hashed.saturating_add(1);
            hash_bytes_total = hash_bytes_total.saturating_add(content.len() as u64);

            // v1 → v2 digest upgrade: strong rehash without DB duplication when content
            // appears unchanged (legacy FNV match is a migration hint only; strong digest
            // becomes the stored identity after rehash).
            if let Some(existing) = state.get_by_path(root_id, rel).cloned() {
                if existing.needs_digest_upgrade {
                    IndexStats::sat_add(&mut stats.manifest_migrations_started, 1);
                    // A legacy match is only a migration hint. With no prior
                    // authoritative strong digest it cannot authorize
                    // Unchanged/SkipParsing/SkipTokenization, so this source
                    // must pass through the normal full import path.
                }
            }

            // Existing by path with same strong digest + pipeline → skip parse
            if let Some(existing) = state.get_by_path(root_id, rel).cloned() {
                let pv = self.pipeline_versions(existing.parser_id.max(PARSER_PLAIN));
                // Durable rejection cache: same digest + policy → no parse/tokenize.
                if existing.can_reuse_rejection(&dig, meta.size_bytes, &pv, VALIDATOR_VERSION) {
                    self.confirm_rejection(state, existing.source_id.get(), now_ns, stats);
                    outcome.files_processed = outcome.files_processed.saturating_add(1);
                    continue;
                }
                if existing.can_skip_reparse(&dig, &pv) {
                    // Metadata-only observation (including mtime absence): no generation.
                    if let Some(m) = state.sources.get_mut(&existing.source_id.get()) {
                        m.size_bytes = meta.size_bytes;
                        m.modified_at_ns = meta.modified_at_ns;
                        m.fast_fingerprint = Some(ff);
                        m.state = SourceState::Indexed;
                    }
                    IndexStats::sat_add(&mut stats.files_unchanged, 1);
                    IndexStats::sat_add(&mut stats.strong_hash_unchanged, 1);
                    outcome.files_processed = outcome.files_processed.saturating_add(1);
                    continue;
                }
                // Metadata changed but content same → update metadata only (no generation)
                if existing.has_strong_digest()
                    && existing.content_digest.equals(&dig)
                    && existing.pipeline_matches(&pv)
                    && matches!(
                        existing.state,
                        SourceState::Indexed | SourceState::Stable | SourceState::Changed
                    )
                {
                    if let Some(m) = state.sources.get_mut(&existing.source_id.get()) {
                        m.size_bytes = meta.size_bytes;
                        m.modified_at_ns = meta.modified_at_ns;
                        m.fast_fingerprint = Some(ff);
                        m.state = SourceState::Indexed;
                    }
                    IndexStats::sat_add(&mut stats.files_unchanged, 1);
                    IndexStats::sat_add(&mut stats.strong_hash_unchanged, 1);
                    outcome.files_processed = outcome.files_processed.saturating_add(1);
                    continue;
                }
            }

            // Rename detection
            let mut source_id_opt = state.get_by_path(root_id, rel).map(|m| m.source_id);
            let mut is_rename = false;
            if source_id_opt.is_none() {
                if let Some(cand) = state.find_rename_candidate(root_id, &dig, None) {
                    if let Some(m) = state.sources.get(&cand.get()) {
                        if m.state == SourceState::Missing || m.relative_path != *rel {
                            if listing.iter().all(|(p, _, _)| p != &m.relative_path) {
                                source_id_opt = Some(cand);
                                is_rename = true;
                                IndexStats::sat_add(&mut stats.files_renamed, 1);
                            }
                        }
                    }
                }
            }

            IndexStats::sat_add(&mut stats.files_validated, 1);
            let text = match validate_utf8_text(content, &self.config.quotas) {
                Ok(t) => t,
                Err(IndexError::InvalidUtf8) => {
                    self.record_failure(
                        state,
                        &root,
                        rel,
                        SourceFailureKind::InvalidUtf8,
                        now_ns,
                        stats,
                        Some(dig),
                        meta.size_bytes,
                        meta.modified_at_ns,
                        Some(ff),
                    );
                    continue;
                }
                Err(IndexError::BinaryContent) => {
                    self.record_failure(
                        state,
                        &root,
                        rel,
                        SourceFailureKind::BinaryContent,
                        now_ns,
                        stats,
                        Some(dig),
                        meta.size_bytes,
                        meta.modified_at_ns,
                        Some(ff),
                    );
                    continue;
                }
                Err(e) => {
                    self.record_failure(
                        state,
                        &root,
                        rel,
                        failure_from_error(&e),
                        now_ns,
                        stats,
                        Some(dig),
                        meta.size_bytes,
                        meta.modified_at_ns,
                        Some(ff),
                    );
                    continue;
                }
            };

            let parser = match select_parser(&ext) {
                Some(p) => p,
                None => {
                    self.record_failure(
                        state,
                        &root,
                        rel,
                        SourceFailureKind::UnsupportedFormat,
                        now_ns,
                        stats,
                        Some(dig),
                        meta.size_bytes,
                        meta.modified_at_ns,
                        Some(ff),
                    );
                    continue;
                }
            };

            if parsed >= self.config.quotas.max_files_parsed_per_scan
                || parse_bytes_total.saturating_add(content.len() as u64)
                    > self.config.quotas.max_parse_bytes_per_scan
            {
                outcome.budget_exhausted = true;
                IndexStats::sat_add(&mut stats.scan_budget_exhaustions, 1);
                break;
            }

            let normalized = normalize_newlines_owned(text);
            let mut blocks = Vec::new();
            match parser.parse(&normalized, &self.config.quotas, &mut blocks) {
                Ok(sum) => {
                    IndexStats::sat_add(&mut stats.blocks_parsed, sum.blocks as u64);
                    IndexStats::sat_add(&mut stats.bytes_parsed, sum.bytes_consumed);
                    IndexStats::sat_add(&mut stats.files_reparsed, 1);
                    parse_bytes_total = parse_bytes_total.saturating_add(sum.bytes_consumed);
                    parsed = parsed.saturating_add(1);
                }
                Err(_) => {
                    self.record_failure(
                        state,
                        &root,
                        rel,
                        SourceFailureKind::ParseFailed,
                        now_ns,
                        stats,
                        Some(dig),
                        meta.size_bytes,
                        meta.modified_at_ns,
                        Some(ff),
                    );
                    continue;
                }
            }

            let source_id = source_id_opt.unwrap_or_else(|| state.alloc_source_id());
            let prev = state.sources.get(&source_id.get()).cloned();
            // Content change supersedes a previous active document generation.
            let had_active_generation = prev
                .as_ref()
                .map(|m| m.document_memory_id.is_some() && m.source_revision > 0)
                .unwrap_or(false);
            let mut manifest = prev.unwrap_or_else(|| {
                let mut m = SourceManifest::new_v2(
                    source_id,
                    root_id,
                    root.scope,
                    root.owner,
                    rel.clone(),
                    IndexerState::path_hash(root_id, rel),
                    dig,
                    Some(ff),
                );
                m.state = SourceState::Stable;
                m
            });
            let was_legacy_digest_upgrade = manifest.needs_digest_upgrade;

            if is_rename {
                state.remove_path_binding(root_id, &manifest.relative_path);
                manifest.relative_path = rel.clone();
                manifest.canonical_path_hash = IndexerState::path_hash(root_id, rel);
            }

            let is_new = manifest.document_memory_id.is_none();
            if is_new {
                IndexStats::sat_add(&mut stats.files_new, 1);
            } else {
                IndexStats::sat_add(&mut stats.files_changed, 1);
            }

            manifest.manifest_version = SourceManifest::MANIFEST_VERSION;
            manifest.content_digest = dig;
            manifest.fast_fingerprint = Some(ff);
            manifest.needs_digest_upgrade = false;
            manifest.size_bytes = meta.size_bytes;
            manifest.modified_at_ns = meta.modified_at_ns;
            manifest.parser_id = parser.parser_id();
            manifest.parser_version = parser.parser_version();
            manifest.tokenizer_id = self.config.tokenizer_id;
            manifest.tokenizer_version = self.config.tokenizer_version;
            manifest.chunking_id = self.config.chunking_id;
            manifest.chunking_version = self.config.chunking_version;
            manifest.ignore_config_version = self.config.ignore_config_version;
            manifest.state = SourceState::Stable;
            manifest.failure = None;
            manifest.missing_confirmations = 0;

            let profile = ChunkingProfile {
                chunking_id: self.config.chunking_id,
                version: self.config.chunking_version,
                maximum_bytes: self.config.quotas.max_bytes_per_chunk,
                ..ChunkingProfile::default()
            };
            let chunks = match chunk_blocks(
                source_id,
                manifest.source_revision.saturating_add(1),
                parser.parser_id(),
                parser.parser_version(),
                &profile,
                &blocks,
                &self.config.quotas,
            ) {
                Ok(c) => c,
                Err(_) => {
                    self.record_failure(
                        state,
                        &root,
                        rel,
                        SourceFailureKind::QuotaExceeded,
                        now_ns,
                        stats,
                        Some(dig),
                        meta.size_bytes,
                        meta.modified_at_ns,
                        Some(ff),
                    );
                    continue;
                }
            };
            IndexStats::sat_add(&mut stats.chunks_created, chunks.len() as u64);

            let tokenizer = WiseOwlLexicalV1;
            let prepared = match prepare_chunks_from_text(
                chunks,
                &tokenizer,
                &mut self.dict,
                &self.config.quotas,
            ) {
                Ok(p) => {
                    IndexStats::sat_add(&mut stats.files_retokenized, 1);
                    p
                }
                Err(IndexError::TokenCollision) => {
                    IndexStats::sat_add(&mut stats.token_collisions_detected, 1);
                    self.record_failure(
                        state,
                        &root,
                        rel,
                        SourceFailureKind::TokenizationFailed,
                        now_ns,
                        stats,
                        Some(dig),
                        meta.size_bytes,
                        meta.modified_at_ns,
                        Some(ff),
                    );
                    continue;
                }
                Err(_) => {
                    self.record_failure(
                        state,
                        &root,
                        rel,
                        SourceFailureKind::TokenizationFailed,
                        now_ns,
                        stats,
                        Some(dig),
                        meta.size_bytes,
                        meta.modified_at_ns,
                        Some(ff),
                    );
                    continue;
                }
            };
            for pc in &prepared {
                IndexStats::sat_add(
                    &mut stats.tokens_emitted,
                    pc.tokens.iter().map(|t| t.frequency as u64).sum(),
                );
                IndexStats::sat_add(&mut stats.unique_tokens, pc.tokens.len() as u64);
            }

            // Persist pending import before transaction (crash window).
            let planned_rev = manifest.source_revision.saturating_add(1).max(1);
            let ik = build_import_key(
                source_id,
                planned_rev,
                dig,
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
            manifest.pending_import = Some(pending_for_manifest(&ik, now_ns, None));
            manifest.state = SourceState::ImportPending;
            // Write pending into state before commit.
            state.remove_path_binding(root_id, rel);
            state.insert_manifest(manifest.clone());
            backend.persist_prepared_import(&manifest)?;

            IndexStats::sat_add(&mut stats.database_transactions_started, 1);
            match ingest_source_atomic(
                backend,
                &manifest,
                rel,
                &prepared,
                &self.config.quotas,
                now_ns,
            ) {
                Ok(res) => {
                    IndexStats::sat_add(&mut stats.database_transactions_committed, 1);
                    if !res.already_committed {
                        IndexStats::sat_add(&mut stats.database_generations_created, 1);
                        if had_active_generation {
                            IndexStats::sat_add(&mut stats.database_generations_superseded, 1);
                        }
                    } else {
                        IndexStats::sat_add(&mut stats.imports_reconciled_committed, 1);
                    }
                    if let Some(m) = state.sources.get_mut(&source_id.get()) {
                        m.document_memory_id = Some(res.document_id.get());
                        m.source_revision = res.source_revision;
                        m.chunk_count = prepared.len() as u32;
                        m.indexed_at_ns = now_ns;
                        m.state = SourceState::Indexed;
                        m.pending_import = None;
                        m.content_digest = dig;
                        m.fast_fingerprint = Some(ff);
                    }
                    if was_legacy_digest_upgrade {
                        IndexStats::sat_add(&mut stats.manifest_migrations_completed, 1);
                        state.migrations_completed = state.migrations_completed.saturating_add(1);
                    }
                    IndexStats::sat_add(&mut stats.files_indexed, 1);
                    let _ = backend.clear_prepared_import(source_id);
                    inserted = inserted.saturating_add(1);
                }
                Err(IndexError::DatabaseUnavailable) => {
                    IndexStats::sat_add(&mut stats.database_transactions_aborted, 1);
                    IndexStats::sat_add(&mut stats.memorydb_unavailable_operations, 1);
                    // Leave pending_import for reconciliation.
                    return Err(IndexError::DatabaseUnavailable);
                }
                Err(_) => {
                    IndexStats::sat_add(&mut stats.database_transactions_aborted, 1);
                    if let Some(m) = state.sources.get_mut(&source_id.get()) {
                        m.pending_import = None;
                    }
                    self.record_failure(
                        state,
                        &root,
                        rel,
                        SourceFailureKind::TransactionRejected,
                        now_ns,
                        stats,
                        Some(dig),
                        meta.size_bytes,
                        meta.modified_at_ns,
                        Some(ff),
                    );
                }
            }
            outcome.files_processed = outcome.files_processed.saturating_add(1);

            if inserted >= self.config.quotas.max_records_inserted_per_scan {
                outcome.budget_exhausted = true;
                break;
            }
        }

        // Missing files — never mass-delete on root outage (handled above).
        if !outcome.budget_exhausted {
            let tracked: Vec<_> = state
                .sources
                .values()
                .filter(|m| m.root_id == root_id)
                .map(|m| (m.source_id, m.relative_path.clone(), m.state))
                .collect();
            let mut deletions = 0u32;
            for (sid, path, st) in tracked {
                if seen_paths.iter().any(|p| p == &path) {
                    if let Some(m) = state.sources.get_mut(&sid.get()) {
                        m.missing_confirmations = 0;
                    }
                    continue;
                }
                if matches!(
                    st,
                    SourceState::Indexed
                        | SourceState::Missing
                        | SourceState::DeletePending
                        | SourceState::Failed
                        | SourceState::Changed
                        | SourceState::Stable
                        | SourceState::Discovered
                        | SourceState::ImportPending
                ) {
                    IndexStats::sat_add(&mut stats.files_missing, 1);
                    if let Some(m) = state.sources.get_mut(&sid.get()) {
                        m.state = SourceState::Missing;
                        m.missing_confirmations = m.missing_confirmations.saturating_add(1);
                        if m.missing_confirmations
                            >= self.config.quotas.deletion_grace_confirmations
                        {
                            IndexStats::sat_add(&mut stats.files_missing_confirmed, 1);
                            m.state = SourceState::DeletePending;
                            if deletions < self.config.quotas.max_deletions_per_scan {
                                IndexStats::sat_add(&mut stats.source_delete_batches, 1);
                                IndexStats::sat_add(&mut stats.source_delete_requests, 1);
                                match delete_source_bounded(backend, sid, 16) {
                                    Ok((_n, _more)) => {
                                        IndexStats::sat_add(&mut stats.files_deleted, 1);
                                        IndexStats::sat_add(&mut stats.source_delete_commits, 1);
                                        state.sources_by_path.remove(&(root_id, path));
                                        state.sources.remove(&sid.get());
                                        deletions = deletions.saturating_add(1);
                                    }
                                    Err(_) => {}
                                }
                            }
                        }
                    }
                }
            }
        }

        state.last_successful_scan_ns = now_ns;
        stats.token_collisions_detected = self.dict.collisions_detected;
        stats.pending_imports = state.pending_import_count();
        Ok(outcome)
    }

    /// Confirm an unchanged permanent rejection without parse/tokenize.
    fn confirm_rejection(
        &self,
        state: &mut IndexerState,
        source_raw: u64,
        now_ns: u64,
        stats: &mut IndexStats,
    ) {
        IndexStats::sat_add(&mut stats.files_rejected_cached, 1);
        if let Some(man) = state.sources.get_mut(&source_raw) {
            man.state = SourceState::Failed;
            if let Some(ref mut f) = man.failure {
                f.latest_failure_ns = now_ns;
                f.attempt_count = f.attempt_count.saturating_add(1);
                f.confirmation_count = f.confirmation_count.saturating_add(1);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn record_failure(
        &self,
        state: &mut IndexerState,
        root: &crate::config::IndexRootConfig,
        rel: &str,
        kind: SourceFailureKind,
        now_ns: u64,
        stats: &mut IndexStats,
        content_digest: Option<ContentDigest>,
        size_bytes: u64,
        modified_at_ns: Option<u64>,
        fast_fingerprint: Option<crate::digest::FastFingerprint>,
    ) {
        IndexStats::sat_add(&mut stats.files_failed, 1);
        let root_id = root.root_id;
        let meta_hash = fnv1a64(rel.as_bytes());
        let dig = content_digest.unwrap_or_else(ContentDigest::unset);
        let pv = self.pipeline_versions(PARSER_PLAIN);

        if let Some(m) = state.get_by_path(root_id, rel).map(|x| x.source_id) {
            if let Some(man) = state.sources.get_mut(&m.get()) {
                // Same permanent rejection already recorded for this digest/policy.
                if dig.is_set()
                    && man.can_reuse_rejection(&dig, size_bytes, &pv, VALIDATOR_VERSION)
                    && man.failure.as_ref().map(|f| f.kind) == Some(kind)
                {
                    IndexStats::sat_add(&mut stats.files_rejected_cached, 1);
                    man.state = SourceState::Failed;
                    if let Some(ref mut f) = man.failure {
                        f.latest_failure_ns = now_ns;
                        f.attempt_count = f.attempt_count.saturating_add(1);
                        f.confirmation_count = f.confirmation_count.saturating_add(1);
                    }
                    return;
                }
                let attempt = man
                    .failure
                    .as_ref()
                    .map(|f| f.attempt_count.saturating_add(1))
                    .unwrap_or(1);
                let first = man
                    .failure
                    .as_ref()
                    .map(|f| f.first_failure_ns)
                    .unwrap_or(now_ns);
                if kind.is_permanent() {
                    IndexStats::sat_add(&mut stats.files_rejected_new, 1);
                }
                if dig.is_set() {
                    man.content_digest = dig;
                    man.needs_digest_upgrade = false;
                    man.manifest_version = SourceManifest::MANIFEST_VERSION;
                }
                man.scope = root.scope;
                man.owner = root.owner;
                man.size_bytes = size_bytes;
                man.modified_at_ns = modified_at_ns;
                man.fast_fingerprint = fast_fingerprint;
                man.parser_id = PARSER_PLAIN;
                man.parser_version = self.config.parser_version;
                man.tokenizer_id = self.config.tokenizer_id;
                man.tokenizer_version = self.config.tokenizer_version;
                man.chunking_id = self.config.chunking_id;
                man.chunking_version = self.config.chunking_version;
                man.ignore_config_version = self.config.ignore_config_version;
                man.failure = Some(SourceFailure {
                    kind,
                    first_failure_ns: first,
                    latest_failure_ns: now_ns,
                    attempt_count: attempt,
                    confirmation_count: 0,
                    metadata_hash: meta_hash,
                    retry_after_ns: if kind.is_permanent() {
                        u64::MAX
                    } else {
                        now_ns.saturating_add(60_000_000_000)
                    },
                    validator_version: VALIDATOR_VERSION,
                });
                man.state = SourceState::Failed;
            }
        } else {
            let sid = state.alloc_source_id();
            let mut m = SourceManifest::new_v2(
                sid,
                root_id,
                root.scope,
                root.owner,
                String::from(rel),
                IndexerState::path_hash(root_id, rel),
                dig,
                fast_fingerprint,
            );
            m.size_bytes = size_bytes;
            m.modified_at_ns = modified_at_ns;
            m.parser_id = PARSER_PLAIN;
            m.parser_version = self.config.parser_version;
            m.tokenizer_id = self.config.tokenizer_id;
            m.tokenizer_version = self.config.tokenizer_version;
            m.chunking_id = self.config.chunking_id;
            m.chunking_version = self.config.chunking_version;
            m.ignore_config_version = self.config.ignore_config_version;
            m.state = SourceState::Failed;
            if kind.is_permanent() {
                IndexStats::sat_add(&mut stats.files_rejected_new, 1);
            }
            m.failure = Some(SourceFailure {
                kind,
                first_failure_ns: now_ns,
                latest_failure_ns: now_ns,
                attempt_count: 1,
                confirmation_count: 0,
                metadata_hash: meta_hash,
                retry_after_ns: if kind.is_permanent() {
                    u64::MAX
                } else {
                    now_ns.saturating_add(60_000_000_000)
                },
                validator_version: VALIDATOR_VERSION,
            });
            state.insert_manifest(m);
        }
    }
}

fn failure_from_error(e: &IndexError) -> SourceFailureKind {
    match e {
        IndexError::InvalidUtf8 => SourceFailureKind::InvalidUtf8,
        IndexError::BinaryContent => SourceFailureKind::BinaryContent,
        IndexError::FileTooLarge { .. } => SourceFailureKind::FileTooLarge,
        IndexError::ChangedDuringRead => SourceFailureKind::ChangedDuringRead,
        IndexError::UnsupportedFormat => SourceFailureKind::UnsupportedFormat,
        IndexError::QuotaExceeded(_) => SourceFailureKind::QuotaExceeded,
        IndexError::DatabaseUnavailable => SourceFailureKind::DatabaseUnavailable,
        _ => SourceFailureKind::ParseFailed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{BoundedExtensionSet, IndexRootConfig};
    use crate::memorydb_backend::HostMemoryDbBackend;
    use wiseowl_memorydb::database::{Database, DbCaller, MemoryStore};
    use wiseowl_memorydb::record::MemoryScope;
    use wiseowl_memorydb::DbQuotaConfig;

    fn setup() -> (ScanEngine, IndexerState, HostMemoryDbBackend<MemoryStore>) {
        let config = IndexerConfig::default();
        let mut state = IndexerState::new();
        let root = IndexRootConfig {
            root_id: 1,
            path: String::from("/virtual/docs"),
            scope: MemoryScope::User,
            owner: 1,
            enabled: true,
            recursive: true,
            maximum_depth: 8,
            follow_symlinks: false,
            stay_on_filesystem: true,
            include_hidden: false,
            maximum_file_size: 48 * 1024,
            allowed_extensions: BoundedExtensionSet::default_phase3(),
            available: true,
        };
        state.roots.insert(1, root);
        let engine = ScanEngine::new(config);
        let db = Database::open_memory(DbQuotaConfig::default()).unwrap();
        let backend = HostMemoryDbBackend::new(db, DbCaller::user(1));
        (engine, state, backend)
    }

    #[test]
    fn new_file_indexed_and_unchanged_skipped() {
        let (mut engine, mut state, mut backend) = setup();
        let mut stats = IndexStats::default();
        let listing = vec![(
            String::from("a.txt"),
            b"hello thermal fan".to_vec(),
            Some(1000),
        )];
        engine
            .scan_listing(&mut state, &mut backend, 1, &listing, &mut stats, 5000)
            .unwrap();
        assert_eq!(stats.files_indexed, 1);
        assert_eq!(stats.files_new, 1);
        assert!(stats.strong_hash_files >= 1);
        assert_eq!(stats.database_generations_created, 1);

        let mut stats2 = IndexStats::default();
        engine
            .scan_listing(&mut state, &mut backend, 1, &listing, &mut stats2, 6000)
            .unwrap();
        assert_eq!(stats2.files_unchanged, 1);
        assert_eq!(stats2.files_indexed, 0);
        assert_eq!(stats2.files_reparsed, 0);
        assert_eq!(stats2.files_retokenized, 0);
        assert_eq!(stats2.database_generations_created, 0);
    }

    #[test]
    fn mtime_only_no_new_generation() {
        let (mut engine, mut state, mut backend) = setup();
        let mut stats = IndexStats::default();
        let body = b"stable content".to_vec();
        let listing = vec![(String::from("a.txt"), body.clone(), Some(1))];
        engine
            .scan_listing(&mut state, &mut backend, 1, &listing, &mut stats, 1)
            .unwrap();
        let listing2 = vec![(String::from("a.txt"), body, Some(999))]; // mtime only
        let mut stats2 = IndexStats::default();
        engine
            .scan_listing(&mut state, &mut backend, 1, &listing2, &mut stats2, 2)
            .unwrap();
        assert_eq!(stats2.files_unchanged, 1);
        assert_eq!(stats2.files_reparsed, 0);
        assert_eq!(stats2.database_generations_created, 0);
    }

    #[test]
    fn content_change_reindexes() {
        let (mut engine, mut state, mut backend) = setup();
        let mut stats = IndexStats::default();
        let listing = vec![(String::from("a.txt"), b"one".to_vec(), Some(1))];
        engine
            .scan_listing(&mut state, &mut backend, 1, &listing, &mut stats, 1)
            .unwrap();
        let listing2 = vec![(String::from("a.txt"), b"two changed".to_vec(), Some(2))];
        let mut stats2 = IndexStats::default();
        engine
            .scan_listing(&mut state, &mut backend, 1, &listing2, &mut stats2, 2)
            .unwrap();
        assert_eq!(stats2.files_changed, 1);
        assert_eq!(stats2.files_indexed, 1);
        assert_eq!(stats2.database_generations_created, 1);
    }

    #[test]
    fn copy_retains_separate_sources() {
        let (mut engine, mut state, mut backend) = setup();
        let mut stats = IndexStats::default();
        let body = b"same content".to_vec();
        let listing = vec![
            (String::from("a.txt"), body.clone(), Some(1)),
            (String::from("b.txt"), body, Some(1)),
        ];
        engine
            .scan_listing(&mut state, &mut backend, 1, &listing, &mut stats, 1)
            .unwrap();
        assert_eq!(stats.files_indexed, 2);
        assert_eq!(state.sources.len(), 2);
    }

    #[test]
    fn unavailable_root_does_not_delete() {
        let (mut engine, mut state, mut backend) = setup();
        let mut stats = IndexStats::default();
        let listing = vec![(String::from("a.txt"), b"hello".to_vec(), Some(1))];
        engine
            .scan_listing(&mut state, &mut backend, 1, &listing, &mut stats, 1)
            .unwrap();
        state.roots.get_mut(&1).unwrap().available = false;
        let mut stats2 = IndexStats::default();
        let out = engine
            .scan_listing(&mut state, &mut backend, 1, &[], &mut stats2, 2)
            .unwrap();
        assert_eq!(out.roots_unavailable, 1);
        assert_eq!(stats2.files_deleted, 0);
        assert!(!state.sources.is_empty());
    }

    #[test]
    fn binary_rejected() {
        let (mut engine, mut state, mut backend) = setup();
        let mut stats = IndexStats::default();
        let mut bin = vec![0u8; 64];
        for i in 0..64 {
            bin[i] = i as u8;
        }
        let listing = vec![(String::from("a.txt"), bin, Some(1))];
        engine
            .scan_listing(&mut state, &mut backend, 1, &listing, &mut stats, 1)
            .unwrap();
        assert!(stats.files_failed >= 1);
        assert_eq!(stats.files_indexed, 0);
        assert!(stats.files_rejected_new >= 1);
    }

    #[test]
    fn rejected_fixture_cached_no_reparse() {
        let (mut engine, mut state, mut backend) = setup();
        let mut stats = IndexStats::default();
        let mut bin = vec![0u8; 64];
        for i in 0..64 {
            bin[i] = i as u8;
        }
        let listing = vec![
            (String::from("ok.txt"), b"hello wiseowl".to_vec(), None),
            (String::from("bad.txt"), bin.clone(), None),
            (String::from("utf8.txt"), vec![0xff, 0xfe, 0xfd], None),
        ];
        engine
            .scan_listing(&mut state, &mut backend, 1, &listing, &mut stats, 1)
            .unwrap();
        assert_eq!(stats.files_indexed, 1);
        assert!(stats.files_rejected_new >= 2);

        let mut stats2 = IndexStats::default();
        engine
            .scan_listing(&mut state, &mut backend, 1, &listing, &mut stats2, 2)
            .unwrap();
        assert_eq!(stats2.files_reparsed, 0);
        assert_eq!(stats2.files_retokenized, 0);
        assert_eq!(stats2.database_generations_created, 0);
        assert!(stats2.files_rejected_cached >= 2);
        assert_eq!(stats2.files_rejected_new, 0);
        assert_eq!(stats2.files_unchanged, 1);
    }

    #[test]
    fn content_change_exactly_one_generation_and_supersession() {
        let (mut engine, mut state, mut backend) = setup();
        let mut stats = IndexStats::default();
        let listing = vec![(String::from("a.txt"), b"one".to_vec(), None)];
        engine
            .scan_listing(&mut state, &mut backend, 1, &listing, &mut stats, 1)
            .unwrap();
        assert_eq!(stats.database_generations_created, 1);
        assert_eq!(stats.database_generations_superseded, 0);

        let mut stats2 = IndexStats::default();
        let listing2 = vec![(String::from("a.txt"), b"onX".to_vec(), None)];
        engine
            .scan_listing(&mut state, &mut backend, 1, &listing2, &mut stats2, 2)
            .unwrap();
        assert_eq!(stats2.files_reparsed, 1);
        assert_eq!(stats2.files_retokenized, 1);
        assert_eq!(stats2.database_generations_created, 1);
        assert_eq!(stats2.database_generations_superseded, 1);
    }

    #[test]
    fn rejected_becomes_valid_then_stable() {
        let (mut engine, mut state, mut backend) = setup();
        let mut stats = IndexStats::default();
        let listing = vec![(String::from("flip.txt"), vec![0, 1, 2, 3], None)];
        engine
            .scan_listing(&mut state, &mut backend, 1, &listing, &mut stats, 1)
            .unwrap();
        assert!(stats.files_rejected_new >= 1);
        let m1 = state.get_by_path(1, "flip.txt").unwrap();
        assert_eq!(m1.state, SourceState::Failed);
        assert!(m1.has_strong_digest());

        let mut stats2 = IndexStats::default();
        let listing2 = vec![(String::from("flip.txt"), b"now valid text".to_vec(), None)];
        engine
            .scan_listing(&mut state, &mut backend, 1, &listing2, &mut stats2, 2)
            .unwrap();
        assert_eq!(stats2.files_reparsed, 1);
        assert_eq!(stats2.files_retokenized, 1);
        assert_eq!(stats2.database_generations_created, 1);
        assert_eq!(
            state.get_by_path(1, "flip.txt").unwrap().state,
            SourceState::Indexed
        );

        let mut stats3 = IndexStats::default();
        engine
            .scan_listing(&mut state, &mut backend, 1, &listing2, &mut stats3, 3)
            .unwrap();
        assert_eq!(stats3.files_reparsed, 0);
        assert_eq!(stats3.files_retokenized, 0);
        assert_eq!(stats3.database_generations_created, 0);
    }

    #[test]
    fn strong_digest_not_fnv() {
        let body = b"identity proof";
        let dig = digest_bytes(body);
        let fnv = fnv1a64(body);
        // Digests are 32-byte SHA-256, not u64 FNV.
        assert!(dig.is_set());
        assert_ne!(dig.bytes[0..8], fnv.to_le_bytes());
    }

    #[test]
    fn v1_manifest_upgrade_preserves_source_id() {
        let (mut engine, mut state, mut backend) = setup();
        // Simulate a v1-style indexed manifest needing upgrade.
        let sid = state.alloc_source_id();
        let body = b"legacy content";
        let mut m = SourceManifest::new_v2(
            sid,
            1,
            MemoryScope::User,
            1,
            String::from("old.txt"),
            IndexerState::path_hash(1, "old.txt"),
            ContentDigest::unset(),
            None,
        )
        .mark_for_digest_upgrade(fnv1a64(body));
        m.state = SourceState::Indexed;
        m.document_memory_id = Some(99);
        m.source_revision = 1;
        m.parser_id = 1;
        m.parser_version = engine.config.parser_version;
        m.tokenizer_id = engine.config.tokenizer_id;
        m.tokenizer_version = engine.config.tokenizer_version;
        m.chunking_id = engine.config.chunking_id;
        m.chunking_version = engine.config.chunking_version;
        m.ignore_config_version = engine.config.ignore_config_version;
        m.size_bytes = body.len() as u64;
        m.modified_at_ns = Some(1);
        state.insert_manifest(m);

        let mut stats = IndexStats::default();
        let listing = vec![(String::from("old.txt"), body.to_vec(), Some(1))];
        engine
            .scan_listing(&mut state, &mut backend, 1, &listing, &mut stats, 10)
            .unwrap();
        let man = state.get_by_path(1, "old.txt").unwrap();
        assert_eq!(man.source_id, sid);
        assert!(man.has_strong_digest());
        assert!(!man.needs_digest_upgrade);
        assert_eq!(stats.manifest_migrations_completed, 1);
        // A legacy hash cannot authorize an unchanged result. The first strong
        // verification therefore creates one replacement generation.
        assert_eq!(stats.database_generations_created, 1);
        assert_eq!(stats.files_reparsed, 1);
        assert_eq!(stats.files_retokenized, 1);
    }

    #[test]
    fn injected_legacy_collision_cannot_skip_or_confirm_rename() {
        let (mut engine, mut state, mut backend) = setup();
        let sid = state.alloc_source_id();
        let old = b"old payload";
        let new = b"different payload";
        let injected_collision = fnv1a64(new);
        let mut m = SourceManifest::new_v2(
            sid,
            1,
            MemoryScope::User,
            1,
            String::from("old.txt"),
            IndexerState::path_hash(1, "old.txt"),
            ContentDigest::unset(),
            None,
        )
        .mark_for_digest_upgrade(injected_collision);
        m.state = SourceState::Indexed;
        m.document_memory_id = Some(99);
        m.source_revision = 1;
        m.parser_id = 1;
        m.parser_version = engine.config.parser_version;
        m.tokenizer_id = engine.config.tokenizer_id;
        m.tokenizer_version = engine.config.tokenizer_version;
        m.chunking_id = engine.config.chunking_id;
        m.chunking_version = engine.config.chunking_version;
        m.ignore_config_version = engine.config.ignore_config_version;
        m.size_bytes = old.len() as u64;
        state.insert_manifest(m);

        let mut stats = IndexStats::default();
        engine
            .scan_listing(
                &mut state,
                &mut backend,
                1,
                &[(String::from("new.txt"), new.to_vec(), Some(2))],
                &mut stats,
                20,
            )
            .unwrap();
        assert_eq!(stats.files_unchanged, 0);
        assert_eq!(stats.files_renamed, 0);
        assert_eq!(stats.files_reparsed, 1);
        assert_ne!(state.get_by_path(1, "new.txt").unwrap().source_id, sid);
    }
}
