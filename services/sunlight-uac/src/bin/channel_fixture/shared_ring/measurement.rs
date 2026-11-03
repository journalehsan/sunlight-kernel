//! Actor-private observation state: no shared atomics, locks or heap storage.
//! Counters fail closed on overflow; a failed observation cannot become success.
#[derive(Debug)]
pub struct Counters {
    pub compact: bool,
    measuring: bool,
    finished: bool,
    pub count: u64,
    pub bytes: u64,
    pub full: u64,
    pub overflow: bool,
    pub order_valid: bool,
    pub start_sequence: u64,
    pub start_ms: u64,
    pub end_ms: u64,
    pub samples: [u64; 64],
    pub samples_len: usize,
    pub dropped: u64,
}
impl Counters {
    pub const fn new() -> Self {
        Self {
            compact: false,
            measuring: false,
            finished: false,
            count: 0,
            bytes: 0,
            full: 0,
            overflow: false,
            order_valid: true,
            start_sequence: 0,
            start_ms: 0,
            end_ms: 0,
            samples: [0; 64],
            samples_len: 0,
            dropped: 0,
        }
    }
    pub fn begin(&mut self, next: u64, measuring: bool, now: u64) {
        assert!(!self.finished && (!self.measuring || !measuring));
        *self = Self {
            compact: true,
            measuring,
            start_sequence: next,
            start_ms: now,
            ..Self::new()
        };
    }
    fn add(value: &mut u64, amount: u64, overflow: &mut bool) {
        if let Some(next) = value.checked_add(amount) {
            *value = next;
        } else {
            *overflow = true;
        }
    }
    pub fn wants_sample(&self) -> bool {
        self.measuring && self.samples_len < 64
    }
    pub fn success(&mut self, sequence: u64, bytes: u64, timestamp: Option<u64>) {
        if !self.measuring {
            return;
        }
        if let Some(now) = timestamp {
            if self.samples_len < 64 {
                self.samples[self.samples_len] = now;
                self.samples_len += 1;
            } else {
                Self::add(&mut self.dropped, 1, &mut self.overflow);
            }
        } else {
            Self::add(&mut self.dropped, 1, &mut self.overflow);
        }
        self.order_valid &= self.start_sequence.checked_add(self.count) == Some(sequence);
        Self::add(&mut self.count, 1, &mut self.overflow);
        Self::add(&mut self.bytes, bytes, &mut self.overflow);
    }
    pub fn full(&mut self) {
        if self.measuring {
            Self::add(&mut self.full, 1, &mut self.overflow);
        }
    }
    pub fn finish(&mut self, now: u64) {
        assert!(self.measuring && !self.finished);
        self.end_ms = now;
        self.measuring = false;
        self.finished = true;
    }
    #[cfg(not(test))]
    pub fn report(&self, generation: u64, next: u64) {
        // Only once per completed actor window, outside the transfer path.
        sunlight_ipc::debug_log(&alloc::format!("[FL-M53-ACTOR] generation={} pid={} start_sequence={} next={} count={} bytes={} full={} overflow={} dropped={} sample_count={} order={} start_ms={} end_ms={} clock=kernel-bsp-ms resolution_ms=10 coherent=0 authentic=0", generation, sunlight_libc::getpid(), self.start_sequence,next,self.count,self.bytes,self.full,u8::from(self.overflow),self.dropped,self.samples_len,u8::from(self.order_valid),self.start_ms,self.end_ms));
        // DebugLog accepts at most 256 bytes. Four timestamps per row remain
        // within that ABI limit even at the supported clock horizon.
        for (chunk, values) in self.samples[..self.samples_len].chunks(4).enumerate() {
            let times = values
                .iter()
                .map(|v| alloc::format!("{}", v))
                .collect::<alloc::vec::Vec<_>>()
                .join(",");
            sunlight_ipc::debug_log(&alloc::format!(
                "[FL-M53-SAMPLES] generation={} pid={} first={} times={}",
                generation,
                sunlight_libc::getpid(),
                chunk * 4,
                times
            ));
        }
    }
}
#[cfg(not(test))]
pub fn clock_probe(cpu: u64) {
    super::super::doorbell::pin(cpu);
    let begin = sunlight_ipc::monotonic_millis();
    let mut last = begin;
    let mut regressions = 0u64;
    for _ in 0..256 {
        let now = sunlight_ipc::monotonic_millis();
        regressions += u64::from(now < last);
        last = now;
    }
    sunlight_ipc::debug_log(&alloc::format!("[FL-M53-CLOCK] pid={} cpu={} reads=257 begin_ms={} end_ms={} regressions={} resolution_ms=10 source=kernel-bsp-ms", sunlight_libc::getpid(),cpu,begin,last,regressions));
    assert_eq!(regressions, 0);
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn independent_windows_and_exact_counts() {
        let mut p = Counters::new();
        let mut c = Counters::new();
        p.begin(0, false, 10);
        p.success(0, 8, None);
        assert_eq!(p.count, 0);
        p.begin(16, true, 20);
        c.begin(16, true, 20);
        p.success(16, 23, Some(20));
        p.full();
        c.success(16, 23, Some(30));
        p.finish(30);
        c.finish(30);
        assert_eq!((p.count, p.bytes, p.full), (1, 23, 1));
        assert_eq!((c.count, c.bytes, c.full), (1, 23, 0));
        assert!(p.order_valid && c.order_valid && !p.overflow);
    }
    #[test]
    fn overflow_and_order_are_explicit() {
        let mut p = Counters::new();
        p.begin(u64::MAX, true, 0);
        p.count = u64::MAX;
        p.bytes = u64::MAX;
        p.full = u64::MAX;
        p.success(0, 1, None);
        p.full();
        assert!(p.overflow && !p.order_valid);
        assert_eq!(p.count, u64::MAX);
    }
    #[test]
    fn bounded_samples_report_drops() {
        let mut p = Counters::new();
        p.begin(0, true, 0);
        for seq in 0..100 {
            let timestamp = if p.wants_sample() { Some(seq) } else { None };
            p.success(seq, 1, timestamp);
        }
        p.finish(101);
        assert_eq!((p.count, p.samples_len, p.dropped), (100, 64, 36));
        assert!(p.order_valid && !p.overflow);
    }
    #[test]
    #[should_panic]
    fn duplicate_finish_rejected() {
        let mut p = Counters::new();
        p.begin(0, true, 0);
        p.finish(1);
        p.finish(2);
    }
}
