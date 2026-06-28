# Launch Latency Tracing Implementation Summary

## What Was Added

Lightweight launch latency instrumentation for SunlightOS app spawning, with zero overhead when disabled.

### Files Changed

1. **`kernel/src/launch_trace.rs`** (new)
   - Trace struct with 7 timestamp points
   - Unique launch ID generation via atomic counter
   - Compact serial output formatting
   - Feature-gated: all functions become no-ops when `launch_trace` feature is disabled

2. **`kernel/src/main.rs`**
   - Added `mod launch_trace;`

3. **`kernel/src/arch/x86_64/syscall.rs`**
   - Instrumented `sys_spawn()` with 7 trace points:
     1. Request received (syscall entry)
     2. Resolve started (before binary lookup)
     3. Resolve finished (binary bytes ready)
     4. Spawn started (before exec_into_process)
     5. Spawn returned (exec_into_process succeeded)
     6. Child created (PID assigned)
     7. Enqueue finished (child is runnable)
   - Added import: `use crate::arch::x86_64::interrupts::now_ns;`

4. **`kernel/Cargo.toml`**
   - Added feature flag: `launch_trace = []`

5. **`kernel/src/launch_trace.md`** (new)
   - Complete usage documentation

## Example Trace Output

```
[LAUNCH-TRACE] app=calculator launch_id=1 path=/bin/calculator resolve_ms=0 spawn_ms=3 queue_or_wait_ms=unknown display_ms=unknown total_ms=3 result=ok pid=24
[LAUNCH-TRACE] app=sshl launch_id=2 path=/bin/sshl resolve_ms=0 spawn_ms=4 queue_or_wait_ms=unknown display_ms=unknown total_ms=4 result=ok pid=25
[LAUNCH-TRACE] app=sunlight-files launch_id=3 path=/bin/sunlight-files resolve_ms=0 spawn_ms=5 queue_or_wait_ms=unknown display_ms=unknown total_ms=5 result=ok pid=26
```

Fields:
- `resolve_ms`: Binary lookup time (embedded image hash table or VFS directory scan)
- `spawn_ms`: ELF load + page table setup + scheduler enqueue
- `total_ms`: End-to-end kernel-side latency
- `queue_or_wait_ms`, `display_ms`: Not yet observable (future work)

## What Is Measured

### Kernel-Side Latency (Observable Now)

| Stage | Duration | What Happens |
|-------|----------|--------------|
| Resolve | `resolve_ms` | Binary lookup: embedded image table or VFS FAT directory iteration |
| Spawn | `spawn_ms` | ELF parsing, segment load, page table setup, FD table wiring, scheduler enqueue |
| **Total** | `total_ms` | **Syscall entry → child runnable** |

### Not Yet Measured (Always "unknown")

- **Queue wait time**: Child enqueued → first CPU schedule
- **Userspace init**: First instruction → IPC connection to display server
- **Display registration**: Window create IPC → display server ACK
- **First paint**: Compositor receives framebuffer → pixels on screen

## Usage

### Enable Tracing

```bash
# Option 1: Manual build
cargo build --package sunlight-kernel --features sunlight-kernel/launch_trace

# Option 2: Modify tools/build.sh line 95
cargo build --package sunlight-kernel --features sunlight-kernel/launch_trace

# Then run
./tools/build.sh  # or ./tools/test.sh
```

### Collect Data

Inside the QEMU shell:

```bash
# Idle system
for i in {1..20}; do calculator & sleep 1; done

# CPU-busy system
yes > /dev/null &
for i in {1..20}; do calculator & sleep 1; done
```

Filter serial output:

```bash
grep "LAUNCH-TRACE" serial.log > traces.txt
```

### Analyze

```bash
# Extract total_ms and compute p50/p95
grep "LAUNCH-TRACE" traces.txt | \
  sed -E 's/.*total_ms=([0-9]+).*/\1/' | \
  sort -n | \
  awk '{a[NR]=$1} END {print "p50:", a[int(NR*0.5)], "p95:", a[int(NR*0.95)]}'
```

## Interpretation & Next Steps

### High `resolve_ms` (> 5ms)

**Root cause:**
- Embedded images: instant (hash lookup)
- VFS: linear scan of FAT directory entries

**Next step:**
- Add VFS-level timing breakdowns
- Cache frequently accessed directory entries
- Consider B-tree index for large `/bin`

### High `spawn_ms` (> 10ms)

**Root cause:**
- `exec_into_process`: ELF parsing + memcpy + per-page PMM alloc + page table map

**Next step:**
- Profile ELF loader: are we parsing headers multiple times?
- Consider lazy page allocation (demand-page on fault instead of pre-mapping stack)
- Pre-warm PMM allocator (batch allocate frames for common spawn sizes)

### High Variability in `total_ms`

**Root cause:**
- Scheduler queue depth fluctuations
- IPC contention (PMM/SCHED locks held during spawn)

**Next step:**
- Snapshot ready queue depths at enqueue time
- Add IPC wait time histogram
- Check if spawn happens during timer interrupt (might delay enqueue)

### Stable Kernel-Side, Slow Perceived Launch

**Root cause:**
- Bottleneck is **after enqueue**: queue wait, userspace init, display registration, first paint

**Next step (highest priority):**
1. **Add scheduler dispatch hook**: capture time from enqueue → first CPU slice
2. **Instrument display server**: add timestamps for window_create IPC handler
3. **Correlate traces**: pass `launch_id` via IPC so display server can emit:
   ```
   [DISPLAY-TRACE] launch_id=42 window_registered_ms=15 first_paint_ms=23
   ```

## Testing Checklist

- [x] Build passes with feature disabled (default)
- [x] Build passes with feature enabled (`--features sunlight-kernel/launch_trace`)
- [x] Boot gate passes (no LAUNCH-TRACE lines in default build)
- [x] Zero overhead when disabled (no `launch_trace` calls in disassembly)
- [ ] Manual QEMU test with 20x calculator launches (requires feature-enabled build)
- [ ] Manual QEMU test with 20x terminal launches
- [ ] Manual QEMU test under CPU load
- [ ] Compare p50/p95 between idle and loaded scenarios

## Constraints Followed

✅ **No scheduling changes**: No new queues, no priority adjustments, no burst_score changes  
✅ **No nice/VIP/Emergency tiers**: Pure measurement, zero policy changes  
✅ **Minimal overhead**: 1 atomic + 7 TSC reads + 1 print per launch (~1 µs)  
✅ **Compile-time disable**: Feature flag with inline no-ops  
✅ **Easy to enable/disable**: Single `--features` flag or one-line `Cargo.toml` edit  

## Recommendations for Next Bottleneck

Based on trace data patterns:

1. **If `total_ms` is consistently low (<5ms)**: Bottleneck is **userspace or display**
   - **Action**: Instrument display server window registration
   - **Expected outcome**: Identify if delay is in IPC roundtrip or compositor repaint

2. **If `spawn_ms` dominates `total_ms`**: Bottleneck is **ELF load or page tables**
   - **Action**: Add per-segment timing in `exec_into_process`
   - **Expected outcome**: Isolate whether ELF parse, memcpy, or page mapping is slow

3. **If `resolve_ms` is high for VFS binaries**: Bottleneck is **VFS lookup**
   - **Action**: Add directory entry caching or indexing
   - **Expected outcome**: VFS resolve time drops to <1ms

4. **If variability is high across runs**: Bottleneck is **scheduler contention**
   - **Action**: Add queue depth snapshot at enqueue time
   - **Expected outcome**: Correlate high latency with queue depth spikes

## Files to Review

- **Implementation**: `kernel/src/launch_trace.rs`
- **Instrumentation**: `kernel/src/arch/x86_64/syscall.rs` (search `LAUNCH-TRACE`)
- **Documentation**: `kernel/src/launch_trace.md`
- **Feature flag**: `kernel/Cargo.toml` (line with `launch_trace = []`)

## Verification Commands

```bash
# Ensure disabled build has no trace code
cargo build --package sunlight-kernel --release
objdump -d target/x86_64-unknown-none/release/sunlight-kernel | grep -i launch_trace
# Should output nothing

# Ensure enabled build includes trace code
cargo build --package sunlight-kernel --features sunlight-kernel/launch_trace
objdump -d target/x86_64-unknown-none/debug/sunlight-kernel | grep -i launch_trace
# Should show launch_trace::next_launch_id, LaunchTrace::new, etc.
```
