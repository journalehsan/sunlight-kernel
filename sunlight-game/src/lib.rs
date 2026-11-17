//! Shared game foundation. The native host remains sunlight-ui::Window/App.
//! Callers supply time; no platform clock or gameplay decisions live here.
#![no_std]

/// Presentation gate used by Silicon Echoes and the RTS host. Preserves the
/// existing Echoes rule: one refresh after an interval, then reset to now.
pub fn frame_due(now_ms: u64, last_ms: u64, interval_ms: u64) -> bool {
    now_ms.saturating_sub(last_ms) >= interval_ms
}

/// Fixed-step host adapter, with bounded work after a stalled GUI poll.
/// Paused wall time is discarded, never turned into variable simulation dt.
pub struct FixedStep {
    last_ms: u64,
    debt_ms: u64,
    step_ms: u64,
}
impl FixedStep {
    pub fn new(now_ms: u64, step_ms: u64) -> Self {
        assert!(step_ms > 0);
        Self {
            last_ms: now_ms,
            debt_ms: 0,
            step_ms,
        }
    }
    pub fn advance(&mut self, now_ms: u64) -> u32 {
        self.debt_ms += now_ms
            .saturating_sub(self.last_ms)
            .min(self.step_ms.saturating_mul(5));
        self.last_ms = now_ms;
        let steps = self.debt_ms / self.step_ms;
        self.debt_ms %= self.step_ms;
        steps as u32
    }
    pub fn reset(&mut self, now_ms: u64) {
        self.last_ms = now_ms;
        self.debt_ms = 0;
    }
}

/// Integer camera in map tiles, deliberately independent of renderer types.
#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub x: i32,
    pub y: i32,
    pub tile_px: i32,
}
impl Camera {
    pub fn screen_to_tile(&self, x: i32, y: i32, top: i32) -> (i32, i32) {
        (
            self.x + x.div_euclid(self.tile_px),
            self.y + (y - top).div_euclid(self.tile_px),
        )
    }
    pub fn tile_to_screen(&self, x: i32, y: i32, top: i32) -> (i32, i32) {
        (
            (x - self.x) * self.tile_px,
            (y - self.y) * self.tile_px + top,
        )
    }
    pub fn clamp(&mut self, map_w: i32, map_h: i32, view_w: i32, view_h: i32) {
        self.x = self.x.clamp(0, (map_w - view_w / self.tile_px).max(0));
        self.y = self.y.clamp(0, (map_h - view_h / self.tile_px).max(0));
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn presentation_compatibility() {
        for last in [0, 50, 1000, u64::MAX - 100] {
            for now in [0, 50, 89, 90, 1000, u64::MAX] {
                assert_eq!(frame_due(now, last, 90), now.saturating_sub(last) >= 90);
            }
        }
    }
    #[test]
    fn fixed_steps_and_stall() {
        let mut s = FixedStep::new(100, 50);
        assert_eq!(s.advance(149), 0);
        assert_eq!(s.advance(150), 1);
        assert_eq!(s.advance(260), 2);
        assert_eq!(s.advance(5000), 5);
        s.reset(6000);
        assert_eq!(s.advance(6001), 0);
    }
    #[test]
    fn camera_roundtrip_and_bounds() {
        let mut c = Camera {
            x: 3,
            y: 6,
            tile_px: 24,
        };
        assert_eq!(c.screen_to_tile(0, 40, 40), (3, 6));
        assert_eq!(c.tile_to_screen(3, 6, 40), (0, 40));
        assert_eq!(c.screen_to_tile(-1, 39, 40), (2, 5));
        c.x = 90;
        c.y = -5;
        c.clamp(64, 64, 640, 480);
        assert_eq!((c.x, c.y), (38, 0));
    }
}
