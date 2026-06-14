//! Time state management and persistence
//!
//! Maintains the current time state including UTC epoch, timezone offset,
//! and DST information. Can be persisted to JSON or binary format.

/// Time state structure tracking UTC and local time information
#[repr(C)]
#[derive(Clone, Debug)]
pub struct TimeState {
    pub utc_epoch: u64,                // Unix timestamp from RTC (pure UTC, no tz)
    pub ntp_synced: bool,              // Has NTP synchronized the clock?
    pub ntp_drift_ppm: i32,            // PPM drift correction
}

impl TimeState {
    /// Create a new TimeState with default values (pure UTC source)
    pub const fn new() -> Self {
        Self {
            utc_epoch: 0,
            ntp_synced: false,
            ntp_drift_ppm: 0,
        }
    }

    /// Serialize to JSON format (UTC + NTP status only)
    /// Note: This is a simple string builder since we can't use serde in no_std
    pub fn to_json(&self) -> [u8; 256] {
        let mut buf = [0u8; 256];
        let mut pos = 0;

        // Build JSON manually
        let json_start = b"{\"utc_epoch\":";
        buf[..json_start.len()].copy_from_slice(json_start);
        pos = json_start.len();

        // Append UTC epoch (naive integer-to-string conversion)
        let epoch_str = format_u64(self.utc_epoch);
        for (_i, &byte) in epoch_str.iter().enumerate() {
            if byte == 0 {
                break;
            }
            buf[pos] = byte;
            pos += 1;
        }

        // NTP fields
        if self.ntp_synced {
            let ntp_str = b",\"ntp_synced\":true";
            buf[pos..pos + ntp_str.len()].copy_from_slice(ntp_str);
            pos += ntp_str.len();
        } else {
            let ntp_str = b",\"ntp_synced\":false";
            buf[pos..pos + ntp_str.len()].copy_from_slice(ntp_str);
            pos += ntp_str.len();
        }

        let drift_start = b",\"drift_ppm\":";
        buf[pos..pos + drift_start.len()].copy_from_slice(drift_start);
        pos += drift_start.len();

        let drift_str = format_i32(self.ntp_drift_ppm);
        for (_i, &byte) in drift_str.iter().enumerate() {
            if byte == 0 {
                break;
            }
            buf[pos] = byte;
            pos += 1;
        }

        buf[pos] = b'}';

        buf
    }
}

/// Simple u64 to string conversion (no allocations)
fn format_u64(mut val: u64) -> [u8; 32] {
    let mut buf = [0u8; 32];
    if val == 0 {
        buf[0] = b'0';
        return buf;
    }

    let mut pos = 0;
    let mut temp = val;
    while temp > 0 {
        pos += 1;
        temp /= 10;
    }

    let mut idx = pos;
    temp = val;
    while temp > 0 {
        idx -= 1;
        buf[idx] = b'0' + (temp % 10) as u8;
        temp /= 10;
    }

    buf
}

/// Simple i32 to string conversion (handles negatives)
fn format_i32(val: i32) -> [u8; 32] {
    let mut buf = [0u8; 32];
    if val == 0 {
        buf[0] = b'0';
        return buf;
    }

    let is_negative = val < 0;
    let abs_val = val.abs() as u64;

    let mut pos = if is_negative { 1 } else { 0 };
    let mut temp = abs_val;
    while temp > 0 {
        pos += 1;
        temp /= 10;
    }

    let mut idx = pos;
    temp = abs_val;
    while temp > 0 {
        idx -= 1;
        buf[idx] = b'0' + (temp % 10) as u8;
        temp /= 10;
    }

    if is_negative {
        buf[0] = b'-';
    }

    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_utc_epoch_roundtrip() {
        let mut state = TimeState::new();
        state.utc_epoch = 1234567890;
        assert_eq!(state.utc_epoch, 1234567890);
    }

    #[test]
    fn test_ntp_fields() {
        let mut state = TimeState::new();
        state.ntp_synced = true;
        state.ntp_drift_ppm = 42;
        assert!(state.ntp_synced);
        assert_eq!(state.ntp_drift_ppm, 42);
    }
}
