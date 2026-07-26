use std::env;
use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::process::ExitCode;

use wiseowl_index::protocol::{IndexRequest, IndexResponse};

fn main() -> ExitCode {
    let mut args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() {
        print_help();
        return ExitCode::FAILURE;
    }
    let cmd = args.remove(0);
    let socket = env::var("WISEOWL_INDEX_SOCKET")
        .unwrap_or_else(|_| "/tmp/sunlight/wiseowl-index.sock".to_string());

    let req = match cmd.as_str() {
        "status" | "health" => IndexRequest::GetHealth,
        "stats" => IndexRequest::GetStats,
        "roots" => IndexRequest::ListRoots,
        "add-root" => {
            let path = args.first().cloned().unwrap_or_default();
            if path.is_empty() {
                eprintln!("usage: wiseowl-indexctl add-root <path>");
                return ExitCode::FAILURE;
            }
            IndexRequest::RegisterRoot {
                path,
                owner: 1,
                recursive: true,
                maximum_depth: 12,
            }
        }
        "remove-root" => {
            let id: u64 = args
                .first()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            IndexRequest::RemoveRoot { root_id: id }
        }
        "scan" => {
            let mut root_id = None;
            let mut i = 0;
            while i < args.len() {
                if args[i] == "--root" {
                    if let Some(v) = args.get(i + 1) {
                        root_id = v.parse().ok();
                        i += 2;
                        continue;
                    }
                }
                i += 1;
            }
            IndexRequest::StartScan { root_id }
        }
        "sources" => IndexRequest::ListSources {
            offset: 0,
            limit: 64,
        },
        "inspect" => {
            let id: u64 = args
                .first()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            IndexRequest::InspectSource { source_id: id }
        }
        "retry" => {
            let id: u64 = args
                .first()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            IndexRequest::RetrySource { source_id: id }
        }
        "reindex" => {
            let id: u64 = args
                .first()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            IndexRequest::ReindexSource { source_id: id }
        }
        "forget" => {
            let id: u64 = args
                .iter()
                .find(|a| !a.starts_with('-'))
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            let dry_run = args.iter().any(|a| a == "--dry-run");
            IndexRequest::ForgetSource {
                source_id: id,
                dry_run,
            }
        }
        "tokenize" => {
            let text = args.join(" ");
            if text.is_empty() {
                eprintln!("usage: wiseowl-indexctl tokenize <text>");
                return ExitCode::FAILURE;
            }
            IndexRequest::TokenizeText { text }
        }
        "search" => {
            let text = args.join(" ");
            if text.is_empty() {
                eprintln!("usage: wiseowl-indexctl search <text>");
                return ExitCode::FAILURE;
            }
            IndexRequest::SearchText { text, limit: 20 }
        }
        "transport" => IndexRequest::GetTransport,
        "memorydb" => IndexRequest::GetMemoryDb,
        "pending" => IndexRequest::GetPending,
        "reconcile" => IndexRequest::Reconcile,
        "digest" => {
            let id: u64 = args
                .first()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            IndexRequest::GetDigest { source_id: id }
        }
        "help" | "-h" | "--help" => {
            print_help();
            return ExitCode::SUCCESS;
        }
        other => {
            eprintln!("unknown command: {other}");
            print_help();
            return ExitCode::FAILURE;
        }
    };

    match call(&socket, req) {
        Ok(resp) => {
            print_response(&resp);
            match resp {
                IndexResponse::Error { .. } => ExitCode::FAILURE,
                _ => ExitCode::SUCCESS,
            }
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn print_help() {
    eprintln!(
        "wiseowl-indexctl — Wise Owl Phase 3.5 document indexer CLI\n\
         \n\
         Commands:\n\
           status | health\n\
           stats\n\
           transport\n\
           memorydb\n\
           pending\n\
           reconcile\n\
           digest <source-id>\n\
           roots\n\
           add-root <path>\n\
           remove-root <root-id>\n\
           scan [--root <id>]\n\
           sources\n\
           inspect <source-id>\n\
           retry <source-id>\n\
           reindex <source-id>\n\
           forget <source-id> [--dry-run]\n\
           tokenize <text>\n\
           search <text>\n\
         \n\
         Search is lexical relevance only (not intelligence, not an answer).\n\
         Content identity: SHA-256 strong digest (FNV is prefilter only).\n\
         inspect/digest do not display document payloads.\n\
         Env: WISEOWL_INDEX_SOCKET (default /tmp/sunlight/wiseowl-index.sock)"
    );
}

fn call(socket: &str, req: IndexRequest) -> io::Result<IndexResponse> {
    let mut stream = UnixStream::connect(socket)?;
    send_msg(&mut stream, &req)?;
    recv_msg(&mut stream)?.ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "no response"))
}

fn print_response(resp: &IndexResponse) {
    match resp {
        IndexResponse::Ok => println!("ok"),
        IndexResponse::RootId(id) => println!("root_id={id}"),
        IndexResponse::Roots(roots) => {
            for r in roots {
                println!(
                    "root_id={} path={} owner={} enabled={} available={}",
                    r.root_id, r.path, r.owner, r.enabled, r.available
                );
            }
        }
        IndexResponse::ScanStarted => println!("scan started"),
        IndexResponse::ScanStatus {
            scanning,
            last_scan_ns,
        } => println!("scanning={scanning} last_scan_ns={last_scan_ns}"),
        IndexResponse::Sources { items, more } => {
            for s in items {
                println!(
                    "source_id={} root={} state={} chunks={} path={}",
                    s.source_id, s.root_id, s.state, s.chunk_count, s.relative_path
                );
            }
            if *more {
                println!("… more");
            }
        }
        IndexResponse::Source(m) => {
            // No payload text.
            println!(
                "source_id={} path={} state={} digest={} chunks={} parser={}.{} tokenizer={}.{} revision={} manifest_v={}",
                m.source_id.get(),
                m.relative_path,
                m.state.as_str(),
                m.content_digest,
                m.chunk_count,
                m.parser_id,
                m.parser_version,
                m.tokenizer_id,
                m.tokenizer_version,
                m.source_revision,
                m.manifest_version
            );
        }
        IndexResponse::Forget { deleted, more } => {
            println!("deleted={deleted} more={more}");
        }
        IndexResponse::Tokens {
            tokenizer_id,
            tokenizer_version,
            tokens,
        } => {
            println!("tokenizer={tokenizer_id} version={tokenizer_version}");
            for t in tokens {
                println!("  id={} freq={} token={}", t.token_id, t.frequency, t.canonical);
            }
        }
        IndexResponse::Search { label, hits } => {
            println!("search_mode={label}");
            for h in hits {
                println!(
                    "  memory_id={} source_id={:?} lexical_score={} preview={}",
                    h.memory_id, h.source_id, h.lexical_score, h.preview
                );
            }
        }
        IndexResponse::Stats(s) => {
            println!(
                "roots={} available={} indexed={} unchanged={} reparsed={} retokenized={} strong_hash_files={} generations={} failed={} tokens_emitted={} collisions={} pending={}",
                s.configured_roots,
                s.available_roots,
                s.files_indexed,
                s.files_unchanged,
                s.files_reparsed,
                s.files_retokenized,
                s.strong_hash_files,
                s.database_generations_created,
                s.files_failed,
                s.tokens_emitted,
                s.token_collisions_detected,
                s.pending_imports
            );
        }
        IndexResponse::Health(h) => {
            println!(
                "ready={} state={} memorydb={} gen={} digest={} manifest_v={} pending={} reasons={:?}",
                h.ready,
                h.state.as_str(),
                h.memorydb_connection,
                h.memorydb_generation,
                h.content_digest_label,
                h.manifest_format,
                h.pending_imports,
                h.reasons
            );
        }
        IndexResponse::Transport(t) => {
            println!("Indexer endpoint: {}", t.indexer_endpoint);
            println!("MemoryDB endpoint: {}", t.memorydb_endpoint);
            println!("MemoryDB generation: {}", t.memorydb_generation);
            println!("Connection: {}", t.connection);
            println!("IPC protocol: {}", t.ipc_protocol);
            println!("SHM: {}", t.shm);
            println!("Content digest: {}", t.content_digest);
            println!("Manifest format: v{}", t.manifest_format);
            println!("Pending imports: {}", t.pending_imports);
        }
        IndexResponse::MemoryDb {
            ready,
            state,
            generation,
        } => {
            println!("memorydb ready={ready} state={state} generation={generation}");
        }
        IndexResponse::Pending { count } => println!("pending_imports={count}"),
        IndexResponse::Reconciled { count } => println!("reconciled={count}"),
        IndexResponse::Digest {
            algorithm,
            version,
            hex_abbrev,
            source_revision,
            manifest_version,
        } => {
            println!(
                "algorithm={algorithm} version={version} digest={hex_abbrev}… revision={source_revision} manifest_v={manifest_version}"
            );
        }
        IndexResponse::Error { code, message } => {
            eprintln!("error {code}: {message}");
        }
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
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf)?;
    let msg = bincode::deserialize(&buf)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    Ok(Some(msg))
}

fn send_msg<T: serde::Serialize>(stream: &mut UnixStream, msg: &T) -> io::Result<()> {
    let buf = bincode::serialize(msg).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    stream.write_all(&(buf.len() as u32).to_le_bytes())?;
    stream.write_all(&buf)?;
    Ok(())
}
