//! Segmented Sieve of Eratosthenes.

use crate::bench::rdtsc;
use core::hint::black_box;

pub const NAME: &str = "Segmented Sieve (primes <= 10^8)";

const LIMIT: usize = 100_000_000;
const SEG: usize = 1 << 15;
const SQRT_LIMIT: usize = 10_001;
const SEGMENTS_PER_STEP: usize = 64;

pub struct SieveRunner {
    small: [bool; SQRT_LIMIT],
    sprimes: [u32; 1300],
    ns: usize,
    seg_buf: [u8; SEG / 8],
    low: usize,
    total: u64,
    cycles: u64,
}

impl SieveRunner {
    pub fn new() -> Self {
        let mut runner = Self {
            small: [true; SQRT_LIMIT],
            sprimes: [0; 1300],
            ns: 0,
            seg_buf: [0; SEG / 8],
            low: 0,
            total: 0,
            cycles: 0,
        };
        runner.prepare_small_primes();
        runner
    }

    pub fn name(&self) -> &'static str {
        NAME
    }

    pub fn cycles(&self) -> u64 {
        self.cycles
    }

    pub fn progress_bp(&self) -> u16 {
        ((self.low.min(LIMIT) as u64 * 10_000) / LIMIT as u64) as u16
    }

    pub fn step(&mut self) -> bool {
        if self.low >= LIMIT {
            return true;
        }

        let start = rdtsc();
        let mut segments = 0usize;
        while self.low < LIMIT && segments < SEGMENTS_PER_STEP {
            self.process_segment();
            segments += 1;
        }
        self.cycles = self.cycles.saturating_add(rdtsc() - start);

        if self.low >= LIMIT {
            black_box(self.total);
            true
        } else {
            false
        }
    }

    fn prepare_small_primes(&mut self) {
        self.small[0] = false;
        self.small[1] = false;

        let mut p = 2usize;
        while p * p < SQRT_LIMIT {
            if self.small[p] {
                let mut m = p * p;
                while m < SQRT_LIMIT {
                    self.small[m] = false;
                    m += p;
                }
            }
            p += 1;
        }

        for i in 2..SQRT_LIMIT {
            if self.small[i] {
                self.sprimes[self.ns] = i as u32;
                self.ns += 1;
            }
        }
    }

    fn process_segment(&mut self) {
        let low = self.low;
        let high = (low + SEG).min(LIMIT);
        let len = high - low;

        for byte in self.seg_buf.iter_mut() {
            *byte = 0xFF;
        }
        if low == 0 {
            if len > 0 {
                self.seg_buf[0] &= !1;
            }
            if len > 1 {
                self.seg_buf[0] &= !2;
            }
        }

        for idx in 0..self.ns {
            let sp = self.sprimes[idx] as usize;
            let start = if sp * sp >= low {
                sp * sp
            } else {
                let r = low % sp;
                if r == 0 {
                    low
                } else {
                    low - r + sp
                }
            };
            let mut value = start;
            while value < high {
                let bit = value - low;
                self.seg_buf[bit >> 3] &= !(1u8 << (bit & 7));
                value += sp;
            }
        }

        let full_bytes = len / 8;
        for byte in &self.seg_buf[..full_bytes] {
            self.total = self.total.saturating_add(byte.count_ones() as u64);
        }
        let rem = len & 7;
        if rem > 0 {
            let mask = (1u8 << rem).wrapping_sub(1);
            self.total = self
                .total
                .saturating_add((self.seg_buf[full_bytes] & mask).count_ones() as u64);
        }

        self.low += SEG;
    }
}
