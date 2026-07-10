use sunlight_ui::event::Event;
use sunlight_ui::font::VecText;
use sunlight_ui::geom::{Point, Rect};
use sunlight_ui::paint::Canvas;
use sunlight_ui::theme::Theme;

pub struct TextArea<'a, const N: usize> {
    pub rect: Rect,
    pub active: bool,
    len: usize,
    cursor: usize,
    scroll_line: usize,
    buf: [u8; N],
    font: Option<&'a dyn VecText>,
    placeholder: Option<&'a str>,
}

impl<'a, const N: usize> TextArea<'a, N> {
    pub const fn new(rect: Rect) -> Self {
        Self {
            rect,
            active: false,
            len: 0,
            cursor: 0,
            scroll_line: 0,
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
        let bytes = text.as_bytes();
        self.len = bytes.len().min(N);
        self.cursor = self.len;
        self.scroll_line = 0;
        self.buf[..self.len].copy_from_slice(&bytes[..self.len]);
        self.ensure_cursor_visible();
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

        let inner = self.rect.inset(6);
        let line_h = self.line_height();
        let visible_lines = ((inner.h / line_h.max(1)).max(1)) as usize;
        let visible_cols = self.visible_cols(inner.w);

        if self.len == 0 && !self.active {
            let placeholder = self.placeholder.unwrap_or("");
            self.draw_line(canvas, placeholder, inner.x, inner.y, theme.text_dim);
            return;
        }

        let (cursor_line, cursor_col) = self.cursor_line_col();
        let horizontal_offset = cursor_col.saturating_sub(visible_cols.saturating_sub(1));

        let mut line_index = 0usize;
        let mut drawn = 0usize;
        for line in self.value().split('\n') {
            if line_index < self.scroll_line {
                line_index += 1;
                continue;
            }
            if drawn >= visible_lines {
                break;
            }

            let y = inner.y + (drawn as u32 * line_h) as i32;
            let visible = visible_window(line, horizontal_offset, visible_cols);
            self.draw_line(canvas, visible, inner.x, y, theme.text);
            line_index += 1;
            drawn += 1;
        }

        if self.active
            && cursor_line >= self.scroll_line
            && cursor_line < self.scroll_line + visible_lines
        {
            let cursor_y = inner.y + ((cursor_line - self.scroll_line) as u32 * line_h) as i32;
            let cursor_x =
                inner.x + self.measure_text_width(cursor_col.saturating_sub(horizontal_offset));
            canvas.vline(cursor_x, cursor_y, line_h.saturating_sub(2), theme.accent);
        }
    }

    pub fn update(&mut self, event: Event) -> bool {
        match event {
            Event::Click { x, y } => {
                self.active = self.rect.contains(Point::new(x, y));
                if self.active {
                    self.cursor = self.len;
                    self.ensure_cursor_visible();
                }
                self.active
            }
            Event::Key(ch) if self.active => match ch {
                '\u{8}' => self.backspace(),
                '\n' => self.insert_byte(b'\n'),
                '\t' => self.insert_tab(),
                c if c.is_ascii_graphic() || c == ' ' => self.insert_byte(c as u8),
                _ => false,
            },
            Event::KeyPress {
                keycode,
                pressed: true,
                ..
            } if self.active => match keycode {
                0x48 => self.move_vertical(true),
                0x50 => self.move_vertical(false),
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

    fn insert_tab(&mut self) -> bool {
        let mut changed = false;
        for _ in 0..4 {
            changed |= self.insert_byte(b' ');
        }
        changed
    }

    fn insert_byte(&mut self, byte: u8) -> bool {
        if self.len >= N {
            return false;
        }
        let mut index = self.len;
        while index > self.cursor {
            self.buf[index] = self.buf[index - 1];
            index -= 1;
        }
        self.buf[self.cursor] = byte;
        self.len += 1;
        self.cursor += 1;
        self.ensure_cursor_visible();
        true
    }

    fn backspace(&mut self) -> bool {
        if self.cursor == 0 || self.len == 0 {
            return false;
        }
        let mut index = self.cursor - 1;
        while index + 1 < self.len {
            self.buf[index] = self.buf[index + 1];
            index += 1;
        }
        self.len -= 1;
        self.cursor -= 1;
        self.ensure_cursor_visible();
        true
    }

    fn delete_forward(&mut self) -> bool {
        if self.cursor >= self.len {
            return false;
        }
        let mut index = self.cursor;
        while index + 1 < self.len {
            self.buf[index] = self.buf[index + 1];
            index += 1;
        }
        self.len -= 1;
        self.ensure_cursor_visible();
        true
    }

    fn move_left(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        self.cursor -= 1;
        self.ensure_cursor_visible();
        true
    }

    fn move_right(&mut self) -> bool {
        if self.cursor >= self.len {
            return false;
        }
        self.cursor += 1;
        self.ensure_cursor_visible();
        true
    }

    fn move_home(&mut self) -> bool {
        let line_start = self.current_line_start();
        if self.cursor == line_start {
            return false;
        }
        self.cursor = line_start;
        self.ensure_cursor_visible();
        true
    }

    fn move_end(&mut self) -> bool {
        let line_end = self.current_line_end();
        if self.cursor == line_end {
            return false;
        }
        self.cursor = line_end;
        self.ensure_cursor_visible();
        true
    }

    fn move_vertical(&mut self, up: bool) -> bool {
        let current_start = self.current_line_start();
        let current_column = self.cursor - current_start;

        let target_start = if up {
            if current_start == 0 {
                return false;
            }
            previous_line_start(self.value(), current_start)
        } else {
            let current_end = self.current_line_end();
            if current_end >= self.len {
                return false;
            }
            current_end + 1
        };

        let target_end = line_end_from(self.value(), target_start);
        self.cursor = target_start + current_column.min(target_end - target_start);
        self.ensure_cursor_visible();
        true
    }

    fn current_line_start(&self) -> usize {
        line_start_for_cursor(self.value(), self.cursor)
    }

    fn current_line_end(&self) -> usize {
        line_end_from(self.value(), self.current_line_start())
    }

    fn cursor_line_col(&self) -> (usize, usize) {
        let text = self.value().as_bytes();
        let mut line = 0usize;
        let mut col = 0usize;
        for &byte in &text[..self.cursor.min(text.len())] {
            if byte == b'\n' {
                line += 1;
                col = 0;
            } else {
                col += 1;
            }
        }
        (line, col)
    }

    fn ensure_cursor_visible(&mut self) {
        let visible_lines = ((self.rect.h.saturating_sub(12)) / self.line_height().max(1)).max(1);
        let cursor_line = self.cursor_line_col().0;
        if cursor_line < self.scroll_line {
            self.scroll_line = cursor_line;
        } else {
            let bottom = self.scroll_line + visible_lines as usize;
            if cursor_line >= bottom {
                self.scroll_line = cursor_line + 1 - visible_lines as usize;
            }
        }
    }

    fn draw_line(
        &self,
        canvas: &mut Canvas,
        text: &str,
        x: i32,
        y: i32,
        color: sunlight_ui::theme::Color,
    ) {
        if let Some(font) = self.font {
            font.draw(canvas, text, x, y, color);
        } else {
            canvas.draw_text(x, y, text, color);
        }
    }

    fn line_height(&self) -> u32 {
        self.font.map_or(14, |font| font.line_height() + 2)
    }

    fn visible_cols(&self, width: u32) -> usize {
        let glyph_w = if let Some(font) = self.font {
            font.measure_w("M").max(1)
        } else {
            8
        };
        (width / glyph_w.max(1)).max(1) as usize
    }

    fn measure_text_width(&self, cols: usize) -> i32 {
        let glyph_w = if let Some(font) = self.font {
            font.measure_w("M").max(1)
        } else {
            8
        };
        (cols as u32 * glyph_w) as i32
    }
}

fn line_start_for_cursor(text: &str, cursor: usize) -> usize {
    let bytes = text.as_bytes();
    let mut index = cursor.min(bytes.len());
    while index > 0 && bytes[index - 1] != b'\n' {
        index -= 1;
    }
    index
}

fn line_end_from(text: &str, start: usize) -> usize {
    let bytes = text.as_bytes();
    let mut index = start.min(bytes.len());
    while index < bytes.len() && bytes[index] != b'\n' {
        index += 1;
    }
    index
}

fn previous_line_start(text: &str, current_start: usize) -> usize {
    let bytes = text.as_bytes();
    let mut index = current_start.saturating_sub(1);
    while index > 0 && bytes[index - 1] != b'\n' {
        index -= 1;
    }
    index
}

fn visible_window(line: &str, offset: usize, width: usize) -> &str {
    let start = offset.min(line.len());
    let end = (start + width).min(line.len());
    &line[start..end]
}
