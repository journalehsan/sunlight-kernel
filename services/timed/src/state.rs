//! Time state management and persistence
//!
//! Maintains the current time state including UTC epoch, timezone offset,
//! and DST information. Can be persisted to JSON or binary format.

/// Time state structure tracking UTC and local time information
#[repr(C)]
#[derive(Clone, Debug)]
pub struct TimeState {
    pub utc_epoch: u64,                // Unix timestamp from RTC
    pub local_offset_secs: i32,        // Offset from UTC in seconds
    pub dst_active: bool,              // Is DST currently active
    pub timezone_name: [u8; 64],       // Timezone name (e.g., "Asia/Tehran")
    pub timezone_name_len: usize,      // Length of timezone name
    pub ntp_synced: bool,              // Has NTP synchronized the clock?
    pub ntp_drift_ppm: i32,            // PPM drift correction
}

impl TimeState {
    /// Create a new TimeState with default values
    pub const fn new() -> Self {
        Self {
            utc_epoch: 0,
            local_offset_secs: 0,
            dst_active: false,
            timezone_name: [0u8; 64],
            timezone_name_len: 0,
            ntp_synced: false,
            ntp_drift_ppm: 0,
        }
    }

    /// Set timezone name from a string slice
    pub fn set_timezone_name(&mut self, name: &str) {
        let bytes = name.as_bytes();
        let len = core::cmp::min(bytes.len(), 63);
        self.timezone_name[..len].copy_from_slice(&bytes[..len]);
        self.timezone_name[len] = 0; // Null terminator
        self.timezone_name_len = len;
    }

    /// Get timezone name as string slice
    pub fn get_timezone_name(&self) -> &str {
        match core::str::from_utf8(&self.timezone_name[..self.timezone_name_len]) {
            Ok(s) => s,
            Err(_) => "Unknown",
        }
    }

    /// Calculate local time from UTC epoch
    pub fn local_time(&self) -> u64 {
        (self.utc_epoch as i64 + self.local_offset_secs as i64) as u64
    }

    /// Calculate total offset including DST
    pub fn total_offset_secs(&self) -> i32 {
        if self.dst_active {
            self.local_offset_secs + 3600
        } else {
            self.local_offset_secs
        }
    }

    /// Serialize to JSON format
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

        // Append rest of JSON structure
        let json_rest = b",\"offset_secs\":";
        buf[pos..pos + json_rest.len()].copy_from_slice(json_rest);
        pos += json_rest.len();

        // Offset seconds
        let offset_str = format_i32(self.local_offset_secs);
        for (_i, &byte) in offset_str.iter().enumerate() {
            if byte == 0 {
                break;
            }
            buf[pos] = byte;
            pos += 1;
        }

        // DST flag
        let dst_str: &[u8] = if self.dst_active { b",\"dst\":true" } else { b",\"dst\":false" };
        buf[pos..pos + dst_str.len()].copy_from_slice(dst_str);
        pos += dst_str.len();

        // Timezone name (already null-terminated)
        let tz_str = b",\"timezone\":\"";
        buf[pos..pos + tz_str.len()].copy_from_slice(tz_str);
        pos += tz_str.len();
        buf[pos..pos + self.timezone_name_len].copy_from_slice(&self.timezone_name[..self.timezone_name_len]);
        pos += self.timezone_name_len;
        buf[pos] = b'"';
        pos += 1;
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
    fn test_local_time() {
        let mut state = TimeState::new();
        state.utc_epoch = 1000;
        state.local_offset_secs = 3600; // +1 hour
        assert_eq!(state.local_time(), 4600);
    }

    #[test]
    fn test_total_offset_with_dst() {
        let mut state = TimeState::new();
        state.local_offset_secs = 3600;
        state.dst_active = false;
        assert_eq!(state.total_offset_secs(), 3600);

        state.dst_active = true;
        assert_eq!(state.total_offset_secs(), 7200);
    }

    #[test]
    fn test_timezone_name() {
        let mut state = TimeState::new();
        state.set_timezone_name("Asia/Tehran");
        assert_eq!(state.get_timezone_name(), "Asia/Tehran");
    }
}
