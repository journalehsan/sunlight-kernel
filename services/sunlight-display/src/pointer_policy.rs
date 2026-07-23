//! Shared fixed-point pointer-motion policy.

const FP_SHIFT: i32 = 16;
const FP_ONE: i32 = 1 << FP_SHIFT;

const POINTER_SENSITIVITY_FP: i32 = FP_ONE * 9 / 8;
const POINTER_ACCELERATION_ENABLED: bool = true;
const POINTER_ACCEL_START_MAGNITUDE: i32 = 2;
const POINTER_ACCEL_FACTOR_FP: i32 = FP_ONE / 20;
const POINTER_MAX_ACCEL_GAIN_FP: i32 = FP_ONE * 11 / 8;
const POINTER_MAX_DELTA_PX: i32 = 24;
const EDGE_MARGIN: i32 = 0;

#[derive(Clone, Copy)]
struct PointerMotionConfig {
    sensitivity_fp: i32,
    acceleration_enabled: bool,
    acceleration_factor_fp: i32,
    accel_start_magnitude: i32,
    max_accel_gain_fp: i32,
    max_delta_fp: i32,
}

impl PointerMotionConfig {
    const fn moderate_default() -> Self {
        Self {
            sensitivity_fp: POINTER_SENSITIVITY_FP,
            acceleration_enabled: POINTER_ACCELERATION_ENABLED,
            acceleration_factor_fp: POINTER_ACCEL_FACTOR_FP,
            accel_start_magnitude: POINTER_ACCEL_START_MAGNITUDE,
            max_accel_gain_fp: POINTER_MAX_ACCEL_GAIN_FP,
            max_delta_fp: POINTER_MAX_DELTA_PX << FP_SHIFT,
        }
    }
}

#[derive(Clone, Copy)]
pub struct MotionOutcome {
    pub final_dx: i32,
    pub final_dy: i32,
    pub delta_capped: bool,
    pub position_clamped: bool,
}

pub struct PointerPolicy {
    x_fp: i32,
    y_fp: i32,
    buttons: u8,
    fb_width: u32,
    fb_height: u32,
    motion: PointerMotionConfig,
}

impl PointerPolicy {
    pub fn new(fb_w: u32, fb_h: u32) -> Self {
        let cx = ((fb_w as i32 / 2).max(0)) << FP_SHIFT;
        let cy = ((fb_h as i32 / 2).max(0)) << FP_SHIFT;
        Self {
            x_fp: cx,
            y_fp: cy,
            buttons: 0,
            fb_width: fb_w,
            fb_height: fb_h,
            motion: PointerMotionConfig::moderate_default(),
        }
    }

    fn acceleration_gain_fp(&self, magnitude: i32) -> i32 {
        // The curve is intentionally stateless: each batch is scaled only from
        // its current |dx|+|dy|. Small motion stays close to 1:1, while larger
        // sweeps get a modest linear boost up to a hard ceiling. Because the
        // gain uses no velocity history, movement stops immediately when fresh
        // hardware deltas stop.
        if !self.motion.acceleration_enabled || magnitude <= self.motion.accel_start_magnitude {
            return FP_ONE;
        }

        let extra = magnitude.saturating_sub(self.motion.accel_start_magnitude) as i64;
        let max_bonus = (self.motion.max_accel_gain_fp - FP_ONE).max(0) as i64;
        let bonus_fp = (extra * self.motion.acceleration_factor_fp as i64).min(max_bonus);
        (FP_ONE as i64 + bonus_fp) as i32
    }

    pub fn apply_motion(&mut self, dx: i32, dy: i32, buttons: u8) -> MotionOutcome {
        let prev_x = self.x();
        let prev_y = self.y();
        self.buttons = buttons;

        if dx == 0 && dy == 0 {
            return MotionOutcome {
                final_dx: 0,
                final_dy: 0,
                delta_capped: false,
                position_clamped: false,
            };
        }

        let magnitude = dx.abs().saturating_add(dy.abs());
        let accel_gain_fp = self.acceleration_gain_fp(magnitude) as i64;
        let total_gain_fp = ((self.motion.sensitivity_fp as i64) * accel_gain_fp) >> FP_SHIFT;
        let mut move_x_fp = (dx as i64) * total_gain_fp;
        let mut move_y_fp = (dy as i64) * total_gain_fp;

        let max_delta_fp = self.motion.max_delta_fp as i64;
        let mut delta_capped = false;
        if move_x_fp > max_delta_fp {
            move_x_fp = max_delta_fp;
            delta_capped = true;
        } else if move_x_fp < -max_delta_fp {
            move_x_fp = -max_delta_fp;
            delta_capped = true;
        }
        if move_y_fp > max_delta_fp {
            move_y_fp = max_delta_fp;
            delta_capped = true;
        } else if move_y_fp < -max_delta_fp {
            move_y_fp = -max_delta_fp;
            delta_capped = true;
        }

        self.x_fp = self.x_fp.saturating_add(move_x_fp as i32);
        self.y_fp = self.y_fp.saturating_add(move_y_fp as i32);
        let position_clamped = self.sync_clamp();

        MotionOutcome {
            final_dx: self.x() - prev_x,
            final_dy: self.y() - prev_y,
            delta_capped,
            position_clamped,
        }
    }

    pub fn set_motion_settings(&mut self, sensitivity_fp: i32, acceleration_enabled: bool) {
        if sensitivity_fp > 0 {
            self.motion.sensitivity_fp = sensitivity_fp;
        }
        self.motion.acceleration_enabled = acceleration_enabled;
    }

    fn min_fp(&self) -> i32 {
        EDGE_MARGIN << FP_SHIFT
    }

    fn max_x_fp(&self) -> i32 {
        ((self.fb_width as i32 - 1 - EDGE_MARGIN).max(EDGE_MARGIN)) << FP_SHIFT
    }

    fn max_y_fp(&self) -> i32 {
        ((self.fb_height as i32 - 1 - EDGE_MARGIN).max(EDGE_MARGIN)) << FP_SHIFT
    }

    fn sync_clamp(&mut self) -> bool {
        let before_x = self.x_fp;
        let before_y = self.y_fp;
        self.x_fp = self.x_fp.clamp(self.min_fp(), self.max_x_fp());
        self.y_fp = self.y_fp.clamp(self.min_fp(), self.max_y_fp());
        self.x_fp != before_x || self.y_fp != before_y
    }

    pub fn x(&self) -> i32 {
        (self.x_fp >> FP_SHIFT)
            .max(0)
            .min((self.fb_width - 1) as i32)
    }

    pub fn y(&self) -> i32 {
        fixed_y_coordinate_to_pixel(self.y_fp)
            .max(0)
            .min((self.fb_height - 1) as i32)
    }
}

/// Convert a clamped non-negative Q16.16 coordinate to the nearest pixel.
/// Arithmetic right shift floors, which biases equal negative deltas by one
/// pixel whenever a move leaves a fractional coordinate. Nearest rounding
/// keeps positive and negative relative motion symmetric without filtering or
/// changing the accumulated sub-pixel value.
fn fixed_y_coordinate_to_pixel(value_fp: i32) -> i32 {
    value_fp.saturating_add(FP_ONE / 2) >> FP_SHIFT
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pointer_motion_rounding_is_symmetric_for_slow_usb_deltas() {
        let mut down = PointerPolicy::new(800, 600);
        let mut up = PointerPolicy::new(800, 600);

        let down_motion = down.apply_motion(0, 1, 0);
        let up_motion = up.apply_motion(0, -1, 0);

        assert_eq!(down_motion.final_dy, 1);
        assert_eq!(up_motion.final_dy, -1);
        assert_eq!(down.y() - 300, 300 - up.y());
    }

    #[test]
    fn shared_pointer_path_does_not_invert_driver_y() {
        let mut pointer = PointerPolicy::new(800, 600);
        let start = pointer.y();
        assert!(pointer.apply_motion(0, -8, 0).final_dy < 0);
        assert!(pointer.y() < start);

        let before_down = pointer.y();
        assert!(pointer.apply_motion(0, 8, 0).final_dy > 0);
        assert!(pointer.y() > before_down);
    }
}
