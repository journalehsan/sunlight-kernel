//! runas — standalone Ring 3 privilege tool (sudo-style).
//!
//! `runas` is an ordinary user-space binary (`/bin/runas`), not part of the
//! kernel or syscall layer. It runs a command with an elevated (root) session:
//!
//!   1. Parse argv: `runas <command> [args...]`.
//!   2. Look up the `"uac"` broker and prompt for a password (passwd-style).
//!   3. Authenticate the elevation with the broker (`OP_RUNAS`) and authorize
//!      execution of the target binary against the capability policy
//!      (`OP_CHECK_EXEC`). The broker is the single source of truth.
//!   4. On success, `spawn` the resolved binary as a foreground child (kernel
//!      wires its fd0/fd1 to this tab's TTY rings, exactly like the shell's
//!      `run_external`) and `waitpid` for it — so the command's output renders
//!      live. Image-replacing `exec` is intentionally avoided: it bypasses the
//!      TTY foreground routing and that path is unused elsewhere in the OS.
//!
//! Note: the broker's session model does not yet verify the password bytes
//! themselves (see `sunlight_uac::session`); the prompt drives the elevated
//! session cache. The execute authorization is real and capability-based.

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

use sunlight_ipc::{ipc_call, nameserver_lookup, IpcMsg};

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

/// Elevation request opcode (mirrors `uac_service`).
const OP_RUNAS: u64 = 1;
/// Path-execute authorization opcode (mirrors `uac_service`).
const OP_CHECK_EXEC: u64 = 5;
/// Reply label for a handled request.
const REPLY_OK: u64 = 1;

/// Maximum argv entries we forward to the executed command.
const MAX_ARGS: usize = 16;

/// Borrow argv strings out of the exec-time stack arena.
/// SAFETY: argc/argv come from the kernel's SysV stack marshalling.
unsafe fn collect_args<'a>(argc: u64, argv: *const *const u8, out: &mut [&'a str]) -> usize {
    if argv.is_null() {
        return 0;
    }
    let mut count = 0;
    for i in 0..(argc as usize).min(out.len()) {
        let ptr = *argv.add(i);
        if ptr.is_null() {
            break;
        }
        let mut len = 0;
        while len < 256 && *ptr.add(len) != 0 {
            len += 1;
        }
        let slice = core::slice::from_raw_parts(ptr, len);
        out[count] = core::str::from_utf8(slice).unwrap_or("");
        count += 1;
    }
    count
}

/// Resolve a command name to an absolute path. Bare names are looked for in
/// `/bin`; anything already absolute is used as-is.
fn resolve_path(cmd: &str) -> heapless::String<128> {
    let mut p = heapless::String::new();
    if cmd.starts_with('/') {
        let _ = p.push_str(cmd);
    } else {
        let _ = p.push_str("/bin/");
        let _ = p.push_str(cmd);
    }
    p
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

/// Read one line of input. EOF/again-safe so it can never hang: `Ok(0)` stops,
/// `Err(Again)` yields and retries, the newline terminates and is dropped.
fn read_line(buf: &mut [u8]) -> usize {
    let mut len = 0;
    while len < buf.len() {
        let mut byte = [0u8; 1];
        match sunlight_libc::read(sunlight_libc::STDIN, &mut byte) {
            Ok(0) => break,
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

#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8) -> ! {
    let mut storage = [""; MAX_ARGS];
    let count = unsafe { collect_args(argc, argv, &mut storage) };

    // argv[0] is "runas"; argv[1..] is the command to run elevated.
    if count < 2 {
        println!("usage: runas <command> [args...]");
        sunlight_libc::exit(2);
    }
    let cmd_args = &storage[1..count]; // [command, arg1, arg2, ...]
    let command = cmd_args[0];
    let path = resolve_path(command);

    let Some(uac) = nameserver_lookup("uac") else {
        println!("runas: uac broker not found (is it running?)");
        sunlight_libc::exit(1);
    };

    // passwd-style prompt (read_line cannot hang).
    stdout_write("[runas] Password: ");
    let mut pw = [0u8; 64];
    let _ = read_line(&mut pw);

    // Authenticate the elevation to root (uid 0).
    let mut auth = IpcMsg::empty();
    auth.label = OP_RUNAS;
    auth.words[0] = 0; // caller uid
    auth.words[1] = 0; // target uid (root)
    if ipc_call(uac, auth).label != REPLY_OK {
        println!("runas: authentication failed");
        sunlight_libc::exit(1);
    }

    // Authorize execution of the target against the capability policy.
    let mut chk = IpcMsg::empty();
    chk.label = OP_CHECK_EXEC;
    chk.words[0] = 0; // uid (root)
    chk.words[1] = 0; // gid
    pack_str(&mut chk, 2, path.as_str());
    let chk_reply = ipc_call(uac, chk);
    if chk_reply.label != REPLY_OK || chk_reply.words[0] == 0 {
        println!("runas: access denied: {} not executable for root", path.as_str());
        sunlight_libc::exit(13); // EACCES
    }

    // Launch the target as a foreground child and wait for it, mirroring the
    // shell's `run_external`. `spawn(.., None)` makes the kernel wire the
    // child's fd0/fd1 to this tab's TTY rings, so its output renders live —
    // image-replacing `exec` is not used (it bypasses that TTY routing).
    let mut argv_bytes: [&[u8]; MAX_ARGS] = [b""; MAX_ARGS];
    for (i, a) in cmd_args.iter().enumerate() {
        argv_bytes[i] = a.as_bytes();
    }
    match sunlight_libc::spawn(path.as_bytes(), &argv_bytes[..cmd_args.len()], None) {
        Ok(pid) => {
            let code = sunlight_libc::waitpid(pid).unwrap_or(1);
            sunlight_libc::exit(code);
        }
        Err(_) => {
            // No such binary. Builtins (calc, sysfetch, cd, …) live inside the
            // shell, not /bin, so there is nothing to spawn — same as `sudo cd`.
            println!("runas: {}: command not found (not an executable)", command);
            sunlight_libc::exit(127);
        }
    }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("runas: PANIC: {}", info);
    sunlight_libc::exit(1);
}
