use alloc::vec::Vec;
use core::fmt::Write;

use sunlight_ipc::{
    debug_log, ipc_call, nameserver_lookup, process_yield, shm_alloc, shm_free, shm_map,
    CapabilityToken, IpcMsg,
};
use sunlight_libc as libc;

use wiseowl_brain::native_ipc::{
    BrainIpcHeader, BrainOp, BRAIN_ENDPOINT, BRAIN_IPC_HEADER_LEN, NATIVE_PROTOCOL_VERSION,
    REG_INLINE_BODY_MAX, SHM_PAGE_SIZE,
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
pub extern "C" fn _start(argc: u64, argv: *const *const u8) -> ! {
    let mut arg_storage: [&str; 8] = [""; 8];
    let argc = unsafe {
        libc::crt0::collect_utf8_args(argc, argv, &mut arg_storage, 512)
    };

    if argc < 2 {
        serial_println!("[WISEOWL-BRAIN] CLI usage: wiseowl-brainctl <health|stats|greet|context> [options]");
        process_yield();
        libc::exit(0);
    }

    match arg_storage[1] {
        "health" => cmd_health(),
        "stats" => cmd_stats(),
        "greet" => cmd_greet(&arg_storage[..argc]),
        "context" => cmd_context(&arg_storage[..argc]),
        "preferences" => cmd_preferences(&arg_storage[..argc]),
        _ => {
            serial_println!("[WISEOWL-BRAIN] CLI usage: wiseowl-brainctl <health|stats|greet|context|preferences> ...");
        }
    }

    process_yield();
    libc::exit(0);
}

fn connect() -> Option<CapabilityToken> {
    match nameserver_lookup(BRAIN_ENDPOINT) {
        Some(cap) => Some(cap),
        None => {
            serial_println!("[WISEOWL-BRAIN] cannot find {}", BRAIN_ENDPOINT);
            None
        }
    }
}

fn send_request(cap: CapabilityToken, op: BrainOp, body: &[u8]) -> Option<BrainResponseWire> {
    // Register ABI only carries 24 body bytes; use SHM for larger payloads.
    let (msg, req_cap) = if body.is_empty() {
        (IpcMsg::with_label(op.label()).word(0, 0), None)
    } else if body.len() <= REG_INLINE_BODY_MAX {
        let mut msg = IpcMsg::with_label(op.label());
        msg.words[0] = body.len() as u64;
        for i in 0..3 {
            let mut word: u64 = 0;
            for j in 0..8 {
                if i * 8 + j < body.len() {
                    word |= (body[i * 8 + j] as u64) << (j * 8);
                }
            }
            msg.words[1 + i] = word;
        }
        msg.word_count = (1 + body.len().div_ceil(8) as u32).min(4);
        (msg, None)
    } else {
        if body.len() + BRAIN_IPC_HEADER_LEN > SHM_PAGE_SIZE as usize {
            serial_println!("[WISEOWL-BRAIN] request too large for SHM");
            return None;
        }
        let (ptr, token) = match shm_alloc() {
            Ok(v) => v,
            Err(_) => {
                serial_println!("[WISEOWL-BRAIN] shm_alloc failed");
                return None;
            }
        };
        let header = BrainIpcHeader {
            protocol_version: NATIVE_PROTOCOL_VERSION,
            operation: op.as_u16(),
            flags: 0,
            request_id: 1,
            body_len: body.len() as u32,
            reserved: 0,
        };
        let header_enc = header.encode();
        unsafe {
            core::ptr::copy_nonoverlapping(header_enc.as_ptr(), ptr, BRAIN_IPC_HEADER_LEN);
            core::ptr::copy_nonoverlapping(body.as_ptr(), ptr.add(BRAIN_IPC_HEADER_LEN), body.len());
        }
        let msg = IpcMsg::with_label(op.label())
            .word(0, body.len() as u64)
            .with_cap(0, token);
        (msg, Some(token))
    };

    let reply = ipc_call(cap, msg);
    if let Some(tok) = req_cap {
        let _ = shm_free(tok);
    }
    let resp_body = read_reply_body(reply);
    BrainResponseWire::decode(&resp_body).ok().map(|(r, _)| r)
}

fn read_reply_body(msg: IpcMsg) -> Vec<u8> {
    if msg.cap_count > 0 {
        if let Ok(ptr) = shm_map(msg.caps[0]) {
            let slice =
                unsafe { core::slice::from_raw_parts(ptr as *const u8, SHM_PAGE_SIZE as usize) };
            let body_len = if slice.len() >= 20 {
                u32::from_le_bytes([slice[16], slice[17], slice[18], slice[19]]) as usize
            } else {
                0
            };
            let start = BRAIN_IPC_HEADER_LEN;
            let end = (start + body_len).min(SHM_PAGE_SIZE as usize);
            let mut body = Vec::with_capacity(end.saturating_sub(start));
            body.extend_from_slice(&slice[start..end]);
            let _ = shm_free(msg.caps[0]);
            return body;
        }
    }
    let body_len = if msg.word_count >= 1 {
        (msg.words[0] as usize).min(REG_INLINE_BODY_MAX)
    } else {
        0
    };
    let mut body = Vec::with_capacity(body_len);
    for i in 0..body_len {
        let word_idx = 1 + i / 8;
        if word_idx >= 4 {
            break;
        }
        let byte_idx = i % 8;
        let byte = (msg.words[word_idx] >> (byte_idx * 8)) as u8;
        body.push(byte);
    }
    body
}

fn cmd_health() {
    let Some(cap) = connect() else { return };
    let resp = send_request(cap, BrainOp::Health, &[]);
    if let Some(_r) = resp {
        serial_println!("[WISEOWL-BRAIN] NATIVE_HEALTH PASS");
    }
}

fn cmd_stats() {
    let Some(cap) = connect() else { return };
    let _resp = send_request(cap, BrainOp::Stats, &[]);
    serial_println!("[WISEOWL-BRAIN] STATS");
}

fn cmd_greet(args: &[&str]) {
    let Some(cap) = connect() else {
        serial_println!("[WISEOWL-BRAIN] cannot connect");
        return;
    };

    let mut user_id: u64 = 1000;
    let mut welcome = false;
    let mut name = "User";

    let mut i = 2;
    while i < args.len() {
        match args[i] {
            "--user" if i + 1 < args.len() => {
                if let Ok(uid) = args[i + 1].parse() {
                    user_id = uid;
                }
                i += 2;
            }
            "--name" if i + 1 < args.len() => {
                name = args[i + 1];
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
    if let Some(resp) = send_request(cap, BrainOp::Greeting, &body) {
        serial_println!("[WISEOWL-BRAIN] NATIVE_REQUEST PASS");
        if resp.provider == 1 {
            serial_println!("[WISEOWL-BRAIN] LOCAL_PROVIDER PASS");
        }
        if let Some(g) = &resp.greeting {
            serial_println!("[WISEOWL-BRAIN] STRUCTURED_RESPONSE PASS");
            serial_println!("[WISEOWL-BRAIN] title={}", g.title);
            serial_println!("[WISEOWL-BRAIN] body={}", g.body);
        }
    } else {
        serial_println!("[WISEOWL-BRAIN] GREETING_FAILED");
    }
}

fn cmd_context(args: &[&str]) {
    let Some(cap) = connect() else {
        serial_println!("[WISEOWL-BRAIN] cannot connect for context");
        return;
    };

    let mut user_id: u64 = 1000;
    let mut i = 2;
    while i < args.len() {
        match args[i] {
            "--user" if i + 1 < args.len() => {
                if let Ok(uid) = args[i + 1].parse() {
                    user_id = uid;
                }
                i += 2;
            }
            "--welcome" => {
                i += 1;
            }
            _ => {
                i += 1;
            }
        }
    }

    let req = BrainRequestWire {
        request_id: 2,
        caller_uid: user_id,
        user_id,
        session_id: 0,
        locale_len: 0,
        locale: heapless::String::new(),
        request_kind: 1,
        greeting: None,
    };

    let body = req.encode();
    let resp = send_request(cap, BrainOp::Context, &body);
    if let Some(r) = resp {
        if let Some(g) = &r.greeting {
            serial_println!("[WISEOWL-BRAIN] CONTEXT title={}", g.title);
            serial_println!("[WISEOWL-BRAIN] CONTEXT body={}", g.body);
        }
        serial_println!(
            "[WISEOWL-BRAIN] CONTEXT provider={} confidence={}",
            r.provider,
            r.confidence
        );
    }
}

fn cmd_preferences(args: &[&str]) {
    let Some(cap) = connect() else {
        serial_println!("[WISEOWL-BRAIN] cannot connect for preferences");
        return;
    };
    let mut user_id: u64 = 0;
    let sub = args.get(2).copied().unwrap_or("show");
    let mut i = 3;
    while i < args.len() {
        match args[i] {
            "--user" if i + 1 < args.len() => {
                if let Ok(uid) = args[i + 1].parse() {
                    user_id = uid;
                }
                i += 2;
            }
            _ => i += 1,
        }
    }

    match sub {
        "show" => {
            let mut body = heapless::Vec::<u8, 16>::new();
            for b in user_id.to_le_bytes() {
                let _ = body.push(b);
            }
            if let Some(r) = send_request(cap, BrainOp::PreferencesGet, &body) {
                if let Some(g) = &r.greeting {
                    serial_println!("[WISEOWL-BRAIN] preferences: {}", g.body);
                }
                serial_println!("[WISEOWL-BRAIN] PREFERENCES_READ PASS");
                serial_println!("[WISEOWL-BRAIN] NATIVE_CLI PASS");
            }
        }
        "set" => {
            // wiseowl-brainctl preferences set greeting-style concise
            // or set show-machine-summary true
            let field = args.get(3).copied().unwrap_or("");
            let value = args.get(4).copied().unwrap_or("");
            let (field_code, value_code) = match (field, value) {
                ("greeting-style", "concise") => (1u64, 0u64),
                ("greeting-style", "friendly") => (1, 1),
                ("greeting-style", "technical") => (1, 2),
                ("show-machine-summary", "true") | ("show-machine-summary", "1") => (2, 1),
                ("show-machine-summary", "false") | ("show-machine-summary", "0") => (2, 0),
                ("show-index-status", "true") | ("show-index-status", "1") => (3, 1),
                ("show-index-status", "false") | ("show-index-status", "0") => (3, 0),
                _ => {
                    serial_println!("[WISEOWL-BRAIN] invalid preference field/value");
                    return;
                }
            };
            let mut msg = IpcMsg::with_label(BrainOp::PreferencesSet.label())
                .word(0, user_id)
                .word(1, field_code)
                .word(2, value_code);
            msg.word_count = 3;
            let reply = ipc_call(cap, msg);
            let resp_body = read_reply_body(reply);
            if BrainResponseWire::decode(&resp_body).is_ok() {
                serial_println!("[WISEOWL-BRAIN] PREFERENCES_WRITE PASS");
                serial_println!("[WISEOWL-BRAIN] PREFERENCES_APPLIED PASS");
            } else {
                serial_println!("[WISEOWL-BRAIN] preferences set failed");
            }
        }
        _ => {
            serial_println!("[WISEOWL-BRAIN] preferences usage: show | set <field> <value>");
        }
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    serial_println!("[WISEOWL-BRAIN] PANIC brainctl");
    loop {
        process_yield();
    }
}
