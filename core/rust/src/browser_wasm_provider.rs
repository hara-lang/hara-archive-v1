#![cfg(target_arch = "wasm32")]

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::BTreeSet;
use wasm_bindgen::{JsCast, JsValue};

use crate::core::Value;
use crate::extension::{ExtensionManifest, WasmAbi, WasmExtensionProvider};
use crate::wasm_binding::{HaraValueType, Lifting, MemoryBindingPlan, Ownership};

pub(crate) struct BrowserWasmProvider {
    mode: BrowserProviderMode,
}

enum BrowserProviderMode {
    Direct {
        module: js_sys::WebAssembly::Module,
        instance: RefCell<Option<js_sys::WebAssembly::Instance>>,
    },
    Memory {
        module: js_sys::WebAssembly::Module,
        plan: MemoryBindingPlan,
        session: RefCell<Option<BrowserMemorySession>>,
    },
}

struct BrowserMemorySession {
    instance: js_sys::WebAssembly::Instance,
    memory: js_sys::WebAssembly::Memory,
}

impl BrowserWasmProvider {
    pub(crate) fn compile(bytes: &[u8]) -> Result<Self, String> {
        let buffer = js_sys::Uint8Array::from(bytes);
        let module = js_sys::WebAssembly::Module::new(buffer.as_ref())
            .map_err(|error| format!("native/module-invalid: {}", js_error(error)))?;
        if js_sys::WebAssembly::Module::imports(&module).length() != 0 {
            return Err("native/module-import-denied: direct WASM must be import-free".into());
        }
        Ok(Self {
            mode: BrowserProviderMode::Direct {
                module,
                instance: RefCell::new(None),
            },
        })
    }

    pub(crate) fn compile_memory(bytes: &[u8], plan: MemoryBindingPlan) -> Result<Self, String> {
        let inspection = crate::direct_wasm::inspect(bytes)?;
        plan.verify(&inspection)?;
        let buffer = js_sys::Uint8Array::from(bytes);
        let module = js_sys::WebAssembly::Module::new(buffer.as_ref())
            .map_err(|error| format!("native/module-invalid: {}", js_error(error)))?;
        if js_sys::WebAssembly::Module::imports(&module).length() != 0 {
            return Err("native/module-import-denied: memory.v1 must be import-free".into());
        }
        Ok(Self {
            mode: BrowserProviderMode::Memory {
                module,
                plan,
                session: RefCell::new(None),
            },
        })
    }
}

impl WasmExtensionProvider for BrowserWasmProvider {
    fn supports(&self, abi: WasmAbi) -> bool {
        matches!(
            (&self.mode, abi),
            (BrowserProviderMode::Direct { .. }, WasmAbi::CoreV1)
                | (BrowserProviderMode::Memory { .. }, WasmAbi::MemoryV1)
        )
    }

    fn start(&self, manifest: &ExtensionManifest) -> Result<(), String> {
        if !manifest.capabilities.is_empty() {
            return Err("native/capability-denied: direct WASM has no host authority".into());
        }
        let BrowserProviderMode::Direct {
            module,
            instance: instance_slot,
        } = &self.mode
        else {
            let BrowserProviderMode::Memory {
                module,
                plan,
                session,
            } = &self.mode
            else {
                unreachable!()
            };
            if manifest.exports.len() != plan.functions.len()
                || manifest.exports.iter().any(|(name, specification)| {
                    plan.functions
                        .iter()
                        .find(|function| function.name == *name)
                        .map_or(true, |function| {
                            specification.raw_name(name) != function.wasm_export
                        })
                })
            {
                return Err(
                    "native/manifest-mismatch: memory.v1 exports do not match bindings.edn".into(),
                );
            }
            let instance = js_sys::WebAssembly::Instance::new(module, &js_sys::Object::new())
                .map_err(|error| format!("native/module-invalid: {}", js_error(error)))?;
            let memory = memory_export(&instance, &plan.memory.export)?;
            check_memory_limit(&memory)?;
            *session.borrow_mut() = Some(BrowserMemorySession { instance, memory });
            return Ok(());
        };
        let instance = js_sys::WebAssembly::Instance::new(module, &js_sys::Object::new())
            .map_err(|error| format!("native/module-invalid: {}", js_error(error)))?;
        *instance_slot.borrow_mut() = Some(instance);
        Ok(())
    }

    fn invoke(
        &self,
        manifest: &ExtensionManifest,
        export: &str,
        arguments: &[Value],
    ) -> Result<Value, String> {
        if let BrowserProviderMode::Memory { plan, session, .. } = &self.mode {
            return invoke_memory(plan, session, export, arguments);
        }
        let BrowserProviderMode::Direct { instance, .. } = &self.mode else {
            unreachable!()
        };
        let instance = instance.borrow();
        let exports = instance
            .as_ref()
            .ok_or("native/import-not-started")?
            .exports();
        let function = js_sys::Reflect::get(&exports, &JsValue::from_str(export))
            .map_err(js_error)?
            .dyn_into::<js_sys::Function>()
            .map_err(|_| format!("native/export-missing: {export}"))?;
        let args = js_sys::Array::new();
        let specification = manifest
            .exports
            .iter()
            .find(|(name, _)| name == export)
            .map(|(_, specification)| specification)
            .ok_or_else(|| format!("native/export-missing: {export}"))?;
        for (wire, argument) in specification.arguments.iter().zip(arguments) {
            args.push(&match (wire.as_str(), argument) {
                ("i64", Value::Number(value)) => js_sys::BigInt::from(*value).into(),
                ("i64", Value::BigInteger(_)) => {
                    return Err(format!(
                        "native/integer-overflow: {export} expects signed 64-bit integer"
                    ));
                }
                ("i32", Value::Number(value)) if i32::try_from(*value).is_ok() => {
                    JsValue::from_f64(*value as f64)
                }
                ("f32" | "f64", Value::Number(value)) => {
                    let value = *value as f64;
                    if !value.is_finite() {
                        return Err(format!("non-finite number: {export}"));
                    }
                    JsValue::from_f64(value)
                }
                ("f32" | "f64", Value::Float(value)) => {
                    if !value.is_finite() {
                        return Err(format!("non-finite number: {export}"));
                    }
                    JsValue::from_f64(*value)
                }
                _ => return Err(format!("native/type-error: {export} expects {wire}")),
            });
        }
        let result = function
            .apply(&JsValue::UNDEFINED, &args)
            .map_err(|error| format!("native/invoke-failed: {export} ({})", js_error(error)))?;
        match specification.returns.as_str() {
            "void" if result.is_undefined() => Ok(Value::Nil),
            "i64" if result.is_bigint() => i64::try_from(result.unchecked_into::<js_sys::BigInt>())
                .map(Value::Number)
                .map_err(|_| {
                    format!(
                        "native/integer-overflow: {export} result is outside signed 64-bit range"
                    )
                }),
            "i32" => result
                .as_f64()
                .map(|value| Value::Number(value as i32 as i64))
                .ok_or_else(|| format!("native/result-type-invalid: {export}")),
            "f32" | "f64" => {
                let value = result
                    .as_f64()
                    .ok_or_else(|| format!("native/result-type-invalid: {export}"))?;
                if !value.is_finite() {
                    return Err(format!("non-finite number: {export}"));
                }
                Ok(Value::Float(value))
            }
            _ => Err(format!("native/result-type-invalid: {export}")),
        }
    }

    fn cancel(&self, _manifest: &ExtensionManifest, _request: u64) -> Result<(), String> {
        Err("native/cancel-unsupported: core.v1 calls are synchronous".into())
    }

    fn shutdown(&self, _manifest: &ExtensionManifest) {
        match &self.mode {
            BrowserProviderMode::Direct { instance, .. } => {
                instance.borrow_mut().take();
            }
            BrowserProviderMode::Memory { session, .. } => {
                session.borrow_mut().take();
            }
        }
    }
}

const MAX_MEMORY_BYTES: usize = 64 * 1024 * 1024;
const MAX_VALUE_BYTES: usize = 16 * 1024 * 1024;
const MAX_TOTAL_INPUT_BYTES: usize = 32 * 1024 * 1024;
const MAX_TOTAL_COPY_BYTES: usize = MAX_TOTAL_INPUT_BYTES + MAX_VALUE_BYTES;

fn memory_export(
    instance: &js_sys::WebAssembly::Instance,
    name: &str,
) -> Result<js_sys::WebAssembly::Memory, String> {
    let value = js_sys::Reflect::get(&instance.exports(), &JsValue::from_str(name))
        .map_err(|error| format!("native/memory-missing: {}", js_error(error)))?;
    value
        .dyn_into()
        .map_err(|_| format!("native/memory-invalid: {name}"))
}

fn exported_function(
    instance: &js_sys::WebAssembly::Instance,
    name: &str,
) -> Result<js_sys::Function, String> {
    js_sys::Reflect::get(&instance.exports(), &JsValue::from_str(name))
        .map_err(|error| format!("native/export-missing: {} ({})", name, js_error(error)))?
        .dyn_into()
        .map_err(|_| format!("native/export-invalid: {name}"))
}

fn memory_size(memory: &js_sys::WebAssembly::Memory) -> usize {
    js_sys::Uint8Array::new(&memory.buffer()).length() as usize
}

fn check_memory_limit(memory: &js_sys::WebAssembly::Memory) -> Result<(), String> {
    if memory_size(memory) > MAX_MEMORY_BYTES {
        Err("native/resource-limit: memory exceeds the memory.v1 limit".into())
    } else {
        Ok(())
    }
}

fn invoke_memory(
    plan: &MemoryBindingPlan,
    session: &RefCell<Option<BrowserMemorySession>>,
    export: &str,
    arguments: &[Value],
) -> Result<Value, String> {
    let function_plan = plan
        .functions
        .iter()
        .find(|function| function.name == export)
        .ok_or_else(|| format!("native/export-missing: {export}"))?;
    if arguments.len() != function_plan.arguments.len() {
        return Err(format!(
            "native/arity: {export} expects {} arguments, got {}",
            function_plan.arguments.len(),
            arguments.len()
        ));
    }
    let mut slot = session.borrow_mut();
    let mut session = slot.take().ok_or("native/import-not-started")?;
    let mut release_always = BTreeSet::new();
    let mut release_on_failure = BTreeSet::new();
    let mut call_completed = false;
    let result = invoke_memory_inner(
        plan,
        function_plan,
        &mut session,
        arguments,
        &mut release_always,
        &mut release_on_failure,
        &mut call_completed,
    );
    if !call_completed {
        release_always.extend(release_on_failure);
    }
    let cleanup = release_memory(plan, &session, &release_always);
    let result = combine_memory_outcome(result, cleanup);
    *slot = Some(session);
    result
}

#[allow(clippy::too_many_arguments)]
fn invoke_memory_inner(
    plan: &MemoryBindingPlan,
    function_plan: &crate::wasm_binding::MemoryFunctionPlan,
    session: &mut BrowserMemorySession,
    arguments: &[Value],
    release_always: &mut BTreeSet<i32>,
    release_on_failure: &mut BTreeSet<i32>,
    call_completed: &mut bool,
) -> Result<Value, String> {
    let raw_arguments = js_sys::Array::new();
    let mut total_input_bytes = 0usize;
    let mut total_copy_bytes = 0usize;

    for (argument_plan, value) in function_plan.arguments.iter().zip(arguments) {
        if argument_plan.lowering.is_none() {
            raw_arguments.push(&scalar_argument(
                &argument_plan.hara_type,
                value,
                &function_plan.name,
            )?);
            continue;
        }
        let bytes: Cow<'_, [u8]> = match (&argument_plan.hara_type, value) {
            (HaraValueType::Bytes, Value::Bytes(bytes)) => Cow::Borrowed(bytes),
            (HaraValueType::Bytes, Value::ByteBuffer(bytes)) => Cow::Owned(bytes.borrow().clone()),
            (HaraValueType::String, Value::String(value)) => Cow::Borrowed(value.as_bytes()),
            _ => {
                return Err(format!(
                    "native/type-error: {} expects :{}",
                    function_plan.name,
                    hara_type_name(&argument_plan.hara_type)
                ))
            }
        };
        total_input_bytes = total_input_bytes
            .checked_add(bytes.len())
            .ok_or_else(|| "native/resource-limit: input byte count overflow".to_owned())?;
        if bytes.len() > MAX_VALUE_BYTES || total_input_bytes > MAX_TOTAL_INPUT_BYTES {
            return Err(format!(
                "native/resource-limit: {} input exceeds the memory.v1 byte limit",
                function_plan.name
            ));
        }
        let length = i32::try_from(bytes.len()).map_err(|_| {
            format!(
                "native/resource-limit: {} input is too large",
                function_plan.name
            )
        })?;
        let (pointer, length) = if length == 0 {
            (0, 0)
        } else {
            let allocator_name = plan
                .memory
                .allocate
                .as_deref()
                .ok_or_else(|| format!("native/allocator-missing: {}", function_plan.name))?;
            let allocator = exported_function(&session.instance, allocator_name)?;
            let raw_pointer = allocator
                .call1(&JsValue::UNDEFINED, &JsValue::from_f64(length as f64))
                .map_err(|error| {
                    format!(
                        "native/allocator-failed: {} ({})",
                        function_plan.name,
                        js_error(error)
                    )
                })?;
            let pointer = js_i32(&raw_pointer).ok_or_else(|| {
                format!("native/allocator-invalid: {allocator_name} returned a non-integer")
            })?;
            if pointer != 0 && argument_plan.ownership == Some(Ownership::Transferred) {
                release_on_failure.insert(pointer);
            }
            let start = checked_range(&session.memory, pointer, length, &function_plan.name)?;
            check_memory_limit(&session.memory)?;
            let target = js_sys::Uint8Array::new(&session.memory.buffer());
            target.set(&js_sys::Uint8Array::from(bytes.as_ref()), start as u32);
            (pointer, length)
        };
        total_copy_bytes = total_copy_bytes
            .checked_add(bytes.len())
            .ok_or_else(|| "native/resource-limit: copy byte count overflow".to_owned())?;
        if total_copy_bytes > MAX_TOTAL_COPY_BYTES {
            return Err(format!(
                "native/resource-limit: {} exceeds the memory.v1 aggregate copy limit",
                function_plan.name
            ));
        }
        raw_arguments.push(&JsValue::from_f64(pointer as f64));
        raw_arguments.push(&JsValue::from_f64(length as f64));
    }

    let function = exported_function(&session.instance, &function_plan.wasm_export)?;
    let raw_result = function
        .apply(&JsValue::UNDEFINED, &raw_arguments)
        .map_err(|error| {
            format!(
                "native/invoke-failed: {} ({})",
                function_plan.name,
                js_error(error)
            )
        });
    let outcome = match raw_result {
        Ok(raw_result) => {
            *call_completed = true;
            lift_memory_result(
                &function_plan.name,
                &function_plan.returns,
                raw_result,
                session,
                release_always,
                &mut total_copy_bytes,
            )
        }
        Err(error) => Err(error),
    };
    outcome
}

fn scalar_argument(
    expected: &HaraValueType,
    value: &Value,
    export: &str,
) -> Result<JsValue, String> {
    match (expected, value) {
        (HaraValueType::I32, Value::Number(value)) if i32::try_from(*value).is_ok() => {
            Ok(JsValue::from_f64(*value as i32 as f64))
        }
        (HaraValueType::Boolean, Value::Bool(value)) => {
            Ok(JsValue::from_f64(i32::from(*value) as f64))
        }
        (HaraValueType::I64, value) => crate::numeric::to_i64_integer(value)
            .map(|value| js_sys::BigInt::from(value).into())
            .map_err(|_| format!(
                "native/integer-overflow: {export} expects signed 64-bit integer"
            )),
        (HaraValueType::F32 | HaraValueType::F64, Value::Float(value)) => {
            if !value.is_finite() {
                return Err(format!("non-finite number: {export}"));
            }
            Ok(JsValue::from_f64(*value))
        }
        (HaraValueType::F32 | HaraValueType::F64, Value::Number(value)) => {
            let value = *value as f64;
            if !value.is_finite() {
                return Err(format!("non-finite number: {export}"));
            }
            Ok(JsValue::from_f64(value))
        }
        _ => Err(format!(
            "native/type-error: {export} expects :{}",
            hara_type_name(expected)
        )),
    }
}

fn lift_memory_result(
    export: &str,
    result_plan: &crate::wasm_binding::MemoryResultPlan,
    raw: JsValue,
    session: &BrowserMemorySession,
    release_always: &mut BTreeSet<i32>,
    total_copy_bytes: &mut usize,
) -> Result<Value, String> {
    if result_plan.lifting.is_none() {
        return scalar_result(export, &result_plan.hara_type, raw);
    }
    if result_plan.lifting != Some(Lifting::PackedI64) || !raw.is_bigint() {
        return Err(format!(
            "native/abi-type-unsupported: {export} expected packed i64"
        ));
    }
    let packed = u64::try_from(js_sys::BigInt::as_uint_n(
        64.0,
        &raw.unchecked_into::<js_sys::BigInt>(),
    ))
    .map_err(|_| format!("native/result-out-of-range: {export}"))?;
    let pointer_u32 = packed as u32;
    let length_u32 = (packed >> 32) as u32;
    let length = usize::try_from(length_u32).unwrap_or(usize::MAX);
    if length > MAX_VALUE_BYTES {
        return Err(format!(
            "native/resource-limit: {export} result exceeds the memory.v1 byte limit"
        ));
    }
    *total_copy_bytes = total_copy_bytes
        .checked_add(length)
        .ok_or_else(|| format!("native/resource-limit: {export} copy byte count overflow"))?;
    if *total_copy_bytes > MAX_TOTAL_COPY_BYTES {
        return Err(format!(
            "native/resource-limit: {export} exceeds the memory.v1 aggregate copy limit"
        ));
    }
    let pointer = i32::try_from(pointer_u32)
        .map_err(|_| format!("native/memory-range: {export} pointer is out of range"))?;
    if result_plan.ownership == Some(Ownership::Caller) && pointer != 0 {
        release_always.insert(pointer);
    }
    let start = checked_range(
        &session.memory,
        pointer,
        i32::try_from(length)
            .map_err(|_| format!("native/memory-range: {export} length is out of range"))?,
        export,
    )?;
    let mut bytes = vec![0; length];
    js_sys::Uint8Array::new(&session.memory.buffer())
        .subarray(start as u32, (start + length) as u32)
        .copy_to(&mut bytes);
    match result_plan.hara_type {
        HaraValueType::Bytes => Ok(Value::Bytes(bytes)),
        HaraValueType::String => String::from_utf8(bytes)
            .map(Value::String)
            .map_err(|error| format!("native/utf8-invalid: {export} ({error})")),
        _ => Err(format!(
            "native/abi-type-unsupported: {export} cannot lift :{}",
            hara_type_name(&result_plan.hara_type)
        )),
    }
}

fn scalar_result(export: &str, expected: &HaraValueType, raw: JsValue) -> Result<Value, String> {
    match expected {
        HaraValueType::Void if raw.is_undefined() => Ok(Value::Nil),
        HaraValueType::I32 => js_i32(&raw)
            .map(|value| Value::Number(i64::from(value)))
            .ok_or_else(|| format!("native/result-type-invalid: {export}")),
        HaraValueType::Boolean => js_i32(&raw)
            .map(|value| Value::Bool(value != 0))
            .ok_or_else(|| format!("native/result-type-invalid: {export}")),
        HaraValueType::I64 if raw.is_bigint() => {
            i64::try_from(raw.unchecked_into::<js_sys::BigInt>())
                .map(Value::Number)
                .map_err(|_| {
                    format!(
                        "native/integer-overflow: {export} result is outside signed 64-bit range"
                    )
                })
        }
        HaraValueType::F32 | HaraValueType::F64 => {
            let value = raw
                .as_f64()
                .ok_or_else(|| format!("native/result-type-invalid: {export}"))?;
            if !value.is_finite() {
                return Err(format!("non-finite number: {export}"));
            }
            Ok(Value::Float(value))
        }
        _ => Err(format!(
            "native/result-type-invalid: {export} -> :{}",
            hara_type_name(expected)
        )),
    }
}

fn js_i32(value: &JsValue) -> Option<i32> {
    let value = value.as_f64()?;
    (value.is_finite()
        && value.fract() == 0.0
        && value >= i32::MIN as f64
        && value <= i32::MAX as f64)
        .then_some(value as i32)
}

fn checked_range(
    memory: &js_sys::WebAssembly::Memory,
    pointer: i32,
    length: i32,
    export: &str,
) -> Result<usize, String> {
    let start = usize::try_from(pointer)
        .map_err(|_| format!("native/memory-range: {export} pointer is negative"))?;
    let length = usize::try_from(length)
        .map_err(|_| format!("native/memory-range: {export} length is negative"))?;
    let end = start
        .checked_add(length)
        .ok_or_else(|| format!("native/memory-range: {export} range overflow"))?;
    if end > memory_size(memory) {
        return Err(format!(
            "native/memory-range: {export} range {start}..{end} exceeds linear memory"
        ));
    }
    Ok(start)
}

fn release_memory(
    plan: &MemoryBindingPlan,
    session: &BrowserMemorySession,
    pointers: &BTreeSet<i32>,
) -> Result<(), String> {
    if pointers.is_empty() {
        return Ok(());
    }
    let release_name = plan
        .memory
        .release
        .as_deref()
        .ok_or("native/release-missing: cleanup requires a release export")?;
    let release = exported_function(&session.instance, release_name)?;
    let mut failures = Vec::new();
    for pointer in pointers {
        if let Err(error) = release.call1(&JsValue::UNDEFINED, &JsValue::from_f64(*pointer as f64))
        {
            failures.push(format!("{pointer}: {}", js_error(error)));
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!("native/release-failed: {}", failures.join("; ")))
    }
}

fn combine_memory_outcome(
    outcome: Result<Value, String>,
    cleanup: Result<(), String>,
) -> Result<Value, String> {
    match (outcome, cleanup) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(cleanup)) => Err(cleanup),
        (Err(error), Err(cleanup)) => Err(format!("{error}; cleanup: {cleanup}")),
    }
}

fn hara_type_name(value: &HaraValueType) -> &str {
    match value {
        HaraValueType::I32 => "i32",
        HaraValueType::I64 => "i64",
        HaraValueType::F32 => "f32",
        HaraValueType::F64 => "f64",
        HaraValueType::Boolean => "boolean",
        HaraValueType::String => "string",
        HaraValueType::Bytes => "bytes",
        HaraValueType::Record(_) => "record",
        HaraValueType::Variant(_) => "variant",
        HaraValueType::Handle(_) => "handle",
        HaraValueType::Callback(_) => "callback",
        HaraValueType::Void => "void",
    }
}

fn js_error(value: JsValue) -> String {
    value
        .as_string()
        .or_else(|| {
            js_sys::Reflect::get(&value, &JsValue::from_str("message"))
                .ok()
                .and_then(|value| value.as_string())
        })
        .unwrap_or_else(|| format!("{value:?}"))
}
