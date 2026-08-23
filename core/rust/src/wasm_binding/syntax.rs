use std::collections::{BTreeSet, HashSet};

use crate::kernel::Form;

pub(super) fn map<'a>(
    form: &'a Form,
    origin: &str,
    field: &str,
) -> Result<&'a [(Form, Form)], String> {
    match form {
        Form::Map(entries) => Ok(entries),
        _ => Err(malformed(origin, format!("{field} must be a map"))),
    }
}

pub(super) fn vector<'a>(form: &'a Form, origin: &str, field: &str) -> Result<&'a [Form], String> {
    match form {
        Form::Vector(values) => Ok(values),
        _ => Err(malformed(origin, format!("{field} must be a vector"))),
    }
}

pub(super) fn non_empty_string<'a>(
    form: &'a Form,
    origin: &str,
    field: &str,
) -> Result<&'a str, String> {
    match form {
        Form::String(value) if !value.is_empty() => Ok(value),
        _ => Err(malformed(
            origin,
            format!("{field} must be a non-empty string"),
        )),
    }
}

pub(super) fn named<'a>(form: &'a Form, origin: &str, field: &str) -> Result<&'a str, String> {
    match form {
        Form::Symbol(value) | Form::Keyword(value) | Form::String(value) if !value.is_empty() => {
            Ok(value)
        }
        _ => Err(malformed(origin, format!("{field} must be a named value"))),
    }
}

pub(super) fn keyword<'a>(form: &'a Form, origin: &str, field: &str) -> Result<&'a str, String> {
    match form {
        Form::Keyword(value) => Ok(value),
        _ => Err(malformed(origin, format!("{field} must be a keyword"))),
    }
}

pub(super) fn optional_string(
    entries: &[(Form, Form)],
    name: &str,
    origin: &str,
) -> Result<Option<String>, String> {
    optional(entries, name)
        .map(|form| non_empty_string(form, origin, name).map(str::to_owned))
        .transpose()
}

pub(super) fn optional_bool(
    entries: &[(Form, Form)],
    name: &str,
    origin: &str,
) -> Result<Option<bool>, String> {
    optional(entries, name)
        .map(|form| match form {
            Form::Bool(value) => Ok(*value),
            _ => Err(malformed(origin, format!("{name} must be boolean"))),
        })
        .transpose()
}

pub(super) fn keyword_set(
    form: &Form,
    origin: &str,
    field: &str,
) -> Result<BTreeSet<String>, String> {
    vector(form, origin, field)?
        .iter()
        .map(|form| keyword(form, origin, field).map(str::to_owned))
        .collect()
}

pub(super) fn named_set(
    form: &Form,
    origin: &str,
    field: &str,
) -> Result<BTreeSet<String>, String> {
    vector(form, origin, field)?
        .iter()
        .map(|form| named(form, origin, field).map(str::to_owned))
        .collect()
}

pub(super) fn key(form: &Form) -> Option<&str> {
    match form {
        Form::Keyword(value) | Form::Symbol(value) | Form::String(value) => Some(value),
        _ => None,
    }
}

pub(super) fn required<'a>(
    entries: &'a [(Form, Form)],
    name: &str,
    origin: &str,
) -> Result<&'a Form, String> {
    optional(entries, name)
        .ok_or_else(|| malformed(origin, format!("missing required field {name}")))
}

pub(super) fn optional<'a>(entries: &'a [(Form, Form)], name: &str) -> Option<&'a Form> {
    entries
        .iter()
        .find(|(candidate, _)| key(candidate) == Some(name))
        .map(|(_, value)| value)
}

pub(super) fn reject_unknown(
    entries: &[(Form, Form)],
    allowed: &[&str],
    origin: &str,
    scope: &str,
) -> Result<(), String> {
    let mut seen = HashSet::new();
    for (candidate, _) in entries {
        let Some(name) = key(candidate) else {
            return Err(malformed(origin, format!("{scope} keys must be named")));
        };
        if !allowed.contains(&name) {
            return Err(malformed(origin, format!("unknown {scope} field: {name}")));
        }
        if !seen.insert(name) {
            return Err(malformed(
                origin,
                format!("duplicate {scope} field: {name}"),
            ));
        }
    }
    Ok(())
}

pub(super) fn validate_module_path(value: &str, origin: &str) -> Result<(), String> {
    let unsafe_path = !value.ends_with(".wasm")
        || value.starts_with('/')
        || value.contains('\\')
        || value.contains(':')
        || value.bytes().any(|byte| byte == 0)
        || value
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..");
    if unsafe_path {
        return Err(malformed(
            origin,
            "module must be a safe relative .wasm package path",
        ));
    }
    Ok(())
}

pub(super) fn valid_namespace(value: &str) -> bool {
    value.contains('.') && value.split('.').all(valid_component)
}

pub(super) fn valid_tag(value: &str) -> bool {
    value.split('.').all(valid_component)
}

pub(super) fn valid_binding_name(value: &str) -> bool {
    !value.is_empty()
        && value.chars().all(|ch| {
            ch.is_ascii_lowercase()
                || ch.is_ascii_digit()
                || matches!(ch, '-' | '?' | '!' | '*' | '+' | '<' | '>' | '=')
        })
}

fn valid_component(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
}

pub(super) fn malformed(origin: &str, message: impl AsRef<str>) -> String {
    format!("wasm-interface/malformed {origin}: {}", message.as_ref())
}

pub(super) fn unsupported(origin: &str, message: impl AsRef<str>) -> String {
    format!(
        "wasm-interface/feature-unsupported {origin}: {}",
        message.as_ref()
    )
}
