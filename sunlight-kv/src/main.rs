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
use alloc::collections::BTreeMap;
#[cfg(feature = "sunlightos")]
use alloc::string::String;
#[cfg(feature = "sunlightos")]
use alloc::vec;
#[cfg(feature = "sunlightos")]
use alloc::vec::Vec;

#[cfg(feature = "sunlightos")]
use sunlight_ipc::{
    debug_log, endpoint_create, ipc_recv, ipc_reply_and_wait, nameserver_register, shm_alloc,
    shm_free, shm_map, CapabilityToken, IpcMsg,
};
#[cfg(feature = "sunlightos")]
use sunlight_libc::{self as libc, Fd};

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

#[cfg(feature = "sunlightos")]
fn pack_acl_default(caller: &str) -> Vec<u8> {
    let acl = sunlight_kv::Acl::new(caller);
    serialize_acl(&acl)
}

// KV IPC protocol (sunlightos IpcMsg). Supports keys/values that fit in words (~48 bytes combined).
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
const SHM_PAGE: usize = 4096;

#[cfg(feature = "sunlightos")]
static mut LOG_FD: Option<Fd> = None;
#[cfg(feature = "sunlightos")]
static mut LIVE: Option<BTreeMap<String, (Vec<u8>, sunlight_kv::Acl)>> = None;

#[cfg(feature = "sunlightos")]
fn ensure_dirs() {
    let _ = libc::mkdir(b"/var", 0o755);
    let _ = libc::mkdir(b"/var/lib", 0o755);
    let _ = libc::mkdir(b"/var/lib/sunlight", 0o755);
}

#[cfg(feature = "sunlightos")]
fn open_or_create_store() -> Option<Fd> {
    ensure_dirs();
    match libc::open(b"/var/lib/sunlight/kv.store") {
        Ok(fd) => Some(fd),
        Err(_) => match libc::open(b"/var/lib/sunlight/kv.store") {
            Ok(fd) => Some(fd),
            Err(_) => None,
        },
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
fn do_put(key: &str, value: &[u8], caller: &str) -> bool {
    let acl = if let Some((_, existing_acl)) = unsafe { LIVE.as_ref().and_then(|m| m.get(key)) } {
        if !existing_acl.allows_write(caller) {
            return false;
        }
        existing_acl.clone()
    } else {
        sunlight_kv::Acl::new(caller)
    };

    let acl_bytes = serialize_acl(&acl);

    unsafe {
        if let Some(fd) = LOG_FD {
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
                flags: FLAG_PUT,
                key_len: key.len() as u32,
                value_len: value.len() as u32,
                acl_len: acl_bytes.len() as u32,
                crc32: crc,
            };
            let hb = hdr.to_bytes();
            let _ = write_all(fd, &hb);
            let _ = write_all(fd, key.as_bytes());
            let _ = write_all(fd, value);
            let _ = write_all(fd, &acl_bytes);
        }
    }

    unsafe {
        if LIVE.is_none() {
            LIVE = Some(BTreeMap::new());
        }
        if let Some(map) = &mut LIVE {
            map.insert(String::from(key), (value.to_vec(), acl));
        }
    }
    true
}

#[cfg(feature = "sunlightos")]
fn do_get(key: &str, caller: &str) -> Result<Vec<u8>, ()> {
    unsafe {
        if let Some(map) = &LIVE {
            if let Some((val, acl)) = map.get(key) {
                if acl.allows_read(caller) {
                    return Ok(val.clone());
                } else {
                    return Err(());
                }
            }
        }
    }
    Err(())
}

#[cfg(feature = "sunlightos")]
fn do_delete(key: &str, caller: &str) -> bool {
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
        return false;
    }

    let acl_bytes = unsafe {
        LIVE.as_ref()
            .and_then(|m| m.get(key))
            .map(|(_, a)| serialize_acl(a))
            .unwrap_or_else(|| pack_acl_default(caller))
    };

    unsafe {
        if let Some(fd) = LOG_FD {
            let payload: Vec<u8> = key
                .as_bytes()
                .iter()
                .chain((&[] as &[u8]).iter())
                .chain(acl_bytes.iter())
                .copied()
                .collect();
            let crc = crc32_ieee(&payload);
            let hdr = RecordHeader {
                magic: RECORD_MAGIC,
                version: RECORD_VERSION,
                flags: FLAG_DELETE,
                key_len: key.len() as u32,
                value_len: 0,
                acl_len: acl_bytes.len() as u32,
                crc32: crc,
            };
            let hb = hdr.to_bytes();
            let _ = write_all(fd, &hb);
            let _ = write_all(fd, key.as_bytes());
            let _ = write_all(fd, &[]);
            let _ = write_all(fd, &acl_bytes);
        }
        if let Some(map) = &mut LIVE {
            map.remove(key);
        }
    }
    true
}

#[cfg(feature = "sunlightos")]
fn do_scan(prefix: &str) -> Vec<String> {
    unsafe {
        if let Some(map) = &LIVE {
            map.keys().filter(|k| k.starts_with(prefix)).cloned().collect()
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
        if wi >= 8 {
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
    let mut v: Vec<u8> = Vec::new();
    let mut rem = klen;
    let mut wi = 1usize;
    while rem > 0 && wi < 8 {
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
    let mut v: Vec<u8> = Vec::new();
    let mut rem = vlen;
    let mut wi = 1usize + (klen + 7) / 8;
    while rem > 0 && wi < 8 {
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
fn pack_str(msg: &mut IpcMsg, start_word: usize, s: &str) {
    let b = s.as_bytes();
    let mut i = 0;
    for w in start_word..8 {
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
}

#[cfg(feature = "sunlightos")]
fn unpack_str(words: &[u64; 8], start_word: usize, max_len: usize) -> String {
    let mut v: Vec<u8> = Vec::new();
    let mut i = 0;
    for w in start_word..8 {
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
#[no_mangle]
pub extern "C" fn _start(_argc: u64, _argv: *const *const u8, _envp: *const *const u8) -> ! {
    debug_log("[SUNLIGHT-KV] main() reached (no_std)\n");
    serial_println!("[SUNLIGHT-KV] Starting sunlight-kv (SunlightOS mode)");

    let ep = endpoint_create();
    nameserver_register("sunlight-kv", ep);
    serial_println!("[SUNLIGHT-KV] Registered as 'sunlight-kv'");

    let fd_opt = open_or_create_store();
    let live_map = if let Some(fd) = fd_opt {
        let m = recover_store(fd);
        unsafe {
            LOG_FD = Some(fd);
        }
        serial_println!("[SUNLIGHT-KV] store recovered, live keys={}", m.len());
        m
    } else {
        serial_println!("[SUNLIGHT-KV] WARNING: kv.store not available, using volatile store");
        BTreeMap::new()
    };
    unsafe {
        LIVE = Some(live_map);
    }

    serial_println!("[SUNLIGHT-KV] Entering IPC loop");

    let mut msg = ipc_recv(ep);
    loop {
        let mut reply = IpcMsg::empty();
        reply.label = KV_REPLY;

        match msg.label {
            KV_PUT => {
                let key = unpack_kv_key(&msg);
                let val = unpack_kv_value(&msg);
                if key.is_empty() {
                    reply.label = KV_ERROR;
                } else {
                    let caller = "root";
                    if do_put(&key, &val, caller) {
                        reply.words[0] = 0;
                    } else {
                        reply.label = KV_ERROR;
                        reply.words[0] = 2;
                    }
                }
            }
            KV_GET => {
                let key = unpack_kv_key(&msg);
                if key.is_empty() {
                    reply.label = KV_ERROR;
                } else {
                    let caller = "root";
                    match do_get(&key, caller) {
                        Ok(v) => {
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
                        Err(()) => {
                            reply.label = KV_ERROR;
                            reply.words[0] = 2;
                        }
                    }
                }
            }
            KV_PUT_SHM => {
                // word[0] = value_len, words[2..] = key (NUL-padded), caps[0] = page token.
                let key = unpack_str(&msg.words, 2, 48);
                let vlen = msg.words[0] as usize;
                let tok = msg.caps[0];
                if key.is_empty() || tok == CapabilityToken::INVALID || vlen > SHM_PAGE {
                    reply.label = KV_ERROR;
                    reply.words[0] = 1;
                } else {
                    match shm_map(tok) {
                        Ok(ptr) => {
                            let val = unsafe { core::slice::from_raw_parts(ptr, vlen) }.to_vec();
                            let _ = shm_free(tok);
                            if do_put(&key, &val, "root") {
                                reply.words[0] = 0;
                            } else {
                                reply.label = KV_ERROR;
                                reply.words[0] = 2;
                            }
                        }
                        Err(_) => {
                            reply.label = KV_ERROR;
                            reply.words[0] = 3;
                        }
                    }
                }
            }
            KV_GET_SHM => {
                // words[2..] = key (NUL-padded). Reply: KV_VALUE word[0]=len, caps[0]=page token.
                let key = unpack_str(&msg.words, 2, 48);
                if key.is_empty() {
                    reply.label = KV_ERROR;
                } else {
                    match do_get(&key, "root") {
                        Ok(v) if v.len() <= SHM_PAGE => match shm_alloc() {
                            Ok((ptr, token)) => {
                                unsafe {
                                    core::ptr::copy_nonoverlapping(v.as_ptr(), ptr, v.len());
                                }
                                reply.label = KV_VALUE;
                                reply.words[0] = v.len() as u64;
                                reply = reply.with_cap(0, token);
                            }
                            Err(_) => {
                                reply.label = KV_ERROR;
                                reply.words[0] = 3;
                            }
                        },
                        Ok(_) => {
                            reply.label = KV_ERROR;
                            reply.words[0] = 4; // value larger than one shm page
                        }
                        Err(()) => {
                            reply.label = KV_ERROR;
                            reply.words[0] = 2;
                        }
                    }
                }
            }
            KV_DELETE => {
                let key = unpack_kv_key(&msg);
                if key.is_empty() {
                    reply.label = KV_ERROR;
                } else {
                    let caller = "root";
                    if do_delete(&key, caller) {
                        reply.words[0] = 0;
                    } else {
                        reply.label = KV_ERROR;
                    }
                }
            }
            KV_SCAN => {
                let prefix = unpack_str(&msg.words, 0, 64);
                let keys = do_scan(&prefix);
                reply.words[0] = keys.len() as u64;
                let mut wi = 1usize;
                for k in keys.iter().take(3) {
                    if wi >= 8 {
                        break;
                    }
                    pack_str(&mut reply, wi, k);
                    wi += 1;
                }
            }
            _ => {
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
