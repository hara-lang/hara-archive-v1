fn metadata_value(form: &Form) -> Result<MetadataValue, String> {
    match form {
        Form::Nil => Ok(MetadataValue::Nil),
        Form::Bool(value) => Ok(MetadataValue::Boolean(*value)),
        Form::Number(value) => Ok(MetadataValue::Number(*value)),
        Form::Float(value) => Ok(MetadataValue::Float(*value)),
        Form::BigInteger(value) => Ok(MetadataValue::BigInteger(value.clone())),
        Form::Character(value) => Ok(MetadataValue::Character(*value)),
        Form::Regex(value) => Ok(MetadataValue::Regex(value.clone())),
        Form::Tagged(tag, value) => Ok(MetadataValue::Tagged(
            tag.clone(),
            Box::new(metadata_value(value)?),
        )),
        Form::Metadata(_, value) => metadata_value(value),
        Form::Symbol(value) => Ok(MetadataValue::Symbol(Symbol::from(value.clone()))),
        Form::Keyword(value) => Ok(MetadataValue::Keyword(Keyword::from(value.clone()))),
        Form::String(value) => Ok(MetadataValue::String(value.clone())),
        Form::Vector(values) => Ok(MetadataValue::Vector(
            values
                .iter()
                .map(metadata_value)
                .collect::<Result<_, _>>()?,
        )),
        Form::List(values) => Ok(MetadataValue::List(
            values
                .iter()
                .map(metadata_value)
                .collect::<Result<_, _>>()?,
        )),
        Form::Set(values) => Ok(MetadataValue::Set(
            values
                .iter()
                .map(metadata_value)
                .collect::<Result<_, _>>()?,
        )),
        Form::Map(values) => Ok(MetadataValue::Map(
            values
                .iter()
                .map(|(key, value)| Ok((metadata_value(key)?, metadata_value(value)?)))
                .collect::<Result<_, String>>()?,
        )),
    }
}

pub(crate) fn metadata_from_form(form: &Form) -> Result<Rc<Metadata>, String> {
    let MetadataValue::Map(entries) = metadata_value(form)? else {
        return Err("reader metadata must be a map".into());
    };
    Ok(Metadata::new(entries))
}

fn merge_metadata(
    existing: Option<Rc<Metadata>>,
    overlay: Option<Rc<Metadata>>,
) -> Option<Rc<Metadata>> {
    match (existing, overlay) {
        (None, None) => None,
        (Some(metadata), None) | (None, Some(metadata)) => Some(metadata),
        (Some(existing), Some(overlay)) => {
            let mut entries = existing.entries().to_vec();
            for (key, value) in overlay.entries() {
                entries.retain(|(candidate, _)| candidate != key);
                entries.push((key.clone(), value.clone()));
            }
            Some(Metadata::new(entries))
        }
    }
}

fn assoc_metadata(
    metadata: Option<Rc<Metadata>>,
    key: &str,
    value: MetadataValue,
) -> Option<Rc<Metadata>> {
    merge_metadata(
        metadata,
        Some(Metadata::new(vec![(
            MetadataValue::Keyword(Keyword::from(key)),
            value,
        )])),
    )
}

pub(crate) fn definition_metadata(
    mut metadata: Option<Rc<Metadata>>,
    forms: &[Form],
    private: bool,
    macro_form: bool,
) -> Result<(Option<Rc<Metadata>>, &[Form]), String> {
    let mut rest = forms;
    if let Some(Form::String(doc)) = rest.first().map(form_without_metadata) {
        metadata = assoc_metadata(metadata, "doc", MetadataValue::String(doc.clone()));
        rest = &rest[1..];
    }
    if let Some(Form::Map(_)) = rest.first().map(form_without_metadata) {
        metadata = merge_metadata(
            metadata,
            Some(metadata_from_form(form_without_metadata(&rest[0]))?),
        );
        rest = &rest[1..];
    }
    if rest.is_empty() {
        return Ok((metadata, rest));
    }
    let arglists = if matches!(
        rest.first().map(form_without_metadata),
        Some(Form::Vector(_))
    ) {
        vec![metadata_value(form_without_metadata(&rest[0]))?]
    } else {
        rest.iter()
            .map(|clause| match form_without_metadata(clause) {
                Form::List(parts) if !parts.is_empty() => {
                    metadata_value(form_without_metadata(&parts[0]))
                }
                _ => Err(format!(
                    "function arity must be a list beginning with parameters: {clause:?}"
                )),
            })
            .collect::<Result<Vec<_>, String>>()?
    };
    metadata = assoc_metadata(metadata, "arglists", MetadataValue::Vector(arglists));
    if metadata.as_ref().is_some_and(|value| value.flag("inline")) {
        let target = inline_forward_target(rest)
            .ok_or_else(|| ":inline true requires a transparent forwarding function".to_string())?;
        metadata = assoc_metadata(
            metadata,
            "inline-target",
            MetadataValue::Symbol(Symbol::from(target)),
        );
    }
    if private {
        metadata = assoc_metadata(metadata, "private", MetadataValue::Boolean(true));
    }
    if macro_form {
        metadata = assoc_metadata(metadata, "macro", MetadataValue::Boolean(true));
    }
    Ok((metadata, rest))
}

fn inline_forward_target(forms: &[Form]) -> Option<String> {
    fn clause_target(params: &Form, body: &[Form]) -> Option<String> {
        if body.len() != 1 {
            return None;
        }
        let Form::Vector(params) = form_without_metadata(params) else {
            return None;
        };
        let Form::List(call) = form_without_metadata(&body[0]) else {
            return None;
        };
        let parameter_names = params
            .iter()
            .map(form_without_metadata)
            .map(|form| match form {
                Form::Symbol(name) => Some(name.as_str()),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()?;
        if let [ampersand, rest] = parameter_names.as_slice() {
            if *ampersand == "&" {
                return match call.as_slice() {
                    [Form::Symbol(apply), Form::Symbol(target), Form::Symbol(argument)]
                        if apply == "apply" && argument == rest =>
                    {
                        Some(target.clone())
                    }
                    _ => None,
                };
            }
        }
        let Form::Symbol(target) = call.first()? else {
            return None;
        };
        let arguments = call[1..]
            .iter()
            .map(form_without_metadata)
            .map(|form| match form {
                Form::Symbol(name) => Some(name.as_str()),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()?;
        (arguments == parameter_names).then(|| target.clone())
    }

    if matches!(
        forms.first().map(form_without_metadata),
        Some(Form::Vector(_))
    ) {
        return clause_target(forms.first()?, &forms[1..]);
    }
    let mut target = None;
    for clause in forms {
        let Form::List(parts) = form_without_metadata(clause) else {
            return None;
        };
        let candidate = clause_target(parts.first()?, &parts[1..])?;
        if target.as_ref().is_some_and(|value| value != &candidate) {
            return None;
        }
        target = Some(candidate);
    }
    target
}

pub(crate) fn schema_var_reference(metadata: Option<&Metadata>) -> Option<&Symbol> {
    let MetadataValue::List(reference) = metadata?.get_keyword("schema")? else {
        return None;
    };
    match reference.as_slice() {
        [MetadataValue::Symbol(operator), MetadataValue::Symbol(target)]
            if operator.get_namespace().is_none() && operator.get_name() == "var" =>
        {
            Some(target)
        }
        _ => None,
    }
}

fn attach_optional_metadata(value: Value, metadata: Option<Rc<Metadata>>) -> Result<Value, String> {
    Ok(match value {
        Value::Symbol(value) => Value::Symbol(value.with_meta(metadata.clone())),
        Value::Pointer(value) => Value::Pointer(value.with_meta(metadata.clone())),
        Value::Tuple(value) => Value::Tuple(Box::new(value.with_meta(metadata.clone()))),
        Value::Vector(value) => Value::Vector(value.with_meta(metadata.clone())),
        Value::List(value) => Value::List(value.with_meta(metadata.clone())),
        Value::Cons(value) => Value::Cons(Box::new(value.with_meta(metadata.clone()))),
        Value::Queue(value) => Value::Queue(Box::new(value.with_meta(metadata.clone()))),
        Value::Deque(value) => Value::Deque(Box::new(value.with_meta(metadata.clone()))),
        Value::Map(value) => Value::Map(value.with_meta(metadata.clone())),
        Value::OrderedMap(value) => Value::OrderedMap(Box::new(value.with_meta(metadata.clone()))),
        Value::SortedMap(value) => Value::SortedMap(Box::new(value.with_meta(metadata.clone()))),
        Value::Trie(value) => Value::Trie(Box::new(value.with_meta(metadata.clone()))),
        Value::PriorityMap(value) => {
            Value::PriorityMap(Box::new(value.with_meta(metadata.clone())))
        }
        Value::Set(value) => Value::Set(value.with_meta(metadata.clone())),
        Value::OrderedSet(value) => Value::OrderedSet(Box::new(value.with_meta(metadata.clone()))),
        Value::SortedSet(value) => Value::SortedSet(Box::new(value.with_meta(metadata.clone()))),
        Value::Seq(value) => Value::Seq(Box::new(value.with_meta(metadata.clone()))),
        Value::Var(value) => {
            value.set_hara_metadata(metadata);
            Value::Var(value)
        }
        Value::Function(value) => Value::Function(Rc::new(Function {
            metadata,
            ..value.as_ref().clone()
        })),
        Value::Struct(value) => Value::Struct(Rc::new(StructValue {
            ty: value.ty.clone(),
            values: value.values.clone(),
            metadata,
        })),
        Value::Mutable(value) => Value::Mutable(Rc::new(MutableValue {
            ty: value.ty.clone(),
            values: value.values.clone(),
            metadata,
        })),
        Value::NativeType(value) => Value::NativeType(Rc::new(NativeType {
            name: value.name.clone(),
            methods: value.methods.clone(),
            metadata,
        })),
        Value::Keyword(value) => Value::Keyword(value),
        _ => return Err("metadata can only be applied to object values".into()),
    })
}

fn attach_metadata(value: Value, metadata: Rc<Metadata>) -> Result<Value, String> {
    attach_optional_metadata(value, Some(metadata))
}

fn eval_sequential_constructor(
    name: &str,
    forms: &[Form],
    env: &mut HashMap<String, Value>,
) -> Result<Value, String> {
    let maximum = match name {
        "pair" => Some(2),
        "tup" => Some(5),
        _ => None,
    };
    if let Some(maximum) = maximum {
        let valid = if name == "pair" {
            forms.len() == maximum
        } else {
            forms.len() <= maximum
        };
        if !valid {
            return Err(if name == "pair" {
                format!("pair expects two arguments, got {}", forms.len())
            } else {
                format!("tup expects at most 5 arguments, got {}", forms.len())
            });
        }
    }
    let values = forms
        .iter()
        .map(|form| eval(form, env))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(match name {
        "list" => Value::List(values.into()),
        "vector" => Value::Vector(values.into()),
        "pair" | "tup" => Value::Tuple(Box::new(PTuple::from_values(values)?)),
        _ => unreachable!("guarded sequential constructor"),
    })
}

fn eval_collection_constructor(
    name: &str,
    forms: &[Form],
    env: &mut HashMap<String, Value>,
) -> Result<Value, String> {
    let values = forms
        .iter()
        .map(|form| eval(form, env))
        .collect::<Result<Vec<_>, _>>()?;
    collection_constructor_values(name, values)
}

fn collection_constructor_values(name: &str, values: Vec<Value>) -> Result<Value, String> {
    match name {
        "hash-map" | "ordered-map" | "priority-map" | "sorted-map" | "trie" => {
            if values.len() % 2 != 0 {
                return Err(format!(
                    "{name} expects an even number of key/value arguments"
                ));
            }
            let entries = values
                .chunks_exact(2)
                .map(|pair| (pair[0].clone(), pair[1].clone()));
            Ok(match name {
                "hash-map" => Value::Map(PMap::from_iter(entries)),
                "ordered-map" => Value::OrderedMap(Box::new(POrderedMap::from_iter(entries))),
                "priority-map" => Value::PriorityMap(Box::new(PPriorityMap::from_iter(entries))),
                "sorted-map" => Value::SortedMap(Box::new(PSortedMap::from_iter(entries))),
                "trie" => {
                    let mut trie = PTrie::new();
                    for (key, value) in entries {
                        let Value::String(key) = key else {
                            return Err("trie expects string keys".into());
                        };
                        trie = trie.assoc_value(key, value);
                    }
                    Value::Trie(Box::new(trie))
                }
                _ => unreachable!("guarded map constructor"),
            })
        }
        "hash-set" => Ok(Value::Set(values.into_iter().collect())),
        "ordered-set" => Ok(Value::OrderedSet(Box::new(values.into_iter().collect()))),
        "sorted-set" => Ok(Value::SortedSet(Box::new(values.into_iter().collect()))),
        "deque" => Ok(Value::Deque(Box::new(values.into_iter().collect()))),
        "queue" => Ok(Value::Queue(Box::new(values.into_iter().collect()))),
        _ => unreachable!("guarded collection constructor"),
    }
}

fn vector_literal(values: Vec<Value>) -> Result<Value, String> {
    if values.len() <= 8 {
        Ok(Value::Tuple(Box::new(PTuple::from_values(values)?)))
    } else {
        Ok(Value::Vector(values.into()))
    }
}

pub(crate) fn vm_build_vector(values: Vec<Value>) -> Result<Value, String> {
    vector_literal(values)
}

pub(crate) fn vm_build_map(values: Vec<Value>) -> Result<Value, String> {
    if values.len() % 2 != 0 {
        return Err("map construction requires key/value pairs".into());
    }
    Ok(Value::Map(PMap::from_iter(
        values
            .chunks_exact(2)
            .map(|pair| (pair[0].clone(), pair[1].clone())),
    )))
}

pub(crate) fn vm_build_set(values: Vec<Value>) -> Result<Value, String> {
    Ok(Value::OrderedSet(Box::new(POrderedSet::from_iter(values))))
}

pub(crate) fn vm_build_list(values: Vec<Value>) -> Value {
    Value::List(values.into())
}

pub(crate) fn vm_concat_list(values: Vec<Value>) -> Result<Value, String> {
    let mut output = Vec::new();
    for value in values {
        output.extend(iterator_values(value)?);
    }
    Ok(Value::List(output.into()))
}

pub(crate) fn vm_to_vector(value: Value) -> Result<Value, String> {
    vector_literal(iterator_values(value)?)
}

fn literal_value(form: &Form) -> Result<Value, String> {
    match form {
        Form::Nil => Ok(Value::Nil),
        Form::Bool(value) => Ok(Value::Bool(*value)),
        Form::Character(value) => Ok(Value::Character(*value)),
        Form::Float(value) => Ok(Value::Float(*value)),
        Form::BigInteger(value) => Ok(Value::BigInteger(value.clone())),
        Form::Regex(value) => Ok(Value::Regex(value.clone())),
        Form::Tagged(tag, value) if tag == "ptr" => pointer_from_descriptor(literal_value(value)?),
        Form::Tagged(tag, value) => Ok(Value::Tagged(Box::new(PTaggedLiteral::new(
            Symbol::parse(tag),
            literal_value(value)?,
        )))),
        Form::Metadata(metadata, value) => {
            attach_metadata(literal_value(value)?, metadata_from_form(metadata)?)
        }
        Form::Number(v) => Ok(Value::Number(*v)),
        Form::String(v) => Ok(Value::String(v.clone())),
        Form::Keyword(v) => Ok(Value::Keyword(v.clone().into())),
        Form::Symbol(v) => Ok(Value::Symbol(v.clone().into())),
        Form::Vector(values) => {
            vector_literal(values.iter().map(literal_value).collect::<Result<_, _>>()?)
        }
        Form::Set(values) => Ok(Value::OrderedSet(Box::new(
            unique_values(values.iter().map(literal_value).collect::<Result<_, _>>()?)
                .into_iter()
                .collect(),
        ))),
        Form::List(values) => Ok(Value::List(
            values.iter().map(literal_value).collect::<Result<_, _>>()?,
        )),
        Form::Map(values) => Ok(Value::Map(
            values
                .iter()
                .map(|(k, v)| Ok((literal_value(k)?, literal_value(v)?)))
                .collect::<Result<_, String>>()?,
        )),
    }
}

fn generated_function(
    params: Vec<String>,
    body: Vec<Form>,
    mut captured: HashMap<String, Value>,
    bindings: Vec<(&str, Value)>,
) -> Value {
    for (name, value) in bindings {
        captured.insert(name.to_string(), value);
    }
    Value::Function(Rc::new(Function {
        patterns: params.iter().cloned().map(Form::Symbol).collect(),
        params,
        variadic: None,
        variadic_pattern: None,
        body,
        captured: Rc::new(RefCell::new(captured)),
        name: None,
        namespace: function_definition_namespace(),
        native: None,
        fiber_native: None,
        clauses: Vec::new(),
        metadata: None,
        is_macro: false,
    }))
}

fn generated_unary_operation(
    operation: &str,
    parameter: Value,
    env: HashMap<String, Value>,
) -> Value {
    let body = Form::List(vec![
        Form::Symbol("__iterator-transform".into()),
        Form::Symbol("__operation".into()),
        Form::Symbol("__parameter".into()),
        Form::Symbol("value".into()),
    ]);
    generated_function(
        vec!["value".into()],
        vec![body],
        env,
        vec![
            ("__operation", Value::String(operation.to_string())),
            ("__parameter", parameter),
        ],
    )
}

fn function_parts(
    form: &Form,
) -> Result<(Vec<String>, Option<String>, Vec<Form>, Option<Form>), String> {
    let list = match form_without_metadata(form) {
        Form::Vector(values) => values,
        _ => return Err("function parameters must be a vector".into()),
    };
    let mut params = Vec::new();
    let mut variadic = None;
    let mut patterns = Vec::new();
    let mut variadic_pattern = None;
    let mut index = 0;
    while index < list.len() {
        match form_without_metadata(&list[index]) {
            Form::Symbol(name) if name == "&" => {
                if variadic.is_some() || index + 1 >= list.len() || index + 2 != list.len() {
                    return Err("variadic marker must precede the final parameter".into());
                }
                let pattern = form_without_metadata(&list[index + 1]).clone();
                variadic = Some(match &pattern {
                    Form::Symbol(name) => name.clone(),
                    _ => format!("__rest_{}", params.len()),
                });
                variadic_pattern = Some(pattern);
                index += 2;
            }
            pattern @ (Form::Symbol(_) | Form::Vector(_) | Form::Map(_)) => {
                params.push(match pattern {
                    Form::Symbol(name) => name.clone(),
                    _ => format!("__arg_{}", params.len()),
                });
                patterns.push(pattern.clone());
                index += 1;
            }
            _ => return Err("function parameters must be binding patterns".into()),
        }
    }
    Ok((params, variadic, patterns, variadic_pattern))
}

fn collect_capture_names(form: &Form, names: &mut std::collections::HashSet<String>) {
    match form {
        Form::Symbol(name) => {
            names.insert(name.clone());
        }
        Form::List(values) | Form::Vector(values) | Form::Set(values) => {
            for value in values {
                collect_capture_names(value, names);
            }
        }
        Form::Map(entries) => {
            for (key, value) in entries {
                collect_capture_names(key, names);
                collect_capture_names(value, names);
            }
        }
        Form::Metadata(metadata, value) => {
            collect_capture_names(metadata, names);
            collect_capture_names(value, names);
        }
        Form::Tagged(_, value) => collect_capture_names(value, names),
        Form::Nil
        | Form::Bool(_)
        | Form::Number(_)
        | Form::Float(_)
        | Form::BigInteger(_)
        | Form::Character(_)
        | Form::Regex(_)
        | Form::String(_)
        | Form::Keyword(_) => {}
    }
}

fn capture_environment(forms: &[Form], env: &HashMap<String, Value>) -> HashMap<String, Value> {
    let mut names = std::collections::HashSet::new();
    for form in forms {
        collect_capture_names(form, &mut names);
    }
    names
        .into_iter()
        // `macroexpand-1` is an evaluator intrinsic whose result depends on
        // the active macro registry. Capturing a Foundation fallback for it
        // freezes the registry visible to closures such as `macroexpand`.
        .filter(|name| name != "macroexpand-1")
        .filter_map(|name| env.get(&name).cloned().map(|value| (name, value)))
        .collect()
}

fn destructuring_default<'a>(defaults: Option<&'a Form>, name: &str) -> Option<&'a Form> {
    let Form::Map(entries) = defaults? else {
        return None;
    };
    entries.iter().find_map(|(key, value)| {
        matches!(key, Form::Symbol(candidate) if candidate == name).then_some(value)
    })
}

pub(crate) fn bind_pattern(
    pattern: &Form,
    value: Value,
    env: &mut HashMap<String, Value>,
    bound: &mut Vec<String>,
    defaults: Option<&Form>,
) -> Result<(), String> {
    match pattern {
        Form::Symbol(name) => {
            if name == "_" {
                return Ok(());
            }
            if name.contains('/') || bound.iter().any(|candidate| candidate == name) {
                return Err(format!("invalid or duplicate binding: {name}"));
            }
            let value = if matches!(value, Value::Nil) {
                match destructuring_default(defaults, name) {
                    Some(default) => eval(default, env)?,
                    None => value,
                }
            } else {
                value
            };
            env.insert(name.clone(), value);
            bound.push(name.clone());
            Ok(())
        }
        Form::Vector(patterns) => {
            let original = value.clone();
            let values = if matches!(value, Value::Nil) {
                Vec::new()
            } else {
                iterator_values(value)
                    .map_err(|_| "cannot destructure non-sequential value".to_owned())?
            };
            let mut index = 0;
            let mut position = 0;
            while index < patterns.len() {
                match &patterns[index] {
                    Form::Symbol(marker) if marker == "&" => {
                        if index + 1 >= patterns.len() {
                            return Err("& in a destructuring vector requires a binding".into());
                        }
                        bind_pattern(
                            &patterns[index + 1],
                            Value::Vector(values.iter().skip(position).cloned().collect()),
                            env,
                            bound,
                            defaults,
                        )?;
                        index += 2;
                    }
                    Form::Keyword(marker) if marker.as_str() == "as" => {
                        if index + 2 != patterns.len() {
                            return Err(
                                ":as in a destructuring vector must precede its final binding"
                                    .into(),
                            );
                        }
                        bind_pattern(&patterns[index + 1], original, env, bound, defaults)?;
                        return Ok(());
                    }
                    nested => {
                        bind_pattern(
                            nested,
                            values.get(position).cloned().unwrap_or(Value::Nil),
                            env,
                            bound,
                            defaults,
                        )?;
                        position += 1;
                        index += 1;
                    }
                }
            }
            Ok(())
        }
        Form::Map(entries) => {
            if !matches!(value, Value::Nil | Value::Struct(_)) && map_entries(&value).is_none() {
                return Err("cannot destructure non-map value".into());
            }
            let defaults = entries.iter().find_map(|(key, value)| {
                matches!(key, Form::Keyword(keyword) if keyword.as_str() == "or").then_some(value)
            });
            for (binding, key) in entries {
                match binding {
                    Form::Keyword(keyword) if keyword.as_str() == "or" => {}
                    Form::Keyword(keyword) if keyword.as_str() == "as" => {
                        bind_pattern(key, value.clone(), env, bound, defaults)?;
                    }
                    Form::Keyword(keyword)
                        if ["keys", "strs", "syms"].contains(&keyword.as_str()) =>
                    {
                        let Form::Vector(names) = key else {
                            return Err(format!(
                                ":{} destructuring expects a vector of symbols",
                                keyword.as_str()
                            ));
                        };
                        for name in names {
                            let Form::Symbol(name) = name else {
                                return Err(format!(
                                    ":{} destructuring expects symbols",
                                    keyword.as_str()
                                ));
                            };
                            let lookup = match keyword.as_str() {
                                "keys" => Value::Keyword(name.clone().into()),
                                "strs" => Value::String(name.clone()),
                                "syms" => Value::Symbol(name.clone().into()),
                                _ => unreachable!(),
                            };
                            bind_pattern(
                                &Form::Symbol(name.clone()),
                                collection_get(&value, &lookup, Value::Nil)?,
                                env,
                                bound,
                                defaults,
                            )?;
                        }
                    }
                    binding => {
                        let lookup = literal_value(key)?;
                        bind_pattern(
                            binding,
                            collection_get(&value, &lookup, Value::Nil)?,
                            env,
                            bound,
                            defaults,
                        )?;
                    }
                }
            }
            Ok(())
        }
        _ => Err("unsupported binding pattern".into()),
    }
}

fn select_clause(functions: &[Rc<Function>], argument_count: usize) -> Option<Rc<Function>> {
    functions
        .iter()
        .find(|function| function.variadic.is_none() && function.params.len() == argument_count)
        .or_else(|| {
            functions
                .iter()
                .filter(|function| {
                    function.variadic.is_some() && argument_count >= function.params.len()
                })
                .max_by_key(|function| function.params.len())
        })
        .cloned()
}

fn multi_arity_function(
    name: &str,
    clauses: &[Form],
    captured: &HashMap<String, Value>,
    is_macro: bool,
) -> Result<Value, String> {
    let mut functions = Vec::with_capacity(clauses.len());
    for clause in clauses {
        let parts = match form_without_metadata(clause) {
            Form::List(parts) if parts.len() >= 2 => parts,
            _ => return Err("defn arity must contain parameters and a body".into()),
        };
        let (params, variadic, patterns, variadic_pattern) = function_parts(&parts[0])?;
        functions.push(Rc::new(Function {
            params,
            variadic,
            patterns,
            variadic_pattern,
            body: parts[1..].to_vec(),
            captured: Rc::new(RefCell::new(capture_environment(&parts[1..], captured))),
            name: Some(name.into()),
            namespace: function_definition_namespace(),
            native: None,
            fiber_native: None,
            clauses: Vec::new(),
            metadata: None,
            is_macro,
        }));
    }
    if functions.is_empty() {
        return Err("defn expects at least one arity".into());
    }
    Ok(arity_dispatcher(name, functions, is_macro))
}

/// Builds the multi-arity dispatcher shared by the evaluator's defn and
/// the bytecode VM's `MakeMultiArity` (issue #223): exact fixed-arity
/// match first, then the variadic clause with the most parameters.
pub(crate) fn arity_dispatcher(name: &str, functions: Vec<Rc<Function>>, is_macro: bool) -> Value {
    let dispatch_name = name.to_owned();
    let clauses = functions.clone();
    Value::Function(Rc::new(Function {
        params: Vec::new(),
        variadic: Some("arguments".into()),
        patterns: Vec::new(),
        variadic_pattern: None,
        body: Vec::new(),
        captured: Rc::new(RefCell::new(HashMap::new())),
        name: Some(dispatch_name.clone()),
        namespace: function_definition_namespace(),
        clauses,
        native: Some(Rc::new(move |arguments| {
            let function = select_clause(&functions, arguments.len()).ok_or_else(|| {
                format!(
                    "{dispatch_name} has no arity accepting {} arguments",
                    arguments.len()
                )
            })?;
            call_function(&function, arguments)
        })),
        fiber_native: None,
        metadata: None,
        is_macro,
    }))
}

fn deref_value(value: Value) -> Value {
    match value {
        Value::Var(var) => var.deref_value(),
        value => value,
    }
}

fn deref_binding_value(name: &str, value: Value) -> Value {
    match value {
        Value::Var(var)
            if name.starts_with("std.native.")
                || var.symbol().get_name() == Symbol::parse(name).get_name() =>
        {
            var.deref_value()
        }
        value => value,
    }
}

fn binding_value(env: &HashMap<String, Value>, name: &str) -> Option<Value> {
    env.get(name)
        .cloned()
        .map(|value| deref_binding_value(name, value))
        .or_else(|| {
            namespace_registry()
                .ok()?
                .resolve(&crate::lang::data::Symbol::parse(name))
                .map(|var| var.deref_value())
        })
        .or_else(|| {
            let (qualifier, local) = name.rsplit_once('/')?;
            let registry = namespace_registry().ok()?;
            (registry.current().name().as_str() == qualifier)
                .then(|| {
                    env.get(local)
                        .cloned()
                        .map(|value| deref_binding_value(local, value))
                })
                .flatten()
        })
}

fn foundation_fallback_omitted(env: &HashMap<String, Value>, name: &str) -> bool {
    // Namespace introspection is implemented as an evaluator intrinsic, not
    // as an optional Foundation fallback. Keep it available in restricted
    // namespaces so the structural evaluator arms can enforce their own
    // argument and visibility semantics.
    const INTRINSIC_FORMS: &[&str] = &[
        "ns-alias-state",
        "ns-loaded?",
        "ns-name",
        "ns-publics",
        "ns-state",
        "resolve",
        "special-symbol?",
        "the-ns",
    ];
    if name.contains('/')
        || env.contains_key(name)
        || syntax_symbol(name)
        || INTRINSIC_FORMS.contains(&name)
    {
        return false;
    }
    let Ok(registry) = namespace_registry() else {
        return false;
    };
    let local = crate::lang::data::Symbol::parse(name);
    registry.current().resolve(&local).is_none()
        && registry
            .find("std.foundation")
            .and_then(|foundation| foundation.resolve(&local))
            .is_some()
}

fn binding_var(env: &mut HashMap<String, Value>, name: &str) -> Option<KernelVar<Value>> {
    match env.get(name) {
        Some(Value::Var(var)) => Some(var.clone()),
        Some(value) => {
            let var = KernelVar::new(name, value.clone());
            env.insert(name.to_string(), Value::Var(var.clone()));
            Some(var)
        }
        None => {
            if let Some(local) = name.strip_prefix("-/") {
                if let Some(Value::Var(var)) = env.get(local) {
                    return Some(var.clone());
                }
            }
            namespace_registry()
                .ok()?
                .resolve(&crate::lang::data::Symbol::parse(name))
        }
    }
}

pub(crate) fn call_value(callable: Value, arguments: Vec<Value>) -> Result<Value, String> {
    let lookup =
        |target: &Value, key: &Value, fallback: Value| collection_get(target, key, fallback);
    match callable {
        Value::Function(function) => call_function(&function, arguments),
        Value::StructType(ty) => Ok(Value::Struct(Rc::new(StructValue::from_values(
            ty, arguments, None,
        )?))),
        Value::MutableType(ty) => Ok(Value::Mutable(Rc::new(MutableValue::from_values(
            ty, arguments, None,
        )?))),
        value @ (Value::Struct(_) | Value::Mutable(_)) => {
            let mut protocol_arguments = Vec::with_capacity(arguments.len() + 1);
            protocol_arguments.push(value);
            protocol_arguments.extend(arguments);
            protocol_call("std.protocol.ifn/IFn", "invoke", &protocol_arguments)
        }
        Value::Pointer(pointer) => pointer_context_call(
            &pointer,
            pointer_default(&pointer)?,
            "pointer/invoke",
            &arguments,
        ),
        Value::Keyword(keyword) => match arguments.as_slice() {
            [target] => lookup(target, &Value::Keyword(keyword), Value::Nil),
            [target, fallback] => lookup(target, &Value::Keyword(keyword), fallback.clone()),
            _ => Err("keyword invocation expects one or two arguments".into()),
        },
        value @ (Value::Map(_)
        | Value::OrderedMap(_)
        | Value::SortedMap(_)
        | Value::Trie(_)
        | Value::PriorityMap(_)) => match arguments.as_slice() {
            [key] => Ok(map_value(&value, key).cloned().unwrap_or(Value::Nil)),
            [key, fallback] => Ok(map_value(&value, key)
                .cloned()
                .unwrap_or_else(|| fallback.clone())),
            _ => Err("map invocation expects one or two arguments".into()),
        },
        value @ (Value::Set(_) | Value::OrderedSet(_) | Value::SortedSet(_)) => {
            match arguments.as_slice() {
                [key] => Ok(set_find(&value, key).unwrap_or(Value::Nil)),
                [key, fallback] => Ok(set_find(&value, key).unwrap_or_else(|| fallback.clone())),
                _ => Err("set invocation expects one or two arguments".into()),
            }
        }
        _ => Err("value is not callable".into()),
    }
}

/// Invokes a runtime callable with already-decoded values.
///
/// Embedding hosts use this prepare-once/call-many boundary to avoid routing
/// native values back through source text. Namespace, protocol, and host-call
/// contexts remain controlled by the caller.
pub fn invoke_callable(callable: Value, arguments: Vec<Value>) -> Result<Value, String> {
    call_value(callable, arguments)
}

pub(crate) fn call_function(function: &Function, arguments: Vec<Value>) -> Result<Value, String> {
    #[cfg(feature = "evaluation-journal")]
    let operation = evaluation_journal_enter(function, &arguments);
    if let Some(native) = &function.native {
        if function.variadic.is_none() && function.params.len() != arguments.len() {
            #[cfg(feature = "evaluation-journal")]
            evaluation_journal_exit(operation, function, None);
            return Err(format!(
                "function expects {} arguments",
                function.params.len()
            ));
        }
        let result = native(arguments);
        #[cfg(feature = "evaluation-journal")]
        evaluation_journal_exit(operation, function, result.as_ref().ok());
        return result;
    }
    let tracing = tracing_enabled();
    if tracing {
        TRACE_STACK.with(|stack| {
            stack.borrow_mut().push(
                function
                    .name
                    .clone()
                    .unwrap_or_else(|| "<anonymous>".into()),
            )
        });
    }
    let caller_scoped_foundation = function.namespace.as_deref() == Some("std.foundation")
        && matches!(
            function.name.as_deref(),
            Some(
                "macroexpand"
                    | "env-current"
                    | "env-snapshot"
                    | "env-vars"
                    | "env-namespaces"
                    | "env-namespace"
                    | "env-module"
                    | "env-resolve"
            )
        );
    let namespace_scope = namespace_registry().ok().and_then(|registry| {
        (!caller_scoped_foundation)
            .then_some(())
            .and_then(|_| function.namespace.as_ref())
            .map(|namespace| {
            let previous = registry.current().name().as_str().to_owned();
            registry.set_current(namespace);
            (registry, previous)
        })
    });
    let result = (|| {
        if function.variadic.is_none() && function.params.len() != arguments.len() {
            return Err(format!(
                "function expects {} arguments",
                function.params.len()
            ));
        }
        if arguments.len() < function.params.len() {
            return Err(format!(
                "function expects at least {} arguments",
                function.params.len()
            ));
        }
        let mut env = function.captured.borrow().clone();
        for (name, value) in function
            .params
            .iter()
            .zip(arguments.iter().take(function.params.len()))
        {
            env.insert(name.clone(), value.clone());
        }
        let mut bound = Vec::new();
        for (pattern, value) in function
            .patterns
            .iter()
            .zip(arguments.iter().take(function.params.len()))
        {
            bind_pattern(pattern, value.clone(), &mut env, &mut bound, None)?;
        }
        if let Some(name) = &function.variadic {
            let rest = Value::List(arguments.into_iter().skip(function.params.len()).collect());
            env.insert(name.clone(), rest.clone());
            if let Some(pattern) = &function.variadic_pattern {
                bind_pattern(pattern, rest, &mut env, &mut bound, None)?;
            }
        }
        let mut result = Value::Nil;
        for form in &function.body {
            result = eval(form, &mut env)?;
            if matches!(result, Value::Recur(_)) {
                return Err("recur must be inside loop".into());
            }
        }
        Ok(result)
    })();
    if let Some((registry, previous)) = namespace_scope {
        registry.set_current(previous);
    }
    let result = result.map_err(append_trace);
    #[cfg(feature = "evaluation-journal")]
    evaluation_journal_exit(operation, function, result.as_ref().ok());
    if tracing {
        TRACE_STACK.with(|stack| {
            stack.borrow_mut().pop();
        });
    }
    result
}

/// Runs one evaluator operation with a bounded evaluation journal. This is
/// intentionally separate from the legacy stack-trace flag above.
#[cfg(feature = "evaluation-journal")]
pub(crate) fn with_evaluation_journal<T>(
    journal_id: crate::journal::JournalId,
    limits: crate::journal::JournalLimits,
    evaluate: impl FnOnce() -> Result<T, String>,
    preview: impl FnOnce(&T, &crate::journal::JournalCollector) -> crate::journal::ValuePreview,
) -> (Result<T, String>, crate::journal::Journal) {
    EVALUATION_JOURNAL_STACK.with(|stack| stack.borrow_mut().clear());
    let previous = EVALUATION_JOURNAL.with(|active| {
        active.replace(Some(crate::journal::JournalCollector::new(
            journal_id, limits,
        )))
    });
    assert!(
        previous.is_none(),
        "nested evaluation journals are not supported yet"
    );
    EVALUATION_JOURNAL.with(|active| {
        active
            .borrow_mut()
            .as_mut()
            .expect("evaluation journal must be active")
            .record(crate::journal::JournalEvent::new(
                crate::journal::JournalEventKind::EvaluationStart,
            ));
    });
    let result = evaluate();
    let collector = EVALUATION_JOURNAL.with(|active| {
        active
            .replace(previous)
            .expect("evaluation journal must be active")
    });
    EVALUATION_JOURNAL_STACK.with(|stack| stack.borrow_mut().clear());
    let trace = match &result {
        Ok(value) => {
            let result = preview(value, &collector);
            collector.finish(result)
        }
        Err(error) => collector.fail(error.clone()),
    };
    (result, trace)
}

pub(crate) fn binding_symbol(
    form: &Form,
    context: &str,
) -> Result<(String, Option<Rc<Metadata>>), String> {
    match form {
        Form::Symbol(name) => Ok((name.clone(), None)),
        Form::Metadata(metadata, value) => match value.as_ref() {
            Form::Symbol(name) => Ok((name.clone(), Some(metadata_from_form(metadata)?))),
            _ => Err(format!("{context} must be a symbol")),
        },
        _ => Err(format!("{context} must be a symbol")),
    }
}

fn syntax_quote_collection(
    values: &[Form],
    vector: bool,
    env: &mut HashMap<String, Value>,
) -> Result<Value, String> {
    let mut output = Vec::new();
    for value in values {
        match value {
            Form::List(parts)
                if !parts.is_empty()
                    && matches!(&parts[0], Form::Symbol(name) if name == "unquote") =>
            {
                if parts.len() != 2 {
                    return Err("unquote expects one argument".into());
                }
                output.push(eval(&parts[1], env)?);
            }
            Form::List(parts)
                if !parts.is_empty()
                    && matches!(&parts[0], Form::Symbol(name) if name == "unquote-splicing") =>
            {
                if parts.len() != 2 {
                    return Err("unquote-splicing expects one argument".into());
                }
                output.extend(iterator_values(eval(&parts[1], env)?)?);
            }
            value => output.push(syntax_quote_value(value, env)?),
        }
    }
    if vector {
        vector_literal(output)
    } else {
        Ok(Value::List(output.into()))
    }
}

fn syntax_quote_value(form: &Form, env: &mut HashMap<String, Value>) -> Result<Value, String> {
    match form {
        Form::Symbol(_) => literal_value(form),
        Form::List(values)
            if values.len() == 2
                && matches!(&values[0], Form::Symbol(name) if name == "unquote") =>
        {
            eval(&values[1], env)
        }
        Form::List(values) => syntax_quote_collection(values, false, env),
        Form::Vector(values) => syntax_quote_collection(values, true, env),
        Form::Map(values) => Ok(Value::Map(
            values
                .iter()
                .map(|(key, value)| {
                    Ok((
                        syntax_quote_value(key, env)?,
                        syntax_quote_value(value, env)?,
                    ))
                })
                .collect::<Result<Vec<_>, String>>()?
                .into_iter()
                .collect(),
        )),
        _ => literal_value(form),
    }
}
