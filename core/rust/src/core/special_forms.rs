thread_local! {
    static PRINTER_CAPTURES: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

fn printer_write(text: &str) -> Result<(), String> {
    use std::io::Write;
    if PRINTER_CAPTURES.with(|captures| {
        let mut captures = captures.borrow_mut();
        captures
            .last_mut()
            .map(|output| output.push_str(text))
            .is_some()
    }) {
        return Ok(());
    }
    print!("{text}");
    std::io::stdout()
        .flush()
        .map_err(|error| format!("Printer output failed: {error}"))
}

// This compatibility evaluator is also used while loading source-backed
// namespaces. Keeping its large dispatch match out of line prevents the
// recursive namespace/evaluator path from multiplying that frame until a
// normal test or Wasm stack overflows. The fiber evaluator remains the
// stack-safe execution path for ordinary evaluation.
#[inline(never)]
pub fn eval(form: &Form, env: &mut HashMap<String, Value>) -> Result<Value, String> {
    check_evaluation_interrupt()?;
    match form {
        Form::Number(v) => Ok(Value::Number(*v)),
        Form::String(v) => Ok(Value::String(v.clone())),
        Form::Keyword(v) => Ok(Value::Keyword(v.clone().into())),
        Form::Nil => Ok(Value::Nil),
        Form::Bool(value) => Ok(Value::Bool(*value)),
        Form::Character(value) => Ok(Value::Character(*value)),
        Form::Float(value) => Ok(Value::Float(crate::numeric::finite_float(*value)?)),
        Form::BigInteger(value) => Ok(Value::BigInteger(value.clone())),
        Form::Regex(value) => Ok(Value::Regex(value.clone())),
        Form::Tagged(tag, value) if tag == "ptr" => pointer_from_descriptor(literal_value(value)?),
        Form::Tagged(tag, value) => Ok(Value::Tagged(Box::new(PTaggedLiteral::new(
            Symbol::parse(tag),
            literal_value(value)?,
        )))),
        Form::Metadata(_, value) => eval(value, env),
        Form::List(fs)
            if fs.len() == 2 && matches!(&fs[0], Form::Symbol(name) if name == "syntax-quote") =>
        {
            syntax_quote_value(&fs[1], env)
        }
        Form::List(fs)
            if fs.len() == 2 && matches!(&fs[0], Form::Symbol(name) if name == "quote") =>
        {
            literal_value(&fs[1])
        }
        Form::List(fs) if matches!(fs.first(), Some(Form::Symbol(name)) if name == "comment") => {
            Ok(Value::Nil)
        }
        Form::Map(values) => Ok(Value::Map(
            values
                .iter()
                .map(|(key, value)| Ok((eval(key, env)?, eval(value, env)?)))
                .collect::<Result<_, String>>()?,
        )),
        Form::Set(values) => Ok(Value::OrderedSet(Box::new(
            unique_values(
                values
                    .iter()
                    .map(|value| eval(value, env))
                    .collect::<Result<_, _>>()?,
            )
            .into_iter()
            .collect(),
        ))),
        Form::Vector(values) => vector_literal(
            values
                .iter()
                .map(|value| eval(value, env))
                .collect::<Result<_, _>>()?,
        ),
        Form::Symbol(n) if n == "nil" => Ok(Value::Nil),
        Form::Symbol(n) if n == "true" => Ok(Value::Bool(true)),
        Form::Symbol(n) if n == "false" => Ok(Value::Bool(false)),
        Form::Symbol(n) => {
            if n.contains('/') {
                if let Ok(registry) = namespace_registry() {
                    if let Some((namespace, _)) = n.split_once('/') {
                        if registry.load_state(namespace) == Some(NamespaceLoadState::Failed) {
                            return Err(previously_failed_error(&registry, namespace));
                        }
                    }
                    force_lazy_alias(&registry, env, n)?;
                }
            }
            if let Some(value) = binding_value(env, n) {
                return Ok(value);
            }
            if !n.contains('/') {
                if let Ok(registry) = namespace_registry() {
                    if let Some((_, namespace)) = registry
                        .current()
                        .aliases()
                        .into_iter()
                        .find(|(alias, _)| alias.as_str() == n)
                    {
                        return Ok(Value::Namespace(Rc::new(namespace)));
                    }
                    if let Some(namespace) = registry.find(n) {
                        return Ok(Value::Namespace(Rc::new(namespace)));
                    }
                }
            }
            Err(format!("unbound symbol: {n}"))
        }
        Form::List(fs) if fs.is_empty() => Ok(Value::List(PList::new())),
        Form::List(fs) => {
            let normalized_operator;
            let operator = match &fs[0] {
                Form::Symbol(name) if name.starts_with("Iter/iter-") => {
                    normalized_operator = Form::Symbol(name[5..].to_owned());
                    &normalized_operator
                }
                operator => operator,
            };
            if let Form::Symbol(name) = operator {
                if foundation_fallback_omitted(env, name) {
                    return Err(format!("unbound symbol: {name}"));
                }
            }
            match operator {
                Form::Symbol(n) if n == "fn" => {
                    if fs.len() < 3 {
                        return Err("fn expects parameters and a body".into());
                    }
                    if !matches!(form_without_metadata(&fs[1]), Form::Vector(_)) {
                        return multi_arity_function("<anonymous>", &fs[1..], env, false);
                    }
                    let (params, variadic, patterns, variadic_pattern) = function_parts(&fs[1])?;
                    let body = fs[2..].to_vec();
                    Ok(Value::Function(Rc::new(Function {
                        params,
                        variadic,
                        patterns,
                        variadic_pattern,
                        captured: Rc::new(RefCell::new(capture_environment(&body, env))),
                        body,
                        name: None,
                        namespace: function_definition_namespace(),
                        native: None,
                        fiber_native: None,
                        clauses: Vec::new(),
                        metadata: None,
                        is_macro: false,
                    })))
                }
                Form::Symbol(n) if n == "letfn" => {
                    if fs.len() < 3 {
                        return Err("letfn expects a function binding vector and a body".into());
                    }
                    let definitions = match &fs[1] {
                        Form::Vector(values) => values,
                        _ => {
                            return Err("letfn expects a function binding vector and a body".into())
                        }
                    };
                    let mut capture_forms = fs[2..].to_vec();
                    capture_forms.extend(definitions.iter().cloned());
                    let captured = Rc::new(RefCell::new(capture_environment(&capture_forms, env)));
                    let mut functions = Vec::with_capacity(definitions.len());
                    let mut names = std::collections::HashSet::new();
                    for definition in definitions {
                        let Form::List(parts) = definition else {
                            return Err(
                                "letfn definitions must be (name [arguments] body...)".into()
                            );
                        };
                        if parts.len() < 3 {
                            return Err(
                                "letfn definitions must be (name [arguments] body...)".into()
                            );
                        }
                        let Form::Symbol(name) = &parts[0] else {
                            return Err("letfn names must be unqualified symbols".into());
                        };
                        if name.contains('/') {
                            return Err("letfn names must be unqualified symbols".into());
                        }
                        if !names.insert(name.clone()) {
                            return Err(format!("Duplicate letfn name: {name}"));
                        }
                        let (params, variadic, patterns, variadic_pattern) =
                            function_parts(&parts[1])
                                .map_err(|_| "letfn parameters must be a binding vector")?;
                        functions.push((
                            name.clone(),
                            Value::Function(Rc::new(Function {
                                params,
                                variadic,
                                patterns,
                                variadic_pattern,
                                body: parts[2..].to_vec(),
                                captured: captured.clone(),
                                name: Some(name.clone()),
                                namespace: function_definition_namespace(),
                                native: None,
                                fiber_native: None,
                                clauses: Vec::new(),
                                metadata: None,
                                is_macro: false,
                            })),
                        ));
                    }
                    for (name, function) in &functions {
                        captured.borrow_mut().insert(name.clone(), function.clone());
                    }
                    let mut previous = Vec::with_capacity(functions.len());
                    for (name, function) in functions {
                        previous.push((name.clone(), env.insert(name, function)));
                    }
                    let mut result = Ok(Value::Nil);
                    for body in &fs[2..] {
                        result = eval(body, env);
                        if result.is_err() {
                            break;
                        }
                    }
                    for (name, old) in previous.into_iter().rev() {
                        if let Some(old) = old {
                            env.insert(name, old);
                        } else {
                            env.remove(&name);
                        }
                    }
                    result
                }
                Form::Symbol(n) if n == "read-forms" => {
                    if fs.len() != 2 {
                        return Err("read-forms expects a path string".into());
                    }
                    let path = match eval(&fs[1], env)? {
                        Value::String(path) => path,
                        _ => return Err("read-forms expects a path string".into()),
                    };
                    if !(path.ends_with(".hal") || path.ends_with(".hrl")) {
                        return Err("read-forms expects a .hal or .hrl path".into());
                    }
                    let promise = file_provider("read-forms")?
                        .read(&path)
                        .map_err(|error| file_error("read-forms", error))?;
                    let bytes = match promise.wait_state() {
                        PromiseState::Fulfilled(Value::Bytes(bytes)) => bytes,
                        PromiseState::Fulfilled(Value::ByteBuffer(bytes)) => bytes.borrow().clone(),
                        PromiseState::Fulfilled(value) => {
                            return Err(format!(
                                "read-forms expected file bytes, got {}",
                                value.display()
                            ))
                        }
                        PromiseState::Rejected(error) => {
                            return Err(promise_rejection_error(error))
                        }
                        PromiseState::Pending => {
                            return Err("read-forms file read is still pending".into())
                        }
                    };
                    let source = String::from_utf8(bytes)
                        .map_err(|_| format!("read-forms source is not UTF-8: {path}"))?;
                    let forms = crate::kernel::parse_forms(&source)
                        .map_err(|error| format!("read-forms failed: {error}"))?;
                    let values = forms
                        .iter()
                        .map(form_to_value)
                        .collect::<Result<Vec<_>, _>>()?;
                    Ok(Value::Vector(PVector::from_iter(values)))
                }
                Form::Symbol(n) if n.ends_with("/var-sym") => {
                    if fs.len() != 2 {
                        return Err("var-sym expects one var".into());
                    }
                    let target = match &fs[1] {
                        Form::Symbol(name) => match env.get(name) {
                            Some(Value::Var(var)) => Value::Var(var.clone()),
                            _ => eval(&fs[1], env)?,
                        },
                        _ => eval(&fs[1], env)?,
                    };
                    match target {
                        Value::Var(var) => Ok(Value::Symbol(var.symbol().clone())),
                        value => Err(format!("var-sym expects a var, got {}", value.display())),
                    }
                }
                Form::Symbol(n) if n == "var" => {
                    if fs.len() != 2 {
                        return Err("var expects a symbol".into());
                    }
                    let name = match &fs[1] {
                        Form::Symbol(name) => name,
                        _ => return Err("var expects a symbol".into()),
                    };
                    if name.contains('/') {
                        if let Ok(registry) = namespace_registry() {
                            if let Some((namespace, _)) = name.split_once('/') {
                                if registry.load_state(namespace)
                                    == Some(NamespaceLoadState::Failed)
                                {
                                    return Err(previously_failed_error(&registry, namespace));
                                }
                            }
                            force_lazy_alias(&registry, env, name)?;
                        }
                    }
                    let cell =
                        binding_var(env, name).ok_or_else(|| format!("unbound symbol: {name}"))?;
                    Ok(Value::Var(cell))
                }
                Form::Symbol(n) if n == "hash" => {
                    eval_basic_object_form(n, fs, env)
                }
                Form::Symbol(n) if n == "set!" || n == "var/set" => {
                    if fs.len() != 3 {
                        return Err(format!("{n} expects a symbol and value"));
                    }
                    if n == "set!" {
                        if let Form::List(place) = &fs[1] {
                            if matches!(place.first(), Some(Form::Symbol(operation)) if operation == "field")
                            {
                                if place.len() != 3 {
                                    return Err(
                                        "set! field place expects a receiver and field".into()
                                    );
                                }
                                let field = match &place[2] {
                                    Form::Keyword(field) if !field.contains('/') => field.as_str(),
                                    Form::Symbol(field) if !field.contains('/') => field.as_str(),
                                    _ => {
                                        return Err(
                                            "set! field place expects an unqualified literal field"
                                                .into(),
                                        )
                                    }
                                };
                                let receiver = eval(&place[1], env)?;
                                let replacement = eval(&fs[2], env)?;
                                return mutable_field_set(&receiver, field, replacement);
                            }
                        }
                    }
                    let name = match &fs[1] {
                        Form::Symbol(name) => name,
                        _ => return Err(format!("{n} expects a symbol")),
                    };
                    let value = eval(&fs[2], env)?;
                    let cell =
                        binding_var(env, name).ok_or_else(|| format!("unbound var: {name}"))?;
                    if !binding_is_local(&cell) {
                        return Err(format!(
                            "Cannot replace referred Var without ns omission: {name}"
                        ));
                    }
                    cell.reset_value(value.clone());
                    Ok(value)
                }
                Form::Symbol(n) if n == "__throw-at" => {
                    let [_, Form::Number(line), Form::Number(column), value] = fs.as_slice() else {
                        return Err("internal throw location marker is malformed".into());
                    };
                    let value = eval(value, env)?;
                    if !matches!(value, Value::ExceptionInfo(_)) {
                        return Err("throw expects an Exception value created by ex".into());
                    }
                    Err(thrown_error_at(
                        value,
                        exception_site_at(*line as usize, *column as usize),
                    ))
                }
                Form::Symbol(n) if n == "throw" => {
                    if fs.len() != 2 {
                        return Err("throw expects one value".into());
                    }
                    let value = eval(&fs[1], env)?;
                    if !matches!(value, Value::ExceptionInfo(_)) {
                        return Err("throw expects an Exception value created by ex".into());
                    }
                    Err(thrown_error(value))
                }
                Form::Symbol(n) if n == "__ex-at" => {
                    let [_, Form::Number(line), Form::Number(column), rest @ ..] = fs.as_slice()
                    else {
                        return Err("internal exception location marker is malformed".into());
                    };
                    if rest.is_empty() {
                        return Err("internal exception location marker is malformed".into());
                    }
                    let expression = Form::List(rest.to_vec());
                    with_exception_site(
                        exception_site_at(*line as usize, *column as usize)
                            .expect("exception site always exists"),
                        || eval(&expression, env),
                    )
                }
                Form::Symbol(n) if n == "try" => {
                    if fs.len() < 2 {
                        return Err("try expects a body".into());
                    }
                    let mut body = Vec::new();
                    let mut catch_forms = Vec::new();
                    let mut finally_forms = Vec::new();
                    let mut clauses_started = false;
                    for form in &fs[1..] {
                        match form {
                            Form::List(parts)
                                if !parts.is_empty()
                                    && matches!(&parts[0],Form::Symbol(name) if name=="catch") =>
                            {
                                clauses_started = true;
                                catch_forms.push(parts)
                            }
                            Form::List(parts)
                                if !parts.is_empty()
                                    && matches!(&parts[0],Form::Symbol(name) if name=="finally") =>
                            {
                                clauses_started = true;
                                finally_forms.extend_from_slice(&parts[1..])
                            }
                            _ if !clauses_started => body.push(form),
                            _ => return Err("try clauses must follow the body".into()),
                        }
                    }
                    let mut result = Ok(Value::Nil);
                    for form in body {
                        result = eval(form, env);
                        if result.is_err() {
                            break;
                        }
                    }
                    if let Err(ref error) = result {
                        for parts in catch_forms {
                            if parts.len() < 3 {
                                return Err("catch expects a selector, name, and body".into());
                            }
                            let (selector, binding_index, body_index) = match parts.as_slice() {
                                [_, Form::Symbol(name), _]
                                    if name != "Exception" && name != "Throwable" =>
                                {
                                    ("Exception".to_owned(), 1, 2)
                                }
                                [_, Form::Symbol(class), Form::Symbol(_), ..] => {
                                    (class.clone(), 2, 3)
                                }
                                [_, Form::Keyword(code), Form::Symbol(_), ..]
                                    if code.contains('/') =>
                                {
                                    (format!(":{code}"), 2, 3)
                                }
                                [_, Form::Vector(codes), Form::Symbol(_), ..]
                                    if !codes.is_empty()
                                        && codes.iter().all(|code| matches!(code, Form::Keyword(name) if name.contains('/'))) =>
                                {
                                    let selectors = codes
                                        .iter()
                                        .map(|code| match code {
                                            Form::Keyword(name) => format!(":{name}"),
                                            _ => unreachable!(),
                                        })
                                        .collect::<Vec<_>>()
                                        .join(",");
                                    (format!("[{selectors}]"), 2, 3)
                                }
                                _ => return Err("catch selector must be a namespaced keyword, a non-empty vector of namespaced keywords, or omitted".into()),
                            };
                            if !catch_matches(error, &selector) {
                                continue;
                            }
                            let name = match &parts[binding_index] {
                                Form::Symbol(name) => name.clone(),
                                _ => return Err("catch name must be a symbol".into()),
                            };
                            let old = env.insert(name.clone(), caught_error(error));
                            result = Ok(Value::Nil);
                            for form in &parts[body_index..] {
                                result = eval(form, env);
                                if result.is_err() {
                                    break;
                                }
                            }
                            if let Some(old) = old {
                                env.insert(name, old);
                            } else {
                                env.remove(&name);
                            }
                            break;
                        }
                    }
                    for form in finally_forms {
                        let final_result = eval(&form, env);
                        if final_result.is_err() {
                            result = final_result;
                        }
                    }
                    result
                }
                Form::Symbol(n) if n == "def" => {
                    if fs.len() != 3 {
                        return Err("def expects a name and value".into());
                    }
                    let (name, metadata) = binding_symbol(&fs[1], "def name")?;
                    prepare_owned_definition(env, &name)?;
                    let value = eval(&fs[2], env)?;
                    let var = if namespace_registry().is_ok() {
                        let var = vm_def_global(&name, value, metadata)?;
                        env.insert(name, Value::Var(var.clone()));
                        var
                    } else if let Some(Value::Var(var)) = env.get(&name) {
                        if !binding_is_local(var) {
                            let var = KernelVar::new(local_var_name(&name), value.clone());
                            var.set_origin(definition_origin());
                            var.set_hara_metadata(metadata);
                            env.insert(name, Value::Var(var.clone()));
                            var
                        } else {
                            var.reset_value(value);
                            var.set_origin(definition_origin());
                            if metadata.is_some() {
                                var.set_hara_metadata(metadata);
                            }
                            var.clone()
                        }
                    } else {
                        let var = KernelVar::new(local_var_name(&name), value);
                        var.set_origin(definition_origin());
                        var.set_hara_metadata(metadata);
                        env.insert(name, Value::Var(var.clone()));
                        var
                    };
                    refresh_schema_contract(&var)?;
                    Ok(Value::Var(var))
                }
                Form::Symbol(n) if n == "declare" => {
                    if fs.len() < 2 {
                        return Err("declare expects at least one symbol".into());
                    }
                    for form in &fs[1..] {
                        let name = match form {
                            Form::Symbol(name) => name.clone(),
                            _ => return Err("declare expects symbols".into()),
                        };
                        prepare_owned_definition(env, &name)?;
                        let cell = match env.get(&name) {
                            Some(Value::Var(cell)) if binding_is_local(cell) => cell.clone(),
                            _ => KernelVar::new(local_var_name(&name), Value::Nil),
                        };
                        cell.set_origin(definition_origin());
                        env.insert(name, Value::Var(cell));
                    }
                    Ok(Value::Nil)
                }
                Form::Symbol(n) if n == "defstruct" || n == "defmutable" => {
                    if fs.len() < 3 {
                        return Err(format!("{n} expects a name and field vector"));
                    }
                    let name = match &fs[1] {
                        Form::Symbol(name) if !name.contains('/') => name.clone(),
                        _ => return Err(format!("{n} name must be an unqualified symbol")),
                    };
                    let fields = match &fs[2] {
                        Form::Vector(fields) => fields
                            .iter()
                            .map(|field| match field {
                                Form::Symbol(field) if !field.contains('/') => Ok(field.clone()),
                                _ => Err(format!("{n} field names must be unqualified symbols")),
                            })
                            .collect::<Result<Vec<_>, String>>()?,
                        _ => return Err(format!("{n} expects a field vector")),
                    };
                    validate_named_definition(n, &name, &fields)?;
                    let namespace = namespace_registry()?.current().name().as_str().to_owned();
                    let (type_value, map_constructor) = if n == "defstruct" {
                        let ty = Rc::new(StructType {
                            name: format!("{namespace}/{name}"),
                            fields,
                        });
                        let map_type = ty.clone();
                        let constructor =
                            native_function(&format!("map->{name}"), 1, move |values| {
                                let source = values.first().expect("native arity is checked");
                                let values = map_type
                                    .fields
                                    .iter()
                                    .map(|field| {
                                        map_value(source, &named_field_key(field))
                                            .cloned()
                                            .unwrap_or(Value::Nil)
                                    })
                                    .collect();
                                Ok(Value::Struct(Rc::new(StructValue::from_values(
                                    map_type.clone(),
                                    values,
                                    None,
                                )?)))
                            });
                        (Value::StructType(ty), constructor)
                    } else {
                        let ty = Rc::new(MutableType {
                            name: format!("{namespace}/{name}"),
                            fields,
                        });
                        let map_type = ty.clone();
                        let constructor =
                            native_function(&format!("map->{name}"), 1, move |values| {
                                let source = values.first().expect("native arity is checked");
                                let values = map_type
                                    .fields
                                    .iter()
                                    .map(|field| {
                                        map_value(source, &named_field_key(field))
                                            .cloned()
                                            .unwrap_or(Value::Nil)
                                    })
                                    .collect();
                                Ok(Value::Mutable(Rc::new(MutableValue::from_values(
                                    map_type.clone(),
                                    values,
                                    None,
                                )?)))
                            });
                        (Value::MutableType(ty), constructor)
                    };
                    for (binding, value) in [
                        (name.clone(), type_value.clone()),
                        (format!("->{name}"), type_value),
                        (format!("map->{name}"), map_constructor),
                    ] {
                        let var = KernelVar::new(format!("{namespace}/{binding}"), value);
                        var.set_origin(definition_origin());
                        env.insert(binding, Value::Var(var));
                    }
                    let mut index = 3;
                    while index < fs.len() {
                        let Form::Symbol(protocol) = &fs[index] else {
                            return Err(format!("{n} protocol clause expects a protocol symbol"));
                        };
                        index += 1;
                        let start = index;
                        while index < fs.len() && matches!(&fs[index], Form::List(_)) {
                            index += 1;
                        }
                        if start == index {
                            return Err(format!(
                                "{n} protocol clause requires method implementations"
                            ));
                        }
                        let extension = Form::List(
                            std::iter::once(Form::Symbol("extend-type".into()))
                                .chain(std::iter::once(Form::Symbol(name.clone())))
                                .chain(std::iter::once(Form::Symbol(protocol.clone())))
                                .chain(fs[start..index].iter().cloned())
                                .collect(),
                        );
                        eval(&extension, env)?;
                    }
                    Ok(Value::Nil)
                }
                Form::Symbol(n) if n == "field" => {
                    if fs.len() != 3 {
                        return Err("field expects a mutable value and field name".into());
                    }
                    let field = match &fs[2] {
                        Form::Keyword(field) | Form::Symbol(field) if !field.contains('/') => field,
                        _ => {
                            return Err("field name must be an unqualified keyword or symbol".into())
                        }
                    };
                    let value = eval(&fs[1], env)?;
                    mutable_field_value(&value, field)
                }
                Form::Symbol(n) if n == "defprotocol" => {
                    if fs.len() < 3 {
                        return Err("defprotocol expects a name and method declarations".into());
                    }
                    let name = match &fs[1] {
                        Form::Symbol(name) if !name.contains('/') => name.clone(),
                        _ => return Err("defprotocol name must be an unqualified symbol".into()),
                    };
                    let mut methods = HashMap::new();
                    for declaration in &fs[2..] {
                        let Form::List(parts) = declaration else {
                            return Err("defprotocol method declaration must be a list".into());
                        };
                        if parts.len() != 2
                            || !matches!(&parts[0], Form::Symbol(_))
                            || !matches!(&parts[1], Form::Vector(_))
                        {
                            return Err(
                            "defprotocol method declaration expects a name and parameter vector"
                                .into(),
                        );
                        }
                        let Form::Symbol(method) = &parts[0] else {
                            unreachable!()
                        };
                        let Form::Vector(arguments) = &parts[1] else {
                            unreachable!()
                        };
                        if arguments.is_empty()
                            || methods.insert(method.clone(), arguments.len()).is_some()
                        {
                            return Err(
                                "protocol methods must be unique and take a receiver".into()
                            );
                        }
                    }
                    let namespace = namespace_registry()?.current().name().as_str().to_owned();
                    let protocol = Value::Protocol(Rc::new(GuestProtocol {
                        name: format!("{namespace}/{name}"),
                        methods,
                        parents: Vec::new(),
                    }));
                    if let Value::Protocol(protocol_value) = &protocol {
                        let current = namespace_registry()?.current();
                        let previous_protocol = env
                            .get(&name)
                            .cloned()
                            .map(deref_value)
                            .and_then(|value| match value {
                                Value::Protocol(previous)
                                    if previous.name == protocol_value.name =>
                                {
                                    Some(previous)
                                }
                                _ => None,
                            })
                            .or_else(|| {
                                current
                                    .resolve(&crate::lang::data::Symbol::parse(&name))
                                    .filter(|var| var.get_namespace() == Some(namespace.as_str()))
                                    .and_then(|var| match var.deref_value() {
                                        Value::Protocol(previous) => Some(previous),
                                        _ => None,
                                    })
                            });
                        for method in protocol_value.methods.keys() {
                            for (local, var) in current.mappings() {
                                if local.as_str() == name
                                    || var.get_namespace() != Some(namespace.as_str())
                                {
                                    continue;
                                }
                                if let Value::Protocol(other) = var.deref_value() {
                                    if other.methods.contains_key(method) {
                                        return Err(format!(
                                        "Protocol method Var already belongs to {}: {namespace}/{method}",
                                        local.as_str()
                                    ));
                                    }
                                }
                            }
                            let existing_namespace_var = current
                                .resolve(&crate::lang::data::Symbol::parse(method))
                                .filter(|var| var.get_namespace() == Some(namespace.as_str()));
                            let existing_environment_var =
                                matches!(env.get(method), Some(Value::Var(_)));
                            let same_protocol_reload = (existing_namespace_var.is_some()
                                || existing_environment_var)
                                && previous_protocol
                                    .as_ref()
                                    .is_some_and(|previous| previous.methods.contains_key(method));
                            if (existing_namespace_var.is_some() || existing_environment_var)
                                && !same_protocol_reload
                            {
                                return Err(format!(
                                    "Protocol method Var already exists: {namespace}/{method}"
                                ));
                            }
                        }
                        ACTIVE_PROTOCOLS.with(|active| -> Result<(), String> {
                            let registry = active.borrow();
                            let registry = registry
                                .as_ref()
                                .ok_or_else(|| "protocol registry is unavailable".to_string())?;
                            registry.replace_guest_protocol(protocol_value.name.clone());
                            for method in protocol_value.methods.keys() {
                                registry.declare_guest(protocol_value.name.clone(), method.clone());
                            }
                            Ok(())
                        })?;
                        for method in protocol_value.methods.keys() {
                            let local_name = method.clone();
                            let qualified_name = format!("{namespace}/{method}");
                            let protocol_name = protocol_value.name.clone();
                            let method_name = method.clone();
                            let function_name = qualified_name.clone();
                            let method_value =
                                native_variadic_function(&function_name, move |arguments| {
                                    protocol_call(&protocol_name, &method_name, &arguments)
                                });
                            let method_var = current.intern(&local_name, method_value);
                            method_var.set_origin(definition_origin());
                            env.insert(local_name, Value::Var(method_var.clone()));
                            env.insert(qualified_name, Value::Var(method_var));
                        }
                    }
                    let var = KernelVar::new(format!("{namespace}/{name}"), protocol.clone());
                    var.set_origin(definition_origin());
                    env.insert(name, Value::Var(var));
                    Ok(protocol)
                }
                Form::Symbol(n) if n == "extend-type" => {
                    if fs.len() < 4 {
                        return Err(
                            "extend-type expects a type, protocol, and method implementations"
                                .into(),
                        );
                    }
                    let type_name = match eval(&fs[1], env)? {
                        Value::StructType(ty) => ty.name.clone(),
                        Value::MutableType(ty) => ty.name.clone(),
                        _ => return Err("extend-type expects a struct or mutable type".into()),
                    };
                    let protocol = match eval(&fs[2], env)? {
                        Value::Protocol(protocol) => protocol,
                        _ => return Err("extend-type expects a protocol".into()),
                    };
                    let mut seen = HashSet::new();
                    for implementation in &fs[3..] {
                        let Form::List(parts) = implementation else {
                            return Err("extend-type implementations must be method forms".into());
                        };
                        if parts.len() < 3 {
                            return Err("extend-type implementations require a body".into());
                        }
                        let Form::Symbol(method) = &parts[0] else {
                            return Err("extended method name must be a symbol".into());
                        };
                        let Form::Vector(arguments) = &parts[1] else {
                            return Err("extended method arguments must be a vector".into());
                        };
                        if !seen.insert(method.clone()) {
                            return Err("Duplicate extended method".into());
                        }
                        let valid_arity = protocol.methods.get(method).is_some_and(|expected| {
                            *expected == arguments.len()
                                || (*expected == usize::MAX && !arguments.is_empty())
                        });
                        if !valid_arity {
                            return Err(format!(
                                "invalid protocol method implementation: {method}"
                            ));
                        }
                        let function = eval(
                            &Form::List(
                                std::iter::once(Form::Symbol("fn".into()))
                                    .chain(parts[1..].iter().cloned())
                                    .collect(),
                            ),
                            env,
                        )?;
                        let Value::Function(function) = function else {
                            unreachable!()
                        };
                        ACTIVE_PROTOCOLS.with(|active| -> Result<(), String> {
                            let registry = active.borrow();
                            let registry = registry
                                .as_ref()
                                .ok_or_else(|| "protocol registry is unavailable".to_string())?;
                            registry.register_guest(
                                protocol.name.clone(),
                                type_name.clone(),
                                method.clone(),
                                function,
                            );
                            Ok(())
                        })?;
                    }
                    Ok(Value::Protocol(protocol))
                }
                Form::Symbol(n) if n == "defmulti" => {
                    if fs.len() != 3 {
                        return Err("defmulti expects a name and dispatch function".into());
                    }
                    let Form::Symbol(name) = &fs[1] else {
                        return Err("defmulti name must be an unqualified symbol".into());
                    };
                    if name.contains('/') {
                        return Err("defmulti name must be an unqualified symbol".into());
                    }
                    let Value::Function(dispatch) = eval(&fs[2], env)? else {
                        return Err("defmulti dispatch function must be callable".into());
                    };
                    let namespace = namespace_registry()?.current().name().as_str().to_owned();
                    let qualified = format!("{namespace}/{name}");
                    let state = Rc::new(RefCell::new(MultiMethod {
                        dispatch,
                        methods: Vec::new(),
                        default: None,
                    }));
                    let invoke_state = state.clone();
                    let value = native_variadic_function(&qualified, move |arguments| {
                        let state = invoke_state.borrow();
                        let key = call_function(&state.dispatch, arguments.clone())?;
                        let method = state
                            .methods
                            .iter()
                            .find(|(candidate, _)| *candidate == key)
                            .map(|(_, method)| method.clone())
                            .or_else(|| state.default.clone())
                            .ok_or_else(|| {
                                format!(
                                    "No multimethod method for dispatch value {}",
                                    key.display()
                                )
                            })?;
                        call_function(&method, arguments)
                    });
                    let var = namespace_registry()?.current().intern(name, value.clone());
                    var.set_origin(definition_origin());
                    env.insert(name.clone(), Value::Var(var.clone()));
                    env.insert(qualified.clone(), Value::Var(var));
                    ACTIVE_MULTIMETHODS.with(|active| {
                        active.borrow_mut().insert(qualified, state);
                    });
                    Ok(value)
                }
                Form::Symbol(n) if n == "defmethod" => {
                    if fs.len() < 5 {
                        return Err(
                            "defmethod expects a multifn, dispatch value, parameters, and body"
                                .into(),
                        );
                    }
                    let Form::Symbol(name) = &fs[1] else {
                        return Err("defmethod multifn must be a symbol".into());
                    };
                    let namespace = namespace_registry()?.current().name().as_str().to_owned();
                    let qualified = if name.contains('/') {
                        name.clone()
                    } else {
                        format!("{namespace}/{name}")
                    };
                    let key = eval(&fs[2], env)?;
                    let function = eval(
                        &Form::List(
                            std::iter::once(Form::Symbol("fn".into()))
                                .chain(fs[3..].iter().cloned())
                                .collect(),
                        ),
                        env,
                    )?;
                    let Value::Function(function) = function else {
                        unreachable!()
                    };
                    ACTIVE_MULTIMETHODS.with(|active| {
                    let state = active.borrow().get(&qualified).cloned().ok_or_else(|| "defmethod expects an existing multifn".to_string())?;
                    let mut state = state.borrow_mut();
                    if matches!(&key, Value::Keyword(keyword) if keyword.get_namespace().is_none() && keyword.get_name() == "default") { state.default = Some(function); }
                    else if let Some((_, existing)) = state.methods.iter_mut().find(|(candidate, _)| *candidate == key) { *existing = function; }
                    else { state.methods.push((key, function)); }
                    Ok(Value::Nil)
                })
                }
                Form::Symbol(n) if n == "defmacro" => {
                    if fs.len() < 3 {
                        return Err("defmacro expects a name, parameters, and a body".into());
                    }
                    let (name, metadata) = binding_symbol(&fs[1], "defmacro name")?;
                    let (metadata, rest) = definition_metadata(metadata, &fs[2..], false, true)?;
                    if let Some(Value::Var(var)) = env.get(&name) {
                        if var.symbol().get_namespace() == Some("std.foundation") {
                            namespace_registry()?
                                .current()
                                .unmap(&crate::lang::data::Symbol::parse(&name));
                            env.remove(&name);
                        }
                    }
                    prepare_owned_definition(env, &name)?;
                    let cell = match env.get(&name) {
                        Some(Value::Var(cell)) if binding_is_local(cell) => cell.clone(),
                        _ => KernelVar::new(local_var_name(&name), Value::Nil),
                    };
                    if metadata.is_some() {
                        cell.set_hara_metadata(metadata);
                    }
                    env.insert(name.clone(), Value::Var(cell.clone()));
                    if rest.is_empty() {
                        return Err("defmacro expects a name, parameters, and a body".into());
                    }
                    let function = if matches!(
                        rest.first().map(form_without_metadata),
                        Some(Form::Vector(_))
                    ) {
                        let params = match form_without_metadata(&rest[0]) {
                            Form::Vector(params) => params,
                            _ => unreachable!(),
                        };
                        let mut macro_params =
                            vec![Form::Symbol("&form".into()), Form::Symbol("&env".into())];
                        macro_params.extend_from_slice(params);
                        let (params, variadic, patterns, variadic_pattern) =
                            function_parts(&Form::Vector(macro_params))?;
                        let body = rest[1..].to_vec();
                        Value::Function(Rc::new(Function {
                            params,
                            variadic,
                            patterns,
                            variadic_pattern,
                            captured: Rc::new(RefCell::new(capture_environment(&body, env))),
                            body,
                            name: Some(name.clone()),
                            namespace: function_definition_namespace(),
                            native: None,
                            fiber_native: None,
                            clauses: Vec::new(),
                            metadata: None,
                            is_macro: true,
                        }))
                    } else {
                        let clauses = rest
                            .iter()
                            .map(macro_clause_with_implicit_params)
                            .collect::<Result<Vec<_>, _>>()?;
                        multi_arity_function(&name, &clauses, env, true)?
                    };
                    if let Value::Function(ref function) = function {
                        let namespace = namespace_registry()?.current().name().as_str().to_owned();
                        register_macro(&namespace, &name, function.clone())?;
                    }
                    cell.reset_value(function.clone());
                    cell.set_origin(definition_origin());
                    refresh_schema_contract(&cell)?;
                    Ok(function)
                }
                Form::Symbol(n) if n == "defn" || n == "defn-" => {
                    if fs.len() < 4 {
                        return Err("defn expects a name, parameters, and a body".into());
                    }
                    let (name, metadata) = binding_symbol(&fs[1], "defn name")?;
                    let (metadata, rest) =
                        definition_metadata(metadata, &fs[2..], n == "defn-", false)
                            .map_err(|error| format!("{name}: {error}"))?;
                    if let Some(schema) = schema_var_reference(metadata.as_deref()) {
                        if binding_var(env, schema.as_str()).is_none() {
                            return Err(format!("schema Var does not exist: {schema}"));
                        }
                    }
                    prepare_owned_definition(env, &name)?;
                    let cell = match env.get(&name) {
                        Some(Value::Var(cell)) if binding_is_local(cell) => cell.clone(),
                        _ => KernelVar::new(local_var_name(&name), Value::Nil),
                    };
                    if metadata.is_some() {
                        cell.set_hara_metadata(metadata);
                    }
                    env.insert(name.clone(), Value::Var(cell.clone()));
                    if rest.is_empty() {
                        return Err("defn expects a name, parameters, and a body".into());
                    }
                    let function = if matches!(
                        rest.first().map(form_without_metadata),
                        Some(Form::Vector(_))
                    ) {
                        let (params, variadic, patterns, variadic_pattern) =
                            function_parts(&rest[0])?;
                        let body = rest[1..].to_vec();
                        Value::Function(Rc::new(Function {
                            params,
                            variadic,
                            patterns,
                            variadic_pattern,
                            captured: Rc::new(RefCell::new(capture_environment(&body, env))),
                            body,
                            name: Some(name.clone()),
                            namespace: function_definition_namespace(),
                            native: None,
                            fiber_native: None,
                            clauses: Vec::new(),
                            metadata: None,
                            is_macro: false,
                        }))
                    } else {
                        multi_arity_function(&name, rest, env, false)?
                    };
                    cell.reset_value(function.clone());
                    cell.set_origin(definition_origin());
                    refresh_schema_contract(&cell)?;
                    Ok(Value::Var(cell))
                }
                Form::Symbol(n) if n == "do" => {
                    let mut result = Value::Nil;
                    for form in &fs[1..] {
                        result = eval(form, env)?;
                        if matches!(result, Value::Recur(_)) {
                            return Ok(result);
                        }
                    }
                    Ok(result)
                }
                Form::Symbol(n) if n == "declare" => {
                    for form in &fs[1..] {
                        if !matches!(form, Form::Symbol(_)) {
                            return Err("declare expects symbols".into());
                        }
                    }
                    Ok(Value::Nil)
                }
                Form::Symbol(n) if n == "ns" || n == "ns+" || n == "require" => {
                    eval_namespace_form(fs, env)
                }
                Form::Symbol(n) if n == "std.foundation.coroutine/coroutine?" => {
                    if fs.len() != 2 {
                        return Err("coroutine/coroutine? expects one value".into());
                    }
                    Ok(Value::Bool(matches!(
                        eval(&fs[1], env)?,
                        Value::Coroutine(_)
                    )))
                }
                Form::Symbol(n) if n == "std.foundation.coroutine/status" => {
                    if fs.len() != 2 {
                        return Err("coroutine/status expects one coroutine".into());
                    }
                    match eval(&fs[1], env)? {
                        Value::Coroutine(coroutine) => Ok(coroutine_status(&coroutine)),
                        _ => Err("coroutine/status expects a coroutine".into()),
                    }
                }
                Form::Symbol(n) if n == "std.foundation.coroutine/close" => {
                    if fs.len() != 2 {
                        return Err("coroutine/close expects one coroutine".into());
                    }
                    match eval(&fs[1], env)? {
                        Value::Coroutine(coroutine) => {
                            coroutine_close(&coroutine)?;
                            Ok(Value::Coroutine(coroutine))
                        }
                        _ => Err("coroutine/close expects a coroutine".into()),
                    }
                }
                Form::Symbol(n) if n == "std.foundation.coroutine/yield" => {
                    Err("coroutine/yield requires the fiber evaluator".into())
                }
                Form::Symbol(n) if n == "std.foundation.coroutine/await" => {
                    Err("coroutine/await requires the fiber evaluator".into())
                }
                Form::Symbol(n)
                    if resolve_macro(n).is_none()
                        && binding_value(env, n)
                            .is_some_and(|value| matches!(value, Value::Function(_))) =>
                {
                    let function = binding_value(env, n).expect("namespace function binding was checked");
                    let arguments = fs[1..]
                        .iter()
                        .map(|form| eval(form, env))
                        .collect::<Result<Vec<_>, _>>()?;
                    call_value(function, arguments)
                }
                Form::Symbol(n) if n == "promise/run" => {
                    if fs.len() != 2 {
                        return Err("promise expects one function".into());
                    }
                    let function = match eval(&fs[1], env)? {
                        Value::Function(function) => function,
                        _ => return Err("promise expects a function".into()),
                    };
                    let provider = promise_provider();
                    let task = Rc::new(move || call_function(&function, Vec::new()));
                    Ok(Value::Promise(provider.run(task)))
                }
                Form::Symbol(n)
                    if ["bytes?", "array?", "object?", "regexp?", "uuid?"]
                        .contains(&n.as_str()) =>
                {
                    if fs.len() != 2 {
                        return Err(format!("{n} expects one value"));
                    }
                    let value = eval(&fs[1], env)?;
                    Ok(Value::Bool(match n.as_str() {
                        "bytes?" => matches!(value, Value::Bytes(_) | Value::ByteBuffer(_)),
                        "array?" => matches!(value, Value::Array(_)),
                        "object?" => matches!(value, Value::Object(_)),
                        "regexp?" => matches!(value, Value::Regex(_)),
                        // UUID values are not yet represented by the Rust value model.
                        "uuid?" => false,
                        _ => unreachable!(),
                    }))
                }
                Form::Symbol(n) if n == "promise/from" => {
                    if fs.len() != 2 {
                        return Err("promise/from expects one value".into());
                    }
                    let value = eval(&fs[1], env)?;
                    Ok(Value::Promise(promise_from(value)))
                }
                Form::Symbol(n) if n == "promise/all" => {
                    if fs.len() != 2 {
                        return Err("promise/all expects one collection".into());
                    }
                    Ok(Value::Promise(promise_all(iterator_values(eval(
                        &fs[1], env,
                    )?)?)))
                }
                Form::Symbol(n) if n == "promise/state" => {
                    if fs.len() != 2 {
                        return Err("promise/state expects one promise".into());
                    }
                    let promise = promise_value(&eval(&fs[1], env)?, n)?;
                    Ok(promise_state_value(&promise))
                }
                Form::Symbol(n) if n == "promise/value" => {
                    if fs.len() != 2 {
                        return Err("promise/value expects one promise".into());
                    }
                    let promise = promise_value(&eval(&fs[1], env)?, n)?;
                    promise_value_result(&promise)
                }
                Form::Symbol(n) if n == "promise/cancel" => {
                    if fs.len() != 2 {
                        return Err("promise/cancel expects a promise".into());
                    }
                    let promise = promise_value(&eval(&fs[1], env)?, n)?;
                    if !promise.cancel() {
                        return Err("promise is already settled".into());
                    }
                    Ok(Value::Promise(promise))
                }
                Form::Symbol(n)
                    if ["promise/then", "promise/catch", "promise/finally"]
                        .contains(&n.as_str()) =>
                {
                    if fs.len() != 3 {
                        return Err(format!("{n} expects a promise and function"));
                    }
                    let source = promise_value(&eval(&fs[1], env)?, n)?;
                    let function = match eval(&fs[2], env)? {
                        Value::Function(function) => function,
                        _ => return Err(format!("{n} expects a function")),
                    };
                    Ok(Value::Promise(promise_chain(source, n, function)))
                }
                Form::Symbol(n) if n.starts_with("ns:") => eval_namespace_operation(n, fs, env),
                Form::Symbol(n) if n == "." => {
                    if fs.len() != 3 {
                        return Err("dot expects a receiver and method".into());
                    }
                    let receiver = eval(&fs[1], env)?;
                    dot_call(receiver, &fs[2], env)
                }
                Form::Symbol(n)
                    if [
                        "socket/connect",
                        "socket/listen",
                        "socket/endpoint",
                        "socket/events",
                        "socket/next",
                        "socket/send",
                        "socket/close",
                        "socket/receive-stream",
                        "socket/duplex",
                    ]
                    .contains(&n.as_str()) =>
                {
                    socket_operation(n, &fs[1..], env)
                }
                Form::Symbol(n)
                    if [
                        "os/platform",
                        "os/arch",
                        "os/cwd",
                        "os/env",
                        "os/getenv",
                        "os/spawn",
                        "os/process?",
                        "os/process-alive?",
                        "os/process-write",
                        "os/process-close-input",
                        "os/process-stdout",
                        "os/process-stderr",
                        "os/process-wait",
                        "os/process-kill",
                    ]
                    .contains(&n.as_str()) =>
                {
                    os_operation(n, &fs[1..], env)
                }
                Form::Symbol(n)
                    if [
                        "file/resolve",
                        "file/join",
                        "file/read",
                        "file/write",
                        "file/exists?",
                        "file/stat",
                        "file/entries",
                        "file/list",
                        "file/walk",
                        "file/mkdir",
                        "file/delete",
                        "file/copy",
                        "file/move",
                        "file/temp-file",
                        "file/temp-directory",
                    ]
                    .contains(&n.as_str()) =>
                {
                    file_operation(n, &fs[1..], env)
                }
                Form::Symbol(n) if n == "Printer/capture" => {
                    if fs.len() != 2 {
                        return Err("Printer/capture expects one callable".into());
                    }
                    let callable = eval(&fs[1], env)?;
                    PRINTER_CAPTURES.with(|captures| captures.borrow_mut().push(String::new()));
                    let result = call_value(callable, Vec::new());
                    let output = PRINTER_CAPTURES.with(|captures| {
                        captures
                            .borrow_mut()
                            .pop()
                            .expect("Printer/capture stack must contain the active capture")
                    });
                    result.map(|_| Value::String(output))
                }
                Form::Symbol(n)
                    if [
                        "str/pad-left",
                        "str/pad-right",
                        "str/starts-with?",
                        "str/ends-with?",
                        "str/split",
                        "str/join",
                        "str/index-of",
                        "str/to-fixed",
                        "str/replace",
                        "str/trim-left",
                        "str/trim-right",
                        "str/length",
                        "str/blank?",
                        "str/includes?",
                        "str/char-at",
                        "str/slice",
                        "str/last-index-of",
                        "str/split-lines",
                        "str/repeat",
                        "str/replace-first",
                        "str/capitalize",
                        "str/decapitalize",
                        "str/reverse",
                        "str/encode-utf8",
                        "str/decode-utf8",
                    ]
                    .contains(&n.as_str()) =>
                {
                    let values = fs[1..]
                        .iter()
                        .map(|form| eval(form, env))
                        .collect::<Result<Vec<_>, _>>()?;
                    string_operation(n, values)
                }
                Form::Symbol(n) if n == "str/upper" || n == "str/lower" => {
                    if fs.len() != 2 {
                        return Err(format!("{n} expects one string"));
                    }
                    let text = match eval(&fs[1], env)? {
                        Value::String(text) => text,
                        _ => return Err(format!("{n} expects a string")),
                    };
                    match n.as_str() {
                        "str/upper" => Ok(Value::String(text.to_uppercase())),
                        "str/lower" => Ok(Value::String(text.to_lowercase())),
                        _ => unreachable!(),
                    }
                }
                Form::Symbol(n) if n == "bytes/copy" => {
                    if fs.len() != 2 {
                        return Err("bytes/copy expects bytes".into());
                    }
                    byte_copy(&eval(&fs[1], env)?)
                }
                Form::Symbol(n) if n == "bytes/slice" => {
                    if fs.len() != 3 && fs.len() != 4 {
                        return Err("bytes/slice expects bytes, start, and optional end".into());
                    }
                    let value = eval(&fs[1], env)?;
                    let start = eval(&fs[2], env)?;
                    let end = if fs.len() == 4 {
                        eval(&fs[3], env)?
                    } else {
                        byte_count(&value)?
                    };
                    byte_slice(&value, &start, &end)
                }
                Form::Symbol(n) if n == "bytes/count" => {
                    if fs.len() != 2 {
                        return Err("bytes/count expects one argument".into());
                    }
                    byte_count(&eval(&fs[1], env)?)
                }
                Form::Symbol(n) if n == "bytes/get" => {
                    if fs.len() != 3 && fs.len() != 4 {
                        return Err("bytes/get expects an index and optional default".into());
                    }
                    let value = eval(&fs[1], env)?;
                    let index = eval(&fs[2], env)?;
                    let default = if fs.len() == 4 {
                        Some(eval(&fs[3], env)?)
                    } else {
                        None
                    };
                    let index_num = value_index(&index)?;
                    match byte_get(&value, &index, default) {
                        Ok(value) => Ok(value),
                        Err(error) if error.is_empty() => {
                            Err(format!("bytes/get index out of bounds: {index_num}"))
                        }
                        Err(error) => Err(error),
                    }
                }
                Form::Symbol(n) if n == "bytes/set" => {
                    if fs.len() != 4 {
                        return Err("bytes/set expects bytes, index, and value".into());
                    }
                    let value = eval(&fs[1], env)?;
                    let index = eval(&fs[2], env)?;
                    let item = eval(&fs[3], env)?;
                    byte_set(&value, &index, &item)
                }
                Form::Symbol(n) if n == "bytes/u8" || n == "bytes/s8" => {
                    if fs.len() != 2 {
                        return Err(format!("{n} expects one argument"));
                    }
                    let number = match eval(&fs[1], env)? {
                        Value::Number(number) => number,
                        _ => return Err(format!("{n} expects a number")),
                    };
                    if !(-128..=255).contains(&number) {
                        return Err(format!("{n} expects a value in the range -128..255"));
                    }
                    let raw = (number as i8) as u8;
                    Ok(Value::Number(if n == "bytes/u8" {
                        raw as i64
                    } else {
                        raw as i8 as i64
                    }))
                }
                Form::Symbol(n) if n == "iter-finite?" => {
                    if fs.len() != 2 {
                        return Err("iter-finite? expects one argument".into());
                    }
                    Ok(Value::Bool(iterator_is_finite(&eval(&fs[1], env)?)))
                }
                Form::Symbol(n) if n == "iter-materialize" => {
                    if fs.len() != 2 {
                        return Err("iter-materialize expects one argument".into());
                    }
                    Ok(Value::Vector(iterator_to_vec(eval(&fs[1], env)?)?.into()))
                }
                Form::Symbol(n) if n == "iter-close" => {
                    if fs.len() != 2 {
                        return Err("iter-close expects one argument".into());
                    }
                    iterator_close(&eval(&fs[1], env)?)
                }
                Form::Symbol(n)
                    if ["iter-map", "map", "iter-filter", "filter"].contains(&n.as_str()) =>
                {
                    let is_map = n == "iter-map" || n == "map";
                    if n == "map" && fs.len() == 2 {
                        let callable = eval(&fs[1], env)?;
                        let body = Form::List(vec![
                            Form::Symbol("__map-transform".into()),
                            Form::Symbol("__callable".into()),
                            Form::Symbol("value".into()),
                        ]);
                        return Ok(generated_function(
                            vec!["value".into()],
                            vec![body],
                            env.clone(),
                            vec![("__callable", callable)],
                        ));
                    }
                    if n == "filter" && fs.len() == 2 {
                        let predicate = eval(&fs[1], env)?;
                        return Ok(generated_unary_operation(
                            "iter-filter",
                            predicate,
                            env.clone(),
                        ));
                    }
                    if fs.len() < 3 {
                        return Err(format!("{n} expects a function and collection"));
                    }
                    let function = eval(&fs[1], env)?;
                    if is_map && fs.len() > 3 {
                        let sources = fs[2..]
                            .iter()
                            .map(|form| eval(form, env))
                            .collect::<Result<Vec<_>, _>>()?;
                        let primary = sources[0].clone();
                        let zipped = iterator_zip(sources)?;
                        let values = iterator_to_vec(zipped)?;
                        let mut output = Vec::with_capacity(values.len());
                        for value in values {
                            let arguments = iterator_values(value)?;
                            output.push(call_value(function.clone(), arguments)?);
                        }
                        let result = iterator_from_values(output);
                        return if n == "map" {
                            transform_like(&primary, result)
                        } else {
                            Ok(result)
                        };
                    }
                    let raw_collection = if fs.len() == 3 {
                        Some(eval(&fs[2], env)?)
                    } else {
                        None
                    };
                    if fs.len() == 3 {
                        if !is_map {
                            if let Some(value) = raw_collection.clone() {
                                let result = iterator_filter(function.clone(), value)?;
                                return if n == "filter" {
                                    transform_like(raw_collection.as_ref().unwrap(), result)
                                } else {
                                    Ok(result)
                                };
                            }
                        }
                    }
                    let collections = if let Some(value) = raw_collection.clone() {
                        vec![iterator_values(value)?]
                    } else {
                        fs[2..]
                            .iter()
                            .map(|form| eval(form, env).and_then(iterator_values))
                            .collect::<Result<Vec<_>, _>>()?
                    };
                    let mut output = Vec::new();
                    if is_map {
                        let limit = collections.iter().map(Vec::len).min().unwrap_or(0);
                        for index in 0..limit {
                            let args = collections
                                .iter()
                                .map(|values| values[index].clone())
                                .collect();
                            let mapped = call_value(function.clone(), args)?;
                            output.push(mapped);
                        }
                    } else {
                        if collections.len() != 1 {
                            return Err(format!("{n} expects one collection"));
                        }
                        for value in collections.into_iter().next().unwrap() {
                            let mapped = call_value(function.clone(), vec![value.clone()])?;
                            if mapped.truthy() {
                                output.push(value);
                            }
                        }
                    }
                    if n == "map" || n == "filter" {
                        transform_like(
                            raw_collection.as_ref().unwrap(),
                            iterator_from_values(output),
                        )
                    } else {
                        Ok(iterator_from_values(output))
                    }
                }
                Form::Symbol(n) if ["iter-take", "take"].contains(&n.as_str()) => {
                    if n == "take" && fs.len() == 2 {
                        let amount = eval(&fs[1], env)?;
                        return Ok(generated_unary_operation("iter-take", amount, env.clone()));
                    }
                    if fs.len() != 3 {
                        return Err(format!("{n} expects an amount and collection"));
                    }
                    let amount = value_index(&eval(&fs[1], env)?)?;
                    let collection = eval(&fs[2], env)?;
                    let result = iterator_take(collection.clone(), amount)?;
                    if n == "take" {
                        transform_like(&collection, result)
                    } else {
                        Ok(result)
                    }
                }
                Form::Symbol(n) if ["iter-drop", "drop"].contains(&n.as_str()) => {
                    if n == "drop" && fs.len() == 2 {
                        let amount = eval(&fs[1], env)?;
                        return Ok(generated_unary_operation("iter-drop", amount, env.clone()));
                    }
                    if fs.len() != 3 {
                        return Err(format!("{n} expects an amount and collection"));
                    }
                    let amount = value_index(&eval(&fs[1], env)?)?;
                    let collection = eval(&fs[2], env)?;
                    let result = iterator_drop(collection.clone(), amount)?;
                    if n == "drop" {
                        transform_like(&collection, result)
                    } else {
                        Ok(result)
                    }
                }
                Form::Symbol(n)
                    if [
                        "iter-take-while",
                        "take-while",
                        "iter-drop-while",
                        "drop-while",
                    ]
                    .contains(&n.as_str()) =>
                {
                    if !n.starts_with("iter-") && fs.len() == 2 {
                        let predicate = eval(&fs[1], env)?;
                        let operation = if n == "take-while" {
                            "iter-take-while"
                        } else {
                            "iter-drop-while"
                        };
                        return Ok(generated_unary_operation(operation, predicate, env.clone()));
                    }
                    if fs.len() != 3 {
                        return Err(format!("{n} expects a predicate and collection"));
                    }
                    let predicate = eval(&fs[1], env)?;
                    let value = eval(&fs[2], env)?;
                    let result = if n.contains("take-while") {
                        iterator_take_while(predicate, value.clone())?
                    } else {
                        iterator_drop_while(predicate, value.clone())?
                    };
                    if n.starts_with("iter-") {
                        Ok(result)
                    } else {
                        transform_like(&value, result)
                    }
                }
                Form::Symbol(n) if ["iter-mapcat", "mapcat"].contains(&n.as_str()) => {
                    if n == "mapcat" && fs.len() == 2 {
                        let function = eval(&fs[1], env)?;
                        return Ok(generated_unary_operation(
                            "iter-mapcat",
                            function,
                            env.clone(),
                        ));
                    }
                    if fs.len() != 3 {
                        return Err(format!("{n} expects a function and collection"));
                    }
                    let function = eval(&fs[1], env)?;
                    let value = eval(&fs[2], env)?;
                    let result = iterator_mapcat(function, value.clone())?;
                    if n == "mapcat" {
                        transform_like(&value, result)
                    } else {
                        Ok(result)
                    }
                }
                Form::Symbol(n) if ["iter-keep", "keep"].contains(&n.as_str()) => {
                    if n == "keep" && fs.len() == 2 {
                        let function = eval(&fs[1], env)?;
                        return Ok(generated_unary_operation(
                            "iter-keep",
                            function,
                            env.clone(),
                        ));
                    }
                    if fs.len() != 3 {
                        return Err(format!("{n} expects a function and collection"));
                    }
                    let function = eval(&fs[1], env)?;
                    let value = eval(&fs[2], env)?;
                    let result = iterator_keep(function, value.clone())?;
                    if n == "keep" {
                        transform_like(&value, result)
                    } else {
                        Ok(result)
                    }
                }
                Form::Symbol(n)
                    if [
                        "iter-partition-all",
                        "partition-all",
                        "iter-partition",
                        "partition",
                    ]
                    .contains(&n.as_str()) =>
                {
                    if !n.starts_with("iter-") && fs.len() == 2 {
                        let amount = eval(&fs[1], env)?;
                        let operation = if n == "partition-all" {
                            "iter-partition-all"
                        } else {
                            "iter-partition"
                        };
                        return Ok(generated_unary_operation(operation, amount, env.clone()));
                    }
                    if fs.len() != 3 {
                        return Err(format!("{n} expects an amount and collection"));
                    }
                    let amount = value_index(&eval(&fs[1], env)?)?;
                    let collection = eval(&fs[2], env)?;
                    let result = iterator_partition(collection.clone(), amount, n.contains("all"))?;
                    if n.starts_with("iter-") {
                        Ok(result)
                    } else {
                        transform_like(&collection, result)
                    }
                }
                Form::Symbol(n) if ["iter-interpose", "interpose"].contains(&n.as_str()) => {
                    if n == "interpose" && fs.len() == 2 {
                        let separator = eval(&fs[1], env)?;
                        return Ok(generated_unary_operation(
                            "iter-interpose",
                            separator,
                            env.clone(),
                        ));
                    }
                    if fs.len() != 3 {
                        return Err(format!("{n} expects a separator and collection"));
                    }
                    let separator = eval(&fs[1], env)?;
                    let collection = eval(&fs[2], env)?;
                    let result = iterator_interpose(separator, collection.clone())?;
                    if n == "interpose" {
                        transform_like(&collection, result)
                    } else {
                        Ok(result)
                    }
                }
                Form::Symbol(n) if ["iter-interleave", "interleave"].contains(&n.as_str()) => {
                    if fs.len() < 2 {
                        return Err(format!("{n} expects collections"));
                    }
                    let collections = fs[1..]
                        .iter()
                        .map(|form| eval(form, env))
                        .collect::<Result<Vec<_>, _>>()?;
                    let primary = collections[0].clone();
                    let result = iterator_interleave(collections)?;
                    if n == "interleave" {
                        transform_like(&primary, result)
                    } else {
                        Ok(result)
                    }
                }
                Form::Symbol(n)
                    if ["iter-partition-pair", "partition-pair"].contains(&n.as_str()) =>
                {
                    if fs.len() != 2 {
                        return Err(format!("{n} expects one collection"));
                    }
                    let collection = eval(&fs[1], env)?;
                    let result = iterator_partition(collection.clone(), 2, false)?;
                    if n == "partition-pair" {
                        transform_like(&collection, result)
                    } else {
                        Ok(result)
                    }
                }
                Form::Symbol(n) if ["iter-zip", "zip"].contains(&n.as_str()) => {
                    if fs.len() < 3 {
                        return Err(format!("{n} expects collections"));
                    }
                    let collections = fs[1..]
                        .iter()
                        .map(|form| eval(form, env))
                        .collect::<Result<Vec<_>, _>>()?;
                    let primary = collections[0].clone();
                    let result = iterator_zip(collections)?;
                    if n == "zip" {
                        transform_like(&primary, result)
                    } else {
                        Ok(result)
                    }
                }
                Form::Symbol(n) if n == "iter-cycle" || n == "cycle" => {
                    if fs.len() != 2 {
                        return Err(format!("{n} expects one collection"));
                    }
                    let result = iterator_cycle(eval(&fs[1], env)?)?;
                    if n == "cycle" {
                        iterator_seq(result)
                    } else {
                        Ok(result)
                    }
                }
                Form::Symbol(n) if n == "iter-concat" || n == "concat" => {
                    let collections = fs[1..]
                        .iter()
                        .map(|form| eval(form, env))
                        .collect::<Result<Vec<_>, _>>()?;
                    let result = iterator_concat(collections)?;
                    if n == "concat" {
                        iterator_seq(result)
                    } else {
                        Ok(result)
                    }
                }
                Form::Symbol(n) if n == "iter-range" => {
                    if fs.len() != 2 && fs.len() != 3 {
                        return Err("iter-range expects an end or start and end".into());
                    }
                    let nums = fs[1..]
                        .iter()
                        .map(|form| {
                            let value = eval(form, env)?;
                            numeric::to_i64_exact(&value).map_err(|_| {
                                "iter-range bounds must fit signed 64-bit integers".into()
                            })
                        })
                        .collect::<Result<Vec<_>, String>>()?;
                    let (start, end) = match nums.as_slice() {
                        [end] => (0, *end),
                        [start, end] => (*start, *end),
                        _ => unreachable!(),
                    };
                    Ok(iterator_from_values(
                        (start..end).map(Value::Number).collect(),
                    ))
                }
                Form::Symbol(n) if n == "range" => {
                    if fs.len() < 1 || fs.len() > 3 {
                        return Err("range expects zero, one, or two bounds".into());
                    }
                    let nums = fs[1..]
                        .iter()
                        .map(|form| {
                            let value = eval(form, env)?;
                            numeric::to_i64_exact(&value)
                                .map_err(|_| "range bounds must fit signed 64-bit integers".into())
                        })
                        .collect::<Result<Vec<_>, String>>()?;
                    let (start, end) = match nums.as_slice() {
                        [] => (0, 0),
                        [end] => (0, *end),
                        [start, end] => (*start, *end),
                        _ => unreachable!(),
                    };
                    iterator_seq(iterator_from_values(
                        (start..end).map(Value::Number).collect(),
                    ))
                }
                Form::Symbol(n) if n == "repeat" => {
                    if fs.len() != 2 && fs.len() != 3 {
                        return Err("repeat expects a value or amount and value".into());
                    }
                    let (amount, form) = if fs.len() == 2 {
                        (None, &fs[1])
                    } else {
                        (Some(value_index(&eval(&fs[1], env)?)?), &fs[2])
                    };
                    let value = eval(form, env)?;
                    if amount.is_none() {
                        return iterator_seq(iterator_constant(value));
                    }
                    let count = amount.unwrap();
                    iterator_seq(iterator_from_values(
                        (0..count).map(|_| value.clone()).collect(),
                    ))
                }
                Form::Symbol(n) if n == "repeatedly" => {
                    if fs.len() != 2 && fs.len() != 3 {
                        return Err("repeatedly expects a function or amount and function".into());
                    }
                    let (amount, form) = if fs.len() == 2 {
                        (None, &fs[1])
                    } else {
                        (Some(value_index(&eval(&fs[1], env)?)?), &fs[2])
                    };
                    let function = eval(form, env)?;
                    let generated = iterator_repeated(function);
                    let result = if let Some(amount) = amount {
                        iterator_take(generated, amount)?
                    } else {
                        generated
                    };
                    iterator_seq(result)
                }
                Form::Symbol(n) if n == "iterate" => {
                    if fs.len() != 3 {
                        return Err("iterate expects a function and seed".into());
                    }
                    let function = eval(&fs[1], env)?;
                    iterator_seq(iterator_iterate(function, eval(&fs[2], env)?))
                }
                Form::Symbol(n) if n == "iter-constantly" => {
                    if fs.len() != 2 {
                        return Err("iter-constantly expects a value".into());
                    }
                    Ok(iterator_constant(eval(&fs[1], env)?))
                }
                Form::Symbol(n) if n == "iter-repeatedly" => {
                    if fs.len() != 2 {
                        return Err("iter-repeatedly expects a function".into());
                    }
                    Ok(iterator_repeated(eval(&fs[1], env)?))
                }
                Form::Symbol(n) if n == "iter-iterate" => {
                    if fs.len() != 3 {
                        return Err("iter-iterate expects a function and seed".into());
                    }
                    let function = eval(&fs[1], env)?;
                    Ok(iterator_iterate(function, eval(&fs[2], env)?))
                }
                Form::Symbol(n)
                    if [
                        "bit-and",
                        "bit-or",
                        "bit-xor",
                        "bit-not",
                        "bit-shift-left",
                        "bit-shift-right",
                    ]
                    .contains(&n.as_str()) =>
                {
                    bit_operation(n, &fs[1..], env)
                }
                Form::Symbol(n) if n == "__map-transform" => {
                    if fs.len() != 3 {
                        return Err("map transform expects a function and source".into());
                    }
                    let function = eval(&fs[1], env)?;
                    let source = eval(&fs[2], env)?;
                    let result = iterator_map(function, source.clone())?;
                    transform_like(&source, result)
                }
                Form::Symbol(n) if n == "__iterator-transform" => {
                    if fs.len() != 4 {
                        return Err(
                            "iterator transform expects an operation, parameter, and source".into(),
                        );
                    }
                    let operation = match eval(&fs[1], env)? {
                        Value::String(operation) => operation,
                        _ => return Err("iterator transform operation must be a string".into()),
                    };
                    let parameter = eval(&fs[2], env)?;
                    let source = eval(&fs[3], env)?;
                    match operation.as_str() {
                        "iter-filter" => {
                            let result = iterator_filter(parameter, source.clone())?;
                            transform_like(&source, result)
                        }
                        "iter-take" => {
                            let result = iterator_take(source.clone(), value_index(&parameter)?)?;
                            transform_like(&source, result)
                        }
                        "iter-drop" => {
                            let result = iterator_drop(source.clone(), value_index(&parameter)?)?;
                            transform_like(&source, result)
                        }
                        "iter-take-while" => {
                            let result = iterator_take_while(parameter, source.clone())?;
                            transform_like(&source, result)
                        }
                        "iter-drop-while" => {
                            let result = iterator_drop_while(parameter, source.clone())?;
                            transform_like(&source, result)
                        }
                        "iter-mapcat" => {
                            let result = iterator_mapcat(parameter, source.clone())?;
                            transform_like(&source, result)
                        }
                        "iter-keep" => {
                            let result = iterator_keep(parameter, source.clone())?;
                            transform_like(&source, result)
                        }
                        "iter-partition" | "iter-partition-all" => {
                            let result = iterator_partition(
                                source.clone(),
                                value_index(&parameter)?,
                                operation == "iter-partition-all",
                            )?;
                            transform_like(&source, result)
                        }
                        "iter-interpose" => {
                            let result = iterator_interpose(parameter, source.clone())?;
                            transform_like(&source, result)
                        }
                        "every?" | "any?" => {
                            while let Some(value) = iterator_try_next(&source)? {
                                let matched = call_value(parameter.clone(), vec![value])?.truthy();
                                if operation == "every?" && !matched {
                                    return Ok(Value::Bool(false));
                                }
                                if operation == "any?" && matched {
                                    return Ok(Value::Bool(true));
                                }
                            }
                            Ok(Value::Bool(operation == "every?"))
                        }
                        _ => Err(format!("unknown iterator transform: {operation}")),
                    }
                }
                Form::Symbol(n)
                    if ["zero?", "pos?", "neg?", "even?", "odd?"].contains(&n.as_str()) =>
                {
                    if fs.len() != 2 {
                        return Err(format!("{n} expects one number"));
                    }
                    let value = match eval(&fs[1], env)? {
                        Value::Number(value) => value,
                        _ => return Err(format!("{n} expects a number")),
                    };
                    let result = match n.as_str() {
                        "zero?" => value == 0,
                        "pos?" => value > 0,
                        "neg?" => value < 0,
                        "even?" => value % 2 == 0,
                        "odd?" => value % 2 != 0,
                        _ => false,
                    };
                    Ok(Value::Bool(result))
                }
                Form::Symbol(n) if ["nil?", "true?", "false?"].contains(&n.as_str()) => {
                    if fs.len() != 2 {
                        return Err(format!("{n} expects one value"));
                    }
                    let value = eval(&fs[1], env)?;
                    let result = match n.as_str() {
                        "nil?" => matches!(value, Value::Nil),
                        "true?" => matches!(value, Value::Bool(true)),
                        "false?" => matches!(value, Value::Bool(false)),
                        _ => false,
                    };
                    Ok(Value::Bool(result))
                }
                Form::Symbol(n)
                    if ["every?", "any?", "iter-every?", "iter-any?"].contains(&n.as_str()) =>
                {
                    if !n.starts_with("iter-") && fs.len() == 2 {
                        let predicate = eval(&fs[1], env)?;
                        let operation = if n == "every?" { "every?" } else { "any?" };
                        return Ok(generated_unary_operation(operation, predicate, env.clone()));
                    }
                    if fs.len() != 3 {
                        return Err(format!("{n} expects a predicate and collection"));
                    }
                    let predicate = eval(&fs[1], env)?;
                    let values = iterator_values(eval(&fs[2], env)?)?;
                    for value in values {
                        let result = call_value(predicate.clone(), vec![value])?;
                        if n.contains("every?") && !result.truthy() {
                            return Ok(Value::Bool(false));
                        }
                        if n.contains("any?") && result.truthy() {
                            return Ok(Value::Bool(true));
                        }
                    }
                    Ok(Value::Bool(n.contains("every?")))
                }
                Form::Symbol(n) if named_predicate_protocol(n).is_some() => {
                    if fs.len() != 2 {
                        return Err(format!("{n} expects one argument"));
                    }
                    let value = eval(&fs[1], env)?;
                    Ok(Value::Bool(named_protocol_satisfies(n, &value)))
                }
                Form::Symbol(n)
                    if ["cons?", "tuple?", "sequential?", "pointer?"].contains(&n.as_str()) =>
                {
                    if fs.len() != 2 {
                        return Err(format!("{n} expects one argument"));
                    }
                    let value = eval(&fs[1], env)?;
                    Ok(Value::Bool(match n.as_str() {
                        "cons?" => matches!(value, Value::Cons(_)),
                        "tuple?" => matches!(value, Value::Tuple(_)),
                        "sequential?" => matches!(
                            value,
                            Value::List(_)
                                | Value::Cons(_)
                                | Value::Queue(_)
                                | Value::Deque(_)
                                | Value::Vector(_)
                                | Value::Tuple(_)
                                | Value::Seq(_)
                        ),
                        _ => unreachable!(),
                    }))
                }
                Form::Symbol(n) if n == "to-mutable" || n == "to-persistent" => {
                    if fs.len() != 2 {
                        return Err(format!("{n} expects one argument"));
                    }
                    let value = eval(&fs[1], env)?;
                    if n == "to-mutable" {
                        collection_to_mutable(&value)
                    } else {
                        collection_to_persistent(&value)
                    }
                }
                Form::Symbol(n) if n == "recur" => {
                    if fs.len() < 2 {
                        return Err("recur expects values".into());
                    }
                    Ok(Value::Recur(
                        fs[1..]
                            .iter()
                            .map(|form| eval(form, env))
                            .collect::<Result<Vec<_>, _>>()?,
                    ))
                }
                Form::Symbol(n) if n == "binding" => {
                    if fs.len() < 3 {
                        return Err("binding expects bindings and a body".into());
                    }
                    let pairs = match &fs[1] {
                        Form::List(values) | Form::Vector(values) => values,
                        _ => return Err("binding expects a binding list or vector".into()),
                    };
                    if pairs.len() % 2 != 0 {
                        return Err("binding bindings require name/value pairs".into());
                    }
                    let mut pending = Vec::new();
                    for pair in pairs.chunks(2) {
                        let name = match &pair[0] {
                            Form::Symbol(name) => name,
                            _ => return Err("binding name must be a symbol".into()),
                        };
                        let var = binding_var(env, name)
                            .ok_or_else(|| format!("binding expects a Var: {name}"))?;
                        if !var.is_dynamic() {
                            return Err(format!("binding expects a dynamic Var: {name}"));
                        }
                        let value = eval(&pair[1], env)?;
                        pending.push((var, value));
                    }
                    for (var, value) in &pending {
                        var.bind(value.clone());
                    }
                    let bound = pending.into_iter().map(|(var, _)| var).collect::<Vec<_>>();
                    let mut result = Ok(Value::Nil);
                    for form in &fs[2..] {
                        result = eval(form, env);
                        if result.is_err() {
                            break;
                        }
                    }
                    for var in bound.into_iter().rev() {
                        if let Err(error) = var.unbind() {
                            if result.is_ok() {
                                result = Err(error);
                            }
                        }
                    }
                    result
                }
                Form::Symbol(n) if n == "loop" => {
                    if fs.len() != 3 {
                        return Err("loop expects bindings and a body".into());
                    }
                    let bindings = match &fs[1] {
                        Form::List(values) | Form::Vector(values) => values,
                        _ => return Err("loop expects a binding list or vector".into()),
                    };
                    if bindings.len() % 2 != 0 {
                        return Err("loop bindings require name/value pairs".into());
                    }
                    let mut previous = Vec::new();
                    let mut patterns = Vec::new();
                    let mut pattern_names = Vec::new();
                    for pair in bindings.chunks(2) {
                        let value = eval(&pair[1], env)?;
                        let before = env.clone();
                        let mut names = Vec::new();
                        bind_pattern(&pair[0], value, env, &mut names, None)
                            .map_err(|error| format!("loop destructuring failed: {error}"))?;
                        for name in &names {
                            previous.push((name.clone(), before.get(name).cloned()));
                        }
                        patterns.push(pair[0].clone());
                        pattern_names.push(names);
                    }
                    let result = loop {
                        match eval(&fs[2], env)? {
                            Value::Recur(values) => {
                                if values.len() != patterns.len() {
                                    break Err("loop recur arity mismatch".into());
                                }
                                for names in &pattern_names {
                                    for name in names {
                                        env.remove(name);
                                    }
                                }
                                pattern_names.clear();
                                for (pattern, value) in patterns.iter().zip(values) {
                                    let mut names = Vec::new();
                                    bind_pattern(pattern, value, env, &mut names, None)?;
                                    pattern_names.push(names);
                                }
                            }
                            result => break Ok(result),
                        }
                    };
                    for (name, old) in previous.into_iter().rev() {
                        if let Some(old) = old {
                            env.insert(name, old);
                        } else {
                            env.remove(&name);
                        }
                    }
                    result
                }
                Form::Symbol(n) if n == "if" => {
                    if fs.len() != 3 && fs.len() != 4 {
                        return Err("if expects 2 or 3 arguments".into());
                    }
                    if eval(&fs[1], env)?.truthy() {
                        eval(&fs[2], env)
                    } else if fs.len() == 4 {
                        eval(&fs[3], env)
                    } else {
                        Ok(Value::Nil)
                    }
                }
                Form::Symbol(n) if n == "and" => {
                    let mut result = Value::Bool(true);
                    for form in &fs[1..] {
                        result = eval(form, env)?;
                        if !result.truthy() {
                            return Ok(result);
                        }
                    }
                    Ok(result)
                }
                Form::Symbol(n) if n == "or" => {
                    let mut result = Value::Nil;
                    for form in &fs[1..] {
                        result = eval(form, env)?;
                        if result.truthy() {
                            return Ok(result);
                        }
                    }
                    Ok(result)
                }
                Form::Symbol(n) if n == "cond" => {
                    if fs.len() % 2 == 0 {
                        return Err("cond expects test/expression pairs".into());
                    }
                    let mut clauses = fs[1..].chunks_exact(2);
                    for clause in &mut clauses {
                        if eval(&clause[0], env)?.truthy() {
                            return eval(&clause[1], env);
                        }
                    }
                    Ok(Value::Nil)
                }
                Form::Symbol(n) if n == "let" => {
                    if fs.len() < 3 {
                        return Err("let expects bindings and a body".into());
                    }
                    let bindings = match &fs[1] {
                        Form::List(values) | Form::Vector(values) => values,
                        _ => return Err("let expects a binding list or vector".into()),
                    };
                    if bindings.len() % 2 != 0 {
                        return Err("let bindings require name/value pairs".into());
                    }
                    let mut previous = Vec::new();
                    for pair in bindings.chunks(2) {
                        let value = eval(&pair[1], env)?;
                        let before = env.clone();
                        let mut names = Vec::new();
                        bind_pattern(&pair[0], value, env, &mut names, None)
                            .map_err(|error| format!("let destructuring failed: {error}"))?;
                        for name in names {
                            previous.push((name.clone(), before.get(&name).cloned()));
                        }
                    }
                    let mut result = Ok(Value::Nil);
                    for body in &fs[2..] {
                        result = eval(body, env);
                        if result.is_err() {
                            break;
                        }
                    }
                    for (name, old) in previous.into_iter().rev() {
                        if let Some(old) = old {
                            env.insert(name, old);
                        } else {
                            env.remove(&name);
                        }
                    }
                    result
                }
                _ => {
                    if let Form::Symbol(name) = &fs[0] {
                        if let Some(expanded) = macroexpand_call(name, fs, env)? {
                            return eval(&expanded, env);
                        }
                    }
                    let function = eval(&fs[0], env)?;
                    let arguments = fs[1..]
                        .iter()
                        .map(|form| eval(form, env))
                        .collect::<Result<Vec<_>, _>>()?;
                    call_value(function, arguments)
                }
            }
        }
    }
}

pub fn eval_traced(form: &Form, env: &mut HashMap<String, Value>) -> Result<Value, String> {
    let _guard = StackTraceGuard::enable();
    eval(form, env).map_err(append_trace)
}

pub fn eval_text(source: &str, env: &mut HashMap<String, Value>) -> Result<String, String> {
    Ok(eval_value_text(source, env)?.display())
}

pub fn eval_text_traced(source: &str, env: &mut HashMap<String, Value>) -> Result<String, String> {
    let _guard = StackTraceGuard::enable();
    eval_text(source, env).map_err(append_trace)
}

pub fn eval_value_text_traced(
    source: &str,
    env: &mut HashMap<String, Value>,
) -> Result<Value, String> {
    let _guard = StackTraceGuard::enable();
    eval_value_text(source, env).map_err(append_trace)
}

pub fn eval_value_text(source: &str, env: &mut HashMap<String, Value>) -> Result<Value, String> {
    let forms = parse_forms(source)?;
    let mut result = Value::Nil;
    for form in forms {
        result = eval(&form, env)?;
        if matches!(result, Value::Recur(_)) {
            return Err("recur must be inside loop".into());
        }
    }
    Ok(result)
}
