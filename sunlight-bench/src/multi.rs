//! Multi-core integer mixing benchmark.
//!
//! Each worker runs a deterministic xorshift64* style loop for a fixed number
//! of iterations. The workload is branch-light, heap-free, and easy to keep
//! stable in a `no_std` environment while still exercising all cores.

use crate::bench::rdtsc;
use crate::thread::{arrive_and_wait, barrier_reset, spawn};
use alloc::vec::Vec;
use core::hint::black_box;
use core::sync::atomic::{AtomicU64, Ordering};

pub const NAME: &str = "Parallel Integer Mix (64M ops/core)";
pub const MAX_CORES: usize = 32;

const OPS_PER_CORE: u64 = 64_000_000;
const PROGRESS_CHUNK: u64 = 1_000_000;

static THREAD_START: [AtomicU64; MAX_CORES] = {
    const ZERO: AtomicU64 = AtomicU64::new(0);
    [ZERO; MAX_CORES]
};
static THREAD_END: [AtomicU64; MAX_CORES] = {
    const ZERO: AtomicU64 = AtomicU64::new(0);
    [ZERO; MAX_CORES]
};
static THREAD_PROGRESS: [AtomicU64; MAX_CORES] = {
    const ZERO: AtomicU64 = AtomicU64::new(0);
    [ZERO; MAX_CORES]
};

static TOTAL_CORES: AtomicU64 = AtomicU64::new(1);
static ASYNC_STATE: AtomicU64 = AtomicU64::new(0);
static ASYNC_RESULT: AtomicU64 = AtomicU64::new(0);
static ASYNC_CORES: AtomicU64 = AtomicU64::new(1);

unsafe extern "C" fn worker(slot: u64) {
    let n = TOTAL_CORES.load(Ordering::SeqCst);
    arrive_and_wait(n);

    let slot_usize = slot as usize;
    THREAD_START[slot_usize].store(rdtsc(), Ordering::SeqCst);

    let mut remaining = OPS_PER_CORE;
    let mut acc = 0x9E37_79B9_7F4A_7C15u64 ^ slot.wrapping_mul(0xD1B5_4A32_D192_ED03);
    let mut completed = 0u64;

    while remaining > 0 {
        let chunk = remaining.min(PROGRESS_CHUNK);
        for _ in 0..chunk {
            acc ^= acc >> 12;
            acc ^= acc << 25;
            acc ^= acc >> 27;
            acc = acc.wrapping_mul(0x2545_F491_4F6C_DD1D);
        }
        remaining -= chunk;
        completed += chunk;
        THREAD_PROGRESS[slot_usize].store(completed, Ordering::SeqCst);
    }

    black_box(acc);
    THREAD_END[slot_usize].store(rdtsc(), Ordering::SeqCst);
}

unsafe extern "C" fn async_entry(ncores: u64) {
    let bench = ParallelMix::new(ncores as usize);
    let cycles = bench.run_sync();
    ASYNC_RESULT.store(cycles, Ordering::SeqCst);
    ASYNC_STATE.store(2, Ordering::SeqCst);
}

pub struct AsyncHandle {
    _stack: Vec<u8>,
}

pub struct ParallelMix {
    ncores: usize,
}

impl ParallelMix {
    pub fn new(ncores: usize) -> Self {
        Self {
            ncores: ncores.min(MAX_CORES).max(1),
        }
    }

    pub fn run_sync(&self) -> u64 {
        let n = self.ncores as u64;
        TOTAL_CORES.store(n, Ordering::SeqCst);

        for idx in 0..self.ncores {
            THREAD_START[idx].store(0, Ordering::SeqCst);
            THREAD_END[idx].store(0, Ordering::SeqCst);
            THREAD_PROGRESS[idx].store(0, Ordering::SeqCst);
        }
        barrier_reset();

        let mut stacks: Vec<Vec<u8>> = Vec::new();
        for slot in 1..self.ncores {
            let (_, stack) = spawn(worker, slot as u64);
            stacks.push(stack);
        }

        unsafe { worker(0) };

        for slot in 1..self.ncores {
            while THREAD_END[slot].load(Ordering::SeqCst) == 0 {
                sunlight_ipc::process_yield();
            }
        }

        let min_start = (0..self.ncores)
            .map(|idx| THREAD_START[idx].load(Ordering::SeqCst))
            .fold(u64::MAX, u64::min);
        let max_end = (0..self.ncores)
            .map(|idx| THREAD_END[idx].load(Ordering::SeqCst))
            .fold(0u64, u64::max);

        black_box(&stacks);
        drop(stacks);

        max_end.saturating_sub(min_start)
    }
}

pub fn start_async(ncores: usize) -> Option<AsyncHandle> {
    if ASYNC_STATE
        .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return None;
    }

    let ncores = ncores.min(MAX_CORES).max(1);
    ASYNC_RESULT.store(0, Ordering::SeqCst);
    ASYNC_CORES.store(ncores as u64, Ordering::SeqCst);

    for idx in 0..ncores {
        THREAD_PROGRESS[idx].store(0, Ordering::SeqCst);
        THREAD_END[idx].store(0, Ordering::SeqCst);
        THREAD_START[idx].store(0, Ordering::SeqCst);
    }

    let (tid, stack) = spawn(async_entry, ncores as u64);
    if tid == 0 {
        ASYNC_STATE.store(0, Ordering::SeqCst);
        return None;
    }

    Some(AsyncHandle { _stack: stack })
}

pub fn async_progress_bp() -> u16 {
    if ASYNC_STATE.load(Ordering::SeqCst) == 0 {
        return 0;
    }

    let ncores = ASYNC_CORES.load(Ordering::SeqCst) as usize;
    let mut total = 0u64;
    for idx in 0..ncores.min(MAX_CORES) {
        total = total.saturating_add(THREAD_PROGRESS[idx].load(Ordering::SeqCst));
    }
    let denom = OPS_PER_CORE.saturating_mul(ncores.max(1) as u64);
    ((total.min(denom) * 10_000) / denom.max(1)) as u16
}

pub fn take_async_result() -> Option<u64> {
    if ASYNC_STATE.load(Ordering::SeqCst) != 2 {
        return None;
    }
    let cycles = ASYNC_RESULT.load(Ordering::SeqCst);
    ASYNC_STATE.store(3, Ordering::SeqCst);
    Some(cycles)
}
