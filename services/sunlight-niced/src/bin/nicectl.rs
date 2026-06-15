//! nicectl - tiny CLI for talking to the niced watchdog protocol.
//!
//! argv access: SunlightOS userspace binaries DO receive `argc`/`argv` via
//! `_start(argc: u64, argv: *const *const u8)` (see
//! sunlight-utils/src/main.rs `_start`/`collect_args`), so nicectl
//! implements real argv-based subcommands:
//!
//!   nicectl set <pid> <nice>     -> NicedOp::SET_NICE
//!   nicectl wd <pid> <period>    -> NicedOp::SET_WATCHDOG
//!   nicectl status <pid>         -> NicedOp::QUERY
//!   nicectl list                 -> not supported by the current protocol
//!                                    (no enumerate-all opcode); prints a
//!                                    notice and exits.
//!
//! Per the task spec, a sunshell alias `nc` -> `nicectl` is SKIPPED: no
//! alias mechanism exists in sunshell (checked sunshell/src/main.rs — only
//! a fixed builtin/PATH dispatch table, no user-defined aliases).

#![no_std]
#![no_main]

extern crate alloc;

struct BumpAllocator;

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

#[global_allocator]
static BUMP: BumpAllocator = BumpAllocator;

use sunlight_ipc::{debug_log, ipc_call, nameserver_lookup, IpcMsg, ProcessExit};

#[macro_export]
macro_rules! serial_println {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let mut buf = heapless::String::<256>::new();
        let _ = write!(&mut buf, $($arg)*);
        debug_log(&buf);
    }};
}

// Local copies of the opcodes from sunlight_niced::ipc::NicedOp (avoid
// depending on the niced binary crate's internal modules from this bin).
#[allow(dead_code)]
const PING: u64 = 0x9001;
const SET_NICE: u64 = 0x9010;
const SET_WATCHDOG: u64 = 0x9011;
const QUERY: u64 = 0x9013;
const REPLY: u64 = 0x90FF;

const MAX_ARGS: usize = 8;

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    serial_println!("[nicectl] PANIC: {}", _info);
    ProcessExit::exit(101);
}

#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8) -> ! {
    let mut storage = [""; MAX_ARGS];
    // SAFETY: argc/argv come from the kernel's SysV stack marshalling, same
    // pattern as sunlight-utils's _start.
    let count = unsafe { collect_args(argc, argv, &mut storage) };
    let args = &storage[..count];

    let Some(cap) = nameserver_lookup("niced") else {
        serial_println!("[nicectl] niced not running (nameserver lookup failed)");
        ProcessExit::exit(1);
    };

    if args.len() < 2 {
        serial_println!("[nicectl] usage: nicectl <set|wd|status|list> [pid] [value]");
        ProcessExit::exit(1);
    }

    let cmd = args[1];
    let code = match cmd {
        "set" if args.len() >= 4 => {
            let pid: u64 = parse_u64(args[2]);
            let nice: i64 = parse_i64(args[3]);
            let reply = ipc_call(cap, IpcMsg::with_label(SET_NICE).word(0, pid).word(1, nice as u64));
            print_reply("set", &reply)
        }
        "wd" if args.len() >= 4 => {
            let pid: u64 = parse_u64(args[2]);
            let period: u64 = parse_u64(args[3]);
            let reply = ipc_call(cap, IpcMsg::with_label(SET_WATCHDOG).word(0, pid).word(1, period));
            print_reply("wd", &reply)
        }
        "status" if args.len() >= 3 => {
            let pid: u64 = parse_u64(args[2]);
            let reply = ipc_call(cap, IpcMsg::with_label(QUERY).word(0, pid));
            if reply.label == REPLY {
                serial_println!(
                    "[nicectl] pid={} nice={} not_responding={}",
                    pid,
                    reply.words[0] as i64 as i8,
                    reply.words[2]
                );
                0
            } else {
                serial_println!("[nicectl] status query failed (pid={} not monitored?)", pid);
                1
            }
        }
        "list" => {
            // NicedOp has no enumerate-all opcode today; this would require
            // a new multi-message LIST protocol similar to sunlightd's.
            serial_println!("[nicectl] 'list' not supported: NicedOp has no enumerate-all opcode yet");
            1
        }
        _ => {
            serial_println!("[nicectl] usage: nicectl <set|wd|status|list> [pid] [value]");
            1
        }
    };

    ProcessExit::exit(code);
}

fn print_reply(op: &str, reply: &IpcMsg) -> i32 {
    if reply.label == REPLY {
        serial_println!("[nicectl] {} OK", op);
        0
    } else {
        serial_println!("[nicectl] {} failed", op);
        1
    }
}

fn parse_u64(s: &str) -> u64 {
    s.parse::<u64>().unwrap_or(0)
}

fn parse_i64(s: &str) -> i64 {
    s.parse::<i64>().unwrap_or(0)
}

/// SAFETY: caller ensures argc/argv come from the kernel's exec-time stack
/// arena (same contract as sunlight-utils::collect_args).
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
