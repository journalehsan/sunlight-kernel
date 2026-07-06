#![no_std]
#![no_main]
#![allow(static_mut_refs)]

extern crate alloc;

use alloc::string::ToString;
use alloc::vec::Vec;

use sunlight_clipd::{
    bytes_to_lossy, decode_item, decode_summary_list, encode_file_list, encode_set_request,
    ClipError, ClipboardKind, ClipboardSetRequest,
};
use sunlight_ipc::{
    ipc_call, nameserver_lookup, shm_alloc, shm_free, shm_map, CapabilityToken, ClipMsg, IpcMsg,
    ProcessExit, SHM_PAGE,
};

struct BumpAllocator;

unsafe impl core::alloc::GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        static mut HEAP: [u8; 96 * 1024] = [0; 96 * 1024];
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

const MAX_ARGS: usize = 24;

macro_rules! println {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let mut buf = heapless::String::<512>::new();
        let _ = write!(&mut buf, $($arg)*);
        stdout_write(&buf);
        stdout_write("\n");
    }};
}

#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8) -> ! {
    let mut storage = [""; MAX_ARGS];
    let count = unsafe { collect_args(argc, argv, &mut storage) };
    let args = &storage[..count];

    let Some(cap) = nameserver_lookup("clipd") else {
        println!("sunlight-clip: clipd not running");
        ProcessExit::exit(1);
    };

    let code = match args.get(1).copied().unwrap_or("get") {
        "get" => do_get(cap),
        "set" => {
            if args.len() < 3 {
                print_usage();
                1
            } else {
                do_set(
                    cap,
                    ClipboardSetRequest {
                        kind: ClipboardKind::Text,
                        mime: "text/plain".to_string(),
                        payload: args[2].as_bytes().to_vec(),
                        source_app: None,
                    },
                )
            }
        }
        "history" => do_history(cap),
        "use" => {
            if args.len() < 3 {
                print_usage();
                1
            } else {
                do_use(cap, args[2])
            }
        }
        "clear" => do_simple(cap, ClipMsg::CLEAR_CLIPBOARD, "cleared"),
        "clear-history" => do_simple(cap, ClipMsg::CLEAR_CLIPBOARD_HISTORY, "history cleared"),
        "set-file" => {
            if args.len() < 3 {
                print_usage();
                1
            } else {
                do_set(
                    cap,
                    ClipboardSetRequest {
                        kind: ClipboardKind::FileList,
                        mime: "x-sunlight/file-list".to_string(),
                        payload: encode_file_list(&args[2..3]),
                        source_app: None,
                    },
                )
            }
        }
        "set-files" => {
            if args.len() < 3 {
                print_usage();
                1
            } else {
                do_set(
                    cap,
                    ClipboardSetRequest {
                        kind: ClipboardKind::FileList,
                        mime: "x-sunlight/file-list".to_string(),
                        payload: encode_file_list(&args[2..]),
                        source_app: None,
                    },
                )
            }
        }
        _ => {
            print_usage();
            1
        }
    };

    ProcessExit::exit(code);
}

fn do_get(cap: CapabilityToken) -> i32 {
    let reply = ipc_call(cap, IpcMsg::with_label(ClipMsg::GET_CLIPBOARD));
    if reply.label == ClipMsg::ERROR {
        return print_error(reply.words[0]);
    }
    if reply.words[1] == 0 || reply.caps[0] == CapabilityToken::INVALID {
        println!("(empty)");
        return 0;
    }
    let bytes = match take_reply_bytes(&reply) {
        Ok(bytes) => bytes,
        Err(err) => return print_error(err.code()),
    };
    let item = match decode_item(&bytes) {
        Ok(item) => item,
        Err(err) => return print_error(err.code()),
    };
    if item.kind == ClipboardKind::Text {
        println!("{}", bytes_to_lossy(&item.payload));
    } else {
        println!("{} [{}] {}", item.kind.label(), item.mime, item.summary());
    }
    0
}

fn do_set(cap: CapabilityToken, request: ClipboardSetRequest) -> i32 {
    let body = encode_set_request(&request);
    let reply = match call_with_page(cap, ClipMsg::SET_CLIPBOARD, &body) {
        Ok(reply) => reply,
        Err(err) => return print_error(err.code()),
    };
    if reply.label == ClipMsg::ERROR {
        return print_error(reply.words[0]);
    }
    println!("ok {:08x}", reply.words[0] as u32);
    0
}

fn do_history(cap: CapabilityToken) -> i32 {
    let reply = ipc_call(cap, IpcMsg::with_label(ClipMsg::LIST_CLIPBOARD_HISTORY));
    if reply.label == ClipMsg::ERROR {
        return print_error(reply.words[0]);
    }
    if reply.words[1] == 0 || reply.caps[0] == CapabilityToken::INVALID {
        println!("(empty)");
        return 0;
    }
    let bytes = match take_reply_bytes(&reply) {
        Ok(bytes) => bytes,
        Err(err) => return print_error(err.code()),
    };
    let list = match decode_summary_list(&bytes) {
        Ok(list) => list,
        Err(err) => return print_error(err.code()),
    };
    if list.is_empty() {
        println!("(empty)");
        return 0;
    }
    for (index, item) in list.iter().enumerate() {
        println!(
            "{} {:>2} {:08x} {:<5} {}",
            if item.is_current { "*" } else { " " },
            index,
            item.id,
            item.kind.label(),
            item.summary
        );
    }
    0
}

fn do_use(cap: CapabilityToken, selector: &str) -> i32 {
    let (mode, value) = if selector.as_bytes().iter().all(u8::is_ascii_digit) {
        match parse_u64(selector) {
            Some(index) => (ClipMsg::SELECT_BY_INDEX, index),
            None => {
                print_usage();
                return 1;
            }
        }
    } else {
        let text = selector.strip_prefix("0x").unwrap_or(selector);
        match parse_hex_u32(text) {
            Some(id) => (ClipMsg::SELECT_BY_ID, id as u64),
            None => {
                print_usage();
                return 1;
            }
        }
    };
    let reply = ipc_call(
        cap,
        IpcMsg::with_label(ClipMsg::SELECT_CLIPBOARD_HISTORY_ITEM)
            .word(0, mode)
            .word(1, value),
    );
    if reply.label == ClipMsg::ERROR {
        return print_error(reply.words[0]);
    }
    println!("ok {:08x}", reply.words[0] as u32);
    0
}

fn do_simple(cap: CapabilityToken, label: u64, ok_text: &str) -> i32 {
    let reply = ipc_call(cap, IpcMsg::with_label(label));
    if reply.label == ClipMsg::ERROR {
        return print_error(reply.words[0]);
    }
    println!("{}", ok_text);
    0
}

fn call_with_page(cap: CapabilityToken, label: u64, body: &[u8]) -> Result<IpcMsg, ClipError> {
    if body.len() > SHM_PAGE {
        return Err(ClipError::TooLarge);
    }
    let (ptr, token) = shm_alloc().map_err(|_| ClipError::Internal)?;
    unsafe {
        core::ptr::copy_nonoverlapping(body.as_ptr(), ptr, body.len());
    }
    let reply = ipc_call(
        cap,
        IpcMsg::with_label(label)
            .word(0, body.len() as u64)
            .with_cap(0, token),
    );
    let _ = shm_free(token);
    Ok(reply)
}

fn take_reply_bytes(reply: &IpcMsg) -> Result<Vec<u8>, ClipError> {
    let len = reply.words[1] as usize;
    let token = reply.caps[0];
    if len == 0 || len > SHM_PAGE || token == CapabilityToken::INVALID {
        return Err(ClipError::Corrupt);
    }
    let ptr = shm_map(token).map_err(|_| ClipError::Corrupt)?;
    let bytes = unsafe { core::slice::from_raw_parts(ptr, len) }.to_vec();
    let _ = shm_free(token);
    Ok(bytes)
}

fn stdout_write(s: &str) {
    let mut data = s.as_bytes();
    while !data.is_empty() {
        match sunlight_libc::write(sunlight_libc::STDOUT, data) {
            Ok(n) if n > 0 => data = &data[n..],
            _ => break,
        }
    }
}

fn print_usage() {
    println!("Usage:");
    println!("  sunlight-clip get");
    println!("  sunlight-clip set <text>");
    println!("  sunlight-clip history");
    println!("  sunlight-clip use <index|hex-id|0xhex-id>");
    println!("  sunlight-clip clear");
    println!("  sunlight-clip clear-history");
    println!("  sunlight-clip set-file <path>");
    println!("  sunlight-clip set-files <path1> <path2> ...");
}

fn print_error(code: u64) -> i32 {
    match code {
        x if x == ClipMsg::ERR_NOT_FOUND => println!("not found"),
        x if x == ClipMsg::ERR_TOO_LARGE => println!("payload too large"),
        x if x == ClipMsg::ERR_UNSUPPORTED => println!("not implemented yet"),
        x if x == ClipMsg::ERR_CORRUPT => println!("corrupt clipboard state"),
        _ => println!("clipboard error {}", code),
    }
    1
}

fn parse_u64(text: &str) -> Option<u64> {
    let mut value = 0u64;
    if text.is_empty() {
        return None;
    }
    for byte in text.as_bytes() {
        if !byte.is_ascii_digit() {
            return None;
        }
        value = value.checked_mul(10)?;
        value = value.checked_add((byte - b'0') as u64)?;
    }
    Some(value)
}

fn parse_hex_u32(text: &str) -> Option<u32> {
    let mut value = 0u32;
    if text.is_empty() {
        return None;
    }
    for byte in text.as_bytes() {
        let nibble = match *byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => 10 + (byte - b'a'),
            b'A'..=b'F' => 10 + (byte - b'A'),
            _ => return None,
        };
        value = value.checked_mul(16)?;
        value = value.checked_add(nibble as u32)?;
    }
    Some(value)
}

unsafe fn collect_args(argc: u64, argv: *const *const u8, out: &mut [&str]) -> usize {
    let mut count = 0usize;
    for index in 0..(argc as usize).min(out.len()) {
        let ptr = *argv.add(index);
        if ptr.is_null() {
            break;
        }
        let mut len = 0usize;
        while *ptr.add(len) != 0 {
            len += 1;
        }
        out[count] = core::str::from_utf8_unchecked(core::slice::from_raw_parts(ptr, len));
        count += 1;
    }
    count
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    println!("sunlight-clip: PANIC");
    ProcessExit::exit(101);
}
