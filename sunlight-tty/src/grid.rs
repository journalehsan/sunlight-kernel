//! 2D character-grid terminal emulator with VT100/ANSI escape support.
//!
//! Maintains a (cols x rows) grid of styled characters, plus scrollback history.
//! Feeds bytes through the Vt100Parser, interprets output events, and updates grid state.

use crate::vt100::{Vt100Parser, VtOutput};
use alloc::vec::Vec;
// Re-export TermCell from sunlight_tui for use here
pub use sunlight_tui::TermCell;

/// Maximum scrollback lines retained (oldest pushed out beyond this).
const SCROLLBACK_LINES: usize = 64;

/// A single terminal cell: character + foreground/background colors + bold flag.
#[derive(Clone, Copy, Debug)]
pub struct Cell {
    pub ch: u8,
    pub fg: u8,  // ANSI palette index 0-15
    pub bg: u8,
    pub bold: bool,
}

impl Cell {
    const fn blank() -> Self {
        Cell { ch: b' ', fg: 7, bg: 0, bold: false }
    }
}

/// 2D character grid terminal with cursor, scrollback, and ANSI parsing.
pub struct TerminalGrid {
    pub cols: usize,
    pub rows: usize,

    // Current screen cells (row-major order: row 0 col 0..cols, row 1 col 0..cols, etc.)
    cells: Vec<Cell>,

    // Scrollback history: each element is a complete row (cols cells)
    scrollback: Vec<Vec<Cell>>,

    // Cursor position
    cursor_row: usize,
    cursor_col: usize,

    // Current text attributes
    cur_fg: u8,
    cur_bg: u8,
    cur_bold: bool,

    // VT100 escape sequence parser
    parser: Vt100Parser,
}

impl TerminalGrid {
    /// Create a new terminal grid with given dimensions.
    /// Allocates from the global allocator (must be available in no_std context).
    pub fn new(cols: usize, rows: usize) -> Self {
        let mut cells = Vec::new();
        cells.resize(cols * rows, Cell::blank());

        Self {
            cols,
            rows,
            cells,
            scrollback: Vec::new(),
            cursor_row: 0,
            cursor_col: 0,
            cur_fg: 7,  // default white
            cur_bg: 0,  // default black
            cur_bold: false,
            parser: Vt100Parser::new(),
        }
    }

    /// Feed raw bytes into the terminal, updating grid state.
    /// Each byte is parsed as a potential ANSI escape sequence.
    pub fn feed(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            let output = self.parser.feed(byte);
            self.handle_output(output);
        }
    }

    /// Handle a single parsed VtOutput event.
    fn handle_output(&mut self, output: VtOutput) {
        match output {
            VtOutput::Char(ch) => self.write_char(ch),
            VtOutput::Newline => self.newline(),
            VtOutput::CarriageReturn => self.carriage_return(),
            VtOutput::Backspace => self.backspace(),
            VtOutput::SetCursor { row, col } => self.set_cursor(row as usize, col as usize),
            VtOutput::MoveCursor { row, col } => self.move_cursor(row, col),
            VtOutput::ClearScreen => self.clear_screen(),
            VtOutput::ClearLine => self.clear_line(),
            VtOutput::SetColor { fg, bg } => self.set_color(fg, bg),
            VtOutput::ResetAttrs => self.reset_attrs(),
            VtOutput::Bold(b) => self.cur_bold = b,
            VtOutput::Bell | VtOutput::Nothing => {},
        }
    }

    /// Write a character at the current cursor position, advancing the cursor.
    fn write_char(&mut self, ch: u8) {
        if self.cursor_row >= self.rows || self.cursor_col >= self.cols {
            return;
        }

        let idx = self.cursor_row * self.cols + self.cursor_col;
        if idx < self.cells.len() {
            self.cells[idx] = Cell {
                ch,
                fg: self.cur_fg,
                bg: self.cur_bg,
                bold: self.cur_bold,
            };
        }

        self.cursor_col += 1;

        // Wrap to next line if we've overflowed
        if self.cursor_col >= self.cols {
            self.cursor_col = 0;
            self.cursor_row += 1;

            // Scroll if we've moved past the bottom
            if self.cursor_row >= self.rows {
                self.scroll_up();
            }
        }
    }

    /// Move cursor to the next line, scrolling if necessary.
    fn newline(&mut self) {
        self.cursor_row += 1;
        if self.cursor_row >= self.rows {
            self.scroll_up();
        }
    }

    /// Move cursor to the start of the current line.
    fn carriage_return(&mut self) {
        self.cursor_col = 0;
    }

    /// Move cursor back one position (if not at start of line).
    fn backspace(&mut self) {
        self.cursor_col = self.cursor_col.saturating_sub(1);
    }

    /// Move cursor to an absolute position (clamped to grid bounds).
    fn set_cursor(&mut self, row: usize, col: usize) {
        self.cursor_row = row.min(self.rows.saturating_sub(1));
        self.cursor_col = col.min(self.cols.saturating_sub(1));
    }

    /// Move cursor by a relative offset (clamped to grid bounds).
    fn move_cursor(&mut self, drow: i16, dcol: i16) {
        let new_row = (self.cursor_row as i16 + drow).max(0) as usize;
        let new_col = (self.cursor_col as i16 + dcol).max(0) as usize;
        self.set_cursor(new_row, new_col);
    }

    /// Clear the entire screen, reset cursor to origin.
    pub fn clear_screen(&mut self) {
        for cell in &mut self.cells {
            *cell = Cell::blank();
        }
        self.cursor_row = 0;
        self.cursor_col = 0;
    }

    /// Clear from cursor to end of line.
    fn clear_line(&mut self) {
        if self.cursor_row >= self.rows {
            return;
        }
        let row_start = self.cursor_row * self.cols;
        for col in self.cursor_col..self.cols {
            let idx = row_start + col;
            if idx < self.cells.len() {
                self.cells[idx] = Cell::blank();
            }
        }
    }

    /// Set foreground and/or background color (palette indices).
    fn set_color(&mut self, fg: Option<u8>, bg: Option<u8>) {
        if let Some(f) = fg {
            self.cur_fg = f.min(15);
        }
        if let Some(b) = bg {
            self.cur_bg = b.min(15);
        }
    }

    /// Reset text attributes to defaults.
    fn reset_attrs(&mut self) {
        self.cur_fg = 7;
        self.cur_bg = 0;
        self.cur_bold = false;
    }

    /// Scroll the grid up by one line: push current top row to scrollback,
    /// shift all rows up, clear the new bottom row, and move cursor back.
    fn scroll_up(&mut self) {
        // Push top row to scrollback (cap at SCROLLBACK_LINES)
        if self.scrollback.len() >= SCROLLBACK_LINES {
            self.scrollback.remove(0);
        }
        let mut top_row = Vec::with_capacity(self.cols);
        for i in 0..self.cols {
            top_row.push(self.cells[i]);
        }
        self.scrollback.push(top_row);

        // Shift rows up
        for row in 0..self.rows.saturating_sub(1) {
            let src_start = (row + 1) * self.cols;
            let dst_start = row * self.cols;
            for col in 0..self.cols {
                self.cells[dst_start + col] = self.cells[src_start + col];
            }
        }

        // Clear bottom row
        let bottom_start = (self.rows.saturating_sub(1)) * self.cols;
        for i in 0..self.cols {
            self.cells[bottom_start + i] = Cell::blank();
        }

        // Move cursor back (if not at top)
        if self.cursor_row > 0 {
            self.cursor_row -= 1;
        }
    }

    /// Get the cursor position (row, col).
    pub fn cursor(&self) -> (usize, usize) {
        (self.cursor_row, self.cursor_col)
    }

    /// Get a cell at the given position.
    pub fn cell(&self, row: usize, col: usize) -> Cell {
        if row >= self.rows || col >= self.cols {
            return Cell::blank();
        }
        self.cells[row * self.cols + col]
    }

    /// Convert the entire grid to a flat Vec with RGB colors resolved.
    /// Folds `bold` into the palette index (bright variants: if bold, add 8 to indices 0-7).
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

            result.push(TermCell { ch: cell.ch, fg, bg });
        }
        result
    }

    /// Get the number of lines in scrollback history.
    pub fn scrollback_len(&self) -> usize {
        self.scrollback.len()
    }

    /// Convert grid to term cells with viewport offset for scrollback viewing.
    /// viewport_offset: 0 = live view, 1..scrollback_len() = viewing history
    /// For each screen row, either pull from scrollback (if offset > 0) or live cells.
    pub fn to_term_cells_with_offset(&self, ansi_colors: &[u32; 16], viewport_offset: usize) -> Vec<TermCell> {
        let mut result = Vec::new();

        if viewport_offset == 0 {
            // Live view: return normal cells
            return self.to_term_cells(ansi_colors);
        }

        // Scrollback view: render from history
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
