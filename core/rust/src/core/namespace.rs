fn previously_failed_error(registry: &NamespaceRegistry<Value>, namespace: &str) -> String {
    let mut message =
        format!("Namespace load previously failed; use explicit reload to retry: {namespace}");
    if let Some(detail) = registry.load_failure(namespace) {
        message.push_str(&format!(" (initial failure: {detail})"));
    }
    message
}

fn eval_source_form(
    namespace: &str,
    form: &crate::kernel::SpannedForm,
    env: &mut HashMap<String, Value>,
) -> Result<Value, String> {
    let site = ExceptionSite {
        namespace: Some(namespace.to_owned()),
        resource: None,
        line: form.span.start.line,
        column: form.span.start.column,
    };
    let form = attach_exception_sites(form);
    with_exception_site(site, || eval(&form, env))
}

fn load_source_namespace(
    name: &str,
    source: &str,
    registry: &NamespaceRegistry<Value>,
    env: &mut HashMap<String, Value>,
) -> Result<(), String> {
    let forms = crate::kernel::read_forms(source).map_err(|error| error.to_string())?;
    let mut start = 0;
    if forms
        .first()
        .is_some_and(|form| top_level_namespace_form(&form.form))
    {
        eval_source_form(name, &forms[0], env)
            .map_err(|error| format!("{name}: top-level form 1: {error}"))?;
        start = 1;
    }
    let declarations = forms[start..]
        .iter()
        .filter_map(|form| top_level_definition_name(&form.form))
        .map(|name| Form::Symbol(name.to_owned()))
        .collect::<Vec<_>>();
    if !declarations.is_empty() {
        let declaration = Form::List(
            std::iter::once(Form::Symbol("declare".into()))
                .chain(declarations)
                .collect(),
        );
        eval(&declaration, env)
            .map_err(|error| format!("{name}: top-level predeclaration: {error}"))?;
    }
    for (index, form) in forms.into_iter().enumerate().skip(start) {
        eval_source_form(name, &form, env)
            .map_err(|error| format!("{name}: top-level form {}: {error}", index + 1))?;
    }
    if registry.find(name).is_none() {
        return Err(format!(
            "Namespace source did not define expected namespace: {name}"
        ));
    }
    Ok(())
}

fn ensure_namespace(
    registry: &NamespaceRegistry<Value>,
    env: &mut HashMap<String, Value>,
    name: &str,
    reload: bool,
) -> Result<(), String> {
    // Resource replacement currently re-marks a namespace as `Unloaded`.
    // Preserve the sticky-failure contract by treating an accompanying
    // failure detail as `Failed` until an explicit reload succeeds.
    let load_state = match registry.load_state(name) {
        Some(NamespaceLoadState::Unloaded) if registry.load_failure(name).is_some() => {
            Some(NamespaceLoadState::Failed)
        }
        state => state,
    };
    match load_state {
        Some(NamespaceLoadState::Loaded) if !reload => return Ok(()),
        Some(NamespaceLoadState::Loading) => {
            return Err(format!("Cyclic namespace require: {name}"));
        }
        Some(NamespaceLoadState::Failed) if !reload => {
            return Err(previously_failed_error(registry, name));
        }
        _ => {}
    }

    let catalog = package_catalog();
    if let Some(coordinate) = catalog.coordinate_for_namespace(name) {
        if catalog.state(&coordinate).as_deref() != Some("ready") {
            return Err(format!(
                "package/not-installed: namespace is locked but unavailable: {name}; call Package/ensure first"
            ));
        }
    }

    let requiring = registry.current().name().as_str().to_owned();
    // A host resource replacement can mark a materialized namespace as
    // `Unloaded` so the next require considers the new source.  The old
    // namespace is still the rollback baseline, however, and a failed reload
    // must restore it as `Loaded` rather than making the prior generation
    // appear failed.
    let previous_state = match load_state {
        Some(NamespaceLoadState::Unloaded) if registry.find(name).is_some() => {
            Some(NamespaceLoadState::Loaded)
        }
        Some(state) => Some(state),
        None => registry.find(name).map(|_| NamespaceLoadState::Loaded),
    };
    let registry_before = registry.transaction_snapshot([requiring.as_str(), name]);
    // Qualified and aliased bindings are a derived namespace view. Snapshot
    // only unqualified bindings (including lexical locals), then rebuild the
    // derived view on rollback instead of cloning the full cross-namespace
    // environment before every successful require.
    let environment_before = env
        .iter()
        .filter(|(binding, _)| !binding.contains('/'))
        .map(|(binding, value)| (binding.clone(), value.clone()))
        .collect::<HashMap<_, _>>();
    let macros_before = ACTIVE_MACROS.with(|active| {
        active
            .borrow()
            .as_ref()
            .map(|macros| macros.borrow().clone())
    });
    registry.clear_module_dependencies(name);
    registry.set_load_state(name, NamespaceLoadState::Loading);

    let loaded = (|| {
        let resource = NAMESPACE_SOURCE_PROVIDER
            .with(|active| active.borrow().as_ref().and_then(|provider| provider(name)))
            .ok_or_else(|| format!("Cannot require missing namespace: {name}"))?;
        #[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
        let loaded_directly = if let Some(loader) = direct_native_namespace_loader() {
            loader(name, resource.clone(), env)?;
            true
        } else {
            false
        };
        #[cfg(not(all(feature = "direct-native", not(target_arch = "wasm32"))))]
        let loaded_directly = false;
        if !loaded_directly {
            let source = match &resource {
                NamespaceResource::Source(source) => Some(source.clone()),
                #[cfg(not(target_arch = "wasm32"))]
                NamespaceResource::SourcePath(_) => {
                    Some(crate::core::read_source_resource(&resource, name)?)
                }
                #[cfg(feature = "bytecode-vm")]
                NamespaceResource::Bytecode { .. } => None,
            };
            if let Some(source) = source {
                load_source_namespace(name, &source, registry, env)?;
            } else {
                #[cfg(feature = "bytecode-vm")]
                if let NamespaceResource::Bytecode {
                    namespace_form,
                    artifact,
                } = resource
                {
                    let forms = crate::kernel::read_forms(&namespace_form)
                        .map_err(|error| error.to_string())?;
                    for (index, form) in forms.into_iter().enumerate() {
                        eval_source_form(name, &form, env).map_err(|error| {
                            format!("{name}: namespace form {}: {error}", index + 1)
                        })?;
                    }
                    let program = Rc::new(crate::vm::decode_program(&artifact)?);
                    registry.set_current(name);
                    crate::vm::execute_program_with_globals(program, registry)
                        .map_err(|error| error.to_string())?;
                }
            }
        }
        if registry.find(name).is_none() {
            return Err(format!(
                "Namespace source did not define expected namespace: {name}"
            ));
        }
        Ok(())
    })();

    select_namespace_environment(registry, env, &requiring);
    if let Err(error) = loaded {
        *env = environment_before;
        registry.restore_transaction(registry_before);
        refresh_namespace_environment(registry, env);
        if previous_state == Some(NamespaceLoadState::Loaded) {
            // A failed reload must restore the complete previously loaded
            // boundary, including its observable load state and failure
            // marker.  The transaction snapshot restores namespace values,
            // but load state is maintained separately.
            registry.set_load_state(name, NamespaceLoadState::Loaded);
            registry.clear_load_failure(name);
        } else {
            registry.set_load_state(name, NamespaceLoadState::Failed);
            registry.set_load_failure(name, error.clone());
        }
        if let Some(saved) = macros_before {
            ACTIVE_MACROS.with(|active| {
                if let Some(macros) = active.borrow().as_ref() {
                    *macros.borrow_mut() = saved;
                }
            });
        }
        return Err(error);
    }

    registry.set_load_state(name, NamespaceLoadState::Loaded);
    registry.clear_load_failure(name);
    registry.commit_module_revision(name);
    Ok(())
}

fn ensure_foundation_namespace_for_symbol(
    registry: &NamespaceRegistry<Value>,
    env: &mut HashMap<String, Value>,
    symbol: &str,
) -> Result<(), String> {
    let Some((namespace, _)) = symbol.split_once('/') else {
        return Ok(());
    };
    if namespace.starts_with("std.foundation.")
        && registry.load_state(namespace) == Some(NamespaceLoadState::Unloaded)
    {
        ensure_namespace(registry, env, namespace, false)?;
    }
    Ok(())
}

fn top_level_namespace_form(form: &Form) -> bool {
    matches!(form_without_metadata(form), Form::List(values)
        if matches!(values.first(), Some(Form::Symbol(head)) if head == "ns" || head == "ns+"))
}

fn top_level_definition_name(form: &Form) -> Option<&str> {
    let form = match form {
        Form::Metadata(_, value) => value.as_ref(),
        value => value,
    };
    let Form::List(values) = form else {
        return None;
    };
    let head = match values.first()? {
        Form::Symbol(head) => head.as_str(),
        _ => return None,
    };
    if !matches!(head, "def" | "defonce" | "defn" | "defn-" | "defmacro") {
        return None;
    }
    match values.get(1)? {
        Form::Symbol(name) => Some(name),
        Form::Metadata(_, value) => match value.as_ref() {
            Form::Symbol(name) => Some(name),
            _ => None,
        },
        _ => None,
    }
}

pub fn require_namespace(
    registry: &NamespaceRegistry<Value>,
    env: &mut HashMap<String, Value>,
    name: &str,
) -> Result<(), String> {
    ensure_namespace(registry, env, name, false)
}

fn eval_require_spec(
    registry: &NamespaceRegistry<Value>,
    env: &mut HashMap<String, Value>,
    form: &Form,
) -> Result<(), String> {
    let (target, options) = match form {
        Form::Vector(items) => {
            let target = match items.first() {
                Some(Form::Symbol(target)) => target.clone(),
                _ => return Err("require namespace must be a symbol".into()),
            };
            (
                crate::kernel::generated::normalize_namespace(&target).to_owned(),
                &items[1..],
            )
        }
        Form::List(items)
            if items.len() == 2
                && matches!(&items[0], Form::Symbol(q) if q == "quote")
                && matches!(&items[1], Form::Symbol(_)) =>
        {
            let target = match &items[1] {
                Form::Symbol(target) => target.clone(),
                _ => unreachable!(),
            };
            (
                crate::kernel::generated::normalize_namespace(&target).to_owned(),
                &[][..],
            )
        }
        _ => return Err("require expects vectors such as [chrome.api :as api]".into()),
    };
    if options.len() % 2 != 0 {
        return Err(format!("Malformed require options for {target}"));
    }
    let lazy = options.chunks(2).any(|option| {
        matches!(&option[0], Form::Keyword(keyword) if keyword.as_str() == "lazy")
            && matches!(&option[1], Form::Bool(true))
    });
    let reload = options.chunks(2).any(|option| {
        matches!(&option[0], Form::Keyword(keyword) if keyword.as_str() == "reload")
            && matches!(&option[1], Form::Bool(true))
    });
    let excluded = options
        .chunks(2)
        .find_map(|option| {
            matches!(&option[0], Form::Keyword(keyword) if keyword.as_str() == "exclude")
                .then_some(&option[1])
        })
        .map(|value| match value {
            Form::Vector(names) => names
                .iter()
                .map(|name| match name {
                    Form::Symbol(name) if name == "/" || !name.contains('/') => Ok(name.clone()),
                    _ => Err("require :exclude expects unqualified symbols".to_string()),
                })
                .collect::<Result<HashSet<_>, _>>(),
            _ => Err("require :exclude expects a vector of symbols".into()),
        })
        .transpose()?
        .unwrap_or_default();
    if lazy {
        let has_alias = options
            .chunks(2)
            .any(|option| matches!(&option[0], Form::Keyword(keyword) if keyword.as_str() == "as"));
        if !has_alias {
            return Err("require :lazy requires :as".into());
        }
        for option in options.chunks(2) {
            match &option[0] {
                Form::Keyword(keyword)
                    if keyword.as_str() == "refer" || keyword.as_str() == "refer-macros" =>
                {
                    return Err(format!(
                        "require :lazy cannot be combined with :{}",
                        keyword
                    ));
                }
                Form::Keyword(keyword)
                    if keyword.as_str() == "lazy" && !matches!(&option[1], Form::Bool(true)) =>
                {
                    return Err("require :lazy expects true".into());
                }
                _ => {}
            }
        }
    }
    let deferred = lazy && !reload;
    #[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
    let direct_reload = !deferred
        && direct_native_namespace_loader().is_some()
        && namespace_has_interpreted_functions(registry, &target);
    #[cfg(not(all(feature = "direct-native", not(target_arch = "wasm32"))))]
    let direct_reload = false;
    if deferred {
        if registry.load_state(&target).is_none() {
            registry.set_load_state(&target, NamespaceLoadState::Unloaded);
        }
    } else if !crate::kernel::generated::known_namespace(&target) || direct_reload {
        ensure_namespace(registry, env, &target, reload || direct_reload)?;
    }
    let requiring = registry.current().name().as_str().to_owned();
    if requiring != target && registry.load_state(&requiring) == Some(NamespaceLoadState::Loading) {
        registry.record_module_dependency(&requiring, &target);
    }
    if !deferred {
        let destination = registry.current();
        for name in &excluded {
            let local = crate::lang::data::Symbol::parse(name);
            if destination
                .resolve(&local)
                .is_some_and(|var| var.symbol().get_namespace() == Some(target.as_str()))
            {
                destination.unmap(&local);
                env.remove(name);
            }
        }
    }
    for option in options.chunks(2) {
        let name = match &option[0] {
            Form::Keyword(keyword) => keyword.as_str(),
            _ => return Err("Malformed require options".into()),
        };
        match name {
            "as" => {
                let alias = match &option[1] {
                    Form::Symbol(alias) if !alias.contains('/') => alias.clone(),
                    _ => return Err("require :as expects an unqualified symbol".into()),
                };
                if alias == "-" {
                    return Err("Namespace alias is reserved: -".into());
                }
                // Clear a stale materialized Foundation binding before an
                // explicit alias claims the same local name.
                let local = crate::lang::data::Symbol::parse(&alias);
                if registry
                    .current()
                    .resolve(&local)
                    .is_some_and(|var| var.symbol().get_namespace() == Some("std.foundation"))
                {
                    registry.current().unmap(&local);
                    env.remove(&alias);
                }
                if deferred {
                    registry.current().lazy_alias(alias, &target);
                } else {
                    let namespace = registry
                        .find(&target)
                        .ok_or_else(|| format!("Cannot require missing namespace: {target}"))?;
                    registry.current().alias(alias, namespace);
                }
            }
            "refer" => {
                let source = registry
                    .find(&target)
                    .ok_or_else(|| format!("Cannot require missing namespace: {target}"))?;
                let destination = registry.current();
                let destination_name = destination.name().as_str().to_owned();
                let names = match &option[1] {
                    Form::Keyword(name) if name.as_str() == "all" => source
                        .mappings()
                        .into_iter()
                        .map(|(name, _)| name.as_str().to_owned())
                        .collect::<Vec<_>>(),
                    Form::Vector(names) => names
                        .iter()
                        .map(|name| match name {
                            Form::Symbol(name) if !name.contains('/') => Ok(name.clone()),
                            _ => Err("require :refer expects unqualified symbols".to_string()),
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                    _ => return Err("require :refer expects a vector of symbols or :all".into()),
                };
                for name in names {
                    if excluded.contains(&name) {
                        continue;
                    }
                    let var = source
                        .resolve(&crate::lang::data::Symbol::parse(&name))
                        .ok_or_else(|| format!("Cannot refer missing Var: {target}/{name}"))?;
                    destination.map_var(crate::lang::data::Symbol::parse(&name), var);
                    ACTIVE_MACROS.with(|active| {
                        if let Some(macros) = active.borrow().as_ref() {
                            let mut macros = macros.borrow_mut();
                            if let Some(function) =
                                macros.get(&(target.clone(), name.clone())).cloned()
                            {
                                macros.insert((destination_name.clone(), name.clone()), function);
                            }
                        }
                    });
                }
            }
            "refer-macros" => {
                let Form::Vector(names) = &option[1] else {
                    return Err("require :refer-macros expects a vector of symbols".into());
                };
                let destination = registry.current().name().as_str().to_owned();
                ACTIVE_MACROS.with(|active| -> Result<(), String> {
                    let active = active.borrow();
                    let macros = active
                        .as_ref()
                        .ok_or_else(|| "macro runtime is unavailable".to_string())?;
                    let mut macros = macros.borrow_mut();
                    for name in names {
                        let Form::Symbol(name) = name else {
                            return Err("require :refer-macros expects unqualified symbols".into());
                        };
                        if name.contains('/') {
                            return Err("require :refer-macros expects unqualified symbols".into());
                        }
                        let macro_fn = macros
                            .get(&(target.clone(), name.clone()))
                            .cloned()
                            .ok_or_else(|| {
                                format!("Cannot refer missing macro: {target}/{name}")
                            })?;
                        macros.insert((destination.clone(), name.clone()), macro_fn);
                    }
                    Ok(())
                })?;
            }
            "lazy" => {}
            "reload" => {
                if !matches!(&option[1], Form::Bool(true)) {
                    return Err("require :reload expects true".into());
                }
            }
            "exclude" => {}
            other => return Err(format!("Unsupported require option: :{other}")),
        }
    }
    Ok(())
}

fn eval_require_specs(
    registry: &NamespaceRegistry<Value>,
    env: &mut HashMap<String, Value>,
    specs: &[Form],
) -> Result<(), String> {
    for spec in specs {
        eval_require_spec(registry, env, spec)?;
    }
    refresh_namespace_environment(registry, env);
    Ok(())
}

#[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
fn namespace_has_interpreted_functions(registry: &NamespaceRegistry<Value>, name: &str) -> bool {
    registry.find(name).is_some_and(|namespace| {
        namespace.mappings().into_iter().any(|(_, var)| {
            matches!(var.deref_value(), Value::Function(function) if !is_direct_native_function(&function))
        })
    })
}

fn force_lazy_alias(
    registry: &NamespaceRegistry<Value>,
    env: &mut HashMap<String, Value>,
    symbol: &str,
) -> Result<(), String> {
    let Some((alias, _)) = symbol.split_once('/') else {
        return Ok(());
    };
    if registry.current().name().as_str() == alias {
        return Ok(());
    }
    let target = registry.current().lazy_target(alias);
    let Some(target) = target else {
        return Ok(());
    };
    ensure_namespace(registry, env, target.as_str(), false)?;
    let namespace = registry
        .find(target.as_str())
        .ok_or_else(|| format!("Cannot require missing namespace: {target}"))?;
    registry.current().alias(alias, namespace);
    refresh_namespace_environment(registry, env);
    Ok(())
}

/// Handles the `ns`, `ns+`, and `require` special forms.
///
/// Kept out of line so the giant `eval` dispatch does not reserve stack for
/// these locals on every recursive call (the native runtime recurses through
/// `eval` and test threads run on small stacks).
#[inline(never)]
fn eval_namespace_form(fs: &[Form], env: &mut HashMap<String, Value>) -> Result<Value, String> {
    let head = match &fs[0] {
        Form::Symbol(head) => head.as_str(),
        _ => unreachable!("ns/ns+/require dispatch guarantees a symbol head"),
    };
    if head == "require" {
        let registry = namespace_registry()?;
        eval_require_specs(&registry, env, &fs[1..])?;
        return Ok(Value::Nil);
    }
    let registry = namespace_registry()?;
    let (name, clauses) = if head == "ns+" {
        if matches!(fs.get(1), Some(Form::Symbol(_))) {
            return Err("ns+ does not accept a namespace name".into());
        }
        (registry.current().name().as_str().to_owned(), &fs[1..])
    } else {
        if fs.len() < 2 {
            return Err("ns expects a namespace symbol".into());
        }
        let name = match &fs[1] {
            Form::Symbol(name) if !name.contains('/') => name.clone(),
            _ => return Err("ns expects a namespace symbol".into()),
        };
        (name, &fs[2..])
    };
    // Namespace configuration is normally consumed by the generated-runtime
    // orchestration layer. The raw HTA evaluator executes forms directly in
    // an EvalFiber, so the core special form must still honor namespace
    // construction settings, including global aliases and imports. Foundation
    // child-library aliases remain distinct from native runtime symbols.
    let config = crate::kernel::GeneratedNamespaceConfig::configure_with(clauses, |_| true)?;
    if let Some(alias) = config.global_alias() {
        registry.register_global_alias(alias, &name)?;
    }
    for alias in config.declared_global_imports() {
        let canonical =
            crate::core::canonical_native_symbol(alias).unwrap_or_else(|| alias.clone());
        registry.register_global_import(alias, canonical)?;
    }
    apply_global_aliases(&registry, &name);
    crate::core::apply_global_imports(&registry, &name);
    select_namespace_environment(&registry, env, &name);
    let destination = registry.current();
    destination.set_role(config.role());
    destination.set_foundation_visibility(
        config.exposed_foundation(),
        config.excluded_foundation(),
        config.blank(),
    );
    destination.set_native_flavor(config.native_flavor().map(str::to_owned));
    for (local, module) in config.native_imports() {
        destination.import(local, module.clone());
    }
    for (alias, _) in registry.global_aliases() {
        destination.unalias(alias.as_str());
    }
    for (alias, target) in config.aliases() {
        if !target.starts_with("std.foundation.") {
            continue;
        }
        if let Some(namespace) = registry.find(&target) {
            destination.alias(alias, namespace);
        } else {
            destination.lazy_alias(alias, target);
        }
    }
    let omitted = match config.exposed_foundation() {
        Some(exposed) => destination
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
    for overridden in omitted {
        let destination = registry.current();
        let local = crate::lang::data::Symbol::parse(&overridden);
        if destination
            .resolve(&local)
            .is_some_and(|var| var.symbol().get_namespace() == Some("std.foundation"))
        {
            destination.unmap(&local);
            env.remove(&overridden);
        }
        let destination_name = destination.name().as_str().to_owned();
        ACTIVE_MACROS.with(|active| {
            if let Some(macros) = active.borrow().as_ref() {
                macros
                    .borrow_mut()
                    .remove(&(destination_name, overridden.clone()));
            }
        });
    }
    for (alias, target) in destination.aliases() {
        let excluded = config
            .excluded_foundation_libraries()
            .iter()
            .any(|library| target.name().as_str() == format!("std.foundation.{library}"));
        if excluded {
            destination.unalias(alias.as_str());
        }
    }
    for (alias, target) in destination.lazy_aliases() {
        let excluded = config
            .excluded_foundation_libraries()
            .iter()
            .any(|library| target.as_str() == format!("std.foundation.{library}"));
        if excluded {
            destination.unalias(alias.as_str());
        }
    }
    for library in config.excluded_foundation_libraries() {
        if let Some(alias) = crate::kernel::generated::foundation_library_alias(library) {
            destination.unalias(alias);
        }
    }
    for clause in clauses {
        match clause {
            Form::List(clause_forms) if matches!(clause_forms.first(), Some(Form::Keyword(k)) if k == "require") =>
            {
                eval_require_specs(&registry, env, &clause_forms[1..])?;
            }
            Form::List(clause_forms) if matches!(clause_forms.first(), Some(Form::Keyword(k)) if k == "use") =>
            {
                let specs = clause_forms[1..]
                    .iter()
                    .map(|namespace| match namespace {
                        Form::Symbol(name) if !name.contains('/') => Ok(Form::Vector(vec![
                            Form::Symbol(name.clone()),
                            Form::Keyword("refer".into()),
                            Form::Keyword("all".into()),
                        ])),
                        _ => Err("ns :use expects namespace symbols".to_string()),
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                eval_require_specs(&registry, env, &specs)?;
            }
            Form::List(clause_forms) if matches!(clause_forms.first(), Some(Form::Keyword(k)) if k == "config") =>
            {
                // :config is processed by the generated-namespace machinery for
                // top-level ns forms. For ns forms loaded from source files (e.g.
                // runtime-library activation declarations), it is metadata-only
                // and can be ignored here.
            }
            Form::List(clause_forms) if matches!(clause_forms.first(), Some(Form::Keyword(k)) if k == "flavor" || k == "import") =>
                {}
            _ => return Err("unsupported ns clause in evaluator".into()),
        }
    }
    refresh_namespace_environment(&registry, env);
    Ok(Value::Nil)
}

/// Applies a top-level namespace declaration before bytecode compilation.
///
/// Namespace selection and configuration affect how every later global is
/// resolved, so the bytecode compiler performs this analysis-time step before
/// it creates its compilation context. The emitted form still evaluates to
/// nil; only the namespace registry is prepared here.
pub(crate) fn prepare_namespace_form(form: &Form) -> Result<(), String> {
    let Form::List(forms) = form_without_metadata(form) else {
        return Err("namespace declaration must be a list".into());
    };
    if !matches!(forms.first(), Some(Form::Symbol(head)) if head == "ns" || head == "ns+") {
        return Err("namespace declaration must start with ns or ns+".into());
    }
    eval_namespace_form(forms, &mut HashMap::new()).map(|_| ())
}

/// Applies a namespace-management form retained in a validated bytecode
/// constant. This is intentionally separate from `eval_bytecode_declaration`:
/// `ns`, `ns+`, and `require` need the namespace registry's management
/// semantics, but must not re-enter the tree evaluator when a VM or direct
/// native program executes them.
pub(crate) fn eval_bytecode_management(value: &Value) -> Result<Value, String> {
    let registry = namespace_registry()?;
    let mut environment = registry
        .current()
        .mappings()
        .into_iter()
        .map(|(name, var)| (name.as_str().to_owned(), Value::Var(var)))
        .collect();
    refresh_namespace_environment(&registry, &mut environment);
    let result = eval_bytecode_management_in(value, &mut environment)?;
    save_namespace_environment(&registry, &mut environment);
    Ok(result)
}

/// Applies a validated namespace-management value against a caller-owned
/// compatibility environment. Namespace loaders use this form so a direct
/// native frame can prepare `ns`/`require` without entering the tree
/// evaluator. The environment is intentionally borrowed: selecting a module
/// must save the requiring namespace before switching to the loaded one.
pub(crate) fn eval_bytecode_management_in(
    value: &Value,
    environment: &mut HashMap<String, Value>,
) -> Result<Value, String> {
    let form = value_to_form(value)?;
    let Form::List(forms) = form_without_metadata(&form) else {
        return Err("namespace-management instruction expects a list".into());
    };
    let Some(Form::Symbol(operator)) = forms.first() else {
        return Err("namespace-management instruction expects a symbol operator".into());
    };
    if !matches!(operator.as_str(), "ns" | "ns+" | "require") {
        return Err(format!(
            "namespace-management instruction does not support {operator}"
        ));
    }
    eval_namespace_form(forms, environment)
}
