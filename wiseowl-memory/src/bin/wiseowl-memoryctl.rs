//! wiseowl-memoryctl — diagnostic CLI for wiseowl-memoryd.
//!
//! Commands:
//!   status | stats | sessions | list --session <id> | inspect <memory-id> | maintenance

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::process;

use wiseowl_memory::protocol::{
    ListFilter, MaintenanceBudget, ProtocolRequest, ProtocolResponse, PROTOCOL_VERSION,
};
use wiseowl_memory::ids::{MemoryId, SessionId};
use wiseowl_memory::MemoryError;

fn usage() -> ! {
    eprintln!(
        "usage: wiseowl-memoryctl <command> [args]\n\
         \n\
         commands:\n\
           status\n\
           stats\n\
           sessions\n\
           list --session <id>\n\
           inspect <memory-id>\n\
           maintenance\n\
         \n\
         env: WISEOWL_MEMORY_SOCKET (default /tmp/sunlight/wiseowl-memory.sock)"
    );
    process::exit(2);
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        usage();
    }
    let cmd = args.remove(0);
    let socket = std::env::var("WISEOWL_MEMORY_SOCKET")
        .unwrap_or_else(|_| "/tmp/sunlight/wiseowl-memory.sock".to_string());

    let result = match cmd.as_str() {
        "status" => cmd_status(&socket),
        "stats" => cmd_stats(&socket),
        "sessions" => cmd_sessions(&socket),
        "list" => {
            let sid = parse_session_flag(&args).unwrap_or_else(|| usage());
            cmd_list(&socket, sid)
        }
        "inspect" => {
            if args.len() != 1 {
                usage();
            }
            let mid = parse_id(&args[0]).unwrap_or_else(|e| {
                eprintln!("wiseowl-memoryctl: {e}");
                process::exit(2);
            });
            cmd_inspect(&socket, mid)
        }
        "maintenance" => cmd_maintenance(&socket),
        "-h" | "--help" | "help" => usage(),
        other => {
            eprintln!("unknown command: {other}");
            usage();
        }
    };

    match result {
        Ok(out) => {
            print!("{out}");
            if !out.ends_with('\n') {
                println!();
            }
        }
        Err(e) => {
            eprintln!("wiseowl-memoryctl: {e}");
            process::exit(1);
        }
    }
}

fn parse_session_flag(args: &[String]) -> Option<SessionId> {
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--session" {
            let v = args.get(i + 1)?;
            return SessionId::from_raw(v.parse().ok()?).ok();
        }
        i += 1;
    }
    None
}

fn parse_id(s: &str) -> Result<MemoryId, MemoryError> {
    let n: u64 = s
        .parse()
        .map_err(|_| MemoryError::MalformedIdentifier("memory id"))?;
    MemoryId::from_raw(n).map_err(|_| MemoryError::MalformedIdentifier("memory id"))
}

fn connect(socket: &str) -> Result<UnixStream, String> {
    UnixStream::connect(socket).map_err(|e| format!("connect {socket}: {e}"))
}

fn call(socket: &str, req: ProtocolRequest) -> Result<ProtocolResponse, String> {
    let mut stream = connect(socket)?;
    send_msg(&mut stream, &req).map_err(|e| e.to_string())?;
    recv_msg(&mut stream).map_err(|e| e.to_string())
}

fn send_msg<T: serde::Serialize>(
    stream: &mut UnixStream,
    msg: &T,
) -> std::io::Result<()> {
    let bytes = bincode::serialize(msg)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    stream.write_all(&(bytes.len() as u32).to_le_bytes())?;
    stream.write_all(&bytes)?;
    Ok(())
}

fn recv_msg(stream: &mut UnixStream) -> std::io::Result<ProtocolResponse> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf)?;
    let len = u32::from_le_bytes(len_buf) as usize;
    if len > 1024 * 1024 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "frame too large",
        ));
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf)?;
    bincode::deserialize(&buf).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

fn cmd_status(socket: &str) -> Result<String, String> {
    match call(
        socket,
        ProtocolRequest::GetStats {
            protocol_version: PROTOCOL_VERSION,
        },
    )? {
        ProtocolResponse::Stats(s) => Ok(format!(
            "service: wiseowl-memoryd\n\
             status: ok\n\
             protocol: {}\n\
             entries: {}\n\
             sessions: {}\n\
             segments: {}\n\
             working_bytes: {}\n\
             hot_bytes: {}\n\
             cold_compressed_bytes: {}\n",
            PROTOCOL_VERSION,
            s.entry_count,
            s.active_sessions,
            s.segment_count,
            s.working_bytes,
            s.hot_bytes,
            s.cold_compressed_bytes
        )),
        ProtocolResponse::Error(e) => Err(e.to_string()),
        _ => Err("unexpected response".into()),
    }
}

fn cmd_stats(socket: &str) -> Result<String, String> {
    match call(
        socket,
        ProtocolRequest::GetStats {
            protocol_version: PROTOCOL_VERSION,
        },
    )? {
        ProtocolResponse::Stats(s) => Ok(format!(
            "working_bytes={}\n\
             hot_bytes={}\n\
             cold_compressed_bytes={}\n\
             cold_uncompressed_logical_bytes={}\n\
             entry_count={}\n\
             segment_count={}\n\
             active_sessions={}\n\
             creates={}\n\
             reads={}\n\
             seals={}\n\
             expirations={}\n\
             evictions={}\n\
             rejected_allocations={}\n\
             compression_successes={}\n\
             compression_failures={}\n\
             decompression_successes={}\n\
             decompression_failures={}\n\
             checksum_failures={}\n\
             kv_promotion_successes={}\n\
             kv_promotion_failures={}\n\
             shm_validation_failures={}\n\
             malformed_ipc_requests={}\n\
             maintenance_runs={}\n\
             client_disconnects={}\n\
             quarantined_spill_records={}\n",
            s.working_bytes,
            s.hot_bytes,
            s.cold_compressed_bytes,
            s.cold_uncompressed_logical_bytes,
            s.entry_count,
            s.segment_count,
            s.active_sessions,
            s.creates,
            s.reads,
            s.seals,
            s.expirations,
            s.evictions,
            s.rejected_allocations,
            s.compression_successes,
            s.compression_failures,
            s.decompression_successes,
            s.decompression_failures,
            s.checksum_failures,
            s.kv_promotion_successes,
            s.kv_promotion_failures,
            s.shm_validation_failures,
            s.malformed_ipc_requests,
            s.maintenance_runs,
            s.client_disconnects,
            s.quarantined_spill_records,
        )),
        ProtocolResponse::Error(e) => Err(e.to_string()),
        _ => Err("unexpected response".into()),
    }
}

fn cmd_sessions(socket: &str) -> Result<String, String> {
    match call(
        socket,
        ProtocolRequest::ListSessions {
            protocol_version: PROTOCOL_VERSION,
        },
    )? {
        ProtocolResponse::Sessions { ids } => {
            if ids.is_empty() {
                Ok("(no sessions)\n".into())
            } else {
                let mut out = String::new();
                for id in ids {
                    out.push_str(&format!("{id}\n"));
                }
                Ok(out)
            }
        }
        ProtocolResponse::Error(e) => Err(e.to_string()),
        _ => Err("unexpected response".into()),
    }
}

fn cmd_list(socket: &str, session: SessionId) -> Result<String, String> {
    match call(
        socket,
        ProtocolRequest::ListEntries {
            protocol_version: PROTOCOL_VERSION,
            filter: ListFilter {
                session_id: Some(session),
                class: None,
                kind: None,
                max_results: None,
            },
        },
    )? {
        ProtocolResponse::Listed { headers } => {
            if headers.is_empty() {
                return Ok("(no entries)\n".into());
            }
            let mut out = String::new();
            out.push_str("id\tsession\tclass\tkind\tsize\tstate\n");
            for (h, st) in headers {
                out.push_str(&format!(
                    "{}\t{}\t{}\t{}\t{}\t{}\n",
                    h.id,
                    h.session_id,
                    h.class.as_str(),
                    h.kind.as_str(),
                    h.payload_len,
                    st.as_str()
                ));
            }
            Ok(out)
        }
        ProtocolResponse::Error(e) => Err(e.to_string()),
        _ => Err("unexpected response".into()),
    }
}

fn cmd_inspect(socket: &str, memory_id: MemoryId) -> Result<String, String> {
    // Metadata only by default (include_payload = false).
    match call(
        socket,
        ProtocolRequest::ReadEntry {
            protocol_version: PROTOCOL_VERSION,
            memory_id,
            include_payload: false,
        },
    )? {
        ProtocolResponse::Entry {
            header,
            state,
            payload: _,
            promoted,
            segment_id,
        } => {
            let parents: Vec<String> = header
                .provenance
                .parents
                .iter()
                .map(|p| p.to_string())
                .collect();
            Ok(format!(
                "id: {}\n\
                 session: {}\n\
                 class: {}\n\
                 kind: {}\n\
                 size: {}\n\
                 created_at_ns: {}\n\
                 last_access_ns: {}\n\
                 expires_at_ns: {:?}\n\
                 importance: {}\n\
                 confidence: {}\n\
                 state: {}\n\
                 provenance.source_kind: {}\n\
                 provenance.producer: {}\n\
                 provenance.trust: {}\n\
                 provenance.parents: [{}]\n\
                 compression/segment: {:?}\n\
                 kv_promoted: {}\n",
                header.id,
                header.session_id,
                header.class.as_str(),
                header.kind.as_str(),
                header.payload_len,
                header.created_at_ns,
                header.last_access_ns,
                header.expires_at_ns,
                header.importance,
                header.confidence,
                state.as_str(),
                header.provenance.source_kind.as_str(),
                header.provenance.producer_service.as_str(),
                header.provenance.trust.as_str(),
                parents.join(", "),
                segment_id,
                promoted,
            ))
        }
        ProtocolResponse::Error(e) => Err(e.to_string()),
        _ => Err("unexpected response".into()),
    }
}

fn cmd_maintenance(socket: &str) -> Result<String, String> {
    match call(
        socket,
        ProtocolRequest::RunMaintenance {
            protocol_version: PROTOCOL_VERSION,
            budget: MaintenanceBudget::default(),
        },
    )? {
        ProtocolResponse::Maintenance {
            entries_scanned,
            segments_compressed,
            bytes_reclaimed,
            expired,
            evicted,
        } => Ok(format!(
            "entries_scanned={entries_scanned}\n\
             segments_compressed={segments_compressed}\n\
             bytes_reclaimed={bytes_reclaimed}\n\
             expired={expired}\n\
             evicted={evicted}\n"
        )),
        ProtocolResponse::Error(e) => Err(e.to_string()),
        _ => Err("unexpected response".into()),
    }
}
