extern crate alloc;

use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Quote provenance is retained until the final expansion stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quote { Unquoted, Single, Double, Escaped }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WordPart { pub text: String, pub quote: Quote }

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Word { pub parts: Vec<WordPart> }

impl Word {
    fn push(&mut self, ch: char, quote: Quote) {
        if let Some(part) = self.parts.last_mut().filter(|p| p.quote == quote) {
            part.text.push(ch);
        } else {
            self.parts.push(WordPart { text: ch.into(), quote });
        }
    }
    fn quoted_empty(&mut self, quote: Quote) {
        self.parts.push(WordPart { text: String::new(), quote });
    }
}
impl From<&str> for Word {
    fn from(text: &str) -> Self {
        Self { parts: alloc::vec![WordPart { text: text.into(), quote: Quote::Unquoted }] }
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
    Open {
        fd: u8,
        mode: RedirectMode,
        path: W,
    },
    Dup {
        fd: u8,
        source: u8,
    },
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
enum Token {
    Word(Word),
    Pipe,
    Redirect(RedirectMode, u8),
    Dup(u8, u8),
}

pub fn parse_line_checked(line: &str) -> Result<Option<AstNode>, ParseError> {
    let tokens = lex(line)?;
    if tokens.is_empty() {
        return Ok(None);
    }
    let mut commands = Vec::new();
    let mut argv = Vec::new();
    let mut redirections = Vec::new();
    let mut it = tokens.into_iter().peekable();
    while let Some(token) = it.next() {
        match token {
            Token::Word(word) => argv.push(word),
            Token::Pipe => {
                if argv.is_empty() && redirections.is_empty() {
                    return Err(ParseError {
                        message: "empty pipeline stage",
                    });
                }
                commands.push(SimpleCommand {
                    argv: core::mem::take(&mut argv),
                    redirections: core::mem::take(&mut redirections),
                });
            }
            Token::Redirect(mode, fd) => {
                let Some(Token::Word(path)) = it.next() else {
                    return Err(ParseError {
                        message: "redirection requires a path",
                    });
                };
                redirections.push(Redirection::Open { fd, mode, path });
            }
            Token::Dup(fd, source) => redirections.push(Redirection::Dup { fd, source }),
        }
    }
    if argv.is_empty() && redirections.is_empty() {
        return Err(ParseError {
            message: "empty pipeline stage",
        });
    }
    commands.push(SimpleCommand { argv, redirections });
    if commands.iter().any(|c| c.argv.is_empty()) {
        return Err(ParseError {
            message: "redirection-only commands are unsupported",
        });
    }
    if commands.len() == 1 {
        Ok(commands.pop().map(AstNode::Command))
    } else {
        Ok(Some(AstNode::Pipeline(commands)))
    }
}

/// Compatibility entry point. New callers should use `parse_line_checked` so
/// malformed shell syntax can be reported instead of being silently ignored.
pub fn parse_line(line: &str) -> Option<AstNode> {
    parse_line_checked(line).ok().flatten()
}

fn lex(line: &str) -> Result<Vec<Token>, ParseError> {
    let mut out = Vec::new();
    let mut word = Word::default();
    let mut started = false;
    let mut quote = None;
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            } else if ch == '\\' && q == '"' {
                match chars.peek().copied() {
                    Some('$' | '`' | '"' | '\\') => word.push(chars.next().unwrap(), Quote::Escaped),
                    Some('\n') => { chars.next(); }
                    _ => word.push(ch, Quote::Double),
                }
            } else {
                word.push(ch, if q == '\'' { Quote::Single } else { Quote::Double });
            }
            continue;
        }
        match ch {
            '\'' | '"' => {
                quote = Some(ch);
                started = true;
                word.quoted_empty(if ch == '\'' { Quote::Single } else { Quote::Double });
            }
            '\\' => {
                let next = chars.next().ok_or(ParseError { message: "trailing escape" })?;
                if next != '\n' { word.push(next, Quote::Escaped); started = true; }
            }
            ' ' | '\t' | '\n' => flush_word(&mut out, &mut word, &mut started),
            '#' if !started => { while chars.next().is_some_and(|c| c != '\n') {} }
            '|' => {
                flush_word(&mut out, &mut word, &mut started);
                out.push(Token::Pipe);
            }
            '>' | '<' => {
                // An IO number is special only if the entire preceding word is
                // unquoted digits adjacent to the operator: foo2> is foo2 >.
                let mut fd = if ch == '<' { 0 } else { 1 };
                if word.parts.len() == 1 && word.parts[0].quote == Quote::Unquoted
                    && !word.parts[0].text.is_empty() && word.parts[0].text.bytes().all(|c| c.is_ascii_digit()) {
                    fd = word.parts[0].text.parse().map_err(|_| ParseError { message: "invalid descriptor" })?;
                    if fd > 2 { return Err(ParseError { message: "only descriptors 0, 1, and 2 are supported" }); }
                    word = Word::default(); started = false;
                }
                flush_word(&mut out, &mut word, &mut started);
                if chars.peek() == Some(&'&') {
                    chars.next();
                    let mut number = String::new();
                    while chars.peek().is_some_and(|c| c.is_ascii_digit()) { number.push(chars.next().unwrap()); }
                    let source: u8 = number.parse().map_err(|_| ParseError { message: "descriptor duplication requires a descriptor number" })?;
                    if source > 2 { return Err(ParseError { message: "only descriptors 0, 1, and 2 are supported" }); }
                    out.push(Token::Dup(fd, source));
                } else if ch == '>' && chars.peek() == Some(&'>') {
                    chars.next(); out.push(Token::Redirect(RedirectMode::Append, fd));
                } else {
                    out.push(Token::Redirect(if ch == '<' { RedirectMode::Read } else { RedirectMode::Truncate }, fd));
                }
            }
            ';' | '&' | '(' | ')' | '`' => return Err(ParseError {
                message: "command separators, substitutions, background jobs, and grouping are unsupported",
            }),
            _ => { word.push(ch, Quote::Unquoted); started = true; }
        }
    }
    if quote.is_some() { return Err(ParseError { message: "unterminated quote" }); }
    flush_word(&mut out, &mut word, &mut started);
    Ok(out)
}

fn flush_word(out: &mut Vec<Token>, word: &mut Word, started: &mut bool) {
    if *started { out.push(Token::Word(core::mem::take(word))); *started = false; }
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
            let AstNode::Command(cmd) = parse_line_checked(line).unwrap().unwrap() else { panic!() };
            assert_eq!(cmd.argv.len(), 2, "{line}");
            assert!(matches!(cmd.redirections[0], Redirection::Open { fd: 1, .. }));
        }
        let AstNode::Command(cmd) = parse_line_checked("echo 1>out 2>&1").unwrap().unwrap() else { panic!() };
        assert_eq!(cmd.argv.len(), 1);
        assert!(parse_line_checked("echo 2>&12").is_err());
        assert!(parse_line_checked("echo 9>out").is_err());
    }
}
