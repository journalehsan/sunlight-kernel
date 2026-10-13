extern crate alloc;

use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Quote provenance is retained until the final expansion stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quote {
    Unquoted,
    Single,
    Double,
    Escaped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WordPart {
    pub text: String,
    pub quote: Quote,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Word {
    pub parts: Vec<WordPart>,
}

impl Word {
    pub(crate) fn push(&mut self, ch: char, quote: Quote) {
        if let Some(part) = self.parts.last_mut().filter(|p| p.quote == quote) {
            part.text.push(ch);
        } else {
            self.parts.push(WordPart {
                text: ch.into(),
                quote,
            });
        }
    }
    pub(crate) fn quoted_empty(&mut self, quote: Quote) {
        self.parts.push(WordPart {
            text: String::new(),
            quote,
        });
    }
}
impl From<&str> for Word {
    fn from(text: &str) -> Self {
        Self {
            parts: alloc::vec![WordPart {
                text: text.into(),
                quote: Quote::Unquoted
            }],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AstNode<W = Word> {
    Command(SimpleCommand<W>),
    Pipeline(Vec<SimpleCommand<W>>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimpleCommand<W = Word> {
    pub argv: Vec<W>,
    pub redirections: Vec<Redirection<W>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Redirection<W = Word> {
    Open { fd: u8, mode: RedirectMode, path: W },
    Dup { fd: u8, source: u8 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RedirectMode {
    Read,
    Truncate,
    Append,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub message: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Token {
    Word(Word),
    Pipe,
    Separator,
    And,
    Or,
    LeftParen,
    RightParen,
    DoubleSemi,
    Redirect(RedirectMode, u8),
    Dup(u8, u8),
}

pub fn parse_line_checked(line: &str) -> Result<Option<AstNode>, ParseError> {
    use crate::script_parser::CommandKind;
    let tree = crate::script_parser::parse(line).map_err(|e| ParseError { message: e.message })?;
    let CommandKind::SequentialList(mut commands) = tree.kind else {
        unreachable!()
    };
    if commands.is_empty() {
        return Ok(None);
    }
    if commands.len() != 1 {
        return Err(ParseError {
            message: "interactive command lists require script mode",
        });
    }
    let command = commands.pop().unwrap();
    let result = match command.kind {
        CommandKind::SimpleCommand(command) => AstNode::Command(command),
        CommandKind::Pipeline(stages) => {
            let mut commands = Vec::new();
            for stage in stages {
                let CommandKind::SimpleCommand(command) = stage.kind else {
                    return Err(ParseError {
                        message: "compound pipeline stages require script mode",
                    });
                };
                commands.push(command);
            }
            AstNode::Pipeline(commands)
        }
        _ => {
            return Err(ParseError {
                message: "compound commands require script mode",
            })
        }
    };
    if match &result {
        AstNode::Command(command) => command.argv.is_empty(),
        AstNode::Pipeline(commands) => commands.iter().any(|c| c.argv.is_empty()),
    } {
        return Err(ParseError {
            message: "redirection-only commands require script mode",
        });
    }
    Ok(Some(result))
}

/// Compatibility entry point. New callers should use `parse_line_checked` so
/// malformed shell syntax can be reported instead of being silently ignored.
pub fn parse_line(line: &str) -> Option<AstNode> {
    parse_line_checked(line).ok().flatten()
}

/// One lexer for interactive commands and scripts. Offsets are UTF-8 byte offsets.
pub(crate) fn lex_spanned(line: &str) -> Result<Vec<(Token, usize)>, ParseError> {
    if line.len() > 65536 {
        return Err(ParseError {
            message: "source exceeds 64 KiB limit",
        });
    }
    let mut out = Vec::new();
    let mut word = Word::default();
    let mut start = None;
    let mut quote = None;
    let mut chars = line.char_indices().peekable();
    while let Some((pos, ch)) = chars.next() {
        if out.len() >= 8192 {
            return Err(ParseError {
                message: "token limit exceeded",
            });
        }
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            } else if ch == '\\' && q == '"' {
                match chars.peek().map(|(_, c)| *c) {
                    Some('$' | '`' | '"' | '\\') => {
                        word.push(chars.next().unwrap().1, Quote::Escaped)
                    }
                    Some('\n') => {
                        chars.next();
                    }
                    _ => word.push(ch, Quote::Double),
                }
            } else {
                word.push(
                    ch,
                    if q == '\'' {
                        Quote::Single
                    } else {
                        Quote::Double
                    },
                );
            }
            continue;
        }
        match ch {
            '\'' | '"' => {
                start.get_or_insert(pos);
                quote = Some(ch);
                word.quoted_empty(if ch == '\'' {
                    Quote::Single
                } else {
                    Quote::Double
                });
            }
            '\\' => {
                let (_, next) = chars.next().ok_or(ParseError {
                    message: "trailing escape",
                })?;
                if next != '\n' {
                    start.get_or_insert(pos);
                    word.push(next, Quote::Escaped);
                }
            }
            ' ' | '\t' => flush_spanned(&mut out, &mut word, &mut start),
            '\n' => {
                flush_spanned(&mut out, &mut word, &mut start);
                out.push((Token::Separator, pos));
            }
            '#' if start.is_none() => {
                while chars.next().is_some_and(|(_, c)| c != '\n') {}
                out.push((Token::Separator, pos));
            }
            '|' | '&' | ';' => {
                flush_spanned(&mut out, &mut word, &mut start);
                let twice = chars.peek().is_some_and(|(_, c)| *c == ch);
                if twice {
                    chars.next();
                }
                let token = match (ch, twice) {
                    ('|', false) => Token::Pipe,
                    ('|', true) => Token::Or,
                    ('&', true) => Token::And,
                    (';', false) => Token::Separator,
                    (';', true) => Token::DoubleSemi,
                    _ => {
                        return Err(ParseError {
                            message: "background execution is unsupported",
                        })
                    }
                };
                out.push((token, pos));
            }
            '(' | ')' => {
                flush_spanned(&mut out, &mut word, &mut start);
                out.push((
                    if ch == '(' {
                        Token::LeftParen
                    } else {
                        Token::RightParen
                    },
                    pos,
                ));
            }
            '>' | '<' => {
                let mut fd = if ch == '<' { 0 } else { 1 };
                if word.parts.len() == 1
                    && word.parts[0].quote == Quote::Unquoted
                    && !word.parts[0].text.is_empty()
                    && word.parts[0].text.bytes().all(|c| c.is_ascii_digit())
                {
                    fd = word.parts[0].text.parse().map_err(|_| ParseError {
                        message: "invalid descriptor",
                    })?;
                    word = Word::default();
                    start = None;
                }
                if fd > 2 {
                    return Err(ParseError {
                        message: "only descriptors 0, 1, and 2 are supported",
                    });
                }
                flush_spanned(&mut out, &mut word, &mut start);
                if chars.peek().is_some_and(|(_, c)| *c == '&') {
                    chars.next();
                    let mut number = String::new();
                    while chars.peek().is_some_and(|(_, c)| c.is_ascii_digit()) {
                        number.push(chars.next().unwrap().1);
                    }
                    let source = number.parse::<u8>().map_err(|_| ParseError {
                        message: "descriptor duplication requires a descriptor number",
                    })?;
                    if source > 2 {
                        return Err(ParseError {
                            message: "only descriptors 0, 1, and 2 are supported",
                        });
                    }
                    out.push((Token::Dup(fd, source), pos));
                } else if chars.peek().is_some_and(|(_, c)| *c == ch) {
                    if ch == '<' {
                        return Err(ParseError {
                            message: "here documents are not implemented",
                        });
                    }
                    chars.next();
                    out.push((Token::Redirect(RedirectMode::Append, fd), pos));
                } else {
                    out.push((
                        Token::Redirect(
                            if ch == '<' {
                                RedirectMode::Read
                            } else {
                                RedirectMode::Truncate
                            },
                            fd,
                        ),
                        pos,
                    ));
                }
            }
            '`' => {
                return Err(ParseError {
                    message: "backtick substitution is unsupported",
                })
            }
            _ => {
                start.get_or_insert(pos);
                word.push(ch, Quote::Unquoted);
            }
        }
    }
    if quote.is_some() {
        return Err(ParseError {
            message: "unterminated quote",
        });
    }
    flush_spanned(&mut out, &mut word, &mut start);
    if out.len() > 8192 {
        return Err(ParseError {
            message: "token limit exceeded",
        });
    }
    Ok(out)
}
fn flush_spanned(out: &mut Vec<(Token, usize)>, word: &mut Word, start: &mut Option<usize>) {
    if let Some(pos) = start.take() {
        out.push((Token::Word(core::mem::take(word)), pos));
    }
}

/// Backward-compatible whitespace tokenizer.
pub fn tokenize(line: &str) -> Vec<String> {
    line.split_ascii_whitespace()
        .map(ToString::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty() {
        assert_eq!(parse_line_checked("  ").unwrap(), None);
    }
    #[test]
    fn quotes_and_escapes_keep_operators_literal() {
        let AstNode::Pipeline(parts) = parse_line_checked(r#"echo 'a|b' \| c | cat"#)
            .unwrap()
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(parts[0].argv[1].parts[0].quote, Quote::Single);
        assert_eq!(parts[0].argv[2].parts[0].quote, Quote::Escaped);
    }
    #[test]
    fn redirection_order_is_preserved() {
        let AstNode::Command(cmd) = parse_line_checked("cmd 2>&1 > out").unwrap().unwrap() else {
            panic!()
        };
        assert_eq!(
            cmd.redirections,
            [
                Redirection::Dup { fd: 2, source: 1 },
                Redirection::Open {
                    fd: 1,
                    mode: RedirectMode::Truncate,
                    path: "out".into()
                }
            ]
        );
    }
    #[test]
    fn malformed_syntax_is_reported() {
        assert!(parse_line_checked("a | | b").is_err());
        assert!(parse_line_checked("echo 'x").is_err());
        assert!(parse_line_checked("echo >").is_err());
    }
}

#[cfg(test)]
mod io_number_tests {
    use super::*;
    #[test]
    fn descriptor_numbers_require_adjacency_and_unquoted_digits() {
        for line in ["echo foo2>out", "echo '2'>out", "echo 2 >out"] {
            let AstNode::Command(cmd) = parse_line_checked(line).unwrap().unwrap() else {
                panic!()
            };
            assert_eq!(cmd.argv.len(), 2, "{line}");
            assert!(matches!(
                cmd.redirections[0],
                Redirection::Open { fd: 1, .. }
            ));
        }
        let AstNode::Command(cmd) = parse_line_checked("echo 1>out 2>&1").unwrap().unwrap() else {
            panic!()
        };
        assert_eq!(cmd.argv.len(), 1);
        assert!(parse_line_checked("echo 2>&12").is_err());
        assert!(parse_line_checked("echo 9>out").is_err());
    }
}
