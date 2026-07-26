use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::process::ExitCode;

use wiseowl_memory::{MemoryId, SourceId};
use wiseowl_memorydb::protocol::{DbRequest, DbResponse};

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        print_help();
        return ExitCode::FAILURE;
    }
    let socket = std::env::var("WISEOWL_MEMORYDB_SOCKET")
        .unwrap_or_else(|_| "/tmp/sunlight/wiseowl-memorydb.sock".to_string());

    let cmd = args.remove(0);
    let req = match build_request(&cmd, &args) {
        Ok(r) => r,
        Err(code) => return code,
    };

    match call(&socket, req) {
        Ok(resp) => {
            print_response(&resp);
            match resp {
                DbResponse::Error { .. } => ExitCode::FAILURE,
                _ => ExitCode::SUCCESS,
            }
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn build_request(cmd: &str, args: &[String]) -> Result<DbRequest, ExitCode> {
    match cmd {
        "status" | "health" => Ok(DbRequest::Health),
        "stats" => Ok(DbRequest::Stats),
        "get" => {
            let id = parse_id(args.first().map(|s| s.as_str()))?;
            let payload = args.iter().any(|a| a == "--payload");
            Ok(DbRequest::Get { id, payload })
        }
        "history" => {
            let id = parse_id(args.first().map(|s| s.as_str()))?;
            Ok(DbRequest::History { id })
        }
        "source" => {
            let sid = parse_source(args.first().map(|s| s.as_str()))?;
            Ok(DbRequest::Source {
                source_id: sid,
                offset: 0,
                limit: 50,
            })
        }
        "relationships" => {
            let id = parse_id(args.first().map(|s| s.as_str()))?;
            Ok(DbRequest::Relationships { id })
        }
        "query" => {
            if let Some(i) = args.iter().position(|a| a == "--owlql") {
                let text = args.get(i + 1).cloned().unwrap_or_default();
                Ok(DbRequest::OwlQl { text })
            } else {
                Ok(DbRequest::Query {
                    query: Default::default(),
                })
            }
        }
        "checkpoint" => Ok(DbRequest::Checkpoint),
        "compact" => Ok(DbRequest::Compact),
        "verify" => Ok(DbRequest::Verify { max_segments: 16 }),
        "help" | "-h" | "--help" => {
            print_help();
            Err(ExitCode::SUCCESS)
        }
        other => {
            eprintln!("unknown command: {other}");
            print_help();
            Err(ExitCode::FAILURE)
        }
    }
}

fn parse_id(s: Option<&str>) -> Result<MemoryId, ExitCode> {
    let Some(s) = s else {
        eprintln!("missing memory id");
        return Err(ExitCode::FAILURE);
    };
    let Ok(n) = s.parse::<u64>() else {
        eprintln!("invalid id");
        return Err(ExitCode::FAILURE);
    };
    MemoryId::from_raw(n).map_err(|_| {
        eprintln!("invalid id");
        ExitCode::FAILURE
    })
}

fn parse_source(s: Option<&str>) -> Result<SourceId, ExitCode> {
    let Some(s) = s else {
        eprintln!("missing source id");
        return Err(ExitCode::FAILURE);
    };
    let Ok(n) = s.parse::<u64>() else {
        eprintln!("invalid source id");
        return Err(ExitCode::FAILURE);
    };
    SourceId::from_raw(n).map_err(|_| {
        eprintln!("invalid source id");
        ExitCode::FAILURE
    })
}

fn call(socket: &str, req: DbRequest) -> Result<DbResponse, String> {
    let mut stream = UnixStream::connect(socket).map_err(|e| format!("connect: {e}"))?;
    let bytes = bincode::serialize(&req).map_err(|e| format!("encode: {e}"))?;
    stream
        .write_all(&(bytes.len() as u32).to_le_bytes())
        .map_err(|e| format!("write: {e}"))?;
    stream
        .write_all(&bytes)
        .map_err(|e| format!("write: {e}"))?;
    let mut len_buf = [0u8; 4];
    stream
        .read_exact(&mut len_buf)
        .map_err(|e| format!("read: {e}"))?;
    let len = u32::from_le_bytes(len_buf) as usize;
    let mut buf = vec![0u8; len];
    stream
        .read_exact(&mut buf)
        .map_err(|e| format!("read body: {e}"))?;
    bincode::deserialize(&buf).map_err(|e| format!("decode: {e}"))
}

fn print_response(resp: &DbResponse) {
    match resp {
        DbResponse::Ok => println!("ok"),
        DbResponse::TxId(id) => println!("tx_id={id}"),
        DbResponse::Sequence(s) => println!("sequence={s}"),
        DbResponse::MemoryId(id) => println!("memory_id={}", id.get()),
        DbResponse::Record(r) => {
            println!("{}", wiseowl_memorydb::record::record_summary(r));
            if !r.payload.is_empty() {
                println!("payload_len={}", r.payload.len());
            }
        }
        DbResponse::Revisions(v) => println!("revisions={v:?}"),
        DbResponse::SourcePage { ids, more } => {
            println!(
                "ids={} more={more}",
                ids.iter()
                    .map(|i| i.get().to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            );
        }
        DbResponse::Relationships(r) => {
            for rel in r {
                println!(
                    "{} -[{}]-> {} conf={}",
                    rel.source.get(),
                    rel.kind.as_str(),
                    rel.target.get(),
                    rel.confidence
                );
            }
        }
        DbResponse::Query(q) => {
            println!(
                "results={} degraded={} scanned={}",
                q.ids.len(),
                q.degraded,
                q.total_scanned
            );
            for id in &q.ids {
                println!("  {}", id.get());
            }
        }
        DbResponse::SourceDelete { deleted, more } => {
            println!("deleted={deleted} more={more}");
        }
        DbResponse::SourceCount(n) => println!("count={n}"),
        DbResponse::Stats(s) => {
            println!("database_generation={}", s.database_generation);
            println!("last_committed_sequence={}", s.last_committed_sequence);
            println!("wal_bytes={}", s.wal_bytes);
            println!("record_count_active={}", s.record_count_active);
            println!("record_count_tombstoned={}", s.record_count_tombstoned);
            println!("relationship_count={}", s.relationship_count);
            println!("segment_count={}", s.segment_count);
            println!("segment_bytes_compressed={}", s.segment_bytes_compressed);
            println!("primary_index_entries={}", s.primary_index_entries);
            println!("token_dictionary_entries={}", s.token_dictionary_entries);
            println!("transaction_commits={}", s.transaction_commits);
            println!("checkpoint_count={}", s.checkpoint_count);
            println!("compaction_count={}", s.compaction_count);
            println!("quarantined_files={}", s.quarantined_files);
        }
        DbResponse::Health {
            ready,
            state,
            reasons,
        } => {
            println!("ready={ready} state={state}");
            for r in reasons {
                println!("  reason: {r}");
            }
        }
        DbResponse::Verify { ok, bad } => println!("verify ok={ok} bad={bad}"),
        DbResponse::Compacted { reclaimed } => println!("reclaimed_bytes={reclaimed}"),
        DbResponse::Error { code, message } => eprintln!("error {code}: {message}"),
    }
}

fn print_help() {
    eprintln!(
        "\
wiseowl-memorydbctl — long-term memory database diagnostics

Usage:
  wiseowl-memorydbctl status|health
  wiseowl-memorydbctl stats
  wiseowl-memorydbctl get <memory-id> [--payload]
  wiseowl-memorydbctl history <memory-id>
  wiseowl-memorydbctl source <source-id>
  wiseowl-memorydbctl relationships <memory-id>
  wiseowl-memorydbctl query --owlql 'FIND ALL WHERE source_id = 42 LIMIT 20'
  wiseowl-memorydbctl checkpoint
  wiseowl-memorydbctl compact
  wiseowl-memorydbctl verify

Env:
  WISEOWL_MEMORYDB_SOCKET  (default /tmp/sunlight/wiseowl-memorydb.sock)
"
    );
}
