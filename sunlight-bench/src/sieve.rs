//! Segmented Sieve of Eratosthenes.
//!
//! Uses a 32 KiB segment window (fits in L1 cache) to count all primes
//! up to LIMIT = 100_000_000.  Only the small-primes array and the window
//! buffer live on the heap; total extra memory is < 80 KiB.

use crate::bench::{rdtsc, Benchmark};
use core::hint::black_box;

const LIMIT: usize = 100_000_000;
const SEG: usize = 1 << 15; // 32 768 bits = 4 096 bytes (L1-friendly)
const SQRT_LIMIT: usize = 10_001; // ceil(sqrt(LIMIT)) + 1

pub struct SieveBench;

/// Count primes up to LIMIT with a segmented bit-sieve.
#[inline(never)]
fn count_primes() -> u64 {
    // Phase 1 – small sieve up to SQRT_LIMIT.
    let mut small = [true; SQRT_LIMIT];
    small[0] = false;
    small[1] = false;
    let mut p = 2;
    while p * p < SQRT_LIMIT {
        if small[p] {
            let mut m = p * p;
            while m < SQRT_LIMIT {
                small[m] = false;
                m += p;
            }
        }
        p += 1;
    }

    // Collect small primes (p ≥ 2).
    let mut sprimes = [0u32; 1300]; // 1229 primes below 10 000
    let mut ns = 0usize;
    for i in 2..SQRT_LIMIT {
        if small[i] {
            sprimes[ns] = i as u32;
            ns += 1;
        }
    }

    // Phase 2 – segment window.
    let mut seg_buf = [0u8; SEG / 8]; // bit array for one segment
    let mut total: u64 = 0;

    let mut low = 0usize;
    while low < LIMIT {
        let high = (low + SEG).min(LIMIT);
        let len = high - low;

        // Mark all bits as prime (1 = prime).
        for b in seg_buf.iter_mut() {
            *b = 0xFF;
        }
        // Clear 0 and 1 in the very first segment.
        if low == 0 {
            if len > 0 {
                seg_buf[0] &= !1;
            }
            if len > 1 {
                seg_buf[0] &= !2;
            }
        }

        // Cross off multiples of each small prime.
        for i in 0..ns {
            let sp = sprimes[i] as usize;
            // First multiple of sp ≥ low.
            let start = if sp * sp >= low {
                sp * sp
            } else {
                let r = low % sp;
                if r == 0 { low } else { low - r + sp }
            };
            let mut j = start;
            while j < high {
                let bit = j - low;
                seg_buf[bit >> 3] &= !(1u8 << (bit & 7));
                j += sp;
            }
        }

        // Count set bits in this segment (only up to `len` bits).
        let full_bytes = len / 8;
        for b in &seg_buf[..full_bytes] {
            total += b.count_ones() as u64;
        }
        let rem = len & 7;
        if rem > 0 {
            let mask = (1u8 << rem).wrapping_sub(1);
            total += (seg_buf[full_bytes] & mask).count_ones() as u64;
        }

        low += SEG;
    }

    total
}

impl Benchmark for SieveBench {
    fn name(&self) -> &'static str {
        "Segmented Sieve (primes ≤ 10^8)"
    }

    fn run(&self) -> u64 {
        let start = rdtsc();
        let count = black_box(count_primes());
        let elapsed = rdtsc() - start;
        black_box(count);
        elapsed
    }
}
