use crate::{Error, ErrorKind, JsResult, Value};
#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Word(String),
    Number(f64),
    String(Vec<u16>),
    Punct(String),
    End,
}
#[derive(Clone, Debug)]
pub struct Token {
    pub kind: Kind,
    pub pos: usize,
    pub line: usize,
}
pub fn located(mut e: Error, source: &str, name: &str, pos: usize) -> Error {
    e.source = name.into();
    e.offset = pos;
    e.line = 1;
    e.column = 1;
    let mut cr = false;
    for c in source[..pos].chars() {
        if c == '\n' && cr {
            cr = false;
            continue;
        }
        if matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}') {
            e.line += 1;
            e.column = 1;
        } else {
            e.column += 1;
        }
        cr = c == '\r';
    }
    e
}
pub fn lex(s: &str, name: &str) -> JsResult<Vec<Token>> {
    let mut out = Vec::new();
    let mut i = 0;
    let mut line = 1;
    let err = |msg: &str, p| located(Error::new(ErrorKind::Syntax, msg), s, name, p);
    while i < s.len() {
        let c = s[i..].chars().next().unwrap();
        if space(c) {
            if matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}') {
                line += 1;
            }
            i += c.len_utf8();
            continue;
        }
        if s[i..].starts_with("//") {
            while i < s.len() {
                let ch = s[i..].chars().next().unwrap();
                if matches!(ch, '\n' | '\r' | '\u{2028}' | '\u{2029}') {
                    break;
                }
                i += ch.len_utf8();
            }
            continue;
        }
        if s[i..].starts_with("/*") {
            let end = s[i + 2..]
                .find("*/")
                .ok_or_else(|| err("unterminated comment", i))?
                + i
                + 4;
            line += s[i..end]
                .chars()
                .filter(|c| matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}'))
                .count();
            i = end;
            continue;
        }
        let pos = i;
        let start_line = line;
        let kind = if c.is_ascii_alphabetic() || c == '_' || c == '$' {
            i += 1;
            while i < s.len()
                && (s.as_bytes()[i].is_ascii_alphanumeric() || b"_$".contains(&s.as_bytes()[i]))
            {
                i += 1;
            }
            Kind::Word(s[pos..i].into())
        } else if c.is_ascii_digit()
            || (c == '.' && s.as_bytes().get(i + 1).is_some_and(u8::is_ascii_digit))
        {
            if c == '0' && s.as_bytes().get(i + 1).is_some_and(u8::is_ascii_digit) {
                return Err(err("legacy leading-zero number is unsupported", i));
            }
            while i < s.len() && s.as_bytes()[i].is_ascii_digit() {
                i += 1;
            }
            if s.as_bytes().get(i) == Some(&b'.') {
                i += 1;
                while i < s.len() && s.as_bytes()[i].is_ascii_digit() {
                    i += 1;
                }
            }
            if s.as_bytes()
                .get(i)
                .is_some_and(|c| *c == b'e' || *c == b'E')
            {
                i += 1;
                if s.as_bytes()
                    .get(i)
                    .is_some_and(|c| *c == b'+' || *c == b'-')
                {
                    i += 1;
                }
                while i < s.len() && s.as_bytes()[i].is_ascii_digit() {
                    i += 1;
                }
            }
            Kind::Number(
                s[pos..i]
                    .parse()
                    .map_err(|_| err("invalid decimal number", pos))?,
            )
        } else if c == '\'' || c == '"' {
            i += 1;
            let mut units = Vec::new();
            let mut closed = false;
            while i < s.len() {
                let ch = s[i..].chars().next().unwrap();
                i += ch.len_utf8();
                if ch == c {
                    closed = true;
                    break;
                }
                if matches!(ch, '\n' | '\r' | '\u{2028}' | '\u{2029}') {
                    return Err(err("newline in string", pos));
                }
                if ch == '\\' {
                    let esc = s[i..]
                        .chars()
                        .next()
                        .ok_or_else(|| err("unterminated escape", pos))?;
                    i += esc.len_utf8();
                    match esc {
                        'n' => units.push(10),
                        'r' => units.push(13),
                        't' => units.push(9),
                        'b' => units.push(8),
                        'f' => units.push(12),
                        'v' => units.push(11),
                        '\\' | '\'' | '"' => units.push(esc as u16),
                        'u' | 'x' => {
                            let n = if esc == 'u' { 4 } else { 2 };
                            let end = i + n;
                            let hex = s.get(i..end).ok_or_else(|| err("invalid hex escape", i))?;
                            units.push(
                                u16::from_str_radix(hex, 16)
                                    .map_err(|_| err("invalid hex escape", i))?,
                            );
                            i = end;
                        }
                        '0' if !s.as_bytes().get(i).is_some_and(u8::is_ascii_digit) => {
                            units.push(0)
                        }
                        _ => {
                            return Err(err(
                                "unsupported string escape (legacy octal escapes forbidden)",
                                i - 1,
                            ))
                        }
                    }
                } else {
                    units.extend(ch.encode_utf16(&mut [0; 2]).iter().copied());
                }
            }
            if !closed {
                return Err(err("unterminated string", pos));
            }
            Kind::String(units)
        } else {
            let op = [
                "===", "!==", "=>", "&&", "||", "??", "<=", ">=", "==", "!=", "++", "--", "**",
                "+=", "-=", "?.", "...",
            ]
            .into_iter()
            .find(|op| s[i..].starts_with(op));
            if let Some(op) = op {
                i += op.len();
                Kind::Punct(op.into())
            } else if "{}[]();,:.?+-*/%<>=!".contains(c) {
                i += 1;
                Kind::Punct(c.to_string())
            } else {
                return Err(err("unsupported token", i));
            }
        };
        out.push(Token {
            kind,
            pos,
            line: start_line,
        });
    }
    out.push(Token {
        kind: Kind::End,
        pos: s.len(),
        line,
    });
    Ok(out)
}
impl Token {
    pub fn is(&self, s: &str) -> bool {
        matches!(&self.kind, Kind::Word(w) | Kind::Punct(w) if w == s)
    }
    pub fn literal(&self) -> Option<Value> {
        match &self.kind {
            Kind::Number(n) => Some(Value::Number(*n)),
            Kind::String(s) => Some(Value::String(s.clone())),
            _ => None,
        }
    }
}

pub(crate) fn space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}
