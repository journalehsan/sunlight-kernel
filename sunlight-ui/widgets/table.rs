//! Table widget — fixed-column data grid with alternating row colors and scrolling.

use crate::font::VecText;
use crate::geom::Rect;
use crate::paint::Canvas;
use crate::theme::Theme;
use core::cmp;

/// Application-owned sorting; column IDs remain stable when columns move.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SortState {
    pub column: usize,
    pub descending: bool,
}
impl SortState {
    pub fn toggle(current: Option<Self>, column: usize) -> Self {
        Self {
            column,
            descending: current.is_some_and(|s| s.column == column && !s.descending),
        }
    }
    /// Apply direction without reversing the caller's stable identity tie-breaker.
    pub fn compare(self, order: cmp::Ordering, tie: cmp::Ordering) -> cmp::Ordering {
        if order.is_eq() {
            tie
        } else if self.descending {
            order.reverse()
        } else {
            order
        }
    }
}

/// Typed keys compare underlying values, never rendered cell strings. Missing values
/// precede present values ascending. Custom domains may supply their own comparator
/// to `SortState::compare` instead.
#[derive(Clone, Copy)]
pub enum SortKey<'a> {
    Missing,
    Text(&'a str),
    Integer(i128),
    Float(f64),
    Bytes(u64),
    Rank(u32),
}
impl SortKey<'_> {
    pub fn compare(self, other: Self) -> cmp::Ordering {
        use SortKey::*;
        match (self, other) {
            (Missing, Missing) => cmp::Ordering::Equal,
            (Missing, _) => cmp::Ordering::Less,
            (_, Missing) => cmp::Ordering::Greater,
            (Text(a), Text(b)) => a
                .chars()
                .flat_map(char::to_lowercase)
                .cmp(b.chars().flat_map(char::to_lowercase)),
            (Integer(a), Integer(b)) => a.cmp(&b),
            (Float(a), Float(b)) => a.total_cmp(&b),
            (Bytes(a), Bytes(b)) => a.cmp(&b),
            (Rank(a), Rank(b)) => a.cmp(&b),
            _ => cmp::Ordering::Equal,
        }
    }
}

/// Use this ID for a displayed column whose header must not sort.
pub const UNSORTABLE_COLUMN: usize = usize::MAX;

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
    sortable: &'a [usize],
    sort: Option<SortState>,
    hovered_header: Option<usize>,
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
            sortable: &[],
            sort: None,
            hovered_header: None,
        }
    }

    /// IDs correspond to displayed columns; an empty slice disables sorting.
    /// Use `UNSORTABLE_COLUMN` for individual disabled headers. The caller owns
    /// row order and applies keys or a domain comparator with `SortState::compare`.
    pub fn with_sorting(mut self, ids: &'a [usize], state: Option<SortState>) -> Self {
        self.sortable = ids;
        self.sort = state;
        self
    }
    pub fn with_header_hover(mut self, column: Option<usize>) -> Self {
        self.hovered_header = column;
        self
    }
    pub fn header_hit_test(&self, x: i32, y: i32) -> Option<usize> {
        if !self.rect.contains(crate::geom::Point::new(x, y))
            || y >= self.rect.y + self.header_h as i32
        {
            return None;
        }
        (0..self.columns.len())
            .find(|&i| x >= self.col_x(i) && x < self.col_x(i) + self.columns[i].width as i32)
    }
    /// Return a controlled sort change from a header click, separately from row selection.
    pub fn sort_at(&self, x: i32, y: i32) -> Option<SortState> {
        let column = *self.sortable.get(self.header_hit_test(x, y)?)?;
        if column == UNSORTABLE_COLUMN {
            return None;
        }
        Some(SortState::toggle(self.sort, column))
    }
    pub fn draw_header(&self, canvas: &mut Canvas, theme: &Theme) {
        self.draw_header_inner(canvas, theme);
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
        let max_fit = ((self.rect.h.saturating_sub(self.header_h)) / self.row_h.max(1)) as usize;
        max_fit.min(
            self.rows
                .len()
                .saturating_sub(self.scroll_offset.min(self.max_scroll())),
        )
    }

    /// Maximum scroll offset so the last row is still (at least partially) visible.
    fn max_scroll(&self) -> usize {
        let max_fit = ((self.rect.h.saturating_sub(self.header_h)) / self.row_h.max(1)) as usize;
        self.rows.len().saturating_sub(max_fit)
    }

    fn draw_header_inner(&self, canvas: &mut Canvas, theme: &Theme) {
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
        for (index, col) in self.columns.iter().enumerate() {
            let width = col
                .width
                .min(self.rect.right().saturating_sub(cx).max(0) as u32);
            if width == 0 {
                break;
            }
            if self
                .sortable
                .get(index)
                .is_some_and(|id| *id != UNSORTABLE_COLUMN)
                && self.hovered_header == Some(index)
            {
                canvas.fill_rect(
                    Rect::new(cx, self.rect.y, width, self.header_h),
                    theme.chrome.card_bg,
                );
            }
            if self
                .sort
                .is_some_and(|s| self.sortable.get(index) == Some(&s.column))
            {
                canvas.draw_text(
                    cx + width.saturating_sub(12) as i32,
                    self.rect.y + 4,
                    if self.sort.unwrap().descending {
                        "▼"
                    } else {
                        "▲"
                    },
                    theme.accent,
                );
            }
            let col_rect = Rect::new(cx + 4, self.rect.y, width.saturating_sub(4), self.header_h);
            let mut header = col.header;
            let available = width.saturating_sub(20);
            while !header.is_empty()
                && self
                    .font
                    .map_or(header.chars().count() as u32 * 8, |f| f.measure_w(header))
                    > available
            {
                header = &header[..header.char_indices().last().unwrap().0];
            }
            if let Some(f) = self.font {
                f.draw_vcenter(
                    canvas,
                    header,
                    col_rect.x,
                    col_rect.y,
                    self.header_h,
                    theme.text,
                );
            } else {
                canvas.draw_text(
                    col_rect.x,
                    col_rect.y + (self.header_h as i32 - 10) / 2,
                    header,
                    theme.text,
                );
            }
            cx += width as i32;
        }
    }

    pub fn draw(&self, canvas: &mut Canvas, theme: &Theme) {
        self.draw_header_inner(canvas, theme);
        // ── Rows ─────────────────────────────────────────────────────────────
        let visible_rows = self.visible_count();
        let offset = self.scroll_offset.min(self.max_scroll());

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
                let mut cell_text = row_data.get(col_idx).copied().unwrap_or("");
                let cx2 = self.col_x(col_idx);
                let width = col
                    .width
                    .min(self.rect.right().saturating_sub(cx2).max(0) as u32);
                if width == 0 {
                    break;
                }
                let available = width.saturating_sub(8);
                while !cell_text.is_empty()
                    && self.font.map_or(cell_text.chars().count() as u32 * 8, |f| {
                        f.measure_w(cell_text)
                    }) > available
                {
                    cell_text = &cell_text[..cell_text.char_indices().last().unwrap().0];
                }
                let cell_rect = Rect::new(cx2, ry, width, self.row_h);
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
        let local_row = (rel_y as u32) / self.row_h.max(1);
        if local_row as usize >= self.visible_row_count() {
            return None;
        }
        let row = self.scroll_offset.min(self.max_scroll()) + local_row as usize;
        if row < self.rows.len() {
            Some(row)
        } else {
            None
        }
    }

    /// Number of rows that fit in the visible area (for scroll calculation).
    pub fn visible_row_count(&self) -> usize {
        let max_fit = ((self.rect.h.saturating_sub(self.header_h)) / self.row_h.max(1)) as usize;
        cmp::min(max_fit, self.rows.len())
    }
}

#[cfg(test)]
mod sorting_tests {
    use super::*;
    #[test]
    fn small_scrollview_track_and_disabled_column_are_safe() {
        let geometry =
            crate::widgets::ScrollView::geometry(10, 100, 30, 90, Rect::new(0, 0, 8, 8), 8);
        assert!(geometry.thumb.h <= geometry.track.h);
        let columns = [Column {
            header: "Name",
            width: 80,
            right_align: false,
        }];
        let table = Table::new(Rect::new(0, 0, 80, 80), &columns, &[])
            .with_sorting(&[UNSORTABLE_COLUMN], None);
        assert_eq!(table.sort_at(4, 4), None);
    }
    #[test]
    fn controlled_sort_toggles_and_switches() {
        let a = SortState::toggle(None, 4);
        assert!(!a.descending);
        let b = SortState::toggle(Some(a), 4);
        assert!(b.descending);
        assert_eq!(SortState::toggle(Some(b), 4), a);
        assert!(!SortState::toggle(Some(b), 1).descending);
    }
    #[test]
    fn typed_keys_and_identity_ties() {
        assert!(SortKey::Bytes(900 * 1024)
            .compare(SortKey::Bytes(2 * 1024 * 1024))
            .is_lt());
        assert!(SortKey::Float(9.8).compare(SortKey::Float(20.0)).is_lt());
        assert!(SortKey::Integer(9).compare(SortKey::Integer(20)).is_lt());
        assert_eq!(
            SortKey::Text("AbC").compare(SortKey::Text("abc")),
            cmp::Ordering::Equal
        );
        assert!(SortKey::Missing.compare(SortKey::Rank(0)).is_lt());
        assert!(SortState {
            column: 0,
            descending: true
        }
        .compare(cmp::Ordering::Equal, cmp::Ordering::Less)
        .is_lt());
    }
    #[test]
    fn optional_headers_never_select_rows_and_offsets_clamp() {
        let columns = [Column {
            header: "PID",
            width: 100,
            right_align: true,
        }];
        let row: &[&str] = &["1"];
        let rows = [row; 10];
        let table = Table::new(Rect::new(0, 0, 100, 50), &columns, &rows).with_scroll_offset(999);
        assert_eq!(table.sort_at(5, 5), None);
        assert_eq!(table.hit_test(5, 5), None);
        assert_eq!(table.hit_test(5, 20), Some(8));
        let table = table.with_sorting(&[42], None);
        assert_eq!(table.sort_at(5, 5).unwrap().column, 42);
        assert_eq!(table.sort_at(5, 20), None);
    }
}
