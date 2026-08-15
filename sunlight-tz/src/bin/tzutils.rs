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

use sunlight_ipc::{NtpSyncState, ProcessExit, TimeMsg};
use sunlight_tz::cli_flags::{parse_tzutils_args, TzutilsArgs};
use sunlight_tz::client::{TimeClient, TimeClientError, TzClient};

const MAX_ARGS: usize = 16;

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
    if args.invalid {
        println!("tzutils: unsupported argument (use tzctl to view or change timezones)");
        print_usage();
        return 2;
    }

    let Ok(client) = TimeClient::connect() else {
        println!("tzutils: timed service not available");
        return 1;
    };

    let mut exit = 0i32;

    if args.sync {
        let mut flags = 0u64;
        if args.force {
            flags |= TimeMsg::SYNC_FLAG_FORCE;
        }
        match client.sync_now_with_flags(flags) {
            Ok(_) => {
                println!("tzutils: synchronization succeeded");
            }
            Err(TimeClientError::Failed(code)) => {
                println!("tzutils: sync failed: {}", err_name(code));
                exit = 1;
            }
            Err(TimeClientError::Timeout) => {
                println!("tzutils: sync timed out waiting for timed");
                exit = 1;
            }
            Err(_) => {
                println!("tzutils: unexpected reply from timed");
                exit = 1;
            }
        }
    }

    if args.status || args.sync {
        match client.sync_status() {
            Ok(st) => print_status(&st),
            Err(_) => {
                if !args.sync {
                    println!("tzutils: failed to query sync status");
                    exit = 1;
                }
            }
        }
    }

    exit
}

fn print_status(st: &sunlight_tz::client::SyncStatusSnapshot) {
    println!("Synchronization state: {}", state_name(st.state));
    println!("NTP region: {}", st.region_label());
    match TzClient::connect().and_then(|client| client.get_zone()) {
        Ok(zone) => println!("Timezone: {}", zone.id_str()),
        Err(_) => println!("Timezone: unavailable"),
    }
    println!(
        "Configured servers: {} ({})",
        st.server_count,
        if st.explicit_servers {
            "explicit"
        } else {
            "regional pool"
        }
    );
    println!("Stratum: {}", st.stratum);
    println!("Last offset (ms): {}", st.last_offset_ms);
    println!("Last delay (ms): {}", st.last_delay_ms);
    println!("Last successful sync (UTC unix): {}", st.last_sync_unix);
    println!("Last error: {}", err_name(st.last_error));
    println!(
        "NTP synchronized: {}",
        if st.ntp_synced { "yes" } else { "no" }
    );
    println!(
        "RTC updated: {}",
        if st.rtc_updated {
            "yes"
        } else {
            "no (UTC write API not available)"
        }
    );
    let _ = NtpSyncState::SYNCHRONIZED;
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

fn err_name(e: u64) -> &'static str {
    sunlight_tz::client::SyncStatusSnapshot::error_label(e)
}

fn print_usage() {
    println!("Usage: tzutils [--sync|-s] [--force|-f] [--status|-S]");
    println!("  -s, --sync     request immediate NTP synchronization via timed");
    println!("  -f, --force    with --sync: bypass backoff, allow forced step");
    println!("  -sf            equivalent to --sync --force");
    println!("  -S, --status   print synchronization status");
    println!("tzutils does not set the clock itself; timed owns UTC sync.");
    println!("Use 'tzctl get', 'tzctl list', or 'tzctl set <IANA-zone>' for timezones.");
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
