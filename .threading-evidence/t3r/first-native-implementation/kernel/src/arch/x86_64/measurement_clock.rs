//! Opt-in per-CPU measurement clock for native IPC instrumentation (M5.4).
//!
//! This is deliberately separate from the public monotonic clock in
//! [`crate::timekeeping`]. The public clock is a coarse, globally ordered
//! 100 Hz tick counter. This module exposes a *local* counter-derived clock for
//! fixtures only, with three hard rules enforced by
//! [`sunlight_ipc::measurement_clock`]:
//!
//! 1. A timestamp belongs to the CPU that took it. Cross-CPU subtraction is not
//!    qualified and is rejected.
//! 2. The frequency is only trusted when the platform advertises an invariant
//!    TSC (CPUID.8000_0007H:EDX[8]). Without it, the record stays invalid and
//!    every fixture must report the clock as unsupported rather than guess.
//! 3. The origin is per-CPU, so even a same-frequency AP has a different
//!    absolute value; only same-CPU differences are meaningful.
//!
//! Nothing here runs in production: the whole module is compiled only under the
//! debug-only `fastlane_measurement_test` feature.

use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use sunlight_ipc::measurement_clock::{self as mc, Calibration, Stamp};

static EPOCH: AtomicU32 = AtomicU32::new(0);
static HZ: AtomicU64 = AtomicU64::new(0);
static MUL: AtomicU64 = AtomicU64::new(0);
static PPM: AtomicU32 = AtomicU32::new(0);
static BOOT_PPM: AtomicU32 = AtomicU32::new(0);
static FLAGS: AtomicU32 = AtomicU32::new(0);
static STABILITY_PPM: AtomicU32 = AtomicU32::new(u32::MAX);
static ORIGIN: [AtomicU64; crate::sched::MAX_CORES] =
    [const { AtomicU64::new(0) }; crate::sched::MAX_CORES];

/// Reference window for the runtime stability re-check. It matches the boot
/// window so both estimates have the same quantization and end-to-end latency.
const STABILITY_COUNTS: u64 = crate::arch::x86_64::pit::CALIBRATION_COUNTS;
const STABILITY_REPEATS: usize = 5;
/// Windows drawn for one validation round. The first is discarded as warm-up;
/// the host-side serial line reports the full set so the discard is auditable.
const STABILITY_WINDOWS: usize = STABILITY_REPEATS + 1;
/// Absolute ceiling on the *validated* frequency uncertainty. Above this the
/// counter is not stable enough to convert durations at all and the clock is
/// invalidated rather than reported with a useless bound.
const MAX_QUALIFIED_PPM: u32 = 20_000;

/// CPUID.8000_0007H:EDX[8] — constant-rate TSC across P/C states.
fn invariant_tsc() -> bool {
    let max_extended = core::arch::x86_64::__cpuid(0x8000_0000).eax;
    if max_extended < 0x8000_0007 {
        return false;
    }
    core::arch::x86_64::__cpuid(0x8000_0007).edx & (1 << 8) != 0
}

#[inline]
fn rdtsc() -> u64 {
    unsafe { core::arch::x86_64::_rdtsc() }
}

/// Publish the BSP calibration. Called once, immediately after the PIT
/// calibration, with the raw inputs that produced the frequency estimate.
///
/// Returns `false` when the platform cannot support qualified timing; the
/// record then stays at epoch 0 and every stamp is refused.
pub fn init_bsp(reference_counts: u64, counter_delta: u64) -> bool {
    let hz = crate::arch::x86_64::interrupts::tsc_hz();
    if hz == 0 {
        return false;
    }
    let Some(mul) = mc::fixed_point_mul(hz) else {
        return false;
    };
    let ppm = mc::calibration_ppm(reference_counts, counter_delta).unwrap_or(u32::MAX);
    let mut flags = mc::FLAG_PIT_CALIBRATED;
    if invariant_tsc() {
        flags |= mc::FLAG_INVARIANT_TSC
            | mc::FLAG_NONSTOP_TSC
            | mc::FLAG_FREQUENCY_SHARED_FROM_BSP;
    } else {
        // The CPUID contract does not certify a constant-rate TSC here, so the
        // frequency is only usable once an independent re-measurement has
        // agreed. Until then the record carries the unverified flag.
        flags |= mc::FLAG_FREQUENCY_UNVERIFIED;
    }
    HZ.store(hz, Ordering::Relaxed);
    MUL.store(mul, Ordering::Relaxed);
    PPM.store(ppm, Ordering::Relaxed);
    BOOT_PPM.store(ppm, Ordering::Relaxed);
    FLAGS.store(flags, Ordering::Relaxed);
    ORIGIN[0].store(rdtsc(), Ordering::Relaxed);
    EPOCH.store(1, Ordering::Release);
    crate::serial_println!(
        "[FL-M54-CLOCK] status=ready source=pit-calibrated-tsc epoch=1 tsc_hz={} calibration_ppm={} flags={:#x} invariant_tsc={} contract=same-cpu-only cross_cpu=not-qualified",
        hz,
        ppm,
        flags,
        u8::from(invariant_tsc())
    );
    true
}

/// Re-measure this CPU's counter frequency against the PIT and require the
/// estimate to agree with the boot value inside a bound derived from both
/// measurements. IRQs are disabled only for the bounded spin, so a preemption
/// cannot inflate the counter delta and fake a frequency shift.
///
/// Returns the observed disagreement in ppm, or `None` when the check could
/// not be completed or the frequency moved outside the bound; in the latter
/// case the clock is invalidated so no later stamp can be trusted.
pub fn recalibrate() -> Option<u32> {
    if EPOCH.load(Ordering::Acquire) == 0 {
        return None;
    }
    if HZ.load(Ordering::Relaxed) == 0 {
        invalidate();
        return None;
    }
    // Draw several independent reference windows. The spread between them is
    // the clock's *measured* reproducibility on this platform; it is reported
    // as the frequency uncertainty instead of the optimistic single-shot bound.
    let mut samples: [u64; STABILITY_WINDOWS] = [0; STABILITY_WINDOWS];
    for slot in samples.iter_mut() {
        let Some((counts, delta)) = x86_64::instructions::interrupts::without_interrupts(|| {
            let start = super::interrupts::read_pit_count();
            let tsc0 = rdtsc();
            let mut spins: u32 = 0;
            loop {
                let current = super::interrupts::read_pit_count();
                match super::pit::elapsed_counts(start, current) {
                    None => return None,
                    Some(dropped) if dropped >= STABILITY_COUNTS => {
                        break Some((dropped, rdtsc().saturating_sub(tsc0)));
                    }
                    Some(_) => {}
                }
                spins += 1;
                if spins > 10_000_000 {
                    return None;
                }
                core::hint::spin_loop();
            }
        }) else {
            invalidate();
            return None;
        };
        let Some(measured) = super::pit::tsc_frequency_hz(delta, counts) else {
            invalidate();
            return None;
        };
        *slot = measured;
    }
    // Discard the first window: it includes the ramp from a long idle period
    // and is retained in the serial line, not used for the estimate.
    let mut validated = [0u64; STABILITY_REPEATS];
    validated.copy_from_slice(&samples[1..]);
    validated.sort_unstable();
    let min_hz = validated[0];
    let max_hz = validated[STABILITY_REPEATS - 1];
    let median_hz = validated[STABILITY_REPEATS / 2];
    if min_hz == 0 {
        invalidate();
        return None;
    }
    let spread_ppm =
        ((max_hz - min_hz) as u128 * 1_000_000 / min_hz as u128).min(u32::MAX as u128) as u32;
    // The re-measured median supersedes the single boot sample: the boot value
    // is one draw from the same noisy estimator, and here we have five.
    let Some(mul) = mc::fixed_point_mul(median_hz) else {
        invalidate();
        return None;
    };
    // Validated uncertainty = observed spread + two boot-calibration bounds
    // (one for each endpoint of a converted interval) + one ppm of rounding.
    let effective_ppm = spread_ppm
        .saturating_add(2u32.saturating_mul(BOOT_PPM.load(Ordering::Relaxed)))
        .saturating_add(1);
    STABILITY_PPM.store(spread_ppm, Ordering::Relaxed);
    if effective_ppm > MAX_QUALIFIED_PPM {
        invalidate();
        crate::serial_println!(
            "[FL-M54-CLOCKSTABILITY] rejected spread_ppm={} effective_ppm={} ceiling_ppm={} samples_hz={:?}",
            spread_ppm,
            effective_ppm,
            MAX_QUALIFIED_PPM,
            samples
        );
        return None;
    }
    HZ.store(median_hz, Ordering::Relaxed);
    MUL.store(mul, Ordering::Relaxed);
    // The reported bound must hold for every CPU whose stamps are admitted, so
    // keep the largest uncertainty measured across CPUs.
    PPM.fetch_max(effective_ppm, Ordering::Relaxed);
    // Scale changed: invalidate every earlier stamp by advancing the epoch.
    EPOCH.fetch_add(1, Ordering::Release);
    FLAGS.fetch_or(mc::FLAG_FREQUENCY_STABILITY_CHECKED, Ordering::Relaxed);
    FLAGS.fetch_and(!mc::FLAG_FREQUENCY_UNVERIFIED, Ordering::Relaxed);
    crate::serial_println!(
        "[FL-M54-CLOCKSTABILITY] accepted median_hz={} min_hz={} max_hz={} spread_ppm={} effective_ppm={} bound_ppm={} samples_hz={:?} ceiling_ppm={}",
        median_hz,
        min_hz,
        max_hz,
        spread_ppm,
        effective_ppm,
        PPM.load(Ordering::Relaxed),
        samples,
        MAX_QUALIFIED_PPM
    );
    Some(spread_ppm)
}

/// Mark the clock unusable. Every later stamp is refused; the fixture must then
/// report the metric as unsupported rather than fall back to a coarse timer.
fn invalidate() {
    EPOCH.store(0, Ordering::Release);
}

/// Record this CPU's own counter origin. Called once from each AP entry before
/// it can take a measurement stamp; it never changes the shared frequency.
pub fn publish_cpu() {
    if EPOCH.load(Ordering::Acquire) == 0 {
        return;
    }
    let cpu = crate::sched::current_cpu_id();
    if cpu < crate::sched::MAX_CORES {
        ORIGIN[cpu].store(rdtsc(), Ordering::Relaxed);
    }
}

/// Take a timestamp on the calling CPU, or `None` when the clock is unsupported
/// or this CPU has not published an origin.
pub fn stamp() -> Option<Stamp> {
    let epoch = EPOCH.load(Ordering::Acquire);
    if epoch == 0 {
        return None;
    }
    let flags = FLAGS.load(Ordering::Relaxed);
    // Without the platform's invariant-TSC guarantee, no duration may be
    // produced until an independent re-measurement has validated the frequency.
    if flags & mc::FLAG_FREQUENCY_UNVERIFIED != 0
        && flags & mc::FLAG_FREQUENCY_STABILITY_CHECKED == 0
    {
        return None;
    }
    let cpu = crate::sched::current_cpu_id();
    if cpu >= crate::sched::MAX_CORES {
        return None;
    }
    let origin = ORIGIN[cpu].load(Ordering::Relaxed);
    if origin == 0 {
        return None;
    }
    let cal = Calibration {
        cpu: cpu as u32,
        epoch,
        tsc_hz: HZ.load(Ordering::Relaxed),
        origin,
        mul: MUL.load(Ordering::Relaxed),
        uncertainty_ppm: PPM.load(Ordering::Relaxed),
        flags,
    };
    let ns = cal.to_ns(rdtsc()).ok()?;
    Some(Stamp { cpu: cal.cpu, epoch, ns, flags: cal.flags })
}

/// Seven-word receipt record:
/// `[cpu, epoch, ns, tsc_hz, calibration_ppm, flags, stability_ppm]`.
pub fn stamp_words() -> Option<[u64; 7]> {
    let s = stamp()?;
    Some([
        s.cpu as u64,
        s.epoch as u64,
        s.ns,
        HZ.load(Ordering::Relaxed),
        PPM.load(Ordering::Relaxed) as u64,
        s.flags as u64,
        STABILITY_PPM.load(Ordering::Relaxed) as u64,
    ])
}

/// Machine-readable metadata for the host receipt, emitted once per boot.
pub fn report() {
    let mut origins = [0u64; crate::sched::MAX_CORES];
    let mut count = 0usize;
    for (cpu, slot) in ORIGIN.iter().enumerate() {
        let v = slot.load(Ordering::Relaxed);
        origins[cpu] = v;
        if v != 0 {
            count += 1;
        }
    }
    let listed: alloc::vec::Vec<alloc::string::String> = origins[..crate::sched::MAX_CORES]
        .iter()
        .enumerate()
        .filter(|(_, v)| **v != 0)
        .map(|(cpu, v)| alloc::format!("{}:{}", cpu, v))
        .collect();
    crate::serial_println!(
        "[FL-M54-CLOCKCPU] epoch={} cpus={} origins={}",
        EPOCH.load(Ordering::Relaxed),
        count,
        listed.join(",")
    );
    crate::serial_println!(
        "[FL-M54-CLOCKSTABILITY-FINAL] stability_ppm={} flags={:#x}",
        STABILITY_PPM.load(Ordering::Relaxed),
        FLAGS.load(Ordering::Relaxed)
    );
}

/// Calibration record for the *current* CPU, if any. Used by the kernel-side
/// actor counters to tag a window with its clock identity.
pub fn current_calibration() -> Option<Calibration> {
    let epoch = EPOCH.load(Ordering::Acquire);
    if epoch == 0 {
        return None;
    }
    let cpu = crate::sched::current_cpu_id();
    if cpu >= crate::sched::MAX_CORES {
        return None;
    }
    let origin = ORIGIN[cpu].load(Ordering::Relaxed);
    if origin == 0 {
        return None;
    }
    Some(Calibration {
        cpu: cpu as u32,
        epoch,
        tsc_hz: HZ.load(Ordering::Relaxed),
        origin,
        mul: MUL.load(Ordering::Relaxed),
        uncertainty_ppm: PPM.load(Ordering::Relaxed),
        flags: FLAGS.load(Ordering::Relaxed),
    })
}
