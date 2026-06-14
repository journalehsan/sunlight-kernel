//! Pure integer UTC offset, DST, and civil calendar math.
//! No floats, no alloc. For use in no_std services and tests.

use crate::csv::TzEntry;

/// Total UTC offset in seconds, including DST if currently active.
/// utc_secs: seconds since QEMU RTC epoch (2000-01-01 00:00:00 UTC).
pub fn local_offset_secs(entry: &TzEntry, utc_secs: u64) -> i64 {
    let base: i64 = (entry.utc_offset_hours as i64) * 3600
                  + (entry.utc_offset_minutes as i64) * 60;
    let dst: i64 = if is_dst_active(entry, utc_secs) {
        (entry.dst_offset_minutes as i64) * 60
    } else { 0 };
    base + dst
}

/// Month-range DST approximation (1-12).
/// Handles both northern hemisphere (start < end) and
/// southern hemisphere (start > end, e.g. Oct–Apr).
pub fn is_dst_active(entry: &TzEntry, utc_secs: u64) -> bool {
    if entry.dst_start_month == 0 { return false; }
    let month = month_from_secs(utc_secs);
    if entry.dst_start_month <= entry.dst_end_month {
        // Northern hemisphere: DST in spring/summer
        month >= entry.dst_start_month && month <= entry.dst_end_month
    } else {
        // Southern hemisphere: DST spans year-end
        month >= entry.dst_start_month || month <= entry.dst_end_month
    }
}

/// Extract month (1-12) from utc_secs using the integer calendar.
fn month_from_secs(utc_secs: u64) -> u8 {
    let (_y, m, _d, _h, _mi, _s) = decompose(utc_secs);
    m
}

/// Decompose seconds since 2000-01-01 UTC into (year, month, day, hour, min, sec).
/// Uses Howard Hinnant's public-domain civil_from_days algorithm, adjusted for
/// 2000-01-01 epoch (10957 days after 1970-01-01).
pub fn decompose(utc_secs: u64) -> (u16, u8, u8, u8, u8, u8) {
    let days: u64 = utc_secs / 86400;
    let secs_in_day: u64 = utc_secs % 86400;

    let hour: u8 = (secs_in_day / 3600) as u8;
    let min:  u8 = ((secs_in_day % 3600) / 60) as u8;
    let sec:  u8 = (secs_in_day % 60) as u8;

    // Epoch adjustment: our days are since 2000-01-01, Hinnant is since 1970-01-01.
    // 2000-01-01 is day 10957 after 1970-01-01.
    let unix_days: i64 = (days as i64) + 10957;

    let (y, m, d) = civil_from_days(unix_days);

    (y as u16, m, d, hour, min, sec)
}

/// Howard Hinnant civil_from_days (adapted, integer only).
/// Returns (year, month 1-12, day 1-31) for given days since 1970-01-01 (signed ok).
fn civil_from_days(mut z: i64) -> (i32, u8, u8) {
    // z is unix_days = days since 1970
    z += 719468; // shift to internal era
    let era: i64 = if z >= 0 { z / 146097 } else { (z - 146096) / 146097 }; // floor div for neg
    let doe: i64 = z - era * 146097;                 // [0, 146096]
    let yoe: i64 = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let mut y: i32 = (yoe + era * 400) as i32;
    let doy: i64 = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp: i64 = (5 * doy + 2) / 153;                        // [0, 11]
    let d: i32 = (doy - (153 * mp + 2) / 5 + 1) as i32;
    let m: i32 = (mp + if mp < 10 { 3 } else { -9 }) as i32;
    if m <= 2 {
        y += 1;
    }
    (y, m as u8, d as u8)
}

/// Local date/time with offset and abbreviation info.
#[derive(Clone, Copy, Debug)]
pub struct LocalDateTime {
    pub year:   u16,
    pub month:  u8,    // 1-12
    pub day:    u8,    // 1-31
    pub hour:   u8,    // 0-23
    pub minute: u8,    // 0-59
    pub second: u8,    // 0-59
    pub utc_offset_secs: i64,   // total offset including DST
    pub is_dst:          bool,
    pub abbr: [u8; 8],          // timezone abbreviation, null-terminated
}

impl LocalDateTime {
    /// Format as "HH:MM:SS" (local time). Fixed 8-byte buffer, NUL padded if needed.
    pub fn fmt_time(&self, buf: &mut [u8; 8]) {
        buf[0] = b'0' + self.hour / 10;
        buf[1] = b'0' + self.hour % 10;
        buf[2] = b':';
        buf[3] = b'0' + self.minute / 10;
        buf[4] = b'0' + self.minute % 10;
        buf[5] = b':';
        buf[6] = b'0' + self.second / 10;
        buf[7] = b'0' + self.second % 10;
    }

    /// Format as "YYYY/MM/DD" into 10-byte buffer.
    pub fn fmt_date(&self, buf: &mut [u8; 10]) {
        write_u16_4(&mut buf[0..4], self.year);
        buf[4] = b'/';
        buf[5] = b'0' + self.month / 10;
        buf[6] = b'0' + self.month % 10;
        buf[7] = b'/';
        buf[8] = b'0' + self.day / 10;
        buf[9] = b'0' + self.day % 10;
    }

    /// Format as "YYYY-MM-DDTHH:MM:SS+HH:MM" (ISO 8601 with offset) into 32-byte buf.
    pub fn fmt_iso8601(&self, buf: &mut [u8; 32]) {
        // date part
        write_u16_4(&mut buf[0..4], self.year);
        buf[4] = b'-';
        buf[5] = b'0' + self.month / 10; buf[6] = b'0' + self.month % 10;
        buf[7] = b'-';
        buf[8] = b'0' + self.day / 10; buf[9] = b'0' + self.day % 10;
        buf[10] = b'T';
        // time
        buf[11] = b'0' + self.hour / 10; buf[12] = b'0' + self.hour % 10;
        buf[13] = b':';
        buf[14] = b'0' + self.minute / 10; buf[15] = b'0' + self.minute % 10;
        buf[16] = b':';
        buf[17] = b'0' + self.second / 10; buf[18] = b'0' + self.second % 10;
        // offset sign + HH:MM
        let off = self.utc_offset_secs;
        let sign = if off < 0 { b'-' } else { b'+' };
        let off_abs = if off < 0 { -off } else { off };
        let oh = (off_abs / 3600) as u8;
        let om = ((off_abs % 3600) / 60) as u8;
        buf[19] = sign;
        buf[20] = b'0' + oh / 10; buf[21] = b'0' + oh % 10;
        buf[22] = b':';
        buf[23] = b'0' + om / 10; buf[24] = b'0' + om % 10;
        // zero the rest
        for i in 25..32 { buf[i] = 0; }
    }

    /// Format as "Day Mon DD HH:MM:SS TZ YYYY" (date(1) style) into 40-byte buf.
    pub fn fmt_date_cmd(&self, buf: &mut [u8; 40]) {
        // Weekday approx (simple Doomsday rule or fixed table not full; use fixed names, compute wday minimally)
        let wday = weekday_from_ymd(self.year as i32, self.month, self.day);
        let wname = match wday {
            0 => b"Sun", 1 => b"Mon", 2 => b"Tue", 3 => b"Wed",
            4 => b"Thu", 5 => b"Fri", _ => b"Sat",
        };
        buf[0] = wname[0]; buf[1]=wname[1]; buf[2]=wname[2]; buf[3]=b' ';

        let mname = match self.month {
            1=>b"Jan",2=>b"Feb",3=>b"Mar",4=>b"Apr",5=>b"May",6=>b"Jun",
            7=>b"Jul",8=>b"Aug",9=>b"Sep",10=>b"Oct",11=>b"Nov",_=>b"Dec",
        };
        buf[4]=mname[0];buf[5]=mname[1];buf[6]=mname[2]; buf[7]=b' ';

        buf[8] = b'0' + self.day/10; buf[9] = b'0' + self.day%10; buf[10]=b' ';

        buf[11] = b'0' + self.hour/10; buf[12] = b'0' + self.hour%10; buf[13]=b':';
        buf[14] = b'0' + self.minute/10; buf[15] = b'0' + self.minute%10; buf[16]=b':';
        buf[17] = b'0' + self.second/10; buf[18] = b'0' + self.second%10; buf[19]=b' ';

        // TZ abbr (up to 8 but copy until nul or 3-4 chars)
        let mut p = 20usize;
        for i in 0..8 {
            if self.abbr[i] == 0 { break; }
            if p < 39 { buf[p] = self.abbr[i]; p += 1; }
        }
        buf[p] = b' '; p += 1;

        write_u16_4(&mut buf[p..p+4], self.year);
        p += 4;
        for i in p..40 { buf[i] = 0; }
    }
}

fn write_u16_4(dst: &mut [u8], v: u16) {
    dst[0] = b'0' + (v / 1000 % 10) as u8;
    dst[1] = b'0' + (v / 100 % 10) as u8;
    dst[2] = b'0' + (v / 10 % 10) as u8;
    dst[3] = b'0' + (v % 10) as u8;
}

/// Very small weekday calculator (0=Sun .. 6=Sat). Zeller-ish for proleptic Gregorian.
fn weekday_from_ymd(y: i32, m: u8, d: u8) -> u8 {
    let mut yy = y;
    let mut mm = m as i32;
    if mm <= 2 { yy -= 1; mm += 12; }
    let c = yy / 100;
    let k = yy % 100;
    let w = (d as i32 + (13 * (mm + 1) / 5) + k + (k / 4) + (c / 4) + 5 * c) % 7;
    // adjust to Sun=0
    ((w + 6) % 7) as u8  // trial; sufficient for formatting
}

/// Derive 8-byte null-terminated abbreviation.
/// UTC special case, first-letter words, S->D for DST, ambig fallback to UTC offset form.
fn derive_abbr(entry: &TzEntry, in_dst: bool) -> [u8; 8] {
    if entry.id == "UTC" || (entry.utc_offset_hours == 0 && entry.utc_offset_minutes == 0 && entry.display_name == "Coordinated Universal Time") {
        return *b"UTC\0\0\0\0\0";
    }
    // Build from display name first letters
    let mut abbr = [0u8; 8];
    let mut pos = 0usize;
    let bytes = entry.display_name.as_bytes();
    let mut prev_sep = true;
    for &b in bytes.iter() {
        if pos >= 7 { break; }
        if b == b' ' || b == b'-' || b == b'/' || b == b'.' {
            prev_sep = true;
            continue;
        }
        if prev_sep && b.is_ascii_alphabetic() {
            let ch = if (b'A'..=b'Z').contains(&b) { b } else { b.to_ascii_uppercase() };
            abbr[pos] = ch;
            pos += 1;
            prev_sep = false;
        } else {
            prev_sep = false;
        }
    }

    if in_dst && pos > 0 {
        // Replace first 'S' (Standard -> Daylight) with 'D'
        for k in 0..pos {
            if abbr[k] == b'S' {
                abbr[k] = b'D';
                break;
            }
        }
    }

    // Ambiguous abbreviations (IST etc): use offset form "U+0330\0" style within 8 bytes
    let is_potentially_ambig = entry.display_name.contains("Iran") ||
                               entry.display_name.contains("India") ||
                               entry.display_name.contains("Israel") ||
                               (abbr[0] == b'I' && pos >= 3 && abbr[1] == b'S' && abbr[2] == b'T');
    if is_potentially_ambig {
        return make_offset_abbr(entry.utc_offset_hours, entry.utc_offset_minutes);
    }

    // null pad already zeroed
    abbr
}

fn make_offset_abbr(h: i8, m: u8) -> [u8; 8] {
    // "U+0330\0\0" or with sign;  fits in 8 incl nul: U + 0 3 3 0 \0 \0
    let mut b = [0u8; 8];
    b[0] = b'U';
    b[1] = if h < 0 { b'-' } else { b'+' };
    let ah = h.abs() as u8;
    b[2] = b'0' + (ah / 10);
    b[3] = b'0' + (ah % 10);
    b[4] = b'0' + (m / 10);
    b[5] = b'0' + (m % 10);
    // b[6],b[7] remain 0
    b
}

/// Public conversion: compute LocalDateTime for given UTC seconds and TzEntry.
pub fn local_now(utc_secs: u64, entry: &TzEntry) -> LocalDateTime {
    let offset = local_offset_secs(entry, utc_secs);
    let local_secs = (utc_secs as i64 + offset) as u64;
    let (year, month, day, hour, minute, second) = decompose(local_secs);
    let is_dst = is_dst_active(entry, utc_secs);
    let abbr = derive_abbr(entry, is_dst);
    LocalDateTime {
        year, month, day, hour, minute, second,
        utc_offset_secs: offset,
        is_dst,
        abbr,
    }
}
