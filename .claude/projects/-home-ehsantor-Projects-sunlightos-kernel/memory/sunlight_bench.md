---
name: sunlight-bench
description: SunLight-Bench performance benchmarking suite — architecture, constraints, and how to wire it into the OS image
metadata:
  type: project
---

Crate `sunlight-bench` at `sunlight-bench/` (workspace member) produces binary `sunbench`.

**Benchmarks:**
- Single-core: Pi (Machin formula, 200k iterations, u128 fixed-point), Segmented Sieve (primes ≤ 10^8, 32 KiB L1 window), Matrix multiply 1024² (i32 ikj-order — NOT f32, because kernel does not save/restore XMM registers on context switch)
- Multi-core: Parallel SHA-256 (1 MiB/core, spin barrier, thread spawn via syscall 22)

**Key design:**
- 16 MiB `static mut AlignedHeap` in BSS (NOBITS) → binary stays ≤ 25 KB
- Thread spawn: raw `syscall 22` + `unsafe extern "C" fn thread_trampoline(func, arg)` ABI
- Scheduler isolation: `set_nice(pid, -10)` at startup (userspace equivalent of kernel `set_state`)
- Scoring: `(BASELINE_CYCLES / measured) * 1000`; total = sum of scores; output via `debug_log`

**Build:** embedded in kernel image via `kernel/build.rs` entry `sunlight-bench → sunbench`

**Why:** No f32 matrix: kernel's timer interrupt saves GP regs but not XMM (no fxsave/fxrstor in context switch). Using i32 for correctness; comment in matrix.rs explains.
