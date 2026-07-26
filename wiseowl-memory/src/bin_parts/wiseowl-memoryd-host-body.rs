
use std::io::{self, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use wiseowl_memory::protocol::ProtocolRequest;
use wiseowl_memory::{
    CallerIdentity, CapabilitySet, MemoryService, ServiceConfig,
};

fn main() {
    let socket = std::env::var("WISEOWL_MEMORY_SOCKET")
        .unwrap_or_else(|_| "/tmp/sunlight/wiseowl-memory.sock".to_string());
    let spill = std::env::var("WISEOWL_MEMORY_SPILL")
        .unwrap_or_else(|_| "/tmp/sunlight/wiseowl-memory-spill".to_string());

    if let Some(parent) = PathBuf::from(&socket).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::create_dir_all(&spill);
    if PathBuf::from(&socket).exists() {
        let _ = std::fs::remove_file(&socket);
    }

    let mut cfg = ServiceConfig::default();
    cfg.spill_dir = Some(PathBuf::from(spill));

    let service = MemoryService::new(cfg).expect("init wiseowl-memoryd");
    let service = Arc::new(Mutex::new(service));

    let listener = UnixListener::bind(&socket).expect("bind socket");
    eprintln!("wiseowl-memoryd listening on {socket}");

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let svc = Arc::clone(&service);
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

fn handle_client(
    mut stream: UnixStream,
    service: Arc<Mutex<MemoryService>>,
) -> io::Result<()> {
    // Default diagnostic capability for host socket clients.
    let caller = CallerIdentity {
        client_id: None,
        caps: CapabilitySet::admin(),
        owned_sessions: Vec::new(),
    };

    loop {
        let req: ProtocolRequest = match recv_msg(&mut stream)? {
            Some(r) => r,
            None => return Ok(()),
        };
        let response = {
            let mut guard = service.lock().expect("service mutex");
            // Advance monotonic clock from wall for host demos.
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(1);
            guard.set_now_ns(now.max(1));
            guard.handle(&caller, req)
        };
        send_msg(&mut stream, &response)?;
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
    // Hard cap malformed lengths (1 MiB).
    if len > 1024 * 1024 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame too large",
        ));
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf)?;
    let msg = bincode::deserialize(&buf)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    Ok(Some(msg))
}

fn send_msg<T: serde::Serialize>(stream: &mut UnixStream, msg: &T) -> io::Result<()> {
    let bytes = bincode::serialize(msg).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    let len = (bytes.len() as u32).to_le_bytes();
    stream.write_all(&len)?;
    stream.write_all(&bytes)?;
    Ok(())
}
