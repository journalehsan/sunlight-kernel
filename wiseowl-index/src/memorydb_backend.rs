//! MemoryDB backend abstraction for the shared indexer engine.
//!
//! ```text
//! HostMemoryDbBackend  — in-process Database (host tests / host daemon)
//! NativeMemoryDbClient — independent wiseowl.memorydb.v1 service (target)
//! ```
//!
//! The indexer engine must not know whether the transport is host or native.
//! Native production must never fall back to an embedded store.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use wiseowl_memory::{MemoryId, SourceId};
use wiseowl_memorydb::attributes::AttributeValue;
use wiseowl_memorydb::database::{Database, DbCaller, DurableStore, InsertRequest};
use wiseowl_memorydb::query::{MemoryQuery, QueryResult};
use wiseowl_memorydb::record::LongTermMemoryRecord;

use crate::error::IndexError;
use crate::import_key::{ImportKey, ImportReconcileResult, ImportState};
use crate::source::SourceManifest;

/// Lightweight health view from MemoryDB.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryDbHealth {
    pub ready: bool,
    pub state: String,
    pub database_generation: u64,
}

/// Backend interface used by scan / ingest / reconciliation.
pub trait IndexMemoryDb {
    /// Native operational-state barrier. Implementations must durably persist
    /// this prepared manifest before the first mutating transaction request.
    fn persist_prepared_import(&mut self, _manifest: &SourceManifest) -> Result<(), IndexError> {
        Ok(())
    }

    fn clear_prepared_import(&mut self, _source_id: SourceId) -> Result<(), IndexError> {
        Ok(())
    }

    fn health(&mut self) -> Result<MemoryDbHealth, IndexError>;

    fn begin_transaction(&mut self) -> Result<u64, IndexError>;

    fn insert_record(&mut self, tx: u64, req: InsertRequest) -> Result<MemoryId, IndexError>;

    fn commit_transaction(&mut self, tx: u64) -> Result<u64, IndexError>;

    fn abort_transaction(&mut self, tx: u64) -> Result<(), IndexError>;

    fn delete_source(&mut self, source_id: SourceId, batch: u32) -> Result<(u32, bool), IndexError>;

    fn delete_source_dry_run(&mut self, source_id: SourceId) -> Result<u32, IndexError>;

    fn get_record(
        &mut self,
        id: MemoryId,
        payload: bool,
    ) -> Result<LongTermMemoryRecord, IndexError>;

    fn source_lookup(
        &mut self,
        source_id: SourceId,
        offset: u32,
        limit: u32,
    ) -> Result<(Vec<MemoryId>, bool), IndexError>;

    fn query(&mut self, q: MemoryQuery) -> Result<QueryResult, IndexError>;

    /// Query import status by stable ImportKey (idempotent reconciliation).
    fn reconcile_import(&mut self, key: &ImportKey) -> Result<ImportReconcileResult, IndexError>;

    /// Bounded generation census: (sources, active, superseded, multi_active, dup_keys, orphans).
    fn generation_census(
        &mut self,
        _source_id: Option<SourceId>,
        _max_sources: u32,
    ) -> Result<(u64, u64, u64, u64, u64, u64), IndexError> {
        Err(IndexError::DatabaseUnavailable)
    }

    /// Verify generation invariants: (ok, multi, dups, orphans, invalid_chains, active).
    fn verify_generations(&mut self) -> Result<(bool, u64, u64, u64, u64, u64), IndexError> {
        Err(IndexError::DatabaseUnavailable)
    }
}

/// Host / test backend wrapping an in-process `Database<S>`.
pub struct HostMemoryDbBackend<S: DurableStore> {
    pub db: Database<S>,
    pub caller: DbCaller,
}

impl<S: DurableStore> HostMemoryDbBackend<S> {
    pub fn new(db: Database<S>, caller: DbCaller) -> Self {
        Self { db, caller }
    }

    pub fn set_now_ns(&mut self, ns: u64) {
        self.db.set_now_ns(ns);
    }

    pub fn inner(&self) -> &Database<S> {
        &self.db
    }

    pub fn inner_mut(&mut self) -> &mut Database<S> {
        &mut self.db
    }
}

impl<S: DurableStore> IndexMemoryDb for HostMemoryDbBackend<S> {
    fn health(&mut self) -> Result<MemoryDbHealth, IndexError> {
        let h = self.db.health();
        let s = self.db.stats();
        let state = match h.state {
            wiseowl_memorydb::HealthState::Starting => "starting",
            wiseowl_memorydb::HealthState::Ready => "ready",
            wiseowl_memorydb::HealthState::Degraded => "degraded",
            wiseowl_memorydb::HealthState::Failed => "failed",
        };
        Ok(MemoryDbHealth {
            ready: h.ready,
            state: String::from(state),
            database_generation: s.database_generation,
        })
    }

    fn begin_transaction(&mut self) -> Result<u64, IndexError> {
        Ok(self.db.begin_transaction(&self.caller)?)
    }

    fn insert_record(&mut self, tx: u64, req: InsertRequest) -> Result<MemoryId, IndexError> {
        Ok(self.db.insert_record(&self.caller, tx, req)?)
    }

    fn commit_transaction(&mut self, tx: u64) -> Result<u64, IndexError> {
        Ok(self.db.commit_transaction(&self.caller, tx)?)
    }

    fn abort_transaction(&mut self, tx: u64) -> Result<(), IndexError> {
        Ok(self.db.abort_transaction(&self.caller, tx)?)
    }

    fn delete_source(&mut self, source_id: SourceId, batch: u32) -> Result<(u32, bool), IndexError> {
        Ok(self.db.delete_source(&self.caller, source_id, batch)?)
    }

    fn delete_source_dry_run(&mut self, source_id: SourceId) -> Result<u32, IndexError> {
        Ok(self.db.delete_source_dry_run(&self.caller, source_id)?)
    }

    fn get_record(
        &mut self,
        id: MemoryId,
        payload: bool,
    ) -> Result<LongTermMemoryRecord, IndexError> {
        Ok(self.db.get_record(&self.caller, id, payload)?)
    }

    fn source_lookup(
        &mut self,
        source_id: SourceId,
        offset: u32,
        limit: u32,
    ) -> Result<(Vec<MemoryId>, bool), IndexError> {
        Ok(self.db.source_lookup(
            &self.caller,
            source_id,
            offset as usize,
            limit as usize,
        )?)
    }

    fn query(&mut self, q: MemoryQuery) -> Result<QueryResult, IndexError> {
        Ok(self.db.query(&self.caller, q)?)
    }

    fn reconcile_import(&mut self, key: &ImportKey) -> Result<ImportReconcileResult, IndexError> {
        // Look up records for this source and match import_key attribute.
        let (ids, _) = self
            .db
            .source_lookup(&self.caller, key.source_id, 0, 64usize)?;
        let want = key.key_hex();
        let mut found_doc: Option<(u64, u32)> = None;
        let mut conflict = false;
        for id in ids {
            let rec = match self.db.get_record(&self.caller, id, false) {
                Ok(r) => r,
                Err(_) => continue,
            };
            let mut role: Option<String> = None;
            let mut import_key: Option<String> = None;
            let mut rev: Option<u32> = None;
            for a in &rec.attributes.entries {
                match a.key.as_str() {
                    "record_role" => {
                        if let AttributeValue::Text(t) = &a.value {
                            role = Some(t.clone());
                        }
                    }
                    "import_key" => {
                        if let AttributeValue::Text(t) = &a.value {
                            import_key = Some(t.clone());
                        }
                    }
                    "source_revision" => {
                        if let AttributeValue::Unsigned(u) = &a.value {
                            rev = Some(*u as u32);
                        }
                    }
                    _ => {}
                }
            }
            if role.as_deref() != Some("document") {
                continue;
            }
            match import_key.as_deref() {
                Some(ik) if ik == want.as_str() => {
                    found_doc = Some((rec.id.get(), rev.unwrap_or(key.source_revision)));
                }
                Some(ik) if !ik.is_empty() && rev == Some(key.source_revision) => {
                    // Same revision, different non-empty key → conflict
                    conflict = true;
                }
                // Missing import_key on an older document with same revision: treat as
                // already committed for that revision (legacy Phase 3 records).
                None if rev == Some(key.source_revision) => {
                    found_doc = Some((rec.id.get(), key.source_revision));
                }
                _ => {}
            }
        }
        if let Some((doc_id, rev)) = found_doc {
            return Ok(ImportReconcileResult {
                state: ImportState::AlreadyCommitted,
                document_memory_id: Some(doc_id),
                source_revision: Some(rev),
            });
        }
        if conflict {
            return Ok(ImportReconcileResult {
                state: ImportState::Conflict,
                document_memory_id: None,
                source_revision: Some(key.source_revision),
            });
        }
        Ok(ImportReconcileResult {
            state: ImportState::NotFound,
            document_memory_id: None,
            source_revision: None,
        })
    }

    fn generation_census(
        &mut self,
        source_id: Option<SourceId>,
        max_sources: u32,
    ) -> Result<(u64, u64, u64, u64, u64, u64), IndexError> {
        let (g, _) = self.db.generation_census(source_id, max_sources.max(1));
        Ok((
            g.sources as u64,
            g.active_document_generations,
            g.superseded_document_generations,
            g.sources_with_multiple_active_generations as u64,
            g.duplicate_import_keys as u64,
            g.orphan_chunks as u64,
        ))
    }

    fn verify_generations(&mut self) -> Result<(bool, u64, u64, u64, u64, u64), IndexError> {
        let v = self.db.verify_generations();
        Ok((
            v.ok,
            v.multi_active_sources as u64,
            v.duplicate_import_keys as u64,
            v.orphan_chunks as u64,
            v.invalid_supersession_chains as u64,
            v.census.active_document_generations,
        ))
    }
}

/// Stub backend that always reports MemoryDB unavailable (tests / degraded).
pub struct UnavailableMemoryDb;

impl IndexMemoryDb for UnavailableMemoryDb {
    fn health(&mut self) -> Result<MemoryDbHealth, IndexError> {
        Err(IndexError::DatabaseUnavailable)
    }
    fn begin_transaction(&mut self) -> Result<u64, IndexError> {
        Err(IndexError::DatabaseUnavailable)
    }
    fn insert_record(&mut self, _tx: u64, _req: InsertRequest) -> Result<MemoryId, IndexError> {
        Err(IndexError::DatabaseUnavailable)
    }
    fn commit_transaction(&mut self, _tx: u64) -> Result<u64, IndexError> {
        Err(IndexError::DatabaseUnavailable)
    }
    fn abort_transaction(&mut self, _tx: u64) -> Result<(), IndexError> {
        Err(IndexError::DatabaseUnavailable)
    }
    fn delete_source(
        &mut self,
        _source_id: SourceId,
        _batch: u32,
    ) -> Result<(u32, bool), IndexError> {
        Err(IndexError::DatabaseUnavailable)
    }
    fn delete_source_dry_run(&mut self, _source_id: SourceId) -> Result<u32, IndexError> {
        Err(IndexError::DatabaseUnavailable)
    }
    fn get_record(
        &mut self,
        _id: MemoryId,
        _payload: bool,
    ) -> Result<LongTermMemoryRecord, IndexError> {
        Err(IndexError::DatabaseUnavailable)
    }
    fn source_lookup(
        &mut self,
        _source_id: SourceId,
        _offset: u32,
        _limit: u32,
    ) -> Result<(Vec<MemoryId>, bool), IndexError> {
        Err(IndexError::DatabaseUnavailable)
    }
    fn query(&mut self, _q: MemoryQuery) -> Result<QueryResult, IndexError> {
        Err(IndexError::DatabaseUnavailable)
    }
    fn reconcile_import(&mut self, _key: &ImportKey) -> Result<ImportReconcileResult, IndexError> {
        Err(IndexError::DatabaseUnavailable)
    }
}
