#[cfg(all(target_arch = "wasm32", not(feature = "raw-wasm")))]
fn js_error_string(error: JsValue) -> String {
    if let Some(message) = error.as_string() {
        return message;
    }
    js_sys::Reflect::get(&error, &JsValue::from_str("message"))
        .ok()
        .and_then(|message| message.as_string())
        .unwrap_or_else(|| format!("{error:?}"))
}

#[cfg(all(target_arch = "wasm32", not(feature = "raw-wasm")))]
fn host_key_to_string(key: &core::Value) -> String {
    match key {
        core::Value::String(text) => text.clone(),
        core::Value::Keyword(keyword) => keyword.as_str().to_owned(),
        core::Value::Symbol(symbol) => symbol.as_str().to_owned(),
        other => other.display(),
    }
}

#[cfg(all(target_arch = "wasm32", not(feature = "raw-wasm")))]
fn host_seq_to_js<'a>(values: impl Iterator<Item = &'a core::Value>) -> Result<JsValue, String> {
    let array = js_sys::Array::new();
    for value in values {
        array.push(&value_to_js(value)?);
    }
    Ok(array.into())
}

#[cfg(all(target_arch = "wasm32", not(feature = "raw-wasm")))]
fn value_to_js(value: &core::Value) -> Result<JsValue, String> {
    match value {
        core::Value::Nil => Ok(JsValue::NULL),
        core::Value::Bool(flag) => Ok(JsValue::from_bool(*flag)),
        core::Value::Number(number)
            if (*number as i128).abs() <= js_sys::Number::MAX_SAFE_INTEGER as i128 =>
        {
            Ok(JsValue::from_f64(*number as f64))
        }
        core::Value::Number(number) => Ok(js_sys::BigInt::from(*number).into()),
        core::Value::BigInteger(number) => js_sys::BigInt::new(&JsValue::from_str(&number.to_string()))
            .map(Into::into)
            .map_err(|error| format!("std.native.Host/call integer-invalid: {}", js_error_string(error.into()))),
        core::Value::Float(number) => crate::numeric::finite_float(*number)
            .map(|number| JsValue::from_f64(number)),
        core::Value::String(text) => Ok(JsValue::from_str(text)),
        core::Value::Keyword(keyword) => Ok(JsValue::from_str(keyword.as_str())),
        core::Value::Symbol(symbol) => Ok(JsValue::from_str(symbol.as_str())),
        core::Value::Bytes(bytes) => Ok(js_sys::Uint8Array::from(&bytes[..]).into()),
        core::Value::Vector(values) => host_seq_to_js(values.iter()),
        core::Value::List(values) => host_seq_to_js(values.iter()),
        core::Value::Set(values) => host_seq_to_js(values.iter()),
        core::Value::OrderedSet(values) => host_seq_to_js(values.iter()),
        core::Value::Map(values) => {
            let object = js_sys::Object::new();
            for (key, value) in values.iter() {
                js_sys::Reflect::set(
                    &object,
                    &JsValue::from_str(&host_key_to_string(key)),
                    &value_to_js(value)?,
                )
                .map_err(js_error_string)?;
            }
            Ok(object.into())
        }
        core::Value::OrderedMap(values) => {
            let object = js_sys::Object::new();
            for entry in values.iter() {
                js_sys::Reflect::set(
                    &object,
                    &JsValue::from_str(&host_key_to_string(&entry.0)),
                    &value_to_js(&entry.1)?,
                )
                .map_err(js_error_string)?;
            }
            Ok(object.into())
        }
        other => Err(format!(
            "std.native.Host/call type-not-transportable: {}",
            other.display()
        )),
    }
}

#[cfg(all(target_arch = "wasm32", not(feature = "raw-wasm")))]
fn js_to_value(value: &JsValue) -> Result<core::Value, String> {
    use crate::lang::data::{OrderedMap as POrderedMap, Vector as PVector};
    use wasm_bindgen::{closure::Closure, JsCast};

    if value.is_null() || value.is_undefined() {
        return Ok(core::Value::Nil);
    }
    if let Some(flag) = value.as_bool() {
        return Ok(core::Value::Bool(flag));
    }
    if value.is_bigint() {
        let integer: js_sys::BigInt = value.clone().unchecked_into();
        if let Ok(value) = i64::try_from(integer.clone()) {
            return Ok(core::Value::Number(value));
        }
        let text = integer
            .to_string(10)
            .map_err(|error| format!("std.native.Host/call bigint is invalid: {error:?}"))?
            .as_string()
            .ok_or("std.native.Host/call bigint has no decimal representation")?;
        let value = num_bigint::BigInt::parse_bytes(text.as_bytes(), 10)
            .ok_or("std.native.Host/call bigint is invalid")?;
        return Ok(crate::numeric::compact_integer(value));
    }
    if let Some(number) = value.as_f64() {
        if number.fract() == 0.0
            && number >= js_sys::Number::MIN_SAFE_INTEGER
            && number <= js_sys::Number::MAX_SAFE_INTEGER
        {
            return Ok(core::Value::Number(number as i64));
        }
        return crate::numeric::finite_float(number).map(core::Value::Float);
    }
    if let Some(text) = value.as_string() {
        return Ok(core::Value::String(text));
    }
    if value.is_instance_of::<js_sys::Uint8Array>() {
        return Ok(core::Value::Bytes(js_sys::Uint8Array::new(value).to_vec()));
    }
    if value.is_instance_of::<js_sys::Promise>() {
        let source: js_sys::Promise = value.clone().unchecked_into();
        let pending = core::Promise::new();
        let fulfilled = pending.clone();
        let rejected = pending.clone();
        let on_fulfilled = Closure::once(move |value: JsValue| {
            match js_to_value(&value) {
                Ok(value) => fulfilled.resolve(value),
                Err(error) => fulfilled.reject(format!("host/result-invalid: {error}")),
            };
        });
        let on_rejected = Closure::once(move |error: JsValue| {
            rejected.reject(format!("host/rejected: {}", js_error_string(error)));
        });
        let _ = source.then2(&on_fulfilled, &on_rejected);
        on_fulfilled.forget();
        on_rejected.forget();
        return Ok(core::Value::Promise(pending));
    }
    if js_sys::Array::is_array(value) {
        let array = js_sys::Array::from(value);
        let mut items = Vec::with_capacity(array.length() as usize);
        for index in 0..array.length() {
            items.push(js_to_value(&array.get(index))?);
        }
        return Ok(core::Value::Vector(PVector::from_iter(items)));
    }
    if value.is_object() {
        let entries = js_sys::Object::entries(value.unchecked_ref::<js_sys::Object>());
        let mut items = Vec::with_capacity(entries.length() as usize);
        for index in 0..entries.length() {
            let entry = js_sys::Array::from(&entries.get(index));
            let key = entry.get(0).as_string().unwrap_or_default();
            let item = js_to_value(&entry.get(1))?;
            items.push((core::Value::String(key), item));
        }
        return Ok(core::Value::OrderedMap(Box::new(POrderedMap::from_iter(
            items,
        ))));
    }
    Err("std.native.Host/call type-not-transportable: unsupported JS result".into())
}

#[cfg(all(target_arch = "wasm32", not(feature = "raw-wasm")))]
fn host_call_bridge(
    handler: js_sys::Function,
) -> Rc<dyn Fn(String, String, Vec<core::Value>) -> Result<core::Value, String>> {
    Rc::new(move |service, method, args| {
        let js_args = js_sys::Array::new();
        for value in &args {
            js_args.push(&value_to_js(value)?);
        }
        let result = handler
            .call3(
                &JsValue::NULL,
                &JsValue::from(service),
                &JsValue::from(method),
                js_args.as_ref(),
            )
            .map_err(js_error_string)?;
        js_to_value(&result)
    })
}

#[cfg(all(target_arch = "wasm32", not(feature = "raw-wasm")))]
#[wasm_bindgen]
pub fn target_profile() -> String {
    if cfg!(target_os = "wasi") {
        "wasi".into()
    } else if cfg!(target_arch = "wasm32") {
        "wasm".into()
    } else {
        "native".into()
    }
}

#[cfg(all(target_arch = "wasm32", not(feature = "raw-wasm")))]
#[wasm_bindgen]
pub fn version() -> String {
    "hara-wasm/0.1 core-language slice".to_string()
}
