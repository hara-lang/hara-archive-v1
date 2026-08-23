pub(crate) fn direct_callable_spec(name: &str) -> Option<&'static DirectCallableSpec> {
    DIRECT_CALLABLE_CATALOG
        .iter()
        .find(|specification| specification.symbol == name)
}

pub(crate) fn validate_direct_callable_catalog() -> Result<(), String> {
    let mut inventory = std::collections::BTreeSet::new();
    let mut duplicate_inventory = Vec::new();
    for symbol in RUNTIME_CALLABLE_INVENTORY {
        if !inventory.insert(*symbol) {
            duplicate_inventory.push(*symbol);
        }
    }

    let mut catalog = std::collections::BTreeSet::new();
    let mut duplicate_catalog = Vec::new();
    for specification in DIRECT_CALLABLE_CATALOG {
        match specification.availability {
            DirectCallableAvailability::AllTargets => {}
        }
        if !catalog.insert(specification.symbol) {
            duplicate_catalog.push(specification.symbol);
        }
    }

    let missing = inventory.difference(&catalog).copied().collect::<Vec<_>>();
    let extra = catalog.difference(&inventory).copied().collect::<Vec<_>>();
    if duplicate_inventory.is_empty()
        && duplicate_catalog.is_empty()
        && missing.is_empty()
        && extra.is_empty()
    {
        return Ok(());
    }

    Err(format!(
        "runtime callable inventory/catalog mismatch: missing={missing:?}; extra={extra:?}; duplicate-inventory={duplicate_inventory:?}; duplicate-catalog={duplicate_catalog:?}"
    ))
}

pub(crate) fn direct_callable_values() -> Result<Vec<(&'static str, Value)>, String> {
    validate_direct_callable_catalog()?;
    DIRECT_CALLABLE_CATALOG
        .iter()
        .map(|specification| {
            direct_callable_value(specification.symbol)
                .map(|value| (specification.symbol, value))
                .ok_or_else(|| {
                    format!(
                        "direct callable catalog entry has no implementation: {}",
                        specification.symbol
                    )
                })
        })
        .collect()
}

pub(crate) fn direct_callable_value(name: &str) -> Option<Value> {
    let specification = direct_callable_spec(name)?;
    match specification.implementation {
        DirectCallableImplementation::Basic => direct_function_value(name),
        DirectCallableImplementation::Exception => exception_function_values()
            .into_iter()
            .find_map(|(candidate, value)| (candidate == name).then_some(value)),
        DirectCallableImplementation::Runtime(DirectRuntimeCallable::Deref) => {
            Some(direct_deref_callable())
        }
        DirectCallableImplementation::Runtime(_) | DirectCallableImplementation::Operation(_) => {
            let specification = *specification;
            Some(match specification.arity {
                DirectCallableArity::Exact(arity) => {
                    native_function(specification.symbol, arity, move |arguments| {
                        invoke_direct_callable(&specification, arguments)
                    })
                }
                _ => native_variadic_function(specification.symbol, move |arguments| {
                    invoke_direct_callable(&specification, arguments)
                }),
            })
        }
    }
}

fn invoke_direct_callable(
    specification: &DirectCallableSpec,
    arguments: Vec<Value>,
) -> Result<Value, String> {
    if !specification.arity.accepts(arguments.len()) {
        return Err(format!(
            "{} expects {} arguments, got {}",
            specification.symbol,
            specification.arity.description(),
            arguments.len()
        ));
    }
    match specification.implementation {
        DirectCallableImplementation::Runtime(implementation) => {
            invoke_direct_runtime_callable(implementation, arguments)
        }
        DirectCallableImplementation::Operation(implementation) => {
            implementation(specification, arguments)
        }
        DirectCallableImplementation::Basic | DirectCallableImplementation::Exception => {
            Err(format!(
                "{} is not a catalog-wrapped operation",
                specification.symbol
            ))
        }
    }
}

fn invoke_direct_runtime_callable(
    implementation: DirectRuntimeCallable,
    arguments: Vec<Value>,
) -> Result<Value, String> {
    match implementation {
        DirectRuntimeCallable::AlterVarRoot => direct_alter_var_root(arguments),
        DirectRuntimeCallable::Array => apply_primitive(Primitive::ArrayNew, &arguments),
        DirectRuntimeCallable::Bytes => native_bytes_operation("new", arguments),
        DirectRuntimeCallable::Capture => native_printer_values("capture", arguments),
        DirectRuntimeCallable::Compose2 | DirectRuntimeCallable::Compose3 => {
            direct_composition(arguments)
        }
        DirectRuntimeCallable::Conj => direct_conj(arguments),
        DirectRuntimeCallable::Cons => protocol_cons(&[arguments[1].clone(), arguments[0].clone()]),
        DirectRuntimeCallable::CurrentNamespace => Ok(Value::String(
            namespace_registry()?.current().name().as_str().to_owned(),
        )),
        DirectRuntimeCallable::Deref => {
            unreachable!("deref installs a fiber-aware direct callable")
        }
        DirectRuntimeCallable::Dissoc => collection_dissoc(&arguments[0], &arguments[1..]),
        DirectRuntimeCallable::Empty => collection_empty_value(arguments[0].clone()),
        DirectRuntimeCallable::Iter => native_iter_operation("iter", arguments),
        DirectRuntimeCallable::IterNext => native_iter_operation("iter-next", arguments),
        DirectRuntimeCallable::IterNextPredicate => native_iter_operation("iter-next?", arguments),
        DirectRuntimeCallable::IterPredicate => native_iter_operation("iter?", arguments),
        DirectRuntimeCallable::List => Ok(Value::List(arguments.into())),
        DirectRuntimeCallable::LoadString => direct_load_string(arguments),
        DirectRuntimeCallable::Name => direct_name(arguments),
        DirectRuntimeCallable::Namespace => protocol_namespaced_namespace(&arguments),
        DirectRuntimeCallable::NamespaceLoaded => direct_namespace_state(arguments, true),
        DirectRuntimeCallable::NamespaceState => direct_namespace_state(arguments, false),
        DirectRuntimeCallable::NamespaceCreate => direct_namespace_create(arguments),
        DirectRuntimeCallable::Object => direct_object(arguments),
        DirectRuntimeCallable::Peek => collection_first(arguments[0].clone()),
        DirectRuntimeCallable::Print => native_printer_values("p", arguments),
        DirectRuntimeCallable::PrintRepresentation => Ok(Value::String(arguments[0].display())),
        DirectRuntimeCallable::Println => native_printer_values("println", arguments),
        DirectRuntimeCallable::Promise => native_promise_values("run", arguments),
        DirectRuntimeCallable::PromisePredicate => native_promise_values("instance?", arguments),
        DirectRuntimeCallable::ReadString => match arguments.as_slice() {
            [Value::String(source)] => read_edn(source),
            _ => Err("read-string expects one string".into()),
        },
        DirectRuntimeCallable::Resolve => direct_resolve(arguments),
        DirectRuntimeCallable::Seq => direct_seq(arguments),
        DirectRuntimeCallable::SeqPredicate => {
            Ok(Value::Bool(matches!(arguments[0], Value::Seq(_))))
        }
        DirectRuntimeCallable::String => Ok(direct_string(arguments)),
        DirectRuntimeCallable::Tuple => Ok(Value::Tuple(Box::new(PTuple::from_values(arguments)?))),
        DirectRuntimeCallable::Type => Ok(Value::Keyword(portable_type_keyword(&arguments[0])?)),
        DirectRuntimeCallable::WithMeta => protocol_with_meta(&arguments),
    }
}

fn direct_alter_var_root(arguments: Vec<Value>) -> Result<Value, String> {
    let Value::Var(target) = &arguments[0] else {
        return Err("alter-var-root expects a var".into());
    };
    let Value::Function(function) = &arguments[1] else {
        return Err("alter-var-root expects a function".into());
    };
    let mut call_arguments = vec![target.deref_value()];
    call_arguments.extend(arguments[2..].iter().cloned());
    let value = call_function(function, call_arguments)?;
    target.reset_value(value.clone());
    Ok(value)
}

fn direct_composition(arguments: Vec<Value>) -> Result<Value, String> {
    if arguments
        .iter()
        .any(|value| !matches!(value, Value::Function(_)))
    {
        return Err("composition expects functions".into());
    }
    let functions = Rc::new(arguments);
    Ok(native_function("composition", 1, move |mut values| {
        let mut value = values.remove(0);
        for function in functions.iter().rev() {
            value = call_value(function.clone(), vec![value])?;
        }
        Ok(value)
    }))
}

fn direct_conj(arguments: Vec<Value>) -> Result<Value, String> {
    let mut collection = arguments[0].clone();
    for item in &arguments[1..] {
        collection = protocol_conj(&[collection, item.clone()])?;
    }
    Ok(collection)
}

fn direct_deref_value(value: &Value) -> Result<Value, String> {
    match value {
        Value::Var(var) => Ok(var.deref_value()),
        Value::Atom(atom) => Ok(atom.deref_value()),
        Value::Promise(promise) => match promise.state() {
            PromiseState::Fulfilled(value) => Ok(value),
            PromiseState::Rejected(error) => Err(promise_rejection_error(error)),
            PromiseState::Pending => {
                Err("deref cannot block on a pending promise outside an HTA fiber".into())
            }
        },
        Value::Pointer(pointer) => {
            pointer_context_call(pointer, pointer_default(pointer)?, "pointer/deref", &[])
        }
        Value::Schema(schema) => form_to_value(&crate::lang::protocol::IDeref::deref(&schema.ast)),
        Value::Result(result) => result.deref_value(),
        value => Err(format!(
            "deref expects a var, atom, promise, pointer, result, or schema, got {}",
            value.display()
        )),
    }
}

fn direct_deref_callable() -> Value {
    native_fiber_function(
        "deref",
        1,
        false,
        |arguments| direct_deref_value(&arguments[0]),
        |arguments, continuation| {
            let value = arguments[0].clone();
            match value {
                Value::Promise(promise) => match promise.state() {
                    PromiseState::Fulfilled(value) => continuation(Ok(value)),
                    PromiseState::Rejected(error) => {
                        continuation(Err(promise_rejection_error(error)))
                    }
                    PromiseState::Pending => Step::Wait(
                        promise,
                        Box::new(move |state| match state {
                            PromiseState::Fulfilled(value) => continuation(Ok(value)),
                            PromiseState::Rejected(error) => {
                                continuation(Err(promise_rejection_error(error)))
                            }
                            PromiseState::Pending => {
                                continuation(Err("fiber resumed pending".into()))
                            }
                        }),
                    ),
                },
                value => continuation(direct_deref_value(&value)),
            }
        },
    )
}

fn direct_load_string(arguments: Vec<Value>) -> Result<Value, String> {
    let [Value::String(source)] = arguments.as_slice() else {
        return Err("load-string expects a string".into());
    };
    let mut environment = current_namespace_environment()?;
    eval_value_text(source, &mut environment)
}

fn direct_name(arguments: Vec<Value>) -> Result<Value, String> {
    match &arguments[0] {
        Value::Keyword(value) => Ok(Value::String(value.get_name().into())),
        Value::Symbol(value) => Ok(Value::String(value.get_name().into())),
        _ => Err("name expects a keyword or symbol".into()),
    }
}

fn direct_namespace_identifier(value: &Value, operation: &str) -> Result<String, String> {
    match value {
        Value::Symbol(name) if name.get_namespace().is_none() => Ok(name.as_str().to_owned()),
        Value::String(name) => Ok(name.clone()),
        _ => Err(format!("{operation} expects a namespace symbol or string")),
    }
}

fn direct_namespace_state(arguments: Vec<Value>, loaded_only: bool) -> Result<Value, String> {
    let operation = if loaded_only {
        "ns-loaded?"
    } else {
        "ns-state"
    };
    let name = direct_namespace_identifier(&arguments[0], operation)?;
    let registry = namespace_registry()?;
    let state = registry
        .load_state(&name)
        .or_else(|| registry.find(&name).map(|_| NamespaceLoadState::Loaded));
    if loaded_only {
        Ok(Value::Bool(state == Some(NamespaceLoadState::Loaded)))
    } else {
        Ok(Value::Keyword(
            state
                .map(NamespaceLoadState::as_str)
                .unwrap_or("unknown")
                .into(),
        ))
    }
}

fn direct_namespace_create(arguments: Vec<Value>) -> Result<Value, String> {
    let [Value::Symbol(name)] = arguments.as_slice() else {
        return Err("ns:create expects an unqualified symbol".into());
    };
    if name.get_namespace().is_some() {
        return Err("ns:create expects an unqualified symbol".into());
    }
    Ok(Value::Namespace(Rc::new(
        namespace_registry()?.find_or_create(name.as_str()),
    )))
}

fn direct_object(arguments: Vec<Value>) -> Result<Value, String> {
    let mut values = Vec::with_capacity(arguments.len() / 2);
    for pair in arguments.chunks_exact(2) {
        values.push((marker_key(&pair[0], "object")?, pair[1].clone()));
    }
    Ok(Value::Object(Rc::new(RefCell::new(values))))
}

fn direct_resolve(arguments: Vec<Value>) -> Result<Value, String> {
    let [Value::Symbol(symbol)] = arguments.as_slice() else {
        return Err("resolve expects a symbol".into());
    };
    let registry = namespace_registry()?;
    let mut environment = current_namespace_environment()?;
    force_lazy_alias(&registry, &mut environment, symbol.path_string().as_str())?;
    Ok(registry
        .resolve(symbol)
        .map(Value::Var)
        .unwrap_or(Value::Nil))
}

fn direct_seq(arguments: Vec<Value>) -> Result<Value, String> {
    let source = arguments
        .last()
        .cloned()
        .expect("catalog requires at least one seq argument");
    let sequence = iterator_seq(source)?;
    if arguments.len() == 1 {
        return Ok(sequence);
    }
    let Value::Function(function) = &arguments[0] else {
        return Err("seq expects a function and source".into());
    };
    iterator_seq(call_function(function, vec![sequence])?)
}

fn direct_string(arguments: Vec<Value>) -> Value {
    Value::String(
        arguments
            .iter()
            .map(|value| match value {
                Value::Nil => String::new(),
                Value::String(text) => text.clone(),
                Value::Character(character) => character.to_string(),
                _ => value.display(),
            })
            .collect::<Vec<_>>()
            .join(""),
    )
}

fn current_namespace_environment() -> Result<HashMap<String, Value>, String> {
    let registry = namespace_registry()?;
    let mut environment = registry
        .current()
        .mappings()
        .into_iter()
        .map(|(name, var)| (name.as_str().to_owned(), Value::Var(var)))
        .collect::<HashMap<_, _>>();
    refresh_namespace_environment(&registry, &mut environment);
    Ok(environment)
}

#[cfg(feature = "bytecode-vm")]
pub(crate) fn direct_bootstrap_callable_value(name: &str) -> Option<Value> {
    if let Some(value) = direct_callable_value(name) {
        return Some(value);
    }
    // Compiler-generated destructuring uses the canonical Foundation owner
    // to avoid capture by guest locals. It is a qualified reference, not a
    // guest namespace alias, so resolve its ordinary callable by local name.
    if let Some(local) = name.strip_prefix("std.foundation/") {
        if let Some(value) = direct_callable_value(local) {
            return Some(value);
        }
    }
    match name {
        "gensym" => Some(native_variadic_function("gensym", |arguments| {
            direct_gensym_value(arguments)
        })),
        "macroexpand-1" => Some(native_function("macroexpand-1", 1, |arguments| {
            direct_macroexpand_one_value(arguments)
        })),
        "ns-publics" => Some(native_function("ns-publics", 1, |arguments| {
            direct_namespace_publics_value(arguments)
        })),
        _ => None,
    }
}

#[cfg(feature = "bytecode-vm")]
pub(crate) fn bytecode_callable_value(name: &str) -> Result<Value, String> {
    if let Some(value) = direct_bootstrap_callable_value(name) {
        return Ok(value);
    }
    if let Some(primitive) = Primitive::from_symbol(name) {
        return Ok(native_variadic_function(name, move |arguments| {
            apply_primitive(primitive, &arguments)
        }));
    }
    if let Some(value) = direct_protocol_predicate_function_value(name) {
        return Ok(value);
    }
    if name == "disj" {
        let specification = DirectCallableSpec {
            symbol: "disj",
            arity: DirectCallableArity::AtLeast(2),
            availability: DirectCallableAvailability::AllTargets,
            origin: DirectCallableOrigin::RuntimePrimitive,
            implementation: DirectCallableImplementation::Runtime(DirectRuntimeCallable::Dissoc),
        };
        return Ok(native_variadic_function("disj", move |arguments| {
            invoke_direct_callable(&specification, arguments)
        }));
    }
    if let Some((namespace, method)) = name.rsplit_once('/') {
        if let Some(native_type) = namespace.strip_prefix("std.native.") {
            return native_type_function_value(native_type, method);
        }
    }
    if let Ok(registry) = namespace_registry() {
        let symbol = Symbol::parse(name);
        let resolved = registry
            .resolve(&symbol)
            .or_else(|| registry.current().resolve(&symbol));
        if let Some(var) = resolved {
            let value = var.deref_value();
            if matches!(value, Value::Function(_)) {
                return Ok(value);
            }
        }
    }
    match name.rsplit_once('/').map_or(name, |(_, local)| local) {
        "special-symbol?" => Ok(native_function(
            "special-symbol?",
            1,
            |arguments| match &arguments[0] {
                Value::Symbol(symbol) => Ok(Value::Bool(syntax_symbol(symbol.as_str()))),
                _ => Err("special-symbol? expects a symbol".into()),
            },
        )),
        "the-ns" => Ok(native_function("the-ns", 1, |arguments| {
            direct_the_namespace(&arguments[0])
        })),
        "ns-name" => Ok(native_function("ns-name", 1, |arguments| {
            direct_namespace_name(&arguments[0])
        })),
        _ => Err(format!("missing direct bytecode callable: {name}")),
    }
}

#[cfg(feature = "bytecode-vm")]
fn direct_gensym_value(arguments: Vec<Value>) -> Result<Value, String> {
    let prefix = match arguments.as_slice() {
        [] => "G__",
        [Value::String(prefix)] => prefix.as_str(),
        [value] => {
            return Err(format!(
                "gensym expects a string prefix, got {}",
                portable_type_name(value)
            ))
        }
        _ => return Err("gensym expects zero or one arguments".into()),
    };
    Ok(Value::Symbol(Symbol::from(gensym(prefix))))
}

#[cfg(feature = "bytecode-vm")]
fn direct_macroexpand_one_value(arguments: Vec<Value>) -> Result<Value, String> {
    let form = value_to_form(&arguments[0])?;
    let mut environment = current_namespace_environment()?;
    form_to_value(&macroexpand_once(&form, &mut environment)?)
}

#[cfg(feature = "bytecode-vm")]
fn direct_namespace_publics_value(arguments: Vec<Value>) -> Result<Value, String> {
    let namespace = match &arguments[0] {
        Value::Symbol(name) if name.get_namespace().is_none() => name.as_str().to_owned(),
        Value::String(name) => name.clone(),
        Value::Namespace(namespace) => namespace.name().as_str().to_owned(),
        _ => return Err("ns-publics expects a namespace symbol or string".into()),
    };
    let target = namespace_registry()?
        .find(&namespace)
        .ok_or_else(|| format!("No such namespace: {namespace}"))?;
    let mut mappings = target.mappings();
    mappings.retain(|(_, var)| var.symbol().get_namespace() == Some(namespace.as_str()));
    mappings.sort_by(|(left, _), (right, _)| left.as_str().cmp(right.as_str()));
    Ok(Value::OrderedMap(Box::new(POrderedMap::from_iter(
        mappings
            .into_iter()
            .map(|(name, var)| (Value::Symbol(Symbol::parse(name.as_str())), Value::Var(var))),
    ))))
}

#[cfg(feature = "bytecode-vm")]
fn direct_the_namespace(value: &Value) -> Result<Value, String> {
    let Value::Symbol(name) = value else {
        return Err("the-ns expects an unqualified symbol".into());
    };
    if name.get_namespace().is_some() {
        return Err("the-ns expects an unqualified symbol".into());
    }
    Ok(namespace_registry()?
        .find(name.as_str())
        .map(|namespace| Value::Namespace(Rc::new(namespace)))
        .unwrap_or(Value::Nil))
}

#[cfg(feature = "bytecode-vm")]
fn direct_namespace_name(value: &Value) -> Result<Value, String> {
    match value {
        Value::Namespace(namespace) => Ok(Value::Symbol(namespace.name().clone())),
        Value::Symbol(name)
            if name.get_namespace().is_none()
                && namespace_registry()?.find(name.as_str()).is_some() =>
        {
            Ok(Value::Symbol(name.clone()))
        }
        _ => Err("ns-name expects a namespace".into()),
    }
}
