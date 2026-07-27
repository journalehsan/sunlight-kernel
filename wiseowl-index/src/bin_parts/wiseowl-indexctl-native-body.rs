// Native CLI for wiseowl-indexctl (Phase 3.5 diagnostics).

use core::fmt::Write;

use sunlight_ipc::{ipc_call, nameserver_lookup, shm_alloc, shm_free, IpcMsg, ProcessExit, SHM_PAGE};
use sunlight_libc as libc;

use wiseowl_index::native_ipc::IndexOp;
use wiseowl_index::ENDPOINT_NAME;

fn stdout_write(s: &str) {
    let mut data = s.as_bytes();
    while !data.is_empty() {
        match libc::write(libc::STDOUT, data) {
            Ok(n) if n > 0 => data = &data[n..],
            _ => break,
        }
    }
}

macro_rules! println {
    ($($arg:tt)*) => {{
        let mut buf = heapless::String::<256>::new();
        let _ = write!(&mut buf, $($arg)*);
        stdout_write(buf.as_str());
        stdout_write("\n");
    }};
}

#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8) -> ! {
    let mut args = [""; 4];
    let argc = unsafe { libc::crt0::collect_utf8_args(argc, argv, &mut args, 512) };
    let command = if argc >= 2 { args[1] } else { "status" };
    let Some(cap) = nameserver_lookup(ENDPOINT_NAME)
        .or_else(|| nameserver_lookup("wiseowl-indexd"))
    else {
        println!("wiseowl-indexd not registered");
        ProcessExit::exit(1);
    };

    let ok = match command {
        "status" | "health" => {
            let reply = ipc_call(cap, IpcMsg::with_label(IndexOp::GetHealth as u64));
            if reply.label == IndexOp::Reply as u64 {
                println!("service=wiseowl-indexd endpoint={}", ENDPOINT_NAME);
                println!("health={} state={} pending_imports={}", reply.words[0], reply.words[1], reply.words[2]);
                println!("memorydb_generation={} memorydb_ready={}", reply.words[3], reply.words[4]);
                true
            } else { println!("health error"); false }
        }
        "transport" => {
            let reply = ipc_call(cap, IpcMsg::with_label(IndexOp::GetTransport as u64));
            if reply.label == IndexOp::Reply as u64 {
                println!("inline_request_limit={}", reply.words[0]);
                println!("shm_support={}", reply.words[1]);
                println!("active_leases={}", reply.words[2]);
                println!("active_shm_bytes={}", reply.words[3]);
                println!("ownership_model=indexer-owner-retained");
                println!("memorydb_generation={}", reply.words[5]);
                true
            } else { false }
        }
        "memorydb" => {
            let reply = ipc_call(cap, IpcMsg::with_label(IndexOp::GetMemoryDb as u64));
            if reply.label == IndexOp::Reply as u64 {
                println!("endpoint=wiseowl.memorydb.v1 endpoint_generation={}", reply.words[2]);
                println!("protocol_version={} connection_state={}", reply.words[3], reply.words[0]);
                println!("database_health={} database_generation={}", reply.words[0], reply.words[1]);
                println!("disconnects={} reconnect_attempts={}", reply.words[4], reply.words[5]);
                true
            } else { println!("memorydb unavailable"); false }
        }
        "pending" => {
            let reply = ipc_call(cap, IpcMsg::with_label(IndexOp::GetPending as u64));
            println!("pending_imports={}", reply.words[0]);
            reply.label == IndexOp::Reply as u64
        }
        "reconcile" => {
            let reply = ipc_call(cap, IpcMsg::with_label(IndexOp::Reconcile as u64));
            println!("reconciled={}", reply.words[0]);
            reply.label == IndexOp::Reply as u64
        }
        "scan" => {
            let reply = ipc_call(cap, IpcMsg::with_label(IndexOp::StartScan as u64));
            println!("scan_result={}", if reply.label == IndexOp::Reply as u64 { "ok" } else { "rejected" });
            reply.label == IndexOp::Reply as u64
        }
        "stats" => {
            for page in 0..5u64 {
                let reply = ipc_call(cap, IpcMsg::with_label(IndexOp::GetStats as u64).word(0, page));
                if reply.label != IndexOp::Reply as u64 { ProcessExit::exit(1); }
                match page {
                    0 => println!("roots={} indexed={} unchanged={} strong_hash_runs={} sources={} generations={}", reply.words[0], reply.words[1], reply.words[2], reply.words[3], reply.words[4], reply.words[5]),
                    1 => println!("reparsed={} retokenized={} strong_hash_unchanged={} metadata_skips={} hash_bytes={} tokens={}", reply.words[0], reply.words[1], reply.words[2], reply.words[3], reply.words[4], reply.words[5]),
                    2 => println!("connect_attempts={} connect_successes={} disconnects={} reconnects={} retry_queue={} pending={}", reply.words[0], reply.words[1], reply.words[2], reply.words[3], reply.words[4], reply.words[5]),
                    3 => println!("shm_allocations={} shm_maps={} shm_unmaps={} shm_owner_frees={} shm_bytes_peak={} active_shm_leases={}", reply.words[0], reply.words[1], reply.words[2], reply.words[3], reply.words[4], reply.words[5]),
                    _ => println!("rejected_new={} rejected_cached={} gen_superseded={} delete_requests={} delete_commits={} missing_confirmed={}", reply.words[0], reply.words[1], reply.words[2], reply.words[3], reply.words[4], reply.words[5]),
                }
            }
            true
        }
        "digest" if argc >= 3 => {
            let sid = args[2].parse::<u64>().unwrap_or(0);
            let reply = ipc_call(cap, IpcMsg::with_label(IndexOp::GetDigest as u64).word(0, sid));
            if reply.label == IndexOp::Reply as u64 {
                println!("algorithm=SHA-256 version={} digest={:016x}... source_revision={} manifest_version={}", reply.words[1], reply.words[4], reply.words[2], reply.words[3]);
                println!("legacy_hash_present={} legacy_hash_authoritative=no", if reply.words[5] != 0 { "yes" } else { "no" });
                true
            } else { false }
        }
        "search" if argc >= 3 => run_search(cap, args[2].as_bytes()),
        "search-persian-fixture" => run_search(cap, "حافظه".as_bytes()),
        "inject-uncertain" if argc >= 3 => {
            let mode = if args[2] == "after" { 1 } else if args[2] == "before" { 2 } else { 0 };
            let reply = ipc_call(cap, IpcMsg::with_label(IndexOp::TestArmCommitCrash as u64).word(0, mode));
            println!("commit_crash_hook={} armed={}", args[2], reply.label == IndexOp::Reply as u64);
            reply.label == IndexOp::Reply as u64
        }
        "phase375-verdict" => {
            let reply = ipc_call(cap, IpcMsg::with_label(IndexOp::TestNativeVerdict as u64));
            println!("phase375_native_verdict={}", reply.words[0]);
            reply.label == IndexOp::Reply as u64
        }
        "phase3875-soak" => {
            let reply = ipc_call(cap, IpcMsg::with_label(IndexOp::TestPhase3875Soak as u64));
            println!("phase3875_soak={}", reply.words[0]);
            reply.label == IndexOp::Reply as u64
        }
        "inject-shm-crash" => {
            let reply = ipc_call(cap, IpcMsg::with_label(IndexOp::TestArmShmCrash as u64));
            println!("shm_crash_armed={}", reply.words[0]);
            reply.label == IndexOp::Reply as u64
        }
        _ => {
            println!("usage: wiseowl-indexctl status|health|memorydb|transport|pending|reconcile|stats|scan|digest <source-id>|search <text>|phase3875-soak");
            false
        }
    };
    ProcessExit::exit(if ok { 0 } else { 1 });
}

fn run_search(cap: sunlight_ipc::CapabilityToken, text: &[u8]) -> bool {
    if text.len() > SHM_PAGE { return false; }
    let Ok((ptr, token)) = shm_alloc() else { return false };
    unsafe { core::ptr::copy_nonoverlapping(text.as_ptr(), ptr, text.len()); }
    let reply = ipc_call(cap, IpcMsg::with_label(IndexOp::SearchText as u64)
        .word(0, 20).word(1, text.len() as u64).with_cap(0, token));
    let _ = shm_free(token);
    println!("label=lexical_relevance count={} first_memory_id={}", reply.words[0], reply.words[1]);
    reply.label == IndexOp::Reply as u64
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    ProcessExit::exit(101);
}
