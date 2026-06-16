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

// ---------------------------------------------------------------------------
// Host build
// ---------------------------------------------------------------------------

#[cfg(feature = "host")]
mod cli;

#[cfg(feature = "host")]
fn main() {
    use std::process;
    use cli::{execute, parse_args, CliError};

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
use sunlight_ipc::{ipc_call, nameserver_lookup, IpcMsg};

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

// Pack the first 8 bytes of a key into u64 — matches daemon's pack_key.
#[cfg(feature = "sunlightos")]
fn pack_key(key: &str) -> u64 {
    let b = key.as_bytes();
    let mut out = 0u64;
    for i in 0..b.len().min(8) {
        out |= (b[i] as u64) << (i * 8);
    }
    out
}

// IPC op labels — must stay in sync with sunlight-kv/src/main.rs.
#[cfg(feature = "sunlightos")] const KV_PUT:    u64 = 0x4B01;
#[cfg(feature = "sunlightos")] const KV_GET:    u64 = 0x4B02;
#[cfg(feature = "sunlightos")] const KV_DELETE: u64 = 0x4B03;
#[cfg(feature = "sunlightos")] const KV_SCAN:   u64 = 0x4B04;
#[cfg(feature = "sunlightos")] const KV_REPLY:  u64 = 0x4BFF;
#[cfg(feature = "sunlightos")] const KV_ERROR:  u64 = 0x4BEE;

#[cfg(feature = "sunlightos")]
fn print_usage() {
    println!("sunlight-kvctl - client for sunlight-kv daemon");
    println!("");
    println!("Usage:");
    println!("  sunlight-kvctl put KEY VALUE");
    println!("  sunlight-kvctl get KEY");
    println!("  sunlight-kvctl delete KEY");
    println!("  sunlight-kvctl scan");
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
            let val = subargs[2];
            let mut msg = IpcMsg::empty();
            msg.label    = KV_PUT;
            msg.words[0] = pack_key(key);
            msg.words[1] = val.len().min(255) as u64;
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
            let mut msg = IpcMsg::empty();
            msg.label    = KV_GET;
            msg.words[0] = pack_key(subargs[1]);
            let reply = ipc_call(kv_cap, msg);
            if reply.label == KV_ERROR {
                println!("not found");
                sunlight_libc::exit(1);
            } else if reply.label == KV_REPLY && reply.words[0] == 1 {
                println!("found (length: {} bytes)", reply.words[1]);
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
            let mut msg = IpcMsg::empty();
            msg.label    = KV_DELETE;
            msg.words[0] = pack_key(subargs[1]);
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
        _ => {
            println!("unknown command (try: put get delete scan)");
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
