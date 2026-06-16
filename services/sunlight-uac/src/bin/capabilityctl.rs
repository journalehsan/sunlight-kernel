//! capabilityctl — control client for the `uac_service` daemon.
//!
//! Lives in RamFs at `/bin/capabilityctl`. It is a thin IPC client: it looks up
//! the `"uac"` endpoint and exercises the two daemon operations (runas cache
//! check + path access check), printing a short report. The wire protocol and
//! the underlying models are shared via the `sunlight_uac` library.

#![no_std]
#![no_main]

extern crate alloc;

struct BumpAllocator;

unsafe impl core::alloc::GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        static mut HEAP: [u8; 16 * 1024] = [0; 16 * 1024];
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

#[global_allocator]
static BUMP: BumpAllocator = BumpAllocator;

use sunlight_ipc::{ipc_call, nameserver_lookup, CapabilityToken, IpcMsg};

fn stdout_write(s: &str) {
    let mut data = s.as_bytes();
    while !data.is_empty() {
        match sunlight_libc::write(sunlight_libc::STDOUT, data) {
            Ok(n) if n > 0 => data = &data[n..],
            _ => break,
        }
    }
}

macro_rules! println {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let mut buf = heapless::String::<256>::new();
        let _ = write!(&mut buf, $($arg)*);
        stdout_write(&buf);
        stdout_write("\n");
    }};
}

/// Mirror of the daemon opcodes (see `uac_service.rs`).
const OP_RUNAS: u64 = 1;
const OP_CHECK: u64 = 2;
const OP_CHECK_EXEC: u64 = 5;
const REPLY_OK: u64 = 1;

/// Read a single line from stdin into `buf`, returning the byte length.
///
/// This is the passwd-style input path. It is written to be *impossible to
/// hang on*:
///   * `Ok(0)` means EOF (the TTY closed / no foreground input) — we stop
///     instead of spinning forever waiting for bytes that will never come.
///   * `Err(Again)` (would-block) yields the CPU and retries.
///   * any other error stops the read.
/// The newline terminates the line and is not stored.
fn read_line(buf: &mut [u8]) -> usize {
    let mut len = 0;
    while len < buf.len() {
        let mut byte = [0u8; 1];
        match sunlight_libc::read(sunlight_libc::STDIN, &mut byte) {
            Ok(0) => break,                       // EOF — never block forever.
            Ok(_) => {
                if byte[0] == b'\n' || byte[0] == b'\r' {
                    break;
                }
                buf[len] = byte[0];
                len += 1;
            }
            Err(sunlight_libc::Errno::Again) => sunlight_libc::yield_now(),
            Err(_) => break,
        }
    }
    len
}

/// Pack a NUL-terminated little-endian string into `msg.words[start..]`.
fn pack_str(msg: &mut IpcMsg, start: usize, s: &str) {
    let bytes = s.as_bytes();
    let mut i = 0;
    for word_idx in start..msg.words.len() {
        let mut word: u64 = 0;
        for j in 0..8 {
            if i < bytes.len() {
                word |= (bytes[i] as u64) << (j * 8);
                i += 1;
            }
        }
        msg.words[word_idx] = word;
    }
}

fn report(cap: CapabilityToken) {
    // runas cache check for root.
    let mut m = IpcMsg::empty();
    m.label = OP_RUNAS;
    m.words[0] = 0; // caller uid
    m.words[1] = 0; // target uid
    let r = ipc_call(cap, m);
    if r.label == REPLY_OK {
        let outcome = if r.words[0] == 0 { "cache-used" } else { "prompted" };
        println!("runas uid=0 -> {}", outcome);
    } else {
        println!("runas: ERROR");
    }

    // Access check for "/".
    let mut m2 = IpcMsg::empty();
    m2.label = OP_CHECK;
    m2.words[0] = 0; // uid
    m2.words[1] = 0; // gid
    pack_str(&mut m2, 2, "/");
    let r2 = ipc_call(cap, m2);
    if r2.label == REPLY_OK {
        let decision = if r2.words[0] != 0 { "allow" } else { "deny" };
        println!("access uid=0 path=/ read -> {}", decision);
    } else {
        println!("access: ERROR");
    }

    // Execute check for "/bin/calc": confirms the elevated session carries the
    // /bin/* execution capability granted by the broker policy.
    let mut m3 = IpcMsg::empty();
    m3.label = OP_CHECK_EXEC;
    m3.words[0] = 0; // uid
    m3.words[1] = 0; // gid
    pack_str(&mut m3, 2, "/bin/calc");
    let r3 = ipc_call(cap, m3);
    if r3.label == REPLY_OK {
        let decision = if r3.words[0] != 0 { "allow" } else { "deny" };
        println!("access uid=0 path=/bin/calc exec -> {}", decision);
    } else {
        println!("access: ERROR");
    }
}

#[no_mangle]
fn _start() -> ! {
    let Some(cap) = nameserver_lookup("uac") else {
        println!("capabilityctl: uac service not found (is it running?)");
        sunlight_libc::exit(1);
    };

    // passwd-style prompt. `read_line` is EOF/again-safe, so an unattended or
    // non-foreground invocation returns an empty line instead of hanging.
    stdout_write("Password: ");
    let mut pw = [0u8; 64];
    let n = read_line(&mut pw);
    if n == 0 {
        println!("(no password entered; continuing read-only checks)");
    }

    report(cap);
    sunlight_libc::exit(0);
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("capabilityctl: PANIC: {}", info);
    sunlight_libc::exit(1);
}
