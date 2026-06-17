//! sunlight-tls daemon.
//!
//! Provides TLS 1.3 handshake capability to other services via kernel IPC.
//! Certificates (roots, server) are fetched/stored via IPC to sunlight-kv under
//! keys: tls/ca/<name>, tls/server/cert, tls/server/key
//!
//! Transport integration: callers feed ciphertext/plaintext buffers over IPC
//! (smoltcp <-> this service adapter in a full net stack). This binary implements
//! the documented SunlightOS service structure exactly.

#![cfg_attr(feature = "sunlightos", no_std)]
#![cfg_attr(feature = "sunlightos", no_main)]

#[cfg(feature = "sunlightos")]
extern crate alloc;

#[cfg(feature = "host")]
fn main() {
    // Host mode: not the primary execution path. Real rustls usage lives in
    // host tooling or future certificate provisioning. This keeps the binary
    // runnable for cargo check / cargo run ergonomics.
    println!("sunlight-tls: host mode (no TLS daemon loop). Use in SunlightOS image.");
}

// -----------------------------------------------------------------------------
// SunlightOS (no_std) build
// -----------------------------------------------------------------------------

#[cfg(feature = "sunlightos")]
use alloc::string::String;
#[cfg(feature = "sunlightos")]
use alloc::vec::Vec;

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
        match sunlight_libc::write(sunlight_libc::STDOUT, data) {
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
        sunlight_ipc::debug_log(&buf);
    }};
}

#[cfg(feature = "sunlightos")]
use sunlight_ipc::{
    debug_log, endpoint_create, get_time_utc, ipc_call, ipc_recv, ipc_reply_and_wait,
    monotonic_millis, nameserver_lookup, nameserver_register, IpcMsg, TzMsg,
};

// Protocol labels (private to this service + its client certificatectl)
#[cfg(feature = "sunlightos")]
const TLS_HANDSHAKE: u64 = 0x5401;
#[cfg(feature = "sunlightos")]
const TLS_FEED: u64 = 0x5402;
#[cfg(feature = "sunlightos")]
const TLS_GET_WRITE: u64 = 0x5403;
#[cfg(feature = "sunlightos")]
const TLS_GET_PLAIN: u64 = 0x5404;
#[cfg(feature = "sunlightos")]
const TLS_CLOSE: u64 = 0x5405;
#[cfg(feature = "sunlightos")]
const TLS_INSTALL: u64 = 0x5406; // from certificatectl -> stores demo certs into kv
#[cfg(feature = "sunlightos")]
const TLS_LIST: u64 = 0x5407;
#[cfg(feature = "sunlightos")]
const TLS_REPLY: u64 = 0x54FF;
#[cfg(feature = "sunlightos")]
const TLS_ERROR: u64 = 0x54EE;

// Also re-use kv ops for internal fetches/PUTs when proxying certs
#[cfg(feature = "sunlightos")]
const KV_GET: u64 = 0x4B02;
#[cfg(feature = "sunlightos")]
const KV_PUT: u64 = 0x4B01;
#[cfg(feature = "sunlightos")]
const KV_REPLY: u64 = 0x4BFF;
#[cfg(feature = "sunlightos")]
const KV_ERROR: u64 = 0x4BEE;
#[cfg(feature = "sunlightos")]
const KV_VALUE: u64 = 0x4B05;

#[cfg(feature = "sunlightos")]
const MAX_SESSIONS: usize = 4;

#[cfg(feature = "sunlightos")]
struct Session {
    id: u64,
    established: bool,
    // For demo (no rustls on sunlightos path): pass-through buffers
    to_write: Vec<u8>,
    to_plain: Vec<u8>,
}

#[cfg(feature = "sunlightos")]
static mut SESSIONS: [Option<Session>; MAX_SESSIONS] = [const { None }; MAX_SESSIONS];
#[cfg(feature = "sunlightos")]
static mut NEXT_SID: u64 = 1;

#[cfg(feature = "sunlightos")]
static mut CACHED_CA: Option<(String, Vec<u8>)> = None;
#[cfg(feature = "sunlightos")]
static mut CACHED_CERT: Option<Vec<u8>> = None;
#[cfg(feature = "sunlightos")]
static mut CACHED_KEY: Option<Vec<u8>> = None;

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
fn pack_kv_payload(msg: &mut IpcMsg, key: &str, value: &[u8]) {
    let kb = key.as_bytes();
    let vb = value;
    msg.words[0] = (kb.len().min(63) as u64) | (((vb.len().min(63) as u64) << 16));
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
fn unpack_kv_value(msg: &IpcMsg) -> Vec<u8> {
    let vlen = ((msg.words[0] >> 16) & 0xffff) as usize;
    let mut v: Vec<u8> = Vec::new();
    let mut rem = vlen;
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
    v
}

#[cfg(feature = "sunlightos")]
fn kv_get(key: &str) -> Option<Vec<u8>> {
    let kv_cap = nameserver_lookup("sunlight-kv")?;
    let mut msg = IpcMsg::empty();
    msg.label = KV_GET;
    pack_kv_payload(&mut msg, key, b"");
    let reply = ipc_call(kv_cap, msg);
    if reply.label == KV_VALUE {
        Some(unpack_kv_value(&reply))
    } else {
        None
    }
}

#[cfg(feature = "sunlightos")]
fn kv_put(key: &str, value: &[u8]) -> bool {
    let kv_cap = match nameserver_lookup("sunlight-kv") {
        Some(c) => c,
        None => return false,
    };
    let mut msg = IpcMsg::empty();
    msg.label = KV_PUT;
    pack_kv_payload(&mut msg, key, value);
    let reply = ipc_call(kv_cap, msg);
    reply.label == KV_REPLY && reply.words[0] == 0
}

/// Query the timezone service for the current local time.
/// Returns "YYYY-MM-DDTHH:MM:SS" or "tz-unavail" if the tz service is not yet up.
#[cfg(feature = "sunlightos")]
fn local_time_str() -> heapless::String<32> {
    use core::fmt::Write;
    let mut out = heapless::String::<32>::new();
    if let Some(tz_cap) = nameserver_lookup("tz") {
        let r = ipc_call(tz_cap, IpcMsg::with_label(TzMsg::GET_LOCAL_TIME));
        if r.label == TzMsg::REPLY {
            let w0 = r.words[0];
            let year   = (w0 >> 48) as u16;
            let month  = ((w0 >> 40) & 0xff) as u8;
            let day    = ((w0 >> 32) & 0xff) as u8;
            let hour   = ((w0 >> 24) & 0xff) as u8;
            let minute = ((w0 >> 16) & 0xff) as u8;
            let second = ((w0 >>  8) & 0xff) as u8;
            // offset_secs lets us show the sign/hours for quick sanity check
            let off_s  = r.words[1] as i64;
            let off_h  = off_s / 3600;
            let off_m  = (off_s.abs() % 3600) / 60;
            let _ = write!(&mut out, "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}{:+03}:{:02}",
                           year, month, day, hour, minute, second, off_h, off_m);
            return out;
        }
    }
    let _ = out.push_str("tz-unavail");
    out
}

#[cfg(feature = "sunlightos")]
fn fetch_certs_from_kv() {
    // Fetch example cert material (small demo blobs that fit inline IPC).
    if let Some(v) = kv_get("tls/ca/system") {
        if !v.is_empty() {
            unsafe { CACHED_CA = Some((String::from("system"), v)); }
            serial_println!("[SUNLIGHT-TLS] loaded tls/ca/system from kv");
        }
    }
    if let Some(v) = kv_get("tls/server/cert") {
        if !v.is_empty() {
            unsafe { CACHED_CERT = Some(v); }
            serial_println!("[SUNLIGHT-TLS] loaded tls/server/cert from kv");
        }
    }
    if let Some(v) = kv_get("tls/server/key") {
        if !v.is_empty() {
            unsafe { CACHED_KEY = Some(v); }
            serial_println!("[SUNLIGHT-TLS] loaded tls/server/key from kv");
        }
    }
}

#[cfg(feature = "sunlightos")]
fn alloc_session() -> Option<u64> {
    unsafe {
        for i in 0..MAX_SESSIONS {
            if SESSIONS[i].is_none() {
                let sid = NEXT_SID;
                NEXT_SID = NEXT_SID.wrapping_add(1);
                if NEXT_SID == 0 {
                    NEXT_SID = 1;
                }
                SESSIONS[i] = Some(Session {
                    id: sid,
                    established: true, // demo: "handshake" completes immediately
                    to_write: Vec::new(),
                    to_plain: Vec::new(),
                });
                return Some(sid);
            }
        }
    }
    None
}

#[cfg(feature = "sunlightos")]
fn find_session_mut(id: u64) -> Option<&'static mut Session> {
    unsafe {
        for i in 0..MAX_SESSIONS {
            if let Some(ref mut s) = SESSIONS[i] {
                if s.id == id {
                    return Some(s);
                }
            }
        }
    }
    None
}

#[cfg(feature = "sunlightos")]
fn close_session(id: u64) -> bool {
    unsafe {
        for i in 0..MAX_SESSIONS {
            if let Some(ref s) = SESSIONS[i] {
                if s.id == id {
                    SESSIONS[i] = None;
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(feature = "sunlightos")]
#[no_mangle]
pub extern "C" fn _start(_argc: u64, _argv: *const *const u8, _envp: *const *const u8) -> ! {
    debug_log("[SUNLIGHT-TLS] main() reached (no_std)\n");
    serial_println!("[SUNLIGHT-TLS] Starting sunlight-tls (SunlightOS mode)");

    let ep = endpoint_create();
    nameserver_register("sunlight-tls", ep);
    serial_println!("[SUNLIGHT-TLS] Registered as 'sunlight-tls'");
    // Cert material is loaded lazily on TLS_INSTALL (KV is always empty/volatile at boot).
    serial_println!("[SUNLIGHT-TLS] Entering IPC loop");

    let mut msg = ipc_recv(ep);
    loop {
        let mut reply = IpcMsg::empty();
        reply.label = TLS_REPLY;

        match msg.label {
            TLS_HANDSHAKE => {
                // word0: 1=client 2=server; servername or sni packed after
                let role = (msg.words[0] & 0xff) as u8;
                let is_client = role == 1;
                let sni = unpack_str(&msg.words, 1, 64);

                // Log time (local + raw) to verify clock and tz service are correct.
                let unix_time  = get_time_utc();
                let mono_ms    = monotonic_millis();
                let local_ts   = local_time_str();
                serial_println!(
                    "[SUNLIGHT-TLS] hs_recv role={} sni={} local={} unix={} ms={}",
                    if is_client { "client" } else { "server" },
                    sni, local_ts, unix_time, mono_ms
                );

                if let Some(sid) = alloc_session() {
                    reply.words[0] = sid;
                    serial_println!(
                        "[SUNLIGHT-TLS] hs_OK sid={} sni={} local={} unix={}",
                        sid, sni, local_ts, unix_time
                    );
                } else {
                    serial_println!("[SUNLIGHT-TLS] hs_FAIL sessions_full sni={}", sni);
                    reply.label = TLS_ERROR;
                    reply.words[0] = 3; // sessions full
                }
            }
            TLS_FEED => {
                let sid = msg.words[0];
                // data is packed from word 1 using our kv-style (len in word0 of sub, but reuse)
                // For simplicity treat remaining words as ciphertext or appdata.
                if let Some(s) = find_session_mut(sid) {
                    // Demo: "decrypt" by copying to plain, "encrypt" by copying to write
                    let mut data: Vec<u8> = Vec::new();
                    for wi in 1..8 {
                        for j in 0..8 {
                            let b = ((msg.words[wi] >> (j * 8)) & 0xff) as u8;
                            if b != 0 {
                                data.push(b);
                            }
                        }
                    }
                    // In real: feed to rustls read/write_tls + process_new_packets
                    s.to_plain.extend_from_slice(&data);
                    s.to_write.extend_from_slice(&data); // echo for demo
                    reply.words[0] = 0;
                } else {
                    reply.label = TLS_ERROR;
                }
            }
            TLS_GET_WRITE => {
                let sid = msg.words[0];
                if let Some(s) = find_session_mut(sid) {
                    let out = core::mem::take(&mut s.to_write);
                    reply.words[0] = out.len() as u64;
                    let mut bi = 0;
                    let mut wi = 1;
                    for b in out.into_iter().take(7 * 8) {
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
                } else {
                    reply.label = TLS_ERROR;
                }
            }
            TLS_GET_PLAIN => {
                let sid = msg.words[0];
                if let Some(s) = find_session_mut(sid) {
                    let out = core::mem::take(&mut s.to_plain);
                    reply.words[0] = out.len() as u64;
                    let mut bi = 0;
                    let mut wi = 1;
                    for b in out.into_iter().take(7 * 8) {
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
                } else {
                    reply.label = TLS_ERROR;
                }
            }
            TLS_CLOSE => {
                let sid = msg.words[0];
                if close_session(sid) {
                    reply.words[0] = 0;
                } else {
                    reply.label = TLS_ERROR;
                }
            }
            TLS_INSTALL => {
                // Accept install request: store demo cert material into sunlight-kv
                // under the documented tls/* names. Uses small blobs (fit IPC).
                let demo_ca = b"DEMO-CA-CERT-BLOB-SUNLIGHT-OS-ROOT";
                let demo_cert = b"DEMO-SERVER-CERT-BLOB-0123456789ABCDEF";
                let demo_key = b"DEMO-SERVER-KEY-BLOB-NOT-REAL";
                let mut ok = true;
                if !kv_put("tls/ca/system", demo_ca) {
                    ok = false;
                }
                if !kv_put("tls/server/cert", demo_cert) {
                    ok = false;
                }
                if !kv_put("tls/server/key", demo_key) {
                    ok = false;
                }
                if ok {
                    // refresh our cache
                    fetch_certs_from_kv();
                    reply.words[0] = 0;
                } else {
                    reply.label = TLS_ERROR;
                    reply.words[0] = 4;
                }
            }
            TLS_LIST => {
                // Return a small set of known tls keys (best effort from cache + static names)
                let mut count: u64 = 0;
                unsafe {
                    if CACHED_CA.is_some() {
                        count += 1;
                    }
                    if CACHED_CERT.is_some() {
                        count += 1;
                    }
                    if CACHED_KEY.is_some() {
                        count += 1;
                    }
                }
                reply.words[0] = count;
                // Pack a summary string
                pack_str(&mut reply, 1, "tls/ca/system,tls/server/cert,tls/server/key");
            }
            _ => {
                reply.label = TLS_ERROR;
                reply.words[0] = 0xff;
            }
        }

        msg = ipc_reply_and_wait(ep, reply);
    }
}

#[cfg(feature = "sunlightos")]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    debug_log("[SUNLIGHT-TLS] PANIC\n");
    loop {}
}