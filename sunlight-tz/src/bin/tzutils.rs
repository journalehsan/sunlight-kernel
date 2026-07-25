//! tzutils — time-sync control client for SunlightOS.
//!
//! Requests NTP synchronization from the Time Service (`timed`). Does not
//! implement NTP or set the wall clock directly.
//!
//! ```text
//! tzutils --sync
//! tzutils --sync --force
//! tzutils -s
//! tzutils -sf
//! tzutils --status
//! ```

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

use sunlight_ipc::{
    ipc_call_timeout, nameserver_lookup_timeout, IpcMsg, NtpSyncState, ProcessExit, TimeMsg,
};
use sunlight_tz::cli_flags::{parse_tzutils_args, TzutilsArgs};

const MAX_ARGS: usize = 16;
const SYNC_TIMEOUT_MS: u64 = 35_000;

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
        let mut buf = heapless::String::<512>::new();
        let _ = write!(&mut buf, $($arg)*);
        stdout_write(&buf);
        stdout_write("\n");
    }};
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    println!("tzutils: PANIC");
    ProcessExit::exit(101);
}

#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8) -> ! {
    let mut storage = [""; MAX_ARGS];
    let count = unsafe { collect_args(argc, argv, &mut storage) };
    let cli = parse_tzutils_args(&storage[..count]);
    let code = run(cli);
    ProcessExit::exit(code);
}

fn run(args: TzutilsArgs) -> i32 {
    if args.help {
        print_usage();
        return 0;
    }

    let Some(timed) = nameserver_lookup_timeout("timed", 1_000) else {
        println!("tzutils: timed service not available");
        return 1;
    };

    let mut exit = 0i32;

    if args.sync {
        let mut flags = 0u64;
        if args.force {
            flags |= TimeMsg::SYNC_FLAG_FORCE;
        }
        let req = IpcMsg::with_label(TimeMsg::SYNC_NTP).word(0, flags);
        match ipc_call_timeout(timed, req, SYNC_TIMEOUT_MS) {
            Ok(reply) if reply.label == TimeMsg::REPLY => {
                println!("tzutils: synchronization succeeded");
            }
            Ok(reply) if reply.label == TimeMsg::ERROR => {
                println!(
                    "tzutils: sync failed: {}",
                    err_name(reply.words[0])
                );
                exit = 1;
            }
            Ok(_) => {
                println!("tzutils: unexpected reply from timed");
                exit = 1;
            }
            Err(_) => {
                println!("tzutils: sync timed out waiting for timed");
                exit = 1;
            }
        }
    }

    if args.status || args.sync {
        let req = IpcMsg::with_label(TimeMsg::GET_SYNC_STATUS);
        match ipc_call_timeout(timed, req, 2_000) {
            Ok(reply) if reply.label == TimeMsg::REPLY => print_status(&reply),
            _ => {
                if !args.sync {
                    println!("tzutils: failed to query sync status");
                    exit = 1;
                }
            }
        }
    }

    exit
}

fn print_status(r: &IpcMsg) {
    let state = r.words[0] & 0xff;
    let stratum = (r.words[0] >> 8) & 0xff;
    let region = (r.words[0] >> 16) & 0xff;
    let server_count = (r.words[0] >> 24) & 0xff;
    let flags = (r.words[0] >> 32) & 0xff;
    let ntp_synced = (flags & 1) != 0;
    let rtc_updated = (flags & 2) != 0;
    let explicit = (flags & 4) != 0;
    let offset_ms = r.words[1] as i64;
    let delay_ms = r.words[2] & 0xffff_ffff_ffff;
    let last_error = r.words[2] >> 48;
    let last_sync = r.words[3];
    let next_attempt = r.words[4];
    let backoff = r.words[5];
    let last_server = unpack_cstr(r.words[6]);
    let zone = unpack_cstr(r.words[7]);

    println!("Synchronization state: {}", state_name(state));
    println!("NTP region: {}", region_name(region as u8));
    println!("Timezone: {}", zone);
    println!(
        "Configured servers: {} ({})",
        server_count,
        if explicit {
            "explicit"
        } else {
            "regional pool"
        }
    );
    println!("Last successful server: {}", last_server);
    println!("Stratum: {}", stratum);
    println!("Last offset (ms): {}", offset_ms);
    println!("Last delay (ms): {}", delay_ms);
    println!("Last successful sync (UTC unix): {}", last_sync);
    println!("Last error: {}", err_name(last_error));
    println!("Next attempt (mono ms): {}", next_attempt);
    println!("Current backoff (ms): {}", backoff);
    println!("NTP synchronized: {}", if ntp_synced { "yes" } else { "no" });
    println!(
        "RTC updated: {}",
        if rtc_updated {
            "yes"
        } else {
            "no (UTC write API not available)"
        }
    );
    let _ = NtpSyncState::SYNCHRONIZED;
}

fn unpack_cstr(w: u64) -> heapless::String<16> {
    let mut s = heapless::String::<16>::new();
    for i in 0..8 {
        let ch = ((w >> (i * 8)) & 0xff) as u8;
        if ch == 0 {
            break;
        }
        let _ = s.push(ch as char);
    }
    s
}

fn state_name(s: u64) -> &'static str {
    match s {
        NtpSyncState::UNSYNCHRONIZED => "unsynchronized",
        NtpSyncState::WAITING_NETWORK => "waiting-for-network",
        NtpSyncState::RESOLVING => "resolving",
        NtpSyncState::SAMPLING => "sampling",
        NtpSyncState::SYNCHRONIZED => "synchronized",
        NtpSyncState::DEGRADED => "degraded",
        NtpSyncState::FAILED => "failed",
        _ => "unknown",
    }
}

fn region_name(r: u8) -> &'static str {
    match r {
        1 => "africa",
        2 => "asia",
        3 => "europe",
        4 => "north-america",
        5 => "south-america",
        6 => "oceania",
        _ => "global",
    }
}

fn err_name(e: u64) -> &'static str {
    match e {
        0 => "none",
        TimeMsg::ERR_FAILED => "failed",
        TimeMsg::ERR_NETWORK => "network",
        TimeMsg::ERR_DNS => "dns",
        TimeMsg::ERR_TIMEOUT => "timeout",
        TimeMsg::ERR_VALIDATION => "validation",
        TimeMsg::ERR_PERMISSION => "permission",
        TimeMsg::ERR_CLOCK_UPDATE => "clock-update",
        TimeMsg::ERR_BUSY => "busy",
        _ => "unknown",
    }
}

fn print_usage() {
    println!("Usage: tzutils [--sync|-s] [--force|-f] [--status|-S]");
    println!("  -s, --sync     request immediate NTP synchronization via timed");
    println!("  -f, --force    with --sync: bypass backoff, allow forced step");
    println!("  -sf            equivalent to --sync --force");
    println!("  -S, --status   print synchronization status");
    println!("tzutils does not set the clock itself; timed owns UTC sync.");
    println!("Pool NTP is unauthenticated (NTS is future work).");
}

unsafe fn collect_args(argc: u64, argv: *const *const u8, out: &mut [&str]) -> usize {
    let mut n = 0usize;
    if argv.is_null() {
        return 0;
    }
    let c = argc as usize;
    for i in 0..c.min(out.len()) {
        let p = *argv.add(i);
        if p.is_null() {
            break;
        }
        let mut len = 0;
        while *p.add(len) != 0 {
            len += 1;
        }
        let s = core::str::from_utf8_unchecked(core::slice::from_raw_parts(p, len));
        out[n] = s;
        n += 1;
    }
    n
}

// Parser unit tests live in sunlight-tz lib tests via a free function re-export
// is awkward for no_main bins; host tests cover ntp_region + timed protocol.
