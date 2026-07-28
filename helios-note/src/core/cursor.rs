//! Cursor and viewport tracking with boundary clamping.

use super::buffer::TextBuffer;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor {
    pub line: usize,
    pub col: usize,
    pub scroll_y: usize,
    pub scroll_x: usize,
}

impl Default for Cursor {
    fn default() -> Self {
        Self::new()
    }
}

impl Cursor {
    pub fn new() -> Self {
        Self {
            line: 0,
            col: 0,
            scroll_y: 0,
            scroll_x: 0,
        }
    }

    /// Move cursor up by 1 line.
    pub fn move_up(&mut self, buf: &TextBuffer) {
        if self.line > 0 {
            self.line -= 1;
            self.clamp_col(buf);
        }
    }

    /// Move cursor down by 1 line.
    pub fn move_down(&mut self, buf: &TextBuffer) {
        if self.line + 1 < buf.len() {
            self.line += 1;
            self.clamp_col(buf);
        }
    }

    /// Move cursor left by 1 character. If at col 0, wrap to end of previous line.
    pub fn move_left(&mut self, buf: &TextBuffer) {
        if self.col > 0 {
            self.col -= 1;
        } else if self.line > 0 {
            self.line -= 1;
            self.col = buf.line_char_count(self.line);
        }
    }

    /// Move cursor right by 1 character. If at end of line, wrap to start of next line.
    pub fn move_right(&mut self, buf: &TextBuffer) {
        let max_col = buf.line_char_count(self.line);
        if self.col < max_col {
            self.col += 1;
        } else if self.line + 1 < buf.len() {
            self.line += 1;
            self.col = 0;
        }
    }

    /// Move cursor to start of line (Home).
    pub fn move_home(&mut self) {
        self.col = 0;
    }

    /// Move cursor to end of line (End).
    pub fn move_end(&mut self, buf: &TextBuffer) {
        self.col = buf.line_char_count(self.line);
    }

    /// Page Up navigation by `view_height` lines.
    pub fn page_up(&mut self, buf: &TextBuffer, view_height: usize) {
        self.line = self.line.saturating_sub(view_height);
        self.clamp_col(buf);
    }

    /// Page Down navigation by `view_height` lines.
    pub fn page_down(&mut self, buf: &TextBuffer, view_height: usize) {
        self.line = (self.line + view_height).min(buf.len().saturating_sub(1));
        self.clamp_col(buf);
    }

    /// Move to top of file (Ctrl+Home).
    pub fn move_top(&mut self) {
        self.line = 0;
        self.col = 0;
    }

    /// Move to bottom of file (Ctrl+End).
    pub fn move_bottom(&mut self, buf: &TextBuffer) {
        self.line = buf.len().saturating_sub(1);
        self.col = buf.line_char_count(self.line);
    }

    /// Ensure cursor column is within valid bounds of current line.
    pub fn clamp_col(&mut self, buf: &TextBuffer) {
        let max_col = buf.line_char_count(self.line);
        if self.col > max_col {
            self.col = max_col;
        }
    }

    /// Synchronize viewport scroll offsets (`scroll_y`, `scroll_x`) with cursor position.
    pub fn adjust_viewport(
        &mut self,
        view_width: usize,
        view_height: usize,
        buf: &TextBuffer,
        tab_width: usize,
    ) {
        if view_height == 0 || view_width == 0 {
            return;
        }

        // Vertical scroll
        if self.line < self.scroll_y {
            self.scroll_y = self.line;
        } else if self.line >= self.scroll_y + view_height {
            self.scroll_y = self.line - view_height + 1;
        }

        // Horizontal scroll based on visual column
        let vcol = buf.visual_col(self.line, self.col, tab_width);
        if vcol < self.scroll_x {
            self.scroll_x = vcol;
        } else if vcol >= self.scroll_x + view_width {
            self.scroll_x = vcol - view_width + 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::buffer::TextBuffer;

    #[test]
    fn cursor_navigation_bounds() {
        let buf = TextBuffer::from_str("line1\nline22\nline333");
        let mut cursor = Cursor::new();

        cursor.move_down(&buf);
        assert_eq!(cursor.line, 1);

        cursor.move_end(&buf);
        assert_eq!(cursor.col, 6);

        cursor.move_up(&buf);
        assert_eq!(cursor.line, 0);
        // Column should be clamped to line1 length (5)
        assert_eq!(cursor.col, 5);
    }
}
