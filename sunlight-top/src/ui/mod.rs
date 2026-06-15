pub mod header;
pub mod table;

use crate::telemetry::SystemSnapshot;
use crate::terminal::Canvas;
use table::{SortColumn, SortKey};

pub struct ViewState {
    pub term_rows: u16,
    pub term_cols: u16,
    pub sort: SortKey,
    pub scroll: usize,
}

impl ViewState {
    pub fn new() -> Self {
        Self {
            term_rows: 24,
            term_cols: 80,
            sort: SortKey {
                column: SortColumn::Cpu,
                descending: true,
            },
            scroll: 0,
        }
    }

    pub fn render(&self, c: &mut Canvas, snap: &SystemSnapshot, my_pid: u32) {
        header::render_header(c, snap, self.term_cols);

        c.move_to(7, 1);
        c.fg_dim();
        for _ in 0..self.term_cols {
            c.push(b'-');
        }
        c.reset();

        table::render_table_header(c, 8);

        let table_start = 9u16;
        let table_rows = self.term_rows.saturating_sub(table_start + 1);
        table::render_table(
            c,
            snap,
            table_start,
            table_rows,
            self.term_cols,
            &self.sort,
            my_pid,
        );

        self.render_footer(c);
        c.flush();
    }

    fn render_footer(&self, c: &mut Canvas) {
        c.move_to(self.term_rows, 1);
        c.bg_surface();
        c.fg_dim();
        c.push_str(" q:quit  s:sort-cpu  m:sort-mem  p:sort-pid  n:sort-name ");
        c.reset();
        c.clear_eol();
    }
}
