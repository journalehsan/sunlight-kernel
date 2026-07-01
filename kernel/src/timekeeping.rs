//! Central SMP-safe timekeeping state.
//!
//! Global uptime is advanced from exactly one path: the BSP's periodic timer
//! interrupt. AP LAPIC timer interrupts may still drive local preemption and
//! scheduler accounting, but they must not mutate the kernel's exported global
//! tick counters.

use core::sync::atomic::{AtomicU64, Ordering};

pub const TICK_HZ: u64 = 100;
pub const NS_PER_TICK: u64 = 1_000_000_000 / TICK_HZ;
pub const TIMEKEEPER_CORE_ID: usize = 0;

static GLOBAL_TIMEKEEPER_TICKS: AtomicU64 = AtomicU64::new(0);
static LAST_TICK_MONOTONIC_NS: AtomicU64 = AtomicU64::new(0);
static LAST_DRIFT_WARNING_TICK: AtomicU64 = AtomicU64::new(0);

const DRIFT_WARNING_NS: u64 = 250_000_000;

#[inline]
pub fn timekeeper_core() -> usize {
    TIMEKEEPER_CORE_ID
}

#[inline]
pub fn global_ticks() -> u64 {
    GLOBAL_TIMEKEEPER_TICKS.load(Ordering::Relaxed)
}

#[inline]
pub fn uptime_secs() -> u64 {
    global_ticks() / TICK_HZ
}

#[inline]
pub fn monotonic_ms() -> u64 {
    global_ticks().saturating_mul(1000) / TICK_HZ
}

#[inline]
pub fn last_tick_monotonic_ns() -> u64 {
    LAST_TICK_MONOTONIC_NS.load(Ordering::Relaxed)
}

pub fn drift_warning_active() -> bool {
    let ticks = global_ticks();
    if ticks == 0 {
        return false;
    }
    let tick_ns = ticks.saturating_mul(NS_PER_TICK);
    let monotonic_ns = last_tick_monotonic_ns();
    monotonic_ns > 0 && tick_ns > monotonic_ns.saturating_add(DRIFT_WARNING_NS)
}

/// Advance the centralized global timekeeper by one tick.
///
/// Only the BSP timer path may call this. The returned value is the canonical
/// global tick count that legacy interfaces mirror for backward compatibility.
pub fn advance_global_tick(cpu_id: usize, monotonic_ns: u64) -> u64 {
    debug_assert_eq!(cpu_id, TIMEKEEPER_CORE_ID);

    LAST_TICK_MONOTONIC_NS.store(monotonic_ns, Ordering::Relaxed);
    let ticks = GLOBAL_TIMEKEEPER_TICKS.fetch_add(1, Ordering::Relaxed) + 1;

    let tick_ns = ticks.saturating_mul(NS_PER_TICK);
    if tick_ns > monotonic_ns.saturating_add(DRIFT_WARNING_NS) {
        let last_warn = LAST_DRIFT_WARNING_TICK.load(Ordering::Relaxed);
        if ticks.saturating_sub(last_warn) >= TICK_HZ
            && LAST_DRIFT_WARNING_TICK
                .compare_exchange(last_warn, ticks, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
        {
            crate::serial_println!(
                "[TIME] warning: uptime advancing suspiciously fast timekeeper_core={} global_timekeeper_ticks={} monotonic_ns={} tick_ns={}",
                TIMEKEEPER_CORE_ID,
                ticks,
                monotonic_ns,
                tick_ns
            );
        }
    }

    ticks
}
