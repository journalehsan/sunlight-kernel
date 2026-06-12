# Scrollback Navigation - Quick Reference

## Keyboard Controls

### Scrollback Navigation
```
CTRL+UP        Scroll up 1 line (view older history)
CTRL+DOWN      Scroll down 1 line (view newer history)
SHIFT+PageUp   Scroll up 24 lines (full page older)
SHIFT+PageDown Scroll down 24 lines (full page newer)
<any key>      Return to live view (reset scroll)
```

## Architecture Diagram

```
┌─────────────────────────────────────────────────────────────┐
│                     Keyboard Input                           │
│            (PS/2 Scancode via IRQ1)                          │
└────────────────────┬────────────────────────────────────────┘
                     │
                     ▼
┌─────────────────────────────────────────────────────────────┐
│     kernel/arch/x86_64/keyboard.rs                           │
│  • PS/2 scancode translation to KeyEvent                     │
│  • Modifier state tracking (Ctrl, Shift, Alt)                │
│  • Key code: 0x48=Up, 0x50=Down, 0x49=PageUp, 0x51=PageDown│
└────────────────────┬────────────────────────────────────────┘
                     │
        IPC:KbdMsg::KEY_EVENT
                     │
                     ▼
┌─────────────────────────────────────────────────────────────┐
│        services/tty_server/src/main.rs                       │
│                                                              │
│  ┌─────────────────────────────────────────────────┐        │
│  │ Keyboard Handler                                 │        │
│  │ • Detect Ctrl+Up/Down → scroll_offset±1         │        │
│  │ • Detect Shift+PageUp/Down → scroll_offset±24   │        │
│  │ • Any key press → reset scroll_offset=0         │        │
│  │ • Set needs_render = true                        │        │
│  └─────────────────────────────────────────────────┘        │
│                         │                                    │
│                         ▼                                    │
│  ┌─────────────────────────────────────────────────┐        │
│  │ SCROLLBACK_STATE[tab_idx]                        │        │
│  │ {                                                │        │
│  │   viewport_offset: usize (0..256)               │        │
│  │ }                                                │        │
│  └─────────────────────────────────────────────────┘        │
│                         │                                    │
│                         ▼                                    │
│  ┌─────────────────────────────────────────────────┐        │
│  │ Render Pipeline (render_active_shell_fb)        │        │
│  │ • Calculate: cols = fb_width / 8                │        │
│  │ •            rows = (fb_height - 114) / 16      │        │
│  │ • Create TerminalGrid(cols, rows)               │        │
│  │ • grid.feed(output_bytes)                       │        │
│  │ • Update TERMINAL_GEOMETRY[tab_idx]             │        │
│  │ • term_cells = grid.to_term_cells_with_offset() │        │
│  └─────────────────────────────────────────────────┘        │
│                         │                                    │
│  ┌─────────────────────────────────────────────────┐        │
│  │ TERMINAL_GEOMETRY[tab_idx]                       │        │
│  │ {                                                │        │
│  │   cols: u32,                                    │        │
│  │   rows: u32,                                    │        │
│  │   viewport_offset: usize,                       │        │
│  │   max_scrollback: usize,                        │        │
│  │ }                                                │        │
│  └─────────────────────────────────────────────────┘        │
└────────────────────┬────────────────────────────────────────┘
                     │
                     ▼
┌─────────────────────────────────────────────────────────────┐
│        sunlight-tui (render_terminal_grid)                   │
│  • Render term_cells to framebuffer                          │
│  • Draw chrome (header, tabbar, footer)                      │
│  • Display scrollback offset indicator (optional)            │
└────────────────────┬────────────────────────────────────────┘
                     │
                     ▼
            ┌─────────────────┐
            │  Framebuffer    │
            │   Display       │
            └─────────────────┘
```

## Console Buffer Structure

```
Live Screen (current display, cols×rows cells):
┌─────────────────────────┐
│ Row 0: [Cell Cell Cell] │
│ Row 1: [Cell Cell Cell] │
│ Row 2: [Cell Cell Cell] │ ← cursor_y
│ Row 3: [Cell Cell Cell] │
│ Row 4: [Cell Cell Cell] │
└─────────────────────────┘

When newline at row 4, col 79:
  → Save row 0 to scrollback
  → Shift rows 1-4 up
  → Clear row 4

Scrollback History (up to 256 rows):
┌──────────────────────────┐
│ Row[0]: [old output..] │  ← oldest (first to be discarded)
│ Row[1]: [old output..] │
│ Row[2]: [old output..] │
│ ...                      │
│ Row[254]: [old output..] │
│ Row[255]: [old output..] │  ← newest (most recent)
└──────────────────────────┘

scroll_offset behavior:
  offset = 0   → Show current live screen (row 0-23)
  offset = 1   → Show scrollback[255] + live[0-22]
  offset = 24  → Show scrollback[232-255]
  offset = 256 → Show scrollback[0-23] (oldest history)
```

## Terminal Geometry Query API

```rust
// Get full geometry struct
pub fn get_terminal_geometry(tab_idx: usize) -> Option<TerminalGeometry>

// Get dimensions only
pub fn get_terminal_dims(tab_idx: usize) -> Option<(u32, u32)>

// Get current viewport offset
pub fn get_viewport_offset(tab_idx: usize) -> usize

// Example usage in userland:
if let Some((cols, rows)) = get_terminal_dims(0) {
    // Adjust layout based on cols × rows
    render_layout(cols, rows);
}
```

## File Structure

```
SunlightOS Kernel
├── kernel/src/arch/x86_64/
│   └── keyboard.rs              (KEY CODE DEFS)
│       • PAGE_UP_EXT = 0x49
│       • PAGE_DOWN_EXT = 0x51
│
├── sunlight-tty/src/
│   ├── console.rs               (VIEWPORT RENDERING)
│   │   • scroll_offset: usize
│   │   • to_term_cells_with_offset()
│   │   • scroll_up/down_viewport()
│   │
│   └── grid.rs                  (LEGACY, COMPATIBLE)
│       • Also implements viewport rendering
│       • 64-line scrollback limit
│
└── services/tty_server/src/
    └── main.rs                  (SCROLL CONTROL & GEOMETRY)
        • TabScrollback struct
        • TerminalGeometry struct
        • Keyboard handler (Ctrl+/Shift+keys)
        • render_active_shell_fb() geometry update
        • get_terminal_*() APIs
```

## Performance Characteristics

| Metric | Value |
|--------|-------|
| Scrollback memory (per tab) | ~64 KB (256 lines @ 80×24) |
| Max tabs | 10 |
| Total memory | ~640 KB |
| Scroll response | <1ms (no reallocation) |
| Render with offset | Single pass, no copying |
| Geometry update | Per-frame (efficient) |

## Implementation Summary

✅ **Scrollback View Window**
  - `scroll_offset` state variable in Console
  - Default offset=0 (live view)
  - Rendering reads offset for viewport positioning

✅ **Keyboard Scrolling Interceptors**
  - Ctrl+Up/Down: 1-line scroll
  - Shift+PageUp/Down: 24-line scroll
  - Auto-reset on typing

✅ **Terminal Geometry Mapping**
  - Dimension tracking per tab
  - Dynamic calculation from framebuffer size
  - Public query API for userland

✅ **Window Resize Signaling** (Partial)
  - Geometry state updated each render
  - Can be queried without IPC
  - Foundation for SIGWINCH-like signaling
