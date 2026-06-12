# SunlightOS Process Leak Diagnostic Tracing Report

## Issue Identified

**Critical Bug Found:** Processes are never reaped after exiting.

When a process calls `exit()` or is terminated by a signal:
1. State is set to `ProcessState::Finished`
2. BUT the process object remains in `scheduler.processes` vector
3. The 32KB kernel stack is never freed
4. All allocated physical frames are never freed

After 4-6 commands, all physical memory is exhausted → hard freeze.

## Diagnostic Traces Added

### 1. PMM Frame Tracking (`kernel/src/memory/pmm.rs`)

Added atomic counters to track all frame allocations and deallocations:

```rust
static ALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);
static FREE_COUNT: AtomicUsize = AtomicUsize::new(0);
```

**What to watch for:**
- `[PMM] ALLOC #N addr=...` - Frame allocation (logged every 100 ops or first 10)
- `[PMM] FREE #N addr=...` - Frame deallocation
- If ALLOC count strictly increases and FREE count stays near 0, we have confirmed the frame leak

**Diagnostic function added:**
- `PMM::diagnostic_report()` - Shows total_frames, free_frames, allocated_frames, alloc_ops, free_ops, and delta

### 2. Interrupt Handler EOI Logging (`kernel/src/arch/x86_64/interrupts.rs`)

Added logging to verify EOI signals are being sent:

**Timer Interrupt (IRQ0):**
```rust
[IRQ0] Timer interrupt - EOI sent to PIC
```

**Keyboard Interrupt (IRQ1):**
```rust
[IRQ1] Keyboard interrupt - EOI sent to PIC
```

These logs verify that:
- Interrupts are being serviced
- EOI is being sent to both PIC controllers
- No branch is skipping the EOI (would cause interrupt masking)

### 3. Process Lifecycle Tracking (`kernel/src/sched/mod.rs`)

Added atomic counters for process lifecycle:

```rust
static PROCESS_CREATED: AtomicUsize = AtomicUsize::new(0);
static PROCESS_FINISHED: AtomicUsize = AtomicUsize::new(0);
```

**Process creation logging:**
```
[SCHED] CREATED process #1 'sshl' idx=0 burst_score=256 tier=Medium
[SCHED] CREATED process #2 'sshl' idx=1 burst_score=256 tier=Medium
```

**Process completion detection:**
```
[SCHED] FINISHED process pid=2 name='sshl' still in vector (LEAK!)
```

This will show whenever a Finished process is detected but remains in the scheduler.

**Diagnostic reporting (every 1000 ticks):**
```
[SCHED-DIAG] created=5 finished=3 alive=5 ready_queues=(1,0,0) delta_created-finished=2
```

This shows:
- `created`: Total processes spawned
- `finished`: Processes marked Finished (still in vector = LEAK)
- `alive`: Current size of process vector (should match "finished")
- `ready_queues`: How many processes in each priority tier
- `delta_created-finished`: Should be close to "alive" if no leak exists

## Expected Output Pattern During Freeze

When you run 4-6 commands and see a freeze, the serial console should show:

```
[SCHED] CREATED process #1 'sshl' ...
[SCHED] CREATED process #2 'sshl' ...
[SCHED] FINISHED process pid=1 name='sshl' still in vector (LEAK!)

[SCHED] CREATED process #3 'sshl' ...
[SCHED] FINISHED process pid=2 name='sshl' still in vector (LEAK!)

... (more processes)

[SCHED-DIAG] created=7 finished=6 alive=7 ready_queues=(1,0,0) delta_created-finished=1
[PMM-DIAG] total=262143 free=100 allocated=262043 alloc_ops=7500 free_ops=50 delta=7450
```

Notice:
- `alive=7` processes in vector but only 1 is Running
- `delta=7450` in PMM means 7450 frames allocated but not freed
- Free frames down to 100 (from initial ~260000)

## Testing Instructions

1. Build the system:
   ```bash
   ./tools/run.sh --build --no-display
   ```

2. In the QEMU serial console, run a few commands:
   ```
   whoami
   id
   whoami
   id
   whoami
   ```

3. Watch the serial output for:
   - `[SCHED] FINISHED` messages without any `FREE` operations
   - `[PMM-DIAG]` showing free frames dropping
   - Eventually the system freezes

4. Collect the diagnostic output and we'll identify exactly which resources are leaking

## Root Cause

The leak is in `kernel/src/arch/x86_64/syscall.rs` lines 513-521:

```rust
fn process_exit(_code: i32) -> ! {
    sched::with_scheduler(|s| {
        s.current_process_mut().state = ProcessState::Finished;
    });
    sched::request_reschedule();
    loop {
        core::arch::x86_64::_mm_pause()
    }
}
```

And signal handler at line 175:
```rust
process.state = crate::process::ProcessState::Finished;
```

**The fix needed:**
Implement a process reaper function in the scheduler that:
1. Removes `Finished` processes from the vector
2. Frees their kernel stacks
3. Frees their address space (all frames)
4. Calls this periodically from `scheduler.tick()`

Without this, processes accumulate forever.

## Next Steps

1. Run the diagnostics to confirm the leak
2. Implement process reaping with address space cleanup
3. Track freed frames to verify the fix works
4. Confirm the system no longer freezes after many commands
