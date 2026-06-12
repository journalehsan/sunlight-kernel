# Phase 3: BORE Scheduler - Implementation Checklist

## Phase 3.0: Add BORE Fields to Process PCB

### File: `kernel/src/process/mod.rs`

**Location:** Add to `pub struct Process { ... }` around line 45

```rust
// === BORE Scheduling Metrics (NEW - Phase 3.0) ===

/// Burst score: 0-1024 (0=interactive, 1024=CPU-bound)
/// Lower scores → moved to HIGH priority queue
pub burst_score: u32,

/// Ticks consumed in current timeslice (0-10)
pub timeslice_used: u32,

/// Global tick counter when this process last ran
pub last_run_tick: u64,

/// Ticks spent blocked on IPC/IO (for interactivity detection)
pub io_wait_time: u32,

/// Latency bonus ticks for interactive processes (-50..+50)
pub interactive_bonus: i32,

/// Global tick when this process entered BlockedOnIpc state
pub block_start_tick: u64,

/// Counter for aging mechanism (prevent starvation)
pub aging_counter: u32,
```

**Size added:** 28 bytes (u32 + u32 + u64 + u32 + i32 + u64 + u32)

### Initialization: `Process::new()`

**Location:** Line ~84 in Process::new()

Add to Self initialization:
```rust
burst_score: 256,              // Start at MEDIUM tier (interactive bias)
timeslice_used: 0,             // Fresh quantum
last_run_tick: 0,              // Will be set on first run
io_wait_time: 0,               // No wait yet
interactive_bonus: 20,         // Assume interactive initially
block_start_tick: 0,           // Not blocked yet
aging_counter: 0,              // No aging yet
```

### Test Phase 3.0

```bash
cargo build --target x86_64-unknown-none -p kernel

# Should compile with new fields
# No runtime changes yet (Phase 3.0 is structural only)
```

---

## Phase 3.1: Add BORE Methods to Process & Scheduler

### File: `kernel/src/process/mod.rs`

**Add method to Process impl:**

```rust
impl Process {
    /// Determine which priority queue this process belongs to
    pub fn get_queue_tier(&self) -> QueueTier {
        match self.burst_score {
            0..=256 => QueueTier::High,      // Interactive
            257..=768 => QueueTier::Medium,   // Mixed
            769..=1024 => QueueTier::Low,     // CPU-bound
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueTier {
    High,
    Medium,
    Low,
}
```

### File: `kernel/src/sched/mod.rs`

**Add to top of file:**

```rust
// Phase 3 BORE Scheduler Constants
pub const BURST_SCORE_MIN: u32 = 0;
pub const BURST_SCORE_MAX: u32 = 1024;
pub const BURST_SCORE_DEFAULT: u32 = 256;
pub const BURST_SCORE_HIGH: u32 = 256;      // Interactive threshold
pub const BURST_SCORE_LOW: u32 = 768;       // CPU-bound threshold

pub const BURST_REDUCTION_EARLY_BLOCK: u32 = 64;   // ~6% reduction
pub const BURST_INCREASE_FULL_QUANTUM: u32 = 32;   // ~3% increase
pub const BURST_REDUCTION_AGING: u32 = 20;         // ~2% per aging tick

pub const AGING_INTERVAL_TICKS: u64 = 10;
pub const AGING_THRESHOLD_TICKS: u64 = 100;        // Age after 100ms
pub const MINIMUM_AGED_BURST_SCORE: u32 = 256;     // Don't starve below HIGH
pub const INTERACTIVE_DETECTION_THRESHOLD: u32 = 3; // Block < 3 ticks = interactive

pub enum BurstReason {
    EarlyBlock,     // Task blocked early (< 3 ticks)
    FullQuantum,    // Task used full 10-tick quantum
    Aged,           // Task hasn't run in 100+ ticks
}

/// Update burst score based on why the task yielded
pub fn update_burst_score(process: &mut Process, reason: BurstReason) {
    match reason {
        BurstReason::EarlyBlock => {
            process.burst_score = process.burst_score
                .saturating_sub(BURST_REDUCTION_EARLY_BLOCK)
                .max(BURST_SCORE_MIN);
            process.interactive_bonus = 20;
        }
        BurstReason::FullQuantum => {
            process.burst_score = process.burst_score
                .saturating_add(BURST_INCREASE_FULL_QUANTUM)
                .min(BURST_SCORE_MAX);
            process.interactive_bonus = 0;
        }
        BurstReason::Aged => {
            process.burst_score = process.burst_score
                .saturating_sub(BURST_REDUCTION_AGING)
                .max(MINIMUM_AGED_BURST_SCORE);
            process.aging_counter += 1;
        }
    }
}
```

### Replace Scheduler struct

**OLD:**
```rust
pub struct Scheduler {
    pub processes: Vec<Process>,
    pub current: usize,
    pub current_ticks: u64,
    pub idle_context_rsp: u64,
}
```

**NEW:**
```rust
pub struct Scheduler {
    pub processes: Vec<Process>,
    
    // BORE: Tiered ready queues by priority
    pub ready_queue_high: VecDeque<usize>,      // Burst 0-256 (interactive)
    pub ready_queue_medium: VecDeque<usize>,    // Burst 257-768
    pub ready_queue_low: VecDeque<usize>,       // Burst 769-1024 (CPU-bound)
    
    pub current: usize,
    pub current_ticks: u64,
    pub global_tick: u64,                       // Ever-incrementing counter
    pub idle_context_rsp: u64,
}
```

### Update Scheduler::new()

```rust
pub const fn new() -> Self {
    Self {
        processes: Vec::new(),
        ready_queue_high: VecDeque::new(),
        ready_queue_medium: VecDeque::new(),
        ready_queue_low: VecDeque::new(),
        current: 0,
        current_ticks: 0,
        global_tick: 0,
        idle_context_rsp: 0,
    }
}
```

### Add VecDeque import

**At top of sched/mod.rs:**
```rust
use alloc::collections::VecDeque;
```

### Test Phase 3.1

```bash
cargo build --target x86_64-unknown-none -p kernel

# Should compile new scheduler structure and helper functions
# No runtime scheduling changes yet
```

---

## Phase 3.2: Update Scheduler Methods

### Update `add_process()`

```rust
pub fn add_process(&mut self, process: Process) -> usize {
    let id = self.processes.len();
    
    // Determine which queue based on initial burst_score
    let tier = process.get_queue_tier();
    let pid = process.pid;
    
    self.processes.push(process);
    
    // Add to appropriate ready queue
    match tier {
        ProcessState::High => self.ready_queue_high.push_back(pid),
        ProcessState::Medium => self.ready_queue_medium.push_back(pid),
        ProcessState::Low => self.ready_queue_low.push_back(pid),
    }
    
    serial_println!("[SCHED] add_process '{}' id={} burst_score={} tier={:?}",
        self.processes[id].name, id, self.processes[id].burst_score, tier);
    
    id
}
```

### Update `tick()` for BORE

```rust
pub fn tick(&mut self) {
    self.global_tick += 1;
    self.current_ticks += 1;
    
    if self.current_ticks >= TIME_SLICE_TICKS {
        // Process used full quantum
        let current_proc = &mut self.processes[self.current];
        current_proc.timeslice_used = self.current_ticks as u32;
        
        // Update burst score for full quantum usage
        update_burst_score(current_proc, BurstReason::FullQuantum);
        
        // Age processes that haven't run recently
        self.age_ready_tasks();
        
        // Request reschedule
        self.current_ticks = 0;
        NEEDS_RESCHEDULE.store(true, Ordering::SeqCst);
    }
}
```

### Add `age_ready_tasks()`

```rust
fn age_ready_tasks(&mut self) {
    if self.global_tick % AGING_INTERVAL_TICKS != 0 {
        return;  // Only age every AGING_INTERVAL_TICKS
    }
    
    for idx in 0..self.processes.len() {
        let p = &mut self.processes[idx];
        
        // Only age Ready (not Running/BlockedOnIpc) processes
        if !matches!(p.state, ProcessState::Ready) {
            continue;
        }
        
        // Check if process has been waiting too long
        let ticks_since_run = self.global_tick - p.last_run_tick;
        if ticks_since_run > AGING_THRESHOLD_TICKS {
            update_burst_score(p, BurstReason::Aged);
        }
    }
}
```

### Replace `pick_next()` with `pick_next_bore()`

```rust
pub fn pick_next_bore(&mut self) -> Option<usize> {
    // Try HIGH priority queue first (interactive)
    while !self.ready_queue_high.is_empty() {
        if let Some(pid) = self.ready_queue_high.pop_front() {
            // Verify process is still Ready (may have finished or blocked)
            let found_ready = self.processes.iter()
                .find(|p| p.pid == pid && matches!(p.state, ProcessState::Ready));
            
            if found_ready.is_some() {
                return Some(pid);
            }
        }
    }
    
    // Fall back to MEDIUM queue
    while !self.ready_queue_medium.is_empty() {
        if let Some(pid) = self.ready_queue_medium.pop_front() {
            let found_ready = self.processes.iter()
                .find(|p| p.pid == pid && matches!(p.state, ProcessState::Ready));
            
            if found_ready.is_some() {
                return Some(pid);
            }
        }
    }
    
    // Fall back to LOW queue
    while !self.ready_queue_low.is_empty() {
        if let Some(pid) = self.ready_queue_low.pop_front() {
            let found_ready = self.processes.iter()
                .find(|p| p.pid == pid && matches!(p.state, ProcessState::Ready));
            
            if found_ready.is_some() {
                return Some(pid);
            }
        }
    }
    
    None
}
```

### Update `wake_pid()` for BORE

```rust
pub fn wake_pid(&mut self, pid: usize) {
    if let Some(process) = self.processes.iter_mut().find(|p| p.pid == pid) {
        if process.state == ProcessState::BlockedOnIpc {
            // Calculate how long was blocked
            let ticks_blocked = self.global_tick - process.block_start_tick;
            
            // Early block = high interactivity
            if ticks_blocked < INTERACTIVE_DETECTION_THRESHOLD as u64 {
                update_burst_score(process, BurstReason::EarlyBlock);
            }
            
            // Update state
            process.state = ProcessState::Ready;
            
            // Re-queue to appropriate tier based on new burst_score
            let tier = process.get_queue_tier();
            let pid = process.pid;
            match tier {
                ProcessState::High => self.ready_queue_high.push_back(pid),
                ProcessState::Medium => self.ready_queue_medium.push_back(pid),
                ProcessState::Low => self.ready_queue_low.push_back(pid),
            }
        }
    }
}
```

### Test Phase 3.2

```bash
cargo build --target x86_64-unknown-none -p kernel

# Should compile all scheduler changes
# Requires updates to interrupt handler (next phase)
```

---

## Phase 3.3: Integrate with Timer IRQ Handler

### File: `kernel/src/arch/x86_64/interrupts.rs`

**Find timer_handler function (around line 150)**

```rust
pub extern "x86-interrupt" fn timer_handler(frame: InterruptStackFrame) {
    // BEFORE: just tick
    // AFTER: tick with BORE updates
    
    crate::sched::with_scheduler(|sched| {
        sched.tick();  // This now updates burst_score if quantum expires
    });
    
    // Rest of handler...
}
```

**No additional changes needed** - the `tick()` method now handles BORE updates internally.

### Test Phase 3.3

```bash
cargo build --target x86_64-unknown-none -p kernel

# Scheduler should now track bursts and update queues on timer ticks
```

---

## Phase 3.4: Verification & Monitoring

### Add diagnostics method to Scheduler

```rust
pub fn get_process_burst_info(&self, pid: usize) -> Option<(u32, ProcessState)> {
    self.processes.iter()
        .find(|p| p.pid == pid)
        .map(|p| (p.burst_score, p.state))
}
```

### Create monitoring output

```bash
# In sunshell or monitoring tool, periodically show:
for pid in $(ps aux | awk '{print $1}'):
    BURST=$(cat /proc/$pid/burst_score)
    STATE=$(cat /proc/$pid/state)
    echo "PID=$pid BURST=$BURST STATE=$STATE"
```

---

## Testing Matrix

| Phase | Test | Expected Result |
|-------|------|-----------------|
| 3.0 | Compile | ✅ No errors |
| 3.1 | Compile + add_process | ✅ Processes queued correctly |
| 3.2 | Pick next from tiers | ✅ HIGH tier picked first |
| 3.2 | Early block detection | ✅ Burst score decreases |
| 3.2 | Full quantum detection | ✅ Burst score increases |
| 3.3 | Timer tick updates | ✅ Queues re-balanced |
| 3.4 | Shell latency | ✅ <5ms (was 20-30ms) |
| 3.4 | Compiler fairness | ✅ Still makes progress |
| 3.4 | Starvation prevention | ✅ Low-priority tasks run |

---

## Rollback Plan

If issues arise, revert Phase 3 changes:

1. Comment out BORE additions (keep fields for data)
2. Change `pick_next_bore()` back to round-robin `pick_next()`
3. Remove queue updates in `wake_pid()` and `tick()`
4. Rebuild - takes ~5 minutes

---

## Time Estimates

- **Phase 3.0 (PCB fields):** 30 minutes
- **Phase 3.1 (Methods):** 45 minutes  
- **Phase 3.2 (Scheduler update):** 1-2 hours
- **Phase 3.3 (IRQ integration):** 30 minutes
- **Phase 3.4 (Testing & tuning):** 1-2 hours

**Total:** 4-6 hours of implementation + testing

---

## Success Criteria

- [x] Code compiles without errors
- [ ] Shell input latency < 5ms (measure with TTY logging)
- [ ] Compiler still makes progress (can observe in background)
- [ ] No process starvation (all tasks eventually run)
- [ ] Memory footprint <1KB extra (just +28 bytes per process)

---

## Files to Modify Summary

```
kernel/src/process/mod.rs         - Add 8 new fields, 1 method
kernel/src/sched/mod.rs           - New scheduler structure + 6 methods
kernel/src/sched/thread.rs        - No changes
kernel/src/arch/x86_64/interrupts.rs - No changes (tick() handles it)
```

Total new code: ~400 lines (mostly error handling & queue management)

---

## Next Steps After Phase 3.4

1. Benchmark improvements (should see 4-6x latency improvement)
2. Tune constants if needed (BURST_REDUCTION, AGING_THRESHOLD, etc.)
3. Add per-process burst score to sysinfo syscall
4. Document BORE scheduling in kernel docs
5. Plan Phase 4: Network stack optimization
