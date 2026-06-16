//! uac_service — SunlightOS User Access Control daemon.
//!
//! Spawned by `sunlightd` and registered with the init nameserver as `"uac"`.
//! It owns the elevated-session prompt cache and the capability rule table
//! (both modelled in the `sunlight_uac` library) and answers two requests:
//!
//!   * `OP_RUNAS`  — words[0]=caller_uid, words[1]=target_uid. Reply words[0]
//!                   is 0 (CacheUsed) or 1 (Prompted).
//!   * `OP_CHECK`  — words[0]=uid, words[1]=gid, words[2..]=NUL-terminated
//!                   path. Reply words[0] is 1 (allow) or 0 (deny) for read.

#![no_std]
#![no_main]

extern crate alloc;

struct BumpAllocator;

unsafe impl core::alloc::GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        static mut HEAP: [u8; 64 * 1024] = [0; 64 * 1024];
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
        sunlight_ipc::debug_log(&buf);
    }};
}

use sunlight_ipc::{
    endpoint_create, get_time_utc, ipc_recv, ipc_reply_and_wait, nameserver_register, IpcMsg,
};
use sunlight_uac::capability::{AccessFlags, PathRule, RuleTable};
use sunlight_uac::session::{runas, RunasOutcome, RunasRequest, SessionStore};

/// Elevation request (runas / prompt-cache).
const OP_RUNAS: u64 = 1;
/// Capability access check for a path.
const OP_CHECK: u64 = 2;
/// Reply label for a handled request.
const REPLY_OK: u64 = 1;
/// Reply label for a rejected/unknown request.
const REPLY_ERR: u64 = 0xff;

/// Maximum cached elevated sessions.
type Sessions = SessionStore<16>;
/// uid rules / gid rules / rules-per-subject bounds for the rule table.
type Rules = RuleTable<8, 8, 8>;

/// Decode a NUL-terminated little-endian string packed from `words[start..]`.
fn unpack_str(words: &[u64], start: usize) -> heapless::String<64> {
    let mut s = heapless::String::new();
    'outer: for &word in &words[start..] {
        for j in 0..8 {
            let byte = ((word >> (j * 8)) & 0xff) as u8;
            if byte == 0 {
                break 'outer;
            }
            if s.push(byte as char).is_err() {
                break 'outer;
            }
        }
    }
    s
}

fn handle(msg: &IpcMsg, store: &mut Sessions, rules: &Rules) -> IpcMsg {
    let mut reply = IpcMsg::empty();

    match msg.label {
        OP_RUNAS => {
            let caller_uid = msg.words[0] as u32;
            let target_uid = msg.words[1] as u32;
            let req = RunasRequest::new(caller_uid, target_uid, "ipc");
            match runas(&req, get_time_utc(), store) {
                Ok(RunasOutcome::CacheUsed) => {
                    reply.label = REPLY_OK;
                    reply.words[0] = 0;
                }
                Ok(RunasOutcome::Prompted) => {
                    reply.label = REPLY_OK;
                    reply.words[0] = 1;
                }
                Err(_) => reply.label = REPLY_ERR,
            }
        }
        OP_CHECK => {
            let uid = msg.words[0] as u32;
            let gid = msg.words[1] as u32;
            let mut path = unpack_str(&msg.words, 2);
            if path.is_empty() {
                let _ = path.push('/');
            }
            let allow = rules.has_access(uid, gid, path.as_str(), AccessFlags::READ);
            reply.label = REPLY_OK;
            reply.words[0] = allow as u64;
        }
        _ => reply.label = REPLY_ERR,
    }

    reply
}

#[no_mangle]
fn _start() -> ! {
    sunlight_ipc::debug_log("[UAC] uac_service main() reached\n");
    serial_println!("[UAC] Starting uac_service v0.1");

    let ep = endpoint_create();
    nameserver_register("uac", ep);
    serial_println!("[UAC] Registered as 'uac'");

    let mut store: Sessions = SessionStore::new();
    let mut rules: Rules = RuleTable::new();

    // Seed the default policy: root (uid 0) has full access to the whole tree.
    let mut root_prefix = heapless::String::<64>::new();
    let _ = root_prefix.push_str("/");
    let _ = rules.add_uid_rule(
        0,
        PathRule {
            allowed_prefix: root_prefix,
            flags: AccessFlags::ALL,
        },
    );

    serial_println!("[SunlightOS] uac OK");

    loop {
        let msg = ipc_recv(ep);
        let reply = handle(&msg, &mut store, &rules);
        let _ = ipc_reply_and_wait(ep, reply);
    }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    serial_println!("[UAC] PANIC: {}", info);
    loop {}
}
