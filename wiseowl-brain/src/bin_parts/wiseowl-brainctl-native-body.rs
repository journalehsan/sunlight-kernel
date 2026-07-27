use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write;

use sunlight_ipc::{
    debug_log, ipc_call_timeout, nameserver_lookup, process_yield,
    shm_alloc, shm_free, shm_map, IpcMsg, SHM_PAGE,
};
use sunlight_libc as libc;
use sunlight_libc::crt0;

use wiseowl_brain::native_ipc::{
    BrainIpcHeader, BrainOp, NATIVE_PROTOCOL_VERSION, BRAIN_ENDPOINT,
    BRAIN_IPC_HEADER_LEN, INLINE_PAYLOAD_THRESHOLD, SHM_PAGE_SIZE,
};
use wiseowl_brain::protocol::{BrainRequestWire, BrainResponseWire, MAX_NAME_LEN, MAX_VERSION_LEN,
    MAX_DEVICE_CLASS_LEN, MAX_MODEL_LEN, GreetingRequestWire};

macro_rules! serial_println {
    ($($arg:tt)*) => {{
        let mut buf = heapless::String::<256>::new();
        let _ = write!(&mut buf, $($arg)*);
        debug_log(&buf);
    }};
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let _ = crt0::init();

    let args: Vec<String> = match crt0::get_args() {
        Some(a) => a.iter().filter_map(|s| {
            let mut string = String::new();
            for &b in s {
                string.push(b as char);
            }
            Some(string)
        }).collect(),
        None => Vec::new(),
    };

    if args.len() < 2 {
        serial_println!("[WISEOWL-BRAIN] CLI usage: wiseowl-brainctl <health|stats|greet> [--user id] [--welcome]");
        process_yield();
        libc::exit(0);
    }

    match args[1].as_str() {
        "health" => cmd_health(),
        "stats" => cmd_stats(),
        "greet" => cmd_greet(&args),
        _ => {
            serial_println!("[WISEOWL-BRAIN] unknown command: {}", args[1]);
        }
    }

    process_yield();
    libc::exit(0);
}

fn connect() -> Option<IpcMsg> {
    match nameserver_lookup(BRAIN_ENDPOINT) {
        Some(ep) => Some(ep),
        None => {
            serial_println!("[WISEOWL-BRAIN] cannot find {}", BRAIN_ENDPOINT);
            None
        }
    }
}

fn send_request(ep: IpcMsg, op: BrainOp, body: &[u8]) -> Option<BrainResponseWire> {
    let shm = if body.len() > INLINE_PAYLOAD_THRESHOLD as usize {
        let page = shm_alloc(SHM_PAGE_SIZE).expect("shm_alloc");
        if let Ok(base) = shm_map(page) {
            let slice = unsafe { core::slice::from_raw_parts_mut(base as *mut u8, SHM_PAGE_SIZE as usize) };
            let header = BrainIpcHeader {
                protocol_version: NATIVE_PROTOCOL_VERSION,
                operation: op.as_u16(),
                flags: 0,
                request_id: 1,
                body_len: body.len() as u32,
                reserved: 0,
            };
            let hdr_enc = header.encode();
            slice[..BRAIN_IPC_HEADER_LEN].copy_from_slice(&hdr_enc);
            let copy_len = body.len().min(SHM_PAGE_SIZE as usize - BRAIN_IPC_HEADER_LEN);
            slice[BRAIN_IPC_HEADER_LEN..BRAIN_IPC_HEADER_LEN + copy_len].copy_from_slice(&body[..copy_len]);
        }
        Some(page)
    } else {
        None
    };

    let mut msg = IpcMsg::with_label(op.label());
    if let Some(shm_cap) = shm {
        msg.words[0] = body.len() as u64;
        msg = msg.with_cap(0, shm_cap);
    } else {
        msg.words[0] = body.len() as u64;
        for (i, chunk) in body.chunks(8).enumerate().take(7) {
            let mut word: u64 = 0;
            for (j, &b) in chunk.iter().enumerate() {
                word |= (b as u64) << (j * 8);
            }
            msg.words[1 + i] = word;
        }
    }

    let reply = match ipc_call_timeout(ep, msg, 200) {
        Ok(r) => r,
        Err(_) => {
            if let Some(cap) = shm {
                let _ = shm_free(cap);
            }
            return None;
        }
    };

    if let Some(cap) = shm {
        let _ = shm_free(cap);
    }

    let resp_body = if reply.cap_count > 0 {
        read_shm_body(reply)
    } else {
        read_inline_body(reply)
    };

    BrainResponseWire::decode(&resp_body).ok().map(|(r, _)| r)
}

fn read_inline_body(msg: IpcMsg) -> Vec<u8> {
    let body_len = if msg.word_count >= 1 {
        msg.words[0] as usize
    } else {
        0
    };
    let mut body = Vec::with_capacity(body_len);
    for i in 0..body_len.min(56) {
        let word_idx = 1 + i / 8;
        let byte_idx = i % 8;
        let byte = (msg.words[word_idx] >> (byte_idx * 8)) as u8;
        body.push(byte);
    }
    body
}

fn read_shm_body(msg: IpcMsg) -> Vec<u8> {
    let body_len = msg.words[0] as usize;
    let mut body = Vec::with_capacity(body_len);
    if msg.cap_count > 0 {
        if let Ok(ptr) = shm_map(msg.caps[0]) {
            let slice = unsafe { core::slice::from_raw_parts(ptr as *const u8, SHM_PAGE_SIZE as usize) };
            let body_end = (BRAIN_IPC_HEADER_LEN + body_len).min(SHM_PAGE_SIZE as usize);
            body.extend_from_slice(&slice[BRAIN_IPC_HEADER_LEN..body_end]);
        }
    }
    body
}

fn cmd_health() {
    let Some(ep) = connect() else { return };
    let resp = send_request(ep, BrainOp::Health, &[]);
    if let Some(r) = resp {
        serial_println!("[WISEOWL-BRAIN] HEALTH PASS");
        serial_println!("[WISEOWL-BRAIN] provider_local={}", r.provider == 1);
    }
}

fn cmd_stats() {
    let Some(ep) = connect() else { return };
    let _resp = send_request(ep, BrainOp::Stats, &[]);
    serial_println!("[WISEOWL-BRAIN] STATS");
}

fn cmd_greet(args: &[String]) {
    let Some(ep) = connect() else {
        serial_println!("[WISEOWL-BRAIN] cannot connect");
        return;
    };

    let mut user_id: u64 = 1000;
    let mut welcome = false;
    let mut name = "User";

    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--user" if i + 1 < args.len() => {
                if let Ok(uid) = args[i + 1].parse() {
                    user_id = uid;
                }
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
    let mut mn: heapless::String<MAX_MODEL_LEN> = heapless::String::new();

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
    if let Some(resp) = send_request(ep, BrainOp::Greeting, &body) {
        serial_println!("[WISEOWL-BRAIN] GREETING_RESPONSE PASS");
        if let Some(g) = &resp.greeting {
            serial_println!("[WISEOWL-BRAIN] title={}", g.title);
            serial_println!("[WISEOWL-BRAIN] body={}", g.body);
        }
    } else {
        serial_println!("[WISEOWL-BRAIN] GREETING_FAILED");
    }
}
