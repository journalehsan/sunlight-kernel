use crate::geom::Rect;
use crate::paint::Canvas;
use crate::theme::Theme;

pub struct Label<'a> {
    pub rect: Rect,
    pub text: &'a str,
    pub dim: bool,
}

impl<'a> Label<'a> {
    pub const fn new(rect: Rect, text: &'a str) -> Self {
        Self {
            rect,
            text,
            dim: false,
        }
    }

    pub const fn dim(mut self) -> Self {
        self.dim = true;
        self
    }

    pub fn draw(&self, canvas: &mut Canvas, theme: &Theme) {
        let color = if self.dim { theme.text_dim } else { theme.text };
        let text_y = self.rect.y + (self.rect.h as i32 - 10) / 2;
        canvas.draw_text(self.rect.x, text_y, self.text, color);
    }
}
