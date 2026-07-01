//! 2D character-grid terminal emulator with VT100/ANSI escape support.
//!
//! Maintains a `(cols x rows)` grid of styled characters, plus scrollback
//! history for the normal screen. Feeds bytes through the VT parser,
//! interprets output events, and updates screen state.

use crate::vt100::{Vt100Parser, VtOutput};
use alloc::vec::Vec;
pub use sunlight_tui::TermCell;

const SCROLLBACK_LINES: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    pub ch: u8,
    pub fg: u8,
    pub bg: u8,
    pub bold: bool,
    pub inverse: bool,
    pub underline: bool,
}

impl Cell {
    const fn blank() -> Self {
        Self {
            ch: b' ',
            fg: 7,
            bg: 0,
            bold: false,
            inverse: false,
            underline: false,
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
    saved_cursor: Option<(usize, usize)>,
    saved_main_cursor: (usize, usize),
    cur_fg: u8,
    cur_bg: u8,
    cur_bold: bool,
    cur_inverse: bool,
    cur_underline: bool,
    cursor_visible: bool,
    use_alt_screen: bool,
    parser: Vt100Parser,
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
                ch: b' ',
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
            saved_cursor: None,
            saved_main_cursor: (0, 0),
            cur_fg: 7,
            cur_bg: 0,
            cur_bold: false,
            cur_inverse: false,
            cur_underline: false,
            cursor_visible: true,
            use_alt_screen: false,
            parser: Vt100Parser::new(),
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            let output = self.parser.feed(byte);
            self.handle_output(output);
        }
    }

    fn handle_output(&mut self, output: VtOutput) {
        match output {
            VtOutput::Char(ch) => self.write_char(ch),
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

    fn write_char(&mut self, ch: u8) {
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
        let rows = self.rows;
        let (cursor_row, cursor_col) = self.cursor_mut();
        *cursor_col += 1;
        if *cursor_col >= cols {
            *cursor_col = 0;
            *cursor_row += 1;
            if *cursor_row >= rows {
                self.scroll_up();
            }
        }
    }

    fn newline(&mut self) {
        let (cursor_row, cursor_col) = self.cursor_mut();
        *cursor_col = 0;
        *cursor_row += 1;
        if *cursor_row >= self.rows {
            self.scroll_up();
        }
    }

    fn carriage_return(&mut self) {
        let (_, cursor_col) = self.cursor_mut();
        *cursor_col = 0;
    }

    fn backspace(&mut self) {
        let (_, cursor_col) = self.cursor_mut();
        *cursor_col = cursor_col.saturating_sub(1);
    }

    fn tab(&mut self) {
        let cols = self.cols;
        let (_, cursor_col) = self.cursor_mut();
        let next = ((*cursor_col / 8) + 1) * 8;
        *cursor_col = next.min(cols.saturating_sub(1));
    }

    fn set_cursor(&mut self, row: usize, col: usize) {
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
        self.scrollback_head = 0;
        self.scrollback_count = 0;
        self.saved_cursor = None;
        self.saved_main_cursor = (0, 0);
        self.cursor_visible = true;
        self.use_alt_screen = false;
        self.reset_attrs();
        self.parser = Vt100Parser::new();
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

    fn apply_sgr(&mut self, params: &[u16; 8], count: usize) {
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
                30..=37 => self.cur_fg = (params[i] - 30) as u8,
                39 => self.cur_fg = 7,
                40..=47 => self.cur_bg = (params[i] - 40) as u8,
                49 => self.cur_bg = 0,
                90..=97 => self.cur_fg = (params[i] - 90 + 8) as u8,
                100..=107 => self.cur_bg = (params[i] - 100 + 8) as u8,
                38 => {
                    if i + 2 < count && params[i + 1] == 5 {
                        self.cur_fg = map_extended_color(params[i + 2]);
                        i += 2;
                    } else if i + 4 < count && params[i + 1] == 2 {
                        self.cur_fg = map_rgb_to_ansi(params[i + 2], params[i + 3], params[i + 4]);
                        i += 4;
                    }
                }
                48 => {
                    if i + 2 < count && params[i + 1] == 5 {
                        self.cur_bg = map_extended_color(params[i + 2]);
                        i += 2;
                    } else if i + 4 < count && params[i + 1] == 2 {
                        self.cur_bg = map_rgb_to_ansi(params[i + 2], params[i + 3], params[i + 4]);
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
        self.cur_fg = 7;
        self.cur_bg = 0;
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

    pub fn to_term_cells_with_offset(
        &mut self,
        ansi_colors: &[u32; 16],
        viewport_offset: usize,
    ) -> &[TermCell] {
        if viewport_offset == 0 || self.use_alt_screen {
            return self.to_term_cells(ansi_colors);
        }

        for screen_row in 0..self.rows {
            let history_row_idx = if self.scrollback_count > viewport_offset {
                self.scrollback_count - viewport_offset + screen_row
            } else {
                screen_row
            };

            let dst_start = screen_row * self.cols;
            if history_row_idx < self.scrollback_count {
                let src_start =
                    ((self.scrollback_head + history_row_idx) % SCROLLBACK_LINES) * self.cols;
                for col in 0..self.cols {
                    self.term_cells[dst_start + col] =
                        resolve_cell(self.scrollback[src_start + col], ansi_colors);
                }
            } else {
                let src_start = screen_row * self.cols;
                for col in 0..self.cols {
                    self.term_cells[dst_start + col] =
                        resolve_cell(self.main_cells[src_start + col], ansi_colors);
                }
            }
        }

        &self.term_cells
    }
}

fn resolve_cell(cell: Cell, ansi_colors: &[u32; 16]) -> TermCell {
    let fg_idx = if cell.bold && cell.fg < 8 {
        cell.fg + 8
    } else {
        cell.fg
    };
    let mut fg = ansi_colors[fg_idx as usize % 16];
    let mut bg = ansi_colors[cell.bg as usize % 16];
    if cell.inverse {
        core::mem::swap(&mut fg, &mut bg);
    }
    TermCell {
        ch: cell.ch,
        fg,
        bg,
    }
}

fn map_extended_color(color: u16) -> u8 {
    match color {
        0..=15 => color as u8,
        16..=231 => {
            let idx = color - 16;
            let r = idx / 36;
            let g = (idx / 6) % 6;
            let b = idx % 6;
            map_rgb_to_ansi((r * 51) as u16, (g * 51) as u16, (b * 51) as u16)
        }
        232..=255 => {
            let gray = 8 + (color - 232) * 10;
            map_rgb_to_ansi(gray, gray, gray)
        }
        _ => 7,
    }
}

fn map_rgb_to_ansi(r: u16, g: u16, b: u16) -> u8 {
    const ANSI_RGB: [(u16, u16, u16); 16] = [
        (0x00, 0x00, 0x00),
        (0xCC, 0x24, 0x1D),
        (0x98, 0x97, 0x1A),
        (0xD7, 0x99, 0x21),
        (0x45, 0x85, 0x88),
        (0xB1, 0x62, 0x86),
        (0x68, 0x9D, 0x6A),
        (0xA8, 0x99, 0x84),
        (0x92, 0x83, 0x74),
        (0xFB, 0x49, 0x34),
        (0xB8, 0xBB, 0x26),
        (0xFA, 0xBD, 0x2F),
        (0x83, 0xA5, 0x98),
        (0xD3, 0x86, 0x9B),
        (0x8E, 0xC0, 0x7C),
        (0xEB, 0xDB, 0xB2),
    ];

    let mut best = 7u8;
    let mut best_dist = u32::MAX;
    for (idx, &(pr, pg, pb)) in ANSI_RGB.iter().enumerate() {
        let dr = pr.abs_diff(r) as u32;
        let dg = pg.abs_diff(g) as u32;
        let db = pb.abs_diff(b) as u32;
        let dist = dr * dr + dg * dg + db * db;
        if dist < best_dist {
            best_dist = dist;
            best = idx as u8;
        }
    }
    best
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
        assert_eq!(grid.cell(0, 0).ch, b'a');
        assert_eq!(grid.cell(0, 1).ch, b'b');
    }

    #[test]
    fn newline_and_carriage_return_work() {
        let mut grid = TerminalGrid::new(4, 2);
        grid.feed(b"ab\rZ\nQ");
        assert_eq!(grid.cell(0, 0).ch, b'Z');
        assert_eq!(grid.cell(1, 0).ch, b'Q');
    }

    #[test]
    fn cursor_movement_updates_in_place() {
        let mut grid = TerminalGrid::new(4, 2);
        grid.feed(b"ab\x1b[1D!");
        assert_eq!(grid.cell(0, 0).ch, b'a');
        assert_eq!(grid.cell(0, 1).ch, b'!');
    }

    #[test]
    fn clear_screen_resets_cells() {
        let mut grid = TerminalGrid::new(4, 2);
        grid.feed(b"ab\x1b[2J");
        assert_eq!(grid.cell(0, 0).ch, b' ');
        assert_eq!(grid.cursor(), (0, 0));
    }

    #[test]
    fn clear_line_mode_two_clears_current_row() {
        let mut grid = TerminalGrid::new(4, 2);
        grid.feed(b"ab\ncd\x1b[2K");
        assert_eq!(grid.cell(1, 0).ch, b' ');
        assert_eq!(grid.cell(1, 1).ch, b' ');
    }

    #[test]
    fn sgr_color_reset_restores_defaults() {
        let mut grid = TerminalGrid::new(4, 1);
        grid.feed(b"\x1b[31mA\x1b[0mB");
        assert_eq!(grid.cell(0, 0).fg, 1);
        assert_eq!(grid.cell(0, 1).fg, 7);
    }

    #[test]
    fn alternate_screen_enter_exit_restores_main_buffer() {
        let mut grid = TerminalGrid::new(4, 2);
        grid.feed(b"main\x1b[?1049hALT\x1b[?1049l");
        assert_eq!(grid.cell(0, 0).ch, b'm');
        let cells = grid.to_term_cells(&ANSI_COLORS);
        assert_eq!(cells[0].ch, b'm');
    }

    #[test]
    fn resize_like_clear_screen_keeps_parser_sane() {
        let mut grid = TerminalGrid::new(4, 2);
        grid.clear_screen();
        grid.feed(b"\x1b[31mX");
        assert_eq!(grid.cell(0, 0).ch, b'X');
        assert_eq!(grid.cell(0, 0).fg, 1);
    }

    #[test]
    fn unknown_escape_does_not_crash_or_print() {
        let mut grid = TerminalGrid::new(4, 2);
        grid.feed(b"\x1b[?9999hA");
        assert_eq!(grid.cell(0, 0).ch, b'A');
    }
}
