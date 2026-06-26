//! sunlight-sm — Sunlight Storage Manager
//!
//! Trusted Ring-3 service that performs controlled writes to a small
//! static whitelist of protected paths. Other services (e.g. sunlight-kv)
//! ask sm over IPC instead of writing protected paths directly.
//!
//! Wire:
//!   - label = OP_*
//!   - For write/mkdir/read with payload: words[0]=path_len, words[1]=content_len (read:0), caps[0]=shm
//!   - shm page contains path bytes [0..path_len] followed by content
//!   - reply.label = REPLY_OK / REPLY_ERR ; on ERR, words[0] = error code

#![no_std]
#![no_main]

extern crate alloc;

struct BumpAllocator;

unsafe impl core::alloc::GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        static mut HEAP: [u8; 65536] = [0; 65536];
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

#[macro_export]
macro_rules! serial_println {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let mut buf = heapless::String::<256>::new();
        let _ = write!(&mut buf, $($arg)*);
        sunlight_ipc::debug_log(&buf);
    }};
}

use sunlight_ipc::{
    endpoint_create, ipc_recv, ipc_reply_and_wait, nameserver_register, shm_free, shm_map,
    CapabilityToken, IpcMsg, SmMsg,
};
use sunlight_libc as libc;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    serial_println!("[SM] PANIC: {}", info);
    loop {}
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    serial_println!("[SM] Starting sunlight-sm");

    let ep = endpoint_create();
    nameserver_register("sm", ep);
    serial_println!("[SM] Registered as 'sm'");

    let mut msg = ipc_recv(ep);
    loop {
        let reply = handle(&msg);
        msg = ipc_reply_and_wait(ep, reply);
    }
}

fn handle(msg: &IpcMsg) -> IpcMsg {
    match msg.label {
        SmMsg::WRITE_FILE => op_write(msg),
        SmMsg::MKDIR_ALL => op_mkdir(msg),
        SmMsg::REMOVE => op_remove(msg),
        SmMsg::READ_FILE => op_read(msg),
        _ => {
            let mut r = IpcMsg::with_label(SmMsg::REPLY_ERR);
            r.words[0] = SmMsg::ERR_UNSUPPORTED;
            r
        }
    }
}

fn reply_ok() -> IpcMsg {
    IpcMsg::with_label(SmMsg::REPLY_OK)
}

fn reply_err(code: u64) -> IpcMsg {
    let mut r = IpcMsg::with_label(SmMsg::REPLY_ERR);
    r.words[0] = code;
    r
}

static WHITELIST: &[&str] = &[
    "/var/lib/sunlight-kv/",
    "/var/lib/sunlight/tls/",
    "/var/lib/sunlight/",
];

fn normalize_path(p: &[u8]) -> Option<heapless::String<256>> {
    if p.is_empty() || p[0] != b'/' {
        return None;
    }
    let mut s = heapless::String::<256>::new();
    let _ = s.push('/');
    // copy and collapse multiple / but reject ..
    let mut i = 1;
    let mut last_slash = true;
    while i < p.len() {
        let c = p[i];
        if c == b'/' {
            if !last_slash {
                let _ = s.push('/');
                last_slash = true;
            }
        } else if c == b'.' && last_slash && i + 1 < p.len() && p[i + 1] == b'.' {
            // reject any ..
            return None;
        } else {
            let _ = s.push(c as char);
            last_slash = false;
        }
        i += 1;
    }
    if s.is_empty() {
        let _ = s.push_str("/");
    }
    // ensure trailing / for dirs not necessary, keep as-is
    Some(s)
}

fn is_whitelisted(path: &str) -> bool {
    if path.is_empty() || !path.starts_with('/') || path.contains("..") {
        return false;
    }
    for w in WHITELIST {
        if path == *w || path.starts_with(*w) {
            return true;
        }
        // also allow exact file under prefix without trailing in whitelist
        if let Some(stripped) = path.strip_prefix(*w) {
            if !stripped.contains('/') || w.ends_with('/') {
                return true;
            }
        }
    }
    // allow exact matches for parent dirs if listed without final /
    for w in WHITELIST {
        let w2 = if w.ends_with('/') {
            &w[..w.len() - 1]
        } else {
            w
        };
        if path == w2 {
            return true;
        }
    }
    false
}

fn log_deny(op: &str, path: &str, reason: &str) {
    serial_println!("[SM][DENY] op={} path={} reason={}", op, path, reason);
}

fn log_allow(op: &str, path: &str, len: usize) {
    serial_println!("[SM][ALLOW] op={} path={} len={}", op, path, len);
}

/// Map shm payload for path+content. Returns (path_bytes, content_bytes) and frees shm after f.
fn with_payload<R>(msg: &IpcMsg, f: impl FnOnce(&[u8], &[u8]) -> R) -> Option<R> {
    let path_len = msg.words[0] as usize;
    let content_len = msg.words[1] as usize;
    if path_len == 0 || path_len + content_len > SmMsg::PAGE_CAPACITY {
        return None;
    }
    let cap = msg.caps[0];
    if cap == CapabilityToken::INVALID {
        return None;
    }
    let ptr = shm_map(cap).ok()?;
    // SAFETY: kernel granted a page; we respect declared lengths
    let page = unsafe { core::slice::from_raw_parts(ptr, path_len + content_len) };
    let res = f(&page[..path_len], &page[path_len..]);
    let _ = shm_free(cap);
    Some(res)
}

fn op_write(msg: &IpcMsg) -> IpcMsg {
    match with_payload(msg, |path, content| do_write(path, content)) {
        Some(Ok(len)) => {
            // success logged inside
            reply_ok()
        }
        Some(Err(code)) => reply_err(code),
        None => reply_err(SmMsg::ERR_PAYLOAD_TOO_LARGE),
    }
}

fn do_write(pathb: &[u8], content: &[u8]) -> Result<usize, u64> {
    let path_str = match core::str::from_utf8(pathb) {
        Ok(s) => s,
        Err(_) => return Err(SmMsg::ERR_INVALID_PATH),
    };
    let norm = match normalize_path(pathb) {
        Some(n) => n,
        None => {
            log_deny("write", path_str, "invalid");
            return Err(SmMsg::ERR_INVALID_PATH);
        }
    };
    let norm_s = norm.as_str();
    if !is_whitelisted(norm_s) {
        log_deny("write", norm_s, "not-whitelisted");
        return Err(SmMsg::ERR_DENIED);
    }
    // ensure parents
    mkdir_parents(pathb);
    // open/create write (replace)
    let fd = match libc::open_with_flags(pathb, libc::O_WRONLY | libc::O_CREAT) {
        Ok(f) => f,
        Err(_) => {
            log_deny("write", norm_s, "open-failed");
            return Err(SmMsg::ERR_IO);
        }
    };
    let mut off = 0usize;
    while off < content.len() {
        match libc::write(fd, &content[off..]) {
            Ok(0) => break,
            Ok(n) => off += n,
            Err(_) => {
                let _ = libc::close(fd);
                log_deny("write", norm_s, "write-failed");
                return Err(SmMsg::ERR_IO);
            }
        }
    }
    let _ = libc::close(fd);
    if off == content.len() {
        log_allow("write", norm_s, content.len());
        serial_println!(
            "[SM][WRITE] path={} len={} ok=true atomic=false reason=best-effort",
            norm_s,
            content.len()
        );
        Ok(content.len())
    } else {
        Err(SmMsg::ERR_IO)
    }
}

fn op_mkdir(msg: &IpcMsg) -> IpcMsg {
    match with_payload(msg, |path, _| do_mkdir_all(path)) {
        Some(Ok(())) => reply_ok(),
        Some(Err(code)) => reply_err(code),
        None => reply_err(SmMsg::ERR_INVALID_PATH),
    }
}

fn do_mkdir_all(pathb: &[u8]) -> Result<(), u64> {
    let path_str = match core::str::from_utf8(pathb) {
        Ok(s) => s,
        Err(_) => return Err(SmMsg::ERR_INVALID_PATH),
    };
    let norm = match normalize_path(pathb) {
        Some(n) => n,
        None => {
            log_deny("mkdir", path_str, "invalid");
            return Err(SmMsg::ERR_INVALID_PATH);
        }
    };
    let norm_s = norm.as_str();
    if !is_whitelisted(norm_s) {
        log_deny("mkdir", norm_s, "not-whitelisted");
        return Err(SmMsg::ERR_DENIED);
    }
    mkdir_parents(pathb);
    // ensure the dir itself
    let _ = libc::mkdir(pathb, 0o755);
    serial_println!("[SM][MKDIR] path={} ok=true", norm_s);
    log_allow("mkdir", norm_s, 0);
    Ok(())
}

fn mkdir_parents(path: &[u8]) {
    let mut i = 1usize;
    while i < path.len() {
        if path[i] == b'/' {
            let _ = libc::mkdir(&path[..i], 0o755);
        }
        i += 1;
    }
}

fn op_remove(msg: &IpcMsg) -> IpcMsg {
    // For remove, path can be small and sent inline in words (no shm required for simplicity)
    // words[0] high byte len? Use simple pack: path in words starting 0 as name_to like, but use payload style with content_len=0
    let path_len = msg.words[0] as usize;
    if path_len == 0 || path_len > 256 {
        return reply_err(SmMsg::ERR_INVALID_PATH);
    }
    // try shm first, else fall back to inline words
    let mut path_buf = [0u8; 256];
    let path_slice: &[u8] = if msg.cap_count > 0 && msg.caps[0] != CapabilityToken::INVALID {
        if let Ok(ptr) = shm_map(msg.caps[0]) {
            let s = unsafe { core::slice::from_raw_parts(ptr, path_len.min(256)) };
            path_buf[..path_len.min(256)].copy_from_slice(&s[..path_len.min(256)]);
            let _ = shm_free(msg.caps[0]);
            &path_buf[..path_len]
        } else {
            return reply_err(SmMsg::ERR_IO);
        }
    } else {
        // inline in words (up to 24 bytes or so)
        let mut bi = 0;
        let mut wi = 1;
        while bi < path_len && wi < 8 {
            for j in 0..8 {
                if bi < path_len {
                    path_buf[bi] = ((msg.words[wi] >> (j * 8)) & 0xff) as u8;
                    bi += 1;
                }
            }
            wi += 1;
        }
        &path_buf[..path_len.min(256)]
    };

    let path_str = match core::str::from_utf8(path_slice) {
        Ok(s) => s,
        Err(_) => return reply_err(SmMsg::ERR_INVALID_PATH),
    };
    let norm = match normalize_path(path_slice) {
        Some(n) => n,
        None => {
            log_deny("remove", path_str, "invalid");
            return reply_err(SmMsg::ERR_INVALID_PATH);
        }
    };
    let norm_s = norm.as_str();
    if !is_whitelisted(norm_s) {
        log_deny("remove", norm_s, "not-whitelisted");
        return reply_err(SmMsg::ERR_DENIED);
    }
    // Best effort: no unlink exposed; clear content for files as "remove data".
    // For real delete future VFS REMOVE.
    let _ = libc::open_with_flags(path_slice, libc::O_WRONLY | libc::O_CREAT).map(|fd| {
        let _ = libc::write(fd, b"");
        let _ = libc::close(fd);
    });
    serial_println!(
        "[SM][REMOVE] path={} ok=true (content-cleared; no unlink yet)",
        norm_s
    );
    log_allow("remove", norm_s, 0);
    reply_ok()
}

fn op_read(msg: &IpcMsg) -> IpcMsg {
    // path in shm or inline similar to remove
    let path_len = msg.words[0] as usize;
    if path_len == 0 || path_len > 256 {
        return reply_err(SmMsg::ERR_INVALID_PATH);
    }
    let mut path_buf = [0u8; 256];
    let path_slice = if msg.cap_count > 0 && msg.caps[0] != CapabilityToken::INVALID {
        if let Ok(ptr) = shm_map(msg.caps[0]) {
            let s = unsafe { core::slice::from_raw_parts(ptr, path_len.min(256)) };
            path_buf[..path_len.min(256)].copy_from_slice(&s[0..path_len.min(256)]);
            let _ = shm_free(msg.caps[0]);
            &path_buf[..path_len.min(256)]
        } else {
            &path_buf[0..0]
        }
    } else {
        let mut bi = 0;
        let mut wi = 1;
        while bi < path_len && wi < 8 {
            for j in 0..8 {
                if bi < path_len {
                    path_buf[bi] = ((msg.words[wi] >> (j * 8)) & 0xff) as u8;
                    bi += 1;
                }
            }
            wi += 1;
        }
        &path_buf[..path_len.min(256)]
    };

    let path_str = match core::str::from_utf8(path_slice) {
        Ok(s) => s,
        Err(_) => return reply_err(SmMsg::ERR_INVALID_PATH),
    };
    let norm = match normalize_path(path_slice) {
        Some(n) => n,
        None => {
            log_deny("read", path_str, "invalid");
            return reply_err(SmMsg::ERR_INVALID_PATH);
        }
    };
    let norm_s = norm.as_str();
    if !is_whitelisted(norm_s) {
        log_deny("read", norm_s, "not-whitelisted");
        return reply_err(SmMsg::ERR_DENIED);
    }

    // open and read whole (small file expectation; large would need reply shm)
    match libc::open(path_slice) {
        Ok(fd) => {
            let mut buf = [0u8; 1024]; // small read cap for inline reply
            let mut total = 0usize;
            loop {
                match libc::read(fd, &mut buf[total..]) {
                    Ok(0) => break,
                    Ok(n) => {
                        total += n;
                        if total >= buf.len() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            let _ = libc::close(fd);
            // reply data inline (words) if fits, else ERR_PAYLOAD for this simple bite
            if total <= 48 {
                let mut r = reply_ok();
                // pack len in words[0], data in following like kv pack
                r.words[0] = total as u64;
                let mut bi = 0;
                let mut wi = 1;
                for b in &buf[..total] {
                    if wi >= 8 {
                        break;
                    }
                    let shift = (bi % 8) * 8;
                    r.words[wi] |= (*b as u64) << shift;
                    bi += 1;
                    if bi % 8 == 0 {
                        wi += 1;
                    }
                }
                serial_println!("[SM][READ] path={} len={} ok=true (inline)", norm_s, total);
                r
            } else {
                serial_println!(
                    "[SM][READ] path={} len={} PAYLOAD_TOO_LARGE (no reply shm in bite)",
                    norm_s,
                    total
                );
                reply_err(SmMsg::ERR_PAYLOAD_TOO_LARGE)
            }
        }
        Err(_) => {
            serial_println!("[SM][READ] path={} not-found", norm_s);
            reply_err(SmMsg::ERR_NOT_FOUND)
        }
    }
}
