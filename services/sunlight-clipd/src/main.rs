#![no_std]
#![no_main]
#![allow(static_mut_refs)]

extern crate alloc;

use alloc::vec::Vec;

use sunlight_clipd::{
    decode_current, decode_history_state, decode_item, decode_set_request, encode_current,
    encode_history_state, encode_item, encode_summary_list, item_key, ClipError, ClipboardState,
    SetOutcome, KV_KEY_CURRENT, KV_KEY_HISTORY,
};
use sunlight_ipc::{
    debug_log, endpoint_create, ipc_call, ipc_recv, ipc_reply_and_wait, monotonic_millis,
    nameserver_lookup, nameserver_register, shm_alloc, shm_free, shm_map, CapabilityToken, ClipMsg,
    IpcMsg, IPC_REGISTER_WORDS, SHM_PAGE,
};

struct BumpAllocator;

unsafe impl core::alloc::GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        static mut HEAP: [u8; 192 * 1024] = [0; 192 * 1024];
        static mut NEXT: usize = 0;
        let start = NEXT;
        let align = layout.align();
        let aligned = (start + align - 1) & !(align - 1);
        let end = aligned + layout.size();
        if end > HEAP.len() {
            return core::ptr::null_mut();
        }
        NEXT = end;
        HEAP.as_mut_ptr().add(aligned)
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: core::alloc::Layout) {}
}

#[global_allocator]
static BUMP: BumpAllocator = BumpAllocator;

macro_rules! serial_println {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let mut buf = heapless::String::<256>::new();
        let _ = write!(&mut buf, $($arg)*);
        debug_log(&buf);
    }};
}

struct KvClient;

impl KvClient {
    fn put(&self, key: &str, value: &[u8]) -> Result<(), ()> {
        if key.len() > 16 || value.len() > SHM_PAGE {
            return Err(());
        }
        let Some(cap) = nameserver_lookup("sunlight-kv") else {
            return Err(());
        };
        let (ptr, token) = shm_alloc().map_err(|_| ())?;
        unsafe {
            core::ptr::copy_nonoverlapping(value.as_ptr(), ptr, value.len());
        }
        let mut msg = IpcMsg::with_label(0x4B06)
            .word(0, value.len() as u64)
            .with_cap(0, token);
        if !pack_str(&mut msg, 2, key) {
            let _ = shm_free(token);
            return Err(());
        }
        let reply = ipc_call(cap, msg);
        let _ = shm_free(token);
        if reply.label == 0x4BFF && reply.words[0] == 0 {
            Ok(())
        } else {
            Err(())
        }
    }

    fn get(&self, key: &str) -> Result<Vec<u8>, ()> {
        if key.len() > 16 {
            return Err(());
        }
        let Some(cap) = nameserver_lookup("sunlight-kv") else {
            return Err(());
        };
        let mut msg = IpcMsg::with_label(0x4B07);
        if !pack_str(&mut msg, 2, key) {
            return Err(());
        }
        let reply = ipc_call(cap, msg);
        if reply.label != 0x4B05 {
            return Err(());
        }
        let len = (reply.words[0] as usize).min(SHM_PAGE);
        let token = reply.caps[0];
        if token == CapabilityToken::INVALID {
            return Ok(Vec::new());
        }
        let ptr = shm_map(token).map_err(|_| ())?;
        let value = unsafe { core::slice::from_raw_parts(ptr, len) }.to_vec();
        let _ = shm_free(token);
        Ok(value)
    }

    fn delete(&self, key: &str) -> Result<(), ()> {
        if key.len() > 24 {
            return Err(());
        }
        let Some(cap) = nameserver_lookup("sunlight-kv") else {
            return Err(());
        };
        let mut msg = IpcMsg::with_label(0x4B03);
        if !pack_str(&mut msg, 0, key) {
            return Err(());
        }
        let reply = ipc_call(cap, msg);
        if reply.label == 0x4BFF && reply.words[0] == 0 {
            Ok(())
        } else {
            Err(())
        }
    }
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    serial_println!("[CLIPD] starting");
    let ep = endpoint_create();
    nameserver_register("clipd", ep);
    serial_println!("[CLIPD] registered");

    let kv = KvClient;
    let mut state = load_state(&kv);
    serial_println!(
        "[CLIPD] history={} current={}",
        state.history().len(),
        state.current_id().unwrap_or(0)
    );

    let mut msg = ipc_recv(ep);
    loop {
        let reply = match handle_request(&mut state, &kv, &msg) {
            Ok(reply) => reply,
            Err(err) => IpcMsg::with_label(ClipMsg::ERROR).word(0, err.code()),
        };
        msg = ipc_reply_and_wait(ep, reply);
    }
}

fn handle_request(
    state: &mut ClipboardState,
    kv: &KvClient,
    msg: &IpcMsg,
) -> Result<IpcMsg, ClipError> {
    match msg.label {
        ClipMsg::SET_CLIPBOARD => {
            let request = decode_set_request(&take_request_page(msg)?)?;
            let outcome = state.set_item(request, monotonic_millis())?;
            persist_after_set(state, kv, &outcome)?;
            Ok(IpcMsg::with_label(ClipMsg::REPLY).word(0, outcome.current_id as u64))
        }
        ClipMsg::GET_CLIPBOARD => {
            if let Some(item) = state.current() {
                reply_with_bytes(ClipMsg::REPLY, item.id as u64, &encode_item(item))
            } else {
                Ok(IpcMsg::with_label(ClipMsg::REPLY).word(0, 0).word(1, 0))
            }
        }
        ClipMsg::GET_CLIPBOARD_SUMMARY => {
            let summaries = state.summaries();
            if let Some(summary) = summaries.iter().find(|entry| entry.is_current) {
                reply_with_bytes(
                    ClipMsg::REPLY,
                    summary.id as u64,
                    &encode_summary_list(core::slice::from_ref(summary)),
                )
            } else {
                Ok(IpcMsg::with_label(ClipMsg::REPLY).word(0, 0).word(1, 0))
            }
        }
        ClipMsg::LIST_CLIPBOARD_HISTORY => {
            let summaries = state.summaries();
            let mut reply = reply_with_bytes(
                ClipMsg::REPLY,
                summaries.len() as u64,
                &encode_summary_list(&summaries),
            )?;
            reply.words[2] = state.current_id().unwrap_or(0) as u64;
            if reply.word_count < 3 {
                reply.word_count = 3;
            }
            Ok(reply)
        }
        ClipMsg::SELECT_CLIPBOARD_HISTORY_ITEM => {
            let id = match msg.words[0] {
                ClipMsg::SELECT_BY_INDEX => state.select_by_index(msg.words[1] as usize)?,
                ClipMsg::SELECT_BY_ID => state.select_by_id(msg.words[1] as u32)?,
                _ => return Err(ClipError::BadRequest),
            };
            persist_state(state, kv);
            Ok(IpcMsg::with_label(ClipMsg::REPLY).word(0, id as u64))
        }
        ClipMsg::CLEAR_CLIPBOARD => {
            state.clear_current();
            persist_current(state, kv);
            Ok(IpcMsg::with_label(ClipMsg::REPLY).word(0, 0))
        }
        ClipMsg::CLEAR_CLIPBOARD_HISTORY => {
            let ids = state.clear_history();
            for id in ids {
                let _ = kv.delete(&item_key(id));
            }
            persist_state(state, kv);
            Ok(IpcMsg::with_label(ClipMsg::REPLY).word(0, 0))
        }
        _ => Err(ClipError::BadRequest),
    }
}

fn load_state(kv: &KvClient) -> ClipboardState {
    let current_id = kv
        .get(KV_KEY_CURRENT)
        .ok()
        .and_then(|bytes| decode_current(&bytes).ok())
        .flatten();

    let Some(history_bytes) = kv.get(KV_KEY_HISTORY).ok() else {
        return ClipboardState::new();
    };
    let Ok((next_id, ids)) = decode_history_state(&history_bytes) else {
        return ClipboardState::new();
    };

    let mut items = Vec::new();
    for id in ids {
        let Ok(bytes) = kv.get(&item_key(id)) else {
            continue;
        };
        let Ok(item) = decode_item(&bytes) else {
            continue;
        };
        items.push(item);
    }
    ClipboardState::from_persisted(current_id, next_id, items)
}

fn persist_after_set(
    state: &ClipboardState,
    kv: &KvClient,
    outcome: &SetOutcome,
) -> Result<(), ClipError> {
    let item = state
        .history()
        .iter()
        .find(|item| item.id == outcome.current_id)
        .ok_or(ClipError::Internal)?;
    kv.put(&item_key(item.id), &encode_item(item))
        .map_err(|_| ClipError::Internal)?;
    for id in &outcome.evicted_ids {
        let _ = kv.delete(&item_key(*id));
    }
    persist_state(state, kv);
    Ok(())
}

fn persist_state(state: &ClipboardState, kv: &KvClient) {
    let ids: Vec<u32> = state.history().iter().map(|item| item.id).collect();
    let _ = kv.put(KV_KEY_HISTORY, &encode_history_state(state.next_id(), &ids));
    persist_current(state, kv);
}

fn persist_current(state: &ClipboardState, kv: &KvClient) {
    let _ = kv.put(KV_KEY_CURRENT, &encode_current(state.current_id()));
}

fn take_request_page(msg: &IpcMsg) -> Result<Vec<u8>, ClipError> {
    let len = msg.words[0] as usize;
    let token = msg.caps[0];
    if len == 0 || len > SHM_PAGE || token == CapabilityToken::INVALID {
        return Err(ClipError::BadRequest);
    }
    let ptr = shm_map(token).map_err(|_| ClipError::BadRequest)?;
    let bytes = unsafe { core::slice::from_raw_parts(ptr, len) }.to_vec();
    let _ = shm_free(token);
    Ok(bytes)
}

fn reply_with_bytes(label: u64, value0: u64, bytes: &[u8]) -> Result<IpcMsg, ClipError> {
    if bytes.len() > SHM_PAGE {
        return Err(ClipError::TooLarge);
    }
    let (ptr, token) = shm_alloc().map_err(|_| ClipError::Internal)?;
    unsafe {
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
    }
    Ok(IpcMsg::with_label(label)
        .word(0, value0)
        .word(1, bytes.len() as u64)
        .with_cap(0, token))
}

fn pack_str(msg: &mut IpcMsg, start_word: usize, text: &str) -> bool {
    if start_word >= IPC_REGISTER_WORDS {
        return false;
    }
    let bytes = text.as_bytes();
    let max = (IPC_REGISTER_WORDS - start_word) * 8;
    if bytes.len() > max {
        return false;
    }
    let mut index = 0usize;
    for word_index in start_word..IPC_REGISTER_WORDS {
        let mut word = 0u64;
        for byte_index in 0..8 {
            if index < bytes.len() {
                word |= (bytes[index] as u64) << (byte_index * 8);
                index += 1;
            }
        }
        msg.words[word_index] = word;
        if msg.word_count < (word_index + 1) as u32 {
            msg.word_count = (word_index + 1) as u32;
        }
        if index >= bytes.len() {
            break;
        }
    }
    true
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    debug_log("[CLIPD] PANIC\n");
    loop {}
}
