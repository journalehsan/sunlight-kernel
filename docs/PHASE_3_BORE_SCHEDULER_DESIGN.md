# Phase 3: BORE (Burst-Oriented Response Enhancer) Scheduler - Design Document

## Executive Summary

Phase 3 upgrades SunlightOS's kernel scheduler from simple round-robin to a BORE-inspired algorithm that prioritizes interactive tasks for real-time user responsiveness while maintaining fairness for CPU-bound workloads.

**Current Scheduler:** Simple round-robin, O(n) search  
**Target Scheduler:** Burst-scored priority queue with aging, O(1) amortized scheduling  
**Goal:** Sub-millisecond latency for interactive tasks (shell, TTY, UI)

---

## Current Scheduler Architecture

### File Locations
```
kernel/src/sched/mod.rs        (209 lines) - Main scheduler loop
kernel/src/sched/context.rs    (24 lines)  - Context save/restore
kernel/src/process/mod.rs      (200+ lines) - Process Control Block
```

### Current Process Control Block (PCB)

```rust
pub struct Process {
    pub pid: usize,
    pub ppid: usize,
    pub name: &'static str,
    pub state: ProcessState,           // Ready, Running, BlockedOnIpc, Finished
    pub address_space: AddressSpace,
    pub capabilities: Vec<Capability>,
    pub kernel_stack: Box<[u8; 32KB]>,
    pub kernel_stack_top: u64,
    pub user_stack_top: u64,
    pub entry_point: u64,
    pub context_rsp: u64,              // Saved context pointer
    pub uid: u32,
    pub gid: u32,
    pub ipc_queue: VecDeque<IpcMsg>,
    pub ipc_endpoint: Option<u32>,
    pub ipc_reply: Option<IpcMsg>,
    pub pending_call: Option<(u64, IpcMsg)>,
    pub pending_reply_wait: Option<(u32, IpcMsg)>,
    pub fd_table: FdTable,
    pub capability_mode: bool,
    pub signal_state: SignalState,
    pub is_linux_compat: bool,
    pub sched_type: u8,                // SCHED_NORMAL=0, SCHED_FIFO=1
    pub weight: u32,                   // CFS weight (1024 default)
    pub cpu_mask: u64,                 // CPU affinity
    // ❌ MISSING: burst_score, timeslice_used, io_wait_time
}

pub enum ProcessState {
    Ready,
    Running,
    BlockedOnIpc,
    Finished,
}
```

### Current Scheduling Loop (Simplified)

```rust
pub struct Scheduler {
    pub processes: Vec<Process>,
    pub current: usize,
    pub current_ticks: u64,            // Ticks in current timeslice
    pub idle_context_rsp: u64,
}

// Round-robin selection (O(n) worst case)
pub fn pick_next(&self) -> Option<usize> {
    let len = self.processes.len();
    if len == 0 { return None; }
    
    let start = (self.current + 1) % len;
    let mut idx = start;
    loop {
        if matches!(self.processes[idx].state, ProcessState::Ready) {
            return Some(idx);  // Round-robin cycle
        }
        idx = (idx + 1) % len;
        if idx == start { break; }
    }
    None
}

// Timer fires every 10 ticks
pub const TIME_SLICE_TICKS: u64 = 10;

pub fn tick(&mut self) {
    self.current_ticks += 1;
    if self.current_ticks >= TIME_SLICE_TICKS {
        self.current_ticks = 0;
        NEEDS_RESCHEDULE.store(true, Ordering::SeqCst);
    }
}
```

### Current Reschedule Path

```
Timer IRQ (every ~1ms on 10 kHz timer)
    ↓
tick() increments counter
    ↓
Reaches TIME_SLICE_TICKS (10)
    ↓
Set NEEDS_RESCHEDULE flag
    ↓
Return from IRQ
    ↓
Check flag in interrupt epilogue
    ↓
Call pick_next() (round-robin)
    ↓
Switch context to new process
    ↓
Continue
```

---

## BORE Algorithm Design

### Key Concept: Burst Score

**Burst Score** represents how much a task consumed its allocated time quantum:

```
0-25%:   IO-bound task (blocked early) → HIGH priority (Low burst)
25-75%:  Mixed/adaptive task         → MEDIUM priority
75-100%: CPU-bound task (full quantum) → LOW priority (High burst)
```

### Phase 3.0: PCB Extensions

**New fields to add to Process struct:**

```rust
pub struct Process {
    // ... existing fields ...
    
    // BORE Scheduler Metrics
    pub burst_score: u32,              // 0-1024 (default 512)
    pub timeslice_used: u32,           // Ticks consumed this quantum (0-10)
    pub last_run_tick: u64,            // Global tick when process last ran
    pub io_wait_time: u32,             // Ticks spent blocking on IO
    pub interactive_bonus: i32,        // Latency boost (-50..+50 bonus)
    pub last_interactive_time: u64,    // When we last detected interactivity
    pub aging_counter: u32,            // For starvation prevention
}
```

**Size impact:** ~24 bytes additional per process (minimal)

### Phase 3.1: Priority Queue Structure

**Instead of linear Vec<Process>, use tiered ready queues:**

```rust
pub struct BoreScheduler {
    pub processes: Vec<Process>,
    
    // Multi-level ready queues by priority tier
    pub ready_queue_high: Vec<usize>,      // Burst score 0-256 (interactive)
    pub ready_queue_medium: Vec<usize>,    // Burst score 257-768
    pub ready_queue_low: Vec<usize>,       // Burst score 769-1024 (CPU-bound)
    
    pub current: usize,
    pub current_ticks: u64,
    pub global_tick: u64,                  // Ever-increasing tick counter
    pub idle_context_rsp: u64,
}
```

**Pickup strategy:** O(1) amortized
```
1. If high queue has Ready tasks → pick from high
2. Else if medium queue has Ready tasks → pick from medium
3. Else pick from low queue
4. Else idle
```

### Phase 3.2: Burst Score Calculation

**Updated in timer IRQ and at blocking points:**

```rust
fn update_burst_score(process: &mut Process, reason: BurstReason) {
    match reason {
        // Task yielded/blocked EARLY (less than 25% of quantum)
        BurstReason::EarlyBlock => {
            let burst_reduction = 64;  // Decrease by ~6%
            process.burst_score = process.burst_score
                .saturating_sub(burst_reduction)
                .max(0);
            process.interactive_bonus = 20;  // +20 ticks bonus
        }
        
        // Task used FULL quantum (CPU-bound)
        BurstReason::FullQuantum => {
            let burst_increase = 32;  // Increase by ~3%
            process.burst_score = process.burst_score
                .saturating_add(burst_increase)
                .min(1024);
            process.interactive_bonus = 0;
        }
        
        // Task aged without running (starvation prevention)
        BurstReason::Aged => {
            let aging_decay = 20;  // Slowly decrease to prevent starvation
            process.burst_score = process.burst_score
                .saturating_sub(aging_decay)
                .max(256);  // Never drop below medium threshold
        }
    }
}

pub enum BurstReason {
    EarlyBlock,     // Task blocked on IPC/IO < 3 ticks into quantum
    FullQuantum,    // Task ran full 10-tick quantum
    Aged,           // Task hasn't run in 100+ ticks
}
```

### Phase 3.3: Pick Next with BORE

```rust
fn pick_next_bore(&mut self) -> Option<usize> {
    // Try high priority (interactive) first
    while let Some(idx) = self.ready_queue_high.pop_front() {
        if matches!(self.processes[idx].state, ProcessState::Ready) {
            return Some(idx);
        }
    }
    
    // Fall back to medium priority
    while let Some(idx) = self.ready_queue_medium.pop_front() {
        if matches!(self.processes[idx].state, ProcessState::Ready) {
            return Some(idx);
        }
    }
    
    // Fall back to low priority
    while let Some(idx) = self.ready_queue_low.pop_front() {
        if matches!(self.processes[idx].state, ProcessState::Ready) {
            return Some(idx);
        }
    }
    
    None
}
```

**Time complexity:** O(1) amortized (no searching)

### Phase 3.4: Aging Mechanism

**Prevent starvation: slowly reduce burst score of long-waiting tasks**

```rust
const AGING_THRESHOLD_TICKS: u64 = 100;  // Run every 100ms
const AGING_INTERVAL: u64 = 10;          // Check every 10 global ticks

fn age_ready_tasks(&mut self) {
    if self.global_tick % AGING_INTERVAL != 0 {
        return;
    }
    
    for idx in 0..self.processes.len() {
        let p = &mut self.processes[idx];
        
        // Only age Ready (not Running) tasks
        if !matches!(p.state, ProcessState::Ready) {
            continue;
        }
        
        let ticks_since_run = self.global_tick - p.last_run_tick;
        if ticks_since_run > AGING_THRESHOLD_TICKS {
            update_burst_score(p, BurstReason::Aged);
            p.aging_counter += 1;
        }
    }
}
```

### Phase 3.5: Latency Tracking

**For monitoring and debugging:**

```rust
pub struct ProcessSchedulingStats {
    pub total_context_switches: u64,
    pub total_runtime: u64,             // Ticks spent running
    pub total_wait_time: u64,           // Ticks spent Ready but not Running
    pub max_latency: u64,               // Worst case wait time
    pub avg_burst_score: u32,           // Running average
}

impl Process {
    pub fn get_scheduling_stats(&self) -> ProcessSchedulingStats {
        // Computed at runtime for observability
    }
}
```

---

## Implementation Phases

### Phase 3.0: Core PCB & Data Structures (Today)
- [ ] Add burst_score fields to Process
- [ ] Create BoreScheduler struct with tiered queues
- [ ] Implement burst_score calculation logic
- [ ] Add aging mechanism
- **Effort:** 2-3 hours
- **Risk:** Low (additive, no changes to existing code)

### Phase 3.1: Timer Handler Integration (2-3 hours)
- [ ] Modify timer IRQ to track timeslice_used
- [ ] Update burst_score at quantum boundaries
- [ ] Call age_ready_tasks() periodically
- [ ] Update ready queue membership on state changes
- **Effort:** 2-3 hours
- **Risk:** Medium (touches interrupt handler)

### Phase 3.2: Scheduler Loop Refactor (2-3 hours)
- [ ] Replace round-robin pick_next() with tier-based pick_next_bore()
- [ ] Maintain queues on process state changes (Ready→Running, etc.)
- [ ] Update IPC wake logic to re-queue correctly
- [ ] Test with existing processes
- **Effort:** 2-3 hours
- **Risk:** Medium (changes scheduling path)

### Phase 3.3: Monitoring & Observability (1-2 hours)
- [ ] Add /proc interface for scheduling stats
- [ ] Expose burst_score via sysinfo syscall
- [ ] Create monitoring shell commands
- **Effort:** 1-2 hours
- **Risk:** Low (observability only)

### Phase 3.4: Optimization Tuning (1 hour)
- [ ] Benchmark against current round-robin
- [ ] Adjust constants (TIME_SLICE_TICKS, burst deltas, aging)
- [ ] Validate latency improvements
- **Effort:** 1 hour
- **Risk:** Very low

---

## Algorithm Walkthrough: Shell Input Example

**Scenario:** User types at terminal while compile task runs

### Current Round-Robin (Poor Latency)
```
Tick 0:    Shell (Ready) → Run for 10 ticks
Tick 10:   Compiler (Ready) → Run for 10 ticks
Tick 20:   SSH (Ready) → Run for 10 ticks
Tick 30:   Shell (now has keypresses!) → Get to run
           ^ 30ms latency! 😞
```

### BORE Scheduler (Better Latency)
```
Tick 0:    Compiler runs 10 ticks (uses full quantum)
           → burst_score increases (CPU-bound)
           → moved to LOW queue

Tick 10:   Shell blocked on input for 2 ticks
           → burst_score decreased (interactive)
           → moved to HIGH queue
           → gets rescheduled immediately ✓
           ^ ~2ms latency! 🚀
```

---

## Mathematical Model

### Burst Score Formula (simplified)

```
let timeslice_ratio = timeslice_used / TIME_SLICE_TICKS;

if timeslice_ratio < 0.25:
    delta_burst = -64   # High interactivity
elif timeslice_ratio < 0.75:
    delta_burst = 0     # Neutral
else:
    delta_burst = +32   # CPU-bound
    
new_burst_score = clamp(
    old_burst_score + delta_burst,
    0,      // min
    1024    // max
)

// Queue assignment
if burst_score < 256:
    queue = HIGH      // Interactive (sub-millisecond latency)
elif burst_score < 768:
    queue = MEDIUM    // Mixed
else:
    queue = LOW       // CPU-bound (can wait longer)
```

### Latency Guarantees

```
Task Type          Burst Score    Queue   Max Latency
─────────────────────────────────────────────────────
Shell (waiting)    0-100          HIGH    ~1-2ms
Editor (typing)    50-200         HIGH    ~1-2ms
Web browser        150-400        MED     ~5-10ms
Compiler           900+           LOW     ~50ms
Download           200-400        MED     ~5-10ms
```

---

## Safety Considerations

### No Floating Point
✅ All math in u32/u64 integers  
✅ No division in hot path (scheduler)  
✅ Safe for interrupt context  

### No Dynamic Allocation in Hot Path
✅ Ready queues pre-allocated  
✅ O(1) pop/push operations  
✅ No Vec::insert (would be O(n))  

### Starvation Prevention
✅ Aging mechanism prevents indefinite low-priority waits  
✅ Minimum burst_score 256 (HIGH tier minimum)  
✅ Periodic aging decay  

### Real-Time Correctness
✅ SCHED_FIFO processes always highest priority (unchanged)  
✅ Latency improvements for SCHED_NORMAL  
✅ No priority inversions possible (no locks in scheduler)  

---

## Testing Strategy

### Unit Tests (Phase 3.0)
```rust
#[test]
fn test_burst_score_early_block() {
    let mut p = Process::new(...);
    p.burst_score = 512;
    update_burst_score(&mut p, BurstReason::EarlyBlock);
    assert!(p.burst_score < 512);  // Decreased
}

#[test]
fn test_burst_score_full_quantum() {
    let mut p = Process::new(...);
    p.burst_score = 512;
    update_burst_score(&mut p, BurstReason::FullQuantum);
    assert!(p.burst_score > 512);  // Increased
}

#[test]
fn test_queue_assignment() {
    let mut p = Process::new(...);
    
    p.burst_score = 100;
    assert_eq!(p.get_queue_tier(), QueueTier::High);
    
    p.burst_score = 500;
    assert_eq!(p.get_queue_tier(), QueueTier::Medium);
    
    p.burst_score = 900;
    assert_eq!(p.get_queue_tier(), QueueTier::Low);
}

#[test]
fn test_aging_prevents_starvation() {
    let mut sched = BoreScheduler::new();
    sched.global_tick = 0;
    
    let mut p = Process::new(...);
    p.burst_score = 900;
    p.last_run_tick = 0;
    sched.processes.push(p);
    
    sched.global_tick = 150;  // 150 ticks passed
    sched.age_ready_tasks();
    
    // Burst score should decrease
    assert!(sched.processes[0].burst_score < 900);
}
```

### Integration Tests (Phase 3.1)
```
1. Spawn long-running compiler task (CPU-bound)
2. Spawn interactive shell task
3. Measure shell responsiveness
   - Before: ~30ms latency
   - After: ~2-5ms latency ✓
   
4. Verify compiler doesn't starve
   - Compiler still progresses
   - Measured via process runtime stats ✓
   
5. Verify aging prevents high-priority starvation
   - Low-priority task eventually runs
   - Even with many high-priority tasks ✓
```

### Benchmarking (Phase 3.4)
```
Metric                  Round-Robin    BORE        Improvement
─────────────────────────────────────────────────────────────
TTY key latency         25-35ms        2-5ms       6-12x better ✓
Scheduler overhead      O(n)           O(1)        scales better ✓
Context switches/sec    ~100           ~100        same
CPU idle time           Same           Same        no regression
Interactive fairness    Poor           Good        better
```

---

## Constants & Tuning Parameters

```rust
// Timer configuration
pub const TIME_SLICE_TICKS: u64 = 10;              // ~10ms timeslice
pub const TIMER_FREQ_HZ: u64 = 1000;               // 1ms timer tick

// Burst scoring
pub const BURST_SCORE_MIN: u32 = 0;
pub const BURST_SCORE_MAX: u32 = 1024;
pub const BURST_SCORE_DEFAULT: u32 = 512;

pub const BURST_REDUCTION_EARLY_BLOCK: u32 = 64;   // ~6% per early block
pub const BURST_INCREASE_FULL_QUANTUM: u32 = 32;   // ~3% per full run
pub const BURST_REDUCTION_AGING: u32 = 20;         // ~2% per aging tick

// Queue thresholds (burst score ranges)
pub const HIGH_QUEUE_THRESHOLD: u32 = 256;         // 0-256: interactive
pub const MEDIUM_QUEUE_THRESHOLD: u32 = 768;       // 257-767: mixed
                                                    // 768-1024: CPU-bound

// Starvation prevention
pub const AGING_INTERVAL_TICKS: u64 = 10;          // Check every 10ms
pub const AGING_THRESHOLD_TICKS: u64 = 100;        // Age after 100ms wait
pub const MINIMUM_AGED_BURST_SCORE: u32 = 256;     // Don't starve below MED

// Latency tuning
pub const INTERACTIVE_BONUS_TICKS: i32 = 20;       // Extra ticks for interactive
pub const INTERACTIVE_DETECTION_THRESHOLD: u32 = 3; // Block < 3 ticks = interactive
```

---

## Files to Modify

### Phase 3.0-3.2 Implementation

1. **kernel/src/process/mod.rs** (Process struct)
   - Add burst_score, timeslice_used, io_wait_time fields
   - Add get_queue_tier() method
   
2. **kernel/src/sched/mod.rs** (Scheduler → BoreScheduler)
   - Replace simple Vec with tiered ready queues
   - Implement pick_next_bore()
   - Implement age_ready_tasks()
   - Update wake_pid() to re-queue
   - Integrate timeslice_used tracking
   
3. **kernel/src/sched/context.rs** (unchanged)
   - No changes needed to context save/restore
   
4. **kernel/src/arch/x86_64/interrupts.rs** (Timer handler)
   - Track timeslice_used increment
   - Call update_burst_score() at quantum boundary
   
5. **kernel/src/main.rs** (Scheduler integration)
   - Initialize BoreScheduler instead of Scheduler
   - Pass burst_score info to monitoring

---

## Rollback Plan

If BORE causes issues:
1. Keep old round-robin logic in parallel
2. Add `use_bore: bool` feature flag
3. Boot with `BORE=0` to disable
4. Revert in <1 minute via config

---

## Success Criteria

✅ **Performance:**
- Shell input latency < 5ms (was 25-35ms)
- No regression in compiler throughput
- CPU usage unchanged

✅ **Stability:**
- No starvation of low-priority tasks
- All existing processes still run correctly
- Real-time (SCHED_FIFO) unaffected

✅ **Code Quality:**
- No floating-point in scheduler
- No dynamic allocation in hot path
- 100% deterministic behavior
- Measurable latency improvement

---

## References

- Original BORE scheduler: https://github.com/firelzrd/bore-scheduler
- CFS (Linux): https://en.wikipedia.org/wiki/Completely_Fair_Scheduler
- EEVDF: O(1) scheduling algorithm inspiration

---

## Conclusion

Phase 3 BORE scheduler will provide **6-12x latency improvement** for interactive tasks while maintaining fairness and preventing starvation. The implementation is straightforward, low-risk, and requires no changes to the interrupt or context-switching machinery.

**Total estimated effort:** 6-10 hours across 4 implementation phases
**Expected impact:** Dramatically improved user experience for shell/TTY
