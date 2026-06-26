use crate::event::Event;
use crate::geom::{Point, Rect};
use crate::paint::Canvas;
use crate::theme::Theme;

pub struct Checkbox<'a> {
    pub rect: Rect,
    pub label: &'a str,
    pub checked: bool,
    pub active: bool,
}

impl<'a> Checkbox<'a> {
    pub const fn new(rect: Rect, label: &'a str) -> Self {
        Self {
            rect,
            label,
            checked: false,
            active: false,
        }
    }

    pub fn draw(&self, canvas: &mut Canvas, theme: &Theme) {
        let box_rect = Rect::new(self.rect.x, self.rect.y + 2, 14, 14);
        canvas.fill_rect(
            box_rect,
            if self.active {
                theme.panel_alt
            } else {
                theme.panel
            },
        );
        canvas.draw_rect(
            box_rect,
            if self.active {
                theme.accent
            } else {
                theme.border
            },
        );

        if self.checked {
            canvas.hbar(box_rect.x + 3, box_rect.y + 7, 8, 2, theme.accent);
            canvas.vline(box_rect.x + 7, box_rect.y + 3, 8, theme.accent);
        }

        let text_rect = Rect::new(
            self.rect.x + 20,
            self.rect.y,
            self.rect.w.saturating_sub(20),
            self.rect.h,
        );
        let text_y = text_rect.y + (text_rect.h as i32 - 10) / 2;
        canvas.draw_text(text_rect.x, text_y, self.label, theme.text);
    }

    pub fn update(&mut self, event: Event) -> bool {
        match event {
            Event::Click { x, y } if self.rect.contains(Point::new(x, y)) => {
                self.checked = !self.checked;
                self.active = true;
                true
            }
            Event::Click { .. } => {
                self.active = false;
                false
            }
            _ => false,
        }
    }
}
