# Terminal Code Changes Analysis - Root Cause Investigation

## Summary
Compared working code (GitHub, Jun 11) with current code (Current, Jun 12) and found **significant new terminal infrastructure** that likely introduced the freeze.

## Key Structural Changes

### 1. **NEW MODULES ADDED**
Working version:
```
sunlight-tty/src/
├── lib.rs (87 bytes - minimal)
├── login.rs
├── mux.rs
├── session.rs
├── shell.rs
└── vt100.rs
```

Current version:
```
sunlight-tty/src/
├── lib.rs (267 bytes)
├── console.rs (15KB) ← NEW
├── grid.rs (11KB) ← NEW
├── login.rs
├── mux.rs
├── session.rs
├── shell.rs
└── vt100.rs
```

### 2. **TWO TERMINAL GRID IMPLEMENTATIONS**

**NEW: console.rs - Simple ANSI Stream Terminal**
```rust
pub struct Console {
    cols: usize,
    rows: usize,
    cells: Vec<Cell>,
    scrollback: Vec<Vec<Cell>>,        // ← Each line is a Vec!
    scroll_offset: usize,
    cursor_x: usize,
    cursor_y: usize,
    // ... more state ...
}

const SCROLLBACK_MAX: usize = 256;     // ← Limited but still large
```

**NEW: grid.rs - Terminal Grid with VT100 Parser**
```rust
pub struct TerminalGrid {
    cells: Vec<Cell>,
    scrollback: Vec<Vec<Cell>>,        // ← ANOTHER scrollback!
    cursor_row: usize,
    cursor_col: usize,
    // ... more state ...
}

const SCROLLBACK_LINES: usize = 64;
```

**Problem:** Both implementations allocate `Vec<Vec<Cell>>` for scrollback. If both are being instantiated and not properly cleaned up, memory accumulates.

---

## TTY Server Changes

### Heap Size Increase
**Before:**
```rust
static mut HEAP: [u8; 65536] = [0; 65536];  // 64KB
```

**After:**
```rust
static mut HEAP: [u8; 4 * 1024 * 1024] = [0; 4 * 1024 * 1024];  // 4MB
```

**Implication:** Suggests terminal code needs MORE memory - either it's fragmenting or allocating heavily.

### New Terminal State Management
```rust
// NEW: Terminal geometry tracking per tab
static mut TERMINAL_GEOMETRY: [TerminalGeometry; MAX_TABS] = [TerminalGeometry { ... }; MAX_TABS];

// NEW: Scrollback state per tab
static mut SCROLLBACK_STATE: [TabScrollback; MAX_TABS] = [TabScrollback { viewport_offset: 0 }; MAX_TABS];

// NEW: Terminal grid per tab
struct ShellTab {
    // ... existing fields ...
    username: [u8; 32],              // ← NEW
    username_len: usize,             // ← NEW
}
```

### New Features Added
1. **Scrollback Viewport Navigation** (Ctrl+Up/Down, Shift+PageUp/Down)
2. **Terminal Geometry State** per tab (cols, rows, viewport_offset, max_scrollback)
3. **Tab-based Terminal Instances** (up to 10 tabs, each with own state)
4. **Dynamic Rendering Logic** checking output changes

---

## Likely Culprits

### 1. **Multiple Terminal Grid Instances** ⚠️ CRITICAL
If `Console` and `TerminalGrid` are BOTH being instantiated for each tab:
- 10 tabs × 2 implementations = 20 terminal grid instances
- Each with `Vec<Cell>` (80 × 24 = 1920 cells × 48 bytes per cell = ~92KB per grid)
- Plus scrollback: 256 lines × 80 cells per tab = 20+ MB potential memory use

### 2. **Scrollback Vec Allocation Pattern** ⚠️ MEDIUM
In console.rs line 313:
```rust
let temp: Vec<Cell> = self.cells[src_start..src_end].to_vec();
```
This creates a **new Vec on every line scroll**. On rapid output, this allocates/deallocates constantly, causing fragmentation.

### 3. **Unbounded Output Buffer** ⚠️ MEDIUM
```rust
const TERM_OUTPUT_MAX: usize = 4096;  // Increased from 2048
```
If output isn't being flushed properly, this could accumulate.

### 4. **IPC Buffer Management** ⚠️ LOW
```rust
const IPC_OUTPUT_BYTES: usize = 16;  // NEW, unclear purpose
```

---

## Memory Leak Mechanism Hypothesis

**Most Likely Scenario:**

1. Shell spawns for first command
2. Console/TerminalGrid instance created
3. Command output fills terminal, allocates scrollback rows
4. When shell exits, process is marked `Finished` (process reaping issue)
5. But the terminal grid instance with its scrollback memory is NOT freed
6. Next shell command spawns, allocates ANOTHER terminal grid
7. After 4-6 commands: Multiple terminal grids in memory consuming:
   - Scrollback: 256 lines/grid × 8 grids = 2000+ rows
   - Each row: 80 cells × 48 bytes = ~4KB
   - Total: 2000 × 4KB = 8MB+ just in scrollback alone

8. Process kernel stacks also accumulating (32KB each)
9. Together: Memory exhausted → freeze

---

## Secondary Issue: The Process Reaping

Even if we fix the terminal leak, the fundamental **process reaping is still broken**:

```rust
fn process_exit(_code: i32) -> ! {
    sched::with_scheduler(|s| {
        s.current_process_mut().state = ProcessState::Finished;  // ← Only this
    });
    // Process still in vector with all allocations intact!
}
```

---

## Recommended Fixes (In Order)

### Fix 1: Determine Which Implementation is Used
Check which of `Console` or `TerminalGrid` is actually being used:
```bash
grep -r "Console\|TerminalGrid" /home/ehsantor/Projects/sunlightos-kernel/services/tty_server/
```

If both are being allocated → REMOVE the unused one and use only one implementation.

### Fix 2: Clean Up Terminal on Shell Exit
When a shell process exits:
```rust
// When shell process marked Finished:
if let Some(tab) = active_shell_tab_mut(&mut tabs, active_tab) {
    // Clear the terminal grid
    tab.grid.clear();  // or drop it entirely
    tab.output_len = 0;
    tab.output.clear();
}
```

### Fix 3: Implement Process Reaping
(Already documented in FREEZE_DIAGNOSIS_COMPLETE.md)

### Fix 4: Optimize Terminal Scrollback
Either:
1. Use a ring buffer instead of `Vec<Vec<Cell>>` to avoid allocations
2. Limit scrollback size more aggressively
3. Use a fixed-size array instead of Vec

---

## Differences in Constants

| Constant | Before | After | Change |
|----------|--------|-------|--------|
| TERM_OUTPUT_MAX | 2048 | 4096 | ↑ 2x |
| INPUT_LINE_MAX | 128 | 256 | ↑ 2x |
| HEAP | 64KB | 4MB | ↑ 64x |
| SCROLLBACK_MAX (Console) | N/A | 256 | NEW |
| SCROLLBACK_LINES (Grid) | 64 | 64 | No change |

The 64x heap increase suggests developers knew terminal code was memory-hungry.

---

## Code Smell Indicators

```rust
// In tty_server:
unsafe { SCROLLBACK_STATE[active_tab].viewport_offset = 0; }  // Per-tab state
unsafe { TERMINAL_GEOMETRY[active_tab] }.viewport_offset }  // More per-tab state

// Multiple static mut arrays for different aspects:
// - SCROLLBACK_STATE[MAX_TABS]
// - TERMINAL_GEOMETRY[MAX_TABS]
// - Two different grid implementations (Console + TerminalGrid)

// Unclear: which one is actually in use?
pub use console::Console as TerminalGrid;  // Alias in lib.rs
```

The alias suggests one was supposed to replace the other, but both modules might still be instantiated.

---

## Test Strategy

1. **Confirm Multiple Grids Issue:**
   ```bash
   grep -c "Console::new\|TerminalGrid::new" services/tty_server/src/main.rs
   ```

2. **Check if Both Modules Are Used:**
   ```bash
   grep "use.*console\|use.*grid" services/tty_server/src/main.rs
   ```

3. **Monitor Instance Count:**
   Add logging to `Console::new()` and `TerminalGrid::new()`:
   ```rust
   pub fn new(cols: usize, rows: usize) -> Self {
       crate::serial_println!("[CONSOLE] Creating new instance");
       // ...
   }
   ```

4. **Run stress test and count allocations in serial output**

---

## Conclusion

The terminal improvements (Console grid, scrollback, viewport navigation) likely introduced:

1. **Multiple Terminal Grid Instances** - Each shell spawn gets a grid that's never freed
2. **Scrollback Memory Accumulation** - Lines stored as Vec<Cell> rows, all kept in memory
3. **Process Not Cleaning Up Terminal State** - When shell exits, terminal grid is abandoned in memory

Combined with the **unimplemented process reaping**, this creates a perfect memory leak:
- Process exits → Finished state → stays in scheduler.processes vector
- Terminal grid stays allocated → scrollback stays in memory
- Next shell → allocates another grid
- Repeat 4-6 times → memory exhausted → freeze

The 64x heap increase (64KB → 4MB) suggests this was partially worked around but not truly fixed.

---

## Immediate Action

**To restore working state (rollback):**
```bash
git checkout d77e535  # Last known good commit
# Or cherry-pick changes selectively, excluding Console/TerminalGrid implementation
```

**To fix properly:**
1. Remove one of the duplicate terminal grid implementations
2. Ensure terminal state is cleaned when shell exits
3. Implement process reaping
4. Test with 25+ commands without freeze
