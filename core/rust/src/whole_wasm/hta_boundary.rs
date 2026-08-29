use std::collections::HashSet;

use super::{reps, NativeModule, Rep};
use crate::core::Value;
use crate::kernel::{Form, FunctionSchema, SchemaField, SchemaType};
use crate::vm::FunctionId;

const INVOCATION_ARGUMENTS_ERROR: &str =
    "hta/invocation-malformed: expected an HTA sequence of arguments";
const INVOCATION_ABI_ERROR: &str =
    "hta/invocation-abi: whole-Wasm function must declare handle-backed arguments and result";
const INVOCATION_SCHEMA_ERROR: &str = "hta/invocation-schema";

impl NativeModule {
    /// Calls a whole-Wasm function through the portable HTA0 value boundary.
    ///
    /// `request` is one HTA0-encoded Hara list, tuple, or vector containing
    /// the function arguments. The result is returned as one HTA0 frame.
    /// Internally, decoded values use the process-local scoped arena so calls
    /// between compiled Hara functions do not repeatedly encode or decode.
    ///
    /// The portable adapter accepts functions whose declared arguments and
    /// result use the dynamic handle representation, including the structured
    /// schemas retained by the compiled program. Scalar kernels retain the
    /// existing `call_i64` fast path.
    pub fn call_hta(&mut self, function: FunctionId, request: &[u8]) -> Result<Vec<u8>, String> {
        let schema = ensure_hta_value_abi(self, function)?;
        let arguments = decode_arguments(request)?;
        validate_hta_arguments(&self.artifact().program, &schema, &arguments)?;
        let result = self.call_value(function, &arguments)?;
        validate_hta_value(
            &self.artifact().program,
            &schema.output,
            &result,
            "result",
            &mut HashSet::new(),
        )?;
        crate::hta::encode(&result)
    }
}

fn decode_arguments(request: &[u8]) -> Result<Vec<Value>, String> {
    match crate::hta::decode_canonical(request)? {
        Value::List(values) => Ok(values.iter().cloned().collect()),
        Value::Tuple(values) => Ok(values.iter().cloned().collect()),
        Value::Vector(values) => Ok(values.iter().cloned().collect()),
        _ => Err(INVOCATION_ARGUMENTS_ERROR.into()),
    }
}

fn ensure_hta_value_abi(
    module: &NativeModule,
    function: FunctionId,
) -> Result<FunctionSchema, String> {
    let program = &module.artifact().program;
    let prototype = program
        .functions
        .get(usize::from(function))
        .ok_or_else(|| format!("unknown whole-Wasm function {function}"))?;

    let parameters_are_handles = (0..usize::from(prototype.arity)).all(|parameter| {
        reps::declared_parameter_rep(program, function, prototype, parameter)
            == Some(Rep::TruthyHandle)
    });
    let result_is_handle = reps::declared_result_rep(program, function) == Some(Rep::TruthyHandle);

    if parameters_are_handles && result_is_handle {
        match program.function_schema(function) {
            Some(SchemaType::Function(arities)) => arities
                .iter()
                .find(|arity| {
                    arity.fixed.len() == usize::from(prototype.arity)
                        && arity.rest.is_some() == prototype.variadic
                })
                .cloned()
                .ok_or_else(|| format!("{INVOCATION_ABI_ERROR}: {function}")),
            _ => Err(format!("{INVOCATION_ABI_ERROR}: {function}")),
        }
    } else {
        Err(format!("{INVOCATION_ABI_ERROR}: {function}"))
    }
}

fn validate_hta_arguments(
    program: &crate::vm::Program,
    schema: &FunctionSchema,
    arguments: &[Value],
) -> Result<(), String> {
    let fixed = schema.fixed.len();
    if arguments.len() < fixed || (schema.rest.is_none() && arguments.len() != fixed) {
        let expected = if schema.rest.is_some() {
            format!("at least {fixed}")
        } else {
            fixed.to_string()
        };
        return Err(format!(
            "{INVOCATION_SCHEMA_ERROR}: arguments expected {expected} values, got {}",
            arguments.len()
        ));
    }

    for (index, (schema, value)) in schema.fixed.iter().zip(arguments).enumerate() {
        validate_hta_value(
            program,
            schema,
            value,
            &format!("argument {index}"),
            &mut HashSet::new(),
        )?;
    }
    if let Some(rest) = schema.rest.as_deref() {
        for (index, value) in arguments.iter().enumerate().skip(fixed) {
            validate_hta_value(
                program,
                rest,
                value,
                &format!("argument {index}"),
                &mut HashSet::new(),
            )?;
        }
    }
    Ok(())
}

fn validate_hta_value(
    program: &crate::vm::Program,
    schema: &SchemaType,
    value: &Value,
    path: &str,
    references: &mut HashSet<String>,
) -> Result<(), String> {
    match schema {
        SchemaType::Primitive(name) => validate_primitive(name, value, path),
        SchemaType::Reference(name) => {
            if !references.insert(name.clone()) {
                return Ok(());
            }
            let result = match program.schema_types.get(name) {
                Some(schema) => validate_hta_value(program, schema, value, path, references),
                None => Err(format!(
                    "{INVOCATION_SCHEMA_ERROR}: {path} references unavailable schema {name}"
                )),
            };
            references.remove(name);
            result
        }
        SchemaType::Union(types) => {
            if types.iter().any(|schema| {
                validate_hta_value(program, schema, value, path, &mut references.clone()).is_ok()
            }) {
                Ok(())
            } else {
                Err(schema_mismatch(path, "a declared union", value))
            }
        }
        SchemaType::Vector(item) => {
            let Some(values) = sequence_values(value) else {
                return Err(schema_mismatch(path, "a vector", value));
            };
            for (index, item_value) in values.iter().enumerate() {
                validate_hta_value(
                    program,
                    item,
                    item_value,
                    &format!("{path}[{index}]"),
                    references,
                )?;
            }
            Ok(())
        }
        SchemaType::Set(item) => {
            let values = match value {
                Value::Set(values) => values.iter().collect::<Vec<_>>(),
                Value::OrderedSet(values) => values.iter().collect::<Vec<_>>(),
                Value::SortedSet(values) => values.iter().collect::<Vec<_>>(),
                _ => return Err(schema_mismatch(path, "a set", value)),
            };
            for (index, item_value) in values.iter().enumerate() {
                validate_hta_value(
                    program,
                    item,
                    item_value,
                    &format!("{path}[{index}]"),
                    references,
                )?;
            }
            Ok(())
        }
        SchemaType::Tuple(types) => {
            let Value::Tuple(values) = value else {
                return Err(schema_mismatch(path, "a tuple", value));
            };
            if values.len() != types.len() {
                return Err(format!(
                    "{INVOCATION_SCHEMA_ERROR}: {path} expected tuple arity {}, got {}",
                    types.len(),
                    values.len()
                ));
            }
            for (index, (schema, item_value)) in types.iter().zip(values.iter()).enumerate() {
                validate_hta_value(
                    program,
                    schema,
                    item_value,
                    &format!("{path}[{index}]"),
                    references,
                )?;
            }
            Ok(())
        }
        SchemaType::Map(fields) => validate_map(program, fields, value, path, references),
        SchemaType::Struct {
            name,
            mutable: true,
            ..
        } => Err(format!(
            "{INVOCATION_SCHEMA_ERROR}: {path} mutable struct {name} is not transportable over HTA0"
        )),
        SchemaType::Struct {
            name,
            fields,
            mutable: false,
        } => validate_struct(program, name, fields, value, path, references),
        SchemaType::WithProperties { schema, .. } => {
            validate_hta_value(program, schema, value, path, references)
        }
        // These forms do not yet carry an executable HTA value predicate. They
        // remain dynamic so older declarations do not acquire a false negative
        // merely because they use an extension or retained source form.
        SchemaType::Function(_)
        | SchemaType::Enum(_)
        | SchemaType::Extension { .. }
        | SchemaType::Unknown(_) => Ok(()),
    }
}

fn validate_primitive(name: &str, value: &Value, path: &str) -> Result<(), String> {
    let valid = match name {
        "any" | "value" => true,
        "nil" => matches!(value, Value::Nil),
        "bool" | "boolean" => matches!(value, Value::Bool(_)),
        "int" | "long" | "i64" => crate::numeric::is_long_value(value),
        "bigint" => crate::numeric::is_big_integer_value(value),
        "integer" => crate::numeric::integer_kind(value).is_some(),
        "float" | "f64" => matches!(value, Value::Float(_)),
        "number" => matches!(
            value,
            Value::Number(_) | Value::BigInteger(_) | Value::Float(_)
        ),
        "str" | "string" => matches!(value, Value::String(_)),
        "keyword" => matches!(value, Value::Keyword(_)),
        "symbol" => matches!(value, Value::Symbol(_)),
        "bytes" => matches!(value, Value::Bytes(_) | Value::ByteBuffer(_)),
        "char" | "character" => matches!(value, Value::Character(_)),
        _ => true,
    };
    if valid {
        Ok(())
    } else {
        Err(schema_mismatch(path, &format!(":{name}"), value))
    }
}

fn validate_map(
    program: &crate::vm::Program,
    fields: &[SchemaField],
    value: &Value,
    path: &str,
    references: &mut HashSet<String>,
) -> Result<(), String> {
    let Some(entries) = crate::core::map_entries(value) else {
        return Err(schema_mismatch(path, "a map", value));
    };
    for field in fields {
        let Some(name) = form_field_name(&field.name) else {
            return Err(format!(
                "{INVOCATION_SCHEMA_ERROR}: {path} contains an invalid field name {}",
                field.name
            ));
        };
        let found = entries.iter().find_map(|(key, value)| {
            value_field_name(key)
                .filter(|key_name| *key_name == name)
                .map(|_| value)
        });
        match found {
            Some(field_value) => {
                if is_optional(field.properties.as_ref()) && matches!(field_value, Value::Nil) {
                    continue;
                }
                validate_hta_value(
                    program,
                    &field.value_type,
                    field_value,
                    &format!("{path}.{}", name),
                    references,
                )?;
            }
            None if is_optional(field.properties.as_ref()) => {}
            None => {
                return Err(format!(
                    "{INVOCATION_SCHEMA_ERROR}: {path} is missing required field {name}"
                ))
            }
        }
    }
    Ok(())
}

fn validate_struct(
    program: &crate::vm::Program,
    name: &str,
    fields: &[SchemaField],
    value: &Value,
    path: &str,
    references: &mut HashSet<String>,
) -> Result<(), String> {
    let Value::Struct(value) = value else {
        return Err(schema_mismatch(path, &format!("struct {name}"), value));
    };
    if value.ty.name != name {
        return Err(format!(
            "{INVOCATION_SCHEMA_ERROR}: {path} expected struct {name}, got struct {}",
            value.ty.name
        ));
    }
    let expected_fields = fields
        .iter()
        .filter_map(|field| form_field_name(&field.name))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if expected_fields.len() != fields.len() || value.ty.fields != expected_fields {
        return Err(format!(
            "{INVOCATION_SCHEMA_ERROR}: {path} has a field layout different from struct {name}"
        ));
    }
    for field in fields {
        let field_name = form_field_name(&field.name).expect("validated struct field name");
        let Some(field_value) = value.get(field_name) else {
            if is_optional(field.properties.as_ref()) {
                continue;
            }
            return Err(format!(
                "{INVOCATION_SCHEMA_ERROR}: {path} is missing required field {field_name}"
            ));
        };
        if is_optional(field.properties.as_ref()) && matches!(field_value, Value::Nil) {
            continue;
        }
        validate_hta_value(
            program,
            &field.value_type,
            field_value,
            &format!("{path}.{}", field_name),
            references,
        )?;
    }
    Ok(())
}

fn sequence_values(value: &Value) -> Option<Vec<&Value>> {
    match value {
        Value::Tuple(values) => Some(values.iter().collect()),
        Value::Vector(values) => Some(values.iter().collect()),
        _ => None,
    }
}

fn form_field_name(form: &Form) -> Option<&str> {
    match form {
        Form::Symbol(name) | Form::Keyword(name) | Form::String(name) => Some(name.as_str()),
        _ => None,
    }
}

fn value_field_name(value: &Value) -> Option<&str> {
    match value {
        Value::String(name) => Some(name.as_str()),
        Value::Keyword(name) => Some(name.as_str()),
        Value::Symbol(name) => Some(name.as_str()),
        _ => None,
    }
}

fn is_optional(properties: Option<&Form>) -> bool {
    let Some(Form::Map(entries)) = properties else {
        return false;
    };
    entries.iter().any(|(key, value)| {
        matches!(key, Form::Keyword(name) if name == "optional")
            && matches!(value, Form::Bool(true))
    })
}

fn schema_mismatch(path: &str, expected: &str, value: &Value) -> String {
    format!(
        "{INVOCATION_SCHEMA_ERROR}: {path} expected {expected}, got {}",
        value_kind(value)
    )
}

fn value_kind(value: &Value) -> &'static str {
    match value {
        Value::Nil => "nil",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "long",
        Value::BigInteger(_) if crate::numeric::is_long_value(value) => "long",
        Value::BigInteger(_) => "bigint",
        Value::Float(_) => "float",
        Value::String(_) => "string",
        Value::Keyword(_) => "keyword",
        Value::Symbol(_) => "symbol",
        Value::Bytes(_) | Value::ByteBuffer(_) => "bytes",
        Value::Vector(_) => "vector",
        Value::List(_) => "list",
        Value::Tuple(_) => "tuple",
        Value::MapEntry(_) => "map-entry",
        Value::Set(_) | Value::OrderedSet(_) | Value::SortedSet(_) => "set",
        Value::Map(_)
        | Value::OrderedMap(_)
        | Value::SortedMap(_)
        | Value::Trie(_)
        | Value::PriorityMap(_) => "map",
        Value::Struct(_) => "struct",
        Value::Mutable(_) => "mutable",
        _ => "value",
    }
}
