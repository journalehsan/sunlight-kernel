//! certificatectl — CLI for sunlight-tls certificate management.
//!
//! Talks to the sunlight-tls daemon over kernel IPC (nameserver + IpcMsg).
//! Commands: install, remove, list
//!
//! Follows ADDING_A_BINARY.md exactly (host/sunlightos features, BumpAllocator,
//! stdout_write + println!, _start(argc, argv, envp), pack_str, nameserver_lookup + ipc_call).

#![cfg_attr(feature = "sunlightos", no_std)]
#![cfg_attr(feature = "sunlightos", no_main)]

#[cfg(feature = "sunlightos")]
extern crate alloc;

#[cfg(feature = "host")]
fn main() {
    // Simple host shim (real cert ops are performed inside the OS image).
    eprintln!("certificatectl: host stub. Run under SunlightOS for full IPC to sunlight-tls.");
}

// -----------------------------------------------------------------------------
// SunlightOS (no_std) build
// -----------------------------------------------------------------------------

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
const TLS_INSTALL: u64 = 0x5406;
#[cfg(feature = "sunlightos")]
const TLS_LIST: u64 = 0x5407;
#[cfg(feature = "sunlightos")]
const TLS_REPLY: u64 = 0x54FF;
#[cfg(feature = "sunlightos")]
const TLS_ERROR: u64 = 0x54EE;

#[cfg(feature = "sunlightos")]
fn print_usage() {
    println!("certificatectl - manage TLS certificates via sunlight-tls");
    println!("");
    println!("Usage:");
    println!("  certificatectl install [ca|server] [name]");
    println!("  certificatectl remove  <name>");
    println!("  certificatectl list");
    println!("");
    println!("Examples:");
    println!("  certificatectl install ca system");
    println!("  certificatectl install server");
    println!("  certificatectl list");
}

#[cfg(feature = "sunlightos")]
#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8, _envp: *const *const u8) -> ! {
    // argv parse exactly as documented
    let mut args: heapless::Vec<&str, 8> = heapless::Vec::new();
    for i in 0..argc.min(8) as usize {
        let ptr = unsafe { *argv.add(i) };
        if ptr.is_null() {
            break;
        }
        let mut len = 0;
        while unsafe { *ptr.add(len) } != 0 {
            len += 1;
        }
        let s = unsafe {
            core::str::from_utf8(core::slice::from_raw_parts(ptr, len)).unwrap_or("")
        };
        let _ = args.push(s);
    }

    let subargs: &[&str] = if args.len() > 1 { &args[1..] } else { &[] };
    let cmd = subargs.first().copied().unwrap_or("");

    if cmd.is_empty() || cmd == "help" || cmd == "-h" || cmd == "--help" {
        print_usage();
        sunlight_libc::exit(0);
    }

    let Some(tls_cap) = nameserver_lookup("sunlight-tls") else {
        println!("certificatectl: sunlight-tls daemon not running");
        sunlight_libc::exit(1);
    };

    match cmd {
        "install" | "add" => {
            // certificatectl install ca <name>  or  install server
            let what = if subargs.len() > 1 { subargs[1] } else { "server" };
            let mut msg = IpcMsg::empty();
            msg.label = TLS_INSTALL;
            pack_str(&mut msg, 1, what);
            let reply = ipc_call(tls_cap, msg);
            if reply.label == TLS_REPLY {
                println!("install: OK (certs pushed to sunlight-kv under tls/*)");
            } else {
                println!("install: ERROR");
                sunlight_libc::exit(1);
            }
        }
        "remove" | "rm" | "del" => {
            if subargs.len() < 2 {
                println!("usage: certificatectl remove <name>");
                sunlight_libc::exit(2);
            }
            // remove is advisory in this demo (real cleanup would issue kv delete)
            println!("remove: (demo) name={} - restart or use kvctl for full removal", subargs[1]);
        }
        "list" | "ls" => {
            let mut msg = IpcMsg::empty();
            msg.label = TLS_LIST;
            let reply = ipc_call(tls_cap, msg);
            if reply.label == TLS_REPLY {
                println!("known tls material: {}", reply.words[0]);
                // crude: the tls daemon packs a summary in words[1]
                println!("  tls/ca/system, tls/server/cert, tls/server/key");
            } else {
                println!("list: ERROR (daemon may be starting)");
            }
        }
        _ => {
            println!("unknown command");
            print_usage();
            sunlight_libc::exit(1);
        }
    }

    sunlight_libc::exit(0);
}

#[cfg(feature = "sunlightos")]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    stdout_write("certificatectl: PANIC\n");
    loop {}
}