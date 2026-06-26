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
/// Privilege delegation: mint a capability for a target context (used by `runas`).
const OP_DELEGATE: u64 = 3;
/// Request the base set of VFS exec capabilities for an elevated session.
/// words[0]=target_uid; caps[0] must carry the caller's admin grant. Reply
/// words[0] = number of base exec prefixes granted.
const OP_BASE_CAPS: u64 = 4;
/// Path-execute access check. Same wire layout as `OP_CHECK` but tests the
/// execute right instead of read.
const OP_CHECK_EXEC: u64 = 5;
/// Reply label for a handled request.
const REPLY_OK: u64 = 1;
/// Reply label for a rejected/unknown request.
const REPLY_ERR: u64 = 0xff;

/// Maximum cached elevated sessions.
type Sessions = SessionStore<16>;
/// uid rules / gid rules / rules-per-subject bounds for the rule table.
type Rules = RuleTable<8, 8, 8>;

/// Standard executable directories an elevated session is granted execute
/// access to. Mirrors the kernel broker's `ELEVATED_EXEC_PREFIXES`; each entry
/// is a directory-prefix that acts as a `/bin/*` execution wildcard.
const BASE_EXEC_PREFIXES: [&str; 2] = ["/bin", "/usr/bin"];

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
        OP_CHECK_EXEC => {
            let uid = msg.words[0] as u32;
            let gid = msg.words[1] as u32;
            let mut path = unpack_str(&msg.words, 2);
            if path.is_empty() {
                let _ = path.push('/');
            }
            let allow = rules.has_access(uid, gid, path.as_str(), AccessFlags::EXECUTE);
            reply.label = REPLY_OK;
            reply.words[0] = allow as u64;
        }
        OP_BASE_CAPS => {
            // Base VFS-capability grant for an elevated session. Requires the
            // caller's admin grant in caps[0]; refuse otherwise so it cannot be
            // used to self-escalate.
            let target_uid = msg.words[0] as u32;
            let admin = msg.caps[0];
            if admin == sunlight_ipc::CapabilityToken::INVALID {
                serial_println!(
                    "[UAC] base-caps denied: no admin grant (uid={})",
                    target_uid
                );
                reply.label = REPLY_ERR;
            } else {
                // The actual kernel mint (grant_elevated_vfs) is driven by the
                // trusted broker pid via SYS_GRANT_CAPABILITY; here we report
                // how many base exec prefixes the policy grants so the client
                // can confirm the session is provisioned.
                // TODO(kernel-mint): forward minted tokens in reply.caps[..].
                serial_println!("[UAC] base-caps granted for uid={}", target_uid);
                reply.label = REPLY_OK;
                reply.words[0] = BASE_EXEC_PREFIXES.len() as u64;
            }
        }
        OP_DELEGATE => {
            // words[0]=target_uid, words[1..]=NUL-terminated command path.
            // caps[0] MUST carry the caller's prerequisite administrative grant
            // capability — without it the broker refuses to delegate, so an
            // unprivileged `runas` can never escalate on its own authority.
            let target_uid = msg.words[0] as u32;
            let admin = msg.caps[0];
            if admin == sunlight_ipc::CapabilityToken::INVALID {
                serial_println!(
                    "[UAC] delegate denied: no admin grant for uid={}",
                    target_uid
                );
                reply.label = REPLY_ERR;
            } else {
                // The kernel-trusted mint (sys_grant_capability) runs once the
                // broker is wired to CAPABILITY_BROKER_PID; here we acknowledge
                // the verified request and return the scoped capability.
                // TODO(kernel-mint): replace echo with a real minted token.
                serial_println!("[UAC] delegate granted for uid={}", target_uid);
                reply.label = REPLY_OK;
                reply.caps[0] = admin;
            }
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

    // Seed the base execute policy for elevated sessions explicitly. The root
    // "/" rule above already covers these by prefix, but seeding the standard
    // binary directories makes the `/bin/*` execution wildcard a first-class,
    // auditable rule that a non-root elevated session can also be granted.
    for prefix in BASE_EXEC_PREFIXES {
        let mut p = heapless::String::<64>::new();
        if p.push_str(prefix).is_ok() {
            let _ = rules.add_uid_rule(
                0,
                PathRule {
                    allowed_prefix: p,
                    flags: AccessFlags::READ_EXECUTE,
                },
            );
        }
    }

    serial_println!("[SunlightOS] uac OK");

    // Receive the first request, then loop: reply to the current request AND
    // atomically wait for the next one. `ipc_reply_and_wait` already returns the
    // next message, so we must NOT call `ipc_recv` again — doing so drops that
    // message and deadlocks the next client (the bug that hung capabilityctl on
    // its second IPC call).
    let mut msg = ipc_recv(ep);
    loop {
        let reply = handle(&msg, &mut store, &rules);
        msg = ipc_reply_and_wait(ep, reply);
    }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    serial_println!("[UAC] PANIC: {}", info);
    loop {}
}
