//! Table widget — fixed-column data grid with alternating row colors.

use crate::geom::Rect;
use crate::paint::Canvas;
use crate::theme::Theme;

pub struct Column<'a> {
    pub header: &'a str,
    pub width: u32,
    pub right_align: bool,
}

pub struct Table<'a> {
    pub rect: Rect,
    pub columns: &'a [Column<'a>],
    pub rows: &'a [&'a [&'a str]],
    pub selected: Option<usize>,
    pub header_h: u32,
    pub row_h: u32,
}

impl<'a> Table<'a> {
    pub fn new(rect: Rect, columns: &'a [Column<'a>], rows: &'a [&'a [&'a str]]) -> Self {
        Self {
            rect,
            columns,
            rows,
            selected: None,
            header_h: 18,
            row_h: 16,
        }
    }

    fn col_x(&self, col_idx: usize) -> i32 {
        self.rect.x
            + self.columns[..col_idx]
                .iter()
                .map(|c| c.width as i32)
                .sum::<i32>()
    }

    pub fn draw(&self, canvas: &mut Canvas, theme: &Theme) {
        // ── Header ───────────────────────────────────────────────────────────
        let header_rect = Rect::new(self.rect.x, self.rect.y, self.rect.w, self.header_h);
        canvas.fill_rect(header_rect, theme.panel_alt);
        canvas.hbar(
            self.rect.x,
            self.rect.y + self.header_h as i32 - 1,
            self.rect.w,
            1,
            theme.accent,
        );

        let mut cx = self.rect.x;
        for col in self.columns.iter() {
            let col_rect = Rect::new(
                cx + 4,
                self.rect.y,
                col.width.saturating_sub(4),
                self.header_h,
            );
            canvas.draw_text(
                col_rect.x,
                col_rect.y + (self.header_h as i32 - 10) / 2,
                col.header,
                theme.text,
            );
            cx += col.width as i32;
        }

        // ── Rows ─────────────────────────────────────────────────────────────
        let visible_rows = ((self.rect.h.saturating_sub(self.header_h)) / self.row_h) as usize;
        let row_count = self.rows.len().min(visible_rows);

        for (row_idx, row_data) in self.rows.iter().take(row_count).enumerate() {
            let ry = self.rect.y + self.header_h as i32 + (row_idx as u32 * self.row_h) as i32;
            let row_rect = Rect::new(self.rect.x, ry, self.rect.w, self.row_h);

            let bg = if self.selected == Some(row_idx) {
                theme.accent.darken(180)
            } else if row_idx % 2 == 0 {
                theme.panel
            } else {
                theme.panel_alt
            };
            canvas.fill_rect(row_rect, bg);

            let text_color = if self.selected == Some(row_idx) {
                theme.accent
            } else {
                theme.text
            };

            for (col_idx, col) in self.columns.iter().enumerate() {
                let cell_text = row_data.get(col_idx).copied().unwrap_or("");
                let cx2 = self.col_x(col_idx);
                let cell_rect = Rect::new(cx2, ry, col.width, self.row_h);
                let pad = 4;

                if col.right_align {
                    canvas.draw_text_right(cell_rect, cell_text, text_color, pad);
                } else {
                    let ty = ry + (self.row_h as i32 - 10) / 2;
                    canvas.draw_text(cx2 + pad, ty, cell_text, text_color);
                }
            }

            // Row separator
            canvas.hbar(
                self.rect.x,
                ry + self.row_h as i32 - 1,
                self.rect.w,
                1,
                theme.border,
            );
        }

        // Outer border
        canvas.draw_rect(self.rect, theme.border);
    }

    /// Returns the row index clicked at `(x, y)`, if any.
    pub fn hit_test(&self, x: i32, y: i32) -> Option<usize> {
        let p = crate::geom::Point::new(x, y);
        if !self.rect.contains(p) {
            return None;
        }
        let rel_y = y - self.rect.y - self.header_h as i32;
        if rel_y < 0 {
            return None;
        }
        let row = (rel_y as u32) / self.row_h;
        if row < self.rows.len() as u32 {
            Some(row as usize)
        } else {
            None
        }
    }
}
