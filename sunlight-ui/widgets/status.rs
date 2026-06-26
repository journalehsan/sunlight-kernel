//! StatusBar — fixed-height bottom bar with left/center/right sections.

use crate::geom::Rect;
use crate::paint::Canvas;
use crate::theme::Theme;

pub struct StatusBar<'a> {
    pub rect:   Rect,
    pub left:   &'a str,
    pub center: &'a str,
    pub right:  &'a str,
}

impl<'a> StatusBar<'a> {
    pub const HEIGHT: u32 = 18;

    pub fn new(rect: Rect, left: &'a str, center: &'a str, right: &'a str) -> Self {
        Self { rect, left, center, right }
    }

    pub fn draw(&self, canvas: &mut Canvas, theme: &Theme) {
        canvas.fill_rect(self.rect, theme.panel_alt);
        // Top separator
        canvas.hbar(self.rect.x, self.rect.y, self.rect.w, 1, theme.border);

        let ty = self.rect.y + (Self::HEIGHT as i32 - 10) / 2;

        // Left
        canvas.draw_text(self.rect.x + 8, ty, self.left, theme.text_dim);

        // Center
        let cw = Canvas::measure_text(self.center);
        let cx = self.rect.x + (self.rect.w as i32 - cw as i32) / 2;
        canvas.draw_text(cx, ty, self.center, theme.text);

        // Right
        canvas.draw_text_right(self.rect, self.right, theme.text_dim, 8);
    }
}
