// Native SunlightOS body for wiseowl-memorydb.

use alloc::string::String;
use alloc::vec::Vec;

use sunlight_ipc::{
    debug_log, endpoint_create, ipc_recv, ipc_reply_and_wait, nameserver_register, process_yield,
    shm_alloc, shm_free, shm_map, CapabilityToken, IpcMsg, SHM_PAGE,
};
use sunlight_libc as libc;

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
                path[33 + i] = hex[nibble];
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

    let caller = DbCaller {
        caps: DbCapabilitySet::admin(),
        owner: 0,
        is_system: true,
    };

    // Block on ipc_recv / reply-and-wait (no permanent busy poll).
    let mut msg = ipc_recv(ep);
    loop {
        let reply = handle_msg(&mut db, &caller, &msg);
        msg = ipc_reply_and_wait(ep, reply);
    }
}

fn handle_msg(
    db: &mut Database<NativeFsStore>,
    caller: &DbCaller,
    msg: &IpcMsg,
) -> IpcMsg {
    let op = MemoryDbOp::from_u16(msg.label as u16);
    match op {
        Some(MemoryDbOp::GetHealth) => {
            let h = db.health();
            IpcMsg::with_label(MemoryDbOp::Reply as u64)
                .word(0, if h.ready { 1 } else { 0 })
                .word(1, h.state as u8 as u64)
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
            // Phase 3.5: full insert wire via SHM (read-only consume + free lease).
            let tx = msg.words[0];
            let body_len = msg.words[1] as usize;
            if body_len > SHM_PAGE || body_len > MAX_BODY {
                return IpcMsg::with_label(MemoryDbOp::Error as u64).word(0, 4);
            }
            let token = msg.caps[0];
            if token == CapabilityToken::INVALID {
                return IpcMsg::with_label(MemoryDbOp::Error as u64).word(0, 5);
            }
            let Ok(ptr) = shm_map(token) else {
                return IpcMsg::with_label(MemoryDbOp::Error as u64).word(0, 5);
            };
            let body = unsafe { core::slice::from_raw_parts(ptr, body_len) };
            let quotas = wiseowl_memorydb::DbQuotaConfig::default();
            let req = match wiseowl_memorydb::insert_wire::decode_insert_request(body, &quotas) {
                Ok(r) => r,
                Err(_) => {
                    // Fallback: raw payload insert (legacy clients).
                    wiseowl_memorydb::InsertRequest {
                        kind: wiseowl_memorydb::LongTermMemoryKind::ImportedRecord,
                        scope: wiseowl_memorydb::MemoryScope::User,
                        owner: msg.words[2],
                        payload: body.to_vec(),
                        provenance: wiseowl_memorydb::provenance::LongTermProvenance {
                            source_kind: wiseowl_memory::SourceKind::LocalService,
                            source_id: None,
                            producer_service: String::from("native"),
                            original_memory_ids: Vec::new(),
                            parent_lt_ids: Vec::new(),
                            insertion_time_ns: 0,
                            trust: wiseowl_memory::TrustLevel::Untrusted,
                            source_content_hash: None,
                            external_ref: None,
                            derivation: wiseowl_memorydb::provenance::DerivationKind::DirectImport,
                        },
                        confidence: 500,
                        importance: 100,
                        trust: wiseowl_memory::TrustLevel::Untrusted,
                        valid_from_ns: None,
                        valid_until_ns: None,
                        tokens: None,
                        attributes: Default::default(),
                        supersedes: None,
                        relationships: Vec::new(),
                        dedup: Default::default(),
                        id: None,
                        revision: 1,
                    }
                }
            };
            let _ = shm_free(token);
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
            match db.source_lookup(caller, sid, offset, limit) {
                Ok((ids, more)) => IpcMsg::with_label(MemoryDbOp::Reply as u64)
                    .word(0, ids.len() as u64)
                    .word(1, ids.first().map(|i| i.get()).unwrap_or(0))
                    .word(2, if more { 1 } else { 0 }),
                Err(_) => IpcMsg::with_label(MemoryDbOp::Error as u64).word(0, 3),
            }
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

const MAX_BODY: usize = 4096;

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
