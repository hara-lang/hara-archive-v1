use std::cell::RefCell;
use std::rc::Rc;

use wasmtime::{Engine, FuncType, Instance, Linker, Module, Store, Val, ValType};

use crate::core::Value;
use crate::instrumentation::{
    EventAccess, EventKind, InstrumentationHub, ProducerEvent, TargetHandle,
};
use crate::vm::{FunctionId, Machine, VmOutcome};

use super::artifact::{decode_artifact, NativeArtifact};
use super::bridge::{
    self, Slot, TargetKind, RESULT_BOOL, RESULT_HANDLE, RESULT_I64, SLOT_BOOL, SLOT_CONSTANT,
    SLOT_HANDLE, SLOT_I64, SLOT_NIL,
};
use super::codegen::{
    ERROR_ARRAY_BOUNDS, ERROR_DIVISION_BY_ZERO, ERROR_INTEGER_OVERFLOW, ERROR_OBJECT_KEY,
};
use super::handles::{Handle, HandleScope};

#[derive(Default)]
struct HostState {
    handles: HandleScope,
    constants: Vec<Value>,
    targets: Vec<bridge::TargetDescriptor>,
    error_code: i32,
    instrumentation: Option<BridgeInstrumentation>,
}

#[derive(Clone)]
struct BridgeInstrumentation {
    hub: Rc<RefCell<InstrumentationHub>>,
    target: TargetHandle,
}

/// A validated HNW0 module instantiated by Wasmtime. Calls enter a generated
/// whole Wasm function directly; the bytecode program is retained as fallback
/// metadata, not interpreted on this path.
pub struct NativeModule {
    artifact: NativeArtifact,
    store: Store<HostState>,
    instance: Instance,
    heap_base: i32,
}

impl NativeModule {
    pub fn load(bytes: &[u8]) -> Result<Self, String> {
        Self::load_internal(bytes, None)
    }

    pub(crate) fn load_with_instrumentation(
        bytes: &[u8],
        hub: Rc<RefCell<InstrumentationHub>>,
        target: TargetHandle,
    ) -> Result<Self, String> {
        Self::load_internal(bytes, Some(BridgeInstrumentation { hub, target }))
    }

    fn load_internal(
        bytes: &[u8],
        instrumentation: Option<BridgeInstrumentation>,
    ) -> Result<Self, String> {
        let artifact = decode_artifact(bytes)?;
        let engine = Engine::default();
        let module = Module::new(&engine, &artifact.wasm).map_err(|error| error.to_string())?;
        let mut store = Store::new(
            &engine,
            HostState {
                handles: HandleScope::default(),
                constants: artifact.program.constants.clone(),
                targets: artifact.targets.clone(),
                error_code: 0,
                instrumentation,
            },
        );
        let mut linker = Linker::new(&engine);
        define_persistent_imports(&mut linker)?;
        let instance = linker
            .instantiate(&mut store, &module)
            .map_err(|error| error.to_string())?;
        let heap_base = instance
            .get_global(&mut store, "hara_heap")
            .and_then(|global| global.get(&mut store).i32())
            .ok_or("whole-Wasm module has no valid hara_heap global")?;
        Ok(Self {
            artifact,
            store,
            instance,
            heap_base,
        })
    }

    pub fn artifact(&self) -> &NativeArtifact {
        &self.artifact
    }

    pub(crate) fn emit_terminal(&mut self, status: &str) -> Result<(), String> {
        let Some(instrumentation) = self.store.data().instrumentation.clone() else {
            return Ok(());
        };
        let event = ProducerEvent::live(EventKind::ExecutionTerminal).with_data("status", status);
        let result = instrumentation
            .hub
            .borrow_mut()
            .emit(&instrumentation.target, event, &mut BridgeEventAccess)
            .map(|_| ())
            .map_err(|error| error.to_string());
        result
    }

    /// Calls a whole-Wasm function whose arguments and result use the dynamic
    /// Hara value-handle ABI. Values remain owned by this prepared call and
    /// cross the Wasm boundary without serialisation.
    pub fn call_value(
        &mut self,
        function: FunctionId,
        arguments: &[Value],
    ) -> Result<Value, String> {
        self.store.data_mut().handles.begin_call();
        if !self
            .artifact
            .capabilities
            .get(usize::from(function))
            .copied()
            .unwrap_or(false)
        {
            return self.execute_bytecode_function(function, arguments.to_vec());
        }
        let mut encoded = Vec::with_capacity(arguments.len());
        for argument in arguments {
            encoded.push(
                self.store
                    .data_mut()
                    .handles
                    .insert(argument.clone())?
                    .to_abi(),
            );
        }
        match self.call_prepared_i64(function, &encoded) {
            Ok(result) => self.store.data().handles.get(Handle::from_abi(result)),
            Err(error) if should_fallback_to_bytecode(&error) => {
                self.execute_bytecode_function(function, arguments.to_vec())
            }
            Err(error) => Err(error),
        }
    }

    /// Calls the zero-arity entry through the dynamic Hara value-handle ABI.
    pub fn call_entry_value(&mut self) -> Result<Value, String> {
        let entry = self.artifact.program.entry;
        self.call_value(entry, &[])
    }

    pub fn call_i64(&mut self, function: FunctionId, arguments: &[i64]) -> Result<i64, String> {
        self.store.data_mut().handles.begin_call();
        if !self
            .artifact
            .capabilities
            .get(usize::from(function))
            .copied()
            .unwrap_or(false)
        {
            let values = arguments.iter().copied().map(Value::Number).collect();
            return match self.execute_bytecode_function(function, values)? {
                Value::Number(value) => Ok(value),
                _ => Err("whole-Wasm result is not an i64".into()),
            };
        }
        match self.call_prepared_i64(function, arguments) {
            Ok(result) => Ok(result),
            Err(error) if should_fallback_to_bytecode(&error) => {
                let values = arguments.iter().copied().map(Value::Number).collect();
                match self.execute_bytecode_function(function, values)? {
                    Value::Number(value) => Ok(value),
                    _ => Err("whole-Wasm result is not an i64".into()),
                }
            }
            Err(error) => Err(error),
        }
    }

    fn call_prepared_i64(
        &mut self,
        function: FunctionId,
        arguments: &[i64],
    ) -> Result<i64, String> {
        self.store.data_mut().error_code = 0;
        let (_, arity) = self
            .artifact
            .functions
            .get(usize::from(function))
            .ok_or_else(|| format!("unknown whole-Wasm function {function}"))?;
        if arguments.len() != usize::from(*arity) {
            return Err(format!(
                "whole-Wasm function {function} expects {arity} arguments, got {}",
                arguments.len()
            ));
        }
        let error = self
            .instance
            .get_global(&mut self.store, "hara_error")
            .ok_or("whole-Wasm module has no hara_error global")?;
        error
            .set(&mut self.store, Val::I32(0))
            .map_err(|error| error.to_string())?;
        self.instance
            .get_global(&mut self.store, "hara_heap")
            .ok_or("whole-Wasm module has no hara_heap global")?
            .set(&mut self.store, Val::I32(self.heap_base))
            .map_err(|error| error.to_string())?;
        let callable = self
            .instance
            .get_func(&mut self.store, &format!("hara_fn_{function}"))
            .ok_or_else(|| format!("whole-Wasm module has no function {function}"))?;
        let inputs = arguments.iter().copied().map(Val::I64).collect::<Vec<_>>();
        let mut outputs = [Val::I64(0)];
        match callable.call(&mut self.store, &inputs, &mut outputs) {
            Ok(()) => outputs[0]
                .i64()
                .ok_or_else(|| "whole-Wasm function returned a non-i64 result".into()),
            Err(trap) => {
                let code = error
                    .get(&mut self.store)
                    .i32()
                    .unwrap_or_default()
                    .max(self.store.data().error_code);
                match code {
                    ERROR_INTEGER_OVERFLOW => Err("integer overflow".into()),
                    ERROR_DIVISION_BY_ZERO => Err("division by zero".into()),
                    ERROR_ARRAY_BOUNDS => Err("array index out of bounds".into()),
                    ERROR_OBJECT_KEY => Err("object key not found".into()),
                    _ => Err(format!("whole-Wasm trap: {trap:#}")),
                }
            }
        }
    }

    /// Calls the zero-arity entry through the initial scalar ABI. Returning a
    /// raw i64 is intentional: MIR result-representation metadata must exist
    /// before this boundary can faithfully construct a dynamic Hara `Value`.
    pub fn call_entry_i64(&mut self) -> Result<i64, String> {
        let entry = self.artifact.program.entry;
        self.call_i64(entry, &[])
    }

    fn execute_bytecode_function(
        &self,
        function: FunctionId,
        arguments: Vec<Value>,
    ) -> Result<Value, String> {
        let registry = crate::bytecode_namespace_registry();
        let mut machine = Machine::call(
            Rc::new(self.artifact.program.clone()),
            function,
            arguments,
            Vec::new(),
        );
        crate::core::with_namespace_registry(&registry, || match machine.run() {
            VmOutcome::Returned(value) => Ok(value),
            VmOutcome::Failed(error) => Err(error.to_string()),
            VmOutcome::Suspended(_) => Err("VM fiber suspended on an unresolved promise".into()),
            VmOutcome::Yielded(_) => Err("coroutine/yield used outside of a coroutine".into()),
        })
    }
}

fn should_fallback_to_bytecode(error: &str) -> bool {
    error == "integer overflow" || error.contains("whole-Wasm value is not an integer")
}

fn define_target_import(linker: &mut Linker<HostState>) -> Result<(), String> {
    linker
        .func_new(
            "hara",
            "target_call",
            FuncType::new(
                [ValType::I64, ValType::I64, ValType::I64, ValType::I64],
                [ValType::I64],
            ),
            |mut caller, inputs, outputs| {
                let target = inputs[0].i64().unwrap();
                let pointer = inputs[1].i64().unwrap();
                let argc = inputs[2].i64().unwrap();
                let result_mode = inputs[3].i64().unwrap();
                let slots = read_bridge_slots(&mut caller, pointer, argc).map_err(host_error)?;
                let arguments = slots
                    .iter()
                    .map(|slot| resolve_bridge_slot(&caller, *slot))
                    .collect::<Result<Vec<_>, _>>()?;
                let descriptor = usize::try_from(target)
                    .ok()
                    .and_then(|index| caller.data().targets.get(index))
                    .cloned()
                    .ok_or_else(|| host_error(format!("unknown whole-Wasm target {target}")))?;
                bridge::validate_target_call(&descriptor, arguments.len(), result_mode)
                    .map_err(host_error)?;
                let name = descriptor.symbol.as_str();
                let instrumentation = caller.data().instrumentation.clone();
                emit_bridge_event(
                    instrumentation.as_ref(),
                    name,
                    arguments.len(),
                    result_mode,
                    "enter",
                )
                .map_err(host_error)?;
                let value = match match descriptor.kind {
                    TargetKind::Protocol => {
                        crate::core::protocol_intrinsic_call(name, &arguments).map_err(host_error)
                    }
                    TargetKind::Native => {
                        crate::core::apply_intrinsic_name(name, &arguments).map_err(host_error)
                    }
                    TargetKind::VectorConstruct | TargetKind::MapConstruct => Err(host_error(
                        format!("whole-Wasm target is not callable: {name}"),
                    )),
                } {
                    Ok(value) => value,
                    Err(error) => {
                        let _ = emit_bridge_event(
                            instrumentation.as_ref(),
                            name,
                            arguments.len(),
                            result_mode,
                            "error",
                        );
                        return Err(error);
                    }
                };
                let encoded = match encode_bridge_result(&mut caller, value, result_mode) {
                    Ok(encoded) => encoded,
                    Err(error) => {
                        let _ = emit_bridge_event(
                            instrumentation.as_ref(),
                            name,
                            arguments.len(),
                            result_mode,
                            "error",
                        );
                        return Err(error);
                    }
                };
                emit_bridge_event(
                    instrumentation.as_ref(),
                    name,
                    arguments.len(),
                    result_mode,
                    "return",
                )
                .map_err(host_error)?;
                outputs[0] = Val::I64(encoded);
                Ok(())
            },
        )
        .map_err(|error| error.to_string())
        .map(|_| ())
}

struct BridgeEventAccess;

impl EventAccess for BridgeEventAccess {}

fn emit_bridge_event(
    instrumentation: Option<&BridgeInstrumentation>,
    target: &str,
    arity: usize,
    result_mode: i64,
    status: &str,
) -> Result<(), String> {
    let Some(instrumentation) = instrumentation else {
        return Ok(());
    };
    let result_mode = bridge::result_mode_name(result_mode)
        .ok_or_else(|| format!("unknown whole-Wasm result mode {result_mode}"))?;
    let event = ProducerEvent::live(EventKind::ProtocolCall)
        .with_data("target", target)
        .with_data("arity", arity.to_string())
        .with_data("result-mode", result_mode)
        .with_data("status", status);
    instrumentation
        .hub
        .borrow_mut()
        .emit(&instrumentation.target, event, &mut BridgeEventAccess)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn define_value_construct_import(linker: &mut Linker<HostState>) -> Result<(), String> {
    linker
        .func_new(
            "hara",
            "value_construct",
            FuncType::new([ValType::I64, ValType::I64, ValType::I64], [ValType::I64]),
            |mut caller, inputs, outputs| {
                let target = inputs[0].i64().unwrap();
                let pointer = inputs[1].i64().unwrap();
                let argc = inputs[2].i64().unwrap();
                let slots = read_bridge_slots(&mut caller, pointer, argc).map_err(host_error)?;
                let values = slots
                    .iter()
                    .map(|slot| resolve_bridge_slot(&caller, *slot))
                    .collect::<Result<Vec<_>, _>>()?;
                let descriptor = usize::try_from(target)
                    .ok()
                    .and_then(|index| caller.data().targets.get(index))
                    .cloned()
                    .ok_or_else(|| {
                        host_error(format!("unknown whole-Wasm structural target {target}"))
                    })?;
                bridge::validate_value_construct(&descriptor, values.len()).map_err(host_error)?;
                let value = match descriptor.kind {
                    TargetKind::VectorConstruct => {
                        Value::Vector(crate::lang::data::Vector::from_iter(values))
                    }
                    TargetKind::MapConstruct => {
                        crate::core::vm_build_map(values.into_iter().collect::<Vec<_>>())
                            .map_err(host_error)?
                    }
                    TargetKind::Protocol | TargetKind::Native => {
                        return Err(host_error(format!(
                            "unknown whole-Wasm structural target {target}"
                        )))
                    }
                };
                outputs[0] = Val::I64(
                    caller
                        .data_mut()
                        .handles
                        .insert(value)
                        .map(Handle::to_abi)
                        .map_err(host_error)?,
                );
                Ok(())
            },
        )
        .map_err(|error| error.to_string())
        .map(|_| ())
}

fn read_bridge_slots(
    caller: &mut wasmtime::Caller<'_, HostState>,
    pointer: i64,
    argc: i64,
) -> Result<Vec<Slot>, String> {
    let pointer = usize::try_from(pointer).map_err(|_| "whole-Wasm bridge pointer is invalid")?;
    let argc = usize::try_from(argc).map_err(|_| "whole-Wasm bridge arity is invalid")?;
    if argc > usize::try_from(bridge::MAX_SLOTS).expect("constant fits usize") {
        return Err("whole-Wasm bridge arity exceeds its bound".into());
    }
    let bytes = argc
        .checked_mul(usize::try_from(bridge::SLOT_BYTES).expect("constant fits usize"))
        .and_then(|size| pointer.checked_add(size))
        .ok_or("whole-Wasm bridge memory range overflow")?;
    let memory = caller
        .get_export("hara_memory")
        .and_then(|export| export.into_memory())
        .ok_or("whole-Wasm module has no hara_memory export")?;
    let data = memory.data(caller);
    if bytes > data.len() {
        return Err("whole-Wasm bridge memory range is out of bounds".into());
    }
    let mut slots = Vec::with_capacity(argc);
    for index in 0..argc {
        let offset = pointer + index * usize::try_from(bridge::SLOT_BYTES).unwrap();
        let kind = u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap());
        let payload = i64::from_le_bytes(data[offset + 8..offset + 16].try_into().unwrap());
        slots.push(Slot { kind, payload });
    }
    bridge::validate_slots(&slots)?;
    Ok(slots)
}

fn resolve_bridge_slot(
    caller: &wasmtime::Caller<'_, HostState>,
    slot: Slot,
) -> Result<Value, wasmtime::Error> {
    match slot.kind {
        SLOT_HANDLE => caller
            .data()
            .handles
            .get(Handle::from_abi(slot.payload))
            .map_err(host_error),
        SLOT_I64 => Ok(Value::Number(slot.payload)),
        SLOT_BOOL => Ok(Value::Bool(slot.payload != 0)),
        SLOT_NIL => Ok(Value::Nil),
        SLOT_CONSTANT => caller
            .data()
            .constants
            .get(usize::try_from(slot.payload).map_err(|_| host_error("invalid constant".into()))?)
            .cloned()
            .ok_or_else(|| host_error("whole-Wasm constant index out of range".into())),
        _ => Err(host_error("invalid whole-Wasm bridge slot".into())),
    }
}

fn encode_bridge_result(
    caller: &mut wasmtime::Caller<'_, HostState>,
    value: Value,
    mode: i64,
) -> Result<i64, wasmtime::Error> {
    bridge::validate_result_mode(mode).map_err(host_error)?;
    match mode {
        RESULT_HANDLE => caller
            .data_mut()
            .handles
            .insert(value)
            .map(Handle::to_abi)
            .map_err(host_error),
        RESULT_I64 => crate::numeric::to_i64_integer(&value).map_err(|_| {
                caller.data_mut().error_code = ERROR_INTEGER_OVERFLOW;
                host_error("integer overflow".into())
            }),
        RESULT_BOOL => match value {
            Value::Bool(value) => Ok(i64::from(value)),
            _ => Err(host_error(
                "whole-Wasm target did not return a boolean".into(),
            )),
        },
        _ => unreachable!("result mode validated above"),
    }
}

fn define_persistent_imports(linker: &mut Linker<HostState>) -> Result<(), String> {
    define_target_import(linker)?;
    define_value_construct_import(linker)?;
    linker
        .func_wrap(
            "hara",
            "constant_handle",
            |mut caller: wasmtime::Caller<'_, HostState>, index: i64| {
                let value = caller
                    .data()
                    .constants
                    .get(
                        usize::try_from(index)
                            .map_err(|_| host_error("invalid constant".into()))?,
                    )
                    .cloned()
                    .ok_or_else(|| host_error("constant index out of range".into()))?;
                caller
                    .data_mut()
                    .handles
                    .insert(value)
                    .map(Handle::to_abi)
                    .map_err(host_error)
            },
        )
        .map_err(|error| error.to_string())?;
    linker
        .func_wrap(
            "hara",
            "box_i64",
            |mut caller: wasmtime::Caller<'_, HostState>, value: i64| {
                caller
                    .data_mut()
                    .handles
                    .insert(Value::Number(value))
                    .map(Handle::to_abi)
                    .map_err(host_error)
            },
        )
        .map_err(|error| error.to_string())?;
    linker
        .func_wrap(
            "hara",
            "unbox_i64",
            |mut caller: wasmtime::Caller<'_, HostState>, handle: i64| {
                let value = caller.data().handles.get(Handle::from_abi(handle));
                match value {
                    Ok(value) => crate::numeric::to_i64_integer(&value).map_err(|_| {
                        caller.data_mut().error_code = ERROR_INTEGER_OVERFLOW;
                        host_error("integer overflow".into())
                    }),
                    Err(error) => Err(host_error(error)),
                }
            },
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn host_error(message: String) -> wasmtime::Error {
    wasmtime::Error::msg(message)
}
