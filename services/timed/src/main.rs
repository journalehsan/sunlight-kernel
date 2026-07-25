//! timed — SunlightOS Time Service (authoritative UTC + NTP sync).
//!
//! Owns UTC synchronization status and wall-clock steps. Local civil time is
//! derived only by timezone_service. NTP timestamps are UTC.
//!
//! Unauthenticated pool NTP is not cryptographically trusted; NTS is future work.
//! Kernel provides discrete wall-clock steps only (no slew primitive).

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
    debug_log, endpoint_create, get_time_utc, getuid, ipc_call, ipc_call_timeout, ipc_recv_timeout,
    ipc_reply, monotonic_millis, nameserver_lookup, nameserver_lookup_timeout, nameserver_register,
    set_time_utc, shm_alloc, shm_free, CapabilityToken, IpcMsg, NetOp, NtpSyncState, TimeMsg,
    TzMsg,
};
use sunlight_timed::ntp::{
    allow_normal_step, apply_offset_secs, build_client_request, jitter_ms, next_backoff_ms,
    parse_and_validate, select_sample, unix_to_ntp, NtpError, SyncState, BACKOFF_INITIAL_MS,
    MAX_FORCE_STEP_SECS, NTP_PACKET_LEN, NTP_PORT, PERIODIC_INTERVAL_MS, PERIODIC_JITTER_MS,
    SERVER_TIMEOUT_MS,
};
use sunlight_timed::state::{SampleBuf, SyncStatus, TimeState, HOSTNAME_MAX, MAX_SERVERS};
use sunlight_tz::{
    format_pool_hostname_into, ntp_region_from_zone_id, NtpRegion, NTP_HOSTNAME_MAX,
    NTP_POOL_SERVER_COUNT,
};

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

const CONFIG_PATH: &str = "/etc/sunlight/ntp.conf";

#[no_mangle]
pub extern "C" fn _start() -> ! {
    debug_log("[timed] Time daemon started (UTC + NTP)");

    let mut time_state = TimeState::new();
    let mut sync = SyncStatus::new();

    time_state.utc_epoch = get_time_utc();
    load_explicit_servers(&mut sync);
    refresh_region_from_tz(&mut sync);

    // Schedule first attempt shortly after boot (do not block registration).
    sync.state = SyncState::WaitingNetwork;
    sync.next_attempt_mono_ms = monotonic_millis().saturating_add(2_000);

    let ep = endpoint_create();
    nameserver_register("timed", ep);
    debug_log("[timed] Registered as 'timed'");

    loop {
        let now = monotonic_millis();
        // Cap per-wait so a single long sleep cannot fail arming; we re-check
        // next_attempt after each wake. Also avoids saturating deadline ticks.
        const MAX_WAIT_MS: u64 = 30_000;
        let wait_ms = if sync.next_attempt_mono_ms > now {
            (sync.next_attempt_mono_ms - now).min(MAX_WAIT_MS).max(1)
        } else {
            1
        };

        match ipc_recv_timeout(ep, wait_ms) {
            Some(msg) => {
                let reply = handle_msg(&msg, &mut time_state, &mut sync);
                ipc_reply(reply);
            }
            None => {
                // Only run automatic sync when the scheduled time is due.
                // Spurious early wakes (deadline arm failure, partial wait) must
                // not thrash DNS/UDP and starve interactive IPC (shell keys).
                let now = monotonic_millis();
                if now >= sync.next_attempt_mono_ms {
                    let _ = run_sync_attempt(&mut time_state, &mut sync, false);
                }
            }
        }
    }
}

fn handle_msg(msg: &IpcMsg, time_state: &mut TimeState, sync: &mut SyncStatus) -> IpcMsg {
    time_state.utc_epoch = get_time_utc();
    time_state.ntp_synced = sync.ntp_synced;

    match msg.label {
        TimeMsg::GET_TIME | TimeMsg::GET_UTC => {
            IpcMsg::with_label(TimeMsg::REPLY).word(0, time_state.utc_epoch)
        }
        TimeMsg::GET_STATE => IpcMsg::with_label(TimeMsg::REPLY)
            .word(0, time_state.utc_epoch)
            .word(1, 0u64) // offset always 0 (UTC service)
            .word(2, 0u64) // dst always false
            .word(3, u64::from(sync.ntp_synced)),
        TimeMsg::SET_TIMEZONE => {
            // No-op: timezone is owned by timezone_service.
            IpcMsg::with_label(TimeMsg::REPLY)
        }
        TzMsg::NOTIFY_CHANGED => {
            apply_timezone_notification(msg, sync);
            IpcMsg::with_label(TimeMsg::REPLY)
        }
        TimeMsg::SYNC_NTP => {
            let force = (msg.words[0] & TimeMsg::SYNC_FLAG_FORCE) != 0;
            if force && getuid() != 0 {
                sync.last_error = TimeMsg::ERR_PERMISSION;
                return IpcMsg::with_label(TimeMsg::ERROR).word(0, TimeMsg::ERR_PERMISSION);
            }
            // Manual sync always runs now; --force also clears backoff state
            // and permits the larger-step correction policy.
            sync.next_attempt_mono_ms = 0;
            if force {
                sync.backoff_ms = 0;
            }
            match run_sync_attempt(time_state, sync, force) {
                Ok(()) => IpcMsg::with_label(TimeMsg::REPLY)
                    .word(0, 0)
                    .word(1, sync.last_offset_ms as u64)
                    .word(2, sync.last_delay_ms)
                    .word(3, sync.stratum as u64),
                Err(err) => {
                    let code = err.as_time_err();
                    sync.last_error = code;
                    IpcMsg::with_label(TimeMsg::ERROR).word(0, code)
                }
            }
        }
        TimeMsg::GET_SYNC_STATUS => pack_sync_status(sync, time_state.utc_epoch),
        _ => IpcMsg::with_label(TimeMsg::ERROR).word(0, TimeMsg::ERR_FAILED),
    }
}

fn apply_timezone_notification(msg: &IpcMsg, sync: &mut SyncStatus) {
    if sync.explicit_servers {
        return;
    }
    let mut idbuf = [0u8; 64];
    let mut len = 0usize;
    'words: for wi in 1..sunlight_ipc::IPC_MAX_WORDS {
        for byte in msg.words[wi].to_le_bytes() {
            if byte == 0 || len >= idbuf.len() - 1 {
                break 'words;
            }
            idbuf[len] = byte;
            len += 1;
        }
    }
    if len > 0 {
        if let Ok(id) = core::str::from_utf8(&idbuf[..len]) {
            sync.set_zone(id);
        }
    }
    sync.ntp_region = NtpRegion::from_u8((msg.words[0] & 0xff) as u8) as u8;
    select_regional_servers(sync);
}

fn pack_sync_status(sync: &SyncStatus, utc: u64) -> IpcMsg {
    // Compact status for CLI:
    // w0: state | stratum<<8 | region<<16 | server_count<<24 | flags<<32
    //     flags: ntp_synced bit0, rtc_updated bit1, explicit bit2
    // w1: last_offset_ms as u64 (bitcast i64)
    // w2: last_delay_ms | last_error<<48
    // w3: last_sync_unix
    // w4: next_attempt_mono_ms
    // w5: backoff_ms
    // w6: first server name packed (8 bytes) — full list via zone id in last word
    // w7: zone id first 8 bytes
    let flags = u64::from(sync.ntp_synced)
        | (u64::from(sync.rtc_updated) << 1)
        | (u64::from(sync.explicit_servers) << 2);
    let w0 = sync.state.as_u64()
        | ((sync.stratum as u64) << 8)
        | ((sync.ntp_region as u64) << 16)
        | ((sync.server_count as u64) << 24)
        | (flags << 32);
    let w1 = sync.last_offset_ms as u64;
    let w2 = sync.last_delay_ms | (sync.last_error << 48);
    let mut w6 = 0u64;
    if sync.last_server_len > 0 {
        for i in 0..8.min(sync.last_server_len) {
            w6 |= (sync.last_server[i] as u64) << (i * 8);
        }
    }
    let mut w7 = 0u64;
    for i in 0..8.min(sync.zone_len) {
        w7 |= (sync.zone_id[i] as u64) << (i * 8);
    }
    let _ = utc;
    IpcMsg::with_label(TimeMsg::REPLY)
        .word(0, w0)
        .word(1, w1)
        .word(2, w2)
        .word(3, sync.last_sync_unix)
        .word(4, sync.next_attempt_mono_ms)
        .word(5, sync.backoff_ms)
        .word(6, w6)
        .word(7, w7)
}

fn run_sync_attempt(
    time_state: &mut TimeState,
    sync: &mut SyncStatus,
    force: bool,
) -> Result<(), NtpError> {
    refresh_region_from_tz(sync);

    let net = match nameserver_lookup_timeout("net", 200) {
        Some(c) => c,
        None => {
            sync.state = SyncState::WaitingNetwork;
            schedule_backoff(sync, NtpError::NetworkDown);
            return Err(NtpError::NetworkDown);
        }
    };

    // Network readiness: GETIP non-zero.
    let ip_reply = ipc_call_timeout(net, IpcMsg::with_label(NetOp::GETIP), 500);
    let has_ip = match ip_reply {
        Ok(r) if r.words[0] != 0 => true,
        _ => false,
    };
    if !has_ip {
        sync.state = SyncState::WaitingNetwork;
        schedule_backoff(sync, NtpError::NetworkDown);
        return Err(NtpError::NetworkDown);
    }

    if sync.server_count == 0 {
        select_regional_servers(sync);
    }

    sync.state = SyncState::Resolving;
    let mut samples = SampleBuf::new();
    let mut last_err = NtpError::NoSample;
    let mut used_server: [u8; HOSTNAME_MAX] = [0; HOSTNAME_MAX];
    let mut used_len = 0usize;

    // Bound work: stop once we have a usable sample set. Querying all four
    // pool hosts on every attempt holds net_server in UDP_EXCHANGE/RESOLVE
    // for many seconds and starves interactive IPC (shell keys time out at 200ms).
    for i in 0..sync.server_count {
        // Prefer one good sample; collect a second only when the first shows a
        // large offset that needs agreement under the normal (non-force) policy.
        if samples.len >= 1 {
            let first = samples.as_slice()[0];
            if force || allow_normal_step(first.offset_ms) || samples.len >= 2 {
                break;
            }
        }

        let mut host_buf = [0u8; HOSTNAME_MAX];
        let host_src = sync.server_str(i);
        if host_src.is_empty() {
            continue;
        }
        let hn = host_src.as_bytes().len().min(HOSTNAME_MAX - 1);
        host_buf[..hn].copy_from_slice(&host_src.as_bytes()[..hn]);
        let host = core::str::from_utf8(&host_buf[..hn]).unwrap_or("");

        sync.state = SyncState::Resolving;
        let ip = match resolve_hostname(net, host) {
            Some(ip) => ip,
            None => {
                last_err = NtpError::DnsError;
                continue; // try next server
            }
        };

        sync.state = SyncState::Sampling;
        match query_one_server(net, ip) {
            Ok(sample) => {
                samples.push(sample);
                used_server[..hn].copy_from_slice(&host_buf[..hn]);
                used_len = hn;
            }
            Err(e) => {
                last_err = e;
                // continue other servers
            }
        }
    }

    let sample = match select_sample(samples.as_slice()) {
        Some(s) => s,
        None => {
            schedule_backoff(sync, last_err);
            return Err(last_err);
        }
    };

    let local_unix = get_time_utc();
    if local_unix == u64::MAX {
        schedule_backoff(sync, NtpError::ClockUpdate);
        return Err(NtpError::ClockUpdate);
    }

    if !force && !allow_normal_step(sample.offset_ms) {
        // Large offset without force: require stronger agreement (≥2 samples)
        // or refuse.
        if samples.len < 2 {
            sync.last_error = TimeMsg::ERR_VALIDATION;
            schedule_backoff(sync, NtpError::InvalidResponse);
            return Err(NtpError::InvalidResponse);
        }
        // With multiple samples, check they agree within 500ms.
        let mut ok = true;
        for s in samples.as_slice() {
            if (s.offset_ms - sample.offset_ms).abs() > 500 {
                ok = false;
                break;
            }
        }
        if !ok || sample.offset_ms.abs() > MAX_FORCE_STEP_SECS.saturating_mul(1000) {
            schedule_backoff(sync, NtpError::InvalidResponse);
            return Err(NtpError::InvalidResponse);
        }
    }

    if force && sample.offset_ms.abs() > MAX_FORCE_STEP_SECS.saturating_mul(1000) {
        schedule_backoff(sync, NtpError::InvalidResponse);
        return Err(NtpError::InvalidResponse);
    }

    let new_unix = match apply_offset_secs(local_unix, sample.offset_ms) {
        Some(u) => u,
        None => {
            schedule_backoff(sync, NtpError::ClockUpdate);
            return Err(NtpError::ClockUpdate);
        }
    };

    // Discrete step — no slew API available. Documented limitation.
    if set_time_utc(new_unix) != 0 {
        schedule_backoff(sync, NtpError::ClockUpdate);
        return Err(NtpError::ClockUpdate);
    }

    time_state.utc_epoch = get_time_utc();
    time_state.ntp_synced = true;
    // RTC write: not available (no safe CMOS write API). Remain UTC-only policy.
    sync.rtc_updated = false;

    let server_name = core::str::from_utf8(&used_server[..used_len]).unwrap_or("ntp");
    sync.record_success(
        server_name,
        sample.stratum,
        sample.offset_ms,
        sample.delay_ms,
        time_state.utc_epoch,
    );

    // Schedule next periodic attempt with jitter.
    let rand = get_entropy_word();
    let j = jitter_ms(rand, PERIODIC_JITTER_MS);
    sync.backoff_ms = 0;
    sync.next_attempt_mono_ms = monotonic_millis().saturating_add(PERIODIC_INTERVAL_MS + j);

    debug_log("[timed] NTP sync OK (UTC step applied; monotonic unchanged; no RTC write)");
    let _ = NtpSyncState::SYNCHRONIZED;
    Ok(())
}

fn schedule_backoff(sync: &mut SyncStatus, err: NtpError) {
    sync.record_error(err);
    sync.backoff_ms = next_backoff_ms(sync.backoff_ms);
    if sync.backoff_ms == 0 {
        sync.backoff_ms = BACKOFF_INITIAL_MS;
    }
    let rand = get_entropy_word();
    let j = jitter_ms(rand, sync.backoff_ms / 4);
    sync.next_attempt_mono_ms = monotonic_millis().saturating_add(sync.backoff_ms + j);
}

fn query_one_server(
    net: CapabilityToken,
    ip: [u8; 4],
) -> Result<sunlight_timed::NtpSample, NtpError> {
    let local_unix = get_time_utc();
    if local_unix == 0 || local_unix == u64::MAX {
        return Err(NtpError::ClockUpdate);
    }
    let mono_before = monotonic_millis();
    // Sub-second from monotonic for fractional uniqueness.
    let ms_part = (mono_before % 1000) as u32;
    let (xmit_secs, xmit_frac) = unix_to_ntp(local_unix, ms_part).ok_or(NtpError::ClockUpdate)?;
    let req = build_client_request(xmit_secs, xmit_frac);

    let (ptr, tok) = shm_alloc().map_err(|_| NtpError::SocketError)?;
    // SAFETY: freshly allocated page of at least 4096 bytes.
    unsafe {
        core::ptr::write_bytes(ptr, 0, 4096);
        core::ptr::copy_nonoverlapping(req.as_ptr(), ptr, NTP_PACKET_LEN);
    }

    let t1_ms = local_unix
        .saturating_mul(1000)
        .saturating_add(ms_part as u64);

    let msg = IpcMsg::with_label(NetOp::UDP_EXCHANGE)
        .word(0, pack_ipv4(ip))
        .word(1, NTP_PORT as u64)
        .word(2, NTP_PACKET_LEN as u64)
        .word(3, SERVER_TIMEOUT_MS)
        .with_cap(0, tok);

    let reply = match ipc_call_timeout(net, msg, SERVER_TIMEOUT_MS + 500) {
        Ok(r) => r,
        Err(_) => {
            let _ = shm_free(tok);
            return Err(NtpError::Timeout);
        }
    };

    if reply.label != NetOp::UDP_EXCHANGE || reply.words[0] == 0 {
        let _ = shm_free(tok);
        return Err(if reply.words[1] == 5 {
            NtpError::Timeout
        } else {
            NtpError::SocketError
        });
    }

    let resp_len = (reply.words[0] as usize).min(4096);
    let mut resp = [0u8; 64];
    let copy_len = resp_len.min(64);
    // SAFETY: net_server wrote resp_len bytes into the page.
    unsafe {
        core::ptr::copy_nonoverlapping(ptr, resp.as_mut_ptr(), copy_len);
    }
    let _ = shm_free(tok);

    let mono_after = monotonic_millis();
    let t4_ms = t1_ms.saturating_add(mono_after.saturating_sub(mono_before));

    parse_and_validate(&resp[..copy_len], xmit_secs, xmit_frac, t1_ms, t4_ms)
}

fn resolve_hostname(net: CapabilityToken, hostname: &str) -> Option<[u8; 4]> {
    let bytes = hostname.as_bytes();
    let name_len = bytes.len().min(48);
    let mut msg = IpcMsg::with_label(NetOp::RESOLVE).word(0, name_len as u64);
    let mut w_idx = 1usize;
    let mut b_idx = 0usize;
    while b_idx < name_len && w_idx < 8 {
        let mut w = 0u64;
        for j in 0..8 {
            if b_idx >= name_len {
                break;
            }
            w |= (bytes[b_idx] as u64) << (j * 8);
            b_idx += 1;
        }
        msg = msg.word(w_idx, w);
        w_idx += 1;
    }
    let reply = ipc_call_timeout(net, msg, 5_000).ok()?;
    if reply.label != NetOp::RESOLVE || reply.words[0] == 0 {
        return None;
    }
    Some(unpack_ipv4(reply.words[0]))
}

fn refresh_region_from_tz(sync: &mut SyncStatus) {
    if sync.explicit_servers {
        return;
    }
    // Prefer timezone_service metadata (static; no wall clock needed).
    if let Some(tz) = nameserver_lookup_timeout("tz", 100) {
        let reply = ipc_call(tz, IpcMsg::with_label(TzMsg::GET_NTP_REGION));
        if reply.label == TzMsg::REPLY {
            sync.ntp_region = (reply.words[0] & 0xff) as u8;
            // Zone id packed from word 2..
            let mut idbuf = [0u8; 64];
            let mut i = 0usize;
            for wi in 2..8 {
                let w = reply.words[wi];
                for b in 0..8 {
                    if i >= 63 {
                        break;
                    }
                    let ch = ((w >> (b * 8)) & 0xff) as u8;
                    if ch == 0 {
                        break;
                    }
                    idbuf[i] = ch;
                    i += 1;
                }
            }
            if i > 0 {
                if let Ok(s) = core::str::from_utf8(&idbuf[..i]) {
                    sync.set_zone(s);
                }
            }
            select_regional_servers(sync);
            return;
        }
        // Fallback: GET_ZONE id
        let reply = ipc_call(tz, IpcMsg::with_label(TzMsg::GET_ZONE));
        if reply.label == TzMsg::REPLY {
            let mut idbuf = [0u8; 64];
            let mut i = 0usize;
            for wi in 2..8 {
                let w = reply.words[wi];
                for b in 0..8 {
                    if i >= 63 {
                        break;
                    }
                    let ch = ((w >> (b * 8)) & 0xff) as u8;
                    if ch == 0 {
                        break;
                    }
                    idbuf[i] = ch;
                    i += 1;
                }
            }
            if let Ok(s) = core::str::from_utf8(&idbuf[..i]) {
                if !s.is_empty() {
                    sync.set_zone(s);
                    let region = ntp_region_from_zone_id(s);
                    sync.ntp_region = region as u8;
                    select_regional_servers(sync);
                    return;
                }
            }
        }
    }
    // Default: global pool
    sync.set_zone("UTC");
    sync.ntp_region = NtpRegion::Global as u8;
    select_regional_servers(sync);
}

fn select_regional_servers(sync: &mut SyncStatus) {
    if sync.explicit_servers {
        return;
    }
    let region = NtpRegion::from_u8(sync.ntp_region);
    sync.server_count = 0;
    for i in 0..NTP_POOL_SERVER_COUNT.min(MAX_SERVERS) {
        let mut host = [0u8; NTP_HOSTNAME_MAX];
        format_pool_hostname_into(i as u8, region, &mut host);
        sync.set_server_bytes(i, &host);
    }
}

fn load_explicit_servers(sync: &mut SyncStatus) {
    // Optional /etc/sunlight/ntp.conf lines: "server hostname"
    // Best-effort via VFS; absence is fine.
    let Some(vfs) = nameserver_lookup("vfs") else {
        return;
    };
    let open = path_msg(sunlight_ipc::VfsMsg::OPEN, CONFIG_PATH);
    let reply = ipc_call(vfs, open);
    if reply.label != sunlight_ipc::VfsMsg::REPLY || reply.words[0] != 0 {
        return;
    }
    let handle = reply.words[1];
    let mut buf = [0u8; 512];
    let mut len = 0usize;
    let mut offset = 0usize;
    loop {
        let r = ipc_call(
            vfs,
            IpcMsg::with_label(sunlight_ipc::VfsMsg::READ)
                .word(0, handle)
                .word(1, offset as u64)
                .word(2, 16),
        );
        if r.label != sunlight_ipc::VfsMsg::REPLY {
            break;
        }
        let n = r.words[1] as usize;
        if n == 0 {
            break;
        }
        let src_words = &r.words[2..];
        for i in 0..n {
            if len >= buf.len() {
                break;
            }
            let word_idx = i / 8;
            let byte_idx = i % 8;
            buf[len] = ((src_words[word_idx] >> (byte_idx * 8)) & 0xff) as u8;
            len += 1;
        }
        offset += n;
        if n < 16 {
            break;
        }
    }
    let _ = ipc_call(
        vfs,
        IpcMsg::with_label(sunlight_ipc::VfsMsg::CLOSE).word(0, handle),
    );

    if len == 0 {
        return;
    }
    let text = core::str::from_utf8(&buf[..len]).unwrap_or("");
    let mut count = 0usize;
    for line in text.split(|c| c == '\n' || c == '\r') {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let key = parts.next().unwrap_or("");
        let host = parts.next().unwrap_or("");
        if !eq_ascii_ignore(key, "server") || host.is_empty() {
            continue;
        }
        if count < MAX_SERVERS {
            sync.set_server_bytes(count, host.as_bytes());
            count += 1;
        }
    }
    if count > 0 {
        sync.server_count = count;
        sync.explicit_servers = true;
        debug_log("[timed] using explicit NTP servers from ntp.conf");
    }
}

fn path_msg(label: u64, path: &str) -> IpcMsg {
    let bytes = path.as_bytes();
    let mut msg = IpcMsg::with_label(label).word(0, bytes.len() as u64);
    let mut w_idx = 1usize;
    let mut b_idx = 0usize;
    while b_idx < bytes.len() && w_idx < 8 {
        let mut w = 0u64;
        for j in 0..8 {
            if b_idx >= bytes.len() {
                break;
            }
            w |= (bytes[b_idx] as u64) << (j * 8);
            b_idx += 1;
        }
        msg = msg.word(w_idx, w);
        w_idx += 1;
    }
    msg
}

fn eq_ascii_ignore(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.as_bytes()
        .iter()
        .zip(b.as_bytes())
        .all(|(&x, &y)| x.to_ascii_lowercase() == y.to_ascii_lowercase())
}

fn pack_ipv4(ip: [u8; 4]) -> u64 {
    (ip[0] as u64) | ((ip[1] as u64) << 8) | ((ip[2] as u64) << 16) | ((ip[3] as u64) << 24)
}

fn unpack_ipv4(v: u64) -> [u8; 4] {
    [
        (v & 0xff) as u8,
        ((v >> 8) & 0xff) as u8,
        ((v >> 16) & 0xff) as u8,
        ((v >> 24) & 0xff) as u8,
    ]
}

fn get_entropy_word() -> u64 {
    // Prefer rand service; fall back to monotonic mix.
    if let Some(cap) = nameserver_lookup_timeout("rand", 50) {
        let r = ipc_call(
            cap,
            IpcMsg::with_label(sunlight_ipc::RandMsg::GET).word(0, 8),
        );
        if r.label == sunlight_ipc::RandMsg::REPLY {
            return r.words[0];
        }
    }
    monotonic_millis().wrapping_mul(0x9E37_79B9_7F4A_7C15)
}
