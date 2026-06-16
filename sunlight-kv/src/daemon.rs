//! Daemon main loop and IPC handling for sunlight-kv.
//!
//! Transport: Unix domain socket with length-prefixed bincode frames.
//!   Frame: u32 little-endian length || bincode(Request) / bincode(Response)
//!
//! Caller identity is obtained from SO_PEERCRED on each accepted connection.
//! "root" (uid 0) bypasses all ACL checks.

use std::io::{self, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::io::AsRawFd;
use std::sync::{Arc, Mutex};
use std::thread;

use log::{error, info, warn};

use crate::ipc::{Request, Response};
use crate::storage::{StorageEngine, StorageError};

/// Configuration for the running daemon.
#[derive(Clone, Debug)]
pub struct DaemonConfig {
    pub store_path: std::path::PathBuf,
    pub socket_path: std::path::PathBuf,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        let store = std::env::var("SUNLIGHT_KV_STORE")
            .unwrap_or_else(|_| "/tmp/sunlight/kv.store".to_string());
        let sock = std::env::var("SUNLIGHT_KV_SOCKET")
            .unwrap_or_else(|_| "/tmp/sunlight/kv.sock".to_string());
        Self {
            store_path: store.into(),
            socket_path: sock.into(),
        }
    }
}

/// Errors that can occur while running the daemon.
#[derive(Debug, thiserror::Error)]
pub enum DaemonError {
    #[error("io: {0}")]
    Io(#[from] io::Error),

    #[error("storage: {0}")]
    Storage(#[from] StorageError),
}

/// Start the daemon and block forever serving requests.
/// Creates parent directories for both store and socket as needed.
pub fn run_daemon(cfg: DaemonConfig) -> Result<(), DaemonError> {
    // Prepare directories.
    if let Some(p) = cfg.store_path.parent() {
        let _ = std::fs::create_dir_all(p);
    }
    if let Some(p) = cfg.socket_path.parent() {
        let _ = std::fs::create_dir_all(p);
    }

    // Remove stale socket if present.
    if cfg.socket_path.exists() {
        let _ = std::fs::remove_file(&cfg.socket_path);
    }

    let listener = UnixListener::bind(&cfg.socket_path)?;
    info!("sunlight-kv listening on {}", cfg.socket_path.display());
    info!("store file: {}", cfg.store_path.display());

    // Open storage engine (does full recovery).
    let storage = StorageEngine::open(&cfg.store_path)?;
    let storage = Arc::new(Mutex::new(storage));

    // Accept loop. Each client gets its own thread so one slow client cannot
    // starve others. Storage operations are serialized via the Mutex.
    for incoming in listener.incoming() {
        match incoming {
            Ok(stream) => {
                let st = Arc::clone(&storage);
                let sock_path = cfg.socket_path.clone();
                thread::spawn(move || {
                    if let Err(e) = handle_client(stream, st) {
                        warn!("client handler error on {}: {}", sock_path.display(), e);
                    }
                });
            }
            Err(e) => {
                error!("accept error: {}", e);
            }
        }
    }

    Ok(())
}

/// Handle a single client connection: loop reading requests until EOF.
fn handle_client(
    mut stream: UnixStream,
    storage: Arc<Mutex<StorageEngine>>,
) -> io::Result<()> {
    let caller = resolve_caller_identity(&stream);

    loop {
        let request = match recv_request(&mut stream) {
            Ok(Some(r)) => r,
            Ok(None) => {
                // Client closed cleanly.
                return Ok(());
            }
            Err(e) => {
                // Malformed frame or read error: reply ERROR then close.
                let _ = send_response(&mut stream, &Response::ERROR(e.to_string()));
                return Err(e);
            }
        };

        let response = {
            let mut guard = storage.lock().expect("storage mutex poisoned");
            dispatch(&mut *guard, &caller, request)
        };

        if let Err(e) = send_response(&mut stream, &response) {
            return Err(e);
        }
    }
}

/// Map a Request + caller into a Response using the storage engine.
fn dispatch(storage: &mut StorageEngine, caller: &str, req: Request) -> Response {
    match req {
        Request::KV_PUT { key, value } => {
            match storage.put(&key, &value, caller) {
                Ok(()) => {
                    info!("PUT ok key={} caller={}", key, caller);
                    Response::OK
                }
                Err(StorageError::PermissionDenied { .. }) => {
                    warn!("PUT permission denied key={} caller={}", key, caller);
                    Response::PERMISSION_DENIED
                }
                Err(e) => Response::ERROR(e.to_string()),
            }
        }
        Request::KV_GET { key } => {
            match storage.get(&key, caller) {
                Ok(v) => Response::VALUE(v),
                Err(StorageError::NotFound(_)) => Response::NOT_FOUND,
                Err(StorageError::PermissionDenied { .. }) => {
                    warn!("GET permission denied key={} caller={}", key, caller);
                    Response::PERMISSION_DENIED
                }
                Err(e) => Response::ERROR(e.to_string()),
            }
        }
        Request::KV_DELETE { key } => {
            match storage.delete(&key, caller) {
                Ok(()) => {
                    info!("DELETE ok key={} caller={}", key, caller);
                    Response::OK
                }
                Err(StorageError::NotFound(_)) => Response::NOT_FOUND,
                Err(StorageError::PermissionDenied { .. }) => {
                    warn!("DELETE permission denied key={} caller={}", key, caller);
                    Response::PERMISSION_DENIED
                }
                Err(e) => Response::ERROR(e.to_string()),
            }
        }
        Request::KV_SCAN { prefix } => {
            let keys = storage.scan_prefix(&prefix);
            Response::SCAN_RESULT(keys)
        }
    }
}

/// Resolve caller identity from SO_PEERCRED.
/// uid==0 => "root" (full bypass).
/// Otherwise "uid:<uid>".
fn resolve_caller_identity(stream: &UnixStream) -> String {
    unsafe {
        let fd = stream.as_raw_fd();
        #[repr(C)]
        struct Ucred {
            pid: libc::pid_t,
            uid: libc::uid_t,
            gid: libc::gid_t,
        }
        let mut cred: Ucred = std::mem::zeroed();
        let mut len = std::mem::size_of::<Ucred>() as libc::socklen_t;

        let ret = libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut cred as *mut _ as *mut libc::c_void,
            &mut len,
        );
        if ret == 0 {
            if cred.uid == 0 {
                return "root".to_string();
            }
            return format!("uid:{}", cred.uid);
        }
    }
    // Fallback when getsockopt fails (very rare).
    "unknown".to_string()
}

// -------------------------------------------------------------------------
// Length-prefixed bincode framing (binary IPC)
// -------------------------------------------------------------------------

#[allow(dead_code)]
fn send_request(stream: &mut UnixStream, req: &Request) -> io::Result<()> {
    let bytes = bincode::serialize(req)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let len = bytes.len() as u32;
    stream.write_all(&len.to_le_bytes())?;
    stream.write_all(&bytes)?;
    stream.flush()?;
    Ok(())
}

#[allow(dead_code)]
fn recv_request(stream: &mut UnixStream) -> io::Result<Option<Request>> {
    let mut len_buf = [0u8; 4];
    match stream.read_exact(&mut len_buf) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_le_bytes(len_buf) as usize;

    if len > 16 * 1024 * 1024 {
        // Guard against absurdly large frames (16 MiB cap for this KV).
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame too large",
        ));
    }

    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf)?;
    let req: Request = bincode::deserialize(&buf)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    Ok(Some(req))
}

#[allow(dead_code)]
fn send_response(stream: &mut UnixStream, resp: &Response) -> io::Result<()> {
    let bytes = bincode::serialize(resp)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let len = bytes.len() as u32;
    stream.write_all(&len.to_le_bytes())?;
    stream.write_all(&bytes)?;
    stream.flush()?;
    Ok(())
}

#[allow(dead_code)]
fn recv_response(stream: &mut UnixStream) -> io::Result<Response> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf)?;
    let len = u32::from_le_bytes(len_buf) as usize;

    if len > 16 * 1024 * 1024 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame too large",
        ));
    }

    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf)?;
    let resp: Response = bincode::deserialize(&buf)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    Ok(resp)
}

// Framing helpers are intentionally private to the daemon module.
// The CLI duplicates the tiny length+bincode framing (keeps transport concerns local to each side).
