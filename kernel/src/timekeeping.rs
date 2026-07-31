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
static REJECTED_NON_OWNER_ADVANCES: AtomicU64 = AtomicU64::new(0);

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

/// Canonical cross-core monotonic timestamp in nanoseconds.  This is derived
/// from the BSP-only timekeeper rather than the per-core TSC so public clock
/// reads remain ordered across task migration even on hardware without a
/// synchronized TSC.  Its resolution is one timer tick (currently 10 ms).
#[inline]
pub fn monotonic_ns() -> u64 {
    global_ticks().saturating_mul(NS_PER_TICK)
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

/// Advance the centralized global timekeeper from the authoritative BSP path.
///
/// When a calibrated monotonic timestamp is available, the exported tick count
/// is derived from elapsed time. Duplicate callbacks at the same timestamp are
/// therefore idempotent, and delayed callbacks catch up without accumulating
/// callback-count error. The fallback advances once per BSP interrupt only.
pub fn advance_global_tick(cpu_id: usize, calibrated_monotonic_ns: Option<u64>) -> u64 {
    if cpu_id != TIMEKEEPER_CORE_ID {
        let rejected = REJECTED_NON_OWNER_ADVANCES.fetch_add(1, Ordering::Relaxed) + 1;
        if rejected == 1 {
            crate::serial_println!(
                "[TIME] rejected non-owner global advance cpu={} owner_cpu={}",
                cpu_id,
                TIMEKEEPER_CORE_ID
            );
        }
        return global_ticks();
    }

    let ticks = if let Some(monotonic_ns) = calibrated_monotonic_ns {
        LAST_TICK_MONOTONIC_NS.store(monotonic_ns, Ordering::Relaxed);
        let elapsed_ticks = monotonic_ns / NS_PER_TICK;
        let mut current = GLOBAL_TIMEKEEPER_TICKS.load(Ordering::Relaxed);
        loop {
            if current >= elapsed_ticks {
                break current;
            }
            match GLOBAL_TIMEKEEPER_TICKS.compare_exchange_weak(
                current,
                elapsed_ticks,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break elapsed_ticks,
                Err(observed) => current = observed,
            }
        }
    } else {
        loop {
            let current = GLOBAL_TIMEKEEPER_TICKS.load(Ordering::Relaxed);
            if current == u64::MAX {
                break current;
            }
            match GLOBAL_TIMEKEEPER_TICKS.compare_exchange_weak(
                current,
                current + 1,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break current + 1,
                Err(_) => continue,
            }
        }
    };

    if let Some(monotonic_ns) = calibrated_monotonic_ns {
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
    }

    ticks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reset() {
        GLOBAL_TIMEKEEPER_TICKS.store(0, Ordering::Relaxed);
        LAST_TICK_MONOTONIC_NS.store(0, Ordering::Relaxed);
        LAST_DRIFT_WARNING_TICK.store(0, Ordering::Relaxed);
        REJECTED_NON_OWNER_ADVANCES.store(0, Ordering::Relaxed);
    }

    #[test]
    fn calibrated_progression_matches_elapsed_time() {
        for elapsed_secs in [1, 60, 86_400, 132 * 3_600 + 17 * 60] {
            reset();
            let elapsed_ns = elapsed_secs * 1_000_000_000;
            assert_eq!(
                advance_global_tick(TIMEKEEPER_CORE_ID, Some(elapsed_ns)),
                elapsed_secs * TICK_HZ
            );
            assert_eq!(monotonic_ns(), elapsed_ns);
        }
    }

    #[test]
    fn many_small_steps_equal_one_large_step() {
        let elapsed_secs = 132 * 3_600 + 17 * 60;
        let target_ns = elapsed_secs * 1_000_000_000;

        reset();
        for second in 1..=elapsed_secs {
            advance_global_tick(TIMEKEEPER_CORE_ID, Some(second * 1_000_000_000));
        }
        let small_steps = global_ticks();

        reset();
        let large_step = advance_global_tick(TIMEKEEPER_CORE_ID, Some(target_ns));
        assert_eq!(small_steps, large_step);
        assert_eq!(large_step, elapsed_secs * TICK_HZ);
    }

    #[test]
    fn duplicate_callbacks_and_aps_cannot_multiply_global_time() {
        reset();
        let timestamp = 10 * NS_PER_TICK;
        assert_eq!(advance_global_tick(TIMEKEEPER_CORE_ID, Some(timestamp)), 10);
        assert_eq!(advance_global_tick(TIMEKEEPER_CORE_ID, Some(timestamp)), 10);
        assert_eq!(advance_global_tick(1, Some(20 * NS_PER_TICK)), 10);
        assert_eq!(global_ticks(), 10);
    }

    #[test]
    fn uncalibrated_fallback_counts_only_bsp_interrupts() {
        reset();
        assert_eq!(advance_global_tick(TIMEKEEPER_CORE_ID, None), 1);
        assert_eq!(advance_global_tick(3, None), 1);
        assert_eq!(advance_global_tick(TIMEKEEPER_CORE_ID, None), 2);
    }
}
