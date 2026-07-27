//! Native MemoryDB client for `wiseowl.memorydb.v1` (Phase 3.5).
//!
//! Production path: nameserver discovery → IPC + validated SHM.
//! Never falls back to an embedded MemoryDB store.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use sunlight_ipc::{
    ipc_call_timeout, nameserver_lookup, nameserver_lookup_timeout, shm_alloc, shm_create, shm_free, shm_map,
    CapabilityToken, IpcMsg, SHM_PAGE,
};
use wiseowl_memory::{MemoryId, SourceId};
use wiseowl_memorydb::database::InsertRequest;
use wiseowl_memorydb::insert_wire::{decode_insert_request, encode_insert_request};
use wiseowl_memorydb::native_ipc::{
    decode_native_query_result, encode_native_query, MemoryDbOp, INLINE_PAYLOAD_THRESHOLD,
};
use wiseowl_memorydb::query::{MemoryQuery, QueryResult};
use wiseowl_memorydb::record::LongTermMemoryRecord;
use wiseowl_memorydb::{DbQuotaConfig, ENDPOINT_NAME};

use crate::error::IndexError;
use crate::import_key::{ImportKey, ImportReconcileResult, ImportState};
use crate::memorydb_backend::{IndexMemoryDb, MemoryDbHealth};

const PREPARED_STATE_PATH: &[u8] = b"/state/wiseowl-indexd/prepared-state.bin";
const PREPARED_STATE_TMP: &[u8] = b"/state/wiseowl-indexd/prepared-state.tmp";

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
    pub shm: crate::native_ipc::ShmCounters,
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
            shm: crate::native_ipc::ShmCounters::default(),
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
            // Capability tokens are kernel generation-checked endpoint
            // identities; expose that generation-bearing value, not a PID.
            generation: cap.0,
        });
        self.connection_successes = self.connection_successes.saturating_add(1);
        Ok(())
    }

    fn invalidate(&mut self) {
        self.endpoint = None;
        self.disconnects = self.disconnects.saturating_add(1);
    }

    pub fn endpoint_generation(&self) -> u64 {
        self.endpoint.as_ref().map(|e| e.generation).unwrap_or(0)
    }

    pub fn connected(&self) -> bool {
        self.endpoint.is_some()
    }

    #[cfg(feature = "phase375-test")]
    pub fn arm_memorydb_shm_crash(&mut self) -> Result<(), IndexError> {
        let reply = self.call(IpcMsg::with_label(MemoryDbOp::TestArmShmCrash as u64))?;
        if reply.words[0] == 1 { Ok(()) } else { Err(IndexError::InvalidRequest("arm shm crash")) }
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
        let reply = match ipc_call_timeout(cap, msg, 2_000) {
            Ok(reply) => reply,
            Err(_) => {
                self.invalidate();
                return Err(IndexError::DatabaseUnavailable);
            }
        };
        if reply.label as u16 == MemoryDbOp::Error as u16 {
            // Stale endpoint / hard error — discard cache.
            if reply.words[0] == 0xFF {
                self.invalidate();
            }
            return Err(IndexError::TransactionRejected(String::from(
                "memorydb error",
            )));
        }
        if reply.label as u16 != MemoryDbOp::Reply as u16 {
            self.invalidate();
            return Err(IndexError::DatabaseUnavailable);
        }
        Ok(reply)
    }

    fn send_insert(&mut self, tx: u64, req: InsertRequest) -> Result<MemoryId, IndexError> {
        let body = encode_insert_request(&req, 64 * 1024)
            .map_err(|_| IndexError::Internal("encode insert"))?;
        if body.len() > 64 * 1024 {
            return Err(IndexError::PayloadTooLarge {
                size: body.len() as u32,
                max: (64 * 1024) as u32,
            });
        }
        let allocation_bytes = body.len().max(1).saturating_add(SHM_PAGE - 1) / SHM_PAGE * SHM_PAGE;
        let (ptr, token) = shm_create(allocation_bytes, 0).map_err(|_| {
            self.shm_lease_failures = self.shm_lease_failures.saturating_add(1);
            IndexError::Io("shm_alloc")
        })?;
        self.shm.shm_allocations = self.shm.shm_allocations.saturating_add(1);
        self.shm.shm_shares = self.shm.shm_shares.saturating_add(1);
        self.shm.active_shm_leases = self.shm.active_shm_leases.saturating_add(1);
        self.shm.shm_bytes_active = self.shm.shm_bytes_active.saturating_add(allocation_bytes as u64);
        self.shm.shm_bytes_peak = self.shm.shm_bytes_peak.max(self.shm.shm_bytes_active);
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
                if shm_free(token).is_ok() {
                    self.note_owner_free(allocation_bytes);
                }
                return Err(e);
            }
        };
        self.shm.shm_maps = self.shm.shm_maps.saturating_add(1);
        self.shm.shm_unmaps = self.shm.shm_unmaps.saturating_add(1);
        // Owner-retained contract: MemoryDB only unmaps; the indexer frees.
        if shm_free(token).is_ok() {
            self.note_owner_free(allocation_bytes);
        }
        let id_raw = reply.words[0];
        MemoryId::from_raw(id_raw).map_err(|_| IndexError::Internal("memory id"))
    }

    fn note_owner_free(&mut self, bytes: usize) {
        self.shm.shm_owner_frees = self.shm.shm_owner_frees.saturating_add(1);
        self.shm.active_shm_leases = self.shm.active_shm_leases.saturating_sub(1);
        self.shm.shm_bytes_active = self.shm.shm_bytes_active.saturating_sub(bytes as u64);
    }
}

impl Default for NativeMemoryDbClient {
    fn default() -> Self {
        Self::new()
    }
}

impl IndexMemoryDb for NativeMemoryDbClient {
    fn persist_prepared_import(
        &mut self,
        manifest: &crate::source::SourceManifest,
    ) -> Result<(), IndexError> {
        let mut state = crate::state::IndexerState::new();
        state.insert_manifest(manifest.clone());
        let bytes = crate::operational_state::encode_state(&state)?;
        write_atomic_native(PREPARED_STATE_TMP, PREPARED_STATE_PATH, &bytes)
    }

    fn clear_prepared_import(&mut self, _source_id: SourceId) -> Result<(), IndexError> {
        let _ = sunlight_libc::unlink(PREPARED_STATE_PATH);
        Ok(())
    }

    fn health(&mut self) -> Result<MemoryDbHealth, IndexError> {
        let msg = IpcMsg::with_label(MemoryDbOp::GetHealth as u64);
        let reply = self.call(msg)?;
        if reply.words[2] != wiseowl_memorydb::native_ipc::NATIVE_PROTOCOL_VERSION as u64 {
            self.protocol_failures = self.protocol_failures.saturating_add(1);
            self.invalidate();
            return Err(IndexError::UnsupportedProtocolVersion {
                got: reply.words[2] as u16,
                want: wiseowl_memorydb::native_ipc::NATIVE_PROTOCOL_VERSION,
            });
        }
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
        #[cfg(feature = "phase375-test")]
        if sunlight_libc::stat(b"/state/wiseowl-indexd/crash-before-commit").is_ok() {
            let _ = sunlight_libc::unlink(b"/state/wiseowl-indexd/crash-before-commit");
            sunlight_ipc::ProcessExit::exit(73);
        }
        let msg = IpcMsg::with_label(MemoryDbOp::CommitTransaction as u64).word(0, tx);
        let reply = self.call(msg)?;
        #[cfg(feature = "phase375-test")]
        if sunlight_libc::stat(b"/state/wiseowl-indexd/crash-after-commit").is_ok() {
            let _ = sunlight_libc::unlink(b"/state/wiseowl-indexd/crash-after-commit");
            sunlight_ipc::ProcessExit::exit(74);
        }
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
                let release = IpcMsg::with_label(MemoryDbOp::ReleaseLease as u64)
                    .with_cap(0, reply.caps[0]);
                let _ = self.call(release);
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
        let (ptr, token) = shm_alloc().map_err(|_| IndexError::Io("shm_alloc"))?;
        self.shm.shm_allocations = self.shm.shm_allocations.saturating_add(1);
        self.shm.shm_shares = self.shm.shm_shares.saturating_add(1);
        self.shm.active_shm_leases = self.shm.active_shm_leases.saturating_add(1);
        self.shm.shm_bytes_active = self.shm.shm_bytes_active.saturating_add(SHM_PAGE as u64);
        self.shm.shm_bytes_peak = self.shm.shm_bytes_peak.max(self.shm.shm_bytes_active);
        let msg = IpcMsg::with_label(MemoryDbOp::SourceLookup as u64)
            .word(0, source_id.get())
            .word(1, offset as u64)
            .word(2, limit.min(64) as u64)
            .with_cap(0, token);
        let reply = match self.call(msg) {
            Ok(reply) => reply,
            Err(error) => {
                if shm_free(token).is_ok() { self.note_owner_free(SHM_PAGE); }
                return Err(error);
            }
        };
        self.shm.shm_maps = self.shm.shm_maps.saturating_add(1);
        self.shm.shm_unmaps = self.shm.shm_unmaps.saturating_add(1);
        let count = (reply.words[0] as usize).min(SHM_PAGE / 8);
        let mut ids = Vec::with_capacity(count);
        for chunk in unsafe { core::slice::from_raw_parts(ptr, count * 8) }.chunks_exact(8) {
            let raw = u64::from_le_bytes(chunk.try_into().unwrap());
            ids.push(MemoryId::from_raw(raw).map_err(|_| IndexError::Internal("source id page"))?);
        }
        if shm_free(token).is_ok() { self.note_owner_free(SHM_PAGE); }
        Ok((ids, reply.words[2] != 0))
    }

    fn query(&mut self, q: MemoryQuery) -> Result<QueryResult, IndexError> {
        let body = encode_native_query(&q).map_err(|_| IndexError::InvalidRequest("query"))?;
        if body.len() > SHM_PAGE {
            return Err(IndexError::PayloadTooLarge { size: body.len() as u32, max: SHM_PAGE as u32 });
        }
        let (request_ptr, request_token) = shm_alloc().map_err(|_| IndexError::Io("query request shm"))?;
        let (result_ptr, result_token) = match shm_alloc() {
            Ok(v) => v,
            Err(_) => { let _ = shm_free(request_token); return Err(IndexError::Io("query result shm")); }
        };
        unsafe { core::ptr::copy_nonoverlapping(body.as_ptr(), request_ptr, body.len()); }
        self.shm.shm_allocations = self.shm.shm_allocations.saturating_add(2);
        self.shm.shm_shares = self.shm.shm_shares.saturating_add(2);
        self.shm.active_shm_leases = self.shm.active_shm_leases.saturating_add(2);
        self.shm.shm_bytes_active = self.shm.shm_bytes_active.saturating_add((SHM_PAGE * 2) as u64);
        self.shm.shm_bytes_peak = self.shm.shm_bytes_peak.max(self.shm.shm_bytes_active);
        let msg = IpcMsg::with_label(MemoryDbOp::Query as u64)
            .word(0, body.len() as u64)
            .with_cap(0, request_token)
            .with_cap(1, result_token);
        let reply = match self.call(msg) {
            Ok(r) => r,
            Err(e) => {
                if shm_free(request_token).is_ok() { self.note_owner_free(SHM_PAGE); }
                if shm_free(result_token).is_ok() { self.note_owner_free(SHM_PAGE); }
                return Err(e);
            }
        };
        self.shm.shm_maps = self.shm.shm_maps.saturating_add(2);
        self.shm.shm_unmaps = self.shm.shm_unmaps.saturating_add(2);
        let result_len = reply.words[0] as usize;
        let result = if result_len <= SHM_PAGE {
            let bytes = unsafe { core::slice::from_raw_parts(result_ptr, result_len) };
            decode_native_query_result(bytes).map_err(|_| IndexError::InvalidRequest("query result"))
        } else {
            Err(IndexError::PayloadTooLarge { size: result_len as u32, max: SHM_PAGE as u32 })
        };
        if shm_free(request_token).is_ok() { self.note_owner_free(SHM_PAGE); }
        if shm_free(result_token).is_ok() { self.note_owner_free(SHM_PAGE); }
        result
    }

    fn reconcile_import(&mut self, key: &ImportKey) -> Result<ImportReconcileResult, IndexError> {
        let key_hex = key.key_hex();
        let (ptr, token) = shm_alloc().map_err(|_| IndexError::Io("reconcile shm"))?;
        unsafe { core::ptr::copy_nonoverlapping(key_hex.as_ptr(), ptr, key_hex.len()); }
        let msg = IpcMsg::with_label(MemoryDbOp::ReconcileImport as u64)
            .word(0, key.source_id.get())
            .word(1, key.source_revision as u64)
            .word(2, key_hex.len() as u64)
            .with_cap(0, token);
        let reply = self.call(msg);
        let _ = shm_free(token);
        let reply = reply?;
        let state = match reply.words[0] as u8 {
            0 => ImportState::NotFound,
            2 => ImportState::Committed,
            3 => ImportState::Aborted,
            4 => ImportState::Conflict,
            5 => ImportState::AlreadyCommitted,
            _ => ImportState::InProgress,
        };
        Ok(ImportReconcileResult {
            state,
            document_memory_id: (reply.words[1] != 0).then_some(reply.words[1]),
            source_revision: (reply.words[2] != 0).then_some(reply.words[2] as u32),
        })
    }

    fn generation_census(
        &mut self,
        source_id: Option<SourceId>,
        max_sources: u32,
    ) -> Result<(u64, u64, u64, u64, u64, u64), IndexError> {
        let msg = IpcMsg::with_label(MemoryDbOp::GenerationCensus as u64)
            .word(0, source_id.map(|s| s.get()).unwrap_or(0))
            .word(1, max_sources.max(1) as u64);
        let reply = self.call(msg)?;
        Ok((
            reply.words[0],
            reply.words[1],
            reply.words[2],
            reply.words[3],
            reply.words[4],
            reply.words[5],
        ))
    }

    fn verify_generations(&mut self) -> Result<(bool, u64, u64, u64, u64, u64), IndexError> {
        let msg = IpcMsg::with_label(MemoryDbOp::VerifyGenerations as u64);
        let reply = self.call(msg)?;
        Ok((
            reply.words[0] != 0,
            reply.words[1],
            reply.words[2],
            reply.words[3],
            reply.words[4],
            reply.words[5],
        ))
    }
}

// Keep encode available for tests when host builds with sunlightos.
#[allow(dead_code)]
fn _touch_wire() {
    let _ = INLINE_PAYLOAD_THRESHOLD;
    let _ = decode_insert_request;
}

fn write_atomic_native(tmp: &[u8], destination: &[u8], bytes: &[u8]) -> Result<(), IndexError> {
    let fd = sunlight_libc::create(tmp).map_err(|_| IndexError::Io("create state temp"))?;
    let mut remaining = bytes;
    while !remaining.is_empty() {
        let n = sunlight_libc::write(fd, remaining)
            .map_err(|_| IndexError::Io("write state temp"))?;
        if n == 0 {
            let _ = sunlight_libc::close(fd);
            return Err(IndexError::Io("short state write"));
        }
        remaining = &remaining[n..];
    }
    sunlight_libc::close(fd).map_err(|_| IndexError::Io("close state temp"))?;
    sunlight_libc::rename(tmp, destination).map_err(|_| IndexError::Io("publish state"))
}
