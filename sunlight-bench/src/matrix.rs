//! 1024 × 1024 integer matrix multiplication.
//!
//! Uses i32 arithmetic (not f32) because SunlightOS does not save/restore
//! XMM registers on context-switch, making SSE state unreliable in userspace.
//! i32 IMUL exercises the same integer execution units and gives a stable,
//! reproducible cycle count.

use crate::bench::{rdtsc, Benchmark};
use core::hint::black_box;

pub const N: usize = 1024;

pub struct MatrixBench {
    // Three heap-allocated N×N i32 matrices.
    // Allocated once at bench construction to exclude alloc time from the measurement.
    a: alloc::vec::Vec<i32>,
    b: alloc::vec::Vec<i32>,
    c: alloc::vec::Vec<i32>,
}

impl MatrixBench {
    pub fn new() -> Self {
        let mut a = alloc::vec![0i32; N * N];
        let mut b = alloc::vec![0i32; N * N];
        let c = alloc::vec![0i32; N * N];

        // Fill A and B with a deterministic pattern so results are non-trivial.
        for i in 0..N * N {
            a[i] = ((i as u32).wrapping_mul(2654435761) >> 16) as i32;
            b[i] = ((i as u32).wrapping_mul(1234567891) >> 16) as i32;
        }
        Self { a, b, c }
    }
}

/// Naive C = A × B.  Cache-oblivious but intentionally unoptimized so the
/// compiler cannot vectorize it away — black_box on inner accumulators
/// would defeat performance, so we use a tiled (i,k,j) loop order instead,
/// which is cache-friendlier for B's column traversal without requiring SIMD.
#[inline(never)]
fn matmul(a: &[i32], b: &[i32], c: &mut [i32]) {
    for row in c.iter_mut() {
        *row = 0;
    }
    // ikj order: A is streamed row-by-row, B is streamed row-by-row (not column),
    // C accumulates one row at a time → far fewer cache misses than ijk.
    for i in 0..N {
        for k in 0..N {
            let a_ik = a[i * N + k];
            for j in 0..N {
                c[i * N + j] = c[i * N + j].wrapping_add(a_ik.wrapping_mul(b[k * N + j]));
            }
        }
    }
}

impl Benchmark for MatrixBench {
    fn name(&self) -> &'static str {
        "Matrix Multiply 1024² (i32, ikj)"
    }

    fn run(&self) -> u64 {
        // We need a mutable C; clone from our pre-allocated zero buffer.
        let mut c = self.c.clone();
        let start = rdtsc();
        matmul(&self.a, &self.b, &mut c);
        let elapsed = rdtsc() - start;
        // Keep c alive so the compiler cannot elide the computation.
        black_box(&c);
        elapsed
    }
}
