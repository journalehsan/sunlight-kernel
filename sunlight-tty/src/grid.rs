//! 2D character-grid terminal emulator with VT100/ANSI escape support.
//!
//! Maintains a `(cols x rows)` grid of styled characters, plus scrollback
//! history for the normal screen. Feeds bytes through the VT parser,
//! interprets output events, and updates screen state.

use crate::vt100::{Vt100Parser, VtOutput, MAX_PARAMS};
use alloc::vec::Vec;
pub use sunlight_tui::{paint_terminal_symbol, TermCell};

const SCROLLBACK_LINES: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    pub ch: char,
    fg: CellColor,
    bg: CellColor,
    pub bold: bool,
    pub inverse: bool,
    pub underline: bool,
}

impl Cell {
    const fn blank() -> Self {
        Self {
            ch: ' ',
            fg: CellColor::Ansi(7),
            bg: CellColor::Ansi(0),
            bold: false,
            inverse: false,
            underline: false,
        }
    }

    pub fn colors(self, palette: &[u32; 16], cursor: bool) -> (u32, u32) {
        let fg = self.fg.resolve(palette, self.bold);
        let bg = self.bg.resolve(palette, false);
        if self.inverse ^ cursor {
            (bg, fg)
        } else {
            (fg, bg)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CellColor {
    Ansi(u8),
    Rgb(u32),
}

impl CellColor {
    fn resolve(self, palette: &[u32; 16], bold: bool) -> u32 {
        match self {
            Self::Ansi(index) => {
                palette[usize::from(if bold && index < 8 { index + 8 } else { index })]
            }
            Self::Rgb(rgb) => rgb,
        }
    }
}

pub struct TerminalGrid {
    pub cols: usize,
    pub rows: usize,
    main_cells: Vec<Cell>,
    alt_cells: Vec<Cell>,
    scrollback: Vec<Cell>,
    scrollback_head: usize,
    scrollback_count: usize,
    term_cells: Vec<TermCell>,
    main_cursor_row: usize,
    main_cursor_col: usize,
    alt_cursor_row: usize,
    alt_cursor_col: usize,
    main_wrap_pending: bool,
    alt_wrap_pending: bool,
    saved_cursor: Option<(usize, usize)>,
    saved_main_cursor: (usize, usize),
    cur_fg: CellColor,
    cur_bg: CellColor,
    cur_bold: bool,
    cur_inverse: bool,
    cur_underline: bool,
    cursor_visible: bool,
    use_alt_screen: bool,
    parser: Vt100Parser,
    utf8: [u8; 4],
    utf8_len: usize,
    utf8_width: usize,
}

impl TerminalGrid {
    pub fn new(cols: usize, rows: usize) -> Self {
        let mut main_cells = Vec::new();
        main_cells.resize(cols * rows, Cell::blank());

        let mut alt_cells = Vec::new();
        alt_cells.resize(cols * rows, Cell::blank());

        let mut scrollback = Vec::new();
        scrollback.resize(SCROLLBACK_LINES * cols, Cell::blank());

        let mut term_cells = Vec::new();
        term_cells.resize(
            cols * rows,
            TermCell {
                ch: ' ',
                fg: 0,
                bg: 0,
            },
        );

        Self {
            cols,
            rows,
            main_cells,
            alt_cells,
            scrollback,
            scrollback_head: 0,
            scrollback_count: 0,
            term_cells,
            main_cursor_row: 0,
            main_cursor_col: 0,
            alt_cursor_row: 0,
            alt_cursor_col: 0,
            main_wrap_pending: false,
            alt_wrap_pending: false,
            saved_cursor: None,
            saved_main_cursor: (0, 0),
            cur_fg: CellColor::Ansi(7),
            cur_bg: CellColor::Ansi(0),
            cur_bold: false,
            cur_inverse: false,
            cur_underline: false,
            cursor_visible: true,
            use_alt_screen: false,
            parser: Vt100Parser::new(),
            utf8: [0; 4],
            utf8_len: 0,
            utf8_width: 0,
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            if self.utf8_len != 0 {
                if byte & 0xc0 == 0x80 {
                    self.utf8[self.utf8_len] = byte;
                    self.utf8_len += 1;
                    if self.utf8_len == self.utf8_width {
                        let ch = core::str::from_utf8(&self.utf8[..self.utf8_len])
                            .ok()
                            .and_then(|text| text.chars().next())
                            .unwrap_or(char::REPLACEMENT_CHARACTER);
                        self.utf8_len = 0;
                        self.write_char(ch);
                    }
                    continue;
                }
                self.utf8_len = 0;
                self.write_char(char::REPLACEMENT_CHARACTER);
            }
            let output = self.parser.feed(byte);
            self.handle_output(output);
        }
    }

    /// Resize the renderer/model grid while preserving the overlapping visible
    /// cells. Partial old rows are never reinterpreted with the new stride.
    /// Scrollback is cleared because its row width is part of its storage
    /// contract; retaining differently-strided history would corrupt the
    /// viewport. Returns `false` for zero/overflowing geometry or allocation
    /// failure, leaving the existing grid untouched.
    pub fn resize(&mut self, cols: usize, rows: usize) -> bool {
        if cols == 0 || rows == 0 || (cols == self.cols && rows == self.rows) {
            return cols == self.cols && rows == self.rows;
        }
        let Some(cell_count) = cols.checked_mul(rows) else {
            return false;
        };
        let Some(scrollback_count) = SCROLLBACK_LINES.checked_mul(cols) else {
            return false;
        };

        let mut main_cells = Vec::new();
        let mut alt_cells = Vec::new();
        let mut scrollback = Vec::new();
        let mut term_cells = Vec::new();
        if main_cells.try_reserve_exact(cell_count).is_err()
            || alt_cells.try_reserve_exact(cell_count).is_err()
            || scrollback.try_reserve_exact(scrollback_count).is_err()
            || term_cells.try_reserve_exact(cell_count).is_err()
        {
            return false;
        }
        main_cells.resize(cell_count, Cell::blank());
        alt_cells.resize(cell_count, Cell::blank());
        scrollback.resize(scrollback_count, Cell::blank());
        term_cells.resize(
            cell_count,
            TermCell {
                ch: ' ',
                fg: 0,
                bg: 0,
            },
        );

        let copy_rows = rows.min(self.rows);
        let copy_cols = cols.min(self.cols);
        for row in 0..copy_rows {
            let old_start = row * self.cols;
            let new_start = row * cols;
            main_cells[new_start..new_start + copy_cols]
                .copy_from_slice(&self.main_cells[old_start..old_start + copy_cols]);
            alt_cells[new_start..new_start + copy_cols]
                .copy_from_slice(&self.alt_cells[old_start..old_start + copy_cols]);
        }

        self.cols = cols;
        self.rows = rows;
        self.main_cells = main_cells;
        self.alt_cells = alt_cells;
        self.scrollback = scrollback;
        self.scrollback_head = 0;
        self.scrollback_count = 0;
        self.term_cells = term_cells;
        self.main_wrap_pending = false;
        self.alt_wrap_pending = false;
        self.main_cursor_row = self.main_cursor_row.min(rows - 1);
        self.main_cursor_col = self.main_cursor_col.min(cols - 1);
        self.alt_cursor_row = self.alt_cursor_row.min(rows - 1);
        self.alt_cursor_col = self.alt_cursor_col.min(cols - 1);
        self.saved_main_cursor.0 = self.saved_main_cursor.0.min(rows - 1);
        self.saved_main_cursor.1 = self.saved_main_cursor.1.min(cols - 1);
        if let Some((row, col)) = self.saved_cursor {
            self.saved_cursor = Some((row.min(rows - 1), col.min(cols - 1)));
        }
        true
    }

    fn handle_output(&mut self, output: VtOutput) {
        match output {
            VtOutput::Char(byte) => {
                self.utf8_width = match byte {
                    0x20..=0x7e => {
                        self.write_char(char::from(byte));
                        return;
                    }
                    0xc2..=0xdf => 2,
                    0xe0..=0xef => 3,
                    0xf0..=0xf4 => 4,
                    0x80..=0xff => {
                        self.write_char(char::REPLACEMENT_CHARACTER);
                        return;
                    }
                    _ => return,
                };
                self.utf8[0] = byte;
                self.utf8_len = 1;
            }
            VtOutput::MoveCursor { row, col } => self.move_cursor(row, col),
            VtOutput::SetCursor { row, col } => self.set_cursor(row as usize, col as usize),
            VtOutput::ClearScreen { mode } => self.clear_screen_mode(mode),
            VtOutput::ClearLine { mode } => self.clear_line_mode(mode),
            VtOutput::Sgr { params, count } => self.apply_sgr(&params, count),
            VtOutput::DecPrivateMode { mode, enabled } => self.set_private_mode(mode, enabled),
            VtOutput::SaveCursor => self.saved_cursor = Some(self.cursor()),
            VtOutput::RestoreCursor => {
                if let Some((row, col)) = self.saved_cursor {
                    self.set_cursor(row, col);
                }
            }
            VtOutput::CarriageReturn => self.carriage_return(),
            VtOutput::Newline => self.newline(),
            VtOutput::Backspace => self.backspace(),
            VtOutput::Tab => self.tab(),
            VtOutput::Bell | VtOutput::Nothing => {}
        }
    }

    fn active_cells(&self) -> &[Cell] {
        if self.use_alt_screen {
            &self.alt_cells
        } else {
            &self.main_cells
        }
    }

    fn active_cells_mut(&mut self) -> &mut [Cell] {
        if self.use_alt_screen {
            &mut self.alt_cells
        } else {
            &mut self.main_cells
        }
    }

    fn cursor_mut(&mut self) -> (&mut usize, &mut usize) {
        if self.use_alt_screen {
            (&mut self.alt_cursor_row, &mut self.alt_cursor_col)
        } else {
            (&mut self.main_cursor_row, &mut self.main_cursor_col)
        }
    }

    fn write_char(&mut self, ch: char) {
        let wrap_pending = if self.use_alt_screen {
            self.alt_wrap_pending
        } else {
            self.main_wrap_pending
        };
        if wrap_pending {
            self.newline();
        }
        let (row, col) = self.cursor();
        let cell = Cell {
            ch,
            fg: self.cur_fg,
            bg: self.cur_bg,
            bold: self.cur_bold,
            inverse: self.cur_inverse,
            underline: self.cur_underline,
        };
        if row >= self.rows || col >= self.cols {
            return;
        }
        let idx = row * self.cols + col;
        if let Some(slot) = self.active_cells_mut().get_mut(idx) {
            *slot = cell;
        }

        let cols = self.cols;
        let (_, cursor_col) = self.cursor_mut();
        if *cursor_col + 1 < cols {
            *cursor_col += 1;
        } else if self.use_alt_screen {
            self.alt_wrap_pending = true;
        } else {
            self.main_wrap_pending = true;
        }
    }

    fn cancel_wrap(&mut self) {
        if self.use_alt_screen {
            self.alt_wrap_pending = false;
        } else {
            self.main_wrap_pending = false;
        }
    }

    fn newline(&mut self) {
        self.cancel_wrap();
        let (cursor_row, cursor_col) = self.cursor_mut();
        *cursor_col = 0;
        *cursor_row += 1;
        if *cursor_row >= self.rows {
            self.scroll_up();
        }
    }

    fn carriage_return(&mut self) {
        self.cancel_wrap();
        let (_, cursor_col) = self.cursor_mut();
        *cursor_col = 0;
    }

    fn backspace(&mut self) {
        self.cancel_wrap();
        let (_, cursor_col) = self.cursor_mut();
        *cursor_col = cursor_col.saturating_sub(1);
    }

    fn tab(&mut self) {
        self.cancel_wrap();
        let cols = self.cols;
        let (_, cursor_col) = self.cursor_mut();
        let next = ((*cursor_col / 8) + 1) * 8;
        *cursor_col = next.min(cols.saturating_sub(1));
    }

    fn set_cursor(&mut self, row: usize, col: usize) {
        self.cancel_wrap();
        let rows = self.rows;
        let cols = self.cols;
        let (cursor_row, cursor_col) = self.cursor_mut();
        *cursor_row = row.min(rows.saturating_sub(1));
        *cursor_col = col.min(cols.saturating_sub(1));
    }

    fn move_cursor(&mut self, drow: i16, dcol: i16) {
        let (row, col) = self.cursor();
        let new_row = (row as i16 + drow).max(0) as usize;
        let new_col = (col as i16 + dcol).max(0) as usize;
        self.set_cursor(new_row, new_col);
    }

    pub fn clear_screen(&mut self) {
        for cell in &mut self.main_cells {
            *cell = Cell::blank();
        }
        for cell in &mut self.alt_cells {
            *cell = Cell::blank();
        }
        self.main_cursor_row = 0;
        self.main_cursor_col = 0;
        self.alt_cursor_row = 0;
        self.alt_cursor_col = 0;
        self.main_wrap_pending = false;
        self.alt_wrap_pending = false;
        self.scrollback_head = 0;
        self.scrollback_count = 0;
        self.saved_cursor = None;
        self.saved_main_cursor = (0, 0);
        self.cursor_visible = true;
        self.use_alt_screen = false;
        self.reset_attrs();
        self.parser = Vt100Parser::new();
        self.utf8_len = 0;
    }

    fn clear_screen_mode(&mut self, mode: u16) {
        match mode {
            0 => self.clear_from_cursor_to_screen_end(),
            1 => self.clear_from_screen_start_to_cursor(),
            2 | 3 => self.clear_active_screen(),
            _ => self.clear_active_screen(),
        }
    }

    fn clear_active_screen(&mut self) {
        for cell in self.active_cells_mut() {
            *cell = Cell::blank();
        }
        self.set_cursor(0, 0);
    }

    fn clear_from_cursor_to_screen_end(&mut self) {
        let (row, col) = self.cursor();
        let rows = self.rows;
        let cols = self.cols;
        let cells = self.active_cells_mut();
        for r in row..rows {
            let start_col = if r == row { col } else { 0 };
            let start = r * cols + start_col;
            let end = (r + 1) * cols;
            for idx in start..end {
                cells[idx] = Cell::blank();
            }
        }
    }

    fn clear_from_screen_start_to_cursor(&mut self) {
        let (row, col) = self.cursor();
        let rows = self.rows;
        let cols = self.cols;
        let cells = self.active_cells_mut();
        for r in 0..=row.min(rows.saturating_sub(1)) {
            let end_col = if r == row { col + 1 } else { cols };
            let start = r * cols;
            let end = start + end_col.min(cols);
            for idx in start..end {
                cells[idx] = Cell::blank();
            }
        }
    }

    fn clear_line_mode(&mut self, mode: u16) {
        match mode {
            0 => self.clear_line_right(),
            1 => self.clear_line_left(),
            2 => self.clear_line_all(),
            _ => self.clear_line_right(),
        }
    }

    fn clear_line_right(&mut self) {
        let (row, col) = self.cursor();
        if row >= self.rows {
            return;
        }
        let start = row * self.cols + col.min(self.cols);
        let end = (row + 1) * self.cols;
        for idx in start..end {
            self.active_cells_mut()[idx] = Cell::blank();
        }
    }

    fn clear_line_left(&mut self) {
        let (row, col) = self.cursor();
        if row >= self.rows {
            return;
        }
        let start = row * self.cols;
        let end = start + (col + 1).min(self.cols);
        for idx in start..end {
            self.active_cells_mut()[idx] = Cell::blank();
        }
    }

    fn clear_line_all(&mut self) {
        let (row, _) = self.cursor();
        if row >= self.rows {
            return;
        }
        let start = row * self.cols;
        let end = start + self.cols;
        for idx in start..end {
            self.active_cells_mut()[idx] = Cell::blank();
        }
    }

    fn apply_sgr(&mut self, params: &[u16; MAX_PARAMS], count: usize) {
        if count == 0 {
            self.reset_attrs();
            return;
        }

        let mut i = 0usize;
        while i < count {
            match params[i] {
                0 => self.reset_attrs(),
                1 => self.cur_bold = true,
                4 => self.cur_underline = true,
                7 => self.cur_inverse = true,
                22 => self.cur_bold = false,
                24 => self.cur_underline = false,
                27 => self.cur_inverse = false,
                30..=37 => self.cur_fg = CellColor::Ansi((params[i] - 30) as u8),
                39 => self.cur_fg = CellColor::Ansi(7),
                40..=47 => self.cur_bg = CellColor::Ansi((params[i] - 40) as u8),
                49 => self.cur_bg = CellColor::Ansi(0),
                90..=97 => self.cur_fg = CellColor::Ansi((params[i] - 90 + 8) as u8),
                100..=107 => self.cur_bg = CellColor::Ansi((params[i] - 100 + 8) as u8),
                38 => {
                    if i + 2 < count && params[i + 1] == 5 {
                        self.cur_fg = map_extended_color(params[i + 2]);
                        i += 2;
                    } else if i + 4 < count && params[i + 1] == 2 {
                        self.cur_fg = rgb(params[i + 2], params[i + 3], params[i + 4]);
                        i += 4;
                    }
                }
                48 => {
                    if i + 2 < count && params[i + 1] == 5 {
                        self.cur_bg = map_extended_color(params[i + 2]);
                        i += 2;
                    } else if i + 4 < count && params[i + 1] == 2 {
                        self.cur_bg = rgb(params[i + 2], params[i + 3], params[i + 4]);
                        i += 4;
                    }
                }
                _ => {}
            }
            i += 1;
        }
    }

    fn set_private_mode(&mut self, mode: u16, enabled: bool) {
        match mode {
            25 => self.cursor_visible = enabled,
            47 | 1047 | 1049 => {
                if enabled {
                    self.enter_alt_screen();
                } else {
                    self.exit_alt_screen();
                }
            }
            _ => {}
        }
    }

    fn enter_alt_screen(&mut self) {
        if self.use_alt_screen {
            return;
        }
        self.saved_main_cursor = (self.main_cursor_row, self.main_cursor_col);
        self.use_alt_screen = true;
        self.alt_wrap_pending = false;
        self.alt_cursor_row = 0;
        self.alt_cursor_col = 0;
        for cell in &mut self.alt_cells {
            *cell = Cell::blank();
        }
    }

    fn exit_alt_screen(&mut self) {
        if !self.use_alt_screen {
            return;
        }
        self.use_alt_screen = false;
        self.main_cursor_row = self.saved_main_cursor.0.min(self.rows.saturating_sub(1));
        self.main_cursor_col = self.saved_main_cursor.1.min(self.cols.saturating_sub(1));
    }

    fn reset_attrs(&mut self) {
        self.cur_fg = CellColor::Ansi(7);
        self.cur_bg = CellColor::Ansi(0);
        self.cur_bold = false;
        self.cur_inverse = false;
        self.cur_underline = false;
    }

    fn scroll_up(&mut self) {
        let cells = if self.use_alt_screen {
            &mut self.alt_cells
        } else {
            &mut self.main_cells
        };

        if !self.use_alt_screen {
            let slot = if self.scrollback_count == SCROLLBACK_LINES {
                let oldest = self.scrollback_head;
                self.scrollback_head = (self.scrollback_head + 1) % SCROLLBACK_LINES;
                oldest
            } else {
                let next = (self.scrollback_head + self.scrollback_count) % SCROLLBACK_LINES;
                self.scrollback_count += 1;
                next
            };
            let dst = slot * self.cols;
            for i in 0..self.cols {
                self.scrollback[dst + i] = cells[i];
            }
        }

        for row in 0..self.rows.saturating_sub(1) {
            let src_start = (row + 1) * self.cols;
            let dst_start = row * self.cols;
            for col in 0..self.cols {
                cells[dst_start + col] = cells[src_start + col];
            }
        }

        let bottom_start = self.rows.saturating_sub(1) * self.cols;
        for i in 0..self.cols {
            cells[bottom_start + i] = Cell::blank();
        }

        let (cursor_row, _) = self.cursor_mut();
        if *cursor_row > 0 {
            *cursor_row -= 1;
        }
    }

    pub fn cursor(&self) -> (usize, usize) {
        if self.use_alt_screen {
            (self.alt_cursor_row, self.alt_cursor_col)
        } else {
            (self.main_cursor_row, self.main_cursor_col)
        }
    }

    pub fn cursor_visible(&self) -> bool {
        self.cursor_visible
    }

    pub fn in_alt_screen(&self) -> bool {
        self.use_alt_screen
    }

    pub fn scrollback_len(&self) -> usize {
        self.scrollback_count
    }

    pub fn cell(&self, row: usize, col: usize) -> Cell {
        let cells = self.active_cells();
        if row >= self.rows || col >= self.cols {
            return Cell::blank();
        }
        cells[row * self.cols + col]
    }

    pub fn to_term_cells(&mut self, ansi_colors: &[u32; 16]) -> &[TermCell] {
        let len = self.term_cells.len();
        for idx in 0..len {
            self.term_cells[idx] = resolve_cell(self.active_cells()[idx], ansi_colors);
        }
        &self.term_cells
    }

    pub fn viewport_cell(&self, row: usize, col: usize, offset: usize) -> Cell {
        if row >= self.rows || col >= self.cols {
            return Cell::blank();
        }
        if self.use_alt_screen || offset == 0 {
            return self.cell(row, col);
        }
        let history_row = self.scrollback_count - offset.min(self.scrollback_count) + row;
        if history_row < self.scrollback_count {
            let slot = (self.scrollback_head + history_row) % SCROLLBACK_LINES;
            self.scrollback[slot * self.cols + col]
        } else {
            self.main_cells[(history_row - self.scrollback_count) * self.cols + col]
        }
    }

    pub fn to_term_cells_with_offset(
        &mut self,
        ansi_colors: &[u32; 16],
        viewport_offset: usize,
    ) -> &[TermCell] {
        if viewport_offset == 0 || self.use_alt_screen {
            return self.to_term_cells(ansi_colors);
        }

        for screen_row in 0..self.rows {
            let dst_start = screen_row * self.cols;
            for col in 0..self.cols {
                self.term_cells[dst_start + col] = resolve_cell(
                    self.viewport_cell(screen_row, col, viewport_offset),
                    ansi_colors,
                );
            }
        }

        &self.term_cells
    }
}

fn resolve_cell(cell: Cell, ansi_colors: &[u32; 16]) -> TermCell {
    let (fg, bg) = cell.colors(ansi_colors, false);
    TermCell {
        ch: cell.ch,
        fg,
        bg,
    }
}

fn map_extended_color(color: u16) -> CellColor {
    match color {
        0..=15 => CellColor::Ansi(color as u8),
        16..=231 => {
            let index = color - 16;
            let levels = [0, 95, 135, 175, 215, 255];
            rgb(
                levels[(index / 36) as usize],
                levels[(index / 6 % 6) as usize],
                levels[(index % 6) as usize],
            )
        }
        232..=255 => {
            let gray = 8 + (color - 232) * 10;
            rgb(gray, gray, gray)
        }
        _ => CellColor::Ansi(7),
    }
}

fn rgb(red: u16, green: u16, blue: u16) -> CellColor {
    CellColor::Rgb(
        u32::from(red.min(255)) << 16 | u32::from(green.min(255)) << 8 | u32::from(blue.min(255)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const ANSI_COLORS: [u32; 16] = [
        0x000000, 0xaa0000, 0x00aa00, 0xaa5500, 0x0000aa, 0xaa00aa, 0x00aaaa, 0xaaaaaa, 0x555555,
        0xff5555, 0x55ff55, 0xffff55, 0x5555ff, 0xff55ff, 0x55ffff, 0xffffff,
    ];

    #[test]
    fn plain_text_writes_cells() {
        let mut grid = TerminalGrid::new(4, 2);
        grid.feed(b"ab");
        assert_eq!(grid.cell(0, 0).ch, 'a');
        assert_eq!(grid.cell(0, 1).ch, 'b');
    }

    #[test]
    fn utf8_graphics_keep_cell_alignment_across_reads() {
        let mut grid = TerminalGrid::new(8, 2);
        let bytes = "╭─⣿╮X".as_bytes();
        for byte in bytes {
            grid.feed(core::slice::from_ref(byte));
        }
        for (col, expected) in "╭─⣿╮X".chars().enumerate() {
            assert_eq!(grid.cell(0, col).ch, expected);
        }
        assert_eq!(grid.cursor(), (0, 5));
    }

    #[test]
    fn incomplete_utf8_does_not_consume_escape_sequence() {
        let mut grid = TerminalGrid::new(8, 2);
        grid.feed(b"\xe2\x94");
        grid.feed(b"\x1b[2;3HX");
        assert_eq!(grid.cell(0, 0).ch, char::REPLACEMENT_CHARACTER);
        assert_eq!(grid.cell(1, 2).ch, 'X');
    }

    #[test]
    fn sgr_keeps_rgb_background_and_indexed_foreground() {
        let mut grid = TerminalGrid::new(4, 1);
        grid.feed(b"\x1b[1;38;2;12;34;56;48;2;8;9;10mA\x1b[38;5;67mB\x1b[0mC");
        let cells = grid.to_term_cells(&ANSI_COLORS);
        assert_eq!((cells[0].fg, cells[0].bg), (0x0c2238, 0x08090a));
        assert_eq!((cells[1].fg, cells[1].bg), (0x5f87af, 0x08090a));
        assert_eq!((cells[2].fg, cells[2].bg), (ANSI_COLORS[7], ANSI_COLORS[0]));
    }

    #[test]
    fn newline_and_carriage_return_work() {
        let mut grid = TerminalGrid::new(4, 2);
        grid.feed(b"ab\rZ\nQ");
        assert_eq!(grid.cell(0, 0).ch, 'Z');
        assert_eq!(grid.cell(1, 0).ch, 'Q');
    }

    #[test]
    fn cursor_movement_updates_in_place() {
        let mut grid = TerminalGrid::new(4, 2);
        grid.feed(b"ab\x1b[1D!");
        assert_eq!(grid.cell(0, 0).ch, 'a');
        assert_eq!(grid.cell(0, 1).ch, '!');
    }

    #[test]
    fn clear_screen_resets_cells() {
        let mut grid = TerminalGrid::new(4, 2);
        grid.feed(b"ab\x1b[2J");
        assert_eq!(grid.cell(0, 0).ch, ' ');
        assert_eq!(grid.cursor(), (0, 0));
    }

    #[test]
    fn clear_line_mode_two_clears_current_row() {
        let mut grid = TerminalGrid::new(4, 2);
        grid.feed(b"ab\ncd\x1b[2K");
        assert_eq!(grid.cell(1, 0).ch, ' ');
        assert_eq!(grid.cell(1, 1).ch, ' ');
    }

    #[test]
    fn sgr_color_reset_restores_defaults() {
        let mut grid = TerminalGrid::new(4, 1);
        grid.feed(b"\x1b[31mA\x1b[0mB");
        assert_eq!(grid.cell(0, 0).fg, CellColor::Ansi(1));
        assert_eq!(grid.cell(0, 1).fg, CellColor::Ansi(7));
    }

    #[test]
    fn alternate_screen_enter_exit_restores_main_buffer() {
        let mut grid = TerminalGrid::new(4, 2);
        grid.feed(b"main\x1b[?1049hALT\x1b[?1049l");
        assert_eq!(grid.cell(0, 0).ch, 'm');
        let cells = grid.to_term_cells(&ANSI_COLORS);
        assert_eq!(cells[0].ch, 'm');
    }

    #[test]
    fn resize_like_clear_screen_keeps_parser_sane() {
        let mut grid = TerminalGrid::new(4, 2);
        grid.clear_screen();
        grid.feed(b"\x1b[31mX");
        assert_eq!(grid.cell(0, 0).ch, 'X');
        assert_eq!(grid.cell(0, 0).fg, CellColor::Ansi(1));
    }

    #[test]
    fn resize_preserves_overlap_and_clamps_cursor() {
        let mut grid = TerminalGrid::new(4, 2);
        grid.feed(b"ab");
        assert!(grid.resize(8, 4));
        assert_eq!(grid.cell(0, 0).ch, 'a');
        assert_eq!(grid.cell(0, 1).ch, 'b');
        assert!(grid.resize(2, 1));
        assert_eq!(grid.cell(0, 0).ch, 'a');
        let (row, col) = grid.cursor();
        assert!(row < grid.rows && col < grid.cols);
    }

    #[test]
    fn repeated_resize_transitions_remain_bounded_and_consistent() {
        let mut grid = TerminalGrid::new(80, 25);
        for (cols, rows) in [(170, 50), (40, 25), (80, 80), (12, 6), (170, 50)] {
            assert!(grid.resize(cols, rows));
            assert_eq!((grid.cols, grid.rows), (cols, rows));
            assert_eq!(grid.to_term_cells(&ANSI_COLORS).len(), cols * rows);
            let (cursor_row, cursor_col) = grid.cursor();
            assert!(cursor_row < rows && cursor_col < cols);
        }
    }

    #[test]
    fn resize_rejects_zero_without_mutating_grid() {
        let mut grid = TerminalGrid::new(4, 2);
        assert!(!grid.resize(0, 2));
        assert_eq!((grid.cols, grid.rows), (4, 2));
    }

    #[test]
    fn unknown_escape_does_not_crash_or_print() {
        let mut grid = TerminalGrid::new(4, 2);
        grid.feed(b"\x1b[?9999hA");
        assert_eq!(grid.cell(0, 0).ch, 'A');
    }
}
