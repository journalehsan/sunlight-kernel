//! Multi-core benchmarking suite.
//!
//! Runs three workloads sequentially, each dispatching worker threads across
//! all available cores. Results are combined into throughput-based N-core
//! scores with explicit fixed-total-work and work-per-core categories.
//!
//! Workloads:
//!   1. Integer Mix — xorshift64* loop (existing)
//!   2. Matrix Multiply — each core handles a chunk of rows via ikj loop
//!   3. SHA-256 Hash — each core hashes independent 4 KiB blocks

use crate::bench::rdtsc;
use crate::scoring::WorkloadClass;
use crate::thread::{arrive_and_wait, barrier_reset, spawn};
use core::hint::black_box;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use sunlight_ipc::monotonic_millis;

pub const NAME_INTEGER: &str = "Parallel Integer Mix (64M ops/core)";
pub const NAME_MATRIX: &str = "Parallel Matrix Multiply 1024^2";
pub const NAME_SHA256: &str = "Parallel SHA-256 (16 MiB/core)";
pub const MAX_CORES: usize = 32;

const N_MATRIX: usize = 1024;
const HASH_BYTES_PER_CORE: usize = 16 * 1024 * 1024;

// ── Shared statics, reset between workloads ────────────────────────────────

static THREAD_START: [AtomicU64; MAX_CORES] = {
    const ZERO: AtomicU64 = AtomicU64::new(0);
    [ZERO; MAX_CORES]
};
static THREAD_START_MS: [AtomicU64; MAX_CORES] = {
    const ZERO: AtomicU64 = AtomicU64::new(0);
    [ZERO; MAX_CORES]
};
static THREAD_END: [AtomicU64; MAX_CORES] = {
    const ZERO: AtomicU64 = AtomicU64::new(0);
    [ZERO; MAX_CORES]
};
static THREAD_END_MS: [AtomicU64; MAX_CORES] = {
    const ZERO: AtomicU64 = AtomicU64::new(0);
    [ZERO; MAX_CORES]
};
static THREAD_PROGRESS: [AtomicU64; MAX_CORES] = {
    const ZERO: AtomicU64 = AtomicU64::new(0);
    [ZERO; MAX_CORES]
};
static THREAD_GENERATION: [AtomicU64; MAX_CORES] = {
    const ZERO: AtomicU64 = AtomicU64::new(0);
    [ZERO; MAX_CORES]
};

static TOTAL_CORES: AtomicU64 = AtomicU64::new(1);
static BATCH_GENERATION: AtomicU64 = AtomicU64::new(0);
static BATCH_ABORTED: AtomicBool = AtomicBool::new(false);
static BATCH_RETURNED: AtomicU64 = AtomicU64::new(0);
static ASYNC_STATE: AtomicU64 = AtomicU64::new(0);
static ASYNC_RESULT: AtomicU64 = AtomicU64::new(0);
static ASYNC_CORES: AtomicU64 = AtomicU64::new(1);
static ASYNC_WORKLOAD: AtomicU64 = AtomicU64::new(0);

// ── Matrix workload shared data ────────────────────────────────────────────

static MATRIX_A_PTR: AtomicU64 = AtomicU64::new(0);
static MATRIX_B_PTR: AtomicU64 = AtomicU64::new(0);
static MATRIX_C_PTR: AtomicU64 = AtomicU64::new(0);

// ── Integer Mix worker ─────────────────────────────────────────────────────

unsafe extern "C" fn integer_mix_worker(slot: u64) {
    let (generation, slot_usize) = decode_worker_arg(slot);
    let n = TOTAL_CORES.load(Ordering::SeqCst);
    if !arrive_and_wait(n, &BATCH_ABORTED) {
        BATCH_RETURNED.fetch_add(1, Ordering::Release);
        return;
    }

    let slot = slot_usize as u64;
    THREAD_START[slot_usize].store(rdtsc(), Ordering::SeqCst);
    THREAD_START_MS[slot_usize].store(monotonic_millis(), Ordering::SeqCst);

    let ops_per_core: u64 = 64_000_000;
    let progress_chunk: u64 = 1_000_000;
    let mut remaining = ops_per_core;
    let mut acc = 0x9E37_79B9_7F4A_7C15u64 ^ slot.wrapping_mul(0xD1B5_4A32_D192_ED03);
    let mut completed = 0u64;

    while remaining > 0 {
        let chunk = remaining.min(progress_chunk);
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
    THREAD_END_MS[slot_usize].store(monotonic_millis(), Ordering::Relaxed);
    THREAD_END[slot_usize].store(rdtsc(), Ordering::Relaxed);
    publish_worker_completion(slot_usize, generation);
}

// ── Matrix Multiply worker ──────────────────────────────────────────────────

unsafe extern "C" fn matrix_mix_worker(slot: u64) {
    let (generation, slot_usize) = decode_worker_arg(slot);
    let ncores = TOTAL_CORES.load(Ordering::SeqCst) as usize;
    if !arrive_and_wait(ncores as u64, &BATCH_ABORTED) {
        BATCH_RETURNED.fetch_add(1, Ordering::Release);
        return;
    }

    THREAD_START[slot_usize].store(rdtsc(), Ordering::SeqCst);
    THREAD_START_MS[slot_usize].store(monotonic_millis(), Ordering::SeqCst);

    let a_ptr = MATRIX_A_PTR.load(Ordering::SeqCst) as *const i32;
    let b_ptr = MATRIX_B_PTR.load(Ordering::SeqCst) as *const i32;
    let c_ptr = MATRIX_C_PTR.load(Ordering::SeqCst) as *mut i32;

    let total_rows = N_MATRIX;
    let rows_per_core = total_rows / ncores;
    let extra = total_rows % ncores;
    let start_row = if slot_usize < extra {
        slot_usize * (rows_per_core + 1)
    } else {
        extra * (rows_per_core + 1) + (slot_usize - extra) * rows_per_core
    };
    let end_row = if slot_usize < extra {
        start_row + rows_per_core + 1
    } else {
        start_row + rows_per_core
    };

    let total_ops = ((end_row - start_row) * N_MATRIX * N_MATRIX) as u64;
    let mut completed = 0u64;

    for i in start_row..end_row {
        for k in 0..N_MATRIX {
            let aik = *a_ptr.add(i * N_MATRIX + k);
            if aik == 0 {
                continue;
            }
            for j in 0..N_MATRIX {
                *c_ptr.add(i * N_MATRIX + j) += aik * (*b_ptr.add(k * N_MATRIX + j));
            }
        }
        completed += (N_MATRIX * N_MATRIX) as u64;
        THREAD_PROGRESS[slot_usize].store(completed.min(total_ops), Ordering::SeqCst);
    }

    black_box((a_ptr, c_ptr));
    THREAD_PROGRESS[slot_usize].store(total_ops, Ordering::SeqCst);
    THREAD_END_MS[slot_usize].store(monotonic_millis(), Ordering::Relaxed);
    THREAD_END[slot_usize].store(rdtsc(), Ordering::Relaxed);
    publish_worker_completion(slot_usize, generation);
}

// ── SHA-256 Hash worker ────────────────────────────────────────────────────

unsafe extern "C" fn sha256_mix_worker(slot: u64) {
    let (generation, slot_usize) = decode_worker_arg(slot);
    let ncores = TOTAL_CORES.load(Ordering::SeqCst) as usize;
    if !arrive_and_wait(ncores as u64, &BATCH_ABORTED) {
        BATCH_RETURNED.fetch_add(1, Ordering::Release);
        return;
    }

    let slot = slot_usize as u64;
    THREAD_START[slot_usize].store(rdtsc(), Ordering::SeqCst);
    THREAD_START_MS[slot_usize].store(monotonic_millis(), Ordering::SeqCst);

    let total_bytes = HASH_BYTES_PER_CORE;
    const CHUNK: usize = 4096;
    let total_chunks = total_bytes / CHUNK;
    let mut completed = 0u64;

    for chunk_idx in 0..total_chunks {
        let mut block = [0u8; CHUNK];
        let seed = (slot << 32) | (chunk_idx as u64);
        let mut mixer = seed.wrapping_mul(0xD6E8_FEB8_6659_FD93);
        for b in &mut block {
            mixer = mixer
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            *b = (mixer >> 24) as u8;
        }
        let digest = crate::sha256::sha256(&block);
        black_box(digest);
        completed += 1;
        THREAD_PROGRESS[slot_usize].store(completed.min(total_chunks as u64), Ordering::SeqCst);
    }

    THREAD_PROGRESS[slot_usize].store(total_chunks as u64, Ordering::SeqCst);
    THREAD_END_MS[slot_usize].store(monotonic_millis(), Ordering::Relaxed);
    THREAD_END[slot_usize].store(rdtsc(), Ordering::Relaxed);
    publish_worker_completion(slot_usize, generation);
}

// ── Workload runner ─────────────────────────────────────────────────────────

fn reset_statics(ncores: usize) -> u64 {
    let generation = BATCH_GENERATION
        .fetch_add(1, Ordering::AcqRel)
        .wrapping_add(1)
        .max(1)
        & u32::MAX as u64;
    let generation = generation.max(1);
    TOTAL_CORES.store(ncores as u64, Ordering::SeqCst);
    BATCH_ABORTED.store(false, Ordering::Release);
    BATCH_RETURNED.store(0, Ordering::Release);
    for idx in 0..ncores {
        THREAD_START[idx].store(0, Ordering::SeqCst);
        THREAD_START_MS[idx].store(0, Ordering::SeqCst);
        THREAD_END_MS[idx].store(0, Ordering::SeqCst);
        THREAD_END[idx].store(0, Ordering::SeqCst);
        THREAD_PROGRESS[idx].store(0, Ordering::SeqCst);
        THREAD_GENERATION[idx].store(0, Ordering::Release);
    }
    barrier_reset();
    generation
}

fn encode_worker_arg(generation: u64, slot: usize) -> u64 {
    ((generation & u32::MAX as u64) << 32) | slot as u64
}

fn decode_worker_arg(arg: u64) -> (u64, usize) {
    (arg >> 32, (arg & u32::MAX as u64) as usize)
}

fn publish_worker_completion(slot: usize, generation: u64) {
    // Release publishes every result/progress/timestamp write that precedes it.
    // The coordinator waits with Acquire before reading any worker result.
    THREAD_GENERATION[slot].store(generation, Ordering::Release);
    BATCH_RETURNED.fetch_add(1, Ordering::Release);
}

fn generation_complete(ncores: usize, generation: u64) -> bool {
    (0..ncores).all(|idx| THREAD_GENERATION[idx].load(Ordering::Acquire) == generation)
}

fn wait_for_generation(ncores: usize, generation: u64) {
    while !generation_complete(ncores, generation) {
        sunlight_ipc::process_yield();
    }
}

fn wait_for_returns(count: usize) {
    while BATCH_RETURNED.load(Ordering::Acquire) < count as u64 {
        sunlight_ipc::process_yield();
    }
}

fn spawn_workers(ncores: usize, generation: u64, worker: unsafe extern "C" fn(u64)) -> usize {
    let mut spawned = 0;
    for slot in 1..ncores {
        if spawn(worker, encode_worker_arg(generation, slot)) == 0 {
            break;
        }
        spawned += 1;
    }
    spawned
}

unsafe fn run_worker_on_core0(slot: u64, worker: unsafe extern "C" fn(u64)) {
    worker(slot);
}

fn measure_elapsed(ncores: usize) -> u64 {
    let min_start = (0..ncores)
        .map(|idx| THREAD_START[idx].load(Ordering::SeqCst))
        .fold(u64::MAX, u64::min);
    let max_end = (0..ncores)
        .map(|idx| THREAD_END[idx].load(Ordering::SeqCst))
        .fold(0u64, u64::max);
    max_end.saturating_sub(min_start)
}

pub fn run_integer_mix(ncores: usize) -> Option<u64> {
    let n = ncores.min(MAX_CORES).max(1);
    let generation = reset_statics(n);
    let spawned = spawn_workers(n, generation, integer_mix_worker);
    if spawned != n - 1 {
        BATCH_ABORTED.store(true, Ordering::Release);
        wait_for_returns(spawned);
        return None;
    }
    unsafe { run_worker_on_core0(encode_worker_arg(generation, 0), integer_mix_worker) };
    wait_for_generation(n, generation);
    Some(measure_elapsed(n))
}

pub fn run_matrix_mix(ncores: usize) -> Option<u64> {
    let n = ncores.min(MAX_CORES).max(1);

    let mut a = alloc::vec![0i32; N_MATRIX * N_MATRIX];
    let mut b = alloc::vec![0i32; N_MATRIX * N_MATRIX];
    let mut c = alloc::vec![0i32; N_MATRIX * N_MATRIX];

    for idx in 0..N_MATRIX * N_MATRIX {
        a[idx] = ((idx as u32).wrapping_mul(2_654_435_761) >> 16) as i32;
        b[idx] = ((idx as u32).wrapping_mul(1_299_709) >> 16) as i32;
        c[idx] = 0;
    }

    MATRIX_A_PTR.store(a.as_ptr() as u64, Ordering::SeqCst);
    MATRIX_B_PTR.store(b.as_ptr() as u64, Ordering::SeqCst);
    MATRIX_C_PTR.store(c.as_ptr() as u64, Ordering::SeqCst);

    let generation = reset_statics(n);
    let spawned = spawn_workers(n, generation, matrix_mix_worker);
    if spawned != n - 1 {
        BATCH_ABORTED.store(true, Ordering::Release);
        wait_for_returns(spawned);
        MATRIX_A_PTR.store(0, Ordering::Release);
        MATRIX_B_PTR.store(0, Ordering::Release);
        MATRIX_C_PTR.store(0, Ordering::Release);
        return None;
    }
    unsafe { run_worker_on_core0(encode_worker_arg(generation, 0), matrix_mix_worker) };
    wait_for_generation(n, generation);
    let elapsed = measure_elapsed(n);

    MATRIX_A_PTR.store(0, Ordering::SeqCst);
    MATRIX_B_PTR.store(0, Ordering::SeqCst);
    MATRIX_C_PTR.store(0, Ordering::SeqCst);

    black_box(&c);
    Some(elapsed)
}

pub fn run_sha256_mix(ncores: usize) -> Option<u64> {
    let n = ncores.min(MAX_CORES).max(1);
    let generation = reset_statics(n);
    let spawned = spawn_workers(n, generation, sha256_mix_worker);
    if spawned != n - 1 {
        BATCH_ABORTED.store(true, Ordering::Release);
        wait_for_returns(spawned);
        return None;
    }
    unsafe { run_worker_on_core0(encode_worker_arg(generation, 0), sha256_mix_worker) };
    wait_for_generation(n, generation);
    Some(measure_elapsed(n))
}

// ── Async dispatch (for GUI integration) ───────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkloadId {
    Integer = 0,
    Matrix = 1,
    Sha256 = 2,
}

impl WorkloadId {
    pub fn class(self) -> WorkloadClass {
        match self {
            Self::Matrix => WorkloadClass::MultiFixedTotalWork,
            Self::Integer | Self::Sha256 => WorkloadClass::MultiWorkPerCore,
        }
    }

    pub fn work_units_per_worker(self) -> u64 {
        match self {
            Self::Integer => 64_000_000,
            Self::Matrix => (N_MATRIX * N_MATRIX * N_MATRIX) as u64,
            Self::Sha256 => HASH_BYTES_PER_CORE as u64,
        }
    }

    pub fn progress_units_per_worker(self) -> u64 {
        match self {
            Self::Integer => 64_000_000,
            Self::Matrix => (N_MATRIX * N_MATRIX * N_MATRIX) as u64,
            Self::Sha256 => (HASH_BYTES_PER_CORE / 4096) as u64,
        }
    }

    pub fn total_progress_units(self, workers: usize) -> u64 {
        match self.class() {
            WorkloadClass::MultiFixedTotalWork => self.progress_units_per_worker(),
            WorkloadClass::MultiWorkPerCore => self
                .progress_units_per_worker()
                .saturating_mul(workers.max(1) as u64),
            WorkloadClass::SingleCore => self.progress_units_per_worker(),
        }
    }

    pub fn total_work_units(self, workers: usize) -> u64 {
        match self.class() {
            WorkloadClass::MultiFixedTotalWork => self.work_units_per_worker(),
            WorkloadClass::MultiWorkPerCore => self
                .work_units_per_worker()
                .saturating_mul(workers.max(1) as u64),
            WorkloadClass::SingleCore => self.work_units_per_worker(),
        }
    }
}

unsafe extern "C" fn async_entry(ncores: u64) {
    let n = ncores as usize;
    let wl = ASYNC_WORKLOAD.load(Ordering::SeqCst);
    let cycles = match wl {
        0 => run_integer_mix(n),
        1 => run_matrix_mix(n),
        2 => run_sha256_mix(n),
        _ => None,
    };
    match cycles {
        Some(cycles) => {
            ASYNC_RESULT.store(cycles, Ordering::Relaxed);
            ASYNC_STATE.store(2, Ordering::Release);
        }
        None => ASYNC_STATE.store(4, Ordering::Release),
    }
}

pub struct AsyncHandle;

pub fn start_async(ncores: usize, workload: WorkloadId) -> Option<AsyncHandle> {
    if ASYNC_STATE
        .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return None;
    }

    let ncores = ncores.min(MAX_CORES).max(1);
    ASYNC_RESULT.store(0, Ordering::SeqCst);
    ASYNC_CORES.store(ncores as u64, Ordering::SeqCst);
    ASYNC_WORKLOAD.store(workload as u64, Ordering::SeqCst);

    for idx in 0..ncores {
        THREAD_PROGRESS[idx].store(0, Ordering::SeqCst);
        THREAD_END[idx].store(0, Ordering::SeqCst);
        THREAD_END_MS[idx].store(0, Ordering::SeqCst);
        THREAD_START[idx].store(0, Ordering::SeqCst);
        THREAD_START_MS[idx].store(0, Ordering::SeqCst);
    }

    let tid = spawn(async_entry, ncores as u64);
    if tid == 0 {
        ASYNC_STATE.store(0, Ordering::SeqCst);
        return None;
    }

    Some(AsyncHandle)
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

    let workload = match ASYNC_WORKLOAD.load(Ordering::SeqCst) {
        0 => WorkloadId::Integer,
        1 => WorkloadId::Matrix,
        2 => WorkloadId::Sha256,
        _ => WorkloadId::Integer,
    };
    let denom = match workload.class() {
        WorkloadClass::MultiFixedTotalWork => workload.total_progress_units(ncores),
        WorkloadClass::MultiWorkPerCore => workload.total_progress_units(ncores),
        WorkloadClass::SingleCore => workload.total_progress_units(ncores),
    };
    ((total.min(denom) * 10_000) / denom.max(1)) as u16
}

pub fn take_async_result() -> Option<Result<u64, ()>> {
    match ASYNC_STATE.load(Ordering::Acquire) {
        2 => {
            let cycles = ASYNC_RESULT.load(Ordering::Relaxed);
            ASYNC_STATE.store(3, Ordering::Release);
            Some(Ok(cycles))
        }
        4 => {
            ASYNC_STATE.store(3, Ordering::Release);
            Some(Err(()))
        }
        _ => None,
    }
}

pub fn reset_async() {
    ASYNC_STATE.store(0, Ordering::SeqCst);
    ASYNC_RESULT.store(0, Ordering::SeqCst);
    ASYNC_CORES.store(1, Ordering::SeqCst);
    ASYNC_WORKLOAD.store(0, Ordering::SeqCst);
    MATRIX_A_PTR.store(0, Ordering::SeqCst);
    MATRIX_B_PTR.store(0, Ordering::SeqCst);
    MATRIX_C_PTR.store(0, Ordering::SeqCst);
    for idx in 0..MAX_CORES {
        THREAD_START[idx].store(0, Ordering::SeqCst);
        THREAD_START_MS[idx].store(0, Ordering::SeqCst);
        THREAD_END[idx].store(0, Ordering::SeqCst);
        THREAD_END_MS[idx].store(0, Ordering::SeqCst);
        THREAD_PROGRESS[idx].store(0, Ordering::SeqCst);
        THREAD_GENERATION[idx].store(0, Ordering::Release);
    }
    barrier_reset();
}

/// Earliest worker start tick after the common barrier (for measured multi stages).
pub fn measured_start_tick(ncores: usize) -> u64 {
    let n = ncores.min(MAX_CORES).max(1);
    (0..n)
        .map(|idx| THREAD_START[idx].load(Ordering::SeqCst))
        .filter(|tick| *tick > 0)
        .min()
        .unwrap_or(0)
}

/// Earliest worker start wall time after the common barrier.
pub fn measured_start_ms(ncores: usize) -> u64 {
    let n = ncores.min(MAX_CORES).max(1);
    (0..n)
        .map(|idx| THREAD_START_MS[idx].load(Ordering::SeqCst))
        .filter(|ms| *ms > 0)
        .min()
        .unwrap_or(0)
}

/// Latest worker end tick after the common barrier.
pub fn measured_end_tick(ncores: usize) -> u64 {
    let n = ncores.min(MAX_CORES).max(1);
    (0..n)
        .map(|idx| THREAD_END[idx].load(Ordering::SeqCst))
        .max()
        .unwrap_or(0)
}

/// Latest worker end wall time after the common barrier.
pub fn measured_end_ms(ncores: usize) -> u64 {
    let n = ncores.min(MAX_CORES).max(1);
    (0..n)
        .map(|idx| THREAD_END_MS[idx].load(Ordering::SeqCst))
        .filter(|ms| *ms > 0)
        .max()
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_generation_cannot_complete_recreated_workers() {
        const WORKERS: usize = 12;
        for slot in 0..WORKERS {
            THREAD_GENERATION[slot].store(41, Ordering::Release);
        }
        assert!(generation_complete(WORKERS, 41));

        for slot in 0..WORKERS {
            THREAD_GENERATION[slot].store(0, Ordering::Release);
        }
        THREAD_GENERATION[0].store(41, Ordering::Release);
        assert!(!generation_complete(WORKERS, 42));

        for slot in 0..WORKERS {
            THREAD_GENERATION[slot].store(42, Ordering::Release);
        }
        assert!(generation_complete(WORKERS, 42));
    }

    #[test]
    fn worker_argument_keeps_generation_and_slot_distinct() {
        for slot in 0..12 {
            assert_eq!(decode_worker_arg(encode_worker_arg(77, slot)), (77, slot));
        }
    }
}
