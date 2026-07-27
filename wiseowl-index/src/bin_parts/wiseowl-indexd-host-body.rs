// Host daemon body.
//
// Host path uses explicit `HostMemoryDbBackend` (in-process Database) for
// deterministic tests and local development. Native production uses
// `NativeMemoryDbClient` only — see wiseowl-indexd-native-body.rs.

use std::io::{self, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use wiseowl_index::memorydb_backend::HostMemoryDbBackend;
use wiseowl_index::protocol::{IndexRequest, IndexResponse};
use wiseowl_index::service::{IndexCaller, IndexerService};
use wiseowl_index::{IndexHealth, IndexerConfig};
use wiseowl_memorydb::database::{Database, DbCaller, FsStore, MemoryStore};
use wiseowl_memorydb::DbQuotaConfig;

type HostSvc = IndexerService<HostMemoryDbBackend<FsStore>>;

fn main() {
    let socket = std::env::var("WISEOWL_INDEX_SOCKET")
        .unwrap_or_else(|_| "/tmp/sunlight/wiseowl-index.sock".to_string());
    let state_dir = std::env::var("WISEOWL_INDEX_DIR")
        .unwrap_or_else(|_| "/tmp/sunlight/wiseowl-index".to_string());
    // Host-only test backend: in-process MemoryDB (explicit HostMemoryDbBackend).
    // Native production never embeds MemoryDB — see native body.
    let memdb_dir = std::env::var("WISEOWL_INDEX_MEMDB_DIR")
        .unwrap_or_else(|_| format!("{state_dir}/memorydb"));

    if let Some(parent) = PathBuf::from(&socket).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::create_dir_all(&state_dir);
    let _ = std::fs::create_dir_all(&memdb_dir);
    if PathBuf::from(&socket).exists() {
        let _ = std::fs::remove_file(&socket);
    }

    let db = Database::<FsStore>::open_fs(&memdb_dir, DbQuotaConfig::default())
        .expect("open host test memorydb backend");
    let mut svc = IndexerService::new_host(db, IndexerConfig::default());
    svc.backend.caller = DbCaller::admin();
    svc.refresh_memorydb_health();

    if let Ok(home) = std::env::var("HOME") {
        let caller = IndexCaller::user(1);
        let _ = svc.maybe_register_documents_root(&caller, &home);
    }

    let svc = Arc::new(Mutex::new(svc));
    let listener = UnixListener::bind(&socket).expect("bind socket");
    eprintln!("wiseowl-indexd listening on {socket} (state {state_dir})");
    eprintln!("endpoint wiseowl.index.v1 — Phase 3.5 (host backend: explicit HostMemoryDbBackend)");
    eprintln!("content digest: SHA-256 v1 | manifest: v2");

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let svc = Arc::clone(&svc);
                thread::spawn(move || {
                    if let Err(e) = handle_client(stream, svc) {
                        eprintln!("client error: {e}");
                    }
                });
            }
            Err(e) => eprintln!("accept error: {e}"),
        }
    }
}

fn handle_client(mut stream: UnixStream, svc: Arc<Mutex<HostSvc>>) -> io::Result<()> {
    let caller = IndexCaller::admin();
    loop {
        let req: IndexRequest = match recv_msg(&mut stream)? {
            Some(r) => r,
            None => return Ok(()),
        };
        let response = {
            let mut guard = svc.lock().expect("svc mutex");
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(1);
            guard.set_now_ns(now.max(1));
            dispatch(&mut guard, &caller, req)
        };
        send_msg(&mut stream, &response)?;
    }
}

fn dispatch(svc: &mut HostSvc, caller: &IndexCaller, req: IndexRequest) -> IndexResponse {
    match req {
        IndexRequest::RegisterRoot {
            path,
            owner: _,
            recursive,
            maximum_depth,
        } => match svc.register_root(caller, path, recursive, maximum_depth) {
            Ok(id) => IndexResponse::RootId(id),
            Err(e) => IndexResponse::from_error(e),
        },
        IndexRequest::RemoveRoot { root_id } => match svc.remove_root(caller, root_id) {
            Ok(()) => IndexResponse::Ok,
            Err(e) => IndexResponse::from_error(e),
        },
        IndexRequest::ListRoots => match svc.list_roots(caller) {
            Ok(r) => IndexResponse::Roots(r),
            Err(e) => IndexResponse::from_error(e),
        },
        IndexRequest::StartScan { root_id } => match svc.start_scan(caller, root_id) {
            Ok(()) => IndexResponse::ScanStarted,
            Err(e) => IndexResponse::from_error(e),
        },
        IndexRequest::GetScanStatus => IndexResponse::ScanStatus {
            scanning: svc.engine.scanning,
            last_scan_ns: svc.state.last_successful_scan_ns,
        },
        IndexRequest::ListSources { offset, limit } => match svc.list_sources(caller, offset, limit)
        {
            Ok((items, more)) => IndexResponse::Sources { items, more },
            Err(e) => IndexResponse::from_error(e),
        },
        IndexRequest::InspectSource { source_id } => match svc.inspect_source(caller, source_id) {
            Ok(s) => IndexResponse::Source(s),
            Err(e) => IndexResponse::from_error(e),
        },
        IndexRequest::RetrySource { source_id } | IndexRequest::ReindexSource { source_id } => {
            match svc.reindex_source(caller, source_id) {
                Ok(()) => IndexResponse::Ok,
                Err(e) => IndexResponse::from_error(e),
            }
        }
        IndexRequest::ForgetSource { source_id, dry_run } => {
            match svc.forget_source(caller, source_id, dry_run) {
                Ok((deleted, more)) => IndexResponse::Forget { deleted, more },
                Err(e) => IndexResponse::from_error(e),
            }
        }
        IndexRequest::TokenizeText { text } => match svc.tokenize_text(caller, &text) {
            Ok((tid, tver, tokens)) => IndexResponse::Tokens {
                tokenizer_id: tid,
                tokenizer_version: tver,
                tokens,
            },
            Err(e) => IndexResponse::from_error(e),
        },
        IndexRequest::SearchText { text, limit } => match svc.search_lexical(caller, &text, limit)
        {
            Ok(hits) => IndexResponse::Search {
                label: "lexical relevance (not intelligence or confidence)".into(),
                hits,
            },
            Err(e) => IndexResponse::from_error(e),
        },
        IndexRequest::GetStats => IndexResponse::Stats(svc.stats()),
        IndexRequest::GetHealth => {
            svc.refresh_memorydb_health();
            IndexResponse::Health(svc.health())
        }
        IndexRequest::GetTransport => IndexResponse::Transport(svc.transport_info()),
        IndexRequest::GetMemoryDb => match svc.memorydb_health() {
            Ok(h) => IndexResponse::MemoryDb {
                ready: h.ready,
                state: h.state,
                generation: h.database_generation,
            },
            Err(e) => IndexResponse::from_error(e),
        },
        IndexRequest::GetPending => IndexResponse::Pending {
            count: svc.pending_count(),
        },
        IndexRequest::Reconcile => match svc.reconcile(caller) {
            Ok(n) => IndexResponse::Reconciled { count: n },
            Err(e) => IndexResponse::from_error(e),
        },
        IndexRequest::GetDigest { source_id } => match svc.digest_info(caller, source_id) {
            Ok((d, rev, mv, _legacy_present)) => IndexResponse::Digest {
                algorithm: d.algorithm.as_str().into(),
                version: d.version,
                hex_abbrev: d.abbreviated_hex(),
                source_revision: rev,
                manifest_version: mv,
            },
            Err(e) => IndexResponse::from_error(e),
        },
    }
}

fn recv_msg<T: serde::de::DeserializeOwned>(stream: &mut UnixStream) -> io::Result<Option<T>> {
    let mut len_buf = [0u8; 4];
    match stream.read_exact(&mut len_buf) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_le_bytes(len_buf) as usize;
    if len > 16 * 1024 * 1024 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "message too large"));
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf)?;
    let msg = bincode::deserialize(&buf)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    Ok(Some(msg))
}

fn send_msg<T: serde::Serialize>(stream: &mut UnixStream, msg: &T) -> io::Result<()> {
    let buf = bincode::serialize(msg).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let len = (buf.len() as u32).to_le_bytes();
    stream.write_all(&len)?;
    stream.write_all(&buf)?;
    Ok(())
}

#[allow(dead_code)]
fn _types() {
    let _: Option<MemoryStore> = None;
    let _: Option<IndexHealth> = None;
}
