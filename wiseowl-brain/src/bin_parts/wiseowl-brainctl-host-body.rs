use std::env;
use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use wiseowl_brain::native_ipc::{BrainOp, BrainIpcHeader, BRAIN_IPC_HEADER_LEN, NATIVE_PROTOCOL_VERSION};
use wiseowl_brain::protocol::{
    BrainRequestWire, BrainResponseWire, GreetingRequestWire,
    MAX_DEVICE_CLASS_LEN, MAX_MODEL_LEN, MAX_NAME_LEN, MAX_VERSION_LEN,
};

fn main() {
    let socket = env::var("WISEOWL_BRAIN_SOCKET")
        .unwrap_or_else(|_| "/tmp/sunlight/wiseowl-brain.sock".to_string());

    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: wiseowl-brainctl <health|stats|greet> [options]");
        std::process::exit(1);
    }

    match args[1].as_str() {
        "health" => cmd_health(&socket),
        "stats" => cmd_stats(&socket),
        "greet" => cmd_greet(&socket, &args),
        _ => {
            eprintln!("unknown command: {}", args[1]);
            std::process::exit(1);
        }
    }
}

fn connect(socket: &str) -> io::Result<UnixStream> {
    let stream = UnixStream::connect(socket)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    Ok(stream)
}

fn send_request(stream: &mut UnixStream, op: BrainOp, body: &[u8], request_id: u64) -> io::Result<BrainResponseWire> {
    let header = BrainIpcHeader {
        protocol_version: NATIVE_PROTOCOL_VERSION,
        operation: op.as_u16(),
        flags: 0,
        request_id,
        body_len: body.len() as u32,
        reserved: 0,
    };

    stream.write_all(&header.encode())?;
    if !body.is_empty() {
        stream.write_all(body)?;
    }
    stream.flush()?;

    let mut resp_header_buf = [0u8; BRAIN_IPC_HEADER_LEN];
    stream.read_exact(&mut resp_header_buf)?;
    let resp_header = BrainIpcHeader::decode(&resp_header_buf)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("{:?}", e)))?;

    let mut resp_body = vec![0u8; resp_header.body_len as usize];
    if resp_header.body_len > 0 {
        stream.read_exact(&mut resp_body)?;
    }

    let (response, _) = BrainResponseWire::decode(&resp_body)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("{:?}", e)))?;

    Ok(response)
}

fn cmd_health(socket: &str) {
    let mut stream = connect(socket).expect("connect");
    let resp = send_request(&mut stream, BrainOp::Health, &[], 1).expect("health request");
    println!("service: wiseowl-braind");
    println!("provider_local: {}", resp.provider == 1);
    println!("confidence: {}", resp.confidence);
    if resp.error_code != 0 {
        println!("error_code: {}", resp.error_code);
    }
    if let Some(g) = &resp.greeting {
        println!("message: {}", g.title);
    }
}

fn cmd_stats(socket: &str) {
    let mut stream = connect(socket).expect("connect");
    let resp = send_request(&mut stream, BrainOp::Stats, &[], 1).expect("stats request");
    if let Some(g) = &resp.greeting {
        println!("title: {}", g.title);
        println!("body: {}", g.body);
    }
}

fn cmd_greet(socket: &str, args: &[String]) {
    let mut user_id: u64 = 1000;
    let mut welcome = false;
    let mut name = "User";

    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--user" if i + 1 < args.len() => {
                user_id = args[i + 1].parse().unwrap_or(1000);
                i += 2;
            }
            "--name" if i + 1 < args.len() => {
                name = &args[i + 1];
                i += 2;
            }
            "--welcome" => {
                welcome = true;
                i += 1;
            }
            _ => {
                i += 1;
            }
        }
    }

    let mut dn: heapless::String<MAX_NAME_LEN> = heapless::String::new();
    for c in name.chars().take(MAX_NAME_LEN) {
        let _ = dn.push(c);
    }
    let mut ver: heapless::String<MAX_VERSION_LEN> = heapless::String::new();
    let _ = ver.push_str("0.2.0");
    let mut dc: heapless::String<MAX_DEVICE_CLASS_LEN> = heapless::String::new();
    let _ = dc.push_str("desktop");
    let mn: heapless::String<MAX_MODEL_LEN> = heapless::String::new();

    let req = BrainRequestWire {
        request_id: 1,
        caller_uid: user_id,
        user_id,
        session_id: if welcome { 1 } else { 0 },
        locale_len: 0,
        locale: heapless::String::new(),
        request_kind: 1,
        greeting: Some(GreetingRequestWire {
            welcome_mode: if welcome { 1 } else { 3 },
            first_login: if welcome { 1 } else { 0 },
            first_after_upgrade: 0,
            machine_summary_requested: 1,
            display_name: dn,
            sunlight_version: ver,
            cpu_cores: 0,
            ram_mib: 0,
            device_class: dc,
            model_name: mn,
            screen_w: 0,
            screen_h: 0,
        }),
    };

    let body = req.encode();
    let mut stream = connect(socket).expect("connect");
    let resp = send_request(&mut stream, BrainOp::Greeting, &body, 1).expect("greeting request");

    println!("--- Brain Greeting Response ---");
    println!("provider: {}", if resp.provider == 1 { "local-bounded" } else { "unknown" });
    println!("confidence: {}", resp.confidence);
    println!("error_code: {}", resp.error_code);

    if let Some(g) = &resp.greeting {
        println!();
        println!("Title: {}", g.title);
        println!("Body:  {}", g.body);
        if !g.highlights.is_empty() {
            println!();
            println!("Highlights:");
            for h in &g.highlights {
                println!("  [{}] {}: {}", h.kind, h.label, h.value);
            }
        }
        if !g.suggested_actions.is_empty() {
            println!();
            println!("Suggested Actions:");
            for a in &g.suggested_actions {
                println!("  [{}] {}", a.kind, a.label);
            }
        }
    } else if resp.error_code != 0 {
        println!("Error: code={}", resp.error_code);
    }
}
