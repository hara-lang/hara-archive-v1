//! Decoding of typed VM constant-pool operands.

use super::Program;
use crate::core::{NamedField, Value};

/// Reads a string constant (the global-name operands).
pub(super) fn constant_string(program: &Program, index: u32) -> Option<&str> {
    match program.constants.get(index as usize) {
        Some(Value::String(value)) => Some(value.as_str()),
        _ => None,
    }
}

/// Reads the field-spec vector emitted for a named value declaration. Plain
/// strings remain accepted as the legacy `:any` field form.
pub(super) fn constant_named_fields(
    program: &Program,
    index: u32,
    kind: &str,
) -> Result<Option<Vec<NamedField>>, String> {
    match program.constants.get(index as usize) {
        Some(Value::Vector(fields)) => fields
            .iter()
            .map(|field| NamedField::from_value(field, kind))
            .collect::<Result<Vec<_>, _>>()
            .map(Some),
        _ => Ok(None),
    }
}
