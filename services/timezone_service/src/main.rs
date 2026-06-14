#![no_std]
#![no_main]

extern crate alloc;

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
    debug_log, endpoint_create, ipc_recv, ipc_reply_and_wait, nameserver_lookup,
    nameserver_register, IpcMsg, TzMsg,
};
use sunlight_tz::{LocalTimeCfg, TzEntry, read_localtime, write_localtime, tz_by_id, all_zones, local_now, tz_count};

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

/// Service-local active config (loaded at start, updated on SET_ZONE)
static mut ACTIVE_CFG: LocalTimeCfg = LocalTimeCfg {
    id: [0;64], id_len: 0,
    display_name: [0;128], display_name_len: 0,
    utc_offset_hours:0, utc_offset_minutes:0, dst_offset_minutes:0,
    dst_start_month:0, dst_end_month:0,
};

#[no_mangle]
pub extern "C" fn _start() -> ! {
    debug_log("[TZ] Starting timezone_service");

    // Load active timezone from /etc/localtime (falls back to UTC inside)
    // SAFETY: single-threaded init before main loop.
    // SAFETY: init before any concurrent access; single threaded.
    unsafe {
        ACTIVE_CFG = read_localtime();
    }
    let mut idbuf = [0u8; 64];
    let id = unsafe { ACTIVE_CFG.id_str() };
    // copy for logging
    let idb = id.as_bytes();
    let idl = idb.len().min(63);
    idbuf[..idl].copy_from_slice(&idb[..idl]);
    idbuf[idl] = 0;
    // Log active
    debug_log("[TZ] Active timezone: UTC"); // conservative; real name via later GET

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

            // Pack per spec into 80-byte msg:
            // word(0): year(u16)<<48 | m<<40 | d<<32 | h<<24 | min<<16 | s<<8
            let mut w0: u64 = (ldt.year as u64) << 48;
            w0 |= (ldt.month as u64) << 40;
            w0 |= (ldt.day as u64) << 32;
            w0 |= (ldt.hour as u64) << 24;
            w0 |= (ldt.minute as u64) << 16;
            w0 |= (ldt.second as u64) << 8;

            let mut reply = IpcMsg::with_label(TzMsg::REPLY)
                .word(0, w0)
                .word(1, ldt.utc_offset_secs as u64)
                .word(2, ldt.is_dst as u64);

            // abbr into words[3] low bytes (8 bytes total)
            let mut ab = 0u64;
            for i in 0..8 {
                ab |= (ldt.abbr[i] as u64) << (i * 8);
            }
            reply = reply.word(3, ab);
            reply
        }

        TzMsg::GET_ZONE => {
            // SAFETY: single-threaded service; ACTIVE_CFG only mutated in SET_ZONE handler before reply loop continues.
            let cfg = unsafe { &ACTIVE_CFG };
            // word(0): h | m<<8 | dst_m<<16
            let w0 = (cfg.utc_offset_hours as i64 as u64 & 0xff)
                   | ((cfg.utc_offset_minutes as u64) << 8)
                   | ((cfg.dst_offset_minutes as u64) << 16);
            let w1 = (cfg.dst_start_month as u64) | ((cfg.dst_end_month as u64) << 8);

            let mut reply = IpcMsg::with_label(TzMsg::REPLY)
                .word(0, w0)
                .word(1, w1);

            // Pack id into words starting at 2 (up to 32 bytes)
            let id_str = cfg.id_str();
            reply = pack_str_words(reply, 2, id_str);
            reply
        }

        TzMsg::SET_ZONE => {
            // id from msg words (packed bytes, first 64 bytes worth in words[0..])
            let mut idbuf = [0u8; 64];
            unpack_id_from_words(msg, &mut idbuf);
            let id = core::str::from_utf8(&idbuf).unwrap_or("").trim_end_matches('\0');
            if id.is_empty() {
                return IpcMsg::with_label(TzMsg::ERROR).word(0, 1);
            }
            match tz_by_id(id) {
                Some(entry) => {
                    let new_cfg = LocalTimeCfg {
                        id: {
                            let mut b = [0u8;64];
                            let ib = entry.id.as_bytes();
                            let l = ib.len().min(63); b[..l].copy_from_slice(&ib[..l]); b[l]=0; b
                        },
                        id_len: entry.id.len().min(63),
                        display_name: {
                            let mut b=[0u8;128];
                            let db = entry.display_name.as_bytes();
                            let l = db.len().min(127); b[..l].copy_from_slice(&db[..l]); b[l]=0; b
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
                    unsafe { ACTIVE_CFG = new_cfg; }
                    // best-effort notify to timed (do not block on failure)
                    if let Some(timed_cap) = nameserver_lookup("timed") {
                        let _ = sunlight_ipc::ipc_call(timed_cap, IpcMsg::with_label(TzMsg::NOTIFY_CHANGED));
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

            let mut reply = IpcMsg::with_label(TzMsg::REPLY)
                .word(0, w0)
                .word(1, w1);

            // id into words[2..] ~32 bytes
            reply = pack_str_words(reply, 2, e.id);

            // display truncated into words[6..]
            reply = pack_str_words(reply, 6, e.display_name);

            reply
        }

        TzMsg::REPLY | TzMsg::ERROR => {
            // not for server
            IpcMsg::with_label(TzMsg::ERROR)
        }

        _ => IpcMsg::with_label(TzMsg::ERROR),
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
            w = 0; bi = 0; wi += 1;
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
        if i >= 64 { break; }
        let w = msg.words[wi];
        for b in 0..8 {
            if i >= 64 { break; }
            dst[i] = ((w >> (b*8)) & 0xff) as u8;
            i += 1;
        }
    }
}


