//! Table widget — fixed-column data grid with alternating row colors and scrolling.

use crate::font::VecText;
use crate::geom::Rect;
use crate::paint::Canvas;
use crate::theme::Theme;
use core::cmp;

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
    pub scroll_offset: usize,
    pub header_h: u32,
    pub row_h: u32,
    font: Option<&'a dyn VecText>,
}

impl<'a> Table<'a> {
    pub fn new(rect: Rect, columns: &'a [Column<'a>], rows: &'a [&'a [&'a str]]) -> Self {
        Self {
            rect,
            columns,
            rows,
            selected: None,
            scroll_offset: 0,
            header_h: 18,
            row_h: 16,
            font: None,
        }
    }

    pub fn with_selected(mut self, selected: Option<usize>) -> Self {
        self.selected = selected;
        self
    }

    pub fn with_scroll_offset(mut self, offset: usize) -> Self {
        self.scroll_offset = offset;
        self
    }

    /// Enable vector font rendering and bump row heights to fit the larger glyphs.
    pub fn with_font(mut self, font: &'a dyn VecText) -> Self {
        self.font = Some(font);
        let lh = font.line_height();
        if self.header_h < lh + 8 {
            self.header_h = lh + 8;
        }
        if self.row_h < lh + 5 {
            self.row_h = lh + 5;
        }
        self
    }

    fn col_x(&self, col_idx: usize) -> i32 {
        self.rect.x
            + self.columns[..col_idx]
                .iter()
                .map(|c| c.width as i32)
                .sum::<i32>()
    }

    /// Visible rows fitting in the widget area, clamped to available data.
    fn visible_count(&self) -> usize {
        let max_fit = ((self.rect.h.saturating_sub(self.header_h)) / self.row_h) as usize;
        max_fit.min(self.rows.len().saturating_sub(self.scroll_offset))
    }

    /// Maximum scroll offset so the last row is still (at least partially) visible.
    fn max_scroll(&self) -> usize {
        let max_fit = ((self.rect.h.saturating_sub(self.header_h)) / self.row_h) as usize;
        self.rows.len().saturating_sub(max_fit)
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
            if let Some(f) = self.font {
                f.draw_vcenter(
                    canvas,
                    col.header,
                    col_rect.x,
                    col_rect.y,
                    self.header_h,
                    theme.text,
                );
            } else {
                canvas.draw_text(
                    col_rect.x,
                    col_rect.y + (self.header_h as i32 - 10) / 2,
                    col.header,
                    theme.text,
                );
            }
            cx += col.width as i32;
        }

        // ── Rows ─────────────────────────────────────────────────────────────
        let visible_rows = self.visible_count();
        let offset = self.scroll_offset;

        for local_idx in 0..visible_rows {
            let row_idx = offset + local_idx;
            let row_data = match self.rows.get(row_idx) {
                Some(r) => r,
                None => break,
            };
            let ry = self.rect.y + self.header_h as i32 + (local_idx as u32 * self.row_h) as i32;
            let row_rect = Rect::new(self.rect.x, ry, self.rect.w, self.row_h);

            let bg = if self.selected == Some(row_idx) {
                // Restrained warm selection from shared chrome roles.
                theme.chrome.selection
            } else if local_idx % 2 == 0 {
                theme.chrome.window_bg
            } else {
                theme.chrome.card_bg
            };
            canvas.fill_rect(row_rect, bg);

            let text_color = if self.selected == Some(row_idx) {
                theme.accent_hover
            } else {
                theme.text
            };

            for (col_idx, col) in self.columns.iter().enumerate() {
                let cell_text = row_data.get(col_idx).copied().unwrap_or("");
                let cx2 = self.col_x(col_idx);
                let cell_rect = Rect::new(cx2, ry, col.width, self.row_h);
                let pad = 4;

                if let Some(f) = self.font {
                    if col.right_align {
                        let tw = f.measure_w(cell_text);
                        let tx = cell_rect.right() - tw as i32 - pad;
                        f.draw_vcenter(canvas, cell_text, tx, ry, self.row_h, text_color);
                    } else {
                        f.draw_vcenter(canvas, cell_text, cx2 + pad, ry, self.row_h, text_color);
                    }
                } else if col.right_align {
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

        // ── Scroll indicators ────────────────────────────────────────────────
        if self.max_scroll() > 0 {
            let scroll_indicator_color = theme.text_dim;
            let indicator_size = 4;
            let indicator_y = self.rect.y + self.header_h as i32 + 4;
            if offset > 0 {
                // Up arrow indicator (top of content area)
                let cx2 = self.rect.right() - 10;
                canvas.fill_rect(
                    Rect::new(cx2, indicator_y, 6, indicator_size),
                    scroll_indicator_color,
                );
            }
            if offset + visible_rows < self.rows.len() {
                // Down arrow indicator (bottom of content area)
                let cx2 = self.rect.right() - 10;
                let by = self.rect.bottom() - indicator_size as i32 - 4;
                canvas.fill_rect(
                    Rect::new(cx2, by, 6, indicator_size),
                    scroll_indicator_color,
                );
            }
        }

        // Outer border
        canvas.draw_rect(self.rect, theme.border);
    }

    /// Returns the logical row index clicked at `(x, y)`, if any.
    pub fn hit_test(&self, x: i32, y: i32) -> Option<usize> {
        let p = crate::geom::Point::new(x, y);
        if !self.rect.contains(p) {
            return None;
        }
        let rel_y = y - self.rect.y - self.header_h as i32;
        if rel_y < 0 {
            return None;
        }
        let local_row = (rel_y as u32) / self.row_h;
        let row = self.scroll_offset + local_row as usize;
        if row < self.rows.len() {
            Some(row)
        } else {
            None
        }
    }

    /// Number of rows that fit in the visible area (for scroll calculation).
    pub fn visible_row_count(&self) -> usize {
        let max_fit = ((self.rect.h.saturating_sub(self.header_h)) / self.row_h) as usize;
        cmp::min(max_fit, self.rows.len())
    }
}
