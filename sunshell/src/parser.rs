extern crate alloc;

use alloc::string::{String, ToString};
use alloc::vec::Vec;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AstNode {
    Command(SimpleCommand),
    Pipeline(Vec<SimpleCommand>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimpleCommand {
    pub argv: Vec<String>,
    pub redirections: Vec<Redirection>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Redirection {
    Open {
        fd: u8,
        mode: RedirectMode,
        path: String,
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
    Word(String),
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
    let mut word = String::new();
    let mut started = false;
    let mut quote = None;
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            } else if ch == '\\' && q == '"' {
                word.push(chars.next().ok_or(ParseError {
                    message: "trailing escape",
                })?);
            } else {
                word.push(ch);
            }
            started = true;
            continue;
        }
        match ch {
            '\'' | '"' => {
                quote = Some(ch);
                started = true;
            }
            '\\' => {
                word.push(chars.next().ok_or(ParseError {
                    message: "trailing escape",
                })?);
                started = true;
            }
            c if c.is_whitespace() => flush_word(&mut out, &mut word, &mut started),
            '|' => {
                flush_word(&mut out, &mut word, &mut started);
                out.push(Token::Pipe);
            }
            '>' | '<' | '2' if ch != '2' || chars.peek().is_some_and(|c| *c == '>') => {
                flush_word(&mut out, &mut word, &mut started);
                let fd = if ch == '2' {
                    chars.next();
                    2
                } else if ch == '<' {
                    0
                } else {
                    1
                };
                let op = if ch == '<' { '<' } else { '>' };
                if op == '>' && chars.peek() == Some(&'>') {
                    chars.next();
                    out.push(Token::Redirect(RedirectMode::Append, fd));
                } else if op == '>' && chars.peek() == Some(&'&') {
                    chars.next();
                    let src = chars
                        .next()
                        .and_then(|c| c.to_digit(10))
                        .ok_or(ParseError {
                            message: "descriptor duplication requires a descriptor number",
                        })?;
                    if src > 2 {
                        return Err(ParseError {
                            message: "only descriptors 0, 1, and 2 are supported",
                        });
                    }
                    out.push(Token::Dup(fd, src as u8));
                } else {
                    out.push(Token::Redirect(
                        if op == '<' {
                            RedirectMode::Read
                        } else {
                            RedirectMode::Truncate
                        },
                        fd,
                    ));
                }
            }
            '0' | '1' if chars.peek() == Some(&'>') => {
                return Err(ParseError {
                    message: "explicit descriptor redirection is supported only for descriptor 2",
                })
            }
            ';' | '&' | '(' | ')' => {
                return Err(ParseError {
                    message: "command separators, background jobs, and grouping are unsupported",
                })
            }
            _ => {
                word.push(ch);
                started = true;
            }
        }
    }
    if quote.is_some() {
        return Err(ParseError {
            message: "unterminated quote",
        });
    }
    flush_word(&mut out, &mut word, &mut started);
    Ok(out)
}

fn flush_word(out: &mut Vec<Token>, word: &mut String, started: &mut bool) {
    if *started {
        out.push(Token::Word(core::mem::take(word)));
        *started = false;
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
        assert_eq!(parts[0].argv, ["echo", "a|b", "|", "c"]);
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
