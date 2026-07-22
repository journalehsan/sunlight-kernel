//! CMOS Real-Time Clock driver.
//!
//! Reads the wall-clock once at boot, converts it to a Unix timestamp, then
//! advances it from the centralized kernel timekeeper tick (~100 Hz) so later
//! reads never have to touch the slow CMOS ports again. BCD and 12-hour
//! register quirks are handled per the classic CMOS spec.

use core::sync::atomic::{AtomicU64, Ordering};

const CMOS_INDEX: u16 = 0x70;
const CMOS_DATA: u16 = 0x71;

const RTC_SECS: u8 = 0x00;
const RTC_MINS: u8 = 0x02;
const RTC_HOURS: u8 = 0x04;
const RTC_DAY: u8 = 0x07;
const RTC_MONTH: u8 = 0x08;
const RTC_YEAR: u8 = 0x09;
const RTC_STATUS_A: u8 = 0x0A;
const RTC_STATUS_B: u8 = 0x0B;

const STATUS_A_UPDATE_IN_PROGRESS: u8 = 0x80;
const STATUS_B_24HR: u8 = 0x02;
const STATUS_B_BINARY: u8 = 0x04;
const RTC_MIN_YEAR: u64 = 2024;
const RTC_MAX_YEAR: u64 = 2040;
const RTC_FALLBACK_YEAR: u64 = 2026;
const RTC_FALLBACK_MONTH: u64 = 7;
const RTC_FALLBACK_DAY: u64 = 8;

static BOOT_UNIX_TIME: AtomicU64 = AtomicU64::new(0);
static BOOT_TICKS: AtomicU64 = AtomicU64::new(0);

fn cmos_read(index: u8) -> u8 {
    let value: u8;
    unsafe {
        core::arch::asm!(
            "out dx, al",
            in("dx") CMOS_INDEX,
            in("al") index,
            options(nomem, nostack),
        );
        core::arch::asm!(
            "in al, dx",
            in("dx") CMOS_DATA,
            out("al") value,
            options(nomem, nostack),
        );
    }
    value
}

fn bcd_to_binary(v: u8) -> u8 {
    ((v >> 4) * 10) + (v & 0x0F)
}

fn is_leap_year(year: u64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_in_month(year: u64, month: u64) -> u64 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_valid_rtc_datetime(year: u64, month: u64, day: u64, hour: u64, min: u64, sec: u64) -> bool {
    year >= RTC_MIN_YEAR
        && year <= RTC_MAX_YEAR
        && month >= 1
        && month <= 12
        && day >= 1
        && day <= days_in_month(year, month)
        && hour < 24
        && min < 60
        && sec < 60
}

fn sanitized_rtc_datetime(
    year: u64,
    month: u64,
    day: u64,
    hour: u64,
    min: u64,
    sec: u64,
) -> (u64, u64, u64, u64, u64, u64, bool) {
    if is_valid_rtc_datetime(year, month, day, hour, min, sec) {
        return (year, month, day, hour, min, sec, true);
    }

    (
        RTC_FALLBACK_YEAR,
        RTC_FALLBACK_MONTH,
        RTC_FALLBACK_DAY,
        0,
        0,
        0,
        false,
    )
}

/// Read calendar time from CMOS: (year, month, day, hour, min, sec), 24h.
fn read_cmos_clock() -> (u64, u64, u64, u64, u64, u64) {
    // Wait out an in-progress update, then read until two consecutive reads
    // agree so we never see a torn mid-update value.
    loop {
        while cmos_read(RTC_STATUS_A) & STATUS_A_UPDATE_IN_PROGRESS != 0 {}

        let format = cmos_read(RTC_STATUS_B);
        let sec = cmos_read(RTC_SECS);
        let min = cmos_read(RTC_MINS);
        let hour_raw = cmos_read(RTC_HOURS);
        let day = cmos_read(RTC_DAY);
        let month = cmos_read(RTC_MONTH);
        let year = cmos_read(RTC_YEAR);

        if cmos_read(RTC_SECS) != sec {
            continue;
        }

        let pm = hour_raw & 0x80 != 0;
        let mut hour = hour_raw & 0x7F;

        let (mut year, mut month, mut day, mut min, mut sec) = (year, month, day, min, sec);
        if format & STATUS_B_BINARY == 0 {
            year = bcd_to_binary(year);
            month = bcd_to_binary(month);
            day = bcd_to_binary(day);
            hour = bcd_to_binary(hour);
            min = bcd_to_binary(min);
            sec = bcd_to_binary(sec);
        }

        if format & STATUS_B_24HR == 0 {
            if hour == 12 {
                if !pm {
                    hour = 0;
                }
            } else if pm {
                hour += 12;
            }
        }

        return (
            2000 + year as u64,
            month as u64,
            day as u64,
            hour as u64,
            min as u64,
            sec as u64,
        );
    }
}

/// Days since the Unix epoch for a civil date (Howard Hinnant's algorithm).
fn days_from_civil(year: u64, month: u64, day: u64) -> u64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y / 400;
    let yoe = y - era * 400;
    let mp = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// Current Unix timestamp in seconds. CMOS is only read on the first call;
/// afterwards time advances from the centralized global tick counter.
pub fn unix_time() -> u64 {
    let boot_time = BOOT_UNIX_TIME.load(Ordering::Relaxed);
    if boot_time != 0 {
        let elapsed_ticks =
            crate::timekeeping::global_ticks().saturating_sub(BOOT_TICKS.load(Ordering::Relaxed));
        // The native syscall ABI reserves u64::MAX as generic failure; never
        // silently wrap or clamp a calendar timestamp into a different date.
        return boot_time
            .checked_add(elapsed_ticks / crate::timekeeping::TICK_HZ)
            .unwrap_or(u64::MAX);
    }

    let (year, month, day, hour, min, sec) = read_cmos_clock();
    let (year, month, day, hour, min, sec, _valid) =
        sanitized_rtc_datetime(year, month, day, hour, min, sec);
    let ts = days_from_civil(year, month, day) * 86400 + hour * 3600 + min * 60 + sec;
    BOOT_UNIX_TIME.store(ts, Ordering::Relaxed);
    BOOT_TICKS.store(crate::timekeeping::global_ticks(), Ordering::Relaxed);
    ts
}

/// Seconds since boot, derived from the centralized timekeeper.
pub fn uptime_secs() -> u64 {
    // Legacy uptime API kept intentionally for existing services/apps.
    crate::timekeeping::uptime_secs()
}

/// Read the RTC once and log it. Call after interrupts::init() so the tick
/// baseline is live.
pub fn init() {
    let (year, month, day, hour, min, sec) = read_cmos_clock();
    let raw = (year, month, day, hour, min, sec);
    let (year, month, day, hour, min, sec, valid) =
        sanitized_rtc_datetime(year, month, day, hour, min, sec);
    let ts = days_from_civil(year, month, day) * 86400 + hour * 3600 + min * 60 + sec;
    BOOT_UNIX_TIME.store(ts, Ordering::Relaxed);
    BOOT_TICKS.store(crate::timekeeping::global_ticks(), Ordering::Relaxed);
    if valid {
        crate::serial_println!(
            "[RTC] CMOS clock: {}/{}/{} {:02}:{:02}:{:02} UTC (unix {}) OK",
            year,
            month,
            day,
            hour,
            min,
            sec,
            ts
        );
    } else {
        crate::serial_println!(
            "[RTC] WARNING invalid CMOS clock: {}/{}/{} {:02}:{:02}:{:02}; using {}/{}/{} 00:00:00 UTC (unix {})",
            raw.0,
            raw.1,
            raw.2,
            raw.3,
            raw.4,
            raw.5,
            year,
            month,
            day,
            ts
        );
    }
}
