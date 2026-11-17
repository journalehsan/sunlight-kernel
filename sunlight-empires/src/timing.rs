//! Bounded scheduling preserves tick debt; speed never changes simulation dt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Speed {
    Paused,
    Half,
    Normal,
    Double,
}
pub struct Clock {
    last: u64,
    debt: u64,
    pub speed: Speed,
}
impl Clock {
    pub fn new(now: u64) -> Self {
        Self {
            last: now,
            debt: 0,
            speed: Speed::Normal,
        }
    }
    pub fn set_speed(&mut self, now: u64, speed: Speed) {
        self.last = now;
        self.speed = speed;
    }
    pub fn advance(&mut self, now: u64) -> u32 {
        let elapsed = now.saturating_sub(self.last);
        self.last = now;
        let factor = match self.speed {
            Speed::Paused => return 0,
            Speed::Half => 1,
            Speed::Normal => 2,
            Speed::Double => 4,
        };
        self.debt = self.debt.saturating_add(elapsed.saturating_mul(factor));
        let steps = (self.debt / 100).min(5);
        self.debt -= steps * 100;
        steps as u32
    }
    pub fn reset(&mut self, now: u64) {
        *self = Self::new(now);
    }
}
