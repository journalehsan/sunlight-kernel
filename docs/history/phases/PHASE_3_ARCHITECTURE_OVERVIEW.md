# Phase 3: BORE Scheduler - Complete Architecture Overview

## High-Level Vision

Transform SunlightOS from a simple round-robin scheduler into a burst-aware, interactive-task-optimized scheduler that maintains fairness through aging and starvation prevention.

```
Current State (Phase 2):                  Target State (Phase 3):
┌─────────────────────────────┐          ┌─────────────────────────────┐
│   Simple Round-Robin        │          │   BORE Scheduler            │
├─────────────────────────────┤          ├─────────────────────────────┤
│ • O(n) pick_next()          │    →     │ • O(1) amortized pick_next()│
│ • 20-30ms shell latency     │          │ • 2-5ms shell latency       │
│ • No priority tiers         │          │ • 3 priority tiers          │
│ • No fairness mechanism     │          │ • Aging prevents starvation │
│ • All tasks equal           │          │ • Burst-score tracking      │
└─────────────────────────────┘          └─────────────────────────────┘
         6-12x better latency! 🚀
```

---

## Data Structure Evolution

### Before BORE (Flat Linear Scheduler)

```
┌─────────────────────────────────────────────────────────────┐
│ Vec<Process> - All processes in single flat array           │
├─────────┬────────┬──────────┬────────┬─────────┬────────────┤
│ [0]Init │[1]TTY  │[2]Timer  │[3]SSH  │[4]Shell│[5]Compiler │
└─────────┴────────┴──────────┴────────┴─────────┴────────────┘
           ↑ Round-robin search: O(n) worst case
      
Selection: "Find any Ready task"
  → Might check: TTY (BlockedOnIpc), Timer (BlockedOnIpc), 
                 SSH (BlockedOnIpc), Shell (Ready!) ← finally!
  → Latency: 1-10+ comparisons
```

### After BORE (Tiered Priority Scheduler)

```
┌─────────────────────────────────────────────────────────────┐
│ BORE Scheduler with Tiered Ready Queues                     │
├─────────────────────────────────────────────────────────────┤
│                                                              │
│  HIGH Queue (Burst 0-256 - Interactive):                    │
│  ┌──────────────────────────────────────┐                   │
│  │ [Shell(100)] → [Init(256)]           │ ← Pick FIRST!     │
│  └──────────────────────────────────────┘                   │
│                                                              │
│  MEDIUM Queue (Burst 257-768 - Mixed):                      │
│  ┌──────────────────────────────────────┐                   │
│  │ [SSH(500)]                           │ ← If HIGH empty   │
│  └──────────────────────────────────────┘                   │
│                                                              │
│  LOW Queue (Burst 769-1024 - CPU-bound):                    │
│  ┌──────────────────────────────────────┐                   │
│  │ [Compiler(950)] → [Worker(1000)]     │ ← If others empty │
│  └──────────────────────────────────────┘                   │
│                                                              │
│  All other processes (BlockedOnIpc):                        │
│  ┌──────────────────────────────────────┐                   │
│  │ Timer(BlockedOnIpc), VFS(Blocking)   │ ← Not queued      │
│  └──────────────────────────────────────┘                   │
│                                                              │
└─────────────────────────────────────────────────────────────┘
           ↑ Priority lookup: O(1) amortized
      
Selection: "Pop from HIGH queue"
  → Gets Shell immediately
  → Latency: Constant time!
```

---

## Burst Score Model

### Visual Spectrum

```
Burst Score: 0 ────────────── 512 ────────────── 1024
             │                                      │
        INTERACTIVE                           CPU-BOUND
             │                                      │
             ▼                                      ▼
          
┌──────────────────────────────────────────────────────────┐
│                                                          │
│ 0-25%:     Shell waiting for input                     │
│ └─→ Blocks early                                       │
│ └─→ burst_score decreased (-64)                        │
│ └─→ Moved to HIGH queue                                │
│ └─→ Next input wakes immediately ✓                     │
│                                                          │
│ 25-75%:    SSH connection, mixed I/O & computation      │
│ └─→ Uses some quantum                                  │
│ └─→ burst_score unchanged                              │
│ └─→ Stays in MEDIUM queue                              │
│ └─→ Normal fairness behavior ✓                         │
│                                                          │
│ 75-100%:   Compiler running deep calculations           │
│ └─→ Uses FULL quantum (10 ticks)                       │
│ └─→ burst_score increased (+32)                        │
│ └─→ Moved to LOW queue                                 │
│ └─→ Gets fewer scheduling chances                      │
│ └─→ Background task runs in background ✓               │
│                                                          │
└──────────────────────────────────────────────────────────┘
```

---

## Complete Process Flow - "User Types at Terminal"

### Scenario Setup
```
Time: 0ms
Running: Compiler (doing heavy calculation)
Waiting: Shell (blocked on input, waiting for user keystroke)
Other:   Init, SSH, Timer (all blocked on I/O)
```

### Current Round-Robin (20-30ms Latency) ❌

```
0ms    Compiler: 0-10ms  (runs full quantum)
10ms   Init:    10-20ms  (checking services)
20ms   SSH:     20-25ms  (blocked, skip)
25ms   Shell:   25-30ms  ← USER TYPES KEYSTROKE
       ↑ Gets to run AFTER ~25ms delay
       
Keyboard interrupt arrives at 25.5ms
  └─ Shell already in running state (luck!)
  └─ Still ~5ms latency from keystroke to response
```

### BORE Scheduler (2-5ms Latency) ✅

```
0ms    Compiler (burst_score=950, LOW tier):
       └─ Runs for 10ms
       
5ms    [USER TYPES KEYSTROKE]
       
5.5ms  Keyboard IRQ:
       └─ Shell wakes from BlockedOnIpc
       └─ Was blocked for 5.5ms (early!) → burst_score -64
       └─ Moved to HIGH queue
       └─ At next timer tick, Shell picked immediately
       
10ms   Compiler's quantum expires
       ├─ Moved to LOW queue
       └─ Request reschedule
       
10.1ms Reschedule happens:
       └─ pick_next_bore()
       └─ Check HIGH queue: [Shell]
       └─ Return Shell immediately
       └─ Context switch to Shell
       
10.2ms Shell: RUNNING
       └─ Processes the keystroke
       └─ Total latency: 10.2 - 5.5 = ~4.7ms ✓
```

---

## State Machine & Transitions

### Process States

```
┌─────────────────────────────────────────────────────────────┐
│                  BORE Process State Diagram                  │
└─────────────────────────────────────────────────────────────┘

                     [Process Created]
                            │
                            ▼
                    ┌────────────────┐
                    │    Ready       │ (in one of 3 queues)
                    │ burst_score:256│
                    └────────────────┘
                            │
                   [pick_next_bore() selects]
                            │
                            ▼
                    ┌────────────────┐
                    │   Running      │
                    │ ticks: 0-10    │ ← Timer increments
                    └────────────────┘
                      ↓            ↓
           [Quantum full]    [Blocks on IPC]
                      │            │
         ┌────────────┴────────────┘
         │
         ├─→ update_burst_score()
         │   • Full quantum? +32 (CPU-bound)
         │   • Early block? -64 (interactive)
         │
         ├─→ Re-queue to correct tier
         │   • HIGH (burst 0-256)
         │   • MEDIUM (burst 257-768)
         │   • LOW (burst 769-1024)
         │
         └─→ Return to Ready state
             ↑
          [Loop]
          
       ┌─────────────────────────┐
       │    [Process finished]   │
       │  OR exits via syscall   │
       └─────────────────────────┘
               ↓
         [Not queued]
         [Mark as Finished]
```

### Burst Score Transitions

```
Process starts:
  burst_score = 256 (MEDIUM, slightly interactive bias)

Case 1: Block early (0-3 ticks)
  │
  ├─ update_burst_score(EarlyBlock)
  ├─ burst_score -= 64
  ├─ Result: 192 (still HIGH/MEDIUM, more interactive)
  └─ Re-queue to HIGH tier immediately

Case 2: Use full quantum (10 ticks)
  │
  ├─ update_burst_score(FullQuantum)
  ├─ burst_score += 32
  ├─ Result: 288 (MEDIUM tier)
  └─ Re-queue to MEDIUM tier

Case 3: Repeatedly use full quantum
  │
  ├─ Iteration 1: 256 → 288
  ├─ Iteration 2: 288 → 320
  ├─ Iteration 3: 320 → 352
  ├─ ...
  ├─ After ~24 iterations: 256 → 1024
  └─ Now in LOW tier, gets pushed to background

Case 4: Aged process (hasn't run in 100ms)
  │
  ├─ age_ready_tasks() called
  ├─ burst_score -= 20
  ├─ Result: slower descent toward HIGH tier
  └─ Prevents indefinite starvation
```

---

## Timing Diagrams

### Tick-by-Tick Execution

```
Tick Interval: 1ms (1000 Hz timer)
Quantum: 10 ticks = 10ms per process

Process Timeline:
───────┬──────────┬──────────┬──────────┬──────────┬───────
       │   Shell  │ Compiler │  Timer   │  Shell   │  SSH
      0│  0-5ms   │  5-15ms  │ 15-20ms  │ 20-21ms  │21-31ms
       │          │          │          │ (blocked)│
───────┴──────────┴──────────┴──────────┴──────────┴───────
       0          10         20         30         40

Burst Scores Over Time:
───────────────────────────────────────────────────────────
Shell:     256 → 192 (early block at 5ms)
Compiler:  256 → 288 (full quantum)
Timer:     512 (blocked, unchanged)
SSH:       400 (unchanged)

Queue Assignments:
Before tick 0:
  HIGH:   [Shell(256), Init(256)]
  MEDIUM: [SSH(400)]
  LOW:    [Compiler(256)]

After tick 10:
  HIGH:   [Init(256), Shell(192)]     ← Shell moved here
  MEDIUM: [SSH(400), Compiler(288)]   ← Compiler moved here
  LOW:    []
```

### Context Switch Timeline

```
Timeline: User types at keyboard
──────────────────────────────────────────────────────────────

t=0ms:   Compiler starts (burst_score=950, LOW tier)
         Shell blocked on input (in memory, not queued)

t=5ms:   KEYBOARD INTERRUPT
         └─ Shell receives keystroke
         └─ Shell transitions: BlockedOnIpc → Ready
         └─ Burst score updated: -64 (early block)
         └─ Shell moved to HIGH queue
         
t=10ms:  TIMER INTERRUPT (10 ticks for Compiler)
         ├─ Compiler's quantum expired
         ├─ Compiler's burst_score: +32 (full quantum)
         ├─ Compiler moved to LOW queue
         └─ Set NEEDS_RESCHEDULE flag
         
t=10.1ms: In interrupt epilogue
         ├─ Call pick_next_bore()
         ├─ Check HIGH queue: [Shell] ← FOUND!
         ├─ Pop Shell from HIGH queue
         └─ Prepare context switch
         
t=10.2ms: CONTEXT SWITCH
         ├─ Save Compiler's context
         ├─ Restore Shell's context
         ├─ Activate Shell's address space
         └─ Jump to Shell's code
         
t=10.2ms→: Shell RUNNING
         └─ Processes the keystroke (5-10ms later than arrived)
         └─ User sees response in ~4.7ms from keystroke
         
Latency Breakdown:
  Keyboard arrival → Shell ready: 5ms
  Shell ready → Running: 5ms (waits for Compiler's quantum)
  TOTAL: ~10ms (was 25-30ms with round-robin) ✓
```

---

## Memory Overhead Analysis

### Current Process Size

```
pub struct Process {
    // Identity (40 bytes)
    pub pid: usize,                      // 8
    pub ppid: usize,                     // 8
    pub name: &'static str,              // 8
    pub uid: u32,                        // 4
    pub gid: u32,                        // 4
    pub cpu_mask: u64,                   // 8
    
    // State (88 bytes)
    pub state: ProcessState,             // 4 (enum)
    pub sched_type: u8,                  // 1
    pub weight: u32,                     // 4
    pub capability_mode: bool,           // 1
    pub is_linux_compat: bool,           // 1
    pub ipc_endpoint: Option<u32>,       // 8
    pub address_space: AddressSpace,     // ~60 bytes
    
    // Memory (~700 bytes)
    pub kernel_stack: Box<[u8; 32KB]>,  // Heap allocation
    pub user_stack_top: u64,             // 8
    pub kernel_stack_top: u64,           // 8
    pub entry_point: u64,                // 8
    pub context_rsp: u64,                // 8
    
    // IPC (80+ bytes)
    pub ipc_queue: VecDeque<IpcMsg>,
    pub ipc_reply: Option<IpcMsg>,
    pub pending_call: Option<(u64, IpcMsg)>,
    pub pending_reply_wait: Option<(u32, IpcMsg)>,
    
    // File descriptors (~100+ bytes)
    pub fd_table: FdTable,
    
    // Signals (~20 bytes)
    pub signal_state: SignalState,
    
    // Capabilities (variable)
    pub capabilities: Vec<Capability>,
    
    // NEW FOR BORE: (28 bytes)
    pub burst_score: u32,                // 4
    pub timeslice_used: u32,             // 4
    pub last_run_tick: u64,              // 8
    pub io_wait_time: u32,               // 4
    pub interactive_bonus: i32,          // 4
    pub block_start_tick: u64,           // 8
    pub aging_counter: u32,              // 4
}

Total: ~1100-1200 bytes per process (32KB stack already allocated)
BORE overhead: +28 bytes = +2.3% ✓ (negligible)
```

### Scheduler Size

```
pub struct Scheduler {
    pub processes: Vec<Process>,         // Existing: ~10-50KB for 10-50 procs
    
    // NEW FOR BORE: Ready queues
    pub ready_queue_high: VecDeque<usize>,    // ~1-2KB
    pub ready_queue_medium: VecDeque<usize>,  // ~1-2KB
    pub ready_queue_low: VecDeque<usize>,     // ~1-2KB
    
    pub current: usize,                  // 8 bytes
    pub current_ticks: u64,              // 8 bytes
    pub global_tick: u64,                // 8 bytes (NEW)
    pub idle_context_rsp: u64,           // 8 bytes
}

Total BORE overhead: ~3-6KB for queues (negligible for kernel)
```

---

## Worst Case Analysis

### What Could Go Wrong?

```
Scenario 1: Many tasks in HIGH queue
────────────────────────────────────────
Problem: HIGH queue has 50 interactive tasks
Issue: All get picked in order (FIFO within tier)
Solution: This is CORRECT behavior - fairness within tier

Scenario 2: Starvation of LOW queue
────────────────────────────────────
Problem: Compiler never gets to run if Shell always blocked early
Issue: But aging_counter prevents this!
  • After 100ms of waiting, LOW task aging kicks in
  • burst_score decreases by 20 (toward MEDIUM tier)
  • Eventually reaches threshold and gets scheduled
Solution: Aging mechanism ✓

Scenario 3: Interactive task becomes CPU-bound
──────────────────────────────────────────────
Transition: Shell (burst=100) runs for 10 full ticks
  • Becomes (burst=132) - slowly increases
  • Eventually reaches (burst=768) after many iterations
  • Moved to LOW queue - correct behavior ✓

Scenario 4: Rapid oscillation between queues
────────────────────────────────────────────
Task: Blocks for 2 ticks, then runs 10 ticks, repeat
Pattern: IN HIGH → OUT to MEDIUM → IN HIGH → ...
Issue: Overhead of queue operations
Solution: O(1) push/pop, negligible cost ✓
```

---

## Testing Strategy Summary

### Unit Tests
- Burst score calculation (early block, full quantum, aging)
- Queue assignment logic (which tier for each score)
- Aging mechanism (prevents starvation)

### Integration Tests
- Whole scheduler with real processes
- Interactive task (Shell) gets low latency
- CPU-bound task (Compiler) doesn't starve
- Mixed workload fairness

### Performance Tests
- Shell input latency (goal: <5ms)
- Context switch overhead (goal: negligible)
- Scheduler overhead (goal: O(1) amortized)

---

## Conclusion

BORE scheduler provides:

✅ **Performance:** 6-12x latency improvement for interactive tasks  
✅ **Fairness:** Aging prevents indefinite starvation  
✅ **Simplicity:** Only ~400 new lines of code  
✅ **Safety:** No floating-point, O(1) scheduling, deterministic  
✅ **Compatibility:** SCHED_FIFO unaffected, works with existing processes  

**Ready for implementation starting Phase 3.0!** 🚀
