//! Attribute representation for Golden Fish DOM elements.
//!
//! Attributes are stored as owned name/value pairs. Attribute names are
//! preserved as provided by the parser (case may vary depending on input).

use alloc::string::String;

/// An HTML attribute (name, value) pair.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attribute {
    name: String,
    value: String,
}

impl Attribute {
    pub(crate) fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
        }
    }

    /// Attribute name (e.g. "id", "class", "href").
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Attribute value.
    pub fn value(&self) -> &str {
        &self.value
    }
}
