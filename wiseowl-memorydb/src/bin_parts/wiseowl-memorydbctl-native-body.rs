// Native CLI for wiseowl-memorydbctl (Phase 3.875: status + census).

use core::fmt::Write;

use sunlight_ipc::{ipc_call, nameserver_lookup, IpcMsg, ProcessExit};
use sunlight_libc as libc;

use wiseowl_memorydb::native_ipc::MemoryDbOp;
use wiseowl_memorydb::ENDPOINT_NAME;

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
        .or_else(|| nameserver_lookup("wiseowl-memorydb"))
    else {
        println!("wiseowl-memorydb not registered");
        ProcessExit::exit(1);
    };

    let ok = match command {
        "status" | "health" => {
            let health = ipc_call(cap, IpcMsg::with_label(MemoryDbOp::GetHealth as u64));
            if health.label == MemoryDbOp::Reply as u64 {
                println!(
                    "health ready={} state={}",
                    health.words[0],
                    health.words[1]
                );
            } else {
                println!("health error");
                ProcessExit::exit(1);
            }
            let stats = ipc_call(cap, IpcMsg::with_label(MemoryDbOp::GetStats as u64));
            if stats.label == MemoryDbOp::Reply as u64 {
                println!("database_generation={}", stats.words[0]);
                println!("last_committed_sequence={}", stats.words[1]);
                println!("record_count_active={}", stats.words[2]);
                println!("wal_bytes={}", stats.words[3]);
                println!("segment_count={}", stats.words[4]);
                println!("transaction_commits={}", stats.words[5]);
                true
            } else {
                println!("stats error");
                false
            }
        }
        "census" => {
            let source = if argc >= 4 && args[2] == "--source" {
                args[3].parse::<u64>().unwrap_or(0)
            } else {
                0
            };
            let reply = ipc_call(
                cap,
                IpcMsg::with_label(MemoryDbOp::GenerationCensus as u64)
                    .word(0, source)
                    .word(1, 4096),
            );
            if reply.label == MemoryDbOp::Reply as u64 {
                println!(
                    "sources={} active_generations={} superseded={} multi_active={} dup_import_keys={} orphan_chunks={}",
                    reply.words[0],
                    reply.words[1],
                    reply.words[2],
                    reply.words[3],
                    reply.words[4],
                    reply.words[5]
                );
                true
            } else {
                println!("census error");
                false
            }
        }
        "verify-generations" => {
            let reply = ipc_call(cap, IpcMsg::with_label(MemoryDbOp::VerifyGenerations as u64));
            if reply.label == MemoryDbOp::Reply as u64 {
                println!(
                    "verify_ok={} multi_active={} dup_import_keys={} orphan_chunks={} invalid_chains={} active_generations={}",
                    reply.words[0],
                    reply.words[1],
                    reply.words[2],
                    reply.words[3],
                    reply.words[4],
                    reply.words[5]
                );
                reply.words[0] == 1
            } else {
                println!("verify-generations error");
                false
            }
        }
        _ => {
            println!("usage: wiseowl-memorydbctl status|census|verify-generations");
            false
        }
    };
    ProcessExit::exit(if ok { 0 } else { 1 });
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    ProcessExit::exit(101);
}
