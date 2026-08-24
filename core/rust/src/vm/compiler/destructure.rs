//! Destructuring lowering into the compiler's existing symbol-only forms.

use crate::kernel::Form;

pub(super) fn expand(form: &Form, next: &mut u64) -> Result<Option<Form>, String> {
    let Form::List(items) = form else {
        return Ok(None);
    };
    let Some(Form::Symbol(operator)) = items.first() else {
        return Ok(None);
    };
    match operator.as_str() {
        "let" => expand_let(items, next),
        "loop" => expand_loop(items, next),
        "fn" => expand_fn(items, next),
        "defn" | "defn-" | "defmacro" => expand_definition(items, next),
        "binding" => expand_binding(items, next),
        "letfn" => expand_letfn(items, next),
        _ => Ok(None),
    }
}

fn expand_letfn(items: &[Form], next: &mut u64) -> Result<Option<Form>, String> {
    let Some(Form::Vector(definitions)) = items.get(1) else {
        return Ok(None);
    };
    let mut cells = Vec::new();
    let mut names = Vec::new();
    let mut parsed = Vec::new();
    for definition in definitions {
        let Form::List(parts) = definition else {
            return Err("letfn definitions must be lists".into());
        };
        let Some(Form::Symbol(name)) = parts.first() else {
            return Err("letfn definition requires a name".into());
        };
        if parts.len() < 3 {
            return Err("letfn definition requires parameters and a body".into());
        }
        let cell = temporary(next);
        cells.push(Form::Symbol(cell.clone()));
        cells.push(call("std.native.Base/atom", vec![Form::Nil]));
        names.push((name.clone(), cell));
        parsed.push(parts.clone());
    }
    let mut initialization = Vec::new();
    for parts in parsed {
        let Form::Symbol(name) = &parts[0] else {
            unreachable!()
        };
        let cell = names
            .iter()
            .find(|(local, _)| local == name)
            .unwrap()
            .1
            .clone();
        let function = Form::List(
            std::iter::once(Form::Symbol("fn".into()))
                .chain(
                    parts[1..]
                        .iter()
                        .map(|form| replace_letfn_refs(form, &names)),
                )
                .collect(),
        );
        initialization.push(call(
            "std.protocol.ireset.IReset/reset",
            vec![Form::Symbol(cell), function],
        ));
    }
    let aliases = names
        .iter()
        .flat_map(|(name, cell)| {
            [
                Form::Symbol(name.clone()),
                call(
                    "std.protocol.ideref.IDeref/deref",
                    vec![Form::Symbol(cell.clone())],
                ),
            ]
        })
        .collect::<Vec<_>>();
    initialization.push(list(
        "let",
        std::iter::once(Form::Vector(aliases))
            .chain(items[2..].iter().cloned())
            .collect(),
    ));
    Ok(Some(list(
        "let",
        vec![Form::Vector(cells), list("do", initialization)],
    )))
}

fn replace_letfn_refs(form: &Form, names: &[(String, String)]) -> Form {
    match form {
        Form::Symbol(symbol) => names
            .iter()
            .find(|(name, _)| name == symbol)
            .map(|(_, cell)| {
                call(
                    "std.protocol.ideref.IDeref/deref",
                    vec![Form::Symbol(cell.clone())],
                )
            })
            .unwrap_or_else(|| form.clone()),
        Form::List(items) if matches!(items.first(), Some(Form::Symbol(name)) if name == "quote" || name == "syntax-quote") => {
            form.clone()
        }
        Form::List(items) => Form::List(
            items
                .iter()
                .map(|item| replace_letfn_refs(item, names))
                .collect(),
        ),
        Form::Vector(items) => Form::Vector(
            items
                .iter()
                .map(|item| replace_letfn_refs(item, names))
                .collect(),
        ),
        Form::Set(items) => Form::Set(
            items
                .iter()
                .map(|item| replace_letfn_refs(item, names))
                .collect(),
        ),
        Form::Map(entries) => Form::Map(
            entries
                .iter()
                .map(|(key, value)| {
                    (
                        replace_letfn_refs(key, names),
                        replace_letfn_refs(value, names),
                    )
                })
                .collect(),
        ),
        Form::Metadata(metadata, value) => Form::Metadata(
            Box::new(replace_letfn_refs(metadata, names)),
            Box::new(replace_letfn_refs(value, names)),
        ),
        Form::Tagged(tag, value) => {
            Form::Tagged(tag.clone(), Box::new(replace_letfn_refs(value, names)))
        }
        _ => form.clone(),
    }
}

fn expand_binding(items: &[Form], next: &mut u64) -> Result<Option<Form>, String> {
    let Some(Form::Vector(bindings)) = items.get(1) else {
        return Ok(None);
    };
    if bindings.len() % 2 != 0 {
        return Err("binding bindings require name/value pairs".into());
    }
    let mut temporaries = Vec::new();
    let mut binds = Vec::new();
    let mut unbinds = Vec::new();
    for pair in bindings.chunks(2) {
        let Form::Symbol(name) = &pair[0] else {
            return Err("binding name must be a symbol".into());
        };
        let temporary = temporary(next);
        temporaries.push(Form::Symbol(temporary.clone()));
        temporaries.push(pair[1].clone());
        binds.push(call(
            "__dynamic-bind",
            vec![Form::Symbol(name.clone()), Form::Symbol(temporary)],
        ));
        unbinds.push(call("__dynamic-unbind", vec![Form::Symbol(name.clone())]));
    }
    unbinds.reverse();
    let body = list(
        "do",
        binds
            .into_iter()
            .chain(items[2..].iter().cloned())
            .collect(),
    );
    let cleanup = list("do", unbinds);
    Ok(Some(list(
        "let",
        vec![
            Form::Vector(temporaries),
            list("try", vec![body, list("finally", vec![cleanup])]),
        ],
    )))
}

fn expand_let(items: &[Form], next: &mut u64) -> Result<Option<Form>, String> {
    let Some(Form::Vector(bindings)) = items.get(1) else {
        return Ok(None);
    };
    if bindings.len() % 2 != 0 || !bindings.chunks(2).any(|pair| pattern(&pair[0])) {
        return Ok(None);
    }
    let mut output = Vec::new();
    for pair in bindings.chunks(2) {
        bind(&pair[0], pair[1].clone(), &mut output, next)?;
    }
    Ok(Some(list(
        "let",
        std::iter::once(Form::Vector(output))
            .chain(items[2..].iter().cloned())
            .collect(),
    )))
}

fn expand_loop(items: &[Form], next: &mut u64) -> Result<Option<Form>, String> {
    let Some(Form::Vector(bindings)) = items.get(1) else {
        return Ok(None);
    };
    if bindings.len() % 2 != 0 || !bindings.chunks(2).any(|pair| pattern(&pair[0])) {
        return Ok(None);
    }
    let mut raw = Vec::new();
    let mut inner = Vec::new();
    for pair in bindings.chunks(2) {
        if matches!(pair[0], Form::Symbol(_)) {
            raw.extend(pair.iter().cloned());
        } else {
            let temporary = temporary(next);
            raw.push(Form::Symbol(temporary.clone()));
            raw.push(pair[1].clone());
            bind(&pair[0], Form::Symbol(temporary), &mut inner, next)?;
        }
    }
    let body = if inner.is_empty() {
        items[2..].to_vec()
    } else {
        vec![list(
            "let",
            std::iter::once(Form::Vector(inner))
                .chain(items[2..].iter().cloned())
                .collect(),
        )]
    };
    Ok(Some(list(
        "loop",
        std::iter::once(Form::Vector(raw)).chain(body).collect(),
    )))
}

fn expand_fn(items: &[Form], next: &mut u64) -> Result<Option<Form>, String> {
    if matches!(items.get(1), Some(Form::Vector(_))) {
        let Some((params, body)) = expand_params(&items[1], &items[2..], next)? else {
            return Ok(None);
        };
        return Ok(Some(list(
            "fn",
            std::iter::once(params).chain(body).collect(),
        )));
    }
    let mut changed = false;
    let mut clauses = Vec::new();
    for clause in &items[1..] {
        let Form::List(parts) = clause else {
            return Ok(None);
        };
        let Some((params, body)) = expand_params(&parts[0], &parts[1..], next)? else {
            clauses.push(clause.clone());
            continue;
        };
        changed = true;
        clauses.push(Form::List(std::iter::once(params).chain(body).collect()));
    }
    Ok(changed.then(|| list("fn", clauses)))
}

fn expand_definition(items: &[Form], next: &mut u64) -> Result<Option<Form>, String> {
    let Some(parameter_at) = items
        .iter()
        .position(|item| matches!(item, Form::Vector(_) | Form::List(_)))
    else {
        return Ok(None);
    };
    if matches!(&items[parameter_at], Form::Vector(_)) {
        let Some((params, body)) =
            expand_params(&items[parameter_at], &items[parameter_at + 1..], next)?
        else {
            return Ok(None);
        };
        let mut output = items[..parameter_at].to_vec();
        output.push(params);
        output.extend(body);
        return Ok(Some(Form::List(output)));
    }
    let mut changed = false;
    let mut output = items[..parameter_at].to_vec();
    for clause in &items[parameter_at..] {
        let Form::List(parts) = clause else {
            return Ok(None);
        };
        let Some((params, body)) = expand_params(&parts[0], &parts[1..], next)? else {
            output.push(clause.clone());
            continue;
        };
        changed = true;
        output.push(Form::List(std::iter::once(params).chain(body).collect()));
    }
    Ok(changed.then_some(Form::List(output)))
}

fn expand_params(
    params: &Form,
    body: &[Form],
    next: &mut u64,
) -> Result<Option<(Form, Vec<Form>)>, String> {
    let Form::Vector(params) = params else {
        return Ok(None);
    };
    if !params.iter().any(pattern) {
        return Ok(None);
    }
    let mut raw = Vec::new();
    let mut bindings = Vec::new();
    let mut rest = false;
    for param in params {
        if matches!(param, Form::Symbol(name) if name == "&") {
            raw.push(param.clone());
            rest = true;
            continue;
        }
        if !pattern(param) {
            raw.push(param.clone());
            rest = false;
            continue;
        }
        let temporary = temporary(next);
        raw.push(Form::Symbol(temporary.clone()));
        bind(param, Form::Symbol(temporary), &mut bindings, next)?;
        rest = false;
    }
    if rest {
        return Err("the & rest marker requires a following parameter".into());
    }
    let wrapped = vec![list(
        "let",
        std::iter::once(Form::Vector(bindings))
            .chain(body.iter().cloned())
            .collect(),
    )];
    Ok(Some((Form::Vector(raw), wrapped)))
}

fn bind(pattern: &Form, value: Form, output: &mut Vec<Form>, next: &mut u64) -> Result<(), String> {
    match pattern {
        Form::Symbol(_) => {
            output.push(pattern.clone());
            output.push(value);
        }
        Form::Vector(items) => bind_vector(items, value, output, next)?,
        Form::Map(items) => bind_map(items, value, output, next)?,
        _ => return Err("destructuring pattern must be a symbol, vector, or map".into()),
    }
    Ok(())
}

fn bind_vector(
    items: &[Form],
    value: Form,
    output: &mut Vec<Form>,
    next: &mut u64,
) -> Result<(), String> {
    let source = ensure_symbol(value, output, next);
    let mut index = 0i64;
    let mut cursor = 0usize;
    while cursor < items.len() {
        match &items[cursor] {
            Form::Symbol(marker) if marker == "&" => {
                let rest = items.get(cursor + 1).ok_or("vector & requires a binding")?;
                bind(
                    rest,
                    call(
                        "std.native.Iter/iter-drop",
                        vec![Form::Number(index), source.clone()],
                    ),
                    output,
                    next,
                )?;
                cursor += 2;
            }
            Form::Keyword(marker) if marker == "as" => {
                let alias = items
                    .get(cursor + 1)
                    .ok_or("vector :as requires a binding")?;
                bind(alias, source.clone(), output, next)?;
                cursor += 2;
            }
            item => {
                bind(
                    item,
                    lookup_form(&source, Form::Number(index), None),
                    output,
                    next,
                )?;
                index += 1;
                cursor += 1;
            }
        }
    }
    Ok(())
}

fn bind_map(
    items: &[(Form, Form)],
    value: Form,
    output: &mut Vec<Form>,
    next: &mut u64,
) -> Result<(), String> {
    let source = ensure_symbol(value, output, next);
    let defaults = items.iter().find_map(|(key, value)| {
        matches!(key, Form::Keyword(name) if name == "or").then_some(value)
    });
    for (binding, key) in items {
        match binding {
            Form::Keyword(name) if name == "as" => bind(key, source.clone(), output, next)?,
            Form::Keyword(name) if matches!(name.as_str(), "keys" | "strs" | "syms") => {
                let Form::Vector(names) = key else {
                    return Err(format!(":{name} destructuring expects a vector"));
                };
                for local in names {
                    let Form::Symbol(symbol) = local else {
                        return Err(format!(":{name} destructuring expects symbols"));
                    };
                    let lookup = match name.as_str() {
                        "keys" => Form::Keyword(symbol.clone()),
                        "strs" => Form::String(symbol.clone()),
                        "syms" => list("quote", vec![Form::Symbol(symbol.clone())]),
                        _ => unreachable!(),
                    };
                    bind(
                        local,
                        lookup_form(&source, lookup, default_for(defaults, symbol)),
                        output,
                        next,
                    )?;
                }
            }
            Form::Keyword(name) if name == "or" => {}
            _ => bind(
                binding,
                lookup_form(&source, key.clone(), None),
                output,
                next,
            )?,
        }
    }
    Ok(())
}

fn default_for(defaults: Option<&Form>, name: &str) -> Option<Form> {
    let Form::Map(entries) = defaults? else {
        return None;
    };
    entries.iter().find_map(|(key, value)| {
        matches!(key, Form::Symbol(symbol) if symbol == name).then(|| value.clone())
    })
}

fn lookup_form(source: &Form, key: Form, default: Option<Form>) -> Form {
    let mut arguments = vec![source.clone(), key];
    if let Some(default) = default {
        arguments.push(default);
    }
    call("std.protocol.ilookup.ILookup/lookup", arguments)
}

fn ensure_symbol(value: Form, output: &mut Vec<Form>, next: &mut u64) -> Form {
    if matches!(value, Form::Symbol(_)) {
        return value;
    }
    let name = temporary(next);
    output.push(Form::Symbol(name.clone()));
    output.push(value);
    Form::Symbol(name)
}

fn pattern(form: &Form) -> bool {
    matches!(form, Form::Vector(_) | Form::Map(_))
}

fn temporary(next: &mut u64) -> String {
    let value = format!("__hbc_destructure_{}", *next);
    *next += 1;
    value
}

fn call(operator: &str, arguments: Vec<Form>) -> Form {
    list(operator, arguments)
}

fn list(operator: &str, arguments: Vec<Form>) -> Form {
    Form::List(
        std::iter::once(Form::Symbol(operator.into()))
            .chain(arguments)
            .collect(),
    )
}
