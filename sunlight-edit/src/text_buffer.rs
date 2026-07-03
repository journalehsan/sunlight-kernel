//! Line-oriented UTF-8 text buffer for the SunlightOS text editor.

use alloc::string::String;
use alloc::vec::Vec;

pub struct TextBuffer {
    lines: Vec<String>,
    pub cursor_line: usize,
    /// Character column within the current line (UTF-8 safe).
    pub cursor_col: usize,
    dirty: bool,
}

fn char_index_to_byte(line: &str, char_idx: usize) -> usize {
    if char_idx == 0 {
        return 0;
    }
    line.char_indices()
        .nth(char_idx)
        .map(|(idx, _)| idx)
        .unwrap_or(line.len())
}

impl TextBuffer {
    pub fn new() -> Self {
        Self {
            lines: Vec::from([String::new()]),
            cursor_line: 0,
            cursor_col: 0,
            dirty: false,
        }
    }

    pub fn from_utf8(text: &str) -> Self {
        let mut lines: Vec<String> = if text.is_empty() {
            Vec::from([String::new()])
        } else {
            text.split('\n')
                .map(|line| String::from(line.trim_end_matches('\r')))
                .collect()
        };
        if lines.is_empty() {
            lines.push(String::new());
        }
        Self {
            lines,
            cursor_line: 0,
            cursor_col: 0,
            dirty: false,
        }
    }

    pub fn to_utf8_string(&self) -> String {
        self.lines.join("\n")
    }

    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    pub fn line(&self, idx: usize) -> Option<&str> {
        self.lines.get(idx).map(String::as_str)
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn mark_saved(&mut self) {
        self.dirty = false;
    }

    /// True when the buffer holds no user-visible text.
    pub fn is_content_empty(&self) -> bool {
        self.lines.len() <= 1 && self.lines.first().map_or(true, String::is_empty)
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    pub fn char_count(&self) -> usize {
        let body: usize = self.lines.iter().map(|line| line.chars().count()).sum();
        if self.lines.len() <= 1 {
            body
        } else {
            body + self.lines.len().saturating_sub(1)
        }
    }

    pub fn word_count(&self) -> usize {
        let mut count = 0usize;
        for line in &self.lines {
            let mut in_word = false;
            for ch in line.chars() {
                if ch.is_whitespace() {
                    in_word = false;
                } else if !in_word {
                    in_word = true;
                    count += 1;
                }
            }
        }
        count
    }

    fn clamp_cursor_col(&mut self) {
        let len = self
            .lines
            .get(self.cursor_line)
            .map(|line| line.chars().count())
            .unwrap_or(0);
        if self.cursor_col > len {
            self.cursor_col = len;
        }
    }

    pub fn insert_char(&mut self, ch: char) -> bool {
        if ch == '\n' {
            return self.insert_newline();
        }
        if ch == '\r' || ch == '\u{8}' {
            return false;
        }
        if self.cursor_line >= self.lines.len() {
            self.lines.push(String::new());
        }
        let line = &mut self.lines[self.cursor_line];
        let byte_idx = char_index_to_byte(line, self.cursor_col);
        line.insert(byte_idx, ch);
        self.cursor_col += 1;
        self.dirty = true;
        true
    }

    pub fn insert_newline(&mut self) -> bool {
        if self.cursor_line >= self.lines.len() {
            self.lines.push(String::new());
        }
        let line = self.lines[self.cursor_line].clone();
        let byte_idx = char_index_to_byte(&line, self.cursor_col);
        let rest = String::from(&line[byte_idx..]);
        self.lines[self.cursor_line].truncate(byte_idx);
        self.cursor_line += 1;
        self.lines.insert(self.cursor_line, rest);
        self.cursor_col = 0;
        self.dirty = true;
        true
    }

    pub fn backspace(&mut self) -> bool {
        if self.cursor_line >= self.lines.len() {
            return false;
        }
        if self.cursor_col > 0 {
            let line = &mut self.lines[self.cursor_line];
            let char_idx = self.cursor_col - 1;
            let byte_idx = char_index_to_byte(line, char_idx);
            let ch_len = line[byte_idx..]
                .chars()
                .next()
                .map(|c| c.len_utf8())
                .unwrap_or(1);
            line.drain(byte_idx..byte_idx + ch_len);
            self.cursor_col -= 1;
            self.dirty = true;
            return true;
        }
        if self.cursor_line == 0 {
            return false;
        }
        let current = self.lines.remove(self.cursor_line);
        self.cursor_line -= 1;
        self.cursor_col = self.lines[self.cursor_line].chars().count();
        self.lines[self.cursor_line].push_str(&current);
        self.dirty = true;
        true
    }

    pub fn delete_forward(&mut self) -> bool {
        if self.cursor_line >= self.lines.len() {
            return false;
        }
        let line_len = self.lines[self.cursor_line].chars().count();
        if self.cursor_col < line_len {
            let line = &mut self.lines[self.cursor_line];
            let byte_idx = char_index_to_byte(line, self.cursor_col);
            let ch_len = line[byte_idx..]
                .chars()
                .next()
                .map(|c| c.len_utf8())
                .unwrap_or(1);
            line.drain(byte_idx..byte_idx + ch_len);
            self.dirty = true;
            return true;
        }
        if self.cursor_line + 1 >= self.lines.len() {
            return false;
        }
        let next = self.lines.remove(self.cursor_line + 1);
        self.lines[self.cursor_line].push_str(&next);
        self.dirty = true;
        true
    }

    pub fn move_left(&mut self) -> bool {
        if self.cursor_col > 0 {
            self.cursor_col -= 1;
            return true;
        }
        if self.cursor_line == 0 {
            return false;
        }
        self.cursor_line -= 1;
        self.cursor_col = self.lines[self.cursor_line].chars().count();
        true
    }

    pub fn move_right(&mut self) -> bool {
        self.clamp_cursor_col();
        let line_len = self
            .lines
            .get(self.cursor_line)
            .map(|line| line.chars().count())
            .unwrap_or(0);
        if self.cursor_col < line_len {
            self.cursor_col += 1;
            return true;
        }
        if self.cursor_line + 1 >= self.lines.len() {
            return false;
        }
        self.cursor_line += 1;
        self.cursor_col = 0;
        true
    }

    pub fn move_up(&mut self) -> bool {
        if self.cursor_line == 0 {
            return false;
        }
        self.cursor_line -= 1;
        self.clamp_cursor_col();
        true
    }

    pub fn move_down(&mut self) -> bool {
        if self.cursor_line + 1 >= self.lines.len() {
            return false;
        }
        self.cursor_line += 1;
        self.clamp_cursor_col();
        true
    }

    pub fn move_home(&mut self) -> bool {
        if self.cursor_col == 0 {
            return false;
        }
        self.cursor_col = 0;
        true
    }

    pub fn move_end(&mut self) -> bool {
        let len = self
            .lines
            .get(self.cursor_line)
            .map(|line| line.chars().count())
            .unwrap_or(0);
        if self.cursor_col == len {
            return false;
        }
        self.cursor_col = len;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::TextBuffer;

    #[test]
    fn empty_buffer_has_one_blank_line() {
        let buf = TextBuffer::new();
        assert_eq!(buf.line_count(), 1);
        assert_eq!(buf.line(0), Some(""));
        assert!(!buf.is_dirty());
    }

    #[test]
    fn insert_and_split_lines() {
        let mut buf = TextBuffer::new();
        assert!(buf.insert_char('a'));
        assert!(buf.insert_char('b'));
        assert!(buf.insert_newline());
        assert!(buf.insert_char('c'));
        assert_eq!(buf.line(0), Some("ab"));
        assert_eq!(buf.line(1), Some("c"));
        assert_eq!(buf.cursor_line, 1);
        assert_eq!(buf.cursor_col, 1);
        assert!(buf.is_dirty());
    }

    #[test]
    fn backspace_joins_lines() {
        let mut buf = TextBuffer::from_utf8("ab\nc");
        buf.cursor_line = 1;
        buf.cursor_col = 0;
        assert!(buf.backspace());
        assert_eq!(buf.line(0), Some("abc"));
        assert_eq!(buf.cursor_line, 0);
        assert_eq!(buf.cursor_col, 2);
    }

    #[test]
    fn delete_forward_joins_lines() {
        let mut buf = TextBuffer::from_utf8("ab\nc");
        buf.cursor_line = 0;
        buf.cursor_col = 2;
        assert!(buf.delete_forward());
        assert_eq!(buf.line(0), Some("abc"));
    }

    #[test]
    fn utf8_persian_round_trip() {
        let text = "سلام\nworld";
        let mut buf = TextBuffer::from_utf8(text);
        assert_eq!(buf.char_count(), text.chars().count());
        buf.cursor_line = 0;
        buf.cursor_col = 4;
        assert!(buf.insert_char('!'));
        assert_eq!(buf.to_utf8_string(), "سلام!\nworld");
    }

    #[test]
    fn empty_content_detection() {
        assert!(TextBuffer::new().is_content_empty());
        assert!(!TextBuffer::from_utf8("x").is_content_empty());
        assert!(!TextBuffer::from_utf8("\n").is_content_empty());
    }

    #[test]
    fn counts_words_and_chars() {
        let buf = TextBuffer::from_utf8("hello world\nfoo");
        assert_eq!(buf.word_count(), 3);
        assert_eq!(buf.char_count(), "hello world\nfoo".chars().count());
        assert_eq!(buf.line_count(), 2);
    }

    #[test]
    fn arrow_and_home_end() {
        let mut buf = TextBuffer::from_utf8("abcd");
        assert!(buf.move_end());
        assert_eq!(buf.cursor_col, 4);
        assert!(buf.move_home());
        assert_eq!(buf.cursor_col, 0);
        assert!(buf.move_right());
        assert_eq!(buf.cursor_col, 1);
        assert!(buf.move_left());
        assert_eq!(buf.cursor_col, 0);
    }
}
