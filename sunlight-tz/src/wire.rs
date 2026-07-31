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

pub fn encode_local_time(local: &LocalDateTime, weekday_iso: u8) -> [u64; 5] {
    let mut civil = (local.year as u64) << 48;
    civil |= (local.month as u64) << 40;
    civil |= (local.day as u64) << 32;
    civil |= (local.hour as u64) << 24;
    civil |= (local.minute as u64) << 16;
    civil |= (local.second as u64) << 8;

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
    if words.len() < 5 {
        return None;
    }
    let civil = words[0];
    let weekday_iso = words[4] as u8;
    if !(1..=7).contains(&weekday_iso) {
        return None;
    }
    let mut abbr = [0u8; 8];
    for (index, byte) in abbr.iter_mut().enumerate() {
        *byte = ((words[3] >> (index * 8)) & 0xff) as u8;
    }
    Some(LocalTimeWireSnapshot {
        year: ((civil >> 48) & 0xffff) as u16,
        month: ((civil >> 40) & 0xff) as u8,
        day: ((civil >> 32) & 0xff) as u8,
        hour: ((civil >> 24) & 0xff) as u8,
        minute: ((civil >> 16) & 0xff) as u8,
        second: ((civil >> 8) & 0xff) as u8,
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
    fn rejects_missing_or_invalid_weekday() {
        assert!(decode_local_time(&[0; 4]).is_none());
        assert!(decode_local_time(&[0; 5]).is_none());
    }
}
