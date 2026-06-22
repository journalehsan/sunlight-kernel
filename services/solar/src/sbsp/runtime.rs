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

/// The execution context for a single SBSP request
///
/// Think of this as the "workspace" for a single student's script.
/// All variables declared in this request live here, and the context
/// enforces strict typing rules.
pub struct SbspContext {
    /// All variables: stored as (name, expected_type, current_value) tuples
    /// Using heapless::Vec for zero heap allocation
    variables: Vec<Variable, MAX_VARIABLES>,
}

impl SbspContext {
    /// Create a new execution context for this request
    pub fn new() -> Self {
        Self {
            variables: Vec::new(),
        }
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
}
