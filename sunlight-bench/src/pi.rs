//! Pi calculation via Machin's formula with u128 fixed-point arithmetic.

use crate::bench::rdtsc;
use core::hint::black_box;

pub const NAME: &str = "Pi (Machin fixed-pt, 200k iters)";

const SCALE: u128 = 1u128 << 60;
const ITERATIONS: u64 = 200_000;
const STEP_ITERS: u64 = 5_000;

fn arctan_recip(x: u128) -> u128 {
    let x2 = x * x;
    let mut term = SCALE / x;
    let mut sum = term;
    let mut k: u32 = 1;
    loop {
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

#[inline(never)]
fn compute_pi_once() -> u128 {
    let a = arctan_recip(5);
    let b = arctan_recip(239);
    4 * (4 * a - b)
}

pub struct PiRunner {
    iter: u64,
    acc: u128,
    cycles: u64,
}

impl PiRunner {
    pub fn new() -> Self {
        Self {
            iter: 0,
            acc: 0,
            cycles: 0,
        }
    }

    pub fn name(&self) -> &'static str {
        NAME
    }

    pub fn cycles(&self) -> u64 {
        self.cycles
    }

    pub fn progress_bp(&self) -> u16 {
        ((self.iter.saturating_mul(10_000)) / ITERATIONS) as u16
    }

    pub fn step(&mut self) -> bool {
        if self.iter >= ITERATIONS {
            return true;
        }

        let end = (self.iter + STEP_ITERS).min(ITERATIONS);
        let start = rdtsc();
        while self.iter < end {
            self.acc = self.acc.wrapping_add(black_box(compute_pi_once()));
            self.iter += 1;
        }
        self.cycles = self.cycles.saturating_add(rdtsc() - start);

        if self.iter >= ITERATIONS {
            black_box(self.acc);
            true
        } else {
            false
        }
    }
}
