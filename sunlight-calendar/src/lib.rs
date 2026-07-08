#![cfg_attr(not(test), no_std)]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use core::convert::From;
use core::iter::Iterator;
use core::option::Option::{self, None, Some};

pub const CAL_EVENT_PREFIX: &str = "app.calendar.events/";
pub const CAL_INDEX_ALL: &str = "app.calendar.index/all";
pub const CAL_INDEX_BY_DATE_PREFIX: &str = "app.calendar.index/by-date/";
pub const CAL_SETTINGS_PREFIX: &str = "app.calendar.settings/";
pub const CAL_MIGRATION_FILE_V1: &str = "file-v1-imported";

pub fn event_key(event_id: u64) -> String {
    let mut out = String::from(CAL_EVENT_PREFIX);
    push_u64_hex(&mut out, event_id);
    out
}

pub fn by_date_key(date: &str) -> String {
    let mut out = String::from(CAL_INDEX_BY_DATE_PREFIX);
    out.push_str(date);
    out
}

pub fn setting_key(key: &str) -> String {
    let mut out = String::from(CAL_SETTINGS_PREFIX);
    out.push_str(key);
    out
}

pub fn encode_id_list(ids: &[u64]) -> Vec<u8> {
    let mut out = String::new();
    for id in ids {
        if !out.is_empty() {
            out.push('\n');
        }
        push_u64_decimal(&mut out, *id);
    }
    out.into_bytes()
}

pub fn parse_id_list(bytes: &[u8]) -> Option<Vec<u64>> {
    let text = core::str::from_utf8(bytes).ok()?;
    let mut ids = Vec::new();
    for line in text.lines() {
        if line.is_empty() {
            continue;
        }
        if let Some(id) = parse_u64(line) {
            if !ids.iter().any(|existing| *existing == id) {
                ids.push(id);
            }
        }
    }
    Some(ids)
}

pub fn valid_date_str(s: &str) -> bool {
    parse_date_parts(s).is_some()
}

pub fn format_date(year: i32, month: i32, day: i32) -> String {
    let mut out = String::new();
    push_i32_fixed(&mut out, year, 4);
    out.push('-');
    push_i32_fixed(&mut out, month, 2);
    out.push('-');
    push_i32_fixed(&mut out, day, 2);
    out
}

fn parse_date_parts(s: &str) -> Option<(i32, i32, i32)> {
    let first_sep = s.find(|ch| ch == '-' || ch == '/')?;
    let rest = &s[first_sep + 1..];
    let second_rel = rest.find(|ch| ch == '-' || ch == '/')?;
    let second_sep = first_sep + 1 + second_rel;
    let year_s = &s[..first_sep];
    let month_s = &s[first_sep + 1..second_sep];
    let day_s = &s[second_sep + 1..];
    if year_s.len() != 4
        || month_s.is_empty()
        || month_s.len() > 2
        || day_s.is_empty()
        || day_s.len() > 2
    {
        return None;
    }
    for part in [year_s, month_s, day_s] {
        if !part.as_bytes().iter().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
    }
    let year = parse_i32(year_s);
    let month = parse_i32(month_s);
    let day = parse_i32(day_s);
    if month >= 1 && month <= 12 && day >= 1 && day <= days_in_month(year, month) {
        Some((year, month, day))
    } else {
        None
    }
}

pub fn month_grid_days(year: i32, month: i32) -> [i32; 42] {
    let mut days = [0i32; 42];
    let total = days_in_month(year, month);
    let start_wday = weekday_mon0(year, month, 1);
    for i in 0..total {
        days[(start_wday + i) as usize % 42] = i + 1;
    }
    days
}

pub fn days_in_month(year: i32, month: i32) -> i32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap_year(year) {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

pub fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

pub fn weekday_mon0(year: i32, month: i32, day: i32) -> i32 {
    let t = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = if month < 3 { year - 1 } else { year };
    ((y + y / 4 - y / 100 + y / 400 + t[(month - 1) as usize] + day) % 7 + 6) % 7
}

pub fn parse_i32(s: &str) -> i32 {
    let mut n = 0i32;
    for c in s.chars() {
        if c.is_ascii_digit() {
            n = n * 10 + (c as i32 - '0' as i32);
        }
    }
    n
}

pub fn parse_u64(s: &str) -> Option<u64> {
    let mut n = 0u64;
    let mut seen = false;
    for c in s.chars() {
        if !c.is_ascii_digit() {
            return None;
        }
        seen = true;
        n = n
            .saturating_mul(10)
            .saturating_add((c as u64).saturating_sub('0' as u64));
    }
    if seen {
        Some(n)
    } else {
        None
    }
}

pub fn push_u64_decimal(out: &mut String, mut n: u64) {
    if n == 0 {
        out.push('0');
        return;
    }
    let mut buf = [0u8; 20];
    let mut i = buf.len();
    while n > 0 {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    for &b in &buf[i..] {
        out.push(b as char);
    }
}

pub fn push_u64_hex(out: &mut String, mut n: u64) {
    let mut buf = [0u8; 16];
    for i in (0..16).rev() {
        let digit = (n & 0xF) as u8;
        buf[i] = if digit < 10 {
            b'0' + digit
        } else {
            b'a' + digit - 10
        };
        n >>= 4;
    }
    for &b in &buf {
        out.push(b as char);
    }
}

fn push_i32_fixed(out: &mut String, mut n: i32, digits: usize) {
    if n < 0 {
        out.push('-');
        n = -n;
    }
    let mut buf = [0u8; 12];
    let mut i = buf.len();
    while n > 0 {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    let len = buf.len() - i;
    for _ in len..digits {
        out.push('0');
    }
    if len == 0 {
        if digits == 0 {
            out.push('0');
        }
        return;
    }
    for &b in &buf[i..] {
        out.push(b as char);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn stable_namespace_keys_are_readable() {
        assert_eq!(event_key(42), "app.calendar.events/000000000000002a");
        assert_eq!(
            by_date_key("2026-07-08"),
            "app.calendar.index/by-date/2026-07-08"
        );
        assert_eq!(
            setting_key("selected-date"),
            "app.calendar.settings/selected-date"
        );
    }

    #[test]
    fn id_index_round_trips_and_ignores_malformed_lines() {
        let encoded = encode_id_list(&[7, 9, 7]);
        assert_eq!(core::str::from_utf8(&encoded).unwrap(), "7\n9\n7");
        assert_eq!(parse_id_list(b"7\nbad\n9\n7\n").unwrap(), vec![7, 9]);
    }

    #[test]
    fn date_validation_is_explicit() {
        assert!(valid_date_str("2026-07-08"));
        assert!(valid_date_str("2026/10/6"));
        assert!(valid_date_str("2024-02-29"));
        assert!(!valid_date_str("2026-07-32"));
        assert!(!valid_date_str("2023-02-29"));
        assert!(!valid_date_str("08-07-2026"));
    }

    #[test]
    fn date_format_zero_pads_before_digits() {
        assert_eq!(format_date(2026, 7, 8), "2026-07-08");
        assert_eq!(format_date(2026, 10, 6), "2026-10-06");
    }

    #[test]
    fn july_2026_month_grid_starts_on_wednesday_without_timezone_shift() {
        let days = month_grid_days(2026, 7);
        assert_eq!(weekday_mon0(2026, 7, 1), 2);
        assert_eq!(days[2], 1);
        assert_eq!(days[9], 8);
        assert_eq!(days[32], 31);
    }
}
