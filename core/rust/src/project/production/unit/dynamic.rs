use super::super::plan::BuildPlan;
use super::super::source::Diagnostic;
use super::{qualify, without_metadata, UnitAnalysis};
use crate::kernel::Form;
use crate::lang::data::Symbol;
use crate::Runtime;
use std::collections::BTreeSet;

const DYNAMIC_OPERATIONS: &[&str] = &[
    "resolve",
    "var",
    "require",
    "load-string",
    "eval",
    "eval-in-ns",
];

pub(super) fn collect_resolved_symbols(
    runtime: &Runtime,
    module: &str,
    form: &Form,
    output: &mut BTreeSet<String>,
) {
    match form {
        Form::Symbol(name) => {
            if let Some(resolved) = resolve_existing_symbol(runtime, module, name) {
                output.insert(resolved);
            }
        }
        Form::List(values) => {
            if matches!(values.first(), Some(Form::Symbol(head)) if head == "quote") {
                return;
            }
            for value in values {
                collect_resolved_symbols(runtime, module, value, output);
            }
        }
        Form::Vector(values) | Form::Set(values) => {
            for value in values {
                collect_resolved_symbols(runtime, module, value, output);
            }
        }
        Form::Map(entries) => {
            for (key, value) in entries {
                collect_resolved_symbols(runtime, module, key, output);
                collect_resolved_symbols(runtime, module, value, output);
            }
        }
        Form::Metadata(_, value) | Form::Tagged(_, value) => {
            collect_resolved_symbols(runtime, module, value, output);
        }
        _ => {}
    }
}

pub(super) fn scan_dynamic_access(
    runtime: &Runtime,
    module: &str,
    form: &Form,
    plan: &BuildPlan,
    analysis: &mut UnitAnalysis,
) {
    scan_form(runtime, module, form, plan, &BTreeSet::new(), analysis);
}

fn scan_form(
    runtime: &Runtime,
    module: &str,
    form: &Form,
    plan: &BuildPlan,
    bound: &BTreeSet<String>,
    analysis: &mut UnitAnalysis,
) {
    match without_metadata(form) {
        Form::List(values) => scan_list(runtime, module, values, plan, bound, analysis),
        Form::Vector(values) | Form::Set(values) => {
            scan_forms(runtime, module, values, plan, bound, analysis)
        }
        Form::Map(entries) => {
            for (key, value) in entries {
                scan_form(runtime, module, key, plan, bound, analysis);
                scan_form(runtime, module, value, plan, bound, analysis);
            }
        }
        Form::Tagged(_, value) => scan_form(runtime, module, value, plan, bound, analysis),
        _ => {}
    }
}

fn scan_list(
    runtime: &Runtime,
    module: &str,
    values: &[Form],
    plan: &BuildPlan,
    bound: &BTreeSet<String>,
    analysis: &mut UnitAnalysis,
) {
    let Some(Form::Symbol(operator)) = values.first().map(without_metadata) else {
        scan_forms(runtime, module, values, plan, bound, analysis);
        return;
    };
    let operation = local_name(operator);
    if operation == "quote" || operation == "syntax-quote" {
        return;
    }
    if scan_lexical_form(runtime, module, operation, values, plan, bound, analysis) {
        return;
    }
    if let Some(operation) = dynamic_operation(runtime, module, operator, bound) {
        scan_dynamic_call(runtime, module, operation, values, plan, bound, analysis);
    }
    scan_forms(runtime, module, &values[1..], plan, bound, analysis);
}

fn scan_lexical_form(
    runtime: &Runtime,
    module: &str,
    operation: &str,
    values: &[Form],
    plan: &BuildPlan,
    bound: &BTreeSet<String>,
    analysis: &mut UnitAnalysis,
) -> bool {
    match operation {
        "fn" => {
            scan_function(runtime, module, &values[1..], plan, bound, analysis, true);
            true
        }
        "defn" | "defmacro" => {
            scan_function(runtime, module, &values[1..], plan, bound, analysis, false);
            true
        }
        "let" | "loop" | "binding" | "with-open" => {
            scan_binding_body(runtime, module, values, plan, bound, analysis);
            true
        }
        "letfn" => {
            scan_letfn(runtime, module, values, plan, bound, analysis);
            true
        }
        "if-let" | "if-some" => {
            scan_conditional(runtime, module, values, plan, bound, analysis, true);
            true
        }
        "when-let" | "when-some" | "when-first" => {
            scan_conditional(runtime, module, values, plan, bound, analysis, false);
            true
        }
        "for" | "doseq" => {
            scan_comprehension(runtime, module, values, plan, bound, analysis);
            true
        }
        "catch" => {
            scan_catch(runtime, module, values, plan, bound, analysis);
            true
        }
        _ => false,
    }
}

fn scan_function(
    runtime: &Runtime,
    module: &str,
    values: &[Form],
    plan: &BuildPlan,
    bound: &BTreeSet<String>,
    analysis: &mut UnitAnalysis,
    named_local: bool,
) {
    let mut offset = 0usize;
    let mut outer = bound.clone();
    if named_local {
        if let Some(Form::Symbol(name)) = values.first().map(without_metadata) {
            bind_symbol(name, &mut outer);
            offset = 1;
        }
    } else if !values.is_empty() {
        offset = 1;
        while matches!(
            values.get(offset).map(without_metadata),
            Some(Form::String(_) | Form::Map(_))
        ) {
            offset += 1;
        }
    }
    let Some(signature) = values.get(offset) else {
        return;
    };
    match without_metadata(signature) {
        Form::Vector(_) => {
            let mut scope = outer;
            collect_pattern_bindings(signature, &mut scope);
            scan_forms(
                runtime,
                module,
                &values[offset + 1..],
                plan,
                &scope,
                analysis,
            );
        }
        Form::List(_) => {
            for clause in &values[offset..] {
                scan_function_clause(runtime, module, clause, plan, &outer, analysis);
            }
        }
        _ => scan_forms(runtime, module, &values[offset..], plan, &outer, analysis),
    }
}

fn scan_function_clause(
    runtime: &Runtime,
    module: &str,
    clause: &Form,
    plan: &BuildPlan,
    bound: &BTreeSet<String>,
    analysis: &mut UnitAnalysis,
) {
    let Form::List(values) = without_metadata(clause) else {
        scan_form(runtime, module, clause, plan, bound, analysis);
        return;
    };
    let Some(parameters) = values.first() else {
        return;
    };
    let mut scope = bound.clone();
    collect_pattern_bindings(parameters, &mut scope);
    scan_forms(runtime, module, &values[1..], plan, &scope, analysis);
}

fn scan_binding_body(
    runtime: &Runtime,
    module: &str,
    values: &[Form],
    plan: &BuildPlan,
    bound: &BTreeSet<String>,
    analysis: &mut UnitAnalysis,
) {
    let Some(bindings) = values.get(1) else {
        return;
    };
    let scope = scan_sequential_bindings(runtime, module, bindings, plan, bound, analysis);
    scan_forms(runtime, module, &values[2..], plan, &scope, analysis);
}

fn scan_sequential_bindings(
    runtime: &Runtime,
    module: &str,
    bindings: &Form,
    plan: &BuildPlan,
    bound: &BTreeSet<String>,
    analysis: &mut UnitAnalysis,
) -> BTreeSet<String> {
    let Form::Vector(values) = without_metadata(bindings) else {
        scan_form(runtime, module, bindings, plan, bound, analysis);
        return bound.clone();
    };
    let mut scope = bound.clone();
    for pair in values.chunks(2) {
        if let Some(initializer) = pair.get(1) {
            scan_form(runtime, module, initializer, plan, &scope, analysis);
            collect_pattern_bindings(&pair[0], &mut scope);
        }
    }
    scope
}

fn scan_letfn(
    runtime: &Runtime,
    module: &str,
    values: &[Form],
    plan: &BuildPlan,
    bound: &BTreeSet<String>,
    analysis: &mut UnitAnalysis,
) {
    let Some(Form::Vector(bindings)) = values.get(1).map(without_metadata) else {
        scan_forms(runtime, module, &values[1..], plan, bound, analysis);
        return;
    };
    let mut scope = bound.clone();
    for pair in bindings.chunks(2) {
        if let Some(name) = pair.first() {
            collect_pattern_bindings(name, &mut scope);
        }
    }
    for pair in bindings.chunks(2) {
        if let Some(value) = pair.get(1) {
            scan_form(runtime, module, value, plan, &scope, analysis);
        }
    }
    scan_forms(runtime, module, &values[2..], plan, &scope, analysis);
}

fn scan_conditional(
    runtime: &Runtime,
    module: &str,
    values: &[Form],
    plan: &BuildPlan,
    bound: &BTreeSet<String>,
    analysis: &mut UnitAnalysis,
    has_else: bool,
) {
    let Some(Form::Vector(binding)) = values.get(1).map(without_metadata) else {
        scan_forms(runtime, module, &values[1..], plan, bound, analysis);
        return;
    };
    if let Some(initializer) = binding.get(1) {
        scan_form(runtime, module, initializer, plan, bound, analysis);
    }
    let mut scope = bound.clone();
    if let Some(pattern) = binding.first() {
        collect_pattern_bindings(pattern, &mut scope);
    }
    if let Some(then_form) = values.get(2) {
        scan_form(runtime, module, then_form, plan, &scope, analysis);
    }
    if has_else {
        if let Some(else_form) = values.get(3) {
            scan_form(runtime, module, else_form, plan, bound, analysis);
        }
    } else {
        scan_forms(runtime, module, &values[3..], plan, &scope, analysis);
    }
}

fn scan_comprehension(
    runtime: &Runtime,
    module: &str,
    values: &[Form],
    plan: &BuildPlan,
    bound: &BTreeSet<String>,
    analysis: &mut UnitAnalysis,
) {
    let Some(Form::Vector(bindings)) = values.get(1).map(without_metadata) else {
        scan_forms(runtime, module, &values[1..], plan, bound, analysis);
        return;
    };
    let mut scope = bound.clone();
    let mut index = 0usize;
    while index + 1 < bindings.len() {
        match without_metadata(&bindings[index]) {
            Form::Keyword(keyword) if keyword == "let" => {
                scope = scan_sequential_bindings(
                    runtime,
                    module,
                    &bindings[index + 1],
                    plan,
                    &scope,
                    analysis,
                );
            }
            Form::Keyword(keyword) if keyword == "when" || keyword == "while" => {
                scan_form(
                    runtime,
                    module,
                    &bindings[index + 1],
                    plan,
                    &scope,
                    analysis,
                );
            }
            pattern => {
                scan_form(
                    runtime,
                    module,
                    &bindings[index + 1],
                    plan,
                    &scope,
                    analysis,
                );
                collect_pattern_bindings(pattern, &mut scope);
            }
        }
        index += 2;
    }
    scan_forms(runtime, module, &values[2..], plan, &scope, analysis);
}

fn scan_catch(
    runtime: &Runtime,
    module: &str,
    values: &[Form],
    plan: &BuildPlan,
    bound: &BTreeSet<String>,
    analysis: &mut UnitAnalysis,
) {
    if let Some(error_type) = values.get(1) {
        scan_form(runtime, module, error_type, plan, bound, analysis);
    }
    let mut scope = bound.clone();
    if let Some(binding) = values.get(2) {
        collect_pattern_bindings(binding, &mut scope);
    }
    scan_forms(runtime, module, &values[3..], plan, &scope, analysis);
}

fn scan_forms(
    runtime: &Runtime,
    module: &str,
    forms: &[Form],
    plan: &BuildPlan,
    bound: &BTreeSet<String>,
    analysis: &mut UnitAnalysis,
) {
    for form in forms {
        scan_form(runtime, module, form, plan, bound, analysis);
    }
}

fn collect_pattern_bindings(form: &Form, output: &mut BTreeSet<String>) {
    match without_metadata(form) {
        Form::Symbol(name) => bind_symbol(name, output),
        Form::Vector(values) => {
            for value in values {
                if !matches!(without_metadata(value), Form::Symbol(name) if name == "&")
                    && !matches!(without_metadata(value), Form::Keyword(name) if name == "as")
                {
                    collect_pattern_bindings(value, output);
                }
            }
        }
        Form::Map(entries) => {
            for (key, value) in entries {
                match without_metadata(key) {
                    Form::Keyword(keyword)
                        if keyword == "keys" || keyword == "syms" || keyword == "strs" =>
                    {
                        collect_pattern_bindings(value, output)
                    }
                    Form::Keyword(keyword) if keyword == "as" => {
                        collect_pattern_bindings(value, output)
                    }
                    Form::Keyword(keyword) if keyword == "or" => {
                        if let Form::Map(defaults) = without_metadata(value) {
                            for (name, _) in defaults {
                                collect_pattern_bindings(name, output);
                            }
                        }
                    }
                    Form::Keyword(_) => collect_pattern_bindings(value, output),
                    _ => collect_pattern_bindings(key, output),
                }
            }
        }
        _ => {}
    }
}

fn bind_symbol(name: &str, output: &mut BTreeSet<String>) {
    if name != "_" && name != "&" && !name.contains('/') {
        output.insert(name.into());
    }
}

fn dynamic_operation<'a>(
    runtime: &Runtime,
    module: &str,
    operator: &'a str,
    bound: &BTreeSet<String>,
) -> Option<&'a str> {
    let operation = local_name(operator);
    if !DYNAMIC_OPERATIONS.contains(&operation) {
        return None;
    }
    if !operator.contains('/') && bound.contains(operator) {
        return None;
    }
    match resolve_existing_symbol(runtime, module, operator) {
        Some(resolved) if resolved == format!("std.foundation/{operation}") => Some(operation),
        Some(_) => None,
        None if !operator.contains('/') => Some(operation),
        None if operator == format!("std.foundation/{operation}") => Some(operation),
        None => None,
    }
}

fn scan_dynamic_call(
    runtime: &Runtime,
    module: &str,
    operation: &str,
    values: &[Form],
    plan: &BuildPlan,
    bound: &BTreeSet<String>,
    analysis: &mut UnitAnalysis,
) {
    match operation {
        "resolve" => {
            if let Some(name) = values.get(1).and_then(quoted_symbol) {
                analysis
                    .runtime_edges
                    .insert(canonical_symbol(runtime, module, name));
            } else if unbounded_vars(plan) {
                push_dynamic_diagnostic(analysis, "unbounded-dynamic-var", operation);
            }
        }
        "var" => {
            if let Some(name) = values.get(1).and_then(symbol_literal) {
                analysis
                    .runtime_edges
                    .insert(canonical_symbol(runtime, module, name));
            } else if unbounded_vars(plan) {
                push_dynamic_diagnostic(analysis, "unbounded-dynamic-var", operation);
            }
        }
        "require" => {
            let mut literal = values.len() > 1;
            for value in &values[1..] {
                if let Some(namespace) = require_namespace(value) {
                    analysis.namespace_edges.insert(namespace.into());
                } else {
                    literal = false;
                }
            }
            if !literal && plan.keep_namespaces.is_empty() {
                push_dynamic_diagnostic(analysis, "unbounded-dynamic-namespace", operation);
            }
        }
        "load-string" => {
            if let Some(Form::String(source)) = values.get(1).map(without_metadata) {
                match crate::kernel::parse_forms(source) {
                    Ok(forms) => {
                        for loaded in forms {
                            collect_code_symbols(runtime, module, &loaded, analysis);
                            scan_form(runtime, module, &loaded, plan, bound, analysis);
                        }
                    }
                    Err(error) => push_diagnostic(
                        analysis,
                        "production/invalid-constant-load-string",
                        operation,
                        error,
                    ),
                }
            } else if unbounded_vars(plan) {
                push_dynamic_diagnostic(analysis, "unbounded-generated-source", operation);
            }
        }
        "eval" => {
            if let Some(code) = values.get(1).and_then(quoted_value) {
                collect_code_symbols(runtime, module, code, analysis);
                scan_form(runtime, module, code, plan, bound, analysis);
            } else if unbounded_vars(plan) {
                push_dynamic_diagnostic(analysis, "unbounded-eval", operation);
            }
        }
        "eval-in-ns" => {
            let target = values.get(1).and_then(namespace_literal);
            let code = values.get(2).and_then(quoted_value);
            match (target, code) {
                (Some(target), Some(code)) => {
                    analysis.namespace_edges.insert(target.into());
                    collect_code_symbols(runtime, target, code, analysis);
                    scan_form(runtime, target, code, plan, &BTreeSet::new(), analysis);
                }
                _ if unbounded_vars(plan) => {
                    push_dynamic_diagnostic(analysis, "unbounded-eval-in-ns", operation)
                }
                _ => {}
            }
        }
        _ => {}
    }
}

fn unbounded_vars(plan: &BuildPlan) -> bool {
    plan.keep_vars.is_empty() && plan.keep_namespaces.is_empty()
}

fn collect_code_symbols(runtime: &Runtime, module: &str, form: &Form, analysis: &mut UnitAnalysis) {
    match form {
        Form::Symbol(name) => {
            if let Some(resolved) = resolve_existing_symbol(runtime, module, name) {
                analysis.runtime_edges.insert(resolved);
            }
        }
        Form::List(values) | Form::Vector(values) | Form::Set(values) => {
            for value in values {
                collect_code_symbols(runtime, module, value, analysis);
            }
        }
        Form::Map(entries) => {
            for (key, value) in entries {
                collect_code_symbols(runtime, module, key, analysis);
                collect_code_symbols(runtime, module, value, analysis);
            }
        }
        Form::Metadata(_, value) | Form::Tagged(_, value) => {
            collect_code_symbols(runtime, module, value, analysis)
        }
        _ => {}
    }
}

fn push_dynamic_diagnostic(analysis: &mut UnitAnalysis, code: &str, operation: &str) {
    push_diagnostic(
        analysis,
        &format!("production/{code}"),
        operation,
        format!(
            "reachable non-literal {operation} is not bounded by :build/keep-vars or :build/keep-namespaces"
        ),
    );
}

fn push_diagnostic(analysis: &mut UnitAnalysis, code: &str, operation: &str, message: String) {
    if analysis.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == code
            && diagnostic.operation == operation
            && diagnostic.location == analysis.location
    }) {
        return;
    }
    analysis.diagnostics.push(Diagnostic {
        code: code.into(),
        operation: operation.into(),
        module: analysis.module.clone(),
        location: analysis.location.clone(),
        message,
    });
}

pub(super) fn canonical_symbol(runtime: &Runtime, module: &str, name: &str) -> String {
    resolve_existing_symbol(runtime, module, name).unwrap_or_else(|| {
        if name.contains('/') {
            name.into()
        } else {
            qualify(module, name)
        }
    })
}

fn resolve_existing_symbol(runtime: &Runtime, module: &str, name: &str) -> Option<String> {
    let namespace = runtime.namespace_registry.find(module)?;
    let symbol = Symbol::parse(name);
    let resolved = if name.contains('/') {
        runtime
            .namespace_registry
            .resolve(&symbol)
            .or_else(|| namespace.resolve(&symbol))
    } else {
        namespace.resolve(&symbol)
    }?;
    Some(resolved.symbol().as_str().to_owned())
}

fn symbol_literal(form: &Form) -> Option<&str> {
    match without_metadata(form) {
        Form::Symbol(value) => Some(value),
        value => quoted_symbol(value),
    }
}

fn quoted_symbol(form: &Form) -> Option<&str> {
    match quoted_value(form).map(without_metadata) {
        Some(Form::Symbol(value)) => Some(value),
        _ => None,
    }
}

fn namespace_literal(form: &Form) -> Option<&str> {
    match without_metadata(form) {
        Form::String(value) => Some(value),
        value => quoted_symbol(value),
    }
}

fn require_namespace(form: &Form) -> Option<&str> {
    match without_metadata(form) {
        Form::Vector(values) => match values.first().map(without_metadata) {
            Some(Form::Symbol(value)) => Some(value),
            _ => None,
        },
        value => namespace_literal(value),
    }
}

fn quoted_value(form: &Form) -> Option<&Form> {
    match without_metadata(form) {
        Form::List(values)
            if values.len() == 2
                && matches!(values.first(), Some(Form::Symbol(head)) if head == "quote") =>
        {
            values.get(1)
        }
        _ => None,
    }
}

fn local_name(symbol: &str) -> &str {
    symbol.rsplit('/').next().unwrap_or(symbol)
}
