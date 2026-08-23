//! Deterministic Var-level production reachability and HBX emission.
//!
//! Production analysis parses and expands complete modules before making any
//! removal decision. Emission then compiles only the retained runtime units
//! into the existing HBX0 container and validates the result through the
//! minimal `Runtime::core()` loading path.

#[path = "production/bundle.rs"]
mod bundle;
#[path = "production/graph.rs"]
mod graph;
#[path = "production/plan.rs"]
mod plan;
#[path = "production/report.rs"]
mod report;
#[path = "production/source.rs"]
mod source;
#[path = "production/unit.rs"]
mod unit;

#[cfg(test)]
#[path = "production/tests.rs"]
mod tests;

pub use graph::{Analysis, AnalysisOutput, BuildOutput, ModuleAnalysis, RetentionReason};
pub use plan::BuildPlan;
pub use report::{ANALYSIS_FORMAT, SHAKE_FORMAT};

use crate::compiled_product::{CompiledProduct, CompiledProductKind};
use crate::core::Value;
use crate::kernel::{Form, GeneratedNamespaceConfig};
use crate::lang::data::Symbol;
use crate::project::Project;
use crate::Runtime;
use bundle::ProductionBuild;
use graph::finish_analysis;
use sha2::{Digest, Sha256};
use source::{
    aggregate_digest, collect_embedded_modules, collect_project_modules,
    deterministic_module_order, Diagnostic, SourceLocation, SourceModule,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::{Component, Path, PathBuf};
use unit::{Effect, UnitAnalysis};

const IMPLICIT_FOUNDATION_NAMESPACES: &[&str] = &[
    "std.foundation",
    "std.foundation.bytes",
    "std.foundation.coroutine",
    "std.foundation.pretty",
    "std.foundation.promise",
    "std.foundation.string",
];

pub fn analyze(project: &Project, plan_source: &str) -> Result<Analysis, String> {
    let plan = BuildPlan::parse(plan_source)?;
    let modules = collect_selected_modules(project, &plan)?;
    analyze_modules(&plan, modules)
}

/// Compiles one deterministic production project into the existing HBX0
/// package without writing files. The returned immutable product is suitable
/// for an embedding cache or an artifact-backed LiveSession.
pub fn compile_hbc_package_product(
    project: &Project,
    plan_source: &str,
) -> Result<CompiledProduct, String> {
    let plan = BuildPlan::parse(plan_source)?;
    let modules = collect_selected_modules(project, &plan)?;
    let analysis = analyze_modules(&plan, modules)?;
    if !analysis.succeeded() {
        return Err(format!(
            "production analysis failed with {} diagnostic(s)",
            analysis.diagnostics.len()
        ));
    }
    let build = ProductionBuild {
        plan: plan.clone(),
        analysis: analysis.clone(),
    };
    let compiled = bundle::compile::compile(&build)?;
    bundle::load::validate_bundle(&compiled.bytes, &plan.entrypoints)?;
    let module_digests = analysis
        .modules
        .iter()
        .map(|module| module.digest.clone())
        .collect();
    Ok(CompiledProduct::new(
        CompiledProductKind::HbcPackage,
        analysis.input_digest,
        module_digests,
        format!("hara-production/{}", env!("CARGO_PKG_VERSION")),
        "hbx0",
        format!("{plan:?}"),
        compiled.bytes,
    ))
}

pub fn analyze_and_write(project: &Project, plan_source: &str) -> Result<AnalysisOutput, String> {
    let plan = BuildPlan::parse(plan_source)?;
    let modules = collect_selected_modules(project, &plan)?;
    let analysis = analyze_modules(&plan, modules)?;
    let report_source = report::report_source(&plan, &analysis, None);
    let report_path = safe_output_path(&project.root, &plan.output_report)?;
    write_output(&report_path, report_source.as_bytes())?;
    Ok(AnalysisOutput {
        analysis,
        report_path,
        report_source,
    })
}

pub fn build_and_write(project: &Project, plan_source: &str) -> Result<BuildOutput, String> {
    let plan = BuildPlan::parse(plan_source)?;
    let modules = collect_selected_modules(project, &plan)?;
    let mut analysis = analyze_modules(&plan, modules)?;
    let bundle_path = safe_output_path(&project.root, &plan.output_bundle)?;
    let report_path = safe_output_path(&project.root, &plan.output_report)?;
    if bundle_path == report_path {
        return Err("production bundle and shake report paths must be distinct".into());
    }

    let mut compiled = None;
    if analysis.succeeded() {
        let build = ProductionBuild {
            plan: plan.clone(),
            analysis: analysis.clone(),
        };
        match bundle::compile::compile(&build) {
            Ok(candidate) => compiled = Some(candidate),
            Err(error) => push_bundle_diagnostic(
                &mut analysis,
                &plan,
                "production/bundle-compile-failed",
                "bundle-compile",
                error,
            ),
        }
    }
    if let Some(candidate) = compiled.take() {
        match bundle::load::validate_bundle(&candidate.bytes, &plan.entrypoints) {
            Ok(_) => compiled = Some(candidate),
            Err(error) => push_bundle_diagnostic(
                &mut analysis,
                &plan,
                "production/bundle-load-failed",
                "bundle-load",
                error,
            ),
        }
    }

    let Some(compiled) = compiled else {
        remove_stale_bundle(&bundle_path)?;
        let report_source = report::report_source(&plan, &analysis, None);
        write_output(&report_path, report_source.as_bytes())?;
        return Ok(BuildOutput {
            analysis,
            bundle_path: None,
            report_path,
            report_source,
        });
    };

    let summary = report::BundleSummary {
        output_bytes: compiled.bytes.len(),
        output_digest: sha256_hex(&compiled.bytes),
        module_count: compiled.modules.len(),
    };
    let report_source = report::report_source(&plan, &analysis, Some(&summary));
    write_output(&bundle_path, &compiled.bytes)?;
    if let Err(error) = write_output(&report_path, report_source.as_bytes()) {
        let _ = fs::remove_file(&bundle_path);
        return Err(error);
    }
    Ok(BuildOutput {
        analysis,
        bundle_path: Some(bundle_path),
        report_path,
        report_source,
    })
}

fn collect_selected_modules(
    project: &Project,
    plan: &BuildPlan,
) -> Result<Vec<SourceModule>, String> {
    let project_modules = collect_project_modules(project)?;
    let embedded_modules = collect_embedded_modules()?;
    let project_names = project_modules
        .iter()
        .map(|module| module.name.clone())
        .collect::<BTreeSet<_>>();
    let mut catalogue = embedded_modules
        .into_iter()
        .map(|module| (module.name.clone(), module))
        .collect::<BTreeMap<_, _>>();
    for module in project_modules {
        if catalogue
            .insert(module.name.clone(), module.clone())
            .is_some()
        {
            return Err(format!(
                "project namespace shadows an embedded production module: {}",
                module.name
            ));
        }
    }

    let mut selected = project_names;
    for root in plan
        .entrypoints
        .iter()
        .chain(plan.keep_vars.iter())
        .filter_map(|value| value.split_once('/').map(|(namespace, _)| namespace))
    {
        selected.insert(root.into());
    }
    selected.extend(plan.keep_namespaces.iter().cloned());
    for namespace in IMPLICIT_FOUNDATION_NAMESPACES {
        if catalogue.contains_key(*namespace) {
            selected.insert((*namespace).into());
        }
    }

    let mut pending = selected.clone();
    while let Some(namespace) = pending.pop_first() {
        let Some(module) = catalogue.get(&namespace) else {
            continue;
        };
        for dependency in &module.dependencies {
            if catalogue.contains_key(dependency) && selected.insert(dependency.clone()) {
                pending.insert(dependency.clone());
            }
        }
    }

    let mut modules = selected
        .into_iter()
        .filter_map(|name| catalogue.remove(&name))
        .collect::<Vec<_>>();
    modules.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(modules)
}

fn analyze_modules(plan: &BuildPlan, mut modules: Vec<SourceModule>) -> Result<Analysis, String> {
    modules.sort_by(|left, right| left.name.cmp(&right.name));
    let (input_bytes, input_digest) = aggregate_digest(&modules);
    let order = deterministic_module_order(&modules);
    let mut runtime = Runtime::new();
    prepare_runtime(&mut runtime, &modules)?;

    let mut analyzed_modules = Vec::with_capacity(modules.len());
    let mut units = Vec::new();
    for module_index in order {
        let module = &modules[module_index];
        runtime.use_namespace(&module.name);
        let mut unit_ids = Vec::new();
        for (index, form) in module.forms.iter().enumerate() {
            let location = unit::source_location(&module.path, module.body_line_base, &form.span);
            let config = runtime
                .generated_configs
                .get(&module.name)
                .cloned()
                .unwrap_or_else(GeneratedNamespaceConfig::defaults);
            let seeds = match unit::expand_top_level(
                &runtime,
                &config,
                &module.name,
                index,
                &form.form,
                location.clone(),
            ) {
                Ok(seeds) => seeds,
                Err(error) => {
                    let analysis =
                        failed_expansion_unit(module, index, &form.form, location, error);
                    unit_ids.push(analysis.id.clone());
                    units.push(analysis);
                    continue;
                }
            };
            for seed in seeds {
                predeclare_vars(&runtime, unit::raw_provided_vars(&seed.form, &seed.module));
                let mut compiled = unit::analyze_unit(&runtime, seed, plan);
                if let Err(error) = unit::execute_compile_time_unit(&mut runtime, &compiled) {
                    compiled.analysis.diagnostics.push(Diagnostic {
                        code: "production/compile-time-unit-failed".into(),
                        operation: "compile-time-execute".into(),
                        module: compiled.analysis.module.clone(),
                        location: compiled.analysis.location.clone(),
                        message: error,
                    });
                }
                unit_ids.push(compiled.analysis.id.clone());
                units.push(compiled.analysis);
            }
        }
        analyzed_modules.push(ModuleAnalysis {
            name: module.name.clone(),
            path: module.path.clone(),
            namespace_form: module.namespace_form.clone(),
            digest: module.digest.clone(),
            input_bytes: module.source.len(),
            dependencies: module.dependencies.clone(),
            unit_ids,
            standard_library: module.standard_library,
        });
    }
    analyzed_modules.sort_by(|left, right| left.name.cmp(&right.name));
    units.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(finish_analysis(
        plan,
        analyzed_modules,
        units,
        input_bytes,
        input_digest,
    ))
}

fn prepare_runtime(runtime: &mut Runtime, modules: &[SourceModule]) -> Result<(), String> {
    let available = modules
        .iter()
        .map(|module| module.name.clone())
        .collect::<BTreeSet<_>>();
    for module in modules {
        runtime.register_resource(&module.name, &module.source);
        runtime.namespace_registry.find_or_create(&module.name);
    }
    for module in modules {
        for form in &module.forms {
            predeclare_vars(runtime, unit::raw_provided_vars(&form.form, &module.name));
        }
    }
    for module in modules {
        let namespace_form = crate::kernel::parse(&module.namespace_form)
            .map_err(|error| format!("{}: {error}", module.path))?;
        let Form::List(values) = without_metadata(&namespace_form) else {
            return Err(format!(
                "{}: namespace declaration must be a list",
                module.path
            ));
        };
        let config = GeneratedNamespaceConfig::configure_with(&values[2..], |target| {
            available.contains(target)
                || runtime.namespace_registry.find(target).is_some()
                || runtime.resources.contains_key(target)
        })?;
        runtime
            .generated_configs
            .insert(module.name.clone(), config);
    }
    Ok(())
}

fn predeclare_vars(runtime: &Runtime, vars: BTreeSet<String>) {
    for qualified in vars {
        let Some((namespace, name)) = qualified.rsplit_once('/') else {
            continue;
        };
        let namespace = runtime.namespace_registry.find_or_create(namespace);
        let local = Symbol::create(None, name);
        if namespace.resolve(&local).is_none() {
            namespace.intern(name, Value::Nil);
        }
    }
}

fn failed_expansion_unit(
    module: &SourceModule,
    index: usize,
    form: &Form,
    location: SourceLocation,
    error: String,
) -> UnitAnalysis {
    UnitAnalysis {
        id: format!("{}:{index:05}:000", module.name),
        module: module.name.clone(),
        index: index * 1000,
        form_source: form.to_string(),
        kind: unit::classify_unit_kind(form),
        effect: Effect::Unknown,
        location: location.clone(),
        provides: unit::raw_provided_vars(form, &module.name),
        runtime_edges: BTreeSet::new(),
        compile_time_edges: BTreeSet::new(),
        namespace_edges: BTreeSet::new(),
        native_roots: Default::default(),
        native_primitives: BTreeSet::new(),
        native_types: BTreeSet::new(),
        native_protocols: BTreeSet::new(),
        diagnostics: vec![Diagnostic {
            code: "production/macroexpand-failed".into(),
            operation: "macroexpand".into(),
            module: module.name.clone(),
            location,
            message: error,
        }],
    }
}

fn push_bundle_diagnostic(
    analysis: &mut Analysis,
    plan: &BuildPlan,
    code: &str,
    operation: &str,
    message: String,
) {
    analysis.diagnostics.push(Diagnostic {
        code: code.into(),
        operation: operation.into(),
        module: plan.main.clone(),
        location: SourceLocation {
            path: "project.edn".into(),
            line: 1,
            column: 1,
            end_line: 1,
            end_column: 1,
        },
        message,
    });
}

fn write_output(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    }
    fs::write(path, bytes).map_err(|error| format!("cannot write {}: {error}", path.display()))
}

fn remove_stale_bundle(path: &Path) -> Result<(), String> {
    if path.exists() {
        fs::remove_file(path)
            .map_err(|error| format!("cannot remove stale {}: {error}", path.display()))?;
    }
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(&mut output, "{byte:02x}").expect("writing to a String cannot fail");
    }
    output
}

fn safe_output_path(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let relative = Path::new(relative);
    if relative.as_os_str().is_empty()
        || relative.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err("production output path must remain inside the project root".into());
    }
    Ok(root.join(relative))
}

fn without_metadata(form: &Form) -> &Form {
    match form {
        Form::Metadata(_, value) => without_metadata(value),
        value => value,
    }
}
