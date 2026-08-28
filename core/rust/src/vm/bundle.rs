//! Deterministic indexed container for the embedded Foundation bootstrap.

use sha2::{Digest, Sha256};

use crate::{
    core, kernel, Runtime, EAGER_HAL_RESOURCES, EMBEDDED_CLI_RESOURCES, EMBEDDED_HAL_RESOURCES,
};

#[path = "bundle/order.rs"]
mod order;
use order::order_module_sources;

const MAGIC: &[u8; 4] = b"HBX0";

#[derive(Clone, Copy)]
pub struct ModuleSource<'a> {
    pub resource: &'a str,
    pub source: &'a str,
}

/// One validated module in the shared HBX0 container format.
///
/// Products such as Hoplite use this descriptor to package application HBC0
/// artifacts without maintaining a second, subtly different bundle codec.
#[derive(Clone)]
pub struct BytecodeBundleModule {
    pub resource: String,
    pub namespace_form: String,
    pub source_digest: [u8; 32],
    pub dependencies: Vec<String>,
    pub eager: bool,
    pub artifact: Vec<u8>,
}

pub fn embedded_foundation_bootstrap_sources() -> Vec<ModuleSource<'static>> {
    let ordered = std::iter::once("std.foundation")
        .chain(EAGER_HAL_RESOURCES.iter().copied())
        .chain(
            EMBEDDED_HAL_RESOURCES
                .iter()
                .map(|(namespace, _, _)| *namespace)
                .filter(|namespace| {
                    standard_library_namespace(namespace)
                        && *namespace != "std.foundation"
                        && !EAGER_HAL_RESOURCES.contains(namespace)
                }),
        );
    let sources = ordered
        .map(|resource| {
            let source = EMBEDDED_HAL_RESOURCES
                .iter()
                .find_map(|(name, _, source)| (*name == resource).then_some(*source))
                .unwrap_or_else(|| panic!("missing embedded HAL resource: {resource}"));
            ModuleSource { resource, source }
        })
        .collect::<Vec<_>>();
    order_module_sources(&sources)
        .expect("embedded Foundation bootstrap dependencies must be acyclic")
        .into_iter()
        .map(|index| sources[index])
        .collect()
}

/// Returns the embedded CLI/test-support namespace closure in deterministic
/// dependency order. Foundation namespaces are deliberately excluded because
/// they are already supplied by the runtime's Foundation artifact.
pub fn embedded_cli_sources() -> Vec<ModuleSource<'static>> {
    let sources = EMBEDDED_CLI_RESOURCES
        .iter()
        .map(|(resource, _, source)| ModuleSource { resource, source })
        .collect::<Vec<_>>();
    order_module_sources(&sources)
        .expect("embedded CLI bootstrap dependencies must be acyclic")
        .into_iter()
        .map(|index| sources[index])
        .collect()
}

pub fn compile_bytecode_bundle(sources: &[ModuleSource<'_>]) -> Result<Vec<u8>, String> {
    let mut runtime = Runtime::core();
    for &(name, _, source) in EMBEDDED_HAL_RESOURCES {
        runtime.register_resource(name, source);
    }
    compile_bytecode_bundle_with_runtime(&mut runtime, sources, sources)
}

/// Compiles a package with a fully bootstrapped Foundation runtime. `context`
/// is registered for resolving imports and macros, while only `sources` are
/// emitted into the resulting HBX0 bundle.
pub fn compile_package_bytecode_bundle(
    context: &[ModuleSource<'_>],
    sources: &[ModuleSource<'_>],
) -> Result<Vec<u8>, String> {
    let mut runtime = Runtime::new();
    compile_bytecode_bundle_with_runtime(&mut runtime, context, sources)
}

fn compile_bytecode_bundle_with_runtime(
    runtime: &mut Runtime,
    context: &[ModuleSource<'_>],
    sources: &[ModuleSource<'_>],
) -> Result<Vec<u8>, String> {
    for source in context {
        runtime.register_resource(source.resource, source.source);
    }
    let mut encoded = Vec::new();
    for index in order_module_sources(sources)? {
        let source = &sources[index];
        let (namespace_form, body) = split_namespace_form(source.source)?;
        runtime
            .eval_text(namespace_form)
            .map_err(|error| format!("{}: namespace declaration: {error}", source.resource))?;
        // Required modules and macro expansion are allowed to select their
        // own namespaces. Pin compilation to the module being emitted so
        // aliases become canonical globals owned by its declaration.
        runtime.use_namespace(source.resource);
        let artifact = core::with_definition_origin(kernel::VarOrigin::HalFallback, || {
            runtime.compile_bytecode_artifact(body)
        })
        .map_err(|error| format!("{}: bytecode compilation: {error}", source.resource))?;
        core::with_definition_origin(kernel::VarOrigin::HalFallback, || {
            runtime.eval_bytecode_artifact(&artifact)
        })
        .map_err(|error| format!("{}: bytecode execution: {error}", source.resource))?;
        let source_digest: [u8; 32] = Sha256::digest(source.source.as_bytes()).into();
        let dependencies = namespace_dependencies(namespace_form)?;
        let eager =
            source.resource == "std.foundation" || EAGER_HAL_RESOURCES.contains(&source.resource);
        encoded.push(BytecodeBundleModule {
            resource: source.resource.to_owned(),
            namespace_form: namespace_form.to_owned(),
            source_digest,
            dependencies,
            eager,
            artifact,
        });
    }
    encode_bytecode_bundle(&encoded)
}

pub fn compile_embedded_foundation_bootstrap_bundle() -> Result<Vec<u8>, String> {
    compile_bytecode_bundle(&embedded_foundation_bootstrap_sources())
}

/// Compiles the immutable CLI and `code.test` closure against the already
/// bootstrapped Foundation context. The resulting bundle is installed lazily,
/// so a test or CLI process pays only for the namespaces it actually requires.
pub fn compile_embedded_cli_bundle() -> Result<Vec<u8>, String> {
    let foundation = embedded_foundation_bootstrap_sources();
    let cli = embedded_cli_sources();
    let mut context = foundation;
    context.extend(cli.iter().copied());
    compile_package_bytecode_bundle(&context, &cli)
}

/// Compatibility name retained for embedding hosts built against the original
/// standard-library bundle API. The embedded artifact is now Foundation-only.
pub fn compile_embedded_standard_library_bundle() -> Result<Vec<u8>, String> {
    compile_embedded_foundation_bootstrap_bundle()
}

/// Compatibility name retained for callers that previously inspected the
/// embedded standard-library sources.
pub fn embedded_standard_library_sources() -> Vec<ModuleSource<'static>> {
    embedded_foundation_bootstrap_sources()
}

pub fn eval_bytecode_bundle(runtime: &mut Runtime, bytes: &[u8]) -> Result<(), String> {
    let modules = decode(bytes)?;
    let mut names = std::collections::HashSet::with_capacity(modules.len());
    for module in &modules {
        if !names.insert(module.resource.clone()) {
            return Err(format!(
                "duplicate bytecode bundle module: {}",
                module.resource
            ));
        }
    }
    let namespaces_before = runtime.namespace_registry.snapshot();
    let environment_before = runtime.execution.snapshot();
    let macros_before = runtime.macros.borrow().clone();
    let protocols_before = runtime.protocols.snapshot();
    let multimethods_before = core::snapshot_multimethods();
    let resources_before = runtime.bytecode_resources.clone();
    let loaded_before = runtime.loaded_resources.clone();
    let loaded = (|| {
        for module in &modules {
            let source = if let Some(source) = runtime.resources.get(&module.resource) {
                Some(source.clone())
            } else {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    runtime
                        .source_paths
                        .get(&module.resource)
                        .map(|path| {
                            std::fs::read_to_string(path).map_err(|error| {
                                format!("cannot read bundled source {}: {error}", path.display())
                            })
                        })
                        .transpose()?
                }
                #[cfg(target_arch = "wasm32")]
                {
                    None
                }
            };
            let source_is_current = source
                .as_deref()
                .map(|source| {
                    let digest: [u8; 32] = Sha256::digest(source.as_bytes()).into();
                    digest == module.source_digest
                })
                .unwrap_or(true);
            if !source_is_current {
                if module.eager {
                    return Err(format!(
                        "stale eager bytecode bundle module: {}",
                        module.resource
                    ));
                }
                continue;
            }
            runtime.register_bytecode_resource(
                module.resource.clone(),
                module.namespace_form.clone(),
                module.artifact.clone(),
            );
        }
        for module in modules.iter().filter(|module| module.eager) {
            core::with_definition_origin(kernel::VarOrigin::HalFallback, || {
                runtime.load_bytecode_resource(&module.resource).map(|_| ())
            })
            .map_err(|error| format!("{}: {error}", module.resource))?;
            runtime.loaded_resources.insert(module.resource.clone());
        }
        runtime.use_namespace("user");
        Ok(())
    })();
    if let Err(error) = loaded {
        runtime.namespace_registry.restore(namespaces_before);
        runtime.execution.restore(environment_before);
        *runtime.macros.borrow_mut() = macros_before;
        runtime.protocols.restore(protocols_before);
        core::restore_multimethods(multimethods_before);
        runtime.bytecode_resources = resources_before;
        runtime.loaded_resources = loaded_before;
        return Err(error);
    }
    Ok(())
}

/// Transactionally load a fully eager HBX0 application bundle into an
/// embedding host's existing namespace and protocol registries.
///
/// The ordinary [`eval_bytecode_bundle`] API additionally indexes lazy
/// standard-library resources on a [`Runtime`]. Worker hosts such as Hoplite
/// already own their registries and package every application module eagerly,
/// so this narrower entry point preserves that ownership without falling back
/// to source compilation.
pub fn eval_eager_bytecode_bundle_with_registries(
    namespaces: &kernel::NamespaceRegistry<core::Value>,
    protocols: &core::ProtocolRegistry,
    bytes: &[u8],
) -> Result<(), String> {
    let modules = decode(bytes)?;
    if let Some(module) = modules.iter().find(|module| !module.eager) {
        return Err(format!(
            "embedding bundle module must be eager: {}",
            module.resource
        ));
    }
    let mut positions = std::collections::HashMap::with_capacity(modules.len());
    for (index, module) in modules.iter().enumerate() {
        if positions.insert(module.resource.as_str(), index).is_some() {
            return Err(format!(
                "duplicate bytecode bundle module: {}",
                module.resource
            ));
        }
    }
    for (index, module) in modules.iter().enumerate() {
        for dependency in &module.dependencies {
            if positions
                .get(dependency.as_str())
                .is_some_and(|dependency_index| *dependency_index >= index)
            {
                return Err(format!(
                    "{}: bundled dependency must appear first: {dependency}",
                    module.resource
                ));
            }
        }
    }
    let programs = modules
        .iter()
        .map(|module| {
            crate::vm::decode_program(&module.artifact)
                .map(std::rc::Rc::new)
                .map_err(|error| format!("{}: invalid bytecode artifact: {error}", module.resource))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let namespaces_before = namespaces.snapshot();
    let protocols_before = protocols.snapshot();
    let multimethods_before = core::snapshot_multimethods();
    let loaded = (|| {
        for (module, program) in modules.iter().zip(programs) {
            let forms = kernel::parse_forms(&module.namespace_form)
                .map_err(|error| format!("{}: namespace declaration: {error}", module.resource))?;
            if forms.len() != 1 {
                return Err(format!(
                    "{}: bundle namespace declaration must contain exactly one form",
                    module.resource
                ));
            }
            let mut environment = std::collections::HashMap::new();
            core::with_namespace_registry(namespaces, || {
                core::with_protocols(protocols, || core::eval(&forms[0], &mut environment))
            })
            .map_err(|error| format!("{}: namespace declaration: {error}", module.resource))?;
            core::with_namespace_registry(namespaces, || {
                core::with_protocols(protocols, || {
                    crate::vm::execute_program_with_globals(program, namespaces)
                        .map_err(|error| error.to_string())
                })
            })
            .map_err(|error| format!("{}: bytecode execution: {error}", module.resource))?;
        }
        Ok(())
    })();
    if let Err(error) = loaded {
        namespaces.restore(namespaces_before);
        protocols.restore(protocols_before);
        core::restore_multimethods(multimethods_before);
        return Err(error);
    }
    Ok(())
}

/// Encode modules into the deterministic, checksummed HBX0 container shared by
/// the Rust, Truffle/native-image, and embedding runtimes.
pub fn encode_bytecode_bundle(modules: &[BytecodeBundleModule]) -> Result<Vec<u8>, String> {
    let modules = canonical_modules(modules)?;
    let mut payload = Vec::new();
    put_u32(&mut payload, modules.len())?;
    for module in &modules {
        put_bytes(&mut payload, module.resource.as_bytes())?;
        put_bytes(&mut payload, module.namespace_form.as_bytes())?;
        payload.extend_from_slice(&module.source_digest);
        put_u32(&mut payload, module.dependencies.len())?;
        for dependency in &module.dependencies {
            put_bytes(&mut payload, dependency.as_bytes())?;
        }
        payload.push(u8::from(module.eager));
        put_bytes(&mut payload, &module.artifact)?;
    }
    let checksum = Sha256::digest(&payload);
    let mut output = Vec::with_capacity(4 + checksum.len() + payload.len());
    output.extend_from_slice(MAGIC);
    output.extend_from_slice(&checksum);
    output.extend_from_slice(&payload);
    Ok(output)
}

pub fn decode_bytecode_bundle(bytes: &[u8]) -> Result<Vec<BytecodeBundleModule>, String> {
    if bytes.len() < 36 || &bytes[..4] != MAGIC {
        return Err("invalid HBX0 bytecode bundle header".into());
    }
    let payload = &bytes[36..];
    if Sha256::digest(payload)[..] != bytes[4..36] {
        return Err("HBX0 bytecode bundle checksum mismatch".into());
    }
    let mut input = payload;
    let count = take_u32(&mut input)? as usize;
    let mut modules = Vec::with_capacity(count);
    for _ in 0..count {
        let resource = take_string(&mut input)?;
        let namespace_form = take_string(&mut input)?;
        let source_digest = take(&mut input, 32)?.try_into().unwrap();
        let dependency_count = take_u32(&mut input)? as usize;
        let dependencies = (0..dependency_count)
            .map(|_| take_string(&mut input))
            .collect::<Result<Vec<_>, _>>()?;
        let eager = match take(&mut input, 1)?[0] {
            0 => false,
            1 => true,
            _ => return Err("HBX0 bytecode bundle contains invalid eager flag".into()),
        };
        let artifact = take_bytes(&mut input)?.to_vec();
        modules.push(BytecodeBundleModule {
            resource,
            namespace_form,
            source_digest,
            dependencies,
            eager,
            artifact,
        });
    }
    if !input.is_empty() {
        return Err("trailing bytes in HBX0 bytecode bundle".into());
    }
    validate_bundle_modules(&modules)?;
    for module in &modules {
        crate::vm::decode_program(&module.artifact)
            .map_err(|error| format!("{}: invalid HBC0 artifact: {error}", module.resource))?;
    }
    Ok(modules)
}

fn decode(bytes: &[u8]) -> Result<Vec<BytecodeBundleModule>, String> {
    decode_bytecode_bundle(bytes)
}

fn canonical_modules(
    modules: &[BytecodeBundleModule],
) -> Result<Vec<BytecodeBundleModule>, String> {
    let mut by_resource = std::collections::BTreeMap::new();
    for module in modules {
        if by_resource
            .insert(module.resource.clone(), module.clone())
            .is_some()
        {
            return Err(format!("duplicate HBX0 module: {}", module.resource));
        }
        let mut dependencies = module.dependencies.clone();
        dependencies.sort();
        if dependencies.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(format!("{}: duplicate HBX0 dependency", module.resource));
        }
    }
    let mut ordered = Vec::with_capacity(modules.len());
    while !by_resource.is_empty() {
        let available = by_resource
            .iter()
            .find(|(_, module)| {
                module
                    .dependencies
                    .iter()
                    .all(|dependency| !by_resource.contains_key(dependency))
            })
            .map(|(resource, _)| resource.clone())
            .ok_or("HBX0 module dependencies contain a cycle")?;
        let mut module = by_resource.remove(&available).unwrap();
        module.dependencies.sort();
        ordered.push(module);
    }
    validate_bundle_modules(&ordered)?;
    Ok(ordered)
}

fn validate_bundle_modules(modules: &[BytecodeBundleModule]) -> Result<(), String> {
    let mut positions = std::collections::HashMap::with_capacity(modules.len());
    for (index, module) in modules.iter().enumerate() {
        if module.resource.is_empty() {
            return Err("HBX0 module resource must not be empty".into());
        }
        if module.namespace_form.is_empty() {
            return Err(format!(
                "{}: HBX0 namespace form must not be empty",
                module.resource
            ));
        }
        if positions.insert(module.resource.as_str(), index).is_some() {
            return Err(format!("duplicate HBX0 module: {}", module.resource));
        }
        if module
            .dependencies
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
        {
            return Err(format!(
                "{}: HBX0 dependencies must be unique and sorted",
                module.resource
            ));
        }
    }
    for (index, module) in modules.iter().enumerate() {
        for dependency in &module.dependencies {
            if positions
                .get(dependency.as_str())
                .is_some_and(|position| *position >= index)
            {
                return Err(format!(
                    "{}: HBX0 dependency must appear first: {dependency}",
                    module.resource
                ));
            }
        }
    }
    Ok(())
}

fn standard_library_namespace(namespace: &str) -> bool {
    ["std.", "code.", "db.", "lang."]
        .iter()
        .any(|prefix| namespace.starts_with(prefix))
}

pub(super) fn namespace_dependencies(namespace_form: &str) -> Result<Vec<String>, String> {
    let forms = kernel::parse_forms(namespace_form)?;
    let Some(kernel::Form::List(items)) = forms.first() else {
        return Err("standard-library module has invalid ns form".into());
    };
    let config = kernel::GeneratedNamespaceConfig::configure_with(&items[2..], |_| true)?;
    let mut dependencies = config.required_namespaces().to_vec();
    dependencies.extend(config.used_namespaces().iter().cloned());
    dependencies.sort();
    dependencies.dedup();
    Ok(dependencies)
}

pub(super) fn split_namespace_form(source: &str) -> Result<(&str, &str), String> {
    let start = source.find("(ns ").ok_or("HAL module is missing ns form")?;
    let mut depth = 0usize;
    let mut string = false;
    let mut escape = false;
    for (offset, ch) in source[start..].char_indices() {
        if string {
            if escape {
                escape = false;
            } else if ch == '\\' {
                escape = true;
            } else if ch == '"' {
                string = false;
            }
            continue;
        }
        match ch {
            '"' => string = true,
            '(' => depth += 1,
            ')' => {
                depth = depth.checked_sub(1).ok_or("invalid ns form")?;
                if depth == 0 {
                    let end = start + offset + ch.len_utf8();
                    return Ok((&source[start..end], &source[end..]));
                }
            }
            _ => {}
        }
    }
    Err("unterminated ns form".into())
}

fn put_u32(output: &mut Vec<u8>, value: usize) -> Result<(), String> {
    let value = u32::try_from(value).map_err(|_| "foundation bundle exceeds u32 limits")?;
    output.extend_from_slice(&value.to_le_bytes());
    Ok(())
}

fn put_bytes(output: &mut Vec<u8>, value: &[u8]) -> Result<(), String> {
    put_u32(output, value.len())?;
    output.extend_from_slice(value);
    Ok(())
}

fn take_u32(input: &mut &[u8]) -> Result<u32, String> {
    let bytes = take(input, 4)?;
    Ok(u32::from_le_bytes(bytes.try_into().unwrap()))
}

fn take_bytes<'a>(input: &mut &'a [u8]) -> Result<&'a [u8], String> {
    let len = take_u32(input)? as usize;
    take(input, len)
}

fn take_string(input: &mut &[u8]) -> Result<String, String> {
    String::from_utf8(take_bytes(input)?.to_vec())
        .map_err(|_| "foundation bundle contains invalid UTF-8".into())
}

fn take<'a>(input: &mut &'a [u8], len: usize) -> Result<&'a [u8], String> {
    if input.len() < len {
        return Err("truncated HBX0 bytecode bundle".into());
    }
    let (value, rest) = input.split_at(len);
    *input = rest;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMPILER_GATE_STACK_SIZE: usize = 64 * 1024 * 1024;

    fn on_compiler_gate_stack(test: impl FnOnce() + Send + 'static) {
        std::thread::Builder::new()
            .name("foundation-bytecode-compiler-gate".into())
            // Compiling the complete portable library exercises the recursive
            // debug evaluator used to establish macro and declaration state.
            // Keep that test-only headroom local instead of requiring callers
            // to raise RUST_MIN_STACK for the entire test process.
            .stack_size(COMPILER_GATE_STACK_SIZE)
            .spawn(test)
            .expect("spawn foundation compiler gate")
            .join()
            .expect("foundation compiler gate panicked");
    }

    #[test]
    fn embedded_bundle_round_trips_and_bootstraps() {
        on_compiler_gate_stack(|| {
            let bytes = compile_embedded_foundation_bootstrap_bundle()
                .expect("compile Foundation bootstrap bundle");
            let mut runtime = Runtime::core();
            for &(name, _, source) in EMBEDDED_HAL_RESOURCES {
                runtime.register_resource(name, source);
            }
            eval_bytecode_bundle(&mut runtime, &bytes).expect("load foundation bundle");
            let publics = runtime
                .eval_native("(keys (ns-publics 'std.foundation.string))")
                .expect("inspect string namespace");
            assert!(publics.contains("upper"), "{publics}");
            assert!(runtime.use_namespace("std.foundation.string"));
            assert_eq!(runtime.eval_native("(upper \"hara\")").unwrap(), "\"HARA\"");
            assert!(runtime.use_namespace("std.foundation"));
            assert_eq!(runtime.eval_native("(if-not false 42)").unwrap(), "42");
            assert!(runtime.namespace_registry.find("lang.core").is_none());
            assert!(!runtime.bytecode_resources.contains_key("lang.core"));
        });
    }

    #[test]
    fn foundation_module_loads_through_the_bytecode_index() {
        let source = embedded_standard_library_sources()
            .into_iter()
            .find(|source| source.resource == "std.foundation")
            .expect("embedded foundation source");
        let bytes = compile_bytecode_bundle(&[source]).expect("compile foundation module");
        let modules = decode(&bytes).expect("decode foundation bundle");
        let program =
            crate::vm::decode_program(&modules[0].artifact).expect("decode foundation HBC");
        let first_macro = program
            .entry_function()
            .code
            .iter()
            .position(|instruction| matches!(instruction, crate::vm::Instruction::DefMacro { .. }));
        let first_return = program
            .entry_function()
            .code
            .iter()
            .position(|instruction| matches!(instruction, crate::vm::Instruction::Return));
        assert!(
            first_macro.is_some() && first_return.is_some_and(|index| index > first_macro.unwrap()),
            "Foundation artifact must execute macros before return: macro={first_macro:?}, return={first_return:?}"
        );
        let mut runtime = Runtime::core();
        runtime.register_resource(source.resource, source.source);
        eval_bytecode_bundle(&mut runtime, &bytes).expect("load indexed foundation module");
        assert!(runtime.use_namespace("std.foundation"));
        assert_eq!(
            runtime.eval_native("(vec (repeat 3 :x))").unwrap(),
            "[:x :x :x]"
        );
        assert!(
            runtime
                .macros
                .borrow()
                .contains_key(&("std.foundation".into(), "if-not".into())),
            "indexed Foundation load must register macros: {:?}",
            runtime.macros.borrow().keys().collect::<Vec<_>>()
        );
        assert_eq!(runtime.eval_native("(if-not false 42)").unwrap(), "42");
    }

    #[test]
    fn bundle_encoding_is_deterministic() {
        let sources = [ModuleSource {
            resource: "example.deterministic",
            source: "(ns example.deterministic) (def answer 42)",
        }];
        let first = compile_bytecode_bundle(&sources).expect("first deterministic bundle");
        let second = compile_bytecode_bundle(&sources).expect("second deterministic bundle");
        assert_eq!(first, second);
    }

    #[test]
    fn stale_lazy_bytecode_yields_to_registered_source() {
        let sources = [ModuleSource {
            resource: "example.stale",
            source: "(ns example.stale) (def answer 41)",
        }];
        let bytes = compile_bytecode_bundle(&sources).expect("compile stale fixture");
        let mut runtime = Runtime::core();
        runtime.register_resource("example.stale", "(ns example.stale) (def answer 42)");

        eval_bytecode_bundle(&mut runtime, &bytes).expect("index bundle");

        assert!(!runtime.bytecode_resources.contains_key("example.stale"));
        assert_eq!(
            runtime
                .eval_native("(require [example.stale :as stale]) stale/answer")
                .unwrap(),
            "42"
        );
    }

    #[test]
    fn eager_failure_rolls_back_the_whole_bundle() {
        let mut compiler = Runtime::core();
        compiler.use_namespace("example.good");
        let good_artifact = compiler
            .compile_bytecode_artifact("(def marker 42)")
            .expect("compile successful eager module");
        compiler.use_namespace("example.bad");
        let bad_artifact = compiler
            .compile_bytecode_artifact("(throw \"boom\")")
            .expect("compile failing eager module");
        let good_digest = Sha256::digest(b"good").into();
        let bad_digest = Sha256::digest(b"bad").into();
        let modules = [
            BytecodeBundleModule {
                resource: "example.good".into(),
                namespace_form: "(ns example.good)".into(),
                source_digest: good_digest,
                dependencies: vec![],
                eager: true,
                artifact: good_artifact,
            },
            BytecodeBundleModule {
                resource: "example.bad".into(),
                namespace_form: "(ns example.bad)".into(),
                source_digest: bad_digest,
                dependencies: vec![],
                eager: true,
                artifact: bad_artifact,
            },
        ];
        let bytes = encode_bytecode_bundle(&modules).expect("encode transactional fixture");
        let mut runtime = Runtime::core();
        let namespaces_before = runtime
            .namespace_registry
            .all()
            .into_iter()
            .map(|namespace| namespace.name().as_str().to_owned())
            .collect::<std::collections::HashSet<_>>();

        let error = eval_bytecode_bundle(&mut runtime, &bytes).unwrap_err();

        assert!(error.contains("example.bad"), "{error}");
        assert!(!runtime.bytecode_resources.contains_key("example.good"));
        assert!(!runtime.bytecode_resources.contains_key("example.bad"));
        assert!(!runtime.loaded_resources.contains("example.good"));
        assert!(!runtime.loaded_resources.contains("example.bad"));
        assert_eq!(
            runtime
                .namespace_registry
                .all()
                .into_iter()
                .map(|namespace| namespace.name().as_str().to_owned())
                .collect::<std::collections::HashSet<_>>(),
            namespaces_before
        );
        assert_eq!(runtime.namespace_registry.current().name().as_str(), "user");
    }

    #[test]
    fn lazy_module_loads_protocol_dependency_before_extend_type() {
        let sources = [
            ModuleSource {
                resource: "example.protocol",
                source: "(ns example.protocol) (defprotocol IEmitter (emit-form [value]))",
            },
            ModuleSource {
                resource: "example.emit",
                source: "(ns example.emit (:require [example.protocol :as compiler])) (defstruct Emitter []) (extend-type Emitter compiler/IEmitter (emit-form [value] value))",
            },
        ];
        let bytes = compile_bytecode_bundle(&sources).expect("compile lazy protocol fixture");
        let mut runtime = Runtime::core();
        eval_bytecode_bundle(&mut runtime, &bytes).expect("index lazy protocol fixture");

        runtime
            .load_bytecode_resource("example.emit")
            .expect("load protocol consumer and dependency");

        assert!(runtime
            .namespace_registry
            .find("example.protocol")
            .is_some());
        assert!(runtime.namespace_registry.find("example.emit").is_some());
    }

    #[test]
    fn lazy_alias_compiles_without_an_eager_edge_and_loads_on_first_call() {
        let sources = [
            ModuleSource {
                resource: "example.lazy.target",
                source: "(ns example.lazy.target) (defn answer [] 42)",
            },
            ModuleSource {
                resource: "example.lazy.client",
                source: "(ns example.lazy.client (:require [example.lazy.target :as target :lazy true])) (defn answer [] (target/answer))",
            },
        ];
        let bytes = compile_bytecode_bundle(&sources).expect("compile lazy alias fixture");
        let modules = decode(&bytes).expect("decode lazy alias fixture");
        let client = modules
            .iter()
            .find(|module| module.resource == "example.lazy.client")
            .expect("client module");
        assert!(client.dependencies.is_empty());

        let mut runtime = Runtime::core();
        eval_bytecode_bundle(&mut runtime, &bytes).expect("index lazy alias fixture");
        runtime
            .load_bytecode_resource("example.lazy.client")
            .expect("load lazy client");
        assert!(runtime
            .namespace_registry
            .find("example.lazy.target")
            .is_none());
        assert!(runtime.use_namespace("example.lazy.client"));
        assert_eq!(runtime.eval_native("(answer)").unwrap(), "42");
        assert!(runtime
            .namespace_registry
            .find("example.lazy.target")
            .is_some());
    }

    #[test]
    fn bundle_compilation_orders_eager_dependencies_before_consumers() {
        let sources = [
            ModuleSource {
                resource: "example.client",
                source: "(ns example.client (:require [example.target :as target])) (def answer target/answer)",
            },
            ModuleSource {
                resource: "example.target",
                source: "(ns example.target) (def answer 42)",
            },
        ];
        let bytes = compile_bytecode_bundle(&sources).expect("compile dependency fixture");
        let modules = decode(&bytes).expect("decode dependency fixture");
        assert_eq!(modules[0].resource, "example.target");
        assert_eq!(modules[1].resource, "example.client");
    }

    #[test]
    fn eager_modules_load_in_their_own_namespaces() {
        let sources = embedded_standard_library_sources()
            .into_iter()
            .filter(|source| {
                source.resource == "std.foundation"
                    || EAGER_HAL_RESOURCES.contains(&source.resource)
            })
            .collect::<Vec<_>>();
        let bytes = compile_bytecode_bundle(&sources).expect("compile eager modules");
        let mut runtime = Runtime::core();
        for source in &sources {
            runtime.register_resource(source.resource, source.source);
        }
        eval_bytecode_bundle(&mut runtime, &bytes).expect("load eager modules");
        assert!(runtime.use_namespace("std.foundation.string"));
        assert_eq!(runtime.eval_native("(repeat \"x\" 3)").unwrap(), "\"xxx\"");
    }

    #[cfg(feature = "tracing-jit")]
    #[test]
    fn hbx_installed_functions_remain_eligible_for_jit_compilation() {
        let mut compiler = Runtime::core();
        compiler.use_namespace("example.jit");
        let artifact = compiler
            .compile_bytecode_artifact(
                "(defn sum-to [n] (loop [i 0 total 0] (if (< i n) (recur (+ i 1) (+ total i)) total)))",
            )
            .expect("compile hot bundle function");
        let bytes = encode_bytecode_bundle(&[BytecodeBundleModule {
            resource: "example.jit".into(),
            namespace_form: "(ns example.jit)".into(),
            source_digest: Sha256::digest(b"example.jit hot function").into(),
            dependencies: vec![],
            eager: true,
            artifact,
        }])
        .expect("encode eager JIT fixture");
        let mut runtime = Runtime::core();

        eval_bytecode_bundle(&mut runtime, &bytes).expect("load eager JIT fixture through HBX");
        assert_eq!(
            runtime.eval_native("(example.jit/sum-to 100)").unwrap(),
            "4950"
        );
        let telemetry = crate::vm::machine::active_jit_telemetry();
        assert!(
            crate::vm::machine::active_compiled_trace_count() > 0,
            "an HBC function installed through HBX must retain its program and JIT state: {telemetry:?}"
        );
    }

    #[test]
    fn embedded_bundle_contains_exact_foundation_bootstrap() {
        on_compiler_gate_stack(|| {
            let sources = embedded_standard_library_sources();
            let bytes = compile_bytecode_bundle(&sources).expect("compile Foundation bootstrap");
            let modules = decode(&bytes).expect("decode Foundation bootstrap");
            let actual = modules
                .iter()
                .map(|module| module.resource.as_str())
                .collect::<Vec<_>>();
            assert_eq!(
                actual.len(),
                sources.len(),
                "bundle inventory must be exact"
            );
            let mut inventory = actual.clone();
            inventory.sort_unstable();
            let mut expected = crate::FOUNDATION_BOOTSTRAP_INVENTORY.to_vec();
            expected.sort_unstable();
            assert_eq!(inventory, expected);
            assert!(!modules.iter().any(|module| module.resource == "code.test"));
            assert!(!modules.iter().any(|module| module.resource == "lang.core"));
        });
    }

    #[test]
    fn bundled_global_reads_are_bound_to_their_defining_namespaces() {
        on_compiler_gate_stack(|| {
            let bytes = compile_embedded_foundation_bootstrap_bundle()
                .expect("compile Foundation bootstrap bundle");
            for module in decode(&bytes).expect("decode Foundation bootstrap bundle") {
                let program = crate::vm::decode_program(&module.artifact)
                    .unwrap_or_else(|error| panic!("decode {}: {error}", module.resource));
                assert_eq!(program.namespace.as_deref(), Some(module.resource.as_str()));
                for function in &program.functions {
                    for instruction in &function.code {
                        if let crate::vm::Instruction::GetGlobal(index) = instruction {
                            let name = program.constants[*index as usize].display();
                            assert!(
                                name.contains('/'),
                                "{} contains caller-relative global read {name}",
                                module.resource
                            );
                        }
                    }
                }
            }
        });
    }
}
