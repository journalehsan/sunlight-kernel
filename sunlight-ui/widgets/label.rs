use crate::font::VecText;
use crate::geom::Rect;
use crate::paint::Canvas;
use crate::theme::Theme;

pub struct Label<'a> {
    pub rect: Rect,
    pub text: &'a str,
    pub dim: bool,
    font: Option<&'a dyn VecText>,
}

impl<'a> Label<'a> {
    pub const fn new(rect: Rect, text: &'a str) -> Self {
        Self {
            rect,
            text,
            dim: false,
            font: None,
        }
    }

    pub const fn dim(mut self) -> Self {
        self.dim = true;
        self
    }

    pub fn with_font(mut self, font: &'a dyn VecText) -> Self {
        self.font = Some(font);
        self
    }

    pub fn draw(&self, canvas: &mut Canvas, theme: &Theme) {
        let color = if self.dim { theme.text_dim } else { theme.text };
        if let Some(f) = self.font {
            f.draw_vcenter(canvas, self.text, self.rect.x, self.rect.y, self.rect.h, color);
        } else {
            let text_y = self.rect.y + (self.rect.h as i32 - 10) / 2;
            canvas.draw_text(self.rect.x, text_y, self.text, color);
        }
    }
}
