#[cfg(feature = "bytecode-vm")]
const EMBEDDED_FOUNDATION_BYTECODE: &[u8] = include_bytes!(concat!(
    env!("HARA_SOURCE_ROOT"),
    "/assets/std.foundation.hbx"
));

#[cfg_attr(not(feature = "raw-wasm"), wasm_bindgen)]
impl Runtime {
    pub(crate) fn standalone_eval_context(
        mut self,
    ) -> (
        kernel::NamespaceRegistry<core::Value>,
        HashMap<String, core::Value>,
    ) {
        // A standalone EvalFiber has no Runtime resource provider around it.
        // Materialize the same eager Foundation children that the normal
        // loader exposes before handing the registry to that fiber. This is
        // still one source/native resolution path; it only supplies the
        // missing Runtime owner for the convenience constructor.
        for &name in EAGER_HAL_RESOURCES {
            self.load_resource(name)
                .unwrap_or_else(|error| panic!("embedded Foundation resource {name}: {error:?}"));
            self.loaded_resources.insert(name.into());
            self.namespace_registry
                .set_load_state(name, kernel::NamespaceLoadState::Loaded);
        }
        self.use_namespace("user");
        self.namespace_registry
            .current()
            .set_foundation_visibility(None, &HashSet::new(), false);
        (self.namespace_registry.clone(), HashMap::new())
    }

    pub(crate) fn instrumentation_handle(
        &self,
    ) -> Rc<RefCell<crate::instrumentation::InstrumentationHub>> {
        self.execution.instrumentation_handle()
    }

    fn empty() -> Runtime {
        let namespace_registry = core::minimal_namespace_registry();
        // Keep the runtime substrate available to every evaluator entry point,
        // including `Runtime::core()` and bytecode-only sessions.  The
        // language-level Foundation modules are still loaded separately, but
        // their canonical native/protocol aliases must not depend on which
        // bootstrap constructor happened to be used.
        core::install_foundation_intrinsics(&namespace_registry);
        let vm_provider = namespace_registry.find_or_create("tool.vm.provider");
        for (name, value) in core::vm_tool_provider_values() {
            vm_provider.intern_with_origin(name, value, kernel::VarOrigin::RuntimePrimitive);
        }
        let package_provider = namespace_registry.find_or_create("tool.package.provider");
        for (name, value) in core::package_tool_provider_values() {
            package_provider.intern_with_origin(name, value, kernel::VarOrigin::RuntimePrimitive);
        }
        let work_native = namespace_registry.find_or_create("std.native.Work");
        for (name, value) in crate::work::guest::values() {
            work_native.intern(name, value);
        }
        let mut protocols = core::ProtocolRegistry::core();
        crate::work::guest::install(&mut protocols);
        Runtime {
            execution: RuntimeExecutionState::new(),
            test_runner: "code.test".into(),
            execution_backend: "interpreter".into(),
            protocols,
            extensions: core::ExtensionRegistry::new(),
            wasm_extensions: HashMap::new(),
            native_wasm_imports: HashMap::new(),
            providers: core::ProviderRegistry::new(),
            package_catalog: core::PackageCatalog::default(),
            resources: HashMap::new(),
            #[cfg(not(target_arch = "wasm32"))]
            source_paths: HashMap::new(),
            resource_overrides: HashSet::new(),
            #[cfg(feature = "bytecode-vm")]
            bytecode_resources: HashMap::new(),
            product_cache: RefCell::new(compiled_product::InMemoryProductCache::default()),
            loaded_resources: HashSet::new(),
            halc_schema_definitions: HashMap::new(),
            halc_function_schemas: HashMap::new(),
            halc_schema_types: HashMap::new(),
            halc_function_types: HashMap::new(),
            halc_inferred_function_types: HashMap::new(),
            namespace_registry,
            macros: Rc::new(RefCell::new(HashMap::new())),
            generated_configs: HashMap::from([(
                "user".into(),
                kernel::GeneratedNamespaceConfig::defaults(),
            )]),
            #[cfg(feature = "evaluation-journal")]
            next_journal_id: 1,
            #[cfg(all(target_arch = "wasm32", not(feature = "raw-wasm")))]
            host_handler: None,
            #[cfg(not(target_arch = "wasm32"))]
            native_host_handler: None,
            #[cfg(not(target_arch = "wasm32"))]
            native_modules: native_module::Registry::default(),
            #[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
            direct_native: crate::direct_native::NativeEngine::new(),
            #[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
            direct_native_multimethods: Rc::new(RefCell::new(HashMap::new())),
            #[cfg(not(target_arch = "wasm32"))]
            extension_roots: native_extension::configured_roots(),
        }
    }

    #[cfg_attr(not(feature = "raw-wasm"), wasm_bindgen(constructor))]
    pub fn new() -> Runtime {
        let mut runtime = Runtime::empty();
        runtime
            .bootstrap_foundation()
            .expect("embedded std.foundation fallback must be valid");
        runtime
    }

    pub(crate) fn sandbox() -> Runtime {
        const FORBIDDEN: &[&str] = &[
            "Runtime", "Kernel", "Sandbox", "Package", "Crypto", "OS", "Process", "File", "Socket",
            "Host", "Work",
        ];
        let runtime = Runtime::new();
        for name in FORBIDDEN {
            let namespace = format!("std.native.{name}");
            runtime.namespace_registry.remove(&namespace);
            for owner in ["user", "std.foundation", "std.native"] {
                if let Some(target) = runtime.namespace_registry.find(owner) {
                    target.unalias(name);
                    target.unmap(&crate::lang::data::Symbol::parse(name));
                    target.unmap(&crate::lang::data::Symbol::parse(&namespace));
                }
            }
        }
        runtime
    }

    /// Creates the portable core-language evaluator without loading the language-level
    /// foundation. This is useful for small embedded surfaces whose commands
    /// only require core forms and should become interactive immediately.
    pub fn core() -> Runtime {
        let mut runtime = Runtime::empty();
        runtime.use_namespace("user");
        runtime
    }

    fn configure_test_runner(&mut self, runner: &str) -> Result<(), String> {
        validate_test_runner(runner)?;
        self.test_runner = runner.into();
        Ok(())
    }

    pub fn set_test_runner(&mut self, runner: &str) -> Result<(), JsValue> {
        self.configure_test_runner(runner)
            .map_err(|error| JsValue::from_str(&error))
    }

    pub(crate) fn configure_execution_backend(&mut self, backend: &str) -> Result<(), String> {
        validate_execution_backend(backend)?;
        if self.execution_backend == backend {
            return Ok(());
        }
        self.execution_backend = backend.into();
        Ok(())
    }

    /// Selects the execution backend used by ordinary `eval` and Session
    /// evaluation. The default is `interpreter`; `direct-native` is an
    /// explicit native-target opt-in and never falls back to interpretation.
    pub fn set_execution_backend(&mut self, backend: &str) -> Result<(), JsValue> {
        self.configure_execution_backend(backend)
            .map_err(|error| JsValue::from_str(&error))
    }

    /// Returns the selected ordinary evaluation backend.
    pub fn execution_backend(&self) -> String {
        self.execution_backend.clone()
    }

    fn bootstrap_foundation(&mut self) -> Result<(), String> {
        for &(name, _, source) in EMBEDDED_HAL_RESOURCES {
            self.register_resource(name, source);
        }
        #[cfg(feature = "bytecode-vm")]
        {
            let mut source_fallback = false;
            match vm::eval_bytecode_bundle(self, EMBEDDED_FOUNDATION_BYTECODE) {
                Ok(()) => {
                    self.loaded_resources.insert("std.foundation".into());
                    for &name in EAGER_HAL_RESOURCES {
                        self.loaded_resources.insert(name.into());
                    }
                }
                Err(_) => {
                    // A source checkout may legitimately be newer than its
                    // tracked bytecode artifact while Foundation is being
                    // changed. Keep the CLI usable so the canonical HAL can
                    // validate itself and regenerate that artifact.
                    let foundation =
                        self.resources
                            .get("std.foundation")
                            .cloned()
                            .ok_or_else(|| {
                                "embedded HAL catalog is missing std.foundation".to_owned()
                            })?;
                    core::with_definition_origin(kernel::VarOrigin::HalFallback, || {
                        self.eval_text(&foundation)
                    })?;
                    self.loaded_resources.insert("std.foundation".into());
                    source_fallback = true;
                }
            }
            if source_fallback {
                for &name in EAGER_HAL_RESOURCES {
                    let source = self
                        .resources
                        .get(name)
                        .cloned()
                        .ok_or_else(|| format!("embedded HAL catalog is missing {name}"))?;
                    core::with_definition_origin(kernel::VarOrigin::HalFallback, || {
                        self.eval_text(&source)
                    })?;
                    self.loaded_resources.insert(name.into());
                }
            }
        }
        #[cfg(not(feature = "bytecode-vm"))]
        {
            let foundation = self
                .resources
                .get("std.foundation")
                .cloned()
                .ok_or_else(|| "embedded HAL catalog is missing std.foundation".to_owned())?;
            core::with_definition_origin(kernel::VarOrigin::HalFallback, || {
                self.eval_text(&foundation)
            })?;
            self.loaded_resources.insert("std.foundation".into());
        }
        #[cfg(not(feature = "bytecode-vm"))]
        for &name in EAGER_HAL_RESOURCES {
            let source = self
                .resources
                .get(name)
                .cloned()
                .ok_or_else(|| format!("embedded HAL catalog is missing {name}"))?;
            core::with_definition_origin(kernel::VarOrigin::HalFallback, || {
                self.eval_text(&source)
            })?;
            self.loaded_resources.insert(name.into());
        }
        // Foundation and its eager dependencies have been evaluated above.
        // Keep the registry's load state in sync with the runtime resource
        // state so ns-info reports the actual bootstrap status.
        self.namespace_registry
            .set_load_state("std.foundation", kernel::NamespaceLoadState::Loaded);
        for &name in EAGER_HAL_RESOURCES {
            self.namespace_registry
                .set_load_state(name, kernel::NamespaceLoadState::Loaded);
        }
        self.use_namespace("std.foundation");
        core::apply_global_aliases(&self.namespace_registry, "user");
        self.use_namespace("user");
        Ok(())
    }

    fn eval_text_mode(&mut self, source: &str, traced: bool) -> Result<String, String> {
        self.eval_value_mode(source, traced)
            .map(|result| result.display())
    }

    fn eval_value_mode(&mut self, source: &str, traced: bool) -> Result<core::Value, String> {
        self.product_cache.borrow_mut().clear();
        self.refresh_qualified_bindings();
        let forms = kernel::read_forms(source).map_err(|error| error.to_string())?;
        let mut result = core::Value::Nil;
        for form in forms {
            let site = core::ExceptionSite {
                namespace: Some(self.namespace_registry.current().name().as_str().to_owned()),
                resource: None,
                line: form.span.start.line,
                column: form.span.start.column,
            };
            let form = core::exception_located_form(&form);
            result = core::with_exception_site(site, || self.eval_forms(vec![form], traced))?;
        }
        self.save_namespace();
        self.refresh_qualified_bindings();
        Ok(result)
    }

    fn eval_transfer_text(&mut self, source: &str) -> Result<String, String> {
        self.refresh_qualified_bindings();
        let forms = kernel::parse_forms(source)?;
        let result = self.eval_forms(forms, false)?;
        self.save_namespace();
        self.refresh_qualified_bindings();
        if !core::session_transferable(&result) {
            return Err(format!(
                "SESSION_TRANSFER_REJECTED {}",
                core::portable_type_name(&result)
            ));
        }
        Ok(result.display())
    }

    pub fn eval_halc(&mut self, bytes: &[u8]) -> Result<String, String> {
        self.refresh_qualified_bindings();
        let module = kernel::halc::decode_halc(bytes)?;
        let schemas = module.schemas;
        let result = self.eval_forms(module.forms, false)?;
        self.halc_schema_definitions.extend(schemas.definitions);
        self.halc_function_schemas.extend(schemas.functions);
        self.halc_schema_types.extend(schemas.definition_types);
        self.halc_function_types.extend(schemas.function_types);
        self.save_namespace();
        self.refresh_qualified_bindings();
        Ok(result.display())
    }

    fn eval_forms(&mut self, forms: Vec<Form>, traced: bool) -> Result<core::Value, String> {
        let mut result = core::Value::Nil;
        for form in forms {
            let mut restore_namespace = None;
            if let Form::List(values) = &form {
                if matches!(values.first(), Some(Form::Symbol(name)) if name == "ns" || name == "ns+")
                {
                    let (name, clause_start) = match values.first() {
                        Some(Form::Symbol(operator)) if operator == "ns" => match values.get(1) {
                            Some(Form::Symbol(name)) if !name.contains('/') => (name.clone(), 2),
                            _ => return Err("ns expects an unqualified namespace symbol".into()),
                        },
                        Some(Form::Symbol(_)) => {
                            if matches!(values.get(1), Some(Form::Symbol(_))) {
                                return Err("ns+ does not accept a namespace name".into());
                            }
                            (self.current_namespace(), 1)
                        }
                        _ => unreachable!(),
                    };
                    #[cfg(not(target_arch = "wasm32"))]
                    let roots = self.extension_roots.clone();
                    let config = kernel::GeneratedNamespaceConfig::configure_with(
                        &values[clause_start..],
                        |target| {
                            if self.namespace_registry.find(target).is_some()
                                || self.namespace_registry.load_state(target).is_some()
                                || self.resources.contains_key(target)
                                || self.wasm_extensions.contains_key(target)
                                || self.has_bytecode_resource(target)
                            {
                                return true;
                            }
                            #[cfg(not(target_arch = "wasm32"))]
                            {
                                return native_extension::package_exists(target, &roots);
                            }
                            #[cfg(target_arch = "wasm32")]
                            false
                        },
                    )?;
                    for target in config.required_namespaces() {
                        if self.resources.contains_key(target)
                            || self.loaded_resources.contains(target)
                            || self.namespace_registry.load_state(target)
                                == Some(kernel::NamespaceLoadState::Loaded)
                            || self.has_bytecode_resource(target)
                            || {
                                #[cfg(not(target_arch = "wasm32"))]
                                {
                                    self.source_paths.contains_key(target)
                                }
                                #[cfg(target_arch = "wasm32")]
                                {
                                    false
                                }
                            }
                        {
                            continue;
                        }
                        if target == "std.foundation"
                            || target.starts_with("std.lib.")
                            || target.starts_with("std.foundation.")
                        {
                            continue;
                        }
                        #[cfg(not(target_arch = "wasm32"))]
                        self.install_discovered_extension(target)?;
                        self.load_wasm_extension_namespace(target)?;
                    }

                    let registry_before = self.namespace_registry.snapshot();
                    let environment_before = self.execution.snapshot();
                    let macros_before = self.macros.borrow().clone();
                    let configs_before = self.generated_configs.clone();
                    let loaded_before = self.loaded_resources.clone();
                    if let Some(alias) = config.global_alias() {
                        self.namespace_registry
                            .register_global_alias(alias, &name)?;
                    }
                    for alias in config.declared_global_imports() {
                        let canonical =
                            core::canonical_native_symbol(alias).unwrap_or_else(|| alias.clone());
                        self.namespace_registry
                            .register_global_import(alias, canonical)?;
                    }
                    self.generated_configs.insert(name.clone(), config);
                    self.use_namespace(&name);
                    let config = self
                        .generated_configs
                        .get(&name)
                        .expect("ns config was installed")
                        .clone();
                    let namespace = self.namespace_registry.current();
                    namespace.set_native_flavor(config.native_flavor().map(str::to_owned));
                    for (local, module) in config.native_imports() {
                        namespace.import(local, module.clone());
                    }
                    self.bind_direct_wasm_imports(&config)?;
                    let foundation_bootstrap_child = name.starts_with("std.foundation.");
                    let require_specs = values[clause_start..]
                        .iter()
                        .flat_map(|clause| match clause {
                            Form::List(items)
                                if matches!(items.first(), Some(Form::Keyword(key)) if key == "require") =>
                            {
                                items[1..].to_vec()
                            }
                            Form::List(items)
                                if matches!(items.first(), Some(Form::Keyword(key)) if key == "use") =>
                            {
                                items[1..]
                                    .iter()
                                    .cloned()
                                    .map(|target| Form::Vector(vec![target]))
                                    .collect()
                            }
                            _ => Vec::new(),
                        })
                        // std.foundation is the host bootstrap namespace. Its
                        // child HAL libraries are rewritten against the
                        // catalog while it is still being assembled, so they
                        // must not recursively require the partially-built
                        // namespace through the ordinary module loader.
                        .filter(|spec| {
                            !foundation_bootstrap_child
                                || !matches!(spec,
                                Form::Vector(items)
                                    if matches!(items.first(), Some(Form::Symbol(target)) if target == "std.foundation"))
                        })
                        .collect::<Vec<_>>();
                    if !require_specs.is_empty() {
                        let require_form = Form::List(
                            std::iter::once(Form::Symbol("require".into()))
                                .chain(require_specs)
                                .collect(),
                        );
                        if let Err(error) = self.eval_form(require_form, traced) {
                            self.namespace_registry.restore(registry_before);
                            self.execution.restore(environment_before);
                            *self.macros.borrow_mut() = macros_before;
                            self.generated_configs = configs_before;
                            self.loaded_resources = loaded_before;
                            return Err(error);
                        }
                        let config = self
                            .generated_configs
                            .get(&name)
                            .expect("ns config was installed");
                        self.sync_generated_aliases(config);
                    }
                    // Loading required modules may select their namespaces.
                    // The namespace declaration itself must always finish in
                    // the namespace it declared so later compilation binds
                    // aliases and globals against the defining module.
                    self.use_namespace(&name);
                    result = core::Value::Nil;
                    continue;
                }
            }
            if let Form::List(values) = &form {
                if matches!(values.first(), Some(Form::Symbol(name)) if name == "require") {
                    let current = self.current_namespace();
                    restore_namespace = Some(current.clone());
                    let mut config = self
                        .generated_configs
                        .get(&current)
                        .cloned()
                        .unwrap_or_else(kernel::GeneratedNamespaceConfig::defaults);
                    {
                        #[cfg(not(target_arch = "wasm32"))]
                        let roots = self.extension_roots.clone();
                        let available = |target: &str| {
                            if self.namespace_registry.find(target).is_some()
                                || self.namespace_registry.load_state(target).is_some()
                                || self.resources.contains_key(target)
                                || {
                                    #[cfg(not(target_arch = "wasm32"))]
                                    {
                                        self.source_paths.contains_key(target)
                                    }
                                    #[cfg(target_arch = "wasm32")]
                                    {
                                        false
                                    }
                                }
                                || self.wasm_extensions.contains_key(target)
                            {
                                return true;
                            }
                            #[cfg(not(target_arch = "wasm32"))]
                            {
                                return native_extension::package_exists(target, &roots);
                            }
                            #[cfg(target_arch = "wasm32")]
                            false
                        };
                        for spec in &values[1..] {
                            config.apply_require(spec, &available)?;
                        }
                    }
                    self.sync_generated_aliases(&config);
                    self.generated_configs.insert(current, config);
                }
            }
            let mut config = self
                .generated_configs
                .get(&self.current_namespace())
                .cloned()
                .unwrap_or_else(kernel::GeneratedNamespaceConfig::defaults);
            let excluded = config.excluded_foundation().clone();
            config.set_global_aliases(
                self.namespace_registry
                    .global_aliases()
                    .into_iter()
                    .filter(|(_, namespace)| {
                        !excluded.contains(
                            namespace
                                .as_str()
                                .strip_prefix("std.foundation.")
                                .unwrap_or_default(),
                        )
                    })
                    .map(|(alias, namespace)| {
                        (alias.as_str().to_owned(), namespace.as_str().to_owned())
                    }),
            );
            reject_legacy_iterator_calls(&form)?;
            let resolved = config.rewrite(form);
            result = self.eval_form(resolved, traced)?;
            if let Some(namespace) = restore_namespace {
                self.use_namespace(&namespace);
            }
            if matches!(result, core::Value::Recur(_)) {
                return Err("recur must be inside loop".into());
            }
            self.save_namespace();
            self.refresh_qualified_bindings();
        }
        self.save_namespace();
        self.refresh_qualified_bindings();
        Ok(result)
    }

    fn eval_text(&mut self, source: &str) -> Result<String, String> {
        self.eval_text_mode(source, false)
    }

    fn eval_form(&mut self, form: Form, traced: bool) -> Result<core::Value, String> {
        #[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
        if self.execution_backend == "direct-native" {
            if traced {
                return Err("direct-native does not support traced evaluation".into());
            }
            if !is_interpreter_management_form(&form) {
                return self
                    .compile_form_for_direct_native(&form)
                    .and_then(|program| self.execute_compiled_direct_native(program))
                    .map(|report| report.value);
            }
        }
        if traced {
            return core::with_stack_trace(|| self.eval_form(form, false));
        }
        let namespace_source = self.namespace_source();
        let (result, fiber) = core::with_test_runner(&self.test_runner, || {
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
                                        core::with_protocols(&self.protocols, || -> Result<(Result<core::Value, String>, core::EvalFiber), String> {
                                            let evaluate = || {
                                                let mut fiber = self.execution.start_fiber(form)?;
                                                #[cfg(all(target_arch = "wasm32", not(feature = "raw-wasm")))]
                                                if let Some(handler) = &self.host_handler {
                                                    let handler = handler.clone();
                                                    let result = core::with_host_calls(
                                                        host_call_bridge(handler),
                                                        || fiber.drive_sync(),
                                                    );
                                                    return Ok((result, fiber));
                                                }
                                                #[cfg(not(target_arch = "wasm32"))]
                                                if let Some(handler) = &self.native_host_handler {
                                                    let result = core::with_host_calls(handler.clone(), || {
                                                        fiber.drive_sync()
                                                    });
                                                    return Ok((result, fiber));
                                                }
                                                Ok((fiber.drive_sync(), fiber))
                                            };
                                            #[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
                                            if self.execution_backend == "direct-native" {
                                return core::with_direct_native_namespace_loader(
                                    Self::direct_native_namespace_loader(
                                        self.direct_native.clone(),
                                        self.direct_native_multimethods.clone(),
                                    ),
                                                    evaluate,
                                                );
                                            }
                                            evaluate()
                                        })
                                    })
                                })
                            })
                        })
                    })
                },
            )
        })?;
        self.execution.finish_fiber(&fiber);
        result
    }

    fn refresh_qualified_bindings(&mut self) {
        core::refresh_namespace_environment(
            &self.namespace_registry,
            self.execution.environment_mut(),
        );
    }

    fn save_namespace(&mut self) {
        core::save_namespace_environment(
            &self.namespace_registry,
            self.execution.environment_mut(),
        );
    }

    pub fn create_namespace(&mut self, name: &str) -> bool {
        if name.is_empty() || self.namespace_registry.find(name).is_some() {
            return false;
        }
        self.namespace_registry.find_or_create(name);
        true
    }

    pub fn use_namespace(&mut self, name: &str) -> bool {
        self.product_cache.borrow_mut().clear();
        if name.is_empty() {
            return false;
        }
        let config = self
            .generated_configs
            .get(name)
            .cloned()
            .unwrap_or_else(kernel::GeneratedNamespaceConfig::defaults);
        let target = self.namespace_registry.find_or_create(name);
        target.set_role(config.role());
        target.set_foundation_visibility(
            config.exposed_foundation(),
            config.excluded_foundation(),
            config.blank(),
        );
        if config.blank() {
            for (local, var) in target.mappings() {
                if var.symbol().get_namespace() != Some(name) {
                    target.unmap(&local);
                }
            }
        } else {
            core::apply_global_aliases(&self.namespace_registry, name);
            let omitted = match config.exposed_foundation() {
                Some(exposed) => target
                    .mappings()
                    .into_iter()
                    .filter(|(local, var)| {
                        var.symbol().get_namespace() == Some("std.foundation")
                            && !exposed.contains(local.as_str())
                    })
                    .map(|(local, _)| local.as_str().to_owned())
                    .collect::<Vec<_>>(),
                None => config.excluded_foundation().iter().cloned().collect(),
            };
            for excluded in omitted {
                let local = crate::lang::data::Symbol::parse(&excluded);
                if target
                    .resolve(&local)
                    .is_some_and(|var| var.symbol().get_namespace() == Some("std.foundation"))
                {
                    target.unmap(&local);
                    self.execution.environment_mut().remove(&excluded);
                }
                self.macros
                    .borrow_mut()
                    .remove(&(name.to_owned(), excluded));
            }
        }
        core::apply_global_imports(&self.namespace_registry, name);
        core::select_namespace_environment(
            &self.namespace_registry,
            self.execution.environment_mut(),
            name,
        );
        self.sync_generated_aliases(&config);
        self.refresh_qualified_bindings();
        true
    }

    fn sync_generated_aliases(&self, config: &kernel::GeneratedNamespaceConfig) {
        let target = self.namespace_registry.current();
        for (alias, namespace) in config.aliases() {
            if let Some(source) = self.namespace_registry.find(&namespace) {
                target.alias(alias, source);
            }
        }
        for namespace in config.used_namespaces() {
            if let Some(source) = self.namespace_registry.find(namespace) {
                for (symbol, var) in source.mappings() {
                    if !config.used_symbol_excluded(namespace, symbol.as_str()) {
                        target.map_var(symbol, var);
                    }
                }
                let source_name = source.name().as_str().to_owned();
                let target_name = target.name().as_str().to_owned();
                let referred = self
                    .macros
                    .borrow()
                    .iter()
                    .filter_map(|((namespace, name), function)| {
                        (namespace == &source_name).then(|| (name.clone(), function.clone()))
                    })
                    .collect::<Vec<_>>();
                let mut macros = self.macros.borrow_mut();
                for (name, function) in referred {
                    if !config.used_symbol_excluded(namespace, &name) {
                        macros.insert((target_name.clone(), name), function);
                    }
                }
            }
        }
    }

    pub fn visible_symbols(&self) -> Vec<String> {
        self.namespace_registry.visible_symbol_names()
    }

    pub(crate) fn var_metadata(&self, symbol: &str) -> Option<kernel::VarMetadata> {
        self.namespace_registry
            .resolve(&crate::lang::data::Symbol::parse(symbol))
            .map(|var| var.metadata())
    }

    pub fn current_namespace(&self) -> String {
        self.namespace_registry.current().name().as_str().to_owned()
    }

    pub fn alias_namespace(&mut self, alias: &str, target: &str) -> bool {
        if alias.is_empty() || alias == "-" || target.is_empty() {
            return false;
        }
        let Some(target) = self.namespace_registry.find(target) else {
            return false;
        };
        self.namespace_registry.current().alias(alias, target);
        self.refresh_qualified_bindings();
        true
    }

    pub fn resolve_namespace(&self, name: &str) -> String {
        self.namespace_registry
            .current()
            .aliases()
            .into_iter()
            .find(|(alias, _)| alias.as_str() == name)
            .map(|(_, namespace)| namespace.name().as_str().to_owned())
            .unwrap_or_else(|| name.into())
    }

    /// Evaluates source after selecting a namespace.
    pub fn eval_in_namespace(&mut self, name: &str, source: &str) -> Result<String, JsValue> {
        let name = self.resolve_namespace(name);
        self.use_namespace(&name);
        self.eval_text(source)
            .map_err(|error| JsValue::from_str(&error))
    }

    pub fn require_resource_in_namespace(
        &mut self,
        resource: &str,
        namespace: &str,
    ) -> Result<String, JsValue> {
        let namespace = self.resolve_namespace(namespace);
        self.use_namespace(&namespace);
        self.require_resource(resource)
    }

    pub fn install_memory_file_provider(&mut self, root: &str) {
        self.providers
            .install_file(core::MemoryFileProvider::new(root));
    }

    #[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
    pub fn install_native_file_provider(&mut self, root: &str) {
        self.providers
            .install_file(core::NativeFileProvider::new(root));
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn install_native_socket_provider(&mut self) {
        self.providers
            .install_socket(core::NativeSocketProvider::default());
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn install_native_process_provider(&mut self) {
        self.providers.install_process();
    }

    pub fn install_loopback_socket_provider(&mut self) {
        self.providers
            .install_socket(core::LoopbackSocketProvider::default());
    }

    /// Installs the JS host handler that backs `std.native.Host/call`.
    #[cfg(all(target_arch = "wasm32", not(feature = "raw-wasm")))]
    pub fn install_host_handler(&mut self, handler: js_sys::Function) {
        self.host_handler = Some(handler);
    }

    pub fn file_resolve(&self, root: &str, path: &str) -> Result<String, JsValue> {
        let provider = self
            .providers
            .file()
            .ok_or_else(|| JsValue::from_str("file/unsupported"))?;
        provider
            .resolve(root, path)
            .map_err(|error| JsValue::from_str(&format!("file/{}", error.code())))
    }

    pub fn file_read(&self, path: &str) -> Result<PromiseHandle, JsValue> {
        let provider = self
            .providers
            .file()
            .ok_or_else(|| JsValue::from_str("file/unsupported"))?;
        provider
            .read(path)
            .map(PromiseHandle::from_promise)
            .map_err(|error| JsValue::from_str(&format!("file/{}", error.code())))
    }

    pub fn file_write(&self, path: &str, bytes: Vec<u8>) -> Result<PromiseHandle, JsValue> {
        let provider = self
            .providers
            .file()
            .ok_or_else(|| JsValue::from_str("file/unsupported"))?;
        provider
            .write(path, bytes)
            .map(PromiseHandle::from_promise)
            .map_err(|error| JsValue::from_str(&format!("file/{}", error.code())))
    }

    pub fn file_exists(&self, path: &str) -> Result<PromiseHandle, JsValue> {
        let provider = self
            .providers
            .file()
            .ok_or_else(|| JsValue::from_str("file/unsupported"))?;
        provider
            .exists(path)
            .map(PromiseHandle::from_promise)
            .map_err(|error| JsValue::from_str(&format!("file/{}", error.code())))
    }

    pub fn file_stat(&self, path: &str) -> Result<PromiseHandle, JsValue> {
        let provider = self
            .providers
            .file()
            .ok_or_else(|| JsValue::from_str("file/unsupported"))?;
        provider
            .stat(path)
            .map(PromiseHandle::from_promise)
            .map_err(|error| JsValue::from_str(&format!("file/{}", error.code())))
    }

    pub fn file_list(&self, path: &str) -> Result<PromiseHandle, JsValue> {
        let provider = self
            .providers
            .file()
            .ok_or_else(|| JsValue::from_str("file/unsupported"))?;
        provider
            .list(path)
            .map(PromiseHandle::from_promise)
            .map_err(|error| JsValue::from_str(&format!("file/{}", error.code())))
    }

    pub fn file_mkdir(&self, path: &str) -> Result<PromiseHandle, JsValue> {
        let provider = self
            .providers
            .file()
            .ok_or_else(|| JsValue::from_str("file/unsupported"))?;
        provider
            .mkdir(path)
            .map(PromiseHandle::from_promise)
            .map_err(|error| JsValue::from_str(&format!("file/{}", error.code())))
    }

    pub fn file_walk(&self, path: &str) -> Result<PromiseHandle, JsValue> {
        let provider = self
            .providers
            .file()
            .ok_or_else(|| JsValue::from_str("file/unsupported"))?;
        provider
            .walk(path)
            .map(PromiseHandle::from_promise)
            .map_err(|error| JsValue::from_str(&format!("file/{}", error.code())))
    }

    pub fn file_delete(&self, path: &str) -> Result<PromiseHandle, JsValue> {
        let provider = self
            .providers
            .file()
            .ok_or_else(|| JsValue::from_str("file/unsupported"))?;
        provider
            .delete(path)
            .map(PromiseHandle::from_promise)
            .map_err(|error| JsValue::from_str(&format!("file/{}", error.code())))
    }

    pub fn extension_available(&self, name: &str) -> bool {
        self.extensions.contains(name) || self.wasm_extensions.contains_key(name)
    }

    pub fn require_extension(&mut self, name: &str) -> Result<String, JsValue> {
        if self.wasm_extensions.contains_key(name) {
            return self
                .load_wasm_extension_namespace(name)
                .map_err(|error| JsValue::from_str(&error));
        }
        self.extensions
            .require(name, &mut self.protocols)
            .map_err(|error| JsValue::from_str(&error))
    }

    /// Registers a host-supplied Hara resource. Resources are source text, not executable host code.
    pub fn register_resource(&mut self, name: &str, source: &str) {
        self.product_cache.borrow_mut().clear();
        let name = canonical_resource_name(name);
        let changed = self
            .resources
            .get(&name)
            .is_some_and(|existing| existing != source);
        self.resources.insert(name.clone(), source.into());
        if !self.loaded_resources.contains(&name) {
            self.namespace_registry
                .set_load_state(&name, kernel::NamespaceLoadState::Unloaded);
        }
        if changed {
            self.loaded_resources.remove(&name);
            #[cfg(feature = "bytecode-vm")]
            if self.bytecode_resources.contains_key(&name) {
                self.resource_overrides.insert(name);
            }
        }
    }

    /// Detaches a host-supplied namespace while leaving already captured
    /// values alive. Package providers use this to deactivate one generation.
    pub fn unregister_resource(&mut self, name: &str) -> Result<(), JsValue> {
        self.product_cache.borrow_mut().clear();
        let name = canonical_resource_name(name);
        if self.namespace_registry.current().name().as_str() == name {
            return Err(JsValue::from_str("package/unload-current-namespace"));
        }
        self.resources.remove(&name);
        self.resource_overrides.remove(&name);
        self.loaded_resources.remove(&name);
        #[cfg(feature = "bytecode-vm")]
        self.bytecode_resources.remove(&name);
        self.generated_configs.remove(&name);
        self.macros
            .borrow_mut()
            .retain(|(namespace, _), _| namespace != &name);
        for namespace in self.namespace_registry.all() {
            for (symbol, var) in namespace.mappings() {
                if var.symbol().get_namespace() == Some(name.as_str()) {
                    namespace.unmap(&symbol);
                }
            }
            for (alias, target) in namespace.aliases() {
                if target.name().as_str() == name {
                    namespace.unalias(alias.as_str());
                }
            }
        }
        self.namespace_registry.remove(&name);
        self.refresh_qualified_bindings();
        Ok(())
    }

    /// Registers exact package ownership from project.lock.edn without
    /// downloading or loading any namespace.
    #[cfg_attr(not(feature = "raw-wasm"), wasm_bindgen(js_name = registerPackageLock))]
    pub fn register_package_lock(&mut self, source: &str) -> Result<(), JsValue> {
        let packages = package_catalog::catalog_from_lock(source)
            .map_err(|error| JsValue::from_str(&error))?;
        for package in packages {
            let namespaces = package.namespaces.clone();
            let mut descriptor = vec![
                (
                    core::Value::Keyword("package/coordinate".into()),
                    core::Value::String(package.coordinate.clone()),
                ),
                (
                    core::Value::Keyword("package/version".into()),
                    core::Value::String(package.version),
                ),
                (
                    core::Value::Keyword("package/tap".into()),
                    core::Value::String(package.tap),
                ),
                (
                    core::Value::Keyword("package/registry-commit".into()),
                    core::Value::String(package.registry_commit),
                ),
                (
                    core::Value::Keyword("package/identity-revision".into()),
                    core::Value::String(package.identity_revision),
                ),
                (
                    core::Value::Keyword("package/archive-sha256".into()),
                    core::Value::String(package.archive_sha256),
                ),
                (
                    core::Value::Keyword("package/namespaces".into()),
                    core::Value::Vector(PVector::from(
                        namespaces
                            .iter()
                            .map(|name| core::Value::Symbol(crate::lang::data::Symbol::parse(name)))
                            .collect::<Vec<_>>(),
                    )),
                ),
                (
                    core::Value::Keyword("package/dependencies".into()),
                    core::Value::Vector(PVector::from(
                        package
                            .dependencies
                            .iter()
                            .map(|coordinate| core::Value::String(coordinate.clone()))
                        .collect::<Vec<_>>(),
                    )),
                ),
            ];
            if let Some(name) = &package.name {
                descriptor.push((
                    core::Value::Keyword("package/name".into()),
                    core::Value::String(name.clone()),
                ));
            }
            self.package_catalog
                .register(
                    package.coordinate,
                    package.name,
                    core::Value::OrderedMap(Box::new(POrderedMap::from_iter(descriptor))),
                    namespaces.clone(),
                );
            for namespace in namespaces {
                if self.namespace_registry.load_state(&namespace).is_none() {
                    self.namespace_registry
                        .set_load_state(&namespace, kernel::NamespaceLoadState::Unloaded);
                }
            }
        }
        Ok(())
    }

    #[cfg(feature = "bytecode-vm")]
    fn has_bytecode_resource(&self, name: &str) -> bool {
        self.bytecode_resources
            .contains_key(&canonical_resource_name(name))
    }

    #[cfg(not(feature = "bytecode-vm"))]
    fn has_bytecode_resource(&self, _name: &str) -> bool {
        false
    }

    #[cfg(feature = "bytecode-vm")]
    pub(crate) fn register_bytecode_resource(
        &mut self,
        name: String,
        namespace_form: String,
        artifact: Vec<u8>,
    ) {
        let name = canonical_resource_name(&name);
        self.bytecode_resources
            .insert(name.clone(), (namespace_form, artifact));
        self.loaded_resources.remove(&name);
        self.namespace_registry
            .set_load_state(&name, kernel::NamespaceLoadState::Unloaded);
    }

    #[cfg(feature = "bytecode-vm")]
    pub(crate) fn load_bytecode_resource(&mut self, name: &str) -> Result<String, String> {
        let name = canonical_resource_name(name);
        self.bytecode_resources
            .get(&name)
            .ok_or("module/not-found")?;
        let namespace_source = self.namespace_source();
        core::with_macros(self.macros.clone(), || {
            core::with_namespace_source(namespace_source, || {
                core::with_protocols(&self.protocols, || {
                    core::with_namespace_registry(&self.namespace_registry, || {
                        #[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
                        if self.execution_backend == "direct-native" {
                            return core::with_direct_native_namespace_loader(
                                Self::direct_native_namespace_loader(
                                    self.direct_native.clone(),
                                    self.direct_native_multimethods.clone(),
                                ),
                                || {
                                    core::require_namespace(
                                        &self.namespace_registry,
                                        self.execution.environment_mut(),
                                        &name,
                                    )
                                },
                            );
                        }
                        core::require_namespace(
                            &self.namespace_registry,
                            self.execution.environment_mut(),
                            &name,
                        )
                    })
                })
            })
        })?;
        self.save_namespace();
        self.refresh_qualified_bindings();
        Ok(":loaded".into())
    }

    /// Evaluates a registered resource in the current lexical namespace.
    pub fn load_resource(&mut self, name: &str) -> Result<String, JsValue> {
        let snapshot = self.resource_snapshot();
        let result = self.load_resource_inner(name);
        if result.is_err() {
            self.restore_resource_snapshot(snapshot);
        }
        result
    }

    fn load_resource_inner(&mut self, name: &str) -> Result<String, JsValue> {
        let requested_bytecode = name
            .strip_prefix("classpath:")
            .unwrap_or(name)
            .ends_with(".hbx");
        let name = canonical_resource_name(name);
        ensure_foundation_root(self, name.as_str())?;
        #[cfg(feature = "bytecode-vm")]
        if requested_bytecode && self.bytecode_resources.contains_key(&name) {
            return self
                .load_bytecode_resource(&name)
                .map_err(|error| JsValue::from_str(&error));
        }
        if let Some(source) = self.resources.get(&name).cloned() {
            return self
                .eval_text(&source)
                .map_err(|error| JsValue::from_str(&error));
        }
        #[cfg(not(target_arch = "wasm32"))]
        if self.source_paths.contains_key(&name) {
            self.load_namespace_from_provider(&name)
                .map_err(|error| JsValue::from_str(&error))?;
            return Ok(":loaded".into());
        }
        Err(JsValue::from_str("module/not-found"))
    }

    /// Loads a resource once; subsequent requires return the current loaded marker.
    pub fn require_resource(&mut self, name: &str) -> Result<String, JsValue> {
        let snapshot = self.resource_snapshot();
        let result = self.require_resource_inner(name);
        if result.is_err() {
            self.restore_resource_snapshot(snapshot);
        }
        result
    }

    fn require_resource_inner(&mut self, name: &str) -> Result<String, JsValue> {
        let requested = name.to_owned();
        let name = canonical_resource_name(name);
        ensure_foundation_root(self, &requested)?;
        if self.loaded_resources.contains(&name) {
            self.refresh_loaded_resource_visibility();
            return Ok(":loaded".into());
        }
        if self.resource_overrides.contains(&name) && self.resources.contains_key(&name) {
            let result = self.load_resource(&name)?;
            self.loaded_resources.insert(name.clone());
            self.refresh_loaded_resource_visibility();
            return Ok(result);
        }
        #[cfg(feature = "bytecode-vm")]
        if self.bytecode_resources.contains_key(&name) {
            let result = self
                .load_bytecode_resource(&name)
                .map_err(|error| JsValue::from_str(&error))?;
            self.loaded_resources.insert(name.clone());
            self.refresh_loaded_resource_visibility();
            return Ok(result);
        }
        if self.resources.contains_key(&name) {
            let result = self.load_resource(&name)?;
            self.loaded_resources.insert(name.clone());
            self.refresh_loaded_resource_visibility();
            return Ok(result);
        }
        #[cfg(not(target_arch = "wasm32"))]
        if self.source_paths.contains_key(&name) {
            self.load_namespace_from_provider(&name)
                .map_err(|error| JsValue::from_str(&error))?;
            self.loaded_resources.insert(name.clone());
            self.refresh_loaded_resource_visibility();
            return Ok(":loaded".into());
        }
        if self.extensions.contains(&name) {
            let result = self.require_extension(&name)?;
            self.loaded_resources.insert(name.clone());
            self.refresh_loaded_resource_visibility();
            return Ok(result);
        }
        if self.wasm_extensions.contains_key(&name) {
            let result = self
                .load_wasm_extension_namespace(&name)
                .map_err(|error| JsValue::from_str(&error))?;
            self.loaded_resources.insert(name.clone());
            self.refresh_loaded_resource_visibility();
            return Ok(result);
        }
        Err(JsValue::from_str("module/not-found"))
    }

    fn resource_snapshot(&self) -> ResourceSnapshot {
        ResourceSnapshot {
            namespace_registry: self.namespace_registry.snapshot(),
            environment: self.execution.snapshot(),
            protocols: self.protocols.snapshot(),
            multimethods: core::snapshot_multimethods(),
            macros: self.macros.borrow().clone(),
            generated_configs: self.generated_configs.clone(),
            loaded_resources: self.loaded_resources.clone(),
        }
    }

    fn restore_resource_snapshot(&mut self, snapshot: ResourceSnapshot) {
        self.namespace_registry.restore(snapshot.namespace_registry);
        self.execution.restore(snapshot.environment);
        self.protocols.restore(snapshot.protocols);
        core::restore_multimethods(snapshot.multimethods);
        *self.macros.borrow_mut() = snapshot.macros;
        self.generated_configs = snapshot.generated_configs;
        self.loaded_resources = snapshot.loaded_resources;
        self.refresh_qualified_bindings();
    }

    fn refresh_loaded_resource_visibility(&mut self) {
        let current = self.current_namespace();
        self.use_namespace(&current);
    }

    pub fn file_supported(&self) -> bool {
        self.providers.capabilities().file
    }

    pub fn socket_supported(&self) -> bool {
        self.providers.capabilities().socket
    }

    /// Opens a callback-based socket and returns its provider-owned handle.
    pub fn socket_connect(&self, host: &str, port: u16) -> Result<u64, JsValue> {
        let provider = self
            .providers
            .socket()
            .ok_or_else(|| JsValue::from_str("socket/unsupported"))?;
        provider
            .connect(host, port, Rc::new(ignore_socket_event))
            .map_err(|error| JsValue::from_str(&format!("socket/{}", error.code())))
    }

    pub fn socket_send(&self, socket: u64, bytes: Vec<u8>) -> Result<usize, JsValue> {
        let provider = self
            .providers
            .socket()
            .ok_or_else(|| JsValue::from_str("socket/unsupported"))?;
        provider
            .send(socket, &bytes)
            .map_err(|error| JsValue::from_str(&format!("socket/{}", error.code())))
    }

    pub fn socket_close(&self, socket: u64) -> Result<(), JsValue> {
        let provider = self
            .providers
            .socket()
            .ok_or_else(|| JsValue::from_str("socket/unsupported"))?;
        provider
            .close(socket)
            .map_err(|error| JsValue::from_str(&format!("socket/{}", error.code())))
    }

    /// Returns whether a protocol method is registered in this runtime context.
    pub fn has_protocol_method(&self, protocol: &str, method: &str) -> bool {
        self.protocols.contains(protocol, method)
    }

    pub fn eval(&mut self, source: &str) -> Result<String, JsValue> {
        self.eval_text(source)
            .map_err(|error| JsValue::from_str(&error))
    }

    pub fn eval_traced(&mut self, source: &str) -> Result<String, JsValue> {
        self.eval_text_mode(source, true)
            .map_err(|error| JsValue::from_str(&error))
    }

    #[cfg(all(target_arch = "wasm32", not(feature = "raw-wasm")))]
    #[cfg_attr(not(feature = "raw-wasm"), wasm_bindgen(js_name = installDirectWasmImport))]
    pub fn install_direct_wasm_import_js(
        &mut self,
        logical: &str,
        bytes: &[u8],
    ) -> Result<(), JsValue> {
        self.install_direct_wasm_import_browser(logical, bytes)
            .map_err(|error| JsValue::from_str(&error))
    }

    #[cfg(all(target_arch = "wasm32", not(feature = "raw-wasm")))]
    #[cfg_attr(not(feature = "raw-wasm"), wasm_bindgen(js_name = installMemoryWasmBinding))]
    pub fn install_memory_wasm_binding_js(
        &mut self,
        manifest_source: &str,
        interface_source: &str,
        bindings_source: &str,
        bytes: &[u8],
    ) -> Result<(), JsValue> {
        self.install_memory_wasm_binding_browser(
            manifest_source,
            interface_source,
            bindings_source,
            bytes,
        )
        .map_err(|error| JsValue::from_str(&error))
    }

    #[cfg(feature = "bytecode-vm")]
    #[cfg_attr(not(feature = "raw-wasm"), wasm_bindgen(js_name = compileBytecodeArtifact))]
    pub fn compile_bytecode_artifact_js(&self, source: &str) -> Result<Vec<u8>, JsValue> {
        self.compile_bytecode_product(source)
            .map(|product| product.bytes)
            .map_err(|error| JsValue::from_str(&error))
    }

    /// Returns the immutable manifest for the HBC0 artifact produced from
    /// `source`. Hosts can cache the bytes and manifest without guessing the
    /// target or ABI from a filename.
    #[cfg(feature = "bytecode-vm")]
    #[cfg_attr(not(feature = "raw-wasm"), wasm_bindgen(js_name = compileBytecodeManifest))]
    pub fn compile_bytecode_manifest_js(&self, source: &str) -> Result<String, JsValue> {
        let product = self
            .compile_bytecode_product(source)
            .map_err(|error| JsValue::from_str(&error))?;
        serde_json::to_string(&product.manifest.to_json())
            .map_err(|error| JsValue::from_str(&error.to_string()))
    }

    /// Compiles source into an HNW0 artifact whose generated module can be
    /// instantiated by either Wasmtime or a browser WebAssembly engine.
    #[cfg(feature = "whole-wasm")]
    #[cfg_attr(not(feature = "raw-wasm"), wasm_bindgen(js_name = compileWholeWasmArtifact))]
    pub fn compile_whole_wasm_artifact_js(&self, source: &str) -> Result<Vec<u8>, JsValue> {
        self.compile_whole_wasm_product(source)
            .map(|product| product.bytes)
            .map_err(|error| JsValue::from_str(&error))
    }

    #[cfg(feature = "whole-wasm")]
    #[cfg_attr(not(feature = "raw-wasm"), wasm_bindgen(js_name = compileWholeWasmManifest))]
    pub fn compile_whole_wasm_manifest_js(&self, source: &str) -> Result<String, JsValue> {
        let product = self
            .compile_whole_wasm_product(source)
            .map_err(|error| JsValue::from_str(&error))?;
        serde_json::to_string(&product.manifest.to_json())
            .map_err(|error| JsValue::from_str(&error.to_string()))
    }

    #[cfg(feature = "bytecode-vm")]
    #[cfg_attr(not(feature = "raw-wasm"), wasm_bindgen(js_name = evalBytecodeArtifact))]
    pub fn eval_bytecode_artifact_js(&mut self, bytes: &[u8]) -> Result<String, JsValue> {
        self.eval_bytecode_artifact(bytes)
            .map_err(|error| JsValue::from_str(&error))
    }

    /// Installs a verified HBX0 namespace bundle. The bundle indexes each
    /// module's bytecode without making package loading implicit; callers
    /// still choose when to `require` a registered namespace.
    #[cfg(feature = "bytecode-vm")]
    #[cfg_attr(not(feature = "raw-wasm"), wasm_bindgen(js_name = evalBytecodeBundle))]
    pub fn eval_bytecode_bundle_js(&mut self, bytes: &[u8]) -> Result<(), JsValue> {
        crate::vm::eval_bytecode_bundle(self, bytes)
            .map_err(|error| JsValue::from_str(&error))
    }

    #[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
    pub fn eval_native(&mut self, source: &str) -> Result<String, String> {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(handler) = self.native_host_handler.clone() {
            return core::with_host_calls(handler, || self.eval_text(source));
        }
        self.eval_text(source)
    }

    #[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
    pub fn eval_native_traced(&mut self, source: &str) -> Result<String, String> {
        self.eval_text_mode(source, true)
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Runtime {
    /// Replaces the native project source catalog used by namespace loading.
    /// Only paths are retained; source bodies are read at the require
    /// boundary. Existing in-memory resources and bytecode remain higher
    /// priority than the mounted paths.
    pub fn register_source_catalog(&mut self, catalog: &crate::project::SourceCatalog) {
        self.product_cache.borrow_mut().clear();
        let mounted = catalog
            .entries()
            .iter()
            .map(|(name, path)| (name.clone(), path.clone()))
            .collect();
        let previous = std::mem::replace(&mut self.source_paths, mounted);
        let removed = previous
            .keys()
            .filter(|name| !self.source_paths.contains_key(*name))
            .cloned()
            .collect::<Vec<_>>();
        for name in removed {
            if self.resources.contains_key(&name) || self.has_bytecode_resource(&name) {
                continue;
            }
            self.loaded_resources.remove(&name);
            self.namespace_registry.remove(&name);
        }
        for name in self.source_paths.keys().cloned().collect::<Vec<_>>() {
            if self.resources.contains_key(&name) || self.has_bytecode_resource(&name) {
                continue;
            }
            self.loaded_resources.remove(&name);
            self.namespace_registry
                .set_load_state(&name, kernel::NamespaceLoadState::Unloaded);
        }
    }

    /// Detaches every mounted native project source path and removes its
    /// source-only namespace state. Materialized source namespaces are
    /// intentionally discarded at this explicit teardown boundary.
    pub fn clear_source_catalog(&mut self) {
        let names = self.source_paths.keys().cloned().collect::<Vec<_>>();
        self.source_paths.clear();
        for name in names {
            if self.resources.contains_key(&name) || self.has_bytecode_resource(&name) {
                continue;
            }
            self.loaded_resources.remove(&name);
            self.namespace_registry.remove(&name);
        }
    }

    /// Loads the language-level Foundation from the mounted source catalog.
    /// This is intentionally an interpreter bootstrap: the resulting macros,
    /// aliases, and protocol wiring form the compiler environment for later
    /// direct-native namespace loads.
    pub fn bootstrap_source_foundation(&mut self) -> Result<(), String> {
        self.configure_execution_backend("interpreter")?;
        self.load_namespace_from_provider("std.foundation")?;
        self.loaded_resources.insert("std.foundation".into());
        for &name in EAGER_HAL_RESOURCES {
            self.load_namespace_from_provider(name)?;
            self.loaded_resources.insert(name.into());
        }
        self.use_namespace("std.foundation");
        core::apply_global_aliases(&self.namespace_registry, "user");
        self.use_namespace("user");
        Ok(())
    }

    fn load_namespace_from_provider(&mut self, name: &str) -> Result<(), String> {
        let namespace_source = self.namespace_source();
        let file = self.providers.file();
        let socket = self.providers.socket();
        let process = self.providers.process();
        let kernel = self.providers.kernel();
        let promise = self.providers.promise();
        let package_catalog = &self.package_catalog;
        let protocols = &self.protocols;
        let namespace_registry = &self.namespace_registry;
        let environment = self.execution.environment_mut();
        let macros = self.macros.clone();
        let test_runner = self.test_runner.clone();
        let result = core::with_test_runner(&test_runner, || {
            core::with_capability_providers(file, socket, process, kernel, || {
                core::with_package_catalog(package_catalog, || {
                    core::with_promise_provider(promise, || {
                        core::with_macros(macros, || {
                            core::with_namespace_source(namespace_source, || {
                                core::with_protocols(protocols, || {
                                    core::with_namespace_registry(namespace_registry, || {
                                        let mut require = || {
                                            core::require_namespace(
                                                namespace_registry,
                                                environment,
                                                name,
                                            )
                                        };
                                        #[cfg(all(
                                            feature = "direct-native",
                                            not(target_arch = "wasm32")
                                        ))]
                                        if self.execution_backend == "direct-native" {
                                            return core::with_direct_native_namespace_loader(
                                                Self::direct_native_namespace_loader(
                                                    self.direct_native.clone(),
                                                    self.direct_native_multimethods.clone(),
                                                ),
                                                require,
                                            );
                                        }
                                        require()
                                    })
                                })
                            })
                        })
                    })
                })
            })
        });
        result?;
        self.save_namespace();
        self.refresh_qualified_bindings();
        Ok(())
    }
}

#[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
impl Runtime {
    /// Creates a Runtime whose native-substrate telemetry is shared with
    /// another Runtime owner. Namespace registries, providers, and mutable
    /// Hara state remain owned by each Runtime; bytecode program images remain
    /// owned by the execution that prepared them.
    pub fn with_native_engine(engine: crate::direct_native::NativeEngine) -> Self {
        let mut runtime = Self::new();
        runtime.direct_native = engine;
        runtime
    }

    /// Returns cumulative counters from the Runtime-owned bytecode VM and
    /// native-substrate boundary. The counters belong to the reusable native
    /// engine and are not cleared merely by switching the selected backend;
    /// callers sharing an engine can explicitly call
    /// [`crate::direct_native::NativeEngine::reset`] at their lifecycle
    /// boundary.
    pub fn native_execution_telemetry(&self) -> crate::direct_native::NativeExecutionTelemetry {
        self.direct_native.telemetry()
    }
}

pub(crate) fn validate_execution_backend(backend: &str) -> Result<(), String> {
    match backend {
        "interpreter" => Ok(()),
        "direct-native" => {
            #[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
            {
                Ok(())
            }
            #[cfg(not(all(feature = "direct-native", not(target_arch = "wasm32"))))]
            {
                Err(
                    "direct-native requires a native build with the direct-native Cargo feature"
                        .into(),
                )
            }
        }
        _ => Err(format!(
            "unknown execution backend {backend}; expected interpreter or direct-native"
        )),
    }
}

#[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
fn is_interpreter_management_form(form: &Form) -> bool {
    matches!(
        core::form_without_metadata(form),
        Form::List(items)
            if matches!(
                items.first(),
                Some(Form::Symbol(operator))
                    if matches!(operator.as_str(), "ns" | "ns+" | "require" | "in-ns")
            )
    )
}

struct ResourceSnapshot {
    namespace_registry: kernel::namespace::NamespaceRegistrySnapshot<core::Value>,
    environment: HashMap<String, core::Value>,
    protocols: core::ProtocolRegistrySnapshot,
    multimethods: HashMap<String, core::MultiMethod>,
    macros: HashMap<(String, String), Rc<core::Function>>,
    generated_configs: HashMap<String, kernel::GeneratedNamespaceConfig>,
    loaded_resources: HashSet<String>,
}

fn canonical_resource_name(name: &str) -> String {
    foundation_resource_namespace(name).unwrap_or_else(|| name.to_owned())
}

fn foundation_resource_namespace(name: &str) -> Option<String> {
    let resource = name.strip_prefix("classpath:").unwrap_or(name);
    if resource == "std/foundation.hal" || resource == "std/foundation.hbx" {
        return Some("std.foundation".into());
    }
    let child = resource.strip_prefix("std/foundation/")?;
    let child = child
        .strip_suffix(".hal")
        .or_else(|| child.strip_suffix(".hbx"))?;
    if child.is_empty() {
        return None;
    }
    Some(format!(
        "std.foundation.{}",
        child.replace('/', ".").replace('_', "-")
    ))
}

fn ensure_foundation_root(runtime: &mut Runtime, name: &str) -> Result<(), JsValue> {
    let Some(namespace) = foundation_namespace_for_request(name) else {
        return Ok(());
    };
    if namespace != "std.foundation" && !runtime.loaded_resources.contains("std.foundation") {
        runtime.require_resource("std.foundation")?;
    }
    Ok(())
}

fn foundation_namespace_for_request(name: &str) -> Option<String> {
    foundation_resource_namespace(name).or_else(|| {
        let namespace = name.strip_prefix("classpath:").unwrap_or(name);
        (namespace.starts_with("std.foundation.")).then(|| namespace.to_owned())
    })
}
