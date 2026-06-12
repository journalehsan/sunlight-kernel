# Scrollback Implementation - Code Changes Summary

## Modified Files

### 1. kernel/src/arch/x86_64/keyboard.rs

**Added Key Code Constants:**
```rust
pub const PAGE_UP_EXT: u8 = 0x49;   // prefixed with 0xE0
pub const PAGE_DOWN_EXT: u8 = 0x51; // prefixed with 0xE0
```

PS/2 Set 1 codes for extended keys:
- PageUp: `E0 49` (single byte form: 0x49 when used in process_scancode)
- PageDown: `E0 51` (single byte form: 0x51 when used in process_scancode)
- Regular Up: `0x48`
- Regular Down: `0x50`

---

### 2. sunlight-tty/src/console.rs

**Added Field to Console Struct:**
```rust
pub struct Console {
    cols: usize,
    rows: usize,
    cells: Vec<Cell>,
    scrollback: Vec<Vec<Cell>>,
    
    // NEW: Viewport scroll offset
    scroll_offset: usize,  // 0 = live view, 1..N = history lines
    
    cursor_x: usize,
    cursor_y: usize,
    // ... rest of fields
}
```

**Updated Constructors:**
```rust
pub fn new(cols: usize, rows: usize) -> Self {
    // ... initialization ...
    Self {
        cols,
        rows,
        cells,
        scrollback: Vec::new(),
        scroll_offset: 0,  // NEW: Initialize to live view
        cursor_x: 0,
        cursor_y: 0,
        // ... rest
    }
}

pub fn new_with_margin(cols: usize, rows: usize, top_margin_rows: usize) -> Self {
    // ... initialization ...
    Self {
        cols,
        rows: total_rows,
        cells,
        scrollback: Vec::new(),
        scroll_offset: 0,  // NEW
        cursor_x: 0,
        cursor_y: top_margin_rows,
        // ... rest
    }
}
```

**New Viewport Control Methods:**
```rust
impl Console {
    /// Set absolute scroll offset (0 = live, 1..N = history)
    pub fn set_scroll_offset(&mut self, offset: usize) {
        self.scroll_offset = offset.min(self.scrollback.len());
        self.dirty = true;
    }

    /// Scroll up (view older history)
    pub fn scroll_up_viewport(&mut self) {
        if self.scroll_offset < self.scrollback.len() {
            self.scroll_offset += 1;
            self.dirty = true;
        }
    }

    /// Scroll down (view newer history toward live)
    pub fn scroll_down_viewport(&mut self) {
        if self.scroll_offset > 0 {
            self.scroll_offset -= 1;
            self.dirty = true;
        }
    }

    /// Reset to live view
    pub fn reset_scroll_offset(&mut self) {
        if self.scroll_offset > 0 {
            self.scroll_offset = 0;
            self.dirty = true;
        }
    }

    /// Query current offset
    pub fn get_scroll_offset(&self) -> usize {
        self.scroll_offset
    }

    /// Query terminal dimensions
    pub fn dims(&self) -> (usize, usize) {
        (self.cols, self.rows)
    }
}
```

**Improved Viewport Rendering:**
```rust
/// Render with scrollback history at viewport_offset
pub fn to_term_cells_with_offset(&self, ansi_colors: &[u32; 16], viewport_offset: usize) -> Vec<TermCell> {
    if viewport_offset == 0 {
        return self.to_term_cells(ansi_colors);
    }

    let mut result = Vec::new();

    for screen_row in 0..self.rows {
        let history_row_idx = if self.scrollback.len() > viewport_offset {
            self.scrollback.len() - viewport_offset + screen_row
        } else {
            screen_row
        };

        let cells_to_render = if history_row_idx < self.scrollback.len() {
            &self.scrollback[history_row_idx]
        } else if screen_row < self.rows {
            let live_idx = screen_row * self.cols;
            &self.cells[live_idx..core::cmp::min(live_idx + self.cols, self.cells.len())]
        } else {
            &[]
        };

        for cell in cells_to_render {
            let fg_idx = if cell.bold && cell.fg < 8 {
                cell.fg + 8
            } else {
                cell.fg
            };
            let fg = ansi_colors[fg_idx as usize % 16];
            let bg = ansi_colors[cell.bg as usize % 16];
            result.push(TermCell { ch: cell.ch, fg, bg });
        }

        // Pad row to cols width
        while result.len() % self.cols != 0 {
            result.push(TermCell { ch: b' ', fg: ansi_colors[7], bg: ansi_colors[0] });
        }
    }

    result
}
```

**Fixed Unreachable Pattern:**
Removed duplicate `b'2'` case in `process_csi_start()` (already covered by `b'0'..=b'9'`).

---

### 3. services/tty_server/src/main.rs

**Added Terminal Geometry Structure:**
```rust
#[derive(Clone, Copy, Debug)]
pub struct TerminalGeometry {
    pub cols: u32,
    pub rows: u32,
    pub viewport_offset: usize,
    pub max_scrollback: usize,
}

impl TerminalGeometry {
    const fn new() -> Self {
        Self {
            cols: 80,
            rows: 24,
            viewport_offset: 0,
            max_scrollback: 256,
        }
    }

    fn update(&mut self, cols: u32, rows: u32, viewport_offset: usize) {
        self.cols = cols;
        self.rows = rows;
        self.viewport_offset = viewport_offset;
    }

    fn set_viewport(&mut self, offset: usize) {
        self.viewport_offset = offset;
    }
}
```

**Global Geometry State:**
```rust
static mut TERMINAL_GEOMETRY: [TerminalGeometry; MAX_TABS] =
    [TerminalGeometry { cols: 80, rows: 24, viewport_offset: 0, max_scrollback: 256 }; MAX_TABS];
```

**Enhanced Keyboard Handler:**
```rust
// In main loop TtyState::Shell block:
if msg.label == KbdMsg::KEY_EVENT {
    let (keycode, pressed, shift, ctrl, _alt, ctrl_ascii) =
        unpack_key_event(msg.words[0]);

    // NEW: Scrollback viewport control
    let is_ctrl_scroll = pressed && ctrl && (keycode == 0x48 || keycode == 0x50);
    let is_shift_page = pressed && shift && (keycode == 0x49 || keycode == 0x51);

    if is_ctrl_scroll || is_shift_page {
        unsafe {
            let scrollback = &mut SCROLLBACK_STATE[active_tab];
            match keycode {
                0x48 if ctrl => {
                    // Ctrl+Up: scroll up by 1 line
                    scrollback.viewport_offset =
                        (scrollback.viewport_offset + 1).min(256);
                    needs_render = true;
                }
                0x50 if ctrl => {
                    // Ctrl+Down: scroll down by 1 line
                    scrollback.viewport_offset =
                        scrollback.viewport_offset.saturating_sub(1);
                    needs_render = true;
                }
                0x49 if shift => {
                    // Shift+PageUp: scroll up by 24 lines
                    scrollback.viewport_offset =
                        (scrollback.viewport_offset + 24).min(256);
                    needs_render = true;
                }
                0x51 if shift => {
                    // Shift+PageDown: scroll down by 24 lines
                    if scrollback.viewport_offset >= 24 {
                        scrollback.viewport_offset -= 24;
                    } else {
                        scrollback.viewport_offset = 0;
                    }
                    needs_render = true;
                }
                _ => {}
            }
        }
    } else if pressed && ctrl {
        // Handle other Ctrl+ commands (Ctrl+T, Ctrl+W, Ctrl+1-9)
        if let Some(a) = ctrl_ascii {
            if handle_ctrl_key(...) {
                needs_render = true;
            }
        }
    }
}

// Reset scrollback on normal keypress
if let Some(ascii) = key_ascii_from_msg(&msg) {
    unsafe {
        SCROLLBACK_STATE[active_tab].viewport_offset = 0;  // Return to live
    }
    // ... rest of input handling
}
```

**Updated Render Function:**
```rust
fn render_active_shell_fb(
    fb_addr: u64,
    fb_w: u32,
    fb_h: u32,
    fb_p: u32,
    tabs: &[ShellTab; MAX_TABS],
    tab_count: usize,
    active_tab: usize,
) {
    // Compute terminal dimensions
    let char_w: u32 = 8;
    let char_h: u32 = 16;
    let chrome_h: u32 = 48 + 26 + 32 + 8;  // header + tabbar + footer + gaps
    let avail_h = fb_h.saturating_sub(chrome_h);
    let rows = (avail_h / char_h) as usize;
    let cols = (fb_w / char_w) as usize;

    // NEW: Update terminal geometry state
    unsafe {
        let viewport_offset = SCROLLBACK_STATE[active_tab].viewport_offset;
        TERMINAL_GEOMETRY[active_tab].update(cols as u32, rows as u32, viewport_offset);
    }

    // ... rest of rendering ...
    
    // Get viewport offset for scrollback
    let viewport_offset = unsafe { SCROLLBACK_STATE[active_tab].viewport_offset };

    // Render with scrollback offset if active
    let term_cells = if viewport_offset > 0 {
        grid.to_term_cells_with_offset(&ANSI_COLORS, viewport_offset)
    } else {
        grid.to_term_cells(&ANSI_COLORS)
    };
    
    // ... rest of rendering ...
}
```

**New Public API Functions:**
```rust
/// Get current terminal geometry for the active tab
pub fn get_terminal_geometry(tab_idx: usize) -> Option<TerminalGeometry> {
    if tab_idx < MAX_TABS {
        unsafe { Some(TERMINAL_GEOMETRY[tab_idx]) }
    } else {
        None
    }
}

/// Get terminal dimensions (cols, rows) for the active tab
pub fn get_terminal_dims(tab_idx: usize) -> Option<(u32, u32)> {
    get_terminal_geometry(tab_idx).map(|g| (g.cols, g.rows))
}

/// Get current viewport offset for the active tab
pub fn get_viewport_offset(tab_idx: usize) -> usize {
    if tab_idx < MAX_TABS {
        unsafe { TERMINAL_GEOMETRY[tab_idx].viewport_offset }
    } else {
        0
    }
}
```

---

## Key Points

### Data Flow
1. **Keyboard Input**: PS/2 scancode → keycode + modifiers
2. **Scroll Detection**: Check for (Ctrl & Up/Down) or (Shift & PageUp/PageDown)
3. **State Update**: Modify `SCROLLBACK_STATE[tab].viewport_offset`
4. **Render Trigger**: Set `needs_render = true`
5. **Geometry Update**: Calculate cols/rows and update `TERMINAL_GEOMETRY[tab]`
6. **Viewport Rendering**: Use offset to read from scrollback history instead of live cells
7. **Display**: Render to framebuffer with proper viewport

### Memory Safety
- All viewport operations are bounds-checked
- `scroll_offset` clamped to `scrollback.len()` and 256 max
- Safe array access with bounds checks
- History cells never accessed out of range

### Performance
- O(1) scroll operations (just update offset)
- Single-pass rendering with offset calculation
- No buffer copying or reallocation on scroll
- Geometry update per-frame (negligible overhead)

### Backward Compatibility
- Existing code using `Console` without viewport still works
- `viewport_offset = 0` by default (live view)
- No breaking changes to public API
- Grid.rs also supports viewport rendering
