//! SBSP Runtime Values
//!
//! Dynamically-typed value representation for the SBSP language.
//! Supports strict type checking: assigning a String to an Integer variable is a runtime error.

use heapless::Vec;

/// SBSP runtime values with strict typing
#[derive(Debug, Clone, PartialEq)]
pub enum SbspValue {
    /// String type (immutable)
    String(heapless::String<512>),

    /// Integer type (64-bit signed)
    Number(i64),

    /// Boolean type
    Bool(bool),

    /// Array of values (homogeneous)
    Array(Vec<SbspValue, 256>),

    /// Null/undefined value
    Null,
}

impl SbspValue {
    /// Get the type name for error messages
    pub fn type_name(&self) -> &'static str {
        match self {
            SbspValue::String(_) => "String",
            SbspValue::Number(_) => "Integer",
            SbspValue::Bool(_) => "Boolean",
            SbspValue::Array(_) => "Array",
            SbspValue::Null => "Null",
        }
    }

    /// Convert to boolean (truthy)
    pub fn to_bool(&self) -> bool {
        match self {
            SbspValue::Bool(b) => *b,
            SbspValue::Number(n) => *n != 0,
            SbspValue::String(s) => !s.is_empty(),
            SbspValue::Null => false,
            SbspValue::Array(a) => !a.is_empty(),
        }
    }

    /// Convert to string representation
    pub fn to_string_repr(&self) -> heapless::String<512> {
        match self {
            SbspValue::String(s) => s.clone(),
            SbspValue::Number(n) => {
                let mut s = heapless::String::new();
                let _ = core::fmt::write(&mut s, format_args!("{}", n));
                s
            }
            SbspValue::Bool(b) => {
                if *b {
                    heapless::String::from("True")
                } else {
                    heapless::String::from("False")
                }
            }
            SbspValue::Null => heapless::String::from(""),
            SbspValue::Array(_) => heapless::String::from("[Array]"),
        }
    }

    /// Escape for HTML output (prevent XSS)
    pub fn html_escape(&self) -> heapless::String<1024> {
        let text = self.to_string_repr();
        let mut escaped = heapless::String::new();

        for ch in text.chars() {
            match ch {
                '<' => {
                    let _ = escaped.push_str("&lt;");
                }
                '>' => {
                    let _ = escaped.push_str("&gt;");
                }
                '&' => {
                    let _ = escaped.push_str("&amp;");
                }
                '"' => {
                    let _ = escaped.push_str("&quot;");
                }
                '\'' => {
                    let _ = escaped.push_str("&#39;");
                }
                _ => {
                    let _ = escaped.push(ch);
                }
            }
        }

        escaped
    }
}

/// Variable storage: HashMap-like but using heapless Vec for no_std
pub struct SymbolTable {
    vars: Vec<(heapless::String<64>, SbspValue), 256>,
}

impl SymbolTable {
    pub fn new() -> Self {
        Self {
            vars: Vec::new(),
        }
    }

    /// Define or update a variable
    pub fn set(&mut self, name: &str, value: SbspValue) -> Result<(), &'static str> {
        for (key, val) in &mut self.vars {
            if key == name {
                *val = value;
                return Ok(());
            }
        }
        // New variable
        self.vars
            .push((
                heapless::String::from(name),
                value,
            ))
            .map_err(|_| "Symbol table full")?;
        Ok(())
    }

    /// Get a variable by name
    pub fn get(&self, name: &str) -> Option<&SbspValue> {
        for (key, val) in &self.vars {
            if key == name {
                return Some(val);
            }
        }
        None
    }

    /// Clear all variables
    pub fn clear(&mut self) {
        self.vars.clear();
    }
}
