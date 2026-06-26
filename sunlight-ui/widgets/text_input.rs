use crate::event::Event;
use crate::geom::{Point, Rect};
use crate::paint::Canvas;
use crate::theme::Theme;

pub struct TextInput<const N: usize> {
    pub rect: Rect,
    pub active: bool,
    len: usize,
    cursor: usize,
    buf: [u8; N],
}

impl<const N: usize> TextInput<N> {
    pub const fn new(rect: Rect) -> Self {
        Self {
            rect,
            active: false,
            len: 0,
            cursor: 0,
            buf: [0; N],
        }
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
        let text_y = self.rect.y + (self.rect.h as i32 - 10) / 2;
        canvas.draw_text(text_x, text_y, self.value(), theme.text);

        if self.active {
            let cursor_x =
                text_x + Canvas::measure_text(self.value().get(..self.cursor).unwrap_or("")) as i32;
            canvas.vline(
                cursor_x,
                self.rect.y + 4,
                self.rect.h.saturating_sub(8),
                theme.accent,
            );
        }
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
