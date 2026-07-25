//! Time / NTP sync state for the timed service.

use crate::ntp::{NtpError, SyncState, MAX_SAMPLES};

/// Maximum configured / selected NTP server hostnames we track.
pub const MAX_SERVERS: usize = 4;
pub const HOSTNAME_MAX: usize = 48;
pub const ZONE_ID_MAX: usize = 64;

/// Runtime NTP synchronization state owned by timed.
#[derive(Clone, Debug)]
pub struct SyncStatus {
    pub state: SyncState,
    pub zone_id: [u8; ZONE_ID_MAX],
    pub zone_len: usize,
    pub ntp_region: u8,
    pub servers: [[u8; HOSTNAME_MAX]; MAX_SERVERS],
    pub server_count: usize,
    pub last_server: [u8; HOSTNAME_MAX],
    pub last_server_len: usize,
    pub stratum: u8,
    pub last_offset_ms: i64,
    pub last_delay_ms: u64,
    pub last_sync_unix: u64,
    pub last_error: u64,
    pub next_attempt_mono_ms: u64,
    pub backoff_ms: u64,
    pub rtc_updated: bool,
    pub ntp_synced: bool,
    pub explicit_servers: bool,
}

impl SyncStatus {
    pub const fn new() -> Self {
        Self {
            state: SyncState::Unsynchronized,
            zone_id: [0; ZONE_ID_MAX],
            zone_len: 0,
            ntp_region: 0,
            servers: [[0; HOSTNAME_MAX]; MAX_SERVERS],
            server_count: 0,
            last_server: [0; HOSTNAME_MAX],
            last_server_len: 0,
            stratum: 0,
            last_offset_ms: 0,
            last_delay_ms: 0,
            last_sync_unix: 0,
            last_error: 0,
            next_attempt_mono_ms: 0,
            backoff_ms: 0,
            rtc_updated: false,
            ntp_synced: false,
            explicit_servers: false,
        }
    }

    pub fn set_zone(&mut self, id: &str) {
        self.zone_id = [0; ZONE_ID_MAX];
        let b = id.as_bytes();
        let n = b.len().min(ZONE_ID_MAX - 1);
        self.zone_id[..n].copy_from_slice(&b[..n]);
        self.zone_len = n;
    }

    pub fn zone_str(&self) -> &str {
        core::str::from_utf8(&self.zone_id[..self.zone_len]).unwrap_or("UTC")
    }

    pub fn set_servers_from_hostnames(&mut self, hosts: &[&str]) {
        self.servers = [[0; HOSTNAME_MAX]; MAX_SERVERS];
        self.server_count = 0;
        for host in hosts.iter().take(MAX_SERVERS) {
            let b = host.as_bytes();
            let n = b.len().min(HOSTNAME_MAX - 1);
            self.servers[self.server_count][..n].copy_from_slice(&b[..n]);
            self.server_count += 1;
        }
    }

    pub fn set_server_bytes(&mut self, index: usize, host: &[u8]) {
        if index >= MAX_SERVERS {
            return;
        }
        self.servers[index] = [0; HOSTNAME_MAX];
        let n = host.len().min(HOSTNAME_MAX - 1);
        // trim at first nul
        let end = host[..n].iter().position(|&c| c == 0).unwrap_or(n);
        self.servers[index][..end].copy_from_slice(&host[..end]);
        if index >= self.server_count {
            self.server_count = index + 1;
        }
    }

    pub fn server_str(&self, index: usize) -> &str {
        if index >= self.server_count {
            return "";
        }
        let b = &self.servers[index];
        let n = b.iter().position(|&c| c == 0).unwrap_or(HOSTNAME_MAX);
        core::str::from_utf8(&b[..n]).unwrap_or("")
    }

    pub fn set_last_server(&mut self, host: &str) {
        self.last_server = [0; HOSTNAME_MAX];
        let b = host.as_bytes();
        let n = b.len().min(HOSTNAME_MAX - 1);
        self.last_server[..n].copy_from_slice(&b[..n]);
        self.last_server_len = n;
    }

    pub fn last_server_str(&self) -> &str {
        core::str::from_utf8(&self.last_server[..self.last_server_len]).unwrap_or("")
    }

    pub fn record_error(&mut self, err: NtpError) {
        self.last_error = err.as_time_err();
        if matches!(
            err,
            NtpError::KissOfDeath | NtpError::Timeout | NtpError::NetworkDown | NtpError::DnsError
        ) {
            self.state = SyncState::Degraded;
        } else {
            self.state = SyncState::Failed;
        }
    }

    pub fn record_success(
        &mut self,
        server: &str,
        stratum: u8,
        offset_ms: i64,
        delay_ms: u64,
        sync_unix: u64,
    ) {
        self.state = SyncState::Synchronized;
        self.ntp_synced = true;
        self.stratum = stratum;
        self.last_offset_ms = offset_ms;
        self.last_delay_ms = delay_ms;
        self.last_sync_unix = sync_unix;
        self.last_error = 0;
        self.backoff_ms = 0;
        self.set_last_server(server);
    }
}

/// Legacy TimeState (UTC + NTP flag) kept for GET_STATE back-compat.
#[repr(C)]
#[derive(Clone, Debug)]
pub struct TimeState {
    pub utc_epoch: u64,
    pub ntp_synced: bool,
    pub ntp_drift_ppm: i32,
}

impl TimeState {
    pub const fn new() -> Self {
        Self {
            utc_epoch: 0,
            ntp_synced: false,
            ntp_drift_ppm: 0,
        }
    }
}

/// Bounded sample buffer used during one sync attempt.
pub struct SampleBuf {
    pub samples: [crate::ntp::NtpSample; MAX_SAMPLES],
    pub len: usize,
}

impl SampleBuf {
    pub const fn new() -> Self {
        Self {
            samples: [crate::ntp::NtpSample {
                offset_ms: 0,
                delay_ms: 0,
                stratum: 0,
                server_unix: 0,
                originate_secs: 0,
            }; MAX_SAMPLES],
            len: 0,
        }
    }

    pub fn push(&mut self, s: crate::ntp::NtpSample) {
        if self.len < MAX_SAMPLES {
            self.samples[self.len] = s;
            self.len += 1;
        }
    }

    pub fn as_slice(&self) -> &[crate::ntp::NtpSample] {
        &self.samples[..self.len]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zone_and_server_roundtrip() {
        let mut s = SyncStatus::new();
        s.set_zone("Asia/Baku");
        assert_eq!(s.zone_str(), "Asia/Baku");
        s.set_servers_from_hostnames(&["0.asia.pool.ntp.org", "1.asia.pool.ntp.org"]);
        assert_eq!(s.server_count, 2);
        assert_eq!(s.server_str(0), "0.asia.pool.ntp.org");
    }
}
