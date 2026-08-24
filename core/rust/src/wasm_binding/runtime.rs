#![cfg(not(target_arch = "wasm32"))]

use std::cell::RefCell;
use std::collections::BTreeSet;

use wasmtime::{
    Config, Engine, Instance, Memory, Module, Store, StoreLimits, StoreLimitsBuilder, Val,
};

use crate::core::Value;

use super::{
    inspect_direct, HaraValueType, Lifting, MemoryBindingPlan, MemoryContract, MemoryFunctionPlan,
    MemoryResultPlan, Ownership, WasmValueType,
};

#[cfg(test)]
mod tests;

const MAX_MEMORY_BYTES: usize = 64 * 1024 * 1024;
const MAX_VALUE_BYTES: usize = 16 * 1024 * 1024;
const MAX_TOTAL_INPUT_BYTES: usize = 32 * 1024 * 1024;
const MAX_TOTAL_COPY_BYTES: usize = MAX_TOTAL_INPUT_BYTES + MAX_VALUE_BYTES;
const INVOCATION_FUEL: u64 = 10_000_000;
const CLEANUP_FUEL: u64 = 1_000_000;

struct Session {
    store: Store<StoreLimits>,
    instance: Instance,
    memory: Memory,
}

/// Generic native executor for the closed `memory.v1` binding plan.
///
/// Library-specific behavior remains data in the binding plan. This executor
/// owns bounded allocation, copying, invocation, lifting, and release once for
/// every compatible Wasm library.
pub struct WasmtimeMemoryExecutor {
    plan: MemoryBindingPlan,
    session: RefCell<Session>,
}

impl WasmtimeMemoryExecutor {
    pub fn compile(bytes: &[u8], plan: MemoryBindingPlan) -> Result<Self, String> {
        let inspection = inspect_direct(bytes)?;
        plan.verify(&inspection)?;

        let mut config = Config::new();
        config.consume_fuel(true);
        let engine = Engine::new(&config)
            .map_err(|error| format!("extension/engine-unavailable: {error}"))?;
        let module = Module::new(&engine, bytes)
            .map_err(|error| format!("extension/module-invalid: {error}"))?;
        if module.imports().next().is_some() {
            return Err("extension/module-import-denied: memory.v1 must be import-free".into());
        }

        let limits = StoreLimitsBuilder::new()
            .memory_size(MAX_MEMORY_BYTES)
            .instances(1)
            .memories(1)
            .tables(1)
            .build();
        let mut store = Store::new(&engine, limits);
        store.limiter(|limits| limits);
        let instance = Instance::new(&mut store, &module, &[])
            .map_err(|error| format!("extension/module-invalid: {error}"))?;
        let memory = instance
            .get_memory(&mut store, &plan.memory.export)
            .ok_or_else(|| {
                format!(
                    "extension/memory-missing: module does not export {}",
                    plan.memory.export
                )
            })?;
        validate_instance(&plan, &mut store, &instance)?;

        Ok(Self {
            plan,
            session: RefCell::new(Session {
                store,
                instance,
                memory,
            }),
        })
    }

    pub fn plan(&self) -> &MemoryBindingPlan {
        &self.plan
    }

    pub fn invoke(&self, export: &str, arguments: &[Value]) -> Result<Value, String> {
        let function = self
            .plan
            .functions
            .iter()
            .find(|function| function.name == export)
            .cloned()
            .ok_or_else(|| format!("extension/export-missing: {export}"))?;
        if arguments.len() != function.arguments.len() {
            return Err(format!(
                "extension/arity: {export} expects {} arguments, got {}",
                function.arguments.len(),
                arguments.len()
            ));
        }

        let mut session = self.session.borrow_mut();
        session
            .store
            .set_fuel(INVOCATION_FUEL)
            .map_err(|error| format!("extension/execution-limit: {error}"))?;

        let mut release_always = BTreeSet::new();
        let mut release_on_failure = BTreeSet::new();
        let mut call_completed = false;
        let outcome = invoke_inner(
            &self.plan.memory,
            &function,
            arguments,
            &mut session,
            &mut release_always,
            &mut release_on_failure,
            &mut call_completed,
        );

        if !call_completed {
            release_always.extend(release_on_failure);
        }
        let cleanup = release_pointers(&self.plan.memory, &mut session, &release_always);
        combine_outcome(outcome, cleanup)
    }
}

fn validate_instance(
    plan: &MemoryBindingPlan,
    store: &mut Store<StoreLimits>,
    instance: &Instance,
) -> Result<(), String> {
    for function in &plan.functions {
        instance
            .get_func(&mut *store, &function.wasm_export)
            .ok_or_else(|| {
                format!(
                    "extension/export-missing: {} -> {}",
                    function.name, function.wasm_export
                )
            })?;
    }
    if let Some(name) = plan.memory.allocate.as_deref() {
        instance
            .get_typed_func::<i32, i32>(&mut *store, name)
            .map_err(|error| format!("extension/allocator-invalid: {name} ({error})"))?;
    }
    if let Some(name) = plan.memory.release.as_deref() {
        instance
            .get_typed_func::<i32, ()>(&mut *store, name)
            .map_err(|error| format!("extension/release-invalid: {name} ({error})"))?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn invoke_inner(
    memory_contract: &MemoryContract,
    function_plan: &MemoryFunctionPlan,
    arguments: &[Value],
    session: &mut Session,
    release_always: &mut BTreeSet<i32>,
    release_on_failure: &mut BTreeSet<i32>,
    call_completed: &mut bool,
) -> Result<Value, String> {
    let mut raw_arguments = Vec::with_capacity(function_plan.raw_arguments.len());
    let mut total_input_bytes = 0usize;
    let mut total_copy_bytes = 0usize;

    for (argument_plan, value) in function_plan.arguments.iter().zip(arguments) {
        if argument_plan.lowering.is_none() {
            raw_arguments.push(scalar_argument(
                &function_plan.name,
                &argument_plan.hara_type,
                value,
            )?);
            continue;
        }
        let bytes = memory_argument_bytes(&argument_plan.hara_type, value, &function_plan.name)?;
        total_input_bytes = total_input_bytes
            .checked_add(bytes.len())
            .ok_or_else(|| "extension/resource-limit: input byte count overflow".to_owned())?;
        if bytes.len() > MAX_VALUE_BYTES || total_input_bytes > MAX_TOTAL_INPUT_BYTES {
            return Err(format!(
                "extension/resource-limit: {} input exceeds the memory.v1 byte limit",
                function_plan.name
            ));
        }
        let (pointer, length) = lower_pointer_length(
            memory_contract,
            session,
            bytes,
            &function_plan.name,
            argument_plan.ownership,
            release_on_failure,
        )?;
        total_copy_bytes = total_copy_bytes
            .checked_add(bytes.len())
            .ok_or_else(|| "extension/resource-limit: copy byte count overflow".to_owned())?;
        if total_copy_bytes > MAX_TOTAL_COPY_BYTES {
            return Err(format!(
                "extension/resource-limit: {} exceeds the memory.v1 aggregate copy limit",
                function_plan.name
            ));
        }
        raw_arguments.push(Val::I32(pointer));
        raw_arguments.push(Val::I32(length));
    }

    let function = session
        .instance
        .get_func(&mut session.store, &function_plan.wasm_export)
        .ok_or_else(|| {
            format!(
                "extension/export-missing: {} -> {}",
                function_plan.name, function_plan.wasm_export
            )
        })?;
    let mut raw_results = if function_plan.raw_returns == WasmValueType::Void {
        Vec::new()
    } else {
        vec![default_result(function_plan.raw_returns)]
    };
    function
        .call(&mut session.store, &raw_arguments, &mut raw_results)
        .map_err(|error| format!("extension/invoke-failed: {} ({error})", function_plan.name))?;
    *call_completed = true;

    lift_result(
        &function_plan.name,
        &function_plan.returns,
        raw_results.into_iter().next(),
        session,
        release_always,
        &mut total_copy_bytes,
    )
}

fn memory_argument_bytes<'a>(
    expected: &HaraValueType,
    value: &'a Value,
    export: &str,
) -> Result<&'a [u8], String> {
    match (expected, value) {
        (HaraValueType::Bytes, Value::Bytes(bytes)) => Ok(bytes),
        (HaraValueType::String, Value::String(value)) => Ok(value.as_bytes()),
        _ => Err(format!(
            "extension/type-error: {export} expects :{}",
            hara_type_name(expected)
        )),
    }
}

fn lower_pointer_length(
    contract: &MemoryContract,
    session: &mut Session,
    bytes: &[u8],
    export: &str,
    ownership: Option<Ownership>,
    release_on_failure: &mut BTreeSet<i32>,
) -> Result<(i32, i32), String> {
    let length = i32::try_from(bytes.len())
        .map_err(|_| format!("extension/resource-limit: {export} input is too large"))?;
    if length == 0 {
        return Ok((0, 0));
    }
    let allocator_name = contract
        .allocate
        .as_deref()
        .ok_or_else(|| format!("extension/allocator-missing: {export} requires an allocator"))?;
    let allocator = session
        .instance
        .get_typed_func::<i32, i32>(&mut session.store, allocator_name)
        .map_err(|error| format!("extension/allocator-invalid: {allocator_name} ({error})"))?;
    let pointer = allocator
        .call(&mut session.store, length)
        .map_err(|error| format!("extension/allocator-failed: {export} ({error})"))?;
    if pointer != 0 && ownership == Some(Ownership::Transferred) {
        release_on_failure.insert(pointer);
    }
    let start = checked_range(&session.memory, &session.store, pointer, length, export)?;
    session
        .memory
        .write(&mut session.store, start, bytes)
        .map_err(|error| format!("extension/memory-write-failed: {export} ({error})"))?;
    Ok((pointer, length))
}

fn lift_result(
    export: &str,
    plan: &MemoryResultPlan,
    raw: Option<Val>,
    session: &mut Session,
    release_always: &mut BTreeSet<i32>,
    total_copy_bytes: &mut usize,
) -> Result<Value, String> {
    if plan.lifting.is_none() {
        return scalar_result(export, &plan.hara_type, raw);
    }
    if plan.lifting != Some(Lifting::PackedI64) {
        return Err(format!(
            "extension/abi-type-unsupported: {export} result lifting"
        ));
    }
    let Some(Val::I64(raw)) = raw else {
        return Err(format!(
            "extension/abi-type-unsupported: {export} expected packed i64"
        ));
    };

    // memory.v1 packs the pointer in the low 32 bits and byte length in the
    // high 32 bits. Both fields are interpreted as unsigned values.
    let packed = raw as u64;
    let pointer_u32 = packed as u32;
    let length_u32 = (packed >> 32) as u32;
    let pointer = i32::try_from(pointer_u32)
        .map_err(|_| format!("extension/memory-range: {export} pointer is out of range"))?;
    let length = i32::try_from(length_u32)
        .map_err(|_| format!("extension/resource-limit: {export} result is too large"))?;
    let length_usize = usize::try_from(length)
        .map_err(|_| format!("extension/memory-range: {export} result length is negative"))?;
    if length_usize > MAX_VALUE_BYTES {
        return Err(format!(
            "extension/resource-limit: {export} result exceeds the memory.v1 byte limit"
        ));
    }
    *total_copy_bytes = total_copy_bytes
        .checked_add(length_usize)
        .ok_or_else(|| format!("extension/resource-limit: {export} copy byte count overflow"))?;
    if *total_copy_bytes > MAX_TOTAL_COPY_BYTES {
        return Err(format!(
            "extension/resource-limit: {export} exceeds the memory.v1 aggregate copy limit"
        ));
    }
    if plan.ownership == Some(Ownership::Caller) && pointer != 0 {
        release_always.insert(pointer);
    }
    let start = checked_range(&session.memory, &session.store, pointer, length, export)?;
    let mut bytes = vec![0u8; length_usize];
    session
        .memory
        .read(&session.store, start, &mut bytes)
        .map_err(|error| format!("extension/memory-read-failed: {export} ({error})"))?;
    match plan.hara_type {
        HaraValueType::Bytes => Ok(Value::Bytes(bytes)),
        HaraValueType::String => String::from_utf8(bytes)
            .map(Value::String)
            .map_err(|error| format!("extension/utf8-invalid: {export} ({error})")),
        _ => Err(format!(
            "extension/abi-type-unsupported: {export} cannot lift :{}",
            hara_type_name(&plan.hara_type)
        )),
    }
}

fn checked_range(
    memory: &Memory,
    store: &Store<StoreLimits>,
    pointer: i32,
    length: i32,
    export: &str,
) -> Result<usize, String> {
    let start = usize::try_from(pointer)
        .map_err(|_| format!("extension/memory-range: {export} pointer is negative"))?;
    let length = usize::try_from(length)
        .map_err(|_| format!("extension/memory-range: {export} length is negative"))?;
    let end = start
        .checked_add(length)
        .ok_or_else(|| format!("extension/memory-range: {export} range overflow"))?;
    if end > memory.data_size(store) {
        return Err(format!(
            "extension/memory-range: {export} range {start}..{end} exceeds linear memory"
        ));
    }
    Ok(start)
}

fn release_pointers(
    contract: &MemoryContract,
    session: &mut Session,
    pointers: &BTreeSet<i32>,
) -> Result<(), String> {
    if pointers.is_empty() {
        return Ok(());
    }
    let release_name = contract
        .release
        .as_deref()
        .ok_or_else(|| "extension/release-missing: cleanup requires a release export".to_owned())?;
    let release = session
        .instance
        .get_typed_func::<i32, ()>(&mut session.store, release_name)
        .map_err(|error| format!("extension/release-invalid: {release_name} ({error})"))?;
    session
        .store
        .set_fuel(CLEANUP_FUEL)
        .map_err(|error| format!("extension/release-failed: fuel ({error})"))?;

    let mut failures = Vec::new();
    for pointer in pointers {
        if let Err(error) = release.call(&mut session.store, *pointer) {
            failures.push(format!("{pointer}: {error}"));
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!("extension/release-failed: {}", failures.join("; ")))
    }
}

fn combine_outcome(
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

fn scalar_argument(export: &str, expected: &HaraValueType, value: &Value) -> Result<Val, String> {
    fn finite_f32(value: f64) -> Result<f32, String> {
        let value = value as f32;
        if value.is_finite() {
            Ok(value)
        } else {
            Err("non-finite number".into())
        }
    }
    let type_error = || {
        format!(
            "extension/type-error: {export} expects :{}",
            hara_type_name(expected)
        )
    };
    match (expected, value) {
        (HaraValueType::I32, Value::Number(value)) => i32::try_from(*value)
            .map(Val::I32)
            .map_err(|_| type_error()),
        (HaraValueType::Boolean, Value::Bool(value)) => Ok(Val::I32(i32::from(*value))),
        (HaraValueType::I64, Value::Number(value)) => Ok(Val::I64(*value)),
        (HaraValueType::F32, Value::Float(value)) => {
            Ok(Val::F32(finite_f32(*value)?.to_bits()))
        }
        (HaraValueType::F32, Value::Number(value)) => {
            Ok(Val::F32(finite_f32(*value as f64)?.to_bits()))
        }
        (HaraValueType::F64, Value::Float(value)) => {
            Ok(Val::F64(crate::numeric::finite_float(*value)?.to_bits()))
        }
        (HaraValueType::F64, Value::Number(value)) => Ok(Val::F64((*value as f64).to_bits())),
        _ => Err(type_error()),
    }
}

fn scalar_result(
    export: &str,
    expected: &HaraValueType,
    raw: Option<Val>,
) -> Result<Value, String> {
    match (expected, raw) {
        (HaraValueType::Void, None) => Ok(Value::Nil),
        (HaraValueType::I32, Some(Val::I32(value))) => Ok(Value::Number(i64::from(value))),
        (HaraValueType::Boolean, Some(Val::I32(value))) => Ok(Value::Bool(value != 0)),
        (HaraValueType::I64, Some(Val::I64(value))) => Ok(Value::Number(value)),
        (HaraValueType::F32, Some(Val::F32(value))) => {
            Ok(Value::Float(crate::numeric::finite_float(
                f32::from_bits(value) as f64,
            )?))
        }
        (HaraValueType::F64, Some(Val::F64(value))) => {
            Ok(Value::Float(crate::numeric::finite_float(f64::from_bits(value))?))
        }
        _ => Err(format!(
            "extension/abi-type-unsupported: {export} -> :{}",
            hara_type_name(expected)
        )),
    }
}

fn default_result(raw: WasmValueType) -> Val {
    match raw {
        WasmValueType::I32 => Val::I32(0),
        WasmValueType::I64 => Val::I64(0),
        WasmValueType::F32 => Val::F32(0),
        WasmValueType::F64 => Val::F64(0),
        WasmValueType::Void => unreachable!("void functions do not allocate a result slot"),
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
