//! SBSP Lexer - Character-by-character tokenizer
//!
//! Phase 3: Scans .sbsp files and produces SbspToken stream.
//! Separates HTML (text outside {% %}) from code blocks (text inside {% %}).

use crate::sbsp::token::SbspToken;
use heapless::String;

/// The SBSP lexer
pub struct SbspLexer {
    input: heapless::String<4096>,
    position: usize,
}

impl SbspLexer {
    /// Create a new lexer from input string
    pub fn new(input: &str) -> Self {
        Self {
            input: String::from(input),
            position: 0,
        }
    }

    /// Scan and return the next token
    pub fn next_token(&mut self) -> SbspToken {
        if self.position >= self.input.len() {
            return SbspToken::Eof;
        }

        // TODO: Phase 3 implementation
        // For now, return placeholder
        SbspToken::Text(String::new())
    }

    /// Peek the next character without consuming
    fn peek(&self) -> Option<char> {
        self.input.chars().nth(self.position)
    }

    /// Advance to next character
    fn advance(&mut self) {
        if self.position < self.input.len() {
            self.position += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lexer_simple_html() {
        let mut lexer = SbspLexer::new("Hello world");
        let token = lexer.next_token();
        // TODO: Assert tokens
    }

    #[test]
    fn test_lexer_with_sbsp_tag() {
        let mut lexer = SbspLexer::new("Hello {%| name %}!");
        let _token = lexer.next_token();
        // TODO: Assert proper tokenization
    }
}
