//! Shared `GET_LOCAL_TIME` wire encoding.

use crate::LocalDateTime;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalTimeWireSnapshot {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    pub weekday_iso: u8,
    pub utc_offset_secs: i64,
    pub is_dst: bool,
    pub abbr: [u8; 8],
}

/// Validate the complete local-civil payload carried by `GET_LOCAL_TIME`.
///
/// The wire format has no sentinel values: an out-of-range field makes the
/// whole snapshot unavailable to the consumer.
pub fn is_valid_civil_time(
    year: u16,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
    second: u8,
    weekday_iso: u8,
) -> bool {
    if year < 1970
        || !(1..=12).contains(&month)
        || hour > 23
        || minute > 59
        || second > 59
        || !(1..=7).contains(&weekday_iso)
    {
        return false;
    }
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days_in_month = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=days_in_month).contains(&day)
}

pub fn encode_local_time(local: &LocalDateTime, weekday_iso: u8) -> [u64; 5] {
    let mut civil = (local.year as u64) << 48;
    civil |= (local.month as u64) << 40;
    civil |= (local.day as u64) << 32;
    civil |= (local.hour as u64) << 24;
    civil |= (local.minute as u64) << 16;
    civil |= (local.second as u64) << 8;
    // Register IPC carries only words 0..=3. Keep weekday in the previously
    // unused low byte of the civil word so the snapshot remains atomic.
    civil |= weekday_iso as u64;

    let mut abbreviation = 0u64;
    for (index, byte) in local.abbr.iter().enumerate() {
        abbreviation |= (*byte as u64) << (index * 8);
    }

    [
        civil,
        local.utc_offset_secs as u64,
        local.is_dst as u64,
        abbreviation,
        weekday_iso as u64,
    ]
}

pub fn decode_local_time(words: &[u64]) -> Option<LocalTimeWireSnapshot> {
    if words.len() < 4 {
        return None;
    }
    let civil = words[0];
    let year = ((civil >> 48) & 0xffff) as u16;
    let month = ((civil >> 40) & 0xff) as u8;
    let day = ((civil >> 32) & 0xff) as u8;
    let hour = ((civil >> 24) & 0xff) as u8;
    let minute = ((civil >> 16) & 0xff) as u8;
    let second = ((civil >> 8) & 0xff) as u8;
    let packed_weekday = (civil & 0xff) as u8;
    // Accept the old logical five-word representation for host-side/backward
    // compatibility, but prefer the transport-safe packed field.
    let weekday_iso = if packed_weekday != 0 {
        packed_weekday
    } else {
        words.get(4).copied().unwrap_or(0) as u8
    };
    if !is_valid_civil_time(year, month, day, hour, minute, second, weekday_iso) {
        return None;
    }
    let mut abbr = [0u8; 8];
    for (index, byte) in abbr.iter_mut().enumerate() {
        *byte = ((words[3] >> (index * 8)) & 0xff) as u8;
    }
    Some(LocalTimeWireSnapshot {
        year,
        month,
        day,
        hour,
        minute,
        second,
        weekday_iso,
        utc_offset_secs: words[1] as i64,
        is_dst: words[2] != 0,
        abbr,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_time_wire_round_trips_civil_time_and_weekday() {
        let local = LocalDateTime {
            year: 2026,
            month: 7,
            day: 31,
            hour: 23,
            minute: 59,
            second: 59,
            utc_offset_secs: 12_600,
            is_dst: false,
            abbr: *b"U+0330\0\0",
        };
        let words = encode_local_time(&local, 5);
        let decoded = decode_local_time(&words).unwrap();
        assert_eq!(
            (
                decoded.year,
                decoded.month,
                decoded.day,
                decoded.hour,
                decoded.minute,
                decoded.second,
                decoded.weekday_iso,
            ),
            (2026, 7, 31, 23, 59, 59, 5)
        );
        assert_eq!(decoded.utc_offset_secs, 12_600);
        assert_eq!(decoded.abbr, *b"U+0330\0\0");
    }

    #[test]
    fn weekday_survives_the_four_register_ipc_boundary() {
        let local = LocalDateTime {
            year: 2026,
            month: 8,
            day: 15,
            hour: 14,
            minute: 10,
            second: 28,
            utc_offset_secs: 0,
            is_dst: false,
            abbr: *b"UTC\0\0\0\0\0",
        };
        let words = encode_local_time(&local, 6);
        let decoded = decode_local_time(&words[..4]).unwrap();
        assert_eq!(decoded.weekday_iso, 6);
        assert_eq!(decoded.minute, 10);
    }

    #[test]
    fn rejects_missing_or_invalid_civil_time() {
        assert!(decode_local_time(&[0; 3]).is_none());
        assert!(decode_local_time(&[0; 4]).is_none());

        let valid = LocalDateTime {
            year: 2026,
            month: 8,
            day: 15,
            hour: 15,
            minute: 25,
            second: 30,
            utc_offset_secs: 0,
            is_dst: false,
            abbr: *b"UTC\0\0\0\0\0",
        };
        let mut words = encode_local_time(&valid, 6);
        words[0] = (words[0] & !(0xff << 16)) | (60 << 16);
        assert!(decode_local_time(&words).is_none());
        words[0] = (words[0] & !(0xff << 16)) | (255 << 16);
        assert!(decode_local_time(&words).is_none());
    }

    #[test]
    fn validates_calendar_boundaries() {
        assert!(is_valid_civil_time(2024, 2, 29, 23, 59, 59, 4));
        assert!(!is_valid_civil_time(2025, 2, 29, 0, 0, 0, 6));
        assert!(!is_valid_civil_time(2026, 4, 31, 0, 0, 0, 5));
    }
}
