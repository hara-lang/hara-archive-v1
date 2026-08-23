use crate::kernel::Form;

use super::{
    BindingFunction, BindingParameter, BindingResult, CallbackContract, CancellationPolicy,
    ErrorContract, HandleContract, HaraValueType, HostCallContract, Lifting, Lowering,
    MemoryContract, Ownership, RequestPolicy, WasmInterface,
};

impl HaraValueType {
    fn canonical_form(&self) -> Form {
        match self {
            Self::I32 => keyword_form("i32"),
            Self::I64 => keyword_form("i64"),
            Self::F32 => keyword_form("f32"),
            Self::F64 => keyword_form("f64"),
            Self::Boolean => keyword_form("boolean"),
            Self::String => keyword_form("string"),
            Self::Bytes => keyword_form("bytes"),
            Self::Record(name) => named_type_form("record", name),
            Self::Variant(name) => named_type_form("variant", name),
            Self::Handle(name) => named_type_form("handle", name),
            Self::Callback(name) => named_type_form("callback", name),
            Self::Void => keyword_form("void"),
        }
    }
}

impl Ownership {
    fn as_keyword(self) -> &'static str {
        match self {
            Self::Borrowed => "borrowed",
            Self::Caller => "caller",
            Self::Callee => "callee",
            Self::Transferred => "transferred",
        }
    }
}

impl Lowering {
    fn canonical_form(self) -> Form {
        match self {
            Self::Direct => keyword_form("direct"),
            Self::PointerLength => {
                Form::Vector(vec![keyword_form("pointer"), keyword_form("length")])
            }
        }
    }
}

impl Lifting {
    fn canonical_form(self) -> Form {
        match self {
            Self::Direct => keyword_form("direct"),
            Self::PointerLength => {
                Form::Vector(vec![keyword_form("pointer"), keyword_form("length")])
            }
            Self::PackedI64 => keyword_form("packed-i64"),
        }
    }
}

pub(super) fn source(interface: &WasmInterface) -> String {
    Form::List(vec![symbol_form("wasm/interface"), payload_form(interface)]).to_string()
}

fn payload_form(interface: &WasmInterface) -> Form {
    let mut entries = vec![
        (keyword_form("schema"), string_form(&interface.schema)),
        (keyword_form("namespace"), symbol_form(&interface.namespace)),
        (keyword_form("module"), string_form(&interface.module)),
    ];
    if let Some(memory) = interface.memory.as_ref() {
        entries.push((keyword_form("memory"), memory_form(memory)));
    }
    entries.push((
        keyword_form("exports"),
        Form::Map(
            interface
                .exports
                .iter()
                .map(|export| (symbol_form(&export.name), export_form(export)))
                .collect(),
        ),
    ));
    if !interface.capabilities.is_empty() {
        entries.push((
            keyword_form("capabilities"),
            Form::Vector(
                interface
                    .capabilities
                    .iter()
                    .map(|capability| keyword_form(capability))
                    .collect(),
            ),
        ));
    }
    push_contract_map(
        &mut entries,
        "host-calls",
        &interface.host_calls,
        host_call_form,
    );
    push_contract_map(
        &mut entries,
        "callbacks",
        &interface.callbacks,
        callback_form,
    );
    push_contract_map(&mut entries, "handles", &interface.handles, handle_form);
    push_contract_map(&mut entries, "resources", &interface.resources, handle_form);
    Form::Map(entries)
}

fn memory_form(memory: &MemoryContract) -> Form {
    let mut entries = vec![(keyword_form("export"), string_form(&memory.export))];
    push_optional_string(&mut entries, "allocate", memory.allocate.as_deref());
    push_optional_string(&mut entries, "reallocate", memory.reallocate.as_deref());
    push_optional_string(&mut entries, "release", memory.release.as_deref());
    Form::Map(entries)
}

fn export_form(export: &BindingFunction) -> Form {
    let mut entries = vec![
        (
            keyword_form("wasm/export"),
            string_form(&export.wasm_export),
        ),
        (
            keyword_form("arguments"),
            Form::Vector(export.arguments.iter().map(parameter_form).collect()),
        ),
        (keyword_form("returns"), result_form(&export.returns)),
    ];
    if let Some(operation) = export.operation.as_ref() {
        entries.push((keyword_form("async"), Form::Bool(true)));
        entries.push((keyword_form("hta/operation"), string_form(operation)));
        let request = export.request.clone().unwrap_or_default();
        if request.timeout_ms.is_some() || request.max_in_flight.is_some() {
            let mut request_entries = Vec::new();
            request_form(&mut request_entries, &request);
            entries.push((keyword_form("hta/request"), Form::Map(request_entries)));
        }
        if let Some(cancellation) = export.cancellation {
            if cancellation != CancellationPolicy::Cooperative {
                entries.push((
                    keyword_form("hta/cancellation"),
                    keyword_form(cancellation.as_keyword()),
                ));
            }
        }
    }

    if let Some(errors) = export.errors.as_ref() {
        entries.push((keyword_form("errors"), error_form(errors)));
    }
    if !export.capabilities.is_empty() {
        entries.push((
            keyword_form("capabilities"),
            Form::Vector(
                export
                    .capabilities
                    .iter()
                    .map(|capability| keyword_form(capability))
                    .collect(),
            ),
        ));
    }
    Form::Map(entries)
}

fn request_form(entries: &mut Vec<(Form, Form)>, request: &RequestPolicy) {
    if let Some(timeout_ms) = request.timeout_ms {
        entries.push((keyword_form("timeout-ms"), Form::Number(timeout_ms as i64)));
    }
    if let Some(max_in_flight) = request.max_in_flight {
        entries.push((
            keyword_form("max-in-flight"),
            Form::Number(i64::from(max_in_flight)),
        ));
    }
}

impl CancellationPolicy {
    fn as_keyword(self) -> &'static str {
        match self {
            Self::Cooperative => "cooperative",
            Self::Abort => "abort",
            Self::Ignore => "ignore",
        }
    }
}

fn push_contract_map<T>(
    entries: &mut Vec<(Form, Form)>,
    name: &str,
    values: &std::collections::BTreeMap<String, T>,
    render: fn(&T) -> Form,
) {
    if !values.is_empty() {
        entries.push((
            keyword_form(name),
            Form::Map(
                values
                    .iter()
                    .map(|(key, value)| (symbol_form(key), render(value)))
                    .collect(),
            ),
        ));
    }
}

fn host_call_form(contract: &HostCallContract) -> Form {
    let mut entries = vec![(
        keyword_form("methods"),
        Form::Vector(
            contract
                .methods
                .iter()
                .map(|method| keyword_form(method))
                .collect(),
        ),
    )];
    if !contract.capabilities.is_empty() {
        entries.push((
            keyword_form("capabilities"),
            Form::Vector(
                contract
                    .capabilities
                    .iter()
                    .map(|capability| keyword_form(capability))
                    .collect(),
            ),
        ));
    }
    Form::Map(entries)
}

fn callback_form(contract: &CallbackContract) -> Form {
    let arguments = contract
        .arguments
        .iter()
        .map(|argument| {
            if argument.name.is_empty() {
                argument.hara_type.canonical_form()
            } else {
                Form::Map(vec![
                    (keyword_form("name"), symbol_form(&argument.name)),
                    (
                        keyword_form("hara/type"),
                        argument.hara_type.canonical_form(),
                    ),
                ])
            }
        })
        .collect();
    let mut entries = vec![
        (keyword_form("arguments"), Form::Vector(arguments)),
        (keyword_form("returns"), contract.returns.canonical_form()),
    ];
    if contract.reentrant {
        entries.push((keyword_form("reentrant"), Form::Bool(true)));
    }
    Form::Map(entries)
}

fn handle_form(contract: &HandleContract) -> Form {
    let mut entries = vec![(keyword_form("tag"), symbol_form(&contract.tag))];
    push_optional_string(&mut entries, "release", contract.release.as_deref());
    Form::Map(entries)
}

fn parameter_form(parameter: &BindingParameter) -> Form {
    let mut entries = vec![
        (keyword_form("name"), symbol_form(&parameter.name)),
        (
            keyword_form("hara/type"),
            parameter.hara_type.canonical_form(),
        ),
        (
            keyword_form("wasm/type"),
            keyword_form(parameter.wasm_type.as_keyword()),
        ),
    ];
    if let Some(lowering) = parameter.lowering {
        entries.push((keyword_form("lower"), lowering.canonical_form()));
    }
    if let Some(ownership) = parameter.ownership {
        entries.push((
            keyword_form("ownership"),
            keyword_form(ownership.as_keyword()),
        ));
    }
    Form::Map(entries)
}

fn result_form(result: &BindingResult) -> Form {
    let mut entries = vec![
        (keyword_form("hara/type"), result.hara_type.canonical_form()),
        (
            keyword_form("wasm/type"),
            keyword_form(result.wasm_type.as_keyword()),
        ),
    ];
    if let Some(lifting) = result.lifting {
        entries.push((keyword_form("lift"), lifting.canonical_form()));
    }
    if let Some(ownership) = result.ownership {
        entries.push((
            keyword_form("ownership"),
            keyword_form(ownership.as_keyword()),
        ));
    }
    Form::Map(entries)
}

fn error_form(errors: &ErrorContract) -> Form {
    Form::Map(vec![
        (keyword_form("convention"), keyword_form(&errors.convention)),
        (
            keyword_form("codes"),
            Form::Map(
                errors
                    .codes
                    .iter()
                    .map(|(code, value)| (Form::Number(*code), keyword_form(value)))
                    .collect(),
            ),
        ),
    ])
}

fn named_type_form(kind: &str, name: &str) -> Form {
    Form::Vector(vec![keyword_form(kind), symbol_form(name)])
}

fn push_optional_string(entries: &mut Vec<(Form, Form)>, name: &str, value: Option<&str>) {
    if let Some(value) = value {
        entries.push((keyword_form(name), string_form(value)));
    }
}

fn keyword_form(value: &str) -> Form {
    Form::Keyword(value.to_owned())
}

fn symbol_form(value: &str) -> Form {
    Form::Symbol(value.to_owned())
}

fn string_form(value: &str) -> Form {
    Form::String(value.to_owned())
}
