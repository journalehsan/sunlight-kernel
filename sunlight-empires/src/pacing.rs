//! Guest wall-clock instrumentation; milliseconds are the platform clock's
//! resolution. Frame intervals include Window commit/poll and compositor waits.
use sunlight_ipc::{debug_log, monotonic_millis};
pub struct Pacing {
    intervals: [u32; 2001],
    draw: [u32; 2001],
    sim: [u32; 2001],
    last: u64,
    max: u64,
    frames: u32,
    ticks: u32,
    missed: u32,
    steps: [u32; 6],
    pending_steps: usize,
}
impl Pacing {
    pub fn new() -> Self {
        Self {
            intervals: [0; 2001],
            draw: [0; 2001],
            sim: [0; 2001],
            last: 0,
            max: 0,
            frames: 0,
            ticks: 0,
            missed: 0,
            steps: [0; 6],
            pending_steps: 0,
        }
    }
    pub fn step(&mut self, elapsed: u64) {
        self.sim[elapsed.min(2000) as usize] += 1;
        self.ticks += 1;
        self.pending_steps += 1;
    }
    pub fn frame(&mut self, started: u64, tick: u64, running: bool) {
        self.draw[monotonic_millis().saturating_sub(started).min(2000) as usize] += 1;
        if self.last != 0 && running {
            let elapsed = started.saturating_sub(self.last);
            self.intervals[elapsed.min(2000) as usize] += 1;
            self.max = self.max.max(elapsed);
            if elapsed > 50 {
                self.missed += 1;
            }
        }
        self.last = if running { started } else { 0 };
        self.frames += 1;
        self.steps[self.pending_steps.min(5)] += 1;
        self.pending_steps = 0;
        if self.frames % 128 == 0 {
            let heap = sunlight_libc::alloc::heap_stats();
            debug_log(&alloc::format!("[EMPIRES-PACING] tick={} frames={} interval_p50_p95_p99_ms={:?} max_ms={} over_50ms={}\n",tick,self.frames,percentiles(&self.intervals),self.max,self.missed));
            debug_log(&alloc::format!("[EMPIRES-PACING-WORK] frames={} draw_p50_p95_p99_ms={:?} sim_p50_p95_p99_ms={:?} ticks={} steps_0_1_2_3_4_5plus={:?}\n",self.frames,percentiles(&self.draw),percentiles(&self.sim),self.ticks,self.steps));
            let bands = [
                (0, 16),
                (17, 33),
                (34, 50),
                (51, 100),
                (101, 250),
                (251, 2000),
            ]
            .map(|(a, b)| {
                self.intervals[a..=b]
                    .iter()
                    .map(|v| u64::from(*v))
                    .sum::<u64>()
            });
            debug_log(&alloc::format!("[EMPIRES-PACING-BANDS] frames={} interval_ms_0_16_17_33_34_50_51_100_101_250_251plus={:?}\n",self.frames,bands));
            debug_log(&alloc::format!(
                "[EMPIRES-HEAP] frames={} live={} bytes={} high={} alloc={} free={} realloc={}\n",
                self.frames,
                heap.live_allocation_count,
                heap.requested_user_bytes,
                heap.high_water_allocated_bytes,
                heap.allocation_count,
                heap.free_count,
                heap.realloc_count
            ));
        }
    }
}
fn percentiles(hist: &[u32; 2001]) -> [usize; 3] {
    let total: u64 = hist.iter().map(|x| u64::from(*x)).sum();
    [50, 95, 99].map(|p| {
        if total == 0 {
            return 0;
        }
        let target = (total * p + 99) / 100;
        let mut sum = 0;
        for (ms, n) in hist.iter().enumerate() {
            sum += u64::from(*n);
            if sum >= target {
                return ms;
            }
        }
        2000
    })
}
