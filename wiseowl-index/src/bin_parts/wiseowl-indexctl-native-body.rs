// Native CLI for wiseowl-indexctl (Phase 3.5 diagnostics).

use core::fmt::Write;

use sunlight_ipc::{ipc_call, nameserver_lookup, IpcMsg, ProcessExit};
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
pub extern "C" fn _start() -> ! {
    let Some(cap) = nameserver_lookup(ENDPOINT_NAME)
        .or_else(|| nameserver_lookup("wiseowl-indexd"))
    else {
        println!("wiseowl-indexd not registered");
        ProcessExit::exit(1);
    };

    println!("Indexer endpoint: {}", ENDPOINT_NAME);
    println!("MemoryDB endpoint: wiseowl.memorydb.v1");
    println!("Content digest: SHA-256 v1");
    println!("Manifest format: v2");

    let health = ipc_call(cap, IpcMsg::with_label(IndexOp::GetHealth as u64));
    if health.label == IndexOp::Reply as u64 {
        println!(
            "health ready={} state={} pending={} memdb_gen={} memdb_ready={}",
            health.words[0],
            health.words[1],
            health.words[2],
            health.words[3],
            health.words[4]
        );
    } else {
        println!("health error");
    }

    let transport = ipc_call(cap, IpcMsg::with_label(IndexOp::GetTransport as u64));
    if transport.label == IndexOp::Reply as u64 {
        println!(
            "transport memdb_gen={} pending={} manifest_v={} connected={}",
            transport.words[0],
            transport.words[1],
            transport.words[2],
            transport.words[3]
        );
    }

    let memdb = ipc_call(cap, IpcMsg::with_label(IndexOp::GetMemoryDb as u64));
    if memdb.label == IndexOp::Reply as u64 {
        println!(
            "memorydb ready={} generation={}",
            memdb.words[0],
            memdb.words[1]
        );
    } else {
        println!("memorydb unavailable (indexer may be Degraded)");
    }

    let stats = ipc_call(cap, IpcMsg::with_label(IndexOp::GetStats as u64));
    if stats.label == IndexOp::Reply as u64 {
        println!("configured_roots={}", stats.words[0]);
        println!("files_indexed={}", stats.words[1]);
        println!("files_unchanged={}", stats.words[2]);
        println!("strong_hash_files={}", stats.words[3]);
        println!("sources_tracked={}", stats.words[4]);
        println!("database_generations_created={}", stats.words[5]);
    } else {
        println!("stats error");
        ProcessExit::exit(1);
    }

    let scan = ipc_call(cap, IpcMsg::with_label(IndexOp::GetScanStatus as u64));
    if scan.label == IndexOp::Reply as u64 {
        println!("scanning={} last_scan_ns={}", scan.words[0], scan.words[1]);
    }

    ProcessExit::exit(0);
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    ProcessExit::exit(101);
}
