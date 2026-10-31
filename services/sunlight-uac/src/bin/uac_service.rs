//! uac_service — SunlightOS User Access Control daemon.
//!
//! Spawned by `sunlightd` and registered with the init nameserver as `"uac"`.
//! It owns the elevated-session prompt cache and the capability rule table
//! (both modelled in the `sunlight_uac` library). Policy requests include:
//!
//!   * `OP_RUNAS`  — words[0]=caller_uid, words[1]=target_uid. Reply words[0]
//!                   is 0 (CacheUsed) or 1 (Prompted).
//!   * `OP_CHECK`  — words[0]=uid, words[1]=gid, words[2..]=NUL-terminated
//!                   path. Reply words[0] is 1 (allow) or 0 (deny) for read.
//! Delegation opcodes 3/4 exchange a delivered root password proof through the
//! kernel broker for one-use, exact-caller VFS authorizations.

#![no_std]
#![no_main]

extern crate alloc;
mod account_service;
#[cfg(feature = "session-reply-test")]
mod authenticated_reply;

use linked_list_allocator::LockedHeap;
use zeroize::Zeroize;

#[global_allocator]
static ALLOCATOR: LockedHeap = LockedHeap::empty();

const HEAP_SIZE: usize = 2 * 1024 * 1024;
static mut HEAP_MEM: [u8; HEAP_SIZE] = [0; HEAP_SIZE];

unsafe fn init_heap() {
    ALLOCATOR
        .lock()
        .init(core::ptr::addr_of_mut!(HEAP_MEM).cast::<u8>(), HEAP_SIZE);
}

macro_rules! serial_println {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let mut buf = heapless::String::<256>::new();
        let _ = write!(&mut buf, $($arg)*);
        sunlight_ipc::debug_log(&buf);
    }};
}

use sunlight_ipc::{
    endpoint_create, get_time_utc, ipc_recv, ipc_reply_and_wait, nameserver_register, shm_free,
    shm_map, IpcMsg,
};
use sunlight_uac::auth::{
    migrate_shadow_contents, verify_shadow_credentials, AUTH_FAILURE, AUTH_PASSWD_PATH,
    AUTH_PASSWORD_OP, AUTH_PASSWORD_SESSION_OP, AUTH_SHADOW_PATH, AUTH_SUCCESS, MAX_PASSWORD_LEN,
};
use sunlight_uac::capability::{AccessFlags, PathRule, RuleTable};
use sunlight_uac::session::{runas, RunasOutcome, RunasRequest, SessionStore};

/// Elevation request (runas / prompt-cache).
const OP_RUNAS: u64 = 1;
/// Capability access check for a path.
const OP_CHECK: u64 = 2;
/// Exchange a delivered root proof for one exact canonical executable path.
/// words[0]=0; words[1..4]=NUL-terminated path; caps[0]=password proof.
const OP_DELEGATE: u64 = 3;
/// Exchange the caller's delivered root proof for two base VFS authorizations.
/// words[0]=0; caps[0]=password proof. Reply caps contain new broker grants.
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

// Authentication and account mutations must use the same live backing store.
fn read_account_bytes(path: &str, out: &mut [u8]) -> Option<usize> {
    let mut bytes = account_service::read(path.as_bytes()).ok()?;
    if bytes.len() > out.len() {
        bytes.zeroize();
        return None;
    }
    let len = bytes.len();
    out[..len].copy_from_slice(&bytes);
    bytes.zeroize();
    Some(len)
}

fn migrate_development_shadow() {
    let mut passwd_data = [0u8; 8192];
    let mut shadow_data = [0u8; 8192];
    let Some(passwd_len) = read_account_bytes(AUTH_PASSWD_PATH, &mut passwd_data) else {
        serial_println!("[UAC] auth migration skipped: passwd unavailable");
        return;
    };
    let Some(shadow_len) = read_account_bytes(AUTH_SHADOW_PATH, &mut shadow_data) else {
        serial_println!("[UAC] auth migration skipped: shadow unavailable");
        return;
    };

    let Ok(mut migrated) =
        migrate_shadow_contents(&passwd_data[..passwd_len], &shadow_data[..shadow_len])
    else {
        serial_println!("[UAC] auth migration failed");
        return;
    };

    if migrated.as_bytes() != &shadow_data[..shadow_len] {
        if account_service::publish_shadow(migrated.as_bytes()).is_ok() {
            serial_println!("[UAC] shadow migration applied");
        } else {
            serial_println!("[UAC] shadow migration write failed");
        }
    }
    migrated.zeroize();
    shadow_data.zeroize();
}

fn handle_auth_password(msg: &IpcMsg, issue_session_grant: bool) -> IpcMsg {
    let mut reply = IpcMsg::empty();
    let username = unpack_str(&msg.words, 0);
    let token = msg.caps[0];
    if username.is_empty() || token == sunlight_ipc::CapabilityToken::INVALID {
        reply.label = AUTH_FAILURE;
        return reply;
    }

    let Ok(ptr) = shm_map(token) else {
        reply.label = AUTH_FAILURE;
        let _ = shm_free(token);
        return reply;
    };

    let mut password = [0u8; MAX_PASSWORD_LEN];
    let mut password_len = 0usize;
    while password_len < password.len() {
        let byte = unsafe { *ptr.add(password_len) };
        if byte == 0 {
            break;
        }
        password[password_len] = byte;
        password_len += 1;
    }
    unsafe {
        core::ptr::write_bytes(ptr, 0, MAX_PASSWORD_LEN.min(password_len.saturating_add(1)));
    }
    let _ = shm_free(token);

    let mut passwd_data = [0u8; 8192];
    let mut shadow_data = [0u8; 8192];
    let result = read_account_bytes(AUTH_PASSWD_PATH, &mut passwd_data).and_then(|passwd_len| {
        read_account_bytes(AUTH_SHADOW_PATH, &mut shadow_data).and_then(|shadow_len| {
            verify_shadow_credentials(
                &passwd_data[..passwd_len],
                &shadow_data[..shadow_len],
                username.as_bytes(),
                &password[..password_len],
            )
            .ok()
        })
    });
    shadow_data.zeroize();
    password.zeroize();

    if let Some(success) = result {
        reply.label = AUTH_SUCCESS;
        reply.words[0] = success.uid as u64;
        reply.words[1] = success.gid as u64;
        reply.word_count = 2;
        if issue_session_grant {
            let grant = unsafe {
                sunlight_libc::sys::syscall3(
                    sunlight_libc::sys::SYS_MINT_AUTH_SESSION_GRANT,
                    msg.badge,
                    success.uid as u64,
                    success.gid as u64,
                )
            };
            if grant == u64::MAX {
                reply.label = AUTH_FAILURE;
                return reply;
            }
            reply = reply.with_cap(0, sunlight_ipc::CapabilityToken(grant));
        }
    } else {
        reply.label = AUTH_FAILURE;
    }
    reply
}

fn handle(msg: &IpcMsg, store: &mut Sessions, rules: &Rules) -> IpcMsg {
    #[cfg(feature = "session-race-test")]
    if msg.label == 0xE300 {
        return account_service::consume_probe(msg);
    }
    let mut reply = IpcMsg::empty();

    match msg.label {
        sunlight_ipc::accounts::SNAPSHOT
        | sunlight_ipc::accounts::CHANGE_OWN_PASSWORD
        | sunlight_ipc::accounts::AUTHORIZE
        | sunlight_ipc::accounts::MUTATE_GROUP
        | sunlight_ipc::accounts::UPDATE_OWN_PROFILE => return account_service::handle(msg),
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
        AUTH_PASSWORD_OP => return handle_auth_password(msg, false),
        AUTH_PASSWORD_SESSION_OP => return handle_auth_password(msg, true),
        OP_BASE_CAPS | OP_DELEGATE => return sunlight_uac::delegation::handle(msg),
        _ => reply.label = REPLY_ERR,
    }

    reply
}

#[no_mangle]
fn _start() -> ! {
    unsafe {
        init_heap();
    }
    sunlight_ipc::debug_log("[UAC] uac_service main() reached\n");
    serial_println!("[UAC] Starting uac_service v0.1");

    let ep = endpoint_create();
    nameserver_register("uac", ep);
    serial_println!("[UAC] Registered as 'uac'");
    migrate_development_shadow();

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
        #[cfg(feature = "production-grant-test")]
        let reply = if msg.badge == 0xF4F0 && msg.label == 0xF11B07 && msg.cap_count == 0 {
            // Prior synchronous handler has returned and reply-and-wait has
            // settled publication. No delegation/staging registry exists here.
            let (cached, active) = store.diagnostic_inventory(get_time_utc());
            let heap = ALLOCATOR.lock();
            IpcMsg::with_label(0xF11B07)
                .word(0, msg.words[0])
                .word(1, cached as u64)
                .word(2, active as u64)
                .word(3, heap.used() as u64)
        } else {
            handle(&msg, &mut store, &rules)
        };
        #[cfg(not(feature = "production-grant-test"))]
        let reply = handle(&msg, &mut store, &rules);
        msg = ipc_reply_and_wait(ep, reply);
    }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    serial_println!("[UAC] PANIC: {}", info);
    loop {}
}
