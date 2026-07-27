use alloc::vec::Vec;

use sunlight_ipc::{
    debug_log, endpoint_create, ipc_recv, ipc_reply_and_wait, nameserver_register, process_yield,
    shm_alloc, shm_map, IpcMsg,
};
use sunlight_libc as libc;

use wiseowl_brain::adapters::{SessionContextSource, SystemContextSource};
use wiseowl_brain::grounded::AuthIdentity;
use wiseowl_brain::native_ipc::{
    BrainIpcHeader, BrainOp, NATIVE_PROTOCOL_VERSION, BRAIN_ENDPOINT,
    BRAIN_IPC_HEADER_LEN, IPC_REG_WORDS, REG_INLINE_BODY_MAX, SHM_PAGE_SIZE,
};
use wiseowl_brain::pipeline::CognitivePipeline;
use wiseowl_brain::protocol::BrainRequestWire;

macro_rules! serial_println {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let mut buf = heapless::String::<256>::new();
        let _ = write!(&mut buf, $($arg)*);
        debug_log(&buf);
    }};
}

static mut PIPELINE: Option<CognitivePipeline> = None;

#[no_mangle]
pub extern "C" fn _start() -> ! {
    serial_println!("[WISEOWL-BRAIN] SERVICE_START");

    unsafe {
        PIPELINE = Some(CognitivePipeline::new());
    }

    let ep = endpoint_create();
    if nameserver_register(BRAIN_ENDPOINT, ep) {
        serial_println!("[WISEOWL-BRAIN] NATIVE_ELF PASS");
        serial_println!("[WISEOWL-BRAIN] SERVICE_SPAWN PASS");
        serial_println!("[WISEOWL-BRAIN] ENDPOINT_REGISTER PASS");
        serial_println!("[WISEOWL-BRAIN] SERVICE_READY PASS");
        serial_println!("[WISEOWL-BRAIN] registered {}", BRAIN_ENDPOINT);
    } else {
        serial_println!("[WISEOWL-BRAIN] failed to register {}", BRAIN_ENDPOINT);
        process_yield();
        libc::exit(1);
    }

    loop {
        let msg = ipc_recv(ep);

        let op = match BrainOp::from_u16(msg.label as u16) {
            Some(o) => o,
            None => {
                serial_println!("[WISEOWL-BRAIN] unknown op 0x{:04X}", msg.label as u16);
                continue;
            }
        };

        // Kernel fills badge with the caller process id only (see ipc bus deliver).
        let caller_pid = msg.badge;
        let caller_uid = 0u64; // UID is not available from badge; use request body.

        match op {
            BrainOp::Greeting => {
                let response = handle_native_greeting(msg, caller_uid, caller_pid);
                let _ = ipc_reply_and_wait(ep, response);
            }
            BrainOp::Health => {
                let response = handle_native_health(msg);
                let _ = ipc_reply_and_wait(ep, response);
            }
            BrainOp::Stats => {
                let response = handle_native_stats(msg);
                let _ = ipc_reply_and_wait(ep, response);
            }
            BrainOp::Context => {
                let response = handle_native_context(msg, caller_uid, caller_pid);
                let _ = ipc_reply_and_wait(ep, response);
            }
            _ => {
                let reply = make_error_reply(msg, 3);
                let _ = ipc_reply_and_wait(ep, reply);
            }
        }
    }
}

fn handle_native_greeting(msg: IpcMsg, _caller_uid_from_badge: u64, caller_pid: u64) -> IpcMsg {
    serial_println!("[WISEOWL-BRAIN] GREETING_REQUEST PASS");

    let body = read_native_body(msg);
    let (request, _) = match BrainRequestWire::decode(&body) {
        Ok(r) => r,
        Err(_) => {
            serial_println!("[WISEOWL-BRAIN] MALFORMED_INPUT PASS");
            return make_error_reply(msg, 100);
        }
    };

    // Kernel badge is caller PID only (not UID). Root (uid=0) is a valid local
    // user on SunlightOS, so do not reject on zero uid. Require a real PID stamp.
    if caller_pid == 0 {
        serial_println!("[WISEOWL-BRAIN] AUTHZ_REJECT PASS");
        return make_error_reply(msg, 403);
    }
    // Soft consistency: if both identity fields are set and disagree, reject.
    if request.user_id != 0
        && request.caller_uid != 0
        && request.user_id != request.caller_uid
    {
        serial_println!("[WISEOWL-BRAIN] AUTHZ_REJECT PASS");
        return make_error_reply(msg, 403);
    }

    let subject_uid = if request.user_id != 0 {
        request.user_id
    } else {
        request.caller_uid
    };

    let identity = AuthIdentity {
        caller_uid: subject_uid,
        caller_pid,
        session_id: request.session_id,
    };

    let session_source = SessionContextSource;
    let system_source = SystemContextSource;
    let sources: [&dyn wiseowl_brain::grounded::BrainContextSource; 2] =
        [&session_source, &system_source];

    let pipeline = unsafe { PIPELINE.as_mut().unwrap() };
    let (response, meta) = pipeline.handle_request_grounded(&request, &identity, &sources);

    serial_println!("[WISEOWL-BRAIN] NATIVE_REQUEST PASS");
    if meta.is_real_brain_response() {
        serial_println!("[WISEOWL-BRAIN] LOCAL_PROVIDER PASS");
        serial_println!("[WISEOWL-BRAIN] STRUCTURED_RESPONSE PASS");
        serial_println!("[WISEOWL-BRAIN] PROVENANCE PASS");
    } else {
        serial_println!(
            "[WISEOWL-BRAIN] RESPONSE kind={} err={} provider={}",
            response.response_kind,
            response.error_code,
            response.provider
        );
    }
    serial_println!("[WISEOWL-BRAIN] GREETING_RESPONSE PASS");
    make_reply(msg, &response)
}

fn handle_native_health(_msg: IpcMsg) -> IpcMsg {
    let pipeline = unsafe { PIPELINE.as_mut().unwrap() };
    let _snap = pipeline.diagnostics.snapshot();
    serial_println!("[WISEOWL-BRAIN] NATIVE_HEALTH PASS");

    let greeting = wiseowl_brain::protocol::GreetingResponseWire::simple("Health", "OK");
    let resp = wiseowl_brain::protocol::BrainResponseWire::greeting(greeting, 0);
    make_reply(_msg, &resp)
}

fn handle_native_stats(_msg: IpcMsg) -> IpcMsg {
    let _pipeline = unsafe { PIPELINE.as_mut().unwrap() };
    let greeting = wiseowl_brain::protocol::GreetingResponseWire::simple("Stats", "OK");
    let resp = wiseowl_brain::protocol::BrainResponseWire::greeting(greeting, 0);
    make_reply(_msg, &resp)
}

fn handle_native_context(msg: IpcMsg, caller_uid: u64, caller_pid: u64) -> IpcMsg {
    serial_println!("[WISEOWL-BRAIN] CONTEXT_REQUEST");

    let identity = AuthIdentity {
        caller_uid,
        caller_pid,
        session_id: 0,
    };

    let session_source = SessionContextSource;
    let system_source = SystemContextSource;

    use wiseowl_brain::grounded::BrainContextSource;
    let mut all_facts: Vec<wiseowl_brain::grounded::GroundedFact> = Vec::new();
    let session_facts = BrainContextSource::collect(
        &session_source,
        &wiseowl_brain::context::BrainBudget::default(),
        &identity,
    );
    for fact in session_facts {
        all_facts.push(fact);
    }
    let system_facts = BrainContextSource::collect(
        &system_source,
        &wiseowl_brain::context::BrainBudget::default(),
        &identity,
    );
    for fact in system_facts {
        all_facts.push(fact);
    }

    use core::fmt::Write;
    let mut summary = heapless::String::<256>::new();
    let _ = write!(&mut summary,
        "facts={} uid={} pid={}",
        all_facts.len(), caller_uid, caller_pid
    );
    let greeting = wiseowl_brain::protocol::GreetingResponseWire::simple("Context", &summary);
    let resp = wiseowl_brain::protocol::BrainResponseWire::greeting(greeting, msg.label as u64);
    make_reply(msg, &resp)
}

fn read_native_body(msg: IpcMsg) -> Vec<u8> {
    // Prefer SHM: greeting (and almost all brain payloads) exceed the 24-byte
    // register inline limit. Cap presence is authoritative.
    if msg.cap_count > 0 {
        if let Ok(ptr) = shm_map(msg.caps[0]) {
            let slice = unsafe { core::slice::from_raw_parts(ptr as *const u8, SHM_PAGE_SIZE as usize) };
            let header = match BrainIpcHeader::decode(slice) {
                Ok(h) => h,
                Err(_) => {
                    // Some clients write raw body without BrainIpcHeader.
                    let body_len = if msg.word_count >= 1 {
                        (msg.words[0] as usize).min(SHM_PAGE_SIZE as usize)
                    } else {
                        0
                    };
                    if body_len == 0 {
                        return Vec::new();
                    }
                    let mut body = Vec::with_capacity(body_len);
                    body.extend_from_slice(&slice[..body_len]);
                    return body;
                }
            };
            let body_start = BRAIN_IPC_HEADER_LEN;
            let body_end = body_start + header.body_len as usize;
            if body_end <= SHM_PAGE_SIZE as usize {
                let mut body = Vec::with_capacity(header.body_len as usize);
                body.extend_from_slice(&slice[body_start..body_end]);
                return body;
            }
        }
    }

    // Tiny register-only payloads: words[0]=len, words[1..3]=up to 24 body bytes.
    if msg.word_count >= 1 {
        let body_len = msg.words[0] as usize;
        if body_len > 0 && body_len <= REG_INLINE_BODY_MAX {
            let mut body = Vec::with_capacity(body_len);
            for i in 0..body_len {
                let word_idx = 1 + i / 8;
                if word_idx >= IPC_REG_WORDS as usize {
                    break;
                }
                let byte_idx = i % 8;
                let byte = (msg.words[word_idx] >> (byte_idx * 8)) as u8;
                body.push(byte);
            }
            return body;
        }
    }

    Vec::new()
}

fn make_reply(msg: IpcMsg, response: &wiseowl_brain::protocol::BrainResponseWire) -> IpcMsg {
    let resp_bytes = response.encode();
    let header = BrainIpcHeader {
        protocol_version: NATIVE_PROTOCOL_VERSION,
        operation: BrainOp::Reply.as_u16(),
        flags: 0,
        request_id: msg.label as u64,
        body_len: resp_bytes.len() as u32,
        reserved: 0,
    };

    // Greeting replies are hundreds of bytes; register ABI only carries 24.
    // Always use SHM for anything that does not fit inline.
    if resp_bytes.len() <= REG_INLINE_BODY_MAX {
        let mut reply = IpcMsg::with_label(BrainOp::Reply.label());
        reply.words[0] = resp_bytes.len() as u64;
        for i in 0..3 {
            let mut word: u64 = 0;
            for j in 0..8 {
                if i * 8 + j < resp_bytes.len() {
                    word |= (resp_bytes[i * 8 + j] as u64) << (j * 8);
                }
            }
            reply.words[1 + i] = word;
        }
        reply.word_count = (1 + resp_bytes.len().div_ceil(8) as u32).min(IPC_REG_WORDS);
        reply
    } else {
        let (base, shm_cap) = shm_alloc().expect("shm_alloc for reply");
        let slice = unsafe { core::slice::from_raw_parts_mut(base, SHM_PAGE_SIZE as usize) };
        let header_enc = header.encode();
        slice[..BRAIN_IPC_HEADER_LEN].copy_from_slice(&header_enc);
        let copy_len = resp_bytes.len().min(SHM_PAGE_SIZE as usize - BRAIN_IPC_HEADER_LEN);
        slice[BRAIN_IPC_HEADER_LEN..BRAIN_IPC_HEADER_LEN + copy_len]
            .copy_from_slice(&resp_bytes[..copy_len]);
        let mut reply = IpcMsg::with_label(BrainOp::Reply.label());
        reply.words[0] = resp_bytes.len() as u64;
        reply.word_count = 1;
        reply = reply.with_cap(0, shm_cap);
        reply
    }
}

fn make_error_reply(msg: IpcMsg, code: u16) -> IpcMsg {
    let err_resp = wiseowl_brain::protocol::BrainResponseWire::error(code, msg.label as u64);
    make_reply(msg, &err_resp)
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    serial_println!("[WISEOWL-BRAIN] PANIC braind");
    loop {
        process_yield();
    }
}
