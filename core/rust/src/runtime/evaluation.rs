impl Runtime {
    /// Returns the canonical schema value loaded from HALC for a named schema Var.
    pub fn halc_schema(&self, qualified_var: &str) -> Option<&Form> {
        self.halc_schema_definitions.get(qualified_var)
    }

    /// Returns the canonical schema annotation loaded from HALC for a function Var.
    pub fn halc_function_schema(&self, qualified_var: &str) -> Option<&Form> {
        self.halc_function_schemas.get(qualified_var)
    }

    /// Returns the normalized compiler type for a named schema Var.
    pub fn halc_schema_type(&self, qualified_var: &str) -> Option<&kernel::SchemaType> {
        self.halc_schema_types.get(qualified_var)
    }

    /// Returns a conservative body-derived function signature, when the
    /// compiler could prove one independently of the declared contract.
    pub fn halc_inferred_function_type(&self, qualified_var: &str) -> Option<&kernel::SchemaType> {
        self.halc_inferred_function_types.get(qualified_var)
    }

    /// Returns a function's normalized annotation, resolving one named edge.
    pub fn halc_function_type(&self, qualified_var: &str) -> Option<&kernel::SchemaType> {
        let schema = self.halc_function_types.get(qualified_var)?;
        match schema {
            kernel::SchemaType::Reference(name) => {
                self.halc_schema_types.get(name).or(Some(schema))
            }
            _ => Some(schema),
        }
    }

    /// Evaluates native Hara source and returns its runtime value without a
    /// display round trip. Embedding hosts use this to inspect declarative
    /// values containing Vars, functions, bytes, and persistent collections.
    #[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
    pub fn eval_native_value(&mut self, source: &str) -> Result<core::Value, String> {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(handler) = self.native_host_handler.clone() {
            return core::with_host_calls(handler, || self.eval_value_mode(source, false));
        }
        self.eval_value_mode(source, false)
    }

    /// Evaluates once through the existing evaluator and returns a portable
    /// bounded Evaluation Journal.
    #[cfg(feature = "evaluation-journal")]
    pub fn eval_native_journal(&mut self, source: &str) -> journal::Journal {
        let journal_id = journal::JournalId(self.next_journal_id);
        self.next_journal_id += 1;
        let (_, journal) = core::with_evaluation_journal(
            journal_id,
            journal::JournalLimits::default(),
            || {
                self.refresh_qualified_bindings();
                let forms = kernel::parse_forms(source)?;
                let result = self.eval_forms(forms, true)?;
                self.save_namespace();
                self.refresh_qualified_bindings();
                Ok(result)
            },
            |value, collector| {
                collector.preview_value(core::portable_type_name(value), value.display())
            },
        );
        journal
    }

    #[cfg(feature = "evaluation-journal")]
    #[deprecated(note = "use eval_native_journal")]
    pub fn eval_native_trace(&mut self, source: &str) -> Result<journal::Journal, String> {
        let journal = self.eval_native_journal(source);
        match journal.status {
            journal::JournalStatus::Error => Err(journal.error.clone().unwrap_or_default()),
            _ => Ok(journal),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn add_extension_root(&mut self, root: impl Into<std::path::PathBuf>) {
        self.extension_roots.push(root.into());
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn install_discovered_extension(&mut self, namespace: &str) -> Result<(), String> {
        if self.wasm_extensions.contains_key(namespace) {
            return Ok(());
        }
        let package =
            native_extension::ExtensionPackage::discover(namespace, &self.extension_roots)?
                .ok_or_else(|| format!("extension/not-found: {namespace}"))?;
        if package.manifest.provider == "hta" {
            let target = package
                .manifest
                .targets
                .get("node")
                .ok_or_else(|| format!("extension/target-unsupported: node for {namespace}"))?;
            if target.runtime != "process" {
                return Err(format!(
                    "extension/target-unsupported: node for {namespace}"
                ));
            }
            let module = package.resolve(&target.module)?;
            let provider = process_extension::ProcessExtensionProvider::new(module);
            return self.install_wasm_extension(
                &package.source,
                &package.descriptor.display().to_string(),
                provider,
            );
        }
        if package.manifest.provider != "wasm" {
            return Err(format!(
                "extension/provider-unsupported: {} for {namespace}",
                package.manifest.provider
            ));
        }
        let bytes = package.module_bytes()?;
        let provider = match package.manifest.abi {
            extension::WasmAbi::CoreV1 => {
                wasmtime_provider::WasmtimeExtensionProvider::compile(&bytes)?
            }
            extension::WasmAbi::MemoryV1 => {
                let plan = package.memory_binding_plan(&bytes)?;
                wasmtime_provider::WasmtimeExtensionProvider::compile_memory(&bytes, plan)?
            }
            extension::WasmAbi::HtaV1 => {
                let package_manifest_path = package.root.join("package.edn");
                let package_manifest = package_manifest::PackageManifest::read(
                    &package_manifest_path,
                )
                .map_err(|error| error.to_string())?;
                let module = package_manifest
                    .wasm_imports
                    .keys()
                    .find(|candidate| {
                        candidate.as_str() == package.manifest.module.as_deref().unwrap_or_default()
                            || package_manifest
                                .wasm_imports
                                .get(*candidate)
                                .and_then(|variant| variant.artifact.path.file_name())
                                .and_then(|name| name.to_str())
                                == package
                                    .manifest
                                    .module
                                    .as_deref()
                                    .and_then(|module| {
                                        std::path::Path::new(module)
                                            .file_name()
                                            .and_then(|name| name.to_str())
                                    })
                    })
                    .cloned()
                    .or_else(|| {
                        (package_manifest.wasm_imports.len() == 1)
                            .then(|| package_manifest.wasm_imports.keys().next().cloned())
                            .flatten()
                    })
                    .ok_or_else(|| {
                        format!("package/missing-require-artifact: {namespace}")
                    })?;
                let mut requirements = package_manifest::PackageRuntimeRequirements {
                    supported_targets: ["wasm32-wasi-preview1".to_owned()].into_iter().collect(),
                    supported_abis: ["hta.v1".to_owned()].into_iter().collect(),
                    ..package_manifest::PackageRuntimeRequirements::default()
                };
                if self.native_host_handler.is_some() {
                    requirements.allowed_host_calls = package_manifest
                        .wasm_imports
                        .values()
                        .flat_map(|variant| variant.host_calls.iter().cloned())
                        .collect();
                }
                let loaded = package_hta_loader::load_hta_require_package(
                    &package_manifest,
                    &package.root,
                    &module,
                    &requirements,
                    &package.source,
                    self.native_host_handler.clone(),
                )?;
                if loaded.identity != package_manifest.identity {
                    return Err(format!("package/identity-mismatch: {namespace}"));
                }
                self.wasm_extensions.insert(namespace.to_owned(), loaded.extension);
                return Ok(());
            }
        };
        self.install_wasm_extension(
            &package.source,
            &package.descriptor.display().to_string(),
            provider,
        )
    }

    pub fn install_wasm_extension<P: extension::WasmExtensionProvider + 'static>(
        &mut self,
        manifest_source: &str,
        origin: &str,
        provider: P,
    ) -> Result<(), String> {
        let manifest = extension::ExtensionManifest::parse(manifest_source, origin)?;
        let namespace = manifest.namespace.clone();
        if self.wasm_extensions.contains_key(&namespace)
            || self.extensions.contains(&namespace)
            || self.resources.contains_key(&namespace)
        {
            return Err(format!(
                "extension/ambiguous: namespace already registered: {namespace}"
            ));
        }
        let extension = extension::WasmExtension::new(manifest, provider)?;
        self.wasm_extensions.insert(namespace, extension);
        Ok(())
    }

    /// Installs one package-verified raw WASM module behind a logical import
    /// coordinate. `:import` binds its exports directly and never creates an
    /// HTA or generated Hara namespace.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn install_direct_wasm_import(
        &mut self,
        logical: &str,
        bytes: &[u8],
    ) -> Result<(), String> {
        let compiled = wasmtime_provider::CompiledWasmModule::compile(bytes)?;
        let exports = compiled.direct_exports()?;
        self.install_direct_wasm_provider(logical, exports, compiled.provider())
    }

    #[cfg(target_arch = "wasm32")]
    fn install_direct_wasm_import_browser(
        &mut self,
        logical: &str,
        bytes: &[u8],
    ) -> Result<(), String> {
        let exports = crate::direct_wasm::exports(bytes)?;
        let provider = crate::browser_wasm_provider::BrowserWasmProvider::compile(bytes)?;
        self.install_direct_wasm_provider(logical, exports, provider)
    }

    fn install_direct_wasm_provider<P: extension::WasmExtensionProvider + 'static>(
        &mut self,
        logical: &str,
        exports: Vec<(String, extension::ExtensionExport)>,
        provider: P,
    ) -> Result<(), String> {
        if logical.is_empty() || logical.contains('/') {
            return Err(
                "native/import-invalid: logical import must be an unqualified symbol".into(),
            );
        }
        if self.native_wasm_imports.contains_key(logical) {
            return Err(format!("native/import-ambiguous: {logical}"));
        }
        if exports.is_empty() {
            return Err(format!(
                "native/export-missing: {logical} exports no functions"
            ));
        }
        let manifest = extension::ExtensionManifest {
            namespace: logical.into(),
            root: None,
            identity: None,
            version: "0.0.0".into(),
            provider: "wasm".into(),
            module: None,
            abi: extension::WasmAbi::CoreV1,
            targets: HashMap::new(),
            assets: Vec::new(),
            exports,
            capabilities: Vec::new(),
            host_calls: HashMap::new(),
            handle_tags: HashMap::new(),
        };
        let import = extension::WasmExtension::new(manifest, provider)?;
        self.native_wasm_imports.insert(logical.into(), import);
        Ok(())
    }

    fn bind_direct_wasm_imports(
        &mut self,
        config: &kernel::GeneratedNamespaceConfig,
    ) -> Result<(), String> {
        let namespace = self.namespace_registry.current();
        for (local, logical) in config.native_imports() {
            #[cfg(not(target_arch = "wasm32"))]
            if !self.native_wasm_imports.contains_key(logical) {
                self.install_discovered_wasm_import(logical)?;
            }
            let bindings = self
                .native_wasm_imports
                .get_mut(logical)
                .ok_or_else(|| format!("native/import-missing: {logical}"))?
                .require()
                .map_err(|error| format!("native/import-start: {logical} ({error})"))?;
            for binding in bindings {
                let path = format!("{local}/{}", binding.name);
                if namespace
                    .mappings()
                    .iter()
                    .any(|(_, var)| var.symbol().as_str() == path)
                {
                    return Err(format!("native/import-ambiguous: {path}"));
                }
                let arity = binding.specification.arguments.len();
                let function_path = path.clone();
                let diagnostic_path = function_path.clone();
                let function = core::native_function(&function_path, arity, move |arguments| {
                    binding.invoke(&arguments).map_err(|error| {
                        format!("native/invoke-failed: {diagnostic_path} ({error})")
                    })
                });
                namespace.map_var(
                    crate::lang::data::Symbol::parse(&path),
                    kernel::Var::new(path, function),
                );
            }
        }
        self.refresh_qualified_bindings();
        Ok(())
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn install_discovered_wasm_import(&mut self, logical: &str) -> Result<(), String> {
        let package = native_extension::ExtensionPackage::discover(
            logical,
            &self.extension_roots,
        )?
        .ok_or_else(|| format!("native/import-missing: {logical}"))?;
        let package_manifest_path = package.root.join("package.edn");
        if !package_manifest_path.is_file() {
            return Err(format!("native/import-missing: {logical}"));
        }
        let manifest = package_manifest::PackageManifest::read(&package_manifest_path)
            .map_err(|error| error.to_string())?;
        let requirements = package_manifest::PackageRuntimeRequirements {
            supported_targets: ["wasm32-wasi-preview1".to_owned()].into_iter().collect(),
            supported_abis: ["core.v1".to_owned()].into_iter().collect(),
            ..package_manifest::PackageRuntimeRequirements::default()
        };
        let loaded = package_wasm_loader::load_wasm_import_package(
            &manifest,
            &package.root,
            logical,
            &requirements,
            &package.source,
        )?;
        if loaded.identity != manifest.identity {
            return Err(format!("package/identity-mismatch: {logical}"));
        }
        self.native_wasm_imports
            .insert(logical.to_owned(), loaded.extension);
        Ok(())
    }

    pub fn cancel_wasm_extension(&self, name: &str, request: u64) -> Result<(), String> {
        self.wasm_extensions
            .get(name)
            .ok_or_else(|| format!("extension/not-found: {name}"))?
            .cancel(request)
    }

    /// Invokes an installed WASM extension without routing the call through
    /// source text. Service hosts use this binary-safe boundary for HTA0
    /// arguments and results.
    pub fn invoke_wasm_extension(
        &mut self,
        namespace: &str,
        export: &str,
        arguments: &[extension::Value],
    ) -> Result<extension::Value, String> {
        let binding = self
            .wasm_extensions
            .get_mut(namespace)
            .ok_or_else(|| format!("extension/not-found: {namespace}"))?
            .require()?
            .into_iter()
            .find(|binding| binding.name == export)
            .ok_or_else(|| format!("extension/export-missing: {namespace}/{export}"))?;
        binding.invoke(arguments)
    }

    fn namespace_source(&self) -> Rc<dyn Fn(&str) -> Option<core::NamespaceResource>> {
        let resources = self.resources.clone();
        #[cfg(feature = "bytecode-vm")]
        let bytecode_resources = self.bytecode_resources.clone();
        Rc::new(move |name: &str| {
            #[cfg(feature = "bytecode-vm")]
            if let Some((namespace_form, artifact)) = bytecode_resources.get(name) {
                return Some(core::NamespaceResource::Bytecode {
                    namespace_form: namespace_form.clone(),
                    artifact: artifact.clone(),
                });
            }
            resources
                .get(name)
                .cloned()
                .map(core::NamespaceResource::Source)
        })
    }

    fn load_wasm_extension_namespace(&mut self, name: &str) -> Result<String, String> {
        let bindings = self
            .wasm_extensions
            .get_mut(name)
            .ok_or_else(|| format!("extension/not-found: {name}"))?
            .require()?;
        let namespace = self.namespace_registry.find_or_create(name);
        for binding in bindings {
            let arity = binding.specification.arguments.len();
            let function_name = format!("{name}/{}", binding.name);
            let binding_name = binding.name.clone();
            namespace.intern(
                &binding_name,
                core::native_function(&function_name, arity, move |arguments| {
                    binding.invoke(&arguments)
                }),
            );
        }
        self.refresh_qualified_bindings();
        Ok(":loaded".into())
    }
}
