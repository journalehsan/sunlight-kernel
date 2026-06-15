//! In-process integer expression evaluator for the shell's `=` builtin.
//!
//! Adapted from the standalone sunlight-calc guide, minus the userspace
//! syscall/entry plumbing: the shell already runs in ring 3 and prints through
//! its normal output path, so it only needs the lexer + recursive-descent
//! parser. Supports `+ - * / % ^ ( )`, unary +/-, with overflow and
//! divide-by-zero checking, all on i64.

/// Token types produced by the lexer.
#[derive(Clone, Copy, PartialEq)]
enum Token {
    Number(i64),
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Caret,
    LeftParen,
    RightParen,
    End,
}

/// Lexing failures.
#[derive(Clone, Copy, PartialEq, Eq)]
enum LexError {
    InvalidChar,
    NumberOverflow,
}

struct Lexer<'a> {
    input: &'a [u8],
    pos: usize,
}

impl<'a> Lexer<'a> {
    const fn new(input: &'a [u8]) -> Self {
        Self { input, pos: 0 }
    }

    fn skip_whitespace(&mut self) {
        while self.pos < self.input.len() && self.input[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }

    fn read_number(&mut self) -> Result<i64, LexError> {
        let mut n: i64 = 0;
        while self.pos < self.input.len() && self.input[self.pos].is_ascii_digit() {
            let d = (self.input[self.pos] - b'0') as i64;
            n = n
                .checked_mul(10)
                .ok_or(LexError::NumberOverflow)?
                .checked_add(d)
                .ok_or(LexError::NumberOverflow)?;
            self.pos += 1;
        }
        Ok(n)
    }

    fn next_token(&mut self) -> Result<Token, LexError> {
        self.skip_whitespace();
        if self.pos >= self.input.len() {
            return Ok(Token::End);
        }
        let ch = self.input[self.pos];
        match ch {
            b'0'..=b'9' => Ok(Token::Number(self.read_number()?)),
            b'+' => { self.pos += 1; Ok(Token::Plus) }
            b'-' => { self.pos += 1; Ok(Token::Minus) }
            b'*' => { self.pos += 1; Ok(Token::Star) }
            b'/' => { self.pos += 1; Ok(Token::Slash) }
            b'%' => { self.pos += 1; Ok(Token::Percent) }
            b'^' => { self.pos += 1; Ok(Token::Caret) }
            b'(' => { self.pos += 1; Ok(Token::LeftParen) }
            b')' => { self.pos += 1; Ok(Token::RightParen) }
            _ => Err(LexError::InvalidChar),
        }
    }
}

/// Parse/eval failures, surfaced to the user as messages.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CalcError {
    InvalidChar,
    NumberOverflow,
    DivisionByZero,
    UnmatchedParen,
    UnexpectedToken,
    EmptyInput,
    Overflow,
}

impl From<LexError> for CalcError {
    fn from(e: LexError) -> Self {
        match e {
            LexError::InvalidChar => CalcError::InvalidChar,
            LexError::NumberOverflow => CalcError::NumberOverflow,
        }
    }
}

struct Parser<'a> {
    lexer: Lexer<'a>,
    current: Token,
}

impl<'a> Parser<'a> {
    fn new(input: &'a [u8]) -> Result<Self, CalcError> {
        if input.is_empty() {
            return Err(CalcError::EmptyInput);
        }
        let mut lexer = Lexer::new(input);
        let current = lexer.next_token()?;
        Ok(Self { lexer, current })
    }

    fn advance(&mut self) -> Result<(), CalcError> {
        self.current = self.lexer.next_token()?;
        Ok(())
    }

    fn parse(&mut self) -> Result<i64, CalcError> {
        let result = self.expr()?;
        if self.current != Token::End {
            return Err(CalcError::UnexpectedToken);
        }
        Ok(result)
    }

    fn expr(&mut self) -> Result<i64, CalcError> {
        let mut left = self.term()?;
        loop {
            match self.current {
                Token::Plus => {
                    self.advance()?;
                    let right = self.term()?;
                    left = left.checked_add(right).ok_or(CalcError::Overflow)?;
                }
                Token::Minus => {
                    self.advance()?;
                    let right = self.term()?;
                    left = left.checked_sub(right).ok_or(CalcError::Overflow)?;
                }
                _ => break,
            }
        }
        Ok(left)
    }

    fn term(&mut self) -> Result<i64, CalcError> {
        let mut left = self.power()?;
        loop {
            match self.current {
                Token::Star => {
                    self.advance()?;
                    let right = self.power()?;
                    left = left.checked_mul(right).ok_or(CalcError::Overflow)?;
                }
                Token::Slash => {
                    self.advance()?;
                    let right = self.power()?;
                    if right == 0 {
                        return Err(CalcError::DivisionByZero);
                    }
                    left = left.checked_div(right).ok_or(CalcError::Overflow)?;
                }
                Token::Percent => {
                    self.advance()?;
                    let right = self.power()?;
                    if right == 0 {
                        return Err(CalcError::DivisionByZero);
                    }
                    left = left.checked_rem(right).ok_or(CalcError::Overflow)?;
                }
                _ => break,
            }
        }
        Ok(left)
    }

    fn power(&mut self) -> Result<i64, CalcError> {
        let base = self.unary()?;
        if self.current == Token::Caret {
            self.advance()?;
            let exp = self.power()?;
            return int_pow(base, exp);
        }
        Ok(base)
    }

    fn unary(&mut self) -> Result<i64, CalcError> {
        match self.current {
            Token::Minus => {
                self.advance()?;
                let v = self.unary()?;
                v.checked_neg().ok_or(CalcError::Overflow)
            }
            Token::Plus => {
                self.advance()?;
                self.unary()
            }
            _ => self.primary(),
        }
    }

    fn primary(&mut self) -> Result<i64, CalcError> {
        match self.current {
            Token::Number(n) => {
                self.advance()?;
                Ok(n)
            }
            Token::LeftParen => {
                self.advance()?;
                let result = self.expr()?;
                if self.current != Token::RightParen {
                    return Err(CalcError::UnmatchedParen);
                }
                self.advance()?;
                Ok(result)
            }
            _ => Err(CalcError::UnexpectedToken),
        }
    }
}

/// Integer exponentiation with overflow checking.
fn int_pow(mut base: i64, exp: i64) -> Result<i64, CalcError> {
    if exp < 0 {
        return match base {
            1 => Ok(1),
            -1 => Ok(if exp % 2 == 0 { 1 } else { -1 }),
            _ => Ok(0),
        };
    }
    let mut result: i64 = 1;
    let mut e = exp as u64;
    while e > 0 {
        if e & 1 == 1 {
            result = result.checked_mul(base).ok_or(CalcError::Overflow)?;
        }
        e >>= 1;
        if e > 0 {
            base = base.checked_mul(base).ok_or(CalcError::Overflow)?;
        }
    }
    Ok(result)
}

/// Evaluate a math expression from bytes.
pub fn eval(input: &[u8]) -> Result<i64, CalcError> {
    let mut parser = Parser::new(input)?;
    parser.parse()
}

/// Human-readable error message for display.
pub fn error_msg(e: CalcError) -> &'static [u8] {
    match e {
        CalcError::InvalidChar => b"calc: invalid character",
        CalcError::NumberOverflow => b"calc: number too large",
        CalcError::DivisionByZero => b"calc: division by zero",
        CalcError::UnmatchedParen => b"calc: unmatched parenthesis",
        CalcError::UnexpectedToken => b"calc: unexpected token",
        CalcError::EmptyInput => b"calc: empty expression",
        CalcError::Overflow => b"calc: arithmetic overflow",
    }
}

/// Format an i64 into the front of `buf`, returning the used slice.
pub fn format_i64(value: i64, buf: &mut [u8; 21]) -> &[u8] {
    if value == 0 {
        buf[0] = b'0';
        return &buf[..1];
    }
    let mut v = value;
    let negative = v < 0;
    if negative {
        v = v.wrapping_neg();
    }
    let mut i = 21;
    // wrapping_neg of i64::MIN stays negative; handle via unsigned math.
    let mut uv = if negative && v < 0 { (i64::MAX as u64) + 1 } else { v as u64 };
    while uv > 0 {
        i -= 1;
        buf[i] = b'0' + (uv % 10) as u8;
        uv /= 10;
    }
    if negative {
        i -= 1;
        buf[i] = b'-';
    }
    &buf[i..]
}
