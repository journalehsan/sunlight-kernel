


use sunlight_ipc::{ipc_call, nameserver_lookup, CapabilityToken, IpcMsg, SHM_PAGE};
use sunlight_libc as libc;

use wiseowl_memory::native_ipc::{MemoryOp, INLINE_PAYLOAD_THRESHOLD, NATIVE_PROTOCOL_VERSION};

macro_rules! println {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let mut buf = heapless::String::<512>::new();
        let _ = write!(&mut buf, $($arg)*);
        stdout_write(buf.as_str());
        stdout_write("\n");
    }};
}

fn stdout_write(s: &str) {
    let mut data = s.as_bytes();
    while !data.is_empty() {
        match libc::write(libc::STDOUT, data) {
            Ok(n) if n > 0 => data = &data[n..],
            _ => break,
        }
    }
}

fn lookup() -> Option<CapabilityToken> {
    nameserver_lookup("wiseowl-memoryd")
}

fn usage() {
    println!(
        "usage: wiseowl-memoryctl <command>\n\
         commands:\n\
           status       service health and generation\n\
           stats        logical memory counters\n\
           sessions     list sessions\n\
           list         list entries (admin)\n\
           inspect <id> metadata for memory id\n\
           maintenance  run bounded maintenance\n\
           transport    native endpoint / SHM / protocol info"
    );
}

const MAX_ARGS: usize = 16;

#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8) -> ! {
    let mut storage = [""; MAX_ARGS];
    let count = unsafe { collect_args(argc, argv, &mut storage) };
    let args = &storage[..count];
    let cmd = args.get(1).copied().unwrap_or("status");
    let Some(cap) = lookup() else {
        println!("wiseowl-memoryctl: service not registered (wiseowl-memoryd)");
        sunlight_ipc::ProcessExit::exit(1);
    };

    match cmd {
        "status" | "transport" => {
            let reply = ipc_call(cap, IpcMsg::with_label(MemoryOp::TransportInfo.label()));
            if reply.label == MemoryOp::Error.label() {
                println!("error code={}", reply.words[0]);
                sunlight_ipc::ProcessExit::exit(1);
            }
            let ver = reply.words[0];
            let inline_thr = reply.words[1];
            let health = reply.words[2];
            let degraded = reply.words[3];
            let gen = reply.words[4];
            let health_s = match health {
                1 => "Starting",
                2 => "Ready",
                3 => "Degraded",
                4 => "Stopping",
                5 => "Failed",
                _ => "Unknown",
            };
            println!("service: wiseowl-memoryd");
            println!("endpoint: wiseowl-memoryd (nameserver)");
            println!("health: {health_s}");
            println!("degraded_flags: {degraded:#x}");
            println!("protocol_version: {ver} (native {NATIVE_PROTOCOL_VERSION})");
            println!("inline_threshold: {inline_thr} (cfg {INLINE_PAYLOAD_THRESHOLD})");
            println!("shm_page: {SHM_PAGE}");
            println!("id_generation: {gen}");
            println!("kv: sunlight-kv via native IPC");
            println!("spill: /state/wiseowl-memoryd");
        }
        "stats" => {
            let reply = ipc_call(cap, IpcMsg::with_label(MemoryOp::GetStats.label()));
            if reply.label == MemoryOp::Error.label() {
                println!("error code={}", reply.words[0]);
                sunlight_ipc::ProcessExit::exit(1);
            }
            // words: tag=20, entry_count, ram, cold, sessions
            println!("entries: {}", reply.words[1]);
            println!("logical_ram_bytes: {}", reply.words[2]);
            println!("cold_compressed_bytes: {}", reply.words[3]);
            println!("sessions: {}", reply.words[4]);
        }
        "sessions" => {
            let reply = ipc_call(cap, IpcMsg::with_label(MemoryOp::ListSessions.label()));
            if reply.label == MemoryOp::Error.label() {
                println!("error code={}", reply.words[0]);
                sunlight_ipc::ProcessExit::exit(1);
            }
            println!("session_count: {}", reply.words[1]);
            if reply.words[1] > 0 {
                println!("first_session: {}", reply.words[2]);
            }
        }
        "list" => {
            let mut msg = IpcMsg::with_label(MemoryOp::ListEntries.label());
            msg.words[0] = 0; // all sessions
            msg.words[1] = 64;
            msg.word_count = 2;
            let reply = ipc_call(cap, msg);
            if reply.label == MemoryOp::Error.label() {
                println!("error code={}", reply.words[0]);
                sunlight_ipc::ProcessExit::exit(1);
            }
            println!("listed: {}", reply.words[1]);
        }
        "inspect" => {
            let id: u64 = args.get(2).and_then(|s| parse_u64(s)).unwrap_or(0);
            if id == 0 {
                println!("usage: wiseowl-memoryctl inspect <memory-id>");
                sunlight_ipc::ProcessExit::exit(2);
            }
            let mut msg = IpcMsg::with_label(MemoryOp::ReadEntry.label());
            msg.words[0] = id;
            msg.words[1] = 0; // metadata only
            msg.word_count = 2;
            let reply = ipc_call(cap, msg);
            if reply.label == MemoryOp::Error.label() {
                println!("error code={}", reply.words[0]);
                sunlight_ipc::ProcessExit::exit(1);
            }
            println!("memory_id: {}", reply.words[1]);
            println!("session_id: {}", reply.words[2]);
            println!("state: {}", reply.words[3]);
            println!("promoted: {}", reply.words[4]);
            println!("payload_len: {}", reply.words[5]);
        }
        "maintenance" => {
            let reply = ipc_call(cap, IpcMsg::with_label(MemoryOp::RunMaintenance.label()));
            if reply.label == MemoryOp::Error.label() {
                println!("error code={}", reply.words[0]);
                sunlight_ipc::ProcessExit::exit(1);
            }
            println!("scanned: {}", reply.words[1]);
            println!("bytes_reclaimed: {}", reply.words[2]);
            println!("expired: {}", reply.words[3]);
        }
        "help" | "-h" | "--help" => usage(),
        other => {
            println!("unknown command: {other}");
            usage();
            sunlight_ipc::ProcessExit::exit(2);
        }
    }
    sunlight_ipc::ProcessExit::exit(0);
}

unsafe fn collect_args<'a>(
    argc: u64,
    argv: *const *const u8,
    storage: &mut [&'a str; MAX_ARGS],
) -> usize {
    let n = (argc as usize).min(MAX_ARGS);
    for i in 0..n {
        let p = *argv.add(i);
        if p.is_null() {
            storage[i] = "";
            continue;
        }
        let mut len = 0usize;
        while *p.add(len) != 0 && len < 256 {
            len += 1;
        }
        storage[i] = core::str::from_utf8(core::slice::from_raw_parts(p, len)).unwrap_or("");
    }
    n
}

fn parse_u64(s: &str) -> Option<u64> {
    let mut n = 0u64;
    if s.is_empty() {
        return None;
    }
    for b in s.bytes() {
        if b.is_ascii_digit() {
            n = n.checked_mul(10)?.checked_add((b - b'0') as u64)?;
        } else {
            return None;
        }
    }
    Some(n)
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        sunlight_ipc::process_yield();
    }
}
