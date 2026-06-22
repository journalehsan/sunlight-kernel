//! Strict-Type Math Expression Parser & Evaluator
//!
//! Phase 4: Recursive descent parser with native function support.
//!
//! Precedence hierarchy (lowest to highest):
//!   1. Comparisons: ==, !=, <, >, <=, >=   → returns SbspValue::Bool
//!   2. Additive:    +, -            → returns SbspValue (extracts i64 for ops)
//!   3. Multiplicative: *, /, %      → returns SbspValue (extracts i64 for ops)
//!   4. Primary: numbers, variables, parens, function calls, unary +/-
//!
//! Example: {% IF x + 5 > 10 THEN %}
//!   → parse: ((x + 5) > 10)  → Bool
//!   → not: (x + (5 > 10))    ✗ wrong precedence
//!
//! The parser is strict: only Integer math allowed, no silent String coercion.
//! Division by zero is caught and reported.
//! Native function calls (KV_GET, KV_PUT, etc.) may return any SbspValue type.

use crate::sbsp::native::call_native;
use crate::sbsp::runtime::SbspContext;
use crate::sbsp::value::SbspValue;
use crate::ShmPagePool;
use core::fmt::Write;
use heapless::{String, Vec};

/// Maximum tokens in a single math expression (prevents runaway parsing)
const MAX_TOKENS: usize = 64;

/// Maximum function arguments
const MAX_ARGS: usize = 8;

/// Math token (result of tokenization).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MathToken<'a> {
    /// Numeric literal: 42
    Number(i64),
    /// Variable reference: x, count, etc.
    Identifier(&'a str),
    /// Comparison operators
    Equals,
    NotEquals,
    LessThan,
    GreaterThan,
    LessOrEqual,
    GreaterOrEqual,
    /// Arithmetic operators
    Plus,
    Minus,
    Multiply,
    Divide,
    Modulo,
    /// Parentheses
    LParen,
    RParen,
    /// Argument separator
    Comma,
}

impl<'a> MathToken<'a> {
    pub fn is_operator(&self) -> bool {
        matches!(
            self,
            MathToken::Plus
                | MathToken::Minus
                | MathToken::Multiply
                | MathToken::Divide
                | MathToken::Modulo
                | MathToken::Equals
                | MathToken::NotEquals
                | MathToken::GreaterThan
                | MathToken::LessThan
        )
    }

    pub fn is_additive(&self) -> bool {
        matches!(self, MathToken::Plus | MathToken::Minus)
    }

    pub fn is_multiplicative(&self) -> bool {
        matches!(
            self,
            MathToken::Multiply | MathToken::Divide | MathToken::Modulo
        )
    }

    pub fn is_comparison(&self) -> bool {
        matches!(
            self,
            MathToken::Equals
                | MathToken::NotEquals
                | MathToken::LessThan
                | MathToken::GreaterThan
                | MathToken::LessOrEqual
                | MathToken::GreaterOrEqual
        )
    }
}

// ---------------------------------------------------------------------------
// Tokenizer
// ---------------------------------------------------------------------------

/// Tokenize a math expression into a stream of MathTokens.
///
/// Handles multi-character operators (`==`, `!=`) by looking ahead.
pub fn tokenize_math(input: &str) -> Result<Vec<MathToken, MAX_TOKENS>, String<256>> {
    let mut tokens = Vec::new();
    let mut chars = input.chars().peekable();

    while let Some(&ch) = chars.peek() {
        match ch {
            // Whitespace
            ' ' | '\t' | '\n' | '\r' => {
                chars.next();
            }

            // Arithmetic operators
            '+' => {
                chars.next();
                let _ = tokens.push(MathToken::Plus);
            }
            '-' => {
                chars.next();
                let _ = tokens.push(MathToken::Minus);
            }
            '*' => {
                chars.next();
                let _ = tokens.push(MathToken::Multiply);
            }
            '/' => {
                chars.next();
                let _ = tokens.push(MathToken::Divide);
            }
            '%' => {
                chars.next();
                let _ = tokens.push(MathToken::Modulo);
            }

            // Comparison operators (multi-character aware)
            '=' => {
                chars.next();
                if chars.peek() == Some(&'=') {
                    chars.next();
                    let _ = tokens.push(MathToken::Equals);
                } else {
                    return Err(String::from(
                        "Single '=' not allowed in math. Use '==' for comparison.",
                    ));
                }
            }
            '!' => {
                chars.next();
                if chars.peek() == Some(&'=') {
                    chars.next();
                    let _ = tokens.push(MathToken::NotEquals);
                } else {
                    return Err(String::from(
                        "Unexpected '!'. Use '!=' for not-equal.",
                    ));
                }
            }
            '>' => {
                chars.next();
                if chars.peek() == Some(&'=') {
                    chars.next();
                    let _ = tokens.push(MathToken::GreaterOrEqual);
                } else {
                    let _ = tokens.push(MathToken::GreaterThan);
                }
            }
            '<' => {
                chars.next();
                if chars.peek() == Some(&'=') {
                    chars.next();
                    let _ = tokens.push(MathToken::LessOrEqual);
                } else {
                    let _ = tokens.push(MathToken::LessThan);
                }
            }

            // Parentheses and argument separator
            '(' => {
                chars.next();
                let _ = tokens.push(MathToken::LParen);
            }
            ')' => {
                chars.next();
                let _ = tokens.push(MathToken::RParen);
            }
            ',' => {
                chars.next();
                let _ = tokens.push(MathToken::Comma);
            }

            // Numbers
            '0'..='9' => {
                let mut num = 0i64;
                while let Some(&digit_ch) = chars.peek() {
                    if digit_ch.is_ascii_digit() {
                        num = num.saturating_mul(10).saturating_add(digit_ch as i64 - b'0' as i64);
                        chars.next();
                    } else {
                        break;
                    }
                }
                let _ = tokens.push(MathToken::Number(num));
            }

            // Identifiers (variable names and function names)
            'a'..='z' | 'A'..='Z' | '_' => {
                let start_pos = input.len() - chars.as_str().len();
                let _start_str = chars.as_str();

                while let Some(&id_ch) = chars.peek() {
                    if id_ch.is_alphanumeric() || id_ch == '_' {
                        chars.next();
                    } else {
                        break;
                    }
                }

                let end_pos = input.len() - chars.as_str().len();
                let name = &input[start_pos..end_pos];
                let _ = tokens.push(MathToken::Identifier(name));
            }

            // Invalid character
            _ => {
                let mut msg = String::new();
                let _ = core::fmt::write(
                    &mut msg,
                    format_args!("Invalid character in math expression: '{}'", ch),
                );
                return Err(msg);
            }
        }
    }

    Ok(tokens)
}

// ---------------------------------------------------------------------------
// Recursive Descent Parser
// ---------------------------------------------------------------------------

/// Recursive descent math parser with strict type checking.
///
/// Precedence (low → high):
///   parse_expression   (comparisons: ==, !=, >, <)
///       → parse_additive    (+. -)
///           → parse_multiplicative (*, /, %)
///               → parse_primary (numbers, vars, parens, funcs, unary)
pub struct MathParser<'a, 'ctx> {
    tokens: Vec<MathToken<'a>, MAX_TOKENS>,
    current: usize,
    ctx: &'ctx SbspContext,
    shm_pool: Option<&'ctx ShmPagePool>,
}

impl<'a, 'ctx> MathParser<'a, 'ctx> {
    pub fn new(tokens: Vec<MathToken<'a>, MAX_TOKENS>, ctx: &'ctx SbspContext) -> Self {
        Self {
            tokens,
            current: 0,
            ctx,
            shm_pool: None,
        }
    }

    /// Attach a SHM pool for native function dispatch (KV_GET, KV_PUT, etc.)
    pub fn with_shm_pool(mut self, pool: &'ctx ShmPagePool) -> Self {
        self.shm_pool = Some(pool);
        self
    }

    // -----------------------------------------------------------------------
    // Level 1 — Comparisons (lowest precedence)
    // Returns SbspValue::Boolean when a comparison operator is found,
    // SbspValue::Number for pure arithmetic expressions.
    // -----------------------------------------------------------------------

    pub fn parse_expression(&mut self) -> Result<SbspValue, String<256>> {
        let left = self.parse_additive()?;

        match self.peek() {
            Some(MathToken::Equals)
            | Some(MathToken::NotEquals)
            | Some(MathToken::LessThan)
            | Some(MathToken::GreaterThan)
            | Some(MathToken::LessOrEqual)
            | Some(MathToken::GreaterOrEqual) => {
                let op = self.advance();
                let right = self.parse_additive()?;

                let result = match op {
                    Some(MathToken::Equals) => left == right,
                    Some(MathToken::NotEquals) => left != right,
                    Some(MathToken::LessThan) => left < right,
                    Some(MathToken::GreaterThan) => left > right,
                    Some(MathToken::LessOrEqual) => left <= right,
                    Some(MathToken::GreaterOrEqual) => left >= right,
                    _ => return Err(String::from("Unknown comparison operator.")),
                };

                Ok(SbspValue::Bool(result))
            }
            _ => Ok(SbspValue::Number(left)),
        }
    }

    // -----------------------------------------------------------------------
    // Level 2 — Addition and Subtraction
    // -----------------------------------------------------------------------

    fn parse_additive(&mut self) -> Result<SbspValue, String<256>> {
        let left = self.parse_multiplicative()?;

        // If the left side is not a Number, there's no arithmetic to do
        let mut left_val = match &left {
            SbspValue::Number(n) => *n,
            _ => return Ok(left),
        };

        loop {
            match self.peek() {
                Some(MathToken::Plus) => {
                    self.advance();
                    let right = self.parse_multiplicative()?;
                    let r = match right {
                        SbspValue::Number(n) => n,
                        _ => {
                            let mut msg = String::new();
                            let _ = write!(msg, "Right operand of '+' must be Integer.");
                            return Err(msg);
                        }
                    };
                    left_val = left_val.saturating_add(r);
                }
                Some(MathToken::Minus) => {
                    self.advance();
                    let right = self.parse_multiplicative()?;
                    let r = match right {
                        SbspValue::Number(n) => n,
                        _ => {
                            let mut msg = String::new();
                            let _ = write!(msg, "Right operand of '-' must be Integer.");
                            return Err(msg);
                        }
                    };
                    left_val = left_val.saturating_sub(r);
                }
                _ => break,
            }
        }

        Ok(SbspValue::Number(left_val))
    }

    // -----------------------------------------------------------------------
    // Level 3 — Multiplication, Division, Modulo
    // -----------------------------------------------------------------------

    fn parse_multiplicative(&mut self) -> Result<SbspValue, String<256>> {
        let left = self.parse_primary()?;

        let mut left_val = match &left {
            SbspValue::Number(n) => *n,
            _ => return Ok(left),
        };

        loop {
            match self.peek() {
                Some(MathToken::Multiply) => {
                    self.advance();
                    let right = self.parse_primary()?;
                    let r = match right {
                        SbspValue::Number(n) => n,
                        _ => {
                            let mut msg = String::new();
                            let _ = write!(msg, "Right operand of '*' must be Integer.");
                            return Err(msg);
                        }
                    };
                    left_val = left_val.saturating_mul(r);
                }
                Some(MathToken::Divide) => {
                    self.advance();
                    let right = self.parse_primary()?;
                    let r = match right {
                        SbspValue::Number(n) => n,
                        _ => {
                            let mut msg = String::new();
                            let _ = write!(msg, "Right operand of '/' must be Integer.");
                            return Err(msg);
                        }
                    };
                    if r == 0 {
                        return Err(String::from("Division by zero error."));
                    }
                    left_val = left_val.saturating_div(r);
                }
                Some(MathToken::Modulo) => {
                    self.advance();
                    let right = self.parse_primary()?;
                    let r = match right {
                        SbspValue::Number(n) => n,
                        _ => {
                            let mut msg = String::new();
                            let _ = write!(msg, "Right operand of '%' must be Integer.");
                            return Err(msg);
                        }
                    };
                    if r == 0 {
                        return Err(String::from("Modulo by zero error."));
                    }
                    left_val %= r;
                }
                _ => break,
            }
        }

        Ok(SbspValue::Number(left_val))
    }

    // -----------------------------------------------------------------------
    // Level 4 — Primary: numbers, variables, parens, function calls, unary
    // -----------------------------------------------------------------------

    fn parse_primary(&mut self) -> Result<SbspValue, String<256>> {
        match self.advance() {
            Some(MathToken::Number(n)) => Ok(SbspValue::Number(n)),

            Some(MathToken::Identifier(name)) => {
                // Function call: identifier followed by '('
                if let Some(MathToken::LParen) = self.peek() {
                    self.advance();
                    self.parse_function_call(name)
                } else {
                    // Variable lookup
                    self.lookup_variable(name)
                }
            }

            Some(MathToken::LParen) => {
                let val = self.parse_expression()?;
                match self.advance() {
                    Some(MathToken::RParen) => Ok(val),
                    _ => Err(String::from("Missing closing parenthesis ')'.")),
                }
            }

            Some(MathToken::Minus) => {
                // Unary minus: -5 or -(2 + 3)
                let val = self.parse_primary()?;
                match val {
                    SbspValue::Number(n) => Ok(SbspValue::Number(-n)),
                    other => {
                        let mut msg = String::new();
                        let _ =
                            write!(msg, "Unary minus requires Integer, got {}.", other.type_name());
                        Err(msg)
                    }
                }
            }

            Some(MathToken::Plus) => {
                // Unary plus: +5
                let val = self.parse_primary()?;
                match val {
                    SbspValue::Number(_) => Ok(val),
                    other => {
                        let mut msg = String::new();
                        let _ =
                            write!(msg, "Unary plus requires Integer, got {}.", other.type_name());
                        Err(msg)
                    }
                }
            }

            _ => Err(String::from("Expected a number, variable, or '('.")),
        }
    }

    // -----------------------------------------------------------------------
    // Function call handling
    // -----------------------------------------------------------------------

    fn parse_function_call(&mut self, name: &'a str) -> Result<SbspValue, String<256>> {
        let mut args: Vec<SbspValue, MAX_ARGS> = Vec::new();

        // Check for empty argument list: func()
        if let Some(MathToken::RParen) = self.peek() {
            self.advance();
        } else {
            // Parse first argument
            let arg_val = self.parse_expression()?;
            args.push(arg_val)
                .map_err(|_| String::from("Too many function arguments."))?;

            // Parse remaining comma-separated arguments
            loop {
                match self.peek() {
                    Some(MathToken::Comma) => {
                        self.advance();
                        let arg_val = self.parse_expression()?;
                        args.push(arg_val)
                            .map_err(|_| String::from("Too many function arguments."))?;
                    }
                    Some(MathToken::RParen) => {
                        self.advance();
                        break;
                    }
                    _ => {
                        return Err(String::from(
                            "Expected ',' or ')' after function argument.",
                        ))
                    }
                }
            }
        }

        call_native(name, &args)
    }

    // -----------------------------------------------------------------------
    // Variable lookup with strict type checking
    // -----------------------------------------------------------------------

    fn lookup_variable(&mut self, name: &str) -> Result<SbspValue, String<256>> {
        match self.ctx.get(name) {
            Ok(value) => Ok(value.clone()),
            Err(e) => Err(e),
        }
    }
            },
            Err(_) => {
                let mut msg = String::new();
                let _ =
                    core::fmt::write(&mut msg, format_args!("Variable '{}' is not defined.", name));
                Err(msg)
            }
        }
    }

    // -----------------------------------------------------------------------
    // Position helpers
    // -----------------------------------------------------------------------

    fn peek(&self) -> Option<MathToken<'a>> {
        self.tokens.get(self.current).copied()
    }

    fn advance(&mut self) -> Option<MathToken<'a>> {
        let token = self.tokens.get(self.current).copied();
        self.current += 1;
        token
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Evaluate a math expression and return the result.
///
/// Pure arithmetic returns `SbspValue::Number`.
/// Comparisons return `SbspValue::Bool`.
///
/// # Examples
/// ```ignore
/// evaluate("2 + 3 * 4", &ctx)     → Ok(Number(14))
/// evaluate("(2 + 3) * 4", &ctx)   → Ok(Number(20))
/// evaluate("x > 5", &ctx)         → Ok(Bool(true))   if x = 10
/// ```
/// Evaluate a math expression (no SHM pool — native functions that need IPC will fail).
pub fn evaluate(expr: &str, ctx: &SbspContext) -> Result<SbspValue, String<256>> {
    let tokens = tokenize_math(expr)?;
    let mut parser = MathParser::new(tokens, ctx);
    parser.parse_expression()
}

/// Evaluate a math expression with a SHM pool for native function dispatch.
///
/// Enables KV_GET, KV_PUT, and other IPC-backed native functions that require
/// shared memory pages for payload transfer.
pub fn evaluate_with_shm(
    expr: &str,
    ctx: &SbspContext,
    pool: &ShmPagePool,
) -> Result<SbspValue, String<256>> {
    let tokens = tokenize_math(expr)?;
    let mut parser = MathParser::new(tokens, ctx).with_shm_pool(pool);
    parser.parse_expression()
}

/// Legacy wrapper: evaluate a math expression, returning only integer results.
///
/// Returns `SbspValue::Number` for arithmetic expressions.
/// For expressions containing comparisons, use `evaluate()` instead.
pub fn evaluate_math(expr: &str, ctx: &SbspContext) -> Result<SbspValue, String<256>> {
    evaluate(expr, ctx)
}

/// Evaluate a math expression with SHM pool (legacy wrapper).
pub fn evaluate_math_with_shm(
    expr: &str,
    ctx: &SbspContext,
    pool: &ShmPagePool,
) -> Result<SbspValue, String<256>> {
    evaluate_with_shm(expr, ctx, pool)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sbsp::value::SbspValue;

    fn ctx_with_vars() -> SbspContext {
        let mut ctx = SbspContext::new();
        ctx.declare("x", "Integer", Some(SbspValue::Number(10)))
            .unwrap();
        ctx.declare("y", "Integer", Some(SbspValue::Number(3))).unwrap();
        ctx
    }

    // --- Tokenizer ---

    #[test]
    fn test_tokenize_simple() {
        let tokens = tokenize_math("2 + 3").unwrap();
        assert_eq!(
            tokens.as_slice(),
            &[MathToken::Number(2), MathToken::Plus, MathToken::Number(3)]
        );
    }

    #[test]
    fn test_tokenize_comparison_equals() {
        let tokens = tokenize_math("x == 5").unwrap();
        assert_eq!(
            tokens.as_slice(),
            &[
                MathToken::Identifier("x"),
                MathToken::Equals,
                MathToken::Number(5)
            ]
        );
    }

    #[test]
    fn test_tokenize_comparison_not_equals() {
        let tokens = tokenize_math("x != 5").unwrap();
        assert_eq!(
            tokens.as_slice(),
            &[
                MathToken::Identifier("x"),
                MathToken::NotEquals,
                MathToken::Number(5)
            ]
        );
    }

    #[test]
    fn test_tokenize_comparison_greater() {
        let tokens = tokenize_math("x > 5").unwrap();
        assert_eq!(
            tokens.as_slice(),
            &[
                MathToken::Identifier("x"),
                MathToken::GreaterThan,
                MathToken::Number(5)
            ]
        );
    }

    #[test]
    fn test_tokenize_comparison_less() {
        let tokens = tokenize_math("x < 5").unwrap();
        assert_eq!(
            tokens.as_slice(),
            &[
                MathToken::Identifier("x"),
                MathToken::LessThan,
                MathToken::Number(5)
            ]
        );
    }

    #[test]
    fn test_tokenize_parentheses() {
        let tokens = tokenize_math("(2 + 3) * 4").unwrap();
        assert_eq!(
            tokens.as_slice(),
            &[
                MathToken::LParen,
                MathToken::Number(2),
                MathToken::Plus,
                MathToken::Number(3),
                MathToken::RParen,
                MathToken::Multiply,
                MathToken::Number(4)
            ]
        );
    }

    #[test]
    fn test_tokenize_rejects_single_equals() {
        let result = tokenize_math("x = 5");
        assert!(result.is_err());
    }

    // --- Arithmetic ---

    #[test]
    fn test_arithmetic_simple_addition() {
        let ctx = SbspContext::new();
        assert_eq!(
            evaluate("2 + 3", &ctx).unwrap(),
            SbspValue::Number(5)
        );
    }

    #[test]
    fn test_arithmetic_subtraction() {
        let ctx = SbspContext::new();
        assert_eq!(
            evaluate("10 - 3", &ctx).unwrap(),
            SbspValue::Number(7)
        );
    }

    #[test]
    fn test_arithmetic_multiplication() {
        let ctx = SbspContext::new();
        assert_eq!(
            evaluate("3 * 4", &ctx).unwrap(),
            SbspValue::Number(12)
        );
    }

    #[test]
    fn test_arithmetic_division() {
        let ctx = SbspContext::new();
        assert_eq!(
            evaluate("20 / 4", &ctx).unwrap(),
            SbspValue::Number(5)
        );
    }

    #[test]
    fn test_arithmetic_modulo() {
        let ctx = SbspContext::new();
        assert_eq!(
            evaluate("17 % 5", &ctx).unwrap(),
            SbspValue::Number(2)
        );
    }

    #[test]
    fn test_order_of_operations() {
        let ctx = SbspContext::new();
        // 2 + 3 * 4 = 2 + 12 = 14
        assert_eq!(
            evaluate("2 + 3 * 4", &ctx).unwrap(),
            SbspValue::Number(14)
        );
    }

    #[test]
    fn test_parentheses() {
        let ctx = SbspContext::new();
        // (2 + 3) * 4 = 5 * 4 = 20
        assert_eq!(
            evaluate("(2 + 3) * 4", &ctx).unwrap(),
            SbspValue::Number(20)
        );
    }

    #[test]
    fn test_nested_parentheses() {
        let ctx = SbspContext::new();
        // ((2 + 3) * 4) / 2 = (5 * 4) / 2 = 20 / 2 = 10
        assert_eq!(
            evaluate("((2 + 3) * 4) / 2", &ctx).unwrap(),
            SbspValue::Number(10)
        );
    }

    #[test]
    fn test_unary_minus() {
        let ctx = SbspContext::new();
        assert_eq!(
            evaluate("-5 + 10", &ctx).unwrap(),
            SbspValue::Number(5)
        );
    }

    #[test]
    fn test_unary_plus() {
        let ctx = SbspContext::new();
        assert_eq!(
            evaluate("+5 + 3", &ctx).unwrap(),
            SbspValue::Number(8)
        );
    }

    #[test]
    fn test_division_by_zero() {
        let ctx = SbspContext::new();
        let result = evaluate("10 / 0", &ctx);
        assert!(result.is_err());
    }

    // --- Comparisons ---

    #[test]
    fn test_comparison_equals_true() {
        let ctx = ctx_with_vars();
        assert_eq!(
            evaluate("x == 10", &ctx).unwrap(),
            SbspValue::Bool(true)
        );
    }

    #[test]
    fn test_comparison_equals_false() {
        let ctx = ctx_with_vars();
        assert_eq!(
            evaluate("x == 5", &ctx).unwrap(),
            SbspValue::Bool(false)
        );
    }

    #[test]
    fn test_comparison_not_equals() {
        let ctx = ctx_with_vars();
        assert_eq!(
            evaluate("x != 5", &ctx).unwrap(),
            SbspValue::Bool(true)
        );
    }

    #[test]
    fn test_comparison_greater_than() {
        let ctx = ctx_with_vars();
        assert_eq!(
            evaluate("x > 5", &ctx).unwrap(),
            SbspValue::Bool(true)
        );
    }

    #[test]
    fn test_comparison_less_than() {
        let ctx = ctx_with_vars();
        assert_eq!(
            evaluate("x < 5", &ctx).unwrap(),
            SbspValue::Bool(false)
        );
    }

    #[test]
    fn test_comparison_with_arithmetic() {
        let ctx = ctx_with_vars();
        // x + 5 > 10  →  (10 + 5) > 10  →  15 > 10  →  true
        assert_eq!(
            evaluate("x + 5 > 10", &ctx).unwrap(),
            SbspValue::Bool(true)
        );
    }

    #[test]
    fn test_comparison_with_parentheses() {
        let ctx = ctx_with_vars();
        // (x + y) * 2 == 26  →  (10 + 3) * 2 == 26  →  26 == 26  →  true
        assert_eq!(
            evaluate("(x + y) * 2 == 26", &ctx).unwrap(),
            SbspValue::Bool(true)
        );
    }

    #[test]
    fn test_pure_arithmetic_still_returns_number() {
        let ctx = ctx_with_vars();
        // Ensure pure arithmetic still returns Number, not Bool
        assert_eq!(
            evaluate("x + y", &ctx).unwrap(),
            SbspValue::Number(13)
        );
    }

    // --- Variables ---

    #[test]
    fn test_variable_lookup() {
        let ctx = ctx_with_vars();
        assert_eq!(
            evaluate("x + 3", &ctx).unwrap(),
            SbspValue::Number(13)
        );
    }

    #[test]
    fn test_multiple_variables() {
        let ctx = ctx_with_vars();
        assert_eq!(
            evaluate("x + y", &ctx).unwrap(),
            SbspValue::Number(13)
        );
    }

    #[test]
    fn test_undefined_variable() {
        let ctx = SbspContext::new();
        let result = evaluate("unknown + 5", &ctx);
        assert!(result.is_err());
    }

    #[test]
    fn test_string_type_error() {
        let mut ctx = SbspContext::new();
        ctx.declare("name", "String", Some(SbspValue::String(String::new())))
            .unwrap();
        let result = evaluate("name + 5", &ctx);
        assert!(result.is_err());
    }

    // --- Complex expressions ---

    #[test]
    fn test_complex_expression() {
        let mut ctx = SbspContext::new();
        ctx.declare("x", "Integer", Some(SbspValue::Number(2)))
            .unwrap();
        // (x + 3) * 4 - 5 = (2 + 3) * 4 - 5 = 5 * 4 - 5 = 20 - 5 = 15
        assert_eq!(
            evaluate("(x + 3) * 4 - 5", &ctx).unwrap(),
            SbspValue::Number(15)
        );
    }

    #[test]
    fn test_bmi_example() {
        let mut ctx = SbspContext::new();
        ctx.declare("weight", "Integer", Some(SbspValue::Number(70)))
            .unwrap();
        ctx.declare("height", "Integer", Some(SbspValue::Number(175)))
            .unwrap();
        // weight / (height * height)  —  not realistic BMI, just testing the parser
        let result = evaluate("weight / (height * height)", &ctx);
        // 70 / (175 * 175) = 70 / 30625 = 0 (integer division)
        assert_eq!(result.unwrap(), SbspValue::Number(0));
    }

    #[test]
    fn test_compare_arithmetic_and_comparison_precedence() {
        let ctx = ctx_with_vars();
        // x + 5 > y + 7  →  (10 + 5) > (3 + 7)  →  15 > 10  →  true
        assert_eq!(
            evaluate("x + 5 > y + 7", &ctx).unwrap(),
            SbspValue::Bool(true)
        );
    }

    // --- <= and >= comparisons ---

    #[test]
    fn test_comparison_less_or_equal_true() {
        let ctx = ctx_with_vars();
        // x = 10, so 10 <= 10 and 9 <= 10
        assert_eq!(
            evaluate("x <= 10", &ctx).unwrap(),
            SbspValue::Bool(true)
        );
        assert_eq!(
            evaluate("x <= 11", &ctx).unwrap(),
            SbspValue::Bool(true)
        );
    }

    #[test]
    fn test_comparison_less_or_equal_false() {
        let ctx = ctx_with_vars();
        assert_eq!(
            evaluate("x <= 5", &ctx).unwrap(),
            SbspValue::Bool(false)
        );
    }

    #[test]
    fn test_comparison_greater_or_equal_true() {
        let ctx = ctx_with_vars();
        assert_eq!(
            evaluate("x >= 10", &ctx).unwrap(),
            SbspValue::Bool(true)
        );
        assert_eq!(
            evaluate("x >= 5", &ctx).unwrap(),
            SbspValue::Bool(true)
        );
    }

    #[test]
    fn test_comparison_greater_or_equal_false() {
        let ctx = ctx_with_vars();
        assert_eq!(
            evaluate("x >= 11", &ctx).unwrap(),
            SbspValue::Bool(false)
        );
    }

    #[test]
    fn test_tokenize_less_or_equal() {
        let tokens = tokenize_math("x <= 10").unwrap();
        assert_eq!(
            tokens.as_slice(),
            &[
                MathToken::Identifier("x"),
                MathToken::LessOrEqual,
                MathToken::Number(10)
            ]
        );
    }

    #[test]
    fn test_tokenize_greater_or_equal() {
        let tokens = tokenize_math("x >= 5").unwrap();
        assert_eq!(
            tokens.as_slice(),
            &[
                MathToken::Identifier("x"),
                MathToken::GreaterOrEqual,
                MathToken::Number(5)
            ]
        );
    }

    // --- evaluate_math legacy wrapper ---

    #[test]
    fn test_evaluate_math_backward_compat() {
        let ctx = SbspContext::new();
        assert_eq!(
            evaluate_math("2 + 3", &ctx).unwrap(),
            SbspValue::Number(5)
        );
    }
}
