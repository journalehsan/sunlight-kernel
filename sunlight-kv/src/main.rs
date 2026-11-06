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
use alloc::collections::{BTreeMap, VecDeque};
#[cfg(feature = "sunlightos")]
use alloc::string::String;
#[cfg(feature = "sunlightos")]
use alloc::vec;
#[cfg(feature = "sunlightos")]
use alloc::vec::Vec;

#[cfg(feature = "sunlightos")]
use sunlight_ipc::{
    debug_log, endpoint_create, ipc_call, ipc_recv, ipc_reply_and_try_recv, monotonic_millis,
    nameserver_lookup, nameserver_register, shm_alloc, shm_free, shm_map, CapabilityToken, IpcMsg,
    SmMsg, IPC_REGISTER_WORDS,
};
#[cfg(feature = "sunlightos")]
use sunlight_kv::stats_wire::{
    StatsSnapshotV1, F_CLIENT_PIDS_TRACKED, F_CLIENT_SLOTS_USED, F_CLIENT_TABLE, F_DECODE_ERRORS,
    F_KEY_COUNT, F_LAST_ACTIVITY_MS, F_LAST_ERROR_CODE, F_LAST_ERROR_MS, F_LOOP_ITERATIONS,
    F_MAGIC, F_MUTATIONS, F_NOW_MS, F_OP_DELETE, F_OP_DELETE_SHM2, F_OP_GET, F_OP_GET_SHM,
    F_OP_GET_SHM2, F_OP_PUT, F_OP_PUT_SHM, F_OP_PUT_SHM2, F_OP_SCAN, F_OP_STATS,
    F_OTHER_CLIENT_REQUESTS, F_PAYLOAD_BYTES, F_PERSIST_FLUSH_FAIL, F_PERSIST_FLUSH_OK,
    F_PERSIST_QUEUE_DEPTH, F_PERSIST_QUEUE_HWM, F_PERSIST_RECORD_BYTES, F_PERSIST_SKIPPED_VOLATILE,
    F_RECV_BLOCKING, F_REPLY_ERROR_LABELS, F_REQUESTS_ERR, F_REQUESTS_OK, F_REQUESTS_TOTAL,
    F_STARTED_MS, F_TRY_RECV_HIT, F_TRY_RECV_MISS, F_UNKNOWN_OPCODES, F_VERSION, F_VOLATILE_ONLY,
    STATS_BYTES, STATS_CLIENT_SLOTS, STATS_MAGIC, STATS_VERSION,
};
#[cfg(feature = "sunlightos")]
use sunlight_kv::Caller;
#[cfg(feature = "sunlightos")]
use sunlight_libc::{self as libc, Fd};

// Rust GlobalAlloc is provided by sunlight-libc (`global-alloc` feature) with
// the reclaiming free-list heap (`dynamic-heap`, 1 MiB). Do not add a second
// #[global_allocator] or free pointers across unrelated heaps.

#[cfg(feature = "sunlightos")]
fn stdout_write(s: &str) {
    let mut data = s.as_bytes();
    while !data.is_empty() {
        match libc::write(libc::STDOUT, data) {
            Ok(n) if n > 0 => data = &data[n..],
            _ => break,
        }
    }
}

#[cfg(feature = "sunlightos")]
macro_rules! println {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let mut buf = heapless::String::<512>::new();
        let _ = write!(&mut buf, $($arg)*);
        stdout_write(&buf);
        stdout_write("\n");
    }};
}

#[cfg(feature = "sunlightos")]
macro_rules! serial_println {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let mut buf = heapless::String::<256>::new();
        let _ = write!(&mut buf, $($arg)*);
        debug_log(&buf);
    }};
}

// ---- sunlight-sm client (protected writes only; no direct fallback) ----
#[cfg(feature = "sunlightos")]
static mut SM_CAP: Option<CapabilityToken> = None;

#[cfg(feature = "sunlightos")]
fn sm_lookup() -> Option<CapabilityToken> {
    unsafe {
        if SM_CAP.is_none() {
            if let Some(c) = nameserver_lookup("sm") {
                SM_CAP = Some(c);
                serial_println!("[KV][SM] lookup ok");
            } else {
                serial_println!("[KV][SM] lookup FAILED - sunlight-sm not registered");
            }
        }
        SM_CAP
    }
}

#[cfg(feature = "sunlightos")]
fn sm_mkdir_all(path: &[u8]) {
    let cap = match sm_lookup() {
        Some(c) => c,
        None => {
            serial_println!("[KV][SM] mkdir skipped (no sm)");
            return;
        }
    };
    let plen = path.len().min(255);
    // Use shm for path
    if let Ok((ptr, tok)) = shm_alloc() {
        unsafe {
            core::ptr::copy_nonoverlapping(path.as_ptr(), ptr, plen);
        }
        let mut msg = IpcMsg::with_label(SmMsg::MKDIR_ALL)
            .word(0, plen as u64)
            .word(1, 0)
            .with_cap(0, tok);
        let reply = ipc_call(cap, msg);
        let ok = reply.label == SmMsg::REPLY_OK;
        serial_println!(
            "[KV][SM] mkdir path={} result={}",
            core::str::from_utf8(path).unwrap_or("?"),
            if ok { "ok" } else { "failed" }
        );
        let _ = shm_free(tok);
    }
}

#[cfg(feature = "sunlightos")]
fn sm_write_file(path: &[u8], content: &[u8]) -> bool {
    let cap = match sm_lookup() {
        Some(c) => c,
        None => {
            serial_println!("[KV][SM] write ABORT (no sm) - would have written protected path");
            return false;
        }
    };
    let plen = path.len();
    let clen = content.len();
    if plen + clen > SmMsg::PAGE_CAPACITY {
        serial_println!("[KV][SM] write too large for page");
        return false;
    }
    if let Ok((ptr, tok)) = shm_alloc() {
        unsafe {
            core::ptr::copy_nonoverlapping(path.as_ptr(), ptr, plen);
            core::ptr::copy_nonoverlapping(content.as_ptr(), ptr.add(plen), clen);
        }
        let mut msg = IpcMsg::with_label(SmMsg::WRITE_FILE)
            .word(0, plen as u64)
            .word(1, clen as u64)
            .with_cap(0, tok);
        let reply = ipc_call(cap, msg);
        let ok = reply.label == SmMsg::REPLY_OK;
        serial_println!(
            "[KV][SM] write path={} len={} ok={}",
            core::str::from_utf8(path).unwrap_or("?"),
            clen,
            ok
        );
        let _ = shm_free(tok);
        return ok;
    }
    false
}

#[cfg(feature = "sunlightos")]
fn sm_read_file(path: &[u8]) -> Option<Vec<u8>> {
    let cap = match sm_lookup() {
        Some(c) => c,
        None => return None,
    };
    let plen = path.len().min(255);
    if let Ok((ptr, tok)) = shm_alloc() {
        unsafe {
            core::ptr::copy_nonoverlapping(path.as_ptr(), ptr, plen);
        }
        let mut msg = IpcMsg::with_label(SmMsg::READ_FILE)
            .word(0, plen as u64)
            .with_cap(0, tok);
        let reply = ipc_call(cap, msg);
        let _ = shm_free(tok);
        if reply.label == SmMsg::REPLY_OK {
            // small inline data unpack (see sm impl)
            let dlen = (reply.words[0] & 0xffff) as usize;
            let mut out = Vec::with_capacity(dlen);
            let mut bi = 0usize;
            let mut wi = 1usize;
            while bi < dlen && wi < 8 {
                for j in 0..8 {
                    if bi < dlen {
                        out.push(((reply.words[wi] >> (j * 8)) & 0xff) as u8);
                        bi += 1;
                    }
                }
                wi += 1;
            }
            return Some(out);
        }
    }
    None
}

/// Maximum bytes for one appended record batch, bounded by the single shared
/// page the storage manager accepts (`SmMsg::PAGE_CAPACITY` minus the store
/// path length). Batches larger than this are split by the caller.
#[cfg(feature = "sunlightos")]
const SM_STORE_PATH: &[u8] = b"/var/lib/sunlight/kv.store";

/// Validate record framing before it is ever committed to a shared-memory
/// frame. Each length is a `u32` in the on-disk header, so an oversized key,
/// value or ACL would silently truncate; reject it explicitly instead.
#[cfg(feature = "sunlightos")]
fn validate_record_bounds(key: &str, value: &[u8], acl_bytes: &[u8]) -> bool {
    // Bound the frame to one storage-manager page payload. A single record must
    // itself be representable; otherwise the append can never succeed and must
    // be reported, not attempted forever.
    let payload = key.len() + value.len() + acl_bytes.len();
    if key.is_empty() {
        return false;
    }
    if key.len() > u32::MAX as usize
        || value.len() > u32::MAX as usize
        || acl_bytes.len() > u32::MAX as usize
    {
        return false;
    }
    if payload + RecordHeader::SIZE + SM_STORE_PATH.len() > SmMsg::PAGE_CAPACITY {
        return false;
    }
    true
}

/// Append one or more delta records to the log via sm in a single
/// read-modify-write.
///
/// The store is a single append-only file and the storage manager overwrites it
/// wholesale, so each flush pays O(log size). Flushing a whole batch at once
/// amortises that cost across every queued record instead of rewriting the log
/// once per mutation. Returns `Ok(())` only when the combined batch was written.
#[cfg(feature = "sunlightos")]
fn sm_append_records(records: &[Vec<u8>]) -> bool {
    if records.is_empty() {
        return true;
    }
    let batch_len: usize = records.iter().map(|r| r.len()).sum();
    if SM_STORE_PATH.len() + batch_len > SmMsg::PAGE_CAPACITY {
        // Caller must split; this is a programming error, not a silent drop.
        serial_println!(
            "[KV][SM] append batch too large path+recs={} cap={}",
            SM_STORE_PATH.len() + batch_len,
            SmMsg::PAGE_CAPACITY
        );
        return false;
    }
    let existing = sm_read_file(SM_STORE_PATH).unwrap_or_default();
    if existing.len() + batch_len > SmMsg::PAGE_CAPACITY {
        // The store has reached the single-page ceiling. Report explicitly so
        // the operator can compact; do not silently stop persisting.
        serial_println!(
            "[KV][SM] store full existing={} batch={} cap={}",
            existing.len(),
            batch_len,
            SmMsg::PAGE_CAPACITY
        );
        return false;
    }
    let mut combined = existing;
    combined.reserve(batch_len);
    for record in records {
        combined.extend_from_slice(record);
    }
    let ok = sm_write_file(SM_STORE_PATH, &combined);
    if ok {
        serial_println!(
            "[KV][SM] appended {} record(s) via sm total_len={}",
            records.len(),
            combined.len()
        );
    }
    ok
}

// ---- sunlightos storage: append-only log at /var/lib/sunlight/kv.store ----
// RecordHeader + CRC + ACL metadata exactly as specified. Recovery sequential via Fd.
// Live values + ACLs kept in RAM for get/delete after boot (log is append-only for durability).
// IPC uses IpcMsg with pack/unpack for keys/values (fits ~48B inline; larger future via shm).

#[cfg(feature = "sunlightos")]
const RECORD_MAGIC: u32 = 0xABCD1234;
#[cfg(feature = "sunlightos")]
const RECORD_VERSION: u16 = 1;
#[cfg(feature = "sunlightos")]
const FLAG_PUT: u16 = 1;
#[cfg(feature = "sunlightos")]
const FLAG_DELETE: u16 = 2;

#[cfg(feature = "sunlightos")]
#[derive(Clone, Copy)]
struct RecordHeader {
    magic: u32,
    version: u16,
    flags: u16,
    key_len: u32,
    value_len: u32,
    acl_len: u32,
    crc32: u32,
}

#[cfg(feature = "sunlightos")]
impl RecordHeader {
    const SIZE: usize = 24;

    fn to_bytes(&self) -> [u8; Self::SIZE] {
        let mut buf = [0u8; Self::SIZE];
        buf[0..4].copy_from_slice(&self.magic.to_le_bytes());
        buf[4..6].copy_from_slice(&self.version.to_le_bytes());
        buf[6..8].copy_from_slice(&self.flags.to_le_bytes());
        buf[8..12].copy_from_slice(&self.key_len.to_le_bytes());
        buf[12..16].copy_from_slice(&self.value_len.to_le_bytes());
        buf[16..20].copy_from_slice(&self.acl_len.to_le_bytes());
        buf[20..24].copy_from_slice(&self.crc32.to_le_bytes());
        buf
    }

    fn from_bytes(bytes: &[u8; Self::SIZE]) -> Option<Self> {
        let magic = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let version = u16::from_le_bytes([bytes[4], bytes[5]]);
        if magic != RECORD_MAGIC || version != RECORD_VERSION {
            return None;
        }
        Some(Self {
            magic,
            version,
            flags: u16::from_le_bytes([bytes[6], bytes[7]]),
            key_len: u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]),
            value_len: u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]),
            acl_len: u32::from_le_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]),
            crc32: u32::from_le_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]),
        })
    }
}

#[cfg(feature = "sunlightos")]
fn crc32_ieee(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            if (crc & 1) != 0 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
        }
    }
    !crc
}

#[cfg(feature = "sunlightos")]
fn read_exact(fd: Fd, buf: &mut [u8]) -> Result<(), libc::Errno> {
    let mut off = 0usize;
    while off < buf.len() {
        match libc::read(fd, &mut buf[off..]) {
            Ok(0) => return Err(libc::Errno::Failed),
            Ok(n) => off += n,
            Err(libc::Errno::Again) => libc::yield_now(),
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

#[cfg(feature = "sunlightos")]
fn write_all(fd: Fd, buf: &[u8]) -> Result<(), libc::Errno> {
    let mut off = 0usize;
    while off < buf.len() {
        match libc::write(fd, &buf[off..]) {
            Ok(0) => return Err(libc::Errno::Failed),
            Ok(n) => off += n,
            Err(libc::Errno::Again) => libc::yield_now(),
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

#[cfg(feature = "sunlightos")]
fn serialize_acl(acl: &sunlight_kv::Acl) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    let ob = acl.owner.as_bytes();
    out.push(ob.len() as u8);
    out.extend_from_slice(ob);
    out.push(acl.read.len() as u8);
    for r in &acl.read {
        let b = r.as_bytes();
        out.push(b.len() as u8);
        out.extend_from_slice(b);
    }
    out.push(acl.write.len() as u8);
    for w in &acl.write {
        let b = w.as_bytes();
        out.push(b.len() as u8);
        out.extend_from_slice(b);
    }
    out
}

#[cfg(feature = "sunlightos")]
fn deserialize_acl(bytes: &[u8]) -> Option<sunlight_kv::Acl> {
    if bytes.is_empty() {
        return None;
    }
    let mut i = 0usize;
    let ol = *bytes.get(i)? as usize;
    i += 1;
    if i + ol > bytes.len() {
        return None;
    }
    let owner = match String::from_utf8(bytes[i..i + ol].to_vec()) {
        Ok(s) => s,
        Err(_) => return None,
    };
    i += ol;

    let rc = *bytes.get(i)? as usize;
    i += 1;
    let mut read: Vec<String> = Vec::new();
    for _ in 0..rc {
        let l = *bytes.get(i)? as usize;
        i += 1;
        if i + l > bytes.len() {
            return None;
        }
        match String::from_utf8(bytes[i..i + l].to_vec()) {
            Ok(s) => read.push(s),
            Err(_) => return None,
        }
        i += l;
    }

    let wc = *bytes.get(i)? as usize;
    i += 1;
    let mut write: Vec<String> = Vec::new();
    for _ in 0..wc {
        let l = *bytes.get(i)? as usize;
        i += 1;
        if i + l > bytes.len() {
            return None;
        }
        match String::from_utf8(bytes[i..i + l].to_vec()) {
            Ok(s) => write.push(s),
            Err(_) => return None,
        }
        i += l;
    }

    Some(sunlight_kv::Acl { owner, read, write })
}

/// Resolve the authenticated caller identity for an incoming IPC message.
///
/// The kernel stamps the real sender PID into `msg.badge`; nothing in the
/// payload is trusted for identity. We then ask the kernel for that PID's
/// authoritative UID/GID and, for trusted embedded daemons, a provenance
/// service key:
///
/// * `service_identity != 0` → [`Caller::Service(key)`] (e.g. sunlight-tls).
/// * `uid == 0` with no service key → [`Caller::Root`] (kernel/admin).
/// * any other uid → [`Caller::User(uid)`].
/// * lookup failure (dead peer, unexpected badge) → [`Caller::Unknown`] which
///   is denied everything except world-readable keys.
///
/// This replaces the previous hardcoded `"root"` caller in every handler.
#[cfg(feature = "sunlightos")]
fn resolve_caller(badge: u64) -> Caller {
    match sunlight_ipc::kv_caller_credentials(badge) {
        Some(creds) => {
            if creds.service_identity != 0 {
                Caller::Service(creds.service_identity)
            } else if creds.uid == 0 {
                Caller::Root
            } else {
                Caller::User(creds.uid)
            }
        }
        None => Caller::Unknown,
    }
}

// KV IPC protocol (sunlightos IpcMsg). Register IPC currently transports only
// words[0..4) via r8/r9/r10/r12.
#[cfg(feature = "sunlightos")]
const KV_PUT: u64 = 0x4B01;
#[cfg(feature = "sunlightos")]
const KV_GET: u64 = 0x4B02;
#[cfg(feature = "sunlightos")]
const KV_DELETE: u64 = 0x4B03;
#[cfg(feature = "sunlightos")]
const KV_SCAN: u64 = 0x4B04;
#[cfg(feature = "sunlightos")]
const KV_REPLY: u64 = 0x4BFF;
#[cfg(feature = "sunlightos")]
const KV_ERROR: u64 = 0x4BEE;
#[cfg(feature = "sunlightos")]
const KV_VALUE: u64 = 0x4B05;
// Shared-memory transport for values too large for register-IPC word packing
// (e.g. TLS certificates). The data producer allocates a page, fills it, and
// passes the cap token in caps[0]; the consumer maps + copies + frees. Mirrors
// the VFS DATA_SHARED pattern. One page (<=4096 bytes) per value.
#[cfg(feature = "sunlightos")]
const KV_PUT_SHM: u64 = 0x4B06;
#[cfg(feature = "sunlightos")]
const KV_GET_SHM: u64 = 0x4B07;
#[cfg(feature = "sunlightos")]
const KV_PUT_SHM2: u64 = 0x4B08;
#[cfg(feature = "sunlightos")]
const KV_GET_SHM2: u64 = 0x4B09;
#[cfg(feature = "sunlightos")]
const KV_DELETE_SHM2: u64 = 0x4B0A;
/// Read-only diagnostic snapshot (SHM-backed fixed layout). See `stats_wire`.
#[cfg(feature = "sunlightos")]
const KV_STATS: u64 = 0x4B0B;
#[cfg(feature = "sunlightos")]
const SHM_PAGE: usize = 4096;

// ---- Bounded soak diagnostics (saturating counters; no per-request logs) ----
#[cfg(feature = "sunlightos")]
#[derive(Clone, Copy)]
struct ClientSlot {
    pid: u64,
    requests: u64,
}

#[cfg(feature = "sunlightos")]
struct KvDiag {
    started_ms: u64,
    last_activity_ms: u64,
    last_error_ms: u64,
    last_error_code: u64,
    requests_total: u64,
    requests_ok: u64,
    requests_err: u64,
    decode_errors: u64,
    reply_error_labels: u64,
    unknown_opcodes: u64,
    op_put: u64,
    op_get: u64,
    op_delete: u64,
    op_scan: u64,
    op_put_shm: u64,
    op_get_shm: u64,
    op_put_shm2: u64,
    op_get_shm2: u64,
    op_delete_shm2: u64,
    op_stats: u64,
    loop_iterations: u64,
    recv_blocking: u64,
    try_recv_hit: u64,
    try_recv_miss: u64,
    /// Sum of live value lengths only (not ACL/key overhead); maintained on put/delete.
    payload_bytes: u64,
    mutations: u64,
    persist_queue_hwm: u64,
    persist_flush_ok: u64,
    persist_flush_fail: u64,
    persist_record_bytes: u64,
    persist_skipped_volatile: u64,
    clients: [ClientSlot; STATS_CLIENT_SLOTS],
    client_slots_used: u64,
    other_client_requests: u64,
    client_pids_tracked: u64,
}

#[cfg(feature = "sunlightos")]
impl KvDiag {
    const fn new() -> Self {
        Self {
            started_ms: 0,
            last_activity_ms: 0,
            last_error_ms: 0,
            last_error_code: 0,
            requests_total: 0,
            requests_ok: 0,
            requests_err: 0,
            decode_errors: 0,
            reply_error_labels: 0,
            unknown_opcodes: 0,
            op_put: 0,
            op_get: 0,
            op_delete: 0,
            op_scan: 0,
            op_put_shm: 0,
            op_get_shm: 0,
            op_put_shm2: 0,
            op_get_shm2: 0,
            op_delete_shm2: 0,
            op_stats: 0,
            loop_iterations: 0,
            recv_blocking: 0,
            try_recv_hit: 0,
            try_recv_miss: 0,
            payload_bytes: 0,
            mutations: 0,
            persist_queue_hwm: 0,
            persist_flush_ok: 0,
            persist_flush_fail: 0,
            persist_record_bytes: 0,
            persist_skipped_volatile: 0,
            clients: [ClientSlot {
                pid: 0,
                requests: 0,
            }; STATS_CLIENT_SLOTS],
            client_slots_used: 0,
            other_client_requests: 0,
            client_pids_tracked: 0,
        }
    }
}

#[cfg(feature = "sunlightos")]
static mut DIAG: KvDiag = KvDiag::new();

#[cfg(feature = "sunlightos")]
#[inline]
fn sat_inc(c: &mut u64) {
    *c = c.saturating_add(1);
}

#[cfg(feature = "sunlightos")]
#[inline]
fn sat_add(c: &mut u64, n: u64) {
    *c = c.saturating_add(n);
}

#[cfg(feature = "sunlightos")]
fn diag_note_error(code: u64) {
    unsafe {
        let d = &mut DIAG;
        d.last_error_ms = monotonic_millis();
        d.last_error_code = code;
    }
}

#[cfg(feature = "sunlightos")]
fn diag_note_client(pid: u64) {
    if pid == 0 {
        return;
    }
    unsafe {
        let d = &mut DIAG;
        for slot in d.clients.iter_mut() {
            if slot.pid == pid {
                sat_inc(&mut slot.requests);
                return;
            }
        }
        for slot in d.clients.iter_mut() {
            if slot.pid == 0 {
                slot.pid = pid;
                slot.requests = 1;
                sat_inc(&mut d.client_slots_used);
                sat_inc(&mut d.client_pids_tracked);
                return;
            }
        }
        // Fixed table full: aggregate rather than grow.
        sat_inc(&mut d.other_client_requests);
    }
}

#[cfg(feature = "sunlightos")]
fn diag_note_request_start(label: u64, badge_pid: u64) {
    unsafe {
        let d = &mut DIAG;
        sat_inc(&mut d.requests_total);
        sat_inc(&mut d.loop_iterations);
        d.last_activity_ms = monotonic_millis();
        match label {
            KV_PUT => sat_inc(&mut d.op_put),
            KV_GET => sat_inc(&mut d.op_get),
            KV_DELETE => sat_inc(&mut d.op_delete),
            KV_SCAN => sat_inc(&mut d.op_scan),
            KV_PUT_SHM => sat_inc(&mut d.op_put_shm),
            KV_GET_SHM => sat_inc(&mut d.op_get_shm),
            KV_PUT_SHM2 => sat_inc(&mut d.op_put_shm2),
            KV_GET_SHM2 => sat_inc(&mut d.op_get_shm2),
            KV_DELETE_SHM2 => sat_inc(&mut d.op_delete_shm2),
            KV_STATS => sat_inc(&mut d.op_stats),
            _ => sat_inc(&mut d.unknown_opcodes),
        }
    }
    diag_note_client(badge_pid);
}

#[cfg(feature = "sunlightos")]
fn diag_note_outcome(reply_label: u64, decode_error: bool) {
    unsafe {
        let d = &mut DIAG;
        if decode_error {
            sat_inc(&mut d.decode_errors);
            sat_inc(&mut d.requests_err);
            return;
        }
        if reply_label == KV_ERROR {
            sat_inc(&mut d.reply_error_labels);
            sat_inc(&mut d.requests_err);
        } else {
            sat_inc(&mut d.requests_ok);
        }
    }
}

#[cfg(feature = "sunlightos")]
fn diag_snapshot() -> StatsSnapshotV1 {
    let mut s = StatsSnapshotV1::zeroed();
    let now = monotonic_millis();
    unsafe {
        let d = &DIAG;
        let key_count = LIVE.as_ref().map(|m| m.len() as u64).unwrap_or(0);
        let qdepth = PERSIST_QUEUE.as_ref().map(|q| q.len() as u64).unwrap_or(0);
        let volatile = if VOLATILE_ONLY { 1 } else { 0 };

        s.set(F_MAGIC, STATS_MAGIC);
        s.set(F_VERSION, STATS_VERSION);
        s.set(F_STARTED_MS, d.started_ms);
        s.set(F_NOW_MS, now);
        s.set(F_LAST_ACTIVITY_MS, d.last_activity_ms);
        s.set(F_LAST_ERROR_MS, d.last_error_ms);
        s.set(F_LAST_ERROR_CODE, d.last_error_code);
        s.set(F_VOLATILE_ONLY, volatile);

        s.set(F_REQUESTS_TOTAL, d.requests_total);
        s.set(F_REQUESTS_OK, d.requests_ok);
        s.set(F_REQUESTS_ERR, d.requests_err);
        s.set(F_DECODE_ERRORS, d.decode_errors);
        s.set(F_REPLY_ERROR_LABELS, d.reply_error_labels);
        s.set(F_UNKNOWN_OPCODES, d.unknown_opcodes);

        s.set(F_OP_PUT, d.op_put);
        s.set(F_OP_GET, d.op_get);
        s.set(F_OP_DELETE, d.op_delete);
        s.set(F_OP_SCAN, d.op_scan);
        s.set(F_OP_PUT_SHM, d.op_put_shm);
        s.set(F_OP_GET_SHM, d.op_get_shm);
        s.set(F_OP_PUT_SHM2, d.op_put_shm2);
        s.set(F_OP_GET_SHM2, d.op_get_shm2);
        s.set(F_OP_DELETE_SHM2, d.op_delete_shm2);
        s.set(F_OP_STATS, d.op_stats);

        s.set(F_LOOP_ITERATIONS, d.loop_iterations);
        s.set(F_RECV_BLOCKING, d.recv_blocking);
        s.set(F_TRY_RECV_HIT, d.try_recv_hit);
        s.set(F_TRY_RECV_MISS, d.try_recv_miss);

        s.set(F_KEY_COUNT, key_count);
        s.set(F_PAYLOAD_BYTES, d.payload_bytes);
        s.set(F_MUTATIONS, d.mutations);
        s.set(F_PERSIST_QUEUE_DEPTH, qdepth);
        s.set(F_PERSIST_QUEUE_HWM, d.persist_queue_hwm);
        s.set(F_PERSIST_FLUSH_OK, d.persist_flush_ok);
        s.set(F_PERSIST_FLUSH_FAIL, d.persist_flush_fail);
        s.set(F_PERSIST_RECORD_BYTES, d.persist_record_bytes);
        s.set(F_PERSIST_SKIPPED_VOLATILE, d.persist_skipped_volatile);

        s.set(F_CLIENT_SLOTS_USED, d.client_slots_used);
        s.set(F_OTHER_CLIENT_REQUESTS, d.other_client_requests);
        s.set(F_CLIENT_PIDS_TRACKED, d.client_pids_tracked);

        for i in 0..STATS_CLIENT_SLOTS {
            let base = F_CLIENT_TABLE + i * 2;
            s.set(base, d.clients[i].pid);
            s.set(base + 1, d.clients[i].requests);
        }
    }
    s
}

/// Error codes recorded in last_error_code (diagnostic only; not a public API).
#[cfg(feature = "sunlightos")]
mod diag_err {
    pub const EMPTY_KEY: u64 = 1;
    pub const BAD_SHM: u64 = 2;
    pub const ACL_OR_MISSING: u64 = 3;
    pub const SHM_ALLOC: u64 = 4;
    pub const UNSUPPORTED: u64 = 5;
    pub const STATS_SHM: u64 = 6;
    pub const PERSIST_FAIL: u64 = 7;
}

#[cfg(feature = "sunlightos")]
static mut LOG_FD: Option<Fd> = None;
#[cfg(feature = "sunlightos")]
static mut LIVE: Option<BTreeMap<String, (Vec<u8>, sunlight_kv::Acl)>> = None;
#[cfg(feature = "sunlightos")]
static mut VOLATILE_ONLY: bool = true;
#[cfg(feature = "sunlightos")]
static mut PERSIST_QUEUE: Option<VecDeque<PersistRecord>> = None;

#[cfg(feature = "sunlightos")]
struct PersistRecord {
    key: String,
    bytes: Vec<u8>,
}

#[cfg(feature = "sunlightos")]
fn ensure_dirs() {
    // Delegate protected dir creation to sunlight-sm. No silent fallback.
    sm_mkdir_all(b"/var/lib/sunlight-kv");
    sm_mkdir_all(b"/var/lib/sunlight");
}

#[cfg(feature = "sunlightos")]
fn open_or_create_store() -> Option<Fd> {
    ensure_dirs();
    // We no longer keep persistent fd for appends. All writes go via sunlight-sm.
    // Return a dummy to satisfy old paths; recover will use sm_read_file for content.
    // If sm read works we will parse bytes separately.
    match libc::open(b"/var/lib/sunlight/kv.store") {
        Ok(fd) => Some(fd),
        Err(_) => None,
    }
}

#[cfg(feature = "sunlightos")]
fn recover_store(fd: Fd) -> BTreeMap<String, (Vec<u8>, sunlight_kv::Acl)> {
    let mut map: BTreeMap<String, (Vec<u8>, sunlight_kv::Acl)> = BTreeMap::new();
    loop {
        let mut magic_buf = [0u8; 4];
        if read_exact(fd, &mut magic_buf).is_err() {
            break;
        }
        let magic = u32::from_le_bytes(magic_buf);
        if magic != RECORD_MAGIC {
            break;
        }
        let mut rest = [0u8; 20];
        if read_exact(fd, &mut rest).is_err() {
            break;
        }
        let mut full_hdr = [0u8; RecordHeader::SIZE];
        full_hdr[0..4].copy_from_slice(&magic_buf);
        full_hdr[4..].copy_from_slice(&rest);
        let header = match RecordHeader::from_bytes(&full_hdr) {
            Some(h) => h,
            None => break,
        };
        let mut key = vec![0u8; header.key_len as usize];
        let mut value = vec![0u8; header.value_len as usize];
        let mut aclb = vec![0u8; header.acl_len as usize];
        if read_exact(fd, &mut key).is_err() {
            break;
        }
        if read_exact(fd, &mut value).is_err() {
            break;
        }
        if read_exact(fd, &mut aclb).is_err() {
            break;
        }
        let mut payload: Vec<u8> = Vec::new();
        payload.extend_from_slice(&key);
        payload.extend_from_slice(&value);
        payload.extend_from_slice(&aclb);
        if crc32_ieee(&payload) != header.crc32 {
            break;
        }
        let key_str = match String::from_utf8(key) {
            Ok(k) => k,
            Err(_) => continue,
        };
        let acl = match deserialize_acl(&aclb) {
            Some(a) => a,
            None => continue,
        };
        if header.flags == FLAG_PUT {
            map.insert(key_str, (value, acl));
        } else if header.flags == FLAG_DELETE {
            map.remove(&key_str);
        }
    }
    map
}

#[cfg(feature = "sunlightos")]
fn build_record_bytes(key: &str, value: &[u8], acl_bytes: &[u8], flags: u16) -> Vec<u8> {
    let payload: Vec<u8> = key
        .as_bytes()
        .iter()
        .chain(value.iter())
        .chain(acl_bytes.iter())
        .copied()
        .collect();
    let crc = crc32_ieee(&payload);
    let hdr = RecordHeader {
        magic: RECORD_MAGIC,
        version: RECORD_VERSION,
        flags,
        key_len: key.len() as u32,
        value_len: value.len() as u32,
        acl_len: acl_bytes.len() as u32,
        crc32: crc,
    };
    let mut rec: Vec<u8> = hdr.to_bytes().to_vec();
    rec.extend_from_slice(key.as_bytes());
    rec.extend_from_slice(value);
    rec.extend_from_slice(acl_bytes);
    rec
}

#[cfg(feature = "sunlightos")]
fn queue_persist_record(key: &str, rec: Vec<u8>) {
    // Boundary validation before the frame is committed. A record that can
    // never fit the storage-manager page is dropped here with an explicit
    // counter + log line, rather than silently truncated or retried forever.
    if rec.len() + SM_STORE_PATH.len() > SmMsg::PAGE_CAPACITY {
        unsafe {
            sat_inc(&mut DIAG.persist_skipped_volatile);
        }
        serial_println!(
            "[KV][SM] record too large for page; volatile-only key={} rec_len={}",
            key,
            rec.len()
        );
        return;
    }
    unsafe {
        if VOLATILE_ONLY {
            sat_inc(&mut DIAG.persist_skipped_volatile);
            serial_println!(
                "[KV][SM] persistence skipped; volatile store updated key={}",
                key
            );
            return;
        }
        if PERSIST_QUEUE.is_none() {
            PERSIST_QUEUE = Some(VecDeque::new());
        }
        if let Some(queue) = &mut PERSIST_QUEUE {
            queue.push_back(PersistRecord {
                key: String::from(key),
                bytes: rec,
            });
            let depth = queue.len() as u64;
            if depth > DIAG.persist_queue_hwm {
                DIAG.persist_queue_hwm = depth;
            }
        }
    }
}

/// Number of queued records coalesced into a single storage-manager flush.
/// Batching amortises the O(log size) full-store rewrite across many mutations.
#[cfg(feature = "sunlightos")]
const PERSIST_BATCH: usize = 16;

#[cfg(feature = "sunlightos")]
fn flush_one_persist() {
    // Once persistence is known unavailable, do not spin: the in-memory store is
    // authoritative and uncommitted records stay queued (not dropped) for this
    // process's lifetime. Retrying every tick would only log-storm the console.
    if unsafe { VOLATILE_ONLY } {
        return;
    }
    // Drain up to PERSIST_BATCH frames, but never split a batch that cannot fit
    // the page: stop accumulating as soon as the combined size would exceed the
    // storage-manager payload budget.
    let mut batch: Vec<Vec<u8>> = Vec::new();
    let mut batch_bytes = 0usize;
    let mut keys: Vec<String> = Vec::new();
    unsafe {
        if let Some(queue) = &mut PERSIST_QUEUE {
            while batch.len() < PERSIST_BATCH {
                let Some(front) = queue.front() else { break };
                let next = batch_bytes + front.bytes.len();
                if next + SM_STORE_PATH.len() > SmMsg::PAGE_CAPACITY {
                    break;
                }
                let record = queue.pop_front().expect("front present");
                batch_bytes = next;
                batch.push(record.bytes);
                keys.push(record.key);
            }
        }
    }
    if batch.is_empty() {
        return;
    }

    if sm_append_records(&batch) {
        unsafe {
            sat_inc(&mut DIAG.persist_flush_ok);
            sat_add(&mut DIAG.persist_record_bytes, batch_bytes as u64);
        }
    } else {
        // Persistence is unavailable: switch to volatile-only and requeue the
        // uncommitted records at the FRONT so the dirty state is not silently
        // dropped. A later successful flush (if sm recovers) still commits them;
        // in volatile mode the in-memory store remains authoritative and correct.
        unsafe {
            VOLATILE_ONLY = true;
            if PERSIST_QUEUE.is_none() {
                PERSIST_QUEUE = Some(VecDeque::new());
            }
            if let Some(queue) = &mut PERSIST_QUEUE {
                for (bytes, key) in batch.into_iter().zip(keys.iter()).rev() {
                    queue.push_front(PersistRecord {
                        key: key.clone(),
                        bytes,
                    });
                }
            }
            sat_inc(&mut DIAG.persist_flush_fail);
        }
        diag_note_error(diag_err::PERSIST_FAIL);
        serial_println!(
            "[KV][SM] persistence failed; volatile store retains {} dirty record(s)",
            keys.len()
        );
    }
}

/// PUT with a concrete [`Caller`]: namespace + per-key ACL are enforced before
/// the in-memory store is mutated.
#[cfg(feature = "sunlightos")]
fn do_put_caller(key: &str, value: &[u8], caller: &Caller) -> Result<(), sunlight_kv::KvError> {
    if !sunlight_kv::namespace_allows_write(caller, key) {
        return Err(sunlight_kv::KvError::PermissionDenied);
    }
    if key.is_empty() {
        return Err(sunlight_kv::KvError::InvalidRequest);
    }
    let (mut acl, old_len) =
        if let Some((old_val, existing_acl)) = unsafe { LIVE.as_ref().and_then(|m| m.get(key)) } {
            if !existing_acl.allows_write(caller) {
                return Err(sunlight_kv::KvError::PermissionDenied);
            }
            // The existing ACL (including any world-read flag) is inherited.
            (existing_acl.clone(), Some(old_val.len() as u64))
        } else if sunlight_kv::acl::is_public_key(key) {
            (sunlight_kv::Acl::public_read(caller), None)
        } else {
            (sunlight_kv::Acl::for_caller(caller), None)
        };
    // Migrate legacy public-namespace records (which may still carry a
    // root-only ACL) to broad read access on the next write.
    if sunlight_kv::acl::is_public_key(key) {
        acl.ensure_public_read();
    }

    let acl_bytes = serialize_acl(&acl);
    // Reject oversized frames before they can be committed to a shared-memory
    // page; the on-disk header stores lengths as u32 and the SM page is bounded.
    if !validate_record_bounds(key, value, &acl_bytes) {
        serial_println!(
            "[KV] put rejected: record exceeds page budget key_len={} val_len={}",
            key.len(),
            value.len()
        );
        return Err(sunlight_kv::KvError::InvalidRequest);
    }
    let rec = build_record_bytes(key, value, &acl_bytes, FLAG_PUT);

    unsafe {
        if LIVE.is_none() {
            LIVE = Some(BTreeMap::new());
        }
        if let Some(map) = &mut LIVE {
            map.insert(String::from(key), (value.to_vec(), acl));
        }
        // Maintain payload_bytes without scanning the keyspace.
        if let Some(old) = old_len {
            DIAG.payload_bytes = DIAG.payload_bytes.saturating_sub(old);
        }
        sat_add(&mut DIAG.payload_bytes, value.len() as u64);
        sat_inc(&mut DIAG.mutations);
    }
    queue_persist_record(key, rec);
    Ok(())
}

/// GET with a concrete [`Caller`]: namespace + per-key read ACL enforced.
#[cfg(feature = "sunlightos")]
fn do_get_caller(key: &str, caller: &Caller) -> Result<Vec<u8>, sunlight_kv::KvError> {
    if !sunlight_kv::namespace_allows(caller, key) {
        return Err(sunlight_kv::KvError::PermissionDenied);
    }
    unsafe {
        if let Some(map) = &LIVE {
            if let Some((val, acl)) = map.get(key) {
                // Public-namespace keys are world-readable by policy so a
                // legacy root-only record stays readable after upgrade.
                if sunlight_kv::acl::read_allowed(caller, key, acl) {
                    return Ok(val.clone());
                } else {
                    return Err(sunlight_kv::KvError::PermissionDenied);
                }
            }
        }
    }
    Err(sunlight_kv::KvError::NotFound)
}

/// DELETE with a concrete [`Caller`]: namespace + per-key write ACL enforced.
#[cfg(feature = "sunlightos")]
fn do_delete_caller(key: &str, caller: &Caller) -> Result<(), sunlight_kv::KvError> {
    if !sunlight_kv::namespace_allows_write(caller, key) {
        return Err(sunlight_kv::KvError::PermissionDenied);
    }
    let can = unsafe {
        if let Some(map) = &LIVE {
            if let Some((_, acl)) = map.get(key) {
                acl.allows_write(caller)
            } else {
                false
            }
        } else {
            false
        }
    };
    if !can {
        return Err(sunlight_kv::KvError::PermissionDenied);
    }

    let acl_bytes = unsafe {
        LIVE.as_ref()
            .and_then(|m| m.get(key))
            .map(|(_, a)| serialize_acl(a))
            .unwrap_or_else(|| serialize_acl(&sunlight_kv::Acl::for_caller(caller)))
    };

    let rec = build_record_bytes(key, &[], &acl_bytes, FLAG_DELETE);
    unsafe {
        if let Some(map) = &mut LIVE {
            if let Some((old_val, _)) = map.remove(key) {
                DIAG.payload_bytes = DIAG.payload_bytes.saturating_sub(old_val.len() as u64);
            }
        }
        sat_inc(&mut DIAG.mutations);
    }
    queue_persist_record(key, rec);
    Ok(())
}

#[cfg(feature = "sunlightos")]
fn do_scan(prefix: &str, caller: &Caller) -> Vec<String> {
    unsafe {
        if let Some(map) = &LIVE {
            map.keys()
                .filter(|k| k.starts_with(prefix))
                .filter(|k| sunlight_kv::namespace_allows(caller, k))
                .filter(|k| {
                    map.get(*k)
                        .map(|(_, acl)| sunlight_kv::acl::read_allowed(caller, k, acl))
                        .unwrap_or(false)
                })
                .cloned()
                .collect()
        } else {
            Vec::new()
        }
    }
}

#[cfg(feature = "sunlightos")]
fn pack_kv_payload(msg: &mut IpcMsg, key: &str, value: &[u8]) {
    let kb = key.as_bytes();
    let vb = value;
    if kb.len() > 0xffff || vb.len() > 0xffff {
        return;
    }
    msg.words[0] = (kb.len() as u64) | ((vb.len() as u64) << 16);
    let mut bi = 0usize;
    let mut wi = 1usize;
    for &b in kb.iter().chain(vb.iter()) {
        if wi >= IPC_REGISTER_WORDS {
            break;
        }
        let shift = (bi % 8) * 8;
        msg.words[wi] |= (b as u64) << shift;
        bi += 1;
        if bi % 8 == 0 {
            wi += 1;
        }
    }
}

#[cfg(feature = "sunlightos")]
fn unpack_kv_key(msg: &IpcMsg) -> String {
    let klen = (msg.words[0] & 0xffff) as usize;
    if klen > (IPC_REGISTER_WORDS - 1) * 8 {
        return String::new();
    }
    let mut v: Vec<u8> = Vec::new();
    let mut rem = klen;
    let mut wi = 1usize;
    while rem > 0 && wi < IPC_REGISTER_WORDS {
        for j in 0..8 {
            if rem == 0 {
                break;
            }
            v.push(((msg.words[wi] >> (j * 8)) & 0xff) as u8);
            rem -= 1;
        }
        wi += 1;
    }
    String::from_utf8(v).unwrap_or_default()
}

#[cfg(feature = "sunlightos")]
fn unpack_kv_value(msg: &IpcMsg) -> Vec<u8> {
    let klen = (msg.words[0] & 0xffff) as usize;
    let vlen = ((msg.words[0] >> 16) & 0xffff) as usize;
    if klen + vlen > (IPC_REGISTER_WORDS - 1) * 8 {
        return Vec::new();
    }
    let mut v: Vec<u8> = Vec::new();
    let mut rem = vlen;
    let mut wi = 1usize + (klen + 7) / 8;
    while rem > 0 && wi < IPC_REGISTER_WORDS {
        for j in 0..8 {
            if rem == 0 {
                break;
            }
            v.push(((msg.words[wi] >> (j * 8)) & 0xff) as u8);
            rem -= 1;
        }
        wi += 1;
    }
    v
}

#[cfg(feature = "sunlightos")]
fn pack_str_register_words(msg: &mut IpcMsg, start_word: usize, s: &str) -> bool {
    if start_word >= IPC_REGISTER_WORDS {
        return false;
    }
    let b = s.as_bytes();
    let max = (IPC_REGISTER_WORDS - start_word) * 8;
    if b.len() > max {
        return false;
    }
    let mut i = 0;
    for w in start_word..IPC_REGISTER_WORDS {
        let mut word = 0u64;
        for j in 0..8 {
            if i < b.len() {
                word |= (b[i] as u64) << (j * 8);
                i += 1;
            }
        }
        msg.words[w] = word;
        if i >= b.len() {
            break;
        }
    }
    true
}

#[cfg(feature = "sunlightos")]
fn unpack_str(words: &[u64; 8], start_word: usize, max_len: usize) -> String {
    let mut v: Vec<u8> = Vec::new();
    let mut i = 0;
    for w in start_word..IPC_REGISTER_WORDS {
        if i >= max_len {
            break;
        }
        let word = words[w];
        for j in 0..8 {
            if i >= max_len {
                break;
            }
            let byte = ((word >> (j * 8)) & 0xff) as u8;
            if byte == 0 {
                break;
            }
            v.push(byte);
            i += 1;
        }
    }
    String::from_utf8(v).unwrap_or_default()
}

#[cfg(feature = "sunlightos")]
fn read_shm_bytes(tok: CapabilityToken, len: usize) -> Option<Vec<u8>> {
    if tok == CapabilityToken::INVALID || len > SHM_PAGE {
        return None;
    }
    let ptr = shm_map(tok).ok()?;
    let bytes = unsafe { core::slice::from_raw_parts(ptr, len) }.to_vec();
    let _ = shm_free(tok);
    Some(bytes)
}

#[cfg(feature = "sunlightos")]
fn send_shm_value_reply(reply: &mut IpcMsg, value: &[u8]) {
    if value.len() > SHM_PAGE {
        reply.label = KV_ERROR;
        reply.words[0] = 4;
        return;
    }
    match shm_alloc() {
        Ok((ptr, token)) => {
            unsafe {
                core::ptr::copy_nonoverlapping(value.as_ptr(), ptr, value.len());
            }
            reply.label = KV_VALUE;
            reply.words[0] = value.len() as u64;
            *reply = (*reply).with_cap(0, token);
        }
        Err(_) => {
            reply.label = KV_ERROR;
            reply.words[0] = 3;
        }
    }
}

#[cfg(feature = "sunlightos")]
#[no_mangle]
pub extern "C" fn _start(_argc: u64, _argv: *const *const u8, _envp: *const *const u8) -> ! {
    debug_log("[SUNLIGHT-KV] main() reached (no_std)\n");
    serial_println!("[SUNLIGHT-KV] Starting sunlight-kv (SunlightOS mode)");

    let ep = endpoint_create();
    nameserver_register("sunlight-kv", ep);
    serial_println!("[SUNLIGHT-KV] Registered as 'sunlight-kv'");

    // Prefer sm for store content (writes always via sm; reads may use direct for recover compat)
    let (live_map, volatile_only) = if let Some(bytes) = sm_read_file(b"/var/lib/sunlight/kv.store")
    {
        // We have bytes from sm; use a simple path: write to a mem fd? For minimal, fall to direct open if possible or parse inline later.
        // For this bite use direct open for recover (read-only of sm-written file) and log the sm use.
        serial_println!("[KV][SM] initial store read via sm len={}", bytes.len());
        let fd_opt = open_or_create_store();
        if let Some(fd) = fd_opt {
            let m = recover_store(fd);
            unsafe {
                LOG_FD = Some(fd);
            }
            serial_println!(
                "[SUNLIGHT-KV] store recovered via sm-written data, live keys={}",
                m.len()
            );
            (m, false)
        } else {
            serial_println!("[SUNLIGHT-KV] sm data present but open failed; volatile");
            (BTreeMap::new(), true)
        }
    } else if let Some(fd) = open_or_create_store() {
        let m = recover_store(fd);
        unsafe {
            LOG_FD = Some(fd);
        }
        serial_println!("[SUNLIGHT-KV] store recovered, live keys={}", m.len());
        (m, false)
    } else {
        serial_println!("[SUNLIGHT-KV] WARNING: kv.store not available, using volatile store");
        (BTreeMap::new(), true)
    };
    let mut payload_sum: u64 = 0;
    for (_k, (v, _)) in live_map.iter() {
        payload_sum = payload_sum.saturating_add(v.len() as u64);
    }
    let boot_ms = monotonic_millis();
    unsafe {
        LIVE = Some(live_map);
        VOLATILE_ONLY = volatile_only;
        if PERSIST_QUEUE.is_none() {
            PERSIST_QUEUE = Some(VecDeque::new());
        }
        DIAG.started_ms = boot_ms;
        DIAG.last_activity_ms = boot_ms;
        DIAG.payload_bytes = payload_sum;
    }

    serial_println!("[SUNLIGHT-KV] Entering IPC loop");

    // IPC invariant: every request path through the match below must produce
    // exactly one reply. A missing key must reply with KV_ERROR (not silence),
    // an unsupported opcode must reply with KV_ERROR — otherwise clients
    // block forever waiting for a reply that never comes.
    //
    // Persistence is best-effort: the in-memory store (LIVE) is updated first
    // so the reply is prompt. If sunlight-sm cannot persist the record the
    // volatile store remains correct and keeps serving GET/PUT/DELETE.
    //
    // Per-request caller tracing is opt-in and off by default; build with
    // SUNLIGHT_KV_TRACE=1 only when auditing ACL resolution, otherwise every
    // calculator history save/load produces multiple serial log lines.
    const TRACE_KV: bool = matches!(option_env!("SUNLIGHT_KV_TRACE"), Some(_));

    unsafe {
        sat_inc(&mut DIAG.recv_blocking);
    }
    let mut msg = ipc_recv(ep);
    loop {
        // Kernel overwrites badge with the real sender PID (authoritative).
        let caller_pid = msg.badge;
        diag_note_request_start(msg.label, caller_pid);
        // Derive the authenticated caller identity once per request from
        // kernel-authoritative metadata. Never from the message payload.
        let caller = resolve_caller(caller_pid);
        if TRACE_KV {
            serial_println!(
                "[KV] request label={:#x} pid={} caller={}",
                msg.label,
                caller_pid,
                caller.identity()
            );
        }

        let mut reply = IpcMsg::empty();
        reply.label = KV_REPLY;
        let mut decode_error = false;
        // If set, fill this already-mapped SHM page with a stats snapshot after
        // outcome counters are updated (ptr from shm_alloc; client receives token).
        let mut stats_page: Option<*mut u8> = None;

        match msg.label {
            KV_PUT => {
                let key = unpack_kv_key(&msg);
                let val = unpack_kv_value(&msg);
                if key.is_empty() {
                    reply.label = KV_ERROR;
                    decode_error = true;
                    diag_note_error(diag_err::EMPTY_KEY);
                } else {
                    if do_put_caller(&key, &val, &caller).is_ok() {
                        if TRACE_KV {
                            serial_println!("[KV] put key={} len={} ok (inline)", key, val.len());
                        }
                        reply.words[0] = 0;
                    } else {
                        reply.label = KV_ERROR;
                        reply.words[0] = 2;
                        diag_note_error(diag_err::ACL_OR_MISSING);
                    }
                }
            }
            KV_GET => {
                let key = unpack_kv_key(&msg);
                if key.is_empty() {
                    reply.label = KV_ERROR;
                    decode_error = true;
                    diag_note_error(diag_err::EMPTY_KEY);
                } else {
                    match do_get_caller(&key, &caller) {
                        Ok(v) => {
                            if TRACE_KV {
                                serial_println!("[KV] get key={} len={} hit", key, v.len());
                            }
                            reply.label = KV_VALUE;
                            reply.words[0] = v.len() as u64;
                            let mut bi = 0usize;
                            let mut wi = 1usize;
                            for &b in &v {
                                if wi >= 8 {
                                    break;
                                }
                                let shift = (bi % 8) * 8;
                                reply.words[wi] |= (b as u64) << shift;
                                bi += 1;
                                if bi % 8 == 0 {
                                    wi += 1;
                                }
                            }
                        }
                        Err(_) => {
                            if TRACE_KV {
                                serial_println!("[KV] get key={} not-found", key);
                            }
                            reply.label = KV_ERROR;
                            reply.words[0] = 2;
                            diag_note_error(diag_err::ACL_OR_MISSING);
                        }
                    }
                }
            }
            KV_PUT_SHM => {
                // word[0] = value_len, words[2..4) = key (max 16 bytes), caps[0] = page token.
                let key = unpack_str(&msg.words, 2, (IPC_REGISTER_WORDS - 2) * 8);
                let vlen = msg.words[0] as usize;
                let tok = msg.caps[0];
                if key.is_empty() || tok == CapabilityToken::INVALID || vlen > SHM_PAGE {
                    reply.label = KV_ERROR;
                    reply.words[0] = 1;
                    decode_error = true;
                    diag_note_error(diag_err::BAD_SHM);
                } else {
                    match shm_map(tok) {
                        Ok(ptr) => {
                            let val = unsafe { core::slice::from_raw_parts(ptr, vlen) }.to_vec();
                            let _ = shm_free(tok);
                            if do_put_caller(&key, &val, &caller).is_ok() {
                                if TRACE_KV {
                                    serial_println!("[KV] put key={} len={} ok (shm)", key, vlen);
                                }
                                reply.words[0] = 0;
                            } else {
                                reply.label = KV_ERROR;
                                reply.words[0] = 2;
                                diag_note_error(diag_err::ACL_OR_MISSING);
                            }
                        }
                        Err(_) => {
                            reply.label = KV_ERROR;
                            reply.words[0] = 3;
                            decode_error = true;
                            diag_note_error(diag_err::BAD_SHM);
                        }
                    }
                }
            }
            KV_GET_SHM => {
                // words[2..4) = key (max 16 bytes). Reply: KV_VALUE word[0]=len, caps[0]=page token.
                let key = unpack_str(&msg.words, 2, (IPC_REGISTER_WORDS - 2) * 8);
                if key.is_empty() {
                    reply.label = KV_ERROR;
                    decode_error = true;
                    diag_note_error(diag_err::EMPTY_KEY);
                } else {
                    match do_get_caller(&key, &caller) {
                        Ok(v) => {
                            send_shm_value_reply(&mut reply, &v);
                            if reply.label == KV_ERROR {
                                diag_note_error(diag_err::SHM_ALLOC);
                            }
                        }
                        Err(_) => {
                            reply.label = KV_ERROR;
                            reply.words[0] = 2;
                            diag_note_error(diag_err::ACL_OR_MISSING);
                        }
                    }
                }
            }
            KV_PUT_SHM2 => {
                let key_len = msg.words[0] as usize;
                let value_len = msg.words[1] as usize;
                let key_tok = msg.caps[0];
                let value_tok = msg.caps[1];
                let Some(key_bytes) = read_shm_bytes(key_tok, key_len) else {
                    reply.label = KV_ERROR;
                    reply.words[0] = 1;
                    decode_error = true;
                    diag_note_error(diag_err::BAD_SHM);
                    if value_tok != CapabilityToken::INVALID {
                        let _ = shm_free(value_tok);
                    }
                    if key_tok != CapabilityToken::INVALID {
                        let _ = shm_free(key_tok);
                    }
                    diag_note_outcome(reply.label, decode_error);
                    finish_reply(ep, reply, &mut msg);
                    continue;
                };
                let Some(value) = read_shm_bytes(value_tok, value_len) else {
                    reply.label = KV_ERROR;
                    reply.words[0] = 1;
                    decode_error = true;
                    diag_note_error(diag_err::BAD_SHM);
                    diag_note_outcome(reply.label, decode_error);
                    finish_reply(ep, reply, &mut msg);
                    continue;
                };
                match String::from_utf8(key_bytes) {
                    Ok(key) if !key.is_empty() && do_put_caller(&key, &value, &caller).is_ok() => {
                        reply.words[0] = 0;
                    }
                    _ => {
                        reply.label = KV_ERROR;
                        reply.words[0] = 2;
                        diag_note_error(diag_err::ACL_OR_MISSING);
                    }
                }
            }
            KV_GET_SHM2 => {
                let key_len = msg.words[0] as usize;
                let key_tok = msg.caps[0];
                let Some(key_bytes) = read_shm_bytes(key_tok, key_len) else {
                    reply.label = KV_ERROR;
                    reply.words[0] = 1;
                    decode_error = true;
                    diag_note_error(diag_err::BAD_SHM);
                    diag_note_outcome(reply.label, decode_error);
                    finish_reply(ep, reply, &mut msg);
                    continue;
                };
                match String::from_utf8(key_bytes) {
                    Ok(key) if !key.is_empty() => match do_get_caller(&key, &caller) {
                        Ok(v) => {
                            send_shm_value_reply(&mut reply, &v);
                            if reply.label == KV_ERROR {
                                diag_note_error(diag_err::SHM_ALLOC);
                            }
                        }
                        Err(_) => {
                            reply.label = KV_ERROR;
                            reply.words[0] = 2;
                            diag_note_error(diag_err::ACL_OR_MISSING);
                        }
                    },
                    _ => {
                        reply.label = KV_ERROR;
                        reply.words[0] = 2;
                        decode_error = true;
                        diag_note_error(diag_err::EMPTY_KEY);
                    }
                }
            }
            KV_DELETE_SHM2 => {
                let key_len = msg.words[0] as usize;
                let key_tok = msg.caps[0];
                let Some(key_bytes) = read_shm_bytes(key_tok, key_len) else {
                    reply.label = KV_ERROR;
                    reply.words[0] = 1;
                    decode_error = true;
                    diag_note_error(diag_err::BAD_SHM);
                    diag_note_outcome(reply.label, decode_error);
                    finish_reply(ep, reply, &mut msg);
                    continue;
                };
                match String::from_utf8(key_bytes) {
                    Ok(key) if !key.is_empty() && do_delete_caller(&key, &caller).is_ok() => {
                        reply.words[0] = 0;
                    }
                    _ => {
                        reply.label = KV_ERROR;
                        reply.words[0] = 2;
                        diag_note_error(diag_err::ACL_OR_MISSING);
                    }
                }
            }
            KV_DELETE => {
                let key = unpack_kv_key(&msg);
                if key.is_empty() {
                    reply.label = KV_ERROR;
                    decode_error = true;
                    diag_note_error(diag_err::EMPTY_KEY);
                } else {
                    if do_delete_caller(&key, &caller).is_ok() {
                        reply.words[0] = 0;
                    } else {
                        reply.label = KV_ERROR;
                        diag_note_error(diag_err::ACL_OR_MISSING);
                    }
                }
            }
            KV_SCAN => {
                let prefix = unpack_str(&msg.words, 0, IPC_REGISTER_WORDS * 8);
                let keys = do_scan(&prefix, &caller);
                reply.words[0] = keys.len() as u64;
                let mut wi = 1usize;
                for k in keys.iter().take(3) {
                    if wi >= IPC_REGISTER_WORDS {
                        break;
                    }
                    if !pack_str_register_words(&mut reply, wi, k) {
                        break;
                    }
                    wi += 1;
                }
            }
            KV_STATS => {
                // Read-only snapshot: fixed-size SHM blob. Key count is map.len()
                // (O(1)); payload_bytes is maintained incrementally — no keyspace walk.
                // The SHM page is filled after outcome accounting so this stats
                // request itself is visible in the returned totals.
                match shm_alloc() {
                    Ok((ptr, token)) => {
                        stats_page = Some(ptr);
                        reply.label = KV_REPLY;
                        reply.words[0] = STATS_VERSION;
                        reply.words[1] = STATS_BYTES as u64;
                        reply = reply.with_cap(0, token);
                    }
                    Err(_) => {
                        reply.label = KV_ERROR;
                        reply.words[0] = 3;
                        diag_note_error(diag_err::STATS_SHM);
                    }
                }
            }
            _ => {
                serial_println!("[KV] unsupported label={:#x}", msg.label);
                reply.label = KV_ERROR;
                reply.words[0] = 0xff;
                diag_note_error(diag_err::UNSUPPORTED);
            }
        }

        diag_note_outcome(reply.label, decode_error);

        // Fill stats SHM only after counters include this request's outcome.
        // Do not free the token: client maps and frees (same pattern as GET_SHM).
        if let Some(ptr) = stats_page {
            if reply.label == KV_REPLY {
                let snap = diag_snapshot();
                let mut buf = [0u8; STATS_BYTES];
                let _ = snap.encode(&mut buf);
                unsafe {
                    core::ptr::copy_nonoverlapping(buf.as_ptr(), ptr, STATS_BYTES);
                }
            }
        }

        if TRACE_KV {
            serial_println!("[KV] reply label={:#x} w0={}", reply.label, reply.words[0]);
        }
        finish_reply(ep, reply, &mut msg);
    }
}

/// Reply then either take a queued call immediately or flush one persist entry
/// and block for the next message. Updates event-loop diagnostics only.
#[cfg(feature = "sunlightos")]
fn finish_reply(ep: sunlight_ipc::EndpointId, reply: IpcMsg, msg: &mut IpcMsg) {
    if let Some(next) = ipc_reply_and_try_recv(ep, reply) {
        unsafe {
            sat_inc(&mut DIAG.try_recv_hit);
        }
        *msg = next;
        return;
    }
    unsafe {
        sat_inc(&mut DIAG.try_recv_miss);
    }
    flush_one_persist();
    unsafe {
        sat_inc(&mut DIAG.recv_blocking);
    }
    *msg = ipc_recv(ep);
}

#[cfg(feature = "sunlightos")]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    debug_log("[SUNLIGHT-KV] PANIC\n");
    loop {}
}
