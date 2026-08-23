fn direct_apply_operation(
    _specification: &DirectCallableSpec,
    mut arguments: Vec<Value>,
) -> Result<Value, String> {
    let callable = arguments.remove(0);
    let tail = arguments
        .pop()
        .expect("apply catalog requires a final argument collection");
    arguments.extend(iterator_values(tail)?);
    match callable {
        Value::Var(var) => call_value(var.deref_value(), arguments),
        value => call_value(value, arguments),
    }
}

fn direct_bit_operation(
    specification: &DirectCallableSpec,
    arguments: Vec<Value>,
) -> Result<Value, String> {
    bit_values(specification.symbol, &arguments)
}

fn direct_numeric_operation(
    specification: &DirectCallableSpec,
    arguments: Vec<Value>,
) -> Result<Value, String> {
    apply_binary_primitive(
        if specification.symbol == "inc" {
            Primitive::Add
        } else {
            Primitive::Subtract
        },
        &arguments[0],
        &Value::Number(1),
    )
}

fn direct_predicate_operation(
    specification: &DirectCallableSpec,
    arguments: Vec<Value>,
) -> Result<Value, String> {
    if specification.symbol == "satisfies?" {
        let Value::Protocol(protocol) = &arguments[0] else {
            return Err("satisfies? expects a protocol and value".into());
        };
        return Ok(Value::Bool(protocol_satisfies(protocol, &arguments[1])));
    }

    let value = &arguments[0];
    let result = match specification.symbol {
        "boolean?" => matches!(value, Value::Bool(_)),
        "char?" => matches!(value, Value::Character(_)),
        "double?" => matches!(value, Value::Float(_)),
        "even?" | "odd?" | "pos?" | "neg?" | "zero?" => {
            let Value::Number(number) = value else {
                return Err(format!("{} expects a number", specification.symbol));
            };
            match specification.symbol {
                "even?" => number % 2 == 0,
                "odd?" => number % 2 != 0,
                "pos?" => *number > 0,
                "neg?" => *number < 0,
                "zero?" => *number == 0,
                _ => unreachable!(),
            }
        }
        "false?" => matches!(value, Value::Bool(false)),
        "fn?" => named_protocol_satisfies("fn?", value),
        "function?" => matches!(value, Value::Function(_)),
        "instance?" => return named_instance_of(&arguments[0], &arguments[1]),
        "keyword?" => matches!(value, Value::Keyword(_)),
        "list?" => matches!(value, Value::List(_)),
        "long?" => numeric::to_i64_exact(value).is_ok(),
        "map?" => match value {
            Value::Map(_)
            | Value::OrderedMap(_)
            | Value::SortedMap(_)
            | Value::Trie(_)
            | Value::PriorityMap(_) => true,
            Value::Extension(receiver) => extension_has_category(receiver, "map"),
            _ => false,
        },
        "nil?" => matches!(value, Value::Nil),
        "number?" => numeric::is_numeric_value(value),
        "set?" => matches!(
            value,
            Value::Set(_) | Value::OrderedSet(_) | Value::SortedSet(_)
        ),
        "string?" => matches!(value, Value::String(_)),
        "symbol?" => matches!(value, Value::Symbol(_)),
        "true?" => matches!(value, Value::Bool(true)),
        "vector?" => matches!(value, Value::Vector(_) | Value::Tuple(_)),
        operation => {
            return Err(format!(
                "missing direct predicate implementation: {operation}"
            ))
        }
    };
    Ok(Value::Bool(result))
}

fn direct_promise_operation(
    specification: &DirectCallableSpec,
    arguments: Vec<Value>,
) -> Result<Value, String> {
    let method = specification
        .symbol
        .strip_prefix("promise/")
        .expect("promise catalog operation must use the promise/ prefix");
    native_promise_values(method, arguments)
}

fn direct_function_operation(
    specification: &DirectCallableSpec,
    arguments: Vec<Value>,
) -> Result<Value, String> {
    match specification.symbol {
        "comp" => direct_composition(arguments),
        "complement" => {
            let predicate = arguments[0].clone();
            if !matches!(predicate, Value::Function(_)) {
                return Err("complement expects a function".into());
            }
            Ok(native_function("complement", 1, move |arguments| {
                Ok(Value::Bool(
                    !call_value(predicate.clone(), arguments)?.truthy(),
                ))
            }))
        }
        "constantly" => {
            let value = arguments[0].clone();
            Ok(native_variadic_function("constantly", move |_| {
                Ok(value.clone())
            }))
        }
        "identity" => Ok(arguments[0].clone()),
        "not" => Ok(Value::Bool(!arguments[0].truthy())),
        operation => Err(format!(
            "missing direct function implementation: {operation}"
        )),
    }
}

fn direct_reference_operation(
    specification: &DirectCallableSpec,
    arguments: Vec<Value>,
) -> Result<Value, String> {
    match specification.symbol {
        "atom" => Ok(Value::Atom(Box::new(RuntimeAtom::new(
            arguments[0].clone(),
            true,
        )))),
        "reset!" => protocol_reset(&arguments),
        "cas!" => protocol_cas(&arguments),
        "swap!" => {
            let reference = arguments[0].clone();
            let function = arguments[1].clone();
            if !matches!(function, Value::Function(_)) {
                return Err("swap! expects a function".into());
            }
            let extra = arguments[2..].to_vec();
            loop {
                let old_value = protocol_deref(std::slice::from_ref(&reference))?;
                let mut call_arguments = Vec::with_capacity(extra.len() + 1);
                call_arguments.push(old_value.clone());
                call_arguments.extend(extra.iter().cloned());
                let new_value = call_value(function.clone(), call_arguments)?;
                if protocol_cas(&[reference.clone(), old_value, new_value.clone()])?
                    == Value::Bool(true)
                {
                    return Ok(new_value);
                }
            }
        }
        operation => Err(format!(
            "missing direct reference implementation: {operation}"
        )),
    }
}

fn direct_collection_operation(
    specification: &DirectCallableSpec,
    arguments: Vec<Value>,
) -> Result<Value, String> {
    match specification.symbol {
        "empty?" => collection_empty(arguments[0].clone()),
        "first" => collection_first(arguments[0].clone()),
        "second" => collection_second(arguments[0].clone()),
        "rest" => collection_rest(arguments[0].clone()),
        "last" => collection_last(arguments[0].clone()),
        "key" | "val" => {
            let Some((key, value)) = pair_parts(&arguments[0]) else {
                return Err(format!("{} expects a pair", specification.symbol));
            };
            Ok(if specification.symbol == "key" {
                key
            } else {
                value
            })
        }
        "keys" => collection_keys(&arguments[0]),
        "vals" => collection_vals(&arguments[0]),
        "reverse" => {
            let mut values = iterator_to_vec(arguments[0].clone())?;
            values.reverse();
            Ok(Value::List(values.into_iter().collect()))
        }
        "not-empty" => {
            let value = arguments[0].clone();
            Ok(if collection_empty(value.clone())?.truthy() {
                Value::Nil
            } else {
                value
            })
        }
        operation => Err(format!(
            "missing direct collection implementation: {operation}"
        )),
    }
}

fn direct_nested_collection_operation(
    specification: &DirectCallableSpec,
    arguments: Vec<Value>,
) -> Result<Value, String> {
    match specification.symbol {
        "get-in" => {
            let keys = iterator_values(arguments[1].clone())?;
            collection_get_in(arguments[0].clone(), &keys)
        }
        "assoc-in" => {
            let keys = iterator_values(arguments[1].clone())?;
            collection_assoc_in(arguments[0].clone(), &keys, arguments[2].clone())
        }
        "update" | "update-in" => {
            let value = arguments[0].clone();
            let keys = if specification.symbol == "update" {
                vec![arguments[1].clone()]
            } else {
                iterator_values(arguments[1].clone())?
            };
            let current = collection_get_in(value.clone(), &keys)?;
            let Value::Function(function) = &arguments[2] else {
                return Err(format!("{} expects a function", specification.symbol));
            };
            let mut call_arguments = vec![current];
            call_arguments.extend(arguments[3..].iter().cloned());
            let replacement = call_function(function, call_arguments)?;
            if specification.symbol == "update" {
                collection_assoc(&value, &keys[0], replacement)
            } else {
                collection_assoc_in(value, &keys, replacement)
            }
        }
        operation => Err(format!(
            "missing direct nested collection implementation: {operation}"
        )),
    }
}

fn direct_constructor_operation(
    specification: &DirectCallableSpec,
    arguments: Vec<Value>,
) -> Result<Value, String> {
    match specification.symbol {
        "hash-map" | "hash-set" => collection_constructor_values(specification.symbol, arguments),
        "keyword" | "symbol" => {
            let parts = arguments
                .iter()
                .map(|value| match value {
                    Value::String(value) => Ok(value.clone()),
                    _ => Err(format!("{} expects string arguments", specification.symbol)),
                })
                .collect::<Result<Vec<_>, _>>()?;
            match (specification.symbol, parts.as_slice()) {
                ("keyword", [name]) => Keyword::parse(name)
                    .map(Value::Keyword)
                    .map_err(|error| format!("keyword failed: {error}")),
                ("keyword", [namespace, name]) => Keyword::create(Some(namespace), name)
                    .map(Value::Keyword)
                    .map_err(|error| format!("keyword failed: {error}")),
                ("symbol", [name]) => Ok(Value::Symbol(Symbol::parse(name))),
                ("symbol", [namespace, name]) => {
                    Ok(Value::Symbol(Symbol::create(Some(namespace), name)))
                }
                _ => unreachable!("catalog validates keyword and symbol arity"),
            }
        }
        "pair" => Ok(Value::Tuple(Box::new(PTuple::from_values(arguments)?))),
        "pointer" => pointer_from_descriptor(arguments[0].clone()),
        "vector" => Ok(Value::Vector(arguments.into())),
        operation => Err(format!(
            "missing direct constructor implementation: {operation}"
        )),
    }
}

fn direct_eval_operation(
    _specification: &DirectCallableSpec,
    arguments: Vec<Value>,
) -> Result<Value, String> {
    let mut environment = current_namespace_environment()?;
    eval_value(arguments[0].clone(), &mut environment)
}

fn direct_namespace_callable_operation(
    specification: &DirectCallableSpec,
    arguments: Vec<Value>,
) -> Result<Value, String> {
    match specification.symbol {
        "eval-in-ns" => {
            let target = direct_namespace_identifier(&arguments[0], "eval-in-ns")?;
            let forms = iterator_values(arguments[1].clone())?
                .into_iter()
                .map(|value| value_to_form(&value))
                .collect::<Result<Vec<_>, _>>()?;
            let registry = namespace_registry()?;
            if registry.find(&target).is_none() {
                return Err(format!(
                    "eval-in-ns requires an existing namespace: {target}"
                ));
            }
            let previous = registry.current().name().as_str().to_owned();
            let mut environment = current_namespace_environment()?;
            select_namespace_environment(&registry, &mut environment, &target);
            let result = (|| {
                let mut result = Value::Nil;
                for form in &forms {
                    result = eval(form, &mut environment)?;
                }
                Ok(result)
            })();
            select_namespace_environment(&registry, &mut environment, &previous);
            result
        }
        "intern-var" => direct_intern_var(arguments),
        "ns-alias-state" => direct_namespace_alias_state(arguments),
        "var-sym" => match &arguments[0] {
            Value::Var(var) => Ok(Value::Symbol(var.symbol().clone())),
            value => Err(format!("var-sym expects a var, got {}", value.display())),
        },
        operation => Err(format!(
            "missing direct namespace implementation: {operation}"
        )),
    }
}

fn direct_intern_var(arguments: Vec<Value>) -> Result<Value, String> {
    let target = direct_namespace_identifier(&arguments[0], "intern-var")?;
    let Value::Symbol(name) = &arguments[1] else {
        return Err("intern-var expects an unqualified target symbol".into());
    };
    if name.get_namespace().is_some() {
        return Err("intern-var expects an unqualified target symbol".into());
    }
    let Value::Var(source) = &arguments[2] else {
        return Err("intern-var expects a source Var".into());
    };
    let mut metadata = source.metadata();
    if let Some(extension) = arguments.get(3) {
        let Some(entries) = map_entries(extension) else {
            return Err("intern-var metadata extension must be a map".into());
        };
        for (key, value) in entries {
            metadata.extra.insert(key.display(), value.display());
        }
    }
    let value = source.deref_value();
    if let Value::Function(function) = &value {
        if function.is_macro {
            ACTIVE_MACROS.with(|active| {
                if let Some(macros) = active.borrow().as_ref() {
                    macros
                        .borrow_mut()
                        .insert((target.clone(), name.as_str().to_owned()), function.clone());
                }
            });
        }
    }
    Ok(Value::Var(
        namespace_registry()?
            .find_or_create(&target)
            .intern_with_metadata(name.as_str(), value, metadata),
    ))
}

fn direct_namespace_alias_state(arguments: Vec<Value>) -> Result<Value, String> {
    let registry = namespace_registry()?;
    let (owner, alias_value) = if arguments.len() == 2 {
        (
            direct_namespace_identifier(&arguments[0], "ns-alias-state")?,
            &arguments[1],
        )
    } else {
        (registry.current().name().as_str().to_owned(), &arguments[0])
    };
    let Value::Symbol(alias) = alias_value else {
        return Err("ns-alias-state expects an unqualified alias symbol".into());
    };
    if alias.get_namespace().is_some() {
        return Err("ns-alias-state expects an unqualified alias symbol".into());
    }
    let Some(namespace) = registry.find(&owner) else {
        return Ok(Value::Nil);
    };
    let target = namespace.lazy_target(alias.as_str()).or_else(|| {
        namespace
            .aliases()
            .into_iter()
            .find(|(name, _)| name == alias)
            .map(|(_, target)| target.name().clone())
    });
    let Some(target) = target else {
        return Ok(Value::Nil);
    };
    let state = registry
        .load_state(target.as_str())
        .or_else(|| {
            registry
                .find(target.as_str())
                .map(|_| NamespaceLoadState::Loaded)
        })
        .map(NamespaceLoadState::as_str)
        .unwrap_or("unknown");
    Ok(Value::Map(PMap::from_iter([
        (Value::Keyword("alias".into()), Value::Symbol(alias.clone())),
        (Value::Keyword("target".into()), Value::Symbol(target)),
        (Value::Keyword("state".into()), Value::Keyword(state.into())),
    ])))
}

fn direct_quantifier_operation(
    specification: &DirectCallableSpec,
    arguments: Vec<Value>,
) -> Result<Value, String> {
    if arguments.len() == 1 {
        let predicate = arguments[0].clone();
        let operation = specification.symbol;
        return Ok(native_function(operation, 1, move |arguments| {
            direct_quantifier_values(operation, predicate.clone(), arguments[0].clone())
        }));
    }
    direct_quantifier_values(
        specification.symbol,
        arguments[0].clone(),
        arguments[1].clone(),
    )
}

fn direct_quantifier_values(
    operation: &str,
    predicate: Value,
    collection: Value,
) -> Result<Value, String> {
    for value in iterator_values(collection)? {
        let matched = call_value(predicate.clone(), vec![value])?.truthy();
        if operation == "every?" && !matched {
            return Ok(Value::Bool(false));
        }
        if operation == "any?" && matched {
            return Ok(Value::Bool(true));
        }
    }
    Ok(Value::Bool(operation == "every?"))
}

fn direct_sequence_operation(
    specification: &DirectCallableSpec,
    arguments: Vec<Value>,
) -> Result<Value, String> {
    direct_sequence_values(specification.symbol, arguments)
}

fn direct_sequence_values(operation: &'static str, arguments: Vec<Value>) -> Result<Value, String> {
    match operation {
        "map" => direct_map(arguments),
        "filter" | "drop" | "drop-while" | "interpose" | "keep" | "mapcat" | "partition"
        | "partition-all" | "take" | "take-while"
            if arguments.len() == 1 =>
        {
            let parameter = arguments[0].clone();
            Ok(native_function(operation, 1, move |arguments| {
                direct_sequence_values(operation, vec![parameter.clone(), arguments[0].clone()])
            }))
        }
        "filter" => {
            let source = arguments[1].clone();
            transform_like(
                &source,
                iterator_filter(arguments[0].clone(), source.clone())?,
            )
        }
        "take" | "drop" => {
            let amount = value_index(&arguments[0])?;
            let source = arguments[1].clone();
            let result = if operation == "take" {
                iterator_take(source.clone(), amount)?
            } else {
                iterator_drop(source.clone(), amount)?
            };
            transform_like(&source, result)
        }
        "take-while" | "drop-while" => {
            let source = arguments[1].clone();
            let result = if operation == "take-while" {
                iterator_take_while(arguments[0].clone(), source.clone())?
            } else {
                iterator_drop_while(arguments[0].clone(), source.clone())?
            };
            transform_like(&source, result)
        }
        "mapcat" | "keep" => {
            let source = arguments[1].clone();
            let result = if operation == "mapcat" {
                iterator_mapcat(arguments[0].clone(), source.clone())?
            } else {
                iterator_keep(arguments[0].clone(), source.clone())?
            };
            transform_like(&source, result)
        }
        "partition" | "partition-all" => {
            let amount = value_index(&arguments[0])?;
            let source = arguments[1].clone();
            transform_like(
                &source,
                iterator_partition(source.clone(), amount, operation == "partition-all")?,
            )
        }
        "interpose" => {
            let source = arguments[1].clone();
            transform_like(
                &source,
                iterator_interpose(arguments[0].clone(), source.clone())?,
            )
        }
        "interleave" => {
            let primary = arguments[0].clone();
            transform_like(&primary, iterator_interleave(arguments)?)
        }
        "partition-pair" => {
            let source = arguments[0].clone();
            transform_like(&source, iterator_partition(source.clone(), 2, false)?)
        }
        "zip" => {
            let primary = arguments[0].clone();
            transform_like(&primary, iterator_zip(arguments)?)
        }
        "cycle" => iterator_seq(iterator_cycle(arguments[0].clone())?),
        "concat" => iterator_seq(iterator_concat(arguments)?),
        "range" => {
            let numbers = arguments
                .iter()
                .map(|value| {
                    numeric::to_i64_exact(value)
                        .map_err(|_| "range bounds must fit signed 64-bit integers".to_string())
                })
                .collect::<Result<Vec<_>, _>>()?;
            let (start, end) = match numbers.as_slice() {
                [] => (0, 0),
                [end] => (0, *end),
                [start, end] => (*start, *end),
                _ => unreachable!("catalog validates range arity"),
            };
            iterator_seq(iterator_from_values(
                (start..end).map(Value::Number).collect(),
            ))
        }
        "repeat" => {
            let (amount, value) = if arguments.len() == 1 {
                (None, arguments[0].clone())
            } else {
                (Some(value_index(&arguments[0])?), arguments[1].clone())
            };
            if let Some(amount) = amount {
                iterator_seq(iterator_from_values(
                    (0..amount).map(|_| value.clone()).collect(),
                ))
            } else {
                iterator_seq(iterator_constant(value))
            }
        }
        "repeatedly" => {
            let (amount, function) = if arguments.len() == 1 {
                (None, arguments[0].clone())
            } else {
                (Some(value_index(&arguments[0])?), arguments[1].clone())
            };
            let generated = iterator_repeated(function);
            iterator_seq(if let Some(amount) = amount {
                iterator_take(generated, amount)?
            } else {
                generated
            })
        }
        "iterate" => iterator_seq(iterator_iterate(arguments[0].clone(), arguments[1].clone())),
        operation => Err(format!(
            "missing direct sequence implementation: {operation}"
        )),
    }
}

fn direct_map(arguments: Vec<Value>) -> Result<Value, String> {
    if arguments.len() == 1 {
        let function = arguments[0].clone();
        return Ok(native_function("map", 1, move |arguments| {
            direct_map(vec![function.clone(), arguments[0].clone()])
        }));
    }
    let function = arguments[0].clone();
    let sources = arguments[1..].to_vec();
    let primary = sources[0].clone();
    if sources.len() == 1 {
        return transform_like(&primary, iterator_map(function, primary.clone())?);
    }
    let zipped = iterator_zip(sources)?;
    let mut output = Vec::new();
    for value in iterator_to_vec(zipped)? {
        output.push(call_value(function.clone(), iterator_values(value)?)?);
    }
    transform_like(&primary, iterator_from_values(output))
}
