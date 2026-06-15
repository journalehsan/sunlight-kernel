# CRITICAL: Kernel Memory Allocation Cap

## Issue
The kernel has a hard memory cap that causes allocation failures and kernel panics when exceeded.

**Current Status:** Panic triggered with 382464 bytes allocation failure

```
[KERNEL PANIC] panicked at /rustc/8954863c81df429ebf96ea38a16c76f209995833/library/alloc/src/alloc.rs:573:9:
memory allocation of 382464 bytes failed
```

## Symptoms
- Processes fail to spawn when memory pressure is high
- Random allocation failures during normal operation
- Kernel panic instead of graceful out-of-memory handling
- Shell/sshl process creation fails after system reaches a certain memory state

## Root Cause Analysis
The kernel's memory allocator lacks proper:
1. Memory limit configuration
2. Out-of-memory (OOM) error handling
3. Memory pressure detection and early intervention
4. Graceful degradation or process termination

## Fix Priority
**HIGH** - This blocks reliable operation and process spawning

## FIXES APPLIED (2026-06-15)

### 1. Increased Kernel Heap Size ✓
- **Before:** 1 MiB (1024 * 1024 bytes)
- **After:** 16 MiB (16 * 1024 * 1024 bytes)
- **File:** `kernel/src/memory/heap.rs` lines 8-9
- **Rationale:** The original heap was too small for fragmentation-friendly allocation. 16 MiB provides breathing room for process creation and OS structures.

### 2. Added OOM Error Handler ✓
- **File:** `kernel/src/panic.rs`
- **Change:** Added `#[alloc_error_handler]` to catch allocation failures and log them gracefully instead of panicking with a cryptic message
- **Behavior:** When allocation fails, kernel now logs `[OOM] Allocation of X bytes (align=Y) failed` instead of accessing invalid memory

## Related Metrics (from last panic)
- PMM allocation #6601 showed free_now=510012 bytes (~498 KB)
- Attempted allocation: 382464 bytes (~374 KB)
- With 16 MiB heap, similar allocations should succeed

## Testing
- Build and boot kernel to verify no regressions
- Spawn multiple shells to stress test memory allocation
- Monitor heap usage via serial output

---
**Date Reported:** 2026-06-15  
**Date Fixed:** 2026-06-15  
**Priority:** CRITICAL  
**Status:** FIXED - NEEDS TESTING
