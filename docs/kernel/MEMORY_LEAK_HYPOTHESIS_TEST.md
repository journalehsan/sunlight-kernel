# Memory Leak Hypothesis Test - Terminal & Time Server

## Hypothesis
The hard freeze after 4-6 commands is caused by a memory leak in the recently-added terminal or time server code, NOT the fundamental process reaping issue (which we already identified and documented).

## Test Plan

### Test 1: Increase Memory to 1GB
**Changed:** `tools/run.sh` - Default memory from 256MB → 1024MB (4x increase)

**Expected Result if Memory Leak:**
- With 256MB: Freeze after ~4-6 commands
- With 1024MB: Freeze after ~16-24 commands (scales proportionally)

**Expected Result if Process Reaping Issue:**
- With 1024MB: Freeze after same ~4-6 commands (memory amount doesn't matter much)

### Test 2: Monitor Resource Usage
Use diagnostic traces to watch:
- `[PMM-DIAG]` - free frames count
- `[SCHED-DIAG]` - process accumulation

**Memory Leak Signature:**
```
[PMM-DIAG] free=800000 allocated=224000 ...   # Plenty free at start
[PMM-DIAG] free=400000 allocated=624000 ...   # Dropping as commands run
[PMM-DIAG] free=50000 allocated=974000 ...    # Critical, freeze imminent
[System freezes - no more output]
```

**Process Reaping Signature:**
```
[SCHED-DIAG] created=4 finished=3 alive=4 delta_created-finished=1
[SCHED-DIAG] created=7 finished=6 alive=7 delta_created-finished=1  
[SCHED-DIAG] created=10 finished=9 alive=10 delta_created-finished=1
[Consistent 1 unreaped process = just the current running process]
```

## How to Run the Tests

### Quick Test (1 command)
```bash
./tools/run.sh --no-display
# Type: whoami
# Then: id
# Then: whoami
# Watch when it freezes
```

### Stress Test (automated)
```bash
chmod +x test_memory_stress.sh
./test_memory_stress.sh
# Runs 25+ commands automatically, captures output
```

## What to Look For

### Terminal/TTY Server Leak Indicators
Look in `services/tty_server/src/main.rs`:
- Buffers allocated per command but not freed?
- Grid rendering allocating temporary memory?
- Ring buffer growing instead of recycling?

### Timer Server Leak Indicators
Look in `services/timer_server/src/main.rs`:
- Timer tick queue accumulating?
- Allocating new timers without cleanup?

### Common Leak Patterns
1. **Vec/String not recycled:** Growing collections never cleared
2. **Allocator fragmentation:** Small allocations exhausting heap
3. **IPC message queues:** Messages not being dequeued
4. **Framebuffer updates:** Temporary allocations per frame

## Debugging Strategy

If memory leak is confirmed (freeze delayed with 1GB):

1. **Instrument the allocators:**
   ```rust
   // In leak_allocator or similar
   serial_println!("[ALLOC] {} bytes, total={}", size, TOTAL_ALLOCATED.fetch_add(size, ...));
   ```

2. **Add memory limits to services:**
   ```rust
   // In tty_server
   if HEAP_USED > 10_000_000 {  // 10MB limit
       serial_println!("[TTY] ALERT: Heap usage {} > limit", HEAP_USED);
   }
   ```

3. **Profile specific operations:**
   ```rust
   // Before command execution
   let heap_before = HEAP_USED;
   // ... run command ...
   let heap_after = HEAP_USED;
   serial_println!("[CMD] Heap delta: {} bytes", heap_after - heap_before);
   ```

## Key Files to Examine

### Terminal Code (Most Likely Culprit)
- `services/tty_server/src/main.rs` - Main TTY server loop
- `sunlight-tui/src/lib.rs` - Terminal UI rendering
- Any grid/framebuffer management code

### Timer Code
- `services/timer_server/src/main.rs` - Timer event handling
- Check for accumulating tick events

### IPC/Scheduler
- Process scheduling and IPC message handling
- Could be message queues growing

## Preliminary Analysis

Based on the diagnostic output we collected:

```
Boot shows:
✓ 4 system services created (init, vfs_server, timer_server, tty_server)
✓ 1000+ timer ticks processed
✓ IPC 1000 round-trip calls OK
✓ All phase tests passed

This suggests:
- Core scheduling works fine with services at boot
- Timer server handles many ticks without issue at boot
- Problem appears ONLY when running shell commands after boot
```

**Implication:** The leak is in the shell command execution path, likely:
1. TTY server processing each command (rendering, buffering)
2. Process lifecycle (spawn → exit cycle)
3. IPC communication with TTY during command execution

## Success Criteria

### If Memory Leak Theory is Correct
```
Test with 1GB:
- Run 20-30 commands before freeze (vs 4-6 with 256MB)
- Diagnostics show free frames dropping gradually
- Leak is in new terminal/timer code, not fundamental
```

### If Process Reaping Theory is Correct
```
Test with 1GB:
- Still freeze after 4-6 commands
- Diagnostics show process accumulation
- Must implement process reaper regardless
```

## Next Steps

1. **Run stress test:** `./test_memory_stress.sh`
2. **Count successful commands:** How many before freeze?
3. **Review diagnostics:** Which pattern matches?
4. **If memory leak:** Instrument terminal/timer code
5. **If process reaping:** Implement the reaper we designed

---

## Test Results Template

Fill this in after running the test:

```
Test Date: 
Memory Used: 1024 MB
Commands Completed: ___ / 25
Freeze Point: After command ___ 

[PMM-DIAG] at freeze:
- Total frames: 
- Free frames:
- Allocated:
- Delta:

[SCHED-DIAG] at freeze:
- Created:
- Finished:
- Alive:
- Unreaped:

Conclusion: 
[ ] Memory leak - freeze delayed with more RAM
[ ] Process reaping - freeze still after few commands
[ ] Unknown - need more investigation

Next Action:
```

---

## Important Note

The diagnostic traces we added in the previous session are **already compiled into the ISO**. They will help us distinguish between:
1. Memory exhaustion (PMM diagnostics show free→0)
2. Process accumulation (SCHED diagnostics show unreaped growing)
3. Both issues simultaneously

Run the test and we'll know exactly which path to take for the fix.
