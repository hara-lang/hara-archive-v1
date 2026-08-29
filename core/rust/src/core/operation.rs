fn string_value<'a>(value: &'a Value, operation: &str) -> Result<&'a str, String> {
    match value {
        Value::String(value) => Ok(value),
        _ => Err(format!("{operation} expects a string")),
    }
}

fn code_point_length(text: &str) -> usize {
    text.chars().count()
}

fn code_point_slice(text: &str, start: usize, end: usize) -> Result<String, String> {
    let length = text.chars().count();
    if start > end || end > length {
        return Err("str/slice range is out of bounds".into());
    }
    Ok(text.chars().skip(start).take(end - start).collect())
}

fn code_point_char_at(text: &str, index: usize) -> Result<char, String> {
    text.chars()
        .nth(index)
        .ok_or_else(|| "str/char-at index out of bounds".into())
}

fn code_point_byte_index(text: &str, code_point_offset: usize) -> usize {
    text.char_indices()
        .nth(code_point_offset)
        .map(|(byte_index, _)| byte_index)
        .unwrap_or(text.len())
}

fn code_point_index(text: &str, byte_index: usize) -> usize {
    text[..byte_index.min(text.len())].chars().count()
}

fn code_point_index_of(text: &str, part: &str, offset: usize) -> i64 {
    let byte_offset = code_point_byte_index(text, offset);
    text[byte_offset..]
        .find(part)
        .map(|index| (code_point_index(text, byte_offset + index)) as i64)
        .unwrap_or(-1)
}

fn code_point_last_index_of(text: &str, part: &str, offset: usize) -> i64 {
    let len = code_point_length(text);
    if part.is_empty() {
        return (offset.min(len)) as i64;
    }
    let mut code_point_index = 0;
    let mut last: Option<usize> = None;
    for (byte_index, _) in text.char_indices() {
        if code_point_index > offset && offset < len {
            break;
        }
        if text[byte_index..].starts_with(part) {
            last = Some(code_point_index);
        }
        code_point_index += 1;
    }
    last.map(|index| index as i64).unwrap_or(-1)
}

fn string_operation(operation: &str, values: Vec<Value>) -> Result<Value, String> {
    let pair = |values: &[Value]| -> Result<(String, String), String> {
        if values.len() != 2 {
            return Err(format!("{operation} expects two strings"));
        }
        Ok((
            string_value(&values[0], operation)?.to_owned(),
            string_value(&values[1], operation)?.to_owned(),
        ))
    };
    match operation {
        "str/starts-with?" | "str/ends-with?" => {
            let (text, part) = pair(&values)?;
            Ok(Value::Bool(if operation == "str/starts-with?" {
                text.starts_with(&part)
            } else {
                text.ends_with(&part)
            }))
        }
        "str/includes?" => {
            let (text, part) = pair(&values)?;
            Ok(Value::Bool(text.contains(&part)))
        }
        "str/pad-left" | "str/pad-right" => {
            if values.len() != 3 {
                return Err(format!(
                    "{operation} expects a string, length, and padding string"
                ));
            }
            let text = string_value(&values[0], operation)?;
            let length = value_index(&values[1])?;
            let padding = string_value(&values[2], operation)?;
            let text_length = code_point_length(text);
            if padding.is_empty() || text_length >= length {
                return Ok(Value::String(text.into()));
            }
            let needed = length - text_length;
            let padding_chars: Vec<char> = padding.chars().collect();
            let fill: String = padding_chars.iter().cycle().take(needed).copied().collect();
            Ok(Value::String(if operation == "str/pad-left" {
                format!("{fill}{text}")
            } else {
                format!("{text}{fill}")
            }))
        }
        "str/char-at" => {
            if values.len() != 2 {
                return Err("str/char-at expects a string and index".into());
            }
            let text = string_value(&values[0], operation)?;
            let index = value_index(&values[1])?;
            code_point_char_at(text, index).map(Value::Character)
        }
        "str/split" => {
            if values.len() != 2 {
                return Err("str/split expects a string and string or regexp separator".into());
            }
            let text = string_value(&values[0], operation)?;
            if text.is_empty() {
                return Ok(Value::Nil);
            }
            let parts = match &values[1] {
                Value::String(separator) => text
                    .split(separator)
                    .map(|part| Value::String(part.into()))
                    .collect(),
                Value::Regex(pattern) => {
                    if pattern.is_empty() {
                        return Ok(Value::Vector(PVector::from_iter(
                            text.chars()
                                .map(|character| Value::String(character.to_string())),
                        )));
                    }
                    let mut parts: Vec<Value> = regex::Regex::new(pattern)
                        .map_err(|error| format!("invalid regexp: {error}"))?
                        .split(text)
                        .map(|part| Value::String(part.into()))
                        .collect();
                    while matches!(parts.last(), Some(Value::String(value)) if value.is_empty()) {
                        parts.pop();
                    }
                    parts
                }
                _ => return Err("str/split expects a string and string or regexp separator".into()),
            };
            Ok(Value::Vector(parts.into()))
        }
        "str/split-lines" => {
            if values.len() != 1 {
                return Err("str/split-lines expects one string".into());
            }
            let text = string_value(&values[0], operation)?;
            let parts: Vec<Value> = text
                .split('\n')
                .map(|part| Value::String(part.into()))
                .collect();
            Ok(Value::Vector(parts.into()))
        }
        "str/join" => {
            if values.len() != 2 {
                return Err("str/join expects a separator and collection".into());
            }
            let separator = string_value(&values[0], operation)?;
            let parts = iterator_values(values[1].clone())?
                .into_iter()
                .map(|value| match value {
                    Value::String(value) => Ok(value),
                    Value::Character(value) => Ok(value.to_string()),
                    _ => Err("str/join expects a collection of strings or characters".into()),
                })
                .collect::<Result<Vec<String>, String>>()?;
            Ok(Value::String(parts.join(separator)))
        }
        "str/index-of" => {
            if values.len() != 2 && values.len() != 3 {
                return Err("str/index-of expects a string, substring, and optional offset".into());
            }
            let text = string_value(&values[0], operation)?;
            let part = string_value(&values[1], operation)?;
            let offset = if values.len() == 3 {
                value_index(&values[2])?
            } else {
                0
            };
            Ok(Value::Number(code_point_index_of(text, part, offset)))
        }
        "str/last-index-of" => {
            if values.len() != 2 && values.len() != 3 {
                return Err(
                    "str/last-index-of expects a string, substring, and optional offset".into(),
                );
            }
            let text = string_value(&values[0], operation)?;
            let part = string_value(&values[1], operation)?;
            let offset = if values.len() == 3 {
                value_index(&values[2])?
            } else {
                code_point_length(text)
            };
            Ok(Value::Number(code_point_last_index_of(text, part, offset)))
        }
        "str/slice" => {
            if values.len() != 2 && values.len() != 3 {
                return Err("str/slice expects a string, start, and optional end".into());
            }
            let text = string_value(&values[0], operation)?;
            let start = value_index(&values[1])?;
            let end = if values.len() == 3 {
                value_index(&values[2])?
            } else {
                code_point_length(text)
            };
            code_point_slice(text, start, end).map(Value::String)
        }
        "str/to-fixed" => {
            if values.len() != 2 {
                return Err("str/to-fixed expects a number and precision".into());
            }
            let number = numeric::to_f64_explicit(&values[0])
                .map_err(|_| "str/to-fixed expects a number and precision".to_string())?;
            let precision = value_index(&values[1])?;
            if precision > 100 {
                return Err("str/to-fixed precision must be in the range 0..100".into());
            }
            Ok(Value::String(format!("{number:.precision$}")))
        }
        "str/replace" => {
            if values.len() != 3 {
                return Err("str/replace expects a string, match, and replacement".into());
            }
            Ok(Value::String(string_value(&values[0], operation)?.replace(
                string_value(&values[1], operation)?,
                string_value(&values[2], operation)?,
            )))
        }
        "str/replace-first" => {
            if values.len() != 3 {
                return Err("str/replace-first expects a string, match, and replacement".into());
            }
            let text = string_value(&values[0], operation)?;
            let part = string_value(&values[1], operation)?;
            let replacement = string_value(&values[2], operation)?;
            Ok(Value::String(text.replacen(part, replacement, 1)))
        }
        "str/trim" | "str/trim-left" | "str/trim-right" => {
            if values.len() != 1 {
                return Err(format!("{operation} expects one string"));
            }
            let text = string_value(&values[0], operation)?;
            Ok(Value::String(match operation {
                "str/trim" => text.trim().into(),
                "str/trim-left" => text.trim_start().into(),
                _ => text.trim_end().into(),
            }))
        }
        "str/length" => {
            if values.len() != 1 {
                return Err(format!("{operation} expects one string"));
            }
            let text = string_value(&values[0], operation)?;
            Ok(Value::Number(code_point_length(text) as i64))
        }
        "str/blank?" => {
            if values.len() != 1 {
                return Err("str/blank? expects one string".into());
            }
            let text = string_value(&values[0], operation)?;
            Ok(Value::Bool(text.trim().is_empty()))
        }
        "str/repeat" => {
            if values.len() != 2 {
                return Err("str/repeat expects a string and count".into());
            }
            let text = string_value(&values[0], operation)?;
            let count = value_index(&values[1])?;
            Ok(Value::String(text.repeat(count)))
        }
        "str/capitalize" => {
            if values.len() != 1 {
                return Err("str/capitalize expects one string".into());
            }
            let text = string_value(&values[0], operation)?;
            let mut chars = text.chars();
            match chars.next() {
                Some(first) => Ok(Value::String(
                    first.to_uppercase().collect::<String>() + chars.as_str(),
                )),
                None => Ok(Value::String(text.into())),
            }
        }
        "str/decapitalize" => {
            if values.len() != 1 {
                return Err("str/decapitalize expects one string".into());
            }
            let text = string_value(&values[0], operation)?;
            let mut chars = text.chars();
            match chars.next() {
                Some(first) => Ok(Value::String(
                    first.to_lowercase().collect::<String>() + chars.as_str(),
                )),
                None => Ok(Value::String(text.into())),
            }
        }
        "str/upper" => {
            if values.len() != 1 {
                return Err(format!("{operation} expects one string"));
            }
            let text = string_value(&values[0], operation)?;
            Ok(Value::String(text.to_uppercase()))
        }
        "str/lower" => {
            if values.len() != 1 {
                return Err(format!("{operation} expects one string"));
            }
            let text = string_value(&values[0], operation)?;
            Ok(Value::String(text.to_lowercase()))
        }
        "str/reverse" => {
            if values.len() != 1 {
                return Err("str/reverse expects one string".into());
            }
            let text = string_value(&values[0], operation)?;
            Ok(Value::String(text.chars().rev().collect()))
        }
        "str/encode-utf8" => {
            if values.len() != 1 {
                return Err(format!("{operation} expects one string"));
            }
            match &values[0] {
                Value::String(text) => Ok(Value::ByteBuffer(Rc::new(RefCell::new(
                    text.as_bytes().to_vec(),
                )))),
                _ => Err(format!("{operation} expects a string")),
            }
        }
        "str/decode-utf8" => {
            if values.len() != 1 {
                return Err(format!("{operation} expects bytes"));
            }
            let raw = byte_values(&values[0], operation)?;
            String::from_utf8(raw)
                .map(Value::String)
                .map_err(|_| format!("{operation} invalid UTF-8"))
        }
        _ => Err(format!("unknown string operation: {operation}")),
    }
}
fn marker_key(value: &Value, operation: &str) -> Result<String, String> {
    match value {
        Value::String(key) => Ok(key.clone()),
        Value::Keyword(key) => Ok(key.as_str().to_owned()),
        _ => Err(format!("{operation} expects a string key")),
    }
}

fn native_mutable_values(operation: &str, values: Vec<Value>) -> Result<Value, String> {
    let (type_name, method) = operation
        .strip_prefix("std.native.")
        .and_then(|name| name.split_once('/'))
        .ok_or_else(|| format!("invalid native mutable operation: {operation}"))?;
    if method == "new" {
        return if type_name == "Arr" {
            Ok(Value::Array(Rc::new(RefCell::new(values))))
        } else {
            if values.len() % 2 != 0 {
                return Err("object expects key/value pairs".into());
            }
            let pairs = values
                .chunks(2)
                .map(|pair| Ok((marker_key(&pair[0], "object")?, pair[1].clone())))
                .collect::<Result<Vec<_>, String>>()?;
            Ok(Value::Object(Rc::new(RefCell::new(pairs))))
        };
    }
    if values.is_empty() {
        return Err(format!(
            "std.native.{type_name}/{method} expects a receiver"
        ));
    }
    let supported = native_declarations()
        .iter()
        .find(|declaration| declaration.name == type_name)
        .is_some_and(|declaration| declaration.method(method));
    if !supported {
        return Err(format!("unknown std.native.{type_name} method: {method}"));
    }
    let receiver = values[0].clone();
    let args = values[1..].to_vec();
    marker_call_values(receiver, method, args)
}

fn dot_call(
    receiver: Value,
    method: &Form,
    env: &mut HashMap<String, Value>,
) -> Result<Value, String> {
    let parts = match method {
        Form::List(parts) if !parts.is_empty() => parts,
        _ => return Err("dot call expects a method list".into()),
    };
    let name = match &parts[0] {
        Form::Symbol(name) => name.as_str(),
        _ => return Err("dot method must be a symbol".into()),
    };
    let args = parts[1..]
        .iter()
        .map(|form| eval(form, env))
        .collect::<Result<Vec<_>, _>>()?;
    dot_call_values(receiver, name, args)
}

pub(crate) fn dot_call_values(
    receiver: Value,
    name: &str,
    args: Vec<Value>,
) -> Result<Value, String> {
    if matches!(receiver, Value::Array(_) | Value::Object(_)) {
        return Err(
            "dot calls do not support arrays or objects; use Arr/ or Obj/ functions".into(),
        );
    }
    marker_call_values(receiver, name, args)
}

fn marker_call_values(receiver: Value, name: &str, args: Vec<Value>) -> Result<Value, String> {
    match receiver {
        Value::Array(array) => match name {
            "get" => {
                if args.len() < 1 || args.len() > 2 {
                    return Err("array/get expects an index and optional default".into());
                }
                let index = value_index(&args[0])?;
                Ok(array
                    .borrow()
                    .get(index)
                    .cloned()
                    .or_else(|| args.get(1).cloned())
                    .unwrap_or(Value::Nil))
            }
            "set" => {
                if args.len() != 2 {
                    return Err("array/set expects an index and value".into());
                }
                let index = value_index(&args[0])?;
                let mut values = array.borrow_mut();
                if index >= values.len() {
                    return Err("array/set index out of bounds".into());
                }
                values[index] = args[1].clone();
                drop(values);
                Ok(Value::Array(array))
            }
            "push-first" => {
                if args.len() != 1 {
                    return Err("array/push-first expects one value".into());
                }
                array.borrow_mut().insert(0, args[0].clone());
                Ok(Value::Array(array))
            }
            "push-last" => {
                if args.len() != 1 {
                    return Err("array/push-last expects one value".into());
                }
                array.borrow_mut().push(args[0].clone());
                Ok(Value::Array(array))
            }
            "pop-first" => {
                if !args.is_empty() {
                    return Err("array/pop-first expects no arguments".into());
                }
                let mut values = array.borrow_mut();
                Ok(if values.is_empty() {
                    Value::Nil
                } else {
                    values.remove(0)
                })
            }
            "pop-last" => {
                if !args.is_empty() {
                    return Err("array/pop-last expects no arguments".into());
                }
                Ok(array.borrow_mut().pop().unwrap_or(Value::Nil))
            }
            "insert" => {
                if args.len() != 2 {
                    return Err("array/insert expects an index and value".into());
                }
                let index = value_index(&args[0])?;
                let mut values = array.borrow_mut();
                if index > values.len() {
                    return Err("array/insert index out of bounds".into());
                }
                values.insert(index, args[1].clone());
                drop(values);
                Ok(Value::Array(array))
            }
            "remove" => {
                if args.len() != 1 {
                    return Err("array/remove expects an index".into());
                }
                let index = value_index(&args[0])?;
                let mut values = array.borrow_mut();
                if index >= values.len() {
                    return Err("array/remove index out of bounds".into());
                }
                Ok(values.remove(index))
            }
            "clone" => {
                if !args.is_empty() {
                    return Err("array/clone expects no arguments".into());
                }
                Ok(Value::Array(Rc::new(RefCell::new(array.borrow().clone()))))
            }
            "slice" => {
                if args.is_empty() || args.len() > 2 {
                    return Err("array/slice expects start and optional end".into());
                }
                let start = value_index(&args[0])?;
                let end = if args.len() == 2 {
                    value_index(&args[1])?
                } else {
                    array.borrow().len()
                };
                let values = array.borrow();
                if start > end || end > values.len() {
                    return Err("array/slice range is out of bounds".into());
                }
                Ok(Value::Array(Rc::new(RefCell::new(
                    values[start..end].to_vec(),
                ))))
            }
            "map" | "filter" => {
                if args.len() != 1 {
                    return Err(format!("array/{name} expects one function"));
                }
                let function = match &args[0] {
                    Value::Function(function) => function,
                    _ => return Err(format!("array/{name} expects a function")),
                };
                let mut output = Vec::new();
                for value in array.borrow().iter().cloned() {
                    let mapped = call_function(function, vec![value.clone()])?;
                    if name == "map" {
                        output.push(mapped);
                    } else if mapped.truthy() {
                        output.push(value);
                    }
                }
                Ok(Value::Array(Rc::new(RefCell::new(output))))
            }
            "fold-left" | "fold-right" => {
                if args.len() != 2 {
                    return Err(format!("array/{name} expects a function and initial value"));
                }
                let function = match &args[0] {
                    Value::Function(function) => function,
                    _ => return Err(format!("array/{name} expects a function")),
                };
                let values = array.borrow();
                let mut output = args[1].clone();
                if name == "fold-left" {
                    for value in values.iter().cloned() {
                        output = call_function(function, vec![output, value])?;
                    }
                } else {
                    for value in values.iter().rev().cloned() {
                        output = call_function(function, vec![value, output])?;
                    }
                }
                Ok(output)
            }
            _ => Err(format!("unsupported array method: {name}")),
        },
        Value::Object(object) => match name {
            "has?" => {
                if args.len() != 1 {
                    return Err("object/has? expects a key".into());
                }
                let key = marker_key(&args[0], "object/has?")?;
                Ok(Value::Bool(
                    object
                        .borrow()
                        .iter()
                        .any(|(candidate, _)| candidate == &key),
                ))
            }
            "get" => {
                if args.len() < 1 || args.len() > 2 {
                    return Err("object/get expects a key and optional default".into());
                }
                let key = marker_key(&args[0], "object/get")?;
                Ok(object
                    .borrow()
                    .iter()
                    .find(|(candidate, _)| candidate == &key)
                    .map(|(_, value)| value.clone())
                    .or_else(|| args.get(1).cloned())
                    .unwrap_or(Value::Nil))
            }
            "set" => {
                if args.len() != 2 {
                    return Err("object/set expects a key and value".into());
                }
                let key = marker_key(&args[0], "object/set")?;
                let mut values = object.borrow_mut();
                if let Some((_, value)) = values.iter_mut().find(|(candidate, _)| candidate == &key)
                {
                    *value = args[1].clone();
                } else {
                    values.push((key, args[1].clone()));
                }
                drop(values);
                Ok(Value::Object(object))
            }
            "delete" => {
                if args.len() != 1 {
                    return Err("object/delete expects a key".into());
                }
                let key = marker_key(&args[0], "object/delete")?;
                let mut values = object.borrow_mut();
                if let Some(index) = values.iter().position(|(candidate, _)| candidate == &key) {
                    Ok(values.remove(index).1)
                } else {
                    Ok(Value::Nil)
                }
            }
            "keys" | "vals" | "pairs" => {
                if !args.is_empty() {
                    return Err(format!("object/{name} expects no arguments"));
                }
                let output = object
                    .borrow()
                    .iter()
                    .map(|(key, value)| match name {
                        "keys" => Value::String(key.clone()),
                        "vals" => value.clone(),
                        _ => Value::Array(Rc::new(RefCell::new(vec![
                            Value::String(key.clone()),
                            value.clone(),
                        ]))),
                    })
                    .collect();
                Ok(Value::Array(Rc::new(RefCell::new(output))))
            }
            "assign" => {
                if args.len() != 1 {
                    return Err("object/assign expects an object".into());
                }
                let other = match &args[0] {
                    Value::Object(other) => other.clone(),
                    _ => return Err("object/assign expects an object".into()),
                };
                let mut values = object.borrow_mut();
                for (key, value) in other.borrow().iter() {
                    if let Some((_, existing)) =
                        values.iter_mut().find(|(candidate, _)| candidate == key)
                    {
                        *existing = value.clone();
                    } else {
                        values.push((key.clone(), value.clone()));
                    }
                }
                drop(values);
                Ok(Value::Object(object))
            }
            "clone" => {
                if !args.is_empty() {
                    return Err("object/clone expects no arguments".into());
                }
                Ok(Value::Object(Rc::new(RefCell::new(
                    object.borrow().clone(),
                ))))
            }
            _ => Err(format!("unsupported object method: {name}")),
        },
        _ => Err("dot calls require an array or object marker".into()),
    }
}

fn byte_input(value: &Value, operation: &str) -> Result<u8, String> {
    match value {
        Value::Number(number) if (-128..=255).contains(number) => Ok((*number as i8) as u8),
        _ => Err(format!(
            "{operation} expects a value in the range -128..255"
        )),
    }
}

fn byte_buffer(value: &Value, operation: &str) -> Result<Rc<RefCell<Vec<u8>>>, String> {
    match value {
        Value::ByteBuffer(bytes) => Ok(bytes.clone()),
        _ => Err(format!("{operation} expects bytes")),
    }
}

fn byte_values(value: &Value, operation: &str) -> Result<Vec<u8>, String> {
    match value {
        Value::Bytes(bytes) => Ok(bytes.clone()),
        Value::ByteBuffer(bytes) => Ok(bytes.borrow().clone()),
        _ => Err(format!("{operation} expects bytes")),
    }
}

fn byte_count(value: &Value) -> Result<Value, String> {
    match value {
        Value::Bytes(bytes) => Ok(Value::Number(bytes.len() as i64)),
        Value::ByteBuffer(bytes) => Ok(Value::Number(bytes.borrow().len() as i64)),
        _ => Err("bytes/count expects bytes".into()),
    }
}

fn byte_get(value: &Value, index: &Value, default: Option<Value>) -> Result<Value, String> {
    let index = value_index(index)?;
    let found = match value {
        Value::Bytes(bytes) => bytes.get(index).copied(),
        Value::ByteBuffer(bytes) => bytes.borrow().get(index).copied(),
        _ => return Err("bytes/get expects bytes".into()),
    };
    match found {
        Some(byte) => Ok(Value::Number(byte as i64)),
        None => default.ok_or_else(|| "bytes/get index out of bounds".into()),
    }
}

fn byte_copy(value: &Value) -> Result<Value, String> {
    let bytes = byte_buffer(value, "bytes/copy")?;
    let copied = bytes.borrow().clone();
    Ok(Value::ByteBuffer(Rc::new(RefCell::new(copied))))
}

fn byte_slice(value: &Value, start: &Value, end: &Value) -> Result<Value, String> {
    let start = value_index(start)?;
    let end = value_index(end)?;
    let bytes = byte_buffer(value, "bytes/slice")?;
    let bytes = bytes.borrow();
    if start > end || end > bytes.len() {
        return Err(format!(
            "bytes/slice range is out of bounds: {start}..{end}"
        ));
    }
    Ok(Value::ByteBuffer(Rc::new(RefCell::new(
        bytes[start..end].to_vec(),
    ))))
}

fn byte_set(value: &Value, index: &Value, item: &Value) -> Result<Value, String> {
    let index = value_index(index)?;
    let item = byte_input(item, "bytes/set")?;
    let bytes = byte_buffer(value, "bytes/set")?;
    let mut bytes = bytes.borrow_mut();
    if index >= bytes.len() {
        return Err("bytes/set index out of bounds".into());
    }
    bytes[index] = item;
    Ok(value.clone())
}

pub(crate) fn iterator_values(value: Value) -> Result<Vec<Value>, String> {
    match value {
        Value::Seq(values) => values.iter().collect::<Result<Vec<_>, _>>(),
        Value::Extension(receiver) => {
            let value = Value::Extension(receiver.clone());
            let iterator = extension_protocol_call(
                &receiver,
                "std.protocol.iiter.IIter",
                "iter",
                std::slice::from_ref(&value),
            )?;
            iterator_values(iterator)
        }
        Value::Nil => Ok(Vec::new()),
        Value::Tuple(values) => Ok(values.iter().cloned().collect()),
        Value::Vector(values) => Ok(values.iter().cloned().collect()),
        Value::MapEntry(entry) => Ok(entry.iter().cloned().collect()),
        Value::List(values) => Ok(values.iter().cloned().collect()),
        Value::Cons(values) => Ok(values.iter().collect()),
        Value::Deque(values) => Ok(values.iter().cloned().collect()),
        Value::Queue(values) => Ok(values.iter().cloned().collect()),
        Value::PriorityMap(values) => Ok(values
            .iter()
            .map(|(key, value)| pair_value(key, value))
            .collect()),
        Value::String(text) => Ok(text.chars().map(Value::Character).collect()),
        Value::Bytes(bytes) => Ok(bytes
            .into_iter()
            .map(|byte| Value::Number(byte as i8 as i64))
            .collect()),
        Value::ByteBuffer(bytes) => Ok(bytes
            .borrow()
            .iter()
            .map(|byte| Value::Number(*byte as i8 as i64))
            .collect()),
        Value::Array(values) => Ok(values.borrow().clone()),
        Value::Object(values) => Ok(values
            .borrow()
            .iter()
            .map(|(key, value)| pair_value(Value::String(key.clone()), value.clone()))
            .collect()),
        Value::Struct(value) => Ok(value
            .ordered_entries()
            .into_iter()
            .map(|(key, value)| pair_value(key, value))
            .collect()),
        Value::Mutable(value) => Ok(value
            .ordered_entries()
            .into_iter()
            .map(|(key, value)| pair_value(key, value))
            .collect()),
        Value::MutableCollection(collection) => {
            let borrowed = collection.borrow();
            let collection = borrowed
                .as_ref()
                .ok_or_else(|| "mutable collection used after to-persistent".to_string())?;
            match collection {
                MutableCollection::Map(values) => Ok(values
                    .iter()
                    .map(|(key, value)| pair_value(key.clone(), value.clone()))
                    .collect()),
                MutableCollection::OrderedMap(values) => Ok(values
                    .iter()
                    .map(|(key, value)| pair_value(key.clone(), value.clone()))
                    .collect()),
                MutableCollection::SortedMap(values) => Ok(values
                    .iter()
                    .map(|(key, value)| pair_value(key.clone(), value.clone()))
                    .collect()),
                MutableCollection::Trie(values) => Ok(values
                    .entries()
                    .into_iter()
                    .map(|(key, value)| pair_value(Value::String(key.clone()), value.clone()))
                    .collect()),
                MutableCollection::Set(values) => Ok(values.iter().cloned().collect()),
                MutableCollection::OrderedSet(values) => Ok(values.iter().cloned().collect()),
                MutableCollection::SortedSet(values) => Ok(values.iter().cloned().collect()),
                MutableCollection::List(values) => Ok(values.iter().cloned().collect()),
                MutableCollection::Queue(values) => Ok(values.iter().cloned().collect()),
                MutableCollection::Vector(values) => Ok(values.iter().cloned().collect()),
            }
        }
        Value::Pointer(pointer) => Ok(pointer
            .fields()
            .iter()
            .map(|(key, value)| pair_value(key.clone(), value.clone()))
            .collect()),
        value @ (Value::Map(_) | Value::OrderedMap(_) | Value::SortedMap(_) | Value::Trie(_)) => {
            Ok(map_entries(&value)
                .unwrap()
                .into_iter()
                .map(|(key, value)| pair_value(key, value))
                .collect())
        }
        value @ (Value::Set(_) | Value::OrderedSet(_) | Value::SortedSet(_)) => {
            Ok(set_items(&value).unwrap().into_iter().cloned().collect())
        }
        Value::Iterator(_) => {
            let mut values = Vec::new();
            while let Some(item) = iterator_try_next(&value)? {
                values.push(item);
            }
            Ok(values)
        }
        value => Err(format!(
            "iter expects a collection, got {}",
            value.display()
        )),
    }
}

fn iterator_to_vec(value: Value) -> Result<Vec<Value>, String> {
    iterator_values(value)
}

fn make_iterator(value: Value) -> Result<Value, String> {
    match &value {
        Value::Iterator(_) => Ok(value),
        Value::Seq(sequence) => Ok(Value::Iterator(Rc::new(RefCell::new(
            IteratorState::generated(IteratorGenerator::Seq((**sequence).clone())),
        )))),
        Value::Nil
        | Value::String(_)
        | Value::Bytes(_)
        | Value::ByteBuffer(_)
        | Value::Array(_)
        | Value::Object(_)
        | Value::Struct(_)
        | Value::Mutable(_)
        | Value::MutableCollection(_)
        | Value::Map(_)
        | Value::OrderedMap(_)
        | Value::SortedMap(_)
        | Value::Trie(_)
        | Value::PriorityMap(_)
        | Value::Pointer(_)
        | Value::Set(_)
        | Value::OrderedSet(_)
        | Value::SortedSet(_)
        | Value::List(_)
        | Value::Cons(_)
        | Value::Queue(_)
        | Value::Deque(_)
        | Value::Tuple(_)
        | Value::MapEntry(_)
        | Value::Vector(_) => Ok(Value::Iterator(Rc::new(RefCell::new(IteratorState::new(
            iterator_values(value)?,
        ))))),
        _ => match protocol_call("std.protocol.iiter.IIter", "iter", &[value])? {
            Value::Iterator(iterator) => Ok(Value::Iterator(iterator)),
            _ => Err("IIter/iter must return an iterator".into()),
        },
    }
}

pub fn iterator_from_values(values: Vec<Value>) -> Value {
    Value::Iterator(Rc::new(RefCell::new(IteratorState::new(values))))
}

fn iterator_seq(value: Value) -> Result<Value, String> {
    if matches!(value, Value::Seq(_)) {
        return Ok(value);
    }
    let source = make_iterator(value)?;
    let sequence = PSeq::new(RuntimeSeqSource {
        source,
        finished: false,
    });
    match sequence.peek_first() {
        None => Ok(Value::Nil),
        Some(Ok(_)) => Ok(Value::Seq(Box::new(sequence))),
        Some(Err(error)) => Err(error),
    }
}

struct RuntimeSeqSource {
    source: Value,
    finished: bool,
}

impl Iterator for RuntimeSeqSource {
    type Item = Result<Value, String>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }
        match iterator_try_next(&self.source) {
            Ok(Some(value)) => Some(Ok(value)),
            Ok(None) => {
                self.finished = true;
                None
            }
            Err(error) => {
                self.finished = true;
                Some(Err(error))
            }
        }
    }
}

fn iterator_constant(value: Value) -> Value {
    Value::Iterator(Rc::new(RefCell::new(IteratorState::generated(
        IteratorGenerator::Constant(value),
    ))))
}
fn iterator_prepend(head: Value, source: Value) -> Result<Value, String> {
    let source = match source {
        Value::Iterator(iterator) => Value::Iterator(iterator),
        value => make_iterator(value)?,
    };
    let state = IteratorState::generated(IteratorGenerator::Prepend(Some(head), source));
    Ok(Value::Iterator(Rc::new(RefCell::new(state))))
}
fn iterator_repeated(function: Value) -> Value {
    Value::Iterator(Rc::new(RefCell::new(IteratorState::generated(
        IteratorGenerator::Repeated(function),
    ))))
}
fn iterator_iterate(function: Value, seed: Value) -> Value {
    Value::Iterator(Rc::new(RefCell::new(IteratorState::generated(
        IteratorGenerator::Iterate(function, seed),
    ))))
}
fn iterator_take_while(function: Value, value: Value) -> Result<Value, String> {
    let source = match value {
        Value::Iterator(iterator) => Value::Iterator(iterator),
        value => make_iterator(value)?,
    };
    Ok(Value::Iterator(Rc::new(RefCell::new(
        IteratorState::generated(IteratorGenerator::TakeWhile(function, source)),
    ))))
}
fn iterator_map(function: Value, value: Value) -> Result<Value, String> {
    iterator_map_with(function, value, false)
}
fn iterator_map_with(function: Value, value: Value, spread: bool) -> Result<Value, String> {
    let source = match value {
        Value::Iterator(iterator) => Value::Iterator(iterator),
        value => make_iterator(value)?,
    };
    Ok(Value::Iterator(Rc::new(RefCell::new(
        IteratorState::generated(IteratorGenerator::Map(function, source, spread)),
    ))))
}
fn iterator_partition(value: Value, amount: usize, all: bool) -> Result<Value, String> {
    if amount == 0 {
        return Err("partition amount must be positive".into());
    }
    let source = match value {
        Value::Iterator(iterator) => Value::Iterator(iterator),
        value => make_iterator(value)?,
    };
    Ok(Value::Iterator(Rc::new(RefCell::new(
        IteratorState::generated(IteratorGenerator::Partition(source, amount, all)),
    ))))
}

fn iterator_interleave(values: Vec<Value>) -> Result<Value, String> {
    let sources = values
        .into_iter()
        .map(|value| match value {
            Value::Iterator(iterator) => Ok(Value::Iterator(iterator)),
            value => make_iterator(value),
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Value::Iterator(Rc::new(RefCell::new(
        IteratorState::generated(IteratorGenerator::Interleave(sources, 0)),
    ))))
}

fn iterator_interpose(separator: Value, value: Value) -> Result<Value, String> {
    let source = match value {
        Value::Iterator(iterator) => Value::Iterator(iterator),
        value => make_iterator(value)?,
    };
    Ok(Value::Iterator(Rc::new(RefCell::new(
        IteratorState::generated(IteratorGenerator::Interpose(source, separator, true, None)),
    ))))
}

fn iterator_concat(values: Vec<Value>) -> Result<Value, String> {
    let sources = values
        .into_iter()
        .map(|value| match value {
            Value::Iterator(iterator) => Ok(Value::Iterator(iterator)),
            value => make_iterator(value),
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Value::Iterator(Rc::new(RefCell::new(
        IteratorState::generated(IteratorGenerator::Concat(sources, 0)),
    ))))
}

fn iterator_zip(values: Vec<Value>) -> Result<Value, String> {
    let sources = values
        .into_iter()
        .map(|value| match value {
            Value::Iterator(iterator) => Ok(Value::Iterator(iterator)),
            value => make_iterator(value),
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Value::Iterator(Rc::new(RefCell::new(
        IteratorState::generated(IteratorGenerator::Zip(sources)),
    ))))
}

fn iterator_mapcat(function: Value, value: Value) -> Result<Value, String> {
    let source = match value {
        Value::Iterator(iterator) => Value::Iterator(iterator),
        value => make_iterator(value)?,
    };
    Ok(Value::Iterator(Rc::new(RefCell::new(
        IteratorState::generated(IteratorGenerator::Mapcat(function, source, None)),
    ))))
}
fn iterator_keep(function: Value, value: Value) -> Result<Value, String> {
    let source = match value {
        Value::Iterator(iterator) => Value::Iterator(iterator),
        value => make_iterator(value)?,
    };
    Ok(Value::Iterator(Rc::new(RefCell::new(
        IteratorState::generated(IteratorGenerator::Keep(function, source)),
    ))))
}

fn iterator_filter(function: Value, value: Value) -> Result<Value, String> {
    let source = match value {
        Value::Iterator(iterator) => Value::Iterator(iterator),
        value => make_iterator(value)?,
    };
    Ok(Value::Iterator(Rc::new(RefCell::new(
        IteratorState::generated(IteratorGenerator::Filter(function, source)),
    ))))
}

fn iterator_drop_while(function: Value, value: Value) -> Result<Value, String> {
    let source = match value {
        Value::Iterator(iterator) => Value::Iterator(iterator),
        value => make_iterator(value)?,
    };
    Ok(Value::Iterator(Rc::new(RefCell::new(
        IteratorState::generated(IteratorGenerator::DropWhile(function, source, false)),
    ))))
}
fn iterator_take(value: Value, amount: usize) -> Result<Value, String> {
    let source = match value {
        Value::Iterator(iterator) => Value::Iterator(iterator),
        value => make_iterator(value)?,
    };
    Ok(Value::Iterator(Rc::new(RefCell::new(
        IteratorState::generated(IteratorGenerator::Take(source, amount)),
    ))))
}
fn iterator_drop(value: Value, amount: usize) -> Result<Value, String> {
    let source = match value {
        Value::Iterator(iterator) => Value::Iterator(iterator),
        value => make_iterator(value)?,
    };
    Ok(Value::Iterator(Rc::new(RefCell::new(
        IteratorState::generated(IteratorGenerator::Drop(source, amount)),
    ))))
}

fn iterator_cycle(value: Value) -> Result<Value, String> {
    let source = match value {
        Value::Iterator(iterator) => Value::Iterator(iterator),
        value => make_iterator(value)?,
    };
    if !matches!(iterator_has_next(&source)?, Value::Bool(true)) {
        return Err("cycle expects a non-empty source".into());
    }
    Ok(Value::Iterator(Rc::new(RefCell::new(
        IteratorState::generated(IteratorGenerator::Cycle(source, Vec::new(), 0, false)),
    ))))
}

fn iterator_has_next(value: &Value) -> Result<Value, String> {
    match value {
        Value::Iterator(iterator) => Ok(Value::Bool(iterator.borrow_mut().has_next()?)),
        _ => Err("iter-next? expects an iterator".into()),
    }
}

fn iterator_try_next(value: &Value) -> Result<Option<Value>, String> {
    match value {
        Value::Iterator(iterator) => iterator.borrow_mut().try_next(),
        _ => Err("iter-next expects an iterator".into()),
    }
}

fn iterator_next(value: &Value) -> Result<Value, String> {
    iterator_try_next(value)?.ok_or_else(|| "iter-next reached the end of the iterator".into())
}

fn iterator_close(value: &Value) -> Result<Value, String> {
    match value {
        Value::Iterator(iterator) => {
            iterator.borrow_mut().close();
            Ok(Value::Nil)
        }
        _ => Err("iter-close expects an iterator".into()),
    }
}

fn collection_first(value: Value) -> Result<Value, String> {
    match value {
        Value::Seq(sequence) => sequence
            .peek_first()
            .transpose()?
            .ok_or_else(|| "invalid empty Seq value".to_string()),
        Value::Iterator(iterator) => Ok(iterator.borrow_mut().try_next()?.unwrap_or(Value::Nil)),
        value => Ok(iterator_values(value)?
            .into_iter()
            .next()
            .unwrap_or(Value::Nil)),
    }
}

fn collection_rest(value: Value) -> Result<Value, String> {
    if let Value::Seq(sequence) = value {
        let tail = sequence.pop_first();
        return match tail.peek_first() {
            None => Ok(Value::Nil),
            Some(Ok(_)) => Ok(Value::Seq(Box::new(tail))),
            Some(Err(error)) => Err(error),
        };
    }
    let source = match value {
        Value::Iterator(iterator) => Value::Iterator(iterator),
        value => make_iterator(value)?,
    };
    if iterator_try_next(&source)?.is_none() {
        return Ok(Value::Nil);
    }
    iterator_seq(source)
}

fn collection_last(value: Value) -> Result<Value, String> {
    Ok(iterator_to_vec(value)?
        .into_iter()
        .last()
        .unwrap_or(Value::Nil))
}

fn collection_empty_value(value: Value) -> Result<Value, String> {
    match value {
        Value::Extension(receiver) => extension_protocol_call(
            &receiver,
            "std.protocol.iempty.IEmpty",
            "empty",
            &[Value::Extension(receiver.clone())],
        ),
        Value::Nil => Ok(Value::Nil),
        Value::Array(_) => Ok(Value::Array(Rc::new(RefCell::new(Vec::new())))),
        Value::Object(_) => Ok(Value::Object(Rc::new(RefCell::new(Vec::new())))),
        Value::List(values) => Ok(Value::List(values.empty())),
        Value::Cons(values) => Ok(Value::List(PList::new().with_meta(values.meta().cloned()))),
        Value::Queue(values) => Ok(Value::Queue(Box::new(values.empty()))),
        Value::Deque(values) => Ok(Value::Deque(Box::new(values.empty()))),
        Value::Vector(values) => Ok(Value::Vector(values.empty())),
        Value::Tuple(values) => Ok(Value::Tuple(Box::new(values.empty()))),
        Value::Seq(values) => Ok(Value::Tuple(Box::new(
            PTuple::Tup0.with_meta(values.meta().cloned()),
        ))),
        Value::Map(values) => Ok(Value::Map(values.empty())),
        Value::OrderedMap(values) => Ok(Value::OrderedMap(Box::new(values.empty()))),
        Value::SortedMap(values) => Ok(Value::SortedMap(Box::new(values.empty()))),
        Value::Trie(values) => Ok(Value::Trie(Box::new(values.empty()))),
        Value::PriorityMap(values) => Ok(Value::PriorityMap(Box::new(values.empty()))),
        Value::Set(values) => Ok(Value::Set(values.empty())),
        Value::OrderedSet(values) => Ok(Value::OrderedSet(Box::new(values.empty()))),
        Value::SortedSet(values) => Ok(Value::SortedSet(Box::new(values.empty()))),
        Value::Struct(value) => Ok(Value::Struct(Rc::new(StructValue::from_values(
            value.ty.clone(),
            vec![Value::Nil; value.ty.fields.len()],
            value.metadata.clone(),
        )?))),
        Value::Mutable(_) => Err("empty does not support mutable values".into()),
        value => Err(format!(
            "empty expects a collection, got {}",
            portable_type_name(&value)
        )),
    }
}

fn collection_count(value: &Value) -> Result<Value, String> {
    if let Value::Extension(receiver) = value {
        return extension_protocol_call(
            receiver,
            "std.protocol.icount.ICount",
            "count",
            std::slice::from_ref(value),
        );
    }
    let count = match value {
        Value::Nil => 0,
        Value::String(v) => v.chars().count(),
        Value::Tuple(v) => v.len(),
        Value::Vector(v) => v.len(),
        Value::MapEntry(_) => 2,
        Value::List(v) => v.len(),
        Value::Cons(v) => v.iter().count(),
        Value::Queue(v) => v.len(),
        Value::Deque(v) => v.len(),
        value @ (Value::Map(_)
        | Value::OrderedMap(_)
        | Value::SortedMap(_)
        | Value::Trie(_)
        | Value::PriorityMap(_)) => map_entries(value).unwrap().len(),
        value @ (Value::Set(_) | Value::OrderedSet(_) | Value::SortedSet(_)) => {
            set_items(value).unwrap().len()
        }
        Value::Bytes(v) => v.len(),
        Value::ByteBuffer(v) => v.borrow().len(),
        Value::Array(v) => v.borrow().len(),
        Value::Object(v) => v.borrow().len(),
        Value::Struct(v) => v.ty.fields.len(),
        Value::Mutable(v) => v.ty.fields.len(),
        Value::Pointer(v) => v.fields().len(),
        Value::MutableCollection(collection) => {
            let borrowed = collection.borrow();
            let mutable = borrowed
                .as_ref()
                .ok_or_else(|| "mutable collection used after to-persistent".to_string())?;
            match mutable {
                MutableCollection::Map(values) => values.len(),
                MutableCollection::OrderedMap(values) => values.len(),
                MutableCollection::SortedMap(values) => values.len(),
                MutableCollection::Trie(values) => values.len(),
                MutableCollection::Set(values) => values.len(),
                MutableCollection::OrderedSet(values) => values.len(),
                MutableCollection::SortedSet(values) => values.len(),
                MutableCollection::List(values) => values.len(),
                MutableCollection::Queue(values) => values.len(),
                MutableCollection::Vector(values) => values.len(),
            }
        }
        Value::Seq(sequence) => {
            let mut count = 0;
            for value in sequence.iter() {
                value?;
                count += 1;
            }
            count
        }
        Value::Iterator(_) => {
            let mut count = 0;
            while iterator_try_next(value)?.is_some() {
                count += 1;
            }
            count
        }
        _ => return Err("count expects a collection".into()),
    };
    Ok(Value::Number(count as i64))
}

fn iterator_is_finite(value: &Value) -> bool {
    match value {
        Value::Iterator(iterator) => iterator.borrow().is_finite(),
        Value::Seq(_) => false,
        _ => true,
    }
}

fn collection_get(value: &Value, key: &Value, default: Value) -> Result<Value, String> {
    match value {
        Value::Extension(receiver) => extension_protocol_call(
            receiver,
            "std.protocol.ilookup.ILookup",
            "lookup",
            &[value.clone(), key.clone(), default],
        ),
        Value::Nil | Value::Seq(_) => Ok(default),
        Value::Tuple(values) => {
            let index = value_index(key)?;
            Ok(values.get(index).cloned().unwrap_or(default))
        }
        Value::Vector(values) => {
            let index = value_index(key)?;
            Ok(values.get(index).cloned().unwrap_or(default))
        }
        Value::MapEntry(entry) => {
            let index = value_index(key)?;
            Ok(entry.nth(index).cloned().unwrap_or(default))
        }
        Value::Array(values) => {
            let index = value_index(key)?;
            Ok(values.borrow().get(index).cloned().unwrap_or(default))
        }
        Value::Cons(values) => {
            let index = value_index(key)?;
            Ok(values.iter().nth(index).unwrap_or(default))
        }
        Value::List(values) => {
            let index = value_index(key)?;
            Ok(values.get(index).cloned().unwrap_or(default))
        }
        Value::Queue(values) => {
            let index = value_index(key)?;
            Ok(values.get(index).cloned().unwrap_or(default))
        }
        Value::Deque(values) => {
            let index = value_index(key)?;
            Ok(values.get(index).cloned().unwrap_or(default))
        }
        Value::MutableCollection(collection) => {
            let borrowed = collection.borrow();
            let mutable = borrowed
                .as_ref()
                .ok_or_else(|| "mutable collection used after to-persistent".to_string())?;
            let found = match mutable {
                MutableCollection::Map(values) => values.get(key).cloned(),
                MutableCollection::OrderedMap(values) => values.get(key).cloned(),
                MutableCollection::SortedMap(values) => values.get(key).cloned(),
                MutableCollection::Trie(values) => values.get(&marker_key(key, "trie")?).cloned(),
                MutableCollection::Set(values) => values.get(key).cloned(),
                MutableCollection::OrderedSet(values) => values.get(key).cloned(),
                MutableCollection::SortedSet(values) => values.get(key).cloned(),
                MutableCollection::List(values) => values.get(value_index(key)?).cloned(),
                MutableCollection::Queue(values) => values.get(value_index(key)?).cloned(),
                MutableCollection::Vector(values) => values.get(value_index(key)?).cloned(),
            };
            Ok(found.unwrap_or(default))
        }
        Value::Bytes(_) | Value::ByteBuffer(_) => byte_get(value, key, Some(default)),
        Value::String(text) => {
            let index = value_index(key)?;
            Ok(text
                .chars()
                .nth(index)
                .map(Value::Character)
                .unwrap_or(default))
        }
        value @ (Value::Map(_)
        | Value::OrderedMap(_)
        | Value::SortedMap(_)
        | Value::Trie(_)
        | Value::PriorityMap(_)) => Ok(map_value(value, key).cloned().unwrap_or(default)),
        value @ (Value::Set(_) | Value::OrderedSet(_) | Value::SortedSet(_)) => {
            Ok(set_find(value, key).unwrap_or(default))
        }
        Value::Object(entries) => {
            let name = match key {
                Value::String(name) => name.as_str(),
                Value::Keyword(name) => name.as_str(),
                _ => return Ok(default),
            };
            Ok(entries
                .borrow()
                .iter()
                .find(|(candidate, _)| candidate == name)
                .map(|(_, value)| value.clone())
                .unwrap_or(default))
        }
        Value::Struct(value) => Ok(named_field_name(key)
            .and_then(|name| value.get(name))
            .cloned()
            .unwrap_or(default)),
        Value::Mutable(value) => Ok(named_field_name(key)
            .and_then(|name| value.get(name))
            .unwrap_or(default)),
        Value::Pointer(pointer) => Ok(pointer.get(key).cloned().unwrap_or(default)),
        Value::Result(result) => {
            let Value::Keyword(key) = key else {
                return Ok(default);
            };
            Ok(match key.as_str() {
                "status" => result.status_value(),
                "data" => result.data.clone(),
                "error" => result.error_value(),
                "context" => {
                    if map_entries(&result.context).is_some_and(|entries| entries.is_empty()) {
                        Value::Nil
                    } else {
                        result.context.clone()
                    }
                }
                _ => default,
            })
        }
        value => Err(format!(
            "get expects a collection, received {}",
            portable_type_name(value)
        )),
    }
}

fn collection_nth(value: &Value, key: &Value) -> Result<Value, String> {
    let index = value_index(key)?;
    if let Value::Iterator(iterator) = value {
        let mut state = iterator.borrow_mut();
        for _ in 0..index {
            if state.try_next()?.is_none() {
                return Err("nth index out of bounds".into());
            }
        }
        return state
            .try_next()?
            .ok_or_else(|| "nth index out of bounds".into());
    }
    let result = match value {
        Value::Tuple(values) => values.get(index).cloned(),
        Value::Vector(values) => values.get(index).cloned(),
        Value::MapEntry(entry) => entry.nth(index).cloned(),
        Value::Array(values) => values.borrow().get(index).cloned(),
        Value::Cons(values) => values.iter().nth(index),
        Value::List(values) => values.get(index).cloned(),
        Value::Queue(values) => values.get(index).cloned(),
        Value::Deque(values) => values.get(index).cloned(),
        Value::MutableCollection(collection) => {
            let borrowed = collection.borrow();
            let mutable = borrowed
                .as_ref()
                .ok_or_else(|| "mutable collection used after to-persistent".to_string())?;
            match mutable {
                MutableCollection::List(values) => values.get(index).cloned(),
                MutableCollection::Queue(values) => values.get(index).cloned(),
                MutableCollection::Vector(values) => values.get(index).cloned(),
                _ => return Err("nth expects an indexed collection".into()),
            }
        }
        Value::String(text) => text
            .chars()
            .nth(index)
            .map(Value::Character),
        _ => return Err("nth expects an indexed collection".into()),
    };
    result.ok_or_else(|| "nth index out of bounds".into())
}

fn collection_assoc(value: &Value, key: &Value, replacement: Value) -> Result<Value, String> {
    match value {
        Value::Extension(receiver) => extension_protocol_call(
            receiver,
            "std.protocol.iassoc.IAssoc",
            "assoc",
            &[value.clone(), key.clone(), replacement],
        ),
        Value::MutableCollection(collection) => {
            let mut borrowed = collection.borrow_mut();
            let mutable = borrowed
                .as_mut()
                .ok_or_else(|| "mutable collection used after to-persistent".to_string())?;
            match mutable {
                MutableCollection::Map(values) => {
                    values.assoc(key.clone(), replacement);
                }
                MutableCollection::OrderedMap(values) => {
                    values.assoc(key.clone(), replacement);
                }
                MutableCollection::SortedMap(values) => {
                    values.assoc(key.clone(), replacement);
                }
                MutableCollection::Trie(values) => {
                    values.assoc(marker_key(key, "trie")?, replacement);
                }
                MutableCollection::Vector(values) => {
                    values.assoc(value_index(key)?, replacement);
                }
                MutableCollection::List(values) => {
                    values
                        .assoc(value_index(key)?, replacement)
                        .ok_or_else(|| "assoc index out of bounds".to_string())?;
                }
                _ => return Err("assoc expects a mutable map, vector, or list".into()),
            }
            Ok(Value::MutableCollection(collection.clone()))
        }
        Value::Tuple(values) => {
            let index = value_index(key)?;
            if index == values.len() {
                return tuple_push_last(values, replacement);
            }
            if index > values.len() {
                return Err("assoc index out of bounds".into());
            }
            let mut items: Vec<Value> = values.iter().cloned().collect();
            items[index] = replacement;
            Ok(Value::Tuple(Box::new(
                PTuple::from_values(items)?.with_meta(values.meta().cloned()),
            )))
        }
        Value::Vector(values) => {
            let index = value_index(key)?;
            values
                .assoc_value(index, replacement)
                .map(Value::Vector)
                .ok_or_else(|| "assoc index out of bounds".into())
        }
        Value::Deque(values) => values
            .assoc_value(value_index(key)?, replacement)
            .map(|values| Value::Deque(Box::new(values)))
            .ok_or_else(|| "assoc index out of bounds".to_string()),
        value @ (Value::Map(_)
        | Value::OrderedMap(_)
        | Value::SortedMap(_)
        | Value::Trie(_)
        | Value::PriorityMap(_)) => map_assoc_value(value, key.clone(), replacement),
        Value::Object(entries) => {
            let name = marker_key(key, "object")?;
            let mut output = entries.borrow().clone();
            if let Some((_, item)) = output.iter_mut().find(|(candidate, _)| candidate == &name) {
                *item = replacement;
            } else {
                output.push((name, replacement));
            }
            Ok(Value::Object(Rc::new(RefCell::new(output))))
        }
        Value::Struct(value) => {
            let name = named_field_name(key).ok_or_else(|| {
                "assoc struct field must be an unqualified string, keyword, or symbol".to_string()
            })?;
            if !value.ty.fields.iter().any(|candidate| candidate == name) {
                return Err(format!("unknown struct field: {name}"));
            }
            Ok(Value::Struct(Rc::new(StructValue {
                ty: value.ty.clone(),
                values: value.values.assoc_value(named_field_key(name), replacement),
                metadata: value.metadata.clone(),
            })))
        }
        Value::Mutable(_) => Err("assoc does not support mutable values".into()),
        Value::Nil => Ok(Value::Map(
            PMap::new().assoc_value(key.clone(), replacement),
        )),
        _ => Err("assoc expects a vector, map, object, or struct".into()),
    }
}

fn collection_dissoc(value: &Value, keys: &[Value]) -> Result<Value, String> {
    match value {
        Value::Extension(_) => keys.iter().try_fold(value.clone(), |current, key| {
            let Value::Extension(receiver) = &current else {
                return collection_dissoc(&current, std::slice::from_ref(key));
            };
            extension_protocol_call(
                receiver,
                "std.protocol.idissoc.IDissoc",
                "dissoc",
                &[current.clone(), key.clone()],
            )
        }),
        Value::Mutable(_) => Err("dissoc does not support mutable values".into()),
        Value::Struct(value) => {
            let declared = keys
                .iter()
                .filter_map(named_field_name)
                .any(|name| value.ty.fields.iter().any(|candidate| candidate == name));
            if !declared {
                return Ok(Value::Struct(value.clone()));
            }
            let mut values = value.values.with_meta(value.metadata.clone());
            for key in keys {
                if let Some(name) = named_field_name(key) {
                    values = values.dissoc_value(&named_field_key(name));
                }
            }
            Ok(Value::OrderedMap(Box::new(values)))
        }
        Value::MutableCollection(collection) => {
            let mut collection_value = collection.borrow_mut();
            let mutable = collection_value
                .as_mut()
                .ok_or_else(|| "mutable collection used after to-persistent".to_string())?;
            for key in keys {
                match &mut *mutable {
                    MutableCollection::Map(values) => {
                        values.dissoc(key);
                    }
                    MutableCollection::OrderedMap(values) => {
                        values.dissoc(key);
                    }
                    MutableCollection::SortedMap(values) => {
                        values.dissoc(key);
                    }
                    MutableCollection::Trie(values) => {
                        values.dissoc(&marker_key(key, "trie")?);
                    }
                    MutableCollection::Set(values) => {
                        values.dissoc(key);
                    }
                    MutableCollection::OrderedSet(values) => {
                        values.dissoc(key);
                    }
                    MutableCollection::SortedSet(values) => {
                        values.dissoc(key);
                    }
                    _ => return Err("dissoc expects a mutable map or set".into()),
                }
            }
            drop(collection_value);
            Ok(Value::MutableCollection(collection.clone()))
        }
        value @ (Value::Map(_)
        | Value::OrderedMap(_)
        | Value::SortedMap(_)
        | Value::Trie(_)
        | Value::PriorityMap(_)) => keys
            .iter()
            .try_fold(value.clone(), |map, key| map_dissoc_value(&map, key)),
        value @ (Value::Set(_) | Value::OrderedSet(_) | Value::SortedSet(_)) => keys
            .iter()
            .try_fold(value.clone(), |set, key| set_dissoc_value(&set, key)),
        Value::Nil => Ok(Value::Map(PMap::new())),
        _ => Err("dissoc expects a map".into()),
    }
}

fn unique_values(values: Vec<Value>) -> Vec<Value> {
    let mut unique = Vec::new();
    for value in values {
        if !unique.contains(&value) {
            unique.push(value);
        }
    }
    unique
}
