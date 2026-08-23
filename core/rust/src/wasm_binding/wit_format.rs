use crate::kernel::Form;

use super::wit::{digest, keyword, named_type, string, symbol};
use super::wit_parser::{self, Function, Interface, Type, TypeDecl};
use super::{
    HaraValueType, Lowering, Ownership, WasmValueType, WitDiagnostic, WitDiagnosticSeverity,
    WitRoute, WASM_INTERFACE_SCHEMA, WIT_IR_SCHEMA,
};

pub(super) fn skeleton_source(
    namespace: &str,
    module: &str,
    exports: &[(
        &Function,
        Vec<(String, HaraValueType)>,
        Option<HaraValueType>,
    )],
    memory: bool,
    _route: WitRoute,
) -> String {
    let mut entries = vec![
        (keyword("schema"), string(WASM_INTERFACE_SCHEMA)),
        (keyword("namespace"), symbol(namespace)),
        (keyword("module"), string(module)),
    ];
    if memory {
        entries.push((
            keyword("memory"),
            Form::Map(vec![(keyword("export"), string("memory"))]),
        ));
    }
    entries.push((
        keyword("exports"),
        Form::Map(
            exports
                .iter()
                .map(|(function, arguments, result)| {
                    (
                        symbol(&function.name),
                        Form::Map(vec![
                            (keyword("wasm/export"), string(&function.name)),
                            (
                                keyword("arguments"),
                                Form::Vector(
                                    arguments
                                        .iter()
                                        .enumerate()
                                        .map(|(index, (name, ty))| {
                                            parameter_form(
                                                name,
                                                ty,
                                                index,
                                                function.arguments[index].1.clone(),
                                            )
                                        })
                                        .collect(),
                                ),
                            ),
                            (
                                keyword("returns"),
                                result_form(result.as_ref(), function.result.as_ref()),
                            ),
                        ]),
                    )
                })
                .collect(),
        ),
    ));
    Form::List(vec![symbol("wasm/interface"), Form::Map(entries)]).to_string()
}

fn parameter_form(name: &str, ty: &HaraValueType, index: usize, original: Type) -> Form {
    let name = if name.is_empty() {
        format!("arg-{index}")
    } else {
        name.to_owned()
    };
    let mut entries = vec![
        (keyword("name"), symbol(&name)),
        (keyword("hara/type"), hara_type_form(ty)),
    ];
    match ty {
        HaraValueType::String | HaraValueType::Bytes => {
            entries.push((keyword("wasm/type"), keyword("i32")));
            entries.push((
                keyword("lower"),
                Form::Vector(vec![keyword("pointer"), keyword("length")]),
            ));
            entries.push((keyword("ownership"), keyword("borrowed")));
        }
        HaraValueType::Record(_)
        | HaraValueType::Variant(_)
        | HaraValueType::Handle(_)
        | HaraValueType::Callback(_) => {
            entries.push((keyword("wasm/type"), keyword("unresolved")));
        }
        _ => entries.push((keyword("wasm/type"), keyword(wasm_type_for(ty, &original)))),
    }
    Form::Map(entries)
}

fn result_form(mapped: Option<&HaraValueType>, original: Option<&Type>) -> Form {
    let ty = mapped.cloned().unwrap_or(HaraValueType::Void);
    let mut entries = vec![(keyword("hara/type"), hara_type_form(&ty))];
    match ty {
        HaraValueType::String | HaraValueType::Bytes => {
            entries.push((keyword("wasm/type"), keyword("i64")));
            entries.push((keyword("lift"), keyword("packed-i64")));
            entries.push((keyword("ownership"), keyword("caller")));
        }
        HaraValueType::Record(_)
        | HaraValueType::Variant(_)
        | HaraValueType::Handle(_)
        | HaraValueType::Callback(_) => {
            entries.push((keyword("wasm/type"), keyword("unresolved")));
        }
        _ => entries.push((
            keyword("wasm/type"),
            keyword(wasm_type_for(
                &ty,
                original.unwrap_or(&Type::Atom("unit".into())),
            )),
        )),
    }
    Form::Map(entries)
}

pub(super) fn normalized_source(
    namespace: &str,
    package: Option<&str>,
    world: Option<&str>,
    interface_name: &str,
    route: WitRoute,
    interface: &Interface,
    exports: &[(
        &Function,
        Vec<(String, HaraValueType)>,
        Option<HaraValueType>,
    )],
    diagnostics: &[WitDiagnostic],
    source: &str,
    origin: &str,
) -> String {
    let types = interface
        .types
        .iter()
        .map(|(name, declaration)| {
            Form::Map(vec![
                (keyword("name"), symbol(name)),
                (keyword("kind"), keyword(type_decl_kind(declaration))),
                (keyword("source"), string(&format!("{declaration:?}"))),
            ])
        })
        .collect();
    Form::Map(vec![
        (keyword("schema"), string(WIT_IR_SCHEMA)),
        (keyword("namespace"), symbol(namespace)),
        (keyword("package"), package.map(string).unwrap_or(Form::Nil)),
        (keyword("world"), world.map(symbol).unwrap_or(Form::Nil)),
        (keyword("interface"), symbol(interface_name)),
        (keyword("route"), keyword(route.as_keyword())),
        (keyword("types"), Form::Vector(types)),
        (
            keyword("exports"),
            Form::Vector(
                exports
                    .iter()
                    .map(|(function, arguments, result)| {
                        Form::Map(vec![
                            (keyword("name"), symbol(&function.name)),
                            (
                                keyword("arguments"),
                                Form::Vector(
                                    arguments
                                        .iter()
                                        .enumerate()
                                        .map(|(index, (name, ty))| {
                                            Form::Map(vec![
                                                (keyword("name"), symbol(name)),
                                                (keyword("hara/type"), hara_type_form(ty)),
                                                (
                                                    keyword("wit/type"),
                                                    string(&wit_parser::type_label(
                                                        &function.arguments[index].1,
                                                    )),
                                                ),
                                            ])
                                        })
                                        .collect(),
                                ),
                            ),
                            (
                                keyword("returns"),
                                result.as_ref().map_or_else(
                                    || {
                                        Form::Map(vec![
                                            (keyword("hara/type"), keyword("void")),
                                            (keyword("wit/type"), keyword("unit")),
                                        ])
                                    },
                                    |result| {
                                        Form::Map(vec![
                                            (keyword("hara/type"), hara_type_form(result)),
                                            (
                                                keyword("wit/type"),
                                                string(
                                                    &function
                                                        .result
                                                        .as_ref()
                                                        .map(wit_parser::type_label)
                                                        .unwrap_or_else(|| "unit".into()),
                                                ),
                                            ),
                                        ])
                                    },
                                ),
                            ),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            keyword("diagnostics"),
            Form::Vector(diagnostics.iter().map(diagnostic_form).collect()),
        ),
        (
            keyword("provenance"),
            Form::Map(vec![
                (keyword("origin"), string(origin)),
                (keyword("source-digest"), string(&digest(source.as_bytes()))),
            ]),
        ),
    ])
    .to_string()
}

pub(super) fn wit_projection_type(
    ty: &HaraValueType,
    wasm: WasmValueType,
    lowering: Option<Lowering>,
    ownership: Option<Ownership>,
    diagnostics: &mut Vec<WitDiagnostic>,
    path: &str,
) -> Option<String> {
    let exact_scalar = match ty {
        HaraValueType::Boolean if wasm == WasmValueType::I32 => Some("bool"),
        HaraValueType::I32 if wasm == WasmValueType::I32 => Some("s32"),
        HaraValueType::I64 if wasm == WasmValueType::I64 => Some("s64"),
        HaraValueType::F32 if wasm == WasmValueType::F32 => Some("f32"),
        HaraValueType::F64 if wasm == WasmValueType::F64 => Some("f64"),
        HaraValueType::Void if wasm == WasmValueType::Void => Some("unit"),
        _ => None,
    };
    if let Some(value) = exact_scalar {
        return Some(value.into());
    }
    let memory = lowering.is_some()
        && ownership.is_some()
        && matches!(lowering, Some(Lowering::PointerLength));
    match ty {
        HaraValueType::String if wasm == WasmValueType::I32 || wasm == WasmValueType::I64 => {
            if memory {
                Some("string".into())
            } else {
                projection_error(
                    diagnostics,
                    path,
                    "string lacks an explicit memory lowering",
                );
                None
            }
        }
        HaraValueType::Bytes if wasm == WasmValueType::I32 || wasm == WasmValueType::I64 => {
            if memory {
                Some("list<u8>".into())
            } else {
                projection_error(diagnostics, path, "bytes lacks an explicit memory lowering");
                None
            }
        }
        HaraValueType::Record(_)
        | HaraValueType::Variant(_)
        | HaraValueType::Handle(_)
        | HaraValueType::Callback(_) => {
            projection_error(
                diagnostics,
                path,
                "named Hara types require their canonical definitions before projection",
            );
            None
        }
        _ => {
            projection_error(
                diagnostics,
                path,
                "Hara and Wasm types do not form an exact WIT mapping",
            );
            None
        }
    }
}

fn projection_error(diagnostics: &mut Vec<WitDiagnostic>, path: &str, message: &str) {
    diagnostic(
        diagnostics,
        WitDiagnosticSeverity::Unsupported,
        "projection",
        path,
        message,
    );
}

pub(super) fn diagnostic(
    diagnostics: &mut Vec<WitDiagnostic>,
    severity: WitDiagnosticSeverity,
    code: &str,
    path: &str,
    message: &str,
) {
    diagnostics.push(WitDiagnostic {
        severity,
        code: code.into(),
        path: path.into(),
        message: message.into(),
    });
}

pub(super) fn diagnostic_form(diagnostic: &WitDiagnostic) -> Form {
    Form::Map(vec![
        (
            keyword("severity"),
            keyword(diagnostic.severity.as_keyword()),
        ),
        (keyword("code"), keyword(&diagnostic.code)),
        (keyword("path"), string(&diagnostic.path)),
        (keyword("message"), string(&diagnostic.message)),
    ])
}

fn hara_type_form(ty: &HaraValueType) -> Form {
    match ty {
        HaraValueType::I32 => keyword("i32"),
        HaraValueType::I64 => keyword("i64"),
        HaraValueType::F32 => keyword("f32"),
        HaraValueType::F64 => keyword("f64"),
        HaraValueType::Boolean => keyword("boolean"),
        HaraValueType::String => keyword("string"),
        HaraValueType::Bytes => keyword("bytes"),
        HaraValueType::Record(name) => named_type("record", name),
        HaraValueType::Variant(name) => named_type("variant", name),
        HaraValueType::Handle(name) => named_type("handle", name),
        HaraValueType::Callback(name) => named_type("callback", name),
        HaraValueType::Void => keyword("void"),
    }
}

fn wasm_type_for(ty: &HaraValueType, original: &Type) -> &'static str {
    match ty {
        HaraValueType::Boolean | HaraValueType::I32 => "i32",
        HaraValueType::I64 => "i64",
        HaraValueType::F32 => "f32",
        HaraValueType::F64 => "f64",
        HaraValueType::Void => "void",
        HaraValueType::String | HaraValueType::Bytes => {
            if matches!(original, Type::List(_)) {
                "i32"
            } else {
                "i32"
            }
        }
        _ => "unresolved",
    }
}

fn type_decl_kind(declaration: &TypeDecl) -> &'static str {
    match declaration {
        TypeDecl::Alias(_) => "alias",
        TypeDecl::Record(_) => "record",
        TypeDecl::Variant(_) => "variant",
        TypeDecl::Resource => "resource",
        TypeDecl::Unsupported(_) => "unsupported",
    }
}
