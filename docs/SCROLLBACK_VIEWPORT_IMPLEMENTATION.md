# Scrollback UI & Window Resize Signaling Implementation

## Overview

This document describes the complete implementation of scrollback viewport navigation, keyboard interceptors for scrolling, and terminal geometry tracking for SunlightOS Phase 6.2.

## Architecture

### 1. Console Scrollback Viewport (sunlight-tty/src/console.rs)

**Data Structures Added:**
```rust
pub struct Console {
    // ... existing fields ...
    
    // Viewport scroll offset: 0 = live view, 1..N = viewing history
    scroll_offset: usize,
}
```

**Scrollback Buffer:**
- Maintains up to 256 lines of history in `scrollback: Vec<Vec<Cell>>`
- Each line is a complete row of terminal cells
- When screen scrolls, top row is saved to scrollback before shift-up

**Viewport API Methods:**
- `set_scroll_offset(offset)` - Set absolute scroll position
- `scroll_up_viewport()` - Increment offset (view older history)
- `scroll_down_viewport()` - Decrement offset (view newer content)
- `reset_scroll_offset()` - Return to live view (offset = 0)
- `get_scroll_offset()` - Query current viewport offset
- `dims()` - Get terminal dimensions (cols, rows)

**Rendering with Offset:**
```rust
pub fn to_term_cells_with_offset(&self, ansi_colors: &[u32; 16], viewport_offset: usize) -> Vec<TermCell>
```
- When `viewport_offset == 0`: returns normal live screen cells
- When `viewport_offset > 0`: renders from scrollback history
  - For each screen row, pulls from `scrollback[scrollback.len() - viewport_offset + row_idx]`
  - Falls back to live cells if history exhausted
  - Pads rows to maintain grid structure

### 2. Keyboard Scrolling Interceptors (services/tty_server/src/main.rs)

**Key Bindings Implemented:**

| Combination | Action | Amount |
|-------------|--------|--------|
| Ctrl+Up | Scroll up (view older) | 1 line |
| Ctrl+Down | Scroll down (view newer) | 1 line |
| Shift+PageUp | Scroll up | 24 lines (full page) |
| Shift+PageDown | Scroll down | 24 lines (full page) |
| Any printable key | Reset to live view | - |

**Implementation:**
```rust
struct TabScrollback {
    viewport_offset: usize,
}

// Per-tab scrollback state
static mut SCROLLBACK_STATE: [TabScrollback; MAX_TABS] = [TabScrollback { viewport_offset: 0 }; MAX_TABS];
```

**Keyboard Handler Logic:**
1. Detect key press with modifiers (Ctrl or Shift)
2. Check for arrow keys (0x48=Up, 0x50=Down) or page keys (0x49=PageUp, 0x51=PageDown)
3. Update `SCROLLBACK_STATE[active_tab].viewport_offset` accordingly
4. Clamp offset to valid range (0..256)
5. Set `needs_render = true` to trigger visual update
6. On any normal keypress: reset offset to 0 (snap to live view)

**Key Code Definitions:**
```rust
pub const PAGE_UP_EXT: u8 = 0x49;   // E0 49 (extended)
pub const PAGE_DOWN_EXT: u8 = 0x51; // E0 51 (extended)
```

### 3. Terminal Geometry Tracking (services/tty_server/src/main.rs)

**Geometry State Structure:**
```rust
#[derive(Clone, Copy, Debug)]
pub struct TerminalGeometry {
    pub cols: u32,           // Terminal width in characters
    pub rows: u32,           // Terminal height in characters
    pub viewport_offset: usize,  // Current scroll offset
    pub max_scrollback: usize,   // Maximum history lines (256)
}
```

**Global Geometry State:**
```rust
static mut TERMINAL_GEOMETRY: [TerminalGeometry; MAX_TABS] =
    [TerminalGeometry { cols: 80, rows: 24, viewport_offset: 0, max_scrollback: 256 }; MAX_TABS];
```

**Dimension Calculation (in render_active_shell_fb):**
```
Terminal dimensions = Framebuffer dimensions / Glyph size
  char_w = 8 pixels
  char_h = 16 pixels
  chrome_h = 48 (header) + 26 (tabbar) + 32 (footer) + 8 (gaps) = 114 pixels
  
  cols = fb_width / 8
  rows = (fb_height - 114) / 16
```

**Public Query API:**
```rust
pub fn get_terminal_geometry(tab_idx: usize) -> Option<TerminalGeometry>
pub fn get_terminal_dims(tab_idx: usize) -> Option<(u32, u32)>
pub fn get_viewport_offset(tab_idx: usize) -> usize
```

### 4. Data Flow

```
Keyboard Event
    ↓
[kernel/arch/x86_64/keyboard.rs] process_scancode()
    - Translates PS/2 scancode to KeyEvent
    - Extracts keycode, modifiers (ctrl, shift, alt), ascii
    - Sends via IPC to tty_server
    ↓
[services/tty_server/src/main.rs] Main loop
    - Receives KEY_EVENT message
    - Detects scroll key combinations
    - Updates SCROLLBACK_STATE[active_tab].viewport_offset
    - Sets needs_render = true
    ↓
[services/tty_server/src/main.rs] render_active_shell_fb()
    - Calculates terminal dimensions
    - Updates TERMINAL_GEOMETRY[active_tab]
    - Reads viewport_offset from SCROLLBACK_STATE
    - Calls grid.to_term_cells_with_offset(ansi_colors, viewport_offset)
    ↓
[sunlight_tui] render_terminal_grid()
    - Renders term_cells to framebuffer with chrome/layout
```

## Integration Points

### Console & Grid Compatibility
- Both `Console` (sunlight-tty/console.rs) and `TerminalGrid` (sunlight-tty/grid.rs) have viewport support
- Currently using `TerminalGrid` in tty_server (aliased as `Console` for backward compat)
- Both implementations of `to_term_cells_with_offset()` are functionally equivalent

### Memory Footprint
- `Console` scrollback: 256 lines × cols cells × sizeof(Cell) = ~64KB per tab
- `TerminalGrid` scrollback: 64 lines × cols cells × sizeof(Cell) = ~16KB per tab
- Current: 10 tabs max → ~160-640KB depending on screen resolution
- Efficient: Single pass rendering, no intermediate buffers

### Userland Integration
- Terminal dimensions queryable via `get_terminal_geometry()` 
- Can be exposed to userland processes (e.g., `sysfetch` for responsive layouts)
- Viewport offset visible to rendering layer for correct output

## Testing Scenarios

### Scrollback Navigation
1. Fill terminal with output (> 24 lines)
2. Press Ctrl+Up repeatedly → view scrolls into history ✓
3. Press Shift+PageUp → scroll by full page ✓
4. Press Ctrl+Down → return toward live view ✓
5. Type any character → snap back to live output ✓

### Geometry Updates
1. Terminal renders with correct cols/rows calculation
2. TERMINAL_GEOMETRY updated per render frame
3. Userland can query dimensions without reboot

### Multi-Tab Support
1. Each tab has independent viewport_offset
2. Switch tabs (Ctrl+1..9) → preserves scroll position per tab
3. Geometry tracks per tab correctly

## Files Modified

1. **sunlight-tty/src/console.rs**
   - Added `scroll_offset: usize` field
   - Implemented `set_scroll_offset()`, `scroll_up_viewport()`, `scroll_down_viewport()`, `reset_scroll_offset()`, `get_scroll_offset()`, `dims()`
   - Proper implementation of `to_term_cells_with_offset()` with scrollback history rendering
   - Fixed unreachable pattern in `process_csi_start()`

2. **services/tty_server/src/main.rs**
   - Added `TerminalGeometry` struct with cols, rows, viewport_offset, max_scrollback
   - Added `TERMINAL_GEOMETRY` global state array
   - Enhanced keyboard handler to support Ctrl+Up/Down and Shift+PageUp/PageDown
   - Updated `render_active_shell_fb()` to calculate and store geometry
   - Added public API functions: `get_terminal_geometry()`, `get_terminal_dims()`, `get_viewport_offset()`

3. **kernel/src/arch/x86_64/keyboard.rs**
   - Added `PAGE_UP_EXT: u8 = 0x49` constant
   - Added `PAGE_DOWN_EXT: u8 = 0x51` constant
   - Support for extended key codes (PS/2 E0 prefix)

## Future Enhancements

- Scrollback line limit configuration (currently hardcoded to 256)
- Mouse wheel scrolling support
- Scroll position indicator in status bar
- Dynamic scrollback history allocation (currently fixed-size)
- Full terminal resize event signaling (SIGWINCH-like mechanism)
