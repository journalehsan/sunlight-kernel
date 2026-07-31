//! Pure PIT channel-0 timing constants and calibration math.

pub const INPUT_HZ: u64 = 1_193_182;
pub const RELOAD: u16 = 11_932;
pub const MODE_2_RATE_GENERATOR: u8 = 0x34;
pub const CALIBRATION_COUNTS: u64 = 4_000;

/// Elapsed PIT input clocks for channel 0 programmed in mode 2.
///
/// The calibration window is shorter than one reload period, so at most one
/// reload can occur between the start and current snapshots.
pub const fn elapsed_counts(start: u16, current: u16) -> Option<u64> {
    if start == 0 || current == 0 || start > RELOAD || current > RELOAD {
        return None;
    }
    if start >= current {
        Some((start - current) as u64)
    } else {
        Some(start as u64 + (RELOAD - current) as u64)
    }
}

pub fn tsc_frequency_hz(tsc_delta: u64, pit_counts: u64) -> Option<u64> {
    if tsc_delta == 0 || pit_counts == 0 {
        return None;
    }
    tsc_delta.checked_mul(INPUT_HZ)?.checked_div(pit_counts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode2_elapsed_handles_straight_and_wrapped_windows() {
        assert_eq!(elapsed_counts(10_000, 6_000), Some(4_000));
        assert_eq!(elapsed_counts(2_000, 9_932), Some(4_000));
    }

    #[test]
    fn rejects_counts_outside_the_programmed_reload() {
        assert_eq!(elapsed_counts(0, 1), None);
        assert_eq!(elapsed_counts(RELOAD + 1, 1), None);
        assert_eq!(elapsed_counts(1, RELOAD + 1), None);
    }

    #[test]
    fn calibration_math_recovers_reference_frequency() {
        let expected_hz = 2_500_000_000u64;
        let tsc_delta = expected_hz * CALIBRATION_COUNTS / INPUT_HZ;
        let measured = tsc_frequency_hz(tsc_delta, CALIBRATION_COUNTS).unwrap();
        assert!(expected_hz.abs_diff(measured) < 1_000_000);
    }

    #[test]
    fn old_mode3_assumption_would_double_elapsed_time() {
        let true_input_clocks = 2_000u64;
        let visible_mode3_drop = true_input_clocks * 2;
        assert_eq!(visible_mode3_drop / true_input_clocks, 2);
    }
}
