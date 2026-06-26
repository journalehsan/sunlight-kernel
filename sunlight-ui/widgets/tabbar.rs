//! TabBar widget — horizontal row of named tabs.

use crate::geom::Rect;
use crate::paint::Canvas;
use crate::theme::Theme;

pub struct TabBar<'a> {
    pub rect: Rect,
    pub tabs: &'a [&'a str],
    pub active: usize,
    pub hovered: Option<usize>,
    /// Width of each tab. If None, tabs share space equally.
    pub tab_width: Option<u32>,
}

impl<'a> TabBar<'a> {
    pub fn new(rect: Rect, tabs: &'a [&'a str], active: usize) -> Self {
        Self {
            rect,
            tabs,
            active,
            hovered: None,
            tab_width: None,
        }
    }

    fn tab_rect(&self, idx: usize) -> Rect {
        let n = self.tabs.len() as u32;
        let w = self.tab_width.unwrap_or_else(|| self.rect.w / n.max(1));
        Rect::new(
            self.rect.x + (idx as u32 * w) as i32,
            self.rect.y,
            w,
            self.rect.h,
        )
    }

    pub fn draw(&self, canvas: &mut Canvas, theme: &Theme) {
        // Background strip
        canvas.fill_rect(self.rect, theme.panel);
        // Bottom separator
        canvas.hbar(
            self.rect.x,
            self.rect.bottom() - 1,
            self.rect.w,
            1,
            theme.border,
        );

        for (i, &label) in self.tabs.iter().enumerate() {
            let r = self.tab_rect(i);
            let is_active = i == self.active;
            let is_hovered = self.hovered == Some(i);

            let bg = if is_active {
                theme.panel_alt
            } else if is_hovered {
                theme.panel.lighten(10)
            } else {
                theme.panel
            };

            canvas.fill_rect(r, bg);

            let text_color = if is_active {
                theme.accent
            } else {
                theme.text_dim
            };
            canvas.draw_text_centered(r, label, text_color);

            // Active tab: accent underline
            if is_active {
                canvas.hbar(r.x, r.bottom() - 2, r.w, 2, theme.accent);
            }

            // Right divider between tabs
            if i + 1 < self.tabs.len() {
                canvas.vline(r.right() - 1, r.y + 4, r.h.saturating_sub(8), theme.border);
            }
        }
    }

    /// Returns the tab index at pixel position `(x, y)`, if any.
    pub fn hit_test(&self, x: i32, y: i32) -> Option<usize> {
        if !self.rect.contains(crate::geom::Point::new(x, y)) {
            return None;
        }
        for i in 0..self.tabs.len() {
            if self.tab_rect(i).contains(crate::geom::Point::new(x, y)) {
                return Some(i);
            }
        }
        None
    }
}
