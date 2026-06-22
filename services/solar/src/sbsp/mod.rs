//! SBSP (Solar Basic Server Pages) Engine
//!
//! Phase 3-4: Scripting language lexer, parser, and runtime
//! Executes `.sbsp` files with embedded Rust-like expressions and control flow.
//!
//! Architecture:
//! - Phase 3.1: Streaming Lexer (Iterator-based token generator)
//! - Phase 3.2: Runtime Context (Symbol table, type enforcement)
//! - Phase 3.3: Expression Evaluator (Math, logic, function calls)
//! - Phase 3.4: Control Flow (IF/FOR/WHILE execution engine)

pub mod lexer;
pub mod token;
pub mod value;
pub mod runtime;
pub mod math;
pub mod control;
pub mod execute;

pub use lexer::SbspLexer;
pub use token::SbspToken;
pub use value::SbspValue;
pub use runtime::SbspContext;
pub use math::{evaluate_math, tokenize_math, MathParser, MathToken};
pub use control::{evaluate_condition, skip_to_control_point, ControlFlowState};
pub use execute::SbspExecutor;
