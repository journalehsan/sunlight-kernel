//! Pi calculation via Machin's formula with u128 fixed-point arithmetic.
//!
//! π/4 = 4·arctan(1/5) − arctan(1/239)
//!
//! arctan(1/x) = Σₖ₌₀..∞  (−1)ᵏ / ((2k+1) · x^(2k+1))
//!
//! Scale factor S = 2^60 keeps 18 significant bits while staying inside u128.
//! The loop runs ITERATIONS times so the TSC measurement is meaningful.

use crate::bench::{rdtsc, Benchmark};
use core::hint::black_box;

const SCALE: u128 = 1u128 << 60;
const ITERATIONS: u64 = 200_000;

/// arctan(1/x) · SCALE  using integer long division.
fn arctan_recip(x: u128) -> u128 {
    let x2 = x * x;
    let mut term = SCALE / x;
    let mut sum = term;
    let mut k: u32 = 1;
    loop {
        // term_{k} = term_{k-1} / x²
        term /= x2;
        if term == 0 {
            break;
        }
        let contrib = term / (2 * k as u128 + 1);
        if contrib == 0 {
            break;
        }
        if k & 1 == 1 {
            sum = sum.wrapping_sub(contrib);
        } else {
            sum = sum.wrapping_add(contrib);
        }
        k += 1;
    }
    sum
}

/// One computation of π (fixed-point, returns scaled result).
#[inline(never)]
fn compute_pi_once() -> u128 {
    let a = arctan_recip(5);
    let b = arctan_recip(239);
    // π/4 = 4·arctan(1/5) − arctan(1/239)  →  π = 4·(4a − b)
    4 * (4 * a - b)
}

pub struct PiBench;

impl Benchmark for PiBench {
    fn name(&self) -> &'static str {
        "Pi (Machin fixed-pt, 200k iters)"
    }

    fn run(&self) -> u64 {
        let start = rdtsc();
        let mut acc: u128 = 0;
        for _ in 0..ITERATIONS {
            acc = acc.wrapping_add(black_box(compute_pi_once()));
        }
        let elapsed = rdtsc() - start;
        // Prevent optimizer from discarding acc.
        black_box(acc);
        elapsed
    }
}
