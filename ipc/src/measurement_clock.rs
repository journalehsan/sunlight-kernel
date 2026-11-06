//! Integer-only clock qualification for native IPC measurement (FastLane M5.4).
//!
//! This module is the single place that decides whether a pair of raw counter
//! readings may be turned into a duration. It is deliberately stricter than the
//! kernel's public monotonic clock:
//!
//! * A timestamp carries the identity of the CPU that read it. Subtracting two
//!   timestamps taken on different CPUs is rejected unless a validated
//!   cross-CPU offset exists, and this kernel does not establish one.
//! * A calibration carries an explicit epoch. A timestamp from a different
//!   epoch (re-calibration, reboot, or an invalid record) is rejected.
//! * Conversion is overflow-checked. Counter values beyond the calibrated
//!   horizon are rejected rather than silently truncated.
//!
//! No allocation, no locks, no floating point. Everything here is usable from
//! both the kernel and a Ring 3 fixture so the rejection rules are exercised by
//! the same code that produces native stamps.

/// CPUID.8000_0007H:EDX[8] — TSC runs at a constant rate and is not reset by
/// P/C-state transitions. It does *not* promise cross-core synchronisation.
pub const FLAG_INVARIANT_TSC: u32 = 1 << 0;
/// Architectural TSC deadline / non-stop behaviour advertised by the platform.
pub const FLAG_NONSTOP_TSC: u32 = 1 << 1;
/// Calibration reference was the PIT channel-0 rate generator.
pub const FLAG_PIT_CALIBRATED: u32 = 1 << 2;
/// Calibration was derived on the BSP and reused for other CPUs. Valid only
/// when `FLAG_INVARIANT_TSC` is set; otherwise per-CPU frequency is unverified.
pub const FLAG_FREQUENCY_SHARED_FROM_BSP: u32 = 1 << 3;
/// The platform did not advertise an invariant TSC, so per-CPU frequency is
/// not certified by the CPUID contract.
pub const FLAG_FREQUENCY_UNVERIFIED: u32 = 1 << 4;
/// A second, independent PIT-referenced frequency estimate agreed with the
/// boot estimate inside the reported bound. This is a *stability* check against
/// a re-measured reference; it does not certify the PIT's absolute frequency.
pub const FLAG_FREQUENCY_STABILITY_CHECKED: u32 = 1 << 5;

/// Fixed-point shift used by [`Calibration::to_ns`]: `(dt * mul) >> SHIFT`.
pub const SHIFT: u32 = 32;

/// A rejection reason. Every variant is a deliberate, deterministic refusal to
/// produce a number; none of them is ever coerced into a duration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClockError {
    /// The calibration record is absent, zeroed, or has no epoch.
    Uncalibrated,
    /// Two stamps came from different calibration epochs.
    StaleEpoch,
    /// `b` precedes `a`; the counter moved backwards.
    Backward,
    /// Two stamps were taken on different CPUs. Cross-CPU subtraction is not
    /// qualified on this kernel.
    CrossCpu,
    /// The raw counter is not ahead of the calibration origin.
    BeforeOrigin,
    /// The conversion would exceed `u64`; the reading is outside the horizon.
    Overflow,
}

/// A per-CPU calibration record for one counter domain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Calibration {
    pub cpu: u32,
    /// Nonzero only for a usable record; bumped on every (re)calibration.
    pub epoch: u32,
    pub tsc_hz: u64,
    pub origin: u64,
    /// `(dt * mul) >> SHIFT` yields nanoseconds.
    pub mul: u64,
    /// Calibration-frequency relative error in parts per million. This bounds
    /// the *scale* error of any converted interval, not per-read jitter.
    pub uncertainty_ppm: u32,
    pub flags: u32,
}

impl Calibration {
    pub const INVALID: Self = Self {
        cpu: 0,
        epoch: 0,
        tsc_hz: 0,
        origin: 0,
        mul: 0,
        uncertainty_ppm: 0,
        flags: 0,
    };

    /// A record is usable only with a nonzero epoch, frequency and multiplier.
    pub const fn is_valid(&self) -> bool {
        self.epoch != 0 && self.tsc_hz != 0 && self.mul != 0
    }

    /// Largest raw counter delta that converts without overflowing `u64`.
    /// Readings beyond this are rejected as [`ClockError::Overflow`].
    pub const fn horizon_raw(&self) -> u64 {
        if self.mul == 0 {
            return 0;
        }
        // dt_max = (u64::MAX << SHIFT) / mul, clamped to the counter width.
        let dt = ((u64::MAX as u128) << SHIFT) / (self.mul as u128);
        if dt >= u64::MAX as u128 {
            u64::MAX
        } else {
            dt as u64
        }
    }

    /// Convert an absolute raw counter reading to nanoseconds since the
    /// calibration origin.
    pub fn to_ns(&self, counter: u64) -> Result<u64, ClockError> {
        if !self.is_valid() {
            return Err(ClockError::Uncalibrated);
        }
        let dt = counter
            .checked_sub(self.origin)
            .ok_or(ClockError::BeforeOrigin)?;
        // `dt` and `mul` are u64: the product fits u128, so the intermediate
        // cannot overflow; only the final narrowing to u64 can.
        let ns = ((dt as u128) * (self.mul as u128)) >> SHIFT;
        u64::try_from(ns).map_err(|_| ClockError::Overflow)
    }

    /// Upper bound on the scale error of an interval of `ns` nanoseconds.
    /// Rounds up and always adds one nanosecond of unit quantization.
    pub fn uncertainty_ns(&self, ns: u64) -> u64 {
        let ppm = self.uncertainty_ppm as u128;
        let scaled = ((ns as u128) * ppm).div_ceil(1_000_000);
        (scaled as u64).saturating_add(1)
    }

    /// Serialisable identity of the clock source for a receipt.
    pub fn source_flags(&self) -> u32 {
        self.flags
    }
}

/// A timestamp is inseparable from the CPU and calibration that produced it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    pub cpu: u32,
    pub epoch: u32,
    pub ns: u64,
    pub flags: u32,
}

/// Subtract two stamps, refusing every case the kernel cannot justify.
pub fn elapsed_ns(start: &Stamp, end: &Stamp) -> Result<u64, ClockError> {
    if start.epoch == 0 || end.epoch == 0 {
        return Err(ClockError::Uncalibrated);
    }
    if start.epoch != end.epoch {
        return Err(ClockError::StaleEpoch);
    }
    if start.cpu != end.cpu {
        return Err(ClockError::CrossCpu);
    }
    end.ns.checked_sub(start.ns).ok_or(ClockError::Backward)
}

/// Relative calibration error in whole parts per million, rounded up.
///
/// The estimate compares a raw counter delta against a reference count delta.
/// `reference_counts` is the least-quantized reference interval (one count of
/// uncertainty); `counter_delta` contributes its own single-count rounding.
pub fn calibration_ppm(reference_counts: u64, counter_delta: u64) -> Option<u32> {
    if reference_counts == 0 || counter_delta == 0 {
        return None;
    }
    // relative error ~= 1/reference_counts + 1/counter_delta
    let numerator = 1_000_000u128;
    let from_reference = numerator.div_ceil(reference_counts as u128);
    let from_counter = numerator.div_ceil(counter_delta as u128);
    let total = from_reference + from_counter;
    Some(total.min(u32::MAX as u128) as u32)
}

/// Build the fixed-point multiplier for a counter frequency.
pub fn fixed_point_mul(tsc_hz: u64) -> Option<u64> {
    if tsc_hz == 0 {
        return None;
    }
    let mul = ((1_000_000_000u128) << SHIFT) / (tsc_hz as u128);
    if mul == 0 || mul >= (1u128 << 63) {
        return None;
    }
    Some(mul as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cal(hz: u64) -> Calibration {
        Calibration {
            cpu: 0,
            epoch: 1,
            tsc_hz: hz,
            origin: 1_000,
            mul: fixed_point_mul(hz).unwrap(),
            uncertainty_ppm: 100,
            flags: FLAG_INVARIANT_TSC | FLAG_PIT_CALIBRATED,
        }
    }

    #[test]
    fn conversion_is_linear_and_monotonic() {
        let c = cal(1_000_000_000);
        assert_eq!(c.to_ns(1_000), Ok(0));
        assert_eq!(c.to_ns(2_000), Ok(1_000));
        assert_eq!(c.to_ns(1_001_000), Ok(1_000_000));
        assert!(c.to_ns(2_000).unwrap() > c.to_ns(1_500).unwrap());
    }

    #[test]
    fn invalid_calibration_is_rejected() {
        assert_eq!(
            Calibration::INVALID.to_ns(10),
            Err(ClockError::Uncalibrated)
        );
        let mut zero_mul = cal(1_000_000_000);
        zero_mul.mul = 0;
        assert!(!zero_mul.is_valid());
        assert_eq!(zero_mul.to_ns(1_000), Err(ClockError::Uncalibrated));
    }

    #[test]
    fn backward_counter_is_rejected_not_saturated() {
        let c = cal(1_000_000_000);
        assert_eq!(c.to_ns(999), Err(ClockError::BeforeOrigin));
    }

    #[test]
    fn overflow_beyond_the_horizon_is_rejected() {
        // 500 MHz => one counter is 2 ns, so dt * mul would exceed u64 well
        // before the raw counter wraps.
        let c = cal(500_000_000);
        assert!(c.horizon_raw() > 0 && c.horizon_raw() < u64::MAX);
        assert_eq!(c.to_ns(u64::MAX), Err(ClockError::Overflow));
    }

    #[test]
    fn elapsed_rejects_cross_cpu_and_backward() {
        let a = Stamp {
            cpu: 0,
            epoch: 1,
            ns: 100,
            flags: 0,
        };
        let b = Stamp {
            cpu: 0,
            epoch: 1,
            ns: 260,
            flags: 0,
        };
        let other = Stamp {
            cpu: 1,
            epoch: 1,
            ns: 260,
            flags: 0,
        };
        assert_eq!(elapsed_ns(&a, &b), Ok(160));
        assert_eq!(elapsed_ns(&a, &other), Err(ClockError::CrossCpu));
        assert_eq!(elapsed_ns(&b, &a), Err(ClockError::Backward));
    }

    #[test]
    fn elapsed_rejects_epoch_change_and_uncalibrated() {
        let a = Stamp {
            cpu: 0,
            epoch: 1,
            ns: 10,
            flags: 0,
        };
        let stale = Stamp {
            cpu: 0,
            epoch: 2,
            ns: 20,
            flags: 0,
        };
        let none = Stamp {
            cpu: 0,
            epoch: 0,
            ns: 20,
            flags: 0,
        };
        assert_eq!(elapsed_ns(&a, &stale), Err(ClockError::StaleEpoch));
        assert_eq!(elapsed_ns(&a, &none), Err(ClockError::Uncalibrated));
    }

    #[test]
    fn uncertainty_rounds_up_and_covers_sub_ppm_intervals() {
        let c = cal(1_000_000_000);
        assert_eq!(c.uncertainty_ns(0), 1);
        assert_eq!(c.uncertainty_ns(10_000), 2); // 100 ppm of 10us = 1ns, +1
        assert_eq!(c.uncertainty_ns(1_000_000), 101);
    }

    #[test]
    fn calibration_ppm_is_bounded_by_measurement_quantization() {
        assert_eq!(calibration_ppm(0, 10), None);
        // 4,000 PIT counts dominate the error: 250 ppm, plus 1 ppm from the
        // multi-million-count counter delta.
        assert_eq!(calibration_ppm(4_000, 21_000_000), Some(251));
        assert_eq!(calibration_ppm(4_000, 0), None);
    }

    #[test]
    fn fixed_point_mul_rejects_absurd_frequencies() {
        assert_eq!(fixed_point_mul(0), None);
        // Absurdly slow counters would need a multiplier where dt*mul could
        // overflow the u128 intermediate; those are refused.
        assert_eq!(fixed_point_mul(u64::MAX), None);
        assert!(fixed_point_mul(1_000_000_000).is_some());
    }
}
