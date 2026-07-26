use std::io::{self, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use wiseowl_memorydb::database::{Database, DbCaller, FsStore};
use wiseowl_memorydb::health::HealthState;
use wiseowl_memorydb::owlql::parse_owlql;
use wiseowl_memorydb::protocol::{DbRequest, DbResponse};
use wiseowl_memorydb::{DbCapabilitySet, DbQuotaConfig};

fn main() {
    let socket = std::env::var("WISEOWL_MEMORYDB_SOCKET")
        .unwrap_or_else(|_| "/tmp/sunlight/wiseowl-memorydb.sock".to_string());
    let data_dir = std::env::var("WISEOWL_MEMORYDB_DIR")
        .unwrap_or_else(|_| "/tmp/sunlight/wiseowl-memorydb".to_string());

    if let Some(parent) = PathBuf::from(&socket).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::create_dir_all(&data_dir);
    if PathBuf::from(&socket).exists() {
        let _ = std::fs::remove_file(&socket);
    }

    let db = Database::<FsStore>::open_fs(&data_dir, DbQuotaConfig::default())
        .expect("open wiseowl-memorydb");
    let db = Arc::new(Mutex::new(db));

    let listener = UnixListener::bind(&socket).expect("bind socket");
    eprintln!("wiseowl-memorydb listening on {socket} (data {data_dir})");

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let db = Arc::clone(&db);
                thread::spawn(move || {
                    if let Err(e) = handle_client(stream, db) {
                        eprintln!("client error: {e}");
                    }
                });
            }
            Err(e) => eprintln!("accept error: {e}"),
        }
    }
}

fn handle_client(mut stream: UnixStream, db: Arc<Mutex<Database<FsStore>>>) -> io::Result<()> {
    let caller = DbCaller {
        caps: DbCapabilitySet::admin(),
        owner: 0,
        is_system: true,
    };

    loop {
        let req: DbRequest = match recv_msg(&mut stream)? {
            Some(r) => r,
            None => return Ok(()),
        };
        let response = {
            let mut guard = db.lock().expect("db mutex");
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

fn dispatch(
    db: &mut Database<FsStore>,
    caller: &DbCaller,
    req: DbRequest,
) -> DbResponse {
    match req {
        DbRequest::BeginTransaction => match db.begin_transaction(caller) {
            Ok(id) => DbResponse::TxId(id),
            Err(e) => DbResponse::from_error(e),
        },
        DbRequest::InsertRecord { tx_id, req } => match req.into_request() {
            Ok(r) => match db.insert_record(caller, tx_id, r) {
                Ok(id) => DbResponse::MemoryId(id),
                Err(e) => DbResponse::from_error(e),
            },
            Err(e) => DbResponse::from_error(e),
        },
        DbRequest::InsertRelationship { tx_id, rel } => {
            match db.insert_relationship(caller, tx_id, rel) {
                Ok(()) => DbResponse::Ok,
                Err(e) => DbResponse::from_error(e),
            }
        }
        DbRequest::Tombstone { tx_id, id } => match db.tombstone_record(caller, tx_id, id) {
            Ok(()) => DbResponse::Ok,
            Err(e) => DbResponse::from_error(e),
        },
        DbRequest::Commit { tx_id } => match db.commit_transaction(caller, tx_id) {
            Ok(seq) => DbResponse::Sequence(seq),
            Err(e) => DbResponse::from_error(e),
        },
        DbRequest::Abort { tx_id } => match db.abort_transaction(caller, tx_id) {
            Ok(()) => DbResponse::Ok,
            Err(e) => DbResponse::from_error(e),
        },
        DbRequest::Get { id, payload } => match db.get_record(caller, id, payload) {
            Ok(r) => DbResponse::Record(r),
            Err(e) => DbResponse::from_error(e),
        },
        DbRequest::History { id } => match db.list_revisions(caller, id) {
            Ok(r) => DbResponse::Revisions(r),
            Err(e) => DbResponse::from_error(e),
        },
        DbRequest::Source {
            source_id,
            offset,
            limit,
        } => match db.source_lookup(caller, source_id, offset as usize, limit as usize) {
            Ok((ids, more)) => DbResponse::SourcePage { ids, more },
            Err(e) => DbResponse::from_error(e),
        },
        DbRequest::Relationships { id } => match db.get_relationships(caller, id) {
            Ok(r) => DbResponse::Relationships(r),
            Err(e) => DbResponse::from_error(e),
        },
        DbRequest::Query { query } => match db.query(caller, query) {
            Ok(r) => DbResponse::Query(r),
            Err(e) => DbResponse::from_error(e),
        },
        DbRequest::OwlQl { text } => match parse_owlql(&text) {
            Ok(q) => match db.query(caller, q) {
                Ok(r) => DbResponse::Query(r),
                Err(e) => DbResponse::from_error(e),
            },
            Err(e) => DbResponse::from_error(e),
        },
        DbRequest::DeleteSource { source_id, batch } => {
            match db.delete_source(caller, source_id, batch) {
                Ok((deleted, more)) => DbResponse::SourceDelete { deleted, more },
                Err(e) => DbResponse::from_error(e),
            }
        }
        DbRequest::DeleteSourceDryRun { source_id } => {
            match db.delete_source_dry_run(caller, source_id) {
                Ok(n) => DbResponse::SourceCount(n),
                Err(e) => DbResponse::from_error(e),
            }
        }
        DbRequest::Checkpoint => match db.create_checkpoint(caller) {
            Ok(()) => DbResponse::Ok,
            Err(e) => DbResponse::from_error(e),
        },
        DbRequest::Compact => match db.run_compaction(caller) {
            Ok(reclaimed) => DbResponse::Compacted { reclaimed },
            Err(e) => DbResponse::from_error(e),
        },
        DbRequest::RebuildIndexes => match db.rebuild_indexes(caller) {
            Ok(()) => DbResponse::Ok,
            Err(e) => DbResponse::from_error(e),
        },
        DbRequest::Stats => DbResponse::Stats(db.stats()),
        DbRequest::Health => {
            let h = db.health();
            DbResponse::Health {
                ready: h.ready,
                state: match h.state {
                    HealthState::Starting => "starting".into(),
                    HealthState::Ready => "ready".into(),
                    HealthState::Degraded => "degraded".into(),
                    HealthState::Failed => "failed".into(),
                },
                reasons: h.reasons.clone(),
            }
        }
        DbRequest::Verify { max_segments } => match db.verify_bounded(max_segments) {
            Ok((ok, bad)) => DbResponse::Verify { ok, bad },
            Err(e) => DbResponse::from_error(e),
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
    if len > 4 * 1024 * 1024 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "frame too large"));
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf)?;
    let msg = bincode::deserialize(&buf)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    Ok(Some(msg))
}

fn send_msg<T: serde::Serialize>(stream: &mut UnixStream, msg: &T) -> io::Result<()> {
    let bytes = bincode::serialize(msg).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    stream.write_all(&(bytes.len() as u32).to_le_bytes())?;
    stream.write_all(&bytes)?;
    Ok(())
}
