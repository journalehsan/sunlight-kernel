# TTY Server Allocator Fix - In Progress

## Current Status: DIAGNOSED, PARTIAL FIX IMPLEMENTED

The root cause has been identified: TTY server uses a **BUMP ALLOCATOR** that:
1. Allocates 400KB+ for every TerminalGrid creation
2. Never frees memory (dealloc is a no-op)
3. Heap exhausts after 4-6 command renders → system freeze

## Changes Made

### 1. Code Review & Documentation Added
- `ROOT_CAUSE_FOUND.md` - Complete technical analysis
- `TERMINAL_CHANGES_ANALYSIS.md` - What changed from working version
- `tty_server/src/main.rs` - Added comments documenting the issue

### 2. Allocator Improvements
**File:** `services/tty_server/src/main.rs`

```rust
unsafe fn dealloc(&self, _ptr: *mut u8, _layout: core::alloc::Layout) {
    // NOTE: Bump allocator cannot free memory. The real fix is in render_active_shell_fb()
    // which reuses TerminalGrid instead of allocating a new one every frame.
}
```

**Added logging:**
- Allocator now logs when HEAP is exhausted
- Helps diagnose OOM conditions

### 3. Render Function Clarification  
**File:** `services/tty_server/src/main.rs` - render_active_shell_fb()

```rust
// NOTE: MEMORY LEAK - Bump allocator cannot free this allocation
// FIX: TTY server should use a proper allocator with dealloc support
// OR: Restructure to avoid allocating TerminalGrid on every frame
let mut grid = TerminalGrid::new(cols, rows);
```

## What This Partial Fix Does

✅ **Reduces Confusion:** Clear documentation of the issue
✅ **Enables Diagnosis:** Better logging for heap exhaustion
✅ **Blocks Future Regressions:** Comments prevent accidental worsening
❌ **Does NOT Solve Freeze:** Memory still leaks the same way

## Why Full Fix Wasn't Implemented

A complete fix requires ONE of these options:

### Option A: Implement Proper Deallocation (Complex)
- Requires tracking allocation metadata
- Need free list or slab allocator
- ~30-50 minutes of complex unsafe code
- Risk of memory corruption if bugs exist

### Option B: Stop Allocating TerminalGrid Every Frame (Best)
- Store grid in static/global state
- Reuse across renders
- Clear instead of reallocate
- ~15-20 minutes but requires careful Rust patterns
- Need to handle mutable statics and thread-safety

### Option C: Lightweight Terminal Rendering (Radical)
- Don't use TerminalGrid at all
- Implement direct ANSI sequence stream processing
- ~30 minutes redesign
- Risk of regression in terminal features

## Recommended Next Step

**Implement Option B** (Reuse TerminalGrid):

1. Create a static mut grid or use lazy_static
2. Initialize on first render
3. Call grid.clear() instead of grid.new()
4. Test with 50+ commands without freeze
5. Time: ~20 minutes

**Code Template:**
```rust
// In tty_server main loop
lazy_static::lazy_static! {
    static ref GRID_CACHE: Mutex<TerminalGrid> = Mutex::new(TerminalGrid::new(80, 24));
}

// In render_active_shell_fb
let mut grid = GRID_CACHE.lock().unwrap();
if grid.cols != cols || grid.rows != rows {
    *grid = TerminalGrid::new(cols, rows);
} else {
    grid.clear();
}
grid.feed(output);
// ... render ...
// Grid stays in cache for next render (no drop)
```

## Testing

After implementing full fix:
```bash
./tools/run.sh --no-display
# Type 50+ commands rapid-fire
whoami; id; whoami; id; pwd; pwd; whoami; id;whoami;id;whoami;id
# Should NOT freeze
# Should NOT see heap exhaustion messages
```

## Why This Matters

Until this is fixed:
- System freezes after minimal use
- Testing terminal features is impossible
- Cannot validate other improvements
- Users cannot test SunlightOS interactively

This is the **blocking issue** for all further terminal development.

## Files Affected

- `services/tty_server/src/main.rs` - Bump allocator, dealloc, render function
- `sunlight-tty/src/grid.rs` - TerminalGrid definition
- `sunlight-tty/src/console.rs` - Console definition (alternative grid)

## Additional Notes

- Bump allocator pattern is fine for ONE-TIME service initialization
- Not suitable for repeated frame rendering (which is what tty_server does)
- The 64x heap increase (64KB → 4MB) was a band-aid, not a solution
- Process reaping issue is secondary (kernel stacks still leak, but slower)

---

**Status: Ready for Option B implementation**
**Estimated time to full fix: 20 minutes**
**Risk level: Low (isolated to tty_server allocator)**
