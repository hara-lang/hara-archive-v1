use wasm_bindgen::prelude::*;

use crate::core::{self, Value};

use super::artifact::decode_artifact;
use super::bridge::{self, Slot, TargetKind, RESULT_BOOL, RESULT_HANDLE, RESULT_I64};
use super::handles::{Handle, HandleScope};

/// Browser-side owner for the dynamic Hara values referenced by a generated
/// whole-Wasm module. JavaScript supplies these methods as synchronous imports
/// while scalar and specialized aggregate work remains inside generated Wasm.
#[wasm_bindgen]
pub struct WholeWasmHost {
    constants: Vec<Value>,
    capabilities: Vec<bool>,
    targets: Vec<bridge::TargetDescriptor>,
    entry_function: u16,
    handles: HandleScope,
}

#[wasm_bindgen]
impl WholeWasmHost {
    #[wasm_bindgen(constructor)]
    pub fn new(bytes: &[u8]) -> Result<WholeWasmHost, JsValue> {
        let artifact = decode_artifact(bytes).map_err(js_error)?;
        Ok(Self {
            entry_function: artifact.program.entry,
            constants: artifact.program.constants,
            capabilities: artifact.capabilities,
            targets: artifact.targets,
            handles: HandleScope::default(),
        })
    }

    #[wasm_bindgen(js_name = beginCall)]
    pub fn begin_call(&mut self) {
        self.handles.begin_call();
    }

    #[wasm_bindgen(js_name = supportsNative)]
    pub fn supports_native(&self, function: i64) -> bool {
        usize::try_from(function)
            .ok()
            .and_then(|index| self.capabilities.get(index))
            .copied()
            .unwrap_or(false)
    }

    #[wasm_bindgen(js_name = entryFunction)]
    pub fn entry_function(&self) -> i64 {
        i64::from(self.entry_function)
    }

    /// Dispatches one bounded synchronous target call. JavaScript decodes the
    /// Wasm memory bytes into `[kind, payload]` pairs before entering this
    /// wasm-bindgen method; the logical ABI is identical to the Wasmtime host.
    #[wasm_bindgen(js_name = targetCall)]
    pub fn target_call(
        &mut self,
        target: i64,
        arguments: JsValue,
        result_mode: i64,
    ) -> Result<i64, JsValue> {
        let slots = parse_slots(arguments)?;
        let values = slots
            .iter()
            .map(|slot| self.resolve_slot(*slot))
            .collect::<Result<Vec<_>, _>>()?;
        let descriptor = usize::try_from(target)
            .ok()
            .and_then(|index| self.targets.get(index))
            .cloned()
            .ok_or_else(|| js_error(format!("unknown whole-Wasm target {target}")))?;
        bridge::validate_target_call(&descriptor, values.len(), result_mode).map_err(js_error)?;
        let value = match descriptor.kind {
            TargetKind::Protocol => core::protocol_intrinsic_call(&descriptor.symbol, &values),
            TargetKind::Native => core::apply_intrinsic_name(&descriptor.symbol, &values),
            TargetKind::VectorConstruct | TargetKind::MapConstruct => {
                return Err(js_error(format!(
                    "whole-Wasm target is not callable: {}",
                    descriptor.symbol
                )))
            }
        }
        .map_err(js_error)?;
        self.encode_result(value, result_mode)
    }

    #[wasm_bindgen(js_name = valueConstruct)]
    pub fn value_construct(&mut self, target: i64, arguments: JsValue) -> Result<i64, JsValue> {
        let values = parse_slots(arguments)?
            .iter()
            .map(|slot| self.resolve_slot(*slot))
            .collect::<Result<Vec<_>, _>>()?;
        let descriptor = usize::try_from(target)
            .ok()
            .and_then(|index| self.targets.get(index))
            .cloned()
            .ok_or_else(|| js_error(format!("unknown whole-Wasm structural target {target}")))?;
        bridge::validate_value_construct(&descriptor, values.len()).map_err(js_error)?;
        let value = match descriptor.kind {
            TargetKind::VectorConstruct => {
                Value::Vector(crate::lang::data::Vector::from_iter(values))
            }
            TargetKind::MapConstruct => core::vm_build_map(values).map_err(js_error)?,
            TargetKind::Protocol | TargetKind::Native => {
                return Err(js_error(format!(
                    "whole-Wasm target is not a value constructor: {}",
                    descriptor.symbol
                )))
            }
        };
        self.insert(value)
    }

    #[wasm_bindgen(js_name = constantHandle)]
    pub fn constant_handle(&mut self, index: i64) -> Result<i64, JsValue> {
        let value = self
            .constants
            .get(usize::try_from(index).map_err(|_| js_error("invalid constant".into()))?)
            .cloned()
            .ok_or_else(|| js_error("constant index out of range".into()))?;
        self.insert(value)
    }

    #[wasm_bindgen(js_name = boxI64)]
    pub fn box_i64(&mut self, value: i64) -> Result<i64, JsValue> {
        self.insert(Value::Number(value))
    }

    #[wasm_bindgen(js_name = unboxI64)]
    pub fn unbox_i64(&self, handle: i64) -> Result<i64, JsValue> {
        let value = self.get(handle)?;
        crate::numeric::to_i64_integer(&value).map_err(|error| js_error(error))
    }

    #[wasm_bindgen(js_name = boxBigInt)]
    pub fn box_big_int(&mut self, value: JsValue) -> Result<i64, JsValue> {
        if !value.is_bigint() {
            return Err(js_error("whole-Wasm BigInt value expected".into()));
        }
        let value: js_sys::BigInt = value.unchecked_into();
        let text = value
            .to_string(10)
            .map_err(|error| js_error(format!("whole-Wasm BigInt is invalid: {error:?}")))?
            .as_string()
            .ok_or_else(|| js_error("whole-Wasm BigInt has no decimal representation".into()))?;
        let value = num_bigint::BigInt::parse_bytes(text.as_bytes(), 10)
            .ok_or_else(|| js_error("whole-Wasm BigInt is invalid".into()))?;
        self.insert(crate::numeric::compact_integer(value))
    }

    #[wasm_bindgen(js_name = unboxBigInt)]
    pub fn unbox_big_int(&self, handle: i64) -> Result<JsValue, JsValue> {
        match self.get(handle)? {
            Value::Number(value) => Ok(js_sys::BigInt::from(value).into()),
            Value::BigInteger(value) => js_sys::BigInt::new(&JsValue::from_str(&value.to_string()))
                .map(Into::into)
                .map_err(|error| js_error(format!("whole-Wasm BigInt is invalid: {error:?}"))),
            _ => Err(js_error("whole-Wasm value is not an integer".into())),
        }
    }
}

impl WholeWasmHost {
    fn insert(&mut self, value: Value) -> Result<i64, JsValue> {
        self.handles
            .insert(value)
            .map(Handle::to_abi)
            .map_err(js_error)
    }

    fn get(&self, handle: i64) -> Result<Value, JsValue> {
        self.handles.get(Handle::from_abi(handle)).map_err(js_error)
    }

    fn resolve_slot(&self, slot: Slot) -> Result<Value, JsValue> {
        match slot.kind {
            bridge::SLOT_HANDLE => self.get(slot.payload),
            bridge::SLOT_I64 => Ok(Value::Number(slot.payload)),
            bridge::SLOT_BOOL => Ok(Value::Bool(slot.payload != 0)),
            bridge::SLOT_NIL => Ok(Value::Nil),
            bridge::SLOT_CONSTANT => self
                .constants
                .get(
                    usize::try_from(slot.payload)
                        .map_err(|_| js_error("invalid constant".into()))?,
                )
                .cloned()
                .ok_or_else(|| js_error("whole-Wasm constant index out of range".into())),
            _ => Err(js_error("invalid whole-Wasm bridge slot".into())),
        }
    }

    fn encode_result(&mut self, value: Value, mode: i64) -> Result<i64, JsValue> {
        bridge::validate_result_mode(mode).map_err(js_error)?;
        match mode {
            RESULT_HANDLE => self.insert(value),
            RESULT_I64 => crate::numeric::to_i64_integer(&value)
                .map_err(|_| js_error("whole-Wasm integer overflow".into())),
            RESULT_BOOL => match value {
                Value::Bool(value) => Ok(i64::from(value)),
                _ => Err(js_error(
                    "whole-Wasm target did not return a boolean".into(),
                )),
            },
            _ => unreachable!("result mode validated above"),
        }
    }
}

fn parse_slots(value: JsValue) -> Result<Vec<Slot>, JsValue> {
    let slots = js_sys::Array::from(&value);
    let mut result = Vec::with_capacity(slots.length() as usize);
    for item in slots.iter() {
        let pair = js_sys::Array::from(&item);
        if pair.length() != 2 {
            return Err(js_error(
                "whole-Wasm bridge slot must have two fields".into(),
            ));
        }
        let kind = pair
            .get(0)
            .as_f64()
            .and_then(|value| u32::try_from(value as i64).ok())
            .ok_or_else(|| js_error("whole-Wasm bridge slot kind is invalid".into()))?;
        let payload = js_i64(pair.get(1))?;
        result.push(Slot { kind, payload });
    }
    bridge::validate_slots(&result).map_err(js_error)?;
    Ok(result)
}

fn js_i64(value: JsValue) -> Result<i64, JsValue> {
    if value.is_bigint() {
        let value: js_sys::BigInt = value.unchecked_into();
        let text = value
            .to_string(10)
            .map_err(|error| js_error(format!("invalid bridge integer: {error:?}")))?
            .as_string()
            .ok_or_else(|| js_error("invalid bridge integer".into()))?;
        return text
            .parse::<i64>()
            .map_err(|_| js_error("bridge integer is outside signed 64-bit range".into()));
    }
    value
        .as_f64()
        .and_then(|value| i64::try_from(value as i128).ok())
        .ok_or_else(|| js_error("whole-Wasm bridge integer is invalid".into()))
}

fn js_error(error: String) -> JsValue {
    JsValue::from_str(&error)
}
