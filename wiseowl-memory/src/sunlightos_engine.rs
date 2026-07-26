//! Alloc-based short-term memory engine for the native SunlightOS daemon.
//!
//! Mirrors the host [`crate::service::MemoryService`] lifecycle and quotas so
//! the native transport never reimplements memory rules. Host tests continue
//! to exercise the std engine; this module is compiled for `sunlightos` and
//! for host unit tests of the alloc path.

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use crate::caps::MemoryCapability;
use crate::compression::crc32_ieee;
use crate::entry::{MemoryEntry, MemoryEntryHeader, MemoryState, TokenStreamRef, ENTRY_HEADER_VERSION};
use crate::error::MemoryError;
use crate::ids::{ClientId, IdAllocator, MemoryId, SegmentId, SessionId};
use crate::kinds::{MemoryClass, MemoryKind};
use crate::lifecycle::LifecycleOp;
use crate::protocol::{
    check_protocol_version, request_version, ListFilter, MaintenanceBudget, PromoteRequest,
    PromoteResult, ProtocolRequest, ProtocolResponse,
};
use crate::provenance::Provenance;
use crate::quotas::{QuotaConfig, QuotaSnapshot, SessionQuota};
use crate::segments::{parse_records_v2, Segment, SegmentState};
use crate::caller::CallerIdentity;
use crate::health::{degraded, ServiceHealth};
use crate::stats::ServiceStats;

/// Pluggable KV backend (native sunlight-kv client or test double).
pub trait NativeKvBackend {
    fn put_if_absent(&mut self, key: &str, value: &[u8]) -> Result<NativeKvPut, MemoryError>;
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, MemoryError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeKvPut {
    Written,
    AlreadyPresent,
}

#[derive(Debug, Default)]
pub struct RamKv {
    map: BTreeMap<String, Vec<u8>>,
    pub available: bool,
}

impl RamKv {
    pub fn new() -> Self {
        Self {
            map: BTreeMap::new(),
            available: true,
        }
    }
}

impl NativeKvBackend for RamKv {
    fn put_if_absent(&mut self, key: &str, value: &[u8]) -> Result<NativeKvPut, MemoryError> {
        if !self.available {
            return Err(MemoryError::KvUnavailable);
        }
        if self.map.contains_key(key) {
            return Ok(NativeKvPut::AlreadyPresent);
        }
        self.map.insert(String::from(key), value.to_vec());
        Ok(NativeKvPut::Written)
    }

    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, MemoryError> {
        if !self.available {
            return Err(MemoryError::KvUnavailable);
        }
        Ok(self.map.get(key).cloned())
    }
}

struct SessionRec {
    owner: Option<ClientId>,
    quota: SessionQuota,
    open_segment: Option<SegmentId>,
}

/// Active SHM read lease (service-created immutable view).
#[derive(Debug)]
pub struct ReadLease {
    pub lease_id: u64,
    pub client_id: Option<ClientId>,
    pub memory_id: MemoryId,
    pub length: u32,
    pub expires_at_ns: u64,
}

/// Native short-term memory engine (transport-independent).
pub struct NativeMemoryEngine<K: NativeKvBackend = RamKv> {
    pub quotas: QuotaConfig,
    ids: IdAllocator,
    sessions: BTreeMap<SessionId, SessionRec>,
    entries: BTreeMap<MemoryId, MemoryEntry>,
    segments: BTreeMap<SegmentId, Segment>,
    clients: BTreeMap<ClientId, String>,
    client_sessions: BTreeMap<ClientId, Vec<SessionId>>,
    kv: K,
    stats: ServiceStats,
    now_ns: u64,
    shutting_down: bool,
    health: ServiceHealth,
    degraded_flags: u32,
    leases: BTreeMap<u64, ReadLease>,
    next_lease: u64,
    /// In-memory cold spill blobs keyed by segment id (persisted by the daemon).
    cold_blobs: BTreeMap<SegmentId, Vec<u8>>,
    producer_name: String,
}

impl NativeMemoryEngine<RamKv> {
    pub fn new() -> Self {
        Self::with_kv(RamKv::new())
    }
}

impl<K: NativeKvBackend> NativeMemoryEngine<K> {
    pub fn with_kv(kv: K) -> Self {
        Self {
            quotas: QuotaConfig::default(),
            ids: IdAllocator::new(),
            sessions: BTreeMap::new(),
            entries: BTreeMap::new(),
            segments: BTreeMap::new(),
            clients: BTreeMap::new(),
            client_sessions: BTreeMap::new(),
            kv,
            stats: ServiceStats::default(),
            now_ns: 1,
            shutting_down: false,
            health: ServiceHealth::Ready,
            degraded_flags: 0,
            leases: BTreeMap::new(),
            next_lease: 1,
            cold_blobs: BTreeMap::new(),
            producer_name: String::from("wiseowl-memoryd"),
        }
    }

    /// Configure allocator generation after loading persistent generation.
    pub fn set_generation(&mut self, generation: u16) -> Result<(), MemoryError> {
        self.ids
            .set_generation(generation)
            .map_err(|_| MemoryError::InternalInvariantViolation("generation"))
    }

    pub fn note_seen_id(&mut self, raw: u64) {
        self.ids.note_seen(raw);
    }

    pub fn generation(&self) -> u16 {
        self.ids.generation()
    }

    pub fn set_now_ns(&mut self, now: u64) {
        self.now_ns = now.max(1);
    }

    pub fn now_ns(&self) -> u64 {
        self.now_ns
    }

    pub fn health(&self) -> ServiceHealth {
        self.health
    }

    pub fn degraded_flags(&self) -> u32 {
        self.degraded_flags
    }

    pub fn stats(&self) -> &ServiceStats {
        &self.stats
    }

    pub fn set_kv_degraded(&mut self, degraded: bool) {
        if degraded {
            self.degraded_flags |= degraded::KV_UNAVAILABLE;
        } else {
            self.degraded_flags &= !degraded::KV_UNAVAILABLE;
        }
        self.refresh_health();
    }

    pub fn mark_quarantine_degraded(&mut self) {
        self.degraded_flags |= degraded::SPILL_QUARANTINED;
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

    pub fn begin_shutdown(&mut self) {
        self.shutting_down = true;
        self.health = ServiceHealth::Stopping;
        // Release all leases.
        self.leases.clear();
        self.stats.active_read_leases = 0;
        self.stats.shared_memory_leased_bytes = 0;
    }

    /// Install a recovered cold blob and reconstruct entries.
    pub fn recover_cold_blob(&mut self, blob: &[u8]) -> Result<(), MemoryError> {
        let (mut seg, plain) =
            Segment::from_spill_blob(blob, self.quotas.max_decompress_bytes)?;
        let sid = seg.id;
        self.ids.note_seen(sid.get());
        let recs = parse_records_v2(&plain)?;
        for rec in recs {
            self.ids.note_seen(rec.header.id.get());
            self.ids.note_seen(rec.header.session_id.get());
            self.sessions
                .entry(rec.header.session_id)
                .or_insert(SessionRec {
                    owner: None,
                    quota: SessionQuota::default(),
                    open_segment: None,
                });
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
            entry.header.payload_len = payload_len;
            entry.header.class = MemoryClass::Cold;
            self.entries.insert(rec.header.id, entry);
        }
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
        seg.plain.clear();
        seg.state = SegmentState::Spilled;
        self.stats.cold_compressed_bytes =
            self.stats.cold_compressed_bytes.saturating_add(comp_len);
        self.stats.cold_uncompressed_logical_bytes = self
            .stats
            .cold_uncompressed_logical_bytes
            .saturating_add(plain_len);
        self.stats.segment_count = self.stats.segment_count.saturating_add(1);
        self.cold_blobs.insert(sid, blob.to_vec());
        self.segments.insert(sid, seg);
        self.stats.entry_count = self.entries.len() as u64;
        self.stats.active_sessions = self.sessions.len() as u64;
        Ok(())
    }

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
                let id = self
                    .ids
                    .alloc_client()
                    .map_err(|_| MemoryError::InternalInvariantViolation("client id"))?;
                self.clients.insert(id, name);
                Ok(ProtocolResponse::ClientRegistered { client_id: id })
            }
            ProtocolRequest::ClientDisconnect { client_id, .. } => {
                self.client_disconnect(client_id)?;
                Ok(ProtocolResponse::Ok)
            }
            ProtocolRequest::CreateSession { .. } => {
                caller.caps.require(MemoryCapability::Create)?;
                let sid = self
                    .ids
                    .alloc_session()
                    .map_err(|_| MemoryError::InternalInvariantViolation("session id"))?;
                self.sessions.insert(
                    sid,
                    SessionRec {
                        owner: caller.client_id,
                        quota: SessionQuota::default(),
                        open_segment: None,
                    },
                );
                if let Some(cid) = caller.client_id {
                    self.client_sessions.entry(cid).or_default().push(sid);
                }
                self.stats.active_sessions = self.sessions.len() as u64;
                Ok(ProtocolResponse::SessionCreated { session_id: sid })
            }
            ProtocolRequest::ListSessions { .. } => {
                caller.caps.require(MemoryCapability::InspectMetadata)?;
                let ids: Vec<SessionId> = if caller.caps.has(MemoryCapability::ReadSharedSession)
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
        if let Some(cid) = caller.client_id {
            if let Some(s) = self.sessions.get(&session_id) {
                if s.owner == Some(cid) {
                    return Ok(());
                }
            }
        }
        let _ = for_write;
        if caller.caps.has(MemoryCapability::ReadOwnSession)
            && caller.owned_sessions.contains(&session_id)
        {
            return Ok(());
        }
        // Admin empty owned_sessions with full mask
        if caller.caps.mask() == u64::MAX {
            return Ok(());
        }
        Err(MemoryError::PermissionDenied("session"))
    }

    fn create_entry(
        &mut self,
        caller: &CallerIdentity,
        session_id: SessionId,
        class: MemoryClass,
        kind: MemoryKind,
        importance: u16,
        confidence: u16,
        ttl_ns: Option<u64>,
        payload: Vec<u8>,
        token_stream: Option<TokenStreamRef>,
        provenance: Provenance,
    ) -> Result<MemoryId, MemoryError> {
        MemoryEntryHeader::validate_scores(importance, confidence)?;
        MemoryEntryHeader::validate_payload_len(payload.len() as u32, self.quotas.max_entry_size)?;
        let need = payload.len() as u64;
        let snap = self.quota_snapshot();
        if snap.checked_add_ram(need, &self.quotas).is_err() {
            self.evict_for_space(need)?;
        }
        self.quota_snapshot()
            .checked_add_ram(need, &self.quotas)?;
        if let Some(s) = self.sessions.get(&session_id) {
            s.quota.can_add_ram(need, &self.quotas)?;
        } else {
            return Err(MemoryError::SessionNotFound);
        }
        if self.stats.entry_count as u32 >= self.quotas.max_entries {
            return Err(MemoryError::QuotaExceeded("max entries"));
        }
        let id = self
            .ids
            .alloc_memory()
            .map_err(|_| MemoryError::InternalInvariantViolation("memory id"))?;
        let exp = ttl_ns.map(|t| self.now_ns.saturating_add(t));
        let header = MemoryEntryHeader {
            version: ENTRY_HEADER_VERSION,
            id,
            session_id,
            class,
            kind,
            created_at_ns: self.now_ns,
            last_access_ns: self.now_ns,
            expires_at_ns: exp,
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
        if let Some(s) = self.sessions.get_mut(&session_id) {
            s.quota.ram_bytes = s.quota.ram_bytes.saturating_add(need);
            s.quota.entry_count = s.quota.entry_count.saturating_add(1);
        }
        match class {
            MemoryClass::Working => {
                self.stats.working_bytes = self.stats.working_bytes.saturating_add(need);
            }
            MemoryClass::Hot => {
                self.stats.hot_bytes = self.stats.hot_bytes.saturating_add(need);
            }
            MemoryClass::Cold => {}
        }
        self.stats.entry_count = self.stats.entry_count.saturating_add(1);
        ServiceStats::inc(&mut self.stats.creates);
        self.entries.insert(id, entry);
        Ok(id)
    }

    fn append_entry(
        &mut self,
        caller: &CallerIdentity,
        memory_id: MemoryId,
        data: Vec<u8>,
    ) -> Result<(), MemoryError> {
        let session_id = {
            let e = self
                .entries
                .get(&memory_id)
                .ok_or(MemoryError::EntryNotFound)?;
            e.header.session_id
        };
        self.ensure_session_access(caller, session_id, true)?;
        let (new_len, add, class) = {
            let e = self
                .entries
                .get_mut(&memory_id)
                .ok_or(MemoryError::EntryNotFound)?;
            e.check_not_expired(self.now_ns)?;
            e.state.require(LifecycleOp::Append)?;
            let new_len = e
                .payload
                .len()
                .checked_add(data.len())
                .ok_or(MemoryError::InternalInvariantViolation("append overflow"))?;
            if new_len as u32 > self.quotas.max_entry_size {
                return Err(MemoryError::PayloadTooLarge {
                    size: new_len as u32,
                    max: self.quotas.max_entry_size,
                });
            }
            (new_len, data.len() as u64, e.header.class)
        };
        let _ = new_len;
        self.quota_snapshot()
            .checked_add_ram(add, &self.quotas)?;
        let e = self
            .entries
            .get_mut(&memory_id)
            .ok_or(MemoryError::EntryNotFound)?;
        e.payload.extend_from_slice(&data);
        e.header.payload_len = e.payload.len() as u32;
        e.header.last_access_ns = self.now_ns;
        match class {
            MemoryClass::Working => {
                self.stats.working_bytes = self.stats.working_bytes.saturating_add(add);
            }
            MemoryClass::Hot => {
                self.stats.hot_bytes = self.stats.hot_bytes.saturating_add(add);
            }
            MemoryClass::Cold => {}
        }
        Ok(())
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
        self.ensure_session_access(caller, session_id, false)?;
        // Rehydrate cold if needed.
        let need = {
            let e = self.entries.get(&memory_id).unwrap();
            e.state == MemoryState::Cold && e.payload.is_empty() && e.segment_id.is_some()
        };
        if need {
            self.rehydrate(memory_id)?;
        }
        let e = self
            .entries
            .get_mut(&memory_id)
            .ok_or(MemoryError::EntryNotFound)?;
        e.check_not_expired(self.now_ns)?;
        e.state.require(LifecycleOp::Read)?;
        e.header.last_access_ns = self.now_ns;
        let payload = if include_payload {
            caller.caps.require(MemoryCapability::ReadPayload)?;
            Some(e.payload.clone())
        } else {
            None
        };
        ServiceStats::inc(&mut self.stats.reads);
        Ok(ProtocolResponse::Entry {
            header: e.header.clone(),
            state: e.state,
            payload,
            promoted: e.promoted,
            segment_id: e.segment_id.map(|s| s.get()),
        })
    }

    fn rehydrate(&mut self, memory_id: MemoryId) -> Result<(), MemoryError> {
        let sid = self
            .entries
            .get(&memory_id)
            .and_then(|e| e.segment_id)
            .ok_or(MemoryError::SegmentNotFound)?;
        let blob = self
            .cold_blobs
            .get(&sid)
            .cloned()
            .ok_or(MemoryError::SegmentNotFound)?;
        let (_seg, plain) =
            Segment::from_spill_blob(&blob, self.quotas.max_decompress_bytes)?;
        let payload = parse_records_v2(&plain)?
            .into_iter()
            .find(|r| r.header.id == memory_id)
            .map(|r| r.payload)
            .ok_or(MemoryError::EntryNotFound)?;
        let e = self.entries.get_mut(&memory_id).unwrap();
        e.payload = payload;
        e.header.payload_len = e.payload.len() as u32;
        e.state = MemoryState::Sealed;
        e.header.class = MemoryClass::Hot;
        ServiceStats::inc(&mut self.stats.decompression_successes);
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
        self.ensure_session_access(caller, session_id, false)?;
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
        let session_id = {
            let e = self
                .entries
                .get(&memory_id)
                .ok_or(MemoryError::EntryNotFound)?;
            e.header.session_id
        };
        self.ensure_session_access(caller, session_id, true)?;
        let e = self
            .entries
            .get_mut(&memory_id)
            .ok_or(MemoryError::EntryNotFound)?;
        e.check_not_expired(self.now_ns)?;
        e.state.require(LifecycleOp::Seal)?;
        e.state = MemoryState::Sealed;
        // Shrink excess capacity after seal to keep logical vs actual closer.
        e.payload.shrink_to_fit();
        if promote_class_to_hot && e.header.class == MemoryClass::Working {
            let len = e.payload.len() as u64;
            e.header.class = MemoryClass::Hot;
            self.stats.working_bytes = self.stats.working_bytes.saturating_sub(len);
            self.stats.hot_bytes = self.stats.hot_bytes.saturating_add(len);
        }
        e.header.last_access_ns = self.now_ns;
        ServiceStats::inc(&mut self.stats.seals);
        Ok(())
    }

    fn delete_entry(
        &mut self,
        caller: &CallerIdentity,
        memory_id: MemoryId,
    ) -> Result<(), MemoryError> {
        let (session_id, class, len) = {
            let e = self
                .entries
                .get(&memory_id)
                .ok_or(MemoryError::EntryNotFound)?;
            self.ensure_session_access(caller, e.header.session_id, true)?;
            e.state.require(LifecycleOp::Delete)?;
            (
                e.header.session_id,
                e.header.class,
                e.payload.len() as u64,
            )
        };
        if let Some(e) = self.entries.get_mut(&memory_id) {
            e.state = MemoryState::Deleted;
            e.payload.clear();
            e.header.payload_len = 0;
        }
        self.unaccount(session_id, class, len);
        Ok(())
    }

    fn unaccount(&mut self, session_id: SessionId, class: MemoryClass, len: u64) {
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
        let session_id = {
            let e = self
                .entries
                .get(&req.memory_id)
                .ok_or(MemoryError::EntryNotFound)?;
            e.header.session_id
        };
        self.ensure_session_access(caller, session_id, true)?;
        let (key, value) = {
            let e = self
                .entries
                .get_mut(&req.memory_id)
                .ok_or(MemoryError::EntryNotFound)?;
            e.check_not_expired(self.now_ns)?;
            e.state.require(LifecycleOp::PromoteToKv)?;
            if e.state == MemoryState::Open {
                return Err(MemoryError::InvalidLifecycleTransition {
                    from: "open",
                    op: LifecycleOp::PromoteToKv,
                });
            }
            let ns = if req.namespace.is_empty() {
                "owl.v1.shortterm"
            } else {
                req.namespace.as_str()
            };
            let key = alloc::format!(
                "{}.{}.{}",
                ns,
                e.header.session_id.get(),
                e.header.id.get()
            );
            let value = encode_promo(e, req.expected_record_version)?;
            (key, value)
        };
        let outcome = match self.kv.put_if_absent(&key, &value) {
            Ok(o) => o,
            Err(MemoryError::KvUnavailable) => {
                ServiceStats::inc(&mut self.stats.kv_promotion_failures);
                self.set_kv_degraded(true);
                return Err(MemoryError::KvUnavailable);
            }
            Err(e) => {
                ServiceStats::inc(&mut self.stats.kv_promotion_failures);
                return Err(e);
            }
        };
        let result = match outcome {
            NativeKvPut::Written => PromoteResult::Written { key: key.clone() },
            NativeKvPut::AlreadyPresent => match self.kv.get(&key) {
                Ok(Some(existing)) => {
                    if promo_identical(&existing, &value) {
                        PromoteResult::AlreadyPresent { key: key.clone() }
                    } else {
                        ServiceStats::inc(&mut self.stats.kv_promotion_failures);
                        return Err(MemoryError::PromotionConflict { key: "mismatch" });
                    }
                }
                _ => {
                    ServiceStats::inc(&mut self.stats.kv_promotion_failures);
                    return Err(MemoryError::KvUnavailable);
                }
            },
        };
        let e = self.entries.get_mut(&req.memory_id).unwrap();
        e.kv_key = Some(key);
        e.promoted = true;
        if e.state != MemoryState::Cold {
            e.state = MemoryState::Promoted;
        }
        ServiceStats::inc(&mut self.stats.kv_promotion_successes);
        if req.delete_local_after && result.is_confirmed_success() {
            let class = e.header.class;
            let len = e.payload.len() as u64;
            let sid = e.header.session_id;
            e.state = MemoryState::Deleted;
            e.payload.clear();
            self.unaccount(sid, class, len);
        }
        Ok(result)
    }

    fn list_entries(
        &self,
        caller: &CallerIdentity,
        filter: ListFilter,
    ) -> Result<Vec<(MemoryEntryHeader, MemoryState)>, MemoryError> {
        let max = filter
            .max_results
            .unwrap_or(self.quotas.max_list_results)
            .min(self.quotas.max_list_results) as usize;
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
        // Release leases for this client.
        let lease_ids: Vec<u64> = self
            .leases
            .iter()
            .filter(|(_, l)| l.client_id == Some(client_id))
            .map(|(id, _)| *id)
            .collect();
        for id in lease_ids {
            self.release_lease(id);
        }
        // Unpin entries.
        for e in self.entries.values_mut() {
            if e.owner_client == Some(client_id) {
                e.pin_count = 0;
            }
        }
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
                let sid = e.header.session_id;
                let class = e.header.class;
                let len = e.payload.len() as u64;
                e.state = MemoryState::Deleted;
                e.payload.clear();
                self.unaccount(sid, class, len);
            }
        }
        self.clients.remove(&client_id);
        self.client_sessions.remove(&client_id);
        Ok(())
    }

    /// Issue a read lease tracking service-created SHM length.
    pub fn issue_read_lease(
        &mut self,
        client_id: Option<ClientId>,
        memory_id: MemoryId,
        length: u32,
        ttl_ns: u64,
    ) -> u64 {
        let id = self.next_lease;
        self.next_lease = self.next_lease.saturating_add(1);
        self.leases.insert(
            id,
            ReadLease {
                lease_id: id,
                client_id,
                memory_id,
                length,
                expires_at_ns: self.now_ns.saturating_add(ttl_ns),
            },
        );
        self.stats.active_read_leases = self.leases.len() as u64;
        self.stats.shared_memory_leased_bytes = self
            .stats
            .shared_memory_leased_bytes
            .saturating_add(length as u64);
        id
    }

    pub fn release_lease(&mut self, lease_id: u64) {
        if let Some(l) = self.leases.remove(&lease_id) {
            self.stats.shared_memory_leased_bytes = self
                .stats
                .shared_memory_leased_bytes
                .saturating_sub(l.length as u64);
            self.stats.active_read_leases = self.leases.len() as u64;
        }
    }

    fn run_maintenance(
        &mut self,
        budget: MaintenanceBudget,
    ) -> Result<ProtocolResponse, MemoryError> {
        ServiceStats::inc(&mut self.stats.maintenance_runs);
        let mut scanned = 0u32;
        let mut expired = 0u32;
        let mut reclaimed = 0u64;
        let now = self.now_ns;
        let ids: Vec<MemoryId> = self.entries.keys().copied().collect();
        for id in ids {
            if scanned >= budget.max_entries_scanned {
                break;
            }
            scanned = scanned.saturating_add(1);
            let Some(e) = self.entries.get_mut(&id) else {
                continue;
            };
            if let Some(exp) = e.header.expires_at_ns {
                if now >= exp && e.is_live() {
                    let class = e.header.class;
                    let len = e.payload.len() as u64;
                    let sid = e.header.session_id;
                    e.state = MemoryState::Expired;
                    e.payload.clear();
                    reclaimed = reclaimed.saturating_add(len);
                    expired = expired.saturating_add(1);
                    // unaccount without borrow conflict
                    let _ = e;
                    self.unaccount(sid, class, len);
                    ServiceStats::inc(&mut self.stats.expirations);
                }
            }
            if reclaimed >= budget.max_bytes_reclaimed {
                break;
            }
        }
        // Drop expired leases.
        let dead: Vec<u64> = self
            .leases
            .iter()
            .filter(|(_, l)| l.expires_at_ns <= now)
            .map(|(id, _)| *id)
            .collect();
        for id in dead {
            self.release_lease(id);
        }
        Ok(ProtocolResponse::Maintenance {
            entries_scanned: scanned,
            segments_compressed: 0,
            bytes_reclaimed: reclaimed,
            expired,
            evicted: 0,
        })
    }

    fn evict_for_space(&mut self, need: u64) -> Result<(), MemoryError> {
        let mut candidates: Vec<(u16, u64, MemoryId)> = self
            .entries
            .iter()
            .filter(|(_, e)| {
                e.is_live()
                    && e.pin_count == 0
                    && e.header.class == MemoryClass::Hot
                    && matches!(e.state, MemoryState::Sealed | MemoryState::Promoted)
            })
            .map(|(id, e)| (e.header.importance, e.header.last_access_ns, *id))
            .collect();
        candidates.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
        for (_, _, id) in candidates {
            let _ = self.spill_to_cold(id);
            ServiceStats::inc(&mut self.stats.evictions);
            if self.quota_snapshot().checked_add_ram(need, &self.quotas).is_ok() {
                return Ok(());
            }
        }
        Ok(())
    }

    /// Seal path into cold compressed segment (in-memory blob; daemon persists).
    pub fn spill_to_cold(&mut self, memory_id: MemoryId) -> Result<SegmentId, MemoryError> {
        let (session_id, payload, header, state) = {
            let e = self
                .entries
                .get(&memory_id)
                .ok_or(MemoryError::EntryNotFound)?;
            e.state.require(LifecycleOp::SpillToCold)?;
            if e.pin_count > 0 {
                return Err(MemoryError::InvalidRequest("entry pinned"));
            }
            (
                e.header.session_id,
                e.payload.clone(),
                {
                    let mut h = e.header.clone();
                    h.payload_len = e.payload.len() as u32;
                    h.class = MemoryClass::Cold;
                    h
                },
                MemoryState::Cold,
            )
        };
        let seg_id = self
            .ids
            .alloc_segment()
            .map_err(|_| MemoryError::InternalInvariantViolation("segment id"))?;
        let expires = header.expires_at_ns.unwrap_or(u64::MAX);
        let mut seg = Segment::new_open(seg_id, session_id, self.now_ns, expires);
        seg.append_record_full(&header, state, &payload, self.quotas.max_segment_size)?;
        seg.seal()?;
        match seg.compress_once(self.quotas.max_decompress_bytes) {
            Ok(()) => ServiceStats::inc(&mut self.stats.compression_successes),
            Err(e) => {
                ServiceStats::inc(&mut self.stats.compression_failures);
                return Err(e);
            }
        }
        let blob = seg.encode_spill_blob()?;
        let comp_len = blob.len() as u64;
        self.cold_blobs.insert(seg_id, blob);
        let len = payload.len() as u64;
        if let Some(e) = self.entries.get_mut(&memory_id) {
            match e.header.class {
                MemoryClass::Working => {
                    self.stats.working_bytes = self.stats.working_bytes.saturating_sub(len);
                }
                MemoryClass::Hot => {
                    self.stats.hot_bytes = self.stats.hot_bytes.saturating_sub(len);
                }
                MemoryClass::Cold => {}
            }
            if let Some(s) = self.sessions.get_mut(&e.header.session_id) {
                s.quota.ram_bytes = s.quota.ram_bytes.saturating_sub(len);
            }
            e.payload.clear();
            e.header.payload_len = 0;
            e.header.class = MemoryClass::Cold;
            e.state = MemoryState::Cold;
            e.segment_id = Some(seg_id);
        }
        self.stats.cold_compressed_bytes =
            self.stats.cold_compressed_bytes.saturating_add(comp_len);
        self.stats.segment_count = self.segments.len() as u64 + 1;
        self.segments.insert(seg_id, seg);
        Ok(seg_id)
    }

    /// Export cold blobs for the daemon to persist under /state.
    pub fn cold_blobs(&self) -> &BTreeMap<SegmentId, Vec<u8>> {
        &self.cold_blobs
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
        self.stats.logical_payload_bytes = self
            .stats
            .working_bytes
            .saturating_add(self.stats.hot_bytes);
        self.stats.logical_metadata_bytes =
            (self.entries.len() as u64).saturating_mul(128);
        self.stats.logical_total_bytes = self
            .stats
            .logical_payload_bytes
            .saturating_add(self.stats.logical_metadata_bytes)
            .saturating_add(self.stats.shared_memory_leased_bytes);
        self.stats.active_sessions = self.sessions.len() as u64;
        self.stats.entry_count = self.entries.values().filter(|e| e.is_live()).count() as u64;
        self.stats.segment_count = self.segments.len() as u64;
        self.stats.active_read_leases = self.leases.len() as u64;
    }

    pub fn accounted_within_limits(&self) -> bool {
        self.quota_snapshot().within_limits(&self.quotas)
    }

    pub fn live_entry_count(&self) -> usize {
        self.entries.values().filter(|e| e.is_live()).count()
    }
}

fn encode_promo(entry: &MemoryEntry, record_version: u16) -> Result<Vec<u8>, MemoryError> {
    let payload = &entry.payload;
    let checksum = crc32_ieee(payload);
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
    let exp = entry.header.expires_at_ns.unwrap_or(0);
    out.extend_from_slice(&exp.to_le_bytes());
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

fn promo_identical(existing: &[u8], expected: &[u8]) -> bool {
    if existing.len() < 40 || expected.len() < 40 {
        return existing == expected;
    }
    existing[0..18] == expected[0..18] && existing[24..32] == expected[24..32]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kinds::{SourceKind, TrustLevel};
    use crate::protocol::PROTOCOL_VERSION;

    fn prov() -> Provenance {
        Provenance::new(
            SourceKind::UserInput,
            None,
            1,
            "test",
            TrustLevel::Untrusted,
        )
    }

    #[test]
    fn native_engine_create_seal_promote() {
        let mut eng = NativeMemoryEngine::new();
        let admin = CallerIdentity::admin();
        let sid = match eng.handle(
            &admin,
            ProtocolRequest::CreateSession {
                protocol_version: PROTOCOL_VERSION,
            },
        ) {
            ProtocolResponse::SessionCreated { session_id } => session_id,
            _ => panic!(),
        };
        let mid = match eng.handle(
            &admin,
            ProtocolRequest::CreateEntry {
                protocol_version: PROTOCOL_VERSION,
                session_id: sid,
                class: MemoryClass::Working,
                kind: MemoryKind::Input,
                importance: 10,
                confidence: 10,
                ttl_ns: None,
                payload: b"hello-native".to_vec(),
                token_stream: None,
                provenance: prov(),
            },
        ) {
            ProtocolResponse::Created { memory_id, .. } => memory_id,
            ProtocolResponse::Error(e) => panic!("{e}"),
            _ => panic!(),
        };
        eng.handle(
            &admin,
            ProtocolRequest::SealEntry {
                protocol_version: PROTOCOL_VERSION,
                memory_id: mid,
                promote_class_to_hot: true,
            },
        );
        let r = eng.handle(
            &admin,
            ProtocolRequest::PromoteEntry {
                protocol_version: PROTOCOL_VERSION,
                request: PromoteRequest {
                    memory_id: mid,
                    namespace: String::from("owl.v1.shortterm"),
                    expected_record_version: 1,
                    retention_hint: String::new(),
                    reason: String::from("t"),
                    delete_local_after: false,
                },
            },
        );
        assert!(matches!(
            r,
            ProtocolResponse::Promoted(PromoteResult::Written { .. })
        ));
        // Idempotent
        let r2 = eng.handle(
            &admin,
            ProtocolRequest::PromoteEntry {
                protocol_version: PROTOCOL_VERSION,
                request: PromoteRequest {
                    memory_id: mid,
                    namespace: String::from("owl.v1.shortterm"),
                    expected_record_version: 1,
                    retention_hint: String::new(),
                    reason: String::from("t"),
                    delete_local_after: false,
                },
            },
        );
        assert!(matches!(
            r2,
            ProtocolResponse::Promoted(PromoteResult::AlreadyPresent { .. })
        ));
    }

    #[test]
    fn generation_restart_no_collision() {
        let mut a = NativeMemoryEngine::new();
        a.set_generation(1).unwrap();
        let admin = CallerIdentity::admin();
        let sid = match a.handle(
            &admin,
            ProtocolRequest::CreateSession {
                protocol_version: PROTOCOL_VERSION,
            },
        ) {
            ProtocolResponse::SessionCreated { session_id } => session_id,
            _ => panic!(),
        };
        let mid1 = match a.handle(
            &admin,
            ProtocolRequest::CreateEntry {
                protocol_version: PROTOCOL_VERSION,
                session_id: sid,
                class: MemoryClass::Working,
                kind: MemoryKind::Input,
                importance: 1,
                confidence: 1,
                ttl_ns: None,
                payload: b"a".to_vec(),
                token_stream: None,
                provenance: prov(),
            },
        ) {
            ProtocolResponse::Created { memory_id, .. } => memory_id,
            _ => panic!(),
        };
        // Simulate restart with generation 2
        let mut b = NativeMemoryEngine::new();
        b.set_generation(2).unwrap();
        b.note_seen_id(mid1.get());
        let sid2 = match b.handle(
            &admin,
            ProtocolRequest::CreateSession {
                protocol_version: PROTOCOL_VERSION,
            },
        ) {
            ProtocolResponse::SessionCreated { session_id } => session_id,
            _ => panic!(),
        };
        let mid2 = match b.handle(
            &admin,
            ProtocolRequest::CreateEntry {
                protocol_version: PROTOCOL_VERSION,
                session_id: sid2,
                class: MemoryClass::Working,
                kind: MemoryKind::Input,
                importance: 1,
                confidence: 1,
                ttl_ns: None,
                payload: b"b".to_vec(),
                token_stream: None,
                provenance: prov(),
            },
        ) {
            ProtocolResponse::Created { memory_id, .. } => memory_id,
            _ => panic!(),
        };
        assert_ne!(mid1.get(), mid2.get());
        assert_eq!(mid1.generation(), 1);
        assert_eq!(mid2.generation(), 2);
    }
}
