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
    #[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
    if direct_native_execution() {
        let location = current_exception_site().map_or_else(String::new, |site| {
            format!(
                " at {}:{}:{}",
                site.namespace.as_deref().unwrap_or("<source>"),
                site.line,
                site.column
            )
        });
        return Err(format!(
            "direct-native cannot enter the tree evaluator{location}"
        ));
    }
    match form {
        Form::Number(v) => Ok(Value::Number(*v)),
        Form::String(v) => Ok(Value::String(v.clone())),
        Form::Keyword(v) => Ok(Value::Keyword(v.clone().into())),
        Form::Nil => Ok(Value::Nil),
        Form::Bool(value) => Ok(Value::Bool(*value)),
        Form::Character(value) => Ok(Value::Character(*value)),
        Form::Float(value) => Ok(Value::Float(crate::numeric::finite_float(*value)?)),
        Form::BigInteger(value) => Ok(crate::numeric::compact_integer(value.clone())),
        Form::Regex(value) => Ok(Value::Regex(value.clone())),
        Form::Tagged(tag, value) if tag == "ptr" => pointer_from_descriptor(literal_value(value)?),
        Form::Tagged(tag, value) => Ok(Value::Tagged(Box::new(PTaggedLiteral::new(
            Symbol::parse(tag),
            literal_value(value)?,
        )))),
        Form::Metadata(metadata, value) => {
            if let Some((line, column)) = exception_location_from_metadata(metadata) {
                with_exception_site(
                    exception_site_at(line, column).expect("exception site always exists"),
                    || eval(value, env),
                )
            } else {
                eval(value, env)
            }
        }
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
                    ensure_foundation_namespace_for_symbol(&registry, env, n)?;
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
            let operator = &fs[0];
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
                                [_, Form::Symbol(name), body, ..]
                                    if name != "Exception"
                                        && name != "Throwable"
                                        && !matches!(body, Form::Symbol(_)) =>
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
                    let (name, name_metadata) = binding_symbol(&fs[1], &format!("{n} name"))?;
                    if name.contains('/') {
                        return Err(format!("{n} name must be an unqualified symbol"));
                    }
                    let fields = match &fs[2] {
                        Form::Vector(fields) => fields
                            .iter()
                            .map(|field| match field {
                                Form::Symbol(field) if !field.contains('/') => {
                                    Ok(NamedField::legacy(field))
                                }
                                Form::Vector(_) => NamedField::from_form(field, n),
                                _ => Err(format!(
                                    "{n} fields must be symbols or [name schema] vectors"
                                )),
                            })
                            .collect::<Result<Vec<_>, String>>()?,
                        _ => return Err(format!("{n} expects a field vector")),
                    };
                    let kind = n.clone();
                    with_declaration_transaction(env, |env| {
                        publish_named_value(&kind, &name, fields, env, name_metadata.clone())?;
                        let mut index = 3;
                        while index < fs.len() {
                            let Form::Symbol(protocol) = &fs[index] else {
                                return Err(format!(
                                    "{kind} protocol clause expects a protocol symbol"
                                ));
                            };
                            index += 1;
                            let start = index;
                            while index < fs.len() && matches!(&fs[index], Form::List(_)) {
                                index += 1;
                            }
                            if start == index {
                                return Err(format!(
                                    "{kind} protocol clause requires method implementations"
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
                    })
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
                    publish_guest_protocol(&name, methods, Vec::new(), env)
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
                        Value::Var(var) => match var.deref_value() {
                            Value::Protocol(protocol) => protocol,
                            _ => return Err("extend-type expects a protocol".into()),
                        },
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
                Form::Symbol(n)
                    if resolve_macro(n).is_none()
                        && binding_value(env, n)
                            .is_some_and(|value| matches!(value, Value::Function(_))) =>
                {
                    let function =
                        binding_value(env, n).expect("namespace function binding was checked");
                    let arguments = fs[1..]
                        .iter()
                        .map(|form| eval(form, env))
                        .collect::<Result<Vec<_>, _>>()?;
                    call_value(function, arguments)
                }
                Form::Symbol(n) if n == "." => {
                    if fs.len() != 3 {
                        return Err("dot expects a receiver and method".into());
                    }
                    let receiver = eval(&fs[1], env)?;
                    dot_call(receiver, &fs[2], env)
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
