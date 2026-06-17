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
use sunlight_ipc::{ipc_call, nameserver_lookup, shm_alloc, shm_free, IpcMsg};

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
fn unpack_str(words: &[u64; 8], start_word: usize, max_len: usize) -> heapless::String<64> {
    let mut out = heapless::String::<64>::new();
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
            let _ = out.push(byte as char);
            i += 1;
        }
    }
    out
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
    println!("  certificatectl install ca <name> <der-path>");
    println!("  certificatectl list");
    println!("");
    println!("Examples:");
    println!("  certificatectl install ca myroot /etc/ssl/myroot.der");
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
            // certificatectl install ca <name> <der-path>
            // (also accepts: install <name> <der-path>)
            let (name, path) = if subargs.len() >= 4 && subargs[1] == "ca" {
                (subargs[2], subargs[3])
            } else if subargs.len() >= 3 {
                (subargs[1], subargs[2])
            } else {
                println!("usage: certificatectl install ca <name> <der-path>");
                sunlight_libc::exit(2);
            };

            // Read the DER file from VFS (one shm page = 4096 bytes max).
            let fd = match sunlight_libc::open(path.as_bytes()) {
                Ok(fd) => fd,
                Err(_) => {
                    println!("install: cannot open {}", path);
                    sunlight_libc::exit(1);
                }
            };
            let mut der = [0u8; 4096];
            let mut total = 0usize;
            loop {
                if total >= der.len() {
                    break;
                }
                match sunlight_libc::read(fd, &mut der[total..]) {
                    Ok(0) => break,
                    Ok(n) => total += n,
                    Err(_) => break,
                }
            }
            let _ = sunlight_libc::close(fd);
            if total == 0 {
                println!("install: {} is empty or unreadable", path);
                sunlight_libc::exit(1);
            }

            // Hand the DER bytes to sunlight-tls via a shared page; it stores
            // them in sunlight-kv under tls/ca/<name> and reloads its trust.
            let (ptr, tok) = match shm_alloc() {
                Ok(p) => p,
                Err(_) => {
                    println!("install: shm_alloc failed");
                    sunlight_libc::exit(1);
                }
            };
            unsafe {
                core::ptr::copy_nonoverlapping(der.as_ptr(), ptr, total);
            }
            let mut msg = IpcMsg::with_label(TLS_INSTALL)
                .word(0, total as u64)
                .with_cap(0, tok);
            pack_str(&mut msg, 2, name);
            let reply = ipc_call(tls_cap, msg);
            let _ = shm_free(tok);
            if reply.label == TLS_REPLY {
                println!("install: OK ({} bytes -> sunlight-kv tls/ca/{})", total, name);
            } else {
                println!("install: ERROR (code {})", reply.words[0]);
                sunlight_libc::exit(1);
            }
        }
        "list" | "ls" => {
            let reply = ipc_call(tls_cap, IpcMsg::with_label(TLS_LIST));
            if reply.label == TLS_REPLY {
                println!("trusted CAs in store: {}", reply.words[0]);
                let names = unpack_str(&reply.words, 1, 56);
                if !names.is_empty() {
                    println!("  {}", names);
                }
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