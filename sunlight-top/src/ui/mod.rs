pub mod header;
pub mod table;

use crate::telemetry::SystemSnapshot;
use crate::terminal::Canvas;
use table::{SortColumn, SortKey};

const TABLE_START_ROW: u16 = 7;

pub struct ViewState {
    pub term_rows: u16,
    pub term_cols: u16,
    pub sort: SortKey,
    pub scroll: usize,
}

impl ViewState {
    pub fn new(term_cols: u16, term_rows: u16) -> Self {
        Self {
            term_rows,
            term_cols,
            sort: SortKey {
                column: SortColumn::Cpu,
                descending: true,
            },
            scroll: 0,
        }
    }

    pub fn set_terminal_size(&mut self, term_cols: u16, term_rows: u16) -> bool {
        if (self.term_cols, self.term_rows) == (term_cols, term_rows) {
            return false;
        }
        self.term_cols = term_cols;
        self.term_rows = term_rows;
        true
    }

    pub fn scroll_up(&mut self) {
        self.scroll = self.scroll.saturating_sub(1);
    }

    pub fn scroll_down(&mut self, process_count: usize) {
        self.scroll = core::cmp::min(
            self.scroll.saturating_add(1),
            self.max_scroll(process_count),
        );
    }

    pub fn scroll_to_top(&mut self) {
        self.scroll = 0;
    }

    pub fn scroll_to_bottom(&mut self, process_count: usize) {
        self.scroll = self.max_scroll(process_count);
    }

    pub fn render(&mut self, c: &mut Canvas, snap: &SystemSnapshot, my_pid: u32) {
        self.scroll = core::cmp::min(self.scroll, self.max_scroll(snap.proc_count));
        header::render_header(c, snap, self.term_cols);

        c.move_to(5, 1);
        c.fg_dim();
        for _ in 0..self.term_cols {
            c.push(b'-');
        }
        c.reset();

        table::render_table_header(c, 6);

        let table_rows = self.table_rows();
        table::render_table(
            c,
            snap,
            TABLE_START_ROW,
            table_rows,
            self.term_cols,
            &self.sort,
            self.scroll,
            my_pid,
        );

        self.render_footer(c);
        c.flush();
    }

    fn render_footer(&self, c: &mut Canvas) {
        c.move_to(self.term_rows, 1);
        c.bg_surface();
        c.fg_dim();
        c.push_str(" q:quit  j/k:scroll  g/G:top/bottom  s/m/p/n:sort ");
        c.reset();
        c.clear_eol();
    }

    fn table_rows(&self) -> u16 {
        self.term_rows.saturating_sub(TABLE_START_ROW + 1)
    }

    fn max_scroll(&self, process_count: usize) -> usize {
        process_count.saturating_sub(self.table_rows() as usize)
    }
}
