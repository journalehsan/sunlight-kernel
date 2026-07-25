//! Text buffer model — line-oriented representation with UTF-8 boundary safety.

use std::{fs, io, path::Path};

#[derive(Debug, Clone)]
pub struct TextBuffer {
    pub lines: Vec<String>,
    pub modified: bool,
}

impl Default for TextBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl TextBuffer {
    /// Create a new empty text buffer with a single empty line.
    pub fn new() -> Self {
        Self {
            lines: vec![String::new()],
            modified: false,
        }
    }

    /// Construct buffer from a vector of line strings.
    pub fn from_lines(mut lines: Vec<String>) -> Self {
        if lines.is_empty() {
            lines.push(String::new());
        }
        Self {
            lines,
            modified: false,
        }
    }

    /// Construct buffer from a full string content.
    pub fn from_str(content: &str) -> Self {
        let lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
        Self::from_lines(lines)
    }

    /// Get total line count.
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    /// Check if buffer is empty (0 lines or 1 empty line).
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty() || (self.lines.len() == 1 && self.lines[0].is_empty())
    }

    /// Read line by index.
    pub fn get_line(&self, idx: usize) -> Option<&str> {
        self.lines.get(idx).map(|s| s.as_str())
    }

    /// Total character count of line `idx`.
    pub fn line_char_count(&self, idx: usize) -> usize {
        self.lines.get(idx).map_or(0, |l| l.chars().count())
    }

    /// Calculate visual column width for character index `char_idx` in line `line_idx`,
    /// accounting for tab expansion (default tab_width = 4).
    pub fn visual_col(&self, line_idx: usize, char_idx: usize, tab_width: usize) -> usize {
        let Some(line) = self.lines.get(line_idx) else {
            return 0;
        };

        let mut vcol = 0;
        for (i, ch) in line.chars().enumerate() {
            if i >= char_idx {
                break;
            }
            if ch == '\t' {
                vcol += tab_width - (vcol % tab_width);
            } else {
                vcol += 1;
            }
        }
        vcol
    }

    /// Check if buffer has unsaved changes.
    pub fn is_modified(&self) -> bool {
        self.modified
    }

    /// Mark modified state.
    pub fn set_modified(&mut self, modified: bool) {
        self.modified = modified;
    }

    /// Convert entire buffer back into a single String (newline-joined).
    pub fn to_string_content(&self) -> String {
        let mut res = self.lines.join("\n");
        if !res.is_empty() && !res.ends_with('\n') {
            res.push('\n');
        }
        res
    }

    // --- Editing Operations ---

    /// Insert character at (line, col). Returns updated (line, col).
    pub fn insert_char(&mut self, line: usize, col: usize, ch: char) -> (usize, usize) {
        if line >= self.lines.len() {
            self.lines.resize(line + 1, String::new());
        }

        let l = &mut self.lines[line];
        let byte_idx = l
            .char_indices()
            .map(|(i, _)| i)
            .nth(col)
            .unwrap_or(l.len());

        l.insert(byte_idx, ch);
        self.modified = true;
        (line, col + 1)
    }

    /// Insert newline at (line, col), splitting line into two lines. Returns (new_line, new_col).
    pub fn insert_newline(&mut self, line: usize, col: usize) -> (usize, usize) {
        if line >= self.lines.len() {
            self.lines.resize(line + 1, String::new());
            self.lines.push(String::new());
            self.modified = true;
            return (line + 1, 0);
        }

        let l = &self.lines[line];
        let byte_idx = l
            .char_indices()
            .map(|(i, _)| i)
            .nth(col)
            .unwrap_or(l.len());

        let remainder = self.lines[line][byte_idx..].to_string();
        self.lines[line].truncate(byte_idx);
        self.lines.insert(line + 1, remainder);

        self.modified = true;
        (line + 1, 0)
    }

    /// Perform backspace deletion before (line, col). Returns (new_line, new_col).
    pub fn delete_backspace(&mut self, line: usize, col: usize) -> (usize, usize) {
        if col > 0 {
            if line < self.lines.len() {
                let l = &mut self.lines[line];
                if let Some((byte_idx, _)) = l.char_indices().nth(col - 1) {
                    l.remove(byte_idx);
                    self.modified = true;
                }
            }
            (line, col - 1)
        } else if line > 0 {
            // Join current line with previous line
            let current = self.lines.remove(line);
            let prev_len = self.lines[line - 1].chars().count();
            self.lines[line - 1].push_str(&current);
            self.modified = true;
            (line - 1, prev_len)
        } else {
            (0, 0)
        }
    }

    /// Delete character at (line, col) (Delete key).
    pub fn delete_char(&mut self, line: usize, col: usize) {
        if line >= self.lines.len() {
            return;
        }

        let line_len = self.line_char_count(line);
        if col < line_len {
            let l = &mut self.lines[line];
            if let Some((byte_idx, _)) = l.char_indices().nth(col) {
                l.remove(byte_idx);
                self.modified = true;
            }
        } else if line + 1 < self.lines.len() {
            // Join next line to current line
            let next_line = self.lines.remove(line + 1);
            self.lines[line].push_str(&next_line);
            self.modified = true;
        }
    }

    /// Direct save to file path via truncate + write.
    pub fn save_to_file(&mut self, path_str: &str) -> io::Result<()> {
        let content = self.to_string_content();
        fs::write(Path::new(path_str), content.as_bytes())?;
        self.modified = false;
        Ok(())
    }

    /// Atomic save via temporary file + replace (renameat/rename).
    /// Fallback to direct save if atomic rename fails.
    pub fn save_to_file_atomic(&mut self, path_str: &str) -> io::Result<bool> {
        let path = Path::new(path_str);
        let tmp_filename = format!(
            "{}.tmp.{}.{}",
            path_str,
            std::process::id(),
            sys_time_seed()
        );
        let tmp_path = Path::new(&tmp_filename);

        let content = self.to_string_content();
        if let Err(e) = fs::write(tmp_path, content.as_bytes()) {
            // If temporary file creation fails, fallback to direct save
            self.save_to_file(path_str)?;
            return Ok(false); // false = direct save fallback used
        }

        match fs::rename(tmp_path, path) {
            Ok(()) => {
                self.modified = false;
                Ok(true) // true = atomic replace succeeded
            }
            Err(_) => {
                let _ = fs::remove_file(tmp_path);
                self.save_to_file(path_str)?;
                Ok(false)
            }
        }
    }
}

fn sys_time_seed() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_initialization() {
        let buf = TextBuffer::new();
        assert_eq!(buf.len(), 1);
        assert_eq!(buf.get_line(0), Some(""));
        assert!(!buf.is_modified());
    }

    #[test]
    fn buffer_from_str() {
        let content = "Hello\nWorld\nSunlightOS";
        let buf = TextBuffer::from_str(content);
        assert_eq!(buf.len(), 3);
        assert_eq!(buf.get_line(0), Some("Hello"));
        assert_eq!(buf.get_line(1), Some("World"));
        assert_eq!(buf.get_line(2), Some("SunlightOS"));
    }

    #[test]
    fn visual_column_calculation() {
        let buf = TextBuffer::from_str("a\tb");
        assert_eq!(buf.visual_col(0, 0, 4), 0);
        assert_eq!(buf.visual_col(0, 2, 4), 4);
    }

    #[test]
    fn editing_insert_char() {
        let mut buf = TextBuffer::new();
        let (l, c) = buf.insert_char(0, 0, 'A');
        assert_eq!((l, c), (0, 1));
        assert_eq!(buf.get_line(0), Some("A"));
        assert!(buf.is_modified());
    }

    #[test]
    fn editing_newline_split() {
        let mut buf = TextBuffer::from_str("HelloWorld");
        let (l, c) = buf.insert_newline(0, 5);
        assert_eq!((l, c), (1, 0));
        assert_eq!(buf.len(), 2);
        assert_eq!(buf.get_line(0), Some("Hello"));
        assert_eq!(buf.get_line(1), Some("World"));
    }

    #[test]
    fn editing_backspace_join() {
        let mut buf = TextBuffer::from_str("Hello\nWorld");
        let (l, c) = buf.delete_backspace(1, 0);
        assert_eq!((l, c), (0, 5));
        assert_eq!(buf.len(), 1);
        assert_eq!(buf.get_line(0), Some("HelloWorld"));
    }

    #[test]
    fn direct_save_clears_modified() {
        let mut buf = TextBuffer::from_str("Test Content");
        buf.insert_char(0, 12, '!');
        assert!(buf.is_modified());

        let tmp_path = format!("/tmp/helios_test_save_{}.txt", std::process::id());
        buf.save_to_file(&tmp_path).unwrap();
        assert!(!buf.is_modified());

        let read_back = fs::read_to_string(&tmp_path).unwrap();
        assert_eq!(read_back, "Test Content!\n");
        let _ = fs::remove_file(tmp_path);
    }

    #[test]
    fn atomic_save_workflow() {
        let mut buf = TextBuffer::from_str("Atomic Content");
        buf.insert_char(0, 14, '?');
        assert!(buf.is_modified());

        let tmp_path = format!("/tmp/helios_test_atomic_{}.txt", std::process::id());
        let _atomic_used = buf.save_to_file_atomic(&tmp_path).unwrap();
        assert!(!buf.is_modified());

        let read_back = fs::read_to_string(&tmp_path).unwrap();
        assert_eq!(read_back, "Atomic Content?\n");
        let _ = fs::remove_file(tmp_path);
    }
}
