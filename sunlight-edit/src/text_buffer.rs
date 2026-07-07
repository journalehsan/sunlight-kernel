//! Line-oriented UTF-8 text buffer for the SunlightOS text editor.

use alloc::string::String;
use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct TextPosition {
    pub line: usize,
    pub col: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextRange {
    pub start: TextPosition,
    pub end: TextPosition,
}

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

    pub fn cursor(&self) -> TextPosition {
        TextPosition {
            line: self.cursor_line,
            col: self.cursor_col,
        }
    }

    pub fn set_cursor(&mut self, pos: TextPosition) {
        self.cursor_line = pos.line.min(self.lines.len().saturating_sub(1));
        self.cursor_col = pos.col;
        self.clamp_cursor_col();
    }

    pub fn clamp_position(&self, pos: TextPosition) -> TextPosition {
        let line = pos.line.min(self.lines.len().saturating_sub(1));
        let col = pos.col.min(
            self.lines
                .get(line)
                .map(|line| line.chars().count())
                .unwrap_or(0),
        );
        TextPosition { line, col }
    }

    pub fn normalized_range(&self, start: TextPosition, end: TextPosition) -> TextRange {
        let start = self.clamp_position(start);
        let end = self.clamp_position(end);
        if start <= end {
            TextRange { start, end }
        } else {
            TextRange {
                start: end,
                end: start,
            }
        }
    }

    pub fn has_range(&self, start: TextPosition, end: TextPosition) -> bool {
        self.normalized_range(start, end).start != self.normalized_range(start, end).end
    }

    pub fn document_end(&self) -> TextPosition {
        let line = self.lines.len().saturating_sub(1);
        TextPosition {
            line,
            col: self
                .lines
                .get(line)
                .map(|line| line.chars().count())
                .unwrap_or(0),
        }
    }

    pub fn line_len_chars(&self, line: usize) -> usize {
        self.lines
            .get(line)
            .map(|line| line.chars().count())
            .unwrap_or(0)
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

    pub fn move_document_home(&mut self) -> bool {
        if self.cursor_line == 0 && self.cursor_col == 0 {
            return false;
        }
        self.cursor_line = 0;
        self.cursor_col = 0;
        true
    }

    pub fn move_document_end(&mut self) -> bool {
        let end = self.document_end();
        if self.cursor() == end {
            return false;
        }
        self.set_cursor(end);
        true
    }

    pub fn move_word_left(&mut self) -> bool {
        let mut pos = self.cursor();
        if pos.line == 0 && pos.col == 0 {
            return false;
        }
        if pos.col == 0 {
            pos.line -= 1;
            pos.col = self.line_len_chars(pos.line);
        }
        let line = self.line(pos.line).unwrap_or("");
        let chars: Vec<char> = line.chars().collect();
        let mut idx = pos.col.min(chars.len());
        while idx > 0 && chars[idx - 1].is_whitespace() {
            idx -= 1;
        }
        while idx > 0 && !chars[idx - 1].is_whitespace() {
            idx -= 1;
        }
        self.set_cursor(TextPosition {
            line: pos.line,
            col: idx,
        });
        true
    }

    pub fn move_word_right(&mut self) -> bool {
        let pos = self.cursor();
        let end = self.document_end();
        if pos == end {
            return false;
        }
        let line = self.line(pos.line).unwrap_or("");
        let chars: Vec<char> = line.chars().collect();
        let mut idx = pos.col.min(chars.len());
        while idx < chars.len() && !chars[idx].is_whitespace() {
            idx += 1;
        }
        while idx < chars.len() && chars[idx].is_whitespace() {
            idx += 1;
        }
        if idx < chars.len() || pos.line == end.line {
            self.set_cursor(TextPosition {
                line: pos.line,
                col: idx.min(chars.len()),
            });
        } else {
            self.set_cursor(TextPosition {
                line: pos.line + 1,
                col: 0,
            });
        }
        true
    }

    pub fn insert_text(&mut self, text: &str) -> bool {
        if text.is_empty() {
            return false;
        }
        let mut changed = false;
        for ch in text.chars() {
            changed |= if ch == '\n' {
                self.insert_newline()
            } else {
                self.insert_char(ch)
            };
        }
        changed
    }

    pub fn extract_range(&self, start: TextPosition, end: TextPosition) -> String {
        let range = self.normalized_range(start, end);
        if range.start == range.end {
            return String::new();
        }
        if range.start.line == range.end.line {
            let line = self.line(range.start.line).unwrap_or("");
            let sb = char_index_to_byte(line, range.start.col);
            let eb = char_index_to_byte(line, range.end.col);
            return String::from(&line[sb..eb]);
        }
        let mut out = String::new();
        for line_idx in range.start.line..=range.end.line {
            let line = self.line(line_idx).unwrap_or("");
            if line_idx == range.start.line {
                let sb = char_index_to_byte(line, range.start.col);
                out.push_str(&line[sb..]);
            } else if line_idx == range.end.line {
                let eb = char_index_to_byte(line, range.end.col);
                out.push_str(&line[..eb]);
            } else {
                out.push_str(line);
            }
            if line_idx != range.end.line {
                out.push('\n');
            }
        }
        out
    }

    pub fn delete_range(&mut self, start: TextPosition, end: TextPosition) -> bool {
        let range = self.normalized_range(start, end);
        if range.start == range.end {
            return false;
        }
        if range.start.line == range.end.line {
            let line = &mut self.lines[range.start.line];
            let sb = char_index_to_byte(line, range.start.col);
            let eb = char_index_to_byte(line, range.end.col);
            line.drain(sb..eb);
        } else {
            let first = self.lines[range.start.line].clone();
            let last = self.lines[range.end.line].clone();
            let sb = char_index_to_byte(&first, range.start.col);
            let eb = char_index_to_byte(&last, range.end.col);
            let mut merged = String::from(&first[..sb]);
            merged.push_str(&last[eb..]);
            self.lines
                .splice(range.start.line..=range.end.line, [merged]);
        }
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
        self.set_cursor(range.start);
        self.dirty = true;
        true
    }

    pub fn replace_range(&mut self, start: TextPosition, end: TextPosition, text: &str) -> bool {
        let changed = self.delete_range(start, end);
        self.insert_text(text) || changed
    }

    pub fn select_all_range(&self) -> TextRange {
        TextRange {
            start: TextPosition { line: 0, col: 0 },
            end: self.document_end(),
        }
    }

    pub fn word_range_at(&self, pos: TextPosition) -> Option<TextRange> {
        let pos = self.clamp_position(pos);
        let line = self.line(pos.line)?;
        let chars: Vec<char> = line.chars().collect();
        if chars.is_empty() {
            return None;
        }
        let mut idx = pos.col.min(chars.len().saturating_sub(1));
        if idx == chars.len() && idx > 0 {
            idx -= 1;
        }
        if chars[idx].is_whitespace() {
            return None;
        }
        let mut start = idx;
        let mut end = idx + 1;
        while start > 0 && !chars[start - 1].is_whitespace() {
            start -= 1;
        }
        while end < chars.len() && !chars[end].is_whitespace() {
            end += 1;
        }
        Some(TextRange {
            start: TextPosition {
                line: pos.line,
                col: start,
            },
            end: TextPosition {
                line: pos.line,
                col: end,
            },
        })
    }

    pub fn line_range_at(&self, line: usize) -> Option<TextRange> {
        if line >= self.lines.len() {
            return None;
        }
        Some(TextRange {
            start: TextPosition { line, col: 0 },
            end: TextPosition {
                line,
                col: self.line_len_chars(line),
            },
        })
    }

    pub fn find_all(&self, query: &str) -> Vec<TextRange> {
        let mut out = Vec::new();
        if query.is_empty() {
            return out;
        }
        let query_chars = query.chars().count();
        for (line_idx, line) in self.lines.iter().enumerate() {
            let mut search_from = 0usize;
            while search_from <= line.len() {
                let Some(found) = line[search_from..].find(query) else {
                    break;
                };
                let byte_start = search_from + found;
                let byte_end = byte_start + query.len();
                let start_col = line[..byte_start].chars().count();
                let end_col = start_col + query_chars;
                out.push(TextRange {
                    start: TextPosition {
                        line: line_idx,
                        col: start_col,
                    },
                    end: TextPosition {
                        line: line_idx,
                        col: end_col,
                    },
                });
                search_from = byte_end.max(byte_start + 1);
            }
        }
        out
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
