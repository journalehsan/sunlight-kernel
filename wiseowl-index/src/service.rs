//! Indexer service orchestration (roots, scan, search, capabilities).

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use wiseowl_memory::SourceId;
use wiseowl_memorydb::query::MemoryQuery;
use wiseowl_memorydb::record::MemoryScope;
use wiseowl_memorydb::tokens::{TokenMatchMode, TokenQuery};

use crate::caps::{IndexCapability, IndexCapabilitySet};
use crate::config::{
    documents_path_under_home, BoundedExtensionSet, IndexRootConfig, IndexerConfig,
};
use crate::digest::{ContentDigest, CONTENT_DIGEST_FORMAT_VERSION};
use crate::error::IndexError;
use crate::health::{DegradedReason, HealthState, IndexHealth};
use crate::ingest::delete_source_bounded;
use crate::memorydb_backend::{HostMemoryDbBackend, IndexMemoryDb, MemoryDbHealth};
use crate::protocol::{SearchHit, SourceListItem, TokenWire, TransportInfo};
use crate::scan::ScanEngine;
use crate::source::SourceState;
use crate::state::IndexerState;
use crate::stats::IndexStats;
use crate::tokenize::{
    NormalizedTextBuffer, RetrievalTokenizer, TokenDictionary, TokenSink, WiseOwlLexicalV1,
};
use wiseowl_memorydb::database::{Database, DbCaller, DurableStore};

/// Caller identity for capability and ownership checks.
#[derive(Debug, Clone)]
pub struct IndexCaller {
    pub caps: IndexCapabilitySet,
    pub owner: u64,
    pub is_system: bool,
}

impl IndexCaller {
    pub fn admin() -> Self {
        Self {
            caps: IndexCapabilitySet::admin(),
            owner: 0,
            is_system: true,
        }
    }

    pub fn user(owner: u64) -> Self {
        Self {
            caps: IndexCapabilitySet::default_client(),
            owner,
            is_system: false,
        }
    }
}

/// Bounded reconnect policy for MemoryDB discovery.
#[derive(Debug, Clone)]
pub struct ReconnectPolicy {
    pub max_attempts_per_interval: u32,
    pub base_backoff_ms: u64,
    pub max_backoff_ms: u64,
    pub attempts_this_interval: u32,
    pub next_attempt_ns: u64,
}

impl Default for ReconnectPolicy {
    fn default() -> Self {
        Self {
            max_attempts_per_interval: 8,
            base_backoff_ms: 50,
            max_backoff_ms: 5_000,
            attempts_this_interval: 0,
            next_attempt_ns: 0,
        }
    }
}

/// In-process indexer service (host tests and daemon core).
///
/// Generic over [`IndexMemoryDb`] so host and native share one engine.
pub struct IndexerService<B: IndexMemoryDb> {
    pub state: IndexerState,
    pub engine: ScanEngine,
    pub backend: B,
    pub stats: IndexStats,
    pub health: IndexHealth,
    pub now_ns: u64,
    pub reconnect: ReconnectPolicy,
    /// Virtual listing mode content (host path fills from FS).
    pub virtual_roots: alloc::collections::BTreeMap<u64, Vec<(String, Vec<u8>, Option<u64>)>>,
}

impl<S: DurableStore> IndexerService<HostMemoryDbBackend<S>> {
    pub fn new_host(db: Database<S>, config: IndexerConfig) -> Self {
        let backend = HostMemoryDbBackend::new(db, DbCaller::user(1));
        Self::with_backend(backend, config)
    }
}

impl<B: IndexMemoryDb> IndexerService<B> {
    pub fn with_backend(backend: B, config: IndexerConfig) -> Self {
        let mut stats = IndexStats::default();
        stats.configured_roots = config.roots.len() as u64;
        let mut state = IndexerState::new();
        for r in &config.roots {
            state.roots.insert(r.root_id, r.clone());
            if r.root_id >= state.next_root_id {
                state.next_root_id = r.root_id.saturating_add(1);
            }
        }
        let mut health = IndexHealth::default();
        health.content_digest_label = alloc::format!(
            "SHA-256 v{}",
            CONTENT_DIGEST_FORMAT_VERSION
        );
        health.manifest_format = 2;
        Self {
            state,
            engine: ScanEngine::new(config),
            backend,
            stats,
            health,
            now_ns: 1,
            reconnect: ReconnectPolicy::default(),
            virtual_roots: alloc::collections::BTreeMap::new(),
        }
    }

    pub fn set_now_ns(&mut self, ns: u64) {
        self.now_ns = ns.max(1);
    }

    /// Probe MemoryDB and update health (Ready vs Degraded:MemoryDbUnavailable).
    pub fn refresh_memorydb_health(&mut self) {
        IndexStats::sat_add(&mut self.stats.memorydb_connection_attempts, 1);
        match self.backend.health() {
            Ok(h) if h.ready => {
                IndexStats::sat_add(&mut self.stats.memorydb_connection_successes, 1);
                self.health.memorydb_connection = String::from("Ready");
                self.health.memorydb_generation = h.database_generation;
                self.health.clear_reason(DegradedReason::MemoryDbUnavailable);
                if self.health.state == HealthState::Starting {
                    self.health.state = HealthState::Ready;
                    self.health.ready = true;
                }
                self.reconnect.attempts_this_interval = 0;
            }
            Ok(h) => {
                self.health.memorydb_connection = h.state;
                self.health.memorydb_generation = h.database_generation;
                self.health
                    .set_degraded(DegradedReason::MemoryDbRecovering);
            }
            Err(_) => {
                IndexStats::sat_add(&mut self.stats.memorydb_disconnects, 1);
                self.health.memorydb_connection = String::from("Unavailable");
                self.health
                    .set_degraded(DegradedReason::MemoryDbUnavailable);
            }
        }
        self.health.pending_imports = self.state.pending_import_count();
    }

    /// Bounded MemoryDB reconnect attempt (no busy loop).
    pub fn try_reconnect_memorydb(&mut self) -> bool {
        if self.now_ns < self.reconnect.next_attempt_ns {
            return false;
        }
        if self.reconnect.attempts_this_interval >= self.reconnect.max_attempts_per_interval {
            return false;
        }
        self.reconnect.attempts_this_interval =
            self.reconnect.attempts_this_interval.saturating_add(1);
        let backoff_ms = self
            .reconnect
            .base_backoff_ms
            .saturating_mul(1u64 << self.reconnect.attempts_this_interval.min(6))
            .min(self.reconnect.max_backoff_ms);
        self.reconnect.next_attempt_ns = self
            .now_ns
            .saturating_add(backoff_ms.saturating_mul(1_000_000));
        IndexStats::sat_add(&mut self.stats.memorydb_reconnects, 1);
        self.refresh_memorydb_health();
        self.health.memorydb_connection == "Ready"
    }

    pub fn transport_info(&self) -> TransportInfo {
        TransportInfo {
            indexer_endpoint: String::from(crate::protocol::ENDPOINT_NAME),
            memorydb_endpoint: String::from(wiseowl_memorydb::ENDPOINT_NAME),
            memorydb_generation: self.health.memorydb_generation,
            connection: self.health.memorydb_connection.clone(),
            ipc_protocol: String::from("v1"),
            shm: String::from("Available"),
            content_digest: self.health.content_digest_label.clone(),
            manifest_format: self.health.manifest_format,
            pending_imports: self.health.pending_imports,
        }
    }

    pub fn register_root(
        &mut self,
        caller: &IndexCaller,
        path: String,
        recursive: bool,
        maximum_depth: u16,
    ) -> Result<u64, IndexError> {
        caller.caps.require(IndexCapability::RegisterRoot)?;
        if path.is_empty() || path.contains('\0') || path.contains("..") {
            return Err(IndexError::PathRejected("root path"));
        }
        let owned_count = self
            .state
            .roots
            .values()
            .filter(|r| r.owner == caller.owner)
            .count() as u16;
        if owned_count >= self.engine.config.quotas.max_roots_per_user && !caller.is_system {
            return Err(IndexError::QuotaExceeded("roots per user"));
        }
        let depth = if maximum_depth == 0 {
            self.engine.config.quotas.max_traversal_depth.min(8)
        } else {
            maximum_depth.min(self.engine.config.quotas.max_traversal_depth)
        };
        let root_id = self.state.alloc_root_id();
        let root = IndexRootConfig {
            root_id,
            path,
            scope: MemoryScope::User,
            owner: caller.owner,
            enabled: true,
            recursive,
            maximum_depth: depth,
            follow_symlinks: false,
            stay_on_filesystem: true,
            include_hidden: false,
            maximum_file_size: self.engine.config.quotas.max_file_size_bytes,
            allowed_extensions: BoundedExtensionSet::default_phase3(),
            available: true,
        };
        root.validate(&self.engine.config.quotas)?;
        self.state.roots.insert(root_id, root);
        self.stats.configured_roots = self.state.roots.len() as u64;
        self.stats.available_roots = self.state.roots.values().filter(|r| r.available).count() as u64;
        Ok(root_id)
    }

    pub fn maybe_register_documents_root(
        &mut self,
        caller: &IndexCaller,
        home: &str,
    ) -> Result<Option<u64>, IndexError> {
        if !self.engine.config.default_documents_root {
            return Ok(None);
        }
        let path = documents_path_under_home(home)?;
        let id = self.register_root(caller, path, true, 12)?;
        Ok(Some(id))
    }

    pub fn remove_root(&mut self, caller: &IndexCaller, root_id: u64) -> Result<(), IndexError> {
        caller.caps.require(IndexCapability::RegisterRoot)?;
        let root = self
            .state
            .roots
            .get(&root_id)
            .ok_or(IndexError::RootNotFound)?;
        if root.owner != caller.owner && !caller.is_system {
            return Err(IndexError::Unauthorized("root owner"));
        }
        self.state.roots.remove(&root_id);
        self.stats.configured_roots = self.state.roots.len() as u64;
        Ok(())
    }

    pub fn list_roots(&self, caller: &IndexCaller) -> Result<Vec<IndexRootConfig>, IndexError> {
        caller.caps.require(IndexCapability::ListRoots)?;
        Ok(self
            .state
            .roots
            .values()
            .filter(|r| r.owner == caller.owner || caller.is_system)
            .cloned()
            .collect())
    }

    pub fn start_scan(
        &mut self,
        caller: &IndexCaller,
        root_id: Option<u64>,
    ) -> Result<(), IndexError> {
        if self.engine.scanning {
            return Err(IndexError::ScanAlreadyRunning);
        }
        caller.caps.require(IndexCapability::ScanOwnRoots)?;

        self.refresh_memorydb_health();
        if self.health.memorydb_connection != "Ready" {
            IndexStats::sat_add(&mut self.stats.memorydb_unavailable_operations, 1);
            // Degraded: control ops work; indexing pauses.
            return Err(IndexError::DatabaseUnavailable);
        }

        self.health.state = HealthState::Scanning;
        IndexStats::sat_add(&mut self.stats.active_scans, 1);

        let roots: Vec<u64> = match root_id {
            Some(id) => {
                let r = self
                    .state
                    .roots
                    .get(&id)
                    .ok_or(IndexError::RootNotFound)?;
                if r.owner != caller.owner && !caller.is_system {
                    return Err(IndexError::Unauthorized("root"));
                }
                alloc::vec![id]
            }
            None => self
                .state
                .roots
                .values()
                .filter(|r| r.owner == caller.owner || caller.is_system)
                .map(|r| r.root_id)
                .collect(),
        };

        for rid in roots {
            let listing = self.virtual_roots.get(&rid).cloned().unwrap_or_default();
            #[cfg(feature = "host")]
            let listing = {
                if listing.is_empty() {
                    self.load_host_listing(rid).unwrap_or(listing)
                } else {
                    listing
                }
            };
            match self.engine.scan_listing(
                &mut self.state,
                &mut self.backend,
                rid,
                &listing,
                &mut self.stats,
                self.now_ns,
            ) {
                Ok(_) => {}
                Err(IndexError::DatabaseUnavailable) => {
                    self.health
                        .set_degraded(DegradedReason::MemoryDbUnavailable);
                    break;
                }
                Err(_) => {}
            }
        }

        self.stats.active_scans = self.stats.active_scans.saturating_sub(1);
        if self.health.state == HealthState::Scanning {
            self.health.state = HealthState::Ready;
        }
        self.stats.sources_tracked = self.state.sources.len() as u64;
        self.health.pending_imports = self.state.pending_import_count();
        Ok(())
    }

    #[cfg(feature = "host")]
    fn load_host_listing(
        &self,
        root_id: u64,
    ) -> Result<Vec<(String, Vec<u8>, Option<u64>)>, IndexError> {
        use crate::discover::{discover_files_host, DiscoverBudget, ScanCursor};
        use crate::path_security::join_under_root;
        use crate::stable_file::stable_read_host;

        let root = self
            .state
            .roots
            .get(&root_id)
            .ok_or(IndexError::RootNotFound)?;
        let mut budget = DiscoverBudget::default();
        let cursor = ScanCursor::default();
        let (files, _) = discover_files_host(
            root,
            &self.engine.ignore,
            &self.engine.config.quotas,
            &cursor,
            &mut budget,
        )?;
        let mut out = Vec::new();
        for f in files {
            let full = join_under_root(&root.path, &f.relative_path)?;
            match stable_read_host(&full, root.maximum_file_size, &self.engine.config.quotas) {
                Ok(sr) => out.push((f.relative_path, sr.bytes, sr.meta.modified_at_ns)),
                Err(_) => continue,
            }
        }
        Ok(out)
    }

    pub fn list_sources(
        &self,
        caller: &IndexCaller,
        offset: u32,
        limit: u32,
    ) -> Result<(Vec<SourceListItem>, bool), IndexError> {
        caller
            .caps
            .require(IndexCapability::InspectSourceMetadata)?;
        let limit = limit
            .min(self.engine.config.quotas.max_source_list_results)
            .max(1);
        let mut items: Vec<_> = self
            .state
            .sources
            .values()
            .filter(|m| m.owner == caller.owner || caller.is_system || m.owner == 0)
            .collect();
        items.sort_by_key(|m| m.source_id.get());
        let start = offset as usize;
        if start >= items.len() {
            return Ok((Vec::new(), false));
        }
        let end = (start + limit as usize).min(items.len());
        let more = end < items.len();
        let page = items[start..end]
            .iter()
            .map(|m| SourceListItem {
                source_id: m.source_id.get(),
                root_id: m.root_id,
                relative_path: if caller.is_system
                    || caller.caps.has(IndexCapability::ReadSourceFile)
                {
                    m.relative_path.clone()
                } else {
                    String::from("(redacted)")
                },
                state: String::from(m.state.as_str()),
                content_digest_hex: if m.has_strong_digest() {
                    m.content_digest.to_hex()
                } else {
                    String::from("")
                },
                fast_fingerprint: m.fast_fingerprint.unwrap_or(0),
                chunk_count: m.chunk_count,
                manifest_version: m.manifest_version,
            })
            .collect();
        Ok((page, more))
    }

    pub fn inspect_source(
        &self,
        caller: &IndexCaller,
        source_id: u64,
    ) -> Result<crate::source::SourceManifest, IndexError> {
        caller
            .caps
            .require(IndexCapability::InspectSourceMetadata)?;
        let m = self
            .state
            .sources
            .get(&source_id)
            .ok_or(IndexError::SourceNotFound)?;
        if m.owner != caller.owner && !caller.is_system {
            return Err(IndexError::Unauthorized("source owner"));
        }
        Ok(m.clone())
    }

    pub fn digest_info(
        &self,
        caller: &IndexCaller,
        source_id: u64,
    ) -> Result<(ContentDigest, u32, u16), IndexError> {
        let m = self.inspect_source(caller, source_id)?;
        Ok((m.content_digest, m.source_revision, m.manifest_version))
    }

    pub fn forget_source(
        &mut self,
        caller: &IndexCaller,
        source_id: u64,
        dry_run: bool,
    ) -> Result<(u32, bool), IndexError> {
        caller.caps.require(IndexCapability::RemoveSource)?;
        let sid = SourceId::from_raw(source_id).map_err(|_| IndexError::SourceNotFound)?;
        let m = self
            .state
            .sources
            .get(&source_id)
            .ok_or(IndexError::SourceNotFound)?;
        if m.owner != caller.owner && !caller.is_system {
            return Err(IndexError::Unauthorized("source owner"));
        }
        if dry_run {
            let n = self.backend.delete_source_dry_run(sid)?;
            return Ok((n, false));
        }
        let (n, more) = delete_source_bounded(&mut self.backend, sid, 32)?;
        if !more {
            if let Some(man) = self.state.sources.remove(&source_id) {
                self.state
                    .sources_by_path
                    .remove(&(man.root_id, man.relative_path));
            }
        }
        IndexStats::sat_add(&mut self.stats.files_deleted, n as u64);
        Ok((n, more))
    }

    pub fn reindex_source(
        &mut self,
        caller: &IndexCaller,
        source_id: u64,
    ) -> Result<(), IndexError> {
        caller.caps.require(IndexCapability::ReindexSource)?;
        let m = self
            .state
            .sources
            .get_mut(&source_id)
            .ok_or(IndexError::SourceNotFound)?;
        if m.owner != caller.owner && !caller.is_system {
            return Err(IndexError::Unauthorized("source owner"));
        }
        // Force re-index by clearing strong digest match.
        m.content_digest = ContentDigest::unset();
        m.state = SourceState::Changed;
        let root_id = m.root_id;
        self.start_scan(caller, Some(root_id))
    }

    pub fn reconcile(&mut self, caller: &IndexCaller) -> Result<u32, IndexError> {
        caller.caps.require(IndexCapability::AdminIndexer)?;
        self.refresh_memorydb_health();
        if self.health.memorydb_connection != "Ready" {
            return Err(IndexError::DatabaseUnavailable);
        }
        self.engine.reconcile_pending(
            &mut self.state,
            &mut self.backend,
            &mut self.stats,
            self.now_ns,
            32,
        )
    }

    pub fn pending_count(&self) -> u64 {
        self.state.pending_import_count()
    }

    pub fn tokenize_text(
        &mut self,
        caller: &IndexCaller,
        text: &str,
    ) -> Result<(u32, u32, Vec<TokenWire>), IndexError> {
        caller.caps.require(IndexCapability::TokenizeQuery)?;
        if text.len() > 4096 {
            return Err(IndexError::QuotaExceeded("query text"));
        }
        let tok = WiseOwlLexicalV1;
        let mut norm = NormalizedTextBuffer::default();
        tok.normalize(text, &mut norm)?;
        let mut sink = TokenSink::default();
        tok.tokenize(
            &norm.text,
            &mut self.engine.dict,
            &self.engine.config.quotas,
            &mut sink,
        )?;
        let tokens = sink
            .tokens
            .into_iter()
            .map(|t| TokenWire {
                token_id: t.token_id,
                canonical: t.canonical,
                frequency: t.frequency,
            })
            .collect();
        Ok((tok.tokenizer_id(), tok.version(), tokens))
    }

    pub fn search_lexical(
        &mut self,
        caller: &IndexCaller,
        text: &str,
        limit: u32,
    ) -> Result<Vec<SearchHit>, IndexError> {
        caller.caps.require(IndexCapability::SearchLexical)?;
        self.refresh_memorydb_health();
        if self.health.memorydb_connection != "Ready" {
            return Err(IndexError::DatabaseUnavailable);
        }
        let (tid, tver, tokens) = self.tokenize_text(caller, text)?;
        if tokens.is_empty() {
            return Ok(Vec::new());
        }
        let token_ids: Vec<u64> = tokens.iter().map(|t| t.token_id).collect();
        let limit = limit
            .min(self.engine.config.quotas.max_source_list_results)
            .max(1);
        let q = MemoryQuery {
            token_match: Some(TokenQuery {
                tokenizer_id: tid,
                tokenizer_version: tver,
                token_ids,
                mode: TokenMatchMode::Any,
            }),
            limit,
            ..Default::default()
        };
        let res = self.backend.query(q)?;
        let mut hits = Vec::new();
        for id in res.ids {
            let rec = match self.backend.get_record(id, true) {
                Ok(r) => r,
                Err(_) => match self.backend.get_record(id, false) {
                    Ok(r) => r,
                    Err(_) => continue,
                },
            };
            let preview = if rec.payload.is_empty() {
                alloc::format!(
                    "memory_id={} kind={} conf={}",
                    rec.id.get(),
                    rec.kind.as_str(),
                    rec.confidence
                )
            } else {
                let s = core::str::from_utf8(&rec.payload).unwrap_or("");
                let mut p: String = s.chars().take(80).collect();
                if s.len() > 80 {
                    p.push('…');
                }
                p
            };
            hits.push(SearchHit {
                memory_id: id.get(),
                source_id: rec.provenance.source_id.map(|s| s.get()),
                lexical_score: 1,
                preview,
            });
        }
        Ok(hits)
    }

    pub fn health(&self) -> IndexHealth {
        self.health.clone()
    }

    pub fn stats(&self) -> IndexStats {
        self.stats.clone()
    }

    pub fn memorydb_health(&mut self) -> Result<MemoryDbHealth, IndexError> {
        self.backend.health()
    }
}

// Convenience alias for host tests.
impl<S: DurableStore> IndexerService<HostMemoryDbBackend<S>> {
    pub fn new(db: Database<S>, config: IndexerConfig) -> Self {
        Self::new_host(db, config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiseowl_memorydb::database::MemoryStore;
    use wiseowl_memorydb::DbQuotaConfig;

    #[test]
    fn end_to_end_index_and_search() {
        let db = Database::<MemoryStore>::open_memory(DbQuotaConfig::default()).unwrap();
        let mut svc = IndexerService::new(db, IndexerConfig::default());
        let caller = IndexCaller::user(1);
        let mut db_caller = DbCaller::user(1);
        db_caller.caps = db_caller
            .caps
            .grant(wiseowl_memorydb::DbCapability::ReadPayload);
        svc.backend.caller = db_caller;
        let rid = svc
            .register_root(&caller, String::from("/virtual/docs"), true, 8)
            .unwrap();
        svc.virtual_roots.insert(
            rid,
            vec![(
                String::from("notes.md"),
                b"# Hello\n\nthermal fan service notes\n".to_vec(),
                Some(1),
            )],
        );
        svc.set_now_ns(1000);
        svc.refresh_memorydb_health();
        svc.start_scan(&caller, Some(rid)).unwrap();
        assert!(svc.stats.files_indexed >= 1);
        assert!(svc.stats.strong_hash_files >= 1);

        let hits = svc.search_lexical(&caller, "thermal fan", 10).unwrap();
        assert!(!hits.is_empty(), "expected lexical hits");
        assert_eq!(hits[0].lexical_score, 1);
    }

    #[test]
    fn unauthorized_root_other_user() {
        let db = Database::<MemoryStore>::open_memory(DbQuotaConfig::default()).unwrap();
        let mut svc = IndexerService::new(db, IndexerConfig::default());
        let c1 = IndexCaller::user(1);
        let rid = svc
            .register_root(&c1, String::from("/home/u1/Documents"), true, 4)
            .unwrap();
        let c2 = IndexCaller::user(2);
        assert!(svc.start_scan(&c2, Some(rid)).is_err());
    }

    #[test]
    fn unavailable_backend_degrades() {
        use crate::memorydb_backend::UnavailableMemoryDb;
        let mut svc = IndexerService::with_backend(UnavailableMemoryDb, IndexerConfig::default());
        svc.refresh_memorydb_health();
        assert_eq!(svc.health.state, HealthState::Degraded);
        assert!(svc
            .health
            .reasons
            .iter()
            .any(|r| r == "MemoryDbUnavailable"));
        let caller = IndexCaller::admin();
        assert!(matches!(
            svc.start_scan(&caller, None),
            Err(IndexError::DatabaseUnavailable)
        ));
    }
}
