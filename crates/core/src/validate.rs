use std::fmt;

use serde::Serialize;

/// Max length for MCP-safe names (tool names, parameter names).
pub const MAX_NAME_LEN: usize = 64;

/// `[a-zA-Z0-9_-]{1,64}`: the tool-name rule shared by MCP clients.
pub fn is_valid_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= MAX_NAME_LEN
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// One problem with one input field. `field` is a dotted path, e.g. `kind.params[2].name`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FieldError {
    pub field: String,
    pub message: String,
}

/// All validation problems found in an input, reported together.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ValidationErrors {
    pub errors: Vec<FieldError>,
}

impl ValidationErrors {
    pub fn single(field: impl Into<String>, message: impl Into<String>) -> Self {
        let mut e = Self::default();
        e.push(field, message);
        e
    }

    pub fn push(&mut self, field: impl Into<String>, message: impl Into<String>) {
        self.errors.push(FieldError {
            field: field.into(),
            message: message.into(),
        });
    }

    pub fn is_empty(&self) -> bool {
        self.errors.is_empty()
    }

    pub fn into_result(self) -> Result<(), Self> {
        if self.is_empty() {
            Ok(())
        } else {
            Err(self)
        }
    }

    /// True if any error is reported for exactly `field`.
    pub fn has(&self, field: &str) -> bool {
        self.errors.iter().any(|e| e.field == field)
    }
}

impl fmt::Display for ValidationErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let parts: Vec<String> = self
            .errors
            .iter()
            .map(|e| format!("{}: {}", e.field, e.message))
            .collect();
        write!(f, "{}", parts.join("; "))
    }
}

impl std::error::Error for ValidationErrors {}

#[cfg(test)]
#[path = "../tests/unit/validate.rs"]
mod tests;
