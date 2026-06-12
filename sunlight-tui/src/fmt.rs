//! Number formatting without heap allocation

#![allow(dead_code)]

/// Format u32 as decimal into a fixed buffer
/// Returns the slice of `buf` that was written
pub fn fmt_u32<'a>(buf: &'a mut [u8; 20], n: u32) -> &'a str {
    if n == 0 {
        buf[0] = b'0';
        return unsafe { core::str::from_utf8_unchecked(&buf[0..1]) };
    }

    let mut num = n;
    let mut pos = 19;

    while num > 0 {
        buf[pos] = b'0' + (num % 10) as u8;
        num /= 10;
        if pos == 0 {
            break;
        }
        pos -= 1;
    }

    unsafe { core::str::from_utf8_unchecked(&buf[pos + 1..20]) }
}

/// Format u64 as hex (e.g., "0xFFFF800000000000") into fixed buffer
pub fn fmt_hex<'a>(buf: &'a mut [u8; 20], n: u64) -> &'a str {
    buf[0] = b'0';
    buf[1] = b'x';

    for i in 0..16 {
        let nibble = ((n >> (60 - i * 4)) & 0xF) as u8;
        buf[2 + i] = if nibble < 10 {
            b'0' + nibble
        } else {
            b'A' + (nibble - 10)
        };
    }

    unsafe { core::str::from_utf8_unchecked(&buf[0..18]) }
}

/// Civil date/time from a Unix timestamp: (year, month, day, hour, min, sec).
/// Days-to-date conversion per Howard Hinnant's civil_from_days algorithm.
pub fn unix_to_datetime(ts: u64) -> (u64, u64, u64, u64, u64, u64) {
    let days = ts / 86400;
    let secs = ts % 86400;

    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };

    (year, m, d, secs / 3600, (secs / 60) % 60, secs % 60)
}

/// Format a Unix timestamp as `12:22 AM | 2026/6/12` into `buf`.
/// Returns the number of bytes written.
pub fn fmt_clock(buf: &mut [u8], ts: u64) -> usize {
    let (year, month, day, hour24, min, _sec) = unix_to_datetime(ts);

    let (hour12, suffix) = match hour24 {
        0 => (12, b"AM"),
        1..=11 => (hour24, b"AM"),
        12 => (12, b"PM"),
        _ => (hour24 - 12, b"PM"),
    };

    let mut pos = 0;
    let mut push = |bytes: &[u8], pos: &mut usize| {
        for &b in bytes {
            if *pos < buf.len() {
                buf[*pos] = b;
                *pos += 1;
            }
        }
    };

    let mut num = [0u8; 20];
    push(fmt_u32(&mut num, hour12 as u32).as_bytes(), &mut pos);
    push(b":", &mut pos);
    if min < 10 {
        push(b"0", &mut pos);
    }
    push(fmt_u32(&mut num, min as u32).as_bytes(), &mut pos);
    push(b" ", &mut pos);
    push(suffix, &mut pos);
    push(b" | ", &mut pos);
    push(fmt_u32(&mut num, year as u32).as_bytes(), &mut pos);
    push(b"/", &mut pos);
    push(fmt_u32(&mut num, month as u32).as_bytes(), &mut pos);
    push(b"/", &mut pos);
    push(fmt_u32(&mut num, day as u32).as_bytes(), &mut pos);

    pos
}

/// Format "X/Y MiB" into fixed buffer
pub fn fmt_mib<'a>(buf: &'a mut [u8; 32], used: u32, total: u32) -> &'a str {
    let mut pos = 0;

    // Format used
    let mut temp = [0u8; 20];
    let used_str = fmt_u32(&mut temp, used);
    for &b in used_str.as_bytes() {
        buf[pos] = b;
        pos += 1;
    }

    buf[pos] = b'/';
    pos += 1;

    // Format total
    let total_str = fmt_u32(&mut temp, total);
    for &b in total_str.as_bytes() {
        buf[pos] = b;
        pos += 1;
    }

    // Add " MiB"
    buf[pos] = b' ';
    buf[pos + 1] = b'M';
    buf[pos + 2] = b'i';
    buf[pos + 3] = b'B';
    pos += 4;

    unsafe { core::str::from_utf8_unchecked(&buf[0..pos]) }
}
