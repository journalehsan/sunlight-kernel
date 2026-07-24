//! Compact centered Search Palette primitives.
//!
//! Pure layout/view widgets for a Walker/dmenu/Alfred-style launcher surface.
//! Shells supply bounded view models; no filesystem or process access lives here.

use crate::{
    font::VecText,
    geom::{Point, Rect},
    image::TgaImage,
    material::{Material, SurfaceRole},
    paint::Canvas,
    theme::Theme,
};

/// MiniType faces for the Search Palette (shell passes `sun_font::Typography`).
#[derive(Clone, Copy, Default)]
pub struct SearchPaletteFonts<'a> {
    /// Query field + app titles (UiRegular).
    pub regular: Option<&'a dyn VecText>,
    /// Selected / emphasized titles (UiMedium).
    pub medium: Option<&'a dyn VecText>,
    /// Subtitles, footer hints, empty state (UiSmall).
    pub small: Option<&'a dyn VecText>,
}

/// Maximum characters stored in a toolkit search field.
pub const SEARCH_FIELD_CAP: usize = 64;
/// Visible result rows per page.
pub const SEARCH_PAGE_ROWS: usize = 6;
/// Maximum page-dot slots drawn under the list.
pub const SEARCH_PAGE_DOT_CAP: usize = 8;

/// Bounded in-place text field for search inputs.
#[derive(Clone, Copy)]
pub struct BoundedSearchField<const N: usize = SEARCH_FIELD_CAP> {
    buf: [u8; N],
    len: usize,
    cursor: usize,
    pub active: bool,
}

impl<const N: usize> BoundedSearchField<N> {
    pub const fn new() -> Self {
        Self {
            buf: [0; N],
            len: 0,
            cursor: 0,
            active: false,
        }
    }

    pub fn value(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }

    pub fn clear(&mut self) {
        self.len = 0;
        self.cursor = 0;
    }

    pub fn set_text(&mut self, text: &str) {
        let bytes = text.as_bytes();
        let n = bytes.len().min(N);
        self.buf[..n].copy_from_slice(&bytes[..n]);
        self.len = n;
        self.cursor = n;
    }

    pub fn insert(&mut self, byte: u8) -> bool {
        if self.len >= N {
            return false;
        }
        let mut i = self.len;
        while i > self.cursor {
            self.buf[i] = self.buf[i - 1];
            i -= 1;
        }
        self.buf[self.cursor] = byte;
        self.len += 1;
        self.cursor += 1;
        true
    }

    pub fn backspace(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        let mut i = self.cursor - 1;
        while i + 1 < self.len {
            self.buf[i] = self.buf[i + 1];
            i += 1;
        }
        self.len -= 1;
        self.cursor -= 1;
        true
    }

    pub fn delete_forward(&mut self) -> bool {
        if self.cursor >= self.len {
            return false;
        }
        let mut i = self.cursor;
        while i + 1 < self.len {
            self.buf[i] = self.buf[i + 1];
            i += 1;
        }
        self.len -= 1;
        true
    }

    pub fn move_left(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        self.cursor -= 1;
        true
    }

    pub fn move_right(&mut self) -> bool {
        if self.cursor >= self.len {
            return false;
        }
        self.cursor += 1;
        true
    }

    pub fn move_home(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        self.cursor = 0;
        true
    }

    pub fn move_end(&mut self) -> bool {
        if self.cursor == self.len {
            return false;
        }
        self.cursor = self.len;
        true
    }

    /// Handle a decoded character (backspace / printable / ignore Enter).
    pub fn handle_char(&mut self, ch: char) -> bool {
        match ch {
            '\u{8}' => self.backspace(),
            '\n' | '\r' => false,
            c if c.is_ascii_graphic() || c == ' ' => {
                if (c as u32) <= 0x7F {
                    self.insert(c as u8)
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    pub fn draw(
        &self,
        canvas: &mut Canvas,
        theme: &Theme,
        rect: Rect,
        placeholder: &str,
        font: Option<&dyn VecText>,
    ) {
        canvas.fill_rounded_rect(rect, 8, theme.panel_alt);
        canvas.stroke_rounded_rect(
            rect,
            8,
            if self.active { 2 } else { 1 },
            if self.active {
                theme.accent
            } else {
                theme.border
            },
        );
        let text_x = rect.x + 12;
        if self.len == 0 {
            if let Some(f) = font {
                f.draw_vcenter(canvas, placeholder, text_x, rect.y, rect.h, theme.text_dim);
            } else {
                canvas.draw_text(
                    text_x,
                    rect.y + (rect.h as i32 - 10) / 2,
                    placeholder,
                    theme.text_dim,
                );
            }
        } else {
            let value = self.value();
            if let Some(f) = font {
                f.draw_vcenter(canvas, value, text_x, rect.y, rect.h, theme.text);
                if self.active {
                    let prefix_end = self.cursor.min(value.len());
                    let prefix = value.get(..prefix_end).unwrap_or(value);
                    let cursor_x = text_x + f.measure_w(prefix) as i32 + 1;
                    canvas.vline(
                        cursor_x,
                        rect.y + 8,
                        rect.h.saturating_sub(16),
                        theme.accent,
                    );
                }
            } else {
                canvas.draw_text(text_x, rect.y + (rect.h as i32 - 10) / 2, value, theme.text);
                if self.active {
                    let cursor_x = text_x + (self.cursor as i32) * 6;
                    canvas.vline(
                        cursor_x,
                        rect.y + 6,
                        rect.h.saturating_sub(12),
                        theme.accent,
                    );
                }
            }
        }
    }
}

impl<const N: usize> Default for BoundedSearchField<N> {
    fn default() -> Self {
        Self::new()
    }
}

/// Visual state of one result row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchResultState {
    Normal,
    Hovered,
    Selected,
}

/// Plain view model for a single result row.
#[derive(Debug, Clone, Copy)]
pub struct SearchResultView<'a> {
    pub title: &'a str,
    pub subtitle: Option<&'a str>,
    pub icon: Option<&'a TgaImage>,
    pub state: SearchResultState,
}

/// One result row drawn inside the palette list.
pub struct SearchResultRow<'a> {
    pub rect: Rect,
    pub view: SearchResultView<'a>,
}

impl<'a> SearchResultRow<'a> {
    pub const HEIGHT: u32 = 44;
    pub const ICON: u32 = 28;

    pub fn draw(&self, canvas: &mut Canvas, theme: &Theme, fonts: SearchPaletteFonts<'_>) {
        let radius = 8u32;
        match self.view.state {
            SearchResultState::Selected => {
                canvas.fill_rounded_rect(self.rect, radius, theme.chrome.selection);
                canvas.stroke_rounded_rect(self.rect, radius, 1, theme.accent);
            }
            SearchResultState::Hovered => {
                canvas.fill_rounded_rect(self.rect, radius, theme.panel_alt.lighten(8));
            }
            SearchResultState::Normal => {}
        }

        let icon_x = self.rect.x + 10;
        let icon_y = self.rect.y + (self.rect.h as i32 - Self::ICON as i32) / 2;
        let icon_rect = Rect::new(icon_x, icon_y, Self::ICON, Self::ICON);
        if let Some(icon) = self.view.icon {
            canvas.draw_tga_icon(icon, icon_rect);
        } else {
            canvas.fill_rounded_rect(icon_rect, 6, theme.panel_alt);
            canvas.stroke_rounded_rect(icon_rect, 6, 1, theme.border);
        }

        let text_x = icon_rect.right() + 10;
        let selected = self.view.state == SearchResultState::Selected;
        let title_color = if selected { theme.text } else { theme.text_dim };
        let title_font = if selected {
            fonts.medium.or(fonts.regular)
        } else {
            fonts.regular
        };
        // Title + subtitle band inside the row.
        let title_band_h = if self.view.subtitle.is_some() {
            20
        } else {
            self.rect.h
        };
        let title_y = if self.view.subtitle.is_some() {
            self.rect.y + 4
        } else {
            self.rect.y
        };
        if let Some(f) = title_font {
            f.draw_vcenter(
                canvas,
                self.view.title,
                text_x,
                title_y,
                title_band_h,
                title_color,
            );
        } else {
            canvas.draw_text(text_x, self.rect.y + 8, self.view.title, title_color);
        }
        if let Some(sub) = self.view.subtitle {
            if let Some(f) = fonts.small.or(fonts.regular) {
                f.draw_vcenter(canvas, sub, text_x, self.rect.y + 22, 18, theme.text_muted);
            } else {
                canvas.draw_text(text_x, self.rect.y + 24, sub, theme.text_muted);
            }
        }
    }
}

/// Soft ambient contact shadow (no Solar Focus Glow).
pub fn draw_palette_ambient_shadow(canvas: &mut Canvas, panel: Rect, radius: u32) {
    let layers = [
        (6i32, crate::theme::Color::rgba(0, 0, 0, 18)),
        (3, crate::theme::Color::rgba(0, 0, 0, 28)),
        (1, crate::theme::Color::rgba(0, 0, 0, 40)),
    ];
    for (expand, color) in layers {
        let r = Rect::new(
            panel.x - expand + 1,
            panel.y - expand / 2 + 3,
            panel.w.saturating_add((expand * 2) as u32),
            panel.h.saturating_add(expand as u32),
        );
        canvas.blend_rounded_rect(r, radius.saturating_add(expand as u32), color);
    }
}

/// Layout for a centered non-fullscreen search palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchPaletteLayout {
    pub panel: Rect,
    pub input: Rect,
    pub list: Rect,
    pub page_dots: [Rect; SEARCH_PAGE_DOT_CAP],
    pub page_dot_count: usize,
    pub footer: Rect,
    pub rows: [Rect; SEARCH_PAGE_ROWS],
    pub row_count: usize,
}

impl SearchPaletteLayout {
    pub const PANEL_W: u32 = 520;
    pub const PANEL_H_MAX: u32 = 420;
    pub const PAD: i32 = 14;
    pub const INPUT_H: u32 = 40;
    pub const FOOTER_H: u32 = 22;
    pub const RADIUS: u32 = 14;
    pub const PAGE_DOT: u32 = 8;
    pub const PAGE_DOT_GAP: i32 = 10;

    pub fn compute(screen_w: u32, screen_h: u32, visible_rows: usize, page_count: usize) -> Self {
        let panel_w = Self::PANEL_W.min(screen_w.saturating_sub(24)).max(280);
        let rows = visible_rows.min(SEARCH_PAGE_ROWS).max(1);
        let list_h = (rows as u32)
            .saturating_mul(SearchResultRow::HEIGHT)
            .saturating_add(rows.saturating_sub(1) as u32 * 4);

        let dots_h = if page_count > 1 {
            Self::PAGE_DOT + 10
        } else {
            0
        };
        let content_h =
            Self::INPUT_H + 10 + list_h + dots_h + 8 + Self::FOOTER_H + (Self::PAD as u32) * 2;
        let panel_h = content_h
            .min(Self::PANEL_H_MAX)
            .min(screen_h.saturating_sub(48));

        let panel_x = ((screen_w as i32 - panel_w as i32) / 2).max(8);
        let panel_y = ((screen_h as i32 - panel_h as i32) / 2).max(8);
        let panel = Rect::new(panel_x, panel_y, panel_w, panel_h);

        let input = Rect::new(
            panel.x + Self::PAD,
            panel.y + Self::PAD,
            panel.w.saturating_sub((Self::PAD * 2) as u32),
            Self::INPUT_H,
        );
        let list = Rect::new(
            panel.x + Self::PAD,
            input.bottom() + 10,
            panel.w.saturating_sub((Self::PAD * 2) as u32),
            list_h,
        );

        let mut page_dots = [Rect::new(0, 0, 0, 0); SEARCH_PAGE_DOT_CAP];
        let page_dot_count = page_count.min(SEARCH_PAGE_DOT_CAP);
        if page_dot_count > 1 {
            let dots_w = page_dot_count as i32 * Self::PAGE_DOT as i32
                + (page_dot_count.saturating_sub(1) as i32) * Self::PAGE_DOT_GAP;
            let mut dot_x = panel.x + (panel.w as i32 - dots_w) / 2;
            let dot_y = list.bottom() + 8;
            for i in 0..page_dot_count {
                page_dots[i] = Rect::new(dot_x, dot_y, Self::PAGE_DOT, Self::PAGE_DOT);
                dot_x += Self::PAGE_DOT as i32 + Self::PAGE_DOT_GAP;
            }
        }

        let footer_y = if page_dot_count > 1 {
            list.bottom() + 8 + Self::PAGE_DOT as i32 + 6
        } else {
            panel.bottom() - Self::PAD - Self::FOOTER_H as i32
        };
        let footer = Rect::new(
            panel.x + Self::PAD,
            footer_y.min(panel.bottom() - Self::PAD - Self::FOOTER_H as i32),
            panel.w.saturating_sub((Self::PAD * 2) as u32),
            Self::FOOTER_H,
        );

        let mut row_rects = [Rect::new(0, 0, 0, 0); SEARCH_PAGE_ROWS];
        let mut y = list.y;
        for i in 0..rows {
            row_rects[i] = Rect::new(list.x, y, list.w, SearchResultRow::HEIGHT);
            y += SearchResultRow::HEIGHT as i32 + 4;
        }

        Self {
            panel,
            input,
            list,
            page_dots,
            page_dot_count,
            footer,
            rows: row_rects,
            row_count: rows,
        }
    }

    pub fn contains(&self, point: Point) -> bool {
        self.panel.contains(point)
    }

    pub fn row_index_at(&self, point: Point) -> Option<usize> {
        self.rows[..self.row_count]
            .iter()
            .enumerate()
            .find(|(_, r)| r.contains(point))
            .map(|(i, _)| i)
    }

    pub fn page_dot_at(&self, point: Point) -> Option<usize> {
        self.page_dots[..self.page_dot_count]
            .iter()
            .enumerate()
            .find(|(_, r)| r.contains(point))
            .map(|(i, _)| i)
    }
}

/// Full palette chrome + rows. Caller owns field text and result models.
pub struct SearchPalettePanel<'a> {
    pub layout: SearchPaletteLayout,
    pub field: &'a BoundedSearchField,
    pub results: &'a [SearchResultView<'a>],
    pub empty_label: Option<&'a str>,
    pub footer_hint: &'a str,
    pub status: Option<&'a str>,
    pub active_page: usize,
    pub fonts: SearchPaletteFonts<'a>,
}

impl<'a> SearchPalettePanel<'a> {
    pub fn draw(&self, canvas: &mut Canvas, theme: &Theme) {
        draw_palette_ambient_shadow(canvas, self.layout.panel, SearchPaletteLayout::RADIUS);
        canvas.fill_material(
            self.layout.panel,
            Material::for_role(SurfaceRole::SystemOverlay, theme)
                .with_radius(SearchPaletteLayout::RADIUS),
        );

        self.field.draw(
            canvas,
            theme,
            self.layout.input,
            "Search applications…",
            self.fonts.regular,
        );

        if self.results.is_empty() {
            if let Some(label) = self.empty_label {
                let empty = Rect::new(
                    self.layout.list.x,
                    self.layout.list.y + 8,
                    self.layout.list.w,
                    24,
                );
                if let Some(f) = self.fonts.small.or(self.fonts.regular) {
                    let tw = f.measure_w(label) as i32;
                    let x = empty.x + (empty.w as i32 - tw) / 2;
                    f.draw_vcenter(canvas, label, x, empty.y, empty.h, theme.text_muted);
                } else {
                    canvas.draw_text_centered(empty, label, theme.text_muted);
                }
            }
        } else {
            for (i, view) in self.results.iter().enumerate() {
                if i >= self.layout.row_count {
                    break;
                }
                SearchResultRow {
                    rect: self.layout.rows[i],
                    view: *view,
                }
                .draw(canvas, theme, self.fonts);
            }
        }

        // Start-menu style page dots.
        for (i, dot) in self.layout.page_dots[..self.layout.page_dot_count]
            .iter()
            .enumerate()
        {
            if i == self.active_page {
                canvas.fill_rounded_rect(*dot, SearchPaletteLayout::PAGE_DOT / 2, theme.accent);
            } else {
                canvas.stroke_rounded_rect(
                    *dot,
                    SearchPaletteLayout::PAGE_DOT / 2,
                    1,
                    theme.text_dim,
                );
            }
        }

        let footer_text = self.status.unwrap_or(self.footer_hint);
        if let Some(f) = self.fonts.small.or(self.fonts.regular) {
            f.draw_vcenter(
                canvas,
                footer_text,
                self.layout.footer.x,
                self.layout.footer.y,
                self.layout.footer.h,
                theme.text_muted,
            );
        } else {
            canvas.draw_text(
                self.layout.footer.x,
                self.layout.footer.y + 4,
                footer_text,
                theme.text_muted,
            );
        }
    }
}

/// How many pages are needed for `item_count` with `page_capacity` slots.
pub fn search_page_count(item_count: usize, page_capacity: usize) -> usize {
    if item_count == 0 || page_capacity == 0 {
        0
    } else {
        item_count.div_ceil(page_capacity)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_bounds_query_length() {
        let mut f = BoundedSearchField::<8>::new();
        for b in b"abcdefghij" {
            let _ = f.insert(*b);
        }
        assert_eq!(f.value().len(), 8);
        assert_eq!(f.value(), "abcdefgh");
    }

    #[test]
    fn layout_is_centered_and_in_bounds() {
        for (w, h) in [(1366u32, 768u32), (1920, 1080), (1024, 768)] {
            let layout = SearchPaletteLayout::compute(w, h, 6, 3);
            assert!(layout.panel.w <= w);
            assert!(layout.panel.h <= h);
            assert!(layout.panel.x >= 0);
            assert!(layout.panel.y >= 0);
            assert!(layout.panel.right() <= w as i32);
            assert!(layout.panel.bottom() <= h as i32);
            assert_eq!(layout.page_dot_count, 3);
            let cx = layout.panel.x + layout.panel.w as i32 / 2;
            let cy = layout.panel.y + layout.panel.h as i32 / 2;
            assert!((cx - w as i32 / 2).abs() < 40);
            assert!((cy - h as i32 / 2).abs() < 80);
        }
    }

    #[test]
    fn row_and_dot_hit_tests() {
        let layout = SearchPaletteLayout::compute(1366, 768, 4, 2);
        for i in 0..4 {
            let r = layout.rows[i];
            assert_eq!(layout.row_index_at(Point::new(r.x + 2, r.y + 2)), Some(i));
        }
        assert_eq!(
            layout.page_dot_at(Point::new(
                layout.page_dots[0].x + 1,
                layout.page_dots[0].y + 1
            )),
            Some(0)
        );
        assert_eq!(layout.row_index_at(Point::new(0, 0)), None);
    }

    #[test]
    fn page_count_math() {
        assert_eq!(search_page_count(0, 6), 0);
        assert_eq!(search_page_count(6, 6), 1);
        assert_eq!(search_page_count(15, 6), 3);
    }
}
