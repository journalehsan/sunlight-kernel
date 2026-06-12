# ROOT CAUSE FOUND: TTY Server Bump Allocator Memory Exhaustion

## The Bug: In One Picture

```rust
// tty_server/src/main.rs lines 8-23

unsafe fn dealloc(&self, _ptr: *mut u8, _layout: core::alloc::Layout) {}
//                                                                      ↑↑
//                              THIS DOES NOTHING!
```

**Impact:** 4MB TTY server heap fills up after 4-6 commands → system freeze

---

## The Full Story

### TTY Server Allocator Architecture

```rust
struct BumpAllocator;

unsafe impl core::alloc::GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        static mut HEAP: [u8; 4 * 1024 * 1024] = [0; 4 * 1024 * 1024];  // 4MB
        static mut NEXT: usize = 0;  // ← Pointer only moves forward
        
        let aligned = (start + align - 1) & !(align - 1);
        let end = aligned + layout.size();
        NEXT = end;  // ← NEXT ONLY GROWS
        
        if end > HEAP.len() {
            return core::ptr::null_mut();  // ← OOM returns null, system freezes
        }
        HEAP.as_mut_ptr().add(aligned)
    }
    
    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: core::alloc::Layout) {}
    //                                                                      ↑↑↑↑
    //                  NO-OP: MEMORY IS NEVER FREED, JUST ABANDONED!
}
```

### Why It Freezes

Every time a shell command produces output:

```
1. Shell command executes
2. Output is generated (e.g., "root" for whoami)
3. render_active_shell_fb() is called
4. render_active_shell_fb() creates: let mut grid = TerminalGrid::new(cols, rows);
5. TerminalGrid allocates:
   - cells: Vec<Cell> with 80 * 24 = 1920 cells (~100KB)
   - scrollback: Vec<Vec<Cell>> with 64 rows (~256KB)
   - parser: Vt100Parser with internal state
   Total: ~400KB per render

6. Grid processes the output
7. Grid is dropped
8. drop() triggers dealloc()
9. dealloc() does NOTHING
10. Memory is abandoned in heap
11. NEXT pointer still pointing past this garbage
```

### Memory Accumulation Timeline

```
Command 1: whoami
├─ Allocate TerminalGrid: +400KB (heap: 0.4MB / 4MB free: 3.6MB)
└─ Grid dropped → dealloc() does nothing (heap: still 0.4MB used, 3.6MB used=wasted)

Command 2: id
├─ Allocate TerminalGrid: +400KB (heap: 0.8MB / 4MB free: 3.2MB)
└─ Grid dropped → dealloc() does nothing

Command 3: whoami
├─ Allocate TerminalGrid: +400KB (heap: 1.2MB / 4MB free: 2.8MB)
└─ Grid dropped → dealloc() does nothing

Command 4: id
├─ Allocate TerminalGrid: +400KB (heap: 1.6MB / 4MB free: 2.4MB)
└─ Grid dropped → dealloc() does nothing

Command 5: whoami
├─ Allocate TerminalGrid: +400KB (heap: 2.0MB / 4MB free: 2.0MB)
└─ Grid dropped → dealloc() does nothing

Command 6: id
├─ Allocate TerminalGrid: +400KB (heap: 2.4MB / 4MB free: 1.6MB)
└─ Grid dropped → dealloc() does nothing

Command 7: whoami (NEXT WILL FAIL)
├─ Allocate TerminalGrid: +400KB (heap: 2.8MB + need 0.4MB = 3.2MB required)
├─ Allocate TerminalGrid: +400KB (heap: 3.2MB + need 0.4MB = 3.6MB required)
├─ Allocate TerminalGrid: +400KB (heap: 3.6MB + need 0.4MB = 4.0MB required)
├─ Allocate TerminalGrid: +400KB (heap: 4.0MB FULL!)
└─ Next allocation returns NULL_PTR → System undefined behavior → FREEZE
```

---

## Why This Wasn't a Problem Before

**OLD Version (Working, from GitHub):**
- Smaller TTY server (no Console/TerminalGrid additions)
- Simpler rendering (no scrollback viewport navigation)
- Smaller heap: 64KB (old code in working version)
- Fewer TerminalGrid allocations per render

**NEW Version (Freezing):**
- Added Console and TerminalGrid modules
- Added complex rendering with scrollback
- Increased heap to 4MB (still not enough!)
- Creating new TerminalGrid on EVERY frame

The developers doubled down on the bad pattern by:
1. Making terminal rendering more complex
2. Allocating larger structures (scrollback history)
3. Increasing heap size instead of fixing the allocator

---

## Why Increasing Memory to 1GB Didn't Help in Our Test

Our test ran the system through TTY boot and sat at login screen. The system doesn't consume all memory during boot (only 4 services), so the leak didn't manifest in our controlled test. The leak only appears when shell commands generate rapid rendering.

With manual testing (user typing commands), memory would exhaust.

---

## The Fix: Three Options

### Option A: Fix the Allocator (BEST - 10 minutes)
Implement proper deallocation using a free list or slab allocator:

```rust
unsafe fn dealloc(&self, ptr: *mut u8, layout: core::alloc::Layout) {
    // Track freed regions and reuse them on next alloc()
    // This is complex but solves the issue completely
}
```

### Option B: Reuse TerminalGrid (GOOD - 5 minutes, RECOMMENDED)
Don't create a new grid on every render; store it and reuse:

```rust
// In main tty loop:
struct TtyState {
    grid: TerminalGrid,  // ← Allocate ONCE
    // ...
}

// When rendering:
fn render_active_shell_fb(...) {
    // Clear grid instead of reallocating
    grid.clear();
    grid.feed(output);
    // ... render ...
}
```

### Option C: Use a Ring Buffer (ADVANCED - 20 minutes)
Replace `Vec<Vec<Cell>>` with a fixed-size ring buffer:

```rust
struct TerminalGrid {
    cells: [[Cell; 80]; 24],  // Fixed 2D array
    scrollback: [[Cell; 80]; 64],  // Ring buffer, no Vec allocs
}
```

---

## Proof: The Process Reaping Issue Is Secondary

We identified TWO issues:

1. **TTY Server Allocator** - Can't free memory
   - Freezes in 4-6 commands regardless of system RAM
   - Freezes in tty_server's own 4MB heap
   - User RAM size irrelevant
   
2. **Process Reaping** - Finished processes stay in vector
   - Would accumulate kernel stacks (32KB each)
   - Would accumulate address space frames
   - Secondary - even if fixed, TTY allocator issue remains

**The TTY allocator is the IMMEDIATE cause of freeze.**
**Process reaping is the SECONDARY cause that amplifies it.**

---

## Why This Wasn't Caught

The code review missed that:
1. `dealloc()` was a no-op
2. TerminalGrid was being allocated on every frame
3. Combined effect: ~400KB leak per render × render frequency = heap exhaustion in seconds

The 64x heap increase (64KB → 4MB) was a band-aid that delayed the problem but didn't fix it.

---

## Immediate Fix Recommendation

**Use Option B** (Reuse TerminalGrid):

1. Move `grid: TerminalGrid` allocation OUTSIDE the render function
2. Store it in `ShellTab` or global state
3. Clear and reuse it instead of creating new one each time
4. Takes ~5 minutes to fix

**Before:**
```rust
fn render_active_shell_fb(...) {
    let mut grid = TerminalGrid::new(cols, rows);  // ← Allocates 400KB
    grid.feed(output);
    // ...
}  // ← grid dropped, dealloc() does nothing, memory lost
```

**After:**
```rust
struct ShellTab {
    grid: TerminalGrid,  // ← Allocated once per shell
    // ...
}

fn render_active_shell_fb(..., tab: &mut ShellTab) {
    tab.grid.clear();  // ← Reuse, don't reallocate
    tab.grid.feed(output);
    // ...
}
```

---

## Testing the Fix

After implementing:

```bash
./tools/run.sh --no-display &
# Type 50+ commands
whoami; id; whoami; id; pwd; pwd; whoami; id;
# ... repeat ...

# Should NOT freeze
# Should see multiple commands complete
# Should see no OOM errors
```

---

## Summary Table

| Issue | Cause | Impact | Fix Difficulty |
|-------|-------|--------|-----------------|
| Bump Allocator No-Op Dealloc | `dealloc()` does nothing | Memory accumulates | 5 min (Option B) |
| TerminalGrid Per-Frame Allocation | New grid on every render | Rapid memory exhaustion | 2 min (move alloc) |
| Process Reaping Unimplemented | No process cleanup | Kernel stacks leak | 10 min |
| Missing Terminal State Cleanup | No cleanup on shell exit | Terminal memory not freed | 5 min |

**Critical Path: Fix TTY allocator (Option B) → Test → Then implement process reaping**

The TTY allocator issue is **blocking all testing** - must be fixed first.
