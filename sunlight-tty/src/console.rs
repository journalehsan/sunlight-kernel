//! Simple ANSI stream-based terminal console (replaces Grid)
//!
//! Implements a classic VT100-style terminal with:
//! - Simple cell matrix (2D array of characters + colors)
//! - Sequential byte stream processing
//! - Basic ANSI escape sequences
//! - Full scrollback history with viewport offset

use alloc::vec::Vec;
pub use sunlight_tui::TermCell;

/// A single terminal cell: character + colors + attributes
#[derive(Clone, Copy, Debug)]
pub struct Cell {
    pub ch: u8,
    pub fg: u8,  // ANSI color index 0-15
    pub bg: u8,
    pub bold: bool,
}

impl Cell {
    const fn blank() -> Self {
        Cell { ch: b' ', fg: 7, bg: 0, bold: false }
    }
}

const SCROLLBACK_MAX: usize = 256;

/// Simple ANSI terminal console - maintains a 2D cell buffer
pub struct Console {
    cols: usize,
    rows: usize,

    // Main display buffer (row-major: row 0 col 0..cols, row 1 col 0..cols, etc.)
    cells: Vec<Cell>,

    // Scrollback history: each element is a complete row
    scrollback: Vec<Vec<Cell>>,

    // Viewport scroll offset: 0 = live view, 1..N = viewing history
    scroll_offset: usize,

    // Cursor position
    cursor_x: usize,
    cursor_y: usize,

    // Current text attributes
    fg_color: u8,
    bg_color: u8,
    bold: bool,

    // ANSI escape sequence state machine
    escape_state: EscapeState,

    // Dirty flag: set when content changes, cleared after checking
    pub dirty: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EscapeState {
    Normal,
    EscapeStart,           // saw ESC
    CsiStart,              // saw ESC[
    CsiParam { digits: [u8; 4], digit_count: u8 },  // collecting digits
}

impl Console {
    pub fn new(cols: usize, rows: usize) -> Self {
        let mut cells = Vec::new();
        cells.resize(cols * rows, Cell::blank());

        Self {
            cols,
            rows,
            cells,
            scrollback: Vec::new(),
            scroll_offset: 0,
            cursor_x: 0,
            cursor_y: 0,
            fg_color: 7,     // default white
            bg_color: 0,     // default black
            bold: false,
            escape_state: EscapeState::Normal,
            dirty: false,
        }
    }

    /// Create a console with top margin rows for chrome (titlebar, tabs, etc.)
    /// The returned cells will have blank rows at the top
    pub fn new_with_margin(cols: usize, rows: usize, top_margin_rows: usize) -> Self {
        // Create larger buffer to account for top margin
        let total_rows = rows + top_margin_rows;
        let mut cells = Vec::new();
        cells.resize(cols * total_rows, Cell::blank());

        Self {
            cols,
            rows: total_rows,  // Use total rows internally
            cells,
            scrollback: Vec::new(),
            scroll_offset: 0,
            cursor_x: 0,
            cursor_y: top_margin_rows,  // Start cursor below the margin
            fg_color: 7,
            bg_color: 0,
            bold: false,
            escape_state: EscapeState::Normal,
            dirty: false,
        }
    }

    /// Process a single byte from the input stream
    pub fn feed_byte(&mut self, byte: u8) {
        match self.escape_state {
            EscapeState::Normal => self.process_normal_byte(byte),
            EscapeState::EscapeStart => self.process_escape_start(byte),
            EscapeState::CsiStart => self.process_csi_start(byte),
            EscapeState::CsiParam { .. } => self.process_csi_param(byte),
        }
    }

    /// Process a byte stream (for compatibility with Grid::feed)
    pub fn feed(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.feed_byte(byte);
        }
    }

    /// Process bytes in normal (non-escape) mode
    fn process_normal_byte(&mut self, byte: u8) {
        match byte {
            0x1B => {
                // ESC - start of escape sequence
                self.escape_state = EscapeState::EscapeStart;
            }
            b'\n' => {
                // Newline: reset column and advance row
                self.cursor_x = 0;
                self.advance_row();
            }
            b'\r' => {
                // Carriage return: reset column only
                self.cursor_x = 0;
            }
            0x08 => {
                // Backspace: move cursor left (don't erase)
                if self.cursor_x > 0 {
                    self.cursor_x -= 1;
                }
            }
            b' '..=b'~' => {
                // Printable character
                self.write_char(byte);
                self.cursor_x += 1;

                // Line wrapping: if we exceeded width, wrap to next line
                if self.cursor_x >= self.cols {
                    self.cursor_x = 0;
                    self.advance_row();
                }
            }
            _ => {
                // Ignore other control characters
            }
        }
    }

    fn process_escape_start(&mut self, byte: u8) {
        match byte {
            b'[' => {
                // CSI (Control Sequence Introducer)
                self.escape_state = EscapeState::CsiStart;
            }
            _ => {
                // Unknown escape sequence, return to normal
                self.escape_state = EscapeState::Normal;
            }
        }
    }

    fn process_csi_start(&mut self, byte: u8) {
        match byte {
            b'0'..=b'9' => {
                // Start of parameter digits
                let mut digits = [0u8; 4];
                digits[0] = byte - b'0';
                self.escape_state = EscapeState::CsiParam { digits, digit_count: 1 };
            }
            b'H' => {
                // Cursor home (no parameters) - go to 0,0
                self.cursor_x = 0;
                self.cursor_y = 0;
                self.escape_state = EscapeState::Normal;
            }
            b'J' => {
                // Clear from cursor to end of display
                self.clear_from_cursor();
                self.escape_state = EscapeState::Normal;
            }
            b'm' => {
                // SGR with no parameters - reset attributes
                self.reset_attributes();
                self.escape_state = EscapeState::Normal;
            }
            _ => {
                // Unknown sequence
                self.escape_state = EscapeState::Normal;
            }
        }
    }

    fn process_csi_param(&mut self, byte: u8) {
        if let EscapeState::CsiParam { mut digits, mut digit_count } = self.escape_state {
            match byte {
                b'0'..=b'9' => {
                    // Accumulate digits
                    if digit_count < 4 {
                        digits[digit_count as usize] = byte - b'0';
                        digit_count += 1;
                        self.escape_state = EscapeState::CsiParam { digits, digit_count };
                    }
                }
                b'J' => {
                    // Reconstruct number from digits
                    let param = digits[0] as usize * 10 + digits[1] as usize;
                    if param == 2 {
                        // Clear screen
                        self.clear_screen();
                    } else {
                        // Clear from cursor to end
                        self.clear_from_cursor();
                    }
                    self.escape_state = EscapeState::Normal;
                }
                b'H' => {
                    // Move cursor (for now, treat as home)
                    self.cursor_x = 0;
                    self.cursor_y = 0;
                    self.escape_state = EscapeState::Normal;
                }
                b'm' => {
                    // SGR - select graphic rendition
                    let param = digits[0] as u8;
                    match param {
                        0 => self.reset_attributes(),
                        1 => self.bold = true,
                        30..=37 => self.fg_color = param - 30,
                        40..=47 => self.bg_color = param - 40,
                        _ => {}
                    }
                    self.escape_state = EscapeState::Normal;
                }
                _ => {
                    // End of sequence
                    self.escape_state = EscapeState::Normal;
                }
            }
        }
    }

    /// Write a character at the current cursor position
    fn write_char(&mut self, ch: u8) {
        if self.cursor_y >= self.rows {
            return;
        }

        let idx = self.cursor_y * self.cols + self.cursor_x;
        if idx < self.cells.len() {
            self.cells[idx] = Cell {
                ch,
                fg: self.fg_color,
                bg: self.bg_color,
                bold: self.bold,
            };
            self.dirty = true;
        }
    }

    /// Advance to next row, scrolling if necessary
    fn advance_row(&mut self) {
        self.cursor_y += 1;
        if self.cursor_y >= self.rows {
            // Scroll: move rows up
            self.scroll_up();
            self.cursor_y = self.rows - 1;
        }
    }

    /// Scroll up one line: save top row to scrollback, shift rows up
    fn scroll_up(&mut self) {
        if self.cells.len() < self.cols {
            return;
        }

        // Save top row to scrollback
        let mut top_row = Vec::with_capacity(self.cols);
        top_row.extend_from_slice(&self.cells[..self.cols]);

        if self.scrollback.len() >= SCROLLBACK_MAX {
            self.scrollback.remove(0);
        }
        self.scrollback.push(top_row);

        // Shift rows up (collect source before borrowing destination)
        for i in 0..(self.rows - 1) {
            let src_start = (i + 1) * self.cols;
            let src_end = src_start + self.cols;
            let dst_start = i * self.cols;
            let dst_end = dst_start + self.cols;

            if src_end <= self.cells.len() && dst_end <= self.cells.len() {
                // Copy into a temp vec to avoid borrow checker issues
                let temp: Vec<Cell> = self.cells[src_start..src_end].to_vec();
                self.cells[dst_start..dst_end].copy_from_slice(&temp);
            }
        }

        // Clear bottom row
        let bottom_start = (self.rows - 1) * self.cols;
        for i in bottom_start..bottom_start + self.cols {
            if i < self.cells.len() {
                self.cells[i] = Cell::blank();
            }
        }

        self.dirty = true;
    }

    /// Clear the entire screen
    fn clear_screen(&mut self) {
        for cell in &mut self.cells {
            *cell = Cell::blank();
        }
        self.cursor_x = 0;
        self.cursor_y = 0;
        self.dirty = true;
    }

    /// Clear from cursor to end of display
    fn clear_from_cursor(&mut self) {
        if self.cursor_y >= self.rows {
            return;
        }

        // Clear from cursor to end of current line
        let row_start = self.cursor_y * self.cols;
        for i in (row_start + self.cursor_x)..(row_start + self.cols) {
            if i < self.cells.len() {
                self.cells[i] = Cell::blank();
            }
        }

        // Clear all subsequent rows
        for i in ((self.cursor_y + 1) * self.cols)..self.cells.len() {
            self.cells[i] = Cell::blank();
        }

        self.dirty = true;
    }

    /// Reset text attributes to defaults
    fn reset_attributes(&mut self) {
        self.fg_color = 7;
        self.bg_color = 0;
        self.bold = false;
    }

    /// Get cursor position
    pub fn cursor(&self) -> (usize, usize) {
        (self.cursor_y, self.cursor_x)
    }

    /// Get scrollback line count
    pub fn scrollback_len(&self) -> usize {
        self.scrollback.len()
    }

    /// Set scroll offset (0 = live view, 1..N = history lines)
    pub fn set_scroll_offset(&mut self, offset: usize) {
        self.scroll_offset = offset.min(self.scrollback.len());
        self.dirty = true;
    }

    /// Scroll up (increase offset, view older history)
    pub fn scroll_up_viewport(&mut self) {
        if self.scroll_offset < self.scrollback.len() {
            self.scroll_offset += 1;
            self.dirty = true;
        }
    }

    /// Scroll down (decrease offset, view newer history toward live)
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

    /// Get current scroll offset
    pub fn get_scroll_offset(&self) -> usize {
        self.scroll_offset
    }

    /// Get terminal dimensions
    pub fn dims(&self) -> (usize, usize) {
        (self.cols, self.rows)
    }

    /// Clear the dirty flag (called after rendering)
    pub fn clear_dirty(&mut self) {
        self.dirty = false;
    }

    /// Convert to TermCell array for rendering (no scrollback offset)
    pub fn to_term_cells(&self, ansi_colors: &[u32; 16]) -> Vec<TermCell> {
        let mut result = Vec::with_capacity(self.cells.len());
        for cell in &self.cells {
            let fg_idx = if cell.bold && cell.fg < 8 {
                cell.fg + 8
            } else {
                cell.fg
            };
            let fg = ansi_colors[fg_idx as usize % 16];
            let bg = ansi_colors[cell.bg as usize % 16];

            result.push(TermCell {
                ch: cell.ch,
                fg,
                bg,
            });
        }
        result
    }

    /// Convert with viewport offset for scrollback viewing
    /// viewport_offset: 0 = live view, 1..N = viewing history
    pub fn to_term_cells_with_offset(&self, ansi_colors: &[u32; 16], viewport_offset: usize) -> Vec<TermCell> {
        if viewport_offset == 0 {
            return self.to_term_cells(ansi_colors);
        }

        let mut result = Vec::new();

        // For each screen row, render from scrollback history or live cells
        for screen_row in 0..self.rows {
            let history_row_idx = if self.scrollback.len() > viewport_offset {
                self.scrollback.len() - viewport_offset + screen_row
            } else {
                screen_row
            };

            let cells_to_render = if history_row_idx < self.scrollback.len() {
                &self.scrollback[history_row_idx]
            } else if screen_row < self.rows {
                // Fallback to live cells if out of scrollback range
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

            // Pad row if needed
            while result.len() % self.cols != 0 {
                result.push(TermCell { ch: b' ', fg: ansi_colors[7], bg: ansi_colors[0] });
            }
        }

        result
    }
}
