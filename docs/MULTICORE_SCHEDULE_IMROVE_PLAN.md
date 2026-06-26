# SunlightOS Kernel — Multicore Scheduler: Plan & Implementation Record

This document describes the original design goals for the SMP scheduler refactor and documents what was actually implemented. Divergences from the original plan are noted explicitly so future contributors know the current state of the code.

---

## Background: Original Scheduler (Pre-SMP)

The original scheduler was a single-core global instance:

```rust
pub static SCHEDULER: spin::Mutex<Scheduler> = spin::Mutex::new(Scheduler::new());
```

`Scheduler` held:

```rust
pub struct Scheduler {
    pub processes: Vec<Process>,
    pub ready_queue_high: VecDeque<usize>,
    pub ready_queue_medium: VecDeque<usize>,
    pub ready_queue_low: VecDeque<usize>,
    pub current: usize,
    pub current_ticks: u64,
    pub global_tick: u64,
    pub idle_context_rsp: u64,
}
```

The active scheduling mode was (and still is):

```rust
pub const SCHEDULER_MODE: SchedulerMode = SchedulerMode::RoundRobin;
```

BORE code exists and is preserved; do not remove it. Do not enable BORE unless explicitly requested.

---

## Design Goals (Original Plan)

- Add per-CPU scheduling state.
- Keep `processes: Vec<Process>` global.
- Move runnable queues and current-process tracking to per-CPU state.
- Preserve existing public API signatures where possible (compatibility wrappers).
- Simple CPU-selection policy: prefer previous CPU → idle/least-loaded → cpu_mask → BSP fallback.
- No single global round-robin pointer as the SMP design.
- No single global runqueue as the final architecture.
- Suitable for low-latency workloads.

The plan proposed a struct named `CpuScheduler` containing:

```rust
pub struct CpuScheduler {
    pub ready_queue_high: VecDeque<usize>,
    pub ready_queue_medium: VecDeque<usize>,
    pub ready_queue_low: VecDeque<usize>,
    pub current: Option<usize>,
    pub current_ticks: u64,
    pub idle_context_rsp: u64,
    pub local_tick: u64,
    pub need_resched: bool,
}
```

And a modified `Scheduler`:

```rust
pub struct Scheduler {
    pub processes: Vec<Process>,
    pub cpus: [CpuScheduler; MAX_CPUS],
    pub num_cpus: usize,
    pub global_tick: u64,
}
```

---

## What Was Actually Implemented

### Status: **Shipped** — `kernel/src/sched/mod.rs`

The refactor was completed in full. The naming and a few design details differ from the original plan; this section documents the actual implementation.

---

### Per-Core State: `CoreState` (was `CpuScheduler`)

The implemented struct is named `CoreState` (not `CpuScheduler`):

```rust
pub struct CoreState {
    pub run_queue_high: VecDeque<usize>,
    pub run_queue_medium: VecDeque<usize>,
    pub run_queue_low: VecDeque<usize>,
    pub current_task: Option<usize>,   // was: current: Option<usize>
    pub current_ticks: u64,
    // NOTE: idle_context_rsp is NOT per-core; kept global in Scheduler
    // NOTE: local_tick is NOT present; global_tick in Scheduler is used
    // NOTE: need_resched is NOT per-core; global NEEDS_RESCHEDULE AtomicBool is used
}
```

**Divergences from plan:**
- Field `current` renamed to `current_task` to be unambiguous.
- `idle_context_rsp` was not moved into `CoreState`; it remains on `Scheduler` (single BSP idle context for Phase 0).
- `local_tick` was not added; `Scheduler::global_tick` is sufficient for Phase 0.
- `need_resched` was not made per-core; `NEEDS_RESCHEDULE: AtomicBool` remains global. Per-core reschedule flags are a Phase 1 TODO.

---

### Modified `Scheduler`

```rust
pub struct Scheduler {
    pub processes: Vec<Process>,
    pub cores: [CoreState; MAX_CORES],   // was: cpus: [CpuScheduler; MAX_CPUS]
    pub online_cores: usize,              // was: num_cpus: usize
    pub global_tick: u64,
    pub idle_context_rsp: u64,            // kept here (not per-core yet)
}
```

Constants:

```rust
pub const MAX_CORES: usize = 64;         // was: MAX_CPUS = 64
pub const ONLINE_CORES: AtomicUsize = AtomicUsize::new(1);
// BOOT_CPU_ID is implicitly 0; not stored as a named constant
```

`Scheduler::new()` is `const fn` and initializes the entire `cores` array at compile time using `[const { CoreState::new() }; MAX_CORES]`. All 64 `CoreState` entries are zeroed in BSS.

---

### `init_cores(total_cpus: usize)` — SMP Transition Point

Called from `main.rs` immediately after `smp::start_aps()` with the total CPU count from the Limine MP response:

```rust
// main.rs
smp::start_aps(cpus, bsp_lapic_id);
crate::sched::init_cores(cpus.len());  // Phase 0 → active
```

Implementation:

```rust
pub fn init_cores(total_cpus: usize) {
    let count = total_cpus.min(MAX_CORES).max(1);
    ONLINE_CORES.store(count, Ordering::Release);
    SCHEDULER.lock().online_cores = count;
    serial_println!("[SCHED] Per-core work-stealing scheduler: {} online core(s)", count);
}
```

Until `init_cores` is called, `ONLINE_CORES = 1` and all work goes to `cores[0]` (BSP).

---

### `current_cpu_id() -> usize`

Reads initial APIC ID from CPUID leaf 1 EBX\[31:24\]:

```rust
pub fn current_cpu_id() -> usize {
    let apic_id = core::arch::x86_64::__cpuid(1).ebx as usize >> 24;
    apic_id.min(MAX_CORES - 1)
}
```

BSP has APIC ID 0 on all supported platforms. APs are still parked (SMP Phase 0), so this always returns 0 on the BSP during Phase 0. When per-core LAPIC timers are wired (Phase 1), each AP's timer will call this and get its own APIC ID, mapping directly to `CORES[n]`.

**TODO (Phase 1):** Map non-contiguous LAPIC IDs to contiguous core indices using the Limine MP table, in case the platform uses non-zero or non-contiguous APIC IDs for the BSP.

---

### Queue Operations

#### `remove_from_ready_queues(idx)` — Now Scans All Cores

```rust
pub fn remove_from_ready_queues(&mut self, idx: usize) {
    let online = self.online_cores;
    for core_id in 0..online {
        let core = &mut self.cores[core_id];
        core.run_queue_high.retain(|&q| q != idx);
        core.run_queue_medium.retain(|&q| q != idx);
        core.run_queue_low.retain(|&q| q != idx);
    }
}
```

This satisfies the plan's "process index must not appear in more than one ready queue globally" invariant.

#### `enqueue_process(idx)` — Routes to Least-Loaded Core

```rust
pub fn enqueue_process(&mut self, idx: usize) {
    // validates idx and Ready state
    self.remove_from_ready_queues(idx);   // prevent duplicates
    let core_id = self.target_core_for_process(idx);
    self.enqueue_process_to_core(idx, core_id);
}
```

#### `target_core_for_process(idx) -> usize` — CPU Selection Policy

Implements the plan's CPU selection policy (least-loaded, respects `cpu_mask`):

```rust
fn target_core_for_process(&self, idx: usize) -> usize {
    let cpu_mask = self.processes[idx].cpu_mask;  // existing field, u64 bitmask
    // Iterate online cores allowed by cpu_mask; pick the one with fewest ready tasks
    // Fallback to core 0 (BSP) if no core is allowed.
}
```

**Divergence from plan:** The plan proposed using `last_cpu: Option<usize>` and a scoring function. The implementation uses the existing `cpu_mask: u64` field on `Process` (which was already there) combined with queue length. `last_cpu` was not added to `Process` to keep the patch minimal. A future refinement can add `last_cpu` and the scoring function from the plan:

```rust
// TODO (Phase 1): Add to Process struct:
// pub last_cpu: Option<usize>,
// pub latency_sensitive: bool,
//
// Then implement cpu_score_for_process() scoring:
// score = ready_len * 10
// score -= 8 if last_cpu == Some(core_id)
// score -= 4 if latency_sensitive
```

---

### Work Stealing: `steal_work(thief_id) -> Option<usize>`

**This replaces the plan's push-based `balance_load()`** with a pull-based work-stealing approach. When a core's local queues are empty, it steals from the tail of another core's lowest-priority tier:

```rust
pub fn steal_work(&mut self, thief_id: usize) -> Option<usize> {
    for victim_id in 0..self.online_cores {
        if victim_id == thief_id { continue; }
        // Skip victims with <= 1 task (leave them something to do)
        // Search victim's run_queue_low → medium → high (back-to-front)
        // Check cpu_mask before stealing
        // Return stolen process index
    }
    None
}
```

Stealing is called from `pick_next_bore(cpu_id)` and `pick_next_round_robin(cpu_id)` when local queues are empty.

**Divergence from plan:** The plan proposed a separate `balance_load()` push method. Work-stealing (pull on empty queue) was chosen instead because:
- It has lower overhead (only fires when a core is actually idle).
- It does not require a separate periodic invocation.
- It naturally respects `cpu_mask` affinity.

The two-phase find/steal pattern avoids Rust borrow-checker conflicts between `cores[victim_id]` and `processes`:

```rust
// Phase 1: immutable borrows of cores[victim] and processes simultaneously
let candidate = { let core = &self.cores[victim_id]; Self::find_stealable(&core.run_queue_low, &self.processes, thief_id) };
// Phase 2: mutable borrow of cores[victim] to remove the entry
if let Some((tier, pos)) = candidate { self.cores[victim_id].run_queue_low.remove(pos); }
```

**TODO (Phase 1):** When each `CoreState` gets its own `spin::Mutex`, change `steal_work` to use `try_lock()` on the victim's mutex so the thief's timer handler never blocks on a busy victim.

---

### `schedule_tick(cpu_id, saved_rsp) -> u64` — Central Dispatch

Replaces the 80-line inline scheduling block that previously lived in `timer_rust()` in `interrupts.rs`. The timer handler now calls:

```rust
// interrupts.rs — timer_rust()
let result = sched.schedule_tick(crate::sched::current_cpu_id(), saved_rsp);
```

`schedule_tick` does:
1. Check `NEEDS_RESCHEDULE` (returns 0 if not set).
2. Look up `cores[cpu_id].current_task`.
3. Call `account_and_apply_churn_penalty()`.
4. Save interrupted context (RSP + FS base) into `processes[current]`.
5. Transition `Running → Ready` and re-enqueue (BORE) or leave for RR.
6. Call `pick_next(cpu_id)` → `pick_next_round_robin(cpu_id)` or `pick_next_bore(cpu_id)`.
7. If a next process is found: update `cores[cpu_id].current_task`, switch CR3, update FS base, update TSS RSP0, return `next_rsp`.
8. If no next process: return 0 (stay on current context).

The scheduler lock is **never held across `iretq`** — `schedule_tick` returns the new RSP, drops the lock, and the naked `timer_entry` performs the `iretq` after the lock is released.

---

### Pick-Next: `cpu_id` Parameter Added

The original plan said to add `pick_next_round_robin_for_cpu(cpu_id)` while keeping `pick_next_round_robin()` as a compatibility wrapper.

**What was done instead:** The `cpu_id` parameter was added directly to the existing methods. There is no zero-argument compatibility wrapper because the only caller (`schedule_tick`) always has `cpu_id` available:

```rust
// New signatures (breaking change from original plan):
pub fn pick_next_bore(&mut self, cpu_id: usize) -> Option<usize>
pub fn pick_next_round_robin(&mut self, cpu_id: usize) -> Option<usize>
pub fn pick_next(&mut self, cpu_id: usize) -> Option<usize>
```

**TODO (if needed):** Add zero-arg wrappers that call `current_cpu_id()` internally, if external callers outside the timer handler need them.

---

### Current-Process Accessors

```rust
pub fn current_process(&self) -> &Process {
    let cpu_id = current_cpu_id();
    &self.processes[self.cores[cpu_id].current_task.unwrap_or(0)]
}

pub fn current_pid(&self) -> usize {
    let cpu_id = current_cpu_id();
    self.cores[cpu_id].current_task
        .and_then(|idx| self.processes.get(idx))
        .map(|p| p.pid)
        .unwrap_or(0)
}

pub fn current_process_rsp() -> u64 {
    let sched = SCHEDULER.lock();
    let cpu_id = current_cpu_id();
    match sched.cores[cpu_id].current_task {
        Some(idx) => sched.processes[idx].context_rsp,
        None => 0,   // no idle_context_rsp per-core yet
    }
}
```

The plan's `current_process_rsp_for_cpu(cpu_id)` helper was not added as a separate method since `current_process_rsp()` already uses `current_cpu_id()` internally.

**TODO (Phase 1):** When per-core idle loops are enabled, `current_process_rsp()` should fall back to `self.cores[cpu_id].idle_context_rsp` instead of 0.

---

### Runtime Accounting

`account_current_runtime()` and `account_and_apply_churn_penalty()` were updated to call `current_cpu_id()` internally. External signature is unchanged:

```rust
pub fn account_current_runtime(&mut self) -> u64 {
    let cpu_id = current_cpu_id();
    let current = match self.cores[cpu_id].current_task { Some(idx) => idx, None => return 0 };
    // ... same delta computation as before ...
}
```

**Divergence from plan:** The plan proposed `account_current_runtime_for_cpu(cpu_id)` + a compatibility wrapper. Since `current_cpu_id()` is always cheap (one CPUID instruction), the internal call was kept simple without a separate `_for_cpu` variant.

---

### `set_state(idx, new_state)` Behavior

The plan's requirements are satisfied:

- Non-runnable new state → `remove_from_ready_queues(idx)` (scans all cores).
- Ready new state → **does not** auto-enqueue (callers must call `enqueue_process` explicitly).
- Running new state → not placed in any queue (set only by `schedule_tick`).

**Note:** The plan said `set_state(idx, Ready)` should call `enqueue_process()`. The implementation does **not** do this to avoid double-enqueue bugs. Callers that want to make a process runnable call `set_state` then `enqueue_process` explicitly. This matches the pre-SMP behavior.

---

### Idle Context

`set_idle_context(rsp)` remains as-is. A per-core idle RSP was not implemented because APs are still parked (SMP Phase 0). When Phase 1 enables per-AP idle loops, each AP will call `set_idle_context` with its own RSP after `init_cores` has assigned it a `CoreState` slot.

**TODO (Phase 1):** Move `idle_context_rsp` from `Scheduler` into `CoreState` and update `set_idle_context` to use `current_cpu_id()`.

---

### Reschedule Flags

The plan's per-CPU reschedule flags (`need_resched` in `CpuScheduler`) were **not** implemented. The global `NEEDS_RESCHEDULE: AtomicBool` remains. `request_reschedule()` and `check_reschedule()` are unchanged.

**TODO (Phase 1):** Add `per_core_resched: [AtomicBool; MAX_CORES]` and:

```rust
pub fn request_reschedule_for_cpu(cpu_id: usize)
pub fn check_reschedule_for_cpu(cpu_id: usize) -> bool
```

When a high-priority or latency-sensitive task is enqueued on a remote CPU, send a reschedule IPI to that CPU (requires LAPIC ICR write support).

---

### Process Invariants Upheld

| Invariant | Status |
|---|---|
| A process runs on at most one CPU at a time | ✅ `schedule_tick` sets `state = Running` before returning the new RSP |
| A process appears in at most one ready queue globally | ✅ `remove_from_ready_queues` scans all core queues before enqueue |
| Running process not in any ready queue | ✅ `pick_next` pops from queue; process is never re-enqueued until it yields |
| Blocked/Finished process not queued | ✅ `set_state` removes from all queues on non-runnable transition |
| Scheduler lock not held across `iretq` | ✅ `schedule_tick` returns new RSP; lock drops before `timer_entry` does `iretq` |
| `terminate_process_by_pid` refuses to terminate a running process | ✅ Checks `cores[c].current_task == Some(idx)` for all online cores |

---

## Files Changed

| File | Change |
|---|---|
| `kernel/src/sched/mod.rs` | Full refactor: `CoreState`, `Scheduler`, `schedule_tick`, `steal_work`, `current_cpu_id`, `init_cores` |
| `kernel/src/arch/x86_64/interrupts.rs` | `timer_rust` replaced inline 80-line scheduling block with `sched.schedule_tick(current_cpu_id(), saved_rsp)`; fault handlers use `s.current_pid()` |
| `kernel/src/arch/x86_64/syscall.rs` | Two `sched.current` references updated to `cores[cpu_id].current_task` |
| `kernel/src/main.rs` | `init_cores(cpus.len())` called after `start_aps()` |

---

## SMP Phase Roadmap

### Phase 0 (Current — Shipped)
- [x] Per-core `CoreState` with tiered run queues
- [x] `schedule_tick(cpu_id, saved_rsp)` dispatch
- [x] Work-stealing (`steal_work`) with cpu_mask affinity
- [x] `current_cpu_id()` via CPUID (BSP = 0)
- [x] `init_cores(n)` called from main after SMP bring-up
- [x] BSP-only operation; APs still parked

### Phase 1 (Next — Not Yet Implemented)
- [ ] Per-core LAPIC timer initialization
- [ ] Each AP calls `schedule_tick(current_cpu_id(), saved_rsp)` from its own timer IRQ
- [ ] Per-core TSS (currently BSP TSS only)
- [ ] `idle_context_rsp` moved into `CoreState`
- [ ] Per-core `NEEDS_RESCHEDULE` flags + IPI for cross-core wakeup
- [ ] `try_lock` on per-`CoreState` mutex for steal_work (replace single global lock)
- [ ] `last_cpu: Option<usize>` on `Process` for cache-affinity scheduling
- [ ] `latency_sensitive: bool` on `Process` for audio-friendly policy
- [ ] LAPIC ID → core index mapping table (for non-contiguous APIC IDs)

### Phase 2 (Future)
- [ ] Split global `SCHEDULER` lock into process-table lock + per-core queue locks
- [ ] `balance_load()` push migration in addition to pull stealing
- [ ] NUMA-aware placement
- [ ] Real-time SCHED_FIFO bypass lane
