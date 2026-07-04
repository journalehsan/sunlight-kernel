//! Benchmark execution policy hooks for parallel stages.
//!
//! Multi-core SunLight-Bench stages should run with the full user-selected worker
//! count. Future kernel "smart SMP scaling" may reduce visible or schedulable cores
//! for power saving; that policy must be bypassed during N-core benchmark passes so
//! 4-core vs 8-core vs 12-core comparisons stay meaningful.
//!
//! # Integration TODO
//!
//! No safe userspace API exists yet. When adding one, wire it here:
//!
//! 1. **`ipc/src/lib.rs`** — add `enter_benchmark_mode(workers: u32)` /
//!    `leave_benchmark_mode()` wrapping a new syscall (e.g. `SetBenchmarkMode`).
//! 2. **`kernel/src/sched/mod.rs`** — honour the flag by keeping all requested
//!    cores online and schedulable for the calling process (do not park APs or shrink
//!    `online_cores` while benchmark mode is active).
//! 3. **`kernel/src/telemetry.rs`** — report the benchmark-visible core count so
//!    telemetry consumers see the same worker budget as the benchmark.
//!
//! Until then, callers use [`parallel_workers`] for clamping only and
//! [`enter_parallel_phase`] / [`leave_parallel_phase`] are no-ops aside from logging
//! intent in debug builds via comments.

use crate::multi::MAX_CORES;

/// Worker count for parallel benchmark stages.
///
/// Clamps `requested` to telemetry-reported cores and [`MAX_CORES`]. When a kernel
/// benchmark-mode hook exists, call it from [`enter_parallel_phase`] so the returned
/// value matches cores the scheduler will actually run workers on.
pub fn parallel_workers(requested: usize, telemetry_cores: usize) -> usize {
    requested.max(1).min(telemetry_cores.max(1)).min(MAX_CORES)
}

/// Called immediately before spawning parallel benchmark workers.
pub fn enter_parallel_phase(workers: usize) {
    let _ = workers;
    // TODO(bench): ipc::enter_benchmark_mode(workers as u32) once syscall exists.
}

/// Called after a parallel benchmark stage completes.
pub fn leave_parallel_phase() {
    // TODO(bench): ipc::leave_benchmark_mode() once syscall exists.
}
