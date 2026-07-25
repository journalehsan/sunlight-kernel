//! CMOS Real-Time Clock driver.
//!
//! SunlightOS has one explicit RTC policy: CMOS contains UTC. The RTC is read
//! once at boot, converted to a Unix timestamp, and then advanced from the
//! centralized monotonic timekeeper. Local civil time is produced later by
//! `timezone_service`; this driver never applies a timezone offset.

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use super::rtc_codec::{self, RawRtc, RtcDateTime};

const CMOS_INDEX: u16 = 0x70;
const CMOS_DATA: u16 = 0x71;

const RTC_SECS: u8 = 0x00;
const RTC_MINS: u8 = 0x02;
const RTC_HOURS: u8 = 0x04;
const RTC_DAY: u8 = 0x07;
const RTC_MONTH: u8 = 0x08;
const RTC_YEAR: u8 = 0x09;
const RTC_STATUS_A: u8 = 0x0a;
const RTC_STATUS_B: u8 = 0x0b;

const STATUS_A_UPDATE_IN_PROGRESS: u8 = 0x80;
const RTC_STABLE_READ_ATTEMPTS: usize = 8;
const RTC_UIP_SPIN_LIMIT: usize = 1_000_000;

static BOOT_UNIX_TIME: AtomicU64 = AtomicU64::new(0);
static BOOT_TICKS: AtomicU64 = AtomicU64::new(0);
static RTC_VALID: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RtcReadError {
    UpdateInProgressTimeout,
    UnstableSnapshot,
}

fn cmos_read(index: u8) -> u8 {
    let value: u8;
    unsafe {
        // Keep NMIs masked while a multi-register RTC snapshot is in flight.
        core::arch::asm!(
            "out dx, al",
            in("dx") CMOS_INDEX,
            in("al") index | 0x80,
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

fn finish_cmos_access() {
    unsafe {
        // Re-enable NMI and leave the harmless seconds register selected.
        core::arch::asm!(
            "out dx, al",
            in("dx") CMOS_INDEX,
            in("al") RTC_SECS,
            options(nomem, nostack),
        );
    }
}

fn wait_for_update_complete() -> bool {
    for _ in 0..RTC_UIP_SPIN_LIMIT {
        if cmos_read(RTC_STATUS_A) & STATUS_A_UPDATE_IN_PROGRESS == 0 {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}

fn read_raw_snapshot(century_register: Option<u8>) -> RawRtc {
    RawRtc {
        second: cmos_read(RTC_SECS),
        minute: cmos_read(RTC_MINS),
        hour: cmos_read(RTC_HOURS),
        day: cmos_read(RTC_DAY),
        month: cmos_read(RTC_MONTH),
        year: cmos_read(RTC_YEAR),
        century: century_register.map(cmos_read),
        status_b: cmos_read(RTC_STATUS_B),
    }
}

/// Read two complete, identical register snapshots outside the CMOS update
/// window. Rechecking only seconds is not enough to prove that all fields came
/// from the same calendar second on real hardware.
fn read_cmos_clock() -> Result<RawRtc, RtcReadError> {
    let century_register = crate::arch::x86_64::acpi::rtc_century_register();
    for _ in 0..RTC_STABLE_READ_ATTEMPTS {
        if !wait_for_update_complete() {
            finish_cmos_access();
            return Err(RtcReadError::UpdateInProgressTimeout);
        }
        let first = read_raw_snapshot(century_register);

        if !wait_for_update_complete() {
            finish_cmos_access();
            return Err(RtcReadError::UpdateInProgressTimeout);
        }
        let second = read_raw_snapshot(century_register);
        if first == second {
            finish_cmos_access();
            return Ok(first);
        }
    }
    finish_cmos_access();
    Err(RtcReadError::UnstableSnapshot)
}

fn install_boot_wall_time(raw: RawRtc) -> Result<(RtcDateTime, u64), rtc_codec::RtcDecodeError> {
    let datetime = rtc_codec::decode(raw)?;
    let timestamp = rtc_codec::unix_seconds(datetime)?;
    BOOT_UNIX_TIME.store(timestamp, Ordering::Relaxed);
    BOOT_TICKS.store(crate::timekeeping::global_ticks(), Ordering::Relaxed);
    RTC_VALID.store(true, Ordering::Release);
    Ok((datetime, timestamp))
}

/// Current UTC Unix timestamp in seconds.
///
/// CMOS is read only during `init`; later calls advance the validated boot
/// epoch by the same monotonic tick delta used by uptime. `u64::MAX` is the
/// native syscall failure sentinel when the boot RTC could not be trusted.
pub fn unix_time() -> u64 {
    if !RTC_VALID.load(Ordering::Acquire) {
        return u64::MAX;
    }
    rtc_codec::wall_time_from_ticks(
        BOOT_UNIX_TIME.load(Ordering::Relaxed),
        BOOT_TICKS.load(Ordering::Relaxed),
        crate::timekeeping::global_ticks(),
        crate::timekeeping::TICK_HZ,
    )
    .unwrap_or(u64::MAX)
}

/// Rebaseline the running UTC wall clock to `unix_secs` without touching the
/// monotonic timekeeper.
///
/// This is a discrete step (no kernel slew primitive exists). RTC/CMOS is not
/// rewritten here — CMOS write support is intentionally out of scope for the
/// NTP slice; local civil time must never be written to the RTC.
///
/// Rejects the error sentinel and zero (uninitialized) values.
pub fn set_unix_time(unix_secs: u64) -> Result<(), ()> {
    if unix_secs == 0 || unix_secs == u64::MAX {
        return Err(());
    }
    // Reasonable administrative bounds: 2000-01-01 .. 2100-01-01 UTC.
    const MIN_UNIX: u64 = 946_684_800;
    const MAX_UNIX: u64 = 4_102_444_800;
    if unix_secs < MIN_UNIX || unix_secs > MAX_UNIX {
        return Err(());
    }
    BOOT_UNIX_TIME.store(unix_secs, Ordering::Relaxed);
    BOOT_TICKS.store(crate::timekeeping::global_ticks(), Ordering::Relaxed);
    RTC_VALID.store(true, Ordering::Release);
    Ok(())
}

/// Seconds since boot, derived only from the centralized monotonic timekeeper.
pub fn uptime_secs() -> u64 {
    crate::timekeeping::uptime_secs()
}

/// Read and validate the RTC once. Call after `interrupts::init()` so the
/// monotonic baseline and clocksource diagnostics are available.
pub fn init() {
    let boot_monotonic_ns = crate::timekeeping::monotonic_ns();
    let century_register = crate::arch::x86_64::acpi::rtc_century_register();
    match read_cmos_clock() {
        Ok(raw) => {
            crate::serial_println!(
                "rtc: raw year={:#04x} month={:#04x} day={:#04x} hour={:#04x} minute={:#04x} second={:#04x} century_register={:?} century_raw={:?}",
                raw.year,
                raw.month,
                raw.day,
                raw.hour,
                raw.minute,
                raw.second,
                century_register,
                raw.century
            );
            crate::serial_println!(
                "rtc: mode={} hour_mode={} stable_read=yes century_policy={}",
                if rtc_codec::is_binary_mode(raw.status_b) {
                    "binary"
                } else {
                    "bcd"
                },
                if rtc_codec::is_24_hour_mode(raw.status_b) {
                    "24h"
                } else {
                    "12h"
                },
                if century_register.is_some() {
                    "acpi-register"
                } else {
                    "pivot-1970"
                }
            );
            match install_boot_wall_time(raw) {
                Ok((datetime, timestamp)) => {
                    crate::serial_println!(
                        "rtc: decoded_utc={:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z assumed_basis=UTC",
                        datetime.year,
                        datetime.month,
                        datetime.day,
                        datetime.hour,
                        datetime.minute,
                        datetime.second
                    );
                    crate::serial_println!(
                        "wall: utc={:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z unix={} boot_monotonic_ns={}",
                        datetime.year,
                        datetime.month,
                        datetime.day,
                        datetime.hour,
                        datetime.minute,
                        datetime.second,
                        timestamp,
                        boot_monotonic_ns
                    );
                }
                Err(error) => {
                    RTC_VALID.store(false, Ordering::Release);
                    crate::serial_println!(
                        "rtc: ERROR decode={:?}; wall clock unavailable (no fabricated fallback date)",
                        error
                    );
                }
            }
        }
        Err(error) => {
            RTC_VALID.store(false, Ordering::Release);
            crate::serial_println!(
                "rtc: ERROR read={:?} stable_read=no; wall clock unavailable",
                error
            );
        }
    }
}
