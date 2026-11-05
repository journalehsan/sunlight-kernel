//! CMOS Real-Time Clock driver.
//!
//! SunlightOS has one explicit RTC policy: CMOS contains UTC. The RTC is read
//! once at boot, converted to a Unix timestamp, and then advanced from the
//! centralized monotonic timekeeper. Local civil time is produced later by
//! `timezone_service`; this driver never applies a timezone offset.

use core::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};

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

static REALTIME_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static REALTIME_BASE_UNIX: AtomicU64 = AtomicU64::new(0);
static REALTIME_BASE_TICKS: AtomicU64 = AtomicU64::new(0);
static RTC_VALID: AtomicBool = AtomicBool::new(false);
static REALTIME_UPDATE_OWNER: AtomicU8 = AtomicU8::new(0);
static NEXT_DIAGNOSTIC_CHECKPOINT: AtomicU8 = AtomicU8::new(0);

const OWNER_BOOT_RTC: u8 = 1;
const OWNER_NTP_STEP: u8 = 2;
const DIAGNOSTIC_CHECKPOINT_SECS: [u64; 5] = [60, 3_600, 21_600, 86_400, 476_220];

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
    install_realtime_base(
        timestamp,
        crate::timekeeping::global_ticks(),
        OWNER_BOOT_RTC,
    );
    Ok((datetime, timestamp))
}

fn begin_realtime_write() -> u64 {
    loop {
        let sequence = REALTIME_SEQUENCE.load(Ordering::Acquire);
        if sequence & 1 != 0 {
            core::hint::spin_loop();
            continue;
        }
        if REALTIME_SEQUENCE
            .compare_exchange_weak(sequence, sequence + 1, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            return sequence + 1;
        }
    }
}

fn install_realtime_base(unix_secs: u64, ticks: u64, owner: u8) {
    let write_sequence = begin_realtime_write();
    REALTIME_BASE_UNIX.store(unix_secs, Ordering::Relaxed);
    REALTIME_BASE_TICKS.store(ticks, Ordering::Relaxed);
    REALTIME_UPDATE_OWNER.store(owner, Ordering::Relaxed);
    RTC_VALID.store(true, Ordering::Relaxed);
    REALTIME_SEQUENCE.store(write_sequence + 1, Ordering::Release);
}

fn realtime_snapshot() -> Option<(u64, u64, u64, u8)> {
    loop {
        let before = REALTIME_SEQUENCE.load(Ordering::Acquire);
        if before & 1 != 0 {
            core::hint::spin_loop();
            continue;
        }
        let valid = RTC_VALID.load(Ordering::Relaxed);
        let base_unix = REALTIME_BASE_UNIX.load(Ordering::Relaxed);
        let base_ticks = REALTIME_BASE_TICKS.load(Ordering::Relaxed);
        let owner = REALTIME_UPDATE_OWNER.load(Ordering::Relaxed);
        let current_ticks = crate::timekeeping::global_ticks();
        let after = REALTIME_SEQUENCE.load(Ordering::Acquire);
        if before == after {
            return valid.then_some((base_unix, base_ticks, current_ticks, owner));
        }
    }
}

fn owner_name(owner: u8) -> &'static str {
    match owner {
        OWNER_BOOT_RTC => "boot-rtc",
        OWNER_NTP_STEP => "ntp-step",
        _ => "none",
    }
}

/// Current UTC Unix timestamp in seconds.
///
/// CMOS is read only during `init`; later calls advance the validated boot
/// epoch by the same monotonic tick delta used by uptime. `u64::MAX` is the
/// native syscall failure sentinel when the boot RTC could not be trusted.
pub fn unix_time() -> u64 {
    let Some((base_unix, base_ticks, current_ticks, _)) = realtime_snapshot() else {
        return u64::MAX;
    };
    rtc_codec::wall_time_from_ticks(
        base_unix,
        base_ticks,
        current_ticks,
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
    let ticks = crate::timekeeping::global_ticks();
    install_realtime_base(unix_secs, ticks, OWNER_NTP_STEP);
    crate::serial_println!(
        "[TIME] realtime step owner=ntp-step cpu={} monotonic_ns={} base_ticks={} new_utc_unix={}",
        crate::sched::current_cpu_id(),
        crate::timekeeping::monotonic_ns(),
        ticks,
        unix_secs
    );
    Ok(())
}

/// Seconds since boot, derived only from the centralized monotonic timekeeper.
pub fn uptime_secs() -> u64 {
    crate::timekeeping::uptime_secs()
}

/// Emit a small fixed set of long-uptime checkpoints from the sole BSP
/// timekeeper path. No per-tick output is produced.
pub fn diagnostic_checkpoint(cpu_id: usize, ticks: u64) {
    if !RTC_VALID.load(Ordering::Acquire) {
        return;
    }
    loop {
        let checkpoint_index = NEXT_DIAGNOSTIC_CHECKPOINT.load(Ordering::Relaxed) as usize;
        let Some(&checkpoint_secs) = DIAGNOSTIC_CHECKPOINT_SECS.get(checkpoint_index) else {
            return;
        };
        if ticks / crate::timekeeping::TICK_HZ < checkpoint_secs {
            return;
        }
        if NEXT_DIAGNOSTIC_CHECKPOINT
            .compare_exchange(
                checkpoint_index as u8,
                checkpoint_index as u8 + 1,
                Ordering::Relaxed,
                Ordering::Relaxed,
            )
            .is_err()
        {
            continue;
        }
        if let Some((base_unix, base_ticks, current_ticks, owner)) = realtime_snapshot() {
            let realtime = rtc_codec::wall_time_from_ticks(
                base_unix,
                base_ticks,
                current_ticks,
                crate::timekeeping::TICK_HZ,
            )
            .unwrap_or(u64::MAX);
            crate::serial_println!(
                "[TIME-CHECKPOINT] elapsed_target_secs={} cpu={} monotonic_ns={} global_ticks={} realtime_utc_unix={} realtime_base_unix={} realtime_base_ticks={} last_update_owner={}",
                checkpoint_secs,
                cpu_id,
                crate::timekeeping::monotonic_ns(),
                current_ticks,
                realtime,
                base_unix,
                base_ticks,
                owner_name(owner)
            );
        }
    }
}

/// Read and validate the RTC once. Call after `interrupts::init()` so the
/// monotonic baseline and clocksource diagnostics are available.
pub fn init() {
    let boot_monotonic_ns = crate::timekeeping::monotonic_ns();
    let century_register = crate::arch::x86_64::acpi::rtc_century_register();
    match read_cmos_clock() {
        Ok(raw) => {
            crate::serial_println!(
                "rtc: raw year={:#04x} month={:#04x} day={:#04x} hour={:#04x} minute={:#04x} second={:#04x} status_b={:#04x} century_register={:?} century_raw={:?}",
                raw.year,
                raw.month,
                raw.day,
                raw.hour,
                raw.minute,
                raw.second,
                raw.status_b,
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
                        "wall: utc={:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z unix={} boot_monotonic_ns={} realtime_owner=boot-rtc",
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
