use crate::event::Event;
use crate::font::VecText;
use crate::geom::{Point, Rect};
use crate::paint::Canvas;
use crate::theme::Theme;

pub struct Checkbox<'a> {
    pub rect: Rect,
    pub label: &'a str,
    pub checked: bool,
    pub active: bool,
    /// Vector font for the label. Falls back to the 5×7 bitmap font when `None`.
    font: Option<&'a dyn VecText>,
}

impl<'a> Checkbox<'a> {
    pub const fn new(rect: Rect, label: &'a str) -> Self {
        Self {
            rect,
            label,
            checked: false,
            active: false,
            font: None,
        }
    }

    /// Render the label with a Sunlight vector font (MiniType).
    pub fn with_font(mut self, font: &'a dyn VecText) -> Self {
        self.font = Some(font);
        self
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
        if let Some(font) = self.font {
            font.draw_vcenter(
                canvas,
                self.label,
                text_rect.x,
                text_rect.y,
                text_rect.h,
                theme.text,
            );
        } else {
            let text_y = text_rect.y + (text_rect.h as i32 - 10) / 2;
            canvas.draw_text(text_rect.x, text_y, self.label, theme.text);
        }
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
