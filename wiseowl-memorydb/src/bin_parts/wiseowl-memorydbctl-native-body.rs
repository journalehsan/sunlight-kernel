// Native CLI for wiseowl-memorydbctl.

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
pub extern "C" fn _start() -> ! {
    let Some(cap) = nameserver_lookup(ENDPOINT_NAME)
        .or_else(|| nameserver_lookup("wiseowl-memorydb"))
    else {
        println!("wiseowl-memorydb not registered");
        ProcessExit::exit(1);
    };

    let health = ipc_call(cap, IpcMsg::with_label(MemoryDbOp::GetHealth as u64));
    if health.label == MemoryDbOp::Reply as u64 {
        println!(
            "health ready={} state={}",
            health.words[0],
            health.words[1]
        );
    } else {
        println!("health error");
    }

    let stats = ipc_call(cap, IpcMsg::with_label(MemoryDbOp::GetStats as u64));
    if stats.label == MemoryDbOp::Reply as u64 {
        println!("database_generation={}", stats.words[0]);
        println!("last_committed_sequence={}", stats.words[1]);
        println!("record_count_active={}", stats.words[2]);
        println!("wal_bytes={}", stats.words[3]);
        println!("segment_count={}", stats.words[4]);
        println!("transaction_commits={}", stats.words[5]);
    } else {
        println!("stats error");
        ProcessExit::exit(1);
    }
    ProcessExit::exit(0);
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    ProcessExit::exit(101);
}
