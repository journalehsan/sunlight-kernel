//! Control Flow Execution Engine
//!
//! Phase 3.4: IF/ELSE logic with zero-allocation token skipping
//!
//! Key Insight: When an IF condition is false, we don't allocate memory
//! to "store" the skipped block. We just fast-forward the lexer iterator,
//! counting nesting depth until we find the matching ELSE or END IF.
//!
//! This is how professional template engines work: they treat the token
//! stream as a tape that can be read but not rewound (except for loops,
//! which is harder—see Phase 3.5).

use crate::sbsp::lexer::SbspLexer;
use crate::sbsp::math;
use crate::sbsp::runtime::SbspContext;
use crate::sbsp::token::SbspToken;
use crate::sbsp::value::SbspValue;
use crate::ShmPagePool;
use heapless::String;

/// Evaluate a condition expression into a strict Boolean
///
/// Delegates directly to `math::evaluate` which handles the full
/// expression grammar including comparisons (==, !=, <, >, <=, >=),
/// arithmetic, and identifiers.
///
/// # Example
/// ```ignore
/// evaluate_condition("x + 5 > 10", &ctx)?  // → true if x + 5 > 10
/// ```
pub fn evaluate_condition(expr: &str, ctx: &SbspContext) -> Result<bool, String<256>> {
    let trimmed = expr.trim();
    match math::evaluate(trimmed, ctx)? {
        SbspValue::Bool(b) => Ok(b),
        SbspValue::Number(n) => Ok(n != 0),
        other => {
            let mut msg = String::new();
            let _ = core::fmt::write(&mut msg, format_args!(
                "Condition must evaluate to Boolean or Integer, got {}.",
                other.type_name()
            ));
            Err(msg)
        }
    }
}

/// Evaluate a condition with SHM pool for native function calls.
pub fn evaluate_condition_with_shm(
    expr: &str,
    ctx: &SbspContext,
    pool: &ShmPagePool,
) -> Result<bool, String<256>> {
    let trimmed = expr.trim();
    match math::evaluate_with_shm(trimmed, ctx, pool)? {
        SbspValue::Bool(b) => Ok(b),
        SbspValue::Number(n) => Ok(n != 0),
        other => {
            let mut msg = String::new();
            let _ = core::fmt::write(&mut msg, format_args!(
                "Condition must evaluate to Boolean or Integer, got {}.",
                other.type_name()
            ));
            Err(msg)
        }
    }
}

/// Control flow state machine for nested IF/ELSE blocks
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlFlowState {
    /// Currently executing code
    Executing,
    /// Skipping code until ELSE or END IF (false IF condition)
    Skipping,
    /// After ELSE in a false IF block (now execute this part)
    ExecutingElse,
}

/// Token skipping for false IF conditions
///
/// When an IF condition is false, we fast-forward the lexer to the matching
/// ELSE or END IF, counting nesting depth to handle nested IFs correctly.
///
/// # Arguments
/// * `tokens` - Iterator of tokens to skip through
/// * `should_execute_else` - If true, return when we find ELSE (so caller can execute it)
///                           If false, skip past ELSE/END IF entirely
///
/// # Returns
/// Whether we stopped at an ELSE (Some(true)) or went to END IF (Some(false))
pub fn skip_to_control_point(
    tokens: &mut dyn Iterator<Item = SbspToken>,
    stop_at_else: bool,
) -> Option<bool> {
    let mut nesting_depth = 0;

    while let Some(token) = tokens.next() {
        match token {
            // Entering a nested IF (deeper)
            SbspToken::If(_) => {
                nesting_depth += 1;
            }

            // Potential ELSE
            SbspToken::Else => {
                if nesting_depth == 0 {
                    // This is OUR ELSE (not nested)
                    if stop_at_else {
                        return Some(true); // Found ELSE, caller should execute it
                    }
                    // Otherwise, keep skipping to END IF
                }
            }

            // Exiting an IF block
            SbspToken::EndIf => {
                if nesting_depth == 0 {
                    // This is OUR END IF (not nested)
                    return Some(false); // Reached END IF
                }
                // Exiting a nested IF
                nesting_depth -= 1;
            }

            // All other tokens (Text, Output, DIM, expressions, etc.)
            // are simply discarded—they're part of the skipped block
            _ => {}
        }
    }

    // Reached end of token stream without finding END IF
    // (malformed template, but we handle gracefully)
    None
}

/// Zero-allocation IF-block skipper for SbspLexer
///
/// Fast-forwards the lexer past a false-IF body (and optionally the ELSE
/// branch), counting nesting depth to correctly handle nested IFs.
///
/// Unlike `skip_to_control_point` (which works on owned tokens), this
/// function operates on the borrowed-token lexer so the caller can
/// continue consuming tokens from the same lexer after the skip.
///
/// # Returns
/// - `Some(true)`  → stopped at matching ELSE (lexer positioned after ELSE)
/// - `Some(false)` → stopped at matching END IF (no ELSE in this block)
/// - `None`        → end of input before finding ELSE or END IF (malformed)
pub fn skip_to_else_or_endif<'a>(lexer: &mut SbspLexer<'a>) -> Option<bool> {
    let mut nesting_depth: usize = 0;
    while let Some(token) = lexer.next() {
        use crate::sbsp::lexer::SbspToken as Lx;
        match token {
            Lx::If(_) => nesting_depth += 1,
            Lx::Else if nesting_depth == 0 => return Some(true),
            Lx::EndIf if nesting_depth == 0 => return Some(false),
            Lx::EndIf => nesting_depth -= 1,
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_evaluate_condition_comparison_gt() {
        let mut ctx = SbspContext::new();
        ctx.declare("x", "Integer", Some(SbspValue::Number(10)))
            .unwrap();
        let result = evaluate_condition("x > 5", &ctx).unwrap();
        assert!(result);
    }

    #[test]
    fn test_evaluate_condition_comparison_lt() {
        let mut ctx = SbspContext::new();
        ctx.declare("x", "Integer", Some(SbspValue::Number(3)))
            .unwrap();
        let result = evaluate_condition("x > 5", &ctx).unwrap();
        assert!(!result);
    }

    #[test]
    fn test_evaluate_condition_equality() {
        let mut ctx = SbspContext::new();
        ctx.declare("x", "Integer", Some(SbspValue::Number(5)))
            .unwrap();
        let result = evaluate_condition("x == 5", &ctx).unwrap();
        assert!(result);
    }

    #[test]
    fn test_evaluate_condition_inequality() {
        let mut ctx = SbspContext::new();
        ctx.declare("x", "Integer", Some(SbspValue::Number(5)))
            .unwrap();
        let result = evaluate_condition("x != 5", &ctx).unwrap();
        assert!(!result);
    }

    #[test]
    fn test_evaluate_condition_less_equal() {
        let mut ctx = SbspContext::new();
        ctx.declare("age", "Integer", Some(SbspValue::Number(18)))
            .unwrap();
        let result = evaluate_condition("age <= 18", &ctx).unwrap();
        assert!(result);
    }

    #[test]
    fn test_evaluate_condition_greater_equal() {
        let mut ctx = SbspContext::new();
        ctx.declare("age", "Integer", Some(SbspValue::Number(18)))
            .unwrap();
        let result = evaluate_condition("age >= 18", &ctx).unwrap();
        assert!(result);
    }

    #[test]
    fn test_evaluate_condition_truthy_number() {
        let mut ctx = SbspContext::new();
        ctx.declare("flag", "Integer", Some(SbspValue::Number(1)))
            .unwrap();
        let result = evaluate_condition("flag", &ctx).unwrap();
        assert!(result);
    }

    #[test]
    fn test_evaluate_condition_falsy_number() {
        let mut ctx = SbspContext::new();
        ctx.declare("flag", "Integer", Some(SbspValue::Number(0)))
            .unwrap();
        let result = evaluate_condition("flag", &ctx).unwrap();
        assert!(!result);
    }

    #[test]
    fn test_evaluate_condition_boolean() {
        let mut ctx = SbspContext::new();
        ctx.declare("active", "Boolean", Some(SbspValue::Bool(true)))
            .unwrap();
        let result = evaluate_condition("active", &ctx).unwrap();
        assert!(result);
    }

    #[test]
    fn test_evaluate_condition_compound_expression() {
        let mut ctx = SbspContext::new();
        ctx.declare("x", "Integer", Some(SbspValue::Number(10)))
            .unwrap();
        ctx.declare("y", "Integer", Some(SbspValue::Number(3)))
            .unwrap();
        // x + 5 > y + 7  →  15 > 10  →  true
        let result = evaluate_condition("x + 5 > y + 7", &ctx).unwrap();
        assert!(result);
    }

    #[test]
    fn test_evaluate_condition_boolean_var() {
        let mut ctx = SbspContext::new();
        ctx.declare("active", "Boolean", Some(SbspValue::Bool(true)))
            .unwrap();
        let result = evaluate_condition("active", &ctx).unwrap();
        assert!(result);
    }

    #[test]
    fn test_evaluate_condition_truthy_number() {
        let mut ctx = SbspContext::new();
        ctx.declare("flag", "Integer", Some(SbspValue::Number(1)))
            .unwrap();
        let result = evaluate_condition("flag", &ctx).unwrap();
        assert!(result);
    }

    #[test]
    fn test_evaluate_condition_falsy_number() {
        let mut ctx = SbspContext::new();
        ctx.declare("flag", "Integer", Some(SbspValue::Number(0)))
            .unwrap();
        let result = evaluate_condition("flag", &ctx).unwrap();
        assert!(!result);
    }

    #[test]
    fn test_evaluate_condition_type_mismatch() {
        let mut ctx = SbspContext::new();
        ctx.declare("name", "String", Some(SbspValue::String(heapless::String::new())))
            .unwrap();
        // Math parser will try to parse "name > 5" and fail because
        // String cannot be compared with Integer
        let result = evaluate_condition("name > 5", &ctx);
        assert!(result.is_err());
    }

    #[test]
    fn test_skip_to_else_or_endif_finds_else() {
        let input = "some text {% ELSE %}after else{% END IF %}trailing";
        let mut lexer = SbspLexer::new(input);
        // Consume the first token (Text) to position inside the IF block
        let _ = lexer.next();
        let result = skip_to_else_or_endif(&mut lexer);
        assert_eq!(result, Some(true));
        // Lexer should now be after ELSE
        let next = lexer.next();
        assert_eq!(next, Some(crate::sbsp::lexer::SbspToken::Text("after else")));
    }

    #[test]
    fn test_skip_to_else_or_endif_finds_end_if() {
        let input = "some text{% END IF %}trailing";
        let mut lexer = SbspLexer::new(input);
        let _ = lexer.next();
        let result = skip_to_else_or_endif(&mut lexer);
        assert_eq!(result, Some(false));
        let next = lexer.next();
        assert_eq!(next, Some(crate::sbsp::lexer::SbspToken::Text("trailing")));
    }

    #[test]
    fn test_skip_to_else_or_endif_nested() {
        let input = "{% IF a > 5 THEN %}inner body{% END IF %}{% ELSE %}outer else{% END IF %}tail";
        let mut lexer = SbspLexer::new(input);
        let result = skip_to_else_or_endif(&mut lexer);
        assert_eq!(result, Some(true));
        let next = lexer.next();
        assert_eq!(next, Some(crate::sbsp::lexer::SbspToken::Text("outer else")));
    }

    #[test]
    fn test_skip_to_else_or_endif_eof() {
        let input = "no end in sight";
        let mut lexer = SbspLexer::new(input);
        let _ = lexer.next();
        let result = skip_to_else_or_endif(&mut lexer);
        assert_eq!(result, None);
    }
}
