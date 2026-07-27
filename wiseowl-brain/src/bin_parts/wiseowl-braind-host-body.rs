use std::io::{self, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::Mutex;

use wiseowl_brain::pipeline::CognitivePipeline;
use wiseowl_brain::protocol::BrainRequestWire;
use wiseowl_brain::native_ipc::{BrainOp, BrainIpcHeader, BRAIN_IPC_HEADER_LEN, NATIVE_PROTOCOL_VERSION};

fn main() {
    let socket = std::env::var("WISEOWL_BRAIN_SOCKET")
        .unwrap_or_else(|_| "/tmp/sunlight/wiseowl-brain.sock".to_string());

    if let Some(parent) = PathBuf::from(&socket).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if PathBuf::from(&socket).exists() {
        let _ = std::fs::remove_file(&socket);
    }

    let pipeline = Mutex::new(CognitivePipeline::new());
    let listener = UnixListener::bind(&socket).expect("bind socket");
    eprintln!("wiseowl-braind listening on {socket}");

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                if let Err(e) = handle_client(stream, &pipeline) {
                    eprintln!("client error: {e}");
                }
            }
            Err(e) => eprintln!("accept error: {e}"),
        }
    }
}

fn handle_client(mut stream: UnixStream, pipeline: &Mutex<CognitivePipeline>) -> io::Result<()> {
    loop {
        let mut header_buf = [0u8; BRAIN_IPC_HEADER_LEN];
        if stream.read_exact(&mut header_buf).is_err() {
            return Ok(());
        }

        let header = BrainIpcHeader::decode(&header_buf)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("bad header: {:?}", e)))?;

        let mut body = vec![0u8; header.body_len as usize];
        if header.body_len > 0 {
            stream.read_exact(&mut body)?;
        }

        let op = BrainOp::from_u16(header.operation)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "unknown op"))?;

        let response = match op {
            BrainOp::Greeting => {
                match BrainRequestWire::decode(&body) {
                    Ok((req, _)) => {
                        let mut p = pipeline.lock().unwrap();
                        p.handle_request(&req)
                    }
                    Err(_) => {
                        wiseowl_brain::protocol::BrainResponseWire::error(100, header.request_id)
                    }
                }
            }
            BrainOp::Health => {
                let pipeline = pipeline.lock().unwrap();
                let snap = pipeline.diagnostics.snapshot();
                let mut body_str = format!(
                    "ready=1\nrequests_total={}\nrequests_active={}\nrequests_failed={}\nlocal_provider={}\nfuture_provider={}\n",
                    snap.requests_total,
                    snap.requests_active,
                    snap.requests_failed,
                    snap.provider_local_available as u8,
                    snap.provider_future_available as u8,
                );
                if let Some(code) = snap.last_error_code {
                    body_str.push_str(&format!("last_error={}\n", code));
                }
                wiseowl_brain::protocol::BrainResponseWire {
                    request_id: header.request_id,
                    response_kind: 0xBF80,
                    provider: 1,
                    confidence: 100,
                    error_code: 0,
                    greeting: None,
                }
            }
            BrainOp::Stats => {
                let pipeline = pipeline.lock().unwrap();
                let snap = pipeline.diagnostics.snapshot();
                let body_str = format!(
                    "requests_total={}\nrequests_failed={}\nlocal_provider={}\n",
                    snap.requests_total,
                    snap.requests_failed,
                    snap.provider_local_available as u8,
                );
                let greeting = wiseowl_brain::protocol::GreetingResponseWire::simple("Stats", &body_str);
                wiseowl_brain::protocol::BrainResponseWire::greeting(greeting, header.request_id)
            }
            _ => {
                wiseowl_brain::protocol::BrainResponseWire::error(3, header.request_id)
            }
        };

        let resp_bytes = response.encode();
        let resp_header = BrainIpcHeader {
            protocol_version: NATIVE_PROTOCOL_VERSION,
            operation: BrainOp::Reply.as_u16(),
            flags: 0,
            request_id: header.request_id,
            body_len: resp_bytes.len() as u32,
            reserved: 0,
        };

        stream.write_all(&resp_header.encode())?;
        stream.write_all(&resp_bytes)?;
        stream.flush()?;
    }
}
