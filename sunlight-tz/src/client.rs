//! Shared IPC clients for the Time Service (`timed`) and Timezone Service (`tz`).
//!
//! Used by `tzutils`, sunshell `tzctl`, and Control Panel so the GUI never
//! spawns CLI tools or parses their text output.

use sunlight_ipc::{
    ipc_call_timeout, nameserver_lookup_timeout, CapabilityToken, IpcCallError, IpcMsg,
    NtpSyncState, TimeMsg, TzMsg,
};

/// Lookup / request timeouts (milliseconds).
pub const LOOKUP_TIMEOUT_MS: u64 = 1_000;
pub const QUERY_TIMEOUT_MS: u64 = 2_000;
pub const SET_ZONE_TIMEOUT_MS: u64 = 3_000;
/// Bounded wait for a full NTP attempt (matches tzutils).
pub const SYNC_TIMEOUT_MS: u64 = 35_000;

// ---------------------------------------------------------------------------
// Time Service client
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeClientError {
    ServiceUnavailable,
    Timeout,
    Transport,
    Failed(u64),
    Unexpected,
}

/// Packed NTP synchronization status (authoritative from `timed`).
#[derive(Clone, Copy, Debug, Default)]
pub struct SyncStatusSnapshot {
    pub state: u64,
    pub stratum: u8,
    pub region: u8,
    pub server_count: u8,
    pub ntp_synced: bool,
    pub rtc_updated: bool,
    pub explicit_servers: bool,
    pub last_offset_ms: i64,
    pub last_delay_ms: u64,
    pub last_error: u64,
    pub last_sync_unix: u64,
    pub next_attempt_mono_ms: u64,
    pub backoff_ms: u64,
    /// Last successful server name (up to 8 packed bytes from the wire).
    pub last_server: [u8; 8],
    pub last_server_len: usize,
    /// First 8 bytes of zone id as reported by timed.
    pub zone_prefix: [u8; 8],
    pub zone_prefix_len: usize,
}

impl SyncStatusSnapshot {
    pub fn state_label(self) -> &'static str {
        match self.state {
            NtpSyncState::UNSYNCHRONIZED => "Unsynchronized",
            NtpSyncState::WAITING_NETWORK => "Waiting for network",
            NtpSyncState::RESOLVING => "Resolving",
            NtpSyncState::SAMPLING => "Syncing",
            NtpSyncState::SYNCHRONIZED => "Synchronized",
            NtpSyncState::DEGRADED => "Degraded",
            NtpSyncState::FAILED => "Failed",
            _ => "Unknown",
        }
    }

    pub fn region_label(self) -> &'static str {
        match self.region {
            1 => "africa",
            2 => "asia",
            3 => "europe",
            4 => "north-america",
            5 => "south-america",
            6 => "oceania",
            _ => "global",
        }
    }

    pub fn last_server_str(&self) -> &str {
        core::str::from_utf8(&self.last_server[..self.last_server_len]).unwrap_or("")
    }

    pub fn error_label(code: u64) -> &'static str {
        match code {
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
}

/// Client for the `timed` service.
pub struct TimeClient {
    cap: CapabilityToken,
}

impl TimeClient {
    pub fn connect() -> Result<Self, TimeClientError> {
        let cap = nameserver_lookup_timeout("timed", LOOKUP_TIMEOUT_MS)
            .ok_or(TimeClientError::ServiceUnavailable)?;
        Ok(Self { cap })
    }

    pub fn get_utc(&self) -> Result<u64, TimeClientError> {
        let reply = call(
            self.cap,
            IpcMsg::with_label(TimeMsg::GET_UTC),
            QUERY_TIMEOUT_MS,
        )?;
        if reply.label != TimeMsg::REPLY {
            return Err(TimeClientError::Unexpected);
        }
        Ok(reply.words[0])
    }

    pub fn sync_status(&self) -> Result<SyncStatusSnapshot, TimeClientError> {
        let reply = call(
            self.cap,
            IpcMsg::with_label(TimeMsg::GET_SYNC_STATUS),
            QUERY_TIMEOUT_MS,
        )?;
        if reply.label != TimeMsg::REPLY {
            return Err(TimeClientError::Unexpected);
        }
        Ok(unpack_sync_status(&reply))
    }

    /// Request NTP synchronization using the normal policy (no force).
    ///
    /// Blocks until timed finishes the attempt or the bounded timeout elapses.
    /// Does **not** report success merely because a request was queued.
    pub fn sync_now(&self) -> Result<SyncStatusSnapshot, TimeClientError> {
        self.sync_now_with_flags(0)
    }

    /// Same as [`Self::sync_now`] with optional `TimeMsg::SYNC_FLAG_FORCE`.
    /// Force requires root inside timed; ordinary Control Panel use must not set it.
    pub fn sync_now_with_flags(&self, flags: u64) -> Result<SyncStatusSnapshot, TimeClientError> {
        let req = IpcMsg::with_label(TimeMsg::SYNC_NTP).word(0, flags);
        match ipc_call_timeout(self.cap, req, SYNC_TIMEOUT_MS) {
            Ok(reply) if reply.label == TimeMsg::REPLY => {
                // Authoritative post-sync status (not just "queued").
                self.sync_status()
            }
            Ok(reply) if reply.label == TimeMsg::ERROR => {
                Err(TimeClientError::Failed(reply.words[0]))
            }
            Ok(_) => Err(TimeClientError::Unexpected),
            Err(IpcCallError::Timeout) => Err(TimeClientError::Timeout),
            Err(_) => Err(TimeClientError::Transport),
        }
    }
}

fn unpack_sync_status(r: &IpcMsg) -> SyncStatusSnapshot {
    let flags = (r.words[0] >> 32) & 0xff;
    let mut last_server = [0u8; 8];
    let mut last_server_len = 0usize;
    for i in 0..8 {
        let ch = ((r.words[6] >> (i * 8)) & 0xff) as u8;
        if ch == 0 {
            break;
        }
        last_server[i] = ch;
        last_server_len = i + 1;
    }
    let mut zone_prefix = [0u8; 8];
    let mut zone_prefix_len = 0usize;
    for i in 0..8 {
        let ch = ((r.words[7] >> (i * 8)) & 0xff) as u8;
        if ch == 0 {
            break;
        }
        zone_prefix[i] = ch;
        zone_prefix_len = i + 1;
    }
    SyncStatusSnapshot {
        state: r.words[0] & 0xff,
        stratum: ((r.words[0] >> 8) & 0xff) as u8,
        region: ((r.words[0] >> 16) & 0xff) as u8,
        server_count: ((r.words[0] >> 24) & 0xff) as u8,
        ntp_synced: (flags & 1) != 0,
        rtc_updated: (flags & 2) != 0,
        explicit_servers: (flags & 4) != 0,
        last_offset_ms: r.words[1] as i64,
        last_delay_ms: r.words[2] & 0xffff_ffff_ffff,
        last_error: r.words[2] >> 48,
        last_sync_unix: r.words[3],
        next_attempt_mono_ms: r.words[4],
        backoff_ms: r.words[5],
        last_server,
        last_server_len,
        zone_prefix,
        zone_prefix_len,
    }
}

// ---------------------------------------------------------------------------
// Timezone Service client
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TzClientError {
    ServiceUnavailable,
    Timeout,
    Transport,
    /// Unknown / unsupported zone id (service error code 1).
    UnsupportedZone,
    /// Persistence failure (service error code 3).
    PersistFailed,
    Failed(u64),
    Unexpected,
}

/// Active timezone identity from `GET_ZONE`.
#[derive(Clone, Copy, Debug)]
pub struct ZoneSnapshot {
    pub utc_offset_hours: i8,
    pub utc_offset_minutes: u8,
    pub dst_offset_minutes: u8,
    pub dst_start_month: u8,
    pub dst_end_month: u8,
    pub id: [u8; 64],
    pub id_len: usize,
}

impl ZoneSnapshot {
    pub fn id_str(&self) -> &str {
        core::str::from_utf8(&self.id[..self.id_len]).unwrap_or("")
    }
}

/// Local civil time from `GET_LOCAL_TIME`.
#[derive(Clone, Copy, Debug, Default)]
pub struct LocalTimeSnapshot {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    pub utc_offset_secs: i64,
    pub is_dst: bool,
    pub abbr: [u8; 8],
}

impl LocalTimeSnapshot {
    pub fn abbr_str(&self) -> &str {
        let len = self.abbr.iter().position(|&b| b == 0).unwrap_or(8);
        core::str::from_utf8(&self.abbr[..len]).unwrap_or("")
    }
}

/// Client for the `tz` service.
pub struct TzClient {
    cap: CapabilityToken,
}

impl TzClient {
    pub fn connect() -> Result<Self, TzClientError> {
        let cap = nameserver_lookup_timeout("tz", LOOKUP_TIMEOUT_MS)
            .ok_or(TzClientError::ServiceUnavailable)?;
        Ok(Self { cap })
    }

    pub fn get_zone(&self) -> Result<ZoneSnapshot, TzClientError> {
        let reply = call_tz(
            self.cap,
            IpcMsg::with_label(TzMsg::GET_ZONE),
            QUERY_TIMEOUT_MS,
        )?;
        if reply.label != TzMsg::REPLY {
            return Err(map_tz_error(&reply));
        }
        let w0 = reply.words[0];
        let w1 = reply.words[1];
        let mut id = [0u8; 64];
        let mut id_len = 0usize;
        'words: for wi in 2..6 {
            for b in 0..8 {
                let ch = ((reply.words[wi] >> (b * 8)) & 0xff) as u8;
                if ch == 0 || id_len >= 63 {
                    break 'words;
                }
                id[id_len] = ch;
                id_len += 1;
            }
        }
        Ok(ZoneSnapshot {
            utc_offset_hours: (w0 & 0xff) as i8,
            utc_offset_minutes: ((w0 >> 8) & 0xff) as u8,
            dst_offset_minutes: ((w0 >> 16) & 0xff) as u8,
            dst_start_month: (w1 & 0xff) as u8,
            dst_end_month: ((w1 >> 8) & 0xff) as u8,
            id,
            id_len,
        })
    }

    pub fn get_local_time(&self) -> Result<LocalTimeSnapshot, TzClientError> {
        let reply = call_tz(
            self.cap,
            IpcMsg::with_label(TzMsg::GET_LOCAL_TIME),
            QUERY_TIMEOUT_MS,
        )?;
        if reply.label != TzMsg::REPLY {
            return Err(map_tz_error(&reply));
        }
        let w0 = reply.words[0];
        let mut abbr = [0u8; 8];
        for i in 0..8 {
            abbr[i] = ((reply.words[3] >> (i * 8)) & 0xff) as u8;
        }
        Ok(LocalTimeSnapshot {
            year: ((w0 >> 48) & 0xffff) as u16,
            month: ((w0 >> 40) & 0xff) as u8,
            day: ((w0 >> 32) & 0xff) as u8,
            hour: ((w0 >> 24) & 0xff) as u8,
            minute: ((w0 >> 16) & 0xff) as u8,
            second: ((w0 >> 8) & 0xff) as u8,
            utc_offset_secs: reply.words[1] as i64,
            is_dst: reply.words[2] != 0,
            abbr,
        })
    }

    /// Set the system timezone by canonical IANA id.
    ///
    /// The timezone service preserves UTC wall time and updates local conversion
    /// once (offset/DST + regional NTP notification). Control Panel must not
    /// write a second copy of timezone state.
    pub fn set_zone(&self, zone_id: &str) -> Result<(), TzClientError> {
        if zone_id.is_empty() || zone_id.len() > 63 {
            return Err(TzClientError::UnsupportedZone);
        }
        let mut req = IpcMsg::with_label(TzMsg::SET_ZONE);
        let mut wi = 0usize;
        let mut bi = 0usize;
        let mut w = 0u64;
        for &bb in zone_id.as_bytes().iter().take(32) {
            w |= (bb as u64) << (bi * 8);
            bi += 1;
            if bi == 8 {
                req = req.word(wi, w);
                w = 0;
                bi = 0;
                wi += 1;
            }
        }
        if bi > 0 {
            req = req.word(wi, w);
        }
        let reply = call_tz(self.cap, req, SET_ZONE_TIMEOUT_MS)?;
        if reply.label == TzMsg::REPLY && reply.words[0] == 0 {
            Ok(())
        } else if reply.label == TzMsg::ERROR {
            Err(map_tz_error(&reply))
        } else {
            Err(TzClientError::Unexpected)
        }
    }

    pub fn get_ntp_region(&self) -> Result<(u8, [u8; 64], usize), TzClientError> {
        let reply = call_tz(
            self.cap,
            IpcMsg::with_label(TzMsg::GET_NTP_REGION),
            QUERY_TIMEOUT_MS,
        )?;
        if reply.label != TzMsg::REPLY {
            return Err(map_tz_error(&reply));
        }
        let region = (reply.words[0] & 0xff) as u8;
        let mut id = [0u8; 64];
        let mut id_len = 0usize;
        'words: for wi in 2..6 {
            for b in 0..8 {
                let ch = ((reply.words[wi] >> (b * 8)) & 0xff) as u8;
                if ch == 0 || id_len >= 63 {
                    break 'words;
                }
                id[id_len] = ch;
                id_len += 1;
            }
        }
        Ok((region, id, id_len))
    }
}

fn map_tz_error(reply: &IpcMsg) -> TzClientError {
    match reply.words[0] {
        1 => TzClientError::UnsupportedZone,
        3 => TzClientError::PersistFailed,
        code => TzClientError::Failed(code),
    }
}

fn call(cap: CapabilityToken, msg: IpcMsg, timeout_ms: u64) -> Result<IpcMsg, TimeClientError> {
    match ipc_call_timeout(cap, msg, timeout_ms) {
        Ok(r) => Ok(r),
        Err(IpcCallError::Timeout) => Err(TimeClientError::Timeout),
        Err(_) => Err(TimeClientError::Transport),
    }
}

fn call_tz(cap: CapabilityToken, msg: IpcMsg, timeout_ms: u64) -> Result<IpcMsg, TzClientError> {
    match ipc_call_timeout(cap, msg, timeout_ms) {
        Ok(r) => Ok(r),
        Err(IpcCallError::Timeout) => Err(TzClientError::Timeout),
        Err(_) => Err(TzClientError::Transport),
    }
}
