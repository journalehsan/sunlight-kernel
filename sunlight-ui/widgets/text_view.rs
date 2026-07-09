//! Read-only multiline text viewer with keyboard-style scrolling support.

use crate::font::VecText;
use crate::geom::{Point, Rect};
use crate::paint::Canvas;
use crate::theme::{Color, Theme};

pub struct TextView<'a> {
    pub rect: Rect,
    pub text: &'a str,
    pub scroll_offset: usize,
    pub focused: bool,
    pub line_h: u32,
    pub text_color: Option<Color>,
    font: Option<&'a dyn VecText>,
}

impl<'a> TextView<'a> {
    pub fn new(rect: Rect, text: &'a str) -> Self {
        Self {
            rect,
            text,
            scroll_offset: 0,
            focused: false,
            line_h: 14,
            text_color: None,
            font: None,
        }
    }

    pub fn with_scroll_offset(mut self, offset: usize) -> Self {
        self.scroll_offset = offset;
        self
    }

    pub fn with_focus(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    pub fn with_font(mut self, font: &'a dyn VecText) -> Self {
        self.font = Some(font);
        let min_line_h = font.line_height() + 2;
        if self.line_h < min_line_h {
            self.line_h = min_line_h;
        }
        self
    }

    pub fn with_text_color(mut self, color: Color) -> Self {
        self.text_color = Some(color);
        self
    }

    pub fn visible_line_count(&self) -> usize {
        ((self.rect.h.saturating_sub(8)) / self.line_h).max(1) as usize
    }

    pub fn max_scroll(&self) -> usize {
        self.line_count().saturating_sub(self.visible_line_count())
    }

    pub fn contains(&self, x: i32, y: i32) -> bool {
        self.rect.contains(Point::new(x, y))
    }

    pub fn draw(&self, canvas: &mut Canvas, theme: &Theme) {
        canvas.fill_rect(self.rect, theme.panel);
        canvas.draw_rect(
            self.rect,
            if self.focused {
                theme.accent
            } else {
                theme.border
            },
        );

        let mut line_idx = 0usize;
        let mut drawn = 0usize;
        let visible = self.visible_line_count();
        let inner_x = self.rect.x + 6;
        let inner_y = self.rect.y + 4;
        let inner_w = self.rect.w.saturating_sub(16);
        let text_color = self.text_color.unwrap_or(theme.text);

        for line in self.text.split('\n') {
            if line_idx < self.scroll_offset {
                line_idx += 1;
                continue;
            }
            if drawn >= visible {
                break;
            }

            let y = inner_y + (drawn as u32 * self.line_h) as i32;
            let clipped = self.clipped_line(line, inner_w);
            if let Some(font) = self.font {
                font.draw(canvas, clipped, inner_x, y, text_color);
            } else {
                canvas.draw_text(inner_x, y, clipped, text_color);
            }

            drawn += 1;
            line_idx += 1;
        }

        if self.max_scroll() > 0 {
            let marker_x = self.rect.right() - 8;
            if self.scroll_offset > 0 {
                canvas.fill_rect(Rect::new(marker_x, self.rect.y + 6, 3, 5), theme.text_dim);
            }
            if self.scroll_offset + visible < self.line_count() {
                canvas.fill_rect(
                    Rect::new(marker_x, self.rect.bottom() - 11, 3, 5),
                    theme.text_dim,
                );
            }
        }
    }

    fn line_count(&self) -> usize {
        self.text.bytes().filter(|byte| *byte == b'\n').count() + 1
    }

    fn clipped_line<'b>(&self, line: &'b str, width: u32) -> &'b str {
        if width == 0 {
            return "";
        }
        let max_chars = if let Some(font) = self.font {
            let glyph_w = font.measure_w("M").max(1);
            (width / glyph_w).max(1) as usize
        } else {
            ((width as usize) / 8).max(1)
        };
        if line.len() <= max_chars {
            return line;
        }

        let mut end = 0usize;
        for (count, (idx, ch)) in line.char_indices().enumerate() {
            if count >= max_chars {
                break;
            }
            end = idx + ch.len_utf8();
        }
        &line[..end]
    }
}
