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

use crate::sbsp::math::evaluate_math;
use crate::sbsp::runtime::SbspContext;
use crate::sbsp::token::SbspToken;
use crate::sbsp::value::SbspValue;
use heapless::String;

/// Evaluate a condition expression into a strict Boolean
///
/// Supports:
/// - Math comparisons: x > 5, age >= 18
/// - Equality: x == 5, name != "admin"
/// - Boolean logic: (could extend to AND/OR)
///
/// # Example
/// ```ignore
/// evaluate_condition("x > 10", &ctx)?  // → true if x > 10
/// ```
pub fn evaluate_condition(expr: &str, ctx: &SbspContext) -> Result<bool, String<256>> {
    let trimmed = expr.trim();

    // Try to parse as a comparison: "x > 5", "age == 18", etc.
    if let Some((left, op, right)) = parse_comparison(trimmed) {
        let left_val = evaluate_math(left, ctx)?;
        let right_val = evaluate_math(right, ctx)?;

        match (left_val, right_val) {
            (SbspValue::Number(l), SbspValue::Number(r)) => {
                return Ok(match op {
                    "==" => l == r,
                    "!=" => l != r,
                    "<" => l < r,
                    ">" => l > r,
                    "<=" => l <= r,
                    ">=" => l >= r,
                    _ => false,
                });
            }
            _ => {
                let mut msg = String::new();
                let _ = core::fmt::write(&mut msg, format_args!(
                    "Type mismatch in comparison: can only compare Integer types."
                ));
                return Err(msg);
            }
        }
    }

    // Try to evaluate as a single variable or boolean
    if let Ok(val) = ctx.get(trimmed) {
        match val {
            SbspValue::Bool(b) => return Ok(b),
            SbspValue::Number(n) => return Ok(n != 0), // Truthy: non-zero
            _ => {
                let mut msg = String::new();
                let _ = core::fmt::write(&mut msg, format_args!(
                    "Condition must evaluate to Boolean, got {}.",
                    val.type_name()
                ));
                return Err(msg);
            }
        }
    }

    // Try to evaluate as a math expression (returns 0 or non-zero)
    match evaluate_math(trimmed, ctx) {
        Ok(SbspValue::Number(n)) => Ok(n != 0),
        Ok(other) => {
            let mut msg = String::new();
            let _ = core::fmt::write(&mut msg, format_args!(
                "Condition must be Boolean or Integer, got {}.",
                other.type_name()
            ));
            Err(msg)
        }
        Err(e) => Err(e),
    }
}

/// Parse a comparison expression: "x > 5" → ("x", ">", "5")
///
/// Supports: ==, !=, <, >, <=, >=
fn parse_comparison(expr: &str) -> Option<(&str, &str, &str)> {
    // Try each operator (longest first to avoid "=" matching "==")
    for op in &["==", "!=", "<=", ">=", "<", ">"] {
        if let Some(pos) = expr.find(op) {
            let left = expr[..pos].trim();
            let right = expr[pos + op.len()..].trim();
            if !left.is_empty() && !right.is_empty() {
                return Some((left, op, right));
            }
        }
    }
    None
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
    fn test_parse_comparison_greater_than() {
        let (left, op, right) = parse_comparison("x > 5").unwrap();
        assert_eq!(left, "x");
        assert_eq!(op, ">");
        assert_eq!(right, "5");
    }

    #[test]
    fn test_parse_comparison_equal() {
        let (left, op, right) = parse_comparison("age == 18").unwrap();
        assert_eq!(left, "age");
        assert_eq!(op, "==");
        assert_eq!(right, "18");
    }

    #[test]
    fn test_parse_comparison_not_equal() {
        let (left, op, right) = parse_comparison("status != 0").unwrap();
        assert_eq!(left, "status");
        assert_eq!(op, "!=");
        assert_eq!(right, "0");
    }

    #[test]
    fn test_parse_comparison_with_spaces() {
        let (left, op, right) = parse_comparison("  x   >=   10  ").unwrap();
        assert_eq!(left, "x");
        assert_eq!(op, ">=");
        assert_eq!(right, "10");
    }

    #[test]
    fn test_evaluate_condition_type_mismatch() {
        let mut ctx = SbspContext::new();
        ctx.declare("name", "String", Some(SbspValue::String(heapless::String::new())))
            .unwrap();
        let result = evaluate_condition("name > 5", &ctx);
        assert!(result.is_err());
    }
}
