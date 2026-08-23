#[cfg(feature = "bytecode-vm")]
const EMBEDDED_FOUNDATION_BYTECODE: &[u8] =
    include_bytes!(concat!(env!("HARA_SOURCE_ROOT"), "/assets/std.foundation.hbx"));

#[cfg_attr(not(feature = "raw-wasm"), wasm_bindgen)]
impl Runtime {
    fn empty() -> Runtime {
        let namespace_registry = core::minimal_namespace_registry();
        let foundation = namespace_registry.find_or_create("std.foundation");
        for (name, value) in core::exception_function_values() {
            foundation.intern_with_origin(name, value, kernel::VarOrigin::RuntimePrimitive);
        }
        let vm_provider = namespace_registry.find_or_create("tool.vm.provider");
        for (name, value) in core::vm_tool_provider_values() {
            vm_provider.intern_with_origin(name, value, kernel::VarOrigin::RuntimePrimitive);
        }
        let package_provider = namespace_registry.find_or_create("tool.package.provider");
        for (name, value) in core::package_tool_provider_values() {
            package_provider.intern_with_origin(name, value, kernel::VarOrigin::RuntimePrimitive);
        }
        let work_native = namespace_registry.find_or_create("work.native");
        work_native.intern("default-host", crate::work::guest::default_host_value());
        for (name, value) in crate::work::guest::values() {
            work_native.intern(name, value);
        }
        let mut protocols = core::ProtocolRegistry::core();
        crate::work::guest::install(&mut protocols);
        Runtime {
            evaluator: Evaluator::new(),
            test_runner: "code.test".into(),
            protocols,
            extensions: core::ExtensionRegistry::new(),
            wasm_extensions: HashMap::new(),
            native_wasm_imports: HashMap::new(),
            providers: core::ProviderRegistry::new(),
            package_catalog: core::PackageCatalog::default(),
            resources: HashMap::new(),
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
        runtime.refer_foundation_into("user");
        runtime
    }

    pub(crate) fn sandbox() -> Runtime {
        const FORBIDDEN: &[&str] = &[
            "Runtime", "Kernel", "Sandbox", "Package", "Crypto", "OS", "Process", "File", "Socket",
            "Host",
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
        runtime.refer_foundation_into("user");
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

    #[cfg(feature = "bytecode-vm")]
    pub(crate) fn prepare_foundation_bytecode(&mut self) {
        let foundation = self.namespace_registry.find_or_create("std.foundation");
        for name in core::foundation_bootstrap_callable_names() {
            let symbol = crate::lang::data::Symbol::parse(name);
            if foundation.resolve(&symbol).is_none() {
                let value = core::direct_bootstrap_callable_value(name).unwrap_or_else(|| {
                    panic!("missing direct Foundation bootstrap callable: {name}")
                });
                foundation.intern_with_origin(name, value, kernel::VarOrigin::RuntimePrimitive);
            }
        }
    }

    fn refer_foundation_into(&mut self, namespace: &str) {
        let target = self.namespace_registry.find_or_create(namespace);
        if namespace == "std.foundation" {
            return;
        }
        let Some(foundation) = self.namespace_registry.find("std.foundation") else {
            return;
        };
        for (name, var) in foundation.mappings() {
            if target.resolve(&name).is_none() {
                target.map_var(name, var);
            }
        }
    }

    fn bootstrap_foundation(&mut self) -> Result<(), String> {
        for &(name, _, source) in EMBEDDED_HAL_RESOURCES {
            self.register_resource(name, source);
        }
        #[cfg(feature = "bytecode-vm")]
        {
            let mut source_fallback = false;
            match vm::eval_bytecode_bundle(self, EMBEDDED_FOUNDATION_BYTECODE)
            {
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
        self.use_namespace("std.foundation");
        self.refer_foundation_into("user");
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
                    let environment_before = self.evaluator.snapshot();
                    let macros_before = self.macros.borrow().clone();
                    let configs_before = self.generated_configs.clone();
                    let loaded_before = self.loaded_resources.clone();
                    if let Some(alias) = config.global_alias() {
                        self.namespace_registry.register_global_alias(alias, &name)?;
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
                            self.evaluator.restore(environment_before);
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
        let namespace_source = self.namespace_source();
        if traced {
            return core::with_test_runner(&self.test_runner, || {
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
                                                #[cfg(all(target_arch = "wasm32", not(feature = "raw-wasm")))]
                                                if let Some(handler) = &self.host_handler {
                                                    let handler = handler.clone();
                                                    return core::with_host_calls(
                                                        host_call_bridge(handler),
                                                        || self.evaluator.eval_tree(&form),
                                                    );
                                                }
                                                #[cfg(not(target_arch = "wasm32"))]
                                                if let Some(handler) = &self.native_host_handler {
                                                    return core::with_host_calls(
                                                        handler.clone(),
                                                        || self.evaluator.eval_tree(&form),
                                                    );
                                                }
                                                self.evaluator.eval_tree(&form)
                                            })
                                        })
                                    })
                                })
                            })
                        })
                    },
                )
            });
        }
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
                                    let mut fiber = self.evaluator.start_fiber(form)?;
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
                                })
                                    })
                                })
                            })
                        })
                    })
                },
            )
        })?;
        self.evaluator.finish_fiber(&fiber);
        result
    }

    fn refresh_qualified_bindings(&mut self) {
        core::refresh_namespace_environment(
            &self.namespace_registry,
            self.evaluator.environment_mut(),
        );
    }

    fn save_namespace(&mut self) {
        core::save_namespace_environment(
            &self.namespace_registry,
            self.evaluator.environment_mut(),
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
        self.namespace_registry
            .find_or_create(name)
            .set_role(config.role());
        if config.blank() {
            let target = self.namespace_registry.find_or_create(name);
            for (local, var) in target.mappings() {
                if var.symbol().get_namespace() != Some(name) {
                    target.unmap(&local);
                }
            }
        } else {
            self.refer_foundation_into(name);
            let target = self.namespace_registry.find_or_create(name);
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
                    self.evaluator.environment_mut().remove(&excluded);
                }
                self.macros
                    .borrow_mut()
                    .remove(&(name.to_owned(), excluded));
            }
        }
        core::select_namespace_environment(
            &self.namespace_registry,
            self.evaluator.environment_mut(),
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
        let changed = self
            .resources
            .get(name)
            .is_some_and(|existing| existing != source);
        self.resources.insert(name.into(), source.into());
        if !self.loaded_resources.contains(name) {
            self.namespace_registry
                .set_load_state(name, kernel::NamespaceLoadState::Unloaded);
        }
        if changed {
            self.loaded_resources.remove(name);
            #[cfg(feature = "bytecode-vm")]
            if self.bytecode_resources.contains_key(name) {
                self.resource_overrides.insert(name.into());
            }
        }
    }

    /// Detaches a host-supplied namespace while leaving already captured
    /// values alive. Package providers use this to deactivate one generation.
    pub fn unregister_resource(&mut self, name: &str) -> Result<(), JsValue> {
        self.product_cache.borrow_mut().clear();
        if self.namespace_registry.current().name().as_str() == name {
            return Err(JsValue::from_str("package/unload-current-namespace"));
        }
        self.resources.remove(name);
        self.resource_overrides.remove(name);
        self.loaded_resources.remove(name);
        #[cfg(feature = "bytecode-vm")]
        self.bytecode_resources.remove(name);
        self.generated_configs.remove(name);
        self.macros
            .borrow_mut()
            .retain(|(namespace, _), _| namespace != name);
        for namespace in self.namespace_registry.all() {
            for (symbol, var) in namespace.mappings() {
                if var.symbol().get_namespace() == Some(name) {
                    namespace.unmap(&symbol);
                }
            }
            for (alias, target) in namespace.aliases() {
                if target.name().as_str() == name {
                    namespace.unalias(alias.as_str());
                }
            }
        }
        self.namespace_registry.remove(name);
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
            let descriptor = core::Value::OrderedMap(Box::new(POrderedMap::from_iter([
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
            ])));
            self.package_catalog
                .register(package.coordinate, descriptor, namespaces.clone());
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
        self.bytecode_resources.contains_key(name)
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
        self.bytecode_resources
            .insert(name.clone(), (namespace_form, artifact));
        self.loaded_resources.remove(&name);
        self.namespace_registry
            .set_load_state(&name, kernel::NamespaceLoadState::Unloaded);
    }

    #[cfg(feature = "bytecode-vm")]
    pub(crate) fn load_bytecode_resource(&mut self, name: &str) -> Result<String, String> {
        self.bytecode_resources
            .get(name)
            .ok_or("module/not-found")?;
        let namespace_source = self.namespace_source();
        core::with_macros(self.macros.clone(), || {
            core::with_namespace_source(namespace_source, || {
                core::with_protocols(&self.protocols, || {
                    core::with_namespace_registry(&self.namespace_registry, || {
                        core::require_namespace(
                            &self.namespace_registry,
                            self.evaluator.environment_mut(),
                            name,
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
        let source = self
            .resources
            .get(name)
            .cloned()
            .ok_or_else(|| JsValue::from_str("module/not-found"))?;
        self.eval_text(&source)
            .map_err(|error| JsValue::from_str(&error))
    }

    /// Loads a resource once; subsequent requires return the current loaded marker.
    pub fn require_resource(&mut self, name: &str) -> Result<String, JsValue> {
        if self.loaded_resources.contains(name) {
            return Ok(":loaded".into());
        }
        if self.resource_overrides.contains(name) && self.resources.contains_key(name) {
            let result = self.load_resource(name)?;
            self.loaded_resources.insert(name.into());
            return Ok(result);
        }
        #[cfg(feature = "bytecode-vm")]
        if self.bytecode_resources.contains_key(name) {
            let result = self
                .load_bytecode_resource(name)
                .map_err(|error| JsValue::from_str(&error))?;
            self.loaded_resources.insert(name.into());
            return Ok(result);
        }
        if self.resources.contains_key(name) {
            let result = self.load_resource(name)?;
            self.loaded_resources.insert(name.into());
            return Ok(result);
        }
        if self.extensions.contains(name) {
            let result = self.require_extension(name)?;
            self.loaded_resources.insert(name.into());
            return Ok(result);
        }
        if self.wasm_extensions.contains_key(name) {
            let result = self
                .load_wasm_extension_namespace(name)
                .map_err(|error| JsValue::from_str(&error))?;
            self.loaded_resources.insert(name.into());
            return Ok(result);
        }
        Err(JsValue::from_str("module/not-found"))
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
    #[wasm_bindgen(js_name = installDirectWasmImport)]
    pub fn install_direct_wasm_import_js(
        &mut self,
        logical: &str,
        bytes: &[u8],
    ) -> Result<(), JsValue> {
        self.install_direct_wasm_import_browser(logical, bytes)
            .map_err(|error| JsValue::from_str(&error))
    }

    #[cfg(all(target_arch = "wasm32", not(feature = "raw-wasm")))]
    #[wasm_bindgen(js_name = installMemoryWasmBinding)]
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
