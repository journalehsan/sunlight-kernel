//! sunlight-kv daemon binary.
//!
//! This file supports two build modes via features:
//! - "host" (default): full std implementation using direct filesystem append-only log
//!   and Unix domain sockets for IPC (development / host tooling).
//! - "sunlightos": no_std SunlightOS service using kernel IPC (endpoint + nameserver).
//!   Backing store currently in-memory (full append-log + VFS integration is future work
//!   once a convenient VFS client for services is available).

#![cfg_attr(feature = "sunlightos", no_std)]
#![cfg_attr(feature = "sunlightos", no_main)]

#[cfg(feature = "sunlightos")]
extern crate alloc;

#[cfg(feature = "host")]
mod host {
    // The original host implementation lives here at runtime.
    // We keep the previous logic by including the body via conditional compilation
    // in the functions below.
}

#[cfg(feature = "host")]
fn main() {
    // Reproduce the previous std daemon behavior.
    use std::process;

    use env_logger::Env;
    use log::error;

    // We cannot easily call into the old daemon module without refactoring the whole
    // crate structure. For the host feature we provide a thin launcher that does the
    // same thing the previous main did (the rich implementation was in the crate before
    // the sunlightos porting step). In practice the host developer will usually run
    // via `cargo run -p sunlight-kv` which selects default features.

    // To keep the binary useful on host even after the split, we implement a small
    // compatible std main here that uses the library's daemon facilities if available,
    // otherwise falls back to a friendly message.

    // The library may expose run_daemon under host cfg; we call the public API when present.
    // For simplicity in this unified file we just exec the previous behavior by
    // delegating to a small inline version using the same env/config as before.

    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    // Use the library types when they are host-capable.
    // The daemon module is always compiled for host in this configuration.
    // If the user built without the rich daemon (e.g. only lib), we still produce a runnable bin.

    // The real host daemon lives in the original code paths; here we provide a working
    // entry that prints a hint and exits 0 for cargo install / cargo run ergonomics,
    // while the primary integration path remains the one exercised by `cargo run -p sunlight-kv`.

    // Better: actually run the UDS+file daemon using code from the daemon module.
    // We re-exported via the lib for host; call it.

    let cfg = sunlight_kv::daemon::DaemonConfig::default();
    if let Err(e) = sunlight_kv::daemon::run_daemon(cfg) {
        error!("sunlight-kv fatal: {}", e);
        process::exit(1);
    }
}

// -----------------------------------------------------------------------------
// SunlightOS (no_std) build
// -----------------------------------------------------------------------------

#[cfg(feature = "sunlightos")]
use alloc::string::String;

#[cfg(feature = "sunlightos")]
struct BumpAllocator;

#[cfg(feature = "sunlightos")]
unsafe impl core::alloc::GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        static mut HEAP: [u8; 128 * 1024] = [0; 128 * 1024];
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

#[cfg(feature = "sunlightos")]
#[global_allocator]
static BUMP: BumpAllocator = BumpAllocator;

#[cfg(feature = "sunlightos")]
macro_rules! serial_println {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let mut buf = heapless::String::<256>::new();
        let _ = write!(&mut buf, $($arg)*);
        sunlight_ipc::debug_log(&buf);
    }};
}

#[cfg(feature = "sunlightos")]
use sunlight_ipc::{
    debug_log, endpoint_create, ipc_recv, ipc_reply_and_wait, nameserver_register, IpcMsg,
};

#[cfg(feature = "sunlightos")]
#[no_mangle]
fn _start() -> ! {
    debug_log("[SUNLIGHT-KV] main() reached (no_std)\n");
    serial_println!("[SUNLIGHT-KV] Starting sunlight-kv (SunlightOS mode)");

    // Register with the init nameserver so other services and sunlightctl can find us.
    let ep = endpoint_create();
    nameserver_register("sunlight-kv", ep);
    serial_println!("[SUNLIGHT-KV] Registered as 'sunlight-kv'");

    // For the initial integration we accept connections and reply with a simple
    // protocol using the kernel IpcMsg (label encodes the op).
    // Real KV_PUT/GET/DELETE/SCAN with value transfer will use shm grants for larger
    // payloads in a follow-up. For now we implement a tiny in-memory store so the
    // service is useful immediately and sunlightd can manage it.

    // Very small static key->value table (demo quality, not the append-log engine).
    // Keys are packed as u64 (first 8 bytes) for simplicity in the register IPC model.
    const MAX_KV: usize = 32;
    static mut STORE: [Option<(u64, [u8; 128], usize)>; MAX_KV] = [const { None }; MAX_KV];
    static mut COUNT: usize = 0;

    fn pack_key(key: &str) -> u64 {
        let b = key.as_bytes();
        let mut out = 0u64;
        let n = if b.len() > 8 { 8 } else { b.len() };
        for i in 0..n {
            out |= (b[i] as u64) << (i * 8);
        }
        out
    }

    fn find_slot(key_packed: u64) -> Option<usize> {
        unsafe {
            for i in 0..COUNT {
                if let Some((k, _, _)) = STORE[i] {
                    if k == key_packed {
                        return Some(i);
                    }
                }
            }
        }
        None
    }

    // Simple op labels for the kernel IPC (chosen to not collide with existing services).
    // Clients (when ported) will send these labels.
    const KV_PUT: u64 = 0x4B01;      // 'K' 'V' + 1
    const KV_GET: u64 = 0x4B02;
    const KV_DELETE: u64 = 0x4B03;
    const KV_SCAN: u64 = 0x4B04;
    const KV_REPLY: u64 = 0x4BFF;
    const KV_ERROR: u64 = 0x4BEE;

    serial_println!("[SUNLIGHT-KV] Entering IPC loop");

    let mut msg = ipc_recv(ep);
    loop {
        let mut reply = IpcMsg::empty();
        reply.label = KV_REPLY;

        match msg.label {
            KV_PUT => {
                // For the register-based IPC the key is packed in word 0 as u64 (8 bytes max),
                // a small value can live in word 1 (or we would use shm for bigger values).
                let key_packed = msg.words[0];
                let val_len = (msg.words[1] & 0xff) as usize; // tiny demo: low byte length
                // Store a marker value for now (demo).
                unsafe {
                    if let Some(slot) = find_slot(key_packed) {
                        // overwrite
                        if let Some(entry) = &mut STORE[slot] {
                            entry.2 = val_len;
                        }
                    } else if COUNT < MAX_KV {
                        STORE[COUNT] = Some((key_packed, [0u8; 128], val_len));
                        COUNT += 1;
                    }
                }
                reply.words[0] = 0; // OK
            }
            KV_GET => {
                let key_packed = msg.words[0];
                unsafe {
                    if let Some(slot) = find_slot(key_packed) {
                        if let Some((_, _buf, len)) = STORE[slot] {
                            reply.words[0] = 1;           // found
                            reply.words[1] = len as u64;  // length hint
                        }
                    } else {
                        reply.label = KV_ERROR;
                        reply.words[0] = 1; // not found
                    }
                }
            }
            KV_DELETE => {
                let key_packed = msg.words[0];
                unsafe {
                    if let Some(slot) = find_slot(key_packed) {
                        // Simple tombstone simulation: just remove by swap-remove
                        STORE[slot] = STORE[COUNT - 1];
                        COUNT -= 1;
                        reply.words[0] = 0;
                    } else {
                        reply.label = KV_ERROR;
                    }
                }
            }
            KV_SCAN => {
                // Report count of live entries as a tiny scan result.
                unsafe {
                    reply.words[0] = COUNT as u64;
                }
            }
            _ => {
                // Unknown op or nameserver/etc noise — just ack.
                reply.label = KV_ERROR;
                reply.words[0] = 0xff;
            }
        }

        msg = ipc_reply_and_wait(ep, reply);
    }
}

#[cfg(feature = "sunlightos")]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    debug_log("[SUNLIGHT-KV] PANIC\n");
    loop {}
}
