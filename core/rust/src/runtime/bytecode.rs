/// Experimental bytecode VM entry points (issue #195), gated behind the
/// non-default `bytecode-vm` feature. These accept only closed,
/// namespace-independent forms in the supported synchronous subset;
/// anything else fails as a typed compile error. There is no fallback to
/// the default evaluator, and `Runtime::eval_native` is unaffected.
///
/// Programs are returned inside `Rc` because compiled closures share the
/// program with their executing machines; `Rc::clone` is the cheap way to
/// pass one around.
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
            vm::compile_source_with_config(source, &self.namespace_registry, config)
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
            self.evaluator.environment_mut(),
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
        let namespace_source = self.namespace_source();
        core::with_macros(self.macros.clone(), || {
            core::with_namespace_source(namespace_source, || {
                core::with_protocols(&self.protocols, || {
                    vm::execute_program_with_globals(program, &self.namespace_registry)
                        .map_err(|error| error.to_string())
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
            self.eval_forms(vec![namespace_form.clone()], false)?;
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
        if let Some(namespace) = &program.namespace {
            self.namespace_registry.set_current(namespace);
        }
        let schema_types = program.schema_types.clone();
        let function_types = program.function_types.clone();
        let inferred_function_types = program.inferred_function_types.clone();
        let namespace_source = self.namespace_source();
        let result = core::with_macros(self.macros.clone(), || {
            core::with_namespace_source(namespace_source, || {
                core::with_protocols(&self.protocols, || {
                    vm::execute_program_with_globals(program, &self.namespace_registry)
                        .map(|value| value.display())
                        .map_err(|error| error.to_string())
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
            self.evaluator.environment_mut(),
            &current,
        );
        result
    }
}
