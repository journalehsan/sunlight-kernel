//! SBSP Runtime Execution Context
//!
//! Phase 3.2: Symbol table and strict type enforcement
//!
//! The SbspContext is the "brain" of a single HTTP request execution.
//! It maintains variable state, enforces strict typing (no silent casts),
//! and provides clean error messages when students make mistakes.
//!
//! Design:
//! - Variables stored as (name, type, value) triplets
//! - Type lock: once DIM x AS Integer, x can ONLY hold integers
//! - Errors are caught immediately and reported via 500 response
//! - Perfect for teaching: students see EXACTLY what went wrong
//!
//! Educational Value:
//! Student writes: {% DIM x AS Integer %} {% x = "hello" %}
//! Engine responds: Type mismatch: Variable 'x' is an Integer, but you tried to assign a String.
//! Student learns: Types matter, catch mistakes early!

use crate::sbsp::value::SbspValue;
use heapless::{String, Vec};

/// Maximum number of variables per request (stack-allocated)
const MAX_VARIABLES: usize = 256;

/// A variable declaration (name, type, value)
#[derive(Debug, Clone)]
struct Variable {
    name: String<64>,
    typ: String<64>,
    value: SbspValue,
}

/// Form field entry (key-value pair from HTTP POST)
#[derive(Debug, Clone)]
struct FormField {
    key: String<256>,
    value: String<256>,
}

/// The execution context for a single SBSP request
///
/// Think of this as the "workspace" for a single student's script.
/// All variables declared in this request live here, and the context
/// enforces strict typing rules.
///
/// Also stores incoming HTTP form data (POST requests) so REQUEST()
/// can access user-submitted form fields.
pub struct SbspContext {
    /// All variables: stored as (name, expected_type, current_value) tuples
    /// Using heapless::Vec for zero heap allocation
    variables: Vec<Variable, MAX_VARIABLES>,
    /// HTTP form data from POST requests (key=value pairs)
    form_data: Vec<FormField, 32>,
}

impl SbspContext {
    /// Create a new execution context for this request
    pub fn new() -> Self {
        Self {
            variables: Vec::new(),
            form_data: Vec::new(),
        }
    }

    /// Create a new execution context with form data (from HTTP POST)
    pub fn with_form_data(form_pairs: &[(&str, &str)]) -> Self {
        let mut ctx = Self::new();
        for (key, value) in form_pairs {
            let field = FormField {
                key: String::from(key),
                value: String::from(value),
            };
            let _ = ctx.form_data.push(field);
        }
        ctx
    }

    /// Handle: {% DIM var AS Type [= init] %}
    ///
    /// # Arguments
    /// * `name` - Variable name (e.g., "x", "user_count")
    /// * `type_name` - Type string (e.g., "Integer", "String", "Boolean")
    /// * `initial_value` - Optional initial value (None → default)
    ///
    /// # Returns
    /// - `Ok(())` - Variable declared successfully
    /// - `Err(msg)` - Type error or redefinition
    pub fn declare(
        &mut self,
        name: &str,
        type_name: &str,
        initial_value: Option<SbspValue>,
    ) -> Result<(), heapless::String<256>> {
        // Check if variable already exists (redefinition error)
        if self.variables.iter().any(|v| v.name.as_str() == name) {
            let mut msg = heapless::String::new();
            let _ = core::fmt::write(&mut msg, format_args!("Variable '{}' is already defined.", name));
            return Err(msg);
        }

        // Determine the initial value
        let value = match initial_value {
            Some(v) => v,
            None => {
                // Apply type-specific defaults
                match type_name {
                    "Integer" => SbspValue::Number(0),
                    "String" => SbspValue::String(heapless::String::new()),
                    "Boolean" => SbspValue::Bool(false),
                    _ => {
                        let mut msg = heapless::String::new();
                        let _ = core::fmt::write(&mut msg, format_args!("Unknown type '{}' for variable '{}'.", type_name, name));
                        return Err(msg);
                    }
                }
            }
        };

        // STRICT TYPE CHECK: Ensure the initial value matches the declared type
        if value.type_name() != type_name {
            let mut msg = heapless::String::new();
            let _ = core::fmt::write(
                &mut msg,
                format_args!(
                    "Type mismatch on declaration: Cannot assign {} to variable '{}' of type {}.",
                    value.type_name(),
                    name,
                    type_name
                ),
            );
            return Err(msg);
        }

        // Create the variable and store it
        let var = Variable {
            name: heapless::String::from(name),
            typ: heapless::String::from(type_name),
            value,
        };

        self.variables.push(var).map_err(|_| {
            heapless::String::from("Too many variables: maximum 256 per request.")
        })?;

        Ok(())
    }

    /// Handle: {% var = expression_result %}
    ///
    /// Assigns a new value to an existing variable.
    /// STRICT TYPE CHECK: The new value MUST match the variable's declared type.
    ///
    /// # Arguments
    /// * `name` - Variable name
    /// * `new_value` - The value to assign
    ///
    /// # Returns
    /// - `Ok(())` - Assignment successful
    /// - `Err(msg)` - Undefined variable or type mismatch
    pub fn assign(&mut self, name: &str, new_value: SbspValue) -> Result<(), heapless::String<256>> {
        // Find the variable
        let var = self
            .variables
            .iter_mut()
            .find(|v| v.name.as_str() == name)
            .ok_or_else(|| {
                heapless::String::from("Variable is not defined. Use DIM first.")
            })?;

        // STRICT TYPE CHECK: Ensure the new value matches the declared type
        if var.value.type_name() != new_value.type_name() {
            let mut msg = heapless::String::new();
            let _ = core::fmt::write(
                &mut msg,
                format_args!(
                    "Type mismatch: Variable '{}' is a {}, but you tried to assign a {}.",
                    name,
                    var.value.type_name(),
                    new_value.type_name()
                ),
            );
            return Err(msg);
        }

        // Assignment approved: update the value
        var.value = new_value;
        Ok(())
    }

    /// Retrieve a variable for reading (e.g., {%| var %} or math)
    pub fn get(&self, name: &str) -> Result<&SbspValue, heapless::String<256>> {
        self.variables
            .iter()
            .find(|v| v.name.as_str() == name)
            .map(|v| &v.value)
            .ok_or_else(|| {
                let mut msg = heapless::String::new();
                let _ = core::fmt::write(&mut msg, format_args!("Variable '{}' is not defined.", name));
                msg
            })
    }

    /// Get all variables (for debugging)
    pub fn variables(&self) -> &[Variable] {
        &self.variables
    }

    /// Clear all variables (for pooling contexts)
    pub fn clear(&mut self) {
        self.variables.clear();
    }

    /// Declare or assign a variable (used for loop counters)
    ///
    /// If the variable doesn't exist, declare it.
    /// If it exists, overwrite with strict type checking.
    pub fn declare_or_assign(
        &mut self,
        name: &str,
        type_name: &str,
        value: SbspValue,
    ) -> Result<(), heapless::String<256>> {
        // Check type match
        if value.type_name() != type_name {
            let mut msg = heapless::String::new();
            let _ = core::fmt::write(&mut msg, format_args!(
                "Type mismatch: Variable '{}' expects {}, got {}.",
                name,
                type_name,
                value.type_name()
            ));
            return Err(msg);
        }

        // Find and update if exists
        for var in &mut self.variables {
            if var.name.as_str() == name {
                var.value = value;
                return Ok(());
            }
        }

        // Otherwise declare new
        self.declare(name, type_name, Some(value))
    }

    /// Handle REQUEST(field_name) - retrieve HTTP form data
    ///
    /// Returns the value from POST form data, or empty string if not found.
    /// This allows SBSP scripts to read user-submitted form fields.
    pub fn request(&self, field_name: &str) -> Result<SbspValue, heapless::String<256>> {
        for field in &self.form_data {
            if field.key.as_str() == field_name {
                return Ok(SbspValue::String(field.value.clone()));
            }
        }
        // Field not found: return empty string (not an error)
        Ok(SbspValue::String(heapless::String::new()))
    }
}

/// Loop state for FOR loops (tracks rewind point and end condition)
pub struct ForLoopState<'a> {
    /// Variable name for the loop counter
    pub var_name: heapless::String<64>,
    /// End value (when loop_var > end_val, loop exits)
    pub end_val: i64,
    /// Snapshot of lexer at start of loop body (for rewinding)
    pub lexer_snapshot: crate::sbsp::lexer::SbspLexer<'a>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_declare_integer() {
        let mut ctx = SbspContext::new();
        let result = ctx.declare("x", "Integer", Some(SbspValue::Number(42)));
        assert!(result.is_ok());
        assert_eq!(ctx.get("x").unwrap(), &SbspValue::Number(42));
    }

    #[test]
    fn test_declare_string() {
        let mut ctx = SbspContext::new();
        let s = heapless::String::from("hello");
        let result = ctx.declare("name", "String", Some(SbspValue::String(s)));
        assert!(result.is_ok());
        assert_eq!(
            ctx.get("name").unwrap(),
            &SbspValue::String(heapless::String::from("hello"))
        );
    }

    #[test]
    fn test_declare_boolean() {
        let mut ctx = SbspContext::new();
        let result = ctx.declare("flag", "Boolean", Some(SbspValue::Bool(true)));
        assert!(result.is_ok());
        assert_eq!(ctx.get("flag").unwrap(), &SbspValue::Bool(true));
    }

    #[test]
    fn test_declare_default_integer() {
        let mut ctx = SbspContext::new();
        let result = ctx.declare("count", "Integer", None);
        assert!(result.is_ok());
        assert_eq!(ctx.get("count").unwrap(), &SbspValue::Number(0));
    }

    #[test]
    fn test_declare_default_string() {
        let mut ctx = SbspContext::new();
        let result = ctx.declare("text", "String", None);
        assert!(result.is_ok());
        assert_eq!(
            ctx.get("text").unwrap(),
            &SbspValue::String(heapless::String::new())
        );
    }

    #[test]
    fn test_declare_default_boolean() {
        let mut ctx = SbspContext::new();
        let result = ctx.declare("active", "Boolean", None);
        assert!(result.is_ok());
        assert_eq!(ctx.get("active").unwrap(), &SbspValue::Bool(false));
    }

    #[test]
    fn test_declare_unknown_type() {
        let mut ctx = SbspContext::new();
        let result = ctx.declare("bad", "UnknownType", None);
        assert!(result.is_err());
    }

    #[test]
    fn test_declare_redefinition() {
        let mut ctx = SbspContext::new();
        ctx.declare("x", "Integer", Some(SbspValue::Number(5)))
            .unwrap();
        let result = ctx.declare("x", "Integer", Some(SbspValue::Number(10)));
        assert!(result.is_err());
    }

    #[test]
    fn test_declare_type_mismatch() {
        let mut ctx = SbspContext::new();
        // Try to declare Integer but provide String
        let result = ctx.declare("x", "Integer", Some(SbspValue::String(heapless::String::new())));
        assert!(result.is_err());
    }

    #[test]
    fn test_assign_success() {
        let mut ctx = SbspContext::new();
        ctx.declare("x", "Integer", Some(SbspValue::Number(5)))
            .unwrap();
        let result = ctx.assign("x", SbspValue::Number(42));
        assert!(result.is_ok());
        assert_eq!(ctx.get("x").unwrap(), &SbspValue::Number(42));
    }

    #[test]
    fn test_assign_undefined_variable() {
        let mut ctx = SbspContext::new();
        let result = ctx.assign("undefined", SbspValue::Number(42));
        assert!(result.is_err());
    }

    #[test]
    fn test_assign_type_mismatch() {
        let mut ctx = SbspContext::new();
        ctx.declare("x", "Integer", Some(SbspValue::Number(5)))
            .unwrap();
        // Try to assign String to Integer variable
        let result = ctx.assign("x", SbspValue::String(heapless::String::from("oops")));
        assert!(result.is_err());
    }

    #[test]
    fn test_multiple_variables() {
        let mut ctx = SbspContext::new();
        ctx.declare("x", "Integer", Some(SbspValue::Number(5)))
            .unwrap();
        ctx.declare("name", "String", Some(SbspValue::String(heapless::String::from("Alice"))))
            .unwrap();
        ctx.declare("active", "Boolean", Some(SbspValue::Bool(true)))
            .unwrap();

        assert_eq!(ctx.get("x").unwrap(), &SbspValue::Number(5));
        assert_eq!(
            ctx.get("name").unwrap(),
            &SbspValue::String(heapless::String::from("Alice"))
        );
        assert_eq!(ctx.get("active").unwrap(), &SbspValue::Bool(true));
    }

    #[test]
    fn test_clear_variables() {
        let mut ctx = SbspContext::new();
        ctx.declare("x", "Integer", Some(SbspValue::Number(5)))
            .unwrap();
        assert!(ctx.get("x").is_ok());
        ctx.clear();
        assert!(ctx.get("x").is_err());
    }

    #[test]
    fn test_declare_or_assign_new_variable() {
        let mut ctx = SbspContext::new();
        let result = ctx.declare_or_assign("i", "Integer", SbspValue::Number(1));
        assert!(result.is_ok());
        assert_eq!(ctx.get("i").unwrap(), &SbspValue::Number(1));
    }

    #[test]
    fn test_declare_or_assign_existing_variable() {
        let mut ctx = SbspContext::new();
        ctx.declare("i", "Integer", Some(SbspValue::Number(1)))
            .unwrap();
        let result = ctx.declare_or_assign("i", "Integer", SbspValue::Number(2));
        assert!(result.is_ok());
        assert_eq!(ctx.get("i").unwrap(), &SbspValue::Number(2));
    }

    #[test]
    fn test_declare_or_assign_type_mismatch() {
        let mut ctx = SbspContext::new();
        ctx.declare("i", "Integer", Some(SbspValue::Number(1)))
            .unwrap();
        let result = ctx.declare_or_assign("i", "Integer", SbspValue::String(heapless::String::new()));
        assert!(result.is_err());
    }

    #[test]
    fn test_for_loop_state_creation() {
        let input = "<li>Item</li>";
        let lexer = crate::sbsp::lexer::SbspLexer::new(input);

        let loop_state = ForLoopState {
            var_name: heapless::String::from("i"),
            end_val: 10,
            lexer_snapshot: lexer,
        };

        assert_eq!(loop_state.var_name.as_str(), "i");
        assert_eq!(loop_state.end_val, 10);
        assert_eq!(loop_state.lexer_snapshot.remainder, input);
    }

    #[test]
    fn test_for_loop_state_clone() {
        let input = "<li>Item</li>";
        let lexer = crate::sbsp::lexer::SbspLexer::new(input);

        let loop_state = ForLoopState {
            var_name: heapless::String::from("i"),
            end_val: 10,
            lexer_snapshot: lexer,
        };

        // Clone the lexer snapshot (should be O(1) pointer copy)
        let cloned_lexer = loop_state.lexer_snapshot.clone();
        assert_eq!(cloned_lexer.remainder, loop_state.lexer_snapshot.remainder);
    }

    #[test]
    fn test_context_with_form_data() {
        let form_pairs = [("task_name", "Build OS"), ("priority", "High")];
        let ctx = SbspContext::with_form_data(&form_pairs);

        // Context should have the form data
        assert_eq!(ctx.form_data.len(), 2);
        assert_eq!(ctx.form_data[0].key.as_str(), "task_name");
        assert_eq!(ctx.form_data[0].value.as_str(), "Build OS");
    }

    #[test]
    fn test_request_existing_field() {
        let form_pairs = [("name", "Alice"), ("age", "30")];
        let ctx = SbspContext::with_form_data(&form_pairs);

        let result = ctx.request("name").unwrap();
        assert_eq!(result, SbspValue::String(heapless::String::from("Alice")));
    }

    #[test]
    fn test_request_missing_field() {
        let form_pairs = [("name", "Alice")];
        let ctx = SbspContext::with_form_data(&form_pairs);

        // Missing field returns empty string (not an error)
        let result = ctx.request("missing").unwrap();
        assert_eq!(result, SbspValue::String(heapless::String::new()));
    }

    #[test]
    fn test_request_empty_form() {
        let ctx = SbspContext::new();

        // Any field returns empty string
        let result = ctx.request("anything").unwrap();
        assert_eq!(result, SbspValue::String(heapless::String::new()));
    }
}
