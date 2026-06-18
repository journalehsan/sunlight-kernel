//! sunlight-storage — a trusted file-storage service for SunlightOS.
//!
//! A kv-style service that creates/overwrites files and makes directories on
//! behalf of *OS services* (e.g. writing `/etc/resolv.conf` or `/etc/tor/*`),
//! which the immutable-root policy otherwise denies. It is bounded two ways:
//!   1. **Trusted callers only** — every request is authorized by the kernel-
//!      stamped IPC `badge` (sender pid); only processes the kernel classifies
//!      as a service (`process_is_service`) are honored. User/app processes are
//!      denied.
//!   2. **No core-system writes** — the kernel filesystem policy still re-checks
//!      every write as the `sunlight-storage` actor and refuses `/boot`, `/bin`,
//!      `/sbin`, `/kernel`, `/services`, `/proc`, `/sys`.
//!
//! Wire protocol (one op per call): `label` = WRITE|MKDIR, `words[0]` = path
//! length, `words[1]` = content length (0 for MKDIR), `caps[0]` = a shared page
//! holding `path bytes` then `content bytes`. See `StorageMsg` and the
//! `storage_write`/`storage_mkdir` client helpers in the ipc crate.

#![no_std]
#![no_main]

extern crate alloc;

struct BumpAllocator;

unsafe impl core::alloc::GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        static mut HEAP: [u8; 64 * 1024] = [0; 64 * 1024];
        static mut NEXT: usize = 0;
        let align = layout.align();
        let aligned = (NEXT + align - 1) & !(align - 1);
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
    endpoint_create, ipc_recv, ipc_reply_and_wait, nameserver_register, shm_free, shm_map, IpcMsg,
    StorageMsg,
};
use sunlight_libc as libc;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    serial_println!("[STORAGE] PANIC: {}", info);
    loop {}
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    serial_println!("[STORAGE] Starting sunlight-storage v0.1");

    let ep = endpoint_create();
    nameserver_register("storage", ep);
    serial_println!("[STORAGE] Registered as 'storage'");
    serial_println!("[SunlightOS] storage OK");

    let mut msg = ipc_recv(ep);
    loop {
        let reply = handle(&msg);
        msg = ipc_reply_and_wait(ep, reply);
    }
}

fn handle(msg: &IpcMsg) -> IpcMsg {
    // Authorize the caller: only trusted OS services may use storage. `badge` is
    // the kernel-stamped sender pid (userland cannot forge it).
    if !libc::process_is_service(msg.badge) {
        serial_println!("[STORAGE] denied: caller pid={} is not a service", msg.badge);
        return IpcMsg::with_label(StorageMsg::REPLY_DENIED);
    }

    match msg.label {
        StorageMsg::WRITE => op_write(msg),
        StorageMsg::MKDIR => op_mkdir(msg),
        _ => IpcMsg::with_label(StorageMsg::REPLY_ERR),
    }
}

/// Map the shared payload page and return `(path, content)` slices, or `None` on
/// a bad request. The slices borrow the mapped page for the closure's duration.
fn with_payload<R>(msg: &IpcMsg, f: impl FnOnce(&[u8], &[u8]) -> R) -> Option<R> {
    let path_len = msg.words[0] as usize;
    let content_len = msg.words[1] as usize;
    if path_len == 0 || path_len + content_len > StorageMsg::PAGE_CAPACITY {
        return None;
    }
    let ptr = shm_map(msg.caps[0]).ok()?;
    // SAFETY: the kernel mapped a full page for `caps[0]`; we only read the
    // declared [0, path_len + content_len) prefix.
    let page = unsafe { core::slice::from_raw_parts(ptr, path_len + content_len) };
    let out = f(&page[..path_len], &page[path_len..path_len + content_len]);
    let _ = shm_free(msg.caps[0]);
    Some(out)
}

fn op_write(msg: &IpcMsg) -> IpcMsg {
    match with_payload(msg, |path, content| write_file(path, content)) {
        Some(Ok(())) => IpcMsg::with_label(StorageMsg::REPLY_OK),
        _ => IpcMsg::with_label(StorageMsg::REPLY_ERR),
    }
}

fn op_mkdir(msg: &IpcMsg) -> IpcMsg {
    match with_payload(msg, |path, _content| make_dir(path)) {
        Some(Ok(())) => IpcMsg::with_label(StorageMsg::REPLY_OK),
        _ => IpcMsg::with_label(StorageMsg::REPLY_ERR),
    }
}

/// Create-or-overwrite `path` with `content`. Parent dirs are created first.
/// A write at offset 0 replaces the file and truncates it to `content.len()`
/// (ramfs semantics), so an existing file is fully overwritten.
fn write_file(path: &[u8], content: &[u8]) -> Result<(), ()> {
    mkdir_parents(path);
    let fd = libc::open_with_flags(path, libc::O_WRONLY | libc::O_CREAT).map_err(|_| ())?;
    let mut off = 0;
    while off < content.len() {
        match libc::write(fd, &content[off..]) {
            Ok(0) => break,
            Ok(n) => off += n,
            Err(_) => {
                let _ = libc::close(fd);
                return Err(());
            }
        }
    }
    let _ = libc::close(fd);
    if off == content.len() {
        Ok(())
    } else {
        Err(())
    }
}

/// Make `path` (and any missing ancestors). Idempotent: an existing directory is
/// treated as success.
fn make_dir(path: &[u8]) -> Result<(), ()> {
    mkdir_parents(path);
    if libc::mkdir(path, 0o755).is_ok() {
        return Ok(());
    }
    // mkdir failed — accept it only if the path already exists (a directory).
    match libc::stat(path) {
        Ok(_) => Ok(()),
        Err(_) => Err(()),
    }
}

/// `mkdir -p` the ancestors of `path` (every prefix ending in '/'), ignoring
/// "already exists" failures.
fn mkdir_parents(path: &[u8]) {
    let mut i = 1; // skip the leading '/'
    while i < path.len() {
        if path[i] == b'/' {
            let _ = libc::mkdir(&path[..i], 0o755);
        }
        i += 1;
    }
}
