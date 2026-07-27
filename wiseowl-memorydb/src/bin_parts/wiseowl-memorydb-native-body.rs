// Native SunlightOS body for wiseowl-memorydb.

use alloc::string::String;
use alloc::vec::Vec;

use sunlight_ipc::{
    debug_log, endpoint_create, ipc_recv, ipc_reply_and_wait, nameserver_register, process_yield,
    shm_alloc, shm_free, shm_map, CapabilityToken, IpcMsg, SHM_PAGE,
};
use sunlight_libc as libc;

use wiseowl_memorydb::attributes::AttributeValue;
use wiseowl_memorydb::database::{Database, DbCaller, DurableStore, MemoryStore};
use wiseowl_memorydb::native_ipc::{
    MemoryDbIpcHeader, MemoryDbOp, INLINE_PAYLOAD_THRESHOLD, MEMORYDB_IPC_HEADER_LEN,
    NATIVE_PROTOCOL_VERSION,
};
use wiseowl_memorydb::{DbCapabilitySet, DbQuotaConfig, ENDPOINT_NAME};

macro_rules! serial_println {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let mut buf = heapless::String::<256>::new();
        let _ = write!(&mut buf, $($arg)*);
        debug_log(&buf);
    }};
}

const STATE_DIR: &[u8] = b"/state/wiseowl-memorydb";

/// Native durable store using sunlight-libc file ops under /state/wiseowl-memorydb.
struct NativeFsStore {
    mem: MemoryStore,
}

impl NativeFsStore {
    fn open() -> Self {
        let _ = libc::mkdir(STATE_DIR, 0o700);
        let mut mem = MemoryStore::default();
        let _ = mem.ensure_layout();
        let mut s = Self { mem };
        s.hydrate();
        s
    }

    fn hydrate(&mut self) {
        for (path, rel) in [
            (
                b"/state/wiseowl-memorydb/MANIFEST".as_slice(),
                "MANIFEST",
            ),
            (
                b"/state/wiseowl-memorydb/WAL/wal-000001".as_slice(),
                "WAL/wal-000001",
            ),
        ] {
            if let Ok(fd) = libc::open_with_flags(path, libc::O_RDONLY) {
                let mut buf = [0u8; 4096];
                let mut data = Vec::new();
                loop {
                    match libc::read(fd, &mut buf) {
                        Ok(0) => break,
                        Ok(n) => data.extend_from_slice(&buf[..n]),
                        Err(_) => break,
                    }
                    if data.len() > 2 * 1024 * 1024 {
                        break;
                    }
                }
                let _ = libc::close(fd);
                let _ = self.mem.write_file_atomic(rel, &data);
            }
        }
        // Native segment filenames are physical hash-derived names, while the
        // database recovery core discovers logical `SEGMENTS/data-*` names.
        // Rehydrate every bounded physical segment under a logical name; the
        // authoritative segment id is validated from its encoded header.
        let mut entries = [libc::DirEntry::zeroed(); 64];
        if let Ok(count) = libc::read_dir(b"/state/wiseowl-memorydb/SEGMENTS", &mut entries) {
            for (ordinal, entry) in entries.iter().take(count).enumerate() {
                if entry.file_type != libc::FT_FILE || entry.size > 2 * 1024 * 1024 {
                    continue;
                }
                let Ok(name) = core::str::from_utf8(entry.name_bytes()) else {
                    continue;
                };
                if !name.starts_with('s') || !name.ends_with(".owlseg") {
                    continue;
                }
                let physical = alloc::format!("/state/wiseowl-memorydb/SEGMENTS/{name}");
                let Ok(fd) = libc::open_with_flags(physical.as_bytes(), libc::O_RDONLY) else {
                    continue;
                };
                let mut data = Vec::new();
                let mut buf = [0u8; 4096];
                loop {
                    match libc::read(fd, &mut buf) {
                        Ok(0) => break,
                        Ok(n) if data.len().saturating_add(n) <= 2 * 1024 * 1024 => {
                            data.extend_from_slice(&buf[..n]);
                        }
                        _ => {
                            data.clear();
                            break;
                        }
                    }
                }
                let _ = libc::close(fd);
                if !data.is_empty() {
                    let logical = alloc::format!("SEGMENTS/data-{ordinal:06}.owlseg");
                    let _ = self.mem.write_file_atomic(&logical, &data);
                }
            }
        }
    }

    fn persist_rel(&self, rel: &str, data: &[u8]) {
        let well_known: Option<&[u8]> = if rel == "MANIFEST" {
            Some(b"/state/wiseowl-memorydb/MANIFEST")
        } else if rel == "WAL/wal-000001" {
            Some(b"/state/wiseowl-memorydb/WAL/wal-000001")
        } else if rel.starts_with("SEGMENTS/") {
            // Fixed path for segments by hash.
            None
        } else {
            None
        };
        if let Some(p) = well_known {
            let _ = libc::mkdir(b"/state/wiseowl-memorydb/WAL", 0o700);
            if let Ok(fd) = libc::create(p) {
                let _ = libc::write(fd, data);
                let _ = libc::close(fd);
            }
        }
        if rel.starts_with("SEGMENTS/") {
            let _ = libc::mkdir(b"/state/wiseowl-memorydb/SEGMENTS", 0o700);
            // /state/wiseowl-memorydb/SEGMENTS/sXXXXXXXX.owlseg
            let mut path = *b"/state/wiseowl-memorydb/SEGMENTS/s00000000.owlseg\0";
            let hash = wiseowl_memorydb::codec::fnv1a64(rel.as_bytes()) as u32;
            let hex = b"0123456789abcdef";
            for i in 0..8 {
                let nibble = ((hash >> (28 - i * 4)) & 0xf) as usize;
                // Byte 33 is the literal `s`; the eight hex digits begin at
                // byte 34. Overwriting byte 33 made physical segment names
                // undiscoverable during native restart recovery.
                path[34 + i] = hex[nibble];
            }
            let path_bytes = &path[..path.len() - 1];
            if let Ok(fd) = libc::create(path_bytes) {
                let _ = libc::write(fd, data);
                let _ = libc::close(fd);
            }
        }
    }
}

impl DurableStore for NativeFsStore {
    fn read_file(&self, rel: &str) -> Result<Option<Vec<u8>>, wiseowl_memorydb::DbError> {
        self.mem.read_file(rel)
    }

    fn write_file_atomic(
        &mut self,
        rel: &str,
        data: &[u8],
    ) -> Result<(), wiseowl_memorydb::DbError> {
        self.mem.write_file_atomic(rel, data)?;
        self.persist_rel(rel, data);
        Ok(())
    }

    fn append_file(&mut self, rel: &str, data: &[u8]) -> Result<(), wiseowl_memorydb::DbError> {
        self.mem.append_file(rel, data)?;
        if let Ok(Some(all)) = self.mem.read_file(rel) {
            self.persist_rel(rel, &all);
        }
        Ok(())
    }

    fn remove_file(&mut self, rel: &str) -> Result<(), wiseowl_memorydb::DbError> {
        self.mem.remove_file(rel)
    }

    fn list_prefix(
        &self,
        dir: &str,
        prefix: &str,
    ) -> Result<Vec<String>, wiseowl_memorydb::DbError> {
        self.mem.list_prefix(dir, prefix)
    }

    fn ensure_layout(&mut self) -> Result<(), wiseowl_memorydb::DbError> {
        let _ = libc::mkdir(STATE_DIR, 0o700);
        let _ = libc::mkdir(b"/state/wiseowl-memorydb/WAL", 0o700);
        let _ = libc::mkdir(b"/state/wiseowl-memorydb/SEGMENTS", 0o700);
        let _ = libc::mkdir(b"/state/wiseowl-memorydb/INDEX", 0o700);
        let _ = libc::mkdir(b"/state/wiseowl-memorydb/QUARANTINE", 0o700);
        let _ = libc::mkdir(b"/state/wiseowl-memorydb/TMP", 0o700);
        self.mem.ensure_layout()
    }
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    serial_println!("[WISEOWL-DB] starting wiseowl-memorydb");
    let store = NativeFsStore::open();
    let mut db = match Database::open_with_store(store, DbQuotaConfig::default()) {
        Ok(d) => d,
        Err(_) => {
            serial_println!("[WISEOWL-DB] open failed");
            loop {
                process_yield();
            }
        }
    };

    let ep = endpoint_create();
    if nameserver_register(ENDPOINT_NAME, ep) {
        serial_println!("[WISEOWL-DB] registered {}", ENDPOINT_NAME);
    } else {
        // Also try short process-style name.
        let _ = nameserver_register("wiseowl-memorydb", ep);
        serial_println!("[WISEOWL-DB] registered wiseowl-memorydb");
    }

    // Native indexer authority: no compaction, elevated trust, arbitrary
    // system-scope insertion, or administrative escape hatch.
    let caller = DbCaller {
        caps: DbCapabilitySet::default_client()
            .grant(wiseowl_memorydb::DbCapability::ReadPayload)
            .grant(wiseowl_memorydb::DbCapability::DeleteSource)
            .grant(wiseowl_memorydb::DbCapability::InspectStats),
        owner: 0,
        is_system: false,
    };

    // Block on ipc_recv / reply-and-wait (no permanent busy poll).
    let mut crash_during_next_insert = false;
    let mut msg = ipc_recv(ep);
    loop {
        let reply = handle_msg(&mut db, &caller, &msg, &mut crash_during_next_insert);
        msg = ipc_reply_and_wait(ep, reply);
    }
}

fn handle_msg(
    db: &mut Database<NativeFsStore>,
    caller: &DbCaller,
    msg: &IpcMsg,
    crash_during_next_insert: &mut bool,
) -> IpcMsg {
    let op = MemoryDbOp::from_u16(msg.label as u16);
    match op {
        Some(MemoryDbOp::GetHealth) => {
            let h = db.health();
            IpcMsg::with_label(MemoryDbOp::Reply as u64)
                .word(0, if h.ready { 1 } else { 0 })
                .word(1, h.state as u8 as u64)
                .word(2, NATIVE_PROTOCOL_VERSION as u64)
        }
        Some(MemoryDbOp::GetStats) => {
            let s = db.stats();
            IpcMsg::with_label(MemoryDbOp::Reply as u64)
                .word(0, s.database_generation)
                .word(1, s.last_committed_sequence)
                .word(2, s.record_count_active as u64)
                .word(3, s.wal_bytes)
                .word(4, s.segment_count as u64)
                .word(5, s.transaction_commits)
        }
        Some(MemoryDbOp::GenerationCensus) => {
            let source_filter = if msg.words[0] == 0 {
                None
            } else {
                wiseowl_memory::SourceId::from_raw(msg.words[0]).ok()
            };
            let max = msg.words[1].min(4096).max(1) as u32;
            let (global, _) = db.generation_census(source_filter, max);
            IpcMsg::with_label(MemoryDbOp::Reply as u64)
                .word(0, global.sources as u64)
                .word(1, global.active_document_generations)
                .word(2, global.superseded_document_generations)
                .word(3, global.sources_with_multiple_active_generations as u64)
                .word(4, global.duplicate_import_keys as u64)
                .word(5, global.orphan_chunks as u64)
        }
        Some(MemoryDbOp::VerifyGenerations) => {
            let v = db.verify_generations();
            IpcMsg::with_label(MemoryDbOp::Reply as u64)
                .word(0, if v.ok { 1 } else { 0 })
                .word(1, v.multi_active_sources as u64)
                .word(2, v.duplicate_import_keys as u64)
                .word(3, v.orphan_chunks as u64)
                .word(4, v.invalid_supersession_chains as u64)
                .word(5, v.census.active_document_generations)
        }
        Some(MemoryDbOp::CreateCheckpoint) => match db.create_checkpoint(caller) {
            Ok(()) => IpcMsg::with_label(MemoryDbOp::Reply as u64).word(0, 0),
            Err(_) => IpcMsg::with_label(MemoryDbOp::Error as u64).word(0, 1),
        },
        Some(MemoryDbOp::RunCompaction) => match db.run_compaction(caller) {
            Ok(n) => IpcMsg::with_label(MemoryDbOp::Reply as u64).word(0, n),
            Err(_) => IpcMsg::with_label(MemoryDbOp::Error as u64).word(0, 1),
        },
        Some(MemoryDbOp::BeginTransaction) => match db.begin_transaction(caller) {
            Ok(id) => IpcMsg::with_label(MemoryDbOp::Reply as u64).word(0, id),
            Err(_) => IpcMsg::with_label(MemoryDbOp::Error as u64).word(0, 1),
        },
        Some(MemoryDbOp::CommitTransaction) => {
            let tx = msg.words[0];
            match db.commit_transaction(caller, tx) {
                Ok(seq) => IpcMsg::with_label(MemoryDbOp::Reply as u64).word(0, seq),
                Err(_) => IpcMsg::with_label(MemoryDbOp::Error as u64).word(0, 1),
            }
        }
        Some(MemoryDbOp::AbortTransaction) => {
            let tx = msg.words[0];
            match db.abort_transaction(caller, tx) {
                Ok(()) => IpcMsg::with_label(MemoryDbOp::Reply as u64).word(0, 0),
                Err(_) => IpcMsg::with_label(MemoryDbOp::Error as u64).word(0, 1),
            }
        }
        Some(MemoryDbOp::GetRecord) => {
            let id_raw = msg.words[0];
            let want_payload = msg.words[1] != 0;
            let Ok(id) = wiseowl_memory::MemoryId::from_raw(id_raw) else {
                return IpcMsg::with_label(MemoryDbOp::Error as u64).word(0, 2);
            };
            match db.get_record(caller, id, want_payload) {
                Ok(rec) => {
                    let mut reply = IpcMsg::with_label(MemoryDbOp::Reply as u64)
                        .word(0, rec.id.get())
                        .word(1, rec.revision as u64)
                        .word(2, rec.payload_ref.length as u64)
                        .word(3, rec.confidence as u64);
                    if want_payload && !rec.payload.is_empty() {
                        if rec.payload.len() <= INLINE_PAYLOAD_THRESHOLD as usize {
                            if let Ok((ptr, token)) = shm_alloc() {
                                let n = rec.payload.len().min(SHM_PAGE);
                                unsafe {
                                    core::ptr::copy_nonoverlapping(rec.payload.as_ptr(), ptr, n);
                                }
                                reply = reply.word(4, n as u64).with_cap(0, token);
                            }
                        }
                    }
                    reply
                }
                Err(_) => IpcMsg::with_label(MemoryDbOp::Error as u64).word(0, 3),
            }
        }
        Some(MemoryDbOp::InsertRecord) => {
            // Owner-retained contract: client owns allocation; this service
            // maps, validates/copies, unmaps, and never frees owner storage.
            let tx = msg.words[0];
            let body_len = msg.words[1] as usize;
            if body_len == 0 || body_len > MAX_BODY {
                return IpcMsg::with_label(MemoryDbOp::Error as u64).word(0, 4);
            }
            let token = msg.caps[0];
            if token == CapabilityToken::INVALID {
                return IpcMsg::with_label(MemoryDbOp::Error as u64).word(0, 5);
            }
            let Ok(ptr) = shm_map(token) else {
                return IpcMsg::with_label(MemoryDbOp::Error as u64).word(0, 5);
            };
            #[cfg(feature = "phase375-test")]
            if *crash_during_next_insert {
                sunlight_ipc::ProcessExit::exit(75);
            }
            let body = unsafe { core::slice::from_raw_parts(ptr, body_len) };
            let quotas = wiseowl_memorydb::DbQuotaConfig::default();
            let decoded = wiseowl_memorydb::insert_wire::decode_insert_request(body, &quotas);
            let _ = shm_free(token); // unmap server view only
            let req = match decoded {
                Ok(req) => req,
                Err(_) => return IpcMsg::with_label(MemoryDbOp::Error as u64).word(0, 7),
            };
            match db.insert_record(caller, tx, req) {
                Ok(id) => IpcMsg::with_label(MemoryDbOp::Reply as u64).word(0, id.get()),
                Err(_) => IpcMsg::with_label(MemoryDbOp::Error as u64).word(0, 6),
            }
        }
        Some(MemoryDbOp::SourceLookup) => {
            let Ok(sid) = wiseowl_memory::SourceId::from_raw(msg.words[0]) else {
                return IpcMsg::with_label(MemoryDbOp::Error as u64).word(0, 2);
            };
            let offset = msg.words[1] as usize;
            let limit = (msg.words[2].min(64)) as usize;
            let token = msg.caps[0];
            if token == CapabilityToken::INVALID {
                return IpcMsg::with_label(MemoryDbOp::Error as u64).word(0, 5);
            }
            match db.source_lookup(caller, sid, offset, limit) {
                Ok((ids, more)) => {
                    let Ok(ptr) = shm_map(token) else {
                        return IpcMsg::with_label(MemoryDbOp::Error as u64).word(0, 5);
                    };
                    for (i, id) in ids.iter().enumerate() {
                        unsafe {
                            core::ptr::copy_nonoverlapping(
                                id.get().to_le_bytes().as_ptr(),
                                ptr.add(i * 8),
                                8,
                            );
                        }
                    }
                    let _ = shm_free(token);
                    IpcMsg::with_label(MemoryDbOp::Reply as u64)
                        .word(0, ids.len() as u64)
                        .word(2, if more { 1 } else { 0 })
                }
                Err(_) => IpcMsg::with_label(MemoryDbOp::Error as u64).word(0, 3),
            }
        }
        Some(MemoryDbOp::Query) => {
            let request_len = msg.words[0] as usize;
            let request_token = msg.caps[0];
            let result_token = msg.caps[1];
            if request_len == 0
                || request_len > SHM_PAGE
                || request_token == CapabilityToken::INVALID
                || result_token == CapabilityToken::INVALID
                || request_token == result_token
            {
                return IpcMsg::with_label(MemoryDbOp::Error as u64).word(0, 8);
            }
            let Ok(request_ptr) = shm_map(request_token) else {
                return IpcMsg::with_label(MemoryDbOp::Error as u64).word(0, 5);
            };
            let request = unsafe { core::slice::from_raw_parts(request_ptr, request_len) };
            let decoded = wiseowl_memorydb::native_ipc::decode_native_query(request);
            let _ = shm_free(request_token);
            let query = match decoded {
                Ok(query) => query,
                Err(_) => return IpcMsg::with_label(MemoryDbOp::Error as u64).word(0, 8),
            };
            let result = match db.query(caller, query) {
                Ok(result) => result,
                Err(wiseowl_memorydb::DbError::StaleCursor) => {
                    return IpcMsg::with_label(MemoryDbOp::Error as u64).word(0, 9)
                }
                Err(_) => return IpcMsg::with_label(MemoryDbOp::Error as u64).word(0, 3),
            };
            let encoded = match wiseowl_memorydb::native_ipc::encode_native_query_result(&result) {
                Ok(encoded) if encoded.len() <= SHM_PAGE => encoded,
                _ => return IpcMsg::with_label(MemoryDbOp::Error as u64).word(0, 8),
            };
            let Ok(result_ptr) = shm_map(result_token) else {
                return IpcMsg::with_label(MemoryDbOp::Error as u64).word(0, 5);
            };
            unsafe {
                core::ptr::copy_nonoverlapping(encoded.as_ptr(), result_ptr, encoded.len());
            }
            let _ = shm_free(result_token);
            IpcMsg::with_label(MemoryDbOp::Reply as u64).word(0, encoded.len() as u64)
        }
        Some(MemoryDbOp::ReconcileImport) => {
            let Ok(sid) = wiseowl_memory::SourceId::from_raw(msg.words[0]) else {
                return IpcMsg::with_label(MemoryDbOp::Error as u64).word(0, 2);
            };
            let revision = msg.words[1] as u32;
            let len = msg.words[2] as usize;
            let token = msg.caps[0];
            if revision == 0 || len != 64 || token == CapabilityToken::INVALID {
                return IpcMsg::with_label(MemoryDbOp::Error as u64).word(0, 8);
            }
            let Ok(ptr) = shm_map(token) else {
                return IpcMsg::with_label(MemoryDbOp::Error as u64).word(0, 5);
            };
            let bytes = unsafe { core::slice::from_raw_parts(ptr, len) };
            let key = core::str::from_utf8(bytes).ok().map(String::from);
            let _ = shm_free(token);
            let Some(key) = key else {
                return IpcMsg::with_label(MemoryDbOp::Error as u64).word(0, 8);
            };
            let (ids, _) = match db.source_lookup(caller, sid, 0, 64) {
                Ok(page) => page,
                Err(_) => return IpcMsg::with_label(MemoryDbOp::Error as u64).word(0, 3),
            };
            let mut found = None;
            let mut conflict = false;
            for id in ids {
                let Ok(rec) = db.get_record(caller, id, false) else { continue };
                let role = rec.attributes.get("record_role");
                if role != Some(&AttributeValue::Text(String::from("document"))) {
                    continue;
                }
                let rec_rev = match rec.attributes.get("source_revision") {
                    Some(AttributeValue::Unsigned(value)) => *value as u32,
                    _ => rec.revision,
                };
                match rec.attributes.get("import_key") {
                    Some(AttributeValue::Text(value)) if value == &key => found = Some((id.get(), rec_rev)),
                    Some(AttributeValue::Text(value)) if !value.is_empty() && rec_rev == revision => conflict = true,
                    _ => {}
                }
            }
            if let Some((id, rev)) = found {
                IpcMsg::with_label(MemoryDbOp::Reply as u64)
                    .word(0, 5)
                    .word(1, id)
                    .word(2, rev as u64)
            } else if conflict {
                IpcMsg::with_label(MemoryDbOp::Reply as u64)
                    .word(0, 4)
                    .word(2, revision as u64)
            } else {
                IpcMsg::with_label(MemoryDbOp::Reply as u64).word(0, 0)
            }
        }
        #[cfg(feature = "phase375-test")]
        Some(MemoryDbOp::TestArmShmCrash) => {
            *crash_during_next_insert = true;
            IpcMsg::with_label(MemoryDbOp::Reply as u64).word(0, 1)
        }
        Some(MemoryDbOp::DeleteSource) => {
            let Ok(sid) = wiseowl_memory::SourceId::from_raw(msg.words[0]) else {
                return IpcMsg::with_label(MemoryDbOp::Error as u64).word(0, 2);
            };
            let batch = msg.words[1].min(64) as u32;
            if msg.words[2] != 0 {
                match db.delete_source_dry_run(caller, sid) {
                    Ok(n) => IpcMsg::with_label(MemoryDbOp::Reply as u64).word(0, n as u64),
                    Err(_) => IpcMsg::with_label(MemoryDbOp::Error as u64).word(0, 3),
                }
            } else {
                match db.delete_source(caller, sid, batch) {
                    Ok((n, more)) => IpcMsg::with_label(MemoryDbOp::Reply as u64)
                        .word(0, n as u64)
                        .word(1, if more { 1 } else { 0 }),
                    Err(_) => IpcMsg::with_label(MemoryDbOp::Error as u64).word(0, 3),
                }
            }
        }
        Some(MemoryDbOp::ReleaseLease) => {
            let token = msg.caps[0];
            if token != CapabilityToken::INVALID {
                let _ = shm_free(token);
            }
            IpcMsg::with_label(MemoryDbOp::Reply as u64).word(0, 0)
        }
        _ => {
            let _ = (
                MemoryDbIpcHeader {
                    protocol_version: NATIVE_PROTOCOL_VERSION,
                    operation: 0,
                    flags: 0,
                    request_id: 0,
                    body_len: 0,
                    reserved: 0,
                }
                .encode(),
                MEMORYDB_IPC_HEADER_LEN,
            );
            IpcMsg::with_label(MemoryDbOp::Error as u64).word(0, 0xFF)
        }
    }
}

const MAX_BODY: usize = 64 * 1024;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    use core::fmt::Write;
    let mut buf = heapless::String::<128>::new();
    let _ = write!(&mut buf, "panic: {info}");
    debug_log(&buf);
    loop {
        process_yield();
    }
}
