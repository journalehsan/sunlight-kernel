//! Search matching and result navigation.

use super::buffer::TextBuffer;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchMatch {
    pub line: usize,
    pub col: usize,
}

#[derive(Debug, Clone, Default)]
pub struct SearchState {
    pub query: String,
    pub matches: Vec<SearchMatch>,
    pub current_idx: usize,
}

impl SearchState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Perform search over buffer lines.
    pub fn update_query(&mut self, query: String, buffer: &TextBuffer) {
        self.query = query;
        self.matches.clear();
        self.current_idx = 0;

        if self.query.is_empty() {
            return;
        }

        for (line_idx, line) in buffer.lines.iter().enumerate() {
            let line_lower = line.to_lowercase();
            let query_lower = self.query.to_lowercase();

            let mut start_byte = 0;
            while let Some(pos) = line_lower[start_byte..].find(&query_lower) {
                let match_byte = start_byte + pos;
                let char_col = line[..match_byte].chars().count();
                self.matches.push(SearchMatch {
                    line: line_idx,
                    col: char_col,
                });
                start_byte = match_byte + query_lower.len().max(1);
            }
        }
    }

    /// Advance to next search match. Returns match position if available.
    pub fn next_match(&mut self) -> Option<SearchMatch> {
        if self.matches.is_empty() {
            return None;
        }
        self.current_idx = (self.current_idx + 1) % self.matches.len();
        Some(self.matches[self.current_idx])
    }

    /// Get current match.
    pub fn current_match(&self) -> Option<SearchMatch> {
        self.matches.get(self.current_idx).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_matching() {
        let buf = TextBuffer::from_str("Hello World\nSunlightOS Helios Note\nhello again");
        let mut search = SearchState::new();

        search.update_query("hello".to_string(), &buf);
        assert_eq!(search.matches.len(), 2);
        assert_eq!(search.matches[0], SearchMatch { line: 0, col: 0 });
        assert_eq!(search.matches[1], SearchMatch { line: 2, col: 0 });
    }
}
