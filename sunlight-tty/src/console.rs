pub use crate::grid::{Cell, TermCell};
use crate::TerminalGrid;

pub struct Console {
    grid: TerminalGrid,
    scroll_offset: usize,
    pub dirty: bool,
}

impl Console {
    pub fn new(cols: usize, rows: usize) -> Self {
        Self {
            grid: TerminalGrid::new(cols.max(1), rows.max(1)),
            scroll_offset: 0,
            dirty: false,
        }
    }

    pub fn new_with_margin(cols: usize, rows: usize, top_margin_rows: usize) -> Self {
        let mut console = Self::new(cols, rows.saturating_add(top_margin_rows));
        for _ in 0..top_margin_rows.min(console.grid.rows.saturating_sub(1)) {
            console.grid.feed(b"\n");
        }
        console
    }

    pub fn feed_byte(&mut self, byte: u8) {
        self.feed(&[byte]);
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        self.grid.feed(bytes);
        self.dirty |= !bytes.is_empty();
    }

    pub fn resize(&mut self, cols: usize, rows: usize) -> bool {
        if self.dims() == (cols, rows) {
            return true;
        }
        if !self.grid.resize(cols, rows) {
            return false;
        }
        self.scroll_offset = 0;
        self.dirty = true;
        true
    }

    pub fn cursor(&self) -> (usize, usize) {
        self.grid.cursor()
    }

    pub fn cursor_visible(&self) -> bool {
        self.grid.cursor_visible() && (self.scroll_offset == 0 || self.grid.in_alt_screen())
    }

    pub fn in_alt_screen(&self) -> bool {
        self.grid.in_alt_screen()
    }

    pub fn scrollback_len(&self) -> usize {
        self.grid.scrollback_len()
    }

    pub fn set_scroll_offset(&mut self, offset: usize) {
        let offset = offset.min(self.scrollback_len());
        self.dirty |= offset != self.scroll_offset;
        self.scroll_offset = offset;
    }

    pub fn scroll_up_viewport(&mut self) {
        self.set_scroll_offset(self.scroll_offset.saturating_add(1));
    }

    pub fn scroll_down_viewport(&mut self) {
        self.set_scroll_offset(self.scroll_offset.saturating_sub(1));
    }

    pub fn reset_scroll_offset(&mut self) {
        self.set_scroll_offset(0);
    }

    pub fn get_scroll_offset(&self) -> usize {
        self.scroll_offset
    }

    pub fn dims(&self) -> (usize, usize) {
        (self.grid.cols, self.grid.rows)
    }

    pub fn clear_dirty(&mut self) {
        self.dirty = false;
    }

    pub fn to_term_cells(&mut self, ansi_colors: &[u32; 16]) -> &[TermCell] {
        self.grid.to_term_cells(ansi_colors)
    }

    pub fn to_term_cells_with_offset(
        &mut self,
        ansi_colors: &[u32; 16],
        viewport_offset: usize,
    ) -> &[TermCell] {
        self.grid
            .to_term_cells_with_offset(ansi_colors, viewport_offset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PALETTE: [u32; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];

    #[test]
    fn console_uses_shared_vt_semantics_across_chunk_boundaries() {
        let mut console = Console::new(8, 3);
        let mut grid = TerminalGrid::new(8, 3);
        let stream = b"shell\x1b[?1049h\x1b[2;3H\x1b[1;31mX\x1b[K\x1b[?25l";
        for chunk in stream.chunks(3) {
            console.feed(chunk);
            grid.feed(chunk);
        }
        assert_eq!(console.cursor(), grid.cursor());
        assert!(!console.cursor_visible());
        assert!(console.in_alt_screen());
        let actual = console.to_term_cells(&PALETTE);
        let expected = grid.to_term_cells(&PALETTE);
        for (actual, expected) in actual.iter().zip(expected) {
            assert_eq!(
                (actual.ch, actual.fg, actual.bg),
                (expected.ch, expected.fg, expected.bg)
            );
        }
        console.feed(b"\x1b[?1049l");
        assert_eq!(console.to_term_cells(&PALETTE)[0].ch, b's');
    }

    #[test]
    fn history_joins_the_first_live_row_without_skipping_lines() {
        let mut console = Console::new(8, 3);
        console.feed(b"one\r\ntwo\r\nthree\r\nfour");
        assert_eq!(console.scrollback_len(), 1);
        let cells = console.to_term_cells_with_offset(&PALETTE, 1);
        assert_eq!(cells[0].ch, b'o');
        assert_eq!(cells[8].ch, b't');
        assert_eq!(cells[17].ch, b'h');
    }

    #[test]
    fn empty_geometry_is_safe_and_scroll_offset_is_bounded() {
        let mut console = Console::new(0, 0);
        console.feed(b"abcdef");
        console.set_scroll_offset(usize::MAX);
        assert_eq!(console.get_scroll_offset(), console.scrollback_len());
        assert!(!console.resize(0, 0));
    }

    #[test]
    fn full_width_rows_do_not_scroll_until_the_next_printable_character() {
        let mut console = Console::new(4, 2);
        console.feed(b"abcd\r\nefgh");
        assert_eq!(console.scrollback_len(), 0);
        assert_eq!(console.cursor(), (1, 3));
        let cells = console.to_term_cells(&PALETTE);
        assert_eq!(cells[0].ch, b'a');
        assert_eq!(cells[7].ch, b'h');
        console.feed(b"i");
        assert_eq!(console.scrollback_len(), 1);
        assert_eq!(console.to_term_cells(&PALETTE)[4].ch, b'i');
    }

    #[test]
    fn long_running_alternate_screen_restores_shell_without_replaying_output() {
        let mut console = Console::new(80, 24);
        console.feed(b"shell history\x1b[?1049h");
        for _ in 0..8192 {
            console.feed(b"\x1b[2J\x1b[Hstatus\x1b[24;80HX");
        }
        assert!(console.in_alt_screen());
        assert_eq!(console.scrollback_len(), 0);
        console.feed(b"\x1b[?1049l");
        assert_eq!(console.to_term_cells(&PALETTE)[0].ch, b's');
        assert_eq!(console.cursor(), (0, 13));
    }
}
