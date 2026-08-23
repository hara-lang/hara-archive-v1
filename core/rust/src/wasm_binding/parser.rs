use std::collections::{BTreeMap, BTreeSet, HashSet};

use crate::kernel::{parse, Form};

use super::syntax::*;
use super::{
    BindingFunction, BindingParameter, BindingResult, CallbackParameter, ErrorContract,
    HaraValueType, Lifting, Lowering, MemoryContract, Ownership, WasmInterface, WasmValueType,
    WASM_INTERFACE_SCHEMA,
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
    "resources",
    "host-calls",
    "callbacks",
];
const MEMORY_FIELDS: &[&str] = &["export", "allocate", "reallocate", "release"];
const EXPORT_FIELDS: &[&str] = &[
    "wasm/export",
    "arguments",
    "returns",
    "async",
    "operation",
    "hta/operation",
    "request",
    "hta/request",
    "cancellation",
    "hta/cancellation",
    "errors",
    "capabilities",
];
const REQUEST_FIELDS: &[&str] = &["timeout-ms", "max-in-flight"];
const HANDLE_FIELDS: &[&str] = &["tag", "release"];
const HOST_CALL_FIELDS: &[&str] = &["methods", "capabilities"];
const CALLBACK_FIELDS: &[&str] = &["arguments", "returns", "reentrant"];
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
    let host_calls = optional(entries, "host-calls")
        .map(|form| parse_host_calls(form, origin))
        .transpose()?
        .unwrap_or_default();
    let callbacks = optional(entries, "callbacks")
        .map(|form| parse_callbacks(form, origin))
        .transpose()?
        .unwrap_or_default();
    let handles = optional(entries, "handles")
        .map(|form| parse_handles(form, origin, "handles"))
        .transpose()?
        .unwrap_or_default();
    let resources = optional(entries, "resources")
        .map(|form| parse_handles(form, origin, "resources"))
        .transpose()?
        .unwrap_or_default();
    if handles.keys().any(|name| resources.contains_key(name)) {
        return Err(malformed(
            origin,
            "handle and resource declarations cannot use the same name",
        ));
    }

    let interface = WasmInterface {
        schema,
        namespace,
        module,
        memory,
        exports,
        capabilities,
        host_calls,
        callbacks,
        handles,
        resources,
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
    let (asynchronous, async_policy) = parse_async(entries, name, origin)?;
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
        operation: async_policy.as_ref().map(|policy| policy.operation.clone()),
        request: async_policy.as_ref().map(|policy| policy.request.clone()),
        cancellation: async_policy.map(|policy| policy.cancellation),
        errors,
        capabilities,
    })
}

fn parse_async(
    entries: &[(Form, Form)],
    export: &str,
    origin: &str,
) -> Result<(bool, Option<super::AsyncPolicy>), String> {
    reject_aliases(
        entries,
        &["operation", "hta/operation"],
        origin,
        "async operation",
    )?;
    reject_aliases(
        entries,
        &["request", "hta/request"],
        origin,
        "async request",
    )?;
    reject_aliases(
        entries,
        &["cancellation", "hta/cancellation"],
        origin,
        "async cancellation",
    )?;
    let Some(form) = optional(entries, "async") else {
        let operation = optional_string_alias(entries, &["operation", "hta/operation"], origin)?;
        let request = optional_alias(entries, &["request", "hta/request"])
            .map(|form| parse_request(form, origin, export))
            .transpose()?;
        let cancellation = optional_alias(entries, &["cancellation", "hta/cancellation"])
            .map(|form| parse_cancellation(form, origin, export))
            .transpose()?;
        if operation.is_some() || request.is_some() || cancellation.is_some() {
            return Ok((
                true,
                Some(super::AsyncPolicy {
                    operation: operation.unwrap_or_else(|| export.to_owned()),
                    request: request.unwrap_or_default(),
                    cancellation: cancellation.unwrap_or(super::CancellationPolicy::Cooperative),
                }),
            ));
        }
        return Ok((false, None));
    };
    match form {
        Form::Bool(false) => {
            if optional_alias(entries, &["operation", "hta/operation"]).is_some()
                || optional_alias(entries, &["request", "hta/request"]).is_some()
                || optional_alias(entries, &["cancellation", "hta/cancellation"]).is_some()
            {
                return Err(malformed(
                    origin,
                    format!("synchronous export {export} cannot declare HTA policy"),
                ));
            }
            Ok((false, None))
        }
        Form::Bool(true) => {
            let operation =
                optional_string_alias(entries, &["operation", "hta/operation"], origin)?;
            let request = optional_alias(entries, &["request", "hta/request"])
                .map(|form| parse_request(form, origin, export))
                .transpose()?;
            let cancellation = optional_alias(entries, &["cancellation", "hta/cancellation"])
                .map(|form| parse_cancellation(form, origin, export))
                .transpose()?;
            Ok((
                true,
                Some(super::AsyncPolicy {
                    operation: operation.unwrap_or_else(|| export.to_owned()),
                    request: request.unwrap_or_default(),
                    cancellation: cancellation.unwrap_or(super::CancellationPolicy::Cooperative),
                }),
            ))
        }
        Form::Map(_) => {
            let async_entries = map(form, origin, &format!("export {export} async"))?;
            reject_unknown(
                async_entries,
                &[
                    "operation",
                    "hta/operation",
                    "request",
                    "hta/request",
                    "cancellation",
                    "hta/cancellation",
                ],
                origin,
                &format!("export {export} async"),
            )?;
            reject_aliases(
                async_entries,
                &["operation", "hta/operation"],
                origin,
                "async operation",
            )?;
            reject_aliases(
                async_entries,
                &["request", "hta/request"],
                origin,
                "async request",
            )?;
            reject_aliases(
                async_entries,
                &["cancellation", "hta/cancellation"],
                origin,
                "async cancellation",
            )?;
            let operation = non_empty_string(
                required_alias(async_entries, &["operation", "hta/operation"], origin)?,
                origin,
                "async operation",
            )?
            .to_owned();
            let request = optional_alias(async_entries, &["request", "hta/request"])
                .map(|form| parse_request(form, origin, export))
                .transpose()?
                .unwrap_or_default();
            let cancellation = optional_alias(async_entries, &["cancellation", "hta/cancellation"])
                .map(|form| parse_cancellation(form, origin, export))
                .transpose()?
                .unwrap_or(super::CancellationPolicy::Cooperative);
            Ok((
                true,
                Some(super::AsyncPolicy {
                    operation,
                    request,
                    cancellation,
                }),
            ))
        }
        _ => Err(malformed(
            origin,
            format!("export {export} async must be boolean or a policy map"),
        )),
    }
}

fn optional_alias<'a>(entries: &'a [(Form, Form)], names: &[&str]) -> Option<&'a Form> {
    names.iter().find_map(|name| optional(entries, name))
}

fn required_alias<'a>(
    entries: &'a [(Form, Form)],
    names: &[&str],
    origin: &str,
) -> Result<&'a Form, String> {
    optional_alias(entries, names)
        .ok_or_else(|| malformed(origin, format!("missing required field {}", names[0])))
}

fn optional_string_alias(
    entries: &[(Form, Form)],
    names: &[&str],
    origin: &str,
) -> Result<Option<String>, String> {
    optional_alias(entries, names)
        .map(|form| non_empty_string(form, origin, names[0]).map(str::to_owned))
        .transpose()
}

fn reject_aliases(
    entries: &[(Form, Form)],
    names: &[&str],
    origin: &str,
    scope: &str,
) -> Result<(), String> {
    if names
        .iter()
        .filter(|name| optional(entries, name).is_some())
        .count()
        > 1
    {
        return Err(malformed(
            origin,
            format!("{scope} cannot be declared more than once"),
        ));
    }
    Ok(())
}

fn parse_request(form: &Form, origin: &str, export: &str) -> Result<super::RequestPolicy, String> {
    let entries = map(form, origin, &format!("export {export} request"))?;
    reject_unknown(
        entries,
        REQUEST_FIELDS,
        origin,
        &format!("export {export} request"),
    )?;
    let timeout_ms = optional_number(entries, "timeout-ms", origin, true)?;
    let max_in_flight = optional_number(entries, "max-in-flight", origin, false)?
        .map(|value| {
            u32::try_from(value).map_err(|_| {
                malformed(
                    origin,
                    format!("export {export} request max-in-flight is too large"),
                )
            })
        })
        .transpose()?;
    Ok(super::RequestPolicy {
        timeout_ms,
        max_in_flight,
    })
}

fn optional_number(
    entries: &[(Form, Form)],
    name: &str,
    origin: &str,
    allow_zero: bool,
) -> Result<Option<u64>, String> {
    optional(entries, name)
        .map(|form| match form {
            Form::Number(value) if *value >= 0 && (allow_zero || *value > 0) => {
                u64::try_from(*value).map_err(|_| malformed(origin, format!("{name} is too large")))
            }
            _ => Err(malformed(
                origin,
                format!("{name} must be a positive integer"),
            )),
        })
        .transpose()
}

fn parse_cancellation(
    form: &Form,
    origin: &str,
    export: &str,
) -> Result<super::CancellationPolicy, String> {
    let value = keyword(form, origin, &format!("export {export} cancellation"))?;
    parse_cancellation_value(value, origin, export)
}

fn parse_cancellation_value(
    value: &str,
    origin: &str,
    export: &str,
) -> Result<super::CancellationPolicy, String> {
    match value {
        "cooperative" | "best-effort" => Ok(super::CancellationPolicy::Cooperative),
        "abort" => Ok(super::CancellationPolicy::Abort),
        "ignore" | "none" => Ok(super::CancellationPolicy::Ignore),
        value => Err(unsupported(
            origin,
            format!("export {export} uses unsupported cancellation policy :{value}"),
        )),
    }
}

fn parse_host_calls(
    form: &Form,
    origin: &str,
) -> Result<BTreeMap<String, super::HostCallContract>, String> {
    let mut result = BTreeMap::new();
    for (service, specification) in map(form, origin, "host-calls")? {
        let service = named(service, origin, "host-call service")?.to_owned();
        if !valid_tag(&service) {
            return Err(malformed(
                origin,
                format!("invalid host-call service {service}"),
            ));
        }
        let (methods, capabilities) = match specification {
            Form::Vector(_) => (
                named_set(specification, origin, "host-call methods")?,
                BTreeSet::new(),
            ),
            Form::Map(_) => {
                let entries = map(specification, origin, "host-call specification")?;
                reject_unknown(entries, HOST_CALL_FIELDS, origin, "host-call")?;
                (
                    named_set(
                        required(entries, "methods", origin)?,
                        origin,
                        "host-call methods",
                    )?,
                    optional(entries, "capabilities")
                        .map(|form| named_set(form, origin, "host-call capabilities"))
                        .transpose()?
                        .unwrap_or_default(),
                )
            }
            _ => {
                return Err(malformed(
                    origin,
                    "host-call specification must be a vector or map",
                ))
            }
        };
        if methods.is_empty() {
            return Err(malformed(
                origin,
                format!("host-call service {service} must declare methods"),
            ));
        }
        if result
            .insert(
                service.clone(),
                super::HostCallContract {
                    methods,
                    capabilities,
                },
            )
            .is_some()
        {
            return Err(malformed(
                origin,
                format!("duplicate host-call service {service}"),
            ));
        }
    }
    Ok(result)
}

fn parse_callbacks(
    form: &Form,
    origin: &str,
) -> Result<BTreeMap<String, super::CallbackContract>, String> {
    let mut result = BTreeMap::new();
    for (name, specification) in map(form, origin, "callbacks")? {
        let name = named(name, origin, "callback name")?.to_owned();
        if !valid_binding_name(&name) {
            return Err(malformed(origin, format!("invalid callback name {name}")));
        }
        let entries = map(specification, origin, &format!("callback {name}"))?;
        reject_unknown(
            entries,
            CALLBACK_FIELDS,
            origin,
            &format!("callback {name}"),
        )?;
        if optional_bool(entries, "reentrant", origin)?.unwrap_or(false) {
            return Err(unsupported(
                origin,
                format!("callback {name} cannot be reentrant in HTA v1"),
            ));
        }
        let reentrant = optional_bool(entries, "reentrant", origin)?.unwrap_or(false);
        let arguments = vector(
            required(entries, "arguments", origin)?,
            origin,
            &format!("callback {name} arguments"),
        )?
        .iter()
        .enumerate()
        .map(|(index, form)| {
            let (argument_name, value) =
                callback_parameter(form, origin, &format!("callback {name} argument {index}"))?;
            if matches!(value, HaraValueType::Callback(_)) {
                return Err(unsupported(
                    origin,
                    format!("callback {name} arguments must be transfer-safe"),
                ));
            }
            Ok(CallbackParameter {
                name: argument_name,
                hara_type: value,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
        let (_, returns) = callback_parameter(
            required(entries, "returns", origin)?,
            origin,
            &format!("callback {name} returns"),
        )?;
        if matches!(returns, HaraValueType::Callback(_)) {
            return Err(unsupported(
                origin,
                format!("callback {name} result must be transfer-safe"),
            ));
        }
        if result
            .insert(
                name.clone(),
                super::CallbackContract {
                    arguments,
                    returns,
                    reentrant,
                },
            )
            .is_some()
        {
            return Err(malformed(origin, format!("duplicate callback {name}")));
        }

        fn callback_parameter(
            form: &Form,
            origin: &str,
            field: &str,
        ) -> Result<(String, HaraValueType), String> {
            if let Form::Map(_) = form {
                let entries = map(form, origin, field)?;
                reject_unknown(entries, &["name", "hara/type"], origin, field)?;
                let name = optional(entries, "name")
                    .map(|form| named(form, origin, &format!("{field} name")).map(str::to_owned))
                    .transpose()?
                    .unwrap_or_default();
                let value =
                    HaraValueType::parse(required(entries, "hara/type", origin)?, origin, field)?;
                Ok((name, value))
            } else {
                Ok((String::new(), HaraValueType::parse(form, origin, field)?))
            }
        }
    }
    Ok(result)
}

fn parse_handles(
    form: &Form,
    origin: &str,
    scope: &str,
) -> Result<BTreeMap<String, super::HandleContract>, String> {
    let mut result = BTreeMap::new();
    for (name, specification) in map(form, origin, scope)? {
        let name = named(name, origin, &format!("{scope} type"))?.to_owned();
        if !valid_tag(&name) {
            return Err(malformed(origin, format!("invalid {scope} type {name}")));
        }
        let entries = map(specification, origin, &format!("{scope} {name}"))?;
        reject_unknown(entries, HANDLE_FIELDS, origin, &format!("{scope} {name}"))?;
        let tag = named(
            required(entries, "tag", origin)?,
            origin,
            &format!("{scope} {name} tag"),
        )?
        .to_owned();
        if !valid_tag(&tag) {
            return Err(malformed(
                origin,
                format!("{scope} {name} tag must be lower-case"),
            ));
        }
        let release = optional_string(entries, "release", origin)?;
        if release.is_none() {
            return Err(malformed(
                origin,
                format!("{scope} {name} requires an explicit :release operation"),
            ));
        }
        if result
            .insert(name.clone(), super::HandleContract { tag, release })
            .is_some()
        {
            return Err(malformed(origin, format!("duplicate {scope} type {name}")));
        }
    }
    Ok(result)
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
    let hta = interface.hta_required();
    for export in &interface.exports {
        for argument in &export.arguments {
            uses_memory |= validate_parameter(argument, origin, &export.name, hta)?;
        }
        uses_memory |= validate_result(&export.returns, origin, &export.name, hta)?;
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
    hta: bool,
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
            if hta {
                return Ok(false);
            }
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

fn validate_result(
    result: &BindingResult,
    origin: &str,
    export: &str,
    hta: bool,
) -> Result<bool, String> {
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
            if hta {
                return Ok(false);
            }
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
