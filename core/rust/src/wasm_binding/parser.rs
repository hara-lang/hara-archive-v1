use std::collections::{BTreeMap, BTreeSet, HashSet};

use crate::kernel::{parse, Form};

use super::syntax::*;
use super::{
    BindingFunction, BindingParameter, BindingResult, ErrorContract, HaraValueType, Lifting,
    Lowering, MemoryContract, Ownership, WasmInterface, WasmValueType, WASM_INTERFACE_SCHEMA,
};

const INTERFACE_FIELDS: &[&str] = &[
    "schema",
    "namespace",
    "module",
    "memory",
    "exports",
    "imports",
    "capabilities",
    "handles",
];
const MEMORY_FIELDS: &[&str] = &["export", "allocate", "reallocate", "release"];
const EXPORT_FIELDS: &[&str] = &[
    "wasm/export",
    "arguments",
    "returns",
    "async",
    "errors",
    "capabilities",
];
const PARAMETER_FIELDS: &[&str] = &["name", "hara/type", "wasm/type", "lower", "ownership"];
const RESULT_FIELDS: &[&str] = &["hara/type", "wasm/type", "lift", "ownership"];
const ERROR_FIELDS: &[&str] = &["convention", "codes"];

impl WasmValueType {
    fn parse(form: &Form, origin: &str, field: &str) -> Result<Self, String> {
        match keyword(form, origin, field)? {
            "i32" => Ok(Self::I32),
            "i64" => Ok(Self::I64),
            "f32" => Ok(Self::F32),
            "f64" => Ok(Self::F64),
            "void" => Ok(Self::Void),
            value => Err(unsupported(
                origin,
                format!("{field} uses unsupported Wasm type :{value}"),
            )),
        }
    }
}

impl HaraValueType {
    fn parse(form: &Form, origin: &str, field: &str) -> Result<Self, String> {
        match form {
            Form::Keyword(value) => match value.as_str() {
                "i32" => Ok(Self::I32),
                "i64" => Ok(Self::I64),
                "f32" => Ok(Self::F32),
                "f64" => Ok(Self::F64),
                "boolean" => Ok(Self::Boolean),
                "string" => Ok(Self::String),
                "bytes" => Ok(Self::Bytes),
                "void" => Ok(Self::Void),
                value => Err(unsupported(
                    origin,
                    format!("{field} uses unsupported Hara type :{value}"),
                )),
            },
            Form::Vector(values) if values.len() == 2 => {
                let kind = keyword(&values[0], origin, field)?;
                let name = named(&values[1], origin, field)?.to_owned();
                if !valid_tag(&name) {
                    return Err(malformed(
                        origin,
                        format!("{field} type name must be lower-case"),
                    ));
                }
                match kind {
                    "record" => Ok(Self::Record(name)),
                    "variant" => Ok(Self::Variant(name)),
                    "handle" => Ok(Self::Handle(name)),
                    "callback" => Ok(Self::Callback(name)),
                    value => Err(unsupported(
                        origin,
                        format!("{field} uses unsupported type constructor :{value}"),
                    )),
                }
            }
            _ => Err(malformed(
                origin,
                format!("{field} must be a type keyword or [kind name] vector"),
            )),
        }
    }
}

impl Ownership {
    fn parse(form: &Form, origin: &str, field: &str) -> Result<Self, String> {
        match keyword(form, origin, field)? {
            "borrowed" => Ok(Self::Borrowed),
            "caller" => Ok(Self::Caller),
            "callee" => Ok(Self::Callee),
            "transferred" => Ok(Self::Transferred),
            value => Err(unsupported(
                origin,
                format!("{field} uses unsupported ownership :{value}"),
            )),
        }
    }
}

impl Lowering {
    fn parse(form: &Form, origin: &str, field: &str) -> Result<Self, String> {
        match form {
            Form::Keyword(value) if value == "direct" => Ok(Self::Direct),
            Form::Vector(values) => match values.as_slice() {
                [Form::Keyword(pointer), Form::Keyword(length)]
                    if pointer == "pointer" && length == "length" =>
                {
                    Ok(Self::PointerLength)
                }
                _ => Err(unsupported(
                    origin,
                    format!("{field} uses unsupported lowering"),
                )),
            },
            _ => Err(unsupported(
                origin,
                format!("{field} uses unsupported lowering"),
            )),
        }
    }
}

impl Lifting {
    fn parse(form: &Form, origin: &str, field: &str) -> Result<Self, String> {
        match form {
            Form::Keyword(value) if value == "direct" => Ok(Self::Direct),
            Form::Keyword(value) if value == "packed-i64" => Ok(Self::PackedI64),
            Form::Vector(values) => match values.as_slice() {
                [Form::Keyword(pointer), Form::Keyword(length)]
                    if pointer == "pointer" && length == "length" =>
                {
                    Ok(Self::PointerLength)
                }
                _ => Err(unsupported(
                    origin,
                    format!("{field} uses unsupported lifting"),
                )),
            },
            _ => Err(unsupported(
                origin,
                format!("{field} uses unsupported lifting"),
            )),
        }
    }
}

pub(super) fn parse_interface(source: &str, origin: &str) -> Result<WasmInterface, String> {
    let form = parse(source)
        .map_err(|error| malformed(origin, format!("cannot parse interface: {error}")))?;
    let payload = interface_payload(&form, origin)?;
    let entries = map(payload, origin, "interface")?;
    reject_unknown(entries, INTERFACE_FIELDS, origin, "interface")?;
    reject_reserved_collection(entries, "imports", origin)?;
    reject_reserved_collection(entries, "handles", origin)?;

    let schema = non_empty_string(
        required(entries, "schema", origin)?,
        origin,
        "interface schema",
    )?
    .to_owned();
    if schema != WASM_INTERFACE_SCHEMA {
        return Err(unsupported(
            origin,
            format!("unsupported interface schema {schema}"),
        ));
    }

    let namespace = named(
        required(entries, "namespace", origin)?,
        origin,
        "interface namespace",
    )?
    .to_owned();
    if !valid_namespace(&namespace) {
        return Err(malformed(
            origin,
            "namespace must be a qualified lower-case name",
        ));
    }

    let module = non_empty_string(
        required(entries, "module", origin)?,
        origin,
        "interface module",
    )?
    .to_owned();
    validate_module_path(&module, origin)?;

    let memory = optional(entries, "memory")
        .map(|form| parse_memory(form, origin))
        .transpose()?;
    let exports = parse_exports(required(entries, "exports", origin)?, origin)?;
    let capabilities = optional(entries, "capabilities").map_or_else(
        || Ok(BTreeSet::new()),
        |form| keyword_set(form, origin, "interface capabilities"),
    )?;

    let interface = WasmInterface {
        schema,
        namespace,
        module,
        memory,
        exports,
        capabilities,
    };
    validate_alpha(&interface, origin)?;
    Ok(interface)
}

fn interface_payload<'a>(form: &'a Form, origin: &str) -> Result<&'a Form, String> {
    match form {
        Form::Map(_) => Ok(form),
        Form::List(values)
            if values.len() == 2
                && matches!(&values[0], Form::Symbol(name) if name == "wasm/interface") =>
        {
            Ok(&values[1])
        }
        Form::List(_) => Err(malformed(
            origin,
            "interface must use exactly (wasm/interface {...})",
        )),
        _ => Err(malformed(
            origin,
            "interface must be a map or (wasm/interface {...}) data form",
        )),
    }
}

fn parse_memory(form: &Form, origin: &str) -> Result<MemoryContract, String> {
    let entries = map(form, origin, "memory")?;
    reject_unknown(entries, MEMORY_FIELDS, origin, "memory")?;
    Ok(MemoryContract {
        export: non_empty_string(
            required(entries, "export", origin)?,
            origin,
            "memory export",
        )?
        .to_owned(),
        allocate: optional_string(entries, "allocate", origin)?,
        reallocate: optional_string(entries, "reallocate", origin)?,
        release: optional_string(entries, "release", origin)?,
    })
}

fn parse_exports(form: &Form, origin: &str) -> Result<Vec<BindingFunction>, String> {
    let entries = map(form, origin, "exports")?;
    if entries.is_empty() {
        return Err(malformed(origin, "exports cannot be empty"));
    }

    let mut names = HashSet::new();
    let mut exports = entries
        .iter()
        .map(|(name, specification)| {
            let name = named(name, origin, "export name")?.to_owned();
            if !valid_binding_name(&name) {
                return Err(malformed(
                    origin,
                    format!("invalid Hara export name {name}"),
                ));
            }
            if !names.insert(name.clone()) {
                return Err(malformed(origin, format!("duplicate export {name}")));
            }
            parse_export(&name, specification, origin)
        })
        .collect::<Result<Vec<_>, _>>()?;
    exports.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(exports)
}

fn parse_export(name: &str, form: &Form, origin: &str) -> Result<BindingFunction, String> {
    let entries = map(form, origin, &format!("export {name}"))?;
    reject_unknown(entries, EXPORT_FIELDS, origin, &format!("export {name}"))?;

    let wasm_export = non_empty_string(
        required(entries, "wasm/export", origin)?,
        origin,
        &format!("export {name} wasm/export"),
    )?
    .to_owned();
    let arguments = parse_parameters(required(entries, "arguments", origin)?, origin, name)?;
    let returns = parse_result(required(entries, "returns", origin)?, origin, name)?;
    let asynchronous = optional_bool(entries, "async", origin)?.unwrap_or(false);
    let errors = optional(entries, "errors")
        .map(|form| parse_errors(form, origin, name))
        .transpose()?;
    let capabilities = optional(entries, "capabilities").map_or_else(
        || Ok(BTreeSet::new()),
        |form| keyword_set(form, origin, &format!("export {name} capabilities")),
    )?;

    Ok(BindingFunction {
        name: name.to_owned(),
        wasm_export,
        arguments,
        returns,
        asynchronous,
        errors,
        capabilities,
    })
}

fn parse_parameters(
    form: &Form,
    origin: &str,
    export: &str,
) -> Result<Vec<BindingParameter>, String> {
    let values = vector(form, origin, &format!("export {export} arguments"))?;
    let mut names = HashSet::new();
    values
        .iter()
        .map(|form| {
            let entries = map(form, origin, &format!("export {export} argument"))?;
            reject_unknown(
                entries,
                PARAMETER_FIELDS,
                origin,
                &format!("export {export} argument"),
            )?;
            let name = named(
                required(entries, "name", origin)?,
                origin,
                &format!("export {export} argument name"),
            )?
            .to_owned();
            if !valid_binding_name(&name) {
                return Err(malformed(
                    origin,
                    format!("invalid argument name {name} in export {export}"),
                ));
            }
            if !names.insert(name.clone()) {
                return Err(malformed(
                    origin,
                    format!("duplicate argument {name} in export {export}"),
                ));
            }

            Ok(BindingParameter {
                name,
                hara_type: HaraValueType::parse(
                    required(entries, "hara/type", origin)?,
                    origin,
                    &format!("export {export} argument hara/type"),
                )?,
                wasm_type: WasmValueType::parse(
                    required(entries, "wasm/type", origin)?,
                    origin,
                    &format!("export {export} argument wasm/type"),
                )?,
                lowering: optional(entries, "lower")
                    .map(|form| {
                        Lowering::parse(form, origin, &format!("export {export} argument lower"))
                    })
                    .transpose()?,
                ownership: optional(entries, "ownership")
                    .map(|form| {
                        Ownership::parse(
                            form,
                            origin,
                            &format!("export {export} argument ownership"),
                        )
                    })
                    .transpose()?,
            })
        })
        .collect()
}

fn parse_result(form: &Form, origin: &str, export: &str) -> Result<BindingResult, String> {
    let entries = map(form, origin, &format!("export {export} result"))?;
    reject_unknown(
        entries,
        RESULT_FIELDS,
        origin,
        &format!("export {export} result"),
    )?;

    Ok(BindingResult {
        hara_type: HaraValueType::parse(
            required(entries, "hara/type", origin)?,
            origin,
            &format!("export {export} result hara/type"),
        )?,
        wasm_type: WasmValueType::parse(
            required(entries, "wasm/type", origin)?,
            origin,
            &format!("export {export} result wasm/type"),
        )?,
        lifting: optional(entries, "lift")
            .map(|form| Lifting::parse(form, origin, &format!("export {export} result lift")))
            .transpose()?,
        ownership: optional(entries, "ownership")
            .map(|form| {
                Ownership::parse(form, origin, &format!("export {export} result ownership"))
            })
            .transpose()?,
    })
}

fn parse_errors(form: &Form, origin: &str, export: &str) -> Result<ErrorContract, String> {
    let entries = map(form, origin, &format!("export {export} errors"))?;
    reject_unknown(
        entries,
        ERROR_FIELDS,
        origin,
        &format!("export {export} errors"),
    )?;
    let convention = keyword(
        required(entries, "convention", origin)?,
        origin,
        &format!("export {export} error convention"),
    )?
    .to_owned();
    let code_entries = map(
        required(entries, "codes", origin)?,
        origin,
        &format!("export {export} error codes"),
    )?;
    let mut codes = BTreeMap::new();
    for (code, value) in code_entries {
        let Form::Number(code) = code else {
            return Err(malformed(
                origin,
                format!("export {export} error codes require integer keys"),
            ));
        };
        let value = named(value, origin, &format!("export {export} error code"))?.to_owned();
        if codes.insert(*code, value).is_some() {
            return Err(malformed(
                origin,
                format!("duplicate error code {code} in export {export}"),
            ));
        }
    }
    Ok(ErrorContract { convention, codes })
}

fn validate_alpha(interface: &WasmInterface, origin: &str) -> Result<(), String> {
    let mut uses_memory = false;
    for export in &interface.exports {
        for argument in &export.arguments {
            uses_memory |= validate_parameter(argument, origin, &export.name)?;
        }
        uses_memory |= validate_result(&export.returns, origin, &export.name)?;
    }
    match (uses_memory, interface.memory.is_some()) {
        (true, false) => Err(malformed(
            origin,
            "lowered or lifted values require an explicit :memory contract",
        )),
        (false, true) => Err(malformed(
            origin,
            ":memory is declared but no argument or result uses it",
        )),
        _ => Ok(()),
    }
}

fn validate_parameter(
    parameter: &BindingParameter,
    origin: &str,
    export: &str,
) -> Result<bool, String> {
    if parameter.wasm_type == WasmValueType::Void {
        return Err(malformed(
            origin,
            format!(
                "export {export} argument {} cannot be :void",
                parameter.name
            ),
        ));
    }

    match parameter.hara_type.direct_wasm_type() {
        Some(expected) if expected == parameter.wasm_type => {
            if parameter.lowering.is_some() || parameter.ownership.is_some() {
                return Err(malformed(
                    origin,
                    format!(
                        "scalar argument {} in export {export} cannot declare lowering or ownership",
                        parameter.name
                    ),
                ));
            }
            Ok(false)
        }
        Some(expected) => Err(malformed(
            origin,
            format!(
                "export {export} argument {} maps :{} to :{}",
                parameter.name,
                expected.as_keyword(),
                parameter.wasm_type.as_keyword()
            ),
        )),
        None => {
            if parameter.lowering.is_none() {
                return Err(malformed(
                    origin,
                    format!(
                        "non-scalar argument {} in export {export} requires :lower",
                        parameter.name
                    ),
                ));
            }
            if parameter.ownership.is_none() {
                return Err(malformed(
                    origin,
                    format!(
                        "non-scalar argument {} in export {export} requires :ownership",
                        parameter.name
                    ),
                ));
            }
            Ok(true)
        }
    }
}

fn validate_result(result: &BindingResult, origin: &str, export: &str) -> Result<bool, String> {
    match result.hara_type.direct_wasm_type() {
        Some(expected) if expected == result.wasm_type => {
            if result.lifting.is_some() || result.ownership.is_some() {
                return Err(malformed(
                    origin,
                    format!("scalar result in export {export} cannot declare lifting or ownership"),
                ));
            }
            Ok(false)
        }
        Some(expected) => Err(malformed(
            origin,
            format!(
                "export {export} result maps :{} to :{}",
                expected.as_keyword(),
                result.wasm_type.as_keyword()
            ),
        )),
        None => {
            if result.lifting.is_none() {
                return Err(malformed(
                    origin,
                    format!("non-scalar result in export {export} requires :lift"),
                ));
            }
            if result.ownership.is_none() {
                return Err(malformed(
                    origin,
                    format!("non-scalar result in export {export} requires :ownership"),
                ));
            }
            Ok(true)
        }
    }
}

fn reject_reserved_collection(
    entries: &[(Form, Form)],
    field: &str,
    origin: &str,
) -> Result<(), String> {
    let Some(form) = optional(entries, field) else {
        return Ok(());
    };
    let empty = matches!(form, Form::Vector(values) if values.is_empty())
        || matches!(form, Form::Map(values) if values.is_empty());
    if empty {
        Ok(())
    } else {
        Err(unsupported(
            origin,
            format!("{field} are reserved for the HTA binding tranche"),
        ))
    }
}
