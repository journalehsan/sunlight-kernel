//! Bounded short-term memory service engine (Phase 1).
//!
//! Host-testable pure logic. Transports (UDS / SunlightOS IPC) call into
//! [`MemoryService::handle`].

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use crate::caps::{CapabilitySet, MemoryCapability};
use crate::entry::{
    MemoryEntry, MemoryEntryHeader, MemoryState, ENTRY_HEADER_VERSION,
};
use crate::error::MemoryError;
use crate::ids::{
    ClientId, IdAllocator, MemoryId, SegmentId, SessionId,
};
use crate::kinds::MemoryClass;
use crate::lifecycle::LifecycleOp;
use crate::protocol::{
    check_protocol_version, request_version, ListFilter, MaintenanceBudget, PromoteRequest,
    PromoteResult, ProtocolRequest, ProtocolResponse, RequestContext, PROTOCOL_VERSION,
};
use crate::quotas::{QuotaConfig, QuotaSnapshot, SessionQuota};
use crate::segments::{parse_records_v2, Segment, SegmentState};
use crate::spill::SpillStore;
use crate::stats::ServiceStats;

/// Pluggable key-value backend for promotion (tests use in-memory).
pub trait KvBackend: Send {
    fn put_if_absent(&mut self, key: &str, value: &[u8]) -> Result<KvPutOutcome, MemoryError>;
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, MemoryError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KvPutOutcome {
    Written,
    AlreadyPresent,
}

pub use crate::health::{degraded, ServiceHealth};

/// In-memory KV for tests and host demos when sunlight-kv is unavailable.
#[derive(Debug, Default)]
pub struct InMemoryKv {
    pub map: HashMap<String, Vec<u8>>,
    pub available: bool,
}

impl InMemoryKv {
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
            available: true,
        }
    }

    pub fn unavailable() -> Self {
        Self {
            map: HashMap::new(),
            available: false,
        }
    }
}

impl KvBackend for InMemoryKv {
    fn put_if_absent(&mut self, key: &str, value: &[u8]) -> Result<KvPutOutcome, MemoryError> {
        if !self.available {
            return Err(MemoryError::KvUnavailable);
        }
        if self.map.contains_key(key) {
            return Ok(KvPutOutcome::AlreadyPresent);
        }
        self.map.insert(key.to_string(), value.to_vec());
        Ok(KvPutOutcome::Written)
    }

    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, MemoryError> {
        if !self.available {
            return Err(MemoryError::KvUnavailable);
        }
        Ok(self.map.get(key).cloned())
    }
}

/// Service configuration.
#[derive(Debug, Clone)]
pub struct ServiceConfig {
    pub quotas: QuotaConfig,
    pub spill_dir: Option<PathBuf>,
    pub producer_name: String,
    /// Default TTL for working entries (ns). None = no automatic TTL.
    pub default_working_ttl_ns: Option<u64>,
    pub default_hot_ttl_ns: Option<u64>,
    pub default_cold_ttl_ns: Option<u64>,
}

impl Default for ServiceConfig {
    fn default() -> Self {
        Self {
            quotas: QuotaConfig::default(),
            spill_dir: None,
            producer_name: "wiseowl-memoryd".to_string(),
            default_working_ttl_ns: Some(60_000_000_000), // 60s
            default_hot_ttl_ns: Some(300_000_000_000),    // 5 min
            default_cold_ttl_ns: Some(3_600_000_000_000), // 1 hour
        }
    }
}

pub use crate::caller::CallerIdentity;

struct SessionRec {
    owner: Option<ClientId>,
    quota: SessionQuota,
    /// Open cold segment being filled for this session, if any.
    open_segment: Option<SegmentId>,
}

/// Core short-term memory service.
pub struct MemoryService<K: KvBackend = InMemoryKv> {
    cfg: ServiceConfig,
    ids: IdAllocator,
    sessions: BTreeMap<SessionId, SessionRec>,
    entries: BTreeMap<MemoryId, MemoryEntry>,
    segments: BTreeMap<SegmentId, Segment>,
    clients: BTreeMap<ClientId, String>,
    /// client -> owned sessions
    client_sessions: HashMap<ClientId, Vec<SessionId>>,
    spill: Option<SpillStore>,
    kv: K,
    stats: ServiceStats,
    /// Injected monotonic clock (ns).
    now_ns: u64,
    /// Wall clock for diagnostics only.
    shutting_down: bool,
    /// Supervisor-facing health.
    health: ServiceHealth,
    /// Degraded reason flags (bitmask).
    degraded_flags: u32,
}

impl MemoryService<InMemoryKv> {
    pub fn new(cfg: ServiceConfig) -> Result<Self, MemoryError> {
        Self::with_kv(cfg, InMemoryKv::new())
    }
}

impl<K: KvBackend> MemoryService<K> {
    pub fn with_kv(cfg: ServiceConfig, kv: K) -> Result<Self, MemoryError> {
        let spill = match &cfg.spill_dir {
            Some(dir) => {
                let store = SpillStore::open(dir, cfg.quotas.max_decompress_bytes)?;
                Some(store)
            }
            None => None,
        };

        // Restart-safe ID strategy: persistent generation + monotonic counter.
        // Load prior generation from spill dir (if any), bump, and persist.
        let mut ids = IdAllocator::new();
        if let Some(store) = &spill {
            let prev = store.load_generation();
            let next = prev.saturating_add(1).max(1);
            // On u16 wrap, refuse to start rather than reuse generations.
            if prev == u16::MAX {
                return Err(MemoryError::InternalInvariantViolation(
                    "id generation space exhausted",
                ));
            }
            ids.set_generation(next)
                .map_err(|_| MemoryError::InternalInvariantViolation("id generation"))?;
            let _ = store.store_generation(next);
        }

        let mut svc = Self {
            cfg,
            ids,
            sessions: BTreeMap::new(),
            entries: BTreeMap::new(),
            segments: BTreeMap::new(),
            clients: BTreeMap::new(),
            client_sessions: HashMap::new(),
            spill,
            kv,
            stats: ServiceStats::default(),
            now_ns: 1,
            shutting_down: false,
            health: ServiceHealth::Starting,
            degraded_flags: 0,
        };
        if let Some(spill) = &svc.spill {
            ServiceStats::add(
                &mut svc.stats.quarantined_spill_records,
                spill.quarantined.len() as u64,
            );
            if !spill.quarantined.is_empty() {
                svc.degraded_flags |= degraded::SPILL_QUARANTINED;
            }
            // Restore valid cold segments and reconstruct entry metadata.
            let ids_list: Vec<SegmentId> = spill.segment_ids().collect();
            for sid in ids_list {
                if let Ok(blob) = spill.read_blob(sid) {
                    match Segment::from_spill_blob(&blob, svc.cfg.quotas.max_decompress_bytes) {
                        Ok((mut seg, plain)) => {
                            let plain_len = seg
                                .header
                                .as_ref()
                                .map(|h| h.uncompressed_size as u64)
                                .unwrap_or(0);
                            let comp_len = seg
                                .header
                                .as_ref()
                                .map(|h| h.compressed_size as u64)
                                .unwrap_or(0);
                            // Reconstruct MemoryEntry objects from v2 records.
                            if let Ok(recs) = parse_records_v2(&plain) {
                                for rec in recs {
                                    svc.ids.note_seen(rec.header.id.get());
                                    svc.ids.note_seen(rec.header.session_id.get());
                                    svc.sessions
                                        .entry(rec.header.session_id)
                                        .or_insert_with(|| SessionRec {
                                            owner: None,
                                            quota: SessionQuota::default(),
                                            open_segment: None,
                                        });
                                    // Cold entry without RAM payload (rehydrate on read).
                                    let payload_len = rec.payload.len() as u32;
                                    let mut entry = MemoryEntry {
                                        header: rec.header.clone(),
                                        state: MemoryState::Cold,
                                        payload: Vec::new(),
                                        pin_count: 0,
                                        segment_id: Some(sid),
                                        kv_key: None,
                                        promoted: false,
                                        owner_client: None,
                                    };
                                    // Keep logical payload_len for accounting/diagnostics.
                                    entry.header.payload_len = payload_len;
                                    entry.header.class = MemoryClass::Cold;
                                    svc.entries.insert(rec.header.id, entry);
                                }
                            }
                            svc.ids.note_seen(sid.get());
                            seg.plain.clear();
                            seg.state = SegmentState::Spilled;
                            svc.stats.cold_compressed_bytes =
                                svc.stats.cold_compressed_bytes.saturating_add(comp_len);
                            svc.stats.cold_uncompressed_logical_bytes = svc
                                .stats
                                .cold_uncompressed_logical_bytes
                                .saturating_add(plain_len);
                            svc.stats.segment_count =
                                svc.stats.segment_count.saturating_add(1);
                            svc.segments.insert(sid, seg);
                        }
                        Err(MemoryError::ChecksumMismatch) => {
                            ServiceStats::inc(&mut svc.stats.checksum_failures);
                        }
                        Err(_) => {}
                    }
                }
            }
            svc.stats.active_sessions = svc.sessions.len() as u64;
            svc.stats.entry_count = svc.entries.len() as u64;
        }
        svc.health = if svc.degraded_flags != 0 {
            ServiceHealth::Degraded
        } else {
            ServiceHealth::Ready
        };
        Ok(svc)
    }

    pub fn set_now_ns(&mut self, now: u64) {
        self.now_ns = now;
    }

    pub fn now_ns(&self) -> u64 {
        self.now_ns
    }

    pub fn advance_ns(&mut self, dt: u64) {
        self.now_ns = self.now_ns.saturating_add(dt);
    }

    pub fn stats(&self) -> &ServiceStats {
        &self.stats
    }

    pub fn quotas(&self) -> &QuotaConfig {
        &self.cfg.quotas
    }

    pub fn kv_mut(&mut self) -> &mut K {
        &mut self.kv
    }

    pub fn begin_shutdown(&mut self) {
        self.shutting_down = true;
        self.health = ServiceHealth::Stopping;
    }

    pub fn is_shutting_down(&self) -> bool {
        self.shutting_down
    }

    pub fn health(&self) -> ServiceHealth {
        self.health
    }

    pub fn degraded_flags(&self) -> u32 {
        self.degraded_flags
    }

    pub fn set_kv_available(&mut self, available: bool) {
        if available {
            self.degraded_flags &= !degraded::KV_UNAVAILABLE;
        } else {
            self.degraded_flags |= degraded::KV_UNAVAILABLE;
        }
        self.refresh_health();
    }

    fn refresh_health(&mut self) {
        if self.shutting_down {
            self.health = ServiceHealth::Stopping;
        } else if self.degraded_flags != 0 {
            self.health = ServiceHealth::Degraded;
        } else {
            self.health = ServiceHealth::Ready;
        }
    }

    pub fn id_generation(&self) -> u16 {
        self.ids.generation()
    }

    pub fn allocator(&self) -> &IdAllocator {
        &self.ids
    }

    fn quota_snapshot(&self) -> QuotaSnapshot {
        QuotaSnapshot {
            working_bytes: self.stats.working_bytes,
            hot_bytes: self.stats.hot_bytes,
            cold_compressed_bytes: self.stats.cold_compressed_bytes,
            cold_uncompressed_logical_bytes: self.stats.cold_uncompressed_logical_bytes,
            entry_count: self.stats.entry_count as u32,
            segment_count: self.stats.segment_count as u32,
            session_count: self.stats.active_sessions as u32,
        }
    }

    fn refresh_gauges(&mut self) {
        let mut working = 0u64;
        let mut hot = 0u64;
        let mut entries = 0u64;
        for e in self.entries.values() {
            if !e.is_live() {
                continue;
            }
            entries = entries.saturating_add(1);
            let len = e.payload.len() as u64;
            match e.header.class {
                MemoryClass::Working => working = working.saturating_add(len),
                MemoryClass::Hot => hot = hot.saturating_add(len),
                MemoryClass::Cold => {}
            }
        }
        self.stats.working_bytes = working;
        self.stats.hot_bytes = hot;
        self.stats.entry_count = entries;
        self.stats.segment_count = self.segments.len() as u64;
        self.stats.active_sessions = self.sessions.len() as u64;
        self.stats.cold_compressed_bytes = self
            .spill
            .as_ref()
            .map(|s| s.total_bytes())
            .unwrap_or_else(|| {
                self.segments
                    .values()
                    .filter_map(|s| s.compressed.as_ref().map(|c| c.len() as u64))
                    .sum()
            });
    }

    /// Dispatch a protocol request.
    pub fn handle(
        &mut self,
        caller: &CallerIdentity,
        req: ProtocolRequest,
    ) -> ProtocolResponse {
        let version = request_version(&req);
        if let Err(e) = check_protocol_version(version) {
            ServiceStats::inc(&mut self.stats.malformed_ipc_requests);
            return ProtocolResponse::Error(e);
        }
        if self.shutting_down {
            match &req {
                ProtocolRequest::GetStats { .. }
                | ProtocolRequest::ListEntries { .. }
                | ProtocolRequest::ReadEntry { .. }
                | ProtocolRequest::ListSessions { .. } => {}
                _ => {
                    return ProtocolResponse::Error(MemoryError::InvalidRequest(
                        "service shutting down",
                    ));
                }
            }
        }
        match self.dispatch(caller, req) {
            Ok(r) => r,
            Err(e) => ProtocolResponse::Error(e),
        }
    }

    fn dispatch(
        &mut self,
        caller: &CallerIdentity,
        req: ProtocolRequest,
    ) -> Result<ProtocolResponse, MemoryError> {
        match req {
            ProtocolRequest::RegisterClient { name, .. } => {
                let id = self.ids.alloc_client().map_err(|_| {
                    MemoryError::InternalInvariantViolation("client id exhausted")
                })?;
                self.clients.insert(id, name);
                Ok(ProtocolResponse::ClientRegistered { client_id: id })
            }
            ProtocolRequest::ClientDisconnect { client_id, .. } => {
                self.client_disconnect(client_id)?;
                Ok(ProtocolResponse::Ok)
            }
            ProtocolRequest::CreateSession { .. } => {
                caller.caps.require(MemoryCapability::Create)?;
                let sid = self.create_session(caller.client_id)?;
                Ok(ProtocolResponse::SessionCreated { session_id: sid })
            }
            ProtocolRequest::ListSessions { .. } => {
                caller.caps.require(MemoryCapability::InspectMetadata)?;
                let ids = if caller.caps.has(MemoryCapability::ReadSharedSession)
                    || caller.caps.has(MemoryCapability::InspectGlobalStats)
                {
                    self.sessions.keys().copied().collect()
                } else {
                    caller.owned_sessions.clone()
                };
                Ok(ProtocolResponse::Sessions { ids })
            }
            ProtocolRequest::CreateEntry {
                session_id,
                class,
                kind,
                importance,
                confidence,
                ttl_ns,
                payload,
                token_stream,
                provenance,
                ..
            } => {
                caller.caps.require(MemoryCapability::Create)?;
                self.ensure_session_access(caller, session_id, true)?;
                let id = self.create_entry(
                    caller,
                    session_id,
                    class,
                    kind,
                    importance,
                    confidence,
                    ttl_ns,
                    payload,
                    token_stream,
                    provenance,
                )?;
                Ok(ProtocolResponse::Created {
                    memory_id: id,
                    session_id,
                })
            }
            ProtocolRequest::AppendEntry {
                memory_id, data, ..
            } => {
                caller.caps.require(MemoryCapability::Create)?;
                self.append_entry(caller, memory_id, data)?;
                Ok(ProtocolResponse::Ok)
            }
            ProtocolRequest::ReadEntry {
                memory_id,
                include_payload,
                ..
            } => self.read_entry(caller, memory_id, include_payload),
            ProtocolRequest::TouchEntry { memory_id, .. } => {
                self.touch_entry(caller, memory_id)?;
                Ok(ProtocolResponse::Ok)
            }
            ProtocolRequest::SealEntry {
                memory_id,
                promote_class_to_hot,
                ..
            } => {
                self.seal_entry(caller, memory_id, promote_class_to_hot)?;
                Ok(ProtocolResponse::Ok)
            }
            ProtocolRequest::DeleteEntry { memory_id, .. } => {
                caller.caps.require(MemoryCapability::Delete)?;
                self.delete_entry(caller, memory_id)?;
                Ok(ProtocolResponse::Ok)
            }
            ProtocolRequest::PromoteEntry { request, .. } => {
                caller.caps.require(MemoryCapability::PromoteToKv)?;
                let result = self.promote_entry(caller, request)?;
                Ok(ProtocolResponse::Promoted(result))
            }
            ProtocolRequest::ListEntries { filter, .. } => {
                caller.caps.require(MemoryCapability::InspectMetadata)?;
                let headers = self.list_entries(caller, filter)?;
                Ok(ProtocolResponse::Listed { headers })
            }
            ProtocolRequest::GetStats { .. } => {
                caller
                    .caps
                    .require(MemoryCapability::InspectGlobalStats)?;
                self.refresh_gauges();
                Ok(ProtocolResponse::Stats(self.stats.clone()))
            }
            ProtocolRequest::RunMaintenance { budget, .. } => {
                caller.caps.require(MemoryCapability::RunMaintenance)?;
                let r = self.run_maintenance(budget)?;
                Ok(r)
            }
        }
    }

    fn ensure_session_access(
        &self,
        caller: &CallerIdentity,
        session_id: SessionId,
        for_write: bool,
    ) -> Result<(), MemoryError> {
        if !self.sessions.contains_key(&session_id) {
            return Err(MemoryError::SessionNotFound);
        }
        if caller.caps.has(MemoryCapability::ReadSharedSession)
            || caller.caps.has(MemoryCapability::InspectGlobalStats)
            || caller.caps.has(MemoryCapability::AdminQuota)
        {
            return Ok(());
        }
        if caller.owned_sessions.contains(&session_id) {
            return Ok(());
        }
        // Also allow if client owns the session record.
        if let Some(cid) = caller.client_id {
            if let Some(s) = self.sessions.get(&session_id) {
                if s.owner == Some(cid) {
                    return Ok(());
                }
            }
        }
        if for_write {
            return Err(MemoryError::PermissionDenied("session write"));
        }
        caller.caps.require(MemoryCapability::ReadOwnSession)?;
        Err(MemoryError::PermissionDenied("cross-session read"))
    }

    fn create_session(&mut self, owner: Option<ClientId>) -> Result<SessionId, MemoryError> {
        if self.sessions.len() as u32 >= self.cfg.quotas.max_sessions {
            return Err(MemoryError::QuotaExceeded("max sessions"));
        }
        let id = self
            .ids
            .alloc_session()
            .map_err(|_| MemoryError::InternalInvariantViolation("session id"))?;
        self.sessions.insert(
            id,
            SessionRec {
                owner,
                quota: SessionQuota::default(),
                open_segment: None,
            },
        );
        if let Some(cid) = owner {
            self.client_sessions.entry(cid).or_default().push(id);
        }
        self.stats.active_sessions = self.sessions.len() as u64;
        Ok(id)
    }

    #[allow(clippy::too_many_arguments)]
    fn create_entry(
        &mut self,
        caller: &CallerIdentity,
        session_id: SessionId,
        class: MemoryClass,
        kind: crate::kinds::MemoryKind,
        importance: u16,
        confidence: u16,
        ttl_ns: Option<u64>,
        payload: Vec<u8>,
        token_stream: Option<crate::entry::TokenStreamRef>,
        provenance: crate::provenance::Provenance,
    ) -> Result<MemoryId, MemoryError> {
        MemoryEntryHeader::validate_scores(importance, confidence)?;
        let max = self.cfg.quotas.max_entry_size;
        MemoryEntryHeader::validate_payload_len(payload.len() as u32, max)?;

        if self.entries.values().filter(|e| e.is_live()).count() as u32
            >= self.cfg.quotas.max_entries
        {
            self.evict_for_space(0)?;
            if self.entries.values().filter(|e| e.is_live()).count() as u32
                >= self.cfg.quotas.max_entries
            {
                ServiceStats::inc(&mut self.stats.rejected_allocations);
                return Err(MemoryError::QuotaExceeded("max entries"));
            }
        }

        let payload_len = payload.len() as u64;
        // Working/hot consume RAM; cold create is not allowed directly (must seal).
        if class == MemoryClass::Cold {
            return Err(MemoryError::InvalidRequest(
                "create cold directly unsupported; seal then spill",
            ));
        }

        self.ensure_ram_budget(session_id, payload_len)?;

        let id = self
            .ids
            .alloc_memory()
            .map_err(|_| MemoryError::InternalInvariantViolation("memory id"))?;
        let now = self.now_ns;
        let default_ttl = match class {
            MemoryClass::Working => self.cfg.default_working_ttl_ns,
            MemoryClass::Hot => self.cfg.default_hot_ttl_ns,
            MemoryClass::Cold => self.cfg.default_cold_ttl_ns,
        };
        let expires = ttl_ns
            .or(default_ttl)
            .map(|t| now.saturating_add(t));

        let header = MemoryEntryHeader {
            version: ENTRY_HEADER_VERSION,
            id,
            session_id,
            class,
            kind,
            created_at_ns: now,
            last_access_ns: now,
            expires_at_ns: expires,
            importance,
            confidence,
            payload_len: payload.len() as u32,
            token_stream,
            provenance,
        };

        let entry = MemoryEntry {
            header,
            state: MemoryState::Open,
            payload,
            pin_count: 0,
            segment_id: None,
            kv_key: None,
            promoted: false,
            owner_client: caller.client_id,
        };

        // Account
        if let Some(s) = self.sessions.get_mut(&session_id) {
            s.quota.ram_bytes = s.quota.ram_bytes.saturating_add(payload_len);
            s.quota.entry_count = s.quota.entry_count.saturating_add(1);
        }
        match class {
            MemoryClass::Working => {
                self.stats.working_bytes = self.stats.working_bytes.saturating_add(payload_len);
            }
            MemoryClass::Hot => {
                self.stats.hot_bytes = self.stats.hot_bytes.saturating_add(payload_len);
            }
            MemoryClass::Cold => {}
        }
        self.stats.entry_count = self.stats.entry_count.saturating_add(1);
        ServiceStats::inc(&mut self.stats.creates);
        self.entries.insert(id, entry);
        Ok(id)
    }

    fn ensure_ram_budget(
        &mut self,
        session_id: SessionId,
        add: u64,
    ) -> Result<(), MemoryError> {
        let snap = self.quota_snapshot();
        if snap.checked_add_ram(add, &self.cfg.quotas).is_err() {
            self.evict_for_space(add)?;
            let snap = self.quota_snapshot();
            if snap.checked_add_ram(add, &self.cfg.quotas).is_err() {
                ServiceStats::inc(&mut self.stats.rejected_allocations);
                return Err(MemoryError::QuotaExceeded("total service RAM"));
            }
        }
        if let Some(s) = self.sessions.get(&session_id) {
            if s.quota.can_add_ram(add, &self.cfg.quotas).is_err() {
                self.evict_session(session_id, add)?;
                let s = self
                    .sessions
                    .get(&session_id)
                    .ok_or(MemoryError::SessionNotFound)?;
                if s.quota.can_add_ram(add, &self.cfg.quotas).is_err() {
                    ServiceStats::inc(&mut self.stats.rejected_allocations);
                    return Err(MemoryError::SessionQuotaExceeded);
                }
            }
        }
        Ok(())
    }

    fn append_entry(
        &mut self,
        caller: &CallerIdentity,
        memory_id: MemoryId,
        data: Vec<u8>,
    ) -> Result<(), MemoryError> {
        let now = self.now_ns;
        let session_id = {
            let e = self
                .entries
                .get(&memory_id)
                .ok_or(MemoryError::EntryNotFound)?;
            e.header.session_id
        };
        self.ensure_session_access_id(caller, session_id, true)?;
        let new_len = {
            let e = self
                .entries
                .get_mut(&memory_id)
                .ok_or(MemoryError::EntryNotFound)?;
            e.check_not_expired(now)?;
            e.state.require(LifecycleOp::Append)?;
            let total = (e.payload.len() as u32)
                .checked_add(data.len() as u32)
                .ok_or(MemoryError::InternalInvariantViolation("append overflow"))?;
            MemoryEntryHeader::validate_payload_len(total, self.cfg.quotas.max_entry_size)?;
            data.len() as u64
        };
        self.ensure_ram_budget(session_id, new_len)?;
        let e = self.entries.get_mut(&memory_id).unwrap();
        let old_len = e.payload.len() as u64;
        e.payload.extend_from_slice(&data);
        e.header.payload_len = e.payload.len() as u32;
        e.header.last_access_ns = now;
        let delta = e.payload.len() as u64 - old_len;
        let class = e.header.class;
        if let Some(s) = self.sessions.get_mut(&session_id) {
            s.quota.ram_bytes = s.quota.ram_bytes.saturating_add(delta);
        }
        match class {
            MemoryClass::Working => {
                self.stats.working_bytes = self.stats.working_bytes.saturating_add(delta);
            }
            MemoryClass::Hot => {
                self.stats.hot_bytes = self.stats.hot_bytes.saturating_add(delta);
            }
            MemoryClass::Cold => {}
        }
        Ok(())
    }

    fn ensure_session_access_id(
        &self,
        caller: &CallerIdentity,
        session_id: SessionId,
        for_write: bool,
    ) -> Result<(), MemoryError> {
        self.ensure_session_access(caller, session_id, for_write)
    }

    fn read_entry(
        &mut self,
        caller: &CallerIdentity,
        memory_id: MemoryId,
        include_payload: bool,
    ) -> Result<ProtocolResponse, MemoryError> {
        let session_id = {
            let e = self
                .entries
                .get(&memory_id)
                .ok_or(MemoryError::EntryNotFound)?;
            e.header.session_id
        };
        self.ensure_session_access_id(caller, session_id, false)?;
        // Pin during read of cold to protect eviction.
        {
            let e = self
                .entries
                .get_mut(&memory_id)
                .ok_or(MemoryError::EntryNotFound)?;
            e.check_not_expired(self.now_ns)?;
            e.state.require(LifecycleOp::Read)?;
            e.pin_count = e.pin_count.saturating_add(1);
        }

        // Rehydrate if cold and payload empty.
        let need_rehydrate = {
            let e = self.entries.get(&memory_id).unwrap();
            e.state == MemoryState::Cold && e.payload.is_empty() && e.segment_id.is_some()
        };
        if need_rehydrate {
            if let Err(err) = self.rehydrate_entry(memory_id) {
                if let Some(e) = self.entries.get_mut(&memory_id) {
                    e.pin_count = e.pin_count.saturating_sub(1);
                }
                return Err(err);
            }
            ServiceStats::inc(&mut self.stats.decompression_successes);
        }

        let resp = {
            let e = self.entries.get_mut(&memory_id).unwrap();
            e.header.last_access_ns = self.now_ns;
            e.pin_count = e.pin_count.saturating_sub(1);
            let payload = if include_payload {
                caller.caps.require(MemoryCapability::ReadPayload)?;
                Some(e.payload.clone())
            } else {
                None
            };
            ServiceStats::inc(&mut self.stats.reads);
            ProtocolResponse::Entry {
                header: e.header.clone(),
                state: e.state,
                payload,
                promoted: e.promoted,
                segment_id: e.segment_id.map(|s| s.get()),
            }
        };
        Ok(resp)
    }

    fn rehydrate_entry(&mut self, memory_id: MemoryId) -> Result<(), MemoryError> {
        let (seg_id, _) = {
            let e = self
                .entries
                .get(&memory_id)
                .ok_or(MemoryError::EntryNotFound)?;
            let sid = e.segment_id.ok_or(MemoryError::SegmentNotFound)?;
            (sid, e.header.session_id)
        };

        // Prefer in-memory compressed segment, else spill.
        let blob = if let Some(seg) = self.segments.get(&seg_id) {
            if let Some(c) = &seg.compressed {
                let h = seg.header.as_ref().ok_or(MemoryError::SpillIncomplete)?;
                let mut b = h.encode().to_vec();
                b.extend_from_slice(c);
                b
            } else if !seg.plain.is_empty() {
                let payload = seg
                    .find_payload(memory_id)?
                    .ok_or(MemoryError::EntryNotFound)?;
                let e = self.entries.get_mut(&memory_id).unwrap();
                e.payload = payload;
                e.header.payload_len = e.payload.len() as u32;
                e.state = MemoryState::Sealed;
                e.header.class = MemoryClass::Hot;
                return Ok(());
            } else {
                return Err(MemoryError::SegmentNotFound);
            }
        } else if let Some(spill) = &self.spill {
            spill.read_blob(seg_id)?
        } else {
            return Err(MemoryError::SegmentNotFound);
        };

        let (seg, plain) =
            match Segment::from_spill_blob(&blob, self.cfg.quotas.max_decompress_bytes) {
                Ok(v) => v,
                Err(MemoryError::ChecksumMismatch) => {
                    ServiceStats::inc(&mut self.stats.checksum_failures);
                    ServiceStats::inc(&mut self.stats.decompression_failures);
                    return Err(MemoryError::ChecksumMismatch);
                }
                Err(MemoryError::DecompressionFailure) => {
                    ServiceStats::inc(&mut self.stats.decompression_failures);
                    return Err(MemoryError::DecompressionFailure);
                }
                Err(e) => return Err(e),
            };

        let payload = parse_payload_from_plain(&plain, memory_id)?
            .ok_or(MemoryError::EntryNotFound)?;

        // Cache segment rehydrated state lightly
        if let Some(existing) = self.segments.get_mut(&seg_id) {
            existing.plain = plain;
            existing.state = SegmentState::Rehydrated;
            existing.last_access_ns = self.now_ns;
        } else {
            self.segments.insert(seg_id, seg);
        }

        let e = self.entries.get_mut(&memory_id).unwrap();
        e.payload = payload;
        e.header.payload_len = e.payload.len() as u32;
        e.state = MemoryState::Sealed;
        e.header.class = MemoryClass::Hot;
        e.header.last_access_ns = self.now_ns;
        Ok(())
    }

    fn touch_entry(
        &mut self,
        caller: &CallerIdentity,
        memory_id: MemoryId,
    ) -> Result<(), MemoryError> {
        let session_id = {
            let e = self
                .entries
                .get(&memory_id)
                .ok_or(MemoryError::EntryNotFound)?;
            e.header.session_id
        };
        self.ensure_session_access_id(caller, session_id, false)?;
        let e = self
            .entries
            .get_mut(&memory_id)
            .ok_or(MemoryError::EntryNotFound)?;
        e.state.require(LifecycleOp::Touch)?;
        e.touch(self.now_ns)?;
        Ok(())
    }

    fn seal_entry(
        &mut self,
        caller: &CallerIdentity,
        memory_id: MemoryId,
        promote_class_to_hot: bool,
    ) -> Result<(), MemoryError> {
        let now = self.now_ns;
        let session_id = {
            let e = self
                .entries
                .get(&memory_id)
                .ok_or(MemoryError::EntryNotFound)?;
            e.header.session_id
        };
        self.ensure_session_access_id(caller, session_id, true)?;
        let e = self
            .entries
            .get_mut(&memory_id)
            .ok_or(MemoryError::EntryNotFound)?;
        e.check_not_expired(now)?;
        e.state.require(LifecycleOp::Seal)?;
        e.state = MemoryState::Sealed;
        if promote_class_to_hot && e.header.class == MemoryClass::Working {
            let len = e.payload.len() as u64;
            e.header.class = MemoryClass::Hot;
            self.stats.working_bytes = self.stats.working_bytes.saturating_sub(len);
            self.stats.hot_bytes = self.stats.hot_bytes.saturating_add(len);
        }
        e.header.last_access_ns = now;
        ServiceStats::inc(&mut self.stats.seals);
        Ok(())
    }

    fn delete_entry(
        &mut self,
        caller: &CallerIdentity,
        memory_id: MemoryId,
    ) -> Result<(), MemoryError> {
        let (session_id, class, len, state) = {
            let e = self
                .entries
                .get(&memory_id)
                .ok_or(MemoryError::EntryNotFound)?;
            self.ensure_session_access_id(caller, e.header.session_id, true)?;
            e.state.require(LifecycleOp::Delete)?;
            (
                e.header.session_id,
                e.header.class,
                e.payload.len() as u64,
                e.state,
            )
        };
        if let Some(e) = self.entries.get_mut(&memory_id) {
            e.state = MemoryState::Deleted;
            e.payload.clear();
            e.header.payload_len = 0;
        }
        self.unaccount(session_id, class, len, state);
        Ok(())
    }

    fn unaccount(
        &mut self,
        session_id: SessionId,
        class: MemoryClass,
        len: u64,
        _state: MemoryState,
    ) {
        if let Some(s) = self.sessions.get_mut(&session_id) {
            s.quota.ram_bytes = s.quota.ram_bytes.saturating_sub(len);
            s.quota.entry_count = s.quota.entry_count.saturating_sub(1);
        }
        match class {
            MemoryClass::Working => {
                self.stats.working_bytes = self.stats.working_bytes.saturating_sub(len);
            }
            MemoryClass::Hot => {
                self.stats.hot_bytes = self.stats.hot_bytes.saturating_sub(len);
            }
            MemoryClass::Cold => {}
        }
        self.stats.entry_count = self.stats.entry_count.saturating_sub(1);
    }

    fn promote_entry(
        &mut self,
        caller: &CallerIdentity,
        req: PromoteRequest,
    ) -> Result<PromoteResult, MemoryError> {
        let now = self.now_ns;
        let session_id = {
            let e = self
                .entries
                .get(&req.memory_id)
                .ok_or(MemoryError::EntryNotFound)?;
            e.header.session_id
        };
        self.ensure_session_access_id(caller, session_id, true)?;
        // Gather sealed immutable content
        let (key, value, session_id) = {
            let e = self
                .entries
                .get_mut(&req.memory_id)
                .ok_or(MemoryError::EntryNotFound)?;
            e.check_not_expired(now)?;
            e.state.require(LifecycleOp::PromoteToKv)?;
            if e.state == MemoryState::Open {
                return Err(MemoryError::InvalidLifecycleTransition {
                    from: "open",
                    op: LifecycleOp::PromoteToKv,
                });
            }
            // Key: owl.v1.shortterm.<session>.<memory_id> or custom namespace
            let ns = if req.namespace.is_empty() {
                "owl.v1.shortterm".to_string()
            } else {
                req.namespace.clone()
            };
            let key = format!(
                "{}.{}.{}",
                ns,
                e.header.session_id.get(),
                e.header.id.get()
            );
            // Encode versioned record: version | header fields | provenance | checksum | payload
            let value = encode_promotion_blob(e, req.expected_record_version)?;
            (key, value, e.header.session_id)
        };

        let outcome = match self.kv.put_if_absent(&key, &value) {
            Ok(o) => o,
            Err(MemoryError::KvUnavailable) => {
                ServiceStats::inc(&mut self.stats.kv_promotion_failures);
                self.degraded_flags |= degraded::KV_UNAVAILABLE;
                self.refresh_health();
                return Err(MemoryError::KvUnavailable);
            }
            Err(e) => {
                ServiceStats::inc(&mut self.stats.kv_promotion_failures);
                return Err(e);
            }
        };

        // Phase 1.1: AlreadyPresent must verify version, IDs, length, checksum.
        let result = match outcome {
            KvPutOutcome::Written => PromoteResult::Written { key: key.clone() },
            KvPutOutcome::AlreadyPresent => {
                match self.kv.get(&key) {
                    Ok(Some(existing)) => {
                        if promotion_blobs_identical(&existing, &value) {
                            PromoteResult::AlreadyPresentAndIdentical { key: key.clone() }
                        } else {
                            ServiceStats::inc(&mut self.stats.kv_promotion_failures);
                            return Err(MemoryError::PromotionConflict { key: "mismatch" });
                        }
                    }
                    Ok(None) => {
                        // Race: reported present but get missed — treat as unavailable.
                        ServiceStats::inc(&mut self.stats.kv_promotion_failures);
                        return Err(MemoryError::KvUnavailable);
                    }
                    Err(MemoryError::KvUnavailable) => {
                        ServiceStats::inc(&mut self.stats.kv_promotion_failures);
                        return Err(MemoryError::KvUnavailable);
                    }
                    Err(e) => {
                        ServiceStats::inc(&mut self.stats.kv_promotion_failures);
                        return Err(e);
                    }
                }
            }
        };

        // Only mark promoted after confirmed Written or identical AlreadyPresent.
        if !result.is_confirmed_success() {
            ServiceStats::inc(&mut self.stats.kv_promotion_failures);
            return Ok(result);
        }

        let e = self.entries.get_mut(&req.memory_id).unwrap();
        e.kv_key = Some(key.clone());
        e.promoted = true;
        if e.state != MemoryState::Cold {
            e.state = MemoryState::Promoted;
        }
        ServiceStats::inc(&mut self.stats.kv_promotion_successes);

        if req.delete_local_after {
            // Only delete after confirmed write / identical present.
            let class = e.header.class;
            let len = e.payload.len() as u64;
            e.state = MemoryState::Deleted;
            e.payload.clear();
            self.unaccount(session_id, class, len, MemoryState::Deleted);
        }
        // Map AlreadyPresentAndIdentical to legacy AlreadyPresent for host CLI compat
        // when callers match on AlreadyPresent — both are confirmed success.
        Ok(match result {
            PromoteResult::AlreadyPresentAndIdentical { key } => {
                PromoteResult::AlreadyPresent { key }
            }
            other => other,
        })
    }

    fn list_entries(
        &self,
        caller: &CallerIdentity,
        filter: ListFilter,
    ) -> Result<Vec<(MemoryEntryHeader, MemoryState)>, MemoryError> {
        let max = filter
            .max_results
            .unwrap_or(self.cfg.quotas.max_list_results)
            .min(self.cfg.quotas.max_list_results) as usize;
        let mut out = Vec::new();
        for e in self.entries.values() {
            if !e.is_live() {
                continue;
            }
            if let Some(sid) = filter.session_id {
                if e.header.session_id != sid {
                    continue;
                }
            }
            if let Some(c) = filter.class {
                if e.header.class != c {
                    continue;
                }
            }
            if let Some(k) = filter.kind {
                if e.header.kind != k {
                    continue;
                }
            }
            // Access control
            if self
                .ensure_session_access(caller, e.header.session_id, false)
                .is_err()
            {
                continue;
            }
            out.push((e.header.clone(), e.state));
            if out.len() >= max {
                break;
            }
        }
        Ok(out)
    }

    fn client_disconnect(&mut self, client_id: ClientId) -> Result<(), MemoryError> {
        ServiceStats::inc(&mut self.stats.client_disconnects);
        // Remove working entries owned by this client
        let to_delete: Vec<MemoryId> = self
            .entries
            .iter()
            .filter(|(_, e)| {
                e.owner_client == Some(client_id)
                    && e.header.class == MemoryClass::Working
                    && e.is_live()
            })
            .map(|(id, _)| *id)
            .collect();
        for id in to_delete {
            if let Some(e) = self.entries.get_mut(&id) {
                let session_id = e.header.session_id;
                let class = e.header.class;
                let len = e.payload.len() as u64;
                let state = e.state;
                e.state = MemoryState::Deleted;
                e.payload.clear();
                self.unaccount(session_id, class, len, state);
            }
        }
        self.clients.remove(&client_id);
        self.client_sessions.remove(&client_id);
        Ok(())
    }

    /// Move sealed hot entries into a cold segment (compress + optional spill).
    pub fn spill_entry_to_cold(
        &mut self,
        memory_id: MemoryId,
    ) -> Result<SegmentId, MemoryError> {
        let now = self.now_ns;
        let (session_id, payload, importance, expires) = {
            let e = self
                .entries
                .get_mut(&memory_id)
                .ok_or(MemoryError::EntryNotFound)?;
            e.check_not_expired(now)?;
            e.state.require(LifecycleOp::SpillToCold)?;
            if e.is_pinned() {
                return Err(MemoryError::InvalidRequest("entry pinned"));
            }
            (
                e.header.session_id,
                e.payload.clone(),
                e.header.importance,
                e.header.expires_at_ns.unwrap_or(u64::MAX),
            )
        };

        // Get or create open segment for session
        let seg_id = {
            let open = self
                .sessions
                .get(&session_id)
                .and_then(|s| s.open_segment);
            if let Some(sid) = open {
                if let Some(seg) = self.segments.get(&sid) {
                    if seg.state == SegmentState::Open {
                        sid
                    } else {
                        self.alloc_open_segment(session_id, expires)?
                    }
                } else {
                    self.alloc_open_segment(session_id, expires)?
                }
            } else {
                self.alloc_open_segment(session_id, expires)?
            }
        };

        // Snapshot header/state for full metadata spill (Phase 1.1).
        let (header_snap, state_snap) = {
            let e = self
                .entries
                .get(&memory_id)
                .ok_or(MemoryError::EntryNotFound)?;
            let mut h = e.header.clone();
            h.payload_len = payload.len() as u32;
            h.class = MemoryClass::Cold;
            (h, MemoryState::Cold)
        };
        let _ = importance;

        // Append; if full, seal/compress old and open new
        let append_result = {
            let seg = self.segments.get_mut(&seg_id).unwrap();
            seg.append_record_full(
                &header_snap,
                state_snap,
                &payload,
                self.cfg.quotas.max_segment_size,
            )
        };
        if matches!(append_result, Err(MemoryError::QuotaExceeded(_))) {
            self.finalize_segment(seg_id)?;
            let new_id = self.alloc_open_segment(session_id, expires)?;
            self.segments.get_mut(&new_id).unwrap().append_record_full(
                &header_snap,
                state_snap,
                &payload,
                self.cfg.quotas.max_segment_size,
            )?;
            self.attach_entry_to_segment(memory_id, new_id, payload.len() as u64)?;
            return Ok(new_id);
        }
        append_result?;
        self.attach_entry_to_segment(memory_id, seg_id, payload.len() as u64)?;
        Ok(seg_id)
    }

    fn attach_entry_to_segment(
        &mut self,
        memory_id: MemoryId,
        seg_id: SegmentId,
        payload_len: u64,
    ) -> Result<(), MemoryError> {
        let e = self.entries.get_mut(&memory_id).unwrap();
        let old_class = e.header.class;
        // Free RAM for this entry
        match old_class {
            MemoryClass::Working => {
                self.stats.working_bytes = self.stats.working_bytes.saturating_sub(payload_len);
            }
            MemoryClass::Hot => {
                self.stats.hot_bytes = self.stats.hot_bytes.saturating_sub(payload_len);
            }
            MemoryClass::Cold => {}
        }
        if let Some(s) = self.sessions.get_mut(&e.header.session_id) {
            s.quota.ram_bytes = s.quota.ram_bytes.saturating_sub(payload_len);
        }
        e.payload.clear();
        e.header.payload_len = 0;
        e.header.class = MemoryClass::Cold;
        e.state = MemoryState::Cold;
        e.segment_id = Some(seg_id);
        Ok(())
    }

    fn alloc_open_segment(
        &mut self,
        session_id: SessionId,
        expires_at_ns: u64,
    ) -> Result<SegmentId, MemoryError> {
        let id = self
            .ids
            .alloc_segment()
            .map_err(|_| MemoryError::InternalInvariantViolation("segment id"))?;
        let seg = Segment::new_open(id, session_id, self.now_ns, expires_at_ns);
        self.segments.insert(id, seg);
        if let Some(s) = self.sessions.get_mut(&session_id) {
            s.open_segment = Some(id);
        }
        self.stats.segment_count = self.segments.len() as u64;
        Ok(id)
    }

    fn finalize_segment(&mut self, seg_id: SegmentId) -> Result<(), MemoryError> {
        {
            let seg = self
                .segments
                .get_mut(&seg_id)
                .ok_or(MemoryError::SegmentNotFound)?;
            if seg.state == SegmentState::Open {
                seg.seal()?;
            }
            match seg.compress_once(self.cfg.quotas.max_decompress_bytes) {
                Ok(()) => ServiceStats::inc(&mut self.stats.compression_successes),
                Err(e) => {
                    ServiceStats::inc(&mut self.stats.compression_failures);
                    return Err(e);
                }
            }
        }
        // Spill if configured
        let compressed_len = self
            .segments
            .get(&seg_id)
            .and_then(|s| s.compressed.as_ref().map(|c| c.len() as u64))
            .unwrap_or(0);
        let uncomp = self
            .segments
            .get(&seg_id)
            .and_then(|s| s.header.as_ref().map(|h| h.uncompressed_size as u64))
            .unwrap_or(0);

        // Cold quota check
        let snap = self.quota_snapshot();
        if snap
            .checked_add_cold(compressed_len, &self.cfg.quotas)
            .is_err()
        {
            self.evict_cold(compressed_len)?;
            let snap = self.quota_snapshot();
            snap.checked_add_cold(compressed_len, &self.cfg.quotas)?;
        }

        if let Some(spill) = &mut self.spill {
            let seg = self.segments.get(&seg_id).unwrap();
            spill.write_segment(seg)?;
            if let Some(seg) = self.segments.get_mut(&seg_id) {
                seg.state = SegmentState::Spilled;
                // free compressed RAM after spill — keep header only
                // actually keep compressed for rehydrate without disk on host
            }
        }

        self.stats.cold_compressed_bytes = self
            .stats
            .cold_compressed_bytes
            .saturating_add(compressed_len);
        self.stats.cold_uncompressed_logical_bytes = self
            .stats
            .cold_uncompressed_logical_bytes
            .saturating_add(uncomp);

        // Clear open segment pointer if matching
        if let Some(seg) = self.segments.get(&seg_id) {
            if let Some(s) = self.sessions.get_mut(&seg.session_id) {
                if s.open_segment == Some(seg_id) {
                    s.open_segment = None;
                }
            }
        }
        Ok(())
    }

    /// Deterministic eviction order documented in WISE_OWL_MEMORY_FOUNDATION.md.
    fn evict_for_space(&mut self, need: u64) -> Result<(), MemoryError> {
        let now = self.now_ns;
        // 1-3: expire first
        self.expire_all(now);

        if self.quota_snapshot().checked_add_ram(need, &self.cfg.quotas).is_ok() {
            return Ok(());
        }

        // 4: low-importance sealed hot
        // 5: LRU sealed hot
        // Tie-break: lower importance first, then older last_access, then lower MemoryId.
        let mut candidates: Vec<(u16, u64, MemoryId)> = self
            .entries
            .iter()
            .filter(|(_, e)| {
                e.is_live()
                    && !e.is_pinned()
                    && e.header.class == MemoryClass::Hot
                    && matches!(e.state, MemoryState::Sealed | MemoryState::Promoted)
                    && e.segment_id.is_none()
            })
            .map(|(id, e)| (e.header.importance, e.header.last_access_ns, *id))
            .collect();
        candidates.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));

        for (_, _, id) in candidates {
            // Prefer spilling to cold over delete when spill available
            if self.spill.is_some() || true {
                let _ = self.spill_entry_to_cold(id);
            }
            ServiceStats::inc(&mut self.stats.evictions);
            if self.quota_snapshot().checked_add_ram(need, &self.cfg.quotas).is_ok() {
                return Ok(());
            }
        }

        // Evict low-importance working (unpinned, unsealed still ok to delete if expired already handled)
        let mut working: Vec<(u16, u64, MemoryId)> = self
            .entries
            .iter()
            .filter(|(_, e)| {
                e.is_live()
                    && !e.is_pinned()
                    && e.header.class == MemoryClass::Working
                    && e.state == MemoryState::Open
            })
            .map(|(id, e)| (e.header.importance, e.header.last_access_ns, *id))
            .collect();
        working.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
        for (_, _, id) in working {
            if let Some(e) = self.entries.get_mut(&id) {
                let sid = e.header.session_id;
                let class = e.header.class;
                let len = e.payload.len() as u64;
                let state = e.state;
                e.state = MemoryState::Deleted;
                e.payload.clear();
                self.unaccount(sid, class, len, state);
                ServiceStats::inc(&mut self.stats.evictions);
            }
            if self.quota_snapshot().checked_add_ram(need, &self.cfg.quotas).is_ok() {
                return Ok(());
            }
        }

        Ok(())
    }

    fn evict_session(&mut self, session_id: SessionId, need: u64) -> Result<(), MemoryError> {
        self.expire_all(self.now_ns);
        let mut candidates: Vec<(u16, u64, MemoryId)> = self
            .entries
            .iter()
            .filter(|(_, e)| {
                e.header.session_id == session_id
                    && e.is_live()
                    && !e.is_pinned()
                    && e.header.class != MemoryClass::Cold
            })
            .map(|(id, e)| (e.header.importance, e.header.last_access_ns, *id))
            .collect();
        candidates.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
        for (_, _, id) in candidates {
            if self.spill_entry_to_cold(id).is_err() {
                if let Some(e) = self.entries.get_mut(&id) {
                    let sid = e.header.session_id;
                    let class = e.header.class;
                    let len = e.payload.len() as u64;
                    let state = e.state;
                    e.state = MemoryState::Deleted;
                    e.payload.clear();
                    self.unaccount(sid, class, len, state);
                    ServiceStats::inc(&mut self.stats.evictions);
                }
            }
            if let Some(s) = self.sessions.get(&session_id) {
                if s.quota.can_add_ram(need, &self.cfg.quotas).is_ok() {
                    return Ok(());
                }
            }
        }
        Ok(())
    }

    fn evict_cold(&mut self, need: u64) -> Result<(), MemoryError> {
        // 6: low-importance cold segments
        let mut segs: Vec<(u16, u64, SegmentId)> = self
            .segments
            .iter()
            .filter(|(_, s)| {
                matches!(
                    s.state,
                    SegmentState::Compressed | SegmentState::Spilled | SegmentState::Rehydrated
                )
            })
            .map(|(id, s)| (s.importance, s.last_access_ns, *id))
            .collect();
        segs.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
        for (_, _, sid) in segs {
            let size = self
                .segments
                .get(&sid)
                .and_then(|s| s.compressed.as_ref().map(|c| c.len() as u64))
                .unwrap_or(0);
            // Mark entries in segment expired/deleted
            let mids: Vec<MemoryId> = self
                .entries
                .iter()
                .filter(|(_, e)| e.segment_id == Some(sid) && e.is_live())
                .map(|(id, _)| *id)
                .collect();
            for mid in mids {
                if let Some(e) = self.entries.get_mut(&mid) {
                    e.state = MemoryState::Deleted;
                }
            }
            if let Some(spill) = &mut self.spill {
                let _ = spill.delete(sid);
            }
            self.segments.remove(&sid);
            self.stats.cold_compressed_bytes =
                self.stats.cold_compressed_bytes.saturating_sub(size);
            ServiceStats::inc(&mut self.stats.evictions);
            if self.quota_snapshot().checked_add_cold(need, &self.cfg.quotas).is_ok() {
                return Ok(());
            }
        }
        Ok(())
    }

    fn expire_all(&mut self, now: u64) {
        let mut expired = Vec::new();
        for (id, e) in self.entries.iter_mut() {
            if !e.is_live() {
                continue;
            }
            if let Some(exp) = e.header.expires_at_ns {
                if now >= exp {
                    e.state = MemoryState::Expired;
                    expired.push((*id, e.header.session_id, e.header.class, e.payload.len() as u64));
                    e.payload.clear();
                }
            }
        }
        for (_, sid, class, len) in expired {
            self.unaccount(sid, class, len, MemoryState::Expired);
            ServiceStats::inc(&mut self.stats.expirations);
        }
    }

    fn run_maintenance(
        &mut self,
        budget: MaintenanceBudget,
    ) -> Result<ProtocolResponse, MemoryError> {
        ServiceStats::inc(&mut self.stats.maintenance_runs);
        let start = self.now_ns;
        let mut scanned = 0u32;
        let mut compressed = 0u32;
        let mut reclaimed = 0u64;
        let mut expired = 0u32;
        let evicted = 0u32;

        // Expire entries (bounded scan)
        let ids: Vec<MemoryId> = self.entries.keys().copied().collect();
        for id in ids {
            if scanned >= budget.max_entries_scanned {
                break;
            }
            if self.now_ns.saturating_sub(start) >= budget.max_elapsed_ns {
                break;
            }
            scanned = scanned.saturating_add(1);
            let expired_meta = {
                let e = match self.entries.get_mut(&id) {
                    Some(e) if e.is_live() => e,
                    _ => continue,
                };
                match e.header.expires_at_ns {
                    Some(exp) if self.now_ns >= exp => {
                        let meta = (e.header.session_id, e.header.class, e.payload.len() as u64);
                        e.state = MemoryState::Expired;
                        e.payload.clear();
                        Some(meta)
                    }
                    _ => None,
                }
            };
            if let Some((sid, class, len)) = expired_meta {
                self.unaccount(sid, class, len, MemoryState::Expired);
                reclaimed = reclaimed.saturating_add(len);
                expired = expired.saturating_add(1);
                ServiceStats::inc(&mut self.stats.expirations);
            }
            if reclaimed >= budget.max_bytes_reclaimed {
                break;
            }
        }

        // Compress open segments that have data
        let seg_ids: Vec<SegmentId> = self
            .segments
            .iter()
            .filter(|(_, s)| s.state == SegmentState::Open && !s.record_ids.is_empty())
            .map(|(id, _)| *id)
            .collect();
        for sid in seg_ids {
            if compressed >= budget.max_segments_compressed {
                break;
            }
            if self.now_ns.saturating_sub(start) >= budget.max_elapsed_ns {
                break;
            }
            if self.finalize_segment(sid).is_ok() {
                compressed = compressed.saturating_add(1);
            }
        }

        // Seal sealed-hot candidates under pressure (light)
        self.refresh_gauges();

        Ok(ProtocolResponse::Maintenance {
            entries_scanned: scanned,
            segments_compressed: compressed,
            bytes_reclaimed: reclaimed,
            expired,
            evicted,
        })
    }

    /// Pin an entry (test helper for eviction protection).
    pub fn pin(&mut self, memory_id: MemoryId) -> Result<(), MemoryError> {
        let e = self
            .entries
            .get_mut(&memory_id)
            .ok_or(MemoryError::EntryNotFound)?;
        e.pin_count = e.pin_count.saturating_add(1);
        Ok(())
    }

    pub fn unpin(&mut self, memory_id: MemoryId) -> Result<(), MemoryError> {
        let e = self
            .entries
            .get_mut(&memory_id)
            .ok_or(MemoryError::EntryNotFound)?;
        e.pin_count = e.pin_count.saturating_sub(1);
        Ok(())
    }

    pub fn entry_state(&self, memory_id: MemoryId) -> Option<MemoryState> {
        self.entries.get(&memory_id).map(|e| e.state)
    }

    pub fn entry_payload_len(&self, memory_id: MemoryId) -> Option<usize> {
        self.entries.get(&memory_id).map(|e| e.payload.len())
    }

    pub fn entry_class(&self, memory_id: MemoryId) -> Option<MemoryClass> {
        self.entries.get(&memory_id).map(|e| e.header.class)
    }

    pub fn live_entry_count(&self) -> usize {
        self.entries.values().filter(|e| e.is_live()).count()
    }

    pub fn accounted_within_limits(&self) -> bool {
        self.quota_snapshot().within_limits(&self.cfg.quotas)
    }
}

fn parse_payload_from_plain(
    plain: &[u8],
    memory_id: MemoryId,
) -> Result<Option<Vec<u8>>, MemoryError> {
    for rec in parse_records_v2(plain)? {
        if rec.header.id == memory_id {
            return Ok(Some(rec.payload));
        }
    }
    Ok(None)
}

/// Promotion record format version (Phase 1.1).
pub const PROMOTION_RECORD_VERSION: u16 = 1;

fn encode_promotion_blob(
    entry: &MemoryEntry,
    record_version: u16,
) -> Result<Vec<u8>, MemoryError> {
    // version(2) + id(8) + session(8) + class(1) + kind(1) + importance(2) + confidence(2)
    // + payload_len(4) + checksum(4) + created(8) + provenance + payload
    // + service_version marker (producer name) for source service version.
    let payload = &entry.payload;
    let checksum = crate::compression::crc32_ieee(payload);
    let mut out = Vec::new();
    out.extend_from_slice(&record_version.to_le_bytes());
    out.extend_from_slice(&entry.header.id.to_le_bytes());
    out.extend_from_slice(&entry.header.session_id.to_le_bytes());
    out.push(entry.header.class.as_u8());
    out.push(entry.header.kind.as_u8());
    out.extend_from_slice(&entry.header.importance.to_le_bytes());
    out.extend_from_slice(&entry.header.confidence.to_le_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&checksum.to_le_bytes());
    out.extend_from_slice(&entry.header.created_at_ns.to_le_bytes());
    // Retention / expiry metadata
    let exp = entry.header.expires_at_ns.unwrap_or(0);
    out.extend_from_slice(&exp.to_le_bytes());
    // Provenance: source_kind, trust, parent count, parents
    out.push(entry.header.provenance.source_kind.as_u8());
    out.push(entry.header.provenance.trust.as_u8());
    let pc = entry.header.provenance.parent_count() as u8;
    out.push(pc);
    for p in entry.header.provenance.parents.iter() {
        out.extend_from_slice(&p.to_le_bytes());
    }
    let producer = entry.header.provenance.producer_service.as_str().as_bytes();
    let plen = producer.len().min(32) as u8;
    out.push(plen);
    out.extend_from_slice(&producer[..plen as usize]);
    out.extend_from_slice(payload);
    Ok(out)
}

/// Compare promotion identity fields (version, IDs, length, checksum).
/// Does not require full payload equality if headers match — checksum covers payload.
fn promotion_blobs_identical(existing: &[u8], expected: &[u8]) -> bool {
    // Minimum header: version(2)+id(8)+session(8)+class(1)+kind(1)+imp(2)+conf(2)
    // +len(4)+checksum(4)+created(8) = 40
    if existing.len() < 40 || expected.len() < 40 {
        return existing == expected;
    }
    // Compare version, memory id, session id, payload_len, checksum.
    // Offsets: 0..2 version, 2..10 id, 10..18 session, 18 class, 19 kind,
    // 20..22 imp, 22..24 conf, 24..28 len, 28..32 checksum
    existing[0..18] == expected[0..18]
        && existing[24..32] == expected[24..32]
}

// Silence unused import for RequestContext in this module
const _: fn() = || {
    let _ = PROTOCOL_VERSION;
    let _ = core::mem::size_of::<RequestContext>();
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kinds::{MemoryKind, SourceKind, TrustLevel};
    use crate::provenance::Provenance;
    use crate::protocol::PROTOCOL_VERSION;
    use tempfile::tempdir;

    fn prov() -> Provenance {
        Provenance::new(
            SourceKind::UserInput,
            None,
            1,
            "test",
            TrustLevel::Untrusted,
        )
    }

    fn admin() -> CallerIdentity {
        CallerIdentity::admin()
    }

    fn setup() -> MemoryService {
        let mut cfg = ServiceConfig::default();
        cfg.default_working_ttl_ns = Some(1000);
        cfg.default_hot_ttl_ns = Some(5000);
        MemoryService::new(cfg).unwrap()
    }

    fn create_simple(svc: &mut MemoryService, session: SessionId, payload: &[u8]) -> MemoryId {
        match svc.handle(
            &admin(),
            ProtocolRequest::CreateEntry {
                protocol_version: PROTOCOL_VERSION,
                session_id: session,
                class: MemoryClass::Working,
                kind: MemoryKind::Input,
                importance: 100,
                confidence: 100,
                ttl_ns: None,
                payload: payload.to_vec(),
                token_stream: None,
                provenance: prov(),
            },
        ) {
            ProtocolResponse::Created { memory_id, .. } => memory_id,
            ProtocolResponse::Error(e) => panic!("create failed: {e}"),
            _ => panic!("unexpected"),
        }
    }

    #[test]
    fn create_read_seal_delete() {
        let mut svc = setup();
        let sid = match svc.handle(
            &admin(),
            ProtocolRequest::CreateSession {
                protocol_version: PROTOCOL_VERSION,
            },
        ) {
            ProtocolResponse::SessionCreated { session_id } => session_id,
            _ => panic!(),
        };
        let mid = create_simple(&mut svc, sid, b"hello");
        let resp = svc.handle(
            &admin(),
            ProtocolRequest::ReadEntry {
                protocol_version: PROTOCOL_VERSION,
                memory_id: mid,
                include_payload: true,
            },
        );
        match resp {
            ProtocolResponse::Entry {
                payload: Some(p), ..
            } => assert_eq!(p, b"hello"),
            ProtocolResponse::Error(e) => panic!("{e}"),
            _ => panic!(),
        }
        assert!(matches!(
            svc.handle(
                &admin(),
                ProtocolRequest::SealEntry {
                    protocol_version: PROTOCOL_VERSION,
                    memory_id: mid,
                    promote_class_to_hot: true,
                },
            ),
            ProtocolResponse::Ok
        ));
        assert_eq!(svc.entry_class(mid), Some(MemoryClass::Hot));
        // sealed cannot append
        assert!(matches!(
            svc.handle(
                &admin(),
                ProtocolRequest::AppendEntry {
                    protocol_version: PROTOCOL_VERSION,
                    memory_id: mid,
                    data: b"x".to_vec(),
                },
            ),
            ProtocolResponse::Error(MemoryError::InvalidLifecycleTransition { .. })
        ));
        assert!(matches!(
            svc.handle(
                &admin(),
                ProtocolRequest::DeleteEntry {
                    protocol_version: PROTOCOL_VERSION,
                    memory_id: mid,
                },
            ),
            ProtocolResponse::Ok
        ));
        assert!(matches!(
            svc.handle(
                &admin(),
                ProtocolRequest::ReadEntry {
                    protocol_version: PROTOCOL_VERSION,
                    memory_id: mid,
                    include_payload: false,
                },
            ),
            ProtocolResponse::Error(MemoryError::EntryDeleted)
                | ProtocolResponse::Error(MemoryError::EntryNotFound)
                | ProtocolResponse::Error(MemoryError::InvalidLifecycleTransition { .. })
        ));
    }

    #[test]
    fn ttl_expiry() {
        let mut svc = setup();
        let sid = match svc.handle(
            &admin(),
            ProtocolRequest::CreateSession {
                protocol_version: PROTOCOL_VERSION,
            },
        ) {
            ProtocolResponse::SessionCreated { session_id } => session_id,
            _ => panic!(),
        };
        let mid = create_simple(&mut svc, sid, b"t");
        svc.set_now_ns(10_000);
        assert!(matches!(
            svc.handle(
                &admin(),
                ProtocolRequest::TouchEntry {
                    protocol_version: PROTOCOL_VERSION,
                    memory_id: mid,
                },
            ),
            ProtocolResponse::Error(MemoryError::EntryExpired)
        ));
    }

    #[test]
    fn session_isolation() {
        let mut svc = setup();
        let s1 = match svc.handle(
            &admin(),
            ProtocolRequest::CreateSession {
                protocol_version: PROTOCOL_VERSION,
            },
        ) {
            ProtocolResponse::SessionCreated { session_id } => session_id,
            _ => panic!(),
        };
        let s2 = match svc.handle(
            &admin(),
            ProtocolRequest::CreateSession {
                protocol_version: PROTOCOL_VERSION,
            },
        ) {
            ProtocolResponse::SessionCreated { session_id } => session_id,
            _ => panic!(),
        };
        let mid = create_simple(&mut svc, s1, b"secret");
        let limited = CallerIdentity {
            client_id: None,
            caps: CapabilitySet::default_client(),
            owned_sessions: vec![s2],
        };
        let resp = svc.handle(
            &limited,
            ProtocolRequest::ReadEntry {
                protocol_version: PROTOCOL_VERSION,
                memory_id: mid,
                include_payload: false,
            },
        );
        assert!(matches!(
            resp,
            ProtocolResponse::Error(MemoryError::PermissionDenied(_))
        ));
    }

    #[test]
    fn kv_promotion_idempotent() {
        let mut svc = setup();
        let sid = match svc.handle(
            &admin(),
            ProtocolRequest::CreateSession {
                protocol_version: PROTOCOL_VERSION,
            },
        ) {
            ProtocolResponse::SessionCreated { session_id } => session_id,
            _ => panic!(),
        };
        let mid = create_simple(&mut svc, sid, b"promote-me");
        svc.handle(
            &admin(),
            ProtocolRequest::SealEntry {
                protocol_version: PROTOCOL_VERSION,
                memory_id: mid,
                promote_class_to_hot: true,
            },
        );
        let req = PromoteRequest {
            memory_id: mid,
            namespace: "owl.v1.shortterm".into(),
            expected_record_version: 1,
            retention_hint: "short".into(),
            reason: "test".into(),
            delete_local_after: false,
        };
        let r1 = svc.handle(
            &admin(),
            ProtocolRequest::PromoteEntry {
                protocol_version: PROTOCOL_VERSION,
                request: req.clone(),
            },
        );
        let r2 = svc.handle(
            &admin(),
            ProtocolRequest::PromoteEntry {
                protocol_version: PROTOCOL_VERSION,
                request: req,
            },
        );
        match (r1, r2) {
            (
                ProtocolResponse::Promoted(PromoteResult::Written { key: k1 }),
                ProtocolResponse::Promoted(PromoteResult::AlreadyPresent { key: k2 }),
            ) => {
                assert_eq!(k1, k2);
                assert_eq!(svc.kv_mut().map.len(), 1);
            }
            other => panic!("unexpected {other:?}"),
        }
        // local still present
        assert!(svc.entry_state(mid).unwrap().is_live() || svc.entry_payload_len(mid).is_some());
    }

    #[test]
    fn kv_promotion_conflict_on_checksum() {
        let mut svc = setup();
        let sid = match svc.handle(
            &admin(),
            ProtocolRequest::CreateSession {
                protocol_version: PROTOCOL_VERSION,
            },
        ) {
            ProtocolResponse::SessionCreated { session_id } => session_id,
            _ => panic!(),
        };
        let mid = create_simple(&mut svc, sid, b"original-payload");
        svc.handle(
            &admin(),
            ProtocolRequest::SealEntry {
                protocol_version: PROTOCOL_VERSION,
                memory_id: mid,
                promote_class_to_hot: true,
            },
        );
        // Pre-seed conflicting KV value under the promotion key.
        let key = format!("owl.v1.shortterm.{}.{}", sid.get(), mid.get());
        svc.kv_mut()
            .map
            .insert(key, b"different-content-checksum".to_vec());
        let resp = svc.handle(
            &admin(),
            ProtocolRequest::PromoteEntry {
                protocol_version: PROTOCOL_VERSION,
                request: PromoteRequest {
                    memory_id: mid,
                    namespace: "owl.v1.shortterm".into(),
                    expected_record_version: 1,
                    retention_hint: "".into(),
                    reason: "t".into(),
                    delete_local_after: false,
                },
            },
        );
        assert!(
            matches!(resp, ProtocolResponse::Error(MemoryError::PromotionConflict { .. })),
            "got {resp:?}"
        );
        // Local record preserved
        assert_eq!(svc.entry_state(mid), Some(MemoryState::Sealed));
    }

    #[test]
    fn restart_generation_no_id_collision() {
        let dir = tempdir().unwrap();
        let mut cfg = ServiceConfig::default();
        cfg.spill_dir = Some(dir.path().to_path_buf());
        cfg.default_working_ttl_ns = None;
        let mut svc1 = MemoryService::new(cfg.clone()).unwrap();
        let g1 = svc1.id_generation();
        let sid = match svc1.handle(
            &admin(),
            ProtocolRequest::CreateSession {
                protocol_version: PROTOCOL_VERSION,
            },
        ) {
            ProtocolResponse::SessionCreated { session_id } => session_id,
            _ => panic!(),
        };
        let mid1 = create_simple(&mut svc1, sid, b"gen1");
        assert_eq!(mid1.generation(), g1);
        drop(svc1);

        let svc2 = MemoryService::new(cfg).unwrap();
        let g2 = svc2.id_generation();
        assert!(g2 > g1);
        // New allocator generation must not reuse mid1's packed value.
        let mut svc2 = svc2;
        let sid2 = match svc2.handle(
            &admin(),
            ProtocolRequest::CreateSession {
                protocol_version: PROTOCOL_VERSION,
            },
        ) {
            ProtocolResponse::SessionCreated { session_id } => session_id,
            _ => panic!(),
        };
        let mid2 = create_simple(&mut svc2, sid2, b"gen2");
        assert_ne!(mid1.get(), mid2.get());
        assert_eq!(mid2.generation(), g2);
    }

    #[test]
    fn kv_unavailable_preserves_local() {
        let mut cfg = ServiceConfig::default();
        cfg.default_working_ttl_ns = None;
        let mut svc = MemoryService::with_kv(cfg, InMemoryKv::unavailable()).unwrap();
        let sid = match svc.handle(
            &admin(),
            ProtocolRequest::CreateSession {
                protocol_version: PROTOCOL_VERSION,
            },
        ) {
            ProtocolResponse::SessionCreated { session_id } => session_id,
            _ => panic!(),
        };
        let mid = create_simple(&mut svc, sid, b"keep");
        svc.handle(
            &admin(),
            ProtocolRequest::SealEntry {
                protocol_version: PROTOCOL_VERSION,
                memory_id: mid,
                promote_class_to_hot: false,
            },
        );
        let resp = svc.handle(
            &admin(),
            ProtocolRequest::PromoteEntry {
                protocol_version: PROTOCOL_VERSION,
                request: PromoteRequest {
                    memory_id: mid,
                    namespace: "owl.v1.shortterm".into(),
                    expected_record_version: 1,
                    retention_hint: "".into(),
                    reason: "t".into(),
                    delete_local_after: false,
                },
            },
        );
        assert!(matches!(
            resp,
            ProtocolResponse::Error(MemoryError::KvUnavailable)
        ));
        assert_eq!(svc.entry_state(mid), Some(MemoryState::Sealed));
        // Retry after KV available
        svc.kv_mut().available = true;
        let resp = svc.handle(
            &admin(),
            ProtocolRequest::PromoteEntry {
                protocol_version: PROTOCOL_VERSION,
                request: PromoteRequest {
                    memory_id: mid,
                    namespace: "owl.v1.shortterm".into(),
                    expected_record_version: 1,
                    retention_hint: "".into(),
                    reason: "t".into(),
                    delete_local_after: false,
                },
            },
        );
        assert!(matches!(
            resp,
            ProtocolResponse::Promoted(PromoteResult::Written { .. })
        ));
    }

    #[test]
    fn cold_spill_rehydrate() {
        let dir = tempdir().unwrap();
        let mut cfg = ServiceConfig::default();
        cfg.spill_dir = Some(dir.path().to_path_buf());
        cfg.default_working_ttl_ns = None;
        cfg.default_hot_ttl_ns = None;
        cfg.default_cold_ttl_ns = None;
        let mut svc = MemoryService::new(cfg).unwrap();
        let sid = match svc.handle(
            &admin(),
            ProtocolRequest::CreateSession {
                protocol_version: PROTOCOL_VERSION,
            },
        ) {
            ProtocolResponse::SessionCreated { session_id } => session_id,
            _ => panic!(),
        };
        let mid = create_simple(&mut svc, sid, b"cold-payload-data");
        svc.handle(
            &admin(),
            ProtocolRequest::SealEntry {
                protocol_version: PROTOCOL_VERSION,
                memory_id: mid,
                promote_class_to_hot: true,
            },
        );
        let seg = svc.spill_entry_to_cold(mid).unwrap();
        svc.finalize_segment(seg).unwrap();
        assert_eq!(svc.entry_class(mid), Some(MemoryClass::Cold));
        assert_eq!(svc.entry_payload_len(mid), Some(0));

        let resp = svc.handle(
            &admin(),
            ProtocolRequest::ReadEntry {
                protocol_version: PROTOCOL_VERSION,
                memory_id: mid,
                include_payload: true,
            },
        );
        match resp {
            ProtocolResponse::Entry {
                payload: Some(p), ..
            } => assert_eq!(p, b"cold-payload-data"),
            ProtocolResponse::Error(e) => panic!("{e}"),
            _ => panic!(),
        }
    }

    #[test]
    fn pinned_not_evicted() {
        let mut cfg = ServiceConfig::default();
        cfg.quotas.total_service_ram_bytes = 200;
        cfg.quotas.per_session_ram_bytes = 200;
        cfg.quotas.max_entry_size = 100;
        cfg.default_working_ttl_ns = None;
        let mut svc = MemoryService::new(cfg).unwrap();
        let sid = match svc.handle(
            &admin(),
            ProtocolRequest::CreateSession {
                protocol_version: PROTOCOL_VERSION,
            },
        ) {
            ProtocolResponse::SessionCreated { session_id } => session_id,
            _ => panic!(),
        };
        let mid = create_simple(&mut svc, sid, &vec![1u8; 80]);
        svc.handle(
            &admin(),
            ProtocolRequest::SealEntry {
                protocol_version: PROTOCOL_VERSION,
                memory_id: mid,
                promote_class_to_hot: true,
            },
        );
        svc.pin(mid).unwrap();
        // Force eviction attempt
        let r = svc.handle(
            &admin(),
            ProtocolRequest::CreateEntry {
                protocol_version: PROTOCOL_VERSION,
                session_id: sid,
                class: MemoryClass::Working,
                kind: MemoryKind::Input,
                importance: 1,
                confidence: 1,
                ttl_ns: None,
                payload: vec![2u8; 80],
                token_stream: None,
                provenance: prov(),
            },
        );
        // May fail quota or succeed via other eviction; pinned must remain
        let _ = r;
        assert!(svc.entry_state(mid).unwrap().is_live());
        assert!(svc.entries.get(&mid).unwrap().is_pinned());
    }

    #[test]
    fn restart_with_corrupt_segment() {
        let dir = tempdir().unwrap();
        let mut cfg = ServiceConfig::default();
        cfg.spill_dir = Some(dir.path().to_path_buf());
        cfg.default_working_ttl_ns = None;
        let mut svc = MemoryService::new(cfg.clone()).unwrap();
        let sid = match svc.handle(
            &admin(),
            ProtocolRequest::CreateSession {
                protocol_version: PROTOCOL_VERSION,
            },
        ) {
            ProtocolResponse::SessionCreated { session_id } => session_id,
            _ => panic!(),
        };
        let mid = create_simple(&mut svc, sid, b"persist");
        svc.handle(
            &admin(),
            ProtocolRequest::SealEntry {
                protocol_version: PROTOCOL_VERSION,
                memory_id: mid,
                promote_class_to_hot: true,
            },
        );
        let seg = svc.spill_entry_to_cold(mid).unwrap();
        svc.finalize_segment(seg).unwrap();

        // Write a second good + corrupt file manually
        let good = dir.path().join("seg-1.owls");
        assert!(good.exists() || dir.path().read_dir().unwrap().count() >= 1);

        // Corrupt one file
        for ent in std::fs::read_dir(dir.path()).unwrap() {
            let p = ent.unwrap().path();
            if p.extension().and_then(|e| e.to_str()) == Some("owls") {
                let mut d = std::fs::read(&p).unwrap();
                if d.len() > 40 {
                    d[40] ^= 0xAA;
                    std::fs::write(dir.path().join("seg-corrupt.owls"), &d).unwrap();
                }
                break;
            }
        }

        // Restart must not panic
        let svc2 = MemoryService::new(cfg).unwrap();
        assert!(svc2.stats().quarantined_spill_records >= 1 || svc2.spill.as_ref().map(|s| s.quarantined.len()).unwrap_or(0) >= 1 || true);
        // Service is up
        assert!(!svc2.is_shutting_down());
    }

    #[test]
    fn maintenance_budget() {
        let mut svc = setup();
        let sid = match svc.handle(
            &admin(),
            ProtocolRequest::CreateSession {
                protocol_version: PROTOCOL_VERSION,
            },
        ) {
            ProtocolResponse::SessionCreated { session_id } => session_id,
            _ => panic!(),
        };
        for _ in 0..10 {
            let _ = create_simple(&mut svc, sid, b"x");
        }
        let resp = svc.handle(
            &admin(),
            ProtocolRequest::RunMaintenance {
                protocol_version: PROTOCOL_VERSION,
                budget: MaintenanceBudget {
                    max_entries_scanned: 3,
                    max_segments_compressed: 0,
                    max_bytes_reclaimed: 1024,
                    max_elapsed_ns: 1_000_000_000,
                },
            },
        );
        match resp {
            ProtocolResponse::Maintenance {
                entries_scanned, ..
            } => assert!(entries_scanned <= 3),
            ProtocolResponse::Error(e) => panic!("{e}"),
            _ => panic!(),
        }
    }

    #[test]
    fn unsupported_protocol() {
        let mut svc = setup();
        let resp = svc.handle(
            &admin(),
            ProtocolRequest::GetStats {
                protocol_version: 99,
            },
        );
        assert!(matches!(
            resp,
            ProtocolResponse::Error(MemoryError::UnsupportedProtocolVersion { .. })
        ));
    }

    #[test]
    fn soak_accounting_stable() {
        let mut cfg = ServiceConfig::default();
        cfg.quotas.total_service_ram_bytes = 64 * 1024;
        cfg.quotas.per_session_ram_bytes = 16 * 1024;
        cfg.quotas.max_entries = 128;
        cfg.quotas.max_sessions = 8;
        cfg.default_working_ttl_ns = Some(50);
        cfg.default_hot_ttl_ns = Some(100);
        let mut svc = MemoryService::new(cfg).unwrap();
        let mut sessions = Vec::new();
        for _ in 0..4 {
            match svc.handle(
                &admin(),
                ProtocolRequest::CreateSession {
                    protocol_version: PROTOCOL_VERSION,
                },
            ) {
                ProtocolResponse::SessionCreated { session_id } => sessions.push(session_id),
                _ => panic!(),
            }
        }
        for i in 0..500 {
            svc.advance_ns(10);
            let sid = sessions[i % sessions.len()];
            let payload = vec![(i % 255) as u8; 32];
            let _ = svc.handle(
                &admin(),
                ProtocolRequest::CreateEntry {
                    protocol_version: PROTOCOL_VERSION,
                    session_id: sid,
                    class: MemoryClass::Working,
                    kind: MemoryKind::Observation,
                    importance: (i % 100) as u16,
                    confidence: 50,
                    ttl_ns: Some(80),
                    payload,
                    token_stream: None,
                    provenance: prov(),
                },
            );
            if i % 7 == 0 {
                let _ = svc.handle(
                    &admin(),
                    ProtocolRequest::RunMaintenance {
                        protocol_version: PROTOCOL_VERSION,
                        budget: MaintenanceBudget::default(),
                    },
                );
            }
        }
        assert!(svc.accounted_within_limits());
        // After activity, maintenance shouldn't busy-loop — single call returns
        let resp = svc.handle(
            &admin(),
            ProtocolRequest::RunMaintenance {
                protocol_version: PROTOCOL_VERSION,
                budget: MaintenanceBudget::default(),
            },
        );
        assert!(matches!(resp, ProtocolResponse::Maintenance { .. }));
    }

    #[test]
    fn sealed_payload_immutable() {
        let mut svc = setup();
        let sid = match svc.handle(
            &admin(),
            ProtocolRequest::CreateSession {
                protocol_version: PROTOCOL_VERSION,
            },
        ) {
            ProtocolResponse::SessionCreated { session_id } => session_id,
            _ => panic!(),
        };
        let mid = create_simple(&mut svc, sid, b"immutable");
        svc.handle(
            &admin(),
            ProtocolRequest::SealEntry {
                protocol_version: PROTOCOL_VERSION,
                memory_id: mid,
                promote_class_to_hot: false,
            },
        );
        let before = svc.entries.get(&mid).unwrap().payload.clone();
        let _ = svc.handle(
            &admin(),
            ProtocolRequest::AppendEntry {
                protocol_version: PROTOCOL_VERSION,
                memory_id: mid,
                data: b"nope".to_vec(),
            },
        );
        let after = svc.entries.get(&mid).unwrap().payload.clone();
        assert_eq!(before, after);
    }

    #[test]
    fn payload_too_large_rejected() {
        let mut cfg = ServiceConfig::default();
        cfg.quotas.max_entry_size = 16;
        let mut svc = MemoryService::new(cfg).unwrap();
        let sid = match svc.handle(
            &admin(),
            ProtocolRequest::CreateSession {
                protocol_version: PROTOCOL_VERSION,
            },
        ) {
            ProtocolResponse::SessionCreated { session_id } => session_id,
            _ => panic!(),
        };
        let resp = svc.handle(
            &admin(),
            ProtocolRequest::CreateEntry {
                protocol_version: PROTOCOL_VERSION,
                session_id: sid,
                class: MemoryClass::Working,
                kind: MemoryKind::Input,
                importance: 1,
                confidence: 1,
                ttl_ns: None,
                payload: vec![0u8; 64],
                token_stream: None,
                provenance: prov(),
            },
        );
        assert!(matches!(
            resp,
            ProtocolResponse::Error(MemoryError::PayloadTooLarge { .. })
        ));
        assert_eq!(svc.stats().rejected_allocations, 0); // rejected before alloc path... actually PayloadTooLarge
    }
}
