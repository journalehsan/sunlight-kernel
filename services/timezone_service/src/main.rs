#![no_std]
#![no_main]

extern crate alloc;

use core::sync::atomic::{AtomicUsize, Ordering};

struct BumpAllocator;

unsafe impl core::alloc::GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        static mut HEAP: [u8; 256 * 1024] = [0; 256 * 1024];
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
    debug_log, endpoint_create, ipc_call_timeout, ipc_recv, ipc_reply_and_wait, nameserver_lookup,
    nameserver_register, IpcMsg, TzMsg,
};
use sunlight_tz::{
    all_zones, local_now, ntp_region_from_zone_id, read_localtime, tz_by_id, tz_count,
    write_localtime, LocalTimeCfg, NtpRegion, TzEntry,
};

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

/// Service-local active config (loaded at start, updated on SET_ZONE)
static mut ACTIVE_CFG: LocalTimeCfg = LocalTimeCfg {
    id: [0; 64],
    id_len: 0,
    display_name: [0; 128],
    display_name_len: 0,
    utc_offset_hours: 0,
    utc_offset_minutes: 0,
    dst_offset_minutes: 0,
    dst_start_month: 0,
    dst_end_month: 0,
};

static NEXT_DIAGNOSTIC_CHECKPOINT: AtomicUsize = AtomicUsize::new(0);
const DIAGNOSTIC_CHECKPOINT_SECS: [u64; 5] = [60, 3_600, 21_600, 86_400, 476_220];

#[no_mangle]
pub extern "C" fn _start() -> ! {
    debug_log("[TZ] Starting timezone_service");

    // Load active timezone from /etc/localtime (falls back to UTC inside)
    // SAFETY: single-threaded init before main loop.
    // SAFETY: init before any concurrent access; single threaded.
    unsafe {
        ACTIVE_CFG = read_localtime();
    }
    // One-shot boot diagnostics. The kernel has already logged the raw RTC
    // snapshot and decoded UTC; this records the sole UTC -> local boundary.
    let cfg = unsafe { ACTIVE_CFG };
    let utc = sunlight_ipc::get_time_utc();
    if utc == u64::MAX {
        debug_log("timezone: kernel wall UTC unavailable; local civil time unavailable");
    } else {
        let entry = TzEntry {
            id: "active",
            region: "",
            city: "",
            display_name: "active",
            utc_offset_hours: cfg.utc_offset_hours,
            utc_offset_minutes: cfg.utc_offset_minutes,
            dst_offset_minutes: cfg.dst_offset_minutes,
            dst_start_month: cfg.dst_start_month,
            dst_end_month: cfg.dst_end_month,
        };
        let local = local_now(utc, &entry);
        let weekday = sunlight_tz::weekday_iso(local.year as i32, local.month, local.day);
        debug_log(&alloc::format!(
            "timezone: zone={} offset_seconds={} dst={} application_point=timezone_service",
            cfg.id_str(),
            local.utc_offset_secs,
            local.is_dst
        ));
        debug_log(&alloc::format!(
            "local: {:04}-{:02}-{:02}T{:02}:{:02}:{:02} weekday={} utc_epoch={}",
            local.year,
            local.month,
            local.day,
            local.hour,
            local.minute,
            local.second,
            weekday_name(weekday),
            utc
        ));
    }

    // Ensure CSV is loaded (lazy init inside tz)
    let _zone_count = tz_count();
    // Simple log of count (no format! in no_std here; use a tiny emitter if needed)
    debug_log("[TZ] Zone database loaded");

    // Register with nameserver as "tz"
    let ep = endpoint_create();
    nameserver_register("tz", ep);
    debug_log("[TZ] Registered as 'tz'");

    let mut msg = ipc_recv(ep);
    loop {
        let reply = handle(&mut msg);
        msg = ipc_reply_and_wait(ep, reply);
    }
}

fn handle(msg: &IpcMsg) -> IpcMsg {
    match msg.label {
        TzMsg::GET_LOCAL_TIME => {
            // Compute local time using active cfg + current UTC
            let utc = sunlight_ipc::get_time_utc(); // kernel UTC via syscall wrapper in ipc
            if utc == u64::MAX {
                return IpcMsg::with_label(TzMsg::ERROR).word(0, 2);
            }
            // SAFETY: single-threaded service; ACTIVE_CFG only mutated in SET_ZONE handler before reply loop continues.
            let cfg = unsafe { &ACTIVE_CFG };
            // Build a TzEntry-like from cfg for math (csv may have richer but offset same)
            let entry = TzEntry {
                id: "active",
                region: "",
                city: "",
                display_name: "active",
                utc_offset_hours: cfg.utc_offset_hours,
                utc_offset_minutes: cfg.utc_offset_minutes,
                dst_offset_minutes: cfg.dst_offset_minutes,
                dst_start_month: cfg.dst_start_month,
                dst_end_month: cfg.dst_end_month,
            };
            let ldt = local_now(utc, &entry);
            let weekday = sunlight_tz::weekday_iso(ldt.year as i32, ldt.month, ldt.day);
            diagnostic_checkpoint(utc, &ldt, weekday, cfg.id_str());

            let words = sunlight_tz::encode_local_time(&ldt, weekday);
            IpcMsg::with_label(TzMsg::REPLY)
                .word(0, words[0])
                .word(1, words[1])
                .word(2, words[2])
                .word(3, words[3])
        }

        TzMsg::GET_ZONE => {
            // SAFETY: single-threaded service; ACTIVE_CFG only mutated in SET_ZONE handler before reply loop continues.
            let cfg = unsafe { &ACTIVE_CFG };
            // word(0): h | m<<8 | dst_m<<16
            let w0 = (cfg.utc_offset_hours as i64 as u64 & 0xff)
                | ((cfg.utc_offset_minutes as u64) << 8)
                | ((cfg.dst_offset_minutes as u64) << 16);
            let w1 = (cfg.dst_start_month as u64) | ((cfg.dst_end_month as u64) << 8);

            let mut reply = IpcMsg::with_label(TzMsg::REPLY).word(0, w0).word(1, w1);

            // Pack id into words starting at 2 (up to 32 bytes)
            let id_str = cfg.id_str();
            reply = pack_str_words(reply, 2, id_str);
            reply
        }

        TzMsg::SET_ZONE => {
            // id from msg words (packed bytes, first 64 bytes worth in words[0..])
            let mut idbuf = [0u8; 64];
            unpack_id_from_words(msg, &mut idbuf);
            let id = core::str::from_utf8(&idbuf)
                .unwrap_or("")
                .trim_end_matches('\0');
            if id.is_empty() {
                return IpcMsg::with_label(TzMsg::ERROR).word(0, 1);
            }
            match tz_by_id(id) {
                Some(entry) => {
                    let new_cfg = LocalTimeCfg {
                        id: {
                            let mut b = [0u8; 64];
                            let ib = entry.id.as_bytes();
                            let l = ib.len().min(63);
                            b[..l].copy_from_slice(&ib[..l]);
                            b[l] = 0;
                            b
                        },
                        id_len: entry.id.len().min(63),
                        display_name: {
                            let mut b = [0u8; 128];
                            let db = entry.display_name.as_bytes();
                            let l = db.len().min(127);
                            b[..l].copy_from_slice(&db[..l]);
                            b[l] = 0;
                            b
                        },
                        display_name_len: entry.display_name.len().min(127),
                        utc_offset_hours: entry.utc_offset_hours,
                        utc_offset_minutes: entry.utc_offset_minutes,
                        dst_offset_minutes: entry.dst_offset_minutes,
                        dst_start_month: entry.dst_start_month,
                        dst_end_month: entry.dst_end_month,
                    };
                    if write_localtime(&new_cfg).is_err() {
                        return IpcMsg::with_label(TzMsg::ERROR).word(0, 3);
                    }
                    // update active
                    // SAFETY: single-threaded; mutation visible to subsequent GETs.
                    unsafe {
                        ACTIVE_CFG = new_cfg;
                    }
                    // best-effort notify to timed (do not block on failure)
                    if let Some(timed_cap) = nameserver_lookup("timed") {
                        let region = ntp_region_from_zone_id(entry.id);
                        let notification = pack_str_words(
                            IpcMsg::with_label(TzMsg::NOTIFY_CHANGED).word(0, region as u64),
                            1,
                            entry.id,
                        );
                        let _ = ipc_call_timeout(timed_cap, notification, 100);
                    }
                    IpcMsg::with_label(TzMsg::REPLY).word(0, 0)
                }
                None => IpcMsg::with_label(TzMsg::ERROR).word(0, 1),
            }
        }

        TzMsg::LIST_ZONES => {
            // page ignored for simplicity; use word(0) as 0-based index request
            let req_idx = msg.words[0] as usize;
            let zones = all_zones();
            if req_idx >= zones.len() {
                // end signal
                return IpcMsg::with_label(TzMsg::REPLY).word(0, 0xFFFF_FFFFu64);
            }
            let e = &zones[req_idx];
            let total = zones.len() as u64;

            let w0 = (req_idx as u64) | (total << 32);

            // offsets
            let w1 = (e.utc_offset_hours as i64 as u64 & 0xff)
                | ((e.utc_offset_minutes as u64) << 8)
                | ((e.dst_offset_minutes as u64) << 16);

            let mut reply = IpcMsg::with_label(TzMsg::REPLY).word(0, w0).word(1, w1);

            // id into words[2..] ~32 bytes
            reply = pack_str_words(reply, 2, e.id);

            // display truncated into words[6..]
            reply = pack_str_words(reply, 6, e.display_name);

            reply
        }

        TzMsg::GET_NTP_REGION => {
            // Static continent → NTP pool region. Does not require synchronized wall time.
            let cfg = unsafe { &ACTIVE_CFG };
            let id = cfg.id_str();
            let region = if let Some(entry) = tz_by_id(id) {
                ntp_region_from_zone_id(entry.id)
            } else if !id.is_empty() {
                ntp_region_from_zone_id(id)
            } else {
                NtpRegion::Global
            };
            let mut reply = IpcMsg::with_label(TzMsg::REPLY)
                .word(0, region as u64)
                .word(1, pack_ascii8(region.as_str()));
            reply = pack_str_words(reply, 2, if id.is_empty() { "UTC" } else { id });
            reply
        }

        TzMsg::REPLY | TzMsg::ERROR => {
            // not for server
            IpcMsg::with_label(TzMsg::ERROR)
        }

        _ => IpcMsg::with_label(TzMsg::ERROR),
    }
}

fn diagnostic_checkpoint(utc: u64, local: &sunlight_tz::LocalDateTime, weekday: u8, zone_id: &str) {
    let monotonic_ms = sunlight_ipc::monotonic_millis();
    loop {
        let checkpoint_index = NEXT_DIAGNOSTIC_CHECKPOINT.load(Ordering::Relaxed);
        let Some(&checkpoint_secs) = DIAGNOSTIC_CHECKPOINT_SECS.get(checkpoint_index) else {
            return;
        };
        if monotonic_ms / 1_000 < checkpoint_secs {
            return;
        }
        if NEXT_DIAGNOSTIC_CHECKPOINT
            .compare_exchange(
                checkpoint_index,
                checkpoint_index + 1,
                Ordering::Relaxed,
                Ordering::Relaxed,
            )
            .is_err()
        {
            continue;
        }
        debug_log(&alloc::format!(
            "[TIME-CHECKPOINT] layer=timezone_service elapsed_target_secs={} monotonic_ms={} realtime_utc_unix={} zone={} offset_seconds={} local={:04}-{:02}-{:02}T{:02}:{:02}:{:02} weekday={}",
            checkpoint_secs,
            monotonic_ms,
            utc,
            zone_id,
            local.utc_offset_secs,
            local.year,
            local.month,
            local.day,
            local.hour,
            local.minute,
            local.second,
            weekday_name(weekday)
        ));
    }
}

fn pack_ascii8(s: &str) -> u64 {
    let b = s.as_bytes();
    let mut w = 0u64;
    for i in 0..8.min(b.len()) {
        w |= (b[i] as u64) << (i * 8);
    }
    w
}

fn weekday_name(weekday_iso: u8) -> &'static str {
    match weekday_iso {
        1 => "Monday",
        2 => "Tuesday",
        3 => "Wednesday",
        4 => "Thursday",
        5 => "Friday",
        6 => "Saturday",
        _ => "Sunday",
    }
}

/// Pack bytes (id or short display) into successive words of a fresh IpcMsg starting at base.
/// Returns updated msg. Max ~32 bytes.
fn pack_str_words(mut msg: IpcMsg, base: usize, s: &str) -> IpcMsg {
    let bytes = s.as_bytes();
    let mut wi = base;
    let mut bi = 0usize;
    let mut w = 0u64;
    for &b in bytes.iter().take(32) {
        w |= (b as u64) << (bi * 8);
        bi += 1;
        if bi == 8 {
            if wi < sunlight_ipc::IPC_MAX_WORDS {
                msg = msg.word(wi, w);
            }
            w = 0;
            bi = 0;
            wi += 1;
        }
    }
    if bi > 0 && wi < sunlight_ipc::IPC_MAX_WORDS {
        msg = msg.word(wi, w);
    }
    msg
}

/// Unpack first N bytes of id from incoming msg words (words 0.. used for SET_ZONE id).
fn unpack_id_from_words(msg: &IpcMsg, dst: &mut [u8; 64]) {
    let mut i = 0usize;
    for wi in 0..sunlight_ipc::IPC_MAX_WORDS {
        if i >= 64 {
            break;
        }
        let w = msg.words[wi];
        for b in 0..8 {
            if i >= 64 {
                break;
            }
            dst[i] = ((w >> (b * 8)) & 0xff) as u8;
            i += 1;
        }
    }
}
