


use alloc::string::String;
use alloc::vec::Vec;

use sunlight_ipc::{
    debug_log, endpoint_create, ipc_call, ipc_recv, ipc_reply_and_wait, monotonic_millis,
    nameserver_lookup, nameserver_register, process_is_alive, process_yield, shm_alloc, shm_free,
    shm_map, CapabilityToken, IpcMsg, SHM_PAGE,
};
use sunlight_libc as libc;

use wiseowl_memory::caller::CallerIdentity;
use wiseowl_memory::caps::CapabilitySet;
use wiseowl_memory::native_ipc::{
    MemoryIpcHeader, MemoryOp, INLINE_PAYLOAD_THRESHOLD, MEMORY_IPC_HEADER_LEN,
    NATIVE_PROTOCOL_VERSION,
};
use wiseowl_memory::protocol::{
    ListFilter, MaintenanceBudget, PromoteRequest, ProtocolRequest, ProtocolResponse,
    PROTOCOL_VERSION,
};
use wiseowl_memory::sunlightos_engine::{NativeKvBackend, NativeKvPut, NativeMemoryEngine};
use wiseowl_memory::{MemoryError, MemoryId, SessionId};

// sunlight-libc provides global-alloc with dynamic-heap.

macro_rules! serial_println {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let mut buf = heapless::String::<256>::new();
        let _ = write!(&mut buf, $($arg)*);
        debug_log(&buf);
    }};
}

// ---- Real sunlight-kv backend ----

struct SunlightKv;

impl SunlightKv {
    fn lookup() -> Option<CapabilityToken> {
        nameserver_lookup("sunlight-kv")
    }
}

impl NativeKvBackend for SunlightKv {
    fn put_if_absent(&mut self, key: &str, value: &[u8]) -> Result<NativeKvPut, MemoryError> {
        // Prefer compare via get first for conflict detection path.
        match self.get(key)? {
            Some(existing) => {
                if existing == value {
                    return Ok(NativeKvPut::AlreadyPresent);
                }
                // Different content — still report AlreadyPresent; engine verifies.
                return Ok(NativeKvPut::AlreadyPresent);
            }
            None => {}
        }
        if key.len() > 64 || value.len() > SHM_PAGE {
            return Err(MemoryError::KvPromotionRejected("key or value too large"));
        }
        let Some(cap) = Self::lookup() else {
            return Err(MemoryError::KvUnavailable);
        };
        let (ptr, token) = shm_alloc().map_err(|_| MemoryError::KvUnavailable)?;
        unsafe {
            core::ptr::copy_nonoverlapping(value.as_ptr(), ptr, value.len());
        }
        // Opcode 0x4B06 = PUT_SHM (see sunlight-kv / clipd)
        let mut msg = IpcMsg::with_label(0x4B06)
            .word(0, value.len() as u64)
            .with_cap(0, token);
        if !pack_str(&mut msg, 2, key) {
            let _ = shm_free(token);
            return Err(MemoryError::KvPromotionRejected("key pack"));
        }
        let reply = ipc_call(cap, msg);
        let _ = shm_free(token);
        if reply.label == 0x4BFF && reply.words[0] == 0 {
            Ok(NativeKvPut::Written)
        } else if reply.label == 0x4BFF {
            // Treat non-zero as present/error — try get for idempotency.
            match self.get(key) {
                Ok(Some(_)) => Ok(NativeKvPut::AlreadyPresent),
                _ => Err(MemoryError::KvUnavailable),
            }
        } else {
            Err(MemoryError::KvUnavailable)
        }
    }

    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, MemoryError> {
        if key.len() > 64 {
            return Err(MemoryError::KvPromotionRejected("key too large"));
        }
        let Some(cap) = Self::lookup() else {
            return Err(MemoryError::KvUnavailable);
        };
        let mut msg = IpcMsg::with_label(0x4B07);
        if !pack_str(&mut msg, 2, key) {
            return Err(MemoryError::KvPromotionRejected("key pack"));
        }
        let reply = ipc_call(cap, msg);
        if reply.label != 0x4B05 {
            // Not found or error
            if reply.label == 0x4BFF {
                return Ok(None);
            }
            return Err(MemoryError::KvUnavailable);
        }
        let len = (reply.words[0] as usize).min(SHM_PAGE);
        let token = reply.caps[0];
        if token == CapabilityToken::INVALID {
            return Ok(Some(Vec::new()));
        }
        let ptr = shm_map(token).map_err(|_| MemoryError::KvUnavailable)?;
        let value = unsafe { core::slice::from_raw_parts(ptr, len) }.to_vec();
        let _ = shm_free(token);
        Ok(Some(value))
    }
}

fn pack_str(msg: &mut IpcMsg, start_word: usize, text: &str) -> bool {
    let bytes = text.as_bytes();
    if bytes.len() > 16 {
        return false;
    }
    let mut words = [0u64; 2];
    for (i, b) in bytes.iter().enumerate() {
        let wi = i / 8;
        let bi = (i % 8) * 8;
        words[wi] |= (*b as u64) << bi;
    }
    msg.words[start_word] = words[0];
    msg.words[start_word + 1] = words[1];
    let need = (start_word + 2) as u32;
    if msg.word_count < need {
        msg.word_count = need;
    }
    true
}

// ---- Spill persistence under /state/wiseowl-memoryd ----

const STATE_DIR: &[u8] = b"/state/wiseowl-memoryd";
const GEN_PATH: &[u8] = b"/state/wiseowl-memoryd/generation.bin";

fn ensure_state_dir() {
    let _ = libc::mkdir(STATE_DIR, 0o700);
}

fn load_generation() -> u16 {
    match libc::open_with_flags(GEN_PATH, libc::O_RDONLY) {
        Ok(fd) => {
            let mut buf = [0u8; 2];
            let n = libc::read(fd, &mut buf).unwrap_or(0);
            let _ = libc::close(fd);
            if n >= 2 {
                u16::from_le_bytes([buf[0], buf[1]])
            } else {
                0
            }
        }
        Err(_) => 0,
    }
}

fn store_generation(gen: u16) {
    ensure_state_dir();
    let tmp = b"/state/wiseowl-memoryd/generation.bin.tmp";
    if let Ok(fd) = libc::create(tmp) {
        let bytes = gen.to_le_bytes();
        let _ = libc::write(fd, &bytes);
        let _ = libc::close(fd);
        // Best-effort rename via write final path.
        if let Ok(fd2) = libc::create(GEN_PATH) {
            let _ = libc::write(fd2, &bytes);
            let _ = libc::close(fd2);
        }
    }
}

fn persist_segment(seg_id: u64, blob: &[u8]) {
    ensure_state_dir();
    // Fixed short path: /state/wiseowl-memoryd/sXXXXXXXX.owls
    let mut path = *b"/state/wiseowl-memoryd/s00000000.owls\0";
    // Write hex of low 32 bits into name.
    let hex = b"0123456789abcdef";
    let v = seg_id as u32;
    for i in 0..8 {
        let nibble = ((v >> (28 - i * 4)) & 0xf) as usize;
        path[24 + i] = hex[nibble];
    }
    let path_bytes = &path[..path.len() - 1];
    if let Ok(fd) = libc::create(path_bytes) {
        let _ = libc::write(fd, blob);
        let _ = libc::close(fd);
    }
}

// ---- Client tracking (pid → client_id) for death cleanup ----

struct ClientSlot {
    pid: u64,
    client_id: wiseowl_memory::ClientId,
    active: bool,
}

const MAX_CLIENTS: usize = 32;

// ---- Main ----

#[no_mangle]
pub extern "C" fn _start() -> ! {
    serial_println!("[WISEOWL] starting wiseowl-memoryd");
    ensure_state_dir();

    let mut engine = NativeMemoryEngine::with_kv(SunlightKv);
    let prev = load_generation();
    let next = if prev == u16::MAX {
        serial_println!("[WISEOWL] generation exhausted");
        1
    } else {
        prev.saturating_add(1).max(1)
    };
    let _ = engine.set_generation(next);
    store_generation(next);
    serial_println!("[WISEOWL] generation={}", next);

    // Probe KV; degrade if missing but continue RAM-only.
    if SunlightKv::lookup().is_none() {
        engine.set_kv_degraded(true);
        serial_println!("[WISEOWL] degraded: KV unavailable");
    }

    let ep = endpoint_create();
    // Nameserver name must fit registry conventions used by lookup.
    nameserver_register("wiseowl-memoryd", ep);
    serial_println!("[WISEOWL] registered wiseowl-memoryd");

    let mut clients: [ClientSlot; MAX_CLIENTS] = core::array::from_fn(|_| ClientSlot {
        pid: 0,
        client_id: wiseowl_memory::ClientId::from_raw_unchecked(1),
        active: false,
    });

    let mut msg = ipc_recv(ep);
    loop {
        // Opportunistic client death sweep (bounded).
        for slot in clients.iter_mut() {
            if slot.active && slot.pid != 0 && !process_is_alive(slot.pid) {
                let _ = engine.handle(
                    &CallerIdentity::admin(),
                    ProtocolRequest::ClientDisconnect {
                        protocol_version: PROTOCOL_VERSION,
                        client_id: slot.client_id,
                    },
                );
                slot.active = false;
                slot.pid = 0;
            }
        }

        engine.set_now_ns(monotonic_millis().saturating_mul(1_000_000).max(1));
        let reply = handle_ipc(&mut engine, &mut clients, &msg);
        msg = ipc_reply_and_wait(ep, reply);
    }
}

fn handle_ipc(
    engine: &mut NativeMemoryEngine<SunlightKv>,
    clients: &mut [ClientSlot; MAX_CLIENTS],
    msg: &IpcMsg,
) -> IpcMsg {
    let op = msg.label as u16;
    match MemoryOp::from_u16(op) {
        Some(MemoryOp::TransportInfo) | Some(MemoryOp::GetStats) if op == MemoryOp::TransportInfo.as_u16() => {
            // Transport diagnostic
            return IpcMsg::with_label(MemoryOp::Reply.label())
                .word(0, NATIVE_PROTOCOL_VERSION as u64)
                .word(1, INLINE_PAYLOAD_THRESHOLD as u64)
                .word(2, engine.health() as u64)
                .word(3, engine.degraded_flags() as u64)
                .word(4, engine.generation() as u64);
        }
        Some(MemoryOp::TransportInfo) => {
            return IpcMsg::with_label(MemoryOp::Reply.label())
                .word(0, NATIVE_PROTOCOL_VERSION as u64)
                .word(1, INLINE_PAYLOAD_THRESHOLD as u64)
                .word(2, engine.health() as u64)
                .word(3, engine.degraded_flags() as u64)
                .word(4, engine.generation() as u64);
        }
        Some(MemoryOp::GetStats) => {
            let resp = engine.handle(
                &CallerIdentity::admin(),
                ProtocolRequest::GetStats {
                    protocol_version: PROTOCOL_VERSION,
                },
            );
            return encode_response(resp, msg.words[0]);
        }
        Some(MemoryOp::RegisterClient) => {
            let pid = msg.words[0];
            let name = String::from("client");
            let resp = engine.handle(
                &CallerIdentity::admin(),
                ProtocolRequest::RegisterClient {
                    protocol_version: PROTOCOL_VERSION,
                    name,
                },
            );
            if let ProtocolResponse::ClientRegistered { client_id } = &resp {
                if let Some(slot) = clients.iter_mut().find(|s| !s.active) {
                    slot.pid = pid;
                    slot.client_id = *client_id;
                    slot.active = true;
                }
            }
            return encode_response(resp, msg.words[1]);
        }
        Some(MemoryOp::CreateSession) => {
            let resp = engine.handle(
                &CallerIdentity::admin(),
                ProtocolRequest::CreateSession {
                    protocol_version: PROTOCOL_VERSION,
                },
            );
            return encode_response(resp, msg.words[0]);
        }
        Some(MemoryOp::CreateEntry) => {
            // words: session_id, class, kind, importance, body_len; payload via SHM cap0
            let session = match SessionId::from_raw(msg.words[0]) {
                Ok(s) => s,
                Err(_) => {
                    return error_reply(MemoryError::MalformedIdentifier("session").code(), msg.words[7])
                }
            };
            let class = match wiseowl_memory::MemoryClass::from_u8(msg.words[1] as u8) {
                Some(c) => c,
                None => return error_reply(MemoryError::InvalidRequest("class").code(), 0),
            };
            let kind = match wiseowl_memory::MemoryKind::from_u8(msg.words[2] as u8) {
                Some(k) => k,
                None => return error_reply(MemoryError::InvalidRequest("kind").code(), 0),
            };
            let importance = msg.words[3] as u16;
            let payload = match take_payload_shm(msg, msg.words[4] as u32) {
                Ok(p) => p,
                Err(e) => return error_reply(e.code(), 0),
            };
            let provenance = wiseowl_memory::Provenance::new(
                wiseowl_memory::SourceKind::LocalService,
                None,
                engine.now_ns(),
                "wiseowl-memoryd",
                wiseowl_memory::TrustLevel::Trusted,
            );
            let resp = engine.handle(
                &CallerIdentity::admin(),
                ProtocolRequest::CreateEntry {
                    protocol_version: PROTOCOL_VERSION,
                    session_id: session,
                    class,
                    kind,
                    importance,
                    confidence: 100,
                    ttl_ns: None,
                    payload,
                    token_stream: None,
                    provenance,
                },
            );
            return encode_response(resp, msg.words[7]);
        }
        Some(MemoryOp::SealEntry) => {
            let mid = match MemoryId::from_raw(msg.words[0]) {
                Ok(m) => m,
                Err(_) => return error_reply(3, 0),
            };
            let resp = engine.handle(
                &CallerIdentity::admin(),
                ProtocolRequest::SealEntry {
                    protocol_version: PROTOCOL_VERSION,
                    memory_id: mid,
                    promote_class_to_hot: msg.words[1] != 0,
                },
            );
            return encode_response(resp, 0);
        }
        Some(MemoryOp::ReadEntry) => {
            let mid = match MemoryId::from_raw(msg.words[0]) {
                Ok(m) => m,
                Err(_) => return error_reply(3, 0),
            };
            let include = msg.words[1] != 0;
            let mut caller = CallerIdentity::admin();
            if include {
                caller.caps = CapabilitySet::admin();
            }
            let resp = engine.handle(
                &caller,
                ProtocolRequest::ReadEntry {
                    protocol_version: PROTOCOL_VERSION,
                    memory_id: mid,
                    include_payload: include,
                },
            );
            return encode_response(resp, 0);
        }
        Some(MemoryOp::DeleteEntry) => {
            let mid = match MemoryId::from_raw(msg.words[0]) {
                Ok(m) => m,
                Err(_) => return error_reply(3, 0),
            };
            let resp = engine.handle(
                &CallerIdentity::admin(),
                ProtocolRequest::DeleteEntry {
                    protocol_version: PROTOCOL_VERSION,
                    memory_id: mid,
                },
            );
            return encode_response(resp, 0);
        }
        Some(MemoryOp::PromoteEntry) => {
            let mid = match MemoryId::from_raw(msg.words[0]) {
                Ok(m) => m,
                Err(_) => return error_reply(3, 0),
            };
            let resp = engine.handle(
                &CallerIdentity::admin(),
                ProtocolRequest::PromoteEntry {
                    protocol_version: PROTOCOL_VERSION,
                    request: PromoteRequest {
                        memory_id: mid,
                        namespace: String::from("owl.v1.shortterm"),
                        expected_record_version: 1,
                        retention_hint: String::new(),
                        reason: String::from("promote"),
                        delete_local_after: msg.words[1] != 0,
                    },
                },
            );
            // Persist any new cold blobs after promote path work.
            for (sid, blob) in engine.cold_blobs().iter() {
                persist_segment(sid.get(), blob);
            }
            return encode_response(resp, 0);
        }
        Some(MemoryOp::ListSessions) => {
            let resp = engine.handle(
                &CallerIdentity::admin(),
                ProtocolRequest::ListSessions {
                    protocol_version: PROTOCOL_VERSION,
                },
            );
            return encode_response(resp, 0);
        }
        Some(MemoryOp::ListEntries) => {
            let session = if msg.words[0] == 0 {
                None
            } else {
                SessionId::from_raw(msg.words[0]).ok()
            };
            let resp = engine.handle(
                &CallerIdentity::admin(),
                ProtocolRequest::ListEntries {
                    protocol_version: PROTOCOL_VERSION,
                    filter: ListFilter {
                        session_id: session,
                        class: None,
                        kind: None,
                        max_results: Some(msg.words[1] as u32),
                    },
                },
            );
            return encode_response(resp, 0);
        }
        Some(MemoryOp::RunMaintenance) => {
            let resp = engine.handle(
                &CallerIdentity::admin(),
                ProtocolRequest::RunMaintenance {
                    protocol_version: PROTOCOL_VERSION,
                    budget: MaintenanceBudget::default(),
                },
            );
            return encode_response(resp, 0);
        }
        Some(MemoryOp::ClientDisconnect) => {
            let cid = match wiseowl_memory::ClientId::from_raw(msg.words[0]) {
                Ok(c) => c,
                Err(_) => return error_reply(3, 0),
            };
            let resp = engine.handle(
                &CallerIdentity::admin(),
                ProtocolRequest::ClientDisconnect {
                    protocol_version: PROTOCOL_VERSION,
                    client_id: cid,
                },
            );
            return encode_response(resp, 0);
        }
        Some(MemoryOp::TouchEntry) => {
            let mid = match MemoryId::from_raw(msg.words[0]) {
                Ok(m) => m,
                Err(_) => return error_reply(3, 0),
            };
            let resp = engine.handle(
                &CallerIdentity::admin(),
                ProtocolRequest::TouchEntry {
                    protocol_version: PROTOCOL_VERSION,
                    memory_id: mid,
                },
            );
            return encode_response(resp, 0);
        }
        Some(MemoryOp::AppendEntry) => {
            let mid = match MemoryId::from_raw(msg.words[0]) {
                Ok(m) => m,
                Err(_) => return error_reply(3, 0),
            };
            let payload = match take_payload_shm(msg, msg.words[1] as u32) {
                Ok(p) => p,
                Err(e) => return error_reply(e.code(), 0),
            };
            let resp = engine.handle(
                &CallerIdentity::admin(),
                ProtocolRequest::AppendEntry {
                    protocol_version: PROTOCOL_VERSION,
                    memory_id: mid,
                    data: payload,
                },
            );
            return encode_response(resp, 0);
        }
        Some(MemoryOp::ReleaseLease) => {
            engine.release_lease(msg.words[0]);
            return IpcMsg::with_label(MemoryOp::Reply.label()).word(0, 0);
        }
        _ => {
            // Unsupported version / unknown op via header in SHM if present
            if let Ok(body) = take_payload_shm(msg, msg.words[0] as u32) {
                if body.len() >= MEMORY_IPC_HEADER_LEN {
                    if let Ok(h) = MemoryIpcHeader::decode(&body) {
                        if h.protocol_version != NATIVE_PROTOCOL_VERSION {
                            return error_reply(
                                MemoryError::UnsupportedProtocolVersion {
                                    got: h.protocol_version,
                                    want: NATIVE_PROTOCOL_VERSION,
                                }
                                .code(),
                                h.request_id,
                            );
                        }
                    }
                }
            }
            error_reply(MemoryError::InvalidRequest("unknown op").code(), 0)
        }
    }
}

fn take_payload_shm(msg: &IpcMsg, len: u32) -> Result<Vec<u8>, MemoryError> {
    if len == 0 {
        return Ok(Vec::new());
    }
    if len as usize > SHM_PAGE {
        return Err(MemoryError::PayloadTooLarge {
            size: len,
            max: SHM_PAGE as u32,
        });
    }
    let token = msg.caps[0];
    if token == CapabilityToken::INVALID {
        return Err(MemoryError::SharedMemoryValidationFailure("no shm"));
    }
    let ptr = shm_map(token).map_err(|_| {
        MemoryError::SharedMemoryValidationFailure("map failed")
    })?;
    let slice = unsafe { core::slice::from_raw_parts(ptr, len as usize) };
    let out = slice.to_vec();
    let _ = shm_free(token);
    Ok(out)
}

fn error_reply(code: u32, request_id: u64) -> IpcMsg {
    IpcMsg::with_label(MemoryOp::Error.label())
        .word(0, code as u64)
        .word(1, request_id)
}

fn encode_response(resp: ProtocolResponse, request_id: u64) -> IpcMsg {
    match resp {
        ProtocolResponse::Ok => IpcMsg::with_label(MemoryOp::Reply.label())
            .word(0, 0)
            .word(1, request_id),
        ProtocolResponse::Created {
            memory_id,
            session_id,
        } => IpcMsg::with_label(MemoryOp::Reply.label())
            .word(0, 1)
            .word(1, memory_id.get())
            .word(2, session_id.get())
            .word(3, request_id),
        ProtocolResponse::SessionCreated { session_id } => IpcMsg::with_label(MemoryOp::Reply.label())
            .word(0, 2)
            .word(1, session_id.get())
            .word(2, request_id),
        ProtocolResponse::ClientRegistered { client_id } => {
            IpcMsg::with_label(MemoryOp::Reply.label())
                .word(0, 3)
                .word(1, client_id.get())
                .word(2, request_id)
        }
        ProtocolResponse::Promoted(p) => {
            let code = match p {
                wiseowl_memory::PromoteResult::Written { .. } => 10u64,
                wiseowl_memory::PromoteResult::AlreadyPresent { .. }
                | wiseowl_memory::PromoteResult::AlreadyPresentAndIdentical { .. } => 11,
                wiseowl_memory::PromoteResult::Conflict { .. } => 12,
                wiseowl_memory::PromoteResult::Unavailable => 13,
                wiseowl_memory::PromoteResult::Rejected { .. } => 14,
            };
            IpcMsg::with_label(MemoryOp::Reply.label())
                .word(0, code)
                .word(1, request_id)
        }
        ProtocolResponse::Stats(s) => IpcMsg::with_label(MemoryOp::Reply.label())
            .word(0, 20)
            .word(1, s.entry_count)
            .word(2, s.working_bytes.saturating_add(s.hot_bytes))
            .word(3, s.cold_compressed_bytes)
            .word(4, s.active_sessions)
            .word(5, request_id),
        ProtocolResponse::Entry {
            header,
            state,
            payload,
            promoted,
            ..
        } => {
            let mut reply = IpcMsg::with_label(MemoryOp::Reply.label())
                .word(0, 30)
                .word(1, header.id.get())
                .word(2, header.session_id.get())
                .word(3, state as u64)
                .word(4, if promoted { 1 } else { 0 })
                .word(5, header.payload_len as u64);
            if let Some(p) = payload {
                if let Ok((ptr, token)) = shm_alloc() {
                    let n = p.len().min(SHM_PAGE);
                    unsafe {
                        core::ptr::copy_nonoverlapping(p.as_ptr(), ptr, n);
                    }
                    reply = reply.word(6, n as u64).with_cap(0, token);
                }
            }
            reply
        }
        ProtocolResponse::Listed { headers } => IpcMsg::with_label(MemoryOp::Reply.label())
            .word(0, 40)
            .word(1, headers.len() as u64)
            .word(2, request_id),
        ProtocolResponse::Sessions { ids } => IpcMsg::with_label(MemoryOp::Reply.label())
            .word(0, 41)
            .word(1, ids.len() as u64)
            .word(2, ids.first().map(|s| s.get()).unwrap_or(0))
            .word(3, request_id),
        ProtocolResponse::Maintenance {
            entries_scanned,
            bytes_reclaimed,
            expired,
            ..
        } => IpcMsg::with_label(MemoryOp::Reply.label())
            .word(0, 50)
            .word(1, entries_scanned as u64)
            .word(2, bytes_reclaimed)
            .word(3, expired as u64)
            .word(4, request_id),
        ProtocolResponse::Error(e) => error_reply(e.code(), request_id),
    }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    serial_println!("[WISEOWL] PANIC");
    let _ = info;
    loop {
        process_yield();
    }
}
