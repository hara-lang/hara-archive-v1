use crate::kernel::{parse, Form};

fn map_entries(form: &Form) -> Result<&[(Form, Form)], String> {
    match form {
        Form::Map(entries) => Ok(entries),
        _ => Err("CLI manifest value must be an EDN map".into()),
    }
}

fn map_entries_mut(form: &mut Form) -> Result<&mut Vec<(Form, Form)>, String> {
    match form {
        Form::Map(entries) => Ok(entries),
        _ => Err("CLI manifest value must be an EDN map".into()),
    }
}

fn map_value<'a>(entries: &'a [(Form, Form)], key: &str) -> Option<&'a Form> {
    entries.iter().find_map(|(candidate, value)| {
        matches!(candidate, Form::Keyword(name) if name.as_str() == key).then_some(value)
    })
}

fn map_value_mut<'a>(entries: &'a mut [(Form, Form)], key: &str) -> Option<&'a mut Form> {
    entries.iter_mut().find_map(|(candidate, value)| {
        matches!(candidate, Form::Keyword(name) if name.as_str() == key).then_some(value)
    })
}

fn set_map_value(entries: &mut Vec<(Form, Form)>, key: &str, value: Form) {
    if let Some(current) = map_value_mut(entries, key) {
        *current = value;
    } else {
        entries.push((Form::Keyword(key.into()), value));
    }
}

fn keyword_value(form: &Form) -> Option<&str> {
    match form {
        Form::Keyword(value) => Some(value),
        _ => None,
    }
}

pub(super) fn map_keyword<'a>(form: &'a Form, key: &str) -> Option<&'a str> {
    map_entries(form)
        .ok()
        .and_then(|entries| map_value(entries, key))
        .and_then(keyword_value)
}

fn vector_mut(form: &mut Form) -> Result<&mut Vec<Form>, String> {
    match form {
        Form::Vector(values) => Ok(values),
        _ => Err("CLI manifest collection must be an EDN vector".into()),
    }
}

fn append_unique_entry(values: &mut Vec<Form>, field: &str, id: &str, entry: Form) {
    if !values
        .iter()
        .any(|candidate| map_keyword(candidate, field) == Some(id))
    {
        values.push(entry);
    }
}

pub(super) fn merge_sources(base: &str, extension: &str) -> Result<String, String> {
    let mut manifest = parse(base)?;
    let extension = parse(extension)?;
    let extension_entries = map_entries(&extension)?;

    let app_id = map_value(extension_entries, "app/id")
        .and_then(keyword_value)
        .ok_or("CLI manifest extension is missing keyword :app/id")?
        .to_owned();
    let app_summary = map_value(extension_entries, "app/summary")
        .cloned()
        .ok_or("CLI manifest extension is missing :app/summary")?;
    let route = map_value(extension_entries, "route")
        .cloned()
        .ok_or("CLI manifest extension is missing :route")?;
    let handler = map_value(extension_entries, "handler")
        .cloned()
        .ok_or("CLI manifest extension is missing :handler")?;

    let route_id = map_keyword(&route, "route/id")
        .ok_or("CLI route is missing keyword :route/id")?
        .to_owned();
    let route_handler = map_keyword(&route, "route/handler")
        .ok_or("CLI route is missing keyword :route/handler")?
        .to_owned();
    let handler_id = map_keyword(&handler, "handler/id")
        .ok_or("CLI handler is missing keyword :handler/id")?
        .to_owned();
    if route_handler != handler_id {
        return Err("CLI route and handler ids do not match".into());
    }

    let manifest_entries = map_entries_mut(&mut manifest)?;
    let apps = map_value_mut(manifest_entries, "cli/apps")
        .ok_or_else(|| "CLI manifest is missing :cli/apps".to_owned())?;
    let apps = vector_mut(apps)?;
    let mut app_found = false;
    for app in apps.iter_mut() {
        if map_keyword(app, "app/id") == Some(app_id.as_str()) {
            app_found = true;
            let app_entries = map_entries_mut(app)?;
            set_map_value(app_entries, "app/summary", app_summary.clone());
            let routes = map_value_mut(app_entries, "app/routes")
                .ok_or_else(|| "CLI app is missing :app/routes".to_owned())?;
            let routes = vector_mut(routes)?;
            if !routes
                .iter()
                .any(|candidate| keyword_value(candidate) == Some(route_id.as_str()))
            {
                routes.push(Form::Keyword(route_id.clone()));
            }
        }
    }
    if !app_found {
        return Err(format!("CLI manifest is missing app :{app_id}"));
    }

    let manifest_entries = map_entries_mut(&mut manifest)?;
    let routes = map_value_mut(manifest_entries, "cli/routes")
        .ok_or_else(|| "CLI manifest is missing :cli/routes".to_owned())?;
    let routes = vector_mut(routes)?;
    append_unique_entry(routes, "route/id", &route_id, route);

    let manifest_entries = map_entries_mut(&mut manifest)?;
    let handlers = map_value_mut(manifest_entries, "cli/handlers")
        .ok_or_else(|| "CLI manifest is missing :cli/handlers".to_owned())?;
    let handlers = vector_mut(handlers)?;
    append_unique_entry(handlers, "handler/id", &handler_id, handler);

    Ok(manifest.to_string())
}
