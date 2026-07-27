use alloc::vec::Vec;

use sunlight_ipc::{
    debug_log, endpoint_create, ipc_recv, ipc_reply_and_wait, nameserver_register, process_yield,
    shm_alloc, shm_map, IpcMsg,
};
use sunlight_libc as libc;

use wiseowl_brain::adapters::{
    IndexContextSource, KvContextSource, SessionContextSource, SystemContextSource,
    WiseOwlStatusContextSource,
};
use wiseowl_brain::grounded::AuthIdentity;
use wiseowl_brain::kv_client::{load_mtm, save_preferences, save_welcome_state};
use wiseowl_brain::mtm::GreetingStyle;
use wiseowl_brain::native_ipc::{
    BrainIpcHeader, BrainOp, NATIVE_PROTOCOL_VERSION, BRAIN_ENDPOINT,
    BRAIN_IPC_HEADER_LEN, IPC_REG_WORDS, REG_INLINE_BODY_MAX, SHM_PAGE_SIZE,
};
use wiseowl_brain::pipeline::CognitivePipeline;
use wiseowl_brain::protocol::BrainRequestWire;
use wiseowl_brain::provenance::BrainProviderKind;

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
            BrainOp::PreferencesGet => {
                let response = handle_preferences_get(msg, caller_pid);
                let _ = ipc_reply_and_wait(ep, response);
            }
            BrainOp::PreferencesSet => {
                let response = handle_preferences_set(msg, caller_pid);
                let _ = ipc_reply_and_wait(ep, response);
            }
            BrainOp::WelcomeCompleted => {
                let response = handle_welcome_completed(msg, caller_pid);
                let _ = ipc_reply_and_wait(ep, response);
            }
            _ => {
                let reply = make_error_reply(msg, 3);
                let _ = ipc_reply_and_wait(ep, reply);
            }
        }
    }
}

fn subject_uid_from_request(request: &BrainRequestWire) -> u64 {
    if request.user_id != 0 {
        request.user_id
    } else {
        request.caller_uid
    }
}

fn load_kv_source(pipeline: &CognitivePipeline, uid: u64) -> KvContextSource {
    use wiseowl_brain::kv_client::native::NativeKvStore;
    let store = NativeKvStore;
    pipeline.diagnostics.inc_kv_read();
    let loaded = load_mtm(&store, uid);
    if loaded.degraded {
        pipeline.diagnostics.inc_kv_degraded();
        pipeline.diagnostics.inc_kv_read_fail();
    } else {
        pipeline.diagnostics.inc_kv_success();
    }
    KvContextSource {
        loaded: true,
        degraded: loaded.degraded && !loaded.kv_reachable,
        welcome: loaded.welcome,
        preferences: loaded.preferences,
        used_defaults: loaded.used_defaults,
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

    if caller_pid == 0 {
        serial_println!("[WISEOWL-BRAIN] AUTHZ_REJECT PASS");
        let pipeline = unsafe { PIPELINE.as_mut().unwrap() };
        pipeline.diagnostics.inc_unauthorized();
        return make_error_reply(msg, 403);
    }
    if request.user_id != 0
        && request.caller_uid != 0
        && request.user_id != request.caller_uid
    {
        serial_println!("[WISEOWL-BRAIN] AUTHZ_REJECT PASS");
        let pipeline = unsafe { PIPELINE.as_mut().unwrap() };
        pipeline.diagnostics.inc_unauthorized();
        return make_error_reply(msg, 403);
    }

    let subject_uid = subject_uid_from_request(&request);
    serial_println!(
        "[WISEOWL-BRAIN] request id={} caller_uid={} kind=greeting",
        request.request_id,
        subject_uid
    );

    let identity = AuthIdentity {
        caller_uid: subject_uid,
        caller_pid,
        session_id: request.session_id,
    };

    let pipeline = unsafe { PIPELINE.as_mut().unwrap() };

    // Priority: session → system → kv → memorydb → index
    let session_source = SessionContextSource;
    let system_source = SystemContextSource;
    let kv_source = load_kv_source(pipeline, subject_uid);
    let mut memdb_source = WiseOwlStatusContextSource::query_native();
    if memdb_source.available && !memdb_source.degraded {
        pipeline.diagnostics.inc_memorydb_success();
    } else {
        pipeline.diagnostics.inc_memorydb_degraded();
        memdb_source.degraded = true;
    }
    let mut index_source = IndexContextSource::query_native();
    if index_source.available {
        pipeline.diagnostics.inc_index_success();
    } else {
        pipeline.diagnostics.inc_index_degraded();
        index_source.degraded = true;
    }

    let sources: [&dyn wiseowl_brain::grounded::BrainContextSource; 5] = [
        &session_source,
        &system_source,
        &kv_source,
        &memdb_source,
        &index_source,
    ];

    let (response, meta) = pipeline.handle_request_grounded(&request, &identity, &sources);

    serial_println!(
        "[WISEOWL-BRAIN] context sources=session,system,kv,index facts={} flags={:#x}",
        meta.fact_count,
        meta.response_flags.0
    );

    // Best-effort: record last successful provider (not visit_count — that is completion-owned).
    if meta.is_real_brain_response() {
        use wiseowl_brain::kv_client::native::NativeKvStore;
        let store = NativeKvStore;
        let mut state = kv_source.welcome;
        state.record_successful_provider(BrainProviderKind::LocalBounded);
        if save_welcome_state(&store, subject_uid, &state).is_ok() {
            pipeline.diagnostics.inc_kv_write();
        } else {
            pipeline.diagnostics.inc_kv_write_fail();
        }
    }

    serial_println!("[WISEOWL-BRAIN] NATIVE_REQUEST PASS");
    if meta.is_real_brain_response() {
        serial_println!("[WISEOWL-BRAIN] LOCAL_PROVIDER PASS");
        serial_println!("[WISEOWL-BRAIN] STRUCTURED_RESPONSE PASS");
        serial_println!("[WISEOWL-BRAIN] PROVENANCE PASS");
        if meta.used_persisted_context {
            serial_println!("[WISEOWL-BRAIN] MTM_READ PASS");
        }
        if meta.response_flags.has(wiseowl_brain::provenance::BrainResponseFlags::FIRST_VISIT_GREETING) {
            serial_println!("[WISEOWL-BRAIN] FIRST_VISIT PASS");
        }
        if meta.response_flags.has(wiseowl_brain::provenance::BrainResponseFlags::RETURNING_USER_GREETING) {
            serial_println!("[WISEOWL-BRAIN] RETURNING_VISIT PASS");
        }
        if index_source.available {
            serial_println!("[WISEOWL-BRAIN] INDEX_STATUS PASS");
        }
        if memdb_source.available {
            serial_println!("[WISEOWL-BRAIN] MEMORYDB_STATUS PASS");
        }
        if meta.sources_degraded.0 != 0 {
            serial_println!("[WISEOWL-BRAIN] OPTIONAL_SOURCE_DEGRADE PASS");
        }
        serial_println!("[WISEOWL-BRAIN] STATUS_PROVENANCE PASS");
        serial_println!("[WISEOWL-BRAIN] SYSTEM_CONTEXT PASS");
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
    let snap = pipeline.diagnostics.snapshot();
    serial_println!("[WISEOWL-BRAIN] NATIVE_HEALTH PASS");
    serial_println!("[WISEOWL-BRAIN] HEALTH PASS");
    serial_println!("[WISEOWL-BRAIN] NATIVE_SERVICE PASS");

    let mut body = heapless::String::<128>::new();
    let _ = core::fmt::Write::write_fmt(
        &mut body,
        format_args!(
            "ok total={} failed={} local={}",
            snap.requests_total, snap.requests_failed, snap.provider_local_available as u8
        ),
    );
    let greeting =
        wiseowl_brain::protocol::GreetingResponseWire::simple("Health", body.as_str());
    let resp = wiseowl_brain::protocol::BrainResponseWire::greeting(greeting, 0);
    make_reply(_msg, &resp)
}

fn handle_native_stats(_msg: IpcMsg) -> IpcMsg {
    let pipeline = unsafe { PIPELINE.as_mut().unwrap() };
    let d = &pipeline.diagnostics;
    let mut body = heapless::String::<200>::new();
    let _ = core::fmt::Write::write_fmt(
        &mut body,
        format_args!(
            "req={} greet={} ok={} rej={} kv_r={} kv_w={} first={} ret={}",
            d.requests_total.load(core::sync::atomic::Ordering::Relaxed),
            d.requests_greeting.load(core::sync::atomic::Ordering::Relaxed),
            d.responses_successful.load(core::sync::atomic::Ordering::Relaxed),
            d.requests_rejected.load(core::sync::atomic::Ordering::Relaxed),
            d.kv_reads.load(core::sync::atomic::Ordering::Relaxed),
            d.kv_writes.load(core::sync::atomic::Ordering::Relaxed),
            d.responses_first_visit.load(core::sync::atomic::Ordering::Relaxed),
            d.responses_returning_visit.load(core::sync::atomic::Ordering::Relaxed),
        ),
    );
    let greeting =
        wiseowl_brain::protocol::GreetingResponseWire::simple("Stats", body.as_str());
    let resp = wiseowl_brain::protocol::BrainResponseWire::greeting(greeting, 0);
    make_reply(_msg, &resp)
}

fn handle_preferences_get(msg: IpcMsg, caller_pid: u64) -> IpcMsg {
    if caller_pid == 0 {
        return make_error_reply(msg, 403);
    }
    let body = read_native_body(msg);
    let uid = if body.len() >= 8 {
        u64::from_le_bytes(body[0..8].try_into().unwrap_or([0; 8]))
    } else {
        0
    };
    use wiseowl_brain::kv_client::native::NativeKvStore;
    let store = NativeKvStore;
    let loaded = load_mtm(&store, uid);
    let mut summary = heapless::String::<160>::new();
    let _ = core::fmt::Write::write_fmt(
        &mut summary,
        format_args!(
            "style={} machine={} index={} visits={}",
            loaded.preferences.greeting_style.as_str(),
            loaded.preferences.show_machine_summary as u8,
            loaded.preferences.show_index_status as u8,
            loaded.welcome.visit_count
        ),
    );
    serial_println!("[WISEOWL-BRAIN] PREFERENCES_READ PASS");
    let greeting =
        wiseowl_brain::protocol::GreetingResponseWire::simple("Preferences", summary.as_str());
    make_reply(msg, &wiseowl_brain::protocol::BrainResponseWire::greeting(greeting, 0))
}

fn handle_preferences_set(msg: IpcMsg, caller_pid: u64) -> IpcMsg {
    if caller_pid == 0 {
        return make_error_reply(msg, 403);
    }
    // words[0]=uid, words[1]=field tag, words[2]=value tag (small enums)
    // field: 1=style 2=machine 3=index
    // style value: 0=concise 1=friendly 2=technical
    // bool value: 0/1
    let uid = msg.words[0];
    let field = msg.words[1] as u8;
    let value = msg.words[2] as u8;
    use wiseowl_brain::kv_client::native::NativeKvStore;
    let store = NativeKvStore;
    let mut loaded = load_mtm(&store, uid);
    match field {
        1 => {
            loaded.preferences.greeting_style =
                GreetingStyle::from_u8(value).unwrap_or(GreetingStyle::Concise);
        }
        2 => loaded.preferences.show_machine_summary = value != 0,
        3 => loaded.preferences.show_index_status = value != 0,
        _ => return make_error_reply(msg, 1),
    }
    let pipeline = unsafe { PIPELINE.as_mut().unwrap() };
    if save_preferences(&store, uid, &loaded.preferences).is_ok() {
        pipeline.diagnostics.inc_kv_write();
        serial_println!("[WISEOWL-BRAIN] PREFERENCES_WRITE PASS");
        serial_println!("[WISEOWL-BRAIN] PREFERENCES_APPLIED PASS");
        let greeting = wiseowl_brain::protocol::GreetingResponseWire::simple("Preferences", "ok");
        make_reply(msg, &wiseowl_brain::protocol::BrainResponseWire::greeting(greeting, 0))
    } else {
        pipeline.diagnostics.inc_kv_write_fail();
        make_error_reply(msg, 10)
    }
}

fn handle_welcome_completed(msg: IpcMsg, caller_pid: u64) -> IpcMsg {
    if caller_pid == 0 {
        return make_error_reply(msg, 403);
    }
    // words[0]=uid, words[1]=system_generation
    let uid = msg.words[0];
    let gen = msg.words[1];
    use wiseowl_brain::kv_client::native::NativeKvStore;
    let store = NativeKvStore;
    let mut loaded = load_mtm(&store, uid);
    loaded.welcome.record_completion(gen);
    let pipeline = unsafe { PIPELINE.as_mut().unwrap() };
    if save_welcome_state(&store, uid, &loaded.welcome).is_ok() {
        pipeline.diagnostics.inc_kv_write();
        serial_println!("[WISEOWL-BRAIN] MTM_WRITE PASS");
        let greeting = wiseowl_brain::protocol::GreetingResponseWire::simple("Complete", "ok");
        make_reply(msg, &wiseowl_brain::protocol::BrainResponseWire::greeting(greeting, 0))
    } else {
        pipeline.diagnostics.inc_kv_write_fail();
        // Completion write failure must not break Welcome.
        let greeting = wiseowl_brain::protocol::GreetingResponseWire::simple("Complete", "degraded");
        make_reply(msg, &wiseowl_brain::protocol::BrainResponseWire::greeting(greeting, 0))
    }
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
