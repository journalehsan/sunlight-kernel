//! Toolbar — horizontal row of labeled action buttons.

use crate::geom::Rect;
use crate::paint::Canvas;
use crate::theme::Theme;
use crate::widgets::button::{Button, ButtonState};

pub struct ToolbarItem<'a> {
    pub label: &'a str,
    pub state: ButtonState,
    /// Visually highlighted (e.g. active mode toggle)
    pub active: bool,
}

pub struct Toolbar<'a> {
    pub rect: Rect,
    pub items: &'a [ToolbarItem<'a>],
    pub item_width: u32,
}

impl<'a> Toolbar<'a> {
    pub fn new(rect: Rect, items: &'a [ToolbarItem<'a>], item_width: u32) -> Self {
        Self {
            rect,
            items,
            item_width,
        }
    }

    fn item_rect(&self, idx: usize) -> Rect {
        Rect::new(
            self.rect.x + (idx as u32 * self.item_width) as i32,
            self.rect.y,
            self.item_width,
            self.rect.h,
        )
    }

    pub fn draw(&self, canvas: &mut Canvas, theme: &Theme) {
        canvas.fill_rect(self.rect, theme.panel);
        canvas.hbar(
            self.rect.x,
            self.rect.bottom() - 1,
            self.rect.w,
            1,
            theme.border,
        );

        for (i, item) in self.items.iter().enumerate() {
            let r = self.item_rect(i);
            let mut btn = if item.active {
                Button::new(r, item.label)
            } else {
                Button::secondary(r, item.label)
            };
            btn.state = item.state;
            btn.draw(canvas, theme);
        }
    }

    pub fn hit_test(&self, x: i32, y: i32) -> Option<usize> {
        for i in 0..self.items.len() {
            if self.item_rect(i).contains(crate::geom::Point::new(x, y)) {
                return Some(i);
            }
        }
        None
    }
}
