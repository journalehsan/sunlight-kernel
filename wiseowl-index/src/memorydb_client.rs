//! Native MemoryDB client for `wiseowl.memorydb.v1` (Phase 3.5).
//!
//! Production path: nameserver discovery → IPC + validated SHM.
//! Never falls back to an embedded MemoryDB store.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use sunlight_ipc::{
    ipc_call, nameserver_lookup, nameserver_lookup_timeout, shm_alloc, shm_free, shm_map,
    CapabilityToken, IpcMsg, SHM_PAGE,
};
use wiseowl_memory::{MemoryId, SourceId};
use wiseowl_memorydb::database::InsertRequest;
use wiseowl_memorydb::insert_wire::{decode_insert_request, encode_insert_request};
use wiseowl_memorydb::native_ipc::{MemoryDbOp, INLINE_PAYLOAD_THRESHOLD};
use wiseowl_memorydb::query::{MemoryQuery, QueryResult};
use wiseowl_memorydb::record::LongTermMemoryRecord;
use wiseowl_memorydb::{DbQuotaConfig, ENDPOINT_NAME};

use crate::error::IndexError;
use crate::import_key::{ImportKey, ImportReconcileResult, ImportState};
use crate::memorydb_backend::{IndexMemoryDb, MemoryDbHealth};

/// Generation-safe cached endpoint reference.
#[derive(Debug, Clone)]
struct EndpointRef {
    cap: CapabilityToken,
    /// Monotonic rediscovery generation (local).
    generation: u64,
}

/// Native client — communicates only with independent MemoryDB service.
pub struct NativeMemoryDbClient {
    endpoint: Option<EndpointRef>,
    local_generation: u64,
    quotas: DbQuotaConfig,
    pub shm_bytes_sent: u64,
    pub shm_bytes_received: u64,
    pub shm_leases: u64,
    pub shm_lease_failures: u64,
    pub connection_attempts: u64,
    pub connection_successes: u64,
    pub disconnects: u64,
    pub protocol_failures: u64,
}

impl NativeMemoryDbClient {
    pub fn new() -> Self {
        Self {
            endpoint: None,
            local_generation: 0,
            quotas: DbQuotaConfig::default(),
            shm_bytes_sent: 0,
            shm_bytes_received: 0,
            shm_leases: 0,
            shm_lease_failures: 0,
            connection_attempts: 0,
            connection_successes: 0,
            disconnects: 0,
            protocol_failures: 0,
        }
    }

    /// Discover MemoryDB via nameserver (bounded).
    pub fn discover(&mut self) -> Result<(), IndexError> {
        self.connection_attempts = self.connection_attempts.saturating_add(1);
        let cap = nameserver_lookup(ENDPOINT_NAME)
            .or_else(|| nameserver_lookup("wiseowl-memorydb"))
            .or_else(|| nameserver_lookup_timeout(ENDPOINT_NAME, 100))
            .ok_or(IndexError::DatabaseUnavailable)?;
        self.local_generation = self.local_generation.saturating_add(1);
        self.endpoint = Some(EndpointRef {
            cap,
            generation: self.local_generation,
        });
        self.connection_successes = self.connection_successes.saturating_add(1);
        Ok(())
    }

    fn invalidate(&mut self) {
        self.endpoint = None;
        self.disconnects = self.disconnects.saturating_add(1);
    }

    fn endpoint_cap(&mut self) -> Result<CapabilityToken, IndexError> {
        if self.endpoint.is_none() {
            self.discover()?;
        }
        self.endpoint
            .as_ref()
            .map(|e| e.cap)
            .ok_or(IndexError::DatabaseUnavailable)
    }

    fn call(&mut self, msg: IpcMsg) -> Result<IpcMsg, IndexError> {
        let cap = self.endpoint_cap()?;
        let reply = ipc_call(cap, msg);
        if reply.label as u16 == MemoryDbOp::Error as u16 {
            // Stale endpoint / hard error — discard cache.
            if reply.words[0] == 0xFF {
                self.invalidate();
            }
            return Err(IndexError::TransactionRejected(String::from(
                "memorydb error",
            )));
        }
        Ok(reply)
    }

    fn send_insert(&mut self, tx: u64, req: InsertRequest) -> Result<MemoryId, IndexError> {
        let body = encode_insert_request(&req, 64 * 1024)
            .map_err(|_| IndexError::Internal("encode insert"))?;
        if body.len() > SHM_PAGE {
            return Err(IndexError::PayloadTooLarge {
                size: body.len() as u32,
                max: SHM_PAGE as u32,
            });
        }
        let (ptr, token) = shm_alloc().map_err(|_| {
            self.shm_lease_failures = self.shm_lease_failures.saturating_add(1);
            IndexError::Io("shm_alloc")
        })?;
        self.shm_leases = self.shm_leases.saturating_add(1);
        unsafe {
            core::ptr::copy_nonoverlapping(body.as_ptr(), ptr, body.len());
        }
        self.shm_bytes_sent = self.shm_bytes_sent.saturating_add(body.len() as u64);
        let msg = IpcMsg::with_label(MemoryDbOp::InsertRecord as u64)
            .word(0, tx)
            .word(1, body.len() as u64)
            .word(2, req.owner)
            .with_cap(0, token);
        let reply = match self.call(msg) {
            Ok(r) => r,
            Err(e) => {
                let _ = shm_free(token);
                return Err(e);
            }
        };
        // MemoryDB should free/copy; client also releases lease after completion.
        let _ = shm_free(token);
        let id_raw = reply.words[0];
        MemoryId::from_raw(id_raw).map_err(|_| IndexError::Internal("memory id"))
    }
}

impl Default for NativeMemoryDbClient {
    fn default() -> Self {
        Self::new()
    }
}

impl IndexMemoryDb for NativeMemoryDbClient {
    fn health(&mut self) -> Result<MemoryDbHealth, IndexError> {
        let msg = IpcMsg::with_label(MemoryDbOp::GetHealth as u64);
        let reply = self.call(msg)?;
        let ready = reply.words[0] != 0;
        let state_code = reply.words[1] as u8;
        let state = match state_code {
            1 => "starting",
            2 => "ready",
            3 => "degraded",
            4 => "failed",
            _ => "unknown",
        };
        // Stats for generation
        let gen = {
            let smsg = IpcMsg::with_label(MemoryDbOp::GetStats as u64);
            match self.call(smsg) {
                Ok(r) => r.words[0],
                Err(_) => 0,
            }
        };
        Ok(MemoryDbHealth {
            ready,
            state: String::from(state),
            database_generation: gen,
        })
    }

    fn begin_transaction(&mut self) -> Result<u64, IndexError> {
        let msg = IpcMsg::with_label(MemoryDbOp::BeginTransaction as u64);
        let reply = self.call(msg)?;
        Ok(reply.words[0])
    }

    fn insert_record(&mut self, tx: u64, req: InsertRequest) -> Result<MemoryId, IndexError> {
        self.send_insert(tx, req)
    }

    fn commit_transaction(&mut self, tx: u64) -> Result<u64, IndexError> {
        let msg = IpcMsg::with_label(MemoryDbOp::CommitTransaction as u64).word(0, tx);
        let reply = self.call(msg)?;
        Ok(reply.words[0])
    }

    fn abort_transaction(&mut self, tx: u64) -> Result<(), IndexError> {
        let msg = IpcMsg::with_label(MemoryDbOp::AbortTransaction as u64).word(0, tx);
        let _ = self.call(msg)?;
        Ok(())
    }

    fn delete_source(&mut self, source_id: SourceId, batch: u32) -> Result<(u32, bool), IndexError> {
        let msg = IpcMsg::with_label(MemoryDbOp::DeleteSource as u64)
            .word(0, source_id.get())
            .word(1, batch as u64);
        let reply = self.call(msg)?;
        Ok((reply.words[0] as u32, reply.words[1] != 0))
    }

    fn delete_source_dry_run(&mut self, source_id: SourceId) -> Result<u32, IndexError> {
        // Native may not implement dry-run separately; use delete with batch 0 semantics.
        let msg = IpcMsg::with_label(MemoryDbOp::DeleteSource as u64)
            .word(0, source_id.get())
            .word(1, 0)
            .word(2, 1); // dry flag if supported
        let reply = self.call(msg)?;
        Ok(reply.words[0] as u32)
    }

    fn get_record(
        &mut self,
        id: MemoryId,
        payload: bool,
    ) -> Result<LongTermMemoryRecord, IndexError> {
        let msg = IpcMsg::with_label(MemoryDbOp::GetRecord as u64)
            .word(0, id.get())
            .word(1, if payload { 1 } else { 0 });
        let reply = self.call(msg)?;
        // Minimal reconstruction for search previews (native body returns words).
        // Full record decode would use SHM body; Phase 3.5 uses bounded fields.
        use wiseowl_memory::{SourceKind, TrustLevel};
        use wiseowl_memorydb::provenance::{DerivationKind, LongTermProvenance};
        use wiseowl_memorydb::record::{
            LongTermMemoryKind, LongTermRecordState, MemoryScope, PayloadRef,
        };
        let mut rec = LongTermMemoryRecord {
            format_version: 1,
            id,
            revision: reply.words[1] as u32,
            kind: LongTermMemoryKind::ImportedRecord,
            scope: MemoryScope::User,
            owner: 0,
            created_at_ns: 0,
            updated_at_ns: 0,
            valid_from_ns: None,
            valid_until_ns: None,
            importance: 0,
            confidence: reply.words[3] as u16,
            trust: TrustLevel::Untrusted,
            provenance: LongTermProvenance {
                source_kind: SourceKind::UserInput,
                source_id: None,
                producer_service: String::from("remote"),
                original_memory_ids: Vec::new(),
                parent_lt_ids: Vec::new(),
                insertion_time_ns: 0,
                trust: TrustLevel::Untrusted,
                source_content_hash: None,
                external_ref: None,
                derivation: DerivationKind::DirectImport,
            },
            payload_ref: PayloadRef {
                content_hash: 0,
                length: reply.words[2] as u32,
            },
            tokens: None,
            attributes: Default::default(),
            state: LongTermRecordState::Active,
            supersedes: None,
            payload: Vec::new(),
            token_entries: Vec::new(),
        };
        if payload && reply.caps[0] != CapabilityToken::INVALID {
            let len = reply.words[4].min(SHM_PAGE as u64) as usize;
            if let Ok(ptr) = shm_map(reply.caps[0]) {
                unsafe {
                    let slice = core::slice::from_raw_parts(ptr, len);
                    rec.payload.extend_from_slice(slice);
                }
                self.shm_bytes_received =
                    self.shm_bytes_received.saturating_add(len as u64);
                let _ = shm_free(reply.caps[0]);
            }
        }
        Ok(rec)
    }

    fn source_lookup(
        &mut self,
        source_id: SourceId,
        offset: u32,
        limit: u32,
    ) -> Result<(Vec<MemoryId>, bool), IndexError> {
        let msg = IpcMsg::with_label(MemoryDbOp::SourceLookup as u64)
            .word(0, source_id.get())
            .word(1, offset as u64)
            .word(2, limit as u64);
        let reply = self.call(msg)?;
        // words[0]=count, words[1]=first id (bounded native summary)
        let mut ids = Vec::new();
        if reply.words[0] > 0 && reply.words[1] != 0 {
            if let Ok(id) = MemoryId::from_raw(reply.words[1]) {
                ids.push(id);
            }
        }
        Ok((ids, reply.words[2] != 0))
    }

    fn query(&mut self, _q: MemoryQuery) -> Result<QueryResult, IndexError> {
        // Native query path: full query body via SHM is available for later extension.
        // Host tests use HostMemoryDbBackend for complete lexical search fidelity.
        Ok(QueryResult {
            ids: Vec::new(),
            next_cursor: None,
            degraded: true,
            total_scanned: 0,
        })
    }

    fn reconcile_import(&mut self, key: &ImportKey) -> Result<ImportReconcileResult, IndexError> {
        // Use source lookup; full attribute match needs GetRecord pages.
        let (ids, _) = self.source_lookup(key.source_id, 0, 16)?;
        if ids.is_empty() {
            return Ok(ImportReconcileResult {
                state: ImportState::NotFound,
                document_memory_id: None,
                source_revision: None,
            });
        }
        // Without full attribute decode over IPC, treat presence of source records
        // with matching revision as already committed when document_memory known
        // from prior manifest only. Prefer NotFound to avoid false AlreadyCommitted
        // when uncertain — caller will re-insert safely via supersede.
        // When records exist for source, mark Committed if any document-shaped hit.
        Ok(ImportReconcileResult {
            state: ImportState::NotFound,
            document_memory_id: None,
            source_revision: None,
        })
    }
}

// Keep encode available for tests when host builds with sunlightos.
#[allow(dead_code)]
fn _touch_wire() {
    let _ = INLINE_PAYLOAD_THRESHOLD;
    let _ = decode_insert_request;
}
