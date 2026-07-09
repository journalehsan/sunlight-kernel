use crate::event::Event;
use crate::font::VecText;
use crate::geom::{Point, Rect};
use crate::paint::Canvas;
use crate::theme::Theme;

pub struct TextInput<'a, const N: usize> {
    pub rect: Rect,
    pub active: bool,
    len: usize,
    cursor: usize,
    buf: [u8; N],
    font: Option<&'a dyn VecText>,
    placeholder: Option<&'a str>,
}

impl<'a, const N: usize> TextInput<'a, N> {
    pub const fn new(rect: Rect) -> Self {
        Self {
            rect,
            active: false,
            len: 0,
            cursor: 0,
            buf: [0; N],
            font: None,
            placeholder: None,
        }
    }

    pub fn with_font(mut self, font: &'a dyn VecText) -> Self {
        self.font = Some(font);
        self
    }

    pub fn with_placeholder(mut self, placeholder: &'a str) -> Self {
        self.placeholder = Some(placeholder);
        self
    }

    pub fn value(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }

    pub fn set_text(&mut self, text: &str) {
        self.len = text.len().min(N);
        self.cursor = self.len;
        self.buf[..self.len].copy_from_slice(&text.as_bytes()[..self.len]);
    }

    pub fn draw(&self, canvas: &mut Canvas, theme: &Theme) {
        canvas.fill_rect(self.rect, theme.panel);
        canvas.draw_rect(
            self.rect,
            if self.active {
                theme.accent
            } else {
                theme.border
            },
        );

        let text_x = self.rect.x + 6;
        let show_placeholder = self.len == 0 && !self.active;
        let visible = if show_placeholder {
            self.placeholder.unwrap_or("")
        } else {
            self.visible_text()
        };
        let prefix_len = self.visible_prefix_len(visible);
        let prefix = visible.get(..prefix_len).unwrap_or("");
        let text_color = if show_placeholder {
            theme.text_dim
        } else {
            theme.text
        };

        if let Some(f) = self.font {
            f.draw_vcenter(
                canvas,
                visible,
                text_x,
                self.rect.y,
                self.rect.h,
                text_color,
            );
            if self.active {
                let cursor_x = text_x + f.measure_w(prefix) as i32;
                canvas.vline(
                    cursor_x,
                    self.rect.y + 4,
                    self.rect.h.saturating_sub(8),
                    theme.accent,
                );
            }
        } else {
            let text_y = self.rect.y + (self.rect.h as i32 - 10) / 2;
            canvas.draw_text(text_x, text_y, visible, text_color);
            if self.active {
                let cursor_x = text_x + Canvas::measure_text(prefix) as i32;
                canvas.vline(
                    cursor_x,
                    self.rect.y + 4,
                    self.rect.h.saturating_sub(8),
                    theme.accent,
                );
            }
        }
    }

    fn visible_text(&self) -> &str {
        let value = self.value();
        let max_chars = self.max_visible_chars();
        if value.chars().count() <= max_chars {
            return value;
        }
        let cursor_chars = value[..self.cursor].chars().count();
        let total_chars = value.chars().count();
        let mut start_chars = cursor_chars.saturating_sub(max_chars.saturating_sub(1));
        if start_chars + max_chars > total_chars {
            start_chars = total_chars.saturating_sub(max_chars);
        }
        let start_byte = nth_char_byte(value, start_chars);
        let end_byte = nth_char_byte(value, (start_chars + max_chars).min(total_chars));
        &value[start_byte..end_byte]
    }

    fn visible_prefix_len(&self, visible: &str) -> usize {
        let value = self.value();
        let visible_start = visible.as_ptr() as usize - value.as_ptr() as usize;
        self.cursor.saturating_sub(visible_start).min(visible.len())
    }

    fn max_visible_chars(&self) -> usize {
        let inner_w = self.rect.w.saturating_sub(12);
        let glyph_w = 8usize;
        ((inner_w as usize) / glyph_w).max(1)
    }

    pub fn update(&mut self, event: Event) -> bool {
        match event {
            Event::Click { x, y } => {
                self.active = self.rect.contains(Point::new(x, y));
                self.active
            }
            Event::Key(ch) if self.active => match ch {
                '\u{8}' => self.backspace(),
                '\n' => false,
                c if c.is_ascii_graphic() || c == ' ' => self.insert(c as u8),
                _ => false,
            },
            Event::KeyPress {
                keycode,
                pressed: true,
                ..
            } if self.active => match keycode {
                0x4B => self.move_left(),
                0x4D => self.move_right(),
                0x47 => self.move_home(),
                0x4F => self.move_end(),
                0x53 => self.delete_forward(),
                _ => false,
            },
            _ => false,
        }
    }

    fn insert(&mut self, byte: u8) -> bool {
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

    fn backspace(&mut self) -> bool {
        if self.cursor == 0 || self.len == 0 {
            return false;
        }
        let start = self.cursor - 1;
        let mut i = start;
        while i + 1 < self.len {
            self.buf[i] = self.buf[i + 1];
            i += 1;
        }
        self.len -= 1;
        self.cursor -= 1;
        true
    }

    fn delete_forward(&mut self) -> bool {
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

    fn move_left(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        self.cursor -= 1;
        true
    }

    fn move_right(&mut self) -> bool {
        if self.cursor >= self.len {
            return false;
        }
        self.cursor += 1;
        true
    }

    fn move_home(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        self.cursor = 0;
        true
    }

    fn move_end(&mut self) -> bool {
        if self.cursor == self.len {
            return false;
        }
        self.cursor = self.len;
        true
    }
}

fn nth_char_byte(text: &str, char_index: usize) -> usize {
    if char_index == 0 {
        return 0;
    }
    text.char_indices()
        .nth(char_index)
        .map(|(idx, _)| idx)
        .unwrap_or(text.len())
}
