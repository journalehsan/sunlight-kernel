use alloc::vec::Vec;

use sunlight_ipc::{
    debug_log, endpoint_create, ipc_recv, ipc_reply_and_wait, nameserver_register, process_yield,
    shm_alloc, shm_free, shm_map, IpcMsg, SHM_PAGE,
};
use sunlight_libc as libc;

use wiseowl_brain::native_ipc::{
    BrainIpcHeader, BrainOp, NATIVE_PROTOCOL_VERSION, BRAIN_ENDPOINT,
    BRAIN_IPC_HEADER_LEN, INLINE_PAYLOAD_THRESHOLD, SHM_PAGE_SIZE,
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
        serial_println!("[WISEOWL-BRAIN] SERVICE_READY PASS");
        serial_println!("[WISEOWL-BRAIN] registered {}", BRAIN_ENDPOINT);
    } else {
        serial_println!("[WISEOWL-BRAIN] failed to register {}", BRAIN_ENDPOINT);
        process_yield();
        libc::exit(1);
    }

    loop {
        let msg: IpcMsg = match ipc_recv(ep, 5000) {
            Ok(m) => m,
            Err(_) => {
                process_yield();
                continue;
            }
        };

        let op = match BrainOp::from_u16(msg.label as u16) {
            Some(o) => o,
            None => {
                serial_println!("[WISEOWL-BRAIN] unknown op 0x{:04X}", msg.label as u16);
                continue;
            }
        };

        match op {
            BrainOp::Greeting => {
                let response = handle_native_greeting(msg);
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
            _ => {
                let reply = make_error_reply(BrainOp::Error, msg, 3, 0);
                let _ = ipc_reply_and_wait(ep, reply);
            }
        }
    }
}

fn handle_native_greeting(msg: IpcMsg) -> IpcMsg {
    serial_println!("[WISEOWL-BRAIN] GREETING_REQUEST PASS");

    let body = read_native_body(msg);
    let (request, _) = match BrainRequestWire::decode(&body) {
        Ok(r) => r,
        Err(_) => {
            return make_error_reply(BrainOp::Error, msg, 100, 0);
        }
    };

    let pipeline = unsafe { PIPELINE.as_mut().unwrap() };
    let response = pipeline.handle_request(&request);

    serial_println!("[WISEOWL-BRAIN] GREETING_RESPONSE PASS");
    make_reply(msg, &response)
}

fn handle_native_health(msg: IpcMsg) -> IpcMsg {
    let _pipeline = unsafe { PIPELINE.as_mut().unwrap() };
    let _snap = _pipeline.diagnostics.snapshot();
    serial_println!("[WISEOWL-BRAIN] HEALTH PASS");

    let greeting = wiseowl_brain::protocol::GreetingResponseWire::simple("Health", "OK");
    let resp = wiseowl_brain::protocol::BrainResponseWire::greeting(greeting, msg.label as u64);
    make_reply(msg, &resp)
}

fn handle_native_stats(msg: IpcMsg) -> IpcMsg {
    let pipeline = unsafe { PIPELINE.as_mut().unwrap() };
    let greeting = wiseowl_brain::protocol::GreetingResponseWire::simple("Stats", "OK");
    let resp = wiseowl_brain::protocol::BrainResponseWire::greeting(greeting, msg.label as u64);
    make_reply(msg, &resp)
}

fn read_native_body(msg: IpcMsg) -> Vec<u8> {
    if msg.word_count >= 1 {
        let body_len = msg.words[0] as usize;
        if body_len > 0 && body_len <= INLINE_PAYLOAD_THRESHOLD as usize {
            let mut body = Vec::with_capacity(body_len);
            for i in 0..body_len.min(56) {
                let word_idx = 1 + i / 8;
                let byte_idx = i % 8;
                let byte = (msg.words[word_idx] >> (byte_idx * 8)) as u8;
                body.push(byte);
            }
            return body;
        }
    }

    if msg.cap_count > 0 {
        if let Ok(ptr) = shm_map(msg.caps[0]) {
            let slice = unsafe { core::slice::from_raw_parts(ptr as *const u8, SHM_PAGE_SIZE as usize) };
            let header = match BrainIpcHeader::decode(slice) {
                Ok(h) => h,
                Err(_) => return Vec::new(),
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

    if resp_bytes.len() <= INLINE_PAYLOAD_THRESHOLD as usize {
        let mut reply = IpcMsg::with_label(BrainOp::Reply.label());
        reply.words[0] = resp_bytes.len() as u64;

        let mut packed = [0u8; 56];
        let header_enc = header.encode();
        let copy_len = header_enc.len().min(56);
        packed[..copy_len].copy_from_slice(&header_enc[..copy_len]);
        for i in 0..8 {
            let mut word: u64 = 0;
            for j in 0..8 {
                if i * 8 + j < resp_bytes.len() {
                    word |= (resp_bytes[i * 8 + j] as u64) << (j * 8);
                }
            }
            reply.words[1 + i] = word;
        }
        reply
    } else {
        let shm = shm_alloc(SHM_PAGE_SIZE).expect("shm_alloc for reply");
        if let Ok(base) = shm_map(shm) {
            let slice = unsafe { core::slice::from_raw_parts_mut(base as *mut u8, SHM_PAGE_SIZE as usize) };
            let header_enc = header.encode();
            slice[..BRAIN_IPC_HEADER_LEN].copy_from_slice(&header_enc);
            let copy_len = resp_bytes.len().min(SHM_PAGE_SIZE as usize - BRAIN_IPC_HEADER_LEN);
            slice[BRAIN_IPC_HEADER_LEN..BRAIN_IPC_HEADER_LEN + copy_len].copy_from_slice(&resp_bytes[..copy_len]);
        }
        let mut reply = IpcMsg::with_label(BrainOp::Reply.label());
        reply.words[0] = resp_bytes.len() as u64;
        reply = reply.with_cap(0, shm);
        reply
    }
}

fn make_error_reply(op: BrainOp, msg: IpcMsg, code: u16, _request_id: u64) -> IpcMsg {
    let err_resp = wiseowl_brain::protocol::BrainResponseWire::error(code, msg.label as u64);
    make_reply(msg, &err_resp)
}
