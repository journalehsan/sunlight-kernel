// Native SunlightOS body for wiseowl-indexd (Phase 3.5).
//
// Architecture (production):
//   wiseowl-indexctl → wiseowl-indexd → wiseowl.memorydb.v1 (IPC + SHM)
//
// FORBIDDEN: embedded in-process MemoryDB bootstrap on the native path.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use sunlight_ipc::{
    debug_log, endpoint_create, ipc_recv, ipc_reply_and_wait, nameserver_register, process_yield,
    shm_alloc, shm_free, CapabilityToken, IpcMsg, SHM_PAGE,
};
use sunlight_libc as libc;

use wiseowl_index::health::{DegradedReason, HealthState};
use wiseowl_index::memorydb_client::NativeMemoryDbClient;
use wiseowl_index::native_ipc::{IndexOp, INLINE_PAYLOAD_THRESHOLD};
use wiseowl_index::service::{IndexCaller, IndexerService};
use wiseowl_index::tokenize::{NormalizedTextBuffer, RetrievalTokenizer, TokenSink, WiseOwlLexicalV1};
use wiseowl_index::{IndexQuotaConfig, IndexerConfig, ENDPOINT_NAME};

macro_rules! serial_println {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let mut buf = heapless::String::<256>::new();
        let _ = write!(&mut buf, $($arg)*);
        debug_log(&buf);
    }};
}

const STATE_DIR: &[u8] = b"/state/wiseowl-index";

type NativeSvc = IndexerService<NativeMemoryDbClient>;

#[no_mangle]
pub extern "C" fn _start() -> ! {
    serial_println!("[WISEOWL-INDEX] starting wiseowl-indexd (Phase 3.5 native)");
    let _ = libc::mkdir(STATE_DIR, 0o700);

    // Production: independent MemoryDB client only — NO in-process bootstrap.
    let client = NativeMemoryDbClient::new();
    let mut svc = IndexerService::with_backend(client, IndexerConfig::default());

    // 1. Load config/manifests (defaults; operational state under /state).
    // 2. Register own endpoint.
    let ep = endpoint_create();
    if nameserver_register(ENDPOINT_NAME, ep) {
        serial_println!("[WISEOWL-INDEX] registered {}", ENDPOINT_NAME);
    } else {
        let _ = nameserver_register("wiseowl-indexd", ep);
        serial_println!("[WISEOWL-INDEX] registered wiseowl-indexd");
    }

    // 3–5. Discover MemoryDB; Ready or Degraded:MemoryDbUnavailable.
    // Do not busy-loop or restart-spin; control plane still serves.
    match svc.backend.discover() {
        Ok(()) => {
            serial_println!("[WISEOWL-INDEX] discovered wiseowl.memorydb.v1");
            svc.refresh_memorydb_health();
        }
        Err(_) => {
            serial_println!("[WISEOWL-INDEX] MemoryDB unavailable — Degraded");
            svc.health
                .set_degraded(DegradedReason::MemoryDbUnavailable);
            svc.health.memorydb_connection = String::from("Unavailable");
        }
    }

    if svc.health.state == HealthState::Starting {
        svc.health.state = HealthState::Ready;
        svc.health.ready = true;
    }

    serial_println!(
        "[WISEOWL-INDEX] content digest SHA-256 v1; manifest v2; no embedded MemoryDB"
    );

    let caller = IndexCaller::admin();

    // Block on ipc_recv / reply-and-wait (negligible idle CPU).
    let mut msg = ipc_recv(ep);
    loop {
        // Bounded reconnect when degraded (not a tight loop).
        if svc.health.memorydb_connection != "Ready" {
            let _ = svc.try_reconnect_memorydb();
        }
        let reply = handle_msg(&mut svc, &caller, &msg);
        msg = ipc_reply_and_wait(ep, reply);
    }
}

fn handle_msg(svc: &mut NativeSvc, caller: &IndexCaller, msg: &IpcMsg) -> IpcMsg {
    let op = IndexOp::from_u16(msg.label as u16);
    match op {
        Some(IndexOp::GetHealth) => {
            svc.refresh_memorydb_health();
            let h = svc.health();
            IpcMsg::with_label(IndexOp::Reply as u64)
                .word(0, if h.ready { 1 } else { 0 })
                .word(1, h.state.as_u8() as u64)
                .word(2, h.pending_imports)
                .word(3, h.memorydb_generation)
                .word(
                    4,
                    if h.memorydb_connection == "Ready" { 1 } else { 0 },
                )
        }
        Some(IndexOp::GetTransport) => {
            let t = svc.transport_info();
            IpcMsg::with_label(IndexOp::Reply as u64)
                .word(0, t.memorydb_generation)
                .word(1, t.pending_imports)
                .word(2, t.manifest_format as u64)
                .word(
                    3,
                    if t.connection == "Ready" { 1 } else { 0 },
                )
        }
        Some(IndexOp::GetMemoryDb) => match svc.memorydb_health() {
            Ok(h) => IpcMsg::with_label(IndexOp::Reply as u64)
                .word(0, if h.ready { 1 } else { 0 })
                .word(1, h.database_generation),
            Err(_) => IpcMsg::with_label(IndexOp::Error as u64).word(0, 1),
        },
        Some(IndexOp::GetPending) => IpcMsg::with_label(IndexOp::Reply as u64)
            .word(0, svc.pending_count()),
        Some(IndexOp::Reconcile) => match svc.reconcile(caller) {
            Ok(n) => IpcMsg::with_label(IndexOp::Reply as u64).word(0, n as u64),
            Err(_) => IpcMsg::with_label(IndexOp::Error as u64).word(0, 1),
        },
        Some(IndexOp::GetDigest) => {
            let sid = msg.words[0];
            match svc.digest_info(caller, sid) {
                Ok((d, rev, mv)) => IpcMsg::with_label(IndexOp::Reply as u64)
                    .word(0, d.algorithm.as_u8() as u64)
                    .word(1, d.version as u64)
                    .word(2, rev as u64)
                    .word(3, mv as u64),
                Err(_) => IpcMsg::with_label(IndexOp::Error as u64).word(0, 1),
            }
        }
        Some(IndexOp::GetStats) => {
            let s = svc.stats();
            IpcMsg::with_label(IndexOp::Reply as u64)
                .word(0, s.configured_roots)
                .word(1, s.files_indexed)
                .word(2, s.files_unchanged)
                .word(3, s.strong_hash_files)
                .word(4, s.sources_tracked)
                .word(5, s.database_generations_created)
        }
        Some(IndexOp::GetScanStatus) => IpcMsg::with_label(IndexOp::Reply as u64)
            .word(0, if svc.engine.scanning { 1 } else { 0 })
            .word(1, svc.state.last_successful_scan_ns),
        Some(IndexOp::ListRoots) => {
            let n = svc.state.roots.len() as u64;
            IpcMsg::with_label(IndexOp::Reply as u64).word(0, n)
        }
        Some(IndexOp::StartScan) => {
            let root = if msg.words[0] == 0 {
                None
            } else {
                Some(msg.words[0])
            };
            match svc.start_scan(caller, root) {
                Ok(()) => IpcMsg::with_label(IndexOp::Reply as u64).word(0, 0),
                Err(_) => IpcMsg::with_label(IndexOp::Error as u64).word(0, 1),
            }
        }
        Some(IndexOp::TokenizeText) => {
            let mut text_buf = Vec::new();
            let token = msg.caps[0];
            if token != CapabilityToken::INVALID {
                let len = msg.words[0].min(INLINE_PAYLOAD_THRESHOLD as u64) as usize;
                if let Ok(ptr) = sunlight_ipc::shm_map(token) {
                    unsafe {
                        let slice = core::slice::from_raw_parts(ptr, len.min(SHM_PAGE));
                        text_buf.extend_from_slice(slice);
                    }
                }
            }
            let text = core::str::from_utf8(&text_buf).unwrap_or("");
            match svc.tokenize_text(caller, text) {
                Ok((tid, tver, tokens)) => {
                    let mut reply = IpcMsg::with_label(IndexOp::Reply as u64)
                        .word(0, tid as u64)
                        .word(1, tver as u64)
                        .word(2, tokens.len() as u64);
                    if !tokens.is_empty() {
                        if let Ok((ptr, token)) = shm_alloc() {
                            let mut off = 0usize;
                            for t in tokens.iter().take(SHM_PAGE / 8) {
                                if off + 8 > SHM_PAGE {
                                    break;
                                }
                                unsafe {
                                    core::ptr::copy_nonoverlapping(
                                        t.token_id.to_le_bytes().as_ptr(),
                                        ptr.add(off),
                                        8,
                                    );
                                }
                                off += 8;
                            }
                            reply = reply.word(3, off as u64).with_cap(0, token);
                        }
                    }
                    reply
                }
                Err(_) => IpcMsg::with_label(IndexOp::Error as u64).word(0, 1),
            }
        }
        Some(IndexOp::SearchText) => {
            let limit = msg.words[0].min(64) as u32;
            let mut text_buf = Vec::new();
            let token = msg.caps[0];
            if token != CapabilityToken::INVALID {
                let len = msg.words[1].min(4096) as usize;
                if let Ok(ptr) = sunlight_ipc::shm_map(token) {
                    unsafe {
                        let slice = core::slice::from_raw_parts(ptr, len.min(SHM_PAGE));
                        text_buf.extend_from_slice(slice);
                    }
                }
            }
            let text = core::str::from_utf8(&text_buf).unwrap_or("");
            match svc.search_lexical(caller, text, limit) {
                Ok(hits) => IpcMsg::with_label(IndexOp::Reply as u64)
                    .word(0, hits.len() as u64)
                    .word(1, hits.first().map(|h| h.memory_id).unwrap_or(0)),
                Err(_) => IpcMsg::with_label(IndexOp::Error as u64).word(0, 1),
            }
        }
        Some(IndexOp::RegisterRoot) => {
            let mut path_buf = Vec::new();
            let token = msg.caps[0];
            if token != CapabilityToken::INVALID {
                let len = msg.words[0].min(512) as usize;
                if let Ok(ptr) = sunlight_ipc::shm_map(token) {
                    unsafe {
                        let slice = core::slice::from_raw_parts(ptr, len.min(SHM_PAGE));
                        path_buf.extend_from_slice(slice);
                    }
                }
            }
            let path = core::str::from_utf8(&path_buf).unwrap_or("").to_string();
            let depth = msg.words[1].min(16) as u16;
            match svc.register_root(caller, path, true, depth) {
                Ok(id) => IpcMsg::with_label(IndexOp::Reply as u64).word(0, id),
                Err(_) => IpcMsg::with_label(IndexOp::Error as u64).word(0, 1),
            }
        }
        Some(IndexOp::ListSources) => {
            let offset = msg.words[0] as u32;
            let limit = msg.words[1].min(64) as u32;
            match svc.list_sources(caller, offset, limit) {
                Ok((items, more)) => IpcMsg::with_label(IndexOp::Reply as u64)
                    .word(0, items.len() as u64)
                    .word(1, if more { 1 } else { 0 }),
                Err(_) => IpcMsg::with_label(IndexOp::Error as u64).word(0, 1),
            }
        }
        Some(IndexOp::ForgetSource) => {
            let sid = msg.words[0];
            let dry = msg.words[1] != 0;
            match svc.forget_source(caller, sid, dry) {
                Ok((n, more)) => IpcMsg::with_label(IndexOp::Reply as u64)
                    .word(0, n as u64)
                    .word(1, if more { 1 } else { 0 }),
                Err(_) => IpcMsg::with_label(IndexOp::Error as u64).word(0, 1),
            }
        }
        Some(IndexOp::ReleaseLease) => {
            let token = msg.caps[0];
            if token != CapabilityToken::INVALID {
                let _ = shm_free(token);
            }
            IpcMsg::with_label(IndexOp::Reply as u64).word(0, 0)
        }
        Some(_) => IpcMsg::with_label(IndexOp::Error as u64).word(0, 3),
        None => IpcMsg::with_label(IndexOp::Error as u64).word(0, 2),
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    serial_println!("[WISEOWL-INDEX] panic");
    loop {
        process_yield();
    }
}

#[allow(dead_code)]
fn _touch_tokenizer() {
    let _ = WiseOwlLexicalV1;
    let _ = IndexQuotaConfig::default();
    let _: Option<CapabilityToken> = None;
    let _: Option<NormalizedTextBuffer> = None;
    let _: Option<TokenSink> = None;
}
