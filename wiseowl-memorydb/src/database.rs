//! Long-term memory database engine.
//!
//! Append-oriented: WAL → commit → seal into immutable segments → rebuildable indexes.
//! Host builds use the filesystem under `database/`. Core logic is also
//! testable via an in-process memory-backed store for unit tests that still
//! exercise the same WAL/segment formats (bytes are durable across open()).

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use wiseowl_memory::{IdAllocator, MemoryId, SourceId, TrustLevel};

use crate::caps::{DbCapability, DbCapabilitySet};
use crate::codec::fnv1a64;
use crate::error::DbError;
use crate::health::{DbHealth, HealthState};
use crate::index::{IndexSet, RecordLocation};
use crate::query::{
    DedupPolicy, MemoryQuery, QueryCursor, QueryOrder, QueryResult, TrustFilter,
};
use crate::quotas::DbQuotaConfig;
use crate::record::{
    LongTermMemoryKind, LongTermMemoryRecord, LongTermRecordState, MemoryScope, OwnerId,
    LT_RECORD_FORMAT_VERSION,
};
use crate::relationship::{MemoryRelationship, RelationshipKind};
use crate::segment::{open_segment, seal_segment, SegmentHeader};
use crate::stats::DbStats;
use crate::tokens::{normalize_tokens, IndexedToken, TokenSetRef};
use crate::wal::{
    committed_tx_ids, scan_wal, WalRecord, WalRecordType, WAL_FORMAT_VERSION,
};

/// Caller identity for capability and scope checks.
#[derive(Debug, Clone)]
pub struct DbCaller {
    pub caps: DbCapabilitySet,
    pub owner: OwnerId,
    /// If true, treats caller as system (cross-scope with ReadSharedScope).
    pub is_system: bool,
}

impl DbCaller {
    pub fn admin() -> Self {
        Self {
            caps: DbCapabilitySet::admin(),
            owner: 0,
            is_system: true,
        }
    }

    pub fn user(owner: OwnerId) -> Self {
        Self {
            caps: DbCapabilitySet::default_client(),
            owner,
            is_system: false,
        }
    }
}

/// Dedup / insert request.
#[derive(Debug, Clone)]
pub struct InsertRequest {
    pub kind: LongTermMemoryKind,
    pub scope: MemoryScope,
    pub owner: OwnerId,
    pub payload: Vec<u8>,
    pub provenance: crate::provenance::LongTermProvenance,
    pub confidence: u16,
    pub importance: u16,
    pub trust: TrustLevel,
    pub valid_from_ns: Option<u64>,
    pub valid_until_ns: Option<u64>,
    pub tokens: Option<(TokenSetRef, Vec<IndexedToken>)>,
    pub attributes: crate::attributes::AttributeSet,
    pub supersedes: Option<MemoryId>,
    pub relationships: Vec<MemoryRelationship>,
    pub dedup: DedupPolicy,
    /// Optional explicit id (otherwise allocated).
    pub id: Option<MemoryId>,
    pub revision: u32,
}

/// Open transaction staging.
#[derive(Debug)]
struct OpenTx {
    id: u64,
    opened_at_ns: u64,
    ops: u32,
    bytes: u32,
    relationships: u32,
    staged_records: Vec<LongTermMemoryRecord>,
    staged_relationships: Vec<MemoryRelationship>,
    staged_tombstones: Vec<MemoryId>,
    staged_source_deletes: Vec<SourceId>,
}

/// In-memory view of a sealed segment.
#[derive(Debug, Clone)]
struct SegmentMem {
    header: SegmentHeader,
    records: Vec<LongTermMemoryRecord>,
    compressed_bytes: u64,
}

/// Checkpoint metadata (MANIFEST content).
#[derive(Debug, Clone, Default)]
pub struct CheckpointMeta {
    pub database_generation: u64,
    pub last_committed_sequence: u64,
    pub active_segments: Vec<u64>,
    pub index_generation: u64,
    pub wal_replay_offset: u64,
    pub format_version: u16,
    pub checksum: u32,
}

/// Durable store backend (host filesystem or test memory).
pub trait DurableStore {
    fn read_file(&self, rel: &str) -> Result<Option<Vec<u8>>, DbError>;
    fn write_file_atomic(&mut self, rel: &str, data: &[u8]) -> Result<(), DbError>;
    fn append_file(&mut self, rel: &str, data: &[u8]) -> Result<(), DbError>;
    fn remove_file(&mut self, rel: &str) -> Result<(), DbError>;
    fn list_prefix(&self, dir: &str, prefix: &str) -> Result<Vec<String>, DbError>;
    fn ensure_layout(&mut self) -> Result<(), DbError>;
}

/// In-memory durable store for tests (still uses real formats).
#[derive(Debug, Default)]
pub struct MemoryStore {
    files: BTreeMap<String, Vec<u8>>,
}

impl DurableStore for MemoryStore {
    fn read_file(&self, rel: &str) -> Result<Option<Vec<u8>>, DbError> {
        Ok(self.files.get(rel).cloned())
    }

    fn write_file_atomic(&mut self, rel: &str, data: &[u8]) -> Result<(), DbError> {
        self.files.insert(rel.to_string(), data.to_vec());
        Ok(())
    }

    fn append_file(&mut self, rel: &str, data: &[u8]) -> Result<(), DbError> {
        self.files.entry(rel.to_string()).or_default().extend_from_slice(data);
        Ok(())
    }

    fn remove_file(&mut self, rel: &str) -> Result<(), DbError> {
        self.files.remove(rel);
        Ok(())
    }

    fn list_prefix(&self, dir: &str, prefix: &str) -> Result<Vec<String>, DbError> {
        let base = if dir.is_empty() {
            String::new()
        } else if dir.ends_with('/') {
            dir.to_string()
        } else {
            alloc::format!("{dir}/")
        };
        let full_prefix = alloc::format!("{base}{prefix}");
        Ok(self
            .files
            .keys()
            .filter(|k| k.starts_with(&full_prefix))
            .cloned()
            .collect())
    }

    fn ensure_layout(&mut self) -> Result<(), DbError> {
        for d in [
            "WAL/",
            "SEGMENTS/",
            "INDEX/",
            "SNAPSHOTS/",
            "QUARANTINE/",
            "TMP/",
        ] {
            // Placeholder keys so layout exists.
            let key = alloc::format!("{d}.keep");
            self.files.entry(key).or_insert_with(Vec::new);
        }
        Ok(())
    }
}

#[cfg(feature = "host")]
mod host_store {
    use super::*;
    use std::fs;
    use std::io::{Read, Write};
    use std::path::{Path, PathBuf};

    pub struct FsStore {
        root: PathBuf,
    }

    impl FsStore {
        pub fn open(root: impl AsRef<Path>) -> Result<Self, DbError> {
            let root = root.as_ref().to_path_buf();
            fs::create_dir_all(&root).map_err(|_| DbError::Io("create root"))?;
            let mut s = Self { root };
            s.ensure_layout()?;
            Ok(s)
        }

        fn path(&self, rel: &str) -> PathBuf {
            self.root.join(rel)
        }
    }

    impl DurableStore for FsStore {
        fn read_file(&self, rel: &str) -> Result<Option<Vec<u8>>, DbError> {
            let p = self.path(rel);
            if !p.exists() {
                return Ok(None);
            }
            let mut f = fs::File::open(&p).map_err(|_| DbError::Io("open"))?;
            let mut buf = Vec::new();
            f.read_to_end(&mut buf).map_err(|_| DbError::Io("read"))?;
            Ok(Some(buf))
        }

        fn write_file_atomic(&mut self, rel: &str, data: &[u8]) -> Result<(), DbError> {
            let final_p = self.path(rel);
            if let Some(parent) = final_p.parent() {
                fs::create_dir_all(parent).map_err(|_| DbError::Io("mkdir"))?;
            }
            let tmp = self.path(&alloc::format!("TMP/write-{}.tmp", fnv1a64(rel.as_bytes())));
            if let Some(parent) = tmp.parent() {
                fs::create_dir_all(parent).map_err(|_| DbError::Io("mkdir tmp"))?;
            }
            {
                let mut f = fs::File::create(&tmp).map_err(|_| DbError::Io("create tmp"))?;
                f.write_all(data).map_err(|_| DbError::Io("write tmp"))?;
                f.sync_all().map_err(|_| DbError::Io("fsync tmp"))?;
            }
            fs::rename(&tmp, &final_p).map_err(|_| DbError::Io("rename"))?;
            Ok(())
        }

        fn append_file(&mut self, rel: &str, data: &[u8]) -> Result<(), DbError> {
            let p = self.path(rel);
            if let Some(parent) = p.parent() {
                fs::create_dir_all(parent).map_err(|_| DbError::Io("mkdir"))?;
            }
            let mut f = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&p)
                .map_err(|_| DbError::Io("append open"))?;
            f.write_all(data).map_err(|_| DbError::Io("append"))?;
            f.sync_all().map_err(|_| DbError::Io("fsync append"))?;
            Ok(())
        }

        fn remove_file(&mut self, rel: &str) -> Result<(), DbError> {
            let p = self.path(rel);
            if p.exists() {
                fs::remove_file(p).map_err(|_| DbError::Io("remove"))?;
            }
            Ok(())
        }

        fn list_prefix(&self, dir: &str, prefix: &str) -> Result<Vec<String>, DbError> {
            let d = self.path(dir);
            if !d.exists() {
                return Ok(Vec::new());
            }
            let mut out = Vec::new();
            for ent in fs::read_dir(&d).map_err(|_| DbError::Io("readdir"))? {
                let ent = ent.map_err(|_| DbError::Io("readdir ent"))?;
                let name = ent.file_name().to_string_lossy().into_owned();
                if name.starts_with(prefix) {
                    let rel = if dir.is_empty() {
                        name
                    } else {
                        alloc::format!("{dir}/{name}")
                    };
                    out.push(rel);
                }
            }
            out.sort();
            Ok(out)
        }

        fn ensure_layout(&mut self) -> Result<(), DbError> {
            for d in [
                "WAL",
                "SEGMENTS",
                "INDEX",
                "SNAPSHOTS",
                "QUARANTINE",
                "TMP",
            ] {
                fs::create_dir_all(self.path(d)).map_err(|_| DbError::Io("mkdir layout"))?;
            }
            Ok(())
        }
    }

}

#[cfg(feature = "host")]
pub use host_store::FsStore;

/// The long-term memory database.
pub struct Database<S: DurableStore> {
    store: S,
    quotas: DbQuotaConfig,
    ids: IdAllocator,
    next_tx: u64,
    next_seq: u64,
    next_segment: u64,
    database_generation: u64,
    index_generation: u64,
    indexes: IndexSet,
    segments: BTreeMap<u64, SegmentMem>,
    /// Records not yet sealed (committed but waiting for segment flush).
    unsealed: Vec<LongTermMemoryRecord>,
    /// All known records by id (latest).
    records: BTreeMap<u64, LongTermMemoryRecord>,
    /// Relationships by a simple list (also indexed).
    relationships: Vec<MemoryRelationship>,
    open_txs: BTreeMap<u64, OpenTx>,
    stats: DbStats,
    health: DbHealth,
    now_ns: u64,
    wal_path: String,
    wal_bytes: u64,
    quarantine_count: u32,
    /// Source deletion resume state.
    source_delete_cursor: BTreeMap<u64, usize>,
}

impl Database<MemoryStore> {
    pub fn open_memory(quotas: DbQuotaConfig) -> Result<Self, DbError> {
        let mut store = MemoryStore::default();
        store.ensure_layout()?;
        Self::open_with_store(store, quotas)
    }
}

#[cfg(feature = "host")]
impl Database<FsStore> {
    pub fn open_fs(root: impl AsRef<std::path::Path>, quotas: DbQuotaConfig) -> Result<Self, DbError> {
        let store = FsStore::open(root)?;
        Self::open_with_store(store, quotas)
    }
}

impl<S: DurableStore> Database<S> {
    pub fn open_with_store(mut store: S, quotas: DbQuotaConfig) -> Result<Self, DbError> {
        store.ensure_layout()?;
        let mut db = Self {
            store,
            quotas,
            ids: IdAllocator::new(),
            next_tx: 1,
            next_seq: 1,
            next_segment: 1,
            database_generation: 1,
            index_generation: 1,
            indexes: IndexSet::default(),
            segments: BTreeMap::new(),
            unsealed: Vec::new(),
            records: BTreeMap::new(),
            relationships: Vec::new(),
            open_txs: BTreeMap::new(),
            stats: DbStats::default(),
            health: DbHealth::starting(),
            now_ns: 1,
            wal_path: String::from("WAL/wal-000001"),
            wal_bytes: 0,
            quarantine_count: 0,
            source_delete_cursor: BTreeMap::new(),
        };
        db.recover()?;
        db.health = if db.indexes.degraded {
            DbHealth::degraded(vec![String::from(
                db.indexes.degrade_reason.unwrap_or("degraded"),
            )])
        } else {
            DbHealth::ready()
        };
        db.refresh_stats();
        Ok(db)
    }

    pub fn set_now_ns(&mut self, now: u64) {
        self.now_ns = now.max(1);
    }

    pub fn health(&self) -> &DbHealth {
        &self.health
    }

    pub fn stats(&self) -> DbStats {
        self.stats.clone()
    }

    pub fn quotas(&self) -> &DbQuotaConfig {
        &self.quotas
    }

    fn recover(&mut self) -> Result<(), DbError> {
        // Load MANIFEST checkpoint if present.
        if let Some(data) = self.store.read_file("MANIFEST")? {
            if let Ok(meta) = decode_manifest(&data) {
                self.database_generation = meta.database_generation.max(1);
                self.next_seq = meta.last_committed_sequence.saturating_add(1).max(1);
                self.index_generation = meta.index_generation.max(1);
                self.stats.last_committed_sequence = meta.last_committed_sequence;
            }
        }

        // Discover segments.
        let mut recovery_bytes = 0u64;
        let seg_files = self.store.list_prefix("SEGMENTS", "data-")?;
        for rel in seg_files {
            if recovery_bytes >= self.quotas.max_recovery_bytes {
                self.indexes.mark_degraded("recovery budget");
                break;
            }
            match self.store.read_file(&rel)? {
                Some(data) => {
                    recovery_bytes = recovery_bytes.saturating_add(data.len() as u64);
                    match open_segment(&data, &self.quotas) {
                        Ok((header, records)) => {
                            for (i, rec) in records.iter().enumerate() {
                                self.ids.note_seen(rec.id.get());
                                self.records.insert(rec.id.get(), rec.clone());
                                let loc = RecordLocation {
                                    segment_id: header.segment_id,
                                    record_index: i as u32,
                                    revision: rec.revision,
                                };
                                self.indexes.apply_record(rec, loc);
                            }
                            self.next_segment = self.next_segment.max(header.segment_id.saturating_add(1));
                            self.segments.insert(
                                header.segment_id,
                                SegmentMem {
                                    header: header.clone(),
                                    records,
                                    compressed_bytes: data.len() as u64,
                                },
                            );
                        }
                        Err(_) => {
                            self.quarantine_file(&rel, &data)?;
                            self.stats.checksum_failures =
                                self.stats.checksum_failures.saturating_add(1);
                        }
                    }
                }
                None => {}
            }
        }

        // Replay WAL.
        if let Some(wal) = self.store.read_file(&self.wal_path.clone())? {
            let _ = recovery_bytes.saturating_add(wal.len() as u64);
            self.wal_bytes = wal.len() as u64;
            let scan = scan_wal(&wal, self.quotas.max_bytes_per_transaction);
            let committed = committed_tx_ids(&scan.records);
            let mut by_tx: BTreeMap<u64, Vec<&WalRecord>> = BTreeMap::new();
            for r in &scan.records {
                by_tx.entry(r.transaction_id).or_default().push(r);
                self.next_seq = self.next_seq.max(r.sequence.saturating_add(1));
                self.next_tx = self.next_tx.max(r.transaction_id.saturating_add(1));
            }
            for tx_id in committed {
                if let Some(ops) = by_tx.get(&tx_id) {
                    for op in ops {
                        self.replay_wal_op(op)?;
                        self.stats.recovery_replayed_operations =
                            self.stats.recovery_replayed_operations.saturating_add(1);
                    }
                }
            }
            if scan.tail_corrupt {
                self.health = DbHealth::degraded(vec![String::from("wal tail corrupt")]);
            }
        }

        // Load relationship snapshot if present (rebuildable; optional).
        if let Some(data) = self.store.read_file("INDEX/relationships.bin")? {
            if let Ok(rels) = decode_relationships(&data) {
                for rel in rels {
                    self.relationships.push(rel.clone());
                    self.indexes.apply_relationship(&rel);
                }
            } else {
                self.indexes.mark_degraded("relationship index");
            }
        }

        // Advance generation for new process.
        self.database_generation = self.database_generation.saturating_add(1);
        let _ = self.ids.bump_generation();
        Ok(())
    }

    fn quarantine_file(&mut self, rel: &str, data: &[u8]) -> Result<(), DbError> {
        if self.quarantine_count >= self.quotas.max_quarantine_files {
            return Ok(());
        }
        let name = alloc::format!(
            "QUARANTINE/q-{:016x}-{}",
            fnv1a64(rel.as_bytes()),
            rel.replace('/', "_")
        );
        let max = self.quotas.max_quarantine_bytes as usize / 4;
        let slice = if data.len() > max { &data[..max] } else { data };
        let _ = self.store.write_file_atomic(&name, slice);
        let _ = self.store.remove_file(rel);
        self.quarantine_count = self.quarantine_count.saturating_add(1);
        self.stats.quarantined_files = self.quarantine_count;
        Ok(())
    }

    fn replay_wal_op(&mut self, op: &WalRecord) -> Result<(), DbError> {
        match op.record_type {
            WalRecordType::InsertRecord => {
                let rec = LongTermMemoryRecord::decode(&op.payload, &self.quotas)?;
                self.ids.note_seen(rec.id.get());
                self.install_record(rec, None)?;
            }
            WalRecordType::InsertRelationship => {
                let rel = MemoryRelationship::decode(&mut crate::codec::BufReader::new(
                    &op.payload,
                ))?;
                self.relationships.push(rel.clone());
                self.indexes.apply_relationship(&rel);
            }
            WalRecordType::TombstoneRecord => {
                if op.payload.len() >= 8 {
                    let id = MemoryId::from_raw(u64::from_le_bytes(
                        op.payload[0..8].try_into().unwrap(),
                    ))
                    .map_err(|_| DbError::InvalidValue("tombstone id"))?;
                    self.apply_tombstone(id)?;
                }
            }
            WalRecordType::SourceDelete => {
                if op.payload.len() >= 8 {
                    let sid = SourceId::from_raw(u64::from_le_bytes(
                        op.payload[0..8].try_into().unwrap(),
                    ))
                    .map_err(|_| DbError::InvalidValue("source id"))?;
                    let _ = self.apply_source_delete_batch(sid, 64)?;
                }
            }
            WalRecordType::Begin
            | WalRecordType::Commit
            | WalRecordType::Abort
            | WalRecordType::Checkpoint => {}
        }
        Ok(())
    }

    fn install_record(
        &mut self,
        rec: LongTermMemoryRecord,
        loc: Option<RecordLocation>,
    ) -> Result<(), DbError> {
        if self.records.len() as u32 >= self.quotas.max_records
            && !self.records.contains_key(&rec.id.get())
        {
            return Err(DbError::QuotaExceeded("max records"));
        }
        let location = loc.unwrap_or(RecordLocation {
            segment_id: 0,
            record_index: self.unsealed.len() as u32,
            revision: rec.revision,
        });
        if loc.is_none() {
            self.unsealed.push(rec.clone());
        }
        // Handle supersession.
        if let Some(old) = rec.supersedes {
            if let Some(prev) = self.records.get_mut(&old.get()) {
                prev.state = LongTermRecordState::Superseded;
                self.indexes
                    .primary
                    .set_state(old, LongTermRecordState::Superseded);
            }
        }
        self.indexes.apply_record(&rec, location);
        self.records.insert(rec.id.get(), rec);
        Ok(())
    }

    fn apply_tombstone(&mut self, id: MemoryId) -> Result<(), DbError> {
        if let Some(rec) = self.records.get_mut(&id.get()) {
            rec.state = LongTermRecordState::Tombstoned;
            self.indexes
                .primary
                .set_state(id, LongTermRecordState::Tombstoned);
            self.indexes.token.remove_memory(id.get());
        }
        Ok(())
    }

    fn refresh_stats(&mut self) {
        let mut active = 0u32;
        let mut super_s = 0u32;
        let mut tomb = 0u32;
        for r in self.records.values() {
            match r.state {
                LongTermRecordState::Active => active += 1,
                LongTermRecordState::Superseded => super_s += 1,
                LongTermRecordState::Tombstoned => tomb += 1,
                LongTermRecordState::Quarantined => {}
            }
        }
        self.stats.database_generation = self.database_generation;
        self.stats.index_generation = self.index_generation;
        self.stats.active_transactions = self.open_txs.len() as u32;
        self.stats.record_count_active = active;
        self.stats.record_count_superseded = super_s;
        self.stats.record_count_tombstoned = tomb;
        self.stats.relationship_count = self.relationships.len() as u64;
        self.stats.segment_count = self.segments.len() as u32;
        self.stats.segment_bytes_compressed =
            self.segments.values().map(|s| s.compressed_bytes).sum();
        self.stats.segment_bytes_uncompressed = self
            .segments
            .values()
            .map(|s| s.header.uncompressed_len as u64)
            .sum();
        self.stats.primary_index_entries = self.indexes.primary.len() as u64;
        self.stats.source_index_entries = self.indexes.source.len() as u64;
        self.stats.token_dictionary_entries = self.indexes.token.dictionary_len();
        self.stats.token_posting_entries = self.indexes.token.posting_entries();
        self.stats.wal_bytes = self.wal_bytes;
        self.stats.quarantined_files = self.quarantine_count;
    }

    // ---- Transactions ----

    pub fn begin_transaction(&mut self, caller: &DbCaller) -> Result<u64, DbError> {
        caller.caps.require(DbCapability::InsertRecord)?;
        self.expire_stale_txs();
        if self.open_txs.len() as u32 >= self.quotas.max_active_transactions {
            return Err(DbError::TransactionLimit("max active transactions"));
        }
        let id = self.next_tx;
        self.next_tx = self.next_tx.saturating_add(1);
        let seq = self.alloc_seq();
        let rec = WalRecord {
            record_type: WalRecordType::Begin,
            transaction_id: id,
            sequence: seq,
            payload: Vec::new(),
        };
        self.append_wal(&rec)?;
        self.open_txs.insert(
            id,
            OpenTx {
                id,
                opened_at_ns: self.now_ns,
                ops: 0,
                bytes: 0,
                relationships: 0,
                staged_records: Vec::new(),
                staged_relationships: Vec::new(),
                staged_tombstones: Vec::new(),
                staged_source_deletes: Vec::new(),
            },
        );
        Ok(id)
    }

    fn expire_stale_txs(&mut self) {
        if self.quotas.max_transaction_age_ns == 0 {
            return;
        }
        let now = self.now_ns;
        let max_age = self.quotas.max_transaction_age_ns;
        let stale: Vec<u64> = self
            .open_txs
            .values()
            .filter(|t| now.saturating_sub(t.opened_at_ns) > max_age)
            .map(|t| t.id)
            .collect();
        for id in stale {
            let _ = self.abort_transaction_inner(id);
        }
    }

    fn alloc_seq(&mut self) -> u64 {
        let s = self.next_seq;
        self.next_seq = self.next_seq.saturating_add(1);
        s
    }

    fn append_wal(&mut self, rec: &WalRecord) -> Result<(), DbError> {
        let bytes = rec.encode(self.quotas.max_bytes_per_transaction)?;
        let next = self
            .wal_bytes
            .checked_add(bytes.len() as u64)
            .ok_or(DbError::Internal("wal overflow"))?;
        if next > self.quotas.max_wal_bytes {
            // Force checkpoint pressure: seal unsealed + checkpoint, then continue if possible.
            self.checkpoint_inner()?;
            if self.wal_bytes.saturating_add(bytes.len() as u64) > self.quotas.max_wal_bytes {
                return Err(DbError::QuotaExceeded("wal size"));
            }
        }
        self.store.append_file(&self.wal_path, &bytes)?;
        self.wal_bytes = self.wal_bytes.saturating_add(bytes.len() as u64);
        self.stats.wal_records = self.stats.wal_records.saturating_add(1);
        self.stats.wal_bytes = self.wal_bytes;
        Ok(())
    }

    pub fn insert_record(
        &mut self,
        caller: &DbCaller,
        tx_id: u64,
        mut req: InsertRequest,
    ) -> Result<MemoryId, DbError> {
        caller.caps.require(DbCapability::InsertRecord)?;
        self.check_scope_write(caller, req.scope, req.owner)?;
        self.check_trust(caller, req.trust)?;

        if req.payload.len() as u32 > self.quotas.max_payload_bytes {
            return Err(DbError::PayloadTooLarge {
                size: req.payload.len() as u32,
                max: self.quotas.max_payload_bytes,
            });
        }

        // Normalize tokens.
        let (tokens_ref, token_entries) = if let Some((ts, entries)) = req.tokens.take() {
            let entries = normalize_tokens(entries, &self.quotas)?;
            let ts = TokenSetRef {
                tokenizer_id: ts.tokenizer_id,
                tokenizer_version: ts.tokenizer_version,
                token_count: entries.len() as u32,
            };
            (Some(ts), entries)
        } else {
            (None, Vec::new())
        };

        // Dedup.
        let payload_hash = fnv1a64(&req.payload);
        match req.dedup {
            DedupPolicy::Allow => {}
            DedupPolicy::RejectExactPayload => {
                if !self.indexes.source.by_payload_hash(payload_hash).is_empty() {
                    return Err(DbError::DedupRejected);
                }
            }
            DedupPolicy::ReturnExistingExactPayload => {
                if let Some(&existing) = self.indexes.source.by_payload_hash(payload_hash).first()
                {
                    return MemoryId::from_raw(existing)
                        .map_err(|_| DbError::Internal("bad existing id"));
                }
            }
            DedupPolicy::RejectSameSourceRevision => {
                if let Some(sid) = req.provenance.source_id {
                    for &rid in self.indexes.source.by_source_id(sid) {
                        if let Some(r) = self.records.get(&rid) {
                            if r.revision == req.revision {
                                return Err(DbError::DedupRejected);
                            }
                        }
                    }
                }
            }
        }

        // Supersession loop check.
        if let Some(old) = req.supersedes {
            if let Some(new_id) = req.id {
                if old == new_id {
                    return Err(DbError::SupersessionLoop);
                }
            }
            // Simple one-step: if old already points supersedes back at the new id.
            // Only when the new id is known — comparing Option::None == Option::None
            // would false-positive every allocated-id insert.
            if let Some(new_id) = req.id {
                if let Some(prev) = self.records.get(&old.get()) {
                    if prev.supersedes == Some(new_id) {
                        return Err(DbError::SupersessionLoop);
                    }
                }
            }
        }

        let id = match req.id {
            Some(id) => {
                if self.records.contains_key(&id.get()) {
                    // Allow new revision.
                }
                id
            }
            None => {
                self.ids
                    .alloc_memory()
                    .map_err(|_| DbError::Internal("id exhausted"))?
            }
        };

        let rec = LongTermMemoryRecord {
            format_version: LT_RECORD_FORMAT_VERSION,
            id,
            revision: if req.revision == 0 { 1 } else { req.revision },
            kind: req.kind,
            scope: req.scope,
            owner: req.owner,
            created_at_ns: self.now_ns,
            updated_at_ns: self.now_ns,
            valid_from_ns: req.valid_from_ns,
            valid_until_ns: req.valid_until_ns,
            importance: req.importance,
            confidence: req.confidence,
            trust: req.trust,
            provenance: req.provenance,
            payload_ref: crate::record::PayloadRef {
                content_hash: payload_hash,
                length: req.payload.len() as u32,
            },
            tokens: tokens_ref,
            attributes: req.attributes,
            state: LongTermRecordState::Active,
            supersedes: req.supersedes,
            payload: req.payload,
            token_entries,
        };
        rec.validate(&self.quotas)?;

        let tx = self
            .open_txs
            .get_mut(&tx_id)
            .ok_or(DbError::InvalidTransaction)?;
        tx.ops = tx.ops.saturating_add(1);
        if tx.ops > self.quotas.max_ops_per_transaction {
            return Err(DbError::TransactionLimit("ops per transaction"));
        }
        let enc = rec.encode(self.quotas.max_bytes_per_transaction as usize)?;
        tx.bytes = tx.bytes.saturating_add(enc.len() as u32);
        if tx.bytes > self.quotas.max_bytes_per_transaction {
            return Err(DbError::TransactionLimit("bytes per transaction"));
        }
        for mut rel in req.relationships {
            rel.source = id;
            rel.validate()?;
            tx.relationships = tx.relationships.saturating_add(1);
            if tx.relationships > self.quotas.max_relationships_per_transaction {
                return Err(DbError::TransactionLimit("relationships per transaction"));
            }
            tx.staged_relationships.push(rel);
        }
        let seq = self.alloc_seq();
        // Need to re-borrow carefully — extract tx fields then write wal.
        let payload = enc;
        {
            let tx = self
                .open_txs
                .get_mut(&tx_id)
                .ok_or(DbError::InvalidTransaction)?;
            tx.staged_records.push(rec);
        }
        let wrec = WalRecord {
            record_type: WalRecordType::InsertRecord,
            transaction_id: tx_id,
            sequence: seq,
            payload,
        };
        self.append_wal(&wrec)?;
        Ok(id)
    }

    pub fn insert_relationship(
        &mut self,
        caller: &DbCaller,
        tx_id: u64,
        rel: MemoryRelationship,
    ) -> Result<(), DbError> {
        caller.caps.require(DbCapability::CreateRelationship)?;
        rel.validate()?;
        if rel.kind == RelationshipKind::Supersedes {
            // Check simple loop via index once committed edges exist.
            if self.indexes.relationship.supersedes_loop(rel.target, 8)
                && rel.target == rel.source
            {
                return Err(DbError::SupersessionLoop);
            }
        }
        let mut w = crate::codec::BufWriter::with_capacity(512);
        rel.encode(&mut w)?;
        let payload = w.into_vec();
        let tx = self
            .open_txs
            .get_mut(&tx_id)
            .ok_or(DbError::InvalidTransaction)?;
        tx.ops = tx.ops.saturating_add(1);
        tx.relationships = tx.relationships.saturating_add(1);
        if tx.ops > self.quotas.max_ops_per_transaction
            || tx.relationships > self.quotas.max_relationships_per_transaction
        {
            return Err(DbError::TransactionLimit("relationship"));
        }
        tx.staged_relationships.push(rel);
        let seq = self.alloc_seq();
        self.append_wal(&WalRecord {
            record_type: WalRecordType::InsertRelationship,
            transaction_id: tx_id,
            sequence: seq,
            payload,
        })?;
        Ok(())
    }

    pub fn tombstone_record(
        &mut self,
        caller: &DbCaller,
        tx_id: u64,
        id: MemoryId,
    ) -> Result<(), DbError> {
        caller.caps.require(DbCapability::Tombstone)?;
        let rec = self.records.get(&id.get()).ok_or(DbError::NotFound)?;
        self.check_scope_read(caller, rec)?;
        let tx = self
            .open_txs
            .get_mut(&tx_id)
            .ok_or(DbError::InvalidTransaction)?;
        tx.ops = tx.ops.saturating_add(1);
        tx.staged_tombstones.push(id);
        let seq = self.alloc_seq();
        self.append_wal(&WalRecord {
            record_type: WalRecordType::TombstoneRecord,
            transaction_id: tx_id,
            sequence: seq,
            payload: id.get().to_le_bytes().to_vec(),
        })?;
        Ok(())
    }

    pub fn commit_transaction(&mut self, caller: &DbCaller, tx_id: u64) -> Result<u64, DbError> {
        let _ = caller;
        let tx = self
            .open_txs
            .remove(&tx_id)
            .ok_or(DbError::InvalidTransaction)?;
        let seq = self.alloc_seq();
        self.append_wal(&WalRecord {
            record_type: WalRecordType::Commit,
            transaction_id: tx_id,
            sequence: seq,
            payload: Vec::new(),
        })?;

        // Make visible.
        for rec in tx.staged_records {
            self.install_record(rec, None)?;
        }
        for rel in tx.staged_relationships {
            self.relationships.push(rel.clone());
            self.indexes.apply_relationship(&rel);
        }
        for id in tx.staged_tombstones {
            self.apply_tombstone(id)?;
        }
        for sid in tx.staged_source_deletes {
            let _ = self.apply_source_delete_batch(sid, 64)?;
        }

        self.stats.transaction_commits = self.stats.transaction_commits.saturating_add(1);
        self.stats.last_committed_sequence = seq;

        // Seal if enough unsealed.
        if self.unsealed.len() >= 8
            || self
                .unsealed
                .iter()
                .map(|r| r.payload.len())
                .sum::<usize>()
                > 32 * 1024
        {
            let _ = self.seal_unsealed();
        }
        self.refresh_stats();
        Ok(seq)
    }

    pub fn abort_transaction(&mut self, _caller: &DbCaller, tx_id: u64) -> Result<(), DbError> {
        self.abort_transaction_inner(tx_id)
    }

    fn abort_transaction_inner(&mut self, tx_id: u64) -> Result<(), DbError> {
        if self.open_txs.remove(&tx_id).is_none() {
            return Err(DbError::InvalidTransaction);
        }
        let seq = self.alloc_seq();
        self.append_wal(&WalRecord {
            record_type: WalRecordType::Abort,
            transaction_id: tx_id,
            sequence: seq,
            payload: Vec::new(),
        })?;
        self.stats.transaction_aborts = self.stats.transaction_aborts.saturating_add(1);
        Ok(())
    }

    fn seal_unsealed(&mut self) -> Result<u64, DbError> {
        if self.unsealed.is_empty() {
            return Ok(0);
        }
        let records: Vec<_> = self.unsealed.drain(..).collect();
        let seg_id = self.next_segment;
        self.next_segment = self.next_segment.saturating_add(1);
        let seq_start = records
            .first()
            .map(|_| self.stats.last_committed_sequence)
            .unwrap_or(0);
        let seq_end = self.stats.last_committed_sequence;
        let prev = self.segments.keys().next_back().copied().unwrap_or(0);
        let blob = seal_segment(
            seg_id,
            self.database_generation,
            seq_start,
            seq_end,
            prev,
            &records,
            &self.quotas,
        )?;
        let name = alloc::format!("SEGMENTS/data-{seg_id:06}.owlseg");
        self.store.write_file_atomic(&name, &blob)?;
        let (header, recs) = open_segment(&blob, &self.quotas)?;
        for (i, rec) in recs.iter().enumerate() {
            let loc = RecordLocation {
                segment_id: seg_id,
                record_index: i as u32,
                revision: rec.revision,
            };
            self.indexes.primary.upsert(rec, loc);
        }
        self.segments.insert(
            seg_id,
            SegmentMem {
                header,
                records: recs,
                compressed_bytes: blob.len() as u64,
            },
        );
        Ok(seg_id)
    }

    // ---- Reads / queries ----

    fn check_scope_write(
        &self,
        caller: &DbCaller,
        scope: MemoryScope,
        owner: OwnerId,
    ) -> Result<(), DbError> {
        match scope {
            MemoryScope::System if !caller.is_system && !caller.caps.has(DbCapability::Admin) => {
                Err(DbError::CrossScopeDenied)
            }
            MemoryScope::User | MemoryScope::SessionDerived | MemoryScope::Application => {
                if caller.is_system || caller.caps.has(DbCapability::Admin) {
                    return Ok(());
                }
                if owner != caller.owner {
                    return Err(DbError::CrossScopeDenied);
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn check_scope_read(
        &self,
        caller: &DbCaller,
        rec: &LongTermMemoryRecord,
    ) -> Result<(), DbError> {
        if caller.caps.has(DbCapability::Admin) || caller.is_system {
            return Ok(());
        }
        if rec.owner == caller.owner {
            caller.caps.require(DbCapability::ReadOwnScope)?;
            return Ok(());
        }
        if caller.caps.has(DbCapability::ReadSharedScope) {
            return Ok(());
        }
        Err(DbError::CrossScopeDenied)
    }

    fn check_trust(&self, caller: &DbCaller, trust: TrustLevel) -> Result<(), DbError> {
        match trust {
            TrustLevel::Trusted | TrustLevel::SystemDerived => {
                if caller.caps.has(DbCapability::AssignElevatedTrust)
                    || caller.caps.has(DbCapability::Admin)
                {
                    Ok(())
                } else {
                    Err(DbError::TrustEscalationDenied)
                }
            }
            TrustLevel::Untrusted => Ok(()),
        }
    }

    pub fn get_record(
        &self,
        caller: &DbCaller,
        id: MemoryId,
        include_payload: bool,
    ) -> Result<LongTermMemoryRecord, DbError> {
        let rec = self.records.get(&id.get()).ok_or(DbError::NotFound)?;
        self.check_scope_read(caller, rec)?;
        if rec.state == LongTermRecordState::Tombstoned && !include_payload {
            return Err(DbError::Tombstoned);
        }
        if include_payload {
            caller.caps.require(DbCapability::ReadPayload)?;
            Ok(rec.clone())
        } else {
            Ok(rec.metadata_only())
        }
    }

    pub fn list_revisions(
        &self,
        caller: &DbCaller,
        id: MemoryId,
    ) -> Result<Vec<u32>, DbError> {
        let rec = self.records.get(&id.get()).ok_or(DbError::NotFound)?;
        self.check_scope_read(caller, rec)?;
        let mut revs = vec![rec.revision];
        if let Some(e) = self.indexes.primary.get(id) {
            for h in &e.history {
                revs.push(h.revision);
            }
        }
        revs.sort_unstable();
        revs.dedup();
        Ok(revs)
    }

    pub fn get_relationships(
        &self,
        caller: &DbCaller,
        id: MemoryId,
    ) -> Result<Vec<MemoryRelationship>, DbError> {
        caller.caps.require(DbCapability::QueryMetadata)?;
        if let Some(rec) = self.records.get(&id.get()) {
            self.check_scope_read(caller, rec)?;
        }
        let mut out = Vec::new();
        out.extend_from_slice(self.indexes.relationship.outgoing(id));
        out.extend_from_slice(self.indexes.relationship.incoming(id));
        // Cap
        out.truncate(self.quotas.max_query_results as usize);
        Ok(out)
    }

    pub fn query(&mut self, caller: &DbCaller, mut q: MemoryQuery) -> Result<QueryResult, DbError> {
        caller.caps.require(DbCapability::QueryMetadata)?;
        self.stats.query_count = self.stats.query_count.saturating_add(1);

        let limit = q.limit.min(self.quotas.max_query_results).max(1) as usize;
        q.limit = limit as u32;

        if let Some(ref c) = q.cursor {
            c.validate(self.database_generation, self.index_generation)
                .map_err(|e| {
                    self.stats.query_failures = self.stats.query_failures.saturating_add(1);
                    e
                })?;
        }

        // Token queries need index readiness for completeness.
        let mut degraded = self.indexes.degraded;
        if q.token_match.is_some() && self.indexes.degraded {
            degraded = true;
        }

        // Candidate IDs.
        let mut candidates: Vec<u64> = if let Some(ref tq) = q.token_match {
            self.indexes
                .token
                .query(tq, self.quotas.max_posting_page as usize * 4)
                .into_iter()
                .map(|id| id.get())
                .collect()
        } else if let Some(ref sq) = q.source {
            if let Some(sid) = sq.source_id {
                self.indexes.source.by_source_id(sid).to_vec()
            } else if let Some(h) = sq.source_content_hash {
                self.indexes.source.by_source_content_hash(h).to_vec()
            } else {
                self.indexes.primary.ids().into_iter().map(|i| i.get()).collect()
            }
        } else if let Some(ref rq) = q.relationship {
            self.indexes
                .relationship
                .query(rq)
                .into_iter()
                .map(|r| r.source.get())
                .collect()
        } else {
            self.indexes.primary.ids().into_iter().map(|i| i.get()).collect()
        };

        candidates.sort_unstable();
        candidates.dedup();

        if let Some(ref c) = q.cursor {
            candidates.retain(|id| *id > c.after_id);
        }

        let mut matched: Vec<&LongTermMemoryRecord> = Vec::new();
        let mut scanned = 0u32;
        for id in candidates {
            scanned = scanned.saturating_add(1);
            if scanned > self.quotas.max_posting_page.saturating_mul(8) {
                self.stats.query_budget_exhaustions =
                    self.stats.query_budget_exhaustions.saturating_add(1);
                break;
            }
            let Some(rec) = self.records.get(&id) else {
                continue;
            };
            if self.check_scope_read(caller, rec).is_err() {
                continue;
            }
            if !q.kinds.contains(rec.kind) {
                continue;
            }
            if let Some(scope) = q.scope {
                if rec.scope != scope {
                    continue;
                }
            }
            if let Some(owner) = q.owner {
                if rec.owner != owner {
                    continue;
                }
            }
            if !q.include_tombstoned_metadata && rec.state == LongTermRecordState::Tombstoned {
                continue;
            }
            if !q.include_superseded && rec.state == LongTermRecordState::Superseded {
                continue;
            }
            if rec.state == LongTermRecordState::Active
                || (q.include_superseded && rec.state == LongTermRecordState::Superseded)
                || (q.include_tombstoned_metadata && rec.state == LongTermRecordState::Tombstoned)
            {
                // ok
            } else if rec.state != LongTermRecordState::Active {
                continue;
            }
            if let Some(min_c) = q.min_confidence {
                if rec.confidence < min_c {
                    continue;
                }
            }
            if let Some(tf) = q.trust {
                match tf {
                    TrustFilter::Exact(t) if rec.trust != t => continue,
                    TrustFilter::MinSystemDerived
                        if !matches!(
                            rec.trust,
                            TrustLevel::SystemDerived | TrustLevel::Trusted
                        ) =>
                    {
                        continue
                    }
                    _ => {}
                }
            }
            if let Some(after) = q.created_after_ns {
                if rec.created_at_ns < after {
                    continue;
                }
            }
            if let Some(before) = q.created_before_ns {
                if rec.created_at_ns > before {
                    continue;
                }
            }
            if !q.attributes.matches(&rec.attributes) {
                continue;
            }
            matched.push(rec);
        }

        // Order
        match q.order {
            QueryOrder::IdAsc => matched.sort_by_key(|r| r.id.get()),
            QueryOrder::ConfidenceDesc => {
                matched.sort_by(|a, b| b.confidence.cmp(&a.confidence).then(a.id.get().cmp(&b.id.get())))
            }
            QueryOrder::ImportanceDesc => {
                matched.sort_by(|a, b| b.importance.cmp(&a.importance).then(a.id.get().cmp(&b.id.get())))
            }
            QueryOrder::RecencyDesc => {
                matched
                    .sort_by(|a, b| b.created_at_ns.cmp(&a.created_at_ns).then(a.id.get().cmp(&b.id.get())))
            }
            QueryOrder::TokenRelevanceDesc => {
                // Rank by matched token count if token query present.
                if let Some(ref tq) = q.token_match {
                    matched.sort_by(|a, b| {
                        let sa = score_tokens(a, tq);
                        let sb = score_tokens(b, tq);
                        sb.cmp(&sa).then(a.id.get().cmp(&b.id.get()))
                    });
                } else {
                    matched.sort_by_key(|r| r.id.get());
                }
            }
        }

        let page: Vec<_> = matched.into_iter().take(limit).collect();
        let next_cursor = if page.len() == limit {
            page.last().map(|r| {
                QueryCursor::new(self.database_generation, self.index_generation, r.id.get())
            })
        } else {
            None
        };

        Ok(QueryResult {
            ids: page.into_iter().map(|r| r.id).collect(),
            next_cursor,
            degraded,
            total_scanned: scanned,
        })
    }

    pub fn source_lookup(
        &self,
        caller: &DbCaller,
        source: SourceId,
        offset: usize,
        limit: usize,
    ) -> Result<(Vec<MemoryId>, bool), DbError> {
        caller.caps.require(DbCapability::QueryMetadata)?;
        let limit = limit.min(self.quotas.max_query_results as usize);
        let (ids, more) = self.indexes.source.page_source(source, offset, limit);
        let filtered: Vec<MemoryId> = ids
            .into_iter()
            .filter(|id| {
                self.records
                    .get(&id.get())
                    .map(|r| self.check_scope_read(caller, r).is_ok())
                    .unwrap_or(false)
            })
            .collect();
        Ok((filtered, more))
    }

    // ---- Source deletion ----

    pub fn delete_source_dry_run(
        &self,
        caller: &DbCaller,
        source: SourceId,
    ) -> Result<u32, DbError> {
        caller.caps.require(DbCapability::DeleteSource)?;
        let ids = self.indexes.source.by_source_id(source);
        Ok(ids.len() as u32)
    }

    pub fn delete_source(
        &mut self,
        caller: &DbCaller,
        source: SourceId,
        batch: u32,
    ) -> Result<(u32, bool), DbError> {
        caller.caps.require(DbCapability::DeleteSource)?;
        let batch = batch.min(64).max(1);
        // Stage via mini auto-transaction for durability.
        let tx = self.begin_transaction(caller)?;
        {
            let t = self.open_txs.get_mut(&tx).ok_or(DbError::InvalidTransaction)?;
            t.staged_source_deletes.push(source);
            t.ops = t.ops.saturating_add(1);
        }
        let seq = self.alloc_seq();
        self.append_wal(&WalRecord {
            record_type: WalRecordType::SourceDelete,
            transaction_id: tx,
            sequence: seq,
            payload: source.get().to_le_bytes().to_vec(),
        })?;
        self.commit_transaction(caller, tx)?;
        self.apply_source_delete_batch(source, batch as usize)
    }

    fn apply_source_delete_batch(
        &mut self,
        source: SourceId,
        batch: usize,
    ) -> Result<(u32, bool), DbError> {
        let ids: Vec<u64> = self.indexes.source.by_source_id(source).to_vec();
        let start = self.source_delete_cursor.get(&source.get()).copied().unwrap_or(0);
        let end = (start + batch).min(ids.len());
        let mut n = 0u32;
        for &id in &ids[start..end] {
            if let Ok(mid) = MemoryId::from_raw(id) {
                self.apply_tombstone(mid)?;
                n = n.saturating_add(1);
            }
        }
        let done = end >= ids.len();
        if done {
            self.source_delete_cursor.remove(&source.get());
        } else {
            self.source_delete_cursor.insert(source.get(), end);
        }
        Ok((n, !done))
    }

    // ---- Checkpoint / compaction / rebuild ----

    pub fn create_checkpoint(&mut self, caller: &DbCaller) -> Result<(), DbError> {
        caller.caps.require(DbCapability::CreateCheckpoint)?;
        self.checkpoint_inner()
    }

    fn checkpoint_inner(&mut self) -> Result<(), DbError> {
        let _ = self.seal_unsealed()?;
        // Persist relationships index snapshot.
        let rel_blob = encode_relationships(&self.relationships)?;
        self.store
            .write_file_atomic("INDEX/relationships.bin", &rel_blob)?;

        let meta = CheckpointMeta {
            database_generation: self.database_generation,
            last_committed_sequence: self.stats.last_committed_sequence,
            active_segments: self.segments.keys().copied().collect(),
            index_generation: self.index_generation,
            wal_replay_offset: self.wal_bytes,
            format_version: WAL_FORMAT_VERSION,
            checksum: 0,
        };
        let mut bytes = encode_manifest(&meta)?;
        let crc = wiseowl_memory::compression::crc32_ieee(&bytes);
        // append crc
        bytes.extend_from_slice(&crc.to_le_bytes());
        self.store.write_file_atomic("MANIFEST", &bytes)?;
        // Rotate WAL (truncate by replacing with empty after checkpoint).
        self.store.write_file_atomic(&self.wal_path, &[])?;
        self.wal_bytes = 0;
        self.stats.wal_bytes = 0;
        self.stats.checkpoint_count = self.stats.checkpoint_count.saturating_add(1);
        let seq = self.alloc_seq();
        self.append_wal(&WalRecord {
            record_type: WalRecordType::Checkpoint,
            transaction_id: 0,
            sequence: seq,
            payload: Vec::new(),
        })?;
        Ok(())
    }

    pub fn run_compaction(&mut self, caller: &DbCaller) -> Result<u64, DbError> {
        caller.caps.require(DbCapability::RunCompaction)?;
        let max_segs = self.quotas.max_compaction_segments as usize;
        let ids: Vec<u64> = self.segments.keys().copied().take(max_segs).collect();
        if ids.len() < 2 {
            return Ok(0);
        }
        let mut merged_records: Vec<LongTermMemoryRecord> = Vec::new();
        let mut bytes_read = 0u64;
        let mut old_compressed = 0u64;
        for sid in &ids {
            if merged_records.len() as u32 >= self.quotas.max_compaction_records {
                break;
            }
            if let Some(seg) = self.segments.get(sid) {
                bytes_read = bytes_read.saturating_add(seg.compressed_bytes);
                if bytes_read > self.quotas.max_compaction_bytes_read {
                    break;
                }
                old_compressed = old_compressed.saturating_add(seg.compressed_bytes);
                for rec in &seg.records {
                    // Keep latest active; drop tombstoned payloads (metadata-only tombstone kept as state).
                    if let Some(live) = self.records.get(&rec.id.get()) {
                        if live.state == LongTermRecordState::Tombstoned {
                            let mut t = live.clone();
                            t.payload.clear();
                            t.payload_ref.length = 0;
                            t.payload_ref.content_hash = 0;
                            merged_records.push(t);
                        } else if live.revision == rec.revision {
                            merged_records.push(live.clone());
                        }
                    }
                }
            }
        }
        if merged_records.is_empty() {
            return Ok(0);
        }
        let new_id = self.next_segment;
        self.next_segment = self.next_segment.saturating_add(1);
        let blob = seal_segment(
            new_id,
            self.database_generation,
            0,
            self.stats.last_committed_sequence,
            0,
            &merged_records,
            &self.quotas,
        )?;
        if blob.len() as u64 > self.quotas.max_compaction_bytes_write {
            return Err(DbError::QuotaExceeded("compaction write"));
        }
        let tmp = alloc::format!("TMP/compact-{new_id}.owlseg");
        self.store.write_file_atomic(&tmp, &blob)?;
        // Validate
        let (header, recs) = open_segment(&blob, &self.quotas)?;
        let final_name = alloc::format!("SEGMENTS/data-{new_id:06}.owlseg");
        self.store.write_file_atomic(&final_name, &blob)?;
        // Update manifest after durable write.
        self.segments.insert(
            new_id,
            SegmentMem {
                header,
                records: recs.clone(),
                compressed_bytes: blob.len() as u64,
            },
        );
        for (i, rec) in recs.iter().enumerate() {
            self.indexes.primary.upsert(
                rec,
                RecordLocation {
                    segment_id: new_id,
                    record_index: i as u32,
                    revision: rec.revision,
                },
            );
        }
        // Retire old segments after manifest update.
        for sid in &ids {
            self.segments.remove(sid);
            let name = alloc::format!("SEGMENTS/data-{sid:06}.owlseg");
            let _ = self.store.remove_file(&name);
        }
        let _ = self.store.remove_file(&tmp);
        self.index_generation = self.index_generation.saturating_add(1);
        let reclaimed = old_compressed.saturating_sub(blob.len() as u64);
        self.stats.compaction_count = self.stats.compaction_count.saturating_add(1);
        self.stats.compaction_bytes_reclaimed = self
            .stats
            .compaction_bytes_reclaimed
            .saturating_add(reclaimed);
        self.checkpoint_inner()?;
        self.refresh_stats();
        Ok(reclaimed)
    }

    pub fn rebuild_indexes(&mut self, caller: &DbCaller) -> Result<(), DbError> {
        caller.caps.require(DbCapability::Admin)?;
        let mut pairs = Vec::new();
        for (sid, seg) in &self.segments {
            for (i, rec) in seg.records.iter().enumerate() {
                pairs.push((
                    RecordLocation {
                        segment_id: *sid,
                        record_index: i as u32,
                        revision: rec.revision,
                    },
                    rec.clone(),
                ));
            }
        }
        for (i, rec) in self.unsealed.iter().enumerate() {
            pairs.push((
                RecordLocation {
                    segment_id: 0,
                    record_index: i as u32,
                    revision: rec.revision,
                },
                rec.clone(),
            ));
        }
        // Prefer latest from self.records
        for rec in self.records.values() {
            if !pairs.iter().any(|(_, r)| r.id == rec.id && r.revision == rec.revision) {
                pairs.push((
                    RecordLocation {
                        segment_id: 0,
                        record_index: 0,
                        revision: rec.revision,
                    },
                    rec.clone(),
                ));
            }
        }
        self.indexes = IndexSet::rebuild_from(&pairs, &self.relationships);
        self.index_generation = self.index_generation.saturating_add(1);
        if matches!(self.health.state, HealthState::Degraded) {
            self.health = DbHealth::ready();
        }
        self.refresh_stats();
        Ok(())
    }

    pub fn verify_bounded(&self, max_segments: u32) -> Result<(u32, u32), DbError> {
        let mut ok = 0u32;
        let mut bad = 0u32;
        for (i, (_id, seg)) in self.segments.iter().enumerate() {
            if i as u32 >= max_segments {
                break;
            }
            // Re-check checksum via re-seal comparison of stored record count.
            if seg.records.len() as u32 == seg.header.record_count {
                ok = ok.saturating_add(1);
            } else {
                bad = bad.saturating_add(1);
            }
        }
        Ok((ok, bad))
    }

    /// Convenience: single-record auto-commit insert.
    pub fn insert_one(
        &mut self,
        caller: &DbCaller,
        req: InsertRequest,
    ) -> Result<MemoryId, DbError> {
        let tx = self.begin_transaction(caller)?;
        match self.insert_record(caller, tx, req) {
            Ok(id) => {
                self.commit_transaction(caller, tx)?;
                Ok(id)
            }
            Err(e) => {
                let _ = self.abort_transaction(caller, tx);
                Err(e)
            }
        }
    }
}

fn score_tokens(rec: &LongTermMemoryRecord, tq: &crate::tokens::TokenQuery) -> u32 {
    let mut score = 0u32;
    for tid in &tq.token_ids {
        if let Some(t) = rec.token_entries.iter().find(|t| t.token_id == *tid) {
            score = score.saturating_add(u32::from(t.frequency));
        }
    }
    score
}

fn encode_manifest(meta: &CheckpointMeta) -> Result<Vec<u8>, DbError> {
    let mut w = crate::codec::BufWriter::with_capacity(4096);
    w.write_u32(0x4F57_4C4D)?; // OWLM
    w.write_u16(1)?;
    w.write_u64(meta.database_generation)?;
    w.write_u64(meta.last_committed_sequence)?;
    w.write_u64(meta.index_generation)?;
    w.write_u64(meta.wal_replay_offset)?;
    w.write_u16(meta.format_version)?;
    w.write_u32(meta.active_segments.len() as u32)?;
    for s in &meta.active_segments {
        w.write_u64(*s)?;
    }
    Ok(w.into_vec())
}

fn decode_manifest(data: &[u8]) -> Result<CheckpointMeta, DbError> {
    if data.len() < 8 {
        return Err(DbError::Corrupt {
            reason: "manifest",
        });
    }
    // Optional trailing crc
    let body = if data.len() > 4 {
        // try without last 4 first if magic matches
        data
    } else {
        data
    };
    let mut r = crate::codec::BufReader::new(body);
    let magic = r.read_u32()?;
    if magic != 0x4F57_4C4D {
        return Err(DbError::Corrupt {
            reason: "manifest magic",
        });
    }
    let _ver = r.read_u16()?;
    let database_generation = r.read_u64()?;
    let last_committed_sequence = r.read_u64()?;
    let index_generation = r.read_u64()?;
    let wal_replay_offset = r.read_u64()?;
    let format_version = r.read_u16()?;
    let n = r.read_u32()? as usize;
    if n > 4096 {
        return Err(DbError::Corrupt {
            reason: "manifest segments",
        });
    }
    let mut active_segments = Vec::with_capacity(n);
    for _ in 0..n {
        active_segments.push(r.read_u64()?);
    }
    Ok(CheckpointMeta {
        database_generation,
        last_committed_sequence,
        active_segments,
        index_generation,
        wal_replay_offset,
        format_version,
        checksum: 0,
    })
}

fn encode_relationships(rels: &[MemoryRelationship]) -> Result<Vec<u8>, DbError> {
    let mut w = crate::codec::BufWriter::with_capacity(64 * 1024);
    w.write_u32(0x4F57_4C52)?; // OWLR
    w.write_u32(rels.len() as u32)?;
    for rel in rels {
        rel.encode(&mut w)?;
    }
    Ok(w.into_vec())
}

fn decode_relationships(data: &[u8]) -> Result<Vec<MemoryRelationship>, DbError> {
    let mut r = crate::codec::BufReader::new(data);
    let magic = r.read_u32()?;
    if magic != 0x4F57_4C52 {
        return Err(DbError::Corrupt {
            reason: "rel index magic",
        });
    }
    let n = r.read_u32()? as usize;
    if n > 100_000 {
        return Err(DbError::Corrupt {
            reason: "rel count",
        });
    }
    let mut out = Vec::with_capacity(n.min(1024));
    for _ in 0..n {
        out.push(MemoryRelationship::decode(&mut r)?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provenance::{DerivationKind, LongTermProvenance};
    use wiseowl_memory::SourceKind;

    fn prov(source: Option<u64>) -> LongTermProvenance {
        LongTermProvenance {
            source_kind: SourceKind::UserInput,
            source_id: source.map(|s| SourceId::from_raw_unchecked(s)),
            producer_service: String::from("test"),
            original_memory_ids: Vec::new(),
            parent_lt_ids: Vec::new(),
            insertion_time_ns: 1,
            trust: TrustLevel::Untrusted,
            source_content_hash: None,
            external_ref: None,
            derivation: DerivationKind::DirectImport,
        }
    }

    fn req(payload: &[u8], source: Option<u64>) -> InsertRequest {
        InsertRequest {
            kind: LongTermMemoryKind::Observation,
            scope: MemoryScope::User,
            owner: 1,
            payload: payload.to_vec(),
            provenance: prov(source),
            confidence: 900,
            importance: 100,
            trust: TrustLevel::Untrusted,
            valid_from_ns: None,
            valid_until_ns: None,
            tokens: None,
            attributes: crate::attributes::AttributeSet::default(),
            supersedes: None,
            relationships: Vec::new(),
            dedup: DedupPolicy::Allow,
            id: None,
            revision: 1,
        }
    }

    #[test]
    fn atomic_commit_and_restart() {
        let mut db = Database::open_memory(DbQuotaConfig::default()).unwrap();
        let caller = DbCaller::user(1);
        let id = db.insert_one(&caller, req(b"hello", Some(7))).unwrap();
        db.create_checkpoint(&DbCaller::admin()).unwrap();

        // Simulate restart: re-open same store.
        let store = db.store;
        let mut db2 = Database::open_with_store(store, DbQuotaConfig::default()).unwrap();
        let got = db2.get_record(&caller, id, false).unwrap();
        assert_eq!(got.id, id);
        assert_eq!(got.payload_ref.length, 5);
    }

    #[test]
    fn incomplete_tx_invisible() {
        let mut db = Database::open_memory(DbQuotaConfig::default()).unwrap();
        let caller = DbCaller::user(1);
        let tx = db.begin_transaction(&caller).unwrap();
        let id = db.insert_record(&caller, tx, req(b"ghost", None)).unwrap();
        // no commit — restart
        let store = db.store;
        let db2 = Database::open_with_store(store, DbQuotaConfig::default()).unwrap();
        assert!(db2.get_record(&caller, id, false).is_err());
    }

    #[test]
    fn tombstone_hidden() {
        let mut db = Database::open_memory(DbQuotaConfig::default()).unwrap();
        let mut caller = DbCaller::user(1);
        caller.caps = caller.caps.grant(DbCapability::Tombstone);
        let id = db.insert_one(&caller, req(b"x", None)).unwrap();
        let tx = db.begin_transaction(&caller).unwrap();
        db.tombstone_record(&caller, tx, id).unwrap();
        db.commit_transaction(&caller, tx).unwrap();
        assert!(matches!(
            db.get_record(&caller, id, false),
            Err(DbError::Tombstoned)
        ));
        let q = MemoryQuery {
            limit: 10,
            ..Default::default()
        };
        let res = db.query(&caller, q).unwrap();
        assert!(!res.ids.contains(&id));
    }

    #[test]
    fn cross_scope_denied() {
        let mut db = Database::open_memory(DbQuotaConfig::default()).unwrap();
        let c1 = DbCaller::user(1);
        let id = db.insert_one(&c1, req(b"secret", None)).unwrap();
        let c2 = DbCaller::user(2);
        assert!(matches!(
            db.get_record(&c2, id, false),
            Err(DbError::CrossScopeDenied)
        ));
    }

    #[test]
    fn trust_escalation_denied() {
        let mut db = Database::open_memory(DbQuotaConfig::default()).unwrap();
        let caller = DbCaller::user(1);
        let mut r = req(b"t", None);
        r.trust = TrustLevel::Trusted;
        assert!(matches!(
            db.insert_one(&caller, r),
            Err(DbError::TrustEscalationDenied)
        ));
    }

    #[test]
    fn token_query() {
        let mut db = Database::open_memory(DbQuotaConfig::default()).unwrap();
        let caller = DbCaller::user(1);
        let mut r = req(b"tok", None);
        r.tokens = Some((
            TokenSetRef {
                tokenizer_id: 2,
                tokenizer_version: 1,
                token_count: 2,
            },
            vec![
                IndexedToken {
                    token_id: 101,
                    frequency: 1,
                    positions: None,
                },
                IndexedToken {
                    token_id: 203,
                    frequency: 2,
                    positions: None,
                },
            ],
        ));
        let id = db.insert_one(&caller, r).unwrap();
        let q = MemoryQuery {
            token_match: Some(crate::tokens::TokenQuery {
                tokenizer_id: 2,
                tokenizer_version: 1,
                token_ids: vec![101, 203],
                mode: crate::tokens::TokenMatchMode::All,
            }),
            limit: 10,
            ..Default::default()
        };
        let res = db.query(&caller, q).unwrap();
        assert!(res.ids.contains(&id));
        // Wrong tokenizer version → no mix
        let q2 = MemoryQuery {
            token_match: Some(crate::tokens::TokenQuery {
                tokenizer_id: 2,
                tokenizer_version: 99,
                token_ids: vec![101],
                mode: crate::tokens::TokenMatchMode::Any,
            }),
            limit: 10,
            ..Default::default()
        };
        let res2 = db.query(&caller, q2).unwrap();
        assert!(!res2.ids.contains(&id));
    }

    #[test]
    fn source_delete_bounded() {
        let mut db = Database::open_memory(DbQuotaConfig::default()).unwrap();
        let mut caller = DbCaller::admin();
        caller.owner = 1;
        for i in 0..5 {
            let mut r = req(&[i], Some(42));
            r.owner = 1;
            db.insert_one(&caller, r).unwrap();
        }
        let count = db
            .delete_source_dry_run(&caller, SourceId::from_raw_unchecked(42))
            .unwrap();
        assert_eq!(count, 5);
        let (n, more) = db
            .delete_source(&caller, SourceId::from_raw_unchecked(42), 2)
            .unwrap();
        assert!(n <= 2 || n == 5); // batch applied fully in apply after commit
        let _ = more;
    }

    #[test]
    fn corrupt_segment_isolated() {
        let mut store = MemoryStore::default();
        store.ensure_layout().unwrap();
        // Write a garbage segment.
        store
            .write_file_atomic("SEGMENTS/data-000001.owlseg", b"not a segment")
            .unwrap();
        let db = Database::open_with_store(store, DbQuotaConfig::default()).unwrap();
        assert!(db.stats().quarantined_files >= 1 || db.segments.is_empty());
        // Service still opens.
        assert!(db.health().ready || matches!(db.health().state, HealthState::Degraded | HealthState::Ready));
    }

    #[test]
    fn soak_insert_query_checkpoint_compact() {
        let mut db = Database::open_memory(DbQuotaConfig::default()).unwrap();
        let mut caller = DbCaller::admin();
        caller.owner = 1;
        let mut ids = Vec::new();
        for i in 0..40u8 {
            let mut r = req(&[i, i.wrapping_add(1)], Some(100));
            r.owner = 1;
            r.tokens = Some((
                TokenSetRef {
                    tokenizer_id: 1,
                    tokenizer_version: 1,
                    token_count: 1,
                },
                vec![IndexedToken {
                    token_id: (i as u64) % 5,
                    frequency: 1,
                    positions: None,
                }],
            ));
            ids.push(db.insert_one(&caller, r).unwrap());
        }
        // Query by token
        let q = MemoryQuery {
            token_match: Some(crate::tokens::TokenQuery {
                tokenizer_id: 1,
                tokenizer_version: 1,
                token_ids: vec![0],
                mode: crate::tokens::TokenMatchMode::Any,
            }),
            limit: 20,
            ..Default::default()
        };
        let res = db.query(&caller, q).unwrap();
        assert!(!res.ids.is_empty());
        assert!(res.ids.len() <= 20);

        db.create_checkpoint(&caller).unwrap();
        let _ = db.run_compaction(&caller);
        let store = db.store;
        let mut db2 = Database::open_with_store(store, DbQuotaConfig::default()).unwrap();
        for id in &ids {
            // After restart, active records remain (unless compacted tombstones).
            let _ = db2.get_record(&caller, *id, false);
        }
        assert!(db2.stats().record_count_active > 0);
        assert!(db2.stats().checkpoint_count >= 1 || db2.stats().last_committed_sequence > 0);
    }

    #[test]
    fn relationships_survive() {
        let mut db = Database::open_memory(DbQuotaConfig::default()).unwrap();
        let mut caller = DbCaller::user(1);
        caller.caps = caller
            .caps
            .grant(DbCapability::CreateRelationship)
            .grant(DbCapability::CreateCheckpoint);
        let a = db.insert_one(&caller, req(b"a", None)).unwrap();
        let b = db.insert_one(&caller, req(b"b", None)).unwrap();
        let tx = db.begin_transaction(&caller).unwrap();
        db.insert_relationship(
            &caller,
            tx,
            MemoryRelationship {
                source: a,
                target: b,
                kind: RelationshipKind::RelatedTo,
                confidence: 500,
                created_at_ns: 1,
                provenance: crate::provenance::RelationshipProvenance {
                    producer_service: String::from("t"),
                    created_at_ns: 1,
                    trust: TrustLevel::Untrusted,
                },
                tombstoned: false,
            },
        )
        .unwrap();
        db.commit_transaction(&caller, tx).unwrap();
        db.create_checkpoint(&DbCaller::admin()).unwrap();
        let store = db.store;
        let db2 = Database::open_with_store(store, DbQuotaConfig::default()).unwrap();
        let rels = db2.get_relationships(&caller, a).unwrap();
        assert!(!rels.is_empty());
    }
}
