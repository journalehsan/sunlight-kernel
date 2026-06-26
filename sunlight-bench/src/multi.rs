//! Multi-core parallel SHA-256 benchmark.
//!
//! Spawns (ncores − 1) worker threads plus the main thread — all hashing a
//! 1 MiB block.  A spin barrier synchronises the start; each thread records
//! its own start and end TSC.  The reported elapsed cycles span from the
//! earliest start to the latest finish across all cores.

use crate::bench::{rdtsc, Benchmark};
use crate::sha256::sha256;
use crate::thread::{arrive_and_wait, barrier_reset, spawn};
use alloc::vec::Vec;
use core::hint::black_box;
use core::sync::atomic::{AtomicU64, Ordering};

pub const BLOCK_BYTES: usize = 1024 * 1024; // 1 MiB
pub const MAX_CORES: usize = 32;

// Per-thread TSC timestamps written by each worker; read by the main thread.
static THREAD_START: [AtomicU64; MAX_CORES] = {
    const ZERO: AtomicU64 = AtomicU64::new(0);
    [ZERO; MAX_CORES]
};
static THREAD_END: [AtomicU64; MAX_CORES] = {
    const ZERO: AtomicU64 = AtomicU64::new(0);
    [ZERO; MAX_CORES]
};

// Pointer to the shared read-only 1 MiB data block (set before threads spawn).
static DATA_PTR: AtomicU64 = AtomicU64::new(0);
static DATA_LEN: AtomicU64 = AtomicU64::new(0);
static TOTAL_CORES: AtomicU64 = AtomicU64::new(1);

/// Worker entry point called by every thread (including the main thread via a
/// direct call, not a spawn).  `slot` is the 0-based thread index.
unsafe extern "C" fn worker(slot: u64) {
    let n = TOTAL_CORES.load(Ordering::SeqCst);

    // Wait until every thread has arrived.
    arrive_and_wait(n);

    let ptr = DATA_PTR.load(Ordering::SeqCst) as *const u8;
    let len = DATA_LEN.load(Ordering::SeqCst) as usize;
    let data = core::slice::from_raw_parts(ptr, len);

    THREAD_START[slot as usize].store(rdtsc(), Ordering::SeqCst);
    let digest = black_box(sha256(data));
    THREAD_END[slot as usize].store(rdtsc(), Ordering::SeqCst);

    black_box(digest);
}

pub struct ParallelSha256 {
    ncores: usize,
    data: Vec<u8>,
}

impl ParallelSha256 {
    pub fn new(ncores: usize) -> Self {
        let ncores = ncores.min(MAX_CORES).max(1);
        // Fill the block with a pseudo-random byte pattern.
        let mut data = alloc::vec![0u8; BLOCK_BYTES];
        for (i, b) in data.iter_mut().enumerate() {
            *b = (i.wrapping_mul(6364136223846793005) >> 56) as u8;
        }
        Self { ncores, data }
    }
}

impl Benchmark for ParallelSha256 {
    fn name(&self) -> &'static str {
        "SHA-256 parallel (1 MiB/core)"
    }

    fn run(&self) -> u64 {
        let n = self.ncores as u64;

        // Publish shared data pointer for workers.
        DATA_PTR.store(self.data.as_ptr() as u64, Ordering::SeqCst);
        DATA_LEN.store(BLOCK_BYTES as u64, Ordering::SeqCst);
        TOTAL_CORES.store(n, Ordering::SeqCst);

        // Reset TSC slots and barrier.
        for i in 0..self.ncores {
            THREAD_START[i].store(0, Ordering::SeqCst);
            THREAD_END[i].store(0, Ordering::SeqCst);
        }
        barrier_reset();

        // Spawn (ncores − 1) worker threads.
        let mut stacks: Vec<Vec<u8>> = Vec::new();
        for slot in 1..self.ncores {
            let (_, stack) = spawn(worker, slot as u64);
            stacks.push(stack);
        }

        // Main thread participates as worker slot 0.
        unsafe { worker(0) };

        // Wait for all spawned threads to write their end TSC.
        for slot in 1..self.ncores {
            while THREAD_END[slot].load(Ordering::SeqCst) == 0 {
                sunlight_ipc::process_yield();
            }
        }

        // Elapsed = max(end) − min(start) across all participants.
        let min_start = (0..self.ncores)
            .map(|i| THREAD_START[i].load(Ordering::SeqCst))
            .fold(u64::MAX, u64::min);
        let max_end = (0..self.ncores)
            .map(|i| THREAD_END[i].load(Ordering::SeqCst))
            .fold(0u64, u64::max);

        // Keep stacks alive until threads have exited.
        black_box(&stacks);
        drop(stacks);

        max_end.saturating_sub(min_start)
    }
}
