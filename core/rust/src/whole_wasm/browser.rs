use wasm_bindgen::prelude::*;

use crate::core::{self, Primitive, Value};

use super::artifact::decode_artifact;
use super::handles::{Handle, HandleScope};

/// Browser-side owner for the dynamic Hara values referenced by a generated
/// whole-Wasm module. JavaScript supplies these methods as synchronous imports
/// while scalar and specialized aggregate work remains inside generated Wasm.
#[wasm_bindgen]
pub struct WholeWasmHost {
    constants: Vec<Value>,
    capabilities: Vec<bool>,
    handles: HandleScope,
}

#[wasm_bindgen]
impl WholeWasmHost {
    #[wasm_bindgen(constructor)]
    pub fn new(bytes: &[u8]) -> Result<WholeWasmHost, JsValue> {
        let artifact = decode_artifact(bytes).map_err(js_error)?;
        Ok(Self {
            constants: artifact.program.constants,
            capabilities: artifact.capabilities,
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
        match self.get(handle)? {
            Value::Number(value) => Ok(value),
            Value::BigInteger(_) => Err(js_error(
                "whole-Wasm integer overflow: value is outside signed 64-bit range".into(),
            )),
            _ => Err(js_error("whole-Wasm value is not an integer".into())),
        }
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

    #[wasm_bindgen(js_name = vectorEmpty)]
    pub fn vector_empty(&mut self) -> Result<i64, JsValue> {
        self.insert(Value::Vector(crate::lang::data::Vector::new()))
    }

    #[wasm_bindgen(js_name = vectorPush)]
    pub fn vector_push(&mut self, vector: i64, item: i64) -> Result<i64, JsValue> {
        let Value::Vector(values) = self.get(vector)? else {
            return Err(js_error("whole-Wasm vector handle expected".into()));
        };
        let item = self.get(item)?;
        self.insert(Value::Vector(crate::lang::data::Vector::from_iter(
            values.iter().cloned().chain(std::iter::once(item)),
        )))
    }

    #[wasm_bindgen(js_name = mapEmpty)]
    pub fn map_empty(&mut self) -> Result<i64, JsValue> {
        self.insert(core::vm_build_map(Vec::new()).map_err(js_error)?)
    }

    #[wasm_bindgen(js_name = mapAssoc)]
    pub fn map_assoc(&mut self, map: i64, key: i64, value: i64) -> Result<i64, JsValue> {
        let result = core::apply_primitive(
            Primitive::Assoc,
            &[self.get(map)?, self.get(key)?, self.get(value)?],
        )
        .map_err(js_error)?;
        self.insert(result)
    }

    #[wasm_bindgen(js_name = getValue)]
    pub fn get_value(&mut self, collection: i64, key: i64) -> Result<i64, JsValue> {
        let result =
            core::apply_primitive(Primitive::Get, &[self.get(collection)?, self.get(key)?])
                .map_err(js_error)?;
        self.insert(result)
    }

    #[wasm_bindgen(js_name = isNumber)]
    pub fn is_number(&self, value: i64) -> Result<i64, JsValue> {
        Ok(i64::from(matches!(
            self.get(value)?,
            Value::Number(_) | Value::BigInteger(_)
        )))
    }

    pub fn count(&self, collection: i64) -> Result<i64, JsValue> {
        match core::apply_primitive(Primitive::Count, &[self.get(collection)?]).map_err(js_error)? {
            Value::Number(value) => Ok(value),
            _ => Err(js_error("count returned a non-integer".into())),
        }
    }

    pub fn nth(&mut self, collection: i64, index: i64) -> Result<i64, JsValue> {
        let result = core::apply_primitive(
            Primitive::Nth,
            &[self.get(collection)?, Value::Number(index)],
        )
        .map_err(js_error)?;
        self.insert(result)
    }

    #[wasm_bindgen(js_name = mapI64Pair)]
    pub fn map_i64_pair(&mut self, key: i64, value: i64) -> Result<i64, JsValue> {
        let map =
            core::vm_build_map(vec![self.get(key)?, Value::Number(value)]).map_err(js_error)?;
        self.insert(map)
    }

    #[wasm_bindgen(js_name = getI64)]
    pub fn get_i64(&self, collection: i64, key: i64) -> Result<i64, JsValue> {
        match core::apply_primitive(Primitive::Get, &[self.get(collection)?, self.get(key)?])
            .map_err(js_error)?
        {
            Value::Number(value) => Ok(value),
            Value::BigInteger(_) => Err(js_error(
                "whole-Wasm integer overflow: value is outside signed 64-bit range".into(),
            )),
            _ => Err(js_error("get returned a non-integer".into())),
        }
    }

    #[wasm_bindgen(js_name = getPathI64Constants)]
    pub fn get_path_i64_constants(
        &self,
        collection: i64,
        first_key: i64,
        second_key: i64,
    ) -> Result<i64, JsValue> {
        let constant = |index: i64| {
            usize::try_from(index)
                .ok()
                .and_then(|index| self.constants.get(index))
                .cloned()
                .ok_or_else(|| js_error("whole-Wasm constant is missing".into()))
        };
        let first = core::apply_primitive(
            Primitive::Get,
            &[self.get(collection)?, constant(first_key)?],
        )
        .map_err(js_error)?;
        match core::apply_primitive(Primitive::Get, &[first, constant(second_key)?])
            .map_err(js_error)?
        {
            Value::Number(value) => Ok(value),
            Value::BigInteger(_) => Err(js_error(
                "whole-Wasm integer overflow: value is outside signed 64-bit range".into(),
            )),
            _ => Err(js_error("nested get returned a non-integer".into())),
        }
    }

    #[wasm_bindgen(js_name = assocMapI64Pair)]
    pub fn assoc_map_i64_pair(
        &mut self,
        collection: i64,
        outer_key: i64,
        inner_key: i64,
        value: i64,
    ) -> Result<i64, JsValue> {
        let nested = core::vm_build_map(vec![self.get(inner_key)?, Value::Number(value)])
            .map_err(js_error)?;
        let result = core::apply_primitive(
            Primitive::Assoc,
            &[self.get(collection)?, self.get(outer_key)?, nested],
        )
        .map_err(js_error)?;
        self.insert(result)
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
}

fn js_error(error: String) -> JsValue {
    JsValue::from_str(&error)
}
