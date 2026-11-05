# Launch Latency Tracing for SunlightOS

## Overview

This module provides lightweight instrumentation to measure app launch latency from syscall entry through scheduler enqueue. It's designed for debugging inconsistent launch delays without changing scheduling policy or adding queue priority tiers.

## Design Principles

1. **Measurement only** — no scheduling changes, no VIP queues, no priority adjustments
2. **Minimal overhead** — single atomic counter, TSC timestamps, compile-time disableable
3. **Progressive disclosure** — records what's observable at kernel layer; userspace/display stages remain "unknown"
4. **Zero cost when disabled** — all functions inline to no-ops via `#[cfg(not(feature = "launch_trace"))]`

## Trace Points

Captured timestamps (nanoseconds since boot, from `interrupts::now_ns()`):

| # | Point | Description |
|---|-------|-------------|
| 1 | `request_received_ns` | Syscall entry, path read from userspace |
| 2 | `resolve_started_ns` | Before embedded/VFS binary lookup |
| 3 | `resolve_finished_ns` | Binary bytes ready |
| 4 | `spawn_started_ns` | About to call `exec_into_process` |
| 5 | `spawn_returned_ns` | `exec_into_process` returned Ok |
| 6 | `child_created_ns` | PID assigned, struct prepared |
| 7 | `enqueue_finished_ns` | `sched.enqueue_process()` returned |

Not yet observable (always reported as "unknown"):
- `display_connection_started`
- `window_registration_started`
- `first_window_or_first_paint`

## Usage

### Enabling Tracing

Build the kernel with the `launch_trace` feature:

```bash
cargo build --package sunlight-kernel --features sunlight-kernel/launch_trace
```

Or add to `tools/build.sh` line 95:

```bash
cargo build --package sunlight-kernel --features sunlight-kernel/launch_trace
```

### Output Format

```
[LAUNCH-TRACE] app=calculator launch_id=42 path=/bin/calculator resolve_ms=1 spawn_ms=7 queue_or_wait_ms=unknown display_ms=unknown total_ms=8 result=ok pid=17
```

Fields:
- `app`: basename from path (e.g., "calculator" from "/bin/calculator")
- `launch_id`: unique monotonic ID per launch
- `path`: full path requested
- `resolve_ms`: time from resolve_started to resolve_finished (binary lookup)
- `spawn_ms`: time from spawn_started to enqueue_finished (ELF load + page tables + enqueue)
- `total_ms`: end-to-end kernel-side latency
- `result`: "ok" or "failed:<stage>" (e.g., "failed:not_found", "failed:elf_load")
- `pid`: child process PID, or "none" on failure

### Test Procedure

1. **Build with tracing enabled** (see above)
2. **Boot into SunlightOS** via `./tools/build.sh` or `./tools/test.sh`
3. **Open shell** (sshl) in the QEMU environment
4. **Run launches:**

   ```bash
   # Test 1: idle system
   for i in {1..20}; do calculator & sleep 1; done
   
   # Test 2: CPU-busy system
   yes > /dev/null &
   for i in {1..20}; do calculator & sleep 1; done
   
   # Test 3: terminal launches
   for i in {1..20}; do sshl & sleep 1; done
   ```

5. **Collect serial output** from QEMU and filter:

   ```bash
   grep "LAUNCH-TRACE" serial.log > traces.txt
   ```

6. **Analyze p50/p95** for `resolve_ms`, `spawn_ms`, `total_ms`

### Example Analysis

```bash
# Extract total_ms from traces
grep "LAUNCH-TRACE" serial.log | \
  sed -E 's/.*total_ms=([0-9]+).*/\1/' | \
  sort -n | \
  awk '{a[NR]=$1} END {print "p50:", a[int(NR*0.5)], "p95:", a[int(NR*0.95)]}'
```

## Interpretation

**High `resolve_ms`:**
- Embedded image lookup is instant (hash table)
- VFS lookup iterates FAT directory entries
- **Next step:** Add VFS-layer timing breakdowns or cache directory entries

**High `spawn_ms`:**
- Dominated by `exec_into_process`: ELF parsing + page table setup
- ELF load: linear scan of program headers, memcpy of segments
- Page table: per-page PMM allocation + map_page calls
- **Next step:** Profile ELF loader, consider lazy page allocation, or pre-warm page allocator

**High variability in `total_ms`:**
- Scheduler queue depth or IPC contention
- **Next step:** Add queue depth snapshot at enqueue time

**Stable kernel-side, slow perceived launch:**
- Bottleneck is after enqueue: scheduler dispatch, userspace init, display registration
- **Next step:** Instrument display server window creation

## Known Limitations

1. **No userspace/display timing** — points 8-10 require display-server instrumentation
2. **No queue-wait time** — time from enqueue to first CPU schedule not yet captured
3. **No first-paint time** — compositor repaint latency not measured
4. **No cross-launch correlation** — cannot yet correlate parent spawn() with child's first_paint

## Future Work

- [ ] Add `sched::switch_to()` hook to capture queue-wait (enqueue → first CPU slice)
- [ ] IPC trace correlation: link spawn syscall → display server window_create IPC
- [ ] Display server timestamps: window_registered, first_paint, first_input
- [ ] Aggregation: per-app histogram, outlier detection, automatic p50/p95 reporting

## Overhead

When **disabled** (default):
- Zero runtime cost: all functions inline to no-ops
- Zero binary size: dead-code elimination removes module

When **enabled**:
- Per-launch: 1 atomic increment + 7 TSC reads + 1 serial print (~1 µs total)
- No heap allocation, no locks held across timestamps

## Code Structure

- `kernel/src/launch_trace.rs` — trace module (this doc's sibling)
- `kernel/src/arch/x86_64/syscall.rs` — instrumentation points in `sys_spawn`
- `kernel/Cargo.toml` — feature flag `launch_trace = []`

## References

- TSC timing: `kernel/src/arch/x86_64/interrupts.rs::now_ns()`
- Spawn path: `kernel/src/arch/x86_64/syscall.rs::sys_spawn()`
- ELF loader: `kernel/src/process/spawn.rs::exec_into_process()`
- Scheduler enqueue: `kernel/src/sched/mod.rs::enqueue_process()`
