//! Union of in-flight intervals, with an explicit monotonic clock supplied by the caller.

#[derive(Clone, Copy, Default)]
pub struct BusyTime {
    active: usize,
    since: u64,
    elapsed: u64,
}

impl BusyTime {
    pub const fn new() -> Self {
        Self {
            active: 0,
            since: 0,
            elapsed: 0,
        }
    }

    pub fn begin(&mut self, now: u64) {
        if self.active == 0 {
            self.since = now;
        }
        self.active = self.active.checked_add(1).expect("in-flight overflow");
    }

    pub fn finish(&mut self, now: u64) {
        let elapsed = self.snapshot(now);
        self.active = self.active.checked_sub(1).expect("unpaired retirement");
        if self.active == 0 {
            self.elapsed = elapsed;
        }
    }

    pub fn snapshot(&self, now: u64) -> u64 {
        if self.active == 0 {
            self.elapsed
        } else {
            self.elapsed.saturating_add(now.saturating_sub(self.since))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::BusyTime;

    #[test]
    fn idle_single_and_overlap() {
        let mut busy = BusyTime::new();
        assert_eq!(busy.snapshot(5), 0);
        busy.begin(10);
        assert_eq!(busy.snapshot(12), 2);
        busy.finish(17);
        assert_eq!(busy.snapshot(25), 7);
        busy.begin(30);
        busy.begin(33);
        assert_eq!(busy.snapshot(36), 13);
        busy.finish(38);
        assert_eq!(busy.snapshot(40), 17);
        busy.finish(43);
        assert_eq!(busy.snapshot(90), 20);
    }
}
