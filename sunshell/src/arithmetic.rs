//! Bounded POSIX integer arithmetic. Required C operators, without ++/--,
//! sizeof, comma, exponentiation or recursively interpreted variable values.
extern crate alloc;
use crate::{expansion::ExpansionError, shellenv::ShellEnv};
use alloc::string::ToString;

pub const MAX_EXPRESSION_BYTES: usize = 16384;
const MAX_DEPTH: usize = 64;
const MAX_TOKENS: usize = 4096;
type Result<T> = core::result::Result<T, ExpansionError>;
#[derive(Clone, Copy, Debug, PartialEq)]
enum Token<'a> {
    Number(i64),
    Name(&'a str),
    Op(&'a str),
    End,
}
struct Parser<'a, 'e> {
    source: &'a str,
    pos: usize,
    token: Token<'a>,
    tokens: usize,
    depth: usize,
    env: &'e mut ShellEnv,
}
#[derive(Clone, Copy)]
struct Value<'a> {
    number: i64,
    name: Option<&'a str>,
}
impl Value<'_> {
    fn number(number: i64) -> Self {
        Self { number, name: None }
    }
}
fn error(message: &'static str) -> ExpansionError {
    ExpansionError::Arithmetic(message)
}

/// Unset/null names yield zero. Values must be signed integer constants;
/// expressions/names in variable values are rejected rather than recursed into.
fn integer(text: &str) -> Result<i64> {
    if text.is_empty() {
        return Ok(0);
    }
    let text = text.trim();
    let (negative, text) = if let Some(rest) = text.strip_prefix('-') {
        (true, rest)
    } else {
        (false, text.strip_prefix('+').unwrap_or(text))
    };
    let (radix, digits) = if text.starts_with("0x") || text.starts_with("0X") {
        (16, &text[2..])
    } else if text.len() > 1 && text.starts_with('0') {
        (8, &text[1..])
    } else {
        (10, text)
    };
    if digits.is_empty() || !digits.chars().all(|ch| ch.is_ascii() && ch.is_digit(radix)) {
        return Err(error("arithmetic: invalid integer constant"));
    }
    let magnitude = u64::from_str_radix(digits, radix)
        .map_err(|_| error("arithmetic: invalid or out-of-range integer constant"))?;
    if negative {
        if magnitude == 1u64 << 63 {
            Ok(i64::MIN)
        } else {
            i64::try_from(magnitude)
                .ok()
                .and_then(i64::checked_neg)
                .ok_or_else(|| error("arithmetic: integer overflow"))
        }
    } else {
        i64::try_from(magnitude).map_err(|_| error("arithmetic: integer overflow"))
    }
}

pub fn evaluate(source: &str, env: &mut ShellEnv) -> Result<i64> {
    if source.len() > MAX_EXPRESSION_BYTES {
        return Err(error("arithmetic: expression exceeds 16 KiB"));
    }
    let mut parser = Parser {
        source,
        pos: 0,
        token: Token::End,
        tokens: 0,
        depth: 0,
        env,
    };
    parser.advance()?;
    if parser.token == Token::End {
        return Err(error("arithmetic: empty expression"));
    }
    let value = parser.expression(1, true)?;
    if parser.token != Token::End {
        return Err(error("arithmetic: unexpected token"));
    }
    parser.read(value, true)
}
impl<'a> Parser<'a, '_> {
    fn advance(&mut self) -> Result<()> {
        self.tokens += 1;
        if self.tokens > MAX_TOKENS {
            return Err(error("arithmetic: token limit exceeded"));
        }
        let bytes = self.source.as_bytes();
        while bytes.get(self.pos).is_some_and(u8::is_ascii_whitespace) {
            self.pos += 1;
        }
        let start = self.pos;
        let Some(&first) = bytes.get(start) else {
            self.token = Token::End;
            return Ok(());
        };
        if first.is_ascii_digit() {
            self.pos += 1;
            while bytes
                .get(self.pos)
                .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
            {
                self.pos += 1;
            }
            self.token = Token::Number(integer(&self.source[start..self.pos])?);
        } else if first.is_ascii_alphabetic() || first == b'_' {
            self.pos += 1;
            while bytes
                .get(self.pos)
                .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
            {
                self.pos += 1;
            }
            self.token = Token::Name(&self.source[start..self.pos]);
        } else {
            if !first.is_ascii() {
                return Err(error("arithmetic: invalid character"));
            }
            let rest = &self.source[start..];
            let op = [
                "<<=", ">>=", "+=", "-=", "*=", "/=", "%=", "&=", "^=", "|=", "&&", "||", "<<",
                ">>", "<=", ">=", "==", "!=", "++", "--", "**",
            ]
            .into_iter()
            .find(|op| rest.starts_with(op))
            .unwrap_or(&self.source[start..start + 1]);
            if matches!(op, "++" | "--" | "**") {
                return Err(error("arithmetic: optional operator is unsupported"));
            }
            if !matches!(
                op,
                "(" | ")"
                    | "+"
                    | "-"
                    | "!"
                    | "~"
                    | "*"
                    | "/"
                    | "%"
                    | "<"
                    | ">"
                    | "&"
                    | "^"
                    | "|"
                    | "?"
                    | ":"
                    | "="
                    | "<<="
                    | ">>="
                    | "+="
                    | "-="
                    | "*="
                    | "/="
                    | "%="
                    | "&="
                    | "^="
                    | "|="
                    | "&&"
                    | "||"
                    | "<<"
                    | ">>"
                    | "<="
                    | ">="
                    | "=="
                    | "!="
            ) {
                return Err(error("arithmetic: invalid character"));
            }
            self.pos += op.len();
            self.token = Token::Op(op);
        }
        Ok(())
    }
    fn read(&self, value: Value<'a>, active: bool) -> Result<i64> {
        if !active {
            return Ok(0);
        }
        match value.name {
            Some(name) => integer(self.env.get(name).unwrap_or("")),
            None => Ok(value.number),
        }
    }
    fn expect(&mut self, op: &str) -> Result<()> {
        if self.token != Token::Op(op) {
            return Err(error(
                "arithmetic: missing parenthesis or conditional colon",
            ));
        }
        self.advance()
    }
    fn expression(&mut self, min: u8, active: bool) -> Result<Value<'a>> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(error("arithmetic: nesting exceeds 64 levels"));
        }
        let result = self.expression_inner(min, active);
        self.depth -= 1;
        result
    }
    fn expression_inner(&mut self, min: u8, active: bool) -> Result<Value<'a>> {
        let token = self.token;
        self.advance()?;
        let mut left = match token {
            Token::Number(number) => Value::number(number),
            Token::Name(name) => Value {
                number: 0,
                name: Some(name),
            },
            Token::Op("(") => {
                let v = self.expression(1, active)?;
                self.expect(")")?;
                v
            }
            Token::Op(op @ ("+" | "-" | "!" | "~")) => {
                let v = self.expression(13, active)?;
                let n = self.read(v, active)?;
                Value::number(if !active {
                    0
                } else {
                    match op {
                        "+" => n,
                        "-" => n
                            .checked_neg()
                            .ok_or_else(|| error("arithmetic: integer overflow"))?,
                        "!" => i64::from(n == 0),
                        _ => !n,
                    }
                })
            }
            _ => return Err(error("arithmetic: expected integer, variable or operand")),
        };
        while let Token::Op(op) = self.token {
            let precedence = match op {
                "=" | "+=" | "-=" | "*=" | "/=" | "%=" | "<<=" | ">>=" | "&=" | "^=" | "|=" => 1,
                "?" => 2,
                "||" => 3,
                "&&" => 4,
                "|" => 5,
                "^" => 6,
                "&" => 7,
                "==" | "!=" => 8,
                "<" | "<=" | ">" | ">=" => 9,
                "<<" | ">>" => 10,
                "+" | "-" => 11,
                "*" | "/" | "%" => 12,
                _ => break,
            };
            if precedence < min {
                break;
            }
            self.advance()?;
            if op == "?" {
                let condition = self.read(left, active)?;
                let yes = self.expression(1, active && condition != 0)?;
                let yes = self.read(yes, active && condition != 0)?;
                self.expect(":")?;
                let no = self.expression(2, active && condition == 0)?;
                let no = self.read(no, active && condition == 0)?;
                left = Value::number(if condition != 0 { yes } else { no });
                continue;
            }
            let assignment = precedence == 1;
            let name = if assignment {
                Some(
                    left.name
                        .ok_or_else(|| error("arithmetic: assignment requires a variable"))?,
                )
            } else {
                None
            };
            let lhs = if op == "=" {
                0
            } else {
                self.read(left, active)?
            };
            let rhs_active = active && !(op == "&&" && lhs == 0 || op == "||" && lhs != 0);
            let right = self.expression(
                if assignment {
                    precedence
                } else {
                    precedence + 1
                },
                rhs_active,
            )?;
            let rhs = self.read(right, rhs_active)?;
            let value = if !active {
                0
            } else if op == "=" {
                rhs
            } else {
                binary(if assignment { &op[..op.len() - 1] } else { op }, lhs, rhs)?
            };
            if active {
                if let Some(name) = name {
                    self.env.assign(name, &value.to_string()).map_err(error)?;
                }
            }
            left = Value::number(value);
        }
        Ok(left)
    }
}
fn binary(op: &str, lhs: i64, rhs: i64) -> Result<i64> {
    let overflow = || error("arithmetic: integer overflow");
    Ok(match op {
        "+" => lhs.checked_add(rhs).ok_or_else(overflow)?,
        "-" => lhs.checked_sub(rhs).ok_or_else(overflow)?,
        "*" => lhs.checked_mul(rhs).ok_or_else(overflow)?,
        "/" | "%" => {
            if rhs == 0 {
                return Err(error("arithmetic: division or modulo by zero"));
            }
            if op == "/" {
                lhs.checked_div(rhs)
            } else {
                lhs.checked_rem(rhs)
            }
            .ok_or_else(overflow)?
        }
        "<<" | ">>" => {
            if !(0..64).contains(&rhs) {
                return Err(error("arithmetic: shift count outside 0..63"));
            }
            if op == ">>" {
                lhs >> rhs
            } else {
                let result = (lhs as i128) << rhs;
                i64::try_from(result).map_err(|_| overflow())?
            }
        }
        "<" => i64::from(lhs < rhs),
        "<=" => i64::from(lhs <= rhs),
        ">" => i64::from(lhs > rhs),
        ">=" => i64::from(lhs >= rhs),
        "==" => i64::from(lhs == rhs),
        "!=" => i64::from(lhs != rhs),
        "&" => lhs & rhs,
        "^" => lhs ^ rhs,
        "|" => lhs | rhs,
        "&&" => i64::from(lhs != 0 && rhs != 0),
        "||" => i64::from(lhs != 0 || rhs != 0),
        _ => return Err(error("arithmetic: unsupported operator")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn precedence_assignments_and_short_circuit() {
        let mut env = ShellEnv::new();
        for (source, expected) in [
            ("2 + 3 * 4", 14),
            ("(2 + 3) * 4", 20),
            ("10 % 3", 1),
            ("8 >> 1", 4),
            ("3 < 5", 1),
            ("1 && 0", 0),
            ("0 || 7", 1),
            ("0 ? 10 : 20", 20),
            ("~0", -1),
            ("!7", 0),
            ("010 + 0x10", 24),
            ("a = b = 5", 5),
            ("a += 3", 8),
            ("a *= 2", 16),
            ("a /= 4", 4),
            ("a %= 3", 1),
            ("a <<= 3", 8),
            ("a >>= 1", 4),
            ("a |= 3", 7),
            ("a &= 6", 6),
            ("a ^= 2", 4),
            ("a -= 2", 2),
            ("1 || (a = 99)", 1),
            ("0 && (a = 99)", 0),
            ("0 ? 1/0 : a", 2),
            ("1 ? a : 1/0", 2),
            ("missing + a", 2),
        ] {
            assert_eq!(evaluate(source, &mut env).unwrap(), expected, "{source}");
        }
        assert_eq!(env.get("a"), Some("2"));
    }
    #[test]
    fn errors_bounds_readonly_and_nonrecursive_values() {
        let mut env = ShellEnv::new();
        env.assign("a", "b").unwrap();
        env.assign("b", "a").unwrap();
        env.assign("min", "-9223372036854775808").unwrap();
        env.make_readonly("locked");
        for source in [
            "",
            " ",
            "a",
            "b",
            "locked=1",
            "1/0",
            "1%0",
            "1<<64",
            "1<<-1",
            "1<<63",
            "9223372036854775807+1",
            "min/-1",
            "min%-1",
            "-min",
            "08",
            "0x",
            "0xG",
            "1.5",
            "1=2",
            "++a",
            "a++",
            "2**3",
            "1,2",
            "1?2",
            "(1",
            "1+",
            "1 || (2+)",
            "é",
        ] {
            assert!(evaluate(source, &mut env).is_err(), "{source}");
        }
        for value in ["++1", "-+1", "--1", "+0x+1", " ", "\t\n"] {
            env.assign("bad", value).unwrap();
            assert!(evaluate("bad", &mut env).is_err(), "{value:?}");
        }
        assert!(evaluate(&" ".repeat(MAX_EXPRESSION_BYTES + 1), &mut env).is_err());
        assert!(evaluate(
            &alloc::format!("{}1{}", "(".repeat(65), ")".repeat(65)),
            &mut env
        )
        .is_err());
        assert!(evaluate(&"1+".repeat(4096), &mut env).is_err());
        assert_eq!(evaluate("2+2", &mut env).unwrap(), 4);
    }
}
