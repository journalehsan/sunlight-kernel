//! Strict-Type Math Expression Parser & Evaluator
//!
//! Phase 3.3: Recursive descent parser with full operator precedence
//!
//! Key Features:
//! - Tokenization without allocation (iterator-based scanning)
//! - Three-level precedence hierarchy (Expression → Term → Factor)
//! - Strict type enforcement (only Integer math, no silent String coercion)
//! - Full parentheses support with recursion
//! - Division by zero detection
//! - Student-friendly error messages
//!
//! Order of Operations (standard PEMDAS):
//! 1. Parentheses: (2 + 3)
//! 2. Multiplication/Division: * /
//! 3. Addition/Subtraction: + -
//!
//! Example: 2 + 3 * 4 = 2 + 12 = 14 ✓ (not 5 * 4 = 20 ✗)

use crate::sbsp::runtime::SbspContext;
use crate::sbsp::value::SbspValue;
use heapless::{String, Vec};
use crate::sbsp::native::call_native;

/// Math token (result of tokenization)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MathToken<'a> {
    /// Numeric literal: 42
    Number(i64),
    /// Variable reference: x, count, etc.
    Identifier(&'a str),
    /// Addition operator: +
    Plus,
    /// Subtraction operator: -
    Minus,
    /// Multiplication operator: *
    Multiply,
    /// Division operator: /
    Divide,
    /// Modulo operator: %
    Modulo,
    /// Left parenthesis: (
    LParen,
    /// Right parenthesis: )
    RParen,
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
}

/// Tokenize a math expression into MathToken stream
///
/// # Example
/// ```ignore
/// let tokens = tokenize_math("2 + 3 * x")?;
/// // yields: [Number(2), Plus, Number(3), Multiply, Identifier("x")]
/// ```
pub fn tokenize_math(input: &str) -> Result<Vec<MathToken, 64>, heapless::String<256>> {
    let mut tokens = Vec::new();
    let mut chars = input.chars().peekable();

    while let Some(&ch) = chars.peek() {
        match ch {
            // Skip whitespace
            ' ' | '\t' | '\n' | '\r' => {
                chars.next();
            }

            // Operators
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

            // Parentheses
            '(' => {
                chars.next();
                let _ = tokens.push(MathToken::LParen);
            }
            ')' => {
                chars.next();
                let _ = tokens.push(MathToken::RParen);
            }

            // Numbers
            '0'..='9' => {
                let mut num = 0i64;
                while let Some(&digit_ch) = chars.peek() {
                    if digit_ch.is_ascii_digit() {
                        num = num * 10 + (digit_ch as i64 - b'0' as i64);
                        chars.next();
                    } else {
                        break;
                    }
                }
                let _ = tokens.push(MathToken::Number(num));
            }

            // Identifiers (variable names)
            'a'..='z' | 'A'..='Z' | '_' => {
                let start_pos = input.len() - chars.as_str().len();
                let start_str = chars.as_str();

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
                let mut msg = heapless::String::new();
                let _ = core::fmt::write(&mut msg, format_args!("Invalid character: '{}'", ch));
                return Err(msg);
            }
        }
    }

    Ok(tokens)
}

/// Recursive descent math parser with strict type checking
pub struct MathParser<'a, 'ctx> {
    /// The tokenized expression
    tokens: Vec<MathToken<'a>, 64>,
    /// Current position in token stream
    current: usize,
    /// Reference to symbol table for variable lookup
    ctx: &'ctx SbspContext,
}

impl<'a, 'ctx> MathParser<'a, 'ctx> {
    /// Create a new math parser
    pub fn new(tokens: Vec<MathToken<'a>, 64>, ctx: &'ctx SbspContext) -> Self {
        Self { tokens, current: 0, ctx }
    }

    /// Parse the entire expression (entry point)
    /// Lowest precedence: addition and subtraction
    pub fn parse_expression(&mut self) -> Result<i64, heapless::String<256>> {
        let mut left = self.parse_term()?;

        loop {
            match self.peek() {
                Some(MathToken::Plus) => {
                    self.advance();
                    let right = self.parse_term()?;
                    left = left.saturating_add(right);
                }
                Some(MathToken::Minus) => {
                    self.advance();
                    let right = self.parse_term()?;
                    left = left.saturating_sub(right);
                }
                _ => break,
            }
        }

        Ok(left)
    }

    /// Parse a term (multiplication/division)
    /// Medium precedence
    fn parse_term(&mut self) -> Result<i64, heapless::String<256>> {
        let mut left = self.parse_factor()?;

        loop {
            match self.peek() {
                Some(MathToken::Multiply) => {
                    self.advance();
                    let right = self.parse_factor()?;
                    left = left.saturating_mul(right);
                }
                Some(MathToken::Divide) => {
                    self.advance();
                    let right = self.parse_factor()?;
                    if right == 0 {
                        return Err(heapless::String::from("Division by zero error."));
                    }
                    left = left.saturating_div(right);
                }
                Some(MathToken::Modulo) => {
                    self.advance();
                    let right = self.parse_factor()?;
                    if right == 0 {
                        return Err(heapless::String::from("Modulo by zero error."));
                    }
                    left %= right;
                }
                _ => break,
            }
        }

        Ok(left)
    }

    /// Parse a factor (base values: numbers, variables, parentheses, function calls)
    /// Highest precedence
    fn parse_factor(&mut self) -> Result<i64, heapless::String<256>> {
        match self.advance() {
            Some(MathToken::Number(n)) => Ok(n),

            Some(MathToken::Identifier(name)) => {
                // Check for function call: identifier followed by '('
                if let Some(MathToken::LParen) = self.peek() {
                    self.advance(); // Consume '('

                    // Parse function arguments (for V1, support comma-separated expressions)
                    let mut args: Vec<SbspValue, 8> = Vec::new();

                    // Check for empty argument list
                    if let Some(MathToken::RParen) = self.peek() {
                        self.advance(); // Consume ')'
                    } else {
                        // Parse first argument as an expression
                        let arg_val = self.parse_expression()?;
                        args.push(SbspValue::Number(arg_val))
                            .map_err(|_| heapless::String::from("Too many function arguments."))?;

                        // Parse additional arguments (comma-separated)
                        loop {
                            match self.peek() {
                                Some(MathToken::RParen) => {
                                    self.advance(); // Consume ')'
                                    break;
                                }
                                // TODO: Support comma-separated arguments in V1.1
                                // For now, single argument only
                                _ => {
                                    return Err(heapless::String::from(
                                        "Function calls with multiple arguments not yet supported.",
                                    ))
                                }
                            }
                        }
                    }

                    // Dispatch to native function
                    match call_native(name, &args) {
                        Ok(SbspValue::Number(n)) => Ok(n),
                        Ok(other) => {
                            let mut msg = heapless::String::new();
                            let _ = core::fmt::write(
                                &mut msg,
                                format_args!(
                                    "Function '{}' returned {}, expected Integer for math.",
                                    name,
                                    other.type_name()
                                ),
                            );
                            Err(msg)
                        }
                        Err(e) => Err(e),
                    }
                } else {
                    // Not a function call, treat as variable
                    match self.ctx.get(name) {
                        Ok(value) => match value {
                            SbspValue::Number(n) => Ok(n),
                            _ => {
                                let mut msg = heapless::String::new();
                                let _ = core::fmt::write(
                                    &mut msg,
                                    format_args!(
                                        "Type mismatch: Cannot perform math on variable '{}' of type {}.",
                                        name,
                                        value.type_name()
                                    ),
                                );
                                Err(msg)
                            }
                        },
                        Err(_) => {
                            let mut msg = heapless::String::new();
                            let _ = core::fmt::write(
                                &mut msg,
                                format_args!("Variable '{}' is not defined.", name),
                            );
                            Err(msg)
                        }
                    }
                }
            }

            Some(MathToken::LParen) => {
                // Recursively parse the expression inside parentheses
                let val = self.parse_expression()?;

                // Expect closing parenthesis
                match self.advance() {
                    Some(MathToken::RParen) => Ok(val),
                    _ => Err(heapless::String::from("Missing closing parenthesis ')'."))
                }
            }

            Some(MathToken::Minus) => {
                // Unary minus (negation): -5 or -(2 + 3)
                let val = self.parse_factor()?;
                Ok(-val)
            }

            Some(MathToken::Plus) => {
                // Unary plus (no-op): +5
                self.parse_factor()
            }

            _ => Err(heapless::String::from(
                "Expected number, variable, or '('.",
            )),
        }
    }

    // --- Helper Methods ---

    /// Peek at the current token without consuming it
    fn peek(&self) -> Option<MathToken<'a>> {
        self.tokens.get(self.current).copied()
    }

    /// Consume and return the current token
    fn advance(&mut self) -> Option<MathToken<'a>> {
        let token = self.tokens.get(self.current).copied();
        self.current += 1;
        token
    }
}

/// Top-level API: evaluate a math expression and return the result
///
/// # Arguments
/// * `expr` - The math expression as a string (e.g., "2 + 3 * 4")
/// * `ctx` - The execution context (for variable lookup)
///
/// # Returns
/// - `Ok(SbspValue::Number(result))` - Successfully evaluated
/// - `Err(msg)` - Parse error or type error
pub fn evaluate_math(expr: &str, ctx: &SbspContext) -> Result<SbspValue, heapless::String<256>> {
    let tokens = tokenize_math(expr)?;
    let mut parser = MathParser::new(tokens, ctx);
    let result = parser.parse_expression()?;
    Ok(SbspValue::Number(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sbsp::value::SbspValue;

    #[test]
    fn test_tokenize_simple() {
        let tokens = tokenize_math("2 + 3").unwrap();
        assert_eq!(
            tokens.as_slice(),
            &[MathToken::Number(2), MathToken::Plus, MathToken::Number(3)]
        );
    }

    #[test]
    fn test_tokenize_with_spaces() {
        let tokens = tokenize_math("  2   +   3  ").unwrap();
        assert_eq!(
            tokens.as_slice(),
            &[MathToken::Number(2), MathToken::Plus, MathToken::Number(3)]
        );
    }

    #[test]
    fn test_tokenize_multiplication() {
        let tokens = tokenize_math("5 * 3").unwrap();
        assert_eq!(
            tokens.as_slice(),
            &[
                MathToken::Number(5),
                MathToken::Multiply,
                MathToken::Number(3)
            ]
        );
    }

    #[test]
    fn test_tokenize_identifier() {
        let tokens = tokenize_math("x + 5").unwrap();
        assert_eq!(
            tokens.as_slice(),
            &[MathToken::Identifier("x"), MathToken::Plus, MathToken::Number(5)]
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
    fn test_parse_simple_addition() {
        let mut ctx = SbspContext::new();
        let result = evaluate_math("2 + 3", &ctx).unwrap();
        assert_eq!(result, SbspValue::Number(5));
    }

    #[test]
    fn test_parse_simple_subtraction() {
        let mut ctx = SbspContext::new();
        let result = evaluate_math("10 - 3", &ctx).unwrap();
        assert_eq!(result, SbspValue::Number(7));
    }

    #[test]
    fn test_parse_multiplication() {
        let mut ctx = SbspContext::new();
        let result = evaluate_math("3 * 4", &ctx).unwrap();
        assert_eq!(result, SbspValue::Number(12));
    }

    #[test]
    fn test_parse_division() {
        let mut ctx = SbspContext::new();
        let result = evaluate_math("20 / 4", &ctx).unwrap();
        assert_eq!(result, SbspValue::Number(5));
    }

    #[test]
    fn test_parse_order_of_operations() {
        let mut ctx = SbspContext::new();
        // 2 + 3 * 4 = 2 + 12 = 14
        let result = evaluate_math("2 + 3 * 4", &ctx).unwrap();
        assert_eq!(result, SbspValue::Number(14));
    }

    #[test]
    fn test_parse_parentheses() {
        let mut ctx = SbspContext::new();
        // (2 + 3) * 4 = 5 * 4 = 20
        let result = evaluate_math("(2 + 3) * 4", &ctx).unwrap();
        assert_eq!(result, SbspValue::Number(20));
    }

    #[test]
    fn test_parse_nested_parentheses() {
        let mut ctx = SbspContext::new();
        // ((2 + 3) * 4) / 2 = (5 * 4) / 2 = 20 / 2 = 10
        let result = evaluate_math("((2 + 3) * 4) / 2", &ctx).unwrap();
        assert_eq!(result, SbspValue::Number(10));
    }

    #[test]
    fn test_parse_unary_minus() {
        let mut ctx = SbspContext::new();
        let result = evaluate_math("-5 + 10", &ctx).unwrap();
        assert_eq!(result, SbspValue::Number(5));
    }

    #[test]
    fn test_parse_unary_plus() {
        let mut ctx = SbspContext::new();
        let result = evaluate_math("+5 + 3", &ctx).unwrap();
        assert_eq!(result, SbspValue::Number(8));
    }

    #[test]
    fn test_parse_division_by_zero() {
        let mut ctx = SbspContext::new();
        let result = evaluate_math("10 / 0", &ctx);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_modulo() {
        let mut ctx = SbspContext::new();
        let result = evaluate_math("17 % 5", &ctx).unwrap();
        assert_eq!(result, SbspValue::Number(2));
    }

    #[test]
    fn test_parse_variable_lookup() {
        let mut ctx = SbspContext::new();
        ctx.declare("x", "Integer", Some(SbspValue::Number(5)))
            .unwrap();
        let result = evaluate_math("x + 3", &ctx).unwrap();
        assert_eq!(result, SbspValue::Number(8));
    }

    #[test]
    fn test_parse_multiple_variables() {
        let mut ctx = SbspContext::new();
        ctx.declare("x", "Integer", Some(SbspValue::Number(5)))
            .unwrap();
        ctx.declare("y", "Integer", Some(SbspValue::Number(3)))
            .unwrap();
        let result = evaluate_math("x + y", &ctx).unwrap();
        assert_eq!(result, SbspValue::Number(8));
    }

    #[test]
    fn test_parse_undefined_variable() {
        let ctx = SbspContext::new();
        let result = evaluate_math("undefined_var + 5", &ctx);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_string_type_error() {
        let mut ctx = SbspContext::new();
        ctx.declare("name", "String", Some(SbspValue::String(heapless::String::new())))
            .unwrap();
        let result = evaluate_math("name + 5", &ctx);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_complex_expression() {
        let mut ctx = SbspContext::new();
        ctx.declare("x", "Integer", Some(SbspValue::Number(2)))
            .unwrap();
        // (x + 3) * 4 - 5 = (2 + 3) * 4 - 5 = 5 * 4 - 5 = 20 - 5 = 15
        let result = evaluate_math("(x + 3) * 4 - 5", &ctx).unwrap();
        assert_eq!(result, SbspValue::Number(15));
    }
}
