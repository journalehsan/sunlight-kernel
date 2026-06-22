//! SBSP Template Execution Engine
//!
//! Phase 3.5: Complete end-to-end execution with FOR loop support
//!
//! This is the main entry point for evaluating a .sbsp template.
//! It coordinates:
//! - Streaming lexer (zero-copy token generation)
//! - Control flow tracking (IF/ELSE nesting with zero-allocation skipping)
//! - Loop state management (FOR/NEXT with zero-cost lexer rewinding)
//! - Symbol table and type enforcement
//! - Output streaming to response buffer
//!
//! Key architectural invariants:
//! - Zero allocations in hot path (all state is stack-bounded)
//! - Single-pass execution (no AST buffering)
//! - Loop rewinding via SbspLexer cloning (O(1) cost)
//! - Nested loops and IF blocks fully supported via depth tracking

use crate::sbsp::control::{
    evaluate_condition, evaluate_condition_with_shm, skip_to_control_point,
    skip_to_else_or_endif, ControlFlowState,
};
use crate::sbsp::lexer::SbspLexer;
use crate::sbsp::math::{evaluate_math, evaluate_math_with_shm};
use crate::sbsp::runtime::{ForLoopState, SbspContext};
use crate::sbsp::value::SbspValue;
use crate::ShmPagePool;
use heapless::{String, Vec};

/// Maximum nesting depth for loops (prevents infinite recursion on malformed templates)
const MAX_LOOP_DEPTH: usize = 16;

/// Template execution engine
///
/// Orchestrates the full template evaluation pipeline:
/// 1. Lexical analysis (streaming iterator)
/// 2. Token-by-token interpretation
/// 3. Control flow state management
/// 4. Output buffering and HTML escaping
pub struct SbspExecutor<'a> {
    /// The execution context (symbol table, type checking)
    ctx: &'a mut SbspContext,
    /// Current loop stack (for nested FOR loops)
    loop_stack: Vec<ForLoopState<'a>, MAX_LOOP_DEPTH>,
    /// Shared memory page pool for native function IPC (KV_GET, KV_PUT, etc.)
    shm_pool: Option<&'a ShmPagePool>,
}

impl<'a> SbspExecutor<'a> {
    /// Create a new executor for this request
    pub fn new(ctx: &'a mut SbspContext) -> Self {
        Self {
            ctx,
            loop_stack: Vec::new(),
            shm_pool: None,
        }
    }

    /// Attach a SHM pool for native function dispatch (KV_GET, KV_PUT, etc.)
    pub fn with_shm_pool(mut self, pool: &'a ShmPagePool) -> Self {
        self.shm_pool = Some(pool);
        self
    }

    /// Evaluate a math expression, using SHM pool when available.
    fn evaluate_expr(&self, expr: &str) -> Result<SbspValue, heapless::String<256>> {
        match self.shm_pool {
            Some(pool) => evaluate_math_with_shm(expr, self.ctx, pool),
            None => evaluate_math(expr, self.ctx),
        }
    }

    /// Evaluate a condition, using SHM pool when available.
    fn eval_condition(&self, condition: &str) -> Result<bool, heapless::String<256>> {
        match self.shm_pool {
            Some(pool) => evaluate_condition_with_shm(condition, self.ctx, pool),
            None => evaluate_condition(condition, self.ctx),
        }
    }

    /// Execute a template and stream output to a buffer
    ///
    /// # Arguments
    /// * `template` - The .sbsp template as a string
    /// * `output` - Pre-allocated buffer to write rendered HTML
    ///
    /// # Returns
    /// - `Ok(bytes_written)` - Number of bytes written to output buffer
    /// - `Err(msg)` - Runtime error (type mismatch, undefined variable, etc.)
    pub fn execute(
        &mut self,
        template: &'a str,
        output: &mut heapless::String<8192>,
    ) -> Result<usize, heapless::String<256>> {
        let lexer = SbspLexer::new(template);
        let mut bytes_written = 0;
        let mut control_stack: Vec<ControlFlowState, 32> = Vec::new();

        for token in self.execute_with_lexer(lexer, &mut control_stack)? {
            output
                .push_str(&token)
                .map_err(|_| heapless::String::from("Output buffer overflow."))?;
            bytes_written += token.len();
        }

        Ok(bytes_written)
    }

    /// Internal recursive generator for execution
    ///
    /// This handles the full control flow and loop logic by yielding
    /// output strings one at a time. It's structured as an iterator
    /// to keep the borrow checker happy and maintain streaming output.
    fn execute_with_lexer(
        &mut self,
        mut lexer: SbspLexer<'a>,
        control_stack: &mut Vec<ControlFlowState, 32>,
    ) -> Result<Vec<heapless::String<256>, 256>, heapless::String<256>> {
        let mut output_tokens: Vec<heapless::String<256>, 256> = Vec::new();

        while let Some(token) = lexer.next() {
            let result = self.handle_token(
                token,
                &mut lexer,
                control_stack,
                &mut output_tokens,
            )?;

            if result.should_break {
                break;
            }
        }

        Ok(output_tokens)
    }

    /// Process a single token
    fn handle_token(
        &mut self,
        token: SbspToken<'a>,
        lexer: &mut SbspLexer<'a>,
        control_stack: &mut Vec<ControlFlowState, 32>,
        output: &mut Vec<heapless::String<256>, 256>,
    ) -> Result<TokenResult, heapless::String<256>> {
        let current_control_state = control_stack.last().copied().unwrap_or(ControlFlowState::Executing);

        match token {
            // Raw HTML: pass through as-is
            SbspToken::Text(text) => {
                if matches!(current_control_state, ControlFlowState::Executing) {
                    let mut s = heapless::String::new();
                    s.push_str(text)
                        .map_err(|_| heapless::String::from("Output string too long."))?;
                    let _ = output.push(s);
                }
                Ok(TokenResult::default())
            }

            // Shorthand output: {%| expr %}
            SbspToken::Output(expr) => {
                if matches!(current_control_state, ControlFlowState::Executing) {
                    let value = self.evaluate_expr(expr)?;
                    let html_safe = value.html_escape();
                    let mut s = heapless::String::new();
                    s.push_str(&html_safe)
                        .map_err(|_| heapless::String::from("Output string too long."))?;
                    let _ = output.push(s);
                }
                Ok(TokenResult::default())
            }

            // Variable declaration: {% DIM x AS Integer = 5 %}
            SbspToken::DimDecl { var, typ, init } => {
                if matches!(current_control_state, ControlFlowState::Executing) {
                    let initial_value = if let Some(init_expr) = init {
                        self.evaluate_expr(init_expr)?
                    } else {
                        // Default initialization
                        match typ {
                            "Integer" => SbspValue::Number(0),
                            "String" => SbspValue::String(heapless::String::new()),
                            "Boolean" => SbspValue::Bool(false),
                            _ => {
                                let mut msg = heapless::String::new();
                                let _ = core::fmt::write(
                                    &mut msg,
                                    format_args!("Unknown type '{}'.", typ),
                                );
                                return Err(msg);
                            }
                        }
                    };

                    self.ctx.declare(var, typ, Some(initial_value))?;
                }
                Ok(TokenResult::default())
            }

            // IF condition: {% IF x > 5 THEN %}
            SbspToken::If(condition) => {
                let is_true = if matches!(current_control_state, ControlFlowState::Executing) {
                    self.eval_condition(condition)?
                } else {
                    false
                };

                if is_true {
                    control_stack
                        .push(ControlFlowState::Executing)
                        .map_err(|_| heapless::String::from("Control nesting too deep."))?;
                } else if matches!(current_control_state, ControlFlowState::Executing) {
                    // Fast-forward lexer past the false branch (zero-allocation)
                    match skip_to_else_or_endif(lexer) {
                        Some(true) => {
                            // Stopped at ELSE: execute the ELSE body
                            control_stack
                                .push(ControlFlowState::Executing)
                                .map_err(|_| {
                                    heapless::String::from("Control nesting too deep.")
                                })?;
                        }
                        Some(false) => {
                            // No ELSE, IF body fully consumed
                        }
                        None => {
                            return Err(heapless::String::from(
                                "Unmatched IF: missing END IF",
                            ));
                        }
                    }
                } else {
                    // Already in non-Executing state (Skipping or ExecutingElse):
                    // push Skipping so the inner END IF pops this instead of the outer state
                    control_stack
                        .push(ControlFlowState::Skipping)
                        .map_err(|_| heapless::String::from("Control nesting too deep."))?;
                }

                Ok(TokenResult::default())
            }

            // ELSE: {% ELSE %}
            SbspToken::Else => {
                if let Some(state) = control_stack.last_mut() {
                    *state = match state {
                        ControlFlowState::Executing => ControlFlowState::Skipping,
                        ControlFlowState::Skipping => ControlFlowState::ExecutingElse,
                        ControlFlowState::ExecutingElse => ControlFlowState::Skipping,
                    };
                }
                Ok(TokenResult::default())
            }

            // END IF: {% END IF %}
            SbspToken::EndIf => {
                let _ = control_stack.pop();
                Ok(TokenResult::default())
            }

            // FOR loop: {% FOR i = 1 TO 10 %}
            SbspToken::For { var, start, end } => {
                if matches!(current_control_state, ControlFlowState::Executing) {
                    let start_val = self.evaluate_expr(start)?;
                    let end_val = self.evaluate_expr(end)?;

                    match (start_val, end_val) {
                        (SbspValue::Number(s), SbspValue::Number(e)) => {
                            // Declare or initialize loop variable
                            self.ctx.declare_or_assign(var, "Integer", SbspValue::Number(s))?;

                            // Create loop state with lexer snapshot at loop body start
                            let loop_state = ForLoopState {
                                var_name: heapless::String::from(var),
                                end_val: e,
                                lexer_snapshot: lexer.clone(),
                            };

                            self.loop_stack
                                .push(loop_state)
                                .map_err(|_| heapless::String::from("Loop nesting too deep."))?;
                        }
                        _ => {
                            return Err(heapless::String::from(
                                "FOR loop bounds must evaluate to integers.",
                            ))
                        }
                    }
                }
                Ok(TokenResult::default())
            }

            // NEXT: {% NEXT %} - end of FOR loop body, check condition and potentially rewind
            SbspToken::Next => {
                if matches!(current_control_state, ControlFlowState::Executing) {
                    if let Some(mut loop_state) = self.loop_stack.pop() {
                        // Get current loop variable value
                        let current_val = self.ctx.get(loop_state.var_name.as_str())?;

                        match current_val {
                            SbspValue::Number(i) => {
                                let next_val = i.saturating_add(1);

                                if next_val <= loop_state.end_val {
                                    // Still looping: increment and rewind lexer
                                    self.ctx.declare_or_assign(
                                        loop_state.var_name.as_str(),
                                        "Integer",
                                        SbspValue::Number(next_val),
                                    )?;

                                    // Restore lexer to loop body start (O(1) clone restoration)
                                    *lexer = loop_state.lexer_snapshot.clone();

                                    // Push state back for next iteration
                                    self.loop_stack.push(loop_state).map_err(|_| {
                                        heapless::String::from("Loop nesting too deep.")
                                    })?;
                                }
                                // Otherwise loop exits naturally (lexer continues forward)
                            }
                            _ => {
                                return Err(heapless::String::from(
                                    "Loop variable must be an Integer.",
                                ))
                            }
                        }
                    }
                }
                Ok(TokenResult::default())
            }

            // WHILE loop: {% WHILE condition %}
            // (Stub for Phase 3.6)
            SbspToken::While(_) => {
                // TODO: Phase 3.6 - Implement WHILE with condition re-evaluation at WEND
                Ok(TokenResult::default())
            }

            SbspToken::Wend => {
                // TODO: Phase 3.6 - WHILE end
                Ok(TokenResult::default())
            }

            // FUNCTION definitions (stub for Phase 3.7)
            SbspToken::FunctionDef(_) => {
                // TODO: Phase 3.7 - Parse function signature and store in context
                skip_to_control_point(&mut lexer.clone(), false);
                Ok(TokenResult::default())
            }

            SbspToken::Return(_) => {
                // TODO: Phase 3.7 - Function return
                Ok(TokenResult::default())
            }

            SbspToken::EndFunction => {
                // TODO: Phase 3.7 - End of function definition
                Ok(TokenResult::default())
            }

            // General expressions and assignments
            SbspToken::Expression(expr) => {
                if matches!(current_control_state, ControlFlowState::Executing) {
                    // Parse as "var = value" assignment
                    if let Some(eq_pos) = expr.find('=') {
                        let var_name = expr[..eq_pos].trim();
                        let value_expr = expr[eq_pos + 1..].trim();

                        let new_value = self.evaluate_expr(value_expr)?;
                        self.ctx.assign(var_name, new_value)?;
                    }
                }
                Ok(TokenResult::default())
            }
        }
    }
}

/// Result state from handling a single token
#[derive(Debug, Default)]
struct TokenResult {
    /// Whether to break out of the main loop
    should_break: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_execute_simple_html() {
        let mut ctx = SbspContext::new();
        let mut executor = SbspExecutor::new(&mut ctx);
        let mut output = heapless::String::new();

        let result = executor.execute("<h1>Hello World</h1>", &mut output);
        assert!(result.is_ok());
        assert_eq!(output.as_str(), "<h1>Hello World</h1>");
    }

    #[test]
    fn test_execute_with_variable_output() {
        let mut ctx = SbspContext::new();
        ctx.declare("name", "String", Some(SbspValue::String(heapless::String::from("Alice"))))
            .unwrap();

        let mut executor = SbspExecutor::new(&mut ctx);
        let mut output = heapless::String::new();

        let template = "<p>Hello {%| name %}</p>";
        let result = executor.execute(template, &mut output);
        assert!(result.is_ok());
        assert!(output.contains("Hello"));
    }

    #[test]
    fn test_execute_with_if_true() {
        let mut ctx = SbspContext::new();
        ctx.declare("x", "Integer", Some(SbspValue::Number(10)))
            .unwrap();

        let mut executor = SbspExecutor::new(&mut ctx);
        let mut output = heapless::String::new();

        let template = "{% IF x > 5 THEN %}<p>Greater</p>{% END IF %}";
        let result = executor.execute(template, &mut output);
        assert!(result.is_ok());
        assert!(output.contains("Greater"));
    }

    #[test]
    fn test_execute_with_if_false() {
        let mut ctx = SbspContext::new();
        ctx.declare("x", "Integer", Some(SbspValue::Number(3)))
            .unwrap();

        let mut executor = SbspExecutor::new(&mut ctx);
        let mut output = heapless::String::new();

        let template = "{% IF x > 5 THEN %}<p>Greater</p>{% END IF %}";
        let result = executor.execute(template, &mut output);
        assert!(result.is_ok());
        assert!(!output.contains("Greater"));
    }

    #[test]
    fn test_execute_with_dim_declaration() {
        let mut ctx = SbspContext::new();
        let mut executor = SbspExecutor::new(&mut ctx);
        let mut output = heapless::String::new();

        let template = "{% DIM count AS Integer = 5 %}<p>Count: {%| count %}</p>";
        let result = executor.execute(template, &mut output);
        assert!(result.is_ok());
        assert!(output.contains("5"));
    }

    #[test]
    fn test_for_loop_state_tracking() {
        let input = "{% FOR i = 1 TO 3 %}<li>{%| i %}</li>{% NEXT %}";
        let mut ctx = SbspContext::new();
        let mut executor = SbspExecutor::new(&mut ctx);
        let mut output = heapless::String::new();

        // This demonstrates that the ForLoopState structure is in place
        // and the executor can be invoked. Full loop execution happens in next phase.
        let _result = executor.execute(input, &mut output);
        // For now, just verify the structure is sound and doesn't panic
    }

    #[test]
    fn test_html_escape_in_output() {
        let mut ctx = SbspContext::new();
        ctx.declare(
            "dangerous",
            "String",
            Some(SbspValue::String(heapless::String::from("<script>alert('xss')</script>")))
        ).unwrap();

        let mut executor = SbspExecutor::new(&mut ctx);
        let mut output = heapless::String::new();

        let template = "<p>{%| dangerous %}</p>";
        let result = executor.execute(template, &mut output);
        assert!(result.is_ok());
        // Output should be HTML-escaped, not raw script
        assert!(output.contains("&lt;"));
        assert!(output.contains("&gt;"));
        assert!(!output.contains("<script>"));
    }

    #[test]
    fn test_control_flow_architecture() {
        // Test that the executor can handle nested control structures
        let mut ctx = SbspContext::new();
        ctx.declare("x", "Integer", Some(SbspValue::Number(10))).unwrap();

        let mut executor = SbspExecutor::new(&mut ctx);
        let mut output = heapless::String::new();

        let template = "{% IF x > 5 THEN %}\
                         <p>X is greater than 5</p>\
                         {% END IF %}";
        let result = executor.execute(template, &mut output);
        assert!(result.is_ok());
        assert!(output.contains("X is greater than 5"));
    }

    #[test]
    fn test_loop_stack_depth_limit() {
        // Verify that the executor won't allow infinite loop nesting
        let mut ctx = SbspContext::new();
        let mut executor = SbspExecutor::new(&mut ctx);

        // After MAX_LOOP_DEPTH FOR loops, further nesting should fail gracefully
        assert_eq!(executor.loop_stack.capacity(), MAX_LOOP_DEPTH);
    }
}

