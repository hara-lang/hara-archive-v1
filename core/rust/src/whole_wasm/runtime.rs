use std::cell::RefCell;
use std::rc::Rc;

use wasmtime::{Engine, FuncType, Instance, Linker, Module, Store, Val, ValType};

use crate::core::Value;
use crate::vm::FunctionId;

use super::artifact::{decode_artifact, NativeArtifact};
use super::codegen::{
    ERROR_ARRAY_BOUNDS, ERROR_DIVISION_BY_ZERO, ERROR_INTEGER_OVERFLOW, ERROR_OBJECT_KEY,
};
use super::handles::{Handle, HandleScope};

#[derive(Default)]
struct HostState {
    handles: HandleScope,
    constants: Vec<Value>,
    error_code: i32,
}

/// A validated HNW0 module instantiated by Wasmtime. Calls enter a generated
/// whole Wasm function directly; the bytecode program is retained as fallback
/// metadata, not interpreted on this path.
pub struct NativeModule {
    artifact: NativeArtifact,
    store: Store<HostState>,
    instance: Instance,
}

impl NativeModule {
    pub fn load(bytes: &[u8]) -> Result<Self, String> {
        let artifact = decode_artifact(bytes)?;
        let engine = Engine::default();
        let module = Module::new(&engine, &artifact.wasm).map_err(|error| error.to_string())?;
        let mut store = Store::new(
            &engine,
            HostState {
                handles: HandleScope::default(),
                constants: artifact.program.constants.clone(),
                error_code: 0,
            },
        );
        let mut linker = Linker::new(&engine);
        define_array_imports(&mut linker)?;
        define_persistent_imports(&mut linker)?;
        let instance = linker
            .instantiate(&mut store, &module)
            .map_err(|error| error.to_string())?;
        Ok(Self {
            artifact,
            store,
            instance,
        })
    }

    pub fn artifact(&self) -> &NativeArtifact {
        &self.artifact
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
        let result = self.call_prepared_i64(function, &encoded)?;
        self.store.data().handles.get(Handle::from_abi(result))
    }

    /// Calls the zero-arity entry through the dynamic Hara value-handle ABI.
    pub fn call_entry_value(&mut self) -> Result<Value, String> {
        let entry = self.artifact.program.entry;
        self.call_value(entry, &[])
    }

    pub fn call_i64(&mut self, function: FunctionId, arguments: &[i64]) -> Result<i64, String> {
        self.store.data_mut().handles.begin_call();
        self.call_prepared_i64(function, arguments)
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
            .set(&mut self.store, Val::I32(0))
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
}

fn define_array_imports(linker: &mut Linker<HostState>) -> Result<(), String> {
    linker
        .func_new(
            "hara",
            "array_empty",
            FuncType::new([], [ValType::I64]),
            |mut caller, _, outputs| {
                let value = Value::Array(Rc::new(RefCell::new(Vec::new())));
                outputs[0] = Val::I64(
                    caller
                        .data_mut()
                        .handles
                        .insert(value)
                        .map_err(host_error)?
                        .to_abi(),
                );
                Ok(())
            },
        )
        .map_err(|error| error.to_string())?;
    linker
        .func_new(
            "hara",
            "array_push_i64",
            FuncType::new([ValType::I64, ValType::I64], [ValType::I64]),
            |caller, inputs, outputs| {
                let handle = Handle::from_abi(inputs[0].i64().unwrap());
                let value = inputs[1].i64().unwrap();
                match caller.data().handles.get(handle).map_err(host_error)? {
                    Value::Array(values) => values.borrow_mut().push(Value::Number(value)),
                    _ => return Err(host_error("whole-Wasm array handle expected".into())),
                }
                outputs[0] = Val::I64(handle.to_abi());
                Ok(())
            },
        )
        .map_err(|error| error.to_string())?;
    linker
        .func_new(
            "hara",
            "array_get_i64",
            FuncType::new([ValType::I64, ValType::I64], [ValType::I64]),
            |caller, inputs, outputs| {
                let handle = Handle::from_abi(inputs[0].i64().unwrap());
                let index = array_index(inputs[1].i64().unwrap())?;
                let result = match caller.data().handles.get(handle).map_err(host_error)? {
                    Value::Array(values) => values
                        .borrow()
                        .get(index)
                        .cloned()
                        .ok_or_else(|| host_error("array/get index out of bounds".into()))?,
                    _ => return Err(host_error("whole-Wasm array handle expected".into())),
                };
                let Value::Number(result) = result else {
                    return Err(host_error(
                        "whole-Wasm array element is not an integer".into(),
                    ));
                };
                outputs[0] = Val::I64(result);
                Ok(())
            },
        )
        .map_err(|error| error.to_string())?;
    linker
        .func_new(
            "hara",
            "array_set_i64",
            FuncType::new([ValType::I64, ValType::I64, ValType::I64], [ValType::I64]),
            |caller, inputs, outputs| {
                let handle = Handle::from_abi(inputs[0].i64().unwrap());
                let index = array_index(inputs[1].i64().unwrap())?;
                let value = inputs[2].i64().unwrap();
                match caller.data().handles.get(handle).map_err(host_error)? {
                    Value::Array(values) => {
                        let mut values = values.borrow_mut();
                        let slot = values
                            .get_mut(index)
                            .ok_or_else(|| host_error("array/set index out of bounds".into()))?;
                        *slot = Value::Number(value);
                    }
                    _ => return Err(host_error("whole-Wasm array handle expected".into())),
                }
                outputs[0] = Val::I64(handle.to_abi());
                Ok(())
            },
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn define_persistent_imports(linker: &mut Linker<HostState>) -> Result<(), String> {
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
                    Ok(Value::Number(value)) => Ok(value),
                    Ok(Value::BigInteger(value)) => {
                        let value = Value::BigInteger(value);
                        match crate::numeric::to_i64_exact(&value) {
                            Ok(value) => Ok(value),
                            Err(_) => {
                                caller.data_mut().error_code = ERROR_INTEGER_OVERFLOW;
                                Err(host_error("integer overflow".into()))
                            }
                        }
                    }
                    Ok(_) => Err(host_error("whole-Wasm value is not an integer".into())),
                    Err(error) => Err(host_error(error)),
                }
            },
        )
        .map_err(|error| error.to_string())?;
    linker
        .func_wrap(
            "hara",
            "vector_empty",
            |mut caller: wasmtime::Caller<'_, HostState>| {
                let value = Value::Vector(crate::lang::data::Vector::new());
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
            "vector_push",
            |mut caller: wasmtime::Caller<'_, HostState>, vector: i64, item: i64| {
                let vector = caller
                    .data()
                    .handles
                    .get(Handle::from_abi(vector))
                    .map_err(host_error)?;
                let item = caller
                    .data()
                    .handles
                    .get(Handle::from_abi(item))
                    .map_err(host_error)?;
                let Value::Vector(values) = vector else {
                    return Err(host_error("whole-Wasm vector handle expected".into()));
                };
                let value = Value::Vector(crate::lang::data::Vector::from_iter(
                    values.iter().cloned().chain(std::iter::once(item)),
                ));
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
            "map_empty",
            |mut caller: wasmtime::Caller<'_, HostState>| {
                let value = crate::core::vm_build_map(Vec::new()).map_err(host_error)?;
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
            "map_assoc",
            |mut caller: wasmtime::Caller<'_, HostState>, map: i64, key: i64, value: i64| {
                let arguments = [map, key, value]
                    .into_iter()
                    .map(|handle| {
                        caller
                            .data()
                            .handles
                            .get(Handle::from_abi(handle))
                            .map_err(host_error)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let value = crate::core::apply_primitive(crate::core::Primitive::Assoc, &arguments)
                    .map_err(host_error)?;
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
            "get",
            |mut caller: wasmtime::Caller<'_, HostState>, collection: i64, key: i64| {
                let arguments = [collection, key]
                    .into_iter()
                    .map(|handle| {
                        caller
                            .data()
                            .handles
                            .get(Handle::from_abi(handle))
                            .map_err(host_error)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let value = crate::core::apply_primitive(crate::core::Primitive::Get, &arguments)
                    .map_err(host_error)?;
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
            "is_number",
            |caller: wasmtime::Caller<'_, HostState>, value: i64| {
                let value = caller
                    .data()
                    .handles
                    .get(Handle::from_abi(value))
                    .map_err(host_error)?;
                Ok::<i64, wasmtime::Error>(i64::from(matches!(value, Value::Number(_))))
            },
        )
        .map_err(|error| error.to_string())?;
    linker
        .func_wrap(
            "hara",
            "count",
            |caller: wasmtime::Caller<'_, HostState>, collection: i64| {
                let collection = caller
                    .data()
                    .handles
                    .get(Handle::from_abi(collection))
                    .map_err(host_error)?;
                match crate::core::apply_primitive(crate::core::Primitive::Count, &[collection])
                    .map_err(host_error)?
                {
                    Value::Number(value) => Ok(value),
                    _ => Err(host_error("count returned a non-integer".into())),
                }
            },
        )
        .map_err(|error| error.to_string())?;
    linker
        .func_wrap(
            "hara",
            "nth",
            |mut caller: wasmtime::Caller<'_, HostState>, collection: i64, index: i64| {
                let collection = caller
                    .data()
                    .handles
                    .get(Handle::from_abi(collection))
                    .map_err(host_error)?;
                let value = crate::core::apply_primitive(
                    crate::core::Primitive::Nth,
                    &[collection, Value::Number(index)],
                )
                .map_err(host_error)?;
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
            "map_i64_pair",
            |mut caller: wasmtime::Caller<'_, HostState>, key: i64, value: i64| {
                let key = caller
                    .data()
                    .handles
                    .get(Handle::from_abi(key))
                    .map_err(host_error)?;
                let map = crate::core::vm_build_map(vec![key, Value::Number(value)])
                    .map_err(host_error)?;
                caller
                    .data_mut()
                    .handles
                    .insert(map)
                    .map(Handle::to_abi)
                    .map_err(host_error)
            },
        )
        .map_err(|error| error.to_string())?;
    linker
        .func_wrap(
            "hara",
            "get_i64",
            |caller: wasmtime::Caller<'_, HostState>, collection: i64, key: i64| {
                let collection = caller
                    .data()
                    .handles
                    .get(Handle::from_abi(collection))
                    .map_err(host_error)?;
                let key = caller
                    .data()
                    .handles
                    .get(Handle::from_abi(key))
                    .map_err(host_error)?;
                match crate::core::apply_primitive(crate::core::Primitive::Get, &[collection, key])
                    .map_err(host_error)?
                {
                    Value::Number(value) => Ok(value),
                    _ => Err(host_error("get returned a non-integer".into())),
                }
            },
        )
        .map_err(|error| error.to_string())?;
    linker
        .func_wrap(
            "hara",
            "get_path_i64_constants",
            |caller: wasmtime::Caller<'_, HostState>,
             collection: i64,
             first_key: i64,
             second_key: i64| {
                let collection = caller
                    .data()
                    .handles
                    .get(Handle::from_abi(collection))
                    .map_err(host_error)?;
                let constant = |index: i64| {
                    usize::try_from(index)
                        .ok()
                        .and_then(|index| caller.data().constants.get(index))
                        .cloned()
                        .ok_or_else(|| host_error("whole-Wasm constant is missing".into()))
                };
                let first = crate::core::apply_primitive(
                    crate::core::Primitive::Get,
                    &[collection, constant(first_key)?],
                )
                .map_err(host_error)?;
                match crate::core::apply_primitive(
                    crate::core::Primitive::Get,
                    &[first, constant(second_key)?],
                )
                .map_err(host_error)?
                {
                    Value::Number(value) => Ok(value),
                    _ => Err(host_error("nested get returned a non-integer".into())),
                }
            },
        )
        .map_err(|error| error.to_string())?;
    linker
        .func_wrap(
            "hara",
            "assoc_map_i64_pair",
            |mut caller: wasmtime::Caller<'_, HostState>,
             collection: i64,
             outer_key: i64,
             inner_key: i64,
             value: i64| {
                let resolve = |handle| {
                    caller
                        .data()
                        .handles
                        .get(Handle::from_abi(handle))
                        .map_err(host_error)
                };
                let collection = resolve(collection)?;
                let outer_key = resolve(outer_key)?;
                let inner_key = resolve(inner_key)?;
                let nested = crate::core::vm_build_map(vec![inner_key, Value::Number(value)])
                    .map_err(host_error)?;
                let result = crate::core::apply_primitive(
                    crate::core::Primitive::Assoc,
                    &[collection, outer_key, nested],
                )
                .map_err(host_error)?;
                caller
                    .data_mut()
                    .handles
                    .insert(result)
                    .map(Handle::to_abi)
                    .map_err(host_error)
            },
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn array_index(value: i64) -> Result<usize, wasmtime::Error> {
    usize::try_from(value).map_err(|_| host_error("array index must be non-negative".into()))
}

fn host_error(message: String) -> wasmtime::Error {
    wasmtime::Error::msg(message)
}
