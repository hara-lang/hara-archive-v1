use super::model::{CompiledBundle, ProductionBuild};
use super::{order, render};
use crate::core;
use crate::kernel;
use crate::vm::{encode_bytecode_bundle, BytecodeBundleModule};
use crate::Runtime;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

pub(in crate::task::production) fn compile(
    build: &ProductionBuild,
) -> Result<CompiledBundle, String> {
    let rendered = render::retained_modules(build)?;
    let order = order::module_order(&rendered);
    let mut runtime = Runtime::core();

    let provided = build
        .analysis
        .units
        .iter()
        .filter(|unit| build.analysis.runtime_unit_ids.contains(&unit.id))
        .flat_map(|unit| unit.provides.iter().cloned())
        .collect::<BTreeSet<_>>();
    super::super::predeclare_vars(&runtime, provided);

    // Namespace declarations must never fall back to source loading while the
    // pruned artifacts are being compiled. Every retained provider already has
    // a stable Var, so requires can safely establish aliases before definitions
    // are evaluated in deterministic bundle order.
    for module in &rendered {
        runtime.create_namespace(&module.resource);
        runtime.loaded_resources.insert(module.resource.clone());
    }

    let mut modules = Vec::with_capacity(rendered.len());
    for index in order {
        let module = &rendered[index];
        runtime
            .eval_text(&module.namespace_form)
            .map_err(|error| format!("{}: namespace declaration: {error}", module.resource))?;
        runtime.use_namespace(&module.resource);
        let artifact = runtime
            .compile_bytecode_artifact(&module.body)
            .map_err(|error| format!("{}: bytecode compilation: {error}", module.resource))?;
        core::with_definition_origin(kernel::VarOrigin::HalFallback, || {
            runtime.eval_bytecode_artifact(&artifact)
        })
        .map_err(|error| format!("{}: bytecode execution: {error}", module.resource))?;
        modules.push(BytecodeBundleModule {
            resource: module.resource.clone(),
            namespace_form: module.namespace_form.clone(),
            source_digest: Sha256::digest(module.source.as_bytes()).into(),
            dependencies: module.dependencies.clone(),
            eager: true,
            artifact,
        });
    }
    let bytes = encode_bytecode_bundle(&modules)?;
    Ok(CompiledBundle { bytes, modules })
}
