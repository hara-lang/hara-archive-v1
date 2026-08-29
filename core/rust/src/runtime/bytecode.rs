/// Experimental bytecode VM entry points (issue #195), gated behind the
/// non-default `bytecode-vm` feature. These accept only closed,
/// namespace-independent forms in the supported synchronous subset;
/// anything else fails as a typed compile error. There is no fallback to
/// the default evaluator, and `Runtime::eval_native` is unaffected.
///
/// Programs are returned inside `Rc` because compiled closures share the
/// program with their executing machines; `Rc::clone` is the cheap way to
/// pass one around.
#[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
#[derive(Clone)]
pub(crate) struct SourceBytecodeCache {
    directory: std::path::PathBuf,
}

#[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
impl SourceBytecodeCache {
    pub(crate) fn new(root: &std::path::Path, source_index_fingerprint: [u8; 32]) -> Self {
        Self {
            directory: root
                .join("target/hara/test-bytecode/v1")
                .join(hex_digest(&source_index_fingerprint)),
        }
    }

    fn path_for(&self, namespace: &str, source: &str) -> std::path::PathBuf {
        use sha2::{Digest, Sha256};

        let mut digest = Sha256::new();
        digest.update(b"hara-direct-native-source-v1\0");
        digest.update(env!("CARGO_PKG_VERSION").as_bytes());
        digest.update([0]);
        digest.update(namespace.as_bytes());
        digest.update([0]);
        digest.update(source.as_bytes());
        self.directory
            .join(format!("{}.hbc", hex_digest(&digest.finalize())))
    }

    fn load(
        &self,
        namespace: &str,
        source: &str,
    ) -> Option<crate::direct_native::ValidatedProgram> {
        let path = self.path_for(namespace, source);
        let bytes = std::fs::read(path).ok()?;
        let program = crate::vm::decode_program(&bytes).ok()?;
        if program.namespace.as_deref() != Some(namespace) {
            return None;
        }
        Some(crate::direct_native::ValidatedProgram::from_artifact(
            Rc::new(program),
        ))
    }

    fn store(&self, namespace: &str, source: &str, program: &crate::vm::Program) {
        let path = self.path_for(namespace, source);
        if path.is_file() {
            return;
        }
        let Ok(bytes) = crate::vm::encode_program(program) else {
            return;
        };
        if std::fs::create_dir_all(&self.directory).is_err() {
            return;
        }
        let temporary = path.with_extension(format!("hbc.tmp-{}", std::process::id()));
        if std::fs::write(&temporary, bytes).is_ok() {
            let _ = std::fs::rename(temporary, path);
        }
    }
}

#[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
fn hex_digest(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

#[cfg(feature = "bytecode-vm")]
pub fn bytecode_namespace_registry() -> kernel::NamespaceRegistry<core::Value> {
    core::minimal_namespace_registry()
}

#[cfg(feature = "bytecode-vm")]
pub fn compile_bytecode(source: &str) -> Result<std::rc::Rc<vm::Program>, String> {
    vm::compile_source(source)
        .map(std::rc::Rc::new)
        .map_err(|error| error.to_string())
}

/// Executes a previously compiled and validated program.
#[cfg(feature = "bytecode-vm")]
pub fn execute_bytecode(program: &std::rc::Rc<vm::Program>) -> Result<String, String> {
    vm::execute_program(program.clone())
        .map(|value| value.display())
        .map_err(|error| error.to_string())
}

/// Returns tracing-JIT counters retained for a compiled bytecode program.
/// `None` means this build has no tracing-JIT feature enabled.
#[cfg(all(feature = "bytecode-vm", feature = "tracing-jit"))]
pub fn bytecode_jit_telemetry(program: &std::rc::Rc<vm::Program>) -> jit::JitTelemetry {
    vm::machine::cached_jit_telemetry(program)
}

/// Compiles source into a checksummed, versioned bytecode artifact.
#[cfg(feature = "bytecode-vm")]
pub fn compile_bytecode_artifact(source: &str) -> Result<Vec<u8>, String> {
    let program = compile_bytecode(source)?;
    vm::encode_program(program.as_ref())
}

/// Decodes, validates, and executes a bytecode artifact.
#[cfg(feature = "bytecode-vm")]
pub fn execute_bytecode_artifact(bytes: &[u8]) -> Result<String, String> {
    let program = std::rc::Rc::new(vm::decode_program(bytes)?);
    execute_bytecode(&program)
}

/// Compiles and executes a source string through the experimental VM.
#[cfg(feature = "bytecode-vm")]
pub fn eval_bytecode_native(source: &str) -> Result<String, String> {
    execute_bytecode(&compile_bytecode(source)?)
}

impl Runtime {
    #[cfg(feature = "bytecode-vm")]
    pub(crate) fn compile_bytecode_product(
        &self,
        source: &str,
    ) -> Result<crate::compiled_product::CompiledProduct, String> {
        let source_digest = crate::compiled_product::sha256_hex(source.as_bytes());
        let compiler_id = format!("hara-runtime/{}", env!("CARGO_PKG_VERSION"));
        let options = format!("target=HBC0;namespace={}", self.current_namespace());
        let program = self.compile_bytecode(source)?;
        let bytes = vm::encode_program(program.as_ref())?;
        let module_digest = crate::compiled_product::sha256_hex(&bytes);
        let key = crate::compiled_product::ProductCacheKey::with_module_digests(
            crate::compiled_product::CompiledProductKind::HbcModule,
            source_digest.clone(),
            compiler_id.clone(),
            "hbc0",
            options.as_bytes(),
            vec![module_digest],
        );
        if let Some(product) = self.product_cache.borrow().get(&key).cloned() {
            return Ok(product);
        }
        let product = crate::compiled_product::CompiledProduct::new(
            crate::compiled_product::CompiledProductKind::HbcModule,
            source_digest,
            vec![crate::compiled_product::sha256_hex(&bytes)],
            compiler_id,
            "hbc0",
            options.as_bytes(),
            bytes,
        );
        self.product_cache.borrow_mut().insert(product.clone())?;
        Ok(product)
    }

    #[cfg(feature = "whole-wasm")]
    pub(crate) fn compile_whole_wasm_product(
        &self,
        source: &str,
    ) -> Result<crate::compiled_product::CompiledProduct, String> {
        let source_digest = crate::compiled_product::sha256_hex(source.as_bytes());
        let compiler_id = format!("hara-runtime/{}", env!("CARGO_PKG_VERSION"));
        let options = format!("target=HNW0;namespace={}", self.current_namespace());
        let abi_version = format!("hnw0/{}", crate::whole_wasm::HNW_ABI_VERSION);
        let hbc_product = self.compile_bytecode_product(source)?;
        let module_digest = hbc_product.manifest.artifact_digest.clone();
        let key = crate::compiled_product::ProductCacheKey::with_module_digests(
            crate::compiled_product::CompiledProductKind::WholeWasm,
            source_digest.clone(),
            compiler_id.clone(),
            abi_version.clone(),
            options.as_bytes(),
            vec![module_digest],
        );
        if let Some(product) = self.product_cache.borrow().get(&key).cloned() {
            return Ok(product);
        }
        let hbc = hbc_product.bytes;
        let bytes = crate::whole_wasm::compile_artifact_from_hbc(&hbc)?;
        let product = crate::compiled_product::CompiledProduct::new(
            crate::compiled_product::CompiledProductKind::WholeWasm,
            hbc_product.manifest.source_digest,
            vec![hbc_product.manifest.artifact_digest],
            compiler_id,
            abi_version,
            options.as_bytes(),
            bytes,
        );
        self.product_cache.borrow_mut().insert(product.clone())?;
        Ok(product)
    }

    /// Installs the typed native driver behind `std.native.Kernel/*`.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn install_native_kernel_provider(&mut self, provider: Rc<core::KernelProvider>) {
        self.providers.install_kernel(provider);
    }

    /// Installs the native host service handler used by `std.native.Host/call`.
    /// Embedders can expose process-local services without converting values
    /// through JavaScript or textual serialization.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn install_native_host_handler(
        &mut self,
        handler: Rc<dyn Fn(String, String, Vec<core::Value>) -> Result<core::Value, String>>,
    ) {
        self.native_host_handler = Some(handler);
    }

    /// Installs a publication-linked native ABI module and exposes it through
    /// the same promise-returning Host/call boundary used by browser embedders.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn install_native_module(
        &mut self,
        module: std::sync::Arc<dyn hara_abi::NativeModule>,
    ) -> Result<(), String> {
        self.native_modules.install(module)?;
        let registry = self.native_modules.clone();
        self.native_host_handler = Some(Rc::new(move |service, operation, arguments| {
            registry.invoke(service, operation, arguments)
        }));
        Ok(())
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn native_module_services(&self) -> Vec<String> {
        self.native_modules.services()
    }
}

#[cfg(feature = "bytecode-vm")]
impl Runtime {
    /// Compiles source against this runtime's namespace registry:
    /// std.foundation vars and anything already interned are visible to
    /// the compiler's two-phase global check (issue #223). The program
    /// is validated but not executed; globals intern only at execution.
    pub fn compile_bytecode(&self, source: &str) -> Result<std::rc::Rc<vm::Program>, String> {
        self.compile_bytecode_with_policy(source, false)
    }

    #[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
    fn compile_bytecode_for_direct_native(
        &self,
        source: &str,
    ) -> Result<std::rc::Rc<vm::Program>, String> {
        self.compile_bytecode_with_policy(source, true)
    }

    fn compile_spanned_forms_for_direct_native(
        &self,
        forms: &[kernel::SpannedForm],
    ) -> Result<std::rc::Rc<vm::Program>, String> {
        let namespace = self.current_namespace();
        let config = self
            .generated_configs
            .get(&namespace)
            .cloned()
            .unwrap_or_else(kernel::GeneratedNamespaceConfig::defaults);
        core::with_macros(self.macros.clone(), || {
            vm::compile_spanned_forms_with_config_allow_unbound_globals(
                forms,
                &self.namespace_registry,
                config,
            )
            .map(|mut program| {
                program.namespace = Some(namespace.clone());
                std::rc::Rc::new(program)
            })
            .map_err(|error| error.to_string())
        })
    }

    fn compile_bytecode_with_policy(
        &self,
        source: &str,
        allow_unbound_globals_for_direct_native: bool,
    ) -> Result<std::rc::Rc<vm::Program>, String> {
        core::with_macros(self.macros.clone(), || {
            let forms = kernel::read_forms(source).map_err(|error| error.to_string())?;
            let has_namespace_form = forms.iter().any(|form| {
                matches!(
                    crate::core::form_without_metadata(&form.form),
                    crate::kernel::Form::List(items)
                        if matches!(items.first(), Some(crate::kernel::Form::Symbol(operator)) if operator == "ns" || operator == "ns+")
                )
            });
            let config = if has_namespace_form {
                vm::source_namespace_config(&forms).map_err(|error| error.to_string())?
            } else {
                self.generated_configs
                    .get(&self.current_namespace())
                    .cloned()
                    .unwrap_or_else(kernel::GeneratedNamespaceConfig::defaults)
            };
            #[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
            let allow_unbound_globals = self.execution_backend == "direct-native"
                && (allow_unbound_globals_for_direct_native
                    || vm::source_uses_dynamic_evaluation(source).unwrap_or(false));
            #[cfg(not(all(feature = "direct-native", not(target_arch = "wasm32"))))]
            let allow_unbound_globals = false;
            let compiled = if allow_unbound_globals {
                vm::compile_source_with_config_allow_unbound_globals(
                    source,
                    &self.namespace_registry,
                    config,
                )
            } else {
                vm::compile_source_with_config(source, &self.namespace_registry, config)
            };
            compiled
                .map(|mut program| {
                    program.namespace =
                        Some(self.namespace_registry.current().name().as_str().to_owned());
                    program
                })
                .map(std::rc::Rc::new)
                .map_err(|error| error.to_string())
        })
    }

    /// Executes an already compiled program against this runtime's namespace
    /// registry. Embedding hosts use this for prepare-once/call-many paths
    /// without decoding an artifact or rebuilding the program on every call.
    pub fn execute_compiled_bytecode(
        &mut self,
        program: std::rc::Rc<vm::Program>,
    ) -> Result<String, String> {
        self.execute_compiled_bytecode_value(program)
            .map(|value| value.display())
    }

    /// Executes an already compiled program and returns its immutable runtime
    /// value directly. This avoids display serialization and lets native hosts
    /// inspect persistent results through their shared representation.
    pub fn execute_compiled_bytecode_value(
        &mut self,
        program: std::rc::Rc<vm::Program>,
    ) -> Result<core::Value, String> {
        let result = self.execute_compiled_bytecode_registry_value(program);
        let current = self.namespace_registry.current().name().as_str().to_owned();
        core::select_namespace_environment(
            &self.namespace_registry,
            self.execution.environment_mut(),
            &current,
        );
        result
    }

    /// Executes a prepared program directly against the namespace registry,
    /// without copying bindings into the compatibility environment per call.
    pub fn execute_compiled_bytecode_registry_value(
        &mut self,
        program: std::rc::Rc<vm::Program>,
    ) -> Result<core::Value, String> {
        let mut declaration_environment = HashMap::new();
        let namespace_source = self.namespace_source();
        core::with_macros(self.macros.clone(), || {
            core::with_namespace_source(namespace_source, || {
                core::with_protocols(&self.protocols, || {
                    core::with_namespace_registry(&self.namespace_registry, || {
                        core::with_declaration_transaction(&mut declaration_environment, |_| {
                            vm::execute_program_with_globals(program, &self.namespace_registry)
                                .map_err(|error| error.to_string())
                        })
                    })
                })
            })
        })
    }

    /// Compiles and executes through the experimental VM against this
    /// runtime's registry, then syncs the flat env so later `eval_native`
    /// calls see the vars the program interned. No fallback: unsupported
    /// forms fail as compile errors. `eval_native` is unaffected.
    pub fn eval_bytecode_native(&mut self, source: &str) -> Result<String, String> {
        let program = self.compile_bytecode(source)?;
        self.execute_compiled_bytecode(program)
    }

    /// Executes a validated program through the opt-in bytecode VM plus
    /// native-substrate boundary. Ordinary Hara functions remain VM-owned;
    /// only the closed native/protocol/evaluator target inventory crosses into
    /// Rust callouts.
    #[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
    pub fn execute_compiled_direct_native(
        &mut self,
        program: std::rc::Rc<vm::Program>,
    ) -> Result<crate::direct_native::NativeExecutionReport, String> {
        let program = crate::direct_native::ValidatedProgram::validate(program)?;
        self.execute_compiled_direct_native_validated(program)
    }

    #[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
    fn execute_compiled_direct_native_validated(
        &mut self,
        program: crate::direct_native::ValidatedProgram,
    ) -> Result<crate::direct_native::NativeExecutionReport, String> {
        let program_image = program.program();
        if let Some(namespace) = &program_image.namespace {
            self.namespace_registry.set_current(namespace);
        }
        let mut declaration_environment = HashMap::new();
        let namespace_source = self.namespace_source();
        let execute = || {
            core::with_test_runner(&self.test_runner, || {
                core::with_capability_providers(
                    self.providers.file(),
                    self.providers.socket(),
                    self.providers.process(),
                    self.providers.kernel(),
                    || {
                        core::with_package_catalog(&self.package_catalog, || {
                            core::with_promise_provider(self.providers.promise(), || {
                                core::with_macros(self.macros.clone(), || {
                                    core::with_namespace_registry(&self.namespace_registry, || {
                                        core::with_namespace_source(namespace_source, || {
                                            core::with_protocols(&self.protocols, || {
                                                let loader = Self::direct_native_namespace_loader(
                                                    self.direct_native.clone(),
                                                    self.direct_native_multimethods.clone(),
                                                    self.direct_native_source_cache.clone(),
                                                );
                                                core::with_direct_native_namespace_loader(
                                                    loader,
                                                    || {
                                                        core::with_declaration_transaction(
                                                            &mut declaration_environment,
                                                            |_| {
                                                                self.direct_native
                                                                    .execute_blocking_validated_with_multimethods(
                                                                        program,
                                                                        self.direct_native_multimethods
                                                                            .clone(),
                                                                    )
                                                            },
                                                        )
                                                    },
                                                )
                                            })
                                        })
                                    })
                                })
                            })
                        })
                    },
                )
            })
        };
        #[cfg(not(target_arch = "wasm32"))]
        let result = if let Some(handler) = self.native_host_handler.clone() {
            core::with_host_calls(handler, execute)
        } else {
            execute()
        };
        if result.is_ok() {
            self.save_namespace();
            self.refresh_qualified_bindings();
        }
        result
    }

    /// Builds the resource hook used by the shared namespace transaction when
    /// direct-native execution is selected. The hook compiles source-backed
    /// namespaces after their namespace declaration has been prepared and
    /// executes both source and artifact-backed namespaces through the same
    /// bytecode VM/native-substrate engine.
    #[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
    pub(crate) fn direct_native_namespace_loader(
        engine: crate::direct_native::NativeEngine,
        multimethods: core::MultiMethodRegistry,
        source_cache: Option<SourceBytecodeCache>,
    ) -> Rc<
        dyn Fn(
            &str,
            core::NamespaceResource,
            &mut HashMap<String, core::Value>,
        ) -> Result<(), String>,
    > {
        Rc::new(move |name, resource, environment| {
            load_direct_native_namespace(
                &engine,
                &multimethods,
                source_cache.as_ref(),
                name,
                resource,
                environment,
            )
        })
    }

    /// Compiles and executes source through the bytecode VM/native-substrate
    /// backend. Compilation-time namespace preparation and macro expansion
    /// retain their existing evaluator seam; no evaluator call is permitted
    /// once the validated program enters the native backend.
    #[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
    pub fn eval_direct_native(&mut self, source: &str) -> Result<String, String> {
        let program = self.compile_bytecode_for_direct_native(source)?;
        self.execute_compiled_direct_native_validated(
            crate::direct_native::ValidatedProgram::from_compiler(program),
        )
        .map(|report| report.value.display())
    }

    /// Compiles against this runtime's namespaces and persists the validated
    /// program for later native or browser execution.
    pub fn compile_bytecode_artifact(&self, source: &str) -> Result<Vec<u8>, String> {
        let program = self.compile_bytecode(source)?;
        vm::encode_program(program.as_ref())
    }

    /// Lowers a HALC module directly to persistent bytecode. No source text is
    /// reconstructed, and the module's normalized schema graph is embedded in
    /// the HBC artifact for later inference and specialization tiers.
    pub fn compile_halc_bytecode_artifact(&mut self, bytes: &[u8]) -> Result<Vec<u8>, String> {
        let module = kernel::halc::decode_halc(bytes)?;
        // HALC retains the source namespace declaration as structured data.
        // Apply it through the ordinary module loader before lowering so
        // aliases, refers, intrinsics, and required resources are identical
        // to interpreted HALC. Only the declaration is evaluated here; the
        // remaining forms go directly to the bytecode compiler below.
        if let Some(namespace_form) = module.forms.iter().find(|form| {
            matches!(
                core::form_without_metadata(form),
                Form::List(items)
                    if matches!(items.first(), Some(Form::Symbol(operator)) if operator == "ns")
            )
        }) {
            self.eval_forms(vec![synthetic_spanned_form(namespace_form.clone())], false)?;
        } else {
            self.use_namespace(&module.namespace);
        }
        let program = vm::compile_halc_module(&module, &self.namespace_registry)
            .map_err(|error| error.to_string())?;
        vm::encode_program(&program)
    }

    /// Executes a persisted artifact against this runtime's namespaces.
    pub fn eval_bytecode_artifact(&mut self, bytes: &[u8]) -> Result<String, String> {
        let program = std::rc::Rc::new(vm::decode_program(bytes)?);
        #[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
        if self.execution_backend == "direct-native" {
            let schema_types = program.schema_types.clone();
            let function_types = program.function_types.clone();
            let inferred_function_types = program.inferred_function_types.clone();
            let result = self
                .execute_compiled_direct_native_validated(
                    crate::direct_native::ValidatedProgram::from_artifact(program),
                )
                .map(|report| report.value.display());
            if result.is_ok() {
                self.halc_schema_types.extend(schema_types);
                self.halc_function_types.extend(function_types);
                self.halc_inferred_function_types
                    .extend(inferred_function_types);
            }
            return result;
        }
        if let Some(namespace) = &program.namespace {
            self.namespace_registry.set_current(namespace);
        }
        let schema_types = program.schema_types.clone();
        let function_types = program.function_types.clone();
        let inferred_function_types = program.inferred_function_types.clone();
        let mut declaration_environment = HashMap::new();
        let namespace_source = self.namespace_source();
        let result = core::with_macros(self.macros.clone(), || {
            core::with_namespace_source(namespace_source, || {
                core::with_protocols(&self.protocols, || {
                    core::with_namespace_registry(&self.namespace_registry, || {
                        core::with_declaration_transaction(&mut declaration_environment, |_| {
                            vm::execute_program_with_globals(program, &self.namespace_registry)
                                .map(|value| value.display())
                                .map_err(|error| error.to_string())
                        })
                    })
                })
            })
        });
        if result.is_ok() {
            self.halc_schema_types.extend(schema_types);
            self.halc_function_types.extend(function_types);
            self.halc_inferred_function_types
                .extend(inferred_function_types);
        }
        let current = self.namespace_registry.current().name().as_str().to_owned();
        core::select_namespace_environment(
            &self.namespace_registry,
            self.execution.environment_mut(),
            &current,
        );
        result
    }
}

#[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
fn load_direct_native_namespace(
    engine: &crate::direct_native::NativeEngine,
    multimethods: &core::MultiMethodRegistry,
    source_cache: Option<&SourceBytecodeCache>,
    name: &str,
    resource: core::NamespaceResource,
    environment: &mut HashMap<String, core::Value>,
) -> Result<(), String> {
    let program = match &resource {
        core::NamespaceResource::Source(_) => {
            compile_direct_native_source_namespace(name, &resource, environment, source_cache)?
        }
        #[cfg(not(target_arch = "wasm32"))]
        core::NamespaceResource::SourcePath(_) => {
            compile_direct_native_source_namespace(name, &resource, environment, source_cache)?
        }
        core::NamespaceResource::Bytecode {
            namespace_form,
            artifact,
        } => {
            for (index, form) in kernel::parse_forms(&namespace_form)?
                .into_iter()
                .enumerate()
            {
                let namespace_value = core::form_to_value(&form)?;
                core::eval_bytecode_management_in(&namespace_value, environment)
                    .map_err(|error| format!("{name}: namespace form {}: {error}", index + 1))?;
            }
            let registry = core::namespace_registry()?;
            registry.set_current(name);
            let mut program = vm::decode_program(&artifact)
                .map_err(|error| format!("{name}: direct-native artifact: {error}"))?;
            program.namespace = Some(name.to_owned());
            crate::direct_native::ValidatedProgram::from_artifact(Rc::new(program))
        }
    };
    engine
        .execute_blocking_validated_with_multimethods(program, multimethods.clone())
        .map(|_| ())
        .map_err(|error| format!("{name}: direct-native execution: {error}"))
}

#[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
fn compile_direct_native_source_namespace(
    name: &str,
    resource: &core::NamespaceResource,
    environment: &mut HashMap<String, core::Value>,
    source_cache: Option<&SourceBytecodeCache>,
) -> Result<crate::direct_native::ValidatedProgram, String> {
    let source = core::read_source_resource(resource, name)?;
    let forms = kernel::read_forms(&source).map_err(|error| error.to_string())?;
    let mut body_offset = 0;
    if forms.first().is_some_and(|form| {
        matches!(
            core::form_without_metadata(&form.form),
            kernel::Form::List(items)
                if matches!(items.first(), Some(kernel::Form::Symbol(operator)) if operator == "ns" || operator == "ns+")
        )
    }) {
        let namespace_value = core::form_to_value(&forms[0].form)?;
        core::eval_bytecode_management_in(&namespace_value, environment)
            .map_err(|error| format!("{name}: namespace declaration: {error}"))?;
        body_offset = forms[0].span.end.offset;
    }
    if let Some(program) = source_cache.and_then(|cache| cache.load(name, &source)) {
        return Ok(program);
    }
    let config = vm::source_namespace_config(&forms)
        .map_err(|error| format!("{name}: namespace configuration: {error}"))?;
    let registry = core::namespace_registry()?;
    registry.set_current(name);
    let body = source
        .get(body_offset..)
        .ok_or_else(|| format!("{name}: namespace form offset is invalid"))?;
    let allow_unbound_globals = vm::source_uses_dynamic_evaluation(body).unwrap_or(false);
    let compile = || {
        if allow_unbound_globals {
            vm::compile_source_with_config_allow_unbound_globals(body, &registry, config)
        } else {
            vm::compile_source_with_config(body, &registry, config)
        }
    };
    let mut program = core::without_direct_native_execution(compile)
        .map_err(|error| format!("{name}: direct-native compilation: {error}"))?;
    program.namespace = Some(name.to_owned());
    if let Some(cache) = source_cache {
        cache.store(name, &source, &program);
    }
    Ok(crate::direct_native::ValidatedProgram::from_compiler(
        Rc::new(program),
    ))
}

#[cfg(all(
    test,
    feature = "bytecode-vm",
    feature = "direct-native",
    not(target_arch = "wasm32")
))]
mod source_cache_tests {
    use super::SourceBytecodeCache;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct TempRoot(PathBuf);

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn temp_root() -> TempRoot {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let suffix = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "hara-source-bytecode-cache-{}-{suffix}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("cache test temporary root must be new");
        TempRoot(path)
    }

    #[test]
    fn caches_only_the_matching_namespace_and_source() {
        let root = temp_root();
        let namespace = "example.cache";
        let source = "(+ 1 2)";
        let mut program = crate::vm::compile_source(source).expect("source must compile");
        program.namespace = Some(namespace.to_owned());
        let cache = SourceBytecodeCache::new(&root.0, [7; 32]);

        assert!(cache.load(namespace, source).is_none());
        cache.store(namespace, source, &program);

        let loaded = cache
            .load(namespace, source)
            .expect("stored source must be readable");
        assert_eq!(loaded.program().namespace.as_deref(), Some(namespace));
        assert!(cache.load(namespace, "(+ 1 3)").is_none());
        assert!(cache.load("example.other", source).is_none());
    }
}
