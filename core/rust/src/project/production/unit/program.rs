use super::{qualify, Effect, UnitAnalysis, UnitKind};
use crate::core::{self, Value};
use crate::kernel::{parse, Form};
use crate::vm::{Instruction, Program};

pub(super) fn scan_program(program: &Program, analysis: &mut UnitAnalysis) {
    scan_declaration_roots(analysis);
    for prototype in &program.functions {
        for instruction in &prototype.code {
            match instruction {
                Instruction::GetGlobal(index)
                | Instruction::VarGlobal(index)
                | Instruction::SetGlobal(index)
                | Instruction::DeclareGlobal(index) => {
                    if let Some(name) = string_constant(program, *index) {
                        analysis.runtime_edges.insert(name.to_owned());
                        classify_native_edge(name, analysis);
                    }
                }
                Instruction::DynamicBind(index) | Instruction::DynamicUnbind(index) => {
                    if let Some(name) = string_constant(program, *index) {
                        analysis.runtime_edges.insert(name.to_owned());
                        classify_native_edge(name, analysis);
                    }
                    analysis
                        .native_roots
                        .runtime_shims
                        .insert("hara.runtime/dynamic-binding".into());
                }
                Instruction::IntrinsicCall { target, .. }
                | Instruction::ProtocolCall { target, .. }
                | Instruction::IntrinsicValue(target) => {
                    if let Some(name) = string_constant(program, *target) {
                        analysis.native_roots.primitives.insert(name.to_owned());
                        analysis.native_primitives.insert(name.to_owned());
                    } else {
                        noncanonical_root(
                            analysis,
                            format!(
                                "intrinsic instruction has no string identity at constant {target}"
                            ),
                        );
                    }
                }
                Instruction::BuiltinValue(index) => {
                    if let Some(name) = string_constant(program, *index) {
                        analysis.native_roots.primitives.insert(name.to_owned());
                        analysis.native_primitives.insert(name.to_owned());
                    } else {
                        noncanonical_root(
                            analysis,
                            format!(
                                "builtin instruction has no string identity at constant {index}"
                            ),
                        );
                    }
                }
                Instruction::NamespaceValue(index) => {
                    if let Some(name) = string_constant(program, *index) {
                        analysis.runtime_edges.insert(name.to_owned());
                    } else {
                        noncanonical_root(
                            analysis,
                            format!(
                                "namespace instruction has no string identity at constant {index}"
                            ),
                        );
                    }
                }
                Instruction::NamespaceOperation(index) => {
                    if let Some(value) = program.constants.get(*index as usize) {
                        if let Ok(form) = core::value_to_form(value) {
                            if let Form::List(items) = core::form_without_metadata(&form) {
                                if let Some(Form::Symbol(operator)) = items.first() {
                                    analysis
                                        .runtime_edges
                                        .insert(format!("namespace-operation:{operator}"));
                                }
                            }
                        }
                    } else {
                        noncanonical_root(
                            analysis,
                            format!(
                                "namespace-management instruction has no form at constant {index}"
                            ),
                        );
                    }
                }
                Instruction::HostCall => {
                    let name = "std.native.Host/call".to_owned();
                    analysis.native_roots.host_calls.insert(name.clone());
                    analysis
                        .native_roots
                        .runtime_shims
                        .insert("hara.runtime/host-call".into());
                    analysis.native_primitives.insert(name);
                }
                Instruction::DotCall { method, .. } => {
                    if let Some(name) = string_constant(program, *method) {
                        analysis
                            .native_roots
                            .dynamic_methods
                            .insert(format!("dot:{name}"));
                        analysis.native_primitives.insert(format!("dot:{name}"));
                    } else {
                        noncanonical_root(
                            analysis,
                            format!("dot call has no string method at constant {method}"),
                        );
                    }
                }
                Instruction::DefStruct { name, .. } | Instruction::DefMutable { name, .. } => {
                    if let Some(name) = string_constant(program, *name) {
                        let name = qualify(&analysis.module, name);
                        analysis.native_roots.types.insert(name.clone());
                        analysis.native_types.insert(name);
                    }
                    analysis
                        .native_roots
                        .runtime_shims
                        .insert("hara.runtime/named-values".into());
                }
                Instruction::DefProtocol(_) => {
                    analysis
                        .native_roots
                        .runtime_shims
                        .insert("hara.runtime/protocol-registry".into());
                }
                Instruction::ExtendType(_) => {
                    analysis
                        .native_roots
                        .runtime_shims
                        .insert("hara.runtime/protocol-extension-registry".into());
                }
                Instruction::DefMulti(_) | Instruction::DefMethod(_) => {
                    analysis
                        .native_roots
                        .runtime_shims
                        .insert("hara.runtime/multimethod-registry".into());
                }
                Instruction::Await => {
                    analysis
                        .native_roots
                        .runtime_shims
                        .insert("hara.runtime/promise-await".into());
                }
                Instruction::Yield => {
                    analysis
                        .native_roots
                        .runtime_shims
                        .insert("hara.runtime/coroutine-yield".into());
                }
                _ => {}
            }
        }
    }
}

pub(super) fn classify_effect(program: &Program, kind: UnitKind) -> Effect {
    if kind == UnitKind::Registration {
        return Effect::Unknown;
    }
    let Some(entry) = program.functions.first() else {
        return Effect::Unknown;
    };
    let mut unknown = false;
    for instruction in &entry.code {
        match instruction {
            Instruction::SetGlobal(_)
            | Instruction::MutableFieldSet(_)
            | Instruction::DynamicBind(_)
            | Instruction::DynamicUnbind(_)
            | Instruction::HostCall
            | Instruction::DotCall { .. }
            | Instruction::ExtendType(_)
            | Instruction::DefMethod(_) => return Effect::Effectful,
            Instruction::Call { .. }
            | Instruction::CallStatic { .. }
            | Instruction::Await
            | Instruction::Yield => unknown = true,
            _ => {}
        }
    }
    if unknown {
        Effect::Unknown
    } else {
        Effect::Pure
    }
}

fn string_constant(program: &Program, index: u32) -> Option<&str> {
    match program.constants.get(index as usize) {
        Some(Value::String(value)) => Some(value),
        _ => None,
    }
}

fn classify_native_edge(name: &str, analysis: &mut UnitAnalysis) {
    let Some((namespace, _)) = name.split_once('/') else {
        return;
    };
    if namespace.starts_with("std.native.") {
        analysis.native_roots.types.insert(namespace.to_owned());
        analysis.native_roots.methods.insert(name.to_owned());
        analysis.native_types.insert(namespace.to_owned());
    }
    if namespace.starts_with("std.protocol.") {
        analysis.native_roots.protocols.insert(namespace.to_owned());
        analysis
            .native_roots
            .protocol_methods
            .insert(name.to_owned());
        analysis.native_protocols.insert(namespace.to_owned());
    }
}

fn scan_declaration_roots(analysis: &mut UnitAnalysis) {
    let Ok(form) = parse(&analysis.form_source) else {
        return;
    };
    let Form::List(values) = core::form_without_metadata(&form) else {
        return;
    };
    let Some(Form::Symbol(operator)) = values.first() else {
        return;
    };
    let name = values
        .get(1)
        .map(core::form_without_metadata)
        .and_then(|form| match form {
            Form::Symbol(name) => Some(canonical_name(&analysis.module, name)),
            _ => None,
        });
    match (operator.as_str(), name) {
        ("defprotocol", Some(name)) => {
            analysis.native_roots.protocols.insert(name.clone());
            analysis.native_protocols.insert(name);
        }
        ("extend-type", Some(name)) => {
            analysis.native_roots.types.insert(name);
            collect_native_symbols(&form, analysis);
        }
        ("defmulti" | "defmethod", Some(name)) => {
            analysis.native_roots.multimethods.insert(name.clone());
            analysis
                .native_protocols
                .insert(format!("multimethod:{name}"));
        }
        _ => {}
    }
}

fn collect_native_symbols(form: &Form, analysis: &mut UnitAnalysis) {
    match core::form_without_metadata(form) {
        Form::Symbol(name) => classify_native_edge(name, analysis),
        Form::List(values) | Form::Vector(values) | Form::Set(values) => {
            for value in values {
                collect_native_symbols(value, analysis);
            }
        }
        Form::Map(entries) => {
            for (key, value) in entries {
                collect_native_symbols(key, analysis);
                collect_native_symbols(value, analysis);
            }
        }
        Form::Tagged(_, value) => collect_native_symbols(value, analysis),
        _ => {}
    }
}

fn canonical_name(module: &str, name: &str) -> String {
    if name.contains('/') {
        name.to_owned()
    } else {
        qualify(module, name)
    }
}

fn noncanonical_root(analysis: &mut UnitAnalysis, message: String) {
    analysis.diagnostics.push(super::super::source::Diagnostic {
        code: "production/noncanonical-native-root".into(),
        operation: "native-root".into(),
        module: analysis.module.clone(),
        location: analysis.location.clone(),
        message,
    });
}

#[cfg(test)]
mod tests {
    use super::super::super::source::SourceLocation;
    use super::super::NativeRootInventory;
    use super::*;
    use std::collections::BTreeSet;

    fn analysis(source: &str) -> UnitAnalysis {
        UnitAnalysis {
            id: "demo.core:00000:000".into(),
            module: "demo.core".into(),
            index: 0,
            form_source: source.into(),
            kind: UnitKind::Registration,
            effect: Effect::Unknown,
            location: SourceLocation {
                path: "src/demo/core.hal".into(),
                line: 1,
                column: 1,
                end_line: 1,
                end_column: 1,
            },
            provides: BTreeSet::new(),
            runtime_edges: BTreeSet::new(),
            compile_time_edges: BTreeSet::new(),
            namespace_edges: BTreeSet::new(),
            native_roots: NativeRootInventory::default(),
            native_primitives: BTreeSet::new(),
            native_types: BTreeSet::new(),
            native_protocols: BTreeSet::new(),
            diagnostics: Vec::new(),
        }
    }

    #[test]
    fn declaration_roots_use_canonical_var_identities() {
        let mut protocol = analysis("(defprotocol Greeter [greet [self]])");
        scan_declaration_roots(&mut protocol);
        assert_eq!(
            protocol.native_roots.protocols,
            BTreeSet::from(["demo.core/Greeter".into()])
        );
        assert!(!protocol
            .native_protocols
            .iter()
            .any(|root| root.starts_with("declaration:")));

        let mut multimethod = analysis("(defmethod render :text [value] value)");
        scan_declaration_roots(&mut multimethod);
        assert_eq!(
            multimethod.native_roots.multimethods,
            BTreeSet::from(["demo.core/render".into()])
        );
        assert!(!multimethod
            .native_protocols
            .iter()
            .any(|root| root.starts_with("multimethod:") && root.ends_with(":0")));
    }

    #[test]
    fn native_global_edges_separate_type_and_method_roots() {
        let mut unit = analysis("(def value nil)");
        classify_native_edge("std.native.String/slice", &mut unit);
        classify_native_edge("std.protocol.icount.ICount/count", &mut unit);
        assert!(unit.native_roots.types.contains("std.native.String"));
        assert!(unit
            .native_roots
            .methods
            .contains("std.native.String/slice"));
        assert!(unit.native_roots.protocols.contains("std.protocol.icount"));
        assert!(unit
            .native_roots
            .protocol_methods
            .contains("std.protocol.icount.ICount/count"));
    }
}
