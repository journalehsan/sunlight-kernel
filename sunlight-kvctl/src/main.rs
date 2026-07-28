//! sunlight-kvctl — CLI client for the sunlight-kv key-value daemon.
//!
//! Build modes:
//! - "host" (default): std binary, connects via Unix domain socket + bincode.
//! - "sunlightos": no_std binary embedded in the kernel; talks to
//!   sunlight-kv via the kernel IPC nameserver.

#![cfg_attr(feature = "sunlightos", no_std)]
#![cfg_attr(feature = "sunlightos", no_main)]

#[cfg(feature = "sunlightos")]
extern crate alloc;

#[cfg(feature = "sunlightos")]
use alloc::string::String;

// ---------------------------------------------------------------------------
// Host build
// ---------------------------------------------------------------------------

#[cfg(feature = "host")]
mod cli;

#[cfg(feature = "host")]
fn main() {
    use cli::{execute, parse_args, CliError};
    use std::process;

    let raw_args: Vec<String> = std::env::args().skip(1).collect();

    let cmd = match parse_args(&raw_args) {
        Ok(c) => c,
        Err(CliError::Usage(msg)) => {
            eprintln!("{}", msg);
            process::exit(2);
        }
        Err(e) => {
            eprintln!("sunlight-kvctl: {}", e);
            process::exit(2);
        }
    };

    match execute(cmd) {
        Ok(out) => println!("{}", out),
        Err(CliError::NotFound) => {
            eprintln!("not found");
            process::exit(1);
        }
        Err(CliError::PermissionDenied) => {
            eprintln!("permission denied");
            process::exit(1);
        }
        Err(CliError::Daemon(msg)) => {
            eprintln!("error: {}", msg);
            process::exit(1);
        }
        Err(e) => {
            eprintln!("sunlight-kvctl: {}", e);
            process::exit(1);
        }
    }
}

// ---------------------------------------------------------------------------
// SunlightOS (no_std) build
// ---------------------------------------------------------------------------

#[cfg(feature = "sunlightos")]
struct BumpAllocator;

#[cfg(feature = "sunlightos")]
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

#[cfg(feature = "sunlightos")]
#[global_allocator]
static BUMP: BumpAllocator = BumpAllocator;

#[cfg(feature = "sunlightos")]
use sunlight_ipc::{
    ipc_call, nameserver_lookup, shm_free, shm_map, CapabilityToken, IpcMsg, IPC_REGISTER_WORDS,
};

// Share the fixed stats wire layout with sunlight-kv without linking the full
// daemon crate (which would pull a second GlobalAlloc into this binary).
#[cfg(feature = "sunlightos")]
#[path = "../../sunlight-kv/src/stats_wire.rs"]
mod stats_wire;

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

// Read a NUL-terminated C string from a raw pointer.
#[cfg(feature = "sunlightos")]
unsafe fn cstr_to_str(ptr: *const u8) -> &'static str {
    let mut len = 0;
    while *ptr.add(len) != 0 {
        len += 1;
    }
    core::str::from_utf8(core::slice::from_raw_parts(ptr, len)).unwrap_or("")
}

// Pack/unpack using repository style (multiple words, length prefix in words[0]).
#[cfg(feature = "sunlightos")]
fn pack_kv_payload(msg: &mut IpcMsg, key: &str, value: &[u8]) {
    let kb = key.as_bytes();
    let vb = value;
    if kb.len() > 0xffff || vb.len() > 0xffff || kb.len() + vb.len() > (IPC_REGISTER_WORDS - 1) * 8
    {
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
    let mut v: heapless::Vec<u8, 64> = heapless::Vec::new();
    let mut rem = klen;
    let mut wi = 1usize;
    while rem > 0 && wi < IPC_REGISTER_WORDS {
        for j in 0..8 {
            if rem == 0 {
                break;
            }
            let _ = v.push(((msg.words[wi] >> (j * 8)) & 0xff) as u8);
            rem -= 1;
        }
        wi += 1;
    }
    String::from(core::str::from_utf8(&v).unwrap_or(""))
}

#[cfg(feature = "sunlightos")]
fn unpack_kv_value(msg: &IpcMsg) -> heapless::Vec<u8, 64> {
    let klen = (msg.words[0] & 0xffff) as usize;
    let vlen = ((msg.words[0] >> 16) & 0xffff) as usize;
    let mut v: heapless::Vec<u8, 64> = heapless::Vec::new();
    let mut rem = vlen;
    let mut wi = 1usize + (klen + 7) / 8;
    while rem > 0 && wi < IPC_REGISTER_WORDS {
        for j in 0..8 {
            if rem == 0 {
                break;
            }
            let _ = v.push(((msg.words[wi] >> (j * 8)) & 0xff) as u8);
            rem -= 1;
        }
        wi += 1;
    }
    v
}

#[cfg(feature = "sunlightos")]
fn pack_str(msg: &mut IpcMsg, start_word: usize, s: &str) -> bool {
    if start_word >= IPC_REGISTER_WORDS {
        return false;
    }
    let b = s.as_bytes();
    if b.len() > (IPC_REGISTER_WORDS - start_word) * 8 {
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

// IPC op labels — must stay in sync with sunlight-kv/src/main.rs.
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
#[cfg(feature = "sunlightos")]
const KV_STATS: u64 = 0x4B0B;

#[cfg(feature = "sunlightos")]
fn print_usage() {
    println!("sunlight-kvctl - client for sunlight-kv daemon");
    println!("");
    println!("Usage:");
    println!("  sunlight-kvctl put KEY VALUE");
    println!("  sunlight-kvctl get KEY");
    println!("  sunlight-kvctl delete KEY");
    println!("  sunlight-kvctl scan");
    println!("  sunlight-kvctl stats");
}

#[cfg(feature = "sunlightos")]
fn print_stats(s: &stats_wire::StatsSnapshotV1) {
    use stats_wire::*;
    let started = s.get(F_STARTED_MS);
    let now = s.get(F_NOW_MS);
    let uptime = now.saturating_sub(started);

    println!("sunlight-kv stats (schema v{})", s.get(F_VERSION));
    println!("");
    println!("runtime (monotonic ms since boot)");
    println!(
        "  started_ms={}  now_ms={}  uptime_ms={}",
        started, now, uptime
    );
    println!(
        "  last_activity_ms={}  last_error_ms={}  last_error_code={}",
        s.get(F_LAST_ACTIVITY_MS),
        s.get(F_LAST_ERROR_MS),
        s.get(F_LAST_ERROR_CODE)
    );
    println!(
        "  volatile_only={}  (1=persistence disabled/failed)",
        s.get(F_VOLATILE_ONLY)
    );

    println!("");
    println!("request traffic (lifetime totals)");
    println!(
        "  total={}  ok={}  err={}",
        s.get(F_REQUESTS_TOTAL),
        s.get(F_REQUESTS_OK),
        s.get(F_REQUESTS_ERR)
    );
    println!(
        "  by_op put={} get={} delete={} scan={}",
        s.get(F_OP_PUT),
        s.get(F_OP_GET),
        s.get(F_OP_DELETE),
        s.get(F_OP_SCAN)
    );
    println!(
        "  by_op put_shm={} get_shm={} put_shm2={} get_shm2={} delete_shm2={}",
        s.get(F_OP_PUT_SHM),
        s.get(F_OP_GET_SHM),
        s.get(F_OP_PUT_SHM2),
        s.get(F_OP_GET_SHM2),
        s.get(F_OP_DELETE_SHM2)
    );
    println!("  by_op stats={}", s.get(F_OP_STATS));

    println!("");
    println!("event-loop activity (lifetime totals)");
    println!(
        "  iterations={}  recv_blocking={}  try_recv_hit={}  try_recv_miss={}",
        s.get(F_LOOP_ITERATIONS),
        s.get(F_RECV_BLOCKING),
        s.get(F_TRY_RECV_HIT),
        s.get(F_TRY_RECV_MISS)
    );

    println!("");
    println!("live resources (current / high-water)");
    println!(
        "  key_count={}  payload_bytes={}  mutations_total={}",
        s.get(F_KEY_COUNT),
        s.get(F_PAYLOAD_BYTES),
        s.get(F_MUTATIONS)
    );
    println!(
        "  persist_queue_depth={}  persist_queue_hwm={}",
        s.get(F_PERSIST_QUEUE_DEPTH),
        s.get(F_PERSIST_QUEUE_HWM)
    );

    println!("");
    println!("cleanup/persistence (lifetime totals)");
    println!(
        "  flush_ok={}  flush_fail={}  record_bytes_flushed={}  skipped_volatile={}",
        s.get(F_PERSIST_FLUSH_OK),
        s.get(F_PERSIST_FLUSH_FAIL),
        s.get(F_PERSIST_RECORD_BYTES),
        s.get(F_PERSIST_SKIPPED_VOLATILE)
    );

    println!("");
    println!("errors (lifetime totals)");
    println!(
        "  decode_errors={}  reply_error_labels={}  unknown_opcodes={}",
        s.get(F_DECODE_ERRORS),
        s.get(F_REPLY_ERROR_LABELS),
        s.get(F_UNKNOWN_OPCODES)
    );

    println!("");
    println!(
        "client attribution (kernel badge PID; fixed {} slots)",
        STATS_CLIENT_SLOTS
    );
    println!(
        "  slots_used={}  pids_tracked={}  other_client_requests={}",
        s.get(F_CLIENT_SLOTS_USED),
        s.get(F_CLIENT_PIDS_TRACKED),
        s.get(F_OTHER_CLIENT_REQUESTS)
    );
    let mut any = false;
    for i in 0..STATS_CLIENT_SLOTS {
        let pid = s.client_pid(i);
        let n = s.client_requests(i);
        if pid != 0 {
            any = true;
            println!("  pid={}  requests={}", pid, n);
        }
    }
    if !any {
        println!("  (no clients tracked yet)");
    }
    println!("");
    println!("note: sample twice and subtract for rates; counters are saturating u64");
}

// Kernel sets rdi=argc, rsi=argv, rdx=envp per the SysV x86-64 ABI.
#[cfg(feature = "sunlightos")]
#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8, _envp: *const *const u8) -> ! {
    // Collect argv strings (slot 0 is the program name; skip it).
    let mut args: heapless::Vec<&str, 8> = heapless::Vec::new();
    for i in 0..argc.min(8) as usize {
        let ptr = unsafe { *argv.add(i) };
        if ptr.is_null() {
            break;
        }
        let _ = args.push(unsafe { cstr_to_str(ptr) });
    }

    let subargs: &[&str] = if args.len() > 1 { &args[1..] } else { &[] };
    let cmd = subargs.first().copied().unwrap_or("");

    if cmd.is_empty() || cmd == "help" || cmd == "-h" || cmd == "--help" {
        print_usage();
        sunlight_libc::exit(0);
    }

    let Some(kv_cap) = nameserver_lookup("sunlight-kv") else {
        println!("sunlight-kvctl: sunlight-kv daemon not running");
        sunlight_libc::exit(1);
    };

    match cmd {
        "put" | "p" => {
            if subargs.len() < 3 {
                println!("usage: sunlight-kvctl put KEY VALUE");
                sunlight_libc::exit(2);
            }
            let key = subargs[1];
            let val = subargs[2].as_bytes();
            if key.len() + val.len() > (IPC_REGISTER_WORDS - 1) * 8 {
                println!("ERROR: inline key/value too large for register IPC");
                sunlight_libc::exit(2);
            }
            let mut msg = IpcMsg::empty();
            msg.label = KV_PUT;
            pack_kv_payload(&mut msg, key, val);
            let reply = ipc_call(kv_cap, msg);
            if reply.label == KV_REPLY && reply.words[0] == 0 {
                println!("OK");
            } else {
                println!("ERROR: put failed");
                sunlight_libc::exit(1);
            }
        }
        "get" | "g" => {
            if subargs.len() < 2 {
                println!("usage: sunlight-kvctl get KEY");
                sunlight_libc::exit(2);
            }
            let key = subargs[1];
            let mut msg = IpcMsg::empty();
            msg.label = KV_GET;
            if !pack_str(&mut msg, 1, key) {
                println!("ERROR: key too long for register IPC");
                sunlight_libc::exit(2);
            }
            // Also set word0 len for the new unpacker
            let kb = key.as_bytes();
            msg.words[0] = kb.len() as u64;
            let reply = ipc_call(kv_cap, msg);
            if reply.label == KV_ERROR {
                println!("not found");
                sunlight_libc::exit(1);
            } else if reply.label == KV_VALUE {
                // Unpack value from reply and print (prefer utf8)
                let v = unpack_kv_value(&reply);
                match core::str::from_utf8(&v) {
                    Ok(s) => println!("{}", s),
                    Err(_) => println!("<binary:{} bytes>", v.len()),
                }
            } else if reply.label == KV_REPLY && reply.words[0] > 0 {
                println!("found (length: {} bytes)", reply.words[0]);
            } else {
                println!("ERROR: unexpected reply");
                sunlight_libc::exit(1);
            }
        }
        "delete" | "del" | "d" | "rm" => {
            if subargs.len() < 2 {
                println!("usage: sunlight-kvctl delete KEY");
                sunlight_libc::exit(2);
            }
            let key = subargs[1];
            let mut msg = IpcMsg::empty();
            msg.label = KV_DELETE;
            let kb = key.as_bytes();
            msg.words[0] = kb.len() as u64;
            if !pack_str(&mut msg, 1, key) {
                println!("ERROR: key too long for register IPC");
                sunlight_libc::exit(2);
            }
            let reply = ipc_call(kv_cap, msg);
            if reply.label == KV_REPLY && reply.words[0] == 0 {
                println!("OK");
            } else if reply.label == KV_ERROR {
                println!("not found");
                sunlight_libc::exit(1);
            } else {
                println!("ERROR: delete failed");
                sunlight_libc::exit(1);
            }
        }
        "scan" | "s" | "ls" => {
            let mut msg = IpcMsg::empty();
            msg.label = KV_SCAN;
            let reply = ipc_call(kv_cap, msg);
            if reply.label == KV_REPLY {
                println!("{} keys in store", reply.words[0]);
            } else {
                println!("ERROR: scan failed");
                sunlight_libc::exit(1);
            }
        }
        "stats" | "stat" | "status" => {
            let mut msg = IpcMsg::empty();
            msg.label = KV_STATS;
            let reply = ipc_call(kv_cap, msg);
            if reply.label == KV_ERROR {
                println!("ERROR: stats failed (daemon error)");
                sunlight_libc::exit(1);
            }
            if reply.label != KV_REPLY {
                println!("ERROR: unexpected stats reply label={:#x}", reply.label);
                sunlight_libc::exit(1);
            }
            let version = reply.words[0];
            let nbytes = reply.words[1] as usize;
            let tok = reply.caps[0];
            if version != stats_wire::STATS_VERSION {
                println!(
                    "ERROR: unsupported stats schema version {} (want {})",
                    version,
                    stats_wire::STATS_VERSION
                );
                if tok != CapabilityToken::INVALID {
                    let _ = shm_free(tok);
                }
                sunlight_libc::exit(1);
            }
            if tok == CapabilityToken::INVALID || nbytes < stats_wire::STATS_BYTES {
                println!("ERROR: stats reply missing SHM payload");
                if tok != CapabilityToken::INVALID {
                    let _ = shm_free(tok);
                }
                sunlight_libc::exit(1);
            }
            let ptr = match shm_map(tok) {
                Ok(p) => p,
                Err(_) => {
                    println!("ERROR: shm_map failed for stats");
                    let _ = shm_free(tok);
                    sunlight_libc::exit(1);
                }
            };
            let bytes = unsafe { core::slice::from_raw_parts(ptr, stats_wire::STATS_BYTES) };
            let snap = match stats_wire::StatsSnapshotV1::decode(bytes) {
                Some(s) => s,
                None => {
                    println!("ERROR: stats snapshot decode failed");
                    let _ = shm_free(tok);
                    sunlight_libc::exit(1);
                }
            };
            let _ = shm_free(tok);
            print_stats(&snap);
        }
        _ => {
            println!("unknown command (try: put get delete scan stats)");
            print_usage();
            sunlight_libc::exit(1);
        }
    }

    sunlight_libc::exit(0);
}

#[cfg(feature = "sunlightos")]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    stdout_write("sunlight-kvctl: PANIC\n");
    loop {}
}
