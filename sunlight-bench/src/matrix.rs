//! 1024 x 1024 integer matrix multiplication.

use crate::bench::rdtsc;
use core::hint::black_box;

pub const NAME: &str = "Matrix Multiply 1024^2 (i32, ikj)";
pub const N: usize = 1024;

const OPS_PER_STEP: usize = 8_000_000;
const TOTAL_OPS: u64 = N as u64 * N as u64 * N as u64;

pub struct MatrixRunner {
    a: alloc::vec::Vec<i32>,
    b: alloc::vec::Vec<i32>,
    c: alloc::vec::Vec<i32>,
    i: usize,
    k: usize,
    j: usize,
    ops_done: u64,
    cycles: u64,
}

impl MatrixRunner {
    pub fn new() -> Self {
        let mut a = alloc::vec![0i32; N * N];
        let mut b = alloc::vec![0i32; N * N];
        let c = alloc::vec![0i32; N * N];

        for idx in 0..N * N {
            a[idx] = ((idx as u32).wrapping_mul(2_654_435_761) >> 16) as i32;
            b[idx] = ((idx as u32).wrapping_mul(1_234_567_891) >> 16) as i32;
        }

        Self {
            a,
            b,
            c,
            i: 0,
            k: 0,
            j: 0,
            ops_done: 0,
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
        ((self.ops_done.min(TOTAL_OPS) * 10_000) / TOTAL_OPS) as u16
    }

    pub fn step(&mut self) -> bool {
        if self.i >= N {
            return true;
        }

        let start = rdtsc();
        let mut budget = OPS_PER_STEP;

        while self.i < N && budget > 0 {
            let a_ik = self.a[self.i * N + self.k];
            while self.j < N && budget > 0 {
                let idx = self.i * N + self.j;
                self.c[idx] =
                    self.c[idx].wrapping_add(a_ik.wrapping_mul(self.b[self.k * N + self.j]));
                self.j += 1;
                self.ops_done += 1;
                budget -= 1;
            }

            if self.j == N {
                self.j = 0;
                self.k += 1;
                if self.k == N {
                    self.k = 0;
                    self.i += 1;
                }
            }
        }

        self.cycles = self.cycles.saturating_add(rdtsc() - start);

        if self.i >= N {
            black_box(&self.c);
            true
        } else {
            false
        }
    }
}
