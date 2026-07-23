//! Pure CMOS RTC decoding and calendar conversion.
//!
//! This module deliberately contains no port I/O so the exact production
//! decoder can be exercised by the host-side time proof.

pub const STATUS_B_24HR: u8 = 0x02;
pub const STATUS_B_BINARY: u8 = 0x04;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawRtc {
    pub second: u8,
    pub minute: u8,
    pub hour: u8,
    pub day: u8,
    pub month: u8,
    pub year: u8,
    pub century: Option<u8>,
    pub status_b: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RtcDateTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RtcDecodeError {
    InvalidBcd,
    InvalidCentury,
    InvalidYear,
    InvalidMonth,
    InvalidDay,
    InvalidHour,
    InvalidMinute,
    InvalidSecond,
    Overflow,
}

#[inline]
pub const fn is_binary_mode(status_b: u8) -> bool {
    status_b & STATUS_B_BINARY != 0
}

#[inline]
pub const fn is_24_hour_mode(status_b: u8) -> bool {
    status_b & STATUS_B_24HR != 0
}

#[inline]
const fn valid_bcd(value: u8) -> bool {
    (value & 0x0f) <= 9 && (value >> 4) <= 9
}

#[inline]
fn decode_field(value: u8, binary: bool) -> Result<u8, RtcDecodeError> {
    if binary {
        Ok(value)
    } else if valid_bcd(value) {
        Ok((value >> 4) * 10 + (value & 0x0f))
    } else {
        Err(RtcDecodeError::InvalidBcd)
    }
}

pub const fn is_leap_year(year: u16) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

pub const fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

/// Decode one already-stabilized CMOS register snapshot.
///
/// When ACPI does not advertise a century register, use the conventional
/// Unix-compatible pivot: 70..99 means 1970..1999 and 00..69 means
/// 2000..2069. The selected policy is reported in the boot diagnostics.
pub fn decode(raw: RawRtc) -> Result<RtcDateTime, RtcDecodeError> {
    let binary = is_binary_mode(raw.status_b);
    let year_low = decode_field(raw.year, binary)?;
    let year = match raw.century {
        Some(century_raw) => {
            let century = decode_field(century_raw, binary)?;
            if century == 0 {
                return Err(RtcDecodeError::InvalidCentury);
            }
            (century as u16)
                .checked_mul(100)
                .and_then(|base| base.checked_add(year_low as u16))
                .ok_or(RtcDecodeError::Overflow)?
        }
        None if year_low >= 70 => 1900 + year_low as u16,
        None => 2000 + year_low as u16,
    };

    if !(1970..=9999).contains(&year) {
        return Err(RtcDecodeError::InvalidYear);
    }

    let month = decode_field(raw.month, binary)?;
    if !(1..=12).contains(&month) {
        return Err(RtcDecodeError::InvalidMonth);
    }

    let day = decode_field(raw.day, binary)?;
    if day == 0 || day > days_in_month(year, month) {
        return Err(RtcDecodeError::InvalidDay);
    }

    let minute = decode_field(raw.minute, binary)?;
    if minute >= 60 {
        return Err(RtcDecodeError::InvalidMinute);
    }
    let second = decode_field(raw.second, binary)?;
    if second >= 60 {
        return Err(RtcDecodeError::InvalidSecond);
    }

    let pm = raw.hour & 0x80 != 0;
    let hour_without_pm = raw.hour & 0x7f;
    let mut hour = decode_field(hour_without_pm, binary)?;
    if is_24_hour_mode(raw.status_b) {
        // Bit 7 is only the PM bit in 12-hour mode. Accepting it in 24-hour
        // mode could turn a malformed register into a plausible hour.
        if pm || hour >= 24 {
            return Err(RtcDecodeError::InvalidHour);
        }
    } else {
        if !(1..=12).contains(&hour) {
            return Err(RtcDecodeError::InvalidHour);
        }
        hour = match (hour, pm) {
            (12, false) => 0,
            (12, true) => 12,
            (h, false) => h,
            (h, true) => h + 12,
        };
    }

    Ok(RtcDateTime {
        year,
        month,
        day,
        hour,
        minute,
        second,
    })
}

/// Days since 1970-01-01 for a validated civil date.
pub fn days_from_civil(year: u16, month: u8, day: u8) -> u64 {
    let year = year as u64;
    let month = month as u64;
    let day = day as u64;
    let y = if month <= 2 { year - 1 } else { year };
    let era = y / 400;
    let yoe = y - era * 400;
    let mp = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

pub fn unix_seconds(datetime: RtcDateTime) -> Result<u64, RtcDecodeError> {
    if datetime.year < 1970 || datetime.year > 9999 {
        return Err(RtcDecodeError::InvalidYear);
    }
    if datetime.month == 0 || datetime.month > 12 {
        return Err(RtcDecodeError::InvalidMonth);
    }
    if datetime.day == 0 || datetime.day > days_in_month(datetime.year, datetime.month) {
        return Err(RtcDecodeError::InvalidDay);
    }
    if datetime.hour >= 24 {
        return Err(RtcDecodeError::InvalidHour);
    }
    if datetime.minute >= 60 {
        return Err(RtcDecodeError::InvalidMinute);
    }
    if datetime.second >= 60 {
        return Err(RtcDecodeError::InvalidSecond);
    }

    days_from_civil(datetime.year, datetime.month, datetime.day)
        .checked_mul(86_400)
        .and_then(|value| value.checked_add(datetime.hour as u64 * 3_600))
        .and_then(|value| value.checked_add(datetime.minute as u64 * 60))
        .and_then(|value| value.checked_add(datetime.second as u64))
        .ok_or(RtcDecodeError::Overflow)
}

/// Advance a boot wall-clock epoch from the same monotonic tick delta used by
/// uptime. This never changes the monotonic state itself.
pub fn wall_time_from_ticks(
    boot_unix: u64,
    boot_ticks: u64,
    current_ticks: u64,
    tick_hz: u64,
) -> Option<u64> {
    if tick_hz == 0 {
        return None;
    }
    let elapsed = current_ticks.saturating_sub(boot_ticks) / tick_hz;
    boot_unix.checked_add(elapsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw_bcd(hour: u8) -> RawRtc {
        RawRtc {
            second: 0x56,
            minute: 0x34,
            hour,
            day: 0x23,
            month: 0x07,
            year: 0x26,
            century: Some(0x20),
            status_b: STATUS_B_24HR,
        }
    }

    #[test]
    fn decodes_bcd_and_binary_modes() {
        let bcd = decode(raw_bcd(0x18)).unwrap();
        assert_eq!(
            bcd,
            RtcDateTime {
                year: 2026,
                month: 7,
                day: 23,
                hour: 18,
                minute: 34,
                second: 56,
            }
        );

        let binary = decode(RawRtc {
            second: 56,
            minute: 34,
            hour: 18,
            day: 23,
            month: 7,
            year: 26,
            century: Some(20),
            status_b: STATUS_B_BINARY | STATUS_B_24HR,
        })
        .unwrap();
        assert_eq!(binary, bcd);
    }

    #[test]
    fn converts_12_am_12_pm_and_pm_bit() {
        let mut midnight = raw_bcd(0x12);
        midnight.status_b = 0;
        assert_eq!(decode(midnight).unwrap().hour, 0);

        let mut noon = midnight;
        noon.hour = 0x92;
        assert_eq!(decode(noon).unwrap().hour, 12);

        let mut afternoon = midnight;
        afternoon.hour = 0x89;
        assert_eq!(decode(afternoon).unwrap().hour, 21);
    }

    #[test]
    fn rejects_malformed_bcd_even_when_arithmetic_would_look_plausible() {
        let mut raw = raw_bcd(0x18);
        raw.minute = 0x2a;
        assert_eq!(decode(raw), Err(RtcDecodeError::InvalidBcd));
    }

    #[test]
    fn rejects_malformed_ranges_and_dates() {
        let mut raw = raw_bcd(0x18);
        raw.month = 0x13;
        assert_eq!(decode(raw), Err(RtcDecodeError::InvalidMonth));

        raw = raw_bcd(0x18);
        raw.day = 0x31;
        raw.month = 0x04;
        assert_eq!(decode(raw), Err(RtcDecodeError::InvalidDay));

        raw = raw_bcd(0x18);
        raw.hour = 0x98;
        assert_eq!(decode(raw), Err(RtcDecodeError::InvalidHour));
    }

    #[test]
    fn handles_february_and_leap_years() {
        let leap = RtcDateTime {
            year: 2024,
            month: 2,
            day: 29,
            hour: 23,
            minute: 59,
            second: 59,
        };
        assert!(unix_seconds(leap).is_ok());
        assert_eq!(
            unix_seconds(RtcDateTime { year: 2025, ..leap }),
            Err(RtcDecodeError::InvalidDay)
        );
        assert!(is_leap_year(2000));
        assert!(!is_leap_year(2100));
    }

    #[test]
    fn civil_boundaries_advance_by_one_second() {
        let pairs = [
            ((2026, 7, 31), (2026, 8, 1)),
            ((2025, 2, 28), (2025, 3, 1)),
            ((2024, 2, 29), (2024, 3, 1)),
            ((2026, 12, 31), (2027, 1, 1)),
        ];
        for &(before, after) in &pairs {
            let a = unix_seconds(RtcDateTime {
                year: before.0,
                month: before.1,
                day: before.2,
                hour: 23,
                minute: 59,
                second: 59,
            })
            .unwrap();
            let b = unix_seconds(RtcDateTime {
                year: after.0,
                month: after.1,
                day: after.2,
                hour: 0,
                minute: 0,
                second: 0,
            })
            .unwrap();
            assert_eq!(b - a, 1);
        }
    }

    #[test]
    fn wall_and_uptime_share_exact_unadjusted_progression() {
        let elapsed = (6 * 60 + 46) * 60;
        let boot_wall = 1_800_000_000;
        let boot_ticks = 12_345;
        let now_ticks = boot_ticks + elapsed * 100;
        assert_eq!(
            wall_time_from_ticks(boot_wall, boot_ticks, now_ticks, 100),
            Some(boot_wall + elapsed)
        );
    }
}
