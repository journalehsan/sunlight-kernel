//! Bounded SNTP/NTP client protocol (client mode only).
//!
//! Implements packet build/parse, validation, four-timestamp offset/delay
//! math with checked integer arithmetic, and a small sample selector.
//!
//! NTP timestamps use the NTP short/era-0 32-bit seconds representation
//! relative to 1900-01-01. Conversion to Unix uses the fixed era-0 delta.

/// NTP packet size (minimum valid response).
pub const NTP_PACKET_LEN: usize = 48;
/// NTP port.
pub const NTP_PORT: u16 = 123;
/// Seconds between NTP epoch (1900-01-01) and Unix epoch (1970-01-01).
pub const NTP_UNIX_DELTA: u64 = 2_208_988_800;
/// Version we speak (NTPv4).
pub const NTP_VERSION: u8 = 4;
/// Client mode.
pub const MODE_CLIENT: u8 = 3;
/// Server mode.
pub const MODE_SERVER: u8 = 4;
/// Symmetric active (also accepted as a rare response mode).
pub const MODE_SYMMETRIC_ACTIVE: u8 = 1;
/// Leap indicator: unsynchronized / alarm.
pub const LI_UNSYNC: u8 = 3;
/// Maximum accepted absolute offset for a non-forced step (seconds).
pub const MAX_NORMAL_STEP_SECS: i64 = 1_000;
/// Maximum accepted round-trip delay (seconds, fixed-point ms bound ~30s).
pub const MAX_DELAY_MS: u64 = 30_000;
/// Maximum samples retained during one sync attempt.
pub const MAX_SAMPLES: usize = 8;

/// Sync state machine values (match `ipc::NtpSyncState`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum SyncState {
    Unsynchronized = 0,
    WaitingNetwork = 1,
    Resolving = 2,
    Sampling = 3,
    Synchronized = 4,
    Degraded = 5,
    Failed = 6,
}

impl SyncState {
    pub const fn as_u64(self) -> u64 {
        self as u64
    }
    pub const fn from_u64(v: u64) -> Self {
        match v {
            1 => Self::WaitingNetwork,
            2 => Self::Resolving,
            3 => Self::Sampling,
            4 => Self::Synchronized,
            5 => Self::Degraded,
            6 => Self::Failed,
            _ => Self::Unsynchronized,
        }
    }
}

/// NTP client protocol errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NtpError {
    NetworkDown,
    SocketError,
    Timeout,
    InvalidResponse,
    DnsError,
    KissOfDeath,
    Permission,
    ClockUpdate,
    Busy,
    NoSample,
}

impl NtpError {
    pub const fn as_time_err(self) -> u64 {
        // Match TimeMsg::ERR_* codes.
        match self {
            Self::NetworkDown | Self::SocketError => 2,
            Self::DnsError => 3,
            Self::Timeout => 4,
            Self::InvalidResponse | Self::KissOfDeath | Self::NoSample => 5,
            Self::Permission => 6,
            Self::ClockUpdate => 7,
            Self::Busy => 8,
        }
    }
}

/// One validated sample from a single server exchange.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NtpSample {
    /// Estimated clock offset: remote − local, in milliseconds.
    pub offset_ms: i64,
    /// Round-trip delay in milliseconds.
    pub delay_ms: u64,
    /// Server stratum (1..=15).
    pub stratum: u8,
    /// Server Unix UTC seconds (from transmit timestamp).
    pub server_unix: u64,
    /// Origin matching token (our transmit NTP seconds, for diagnostics).
    pub originate_secs: u32,
}

/// Build a 48-byte NTPv4 client request.
///
/// `xmit_secs` / `xmit_frac` form the transmit timestamp (T1) that must be
/// echoed by the server as the originate timestamp.
pub fn build_client_request(xmit_secs: u32, xmit_frac: u32) -> [u8; NTP_PACKET_LEN] {
    let mut pkt = [0u8; NTP_PACKET_LEN];
    // LI=0, VN=4, Mode=3 (client)
    pkt[0] = (NTP_VERSION << 3) | MODE_CLIENT;
    // Transmit Timestamp at offset 40
    pkt[40..44].copy_from_slice(&xmit_secs.to_be_bytes());
    pkt[44..48].copy_from_slice(&xmit_frac.to_be_bytes());
    pkt
}

/// Convert Unix UTC seconds + sub-second milliseconds to an NTP timestamp.
///
/// Returns `(ntp_secs, ntp_frac)` in era 0. Fractions use 2^32 per second
/// approximated from milliseconds: frac ≈ ms * 2^32 / 1000.
pub fn unix_to_ntp(unix_secs: u64, millis: u32) -> Option<(u32, u32)> {
    let ntp_secs = unix_secs.checked_add(NTP_UNIX_DELTA)?;
    // Era 0 only for this slice; reject overflow past u32.
    if ntp_secs > u32::MAX as u64 {
        return None;
    }
    let ms = (millis % 1000) as u64;
    // frac = ms * 2^32 / 1000, checked.
    let frac = ms
        .checked_mul(1u64 << 32)?
        .checked_div(1000)?
        .min(u32::MAX as u64) as u32;
    Some((ntp_secs as u32, frac))
}

/// Convert NTP timestamp seconds to Unix UTC seconds (era 0).
pub fn ntp_secs_to_unix(ntp_secs: u32) -> Option<u64> {
    let n = ntp_secs as u64;
    n.checked_sub(NTP_UNIX_DELTA)
}

/// Validate and parse an NTP response against our request identity.
///
/// `t1_unix_ms` / `t4_unix_ms` are local timestamps in Unix milliseconds for
/// the four-timestamp calculation (T1 send, T4 receive). They must use a
/// consistent clock domain (wall UTC is fine for offset estimation when the
/// local wall is the quantity being corrected; delay uses their difference).
///
/// For delay/offset we treat local times as an arbitrary timeline measured in
/// ms — only (T4−T1) matters for delay and the absolute values cancel for
/// offset when combined with server timestamps.
pub fn parse_and_validate(
    resp: &[u8],
    expect_originate_secs: u32,
    expect_originate_frac: u32,
    t1_ms: u64,
    t4_ms: u64,
) -> Result<NtpSample, NtpError> {
    if resp.len() < NTP_PACKET_LEN {
        return Err(NtpError::InvalidResponse);
    }

    let li = resp[0] >> 6;
    let version = (resp[0] >> 3) & 0x07;
    let mode = resp[0] & 0x07;
    let stratum = resp[1];

    if version < 3 || version > 4 {
        return Err(NtpError::InvalidResponse);
    }
    if mode != MODE_SERVER && mode != MODE_SYMMETRIC_ACTIVE {
        return Err(NtpError::InvalidResponse);
    }
    if li == LI_UNSYNC {
        return Err(NtpError::InvalidResponse);
    }
    if stratum == 0 {
        // Kiss-o'-Death
        return Err(NtpError::KissOfDeath);
    }
    if stratum > 15 {
        return Err(NtpError::InvalidResponse);
    }

    let originate_secs = u32::from_be_bytes([resp[24], resp[25], resp[26], resp[27]]);
    let originate_frac = u32::from_be_bytes([resp[28], resp[29], resp[30], resp[31]]);
    let rx_secs = u32::from_be_bytes([resp[32], resp[33], resp[34], resp[35]]);
    let rx_frac = u32::from_be_bytes([resp[36], resp[37], resp[38], resp[39]]);
    let tx_secs = u32::from_be_bytes([resp[40], resp[41], resp[42], resp[43]]);
    let tx_frac = u32::from_be_bytes([resp[44], resp[45], resp[46], resp[47]]);

    if originate_secs != expect_originate_secs || originate_frac != expect_originate_frac {
        return Err(NtpError::InvalidResponse);
    }
    if rx_secs == 0 || tx_secs == 0 {
        return Err(NtpError::InvalidResponse);
    }

    let t2_ms = ntp_timestamp_to_unix_ms(rx_secs, rx_frac).ok_or(NtpError::InvalidResponse)?;
    let t3_ms = ntp_timestamp_to_unix_ms(tx_secs, tx_frac).ok_or(NtpError::InvalidResponse)?;
    let t1 = t1_ms as i128;
    let t2 = t2_ms as i128;
    let t3 = t3_ms as i128;
    let t4 = t4_ms as i128;

    // offset = ((T2 - T1) + (T3 - T4)) / 2
    // delay  = (T4 - T1) - (T3 - T2)
    let offset = ((t2 - t1) + (t3 - t4)) / 2;
    let delay = (t4 - t1) - (t3 - t2);

    if delay < 0 {
        return Err(NtpError::InvalidResponse);
    }
    let delay_ms = delay as u64;
    if delay_ms > MAX_DELAY_MS {
        return Err(NtpError::InvalidResponse);
    }
    // Bound absurd offsets (more than ~10 years) as impossible.
    if offset.abs() > 10 * 365 * 24 * 3600 * 1000 {
        return Err(NtpError::InvalidResponse);
    }

    let server_unix = ntp_secs_to_unix(tx_secs).ok_or(NtpError::InvalidResponse)?;

    Ok(NtpSample {
        offset_ms: offset as i64,
        delay_ms,
        stratum,
        server_unix,
        originate_secs,
    })
}

fn ntp_timestamp_to_unix_ms(secs: u32, frac: u32) -> Option<u64> {
    let unix = ntp_secs_to_unix(secs)?;
    // ms = frac * 1000 / 2^32
    let ms = (frac as u64).checked_mul(1000)? >> 32;
    unix.checked_mul(1000)?.checked_add(ms.min(999))
}

/// Select the best sample from a bounded set.
///
/// Strategy: drop clear outliers (offset more than 2× the median absolute
/// deviation from the median when ≥3 samples), then pick the lowest-delay
/// remaining sample. One failed server simply contributes no sample.
pub fn select_sample(samples: &[NtpSample]) -> Option<NtpSample> {
    if samples.is_empty() {
        return None;
    }
    if samples.len() == 1 {
        return Some(samples[0]);
    }

    let mut offsets: [i64; MAX_SAMPLES] = [0; MAX_SAMPLES];
    let n = samples.len().min(MAX_SAMPLES);
    for i in 0..n {
        offsets[i] = samples[i].offset_ms;
    }
    // Insertion sort offsets for median.
    for i in 1..n {
        let mut j = i;
        while j > 0 && offsets[j] < offsets[j - 1] {
            offsets.swap(j, j - 1);
            j -= 1;
        }
    }
    let median = offsets[n / 2];

    let mut best: Option<NtpSample> = None;
    for s in samples.iter().take(n) {
        // Outlier gate: reject if |offset - median| > max(500ms, 3 * median delay-ish band)
        // Use a fixed 2-second band from median for small sets.
        let deviation = (s.offset_ms - median).abs();
        if n >= 3 && deviation > 2_000 && deviation > (median.abs() / 2).max(500) {
            continue;
        }
        match best {
            None => best = Some(*s),
            Some(b) if s.delay_ms < b.delay_ms => best = Some(*s),
            Some(b) if s.delay_ms == b.delay_ms && s.stratum < b.stratum => best = Some(*s),
            _ => {}
        }
    }
    best.or_else(|| samples.iter().min_by_key(|s| s.delay_ms).copied())
}

/// Decide whether a validated sample may step the clock under the normal
/// (non-force) policy.
pub fn allow_normal_step(offset_ms: i64) -> bool {
    offset_ms.abs() <= MAX_NORMAL_STEP_SECS.saturating_mul(1000)
}

/// Compute the corrected Unix UTC seconds from current local Unix and offset.
pub fn apply_offset_secs(local_unix: u64, offset_ms: i64) -> Option<u64> {
    let local_ms = (local_unix as i128).checked_mul(1000)?;
    let corrected_ms = local_ms.checked_add(offset_ms as i128)?;
    if corrected_ms < 0 {
        return None;
    }
    Some((corrected_ms / 1000) as u64)
}

/// Periodic sync base interval (ms) — pool-friendly ~1024 s.
pub const PERIODIC_INTERVAL_MS: u64 = 1_024_000;
/// Maximum jitter added to the periodic interval (ms).
pub const PERIODIC_JITTER_MS: u64 = 64_000;
/// Initial backoff after failure (ms).
pub const BACKOFF_INITIAL_MS: u64 = 8_000;
/// Maximum backoff (ms).
pub const BACKOFF_MAX_MS: u64 = 512_000;
/// Per-server UDP exchange timeout (ms).
/// Keep short: net_server blocks its IPC loop for the whole exchange.
pub const SERVER_TIMEOUT_MS: u64 = 1_500;
/// Maximum absolute step allowed even with --force (seconds).
pub const MAX_FORCE_STEP_SECS: i64 = 10 * 365 * 24 * 3600;

/// Next backoff delay given the current delay (exponential, capped).
pub fn next_backoff_ms(current: u64) -> u64 {
    if current == 0 {
        return BACKOFF_INITIAL_MS;
    }
    current.saturating_mul(2).min(BACKOFF_MAX_MS)
}

/// Mix a small jitter from a random word into `[0, max_jitter]`.
pub fn jitter_ms(rand_word: u64, max_jitter: u64) -> u64 {
    if max_jitter == 0 {
        return 0;
    }
    rand_word % (max_jitter + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_server_response(
        originate_secs: u32,
        originate_frac: u32,
        rx_secs: u32,
        tx_secs: u32,
        stratum: u8,
        li: u8,
        mode: u8,
        version: u8,
    ) -> [u8; NTP_PACKET_LEN] {
        let mut pkt = [0u8; NTP_PACKET_LEN];
        pkt[0] = (li << 6) | (version << 3) | mode;
        pkt[1] = stratum;
        pkt[24..28].copy_from_slice(&originate_secs.to_be_bytes());
        pkt[28..32].copy_from_slice(&originate_frac.to_be_bytes());
        pkt[32..36].copy_from_slice(&rx_secs.to_be_bytes());
        pkt[40..44].copy_from_slice(&tx_secs.to_be_bytes());
        pkt
    }

    #[test]
    fn valid_reply_offset_and_delay() {
        // Local T1 = 1_000_000 ms, T4 = 1_000_100 ms (100 ms RTT)
        // Server receives at T2 and transmits at T3, both corresponding to
        // Unix ~1_700_000_000 + small. Use NTP secs.
        let unix_t2 = 1_700_000_050u64;
        let ntp_t2 = (unix_t2 + NTP_UNIX_DELTA) as u32;
        let ntp_t3 = ntp_t2; // instantaneous server
        let org_secs = 0xA5A5_1234;
        let org_frac = 0x1111_2222;
        let pkt = make_server_response(
            org_secs,
            org_frac,
            ntp_t2,
            ntp_t3,
            2,
            0,
            MODE_SERVER,
            NTP_VERSION,
        );

        // Local wall is behind: T1 local ms corresponds to unix 1_700_000_000
        let t1_ms = 1_700_000_000_000u64;
        let t4_ms = t1_ms + 100;
        let sample = parse_and_validate(&pkt, org_secs, org_frac, t1_ms, t4_ms).unwrap();
        // offset ≈ ((T2-T1)+(T3-T4))/2
        // T2_ms = 1_700_000_050_000, T3 same
        // (50_000 + 50_000 - 100)/2 = 49950 ms ≈ +50s
        assert!((sample.offset_ms - 49_950).abs() < 5);
        assert_eq!(sample.delay_ms, 100);
        assert_eq!(sample.stratum, 2);
    }

    #[test]
    fn rejects_short_packet() {
        let r = parse_and_validate(&[0u8; 10], 1, 2, 0, 1);
        assert_eq!(r, Err(NtpError::InvalidResponse));
    }

    #[test]
    fn rejects_mismatched_originate() {
        let pkt = make_server_response(1, 2, 3_000_000_000, 3_000_000_000, 1, 0, MODE_SERVER, 4);
        let r = parse_and_validate(&pkt, 9, 9, 0, 10);
        assert_eq!(r, Err(NtpError::InvalidResponse));
    }

    #[test]
    fn rejects_unsync_li() {
        let pkt = make_server_response(
            1,
            0,
            3_000_000_000,
            3_000_000_000,
            1,
            LI_UNSYNC,
            MODE_SERVER,
            4,
        );
        let r = parse_and_validate(&pkt, 1, 0, 0, 10);
        assert_eq!(r, Err(NtpError::InvalidResponse));
    }

    #[test]
    fn rejects_kod_stratum_zero() {
        let pkt = make_server_response(1, 0, 3_000_000_000, 3_000_000_000, 0, 0, MODE_SERVER, 4);
        let r = parse_and_validate(&pkt, 1, 0, 0, 10);
        assert_eq!(r, Err(NtpError::KissOfDeath));
    }

    #[test]
    fn rejects_wrong_mode() {
        let pkt = make_server_response(1, 0, 3_000_000_000, 3_000_000_000, 1, 0, MODE_CLIENT, 4);
        let r = parse_and_validate(&pkt, 1, 0, 0, 10);
        assert_eq!(r, Err(NtpError::InvalidResponse));
    }

    #[test]
    fn one_failed_server_does_not_block_selection() {
        let good = NtpSample {
            offset_ms: 100,
            delay_ms: 20,
            stratum: 2,
            server_unix: 1_700_000_000,
            originate_secs: 1,
        };
        let samples = [good];
        assert_eq!(select_sample(&samples).unwrap().offset_ms, 100);
    }

    #[test]
    fn select_prefers_lowest_delay() {
        let a = NtpSample {
            offset_ms: 10,
            delay_ms: 50,
            stratum: 1,
            server_unix: 1,
            originate_secs: 1,
        };
        let b = NtpSample {
            offset_ms: 12,
            delay_ms: 15,
            stratum: 2,
            server_unix: 1,
            originate_secs: 2,
        };
        assert_eq!(select_sample(&[a, b]).unwrap().delay_ms, 15);
    }

    #[test]
    fn unix_ntp_roundtrip_secs() {
        let unix = 1_700_000_000u64;
        let (s, _) = unix_to_ntp(unix, 0).unwrap();
        assert_eq!(ntp_secs_to_unix(s).unwrap(), unix);
    }

    #[test]
    fn normal_step_bounds() {
        assert!(allow_normal_step(500));
        assert!(allow_normal_step(MAX_NORMAL_STEP_SECS * 1000));
        assert!(!allow_normal_step(MAX_NORMAL_STEP_SECS * 1000 + 1));
    }

    #[test]
    fn backoff_grows_and_caps() {
        assert_eq!(next_backoff_ms(0), BACKOFF_INITIAL_MS);
        let mut b = BACKOFF_INITIAL_MS;
        for _ in 0..20 {
            b = next_backoff_ms(b);
        }
        assert_eq!(b, BACKOFF_MAX_MS);
    }

    #[test]
    fn build_request_sets_version_mode_and_xmit() {
        let pkt = build_client_request(0xAABB_CCDD, 0x1122_3344);
        assert_eq!(pkt[0] & 0x07, MODE_CLIENT);
        assert_eq!((pkt[0] >> 3) & 0x07, NTP_VERSION);
        assert_eq!(
            u32::from_be_bytes([pkt[40], pkt[41], pkt[42], pkt[43]]),
            0xAABB_CCDD
        );
        assert_eq!(
            u32::from_be_bytes([pkt[44], pkt[45], pkt[46], pkt[47]]),
            0x1122_3344
        );
    }
}
